//! Live WebSocket document CRDT sync controller (`DocSyncController`) and
//! workspace sidebar views-change watcher (`ViewsWsWatcher`).
//!
//! Bridges the L2 Sans-I/O [`mica_sync::SyncEngine`] with:
//! - Cross-platform [`crate::ws_transport`] (`tungstenite` on Desktop, `web_sys::WebSocket` on Web)
//! - [`crate::editor_bridge::MicaDocumentBridge`] (`load_crdt_doc`, `apply_remote_update`, `on_crdt_diff`)
//! - [`crate::storage::VaultStore`] (`SqliteSyncStorage` on Desktop, `MemoryMirrorStorage` on Web)
//!
//! Async invariants (`docs/dev/async-review-checklist.md`):
//! - Every spawned pump/timer captures `session_gen` (and `epoch` / `timer_gen`)
//!   and validates ownership after every `.await`.
//! - No `RefCell` borrow is ever held across `.await` or UI/bridge callbacks.
//! - Switching pages, switching workspaces, or logging out synchronously flushes
//!   coalesced local CRDT diffs + `SyncEvent::FlushForExit`, closes sockets, and
//!   cancels all timers.

use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(target_arch = "wasm32")]
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
use mica_sync::{IdbMirrorOp, MemoryMirrorStorage};
use mica_sync::{
  PresenceUser, SyncConfig, SyncEffect, SyncEngine, SyncEvent, SyncPhase, TimerKind,
  is_empty_yrs_update, merge_yrs_updates, reconnect_backoff_ms,
};
use nsh_core::prelude::{Owner, Signal, spawn, timer};
use uuid::Uuid;

use crate::cloud_sync;
use crate::editor_bridge::MicaDocumentBridge;
use crate::net::CloudConnector;
use crate::storage::VaultStore;
use crate::web_runtime;
use crate::ws_transport::{self, WsEvent, WsHandle};

const LOCAL_DIFF_COALESCE_MS: u64 = 120;
const LOCAL_DIFF_IMMEDIATE_BATCH: usize = 16;
const VIEWS_REFRESH_DEBOUNCE_MS: u64 = 300;

fn session_client_id(vault: &VaultStore) -> Option<u64> {
  if !web_runtime::is_web() {
    return Some(vault.client_id());
  }
  // Share one stable 53-bit client_id between `SyncEngine` and `MicaDocumentBridge`
  // on Web so both operate on the exact same CRDT actor.
  let bytes = *Uuid::new_v4().as_bytes();
  let raw = u64::from_le_bytes(bytes[..8].try_into().unwrap_or([1; 8]));
  Some((raw & ((1u64 << 53) - 1)).max(1))
}

