//! Pure Rust Sans-I/O synchronization state machine ([`SyncEngine`]).
//!
//! Zero UI, async runtime, or network I/O dependencies: callers feed
//! [`SyncEvent`]s in and execute the returned [`Vec<SyncEffect>`]s. One shared
//! implementation powers desktop FFI, Web WASM, CLI, and deterministic tests.
//!
//! Preserves every red-line invariant from `cloud_sync_session.dart`:
//! - **Epoch isolation**: stale `SocketOpened` / `SocketClosed` / `SocketFrame`
//!   / socket-scoped `Tick` events from prior epochs are silently ignored.
//! - **Never diverge silently (red line #1)**: bad bases or bad remote updates
//!   trigger `Fault` + full `sync.bootstrap` self-heal up to [`MAX_AUTO_REHEAL`]
//!   (3) times before circuit-breaking.
//! - **SV catch-up on reconnect**: always sends `sync.bootstrap` with the local
//!   state vector (never `sync.pull` by cursor alone).
//! - **B2 pull pagination**: keeps pulling while batches make cursor progress.
//! - **64-window pacing & ≥8 backlog merging**: folds long offline stretches up
//!   to 4 MiB into one push, tracks run ranges in `run_from`, and de-merges on
//!   rejection (`de_merge_from`) to isolate poison edits while letting good
//!   edits through.
//! - **Issue #159 contiguous sent floor**: live enqueues advance
//!   `sent_through_clock` only when `clock == sent_through_clock + 1`.
//! - **B11 history repair**: sends diff against server `sv` (capped at
//!   [`MAX_REPAIRS`] = 3 per connection).
//! - **Timerless compaction**: checks log sizes every 32 appends and compacts
//!   past 256 combined rows.
//! - **Single-slot `pending_local_diff`**: retains failed local appends and
//!   retries with exponential backoff (400ms * 2^attempt, up to 5 attempts).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use mica_core::MicaDoc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::protocol::{
  derive_sync_phase, frame_ping, frame_presence_update, frame_sync_bootstrap, frame_sync_pull,
  frame_sync_push, is_empty_yrs_update, local_update_retry_delay_ms, merge_run_length,
  merge_yrs_updates, push_slice, reconnect_backoff_ms, repair_sv_for, PresenceUser, SyncPhase,
  TimerKind, COMPACT_CHECK_EVERY, COMPACT_THRESHOLD, LAG_RESYNC_DEBOUNCE_MS,
  LOCAL_UPDATE_MAX_RETRIES, MAX_AUTO_REHEAL, MAX_PUSH_REJECTS, MAX_REPAIRS, MERGE_MAX_BYTES,
  MERGE_MIN_RUN, PING_EVERY_MS, PUSH_WINDOW, STALE_MS,
};
use crate::storage::{IdbMirrorOp, StorageError, SyncStorage};

/// Configuration for constructing a [`SyncEngine`].
#[derive(Debug, Clone, Default)]
pub struct SyncConfig {
  /// Stable device CRDT actor id (desktop pins from LocalStore identity; web
  /// leaves `None` for a random per-replica actor).
  pub client_id: Option<u64>,
  /// Optional display name published in room presence; `None` opts out.
  pub self_name: Option<String>,
  /// Optional document identifier metadata.
  pub document_id: Option<String>,
  /// Legacy unacked diffs restored at startup when running without a
  /// [`SyncStorage`] (online-only mode).
  pub restore_unacked: Vec<Vec<u8>>,
}

/// Events fed into [`SyncEngine::handle_event`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SyncEvent {
  /// Initiate (or restart) connection and seed from local storage once.
  Connect,
  /// The host opened the WebSocket for `epoch`.
  SocketOpened { epoch: u64 },
  /// The WebSocket for `epoch` closed or failed to connect.
  SocketClosed { epoch: u64, reason: String },
  /// A raw JSON text frame arrived on `epoch`.
  SocketFrame { epoch: u64, json: String },
  /// A scheduled timer fired at host monotonic/wall clock `now_ms`.
  Tick {
    now_ms: u64,
    timer_kind: TimerKind,
    epoch: u64,
  },
  /// Persist and push a local CRDT diff produced by editing `doc_mut()`.
  ApplyLocalDiff(Vec<u8>),
  /// Broadcast (or clear) the local cursor position in room presence.
  SendPresenceCursor {
    block_id: Option<String>,
    offset: Option<i64>,
  },
  /// Synchronous pre-exit durability flush (skips compaction & socket send).
  FlushForExit,
  /// The Web host's asynchronous IndexedDB mirror transaction failed.
  MirrorWriteFailed,
}

/// Side-effect instructions returned by [`SyncEngine::handle_event`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SyncEffect {
  /// Open a new WebSocket connection bound to `epoch`.
  OpenSocket { epoch: u64 },
  /// Close the WebSocket connection bound to `epoch`.
  CloseSocket { epoch: u64 },
  /// Send a JSON text frame over the WebSocket bound to `epoch`.
  SendWsJson { epoch: u64, payload: String },
  /// Schedule (or reschedule) a one-shot timer of `kind` for `delay_ms`.
  ScheduleTimer {
    kind: TimerKind,
    delay_ms: u64,
    epoch: u64,
  },
  /// Cancel any pending timer of `kind`.
  CancelTimer { kind: TimerKind },
  /// Execute ordered, snapshotted IndexedDB mirror transactions (Web host).
  MirrorIdb(Vec<IdbMirrorOp>),
  /// Document replica is ready (either seeded from local storage or cold-bootstrapped).
  Ready { root_block_id: String },
  /// Remote updates were merged into `doc()`. `Some(ids)` lists content-only
  /// changed block ids; `None` signals a structural change (read all blocks).
  RemoteBlocksChanged { changed_ids: Option<Vec<String>> },
  /// Derived sync phase transitioned (`synced` / `syncing` / `offline`).
  SyncPhaseChanged(SyncPhase),
  /// First valid frame arrived from the server in this session.
  ServerConnected,
  /// Remote room presence roster changed (own connection filtered out).
  PresenceChanged(Vec<PresenceUser>),
  /// Server accepted-update sequence advanced (`document.update.accepted`).
  AcceptedSeq(i64),
  /// Integrity, persistence, or push fault surfaced for observability / UI banner.
  Fault { reason: String, count: u32 },
}

