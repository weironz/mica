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

// ── Legacy Flutter Web mirror (`mica-localfirst`) migration ──────────────────
//
// Flutter Web (<= 0.14.x) kept each cloud doc's offline edits in IndexedDB
// database `mica-localfirst`, stores `base` / `outbox` / `remote` / `cursor`,
// one record per doc keyed `"{cache_origin}|{doc_id}"` (the same #111 tuple
// origin used here). Without a migration, edits a user made offline in the
// Flutter Web client and never synced would be stranded at cutover. On every
// cloud doc open the pure-Rust client folds the legacy outbox rows the server
// has not acked (`clock > cursor.pushedClock`) into its own outbox, persists
// that, and only then deletes the legacy record — so a crash in between leaves
// a duplicate (harmless: yrs updates are idempotent), never a loss. `base` and
// `remote` are server-derived caches and are dropped, not migrated.

/// Row key of the legacy Flutter Web `mica-localfirst` stores.
pub fn legacy_flutter_doc_key(cache_origin: &str, doc_id: &str) -> String {
  format!("{cache_origin}|{doc_id}")
}

/// Append the legacy outbox rows the server never acknowledged
/// (`clock > legacy_pushed_clock`) to `snap.outbox`, in their original order,
/// renumbered after the snapshot's own clocks. Rows already present (a
/// re-run after an interrupted migration) are skipped. Returns how many rows
/// were appended.
pub fn merge_legacy_outbox(
  snap: &mut IdbHydrateSnapshot,
  mut legacy_rows: Vec<(i64, Vec<u8>)>,
  legacy_pushed_clock: i64,
) -> usize {
  legacy_rows.retain(|(clock, bytes)| *clock > legacy_pushed_clock && !bytes.is_empty());
  legacy_rows.sort_by_key(|(clock, _)| *clock);
  let mut next = snap
    .outbox
    .iter()
    .map(|e| e.clock)
    .max()
    .unwrap_or(0)
    .max(snap.cursor.pushed_clock);
  let mut appended = 0;
  for (_, bytes) in legacy_rows {
    if snap.outbox.iter().any(|e| e.bytes == bytes) {
      continue;
    }
    next += 1;
    snap
      .outbox
      .push(mica_sync::OutboxEntry { clock: next, bytes });
    appended += 1;
  }
  appended
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
  pub(super) async fn with_doc_web_lock<T, F, Fut>(lock_name: &str, op: F) -> Result<T, String>
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
mod legacy {
  use std::cell::Cell;

  use mica_sync::IdbHydrateSnapshot;
  use wasm_bindgen::closure::Closure;
  use wasm_bindgen::{JsCast, JsValue};
  use wasm_bindgen_futures::JsFuture;
  use web_sys::{Event, IdbDatabase, IdbOpenDbRequest, IdbRequest, IdbTransactionMode};

  use super::{
    decode_snapshot, encode_snapshot, idb_doc_key, legacy_flutter_doc_key, merge_legacy_outbox,
  };

  const LEGACY_DB: &str = "mica-localfirst";
  const LEGACY_STORES: [&str; 4] = ["base", "outbox", "remote", "cursor"];
  const NEW_DB: &str = "mica_sync_v1";
  const NEW_STORE: &str = "docs";

  thread_local! {
    /// Set once this page learns there is no legacy database, so later doc
    /// opens skip the probe.
    static LEGACY_ABSENT: Cell<bool> = const { Cell::new(false) };
  }

  /// Turn an IDBRequest into a future. NOT an `async fn` on purpose: the
  /// success/error handlers must be attached in the same turn the request is
  /// issued. Several requests issued together can complete before a lazily
  /// polled future would attach its handlers, and that future never resolves.
  fn request(req: &IdbRequest) -> JsFuture {
    let target = req.clone();
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
      let ok_req = target.clone();
      let on_ok = Closure::once_into_js(move |_e: Event| {
        let _ = resolve.call1(
          &JsValue::NULL,
          &ok_req.result().unwrap_or(JsValue::UNDEFINED),
        );
      });
      let on_err = Closure::once_into_js(move |_e: Event| {
        let _ = reject.call1(
          &JsValue::NULL,
          &JsValue::from_str("IndexedDB request failed"),
        );
      });
      target.set_onsuccess(Some(on_ok.unchecked_ref()));
      target.set_onerror(Some(on_err.unchecked_ref()));
    });
    JsFuture::from(promise)
  }

  /// Open the legacy database only if it already exists: a fresh open fires
  /// `upgradeneeded`, which we abort so no empty database is left behind.
  async fn open_existing_legacy_db() -> Result<Option<IdbDatabase>, String> {
    let factory = web_sys::window()
      .ok_or("window unavailable")?
      .indexed_db()
      .map_err(|e| format!("{e:?}"))?
      .ok_or("indexedDB disabled")?;
    let open_req: IdbOpenDbRequest = factory.open(LEGACY_DB).map_err(|e| format!("{e:?}"))?;
    let upgrade_req = open_req.clone();
    let on_upgrade = Closure::once_into_js(move |_e: Event| {
      if let Some(tx) = upgrade_req.transaction() {
        let _ = tx.abort();
      }
    });
    open_req.set_onupgradeneeded(Some(on_upgrade.unchecked_ref()));
    let req: &IdbRequest = open_req.as_ref();
    let Ok(val) = request(req).await else {
      return Ok(None); // aborted upgrade = the database did not exist
    };
    let db: IdbDatabase = val.dyn_into().map_err(|_| "not an IDBDatabase")?;
    let names = db.object_store_names();
    if !LEGACY_STORES.iter().all(|s| names.contains(s)) {
      db.close();
      return Ok(None);
    }
    Ok(Some(db))
  }

  fn bytes_of(v: &JsValue) -> Option<Vec<u8>> {
    if let Some(arr) = v.dyn_ref::<js_sys::Uint8Array>() {
      return Some(arr.to_vec());
    }
    let arr = v.dyn_ref::<js_sys::Array>()?;
    Some(
      arr
        .iter()
        .map(|n| n.as_f64().unwrap_or(0.0) as u8)
        .collect(),
    )
  }

  fn number_field(obj: &JsValue, name: &str) -> Option<i64> {
    js_sys::Reflect::get(obj, &JsValue::from_str(name))
      .ok()?
      .as_f64()
      .map(|n| n as i64)
  }

  fn err(e: JsValue) -> String {
    format!("{e:?}")
  }

  // An IndexedDB transaction closes as soon as control returns to the event
  // loop with no request pending, so every request of one transaction is
  // issued BEFORE the first await; a read-modify-write uses two transactions
  // (the caller holds the doc's Web Lock across both).

  /// Read the legacy outbox rows and acked clock for one doc.
  async fn read_legacy(
    db: &IdbDatabase,
    key: &str,
  ) -> Result<(Vec<(i64, Vec<u8>)>, i64, bool), String> {
    let stores = js_sys::Array::new();
    stores.push(&JsValue::from_str("outbox"));
    stores.push(&JsValue::from_str("cursor"));
    let tx = db
      .transaction_with_str_sequence_and_mode(&stores, IdbTransactionMode::Readonly)
      .map_err(err)?;
    let k = JsValue::from_str(key);
    let outbox_req = tx
      .object_store("outbox")
      .map_err(err)?
      .get(&k)
      .map_err(err)?;
    let cursor_req = tx
      .object_store("cursor")
      .map_err(err)?
      .get(&k)
      .map_err(err)?;
    let (outbox_fut, cursor_fut) = (request(&outbox_req), request(&cursor_req));
    let outbox_rec = outbox_fut.await.map_err(err)?;
    let cursor_rec = cursor_fut.await.map_err(err)?;
    let present = !outbox_rec.is_undefined() || !cursor_rec.is_undefined();
    let pushed = if cursor_rec.is_object() {
      number_field(&cursor_rec, "pushedClock").unwrap_or(0)
    } else {
      0
    };
    let mut rows = Vec::new();
    if outbox_rec.is_object()
      && let Ok(list) = js_sys::Reflect::get(&outbox_rec, &JsValue::from_str("rows"))
      && let Some(list) = list.dyn_ref::<js_sys::Array>()
    {
      for row in list.iter() {
        let clock = number_field(&row, "clock");
        let bytes = js_sys::Reflect::get(&row, &JsValue::from_str("bytes"))
          .ok()
          .and_then(|b| bytes_of(&b));
        if let (Some(clock), Some(bytes)) = (clock, bytes) {
          rows.push((clock, bytes));
        }
      }
    }
    Ok((rows, pushed, present))
  }

  async fn delete_legacy(db: &IdbDatabase, key: &str) -> Result<(), String> {
    let stores = js_sys::Array::new();
    for s in LEGACY_STORES {
      stores.push(&JsValue::from_str(s));
    }
    let tx = db
      .transaction_with_str_sequence_and_mode(&stores, IdbTransactionMode::Readwrite)
      .map_err(err)?;
    let k = JsValue::from_str(key);
    let mut pending = Vec::new();
    for s in LEGACY_STORES {
      let req = tx.object_store(s).map_err(err)?.delete(&k).map_err(err)?;
      pending.push(request(&req));
    }
    for fut in pending {
      fut.await.map_err(err)?;
    }
    Ok(())
  }

  /// Fold `rows` into the new `mica_sync_v1` record for the doc, persisted
  /// before returning.
  async fn persist_into_new(
    new_key: &str,
    rows: Vec<(i64, Vec<u8>)>,
    pushed: i64,
  ) -> Result<usize, String> {
    let factory = web_sys::window()
      .ok_or("window unavailable")?
      .indexed_db()
      .map_err(err)?
      .ok_or("indexedDB disabled")?;
    let open_req = factory.open_with_u32(NEW_DB, 1).map_err(err)?;
    let upgrade_req = open_req.clone();
    let on_upgrade = Closure::once_into_js(move |_e: Event| {
      if let Ok(val) = upgrade_req.result()
        && let Ok(db) = val.dyn_into::<IdbDatabase>()
        && !db.object_store_names().contains(NEW_STORE)
      {
        let _ = db.create_object_store(NEW_STORE);
      }
    });
    open_req.set_onupgradeneeded(Some(on_upgrade.unchecked_ref()));
    let req: &IdbRequest = open_req.as_ref();
    let db: IdbDatabase = request(req)
      .await
      .map_err(err)?
      .dyn_into()
      .map_err(|_| "not an IDBDatabase")?;
    let k = JsValue::from_str(new_key);
    let result = async {
      let read = db
        .transaction_with_str_and_mode(NEW_STORE, IdbTransactionMode::Readonly)
        .map_err(err)?
        .object_store(NEW_STORE)
        .map_err(err)?
        .get(&k)
        .map_err(err)?;
      let existing = request(&read).await.map_err(err)?;
      let mut snap: IdbHydrateSnapshot = existing
        .as_string()
        .and_then(|raw| decode_snapshot(&raw).ok())
        .unwrap_or_default();
      let appended = merge_legacy_outbox(&mut snap, rows, pushed);
      if appended > 0 {
        let encoded = encode_snapshot(&snap)?;
        let write = db
          .transaction_with_str_and_mode(NEW_STORE, IdbTransactionMode::Readwrite)
          .map_err(err)?
          .object_store(NEW_STORE)
          .map_err(err)?
          .put_with_key(&JsValue::from_str(&encoded), &k)
          .map_err(err)?;
        request(&write).await.map_err(err)?;
      }
      Ok(appended)
    }
    .await;
    db.close();
    result
  }

  /// Migrate one doc's unsynced Flutter Web edits into the pure-Rust store.
  /// Call before hydrating the doc. Returns how many updates were carried
  /// over. Safe to call repeatedly; a no-op once the legacy record is gone.
  pub async fn migrate_legacy_flutter_doc(
    cache_origin: &str,
    doc_id: &str,
  ) -> Result<usize, String> {
    if LEGACY_ABSENT.with(Cell::get) {
      return Ok(0);
    }
    let Some(legacy_db) = open_existing_legacy_db().await? else {
      LEGACY_ABSENT.with(|c| c.set(true));
      return Ok(0);
    };
    let legacy_key = legacy_flutter_doc_key(cache_origin, doc_id);
    let result = async {
      let (rows, pushed, present) = read_legacy(&legacy_db, &legacy_key).await?;
      if !present {
        return Ok(0);
      }
      // Persist into the new store first, then drop the legacy record: an
      // interruption between the two re-runs next open and dedups.
      let appended = super::imp::with_doc_web_lock(
        &format!("mica-idb:{}", idb_doc_key(cache_origin, doc_id)),
        {
          let new_key = idb_doc_key(cache_origin, doc_id);
          move || async move { persist_into_new(&new_key, rows, pushed).await }
        },
      )
      .await?;
      delete_legacy(&legacy_db, &legacy_key).await?;
      Ok(appended)
    }
    .await;
    legacy_db.close();
    result
  }
}

