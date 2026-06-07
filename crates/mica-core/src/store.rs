//! On-device local store + identity (P2-M2), SQLite via rusqlite.
//!
//! Feature-gated `store` — desktop client only; the server never pulls SQLite in.
//! M2 persists each document as a full base snapshot (a yrs v1 update). The
//! incremental update queue + squash + sync cursor (§4/§5) arrive with
//! incremental saves and cloud sync (P2-M4).

use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use crate::doc::{DocError, MicaDoc};

#[derive(thiserror::Error, Debug)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Doc(#[from] DocError),
}

/// Stable on-device identity. `client_id` is the yrs actor id used for every doc
/// on this device — minted once and reused forever, never random per launch (§6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub device_id: String,
    pub client_id: u64,
}

/// A local document store backed by one SQLite file.
pub struct LocalStore {
    conn: Connection,
}

impl LocalStore {
    /// Open (creating if needed) the store at `path` and ensure the schema.
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS doc_snapshot(
                 doc_id     TEXT PRIMARY KEY,
                 state      BLOB NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS local_meta(
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;
        Ok(LocalStore { conn })
    }

    /// An in-memory store (tests).
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::open(":memory:")
    }

    fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM local_meta WHERE key=?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn meta_set(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO local_meta(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// The persisted identity, minting + storing one on first use.
    pub fn identity(&self) -> Result<Identity, StoreError> {
        if let (Some(device_id), Some(cid)) =
            (self.meta_get("device_id")?, self.meta_get("client_id")?)
        {
            if let Ok(client_id) = cid.parse::<u64>() {
                return Ok(Identity {
                    device_id,
                    client_id,
                });
            }
        }
        let device_id = uuid::Uuid::new_v4().to_string();
        let raw = uuid::Uuid::new_v4();
        // yrs ClientID is 53-bit (Yjs-compatible) — mask the random u64 to fit so
        // it survives ClientID::new unchanged.
        let mut client_id =
            u64::from_le_bytes(raw.as_bytes()[0..8].try_into().unwrap()) & ((1u64 << 53) - 1);
        if client_id == 0 {
            client_id = 1;
        }
        self.meta_set("device_id", &device_id)?;
        self.meta_set("client_id", &client_id.to_string())?;
        Ok(Identity {
            device_id,
            client_id,
        })
    }

    /// Persist a document as its full snapshot (upsert).
    pub fn save_doc(&self, doc_id: &str, doc: &MicaDoc) -> Result<(), StoreError> {
        let state = doc.encode_state();
        self.conn.execute(
            "INSERT INTO doc_snapshot(doc_id,state,updated_at) VALUES(?1,?2,?3)
             ON CONFLICT(doc_id) DO UPDATE SET state=excluded.state, updated_at=excluded.updated_at",
            params![doc_id, state, now_millis()],
        )?;
        Ok(())
    }

    /// Load a document by id, decoding it with `client_id` (the device's actor).
    /// Returns `None` if there's no such document.
    pub fn load_doc(&self, doc_id: &str, client_id: u64) -> Result<Option<MicaDoc>, StoreError> {
        let state: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT state FROM doc_snapshot WHERE doc_id=?1",
                params![doc_id],
                |r| r.get(0),
            )
            .optional()?;
        match state {
            Some(bytes) => Ok(Some(MicaDoc::from_update_with_client_id(
                &bytes,
                Some(client_id),
            )?)),
            None => Ok(None),
        }
    }

    /// All stored document ids (sorted).
    pub fn list_docs(&self) -> Result<Vec<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT doc_id FROM doc_snapshot ORDER BY doc_id")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Delete a document.
    pub fn delete_doc(&self, doc_id: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM doc_snapshot WHERE doc_id=?1", params![doc_id])?;
        Ok(())
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Block;

    fn sample() -> (String, Vec<Block>) {
        (
            "r".to_string(),
            vec![
                Block::new("r", "page").with_children(vec!["a".into()]),
                Block::new("a", "paragraph").with_text("Hello"),
            ],
        )
    }

    #[test]
    fn save_and_load_round_trip() {
        let store = LocalStore::open_in_memory().unwrap();
        let id = store.identity().unwrap();
        let (root, blocks) = sample();
        let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(id.client_id));
        store.save_doc("doc1", &doc).unwrap();

        let loaded = store.load_doc("doc1", id.client_id).unwrap().unwrap();
        assert_eq!(loaded.to_blocks(), doc.to_blocks());
        assert_eq!(loaded.client_id(), id.client_id);
    }

    #[test]
    fn edits_persist_after_resave() {
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();
        let mut doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
        store.save_doc("d", &doc).unwrap();

        doc.text_insert("a", 5, " world");
        store.save_doc("d", &doc).unwrap();

        let loaded = store.load_doc("d", cid).unwrap().unwrap();
        let a = loaded.to_blocks().into_iter().find(|b| b.id == "a").unwrap();
        assert_eq!(a.text, "Hello world");
    }

    #[test]
    fn list_and_delete() {
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();
        let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
        store.save_doc("a", &doc).unwrap();
        store.save_doc("b", &doc).unwrap();
        assert_eq!(store.list_docs().unwrap(), vec!["a", "b"]);
        store.delete_doc("a").unwrap();
        assert_eq!(store.list_docs().unwrap(), vec!["b"]);
        assert!(store.load_doc("a", cid).unwrap().is_none());
    }

    #[test]
    fn identity_is_stable_across_reopen() {
        // A real file so identity must survive a fresh connection.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("mica_store_test_{}.db", std::process::id()));
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&p);

        let first = {
            let s = LocalStore::open(&p).unwrap();
            s.identity().unwrap()
        };
        let second = {
            let s = LocalStore::open(&p).unwrap();
            s.identity().unwrap()
        };
        assert_eq!(first, second, "device id + client id persist across reopen");
        assert_ne!(first.client_id, 0);

        let _ = std::fs::remove_file(&p);
    }
}