/// Entry in the online-only in-memory outbox (used when `storage` is `None`).
#[derive(Debug, Clone)]
struct PendingInMemory {
  id: String,
  bytes: Vec<u8>,
  sent: bool,
  rejected: bool,
}

/// Accumulated dirty state for `changed_blocks_json()`.
#[derive(Debug, Default)]
enum ChangedAccumulator {
  #[default]
  Clean,
  Ids(BTreeSet<String>),
  All,
}

impl ChangedAccumulator {
  fn record(&mut self, delta: Option<Vec<String>>) {
    match delta {
      None => *self = ChangedAccumulator::All,
      Some(ids) => match self {
        ChangedAccumulator::All => {}
        ChangedAccumulator::Clean => {
          if !ids.is_empty() {
            *self = ChangedAccumulator::Ids(ids.into_iter().collect());
          }
        }
        ChangedAccumulator::Ids(existing) => {
          existing.extend(ids);
        }
      },
    }
  }

  fn take(&mut self) -> Option<Vec<String>> {
    match std::mem::take(self) {
      ChangedAccumulator::Clean => Some(Vec::new()),
      ChangedAccumulator::Ids(set) => Some(set.into_iter().collect()),
      ChangedAccumulator::All => None,
    }
  }
}

/// Pure Rust Sans-I/O synchronization engine for one cloud document.
pub struct SyncEngine {
  config: SyncConfig,
  doc: Option<MicaDoc>,
  storage: Option<Box<dyn SyncStorage>>,

  root_block_id: String,
  cursor: i64,
  ready: bool,
  seeded: bool,
  restored: bool,
  saw_server_frame: bool,

  // Epoch & connection lifecycle
  epoch: u64,
  socket_open: bool,
  online: bool,
  last_emitted_phase: Option<SyncPhase>,
  reconnect_attempts: u32,
  reconnect_scheduled: bool,

  // Heartbeat tracking (clock-origin agnostic)
  last_known_now_ms: Option<u64>,
  last_frame_ms: Option<u64>,
  frame_since_heartbeat: bool,

  // Lag resync debounce
  lag_debounce_active: bool,
  lag_trailing_pending: bool,
  lag_resync_count: u32,

  // Integrity & persistence fault accounting
  fault_count: u32,
  persist_fails: u32,
  local_persist_faults: u32,

  // Append-log outbox state (per-connection bookkeeping)
  sent_through_clock: i64,
  acked_ahead: HashSet<i64>,
  run_from: HashMap<i64, i64>,
  de_merge_from: HashSet<i64>,
  push_rejects: usize,
  push_stalled: bool,
  repairs: usize,

  // Online-only in-memory outbox (when storage is None)
  unacked: Vec<PendingInMemory>,
  push_seq: u64,

  // Single-slot local update retry buffer (`PendingDocUpdate`)
  pending_local_diff: Option<Vec<u8>>,
  local_retry_attempts: u32,
  local_retry_scheduled: bool,

  // Compaction counter
  appends_since_compact_check: usize,

  // Room presence
  connection_id: Option<String>,
  presence: BTreeMap<String, PresenceUser>,
  presence_cursor: Option<(String, i64)>,

  // Pending block changes for `changed_blocks_json()`
  changed_acc: ChangedAccumulator,
}

impl SyncEngine {
  /// Construct a new [`SyncEngine`] with optional local [`SyncStorage`].
  ///
  /// Passing `storage: None` runs in online-only mode with an in-memory outbox.
  pub fn new(config: SyncConfig, storage: Option<Box<dyn SyncStorage>>) -> Self {
    Self {
      config,
      doc: None,
      storage,
      root_block_id: String::new(),
      cursor: 0,
      ready: false,
      seeded: false,
      restored: false,
      saw_server_frame: false,
      epoch: 0,
      socket_open: false,
      online: false,
      last_emitted_phase: None,
      reconnect_attempts: 0,
      reconnect_scheduled: false,
      last_known_now_ms: None,
      last_frame_ms: None,
      frame_since_heartbeat: false,
      lag_debounce_active: false,
      lag_trailing_pending: false,
      lag_resync_count: 0,
      fault_count: 0,
      persist_fails: 0,
      local_persist_faults: 0,
      sent_through_clock: 0,
      acked_ahead: HashSet::new(),
      run_from: HashMap::new(),
      de_merge_from: HashSet::new(),
      push_rejects: 0,
      push_stalled: false,
      repairs: 0,
      unacked: Vec::new(),
      push_seq: 0,
      pending_local_diff: None,
      local_retry_attempts: 0,
      local_retry_scheduled: false,
      appends_since_compact_check: 0,
      connection_id: None,
      presence: BTreeMap::new(),
      presence_cursor: None,
      changed_acc: ChangedAccumulator::Clean,
    }
  }

  /// Process one external [`SyncEvent`] and return all resulting [`SyncEffect`]s.
  pub fn handle_event(&mut self, event: SyncEvent) -> Vec<SyncEffect> {
    let mut effects = Vec::new();
    match event {
      SyncEvent::Connect => self.handle_connect(&mut effects),
      SyncEvent::SocketOpened { epoch } => self.handle_socket_opened(epoch, &mut effects),
      SyncEvent::SocketClosed { epoch, reason } => {
        self.handle_socket_closed(epoch, &reason, &mut effects);
      }
      SyncEvent::SocketFrame { epoch, json } => {
        self.handle_socket_frame(epoch, &json, &mut effects);
      }
      SyncEvent::Tick {
        now_ms,
        timer_kind,
        epoch,
      } => self.handle_tick(now_ms, timer_kind, epoch, &mut effects),
      SyncEvent::ApplyLocalDiff(diff) => self.handle_apply_local_diff(diff, &mut effects),
      SyncEvent::SendPresenceCursor { block_id, offset } => {
        self.handle_send_presence_cursor(block_id, offset, &mut effects);
      }
      SyncEvent::FlushForExit => self.handle_flush_for_exit(&mut effects),
      SyncEvent::MirrorWriteFailed => self.handle_mirror_write_failed(&mut effects),
    }
    self.drain_mirror_ops(&mut effects);
    effects
  }

  /// Force immediate compaction (e.g. graceful session teardown) and return any
  /// resulting mirror effects.
  pub fn compact_now(&mut self) -> Vec<SyncEffect> {
    let mut effects = Vec::new();
    if self.doc.is_some() {
      if let Some(store) = self.storage.as_mut() {
        let _ = store.compact();
      }
    }
    self.drain_mirror_ops(&mut effects);
    effects
  }