#[cfg(target_arch = "wasm32")]
pub use imp::{commit_mirror_ops, load_doc_snapshot};
#[cfg(target_arch = "wasm32")]
pub use legacy::migrate_legacy_flutter_doc;

#[cfg(test)]
mod tests {
  use super::*;
  use mica_core::{Block, MicaDoc};
  use mica_sync::{MemoryMirrorStorage, SyncConfig, SyncEffect, SyncEngine, SyncEvent};

  #[test]
  fn legacy_flutter_key_matches_main_web_idb_doc_store() {
    // main: WebIdbDocStore.open -> key = '$origin|$docId', origin = cloudCacheOrigin(...)
    let origin = crate::cloud_cache_origin("https://Mica.Example.com/", "user-a").unwrap();
    assert_eq!(
      legacy_flutter_doc_key(&origin, "doc-1"),
      r#"["https://mica.example.com","user-a"]|doc-1"#
    );
  }

  #[test]
  fn legacy_outbox_merge_keeps_only_unacked_rows_in_order_and_is_idempotent() {
    let mut snap = IdbHydrateSnapshot::default();
    snap.cursor.pushed_clock = 3;
    snap.outbox.push(mica_sync::OutboxEntry {
      clock: 4,
      bytes: vec![9],
    });
    let legacy = vec![
      (7, vec![3]),
      (5, vec![1]),
      (6, vec![2]),
      (2, vec![0]),
      (8, vec![]),
    ];
    // legacy pushedClock = 5: rows 2 and 5 were acked; the empty row is skipped.
    assert_eq!(merge_legacy_outbox(&mut snap, legacy.clone(), 5), 2);
    let got: Vec<(i64, Vec<u8>)> = snap
      .outbox
      .iter()
      .map(|e| (e.clock, e.bytes.clone()))
      .collect();
    assert_eq!(got, vec![(4, vec![9]), (5, vec![2]), (6, vec![3])]);
    // A re-run after an interrupted migration appends nothing.
    assert_eq!(merge_legacy_outbox(&mut snap, legacy, 5), 0);
    assert_eq!(snap.outbox.len(), 3);
  }

