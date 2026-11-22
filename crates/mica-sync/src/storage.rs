//! Unified synchronous local storage abstraction for the Sans-I/O sync engine.
//!
//! Both desktop (`SqliteSyncStorage` over `mica_core::LocalStore`) and web
//! (`MemoryMirrorStorage` paired with host-side IndexedDB mirror transactions)
//! implement [`SyncStorage`], so [`crate::SyncEngine`] runs one identical state
//! machine on every platform.

use serde::{Deserialize, Serialize};

/// A document's sync high-water marks against the cloud.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SyncCursor {
  /// Highest cloud stream id (`rid`) this replica has applied and persisted.
  pub last_synced_rid: i64,
  /// Highest local outbox `clock` contiguously acked by the server.
  pub pushed_clock: i64,
}

/// One durable un-pushed (or not-yet-trimmed) local CRDT diff in the outbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEntry {
  pub clock: i64,
  pub bytes: Vec<u8>,
}

/// One persisted remote stream update keyed by server `rid`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteEntry {
  pub rid: i64,
  pub bytes: Vec<u8>,
}

/// Persisted document state replayed from base + remote log + local outbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredDoc {
  pub state: Vec<u8>,
  pub cursor: i64,
}

/// Row counts of the local outbox and remote update logs (compaction trigger).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LogSizes {
  pub local: usize,
  pub remote: usize,
}

/// Storage failures distinguished by recoverability.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
  /// Stored CRDT bytes failed checksum verification or yrs decoding/integration.
  /// Recoverable: the cloud copy is authoritative, so the engine cold-bootstraps.
  #[error("corrupt local storage: {0}")]
  Corrupt(String),
  /// Storage is temporarily or permanently unavailable (e.g. broken IDB mirror,
  /// poisoned lock, unsupported schema version).
  #[error("storage unavailable: {0}")]
  Unavailable(String),
  /// Underlying I/O or SQLite error.
  #[error("storage io error: {0}")]
  Io(String),
}

/// Immutable, snapshotted IndexedDB write-behind transaction produced at enqueue
/// time (issue #161).
///
/// Every variant carries its own owned snapshot of the records to commit in one
/// atomic IDB `readwrite` transaction, eliminating torn reads when a later
/// mutation (such as `compact` or `advance_cursor`) mutates in-memory state
/// before an earlier async IDB transaction runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum IdbMirrorOp {
  /// Atomically write `base` + `cursor`.
  SaveBase {
    state: Vec<u8>,
    cursor: SyncCursor,
  },
  /// Atomically write the full `outbox` row list snapshot.
  AppendOutbox {
    rows: Vec<OutboxEntry>,
  },
  /// Atomically write the full `remote` row list + `cursor` snapshot.
  AppendRemote {
    rows: Vec<RemoteEntry>,
    cursor: SyncCursor,
  },
  /// Atomically write the `cursor` snapshot.
  AdvanceCursor {
    cursor: SyncCursor,
  },
  /// Atomically write the folded `base`, cleared `remote`, trimmed `outbox`,
  /// and `cursor`.
  Compact {
    state: Vec<u8>,
    outbox: Vec<OutboxEntry>,
    remote: Vec<RemoteEntry>,
    cursor: SyncCursor,
  },
}

/// Unified synchronous persistence interface for one cloud-synced document.
pub trait SyncStorage: Send {
  /// Load and replay the persisted base + remote log + local outbox.
  ///
  /// Returns `Ok(None)` when the document has never been mirrored locally,
  /// `Err(StorageError::Corrupt(_))` when stored bytes are present but corrupt,
  /// and `Err(StorageError::Io(_))` / `Err(StorageError::Unavailable(_))` on
  /// genuine storage faults.
  fn load(&self) -> Result<Option<StoredDoc>, StorageError>;

  /// Persist a full base snapshot alongside its `last_synced_rid` cursor,
  /// preserving `pushed_clock`.
  fn save_base(&mut self, state: &[u8], cursor: i64) -> Result<(), StorageError>;

  /// Append a local CRDT diff to the durable outbox, returning its strictly
  /// monotonic `clock` (always `> max(existing_clock, pushed_clock)`).
  fn append_outbox(&mut self, diff: &[u8]) -> Result<i64, StorageError>;

  /// Return all outbox entries with `clock > pushed_clock`, ordered by `clock`.
  fn outbox_after(&self, pushed_clock: i64) -> Result<Vec<OutboxEntry>, StorageError>;

  /// Return the persisted sync cursor (`(0, 0)` if unset).
  fn cursor(&self) -> Result<SyncCursor, StorageError>;

  /// Advance either or both cursor fields; `None` leaves that field untouched.
  fn advance_cursor(
    &mut self,
    last_synced_rid: Option<i64>,
    pushed_clock: Option<i64>,
  ) -> Result<(), StorageError>;

  /// Drop acked outbox entries with `clock <= min(pushed_clock, cursor.pushed_clock)`.
  fn trim_outbox_through(&mut self, pushed_clock: i64) -> Result<(), StorageError>;

  /// Durably append a batch of remote updates and advance `last_synced_rid` in
  /// a single atomic transaction. Idempotent per `rid`.
  fn append_remote_batch(&mut self, items: &[(i64, Vec<u8>)]) -> Result<(), StorageError>;

  /// Convenience single-row remote append.
  fn append_remote(&mut self, rid: i64, update: &[u8]) -> Result<(), StorageError> {
    self.append_remote_batch(&[(rid, update.to_vec())])
  }

  /// Return `(local_outbox_rows, remote_log_rows)`.
  fn log_sizes(&self) -> Result<LogSizes, StorageError>;

  /// Fold base + remote log + local outbox into a fresh base, clear the remote
  /// log, and trim only the acked outbox prefix (`clock <= pushed_clock`).
  /// Safe no-op when no base is stored.
  fn compact(&mut self) -> Result<(), StorageError>;

  /// True when the write-behind mirror is frozen after an async write failure.
  fn is_broken(&self) -> bool {
    false
  }

  /// Freeze the write-behind mirror after an external async write failure.
  fn mark_broken(&mut self) {}

  /// Drain all snapshotted IDB mirror transactions queued since the last drain.
  fn take_pending_mirror_txns(&mut self) -> Vec<IdbMirrorOp> {
    Vec::new()
  }
}