  // ── Read accessors ───────────────────────────────────────────────────────

  /// Borrow the authoritative CRDT [`MicaDoc`] once ready.
  pub fn doc(&self) -> Option<&MicaDoc> {
    self.doc.as_ref()
  }

  /// Mutably borrow the authoritative CRDT [`MicaDoc`] once ready (for applying
  /// local editor ops before feeding `SyncEvent::ApplyLocalDiff`).
  pub fn doc_mut(&mut self) -> Option<&mut MicaDoc> {
    self.doc.as_mut()
  }

  /// Borrow the backing [`SyncStorage`], if configured.
  pub fn storage(&self) -> Option<&dyn SyncStorage> {
    self.storage.as_deref()
  }

  /// Document root block id (empty before [`SyncEffect::Ready`]).
  pub fn root_block_id(&self) -> &str {
    &self.root_block_id
  }

  /// True once the replica has been seeded from local storage or bootstrapped.
  pub fn is_ready(&self) -> bool {
    self.ready
  }

  /// True when the current socket has delivered at least one valid frame and
  /// has not closed or gone stale.
  pub fn is_online(&self) -> bool {
    self.online
  }

  /// Current socket connection generation (`0` before first `Connect`).
  pub fn epoch(&self) -> u64 {
    self.epoch
  }

  /// Highest server stream `rid` applied by this replica.
  pub fn cursor(&self) -> i64 {
    self.cursor
  }

  /// Count of lag-triggered resyncs (useful for diagnostics and tests).
  pub fn lag_resync_count(&self) -> u32 {
    self.lag_resync_count
  }

  /// True when no local diffs are awaiting persistence or server ack.
  pub fn outbox_empty(&self) -> bool {
    if self.pending_local_diff.is_some() {
      return false;
    }
    if let Some(store) = self.storage.as_deref() {
      let pushed = store.cursor().map(|c| c.pushed_clock).unwrap_or(0);
      store
        .outbox_after(pushed)
        .map(|rows| rows.is_empty())
        .unwrap_or(false)
    } else {
      self.unacked.is_empty()
    }
  }

  /// Current derived UI sync phase.
  pub fn phase(&self) -> SyncPhase {
    derive_sync_phase(self.online, !self.outbox_empty())
  }

  /// Snapshot of unacked diffs in online-only mode (for host crash persistence).
  pub fn unacked_diffs(&self) -> Vec<Vec<u8>> {
    self.unacked.iter().map(|p| p.bytes.clone()).collect()
  }

  /// Serialize all blocks in DFS tree order as JSON (`"[]"` before ready).
  pub fn all_blocks_json(&self) -> String {
    let Some(doc) = self.doc.as_ref() else {
      return "[]".to_string();
    };
    serde_json::to_string(&doc.to_blocks()).unwrap_or_else(|_| "[]".to_string())
  }

  /// Serialize the named blocks as JSON (`"[]"` before ready).
  pub fn blocks_by_id_json(&self, ids: &[String]) -> String {
    let Some(doc) = self.doc.as_ref() else {
      return "[]".to_string();
    };
    serde_json::to_string(&doc.blocks_by_id(ids)).unwrap_or_else(|_| "[]".to_string())
  }

  /// Drain changed block ids since the previous read and return their JSON
  /// array, or `None` when a structural change requires [`Self::all_blocks_json`].
  pub fn changed_blocks_json(&mut self) -> Option<String> {
    let doc = self.doc.as_mut()?;
    let live_delta = doc.take_changed_block_ids();
    self.changed_acc.record(live_delta);
    let ids = self.changed_acc.take()?;
    if ids.is_empty() {
      return Some("[]".to_string());
    }
    let blocks = self.doc.as_ref()?.blocks_by_id(&ids);
    Some(serde_json::to_string(&blocks).unwrap_or_else(|_| "[]".to_string()))
  }

  // ── Internal event handlers ──────────────────────────────────────────────

  fn use_append_log(&self) -> bool {
    self.storage.is_some()
  }

  fn handle_connect(&mut self, effects: &mut Vec<SyncEffect>) {
    effects.push(SyncEffect::CancelTimer {
      kind: TimerKind::Reconnect,
    });
    effects.push(SyncEffect::CancelTimer {
      kind: TimerKind::Heartbeat,
    });
    effects.push(SyncEffect::CancelTimer {
      kind: TimerKind::LagResync,
    });
    if self.socket_open {
      effects.push(SyncEffect::CloseSocket { epoch: self.epoch });
      self.socket_open = false;
    }

    self.epoch = self.epoch.saturating_add(1);
    self.reconnect_scheduled = false;
    self.online = false;
    self.lag_debounce_active = false;
    self.lag_trailing_pending = false;

    // Per-connection outbox & repair bookkeeping resets on every (re)connect.
    self.sent_through_clock = 0;
    self.acked_ahead.clear();
    self.run_from.clear();
    self.de_merge_from.clear();
    self.push_rejects = 0;
    self.push_stalled = false;
    self.repairs = 0;

    self.restore_unacked_once();
    self.seed_from_local_once(effects);

    effects.push(SyncEffect::OpenSocket { epoch: self.epoch });
    self.update_sync_phase(effects);
  }

  fn handle_socket_opened(&mut self, epoch: u64, effects: &mut Vec<SyncEffect>) {
    if epoch != self.epoch {
      return;
    }
    self.socket_open = true;
    self.frame_since_heartbeat = false;
    self.last_frame_ms = None;

    effects.push(SyncEffect::ScheduleTimer {
      kind: TimerKind::Heartbeat,
      delay_ms: PING_EVERY_MS,
      epoch,
    });

    // Cold start → full sync.bootstrap; reconnect/seeded → SV catch-up.
    let frame = self.catch_up_frame();
    self.send_ws(frame, effects);
  }

  fn handle_socket_closed(&mut self, epoch: u64, _reason: &str, effects: &mut Vec<SyncEffect>) {
    if epoch != self.epoch {
      return;
    }
    self.socket_open = false;
    self.lag_debounce_active = false;
    self.lag_trailing_pending = false;
    effects.push(SyncEffect::CancelTimer {
      kind: TimerKind::Heartbeat,
    });
    effects.push(SyncEffect::CancelTimer {
      kind: TimerKind::LagResync,
    });
    self.online = false;
    self.update_sync_phase(effects);
    self.schedule_reconnect(effects);
  }

