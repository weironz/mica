//! Mica L2 headless Sans-I/O synchronization engine and unified local storage.
//!
//! Pure Rust with zero UI, browser, or network I/O dependencies.
//!
//! - [`storage`]: [`SyncStorage`] trait, cursor/outbox/remote DTOs, [`StorageError`],
//!   and snapshotted [`IdbMirrorOp`] transactions.
//! - [`memory`]: [`MemoryMirrorStorage`] for Web IndexedDB mirroring and tests.
//! - [`sqlite`]: [`SqliteSyncStorage`] (behind `feature = "sqlite"`) over
//!   `mica_core::LocalStore`.
//! - [`protocol`]: WebSocket protocol constants, framing, and pure pacing helpers.
//! - [`engine`]: Sans-I/O [`SyncEngine`] state machine (`SyncEvent` → `Vec<SyncEffect>`).

pub mod engine;
pub mod memory;
pub mod protocol;
#[cfg(feature = "sqlite")]
pub mod sqlite;
pub mod storage;

pub use engine::{SyncConfig, SyncEffect, SyncEngine, SyncEvent};
pub use memory::{IdbHydrateSnapshot, MemoryMirrorStorage};
pub use protocol::{
  COMPACT_CHECK_EVERY, COMPACT_THRESHOLD, EMPTY_YRS_UPDATE, LAG_RESYNC_DEBOUNCE_MS,
  LOCAL_UPDATE_MAX_RETRIES, LOCAL_UPDATE_RETRY_BASE_MS, MAX_AUTO_REHEAL, MAX_PUSH_REJECTS,
  MAX_REPAIRS, MERGE_MAX_BYTES, MERGE_MIN_RUN, PING_EVERY_MS, PUSH_WINDOW, PresenceUser,
  RECONNECT_BASE_MS, RECONNECT_MAX_MS, RECONNECT_MAX_SHIFT, STALE_MS, SyncPhase, TimerKind,
  derive_sync_phase, frame_ping, frame_presence_update, frame_sync_bootstrap, frame_sync_pull,
  frame_sync_push, is_empty_yrs_update, local_update_retry_delay_ms, merge_run_length,
  merge_yrs_updates, push_slice, reconnect_backoff_ms, repair_sv_for,
};
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteSyncStorage;
pub use storage::{
  IdbMirrorOp, LogSizes, OutboxEntry, RemoteEntry, StorageError, StoredDoc, SyncCursor, SyncStorage,
};
