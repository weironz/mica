//! Web IndexedDB write-behind mirror & hydration for `crates/mica-sync`.
//!
//! Persists [`mica_sync::IdbHydrateSnapshot`] records keyed by
//! `(cache_origin, doc_id)` (preserving `#111` account isolation) in IndexedDB
//! database `mica_sync_v1` / object store `docs`, applying immutable
//! [`mica_sync::IdbMirrorOp`] snapshots (`#161`) atomically inside Web Locks
//! (`navigator.locks.request`) when supported so multiple browser tabs never
//! interleave read-modify-write transactions on the same document.
//!
//! Pure snapshot-application logic is compiled on all targets and unit-tested
//! natively on desktop.

use mica_sync::{IdbHydrateSnapshot, IdbMirrorOp};

/// Composite primary key for one `(cache_origin, doc_id)` record in IndexedDB.
pub fn idb_doc_key(cache_origin: &str, doc_id: &str) -> String {
  format!("{}\0{}", cache_origin.trim(), doc_id.trim())
}

/// Apply one immutable `#161` [`IdbMirrorOp`] snapshot onto an in-memory
/// [`IdbHydrateSnapshot`], keeping monotonic cursor and outbox invariants.
pub fn apply_mirror_op(snap: &mut IdbHydrateSnapshot, op: &IdbMirrorOp) {
  match op {
    IdbMirrorOp::SaveBase { state, cursor } => {
      snap.base = Some(state.clone());
      snap.cursor.last_synced_rid = snap.cursor.last_synced_rid.max(cursor.last_synced_rid);
      snap.cursor.pushed_clock = snap.cursor.pushed_clock.max(cursor.pushed_clock);
    }
    IdbMirrorOp::AppendOutbox { rows } => {
      snap.outbox = rows.clone();
    }
    IdbMirrorOp::AppendRemote { rows, cursor } => {
      snap.remote = rows.clone();
      snap.cursor.last_synced_rid = snap.cursor.last_synced_rid.max(cursor.last_synced_rid);
      snap.cursor.pushed_clock = snap.cursor.pushed_clock.max(cursor.pushed_clock);
    }
    IdbMirrorOp::AdvanceCursor { cursor } => {
      snap.cursor.last_synced_rid = snap.cursor.last_synced_rid.max(cursor.last_synced_rid);
      snap.cursor.pushed_clock = snap.cursor.pushed_clock.max(cursor.pushed_clock);
    }
    IdbMirrorOp::Compact {
      state,
      outbox,
      remote,
      cursor,
    } => {
      snap.base = Some(state.clone());
      snap.outbox = outbox.clone();
      snap.remote = remote.clone();
      snap.cursor = *cursor;
    }
  }
}

/// Apply a batch of [`IdbMirrorOp`]s sequentially to a snapshot.
pub fn apply_mirror_ops(snap: &mut IdbHydrateSnapshot, ops: &[IdbMirrorOp]) {
  for op in ops {
    apply_mirror_op(snap, op);
  }
}

/// Serialize a snapshot for IndexedDB storage.
pub fn encode_snapshot(snap: &IdbHydrateSnapshot) -> Result<String, String> {
  serde_json::to_string(snap).map_err(|e| format!("failed to encode IDB snapshot: {e}"))
}

/// Decode a persisted IndexedDB snapshot string.
pub fn decode_snapshot(raw: &str) -> Result<IdbHydrateSnapshot, String> {
  serde_json::from_str(raw).map_err(|e| format!("failed to decode IDB snapshot: {e}"))
}

// ── Browser IndexedDB + Web Locks implementation (wasm32) ────────────────────

#[cfg(target_arch = "wasm32")]
mod imp {
  use std::cell::RefCell;
  use std::future::Future;
  use std::pin::Pin;
  use std::rc::Rc;
  use std::task::{Context, Poll, Waker};

  use mica_sync::{IdbHydrateSnapshot, IdbMirrorOp};
  use wasm_bindgen::closure::Closure;
  use wasm_bindgen::{JsCast, JsValue};
  use wasm_bindgen_futures::{JsFuture, future_to_promise};
  use web_sys::{
    Event, IdbDatabase, IdbOpenDbRequest, IdbRequest, IdbTransactionMode, IdbVersionChangeEvent,
  };

  use super::{apply_mirror_ops, decode_snapshot, encode_snapshot, idb_doc_key};

  const DB_NAME: &str = "mica_sync_v1";
  const DB_VERSION: u32 = 1;
  const STORE_DOCS: &str = "docs";

