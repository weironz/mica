//! Pure WebSocket sync protocol types, constants, framing, and pacing helpers.
//!
//! All numeric constants and pure decision functions match the Dart reference
//! (`cloud_sync_session.dart` / `pending_doc_update.dart` / `sync_status.dart`)
//! and the API server (`crates/api-server/src/routes/ws.rs`).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ── Protocol & pacing constants ──────────────────────────────────────────────

/// Max concurrent un-acked pushes in flight (both append-log and in-memory).
pub const PUSH_WINDOW: usize = 64;

/// Max raw byte sum folded into a single merged push (well below server 64 MiB frame cap).
pub const MERGE_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Minimum queued backlog length required before folding into one merged push.
pub const MERGE_MIN_RUN: usize = 8;

/// Max consecutive push rejections without contiguous progress before stalling.
pub const MAX_PUSH_REJECTS: usize = 5;

/// Max automatic integrity-fault re-bootstraps before circuit-breaking.
pub const MAX_AUTO_REHEAL: usize = 3;

/// Max B11 history-repair pushes per connection.
pub const MAX_REPAIRS: usize = 3;

/// Number of log appends between compaction threshold checks.
pub const COMPACT_CHECK_EVERY: usize = 32;

/// Combined `(local + remote)` log row count past which compaction runs.
pub const COMPACT_THRESHOLD: usize = 256;

/// Heartbeat ping interval in milliseconds.
pub const PING_EVERY_MS: u64 = 8_000;

/// Inbound frame silence threshold past which an online socket is treated as dead.
pub const STALE_MS: u64 = 20_000;

/// Trailing-edge debounce window for server broadcast-lag notices (`client_out_of_date`).
pub const LAG_RESYNC_DEBOUNCE_MS: u64 = 1_000;

/// Initial reconnect backoff delay (ms): 500ms → 1s → 2s → 4s → 8s → 16s → 30s.
pub const RECONNECT_BASE_MS: u64 = 500;

/// Maximum reconnect backoff delay (ms).
pub const RECONNECT_MAX_MS: u64 = 30_000;

/// Maximum bit shift applied to [`RECONNECT_BASE_MS`] (`0..=6`).
pub const RECONNECT_MAX_SHIFT: u32 = 6;

/// Initial retry delay for the single-slot `pending_local_diff` buffer (400ms * 2^attempt).
pub const LOCAL_UPDATE_RETRY_BASE_MS: u64 = 400;

/// Maximum retry attempts for a failing local diff append before halting retries.
pub const LOCAL_UPDATE_MAX_RETRIES: u32 = 5;

/// Canonical empty lib0 v1 yrs update (zero structs, empty delete set).
pub const EMPTY_YRS_UPDATE: [u8; 2] = [0, 0];

// ── Shared DTOs ──────────────────────────────────────────────────────────────

/// Derived sync status surfaced to the UI — three states only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
  /// Online and nothing left to send.
  Synced,
  /// Online with local edits still draining to the server.
  Syncing,
  /// Disconnected (edits continue persisting locally).
  Offline,
}

/// Pure derivation of [`SyncPhase`] from `(online, pending)`.
///
/// Offline always dominates: a down link is [`SyncPhase::Offline`] regardless
/// of whether local edits are pending.
pub fn derive_sync_phase(online: bool, pending: bool) -> SyncPhase {
  if !online {
    SyncPhase::Offline
  } else if pending {
    SyncPhase::Syncing
  } else {
    SyncPhase::Synced
  }
}

/// Another connected user's room presence / caret state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceUser {
  pub connection_id: String,
  pub user_id: String,
  pub name: String,
  pub cursor_block_id: Option<String>,
  pub cursor_offset: Option<i64>,
}

/// Logical timer categories scheduled and cancelled by [`crate::SyncEngine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerKind {
  /// Periodic ping + frame-silence watchdog ([`PING_EVERY_MS`] / [`STALE_MS`]).
  Heartbeat,
  /// Exponential backoff reconnect timer ([`reconnect_backoff_ms`]).
  Reconnect,
  /// Trailing-edge debounce timer for `client_out_of_date` lag notices.
  LagResync,
  /// Backoff retry timer for the single-slot `pending_local_diff` buffer.
  LocalUpdateRetry,
}

// ── Pure pacing & protocol helpers ───────────────────────────────────────────

/// Select the slice of an in-memory outbox that may be placed on the wire right
/// now without exceeding `window` active slots.
///
/// `occupies_slot` is narrower than `already_sent`: a rejected push stays marked
/// sent (so live flushes don't re-flood it) but releases its window slot so the
/// backlog behind it can still drain.
pub fn push_slice<T>(
  queue: &[T],
  already_sent: impl Fn(&T) -> bool,
  occupies_slot: impl Fn(&T) -> bool,
  window: usize,
) -> Vec<&T> {
  let mut used = queue.iter().filter(|e| occupies_slot(e)).count();
  let mut out = Vec::new();
  for entry in queue {
    if already_sent(entry) {
      continue;
    }
    if used >= window {
      break;
    }
    out.push(entry);
    used += 1;
  }
  out
}