  fn handle_tick(
    &mut self,
    now_ms: u64,
    timer_kind: TimerKind,
    epoch: u64,
    effects: &mut Vec<SyncEffect>,
  ) {
    self.last_known_now_ms = Some(now_ms);
    match timer_kind {
      TimerKind::Heartbeat => {
        if epoch != self.epoch || !self.socket_open {
          return;
        }
        if self.frame_since_heartbeat {
          self.frame_since_heartbeat = false;
          self.last_frame_ms = Some(now_ms);
        } else if let Some(last) = self.last_frame_ms {
          if self.online && now_ms.saturating_sub(last) > STALE_MS {
            self.online = false;
            self.update_sync_phase(effects);
            effects.push(SyncEffect::CancelTimer {
              kind: TimerKind::Heartbeat,
            });
            effects.push(SyncEffect::CloseSocket { epoch });
            return;
          }
        } else {
          self.last_frame_ms = Some(now_ms);
        }
        self.send_ws(frame_ping(), effects);
        effects.push(SyncEffect::ScheduleTimer {
          kind: TimerKind::Heartbeat,
          delay_ms: PING_EVERY_MS,
          epoch,
        });
      }
      TimerKind::Reconnect => {
        if epoch != self.epoch || self.socket_open {
          return;
        }
        self.reconnect_scheduled = false;
        self.handle_connect(effects);
      }
      TimerKind::LagResync => {
        if epoch != self.epoch {
          return;
        }
        self.lag_debounce_active = false;
        if self.lag_trailing_pending {
          self.lag_trailing_pending = false;
          self.resync_from_lag(effects);
        }
      }
      TimerKind::LocalUpdateRetry => {
        // Local persistence retry is orthogonal to socket epoch: offline edits
        // must survive reconnects without losing their retry schedule.
        self.local_retry_scheduled = false;
        if self.flush_pending_local_diff(effects).is_err() {
          self.local_persist_faults += 1;
          effects.push(SyncEffect::Fault {
            reason: "local_update_persist_failed".to_string(),
            count: self.local_persist_faults,
          });
        }
      }
    }
  }

  fn handle_socket_frame(&mut self, epoch: u64, raw: &str, effects: &mut Vec<SyncEffect>) {
    if epoch != self.epoch || !self.socket_open {
      return;
    }
    let Ok(frame) = serde_json::from_str::<Value>(raw) else {
      return;
    };
    if !frame.is_object() {
      return;
    }

    // Any valid JSON frame proves the link is live.
    self.reconnect_attempts = 0;
    self.frame_since_heartbeat = true;
    if let Some(now) = self.last_known_now_ms {
      self.last_frame_ms = Some(now);
    }
    if !self.online {
      self.online = true;
      self.update_sync_phase(effects);
    }
    if !self.saw_server_frame {
      self.saw_server_frame = true;
      effects.push(SyncEffect::ServerConnected);
    }

    let Some(msg_type) = frame.get("type").and_then(Value::as_str) else {
      return;
    };
    match msg_type {
      "document.bootstrap" => {
        self.connection_id = frame
          .get("connection_id")
          .and_then(Value::as_str)
          .map(String::from);
        self.send_presence(effects);
      }
      "presence.state" => {
        self.presence.clear();
        if let Some(list) = frame.get("presences").and_then(Value::as_array) {
          for entry in list {
            self.upsert_presence(entry);
          }
        }
        self.emit_presence(effects);
      }
      "presence.update" => {
        self.upsert_presence(&frame);
        self.emit_presence(effects);
      }
      "presence.leave" => {
        if let Some(cid) = frame.get("connection_id").and_then(Value::as_str) {
          self.presence.remove(cid);
        }
        self.emit_presence(effects);
      }
      "document.update.accepted" => {
        if let Some(seq) = frame.get("server_seq").and_then(Value::as_i64) {
          effects.push(SyncEffect::AcceptedSeq(seq));
        }
      }
      "sync.base" => self.handle_sync_base(&frame, effects),
      "sync.updates" => self.handle_sync_updates(&frame, effects),
      "sync.update" => {
        if self.apply_remote(&frame, true, effects).is_some() {
          self.emit_remote_blocks_changed(effects);
        }
      }
      "sync.ack" => self.handle_sync_ack(&frame, effects),
      "error" => self.handle_error_frame(&frame, effects),
      _ => {}
    }
  }

  fn handle_sync_base(&mut self, frame: &Value, effects: &mut Vec<SyncEffect>) {
    let Some(b64) = frame.get("base").and_then(Value::as_str) else {
      return;
    };
    let Ok(bytes) = STANDARD.decode(b64.as_bytes()) else {
      self.on_integrity_fault("bad_base", effects);
      return;
    };
    let base_rid = frame.get("base_rid").and_then(Value::as_i64).unwrap_or(0);

    if self.doc.is_none() {
      // Cold bootstrap.
      let Ok(mut doc) = decode_doc_guarded(&bytes, self.config.client_id) else {
        self.on_integrity_fault("bad_base", effects);
        return;
      };
      doc.track_changes();
      let _ = doc.take_changed_block_ids();
      self.root_block_id = doc.root_block_id();
      self.cursor = base_rid;
      self.doc = Some(doc);
      self.ready = true;
      effects.push(SyncEffect::Ready {
        root_block_id: self.root_block_id.clone(),
      });
    } else {
      // Re-bootstrap: merge unconditionally even if `base_rid <= self.cursor`
      // (red line #1: a broadcast may have jumped the cursor past missing deps).
      let ok = self
        .doc
        .as_mut()
        .is_some_and(|doc| apply_update_guarded(doc, &bytes).is_ok());
      if !ok {
        self.on_integrity_fault("bad_base", effects);
        return;
      }
      if base_rid > self.cursor {
        self.cursor = base_rid;
      }
      self.emit_remote_blocks_changed(effects);
    }

    // A good base resets the consecutive integrity-fault streak (B3).
    self.fault_count = 0;

    // Pull tail after base, resend queued local edits, handle B11 repair, persist base.
    let pull = self.pull_frame();
    self.send_ws(pull, effects);
    self.flush_unacked(true, effects);
    if let Some(sv_b64) = repair_sv_for(frame, self.outbox_empty()) {
      self.push_repair(sv_b64, effects);
    }
    self.save_base_now(effects);
    self.update_sync_phase(effects);
  }