  struct ReqState {
    result: Option<Result<JsValue, String>>,
    waker: Option<Waker>,
    _onsuccess: Option<Closure<dyn FnMut(Event)>>,
    _onerror: Option<Closure<dyn FnMut(Event)>>,
    _onupgrade: Option<Closure<dyn FnMut(IdbVersionChangeEvent)>>,
  }

  impl ReqState {
    fn resolve(&mut self, res: Result<JsValue, String>) {
      if self.result.is_some() {
        return;
      }
      self._onsuccess = None;
      self._onerror = None;
      self._onupgrade = None;
      self.result = Some(res);
      if let Some(w) = self.waker.take() {
        w.wake();
      }
    }
  }

  struct ReqFuture {
    state: Rc<RefCell<ReqState>>,
  }

  impl Future for ReqFuture {
    type Output = Result<JsValue, String>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
      let mut st = self.state.borrow_mut();
      if let Some(res) = st.result.take() {
        Poll::Ready(res)
      } else {
        st.waker = Some(cx.waker().clone());
        Poll::Pending
      }
    }
  }

  async fn await_idb_request(req: &IdbRequest) -> Result<JsValue, String> {
    let state = Rc::new(RefCell::new(ReqState {
      result: None,
      waker: None,
      _onsuccess: None,
      _onerror: None,
      _onupgrade: None,
    }));

    let req_ok = req.clone();
    let st_ok = Rc::clone(&state);
    let onsuccess = Closure::<dyn FnMut(Event)>::new(move |_ev: Event| {
      let res = req_ok
        .result()
        .map_err(|e| format!("IDB request result error: {e:?}"));
      st_ok.borrow_mut().resolve(res);
    });

    let req_err = req.clone();
    let st_err = Rc::clone(&state);
    let onerror = Closure::<dyn FnMut(Event)>::new(move |_ev: Event| {
      let err = req_err
        .error()
        .ok()
        .flatten()
        .map(|e| format!("IDB error: {e:?}"))
        .unwrap_or_else(|| "IDB request failed".to_string());
      st_err.borrow_mut().resolve(Err(err));
    });

    req.set_onsuccess(Some(onsuccess.as_ref().unchecked_ref()));
    req.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    {
      let mut st = state.borrow_mut();
      st._onsuccess = Some(onsuccess);
      st._onerror = Some(onerror);
    }

    let out = ReqFuture { state }.await;
    req.set_onsuccess(None);
    req.set_onerror(None);
    out
  }

  async fn open_db() -> Result<IdbDatabase, String> {
    let window = web_sys::window().ok_or_else(|| "window unavailable".to_string())?;
    let factory = window
      .indexed_db()
      .map_err(|e| format!("indexedDB unavailable: {e:?}"))?
      .ok_or_else(|| "indexedDB is disabled in this browser context".to_string())?;
    let open_req: IdbOpenDbRequest = factory
      .open_with_u32(DB_NAME, DB_VERSION)
      .map_err(|e| format!("indexedDB.open failed: {e:?}"))?;

    let state = Rc::new(RefCell::new(ReqState {
      result: None,
      waker: None,
      _onsuccess: None,
      _onerror: None,
      _onupgrade: None,
    }));

    let onupgrade =
      Closure::<dyn FnMut(IdbVersionChangeEvent)>::new(move |ev: IdbVersionChangeEvent| {
        if let Some(target) = ev.target()
          && let Ok(req) = target.dyn_into::<IdbOpenDbRequest>()
          && let Ok(val) = req.result()
          && let Ok(db) = val.dyn_into::<IdbDatabase>()
          && !db.object_store_names().contains(STORE_DOCS)
        {
          let _ = db.create_object_store(STORE_DOCS);
        }
      });

    let req_ok = open_req.clone();
    let st_ok = Rc::clone(&state);
    let onsuccess = Closure::<dyn FnMut(Event)>::new(move |_ev: Event| {
      let res = req_ok
        .result()
        .map_err(|e| format!("indexedDB.open result error: {e:?}"));
      st_ok.borrow_mut().resolve(res);
    });

    let req_err = open_req.clone();
    let st_err = Rc::clone(&state);
    let onerror = Closure::<dyn FnMut(Event)>::new(move |_ev: Event| {
      let err = req_err
        .error()
        .ok()
        .flatten()
        .map(|e| format!("indexedDB.open error: {e:?}"))
        .unwrap_or_else(|| "indexedDB.open failed".to_string());
      st_err.borrow_mut().resolve(Err(err));
    });

    open_req.set_onupgradeneeded(Some(onupgrade.as_ref().unchecked_ref()));
    open_req.set_onsuccess(Some(onsuccess.as_ref().unchecked_ref()));
    open_req.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    {
      let mut st = state.borrow_mut();
      st._onupgrade = Some(onupgrade);
      st._onsuccess = Some(onsuccess);
      st._onerror = Some(onerror);
    }

    let val = ReqFuture { state }.await?;
    open_req.set_onupgradeneeded(None);
    open_req.set_onsuccess(None);
    open_req.set_onerror(None);
    val
      .dyn_into::<IdbDatabase>()
      .map_err(|_| "indexedDB.open did not return IDBDatabase".to_string())
  }

  /// Run `op` inside a browser exclusive Web Lock (`navigator.locks.request`)
  /// when available, falling back to direct execution in browsers without Web
  /// Locks (e.g. insecure contexts). Prevents cross-tab read-modify-write races.
  async fn with_doc_web_lock<T, F, Fut>(lock_name: &str, op: F) -> Result<T, String>
  where
    T: 'static,
    F: FnOnce() -> Fut + 'static,
    Fut: Future<Output = Result<T, String>> + 'static,
  {
    let locks_obj = web_sys::window()
      .and_then(|w| js_sys::Reflect::get(&w, &JsValue::from_str("navigator")).ok())
      .and_then(|nav| js_sys::Reflect::get(&nav, &JsValue::from_str("locks")).ok())
      .filter(|v| v.is_object());

    let Some(locks) = locks_obj else {
      return op().await;
    };
    let Ok(req_fn) = js_sys::Reflect::get(&locks, &JsValue::from_str("request")) else {
      return op().await;
    };
    let Ok(req_fn) = req_fn.dyn_into::<js_sys::Function>() else {
      return op().await;
    };

    let result_slot: Rc<RefCell<Option<Result<T, String>>>> = Rc::new(RefCell::new(None));
    let slot_cb = Rc::clone(&result_slot);
    let op_cell = Rc::new(RefCell::new(Some(op)));

    let cb = Closure::once(move |_lock: JsValue| -> js_sys::Promise {
      let slot = Rc::clone(&slot_cb);
      let func = op_cell.borrow_mut().take();
      future_to_promise(async move {
        if let Some(f) = func {
          let out = f().await;
          *slot.borrow_mut() = Some(out);
        }
        Ok(JsValue::UNDEFINED)
      })
    });

    let promise_val = req_fn
      .call2(
        &locks,
        &JsValue::from_str(lock_name),
        cb.as_ref().unchecked_ref(),
      )
      .map_err(|e| format!("navigator.locks.request failed: {e:?}"))?;
    let promise: js_sys::Promise = promise_val
      .dyn_into()
      .map_err(|_| "navigator.locks.request did not return a Promise".to_string())?;
    JsFuture::from(promise)
      .await
      .map_err(|e| format!("navigator.locks promise rejected: {e:?}"))?;
    drop(cb);

    result_slot
      .borrow_mut()
      .take()
      .unwrap_or_else(|| Err("web lock callback did not complete".to_string()))
  }

  /// Load a persisted [`IdbHydrateSnapshot`] for `(cache_origin, doc_id)` from
  /// IndexedDB under an exclusive Web Lock.
  pub async fn load_doc_snapshot(
    cache_origin: &str,
    doc_id: &str,
  ) -> Result<Option<IdbHydrateSnapshot>, String> {
    let key = idb_doc_key(cache_origin, doc_id);
    let lock_name = format!("mica-idb:{key}");
    with_doc_web_lock(&lock_name, move || async move {
      let db = open_db().await?;
      let tx = db
        .transaction_with_str_and_mode(STORE_DOCS, IdbTransactionMode::Readonly)
        .map_err(|e| format!("IDB readonly tx failed: {e:?}"))?;
      let store = tx
        .object_store(STORE_DOCS)
        .map_err(|e| format!("IDB object_store failed: {e:?}"))?;
      let req = store
        .get(&JsValue::from_str(&key))
        .map_err(|e| format!("IDB get failed: {e:?}"))?;
      let val = await_idb_request(&req).await?;
      db.close();
      if val.is_undefined() || val.is_null() {
        return Ok(None);
      }
      let Some(raw) = val.as_string() else {
        return Ok(None);
      };
      match decode_snapshot(&raw) {
        Ok(snap) => Ok(Some(snap)),
        Err(_) => Ok(None),
      }
    })
    .await
  }

  /// Atomically fold a FIFO batch of [`IdbMirrorOp`] snapshots into IndexedDB
  /// for `(cache_origin, doc_id)` under an exclusive Web Lock.
  pub async fn commit_mirror_ops(
    cache_origin: &str,
    doc_id: &str,
    ops: Vec<IdbMirrorOp>,
  ) -> Result<(), String> {
    if ops.is_empty() {
      return Ok(());
    }
    let key = idb_doc_key(cache_origin, doc_id);
    let lock_name = format!("mica-idb:{key}");
    with_doc_web_lock(&lock_name, move || async move {
      let db = open_db().await?;
      let tx = db
        .transaction_with_str_and_mode(STORE_DOCS, IdbTransactionMode::Readwrite)
        .map_err(|e| format!("IDB readwrite tx failed: {e:?}"))?;
      let store = tx
        .object_store(STORE_DOCS)
        .map_err(|e| format!("IDB object_store failed: {e:?}"))?;
      let get_req = store
        .get(&JsValue::from_str(&key))
        .map_err(|e| format!("IDB get failed: {e:?}"))?;
      let existing_val = await_idb_request(&get_req).await?;
      let mut snap = existing_val
        .as_string()
        .and_then(|raw| decode_snapshot(&raw).ok())
        .unwrap_or_default();
      apply_mirror_ops(&mut snap, &ops);
      let encoded = encode_snapshot(&snap)?;
      let put_req = store
        .put_with_key(&JsValue::from_str(&encoded), &JsValue::from_str(&key))
        .map_err(|e| format!("IDB put failed: {e:?}"))?;
      let _ = await_idb_request(&put_req).await?;
      db.close();
      Ok(())
    })
    .await
  }
}

