//! flutter_rust_bridge surface for the on-device local store (P2-M2).
//!
//! `MicaStore` wraps `mica_core::LocalStore` (SQLite) plus the persisted device
//! identity. Dart opens it once (per local workspace dir), then saves/loads
//! `MicaDocument`s. Loaded docs automatically get this device's stable yrs
//! client id, so offline edits keep one consistent actor across restarts.

use std::sync::Mutex;

use flutter_rust_bridge::frb;
use mica_core::{
    LocalStore, LocalVersion as CoreVersion, LocalView as CoreView,
    LocalWorkspace as CoreWorkspace, SyncCursor as CoreSyncCursor,
};

use crate::api::document::MicaDocument;

/// A document's sync progress against the cloud update stream (P2 local-first).
/// Persisted per-doc so a locally-stored replica knows where to resume: pull the
/// cloud tail after `last_synced_rid`, push local log entries past `pushed_clock`.
pub struct SyncCursor {
    /// Highest cloud stream id (`rid`) this device has pulled and applied.
    pub last_synced_rid: i64,
    /// Highest local update `clock` this device has pushed to the cloud.
    pub pushed_clock: i64,
}

impl From<CoreSyncCursor> for SyncCursor {
    fn from(c: CoreSyncCursor) -> Self {
        SyncCursor { last_synced_rid: c.last_synced_rid, pushed_clock: c.pushed_clock }
    }
}

impl From<SyncCursor> for CoreSyncCursor {
    fn from(c: SyncCursor) -> Self {
        CoreSyncCursor { last_synced_rid: c.last_synced_rid, pushed_clock: c.pushed_clock }
    }
}

/// One entry from a doc's local update log: its monotonic `clock` and the yrs
/// update bytes. `updates_after(pushed_clock)` yields the un-pushed outbox.
pub struct DocUpdate {
    pub clock: i64,
    pub payload: Vec<u8>,
}

/// One entry in a local page's version timeline, mirrored to Dart. `label` is
/// null for an auto snapshot, set for a named checkpoint; `created_at` is unix
/// millis.
pub struct LocalVersion {
    pub id: String,
    pub label: Option<String>,
    pub created_at: i64,
}

impl From<CoreVersion> for LocalVersion {
    fn from(v: CoreVersion) -> Self {
        LocalVersion {
            id: v.id,
            label: v.label,
            created_at: v.created_at,
        }
    }
}

/// A page-tree node mirrored to Dart (P2-M3) — the local mirror of the client's
/// `DocumentView`. `object_id` is the document's `doc_id`.
pub struct LocalView {
    pub id: String,
    pub workspace_id: String,
    pub parent_id: Option<String>,
    pub object_id: String,
    pub name: String,
    pub position: String,
    pub trashed: bool,
    /// Provenance: "local" for on-device pages, or a server URL for a cloud
    /// page mirrored for offline nav. `list_views` filters by this.
    pub origin: String,
    /// "document" (default) or "folder" (a pure container, no content).
    pub object_type: String,
}

/// One on-device image blob for a folder-tree ZIP export: the referencing
/// block's `file_id`, its display name, and the bytes Dart read from the local
/// blob CAS. The store reads views + document payloads itself; only blob bytes
/// (Dart-managed files) must be handed in.
pub struct FolderExportImage {
    pub file_id: String,
    pub name: String,
    pub bytes: Vec<u8>,
}

impl From<CoreView> for LocalView {
    fn from(v: CoreView) -> Self {
        LocalView {
            id: v.id,
            workspace_id: v.workspace_id,
            parent_id: v.parent_id,
            object_id: v.object_id,
            name: v.name,
            position: v.position,
            trashed: v.trashed,
            origin: v.origin,
            object_type: v.object_type,
        }
    }
}

impl From<LocalView> for CoreView {
    fn from(v: LocalView) -> Self {
        CoreView {
            id: v.id,
            workspace_id: v.workspace_id,
            parent_id: v.parent_id,
            object_id: v.object_id,
            name: v.name,
            position: v.position,
            trashed: v.trashed,
            origin: v.origin,
            object_type: v.object_type,
        }
    }
}

/// A local workspace mirrored to Dart (P2-M3).
pub struct LocalWorkspace {
    pub id: String,
    pub name: String,
    pub position: String,
    /// Provenance: "local" or a server URL — same scoping as [`LocalView`].
    pub origin: String,
    /// The user's role in this workspace (mirrored from the server so offline
    /// editing knows whether it's allowed — P2d). Local workspaces: owner.
    pub role: String,
}

