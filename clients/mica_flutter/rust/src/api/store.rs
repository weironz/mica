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

/// What [`MicaStore::clone_view`] produced: where the copy landed, the name it
/// settled on after dedup, and how many documents were actually copied
/// (folders and doc-less views do not count).
pub struct CloneViewResult {
    pub root_view_id: String,
    pub new_name: String,
    pub docs: u32,
}

/// Copy a view row field-by-field.
///
/// `LocalView` is an frb wire struct and deliberately not `Clone` — the bridge
/// owns its shape. The subtree operations need a copy to edit one field of, so
/// this spells it out rather than deriving a trait onto the wire type.
fn clone_view_row(v: &LocalView) -> LocalView {
    LocalView {
        id: v.id.clone(),
        workspace_id: v.workspace_id.clone(),
        parent_id: v.parent_id.clone(),
        object_id: v.object_id.clone(),
        name: v.name.clone(),
        position: v.position.clone(),
        trashed: v.trashed,
        origin: v.origin.clone(),
        object_type: v.object_type.clone(),
    }
}

/// `base` if free among `siblings`, else `base 2`, `base 3`, … — the number is
/// locale-neutral, the caller supplies the localized base.
///
/// Same rule as the server's `dedup_sibling_name` (api-server documents.rs) so
/// a page cloned offline and one cloned in the cloud settle on the same name.
/// Kept a free function: pure, and that is what makes it testable without a
/// store.
fn dedup_sibling_name(base: &str, siblings: &[String]) -> String {
    if !siblings.iter().any(|s| s == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|c| !siblings.iter().any(|s| s == c))
        .expect("an unused suffix always exists")
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
    /// The store lock, recovering from poisoning rather than propagating it.
    /// Same reasoning as `MicaDocument::doc` — one panic must not make the
    /// on-device store permanently unusable. rusqlite statements are executed
    /// one at a time here, so an unwind leaves the connection usable; a
    /// half-written multi-statement change is bounded by SQLite's own
    /// transaction, not by this mutex.
    fn store(&self) -> std::sync::MutexGuard<'_, LocalStore> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
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
        self.store().list_docs().unwrap_or_default()
    }

    #[frb(sync)]
    pub fn delete_doc(&self, doc_id: String) {
        let _ = self.store().delete_doc(&doc_id);
    }

    /// Save the doc's current base as a recovery checkpoint (§10). Call at safe
    /// points (doc open/close) so a later corruption can be rolled back.
    #[frb(sync)]
    pub fn checkpoint_doc(&self, doc_id: String) {
        let _ = self.store().checkpoint_doc(&doc_id);
    }

    /// Restore a doc from its last checkpoint, returning the recovered document
    /// (null if there's no checkpoint).
    #[frb(sync)]
    pub fn rollback_doc(&self, doc_id: String) -> Option<MicaDocument> {
        let loaded = self.store()
            .rollback_doc(&doc_id, self.client_id)
            .ok()??;
        Some(MicaDocument {
            inner: Mutex::new(loaded),
        })
    }

    /// Persist a document under `doc_id` (full snapshot).
    #[frb(sync)]
    pub fn save_doc(&self, doc_id: String, doc: &MicaDocument) {
        let doc_guard = doc.doc();
        let _ = self.store().save_doc(&doc_id, &doc_guard);
    }

    // ── local page version history (docs/version-history-plan.md §6) ──────────

    /// The document's version timeline, newest first — auto snapshots (captured
    /// on a cadence by `save_doc`) and named checkpoints interleaved.
    #[frb(sync)]
    pub fn list_local_versions(&self, doc_id: String) -> Vec<LocalVersion> {
        self.store()
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
        self.store()
            .create_local_version(&doc_id, &label)
            .ok()?
            .map(LocalVersion::from)
    }

    /// Decode a version into a THROWAWAY document for read-only preview (never
    /// the live doc). Null if the version isn't found. The caller renders it with
    /// `to_blocks_json()` / `root_block_id()` in a read-only editor.
    #[frb(sync)]
    pub fn local_version_doc(&self, doc_id: String, version_id: String) -> Option<MicaDocument> {
        let bytes = self.store()
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
        let loaded = self.store()
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
        self.store()
            .list_views(&origin)
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Upsert a view (create / rename / move / trash-toggle).
    #[frb(sync)]
    pub fn save_view(&self, view: LocalView) {
        let _ = self.store().save_view(&view.into());
    }

    /// Permanently remove one `origin`'s view row (delete its document via
    /// [`Self::delete_doc`]). Origin-scoped — can never reach across the
    /// local/cloud namespaces (v4 composite PK).
    #[frb(sync)]
    pub fn purge_view(&self, origin: String, id: String) {
        let _ = self.store().purge_view(&origin, &id);
    }

    /// Where the next child of `parent_id` goes: after the last LIVE sibling,
    /// in steps of ten, zero-padded to ten digits so string ordering is
    /// numeric ordering.
    ///
    /// One rule, one place. It used to be written out separately in the Dart
    /// create path, the Dart reorder path and `clone_view` below — three copies
    /// of a convention where a single disagreement puts a new page in the wrong
    /// spot. The gap of ten is what leaves room to drop something between two
    /// siblings without renumbering them.
    fn next_position(&self, parent_id: Option<&str>) -> String {
        let max = self
            .list_views("local".to_string())
            .iter()
            .filter(|v| !v.trashed && v.parent_id.as_deref() == parent_id)
            .filter_map(|v| v.position.parse::<i64>().ok())
            .max()
            .unwrap_or(0);
        format!("{:010}", max + 10)
    }

    /// Create a page or folder row under `parent_id` and return its id.
    ///
    /// Serves both kinds: a document passes the `object_id` its freshly-created
    /// doc got, a folder passes a placeholder (it owns no document). Position
    /// comes from [`Self::next_position`], so a page created here and a page
    /// cloned by [`Self::clone_view`] land by the same rule.
    #[frb(sync)]
    pub fn create_view(
        &self,
        workspace_id: String,
        parent_id: Option<String>,
        object_id: String,
        name: String,
        object_type: String,
    ) -> String {
        let id = format!("view_{}", uuid::Uuid::new_v4());
        self.save_view(LocalView {
            id: id.clone(),
            workspace_id,
            position: self.next_position(parent_id.as_deref()),
            parent_id,
            object_id,
            name,
            trashed: false,
            origin: "local".to_string(),
            object_type,
        });
        id
    }

    /// Renumber `ordered_ids` as consecutive children of `parent_id`.
    ///
    /// Positions restart at 10 and step by ten, matching
    /// [`Self::next_position`] so a later insert still lands after them. Rows
    /// already in the right place are left alone — an untouched row is one less
    /// write and one less sync update. Reparenting is part of the same move:
    /// dragging into another folder changes both.
    #[frb(sync)]
    pub fn reorder_views(&self, parent_id: Option<String>, ordered_ids: Vec<String>) {
        let all = self.list_views("local".to_string());
        for (i, id) in ordered_ids.iter().enumerate() {
            let Some(v) = all.iter().find(|v| &v.id == id) else {
                continue;
            };
            let position = format!("{:010}", (i as i64 + 1) * 10);
            if v.position == position && v.parent_id == parent_id {
                continue;
            }
            self.save_view(LocalView {
                parent_id: parent_id.clone(),
                position,
                trashed: false,
                ..clone_view_row(v)
            });
        }
    }

    /// Where the next workspace goes — the same step-of-ten rule as
    /// [`Self::next_position`], over workspaces instead of sibling views.
    fn next_workspace_position(&self) -> String {
        let max = self
            .list_workspaces("local".to_string())
            .iter()
            .filter_map(|w| w.position.parse::<i64>().ok())
            .max()
            .unwrap_or(0);
        format!("{:010}", max + 10)
    }

    /// Create a local workspace and return its id.
    #[frb(sync)]
    pub fn create_workspace(&self, name: String) -> String {
        let id = format!("ws_{}", uuid::Uuid::new_v4());
        self.save_workspace(LocalWorkspace {
            id: id.clone(),
            name,
            position: self.next_workspace_position(),
            origin: "local".to_string(),
            role: "owner".to_string(),
        });
        id
    }

    /// Rename a workspace, KEEPING its position and role.
    ///
    /// The Dart version had to read the row back to preserve `position`,
    /// because the only primitive was a whole-row upsert — and when the read
    /// missed it invented `0000000010`, which can collide with a real
    /// neighbour. Doing it here means a missing row is simply a no-op.
    #[frb(sync)]
    pub fn rename_workspace(&self, id: String, name: String) {
        let all = self.list_workspaces("local".to_string());
        let Some(w) = all.iter().find(|w| w.id == id) else {
            return;
        };
        self.save_workspace(LocalWorkspace {
            id: w.id.clone(),
            name,
            position: w.position.clone(),
            origin: w.origin.clone(),
            role: w.role.clone(),
        });
    }

    /// Delete a workspace, its views and their documents. Returns false — and
    /// changes nothing — when it is the last one.
    ///
    /// Both rules live here rather than in the caller. "Keep at least one
    /// workspace" was a UI-side check, so any other path into delete could
    /// leave the device with none and nowhere to put a page; and dropping the
    /// workspace row without its documents left orphaned blobs that nothing
    /// would ever reach again.
    #[frb(sync)]
    pub fn delete_workspace_cascade(&self, id: String) -> bool {
        if self.list_workspaces("local".to_string()).len() <= 1 {
            return false;
        }
        for v in self
            .list_views("local".to_string())
            .iter()
            .filter(|v| v.workspace_id == id)
        {
            self.delete_doc(v.object_id.clone());
            self.purge_view("local".to_string(), v.id.clone());
        }
        self.delete_workspace("local".to_string(), id);
        true
    }

    /// Renumber workspaces to match `ordered_ids` (drag-reorder), same
    /// step-of-ten scheme as [`Self::reorder_views`].
    #[frb(sync)]
    pub fn reorder_workspaces(&self, ordered_ids: Vec<String>) {
        let all = self.list_workspaces("local".to_string());
        for (i, id) in ordered_ids.iter().enumerate() {
            let Some(w) = all.iter().find(|w| &w.id == id) else {
                continue;
            };
            let position = format!("{:010}", (i as i64 + 1) * 10);
            if w.position == position {
                continue;
            }
            self.save_workspace(LocalWorkspace {
                id: w.id.clone(),
                name: w.name.clone(),
                position,
                origin: w.origin.clone(),
                role: w.role.clone(),
            });
        }
    }

    /// `view_id` plus every descendant, in `list_views` order.
    ///
    /// The recursive walk the three subtree operations below share, and the
    /// local mirror of the server's recursive-CTE cascade.
    fn subtree_ids(&self, view_id: &str) -> Vec<String> {
        let all = self.list_views("local".to_string());
        let mut ids: Vec<String> = Vec::new();
        let mut queue = vec![view_id.to_string()];
        while let Some(id) = queue.pop() {
            if ids.iter().any(|k| k == &id) {
                continue; // a cycle must not wedge the walk
            }
            queue.extend(
                all.iter()
                    .filter(|v| v.parent_id.as_deref() == Some(id.as_str()))
                    .map(|v| v.id.clone()),
            );
            ids.push(id);
        }
        // Report in tree order, not pop order — the caller shows these.
        all.iter()
            .filter(|v| ids.iter().any(|k| k == &v.id))
            .map(|v| v.id.clone())
            .collect()
    }

    /// Move `view_id` and its whole subtree to the recycle bin. Returns the
    /// affected ids so the caller can close an editor that was inside it.
    ///
    /// A folder carries its children: trashing one must trash the subtree, or
    /// the children survive as orphans pointing at a trashed parent.
    #[frb(sync)]
    pub fn trash_view_subtree(&self, view_id: String) -> Vec<String> {
        let ids = self.subtree_ids(&view_id);
        self.set_trashed(&ids, true);
        ids
    }

    /// Bring `view_id` and the subtree trashed with it back out of the bin.
    ///
    /// If the restored ROOT's parent is no longer a live view, the root is
    /// lifted to the top level — restoring into a trashed parent would leave it
    /// invisible with no way back. Mirrors the server's `restore_view`.
    #[frb(sync)]
    pub fn restore_view_subtree(&self, view_id: String) -> Vec<String> {
        let ids = self.subtree_ids(&view_id);
        self.set_trashed(&ids, false);

        let all = self.list_views("local".to_string());
        let parent_is_gone = all
            .iter()
            .find(|v| v.id == view_id)
            .and_then(|root| root.parent_id.clone())
            .is_some_and(|pid| !all.iter().any(|v| v.id == pid && !v.trashed));
        if parent_is_gone {
            if let Some(root) = all.iter().find(|v| v.id == view_id) {
                self.save_view(LocalView {
                    parent_id: None,
                    trashed: false,
                    ..clone_view_row(root)
                });
            }
        }
        ids
    }

    /// Permanently remove `view_id` and its subtree, documents included.
    /// Returns the affected ids. There is no undo past this point.
    #[frb(sync)]
    pub fn purge_view_subtree(&self, view_id: String) -> Vec<String> {
        let ids = self.subtree_ids(&view_id);
        let all = self.list_views("local".to_string());
        for id in &ids {
            if let Some(v) = all.iter().find(|v| &v.id == id) {
                self.delete_doc(v.object_id.clone());
            }
            self.purge_view("local".to_string(), id.clone());
        }
        ids
    }

    /// Flip `trashed` on exactly `ids`, leaving every other field alone.
    fn set_trashed(&self, ids: &[String], trashed: bool) {
        let all = self.list_views("local".to_string());
        for v in all.iter().filter(|v| ids.iter().any(|k| k == &v.id)) {
            self.save_view(LocalView { trashed, ..clone_view_row(v) });
        }
    }

    /// Duplicate `view_id` and everything under it, beside the original.
    ///
    /// The WHOLE operation, not a helper: subtree walk, fresh ids, doc copies,
    /// sibling-name dedup and positioning all happen here. That is the point —
    /// moving a leaf helper across the bridge would leave the tree rules
    /// duplicated and pay a crossing per node; moving the operation deletes the
    /// Dart copy outright. Mirrors `export_folder_zip` above, the composite
    /// that was already de-duplicated this way.
    ///
    /// Returns `None` when the view is gone.
    #[frb(sync)]
    pub fn clone_view(&self, view_id: String, root_name: String) -> Option<CloneViewResult> {
        let all = self.list_views("local".to_string());
        let root: &LocalView = all.iter().find(|v| v.id == view_id)?;

        // Subtree = root + every descendant. Walked by REFERENCE: `LocalView`
        // is an frb wire struct with no `Clone`, and the only copies we
        // actually want are the new rows written at the end.
        let mut subtree: Vec<&LocalView> = Vec::new();
        let mut queue: Vec<&LocalView> = vec![root];
        while let Some(v) = queue.pop() {
            queue.extend(
                all.iter()
                    .filter(|c| c.parent_id.as_deref() == Some(v.id.as_str())),
            );
            subtree.push(v);
        }

        // Dedup against LIVE siblings under the same parent; position comes
        // from the shared rule so a clone lands exactly where a newly created
        // page would.
        let sibling_names: Vec<String> = all
            .iter()
            .filter(|v| v.parent_id == root.parent_id && !v.trashed)
            .map(|v| v.name.clone())
            .collect();
        let new_name = dedup_sibling_name(&root_name, &sibling_names);
        let root_position = self.next_position(root.parent_id.as_deref());

        // Fresh id per node. The ROOT keeps its parent (it lands beside the
        // original); inner nodes point at their COPIED parent.
        let id_map: std::collections::HashMap<String, String> = subtree
            .iter()
            .map(|v| (v.id.clone(), format!("view_{}", uuid::Uuid::new_v4())))
            .collect();
        let mut docs: u32 = 0;
        for v in &subtree {
            let is_root = v.id == view_id;
            let new_doc_id = format!("doc_{}", uuid::Uuid::new_v4());
            match self.load_doc(v.object_id.clone()) {
                Some(doc) => {
                    self.save_doc(new_doc_id.clone(), &doc);
                    docs += 1;
                }
                // A view with no document still gets an empty one, or the copy
                // opens to nothing.
                None => self.save_doc(
                    new_doc_id.clone(),
                    &MicaDocument::from_markdown(String::new()),
                ),
            }
            self.save_view(LocalView {
                id: id_map[&v.id].clone(),
                workspace_id: v.workspace_id.clone(),
                parent_id: if is_root {
                    v.parent_id.clone()
                } else {
                    v.parent_id.as_ref().and_then(|p| id_map.get(p).cloned())
                },
                object_id: new_doc_id,
                name: if is_root {
                    new_name.clone()
                } else {
                    v.name.clone()
                },
                position: if is_root {
                    root_position.clone()
                } else {
                    v.position.clone()
                },
                trashed: false,
                origin: "local".to_string(),
                object_type: v.object_type.clone(),
            });
        }
        Some(CloneViewResult {
            root_view_id: id_map[&view_id].clone(),
            new_name,
            docs,
        })
    }

    /// All workspaces for `origin` ("local" or a server URL), ordered by position.
    #[frb(sync)]
    pub fn list_workspaces(&self, origin: String) -> Vec<LocalWorkspace> {
        self.store()
            .list_workspaces(&origin)
            .unwrap_or_default()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Upsert a workspace (create / rename / reorder).
    #[frb(sync)]
    pub fn save_workspace(&self, workspace: LocalWorkspace) {
        let _ = self.store().save_workspace(&workspace.into());
    }

    /// Delete one `origin`'s workspace and all its view rows (delete documents
    /// separately). Origin-scoped (v4 composite PK).
    #[frb(sync)]
    pub fn delete_workspace(&self, origin: String, id: String) {
        let _ = self.store().delete_workspace(&origin, &id);
    }

    /// Load a document by id, decoded with this device's stable client id, or
    /// null if there's no such document.
    #[frb(sync)]
    pub fn load_doc(&self, doc_id: String) -> Option<MicaDocument> {
        let loaded = self.store()
            .load_doc(&doc_id, self.client_id)
            .ok()??;
        Some(MicaDocument {
            inner: Mutex::new(loaded),
        })
    }

    /// Export a folder's subtree (`folder_id = Some`) — or the whole workspace
    /// (`None`) — as a Markdown ZIP, through the SAME shared builder the cloud
    /// uses (`mica_interchange::build_markdown_tree_zip`), so a local export is
    /// the same format as a cloud one and the export→import round-trip holds. The
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
        self.store()
            .append_update(&doc_id, &update)
            .unwrap_or(0)
    }

    /// Log entries with `clock > after`, ordered — the un-pushed outbox when
    /// `after = sync_cursor.pushed_clock`, or catch-up from a known clock.
    #[frb(sync)]
    pub fn updates_after(&self, doc_id: String, after: i64) -> Vec<DocUpdate> {
        self.store()
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
        self.store()
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
        self.store()
            .append_remote_updates(&doc_id, &items)
            .is_ok()
    }

    /// (local outbox rows, remote log rows) — compaction-trigger bookkeeping.
    #[frb(sync)]
    pub fn log_sizes(&self, doc_id: String) -> (i64, i64) {
        self.store()
            .log_sizes(&doc_id)
            .unwrap_or((0, 0))
    }

    /// Fold the update log into the base snapshot and truncate it (compaction),
    /// so the log doesn't grow without bound. Safe no-op if the doc is absent.
    #[frb(sync)]
    pub fn squash(&self, doc_id: String) {
        let _ = self.store().squash(&doc_id, self.client_id);
    }

    /// Drop acked outbox entries (`clock ≤ up_to_clock`, i.e. `pushed_clock`),
    /// bounding the append-log while leaving the un-pushed tail intact. The base
    /// snapshot already folds these, so reads are unchanged; the clock stays
    /// monotonic across this so no future clock is reused below the trim.
    #[frb(sync)]
    pub fn trim_updates_through(&self, doc_id: String, up_to_clock: i64) {
        let _ = self.store()
            .trim_updates_through(&doc_id, up_to_clock);
    }

    /// This doc's sync progress (0/0 if it has never synced).
    #[frb(sync)]
    pub fn sync_cursor(&self, doc_id: String) -> SyncCursor {
        self.store()
            .sync_cursor(&doc_id)
            .unwrap_or(CoreSyncCursor { last_synced_rid: 0, pushed_clock: 0 })
            .into()
    }

    /// Persist this doc's sync progress.
    #[frb(sync)]
    pub fn set_sync_cursor(&self, doc_id: String, cursor: SyncCursor) {
        let _ = self.store()
            .set_sync_cursor(&doc_id, cursor.into());
    }
}