  #[test]
  fn migrated_legacy_edit_replays_through_the_sync_engine() {
    // The server-side base the Flutter Web replica started from:
    let base = MicaDoc::from_blocks_with_client_id(
      "root",
      &[
        Block::new("root", "page").with_children(vec!["b1".into()]),
        Block::new("b1", "paragraph").with_text("Server text"),
      ],
      Some(1),
    );
    // An offline edit the Flutter Web client queued but never pushed:
    let mut flutter = MicaDoc::from_update(&base.encode_state()).unwrap();
    let sv = flutter.state_vector();
    flutter.text_insert("b1", 11, " + flutter offline");
    let pending = flutter.encode_diff(&sv).unwrap();

    // New client: server base cached, then the legacy row is migrated in.
    let mut mem = MemoryMirrorStorage::new();
    use mica_sync::SyncStorage;
    mem.save_base(&base.encode_state(), 5).unwrap();
    let mut snap = IdbHydrateSnapshot::default();
    apply_mirror_ops(&mut snap, &mem.take_pending_mirror_txns());
    assert_eq!(merge_legacy_outbox(&mut snap, vec![(1, pending)], 0), 1);

    let mut engine = SyncEngine::new(
      SyncConfig {
        client_id: Some(33),
        self_name: None,
        document_id: Some("doc-1".into()),
        restore_unacked: Vec::new(),
      },
      Some(Box::new(MemoryMirrorStorage::hydrate(snap))),
    );
    let _ = engine.handle_event(SyncEvent::Connect);
    assert!(
      !engine.outbox_empty(),
      "migrated edit must be queued for push"
    );
    assert!(
      engine
        .all_blocks_json()
        .contains("Server text + flutter offline")
    );
  }

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
