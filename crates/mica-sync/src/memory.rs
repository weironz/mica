//! In-memory hydrated [`SyncStorage`] with snapshotted write-behind mirror
//! transactions ([`IdbMirrorOp`]) for Web IndexedDB and deterministic tests.
//!
//! Mirrors the invariants of `WebIdbDocStore`:
//! - **Snapshot-at-enqueue (#161)**: every mutation clones its target records
//!   into an immutable [`IdbMirrorOp`] immediately, so later mutations (such as
//!   `compact`) can never tear an earlier queued transaction.
//! - **Freeze-on-broken**: once [`MemoryMirrorStorage::mark_broken`] is called,
//!   pending mirror ops are dropped and incremental appends fail with
//!   [`StorageError::Unavailable`] until [`SyncStorage::save_base`] self-heals
//!   with a fresh authoritative snapshot.

use mica_core::MicaDoc;
use serde::{Deserialize, Serialize};

use crate::storage::{
  IdbMirrorOp, LogSizes, OutboxEntry, RemoteEntry, StorageError, StoredDoc, SyncCursor, SyncStorage,
};

/// Initial hydrated state read once from IndexedDB on document open.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdbHydrateSnapshot {
  pub base: Option<Vec<u8>>,
  pub outbox: Vec<OutboxEntry>,
  pub remote: Vec<RemoteEntry>,
  pub cursor: SyncCursor,
}

/// Hydrated in-memory storage backend emitting immutable [`IdbMirrorOp`]
/// snapshots for Web IndexedDB write-behind persistence.
#[derive(Debug, Clone, Default)]
pub struct MemoryMirrorStorage {
  base: Option<Vec<u8>>,
  outbox: Vec<OutboxEntry>,
  remote: Vec<RemoteEntry>,
  last_synced_rid: i64,
  pushed_clock: i64,
  broken: bool,
  pending_txns: Vec<IdbMirrorOp>,
}

impl MemoryMirrorStorage {
  /// Create a fresh, empty storage mirror.
  pub fn new() -> Self {
    Self::default()
  }

  /// Hydrate from an initial IndexedDB snapshot (sorting logs by clock/rid).
  pub fn hydrate(mut snapshot: IdbHydrateSnapshot) -> Self {
    snapshot.outbox.sort_by_key(|e| e.clock);
    snapshot.remote.sort_by_key(|e| e.rid);
    Self {
      base: snapshot.base,
      outbox: snapshot.outbox,
      remote: snapshot.remote,
      last_synced_rid: snapshot.cursor.last_synced_rid,
      pushed_clock: snapshot.cursor.pushed_clock,
      broken: false,
      pending_txns: Vec::new(),
    }
  }

  /// Export the current in-memory state as an [`IdbHydrateSnapshot`] (useful
  /// for simulating a tab reload in tests).
  pub fn snapshot(&self) -> IdbHydrateSnapshot {
    IdbHydrateSnapshot {
      base: self.base.clone(),
      outbox: self.outbox.clone(),
      remote: self.remote.clone(),
      cursor: self.cursor_value(),
    }
  }

  fn cursor_value(&self) -> SyncCursor {
    SyncCursor {
      last_synced_rid: self.last_synced_rid,
      pushed_clock: self.pushed_clock,
    }
  }

  fn ensure_healthy(&self) -> Result<(), StorageError> {
    if self.broken {
      return Err(StorageError::Unavailable(
        "memory mirror storage is broken (frozen after mirror write failure)".into(),
      ));
    }
    Ok(())
  }

  /// Replay `base + remote + outbox` through `MicaDoc`. Returns `Ok(None)` when
  /// no base is stored, or `Err(StorageError::Corrupt(_))` when any blob fails
  /// to decode or integrate.
  fn replayed_state(&self) -> Result<Option<Vec<u8>>, StorageError> {
    let Some(base) = self.base.as_deref() else {
      return Ok(None);
    };
    let remote = &self.remote;
    let outbox = &self.outbox;
    let folded = replay_yrs_state(base, remote, outbox)?;
    Ok(Some(folded))
  }
}

/// Replay `base + remote + outbox` into a single encoded yrs state, containing
/// panics on native targets just like `LocalStore::contain_yrs_panic`.
fn replay_yrs_state(
  base: &[u8],
  remote: &[RemoteEntry],
  outbox: &[OutboxEntry],
) -> Result<Vec<u8>, StorageError> {
  let work = || -> Result<Vec<u8>, StorageError> {
    let mut doc = MicaDoc::from_update(base).map_err(|e| StorageError::Corrupt(e.to_string()))?;
    for entry in remote {
      doc
        .apply_update(&entry.bytes)
        .map_err(|e| StorageError::Corrupt(e.to_string()))?;
    }
    for entry in outbox {
      doc
        .apply_update(&entry.bytes)
        .map_err(|e| StorageError::Corrupt(e.to_string()))?;
    }
    Ok(doc.encode_state())
  };

  #[cfg(not(target_arch = "wasm32"))]
  {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
      Ok(res) => res,
      Err(_) => Err(StorageError::Corrupt(
        "yrs panicked while replaying stored mirror state".into(),
      )),
    }
  }
  #[cfg(target_arch = "wasm32")]
  {
    work()
  }
}