#[cfg(test)]
mod clone_view_tests {
    use super::*;

    // Mirrors the server's own test (api-server documents.rs) on purpose: the
    // two implementations have to agree, so they are checked against the same
    // cases. If one changes, this is where the disagreement shows up.
    #[test]
    fn free_name_is_used_as_is() {
        assert_eq!(dedup_sibling_name("日志方案 副本", &[]), "日志方案 副本");
    }

    #[test]
    fn collision_takes_the_first_free_number() {
        let sibs = vec!["Notes".to_string()];
        assert_eq!(dedup_sibling_name("Notes", &sibs), "Notes 2");
    }

    #[test]
    fn skips_numbers_already_taken() {
        let sibs = vec!["Notes".into(), "Notes 2".into(), "Notes 3".into()];
        assert_eq!(dedup_sibling_name("Notes", &sibs), "Notes 4");
    }

    #[test]
    fn a_gap_is_reused_rather_than_appended_past() {
        let sibs = vec!["Notes".into(), "Notes 3".into()];
        assert_eq!(dedup_sibling_name("Notes", &sibs), "Notes 2");
    }

    // The dedup tests above are pure. These open a REAL store in a temp dir and
    // clone an actual tree, because the parts most likely to break -- the
    // subtree walk, the parent remap, the doc copies -- only exist once rows
    // are in SQLite. This is the layer the Dart mirror used to own.
    fn tmp_store() -> (MicaStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("mica_clone_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = MicaStore::open(dir.join("s.db").to_string_lossy().to_string()).unwrap();
        (store, dir)
    }

    fn view(id: &str, parent: Option<&str>, name: &str, pos: &str) -> LocalView {
        LocalView {
            id: id.into(),
            workspace_id: "ws".into(),
            parent_id: parent.map(str::to_string),
            object_id: format!("doc_{id}"),
            name: name.into(),
            position: pos.into(),
            trashed: false,
            origin: "local".into(),
            object_type: "document".into(),
        }
    }

    #[test]
    fn clones_the_whole_subtree_and_remaps_parents() {
        let (store, _dir) = tmp_store();
        store.save_view(view("root", None, "Parent", "0000000010"));
        store.save_view(view("kid", Some("root"), "Child", "0000000020"));
        store.save_view(view("grandkid", Some("kid"), "Grandchild", "0000000030"));
        for id in ["root", "kid", "grandkid"] {
            store.save_doc(format!("doc_{id}"), &MicaDocument::from_markdown(format!("# {id}")));
        }

        let out = store.clone_view("root".into(), "Parent".into()).unwrap();
        assert_eq!(out.docs, 3, "every node's document should be copied");
        assert_eq!(out.new_name, "Parent 2", "name collides with the original");

        let all = store.list_views("local".to_string());
        assert_eq!(all.len(), 6, "three originals + three copies");

        // The copied root sits beside the original (same parent), and the copy
        // is a SEPARATE row -- the originals must be untouched.
        let new_root = all.iter().find(|v| v.id == out.root_view_id).unwrap();
        assert_eq!(new_root.parent_id, None);
        assert_eq!(new_root.position, "0000000020", "lands after max sibling + 10");

        // Inner nodes point at their COPIED parent, not the original.
        let copied_kid = all
            .iter()
            .find(|v| v.parent_id.as_deref() == Some(out.root_view_id.as_str()))
            .expect("the copied child hangs off the copied root");
        assert_ne!(copied_kid.id, "kid");
        assert_eq!(copied_kid.name, "Child", "only the ROOT gets the deduped name");
        let copied_grandkid = all
            .iter()
            .find(|v| v.parent_id.as_deref() == Some(copied_kid.id.as_str()))
            .expect("the copied grandchild hangs off the copied child");
        assert_ne!(copied_grandkid.id, "grandkid");

        // Content came along, and each copy owns a fresh doc id.
        assert_ne!(copied_kid.object_id, "doc_kid");
        let doc = store.load_doc(copied_kid.object_id.clone()).expect("copied doc exists");
        assert!(doc.export_markdown().contains("kid"), "content copied, not just the row");
    }

    #[test]
    fn a_missing_view_is_none_not_a_panic() {
        let (store, _dir) = tmp_store();
        assert!(store.clone_view("nope".into(), "X".into()).is_none());
    }

    #[test]
    fn a_view_with_no_document_still_gets_one() {
        let (store, _dir) = tmp_store();
        store.save_view(view("solo", None, "Folder", "0000000010"));
        let out = store.clone_view("solo".into(), "Folder".into()).unwrap();
        assert_eq!(out.docs, 0, "nothing was there to copy");
        let all = store.list_views("local".to_string());
        let copy = all.iter().find(|v| v.id == out.root_view_id).unwrap();
        assert!(
            store.load_doc(copy.object_id.clone()).is_some(),
            "the copy still opens to an empty document rather than nothing",
        );
    }

    #[test]
    fn trashing_a_folder_takes_its_whole_subtree() {
        let (store, _dir) = tmp_store();
        store.save_view(view("folder", None, "F", "0000000010"));
        store.save_view(view("kid", Some("folder"), "K", "0000000020"));
        store.save_view(view("grandkid", Some("kid"), "G", "0000000030"));
        store.save_view(view("outside", None, "O", "0000000040"));

        let hit = store.trash_view_subtree("folder".into());
        assert_eq!(hit.len(), 3, "the folder and both descendants");

        let all = store.list_views("local".to_string());
        for id in ["folder", "kid", "grandkid"] {
            assert!(all.iter().find(|v| v.id == id).unwrap().trashed, "{id} should be trashed");
        }
        assert!(
            !all.iter().find(|v| v.id == "outside").unwrap().trashed,
            "an unrelated view must not be touched",
        );
    }

    #[test]
    fn restoring_lifts_a_root_whose_parent_is_still_trashed() {
        let (store, _dir) = tmp_store();
        store.save_view(view("parent", None, "P", "0000000010"));
        store.save_view(view("child", Some("parent"), "C", "0000000020"));
        // Trash both, then restore ONLY the child.
        store.trash_view_subtree("parent".into());
        store.restore_view_subtree("child".into());

        let all = store.list_views("local".to_string());
        let child = all.iter().find(|v| v.id == "child").unwrap();
        assert!(!child.trashed, "the child came back");
        assert_eq!(
            child.parent_id, None,
            "its parent is still in the bin, so the child lifts to the top              level instead of restoring into an invisible parent",
        );
        assert!(all.iter().find(|v| v.id == "parent").unwrap().trashed);
    }

    #[test]
    fn restoring_keeps_the_parent_when_it_is_alive() {
        let (store, _dir) = tmp_store();
        store.save_view(view("parent", None, "P", "0000000010"));
        store.save_view(view("child", Some("parent"), "C", "0000000020"));
        store.trash_view_subtree("child".into());
        store.restore_view_subtree("child".into());

        let all = store.list_views("local".to_string());
        let child = all.iter().find(|v| v.id == "child").unwrap();
        assert_eq!(
            child.parent_id.as_deref(),
            Some("parent"),
            "a live parent must be kept — lifting here would silently reparent",
        );
    }

    #[test]
    fn purging_removes_the_subtree_and_its_documents() {
        let (store, _dir) = tmp_store();
        store.save_view(view("folder", None, "F", "0000000010"));
        store.save_view(view("kid", Some("folder"), "K", "0000000020"));
        store.save_view(view("outside", None, "O", "0000000030"));
        for id in ["folder", "kid", "outside"] {
            store.save_doc(format!("doc_{id}"), &MicaDocument::from_markdown("x".into()));
        }

        let hit = store.purge_view_subtree("folder".into());
        assert_eq!(hit.len(), 2);

        let all = store.list_views("local".to_string());
        assert_eq!(all.len(), 1, "only the unrelated view survives");
        assert_eq!(all[0].id, "outside");
        assert!(store.load_doc("doc_kid".into()).is_none(), "its document went too");
        assert!(
            store.load_doc("doc_outside".into()).is_some(),
            "an unrelated document must survive",
        );
    }

    #[test]
    fn a_parent_cycle_does_not_wedge_the_walk() {
        let (store, _dir) = tmp_store();
        // Corrupt shape: a points at b, b points at a. Real data should never
        // look like this, but a hang here would be unrecoverable for the user.
        store.save_view(view("a", Some("b"), "A", "0000000010"));
        store.save_view(view("b", Some("a"), "B", "0000000020"));
        let hit = store.trash_view_subtree("a".into());
        assert_eq!(hit.len(), 2, "both, once each");
    }

    #[test]
    fn a_new_view_lands_after_its_live_siblings() {
        let (store, _dir) = tmp_store();
        store.save_view(view("a", None, "A", "0000000010"));
        store.save_view(view("b", None, "B", "0000000020"));

        let id = store.create_view(
            "ws".into(), None, "doc_x".into(), "C".into(), "document".into(),
        );
        let all = store.list_views("local".to_string());
        let made = all.iter().find(|v| v.id == id).unwrap();
        assert_eq!(made.position, "0000000030");
        assert!(made.id.starts_with("view_"));
        assert!(!made.trashed);
    }

    #[test]
    fn a_trashed_sibling_does_not_reserve_its_slot() {
        let (store, _dir) = tmp_store();
        store.save_view(view("a", None, "A", "0000000010"));
        store.save_view(view("gone", None, "G", "0000000090"));
        store.trash_view_subtree("gone".into());

        let id = store.create_view(
            "ws".into(), None, "doc_x".into(), "C".into(), "document".into(),
        );
        let all = store.list_views("local".to_string());
        assert_eq!(
            all.iter().find(|v| v.id == id).unwrap().position,
            "0000000020",
            "counting a trashed row would leave a growing gap forever",
        );
    }

    #[test]
    fn positions_are_scoped_to_the_parent() {
        let (store, _dir) = tmp_store();
        store.save_view(view("folder", None, "F", "0000000010"));
        store.save_view(view("deep", Some("folder"), "D", "0000000500"));

        let id = store.create_view(
            "ws".into(), None, "top".into(), "T".into(), "document".into(),
        );
        assert_eq!(
            store.list_views("local".to_string()).iter().find(|v| v.id == id).unwrap().position,
            "0000000020",
            "a child's position must not push the top level along",
        );
    }

    #[test]
    fn a_clone_lands_where_a_new_page_would() {
        // The whole point of sharing next_position: these two must agree.
        let (store, _dir) = tmp_store();
        store.save_view(view("a", None, "A", "0000000010"));
        store.save_doc("doc_a".into(), &MicaDocument::from_markdown("x".into()));

        let cloned = store.clone_view("a".into(), "A".into()).unwrap();
        let all = store.list_views("local".to_string());
        let clone_pos = &all.iter().find(|v| v.id == cloned.root_view_id).unwrap().position;
        assert_eq!(clone_pos, "0000000020");
    }

    #[test]
    fn reorder_renumbers_and_reparents_in_one_move() {
        let (store, _dir) = tmp_store();
        store.save_view(view("x", None, "X", "0000000010"));
        store.save_view(view("y", None, "Y", "0000000020"));
        store.save_view(view("folder", None, "F", "0000000030"));

        // Drag y above x, and move both under the folder.
        store.reorder_views(
            Some("folder".into()),
            vec!["y".to_string(), "x".to_string()],
        );
        let all = store.list_views("local".to_string());
        let y = all.iter().find(|v| v.id == "y").unwrap();
        let x = all.iter().find(|v| v.id == "x").unwrap();
        assert_eq!((y.position.as_str(), y.parent_id.as_deref()), ("0000000010", Some("folder")));
        assert_eq!((x.position.as_str(), x.parent_id.as_deref()), ("0000000020", Some("folder")));
    }

    #[test]
    fn reorder_leaves_an_unknown_id_alone() {
        let (store, _dir) = tmp_store();
        store.save_view(view("x", None, "X", "0000000010"));
        // A stale id from a list the user dragged before a refresh must not
        // panic or create a phantom row.
        store.reorder_views(None, vec!["ghost".to_string(), "x".to_string()]);
        let all = store.list_views("local".to_string());
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].position, "0000000020", "x took the second slot it was given");
    }

    fn ws(id: &str, name: &str, pos: &str) -> LocalWorkspace {
        LocalWorkspace {
            id: id.into(),
            name: name.into(),
            position: pos.into(),
            origin: "local".into(),
            role: "owner".into(),
        }
    }

    #[test]
    fn a_new_workspace_lands_after_the_last_one() {
        let (store, _dir) = tmp_store();
        store.save_workspace(ws("a", "A", "0000000010"));
        let id = store.create_workspace("New".into());
        let all = store.list_workspaces("local".to_string());
        let made = all.iter().find(|w| w.id == id).unwrap();
        assert_eq!(made.position, "0000000020");
        assert!(made.id.starts_with("ws_"));
        assert_eq!(made.role, "owner");
    }

    #[test]
    fn rename_keeps_position_and_role() {
        let (store, _dir) = tmp_store();
        store.save_workspace(ws("a", "Old", "0000000070"));
        store.rename_workspace("a".into(), "New".into());
        let all = store.list_workspaces("local".to_string());
        let w = all.iter().find(|w| w.id == "a").unwrap();
        assert_eq!(w.name, "New");
        assert_eq!(
            w.position, "0000000070",
            "the Dart version reinvented this as 0000000010 when its read missed",
        );
        assert_eq!(w.role, "owner");
    }

    #[test]
    fn renaming_a_missing_workspace_is_a_no_op() {
        let (store, _dir) = tmp_store();
        let before = store.list_workspaces("local".to_string()).len();
        store.rename_workspace("ghost".into(), "X".into());
        let all = store.list_workspaces("local".to_string());
        assert_eq!(all.len(), before, "no phantom row invented");
        assert!(all.iter().all(|w| w.name != "X"));
    }

    #[test]
    fn the_last_workspace_cannot_be_deleted() {
        let (store, _dir) = tmp_store();
        // A fresh store already carries the default workspace (id "local"),
        // so THAT is the last one — deleting it must be refused.
        assert_eq!(store.list_workspaces("local".to_string()).len(), 1);
        assert!(!store.delete_workspace_cascade("local".into()));
        assert_eq!(
            store.list_workspaces("local".to_string()).len(),
            1,
            "the device must always have somewhere to put a page",
        );
    }

    #[test]
    fn deleting_a_workspace_takes_its_views_and_documents() {
        let (store, _dir) = tmp_store();
        store.save_workspace(ws("keep", "Keep", "0000000010"));
        store.save_workspace(ws("drop", "Drop", "0000000020"));
        // Two views in the doomed workspace, one in the survivor.
        for (id, wsid) in [("v1", "drop"), ("v2", "drop"), ("v3", "keep")] {
            store.save_view(LocalView {
                id: id.into(),
                workspace_id: wsid.into(),
                parent_id: None,
                object_id: format!("doc_{id}"),
                name: id.into(),
                position: "0000000010".into(),
                trashed: false,
                origin: "local".into(),
                object_type: "document".into(),
            });
            store.save_doc(format!("doc_{id}"), &MicaDocument::from_markdown("x".into()));
        }

        assert!(store.delete_workspace_cascade("drop".into()));
        assert!(
            store.list_workspaces("local".to_string()).iter().all(|w| w.id != "drop"),
            "the workspace row itself is gone",
        );
        let views = store.list_views("local".to_string());
        assert_eq!(views.len(), 1, "its views went with it");
        assert_eq!(views[0].id, "v3");
        assert!(store.load_doc("doc_v1".into()).is_none(), "orphaned docs would be unreachable forever");
        assert!(store.load_doc("doc_v3".into()).is_some(), "the survivor is untouched");
    }

    #[test]
    fn reorder_workspaces_renumbers_by_ten() {
        let (store, _dir) = tmp_store();
        store.save_workspace(ws("a", "A", "0000000010"));
        store.save_workspace(ws("b", "B", "0000000020"));
        store.reorder_workspaces(vec!["b".into(), "a".into()]);
        let all = store.list_workspaces("local".to_string());
        assert_eq!(all.iter().find(|w| w.id == "b").unwrap().position, "0000000010");
        assert_eq!(all.iter().find(|w| w.id == "a").unwrap().position, "0000000020");
    }

    #[test]
    fn a_created_workspace_lands_where_reorder_would_put_it() {
        // The step-of-ten rule is shared; these must not drift apart.
        let (store, _dir) = tmp_store();
        store.save_workspace(ws("a", "A", "0000000010"));
        store.save_workspace(ws("b", "B", "0000000020"));
        store.reorder_workspaces(vec!["a".into(), "b".into()]);
        let id = store.create_workspace("C".into());
        let all = store.list_workspaces("local".to_string());
        assert_eq!(all.iter().find(|w| w.id == id).unwrap().position, "0000000030");
    }

}