impl From<CoreWorkspace> for LocalWorkspace {
    fn from(w: CoreWorkspace) -> Self {
        LocalWorkspace {
            id: w.id,
            name: w.name,
            position: w.position,
            origin: w.origin,
            role: w.role,
        }
    }
}

impl From<LocalWorkspace> for CoreWorkspace {
    fn from(w: LocalWorkspace) -> Self {
        CoreWorkspace {
            id: w.id,
            name: w.name,
            position: w.position,
            origin: w.origin,
            role: w.role,
        }
    }
}

#[frb(opaque)]
pub struct MicaStore {
    inner: Mutex<LocalStore>,
    client_id: u64,
    device_id: String,
}

impl MicaStore {
    /// Open (creating if needed) the local store at `path`. Returns null on
    /// failure (e.g. an unwritable path).
    #[frb(sync)]
    pub fn open(path: String) -> Option<MicaStore> {
        let store = LocalStore::open(&path).ok()?;
        let id = store.identity().ok()?;
        Some(MicaStore {
            inner: Mutex::new(store),
            client_id: id.client_id,
            device_id: id.device_id,
        })
    }

    /// The stable yrs client id new/loaded documents should use.
    #[frb(sync)]
    pub fn client_id(&self) -> u64 {
        self.client_id
    }

    /// The persisted device id.
    #[frb(sync)]
    pub fn device_id(&self) -> String {
        self.device_id.clone()
    }

    /// Ids of all stored documents (sorted).
    #[frb(sync)]
    pub fn list_docs(&self) -> Vec<String> {
        self.inner.lock().unwrap().list_docs().unwrap_or_default()
    }

    #[frb(sync)]
    pub fn delete_doc(&self, doc_id: String) {
        let _ = self.inner.lock().unwrap().delete_doc(&doc_id);
    }

    /// Save the doc's current base as a recovery checkpoint (§10). Call at safe
    /// points (doc open/close) so a later corruption can be rolled back.
    #[frb(sync)]
    pub fn checkpoint_doc(&self, doc_id: String) {
        let _ = self.inner.lock().unwrap().checkpoint_doc(&doc_id);
    }

    /// Restore a doc from its last checkpoint, returning the recovered document
    /// (null if there's no checkpoint).
    #[frb(sync)]
    pub fn rollback_doc(&self, doc_id: String) -> Option<MicaDocument> {
        let loaded = self
            .inner
            .lock()
            .unwrap()
            .rollback_doc(&doc_id, self.client_id)
            .ok()??;
        Some(MicaDocument {
            inner: Mutex::new(loaded),
        })
    }

    /// Persist a document under `doc_id` (full snapshot).
    #[frb(sync)]
    pub fn save_doc(&self, doc_id: String, doc: &MicaDocument) {
        let doc_guard = doc.inner.lock().unwrap();
        let _ = self.inner.lock().unwrap().save_doc(&doc_id, &doc_guard);
    }

    // ── local page version history (docs/version-history-plan.md §6) ──────────

    /// The document's version timeline, newest first — auto snapshots (captured
    /// on a cadence by `save_doc`) and named checkpoints interleaved.
    #[frb(sync)]
    pub fn list_local_versions(&self, doc_id: String) -> Vec<LocalVersion> {
        self.inner
            .lock()
            .unwrap()
            .list_local_versions(&doc_id)
            .unwrap_or_default()
            .into_iter()
            .map(LocalVersion::from)
            .collect()
    }

    /// Pin the current saved state as a NAMED version (never auto-pruned). Null
    /// if the document has no saved snapshot yet.
    #[frb(sync)]
    pub fn create_local_version(&self, doc_id: String, label: String) -> Option<LocalVersion> {
        self.inner
            .lock()
            .unwrap()
            .create_local_version(&doc_id, &label)
            .ok()?
            .map(LocalVersion::from)
    }

    /// Decode a version into a THROWAWAY document for read-only preview (never
    /// the live doc). Null if the version isn't found. The caller renders it with
    /// `to_blocks_json()` / `root_block_id()` in a read-only editor.
    #[frb(sync)]
    pub fn local_version_doc(&self, doc_id: String, version_id: String) -> Option<MicaDocument> {
        let bytes = self
            .inner
            .lock()
            .unwrap()
            .local_version_state(&doc_id, &version_id)
            .ok()??;
        crate::api::document::MicaDocument::from_state_with_client_id(bytes, self.client_id)
    }

    /// Restore the document to a version, returning the recovered doc (null if
    /// the version isn't found). The pre-restore state is kept as an auto version
    /// so the restore is itself undoable.
    #[frb(sync)]
    pub fn restore_local_version(
        &self,
        doc_id: String,
        version_id: String,
    ) -> Option<MicaDocument> {
        let loaded = self
            .inner
            .lock()
            .unwrap()
            .restore_local_version(&doc_id, &version_id, self.client_id)
            .ok()??;
        Some(MicaDocument {
            inner: Mutex::new(loaded),
        })
    }