/// Compute how many leading queued updates fold into a single merged push capped
/// at `max_bytes`.
///
/// Always returns at least `1` for non-empty input, even when the first entry
/// alone exceeds `max_bytes` (refusing to send it would stall the outbox forever).
pub fn merge_run_length(sizes: &[usize], max_bytes: usize) -> usize {
  if sizes.is_empty() {
    return 0;
  }
  let mut total = 0usize;
  for (i, &sz) in sizes.iter().enumerate() {
    total = total.saturating_add(sz);
    if total > max_bytes {
      return if i == 0 { 1 } else { i };
    }
  }
  sizes.len()
}

/// Extract the server state vector (base64) that a frame asks us to repair
/// against (B11), or `None`.
///
/// Two frames request repair:
/// - `sync.ack` with `missing == true`: the server parked our push as pending.
/// - `sync.base` with `ahead == true` AND an empty local outbox: we hold
///   previously-acked insertions the server lost (e.g. restored from backup).
pub fn repair_sv_for(frame: &Value, outbox_empty: bool) -> Option<&str> {
  let sv = frame.get("sv")?.as_str()?;
  match frame.get("type")?.as_str()? {
    "sync.ack" if frame.get("missing").and_then(Value::as_bool) == Some(true) => Some(sv),
    "sync.base" if frame.get("ahead").and_then(Value::as_bool) == Some(true) && outbox_empty => {
      Some(sv)
    }
    _ => None,
  }
}

/// Capped exponential reconnect backoff in milliseconds:
/// `500, 1000, 2000, 4000, 8000, 16000, 30000, 30000, ...`
pub fn reconnect_backoff_ms(attempt: u32) -> u64 {
  let shift = attempt.min(RECONNECT_MAX_SHIFT);
  (RECONNECT_BASE_MS << shift).clamp(RECONNECT_BASE_MS, RECONNECT_MAX_MS)
}

/// Exponential backoff delay for retrying a failed local diff append:
/// `400, 800, 1600, 3200, 6400` ms for attempts `0..5`.
pub fn local_update_retry_delay_ms(attempt: u32) -> u64 {
  LOCAL_UPDATE_RETRY_BASE_MS << attempt.min(16)
}

/// True when `bytes` carries no CRDT operations (empty or `[0, 0]`).
pub fn is_empty_yrs_update(bytes: &[u8]) -> bool {
  bytes.is_empty() || bytes == EMPTY_YRS_UPDATE
}

/// Merge multiple v1-encoded yrs updates into one v1 update.
///
/// Returns `Err` if any update is corrupt/undecodable (so callers can fall back
/// to individual pushes and isolate the poison entry).
pub fn merge_yrs_updates(updates: &[&[u8]]) -> Result<Vec<u8>, String> {
  if updates.is_empty() {
    return Ok(EMPTY_YRS_UPDATE.to_vec());
  }
  let work = || yrs::merge_updates_v1(updates.iter().copied()).map_err(|e| e.to_string());

  #[cfg(not(target_arch = "wasm32"))]
  {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
      Ok(res) => res,
      Err(_) => Err("yrs panicked while merging updates".into()),
    }
  }
  #[cfg(target_arch = "wasm32")]
  {
    work()
  }
}

// ── Outbound JSON wire frame builders ────────────────────────────────────────

/// Build `sync.bootstrap` (full base when `sv` is `None`, delta catch-up when `Some`).
pub fn frame_sync_bootstrap(sv: Option<&[u8]>) -> String {
  match sv {
    Some(bytes) => json!({
      "type": "sync.bootstrap",
      "payload": { "sv": STANDARD.encode(bytes) },
    })
    .to_string(),
    None => json!({ "type": "sync.bootstrap" }).to_string(),
  }
}

/// Build `sync.pull` carrying `since_rid` and optional state vector `sv`.
pub fn frame_sync_pull(since_rid: i64, sv: Option<&[u8]>) -> String {
  match sv {
    Some(bytes) => json!({
      "type": "sync.pull",
      "payload": {
        "since_rid": since_rid,
        "sv": STANDARD.encode(bytes),
      },
    })
    .to_string(),
    None => json!({
      "type": "sync.pull",
      "payload": {
        "since_rid": since_rid,
      },
    })
    .to_string(),
  }
}

/// Build `sync.push` carrying opaque client `id` and base64-encoded yrs update.
pub fn frame_sync_push(id: &str, update: &[u8]) -> String {
  json!({
    "type": "sync.push",
    "id": id,
    "payload": {
      "update": STANDARD.encode(update),
    },
  })
  .to_string()
}

/// Build `ping` heartbeat frame.
pub fn frame_ping() -> String {
  json!({ "type": "ping" }).to_string()
}

/// Build `presence.update` carrying display name and optional caret position.
pub fn frame_presence_update(name: &str, cursor: Option<(&str, i64)>) -> String {
  match cursor {
    Some((block, offset)) => json!({
      "type": "presence.update",
      "payload": {
        "name": name,
        "cursor": {
          "block": block,
          "offset": offset,
        },
      },
    })
    .to_string(),
    None => json!({
      "type": "presence.update",
      "payload": {
        "name": name,
      },
    })
    .to_string(),
  }
}