  fn handle_sync_updates(&mut self, frame: &Value, effects: &mut Vec<SyncEffect>) {
    let Some(ups) = frame.get("updates").and_then(Value::as_array) else {
      return;
    };
    let before = self.cursor;
    let mut changed = false;
    let mut had_fault = false;
    let mut applied: Vec<(i64, Vec<u8>)> = Vec::new();

    for item in ups {
      if !item.is_object() {
        continue;
      }
      let faults_before = self.fault_count;
      match self.apply_remote(item, false, effects) {
        Some(Some((rid, bytes))) => {
          changed = true;
          applied.push((rid, bytes));
        }
        Some(None) => {
          changed = true;
        }
        None => {
          if self.fault_count > faults_before {
            had_fault = true;
          }
        }
      }
    }

    // Single transaction for the whole batch (P4-1).
    if !applied.is_empty() {
      let append_ok = self
        .storage
        .as_mut()
        .map(|store| store.append_remote_batch(&applied).is_ok())
        .unwrap_or(true);
      if !append_ok {
        self.heal_persist_failure(effects);
      }
      self.maybe_compact(effects);
    }

    if changed {
      self.emit_remote_blocks_changed(effects);
    }

    // B2 verified catch-up: keep pulling while the batch was non-empty and made
    // forward cursor progress (unless an integrity fault triggered re-bootstrap).
    if !ups.is_empty() && self.cursor > before && !had_fault && self.socket_open {
      let pull = self.pull_frame();
      self.send_ws(pull, effects);
    }
  }

  fn handle_sync_ack(&mut self, frame: &Value, effects: &mut Vec<SyncEffect>) {
    // B11: server parked our push as pending due to missing history.
    if let Some(sv_b64) = repair_sv_for(frame, self.outbox_empty()) {
      self.push_repair(sv_b64, effects);
    }

    let ack_id = frame.get("ack_id").and_then(Value::as_str);
    let rid = frame.get("rid").and_then(Value::as_i64);

    if self.use_append_log() {
      if let Some(clock) = ack_id.and_then(|s| s.parse::<i64>().ok()) {
        let pushed_before = self
          .storage
          .as_deref()
          .and_then(|s| s.cursor().ok())
          .map(|c| c.pushed_clock)
          .unwrap_or(0);
        if clock > pushed_before {
          self.acked_ahead.insert(clock);
          // Advance `pushed_clock` strictly through contiguous acked runs so a
          // rejected lower clock is never skipped over.
          let mut pushed = pushed_before;
          loop {
            let next_through = self.acked_ahead.iter().copied().find(|&through| {
              let start = self.run_from.get(&through).copied().unwrap_or(through);
              start == pushed + 1
            });
            let Some(through) = next_through else {
              break;
            };
            self.acked_ahead.remove(&through);
            self.run_from.remove(&through);
            pushed = through;
          }
          if pushed != pushed_before {
            if let Some(store) = self.storage.as_mut() {
              let _ = store.advance_cursor(None, Some(pushed));
            }
            self.de_merge_from.retain(|&from| from > pushed);
            self.push_rejects = 0;
            self.update_sync_phase(effects);
            self.flush_unacked(false, effects);
          }
        }
      }
    } else if let Some(id) = ack_id {
      let before = self.unacked.len();
      self.unacked.retain(|p| p.id != id);
      if self.unacked.len() != before {
        self.update_sync_phase(effects);
        self.flush_unacked(false, effects);
      }
    }

    if let Some(rid) = rid {
      if rid > self.cursor {
        self.cursor = rid;
        if let Some(store) = self.storage.as_mut() {
          let stored_rid = store.cursor().map(|c| c.last_synced_rid).unwrap_or(0);
          if rid > stored_rid {
            let _ = store.advance_cursor(Some(rid), None);
          }
        }
      }
    }
  }

  fn handle_error_frame(&mut self, frame: &Value, effects: &mut Vec<SyncEffect>) {
    let err_id = frame.get("ack_id").and_then(Value::as_str);
    let code = frame.get("code").and_then(Value::as_str);

    // Server broadcast lag notice (`client_out_of_date`, no ack_id).
    if err_id.is_none() && code == Some("client_out_of_date") {
      self.resync_from_lag(effects);
      return;
    }

    // Online-only path: release the rejected push's window slot so later edits
    // can drain; reconnect retries it.
    if !self.use_append_log() {
      if let Some(id) = err_id {
        for p in &mut self.unacked {
          if p.id == id {
            p.rejected = true;
          }
        }
        self.flush_unacked(false, effects);
      }
      return;
    }

    // Durable append-log path: de-merge rejected run & retry bounded times.
    if let Some(clock) = err_id.and_then(|s| s.parse::<i64>().ok()) {
      let from = self.run_from.remove(&clock).unwrap_or(clock);
      if from != clock {
        self.de_merge_from.insert(from);
      }
      self.acked_ahead.remove(&clock);
      if from - 1 < self.sent_through_clock {
        self.sent_through_clock = from - 1;
      }
      if self.push_rejects < MAX_PUSH_REJECTS {
        self.push_rejects += 1;
        self.flush_unacked(false, effects);
      } else {
        self.push_stalled = true;
        effects.push(SyncEffect::Fault {
          reason: "push_rejected".to_string(),
          count: self.push_rejects as u32,
        });
      }
    }
  }

  fn handle_apply_local_diff(&mut self, diff: Vec<u8>, effects: &mut Vec<SyncEffect>) {
    if is_empty_yrs_update(&diff) {
      return;
    }
    // Flush any previously retained diff first so newer edits never overtake a
    // missing causal dependency.
    if self.pending_local_diff.is_some() && self.flush_pending_local_diff(effects).is_err() {
      // Previous persist still failing: fold the new diff into the single retry
      // slot so neither edit is dropped or reordered.
      if let Some(existing) = self.pending_local_diff.as_ref() {
        if let Ok(merged) = merge_yrs_updates(&[existing.as_slice(), diff.as_slice()]) {
          self.pending_local_diff = Some(merged);
        }
      }
      self.local_persist_faults += 1;
      effects.push(SyncEffect::Fault {
        reason: "local_update_persist_failed".to_string(),
        count: self.local_persist_faults,
      });
      self.update_sync_phase(effects);
      return;
    }

    self.pending_local_diff = Some(diff);
    self.update_sync_phase(effects);
    if self.flush_pending_local_diff(effects).is_err() {
      self.local_persist_faults += 1;
      effects.push(SyncEffect::Fault {
        reason: "local_update_persist_failed".to_string(),
        count: self.local_persist_faults,
      });
    }
  }