    // ── page tree (views) — P2-M3 ────────────────────────────────────────────

    /// All views (including trashed) for `origin` ("local" or a server URL),
    /// ordered by position. The client builds the tree from `parent_id` and
    /// filters trash.
    #[frb(sync)]
    pub fn list_views(&self, origin: String) -> Vec<LocalView> {
        self.inner
            .lock()
            .unwrap()
            .list_views(&origin)
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Upsert a view (create / rename / move / trash-toggle).
    #[frb(sync)]
    pub fn save_view(&self, view: LocalView) {
        let _ = self.inner.lock().unwrap().save_view(&view.into());
    }

    /// Permanently remove one `origin`'s view row (delete its document via
    /// [`Self::delete_doc`]). Origin-scoped — can never reach across the
    /// local/cloud namespaces (v4 composite PK).
    #[frb(sync)]
    pub fn purge_view(&self, origin: String, id: String) {
        let _ = self.inner.lock().unwrap().purge_view(&origin, &id);
    }

    /// All workspaces for `origin` ("local" or a server URL), ordered by position.
    #[frb(sync)]
    pub fn list_workspaces(&self, origin: String) -> Vec<LocalWorkspace> {
        self.inner
            .lock()
            .unwrap()
            .list_workspaces(&origin)
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Upsert a workspace (create / rename / reorder).
    #[frb(sync)]
    pub fn save_workspace(&self, workspace: LocalWorkspace) {
        let _ = self.inner.lock().unwrap().save_workspace(&workspace.into());
    }

    /// Delete one `origin`'s workspace and all its view rows (delete documents
    /// separately). Origin-scoped (v4 composite PK).
    #[frb(sync)]
    pub fn delete_workspace(&self, origin: String, id: String) {
        let _ = self.inner.lock().unwrap().delete_workspace(&origin, &id);
    }

    /// Load a document by id, decoded with this device's stable client id, or
    /// null if there's no such document.
    #[frb(sync)]
    pub fn load_doc(&self, doc_id: String) -> Option<MicaDocument> {
        let loaded = self
            .inner
            .lock()
            .unwrap()
            .load_doc(&doc_id, self.client_id)
            .ok()??;
        Some(MicaDocument {
            inner: Mutex::new(loaded),
        })
    }

    /// Export a folder's subtree (`folder_id = Some`) — or the whole workspace
    /// (`None`) — as a Markdown ZIP, through the SAME shared builder the cloud
    /// uses (`mica_interchange::build_markdown_tree_zip`), so a local export is
    /// byte-identical to a cloud one and the export→import round-trip holds. The
    /// store supplies views + document payloads; [`images`] supplies the blob
    /// bytes per `file_id` (Dart reads the on-device CAS). Closes the last local
    /// folder-export gap (was cloud-only).
    #[frb(sync)]
    pub fn export_folder_zip(
        &self,
        workspace_id: String,
        folder_id: Option<String>,
        images: Vec<FolderExportImage>,
    ) -> Vec<u8> {
        let views = self.list_views("local".to_string());
        let nodes: Vec<mica_interchange::TreeNode> = views
            .iter()
            .filter(|v| v.workspace_id == workspace_id && !v.trashed)
            .map(|v| mica_interchange::TreeNode {
                id: v.id.clone(),
                parent_id: v.parent_id.clone(),
                position: v.position.clone(),
                name: v.name.clone(),
                object_type: v.object_type.clone(),
                object_id: v.object_id.clone(),
            })
            .collect();
        let mut payloads = std::collections::HashMap::new();
        for v in &views {
            if v.workspace_id != workspace_id || v.trashed || v.object_type != "document" {
                continue;
            }
            if let Some(doc) = self.load_doc(v.object_id.clone()) {
                payloads.insert(v.object_id.clone(), doc.snapshot());
            }
        }
        let images_map: std::collections::HashMap<String, mica_interchange::ImageAsset> = images
            .into_iter()
            .map(|i| {
                (
                    i.file_id.clone(),
                    mica_interchange::ImageAsset {
                        name: i.name,
                        bytes: i.bytes,
                        // Local CAS is content-addressed by file_id, so it IS the
                        // dedup key (same file_id ⇒ same blob).
                        dedup_key: i.file_id,
                    },
                )
            })
            .collect();
        let entries = mica_interchange::build_markdown_tree_zip(
            &nodes,
            folder_id.as_deref(),
            &payloads,
            &images_map,
        );
        mica_interchange::build_zip(&entries)
    }

    // ── base + append-log + sync cursor (P2 local-first) ─────────────────────
    //
    // The nbstore-shaped durable form the cloud path will adopt: a doc is a base
    // snapshot (save_doc) plus an append-only log of yrs updates. Each local edit
    // (and each merged remote update) appends here so offline work survives a
    // restart and can be re-pushed; `sync_cursor` tracks how far this device has
    // pulled from / pushed to the cloud stream.

    /// Append a yrs `update` to `doc_id`'s local log; returns its new monotonic
    /// `clock` (0 on error). Pair with [`Self::save_doc`] as the base.
    #[frb(sync)]
    pub fn append_update(&self, doc_id: String, update: Vec<u8>) -> i64 {
        self.inner
            .lock()
            .unwrap()
            .append_update(&doc_id, &update)
            .unwrap_or(0)
    }

    /// Log entries with `clock > after`, ordered — the un-pushed outbox when
    /// `after = sync_cursor.pushed_clock`, or catch-up from a known clock.
    #[frb(sync)]
    pub fn updates_after(&self, doc_id: String, after: i64) -> Vec<DocUpdate> {
        self.inner
            .lock()
            .unwrap()
            .updates_after(&doc_id, after)
            .unwrap_or_default()
            .into_iter()
            .map(|(clock, payload)| DocUpdate { clock, payload })
            .collect()
    }

    /// Durably append a REMOTE update and advance `last_synced_rid` in the same
    /// transaction (P4-1) — the persisted cursor can never point past an update
    /// that isn't on disk. Idempotent per `(doc_id, rid)`. Returns false when
    /// the write FAILED (busy/full/io) so the caller can self-heal by writing a
    /// base snapshot from the live in-memory doc — a swallowed failure here plus
    /// a later disk-rebuilding compact would silently lose the update (P4-1
    /// review CRITICAL).
    #[frb(sync)]
    pub fn append_remote_update(&self, doc_id: String, rid: i64, update: Vec<u8>) -> bool {
        self.inner
            .lock()
            .unwrap()
            .append_remote_update(&doc_id, rid, &update)
            .is_ok()
    }

    /// Batch variant: one transaction (one journal sync) for a whole
    /// `sync.updates` catch-up array. Parallel lists (rids[i] ↔ updates[i]).
    #[frb(sync)]
    pub fn append_remote_updates(
        &self,
        doc_id: String,
        rids: Vec<i64>,
        updates: Vec<Vec<u8>>,
    ) -> bool {
        let items: Vec<(i64, Vec<u8>)> =
            rids.into_iter().zip(updates.into_iter()).collect();
        self.inner
            .lock()
            .unwrap()
            .append_remote_updates(&doc_id, &items)
            .is_ok()
    }

    /// (local outbox rows, remote log rows) — compaction-trigger bookkeeping.
    #[frb(sync)]
    pub fn log_sizes(&self, doc_id: String) -> (i64, i64) {
        self.inner
            .lock()
            .unwrap()
            .log_sizes(&doc_id)
            .unwrap_or((0, 0))
    }

    /// Fold the update log into the base snapshot and truncate it (compaction),
    /// so the log doesn't grow without bound. Safe no-op if the doc is absent.
    #[frb(sync)]
    pub fn squash(&self, doc_id: String) {
        let _ = self.inner.lock().unwrap().squash(&doc_id, self.client_id);
    }

    /// Drop acked outbox entries (`clock ≤ up_to_clock`, i.e. `pushed_clock`),
    /// bounding the append-log while leaving the un-pushed tail intact. The base
    /// snapshot already folds these, so reads are unchanged; the clock stays
    /// monotonic across this so no future clock is reused below the trim.
    #[frb(sync)]
    pub fn trim_updates_through(&self, doc_id: String, up_to_clock: i64) {
        let _ = self
            .inner
            .lock()
            .unwrap()
            .trim_updates_through(&doc_id, up_to_clock);
    }

    /// This doc's sync progress (0/0 if it has never synced).
    #[frb(sync)]
    pub fn sync_cursor(&self, doc_id: String) -> SyncCursor {
        self.inner
            .lock()
            .unwrap()
            .sync_cursor(&doc_id)
            .unwrap_or(CoreSyncCursor { last_synced_rid: 0, pushed_clock: 0 })
            .into()
    }

    /// Persist this doc's sync progress.
    #[frb(sync)]
    pub fn set_sync_cursor(&self, doc_id: String, cursor: SyncCursor) {
        let _ = self
            .inner
            .lock()
            .unwrap()
            .set_sync_cursor(&doc_id, cursor.into());
    }
}