impl SyncStorage for MemoryMirrorStorage {
  fn load(&self) -> Result<Option<StoredDoc>, StorageError> {
    let Some(state) = self.replayed_state()? else {
      return Ok(None);
    };
    Ok(Some(StoredDoc {
      state,
      cursor: self.last_synced_rid,
    }))
  }

  fn save_base(&mut self, state: &[u8], cursor: i64) -> Result<(), StorageError> {
    self.base = Some(state.to_vec());
    if cursor > self.last_synced_rid {
      self.last_synced_rid = cursor;
    }
    // Self-heal clears the frozen mirror state and queues a fresh authoritative
    // base snapshot transaction.
    self.broken = false;
    self.pending_txns.push(IdbMirrorOp::SaveBase {
      state: state.to_vec(),
      cursor: self.cursor_value(),
    });
    Ok(())
  }

  fn append_outbox(&mut self, diff: &[u8]) -> Result<i64, StorageError> {
    self.ensure_healthy()?;
    // Strictly monotonic past `pushed_clock` across trims (P2a invariant).
    let max_clock = self.outbox.last().map_or(0, |e| e.clock);
    let clock = max_clock.max(self.pushed_clock) + 1;
    self.outbox.push(OutboxEntry {
      clock,
      bytes: diff.to_vec(),
    });
    // Snapshot at enqueue (#161).
    self.pending_txns.push(IdbMirrorOp::AppendOutbox {
      rows: self.outbox.clone(),
    });
    Ok(clock)
  }

  fn outbox_after(&self, pushed_clock: i64) -> Result<Vec<OutboxEntry>, StorageError> {
    Ok(
      self
        .outbox
        .iter()
        .filter(|e| e.clock > pushed_clock)
        .cloned()
        .collect(),
    )
  }

  fn cursor(&self) -> Result<SyncCursor, StorageError> {
    Ok(self.cursor_value())
  }

  fn advance_cursor(
    &mut self,
    last_synced_rid: Option<i64>,
    pushed_clock: Option<i64>,
  ) -> Result<(), StorageError> {
    self.ensure_healthy()?;
    if let Some(rid) = last_synced_rid {
      self.last_synced_rid = rid;
    }
    if let Some(clock) = pushed_clock {
      self.pushed_clock = clock;
    }
    // Snapshot at enqueue (#161).
    self.pending_txns.push(IdbMirrorOp::AdvanceCursor {
      cursor: self.cursor_value(),
    });
    Ok(())
  }

  fn trim_outbox_through(&mut self, pushed_clock: i64) -> Result<(), StorageError> {
    self.ensure_healthy()?;
    // Clamp to persisted `pushed_clock` so un-pushed entries are never dropped.
    let up_to = pushed_clock.min(self.pushed_clock);
    self.outbox.retain(|e| e.clock > up_to);
    self.pending_txns.push(IdbMirrorOp::AppendOutbox {
      rows: self.outbox.clone(),
    });
    Ok(())
  }

  fn append_remote_batch(&mut self, items: &[(i64, Vec<u8>)]) -> Result<(), StorageError> {
    self.ensure_healthy()?;
    if items.is_empty() {
      return Ok(());
    }
    for (rid, bytes) in items {
      if !self.remote.iter().any(|e| e.rid == *rid) {
        self.remote.push(RemoteEntry {
          rid: *rid,
          bytes: bytes.clone(),
        });
      }
      if *rid > self.last_synced_rid {
        self.last_synced_rid = *rid;
      }
    }
    self.remote.sort_by_key(|e| e.rid);
    // Snapshot at enqueue (#161): carries both rows and cursor atomically.
    self.pending_txns.push(IdbMirrorOp::AppendRemote {
      rows: self.remote.clone(),
      cursor: self.cursor_value(),
    });
    Ok(())
  }

  fn log_sizes(&self) -> Result<LogSizes, StorageError> {
    Ok(LogSizes {
      local: self.outbox.len(),
      remote: self.remote.len(),
    })
  }

  fn compact(&mut self) -> Result<(), StorageError> {
    self.ensure_healthy()?;
    let Some(folded) = self.replayed_state()? else {
      return Ok(());
    };
    self.base = Some(folded.clone());
    self.remote.clear();
    // Preserve un-pushed outbox tail (`clock > pushed_clock`).
    let pushed = self.pushed_clock;
    self.outbox.retain(|e| e.clock > pushed);
    // Snapshot at enqueue (#161).
    self.pending_txns.push(IdbMirrorOp::Compact {
      state: folded,
      outbox: self.outbox.clone(),
      remote: Vec::new(),
      cursor: self.cursor_value(),
    });
    Ok(())
  }

  fn is_broken(&self) -> bool {
    self.broken
  }

  fn mark_broken(&mut self) {
    self.broken = true;
    // Freeze consistently: drop any un-dispatched mirror transactions so a
    // later write cannot commit a cursor ahead of rolled-back data.
    self.pending_txns.clear();
  }

  fn take_pending_mirror_txns(&mut self) -> Vec<IdbMirrorOp> {
    std::mem::take(&mut self.pending_txns)
  }
}