#[cfg(target_arch = "wasm32")]
pub use imp::{commit_mirror_ops, load_doc_snapshot};

#[cfg(test)]
mod tests {
  use super::*;
  use mica_core::{Block, MicaDoc};
  use mica_sync::{MemoryMirrorStorage, SyncConfig, SyncEffect, SyncEngine, SyncEvent};

  #[test]
  fn idb_mirror_ops_persist_offline_edits_and_hydrate_across_reload() {
    let key_a = idb_doc_key(r#"["https://mica.example.com","user-a"]"#, "doc-1");
    let key_b = idb_doc_key(r#"["https://mica.example.com","user-b"]"#, "doc-1");
    assert_ne!(key_a, key_b);

    // Tab 1: seed with server base, apply offline edit, drain MirrorIdb ops into simulated IDB.
    let base = MicaDoc::from_blocks_with_client_id(
      "root",
      &[
        Block::new("root", "page").with_children(vec!["b1".into()]),
        Block::new("b1", "paragraph").with_text("Before offline"),
      ],
      Some(1),
    );
    let mut mem_storage = MemoryMirrorStorage::new();
    use mica_sync::SyncStorage;
    mem_storage.save_base(&base.encode_state(), 5).unwrap();
    let initial_ops = mem_storage.take_pending_mirror_txns();

    let mut idb_row = IdbHydrateSnapshot::default();
    apply_mirror_ops(&mut idb_row, &initial_ops);

    let mut engine1 = SyncEngine::new(
      SyncConfig {
        client_id: Some(11),
        self_name: Some("WebUser".into()),
        document_id: Some("doc-1".into()),
        restore_unacked: Vec::new(),
      },
      Some(Box::new(mem_storage)),
    );
    let _ = engine1.handle_event(SyncEvent::Connect);

    // Offline edit while disconnected:
    let sv0 = engine1.doc().unwrap().state_vector();
    engine1
      .doc_mut()
      .unwrap()
      .text_insert("b1", 14, " + offline edit");
    let diff = engine1.doc().unwrap().encode_diff(&sv0).unwrap();
    let effects = engine1.handle_event(SyncEvent::ApplyLocalDiff(diff));
    for ef in effects {
      if let SyncEffect::MirrorIdb(ops) = ef {
        apply_mirror_ops(&mut idb_row, &ops);
      }
    }

    // Simulate browser tab reload: encode -> decode JSON -> hydrate fresh MemoryMirrorStorage.
    let persisted_json = encode_snapshot(&idb_row).unwrap();
    let hydrated_snap = decode_snapshot(&persisted_json).unwrap();
    let storage2 = MemoryMirrorStorage::hydrate(hydrated_snap);
    let mut engine2 = SyncEngine::new(
      SyncConfig {
        client_id: Some(22),
        self_name: Some("WebUser".into()),
        document_id: Some("doc-1".into()),
        restore_unacked: Vec::new(),
      },
      Some(Box::new(storage2)),
    );
    let boot_effects = engine2.handle_event(SyncEvent::Connect);
    assert!(
      boot_effects
        .iter()
        .any(|e| matches!(e, SyncEffect::Ready { .. }))
    );
    assert!(
      !engine2.outbox_empty(),
      "offline edit must survive reload in IDB outbox"
    );
    assert!(
      engine2
        .all_blocks_json()
        .contains("Before offline + offline edit")
    );
  }
}
