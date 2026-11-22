//! SQLite-backed [`SyncStorage`] adapter over [`mica_core::LocalStore`].
//!
//! Enabled behind the `sqlite` Cargo feature (desktop/native only; never pulled
//! into `wasm32`). Preserves origin-scoped multi-account isolation (`(origin,
//! doc_id)`) and distinguishes absent (`Ok(None)`) from corrupt local blobs
//! (`Err(StorageError::Corrupt(_))`) and genuine SQLite/I/O failures
//! (`Err(StorageError::Io(_))`).

use std::sync::{Arc, Mutex, MutexGuard};

use mica_core::{LocalStore, MicaDoc, StoreError};

use crate::storage::{LogSizes, OutboxEntry, StorageError, StoredDoc, SyncCursor, SyncStorage};

/// Desktop SQLite storage adapter for one `(origin, doc_id)` cloud mirror.
pub struct SqliteSyncStorage {
  store: Arc<Mutex<LocalStore>>,
  origin: String,
  doc_id: String,
  client_id: u64,
}

impl SqliteSyncStorage {
  /// Wrap a shared `Arc<Mutex<LocalStore>>` for `(origin, doc_id)`.
  pub fn new(
    store: Arc<Mutex<LocalStore>>,
    origin: impl Into<String>,
    doc_id: impl Into<String>,
    client_id: u64,
  ) -> Self {
    Self {
      store,
      origin: origin.into(),
      doc_id: doc_id.into(),
      client_id,
    }
  }

  /// Convenience constructor owning a standalone [`LocalStore`] (tests / CLI).
  pub fn from_store(
    store: LocalStore,
    origin: impl Into<String>,
    doc_id: impl Into<String>,
    client_id: u64,
  ) -> Self {
    Self::new(Arc::new(Mutex::new(store)), origin, doc_id, client_id)
  }

  /// Access the underlying shared [`LocalStore`] handle.
  pub fn store(&self) -> &Arc<Mutex<LocalStore>> {
    &self.store
  }

  fn lock_store(&self) -> Result<MutexGuard<'_, LocalStore>, StorageError> {
    self
      .store
      .lock()
      .map_err(|_| StorageError::Unavailable("local sqlite store mutex poisoned".into()))
  }
}

fn map_store_error(err: StoreError) -> StorageError {
  match err {
    StoreError::CorruptDoc { .. } => StorageError::Corrupt(err.to_string()),
    StoreError::Doc(e) => StorageError::Corrupt(e.to_string()),
    StoreError::SchemaTooNew { .. } => StorageError::Unavailable(err.to_string()),
    StoreError::Sqlite(e) => StorageError::Io(e.to_string()),
    StoreError::InvalidParent { .. } => StorageError::Io(err.to_string()),
  }
}

impl SyncStorage for SqliteSyncStorage {
  fn load(&self) -> Result<Option<StoredDoc>, StorageError> {
    let store = self.lock_store()?;
    let doc = store
      .load_doc(&self.origin, &self.doc_id, self.client_id)
      .map_err(map_store_error)?;
    let Some(doc) = doc else {
      return Ok(None);
    };
    let cursor = store
      .sync_cursor(&self.origin, &self.doc_id)
      .map_err(map_store_error)?
      .last_synced_rid;
    Ok(Some(StoredDoc {
      state: doc.encode_state(),
      cursor,
    }))
  }

  fn save_base(&mut self, state: &[u8], cursor: i64) -> Result<(), StorageError> {
    let doc = MicaDoc::from_update_with_client_id(state, Some(self.client_id))
      .map_err(|e| StorageError::Corrupt(e.to_string()))?;
    let store = self.lock_store()?;
    store
      .save_doc(&self.origin, &self.doc_id, &doc)
      .map_err(map_store_error)?;
    // Preserve `pushed_clock` (the outbox high-water); only update `last_synced_rid`.
    let current = store
      .sync_cursor(&self.origin, &self.doc_id)
      .map_err(map_store_error)?;
    store
      .set_sync_cursor(
        &self.origin,
        &self.doc_id,
        mica_core::SyncCursor {
          last_synced_rid: cursor,
          pushed_clock: current.pushed_clock,
        },
      )
      .map_err(map_store_error)?;
    Ok(())
  }

  fn append_outbox(&mut self, diff: &[u8]) -> Result<i64, StorageError> {
    let store = self.lock_store()?;
    let clock = store
      .append_update(&self.origin, &self.doc_id, diff)
      .map_err(map_store_error)?;
    if clock <= 0 {
      return Err(StorageError::Io(format!(
        "outbox append returned non-positive clock {clock} for doc {}",
        self.doc_id
      )));
    }
    Ok(clock)
  }

  fn outbox_after(&self, pushed_clock: i64) -> Result<Vec<OutboxEntry>, StorageError> {
    let store = self.lock_store()?;
    let rows = store
      .updates_after(&self.origin, &self.doc_id, pushed_clock)
      .map_err(map_store_error)?;
    Ok(
      rows
        .into_iter()
        .map(|(clock, bytes)| OutboxEntry { clock, bytes })
        .collect(),
    )
  }

  fn cursor(&self) -> Result<SyncCursor, StorageError> {
    let store = self.lock_store()?;
    let c = store
      .sync_cursor(&self.origin, &self.doc_id)
      .map_err(map_store_error)?;
    Ok(SyncCursor {
      last_synced_rid: c.last_synced_rid,
      pushed_clock: c.pushed_clock,
    })
  }

  fn advance_cursor(
    &mut self,
    last_synced_rid: Option<i64>,
    pushed_clock: Option<i64>,
  ) -> Result<(), StorageError> {
    let store = self.lock_store()?;
    let current = store
      .sync_cursor(&self.origin, &self.doc_id)
      .map_err(map_store_error)?;
    store
      .set_sync_cursor(
        &self.origin,
        &self.doc_id,
        mica_core::SyncCursor {
          last_synced_rid: last_synced_rid.unwrap_or(current.last_synced_rid),
          pushed_clock: pushed_clock.unwrap_or(current.pushed_clock),
        },
      )
      .map_err(map_store_error)?;
    Ok(())
  }

  fn trim_outbox_through(&mut self, pushed_clock: i64) -> Result<(), StorageError> {
    let store = self.lock_store()?;
    store
      .trim_updates_through(&self.origin, &self.doc_id, pushed_clock)
      .map_err(map_store_error)?;
    Ok(())
  }

  fn append_remote_batch(&mut self, items: &[(i64, Vec<u8>)]) -> Result<(), StorageError> {
    if items.is_empty() {
      return Ok(());
    }
    let store = self.lock_store()?;
    store
      .append_remote_updates(&self.origin, &self.doc_id, items)
      .map_err(map_store_error)?;
    Ok(())
  }

  fn log_sizes(&self) -> Result<LogSizes, StorageError> {
    let store = self.lock_store()?;
    let (local, remote) = store
      .log_sizes(&self.origin, &self.doc_id)
      .map_err(map_store_error)?;
    Ok(LogSizes {
      local: local.max(0) as usize,
      remote: remote.max(0) as usize,
    })
  }

  fn compact(&mut self) -> Result<(), StorageError> {
    let store = self.lock_store()?;
    store
      .squash(&self.origin, &self.doc_id, self.client_id)
      .map_err(map_store_error)?;
    Ok(())
  }
}
