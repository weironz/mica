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

/// A page-tree node (P2-M3): the local mirror of the client's `DocumentView`.
/// The local workspace is implicit and single, so there's no workspace id. Each
/// view points at one document (`object_id` = its `doc_id` in `doc_snapshot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalView {
    pub id: String,
    pub parent_id: Option<String>,
    pub object_id: String,
    pub name: String,
    /// Zero-padded sibling ordering (same convention as the cloud `position`).
    pub position: String,
    /// Soft-deleted into the trash; purge removes the row.
    pub trashed: bool,
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
             );
             CREATE TABLE IF NOT EXISTS local_view(
                 id        TEXT PRIMARY KEY,
                 parent_id TEXT,
                 object_id TEXT NOT NULL,
                 name      TEXT NOT NULL,
                 position  TEXT NOT NULL,
                 trashed   INTEGER NOT NULL DEFAULT 0
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

    // ── page tree (views) — P2-M3 ────────────────────────────────────────────

    /// All views (including trashed), ordered by `position`. The client builds
    /// the tree from `parent_id` and filters trash itself.
    pub fn list_views(&self) -> Result<Vec<LocalView>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id,parent_id,object_id,name,position,trashed FROM local_view ORDER BY position",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(LocalView {
                    id: r.get(0)?,
                    parent_id: r.get(1)?,
                    object_id: r.get(2)?,
                    name: r.get(3)?,
                    position: r.get(4)?,
                    trashed: r.get::<_, i64>(5)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Upsert a view — covers create, rename, move (parent/position), and trash
    /// toggling, all by writing the desired row.
    pub fn save_view(&self, v: &LocalView) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO local_view(id,parent_id,object_id,name,position,trashed)
             VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(id) DO UPDATE SET
                 parent_id=excluded.parent_id, object_id=excluded.object_id,
                 name=excluded.name, position=excluded.position, trashed=excluded.trashed",
            params![
                v.id,
                v.parent_id,
                v.object_id,
                v.name,
                v.position,
                v.trashed as i64
            ],
        )?;
        Ok(())
    }

    /// Permanently remove a view row (the document is deleted separately via
    /// [`Self::delete_doc`]).
    pub fn purge_view(&self, id: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM local_view WHERE id=?1", params![id])?;
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

    fn view(id: &str, parent: Option<&str>, name: &str, pos: &str) -> LocalView {
        LocalView {
            id: id.into(),
            parent_id: parent.map(|s| s.into()),
            object_id: format!("doc-{id}"),
            name: name.into(),
            position: pos.into(),
            trashed: false,
        }
    }

    #[test]
    fn views_crud_and_tree_fields() {
        let store = LocalStore::open_in_memory().unwrap();
        store.save_view(&view("v1", None, "Page 1", "0000000010")).unwrap();
        store.save_view(&view("v2", None, "Page 2", "0000000020")).unwrap();
        store.save_view(&view("v3", Some("v1"), "Child", "0000000010")).unwrap();

        let all = store.list_views().unwrap();
        assert_eq!(all.len(), 3);
        // ordered by position; v3 (child, pos 10) and v1 (pos 10) share pos but
        // the child carries its parent.
        let v3 = all.iter().find(|v| v.id == "v3").unwrap();
        assert_eq!(v3.parent_id.as_deref(), Some("v1"));
        assert_eq!(v3.object_id, "doc-v3");

        // Rename + move (upsert same id).
        store.save_view(&LocalView { name: "Renamed".into(), ..view("v2", None, "x", "0000000005") }).unwrap();
        let v2 = store.list_views().unwrap().into_iter().find(|v| v.id == "v2").unwrap();
        assert_eq!(v2.name, "Renamed");
        assert_eq!(v2.position, "0000000005");

        // Trash then purge.
        store.save_view(&LocalView { trashed: true, ..view("v1", None, "Page 1", "0000000010") }).unwrap();
        assert!(store.list_views().unwrap().iter().find(|v| v.id == "v1").unwrap().trashed);
        store.purge_view("v1").unwrap();
        assert!(store.list_views().unwrap().iter().all(|v| v.id != "v1"));
    }

    #[test]
    fn views_survive_reopen() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("mica_views_test_{}.db", std::process::id()));
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&p);
        {
            let s = LocalStore::open(&p).unwrap();
            s.save_view(&view("v1", None, "Persisted", "0000000010")).unwrap();
        }
        {
            let s = LocalStore::open(&p).unwrap();
            let all = s.list_views().unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].name, "Persisted");
        }
        let _ = std::fs::remove_file(&p);
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