  fn handle_send_presence_cursor(
    &mut self,
    block_id: Option<String>,
    offset: Option<i64>,
    effects: &mut Vec<SyncEffect>,
  ) {
    self.presence_cursor = match (block_id, offset) {
      (Some(b), Some(o)) => Some((b, o)),
      _ => None,
    };
    self.send_presence(effects);
  }

  fn handle_flush_for_exit(&mut self, effects: &mut Vec<SyncEffect>) {
    // Fast durability flush only: write the single-slot pending diff to
    // storage (or in-memory unacked), skipping compaction and socket sends.
    let Some(diff) = self.pending_local_diff.clone() else {
      return;
    };
    let result = if let Some(store) = self.storage.as_mut() {
      store.append_outbox(&diff).map(|_| ())
    } else {
      let id = self.push_seq.to_string();
      self.push_seq += 1;
      self.unacked.push(PendingInMemory {
        id,
        bytes: diff,
        sent: false,
        rejected: false,
      });
      Ok(())
    };
    match result {
      Ok(()) => {
        self.pending_local_diff = None;
        self.local_retry_attempts = 0;
        if self.local_retry_scheduled {
          self.local_retry_scheduled = false;
          effects.push(SyncEffect::CancelTimer {
            kind: TimerKind::LocalUpdateRetry,
          });
        }
        self.update_sync_phase(effects);
      }
      Err(_) => {
        self.local_persist_faults += 1;
        effects.push(SyncEffect::Fault {
          reason: "local_update_persist_failed".to_string(),
          count: self.local_persist_faults,
        });
      }
    }
  }

  fn handle_mirror_write_failed(&mut self, effects: &mut Vec<SyncEffect>) {
    if let Some(store) = self.storage.as_mut() {
      store.mark_broken();
    }
    self.persist_fails += 1;
    effects.push(SyncEffect::Fault {
      reason: "mirror_write_failed".to_string(),
      count: self.persist_fails,
    });
  }

  // ── Seeding, persistence, & outbox helpers ───────────────────────────────

  fn restore_unacked_once(&mut self) {
    if self.restored {
      return;
    }
    self.restored = true;
    if self.use_append_log() {
      return;
    }
    let restored = std::mem::take(&mut self.config.restore_unacked);
    for bytes in restored {
      if is_empty_yrs_update(&bytes) {
        continue;
      }
      let id = self.push_seq.to_string();
      self.push_seq += 1;
      self.unacked.push(PendingInMemory {
        id,
        bytes,
        sent: false,
        rejected: false,
      });
    }
  }

  fn seed_from_local_once(&mut self, effects: &mut Vec<SyncEffect>) {
    if self.seeded || self.doc.is_some() || self.storage.is_none() {
      return;
    }
    self.seeded = true;
    let load_res = self.storage.as_deref().unwrap().load();
    let stored = match load_res {
      Ok(Some(doc)) => doc,
      Ok(None) => return,
      Err(StorageError::Corrupt(_)) => {
        // Recoverable corrupt local cache: fall through to cold bootstrap from
        // the authoritative server.
        return;
      }
      Err(StorageError::Unavailable(_) | StorageError::Io(_)) => {
        self.persist_fails += 1;
        effects.push(SyncEffect::Fault {
          reason: "local_seed_failed".to_string(),
          count: self.persist_fails,
        });
        return;
      }
    };
    let Ok(mut doc) = decode_doc_guarded(&stored.state, self.config.client_id) else {
      return;
    };
    doc.track_changes();
    let _ = doc.take_changed_block_ids();
    self.root_block_id = doc.root_block_id();
    self.cursor = stored.cursor;
    self.doc = Some(doc);
    self.ready = true;
    effects.push(SyncEffect::Ready {
      root_block_id: self.root_block_id.clone(),
    });
  }

  fn flush_pending_local_diff(&mut self, effects: &mut Vec<SyncEffect>) -> Result<(), StorageError> {
    let Some(diff) = self.pending_local_diff.clone() else {
      return Ok(());
    };
    match self.enqueue_diff(&diff, effects) {
      Ok(()) => {
        self.pending_local_diff = None;
        self.local_retry_attempts = 0;
        if self.local_retry_scheduled {
          self.local_retry_scheduled = false;
          effects.push(SyncEffect::CancelTimer {
            kind: TimerKind::LocalUpdateRetry,
          });
        }
        self.update_sync_phase(effects);
        Ok(())
      }
      Err(err) => {
        if !self.local_retry_scheduled && self.local_retry_attempts < LOCAL_UPDATE_MAX_RETRIES {
          let delay_ms = local_update_retry_delay_ms(self.local_retry_attempts);
          self.local_retry_attempts += 1;
          self.local_retry_scheduled = true;
          effects.push(SyncEffect::ScheduleTimer {
            kind: TimerKind::LocalUpdateRetry,
            delay_ms,
            epoch: self.epoch,
          });
        }
        Err(err)
      }
    }
  }

  fn enqueue_diff(&mut self, diff: &[u8], effects: &mut Vec<SyncEffect>) -> Result<(), StorageError> {
    if self.use_append_log() {
      let clock = self.storage.as_mut().unwrap().append_outbox(diff)?;
      self.maybe_compact(effects);
      if self.socket_open && self.ready && !self.push_stalled {
        self.send_push_raw(&clock.to_string(), diff, effects);
        // Issue #159: advance ONLY the contiguous sent prefix so a live edit
        // during a capped merge window cannot skip unsent earlier clocks.
        if clock == self.sent_through_clock + 1 {
          self.sent_through_clock = clock;
        }
      }
      return Ok(());
    }

    let id = self.push_seq.to_string();
    self.push_seq += 1;
    self.unacked.push(PendingInMemory {
      id,
      bytes: diff.to_vec(),
      sent: false,
      rejected: false,
    });
    if self.socket_open && self.ready {
      self.flush_unacked(false, effects);
    }
    Ok(())
  }