fn now_ms() -> u64 {
  #[cfg(not(target_arch = "wasm32"))]
  {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
  }
  #[cfg(target_arch = "wasm32")]
  {
    js_sys::Date::now().max(0.0) as u64
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveTarget {
  /// Normalized HTTP(S) server base URL used to build WebSocket URLs.
  server_url: String,
  /// `#111` account-scoped SQLite/mirror cache origin (`["https://...","<account_id>"]`).
  cache_origin: String,
  workspace_id: Uuid,
  document_id: Uuid,
}

#[derive(Default)]
struct DocSyncInner {
  session_gen: u64,
  target: Option<ActiveTarget>,
  client_id: Option<u64>,
  engine: Option<SyncEngine>,
  ws_handle: Option<WsHandle>,
  ws_epoch: u64,
  timer_gens: HashMap<TimerKind, u64>,
  ready: bool,
  pending_diffs: Vec<Vec<u8>>,
  coalesce_gen: u64,
  #[cfg(target_arch = "wasm32")]
  idb_pending: VecDeque<IdbMirrorOp>,
  #[cfg(target_arch = "wasm32")]
  idb_flushing: bool,
}

type SessionExpiredCallback = Rc<dyn Fn(String)>;

/// Manages the active cloud document's real-time `/ws/` CRDT [`SyncEngine`].
#[derive(Clone)]
pub(crate) struct DocSyncController {
  owner: Owner,
  inner: Rc<RefCell<DocSyncInner>>,
  vault: Rc<VaultStore>,
  bridge: Rc<MicaDocumentBridge>,
  docs_store: Rc<RefCell<HashMap<Uuid, String>>>,
  cloud_connector: Rc<RefCell<CloudConnector>>,
  cloud_status: Signal<String>,
  sync_phase: Signal<SyncPhase>,
  presence_users: Signal<Vec<PresenceUser>>,
  sync_badge: Signal<String>,
  doc_loading: Signal<bool>,
  doc_load_error: Signal<Option<String>>,
  allow_local: bool,
  account_email: Signal<String>,
  on_session_expired: Rc<RefCell<Option<SessionExpiredCallback>>>,
}

impl DocSyncController {
  #[allow(clippy::too_many_arguments)]
  pub(crate) fn new(
    vault: Rc<VaultStore>,
    bridge: Rc<MicaDocumentBridge>,
    docs_store: Rc<RefCell<HashMap<Uuid, String>>>,
    cloud_connector: Rc<RefCell<CloudConnector>>,
    cloud_status: Signal<String>,
    sync_phase: Signal<SyncPhase>,
    presence_users: Signal<Vec<PresenceUser>>,
    sync_badge: Signal<String>,
    doc_loading: Signal<bool>,
    doc_load_error: Signal<Option<String>>,
    allow_local: bool,
    account_email: Signal<String>,
  ) -> Self {
    Self {
      owner: Owner::current(),
      inner: Rc::new(RefCell::new(DocSyncInner::default())),
      vault,
      bridge,
      docs_store,
      cloud_connector,
      cloud_status,
      sync_phase,
      presence_users,
      sync_badge,
      doc_loading,
      doc_load_error,
      allow_local,
      account_email,
      on_session_expired: Rc::new(RefCell::new(None)),
    }
  }

  pub(crate) fn set_on_session_expired(&self, cb: SessionExpiredCallback) {
    *self.on_session_expired.borrow_mut() = Some(cb);
  }

  /// Return `true` when real-time `/ws/` CRDT sync owns `(cache_origin, ws_id, doc_id)`.
  ///
  /// Used by `lib.rs` to skip HTTP `PATCH /markdown` (`replace_all`) whenever
  /// CRDT sync is active, preventing concurrent-edit overwrites.
  pub(crate) fn is_active_for(&self, cache_origin: &str, ws_id: Uuid, doc_id: Uuid) -> bool {
    self.inner.borrow().target.as_ref().is_some_and(|t| {
      t.cache_origin == cache_origin && t.workspace_id == ws_id && t.document_id == doc_id
    })
  }

  /// Return `true` once the active document has completed initial CRDT bootstrap
  /// (`SyncEffect::Ready`).
  pub(crate) fn is_ready_for(&self, cache_origin: &str, ws_id: Uuid, doc_id: Uuid) -> bool {
    let inner = self.inner.borrow();
    inner.ready
      && inner.target.as_ref().is_some_and(|t| {
        t.cache_origin == cache_origin && t.workspace_id == ws_id && t.document_id == doc_id
      })
  }

  /// Open (or re-bind) a real-time `/ws/` CRDT session for `(server_url, cache_origin, ws_id, doc_id)`.
  pub(crate) fn open_cloud_doc(
    &self,
    server_url: String,
    cache_origin: String,
    ws_id: Uuid,
    doc_id: Uuid,
    self_name: String,
  ) {
    if self.is_active_for(&cache_origin, ws_id, doc_id) {
      return;
    }
    self.close();

    let client_id = session_client_id(&self.vault);
    let sgen = {
      let mut inner = self.inner.borrow_mut();
      inner.session_gen = inner.session_gen.wrapping_add(1);
      inner.target = Some(ActiveTarget {
        server_url,
        cache_origin: cache_origin.clone(),
        workspace_id: ws_id,
        document_id: doc_id,
      });
      inner.client_id = client_id;
      inner.engine = None;
      inner.ws_handle = None;
      inner.ws_epoch = 0;
      inner.timer_gens.clear();
      inner.ready = false;
      inner.pending_diffs.clear();
      inner.coalesce_gen = inner.coalesce_gen.wrapping_add(1);
      #[cfg(target_arch = "wasm32")]
      {
        inner.idb_pending.clear();
        inner.idb_flushing = false;
      }
      inner.session_gen
    };

    self.sync_phase.set(SyncPhase::Offline);
    self.presence_users.set(Vec::new());
    self.update_badge(SyncPhase::Offline, 0);

    #[cfg(not(target_arch = "wasm32"))]
    {
      let storage = self.vault.create_sync_storage(&cache_origin, &doc_id.to_string());
      self.start_engine_with_storage(sgen, client_id, doc_id, self_name, storage);
    }

    #[cfg(target_arch = "wasm32")]
    {
      let ctrl = self.clone();
      let doc_key = doc_id.to_string();
      self.owner.run(move || {
        spawn(async move {
          // Carry over edits the Flutter Web client (<= 0.14.x) left unsynced in
          // its own IndexedDB before hydrating, so they ride this doc's outbox.
          match crate::web_idb::migrate_legacy_flutter_doc(&cache_origin, &doc_key).await {
            Ok(0) => {}
            Ok(n) => web_sys::console::info_1(
              &format!("[mica] migrated {n} unsynced Flutter Web edit(s) for {doc_key}").into(),
            ),
            Err(e) => web_sys::console::warn_1(
              &format!("[mica] legacy Flutter Web migration skipped for {doc_key}: {e}").into(),
            ),
          }
          let hydrated = crate::web_idb::load_doc_snapshot(&cache_origin, &doc_key)
            .await
            .ok()
            .flatten();
          if ctrl.inner.borrow().session_gen != sgen {
            return;
          }
          let storage: Box<dyn mica_sync::SyncStorage> = match hydrated {
            Some(snap) => Box::new(MemoryMirrorStorage::hydrate(snap)),
            None => Box::new(MemoryMirrorStorage::new()),
          };
          ctrl.start_engine_with_storage(sgen, client_id, doc_id, self_name, storage);
        });
      });
    }
  }

  fn start_engine_with_storage(
    &self,
    sgen: u64,
    client_id: Option<u64>,
    doc_id: Uuid,
    self_name: String,
    storage: Box<dyn mica_sync::SyncStorage>,
  ) {
    let mut engine = SyncEngine::new(
      SyncConfig {
        client_id,
        self_name: (!self_name.trim().is_empty()).then_some(self_name),
        document_id: Some(doc_id.to_string()),
        restore_unacked: Vec::new(),
      },
      Some(storage),
    );
    let effects = engine.handle_event(SyncEvent::Connect);
    {
      let mut inner = self.inner.borrow_mut();
      if inner.session_gen != sgen {
        return;
      }
      inner.engine = Some(engine);
    }
    self.apply_effects(sgen, effects);
  }

  /// Feed an incremental local yrs CRDT diff produced by [`MicaDocumentBridge`]
  /// into the active [`SyncEngine`], coalescing rapid keystrokes cleanly.
  pub(crate) fn push_local_diff(&self, diff: Vec<u8>) {
    if is_empty_yrs_update(&diff) || !self.bridge.is_crdt_synced() {
      return;
    }
    let (sgen, should_flush_now, coalesce_gen, causal_gap) = {
      let mut inner = self.inner.borrow_mut();
      if !inner.ready || inner.target.is_none() {
        return;
      }
      let mut gap = false;
      if let Some(doc) = inner.engine.as_mut().and_then(SyncEngine::doc_mut) {
        let had_pending = doc.has_pending_updates();
        if doc.apply_update(&diff).is_err() || (!had_pending && doc.has_pending_updates()) {
          gap = true;
        }
      }
      if gap {
        (inner.session_gen, false, inner.coalesce_gen, true)
      } else {
        inner.pending_diffs.push(diff);
        let immediate = inner.pending_diffs.len() >= LOCAL_DIFF_IMMEDIATE_BATCH;
        inner.coalesce_gen = inner.coalesce_gen.wrapping_add(1);
        (inner.session_gen, immediate, inner.coalesce_gen, false)
      }
    };
    if causal_gap {
      self
        .cloud_status
        .set("检测到本地 CRDT 因果断层，已阻止发送异常更新".to_string());
      return;
    }

    if should_flush_now {
      self.flush_coalesced_diffs(sgen);
      return;
    }

    let ctrl = self.clone();
    self.owner.run(move || {
      spawn(async move {
        timer(Duration::from_millis(LOCAL_DIFF_COALESCE_MS)).await;
        let should_run = {
          let inner = ctrl.inner.borrow();
          inner.session_gen == sgen && inner.coalesce_gen == coalesce_gen
        };
        if should_run {
          ctrl.flush_coalesced_diffs(sgen);
        }
      });
    });
  }

  /// Broadcast the local caret position to remote peers in the same document room.
  #[allow(dead_code)]
  pub(crate) fn send_presence_cursor(&self, block_id: Option<String>, offset: Option<i64>) {
    let (sgen, effects) = {
      let mut inner = self.inner.borrow_mut();
      if !inner.ready {
        return;
      }
      let sgen = inner.session_gen;
      let Some(engine) = inner.engine.as_mut() else {
        return;
      };
      (
        sgen,
        engine.handle_event(SyncEvent::SendPresenceCursor { block_id, offset }),
      )
    };
    self.apply_effects(sgen, effects);
  }

  /// Synchronously flush pending diffs, run `SyncEvent::FlushForExit` +
  /// compaction, close the active socket, and cancel all timers.
  pub(crate) fn close(&self) {
    let current_gen = self.inner.borrow().session_gen;
    self.flush_coalesced_diffs(current_gen);

    #[cfg(target_arch = "wasm32")]
    let mut final_idb: Option<(String, String, Vec<IdbMirrorOp>)> = None;

    let old_ws = {
      let mut inner = self.inner.borrow_mut();
      #[cfg(target_arch = "wasm32")]
      let mut exit_ops: Vec<IdbMirrorOp> = inner.idb_pending.drain(..).collect();
      if let Some(mut engine) = inner.engine.take() {
        let flush_effects = engine.handle_event(SyncEvent::FlushForExit);
        let compact_effects = engine.compact_now();
        #[cfg(target_arch = "wasm32")]
        for ef in flush_effects.into_iter().chain(compact_effects) {
          if let SyncEffect::MirrorIdb(ops) = ef {
            exit_ops.extend(ops);
          }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
          let _ = (flush_effects, compact_effects);
        }
      }
      #[cfg(target_arch = "wasm32")]
      if !exit_ops.is_empty()
        && let Some(ref t) = inner.target
      {
        final_idb = Some((t.cache_origin.clone(), t.document_id.to_string(), exit_ops));
      }
      inner.session_gen = inner.session_gen.wrapping_add(1);
      inner.coalesce_gen = inner.coalesce_gen.wrapping_add(1);
      inner.target = None;
      inner.client_id = None;
      inner.ready = false;
      inner.pending_diffs.clear();
      inner.timer_gens.clear();
      inner.ws_epoch = 0;
      #[cfg(target_arch = "wasm32")]
      {
        inner.idb_flushing = false;
      }
      inner.ws_handle.take()
    };

    #[cfg(target_arch = "wasm32")]
    if let Some((cache_origin, doc_key, ops)) = final_idb {
      self.owner.run(move || {
        spawn(async move {
          let _ = crate::web_idb::commit_mirror_ops(&cache_origin, &doc_key, ops).await;
        });
      });
    }

    if let Some(ws) = old_ws {
      ws.close();
    }
    self.presence_users.set(Vec::new());
    self.sync_badge.set(String::new());
  }

  fn flush_coalesced_diffs(&self, expected_gen: u64) {
    let effects = {
      let mut inner = self.inner.borrow_mut();
      if inner.session_gen != expected_gen || inner.pending_diffs.is_empty() {
        return;
      }
      let batch = std::mem::take(&mut inner.pending_diffs);
      let merged = if batch.len() == 1 {
        batch.into_iter().next().unwrap_or_default()
      } else {
        let slices: Vec<&[u8]> = batch.iter().map(Vec::as_slice).collect();
        match merge_yrs_updates(&slices) {
          Ok(m) => m,
          Err(_) => {
            // Fall back to feeding diffs individually so poison isolation still works.
            let mut all_effects = Vec::new();
            if let Some(engine) = inner.engine.as_mut() {
              for item in batch {
                all_effects.extend(engine.handle_event(SyncEvent::ApplyLocalDiff(item)));
              }
            }
            drop(inner);
            self.apply_effects(expected_gen, all_effects);
            return;
          }
        }
      };
      if is_empty_yrs_update(&merged) {
        return;
      }
      let Some(engine) = inner.engine.as_mut() else {
        return;
      };
      engine.handle_event(SyncEvent::ApplyLocalDiff(merged))
    };
    self.apply_effects(expected_gen, effects);
  }

  fn dispatch_engine_event(&self, expected_gen: u64, event: SyncEvent) {
    let effects = {
      let mut inner = self.inner.borrow_mut();
      if inner.session_gen != expected_gen {
        return;
      }
      let Some(engine) = inner.engine.as_mut() else {
        return;
      };
      engine.handle_event(event)
    };
    self.apply_effects(expected_gen, effects);
  }

  fn apply_effects(&self, sgen: u64, effects: Vec<SyncEffect>) {
    for effect in effects {
      if self.inner.borrow().session_gen != sgen {
        return;
      }
      match effect {
        SyncEffect::OpenSocket { epoch } => self.open_socket(sgen, epoch),
        SyncEffect::CloseSocket { epoch } => {
          let to_close = {
            let mut inner = self.inner.borrow_mut();
            if inner.session_gen == sgen && inner.ws_epoch == epoch {
              inner.ws_handle.take()
            } else {
              None
            }
          };
          if let Some(ws) = to_close {
            ws.close();
          }
        }
        SyncEffect::SendWsJson { epoch, payload } => {
          let handle = {
            let inner = self.inner.borrow();
            if inner.session_gen == sgen && inner.ws_epoch == epoch {
              inner.ws_handle.clone()
            } else {
              None
            }
          };
          if let Some(ws) = handle {
            let _ = ws.send_text(payload);
          }
        }
        SyncEffect::ScheduleTimer { kind, delay_ms, epoch } => {
          let tgen = {
            let mut inner = self.inner.borrow_mut();
            if inner.session_gen != sgen {
              return;
            }
            let entry = inner.timer_gens.entry(kind).or_insert(0);
            *entry = entry.wrapping_add(1);
            *entry
          };
          let ctrl = self.clone();
          self.owner.run(move || {
            spawn(async move {
              timer(Duration::from_millis(delay_ms)).await;
              let still_valid = {
                let inner = ctrl.inner.borrow();
                inner.session_gen == sgen && inner.timer_gens.get(&kind).copied() == Some(tgen)
              };
              if still_valid {
                ctrl.dispatch_engine_event(
                  sgen,
                  SyncEvent::Tick {
                    now_ms: now_ms(),
                    timer_kind: kind,
                    epoch,
                  },
                );
              }
            });
          });
        }
        SyncEffect::CancelTimer { kind } => {
          let mut inner = self.inner.borrow_mut();
          if inner.session_gen == sgen {
            let entry = inner.timer_gens.entry(kind).or_insert(0);
            *entry = entry.wrapping_add(1);
          }
        }
        SyncEffect::Ready { .. } => {
          let loaded = {
            let mut inner = self.inner.borrow_mut();
            if inner.session_gen != sgen {
              return;
            }
            let state = inner
              .engine
              .as_ref()
              .and_then(SyncEngine::doc)
              .map(|d| d.encode_state());
            let cid = inner.client_id;
            let doc_id = inner.target.as_ref().map(|t| t.document_id);
            inner.ready = state.is_some();
            state.map(|s| (s, cid, doc_id))
          };
          let pre_sync_md = if self.bridge.take_pre_sync_dirty() {
            let md = self.bridge.export_markdown();
            (!md.trim().is_empty()).then_some(md)
          } else {
            None
          };
          if let Some((state, cid, doc_id)) = loaded
            && self.bridge.load_crdt_doc(&state, cid).is_ok()
            && let Some(did) = doc_id
          {
            self.doc_loading.set(false);
            self.doc_load_error.set(None);
            if let Some(pending_md) = pre_sync_md
              && self.bridge.is_effectively_empty()
            {
              self.bridge.append_markdown_preserving_crdt(&pending_md);
            }
            self.docs_store.borrow_mut().insert(did, self.bridge.export_markdown());
            self.update_badge(
              self.sync_phase.get_untracked(),
              self.presence_users.get_untracked().len(),
            );
          }
        }
        SyncEffect::RemoteBlocksChanged { .. } => {
          let bridge_sv = self.bridge.state_vector();
          let remote_diff = {
            let inner = self.inner.borrow();
            if inner.session_gen != sgen || !inner.ready {
              return;
            }
            let doc_id = inner.target.as_ref().map(|t| t.document_id);
            let diff = inner
              .engine
              .as_ref()
              .and_then(SyncEngine::doc)
              .and_then(|d| d.encode_diff(&bridge_sv).ok())
              .filter(|d| !is_empty_yrs_update(d));
            diff.map(|d| (d, doc_id))
          };
          if let Some((diff, doc_id)) = remote_diff
            && self.bridge.apply_remote_update(&diff).is_ok()
            && let Some(did) = doc_id
          {
            self.docs_store.borrow_mut().insert(did, self.bridge.export_markdown());
          }
        }
        SyncEffect::SyncPhaseChanged(phase) => {
          self.sync_phase.set(phase);
          let peer_count = self.presence_users.get_untracked().len();
          self.update_badge(phase, peer_count);
        }
        SyncEffect::PresenceChanged(users) => {
          let peer_count = users.len();
          self.presence_users.set(users);
          self.update_badge(self.sync_phase.get_untracked(), peer_count);
        }
        SyncEffect::Fault { reason, count } => {
          self
            .cloud_status
            .set(format!("实时同步告警（{reason} #{count}），正在自动恢复…"));
        }
        SyncEffect::MirrorIdb(ops) => {
          #[cfg(target_arch = "wasm32")]
          self.enqueue_idb_mirror_ops(sgen, ops);
          #[cfg(not(target_arch = "wasm32"))]
          {
            let _ = ops;
          }
        }
        SyncEffect::ServerConnected | SyncEffect::AcceptedSeq(_) => {}
      }
    }
  }

  #[cfg(target_arch = "wasm32")]
  fn enqueue_idb_mirror_ops(&self, sgen: u64, ops: Vec<IdbMirrorOp>) {
    if ops.is_empty() {
      return;
    }
    let should_start = {
      let mut inner = self.inner.borrow_mut();
      if inner.session_gen != sgen {
        return;
      }
      inner.idb_pending.extend(ops);
      if inner.idb_flushing {
        false
      } else {
        inner.idb_flushing = true;
        true
      }
    };
    if !should_start {
      return;
    }

    let ctrl = self.clone();
    self.owner.run(move || {
      spawn(async move {
        loop {
          let next_batch = {
            let mut inner = ctrl.inner.borrow_mut();
            if inner.session_gen != sgen {
              inner.idb_flushing = false;
              return;
            }
            if inner.idb_pending.is_empty() {
              inner.idb_flushing = false;
              return;
            }
            let Some(target) = inner.target.as_ref() else {
              inner.idb_pending.clear();
              inner.idb_flushing = false;
              return;
            };
            let cache_origin = target.cache_origin.clone();
            let doc_key = target.document_id.to_string();
            let batch: Vec<IdbMirrorOp> = inner.idb_pending.drain(..).collect();
            (cache_origin, doc_key, batch)
          };

          let (cache_origin, doc_key, batch) = next_batch;
          if crate::web_idb::commit_mirror_ops(&cache_origin, &doc_key, batch)
            .await
            .is_err()
          {
            ctrl.dispatch_engine_event(sgen, SyncEvent::MirrorWriteFailed);
          }
        }
      });
    });
  }

  fn open_socket(&self, sgen: u64, epoch: u64) {
    let target = {
      let inner = self.inner.borrow();
      if inner.session_gen != sgen {
        return;
      }
      inner.target.clone()
    };
    let Some(target) = target else {
      return;
    };

    let url = ws_transport::document_ws_url(&target.server_url, target.workspace_id, target.document_id);
    let bearer = {
      let conn = self.cloud_connector.borrow();
      conn.access_token().map(str::to_string)
    };
    let (handle, mut rx) = ws_transport::connect(&url, bearer.as_deref());

    let prev = {
      let mut inner = self.inner.borrow_mut();
      if inner.session_gen != sgen {
        handle.close();
        return;
      }
      inner.ws_epoch = epoch;
      inner.ws_handle.replace(handle)
    };
    if let Some(old) = prev {
      old.close();
    }

    let ctrl = self.clone();
    self.owner.run(move || {
      spawn(async move {
        let mut closed_reported = false;
        while let Some(ev) = rx.recv().await {
          let still_owner = {
            let inner = ctrl.inner.borrow();
            inner.session_gen == sgen && inner.ws_epoch == epoch
          };
          if !still_owner {
            return;
          }
          match ev {
            WsEvent::Opened => {
              ctrl.dispatch_engine_event(sgen, SyncEvent::SocketOpened { epoch });
            }
            WsEvent::Text(json) => {
              ctrl.dispatch_engine_event(sgen, SyncEvent::SocketFrame { epoch, json });
            }
            WsEvent::Closed(_code, reason) => {
              closed_reported = true;
              let is_unauth = CloudConnector::is_unauthorized_error(&reason);
              ctrl.dispatch_engine_event(sgen, SyncEvent::SocketClosed { epoch, reason });
              if is_unauth {
                ctrl.refresh_after_ws_unauthorized(sgen, target.server_url.clone());
              }
              break;
            }
            WsEvent::Error(reason) => {
              closed_reported = true;
              let is_unauth = CloudConnector::is_unauthorized_error(&reason);
              ctrl.dispatch_engine_event(sgen, SyncEvent::SocketClosed { epoch, reason });
              if is_unauth {
                ctrl.refresh_after_ws_unauthorized(sgen, target.server_url.clone());
              }
              break;
            }
          }
        }
        if !closed_reported {
          let still_owner = {
            let inner = ctrl.inner.borrow();
            inner.session_gen == sgen && inner.ws_epoch == epoch
          };
          if still_owner {
            ctrl.dispatch_engine_event(
              sgen,
              SyncEvent::SocketClosed {
                epoch,
                reason: "socket_eof".to_string(),
              },
            );
          }
        }
      });
    });
  }

  fn refresh_after_ws_unauthorized(&self, sgen: u64, server_url: String) {
    let ctrl = self.clone();
    let email = self.account_email.get_untracked();
    let allow_local = self.allow_local;
    self.owner.run(move || {
      spawn(async move {
        let res = cloud_sync::refresh_shared_connector(
          &ctrl.cloud_connector,
          &ctrl.vault,
          allow_local,
          &server_url,
          &email,
        )
        .await;
        if ctrl.inner.borrow().session_gen != sgen {
          return;
        }
        if let Err(err) = res
          && CloudConnector::is_unauthorized_error(&err)
          && let Some(cb) = ctrl.on_session_expired.borrow().clone()
        {
          cb(server_url);
        }
      });
    });
  }

  fn update_badge(&self, phase: SyncPhase, peer_count: usize) {
    let base = match phase {
      SyncPhase::Synced => "已实时同步",
      SyncPhase::Syncing => "正在同步…",
      SyncPhase::Offline => "离线缓存",
    };
    let label = if peer_count > 0 {
      format!("{base} · {} 人在线", peer_count + 1)
    } else {
      base.to_string()
    };
    if self.sync_badge.get_untracked() != label {
      self.sync_badge.set(label.clone());
    }
    self.bridge.set_status_flash(label);
  }
}

// ── Sidebar live views-change watcher (/ws/workspaces/{ws}/views) ────────────

#[derive(Default)]
struct ViewsWatcherInner {
  watcher_gen: u64,
  debounce_gen: u64,
  active: Option<(String, String, Uuid)>,
  ws_handle: Option<WsHandle>,
  reconnect_attempt: u32,
}

type ViewsChangedCallback = Rc<dyn Fn(String, String, Uuid)>;

/// Watches `GET /ws/workspaces/{workspace_id}/views` for `{"type":"views_changed"}`
/// notifications from other devices, CLI, or MCP and triggers a debounced tree refresh.
#[derive(Clone)]
pub(crate) struct ViewsWsWatcher {
  owner: Owner,
  inner: Rc<RefCell<ViewsWatcherInner>>,
  vault: Rc<VaultStore>,
  cloud_connector: Rc<RefCell<CloudConnector>>,
  allow_local: bool,
  account_email: Signal<String>,
  on_views_changed: Rc<RefCell<Option<ViewsChangedCallback>>>,
  on_session_expired: Rc<RefCell<Option<SessionExpiredCallback>>>,
}

impl ViewsWsWatcher {
  pub(crate) fn new(
    vault: Rc<VaultStore>,
    cloud_connector: Rc<RefCell<CloudConnector>>,
    allow_local: bool,
    account_email: Signal<String>,
  ) -> Self {
    Self {
      owner: Owner::current(),
      inner: Rc::new(RefCell::new(ViewsWatcherInner::default())),
      vault,
      cloud_connector,
      allow_local,
      account_email,
      on_views_changed: Rc::new(RefCell::new(None)),
      on_session_expired: Rc::new(RefCell::new(None)),
    }
  }

  pub(crate) fn set_on_views_changed(&self, cb: Rc<dyn Fn(String, String, Uuid)>) {
    *self.on_views_changed.borrow_mut() = Some(cb);
  }

  pub(crate) fn set_on_session_expired(&self, cb: SessionExpiredCallback) {
    *self.on_session_expired.borrow_mut() = Some(cb);
  }

  /// Start watching `(server_url, cache_origin, ws_id)`. No-op if already watching.
  pub(crate) fn watch(&self, server_url: String, cache_origin: String, ws_id: Uuid) {
    if ws_id.is_nil() {
      self.stop();
      return;
    }
    if self
      .inner
      .borrow()
      .active
      .as_ref()
      .is_some_and(|(s, c, id)| s == &server_url && c == &cache_origin && *id == ws_id)
    {
      return;
    }
    self.stop();
    let sgen = {
      let mut inner = self.inner.borrow_mut();
      inner.watcher_gen = inner.watcher_gen.wrapping_add(1);
      inner.active = Some((server_url.clone(), cache_origin.clone(), ws_id));
      inner.reconnect_attempt = 0;
      inner.watcher_gen
    };
    self.connect_once(sgen, server_url, cache_origin, ws_id);
  }

  /// Stop watching and close any active views WebSocket.
  pub(crate) fn stop(&self) {
    let old_ws = {
      let mut inner = self.inner.borrow_mut();
      inner.watcher_gen = inner.watcher_gen.wrapping_add(1);
      inner.debounce_gen = inner.debounce_gen.wrapping_add(1);
      inner.active = None;
      inner.reconnect_attempt = 0;
      inner.ws_handle.take()
    };
    if let Some(ws) = old_ws {
      ws.close();
    }
  }

  fn connect_once(&self, sgen: u64, server_url: String, cache_origin: String, ws_id: Uuid) {
    let url = ws_transport::views_ws_url(&server_url, ws_id);
    let bearer = {
      let conn = self.cloud_connector.borrow();
      conn.access_token().map(str::to_string)
    };
    let (handle, mut rx) = ws_transport::connect(&url, bearer.as_deref());

    let prev = {
      let mut inner = self.inner.borrow_mut();
      if inner.watcher_gen != sgen {
        handle.close();
        return;
      }
      inner.ws_handle.replace(handle)
    };
    if let Some(old) = prev {
      old.close();
    }

    let watcher = self.clone();
    self.owner.run(move || {
      spawn(async move {
        let mut saw_unauthorized = false;
        while let Some(ev) = rx.recv().await {
          if watcher.inner.borrow().watcher_gen != sgen {
            return;
          }
          match ev {
            WsEvent::Opened => {
              watcher.inner.borrow_mut().reconnect_attempt = 0;
            }
            WsEvent::Text(raw) => {
              if is_views_changed_frame(&raw) {
                watcher.schedule_refresh(sgen, server_url.clone(), cache_origin.clone(), ws_id);
              }
            }
            WsEvent::Closed(_, reason) | WsEvent::Error(reason) => {
              saw_unauthorized = CloudConnector::is_unauthorized_error(&reason);
              break;
            }
          }
        }

        if watcher.inner.borrow().watcher_gen != sgen {
          return;
        }
        if saw_unauthorized {
          let email = watcher.account_email.get_untracked();
          let res = cloud_sync::refresh_shared_connector(
            &watcher.cloud_connector,
            &watcher.vault,
            watcher.allow_local,
            &server_url,
            &email,
          )
          .await;
          if watcher.inner.borrow().watcher_gen != sgen {
            return;
          }
          if let Err(err) = res
            && CloudConnector::is_unauthorized_error(&err)
          {
            if let Some(cb) = watcher.on_session_expired.borrow().clone() {
              cb(server_url);
            }
            return;
          }
        }
        let delay_ms = {
          let mut inner = watcher.inner.borrow_mut();
          let d = reconnect_backoff_ms(inner.reconnect_attempt);
          inner.reconnect_attempt = inner.reconnect_attempt.saturating_add(1);
          d
        };
        timer(Duration::from_millis(delay_ms)).await;
        if watcher.inner.borrow().watcher_gen == sgen {
          watcher.connect_once(sgen, server_url, cache_origin, ws_id);
        }
      });
    });
  }

  fn schedule_refresh(&self, sgen: u64, server_url: String, cache_origin: String, ws_id: Uuid) {
    let dgen = {
      let mut inner = self.inner.borrow_mut();
      if inner.watcher_gen != sgen {
        return;
      }
      inner.debounce_gen = inner.debounce_gen.wrapping_add(1);
      inner.debounce_gen
    };
    let watcher = self.clone();
    self.owner.run(move || {
      spawn(async move {
        timer(Duration::from_millis(VIEWS_REFRESH_DEBOUNCE_MS)).await;
        let still_valid = {
          let inner = watcher.inner.borrow();
          inner.watcher_gen == sgen && inner.debounce_gen == dgen
        };
        if !still_valid {
          return;
        }
        let cb = watcher.on_views_changed.borrow().clone();
        if let Some(cb) = cb {
          cb(server_url, cache_origin, ws_id);
        }
      });
    });
  }
}

fn is_views_changed_frame(raw: &str) -> bool {
  serde_json::from_str::<serde_json::Value>(raw)
    .ok()
    .and_then(|v| v.get("type").and_then(serde_json::Value::as_str).map(str::to_string))
    .is_some_and(|t| t == "views_changed")
}

#[cfg(test)]
mod tests {
  use super::*;
  use mica_core::{Block, MicaDoc};
  use mica_editor::{DocPosition, DocSelection, EditorCommand};

  fn with_runtime(f: impl FnOnce()) {
    let rt = nsh_core::element::Runtime::default();
    let _g = rt.enter();
    f();
  }

  #[test]
  fn views_changed_frame_parser_matches_only_views_changed() {
    assert!(is_views_changed_frame(r#"{"type":"views_changed"}"#));
    assert!(!is_views_changed_frame(r#"{"type":"pong"}"#));
    assert!(!is_views_changed_frame("not-json"));
  }

  #[test]
  fn two_editor_bridges_converge_concurrent_crdt_edits_via_diff() {
    with_runtime(|| {
      let base = MicaDoc::from_blocks_with_client_id(
        "root",
        &[
          Block::new("root", "page").with_children(vec!["b1".into(), "b2".into()]),
          Block::new("b1", "paragraph").with_text("Hello "),
          Block::new("b2", "paragraph").with_text("World "),
        ],
        Some(1),
      );
      let base_state = base.encode_state();

      // Peer A (client_id 10) and Peer B (client_id 20) both load server base.
      let bridge_a = MicaDocumentBridge::from_markdown("");
      let bridge_b = MicaDocumentBridge::from_markdown("");
      bridge_a.load_crdt_doc(&base_state, Some(10)).unwrap();
      bridge_b.load_crdt_doc(&base_state, Some(20)).unwrap();

      let diffs_a: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
      let diffs_b: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
      {
        let da = diffs_a.clone();
        bridge_a.set_on_crdt_diff(Rc::new(move |d| da.borrow_mut().push(d)));
        let db = diffs_b.clone();
        bridge_b.set_on_crdt_diff(Rc::new(move |d| db.borrow_mut().push(d)));
      }

      // Peer A appends to block 0 ("Hello ") -> "Hello Alice"
      let pos_a = DocPosition::new(0, 6);
      bridge_a.dispatch(EditorCommand::SetSelection {
        selection: Some(DocSelection::from_positions(pos_a, pos_a)),
      });
      bridge_a.dispatch(EditorCommand::InsertText { text: "Alice".into() });

      // Peer B concurrently appends to block 1 ("World ") -> "World Bob"
      let pos_b = DocPosition::new(1, 6);
      bridge_b.dispatch(EditorCommand::SetSelection {
        selection: Some(DocSelection::from_positions(pos_b, pos_b)),
      });
      bridge_b.dispatch(EditorCommand::InsertText { text: "Bob".into() });

      assert_eq!(diffs_a.borrow().len(), 1);
      assert_eq!(diffs_b.borrow().len(), 1);

      // Cross-apply remote updates (must NOT echo back to on_crdt_diff!).
      let diff_from_a = diffs_a.borrow()[0].clone();
      let diff_from_b = diffs_b.borrow()[0].clone();
      bridge_b.apply_remote_update(&diff_from_a).unwrap();
      bridge_a.apply_remote_update(&diff_from_b).unwrap();

      assert_eq!(diffs_a.borrow().len(), 1, "remote apply must not echo local CRDT diff");
      assert_eq!(diffs_b.borrow().len(), 1, "remote apply must not echo local CRDT diff");

      let md_a = bridge_a.export_markdown();
      let md_b = bridge_b.export_markdown();
      assert_eq!(md_a, md_b);
      assert!(md_a.contains("Hello Alice"));
      assert!(md_a.contains("World Bob"));
    });
  }

  /// Regression for causal-gap data loss: a synthetic `from_markdown` / `load_markdown`
  /// session MUST NOT emit `on_crdt_diff` (its private `0..N` setup clocks are unknown to
  /// the server, so shipping clock `N+1..` parks all edits forever in `pending_updates`).
  /// Replaying pre-sync edits after `load_crdt_doc` must produce causally contiguous diffs
  /// that integrate on the server with `has_pending_updates() == false`.
  #[test]
  fn synthetic_markdown_session_suppresses_crdt_diff_until_load_crdt_doc_binds_lineage() {
    with_runtime(|| {
      let bridge = MicaDocumentBridge::from_markdown("");
      let diffs: Rc<RefCell<Vec<Vec<u8>>>> = Rc::new(RefCell::new(Vec::new()));
      {
        let d = diffs.clone();
        bridge.set_on_crdt_diff(Rc::new(move |bytes| d.borrow_mut().push(bytes)));
      }

      // 1. Editing before `load_crdt_doc` (synthetic Markdown session) must NOT emit CRDT diffs,
      //    and must flag `pre_sync_dirty` so edits are not lost when `Ready` arrives.
      assert!(!bridge.is_crdt_synced());
      bridge.dispatch(EditorCommand::InsertText {
        text: "Typed before sync ready".into(),
      });
      assert!(
        diffs.borrow().is_empty(),
        "synthetic from_markdown session must never emit orphan CRDT diffs"
      );
      assert!(bridge.take_pre_sync_dirty());

      // 2. Hydrating an authoritative empty server doc binds CRDT lineage (`is_crdt_synced == true`).
      let mut server_doc = MicaDoc::from_blocks_with_client_id(
        "block_root",
        &[Block::new("block_root", "page")],
        Some(1),
      );
      bridge
        .load_crdt_doc(&server_doc.encode_state(), Some(42))
        .unwrap();
      assert!(bridge.is_crdt_synced());
      assert!(bridge.is_effectively_empty());

      // 3. Replaying content via `append_markdown_preserving_crdt` emits causally contiguous diffs
      //    that integrate cleanly into `server_doc` with ZERO pending updates!
      bridge.append_markdown_preserving_crdt("## PCIe-Switch\n\nRecovered notes");
      assert!(!diffs.borrow().is_empty());
      for update in diffs.borrow().iter() {
        server_doc.apply_update(update).unwrap();
      }
      assert!(
        !server_doc.has_pending_updates(),
        "every CRDT diff emitted after load_crdt_doc must integrate without causal gaps"
      );
      let server_texts: String = server_doc
        .to_blocks()
        .into_iter()
        .map(|b| b.text)
        .collect::<Vec<_>>()
        .join("\n");
      assert!(server_texts.contains("PCIe-Switch"));
      assert!(server_texts.contains("Recovered notes"));

      // 4. Calling `load_markdown` later drops CRDT sync flag again so it can never emit orphan diffs.
      bridge.load_markdown("detached local preview");
      assert!(!bridge.is_crdt_synced());
      let before_len = diffs.borrow().len();
      bridge.dispatch(EditorCommand::InsertText { text: "!".into() });
      assert_eq!(
        diffs.borrow().len(),
        before_len,
        "load_markdown must revoke CRDT diff emission"
      );
    });
  }
}
