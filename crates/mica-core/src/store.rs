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

/// A document's sync high-water marks against the cloud (P2-M4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncCursor {
    /// Highest cloud stream id (`rid`) this device has pulled and applied.
    pub last_synced_rid: i64,
    /// Highest local update `clock` this device has pushed to the cloud.
    pub pushed_clock: i64,
}

/// A local workspace (P2-M3): a named container for a page tree. The on-device
/// store can hold several, mirroring the cloud workspace list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalWorkspace {
    pub id: String,
    pub name: String,
    /// Zero-padded ordering among workspaces.
    pub position: String,
}

/// A page-tree node (P2-M3): the local mirror of the client's `DocumentView`.
/// Scoped to a `workspace_id`. Each view points at one document (`object_id` =
/// its `doc_id` in `doc_snapshot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalView {
    pub id: String,
    pub workspace_id: String,
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
             );
             CREATE TABLE IF NOT EXISTS local_workspace(
                 id       TEXT PRIMARY KEY,
                 name     TEXT NOT NULL,
                 position TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS doc_update(
                 doc_id  TEXT NOT NULL,
                 clock   INTEGER NOT NULL,
                 payload BLOB NOT NULL,
                 PRIMARY KEY(doc_id, clock)
             );
             CREATE TABLE IF NOT EXISTS sync_cursor(
                 doc_id          TEXT PRIMARY KEY,
                 last_synced_rid INTEGER NOT NULL DEFAULT 0,
                 pushed_clock    INTEGER NOT NULL DEFAULT 0
             );",
        )?;
        // Migrate pre-multi-workspace stores: add the workspace_id column and a
        // default workspace that existing views attach to.
        let has_ws_col: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('local_view') WHERE name='workspace_id'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if !has_ws_col {
            conn.execute(
                "ALTER TABLE local_view ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'local'",
                [],
            )?;
        }
        // Always have a default workspace so migrated views resolve and a fresh
        // store starts usable.
        conn.execute(
            "INSERT OR IGNORE INTO local_workspace(id,name,position) VALUES('local','本地工作区','0000000010')",
            [],
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

    /// Load a document by id, decoding it with `client_id` (the device's actor)
    /// and replaying any incremental updates on top of the base snapshot (P2-M4).
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
        let bytes = match state {
            Some(b) => b,
            None => return Ok(None),
        };
        let mut doc = MicaDoc::from_update_with_client_id(&bytes, Some(client_id))?;
        for (_clock, update) in self.doc_updates(doc_id)? {
            doc.apply_update(&update)?;
        }
        Ok(Some(doc))
    }

    // ── incremental update log + sync cursor — P2-M4 ─────────────────────────

    /// Append an incremental update to a document's log, returning its `clock`
    /// (a per-doc monotonic local sequence). The base snapshot is left untouched;
    /// [`Self::squash`] folds the log back into the base later.
    pub fn append_update(&self, doc_id: &str, update: &[u8]) -> Result<i64, StoreError> {
        let next: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(clock),0)+1 FROM doc_update WHERE doc_id=?1",
            params![doc_id],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO doc_update(doc_id,clock,payload) VALUES(?1,?2,?3)",
            params![doc_id, next, update],
        )?;
        Ok(next)
    }

    /// All of a document's incremental updates, ordered by `clock`.
    pub fn doc_updates(&self, doc_id: &str) -> Result<Vec<(i64, Vec<u8>)>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT clock,payload FROM doc_update WHERE doc_id=?1 ORDER BY clock")?;
        let rows = stmt
            .query_map(params![doc_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Updates with `clock > after` — the queue still to push to the cloud.
    pub fn updates_after(&self, doc_id: &str, after: i64) -> Result<Vec<(i64, Vec<u8>)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT clock,payload FROM doc_update WHERE doc_id=?1 AND clock>?2 ORDER BY clock",
        )?;
        let rows = stmt
            .query_map(params![doc_id, after], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Fold the base snapshot + all logged updates into a single new base, then
    /// drop the log. Coalesces history growth (§4). Caller passes the device
    /// `client_id` so the squashed doc keeps a consistent actor.
    pub fn squash(&self, doc_id: &str, client_id: u64) -> Result<(), StoreError> {
        let doc = match self.load_doc(doc_id, client_id)? {
            Some(d) => d,
            None => return Ok(()),
        };
        let state = doc.encode_state();
        self.conn.execute(
            "INSERT INTO doc_snapshot(doc_id,state,updated_at) VALUES(?1,?2,?3)
             ON CONFLICT(doc_id) DO UPDATE SET state=excluded.state, updated_at=excluded.updated_at",
            params![doc_id, state, now_millis()],
        )?;
        self.conn
            .execute("DELETE FROM doc_update WHERE doc_id=?1", params![doc_id])?;
        Ok(())
    }

    /// A document's cloud sync high-water marks (zeroed if never synced).
    pub fn sync_cursor(&self, doc_id: &str) -> Result<SyncCursor, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT last_synced_rid,pushed_clock FROM sync_cursor WHERE doc_id=?1",
                params![doc_id],
                |r| {
                    Ok(SyncCursor {
                        last_synced_rid: r.get(0)?,
                        pushed_clock: r.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    /// Persist a document's sync high-water marks.
    pub fn set_sync_cursor(&self, doc_id: &str, cursor: SyncCursor) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO sync_cursor(doc_id,last_synced_rid,pushed_clock) VALUES(?1,?2,?3)
             ON CONFLICT(doc_id) DO UPDATE SET
                 last_synced_rid=excluded.last_synced_rid, pushed_clock=excluded.pushed_clock",
            params![doc_id, cursor.last_synced_rid, cursor.pushed_clock],
        )?;
        Ok(())
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

    /// All views across all workspaces (including trashed), ordered by
    /// `position`. The client filters by workspace + trash and builds the tree
    /// from `parent_id`.
    pub fn list_views(&self) -> Result<Vec<LocalView>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id,workspace_id,parent_id,object_id,name,position,trashed \
             FROM local_view ORDER BY position",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(LocalView {
                    id: r.get(0)?,
                    workspace_id: r.get(1)?,
                    parent_id: r.get(2)?,
                    object_id: r.get(3)?,
                    name: r.get(4)?,
                    position: r.get(5)?,
                    trashed: r.get::<_, i64>(6)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Upsert a view — covers create, rename, move (parent/position), and trash
    /// toggling, all by writing the desired row.
    pub fn save_view(&self, v: &LocalView) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO local_view(id,workspace_id,parent_id,object_id,name,position,trashed)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(id) DO UPDATE SET
                 workspace_id=excluded.workspace_id, parent_id=excluded.parent_id,
                 object_id=excluded.object_id, name=excluded.name,
                 position=excluded.position, trashed=excluded.trashed",
            params![
                v.id,
                v.workspace_id,
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

    // ── workspaces — P2-M3 ───────────────────────────────────────────────────

    /// All local workspaces, ordered by `position`.
    pub fn list_workspaces(&self) -> Result<Vec<LocalWorkspace>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,name,position FROM local_workspace ORDER BY position")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(LocalWorkspace {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    position: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Upsert a workspace (create / rename / reorder).
    pub fn save_workspace(&self, w: &LocalWorkspace) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO local_workspace(id,name,position) VALUES(?1,?2,?3)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, position=excluded.position",
            params![w.id, w.name, w.position],
        )?;
        Ok(())
    }

    /// Delete a workspace and all its views' rows. Documents are deleted
    /// separately by the caller (it knows the object ids).
    pub fn delete_workspace(&self, id: &str) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM local_view WHERE workspace_id=?1", params![id])?;
        self.conn
            .execute("DELETE FROM local_workspace WHERE id=?1", params![id])?;
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
            workspace_id: "local".into(),
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
    fn update_log_replays_on_load_and_squash_collapses() {
        let store = LocalStore::open_in_memory().unwrap();
        let cid = store.identity().unwrap().client_id;
        let (root, blocks) = sample();
        let doc = MicaDoc::from_blocks_with_client_id(&root, &blocks, Some(cid));
        store.save_doc("d", &doc).unwrap();

        // Make two edits as incremental updates (capture each as a diff).
        let mut working = store.load_doc("d", cid).unwrap().unwrap();
        let sv0 = working.state_vector();
        working.text_insert("a", 5, " world");
        let u1 = working.encode_diff(&sv0).unwrap();
        store.append_update("d", &u1).unwrap();
        let sv1 = working.state_vector();
        working.insert_block("r", 1, &Block::new("b", "paragraph").with_text("two"));
        let u2 = working.encode_diff(&sv1).unwrap();
        let clock2 = store.append_update("d", &u2).unwrap();
        assert_eq!(clock2, 2);

        // Loading replays base + updates.
        let loaded = store.load_doc("d", cid).unwrap().unwrap();
        let a = loaded.to_blocks().into_iter().find(|b| b.id == "a").unwrap();
        assert_eq!(a.text, "Hello world");
        assert_eq!(loaded.to_blocks().iter().find(|b| b.id == "r").unwrap().children, vec!["a", "b"]);
        assert_eq!(store.doc_updates("d").unwrap().len(), 2);

        // Squash folds them into the base and clears the log; state preserved.
        store.squash("d", cid).unwrap();
        assert_eq!(store.doc_updates("d").unwrap().len(), 0);
        let after = store.load_doc("d", cid).unwrap().unwrap();
        assert_eq!(after.to_blocks(), loaded.to_blocks());
    }

    #[test]
    fn sync_cursor_round_trip() {
        let store = LocalStore::open_in_memory().unwrap();
        assert_eq!(store.sync_cursor("d").unwrap(), SyncCursor::default());
        store
            .set_sync_cursor("d", SyncCursor { last_synced_rid: 42, pushed_clock: 7 })
            .unwrap();
        let c = store.sync_cursor("d").unwrap();
        assert_eq!(c.last_synced_rid, 42);
        assert_eq!(c.pushed_clock, 7);
        // updates_after honours the pushed cursor.
        let cid = store.identity().unwrap().client_id;
        let doc = MicaDoc::from_blocks_with_client_id("r", &[Block::new("r", "page")], Some(cid));
        store.save_doc("d", &doc).unwrap();
        store.append_update("d", &[1, 2, 3]).unwrap(); // clock 1
        store.append_update("d", &[4, 5, 6]).unwrap(); // clock 2
        assert_eq!(store.updates_after("d", 1).unwrap().len(), 1);
        assert_eq!(store.updates_after("d", 0).unwrap().len(), 2);
    }

    #[test]
    fn workspaces_crud_and_default() {
        let store = LocalStore::open_in_memory().unwrap();
        // A default workspace always exists.
        let all = store.list_workspaces().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "local");

        store
            .save_workspace(&LocalWorkspace {
                id: "w2".into(),
                name: "Work".into(),
                position: "0000000020".into(),
            })
            .unwrap();
        assert_eq!(store.list_workspaces().unwrap().len(), 2);

        // Views are scoped; deleting a workspace removes only its views.
        store.save_view(&LocalView { workspace_id: "w2".into(), ..view("a", None, "A", "0000000010") }).unwrap();
        store.save_view(&LocalView { workspace_id: "local".into(), ..view("b", None, "B", "0000000010") }).unwrap();
        store.delete_workspace("w2").unwrap();
        let views = store.list_views().unwrap();
        assert!(views.iter().any(|v| v.id == "b"));
        assert!(views.iter().all(|v| v.id != "a"));
        assert!(store.list_workspaces().unwrap().iter().all(|w| w.id != "w2"));
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