  fn flush_unacked(&mut self, resend_all: bool, effects: &mut Vec<SyncEffect>) {
    if !self.socket_open || !self.ready {
      return;
    }

    if self.use_append_log() {
      if self.push_stalled && !resend_all {
        return;
      }
      let (pushed, pending) = {
        let store = self.storage.as_deref().unwrap();
        let pushed = store.cursor().map(|c| c.pushed_clock).unwrap_or(0);
        let floor = if resend_all || pushed > self.sent_through_clock {
          pushed
        } else {
          self.sent_through_clock
        };
        let Ok(pending) = store.outbox_after(floor) else {
          return;
        };
        (pushed, pending)
      };
      if pending.is_empty() {
        return;
      }

      // Fold ≥8 queued entries (up to 4 MiB) into a single merged push unless
      // this run was previously rejected and is being de-merged to isolate a
      // poison entry.
      let de_merging = self.de_merge_from.contains(&pending[0].clock);
      if !de_merging && pending.len() >= MERGE_MIN_RUN {
        let sizes: Vec<usize> = pending.iter().map(|e| e.bytes.len()).collect();
        let run = merge_run_length(&sizes, MERGE_MAX_BYTES);
        let head = &pending[..run];
        let slices: Vec<&[u8]> = head.iter().map(|e| e.bytes.as_slice()).collect();
        if let Ok(merged) = merge_yrs_updates(&slices) {
          let through = head.last().unwrap().clock;
          self.run_from.insert(through, head[0].clock);
          self.send_push_raw(&through.to_string(), &merged, effects);
          if through > self.sent_through_clock {
            self.sent_through_clock = through;
          }
          return;
        }
        // Merge failed (corrupt entry): fall through to individual pushes to
        // isolate the poison entry.
      }

      // Bounded 64-slot push window.
      let used = if resend_all {
        0
      } else {
        let in_range = (self.sent_through_clock - pushed).max(0) as usize;
        in_range.saturating_sub(self.acked_ahead.len())
      };
      let available = PUSH_WINDOW.saturating_sub(used);
      for entry in pending.iter().take(available) {
        self.send_push_raw(&entry.clock.to_string(), &entry.bytes, effects);
        if entry.clock > self.sent_through_clock {
          self.sent_through_clock = entry.clock;
        }
      }
      return;
    }

    // In-memory outbox (online-only mode).
    if resend_all {
      for p in &mut self.unacked {
        p.sent = false;
        p.rejected = false;
      }
    }
    let indices: Vec<usize> = {
      let indexed: Vec<(usize, &PendingInMemory)> = self.unacked.iter().enumerate().collect();
      push_slice(
        &indexed,
        |(_, p)| p.sent,
        |(_, p)| p.sent && !p.rejected,
        PUSH_WINDOW,
      )
      .into_iter()
      .map(|(idx, _)| *idx)
      .collect()
    };
    for idx in indices {
      self.unacked[idx].sent = true;
      let id = self.unacked[idx].id.clone();
      let bytes = self.unacked[idx].bytes.clone();
      self.send_push_raw(&id, &bytes, effects);
    }
  }

  fn push_repair(&mut self, sv_b64: &str, effects: &mut Vec<SyncEffect>) {
    if !self.socket_open || self.repairs >= MAX_REPAIRS {
      return;
    }
    let Some(doc) = self.doc.as_ref() else {
      return;
    };
    self.repairs += 1;
    let Ok(sv_bytes) = STANDARD.decode(sv_b64.as_bytes()) else {
      return;
    };
    let Ok(diff) = doc.encode_diff(&sv_bytes) else {
      return;
    };
    if is_empty_yrs_update(&diff) {
      return;
    }
    let id = format!("repair-{}", self.repairs);
    self.send_push_raw(&id, &diff, effects);
  }

  /// Apply one remote update frame. Returns `Some(Some((rid, bytes)))` on
  /// success with a `rid`, `Some(None)` on success without a `rid`, or `None`
  /// on skip/fault.
  fn apply_remote(
    &mut self,
    item: &Value,
    persist: bool,
    effects: &mut Vec<SyncEffect>,
  ) -> Option<Option<(i64, Vec<u8>)>> {
    self.doc.as_ref()?;
    let b64 = item.get("update").and_then(Value::as_str)?;
    let Ok(bytes) = STANDARD.decode(b64.as_bytes()) else {
      self.on_integrity_fault("bad_remote_update", effects);
      return None;
    };
    let apply_ok = self
      .doc
      .as_mut()
      .is_some_and(|doc| apply_update_guarded(doc, &bytes).is_ok());
    if !apply_ok {
      // Red line #1: never skip a bad remote update or advance the cursor past it.
      self.on_integrity_fault("bad_remote_update", effects);
      return None;
    }

    let rid = item.get("rid").and_then(Value::as_i64);
    if let Some(rid) = rid {
      if rid > self.cursor {
        self.cursor = rid;
      }
      if persist {
        let persist_ok = self
          .storage
          .as_mut()
          .map(|store| store.append_remote(rid, &bytes).is_ok())
          .unwrap_or(true);
        if !persist_ok {
          self.heal_persist_failure(effects);
        }
        self.maybe_compact(effects);
      }
      Some(Some((rid, bytes)))
    } else {
      Some(None)
    }
  }

  fn heal_persist_failure(&mut self, effects: &mut Vec<SyncEffect>) {
    self.save_base_now(effects);
    self.persist_fails += 1;
    effects.push(SyncEffect::Fault {
      reason: "persist_failed".to_string(),
      count: self.persist_fails,
    });
  }

  fn save_base_now(&mut self, _effects: &mut Vec<SyncEffect>) {
    let (Some(doc), Some(store)) = (self.doc.as_ref(), self.storage.as_mut()) else {
      return;
    };
    let state = doc.encode_state();
    let _ = store.save_base(&state, self.cursor);
  }

  fn maybe_compact(&mut self, _effects: &mut Vec<SyncEffect>) {
    if self.storage.is_none() || self.doc.is_none() {
      return;
    }
    self.appends_since_compact_check += 1;
    if self.appends_since_compact_check < COMPACT_CHECK_EVERY {
      return;
    }
    self.appends_since_compact_check = 0;
    let store = self.storage.as_mut().unwrap();
    if let Ok(sizes) = store.log_sizes() {
      if sizes.local + sizes.remote > COMPACT_THRESHOLD {
        let _ = store.compact();
      }
    }
  }

  fn on_integrity_fault(&mut self, reason: &str, effects: &mut Vec<SyncEffect>) {
    self.fault_count += 1;
    effects.push(SyncEffect::Fault {
      reason: reason.to_string(),
      count: self.fault_count,
    });
    // Self-heal by requesting a FULL base (no sv — a corrupt replica's sv is
    // not a safe diff basis), bounded by MAX_AUTO_REHEAL.
    if self.socket_open && (self.fault_count as usize) <= MAX_AUTO_REHEAL {
      self.send_ws(frame_sync_bootstrap(None), effects);
    }
  }

  fn resync_from_lag(&mut self, effects: &mut Vec<SyncEffect>) {
    if self.lag_debounce_active {
      self.lag_trailing_pending = true;
      return;
    }
    self.lag_debounce_active = true;
    self.lag_trailing_pending = false;
    self.lag_resync_count += 1;
    effects.push(SyncEffect::ScheduleTimer {
      kind: TimerKind::LagResync,
      delay_ms: LAG_RESYNC_DEBOUNCE_MS,
      epoch: self.epoch,
    });
    if self.socket_open {
      let frame = self.catch_up_frame();
      self.send_ws(frame, effects);
    }
  }

  fn schedule_reconnect(&mut self, effects: &mut Vec<SyncEffect>) {
    if self.socket_open || self.reconnect_scheduled {
      return;
    }
    let delay_ms = reconnect_backoff_ms(self.reconnect_attempts);
    self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
    self.reconnect_scheduled = true;
    effects.push(SyncEffect::ScheduleTimer {
      kind: TimerKind::Reconnect,
      delay_ms,
      epoch: self.epoch,
    });
  }

  fn catch_up_frame(&self) -> String {
    match self.doc.as_ref() {
      Some(doc) => frame_sync_bootstrap(Some(&doc.state_vector())),
      None => frame_sync_bootstrap(None),
    }
  }

  fn pull_frame(&self) -> String {
    let sv = self.doc.as_ref().map(MicaDoc::state_vector);
    frame_sync_pull(self.cursor, sv.as_deref())
  }

  fn send_push_raw(&self, id: &str, bytes: &[u8], effects: &mut Vec<SyncEffect>) {
    self.send_ws(frame_sync_push(id, bytes), effects);
  }

  fn send_ws(&self, payload: String, effects: &mut Vec<SyncEffect>) {
    if !self.socket_open {
      return;
    }
    effects.push(SyncEffect::SendWsJson {
      epoch: self.epoch,
      payload,
    });
  }

  fn send_presence(&self, effects: &mut Vec<SyncEffect>) {
    let Some(name) = self.config.self_name.as_deref() else {
      return;
    };
    if !self.socket_open {
      return;
    }
    let cursor = self
      .presence_cursor
      .as_ref()
      .map(|(b, o)| (b.as_str(), *o));
    self.send_ws(frame_presence_update(name, cursor), effects);
  }

  fn upsert_presence(&mut self, msg: &Value) {
    let Some(connection_id) = msg.get("connection_id").and_then(Value::as_str) else {
      return;
    };
    let Some(user_id) = msg.get("user_id").and_then(Value::as_str) else {
      return;
    };
    let mut name = user_id.to_string();
    let mut cursor_block_id = None;
    let mut cursor_offset = None;

    if let Some(data) = msg.get("data").and_then(Value::as_object) {
      if let Some(n) = data.get("name").and_then(Value::as_str) {
        name = n.to_string();
      }
      if let Some(cursor) = data.get("cursor").and_then(Value::as_object) {
        if let (Some(block), Some(offset)) = (
          cursor.get("block").and_then(Value::as_str),
          cursor.get("offset").and_then(Value::as_i64),
        ) {
          cursor_block_id = Some(block.to_string());
          cursor_offset = Some(offset);
        }
      }
    }

    self.presence.insert(
      connection_id.to_string(),
      PresenceUser {
        connection_id: connection_id.to_string(),
        user_id: user_id.to_string(),
        name,
        cursor_block_id,
        cursor_offset,
      },
    );
  }

  fn emit_presence(&self, effects: &mut Vec<SyncEffect>) {
    let others: Vec<PresenceUser> = self
      .presence
      .values()
      .filter(|u| self.connection_id.as_deref() != Some(u.connection_id.as_str()))
      .cloned()
      .collect();
    effects.push(SyncEffect::PresenceChanged(others));
  }

  fn emit_remote_blocks_changed(&mut self, effects: &mut Vec<SyncEffect>) {
    let changed_ids = self.doc.as_mut().and_then(MicaDoc::take_changed_block_ids);
    self.changed_acc.record(changed_ids.clone());
    effects.push(SyncEffect::RemoteBlocksChanged { changed_ids });
  }

  fn update_sync_phase(&mut self, effects: &mut Vec<SyncEffect>) {
    let phase = self.phase();
    if self.last_emitted_phase == Some(phase) {
      return;
    }
    self.last_emitted_phase = Some(phase);
    effects.push(SyncEffect::SyncPhaseChanged(phase));
  }

  fn drain_mirror_ops(&mut self, effects: &mut Vec<SyncEffect>) {
    if let Some(store) = self.storage.as_mut() {
      let ops = store.take_pending_mirror_txns();
      if !ops.is_empty() {
        effects.push(SyncEffect::MirrorIdb(ops));
      }
    }
  }
}

// ── Panic-guarded yrs helpers (native unwind containment) ────────────────────

fn decode_doc_guarded(bytes: &[u8], client_id: Option<u64>) -> Result<MicaDoc, String> {
  let work = || MicaDoc::from_update_with_client_id(bytes, client_id).map_err(|e| e.to_string());
  #[cfg(not(target_arch = "wasm32"))]
  {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
      Ok(res) => res,
      Err(_) => Err("yrs panicked while decoding base update".into()),
    }
  }
  #[cfg(target_arch = "wasm32")]
  {
    work()
  }
}

fn apply_update_guarded(doc: &mut MicaDoc, bytes: &[u8]) -> Result<(), String> {
  #[cfg(not(target_arch = "wasm32"))]
  {
    let work = || doc.apply_update(bytes).map_err(|e| e.to_string());
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
      Ok(res) => res,
      Err(_) => Err("yrs panicked while applying update".into()),
    }
  }
  #[cfg(target_arch = "wasm32")]
  {
    doc.apply_update(bytes).map_err(|e| e.to_string())
  }
}
