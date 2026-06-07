//! flutter_rust_bridge surface for the on-device local store (P2-M2).
//!
//! `MicaStore` wraps `mica_core::LocalStore` (SQLite) plus the persisted device
//! identity. Dart opens it once (per local workspace dir), then saves/loads
//! `MicaDocument`s. Loaded docs automatically get this device's stable yrs
//! client id, so offline edits keep one consistent actor across restarts.

use std::sync::Mutex;

use flutter_rust_bridge::frb;
use mica_core::{LocalStore, LocalView as CoreView};

use crate::api::document::MicaDocument;

/// A page-tree node mirrored to Dart (P2-M3) — the local mirror of the client's
/// `DocumentView`. `object_id` is the document's `doc_id`.
pub struct LocalView {
    pub id: String,
    pub parent_id: Option<String>,
    pub object_id: String,
    pub name: String,
    pub position: String,
    pub trashed: bool,
}

impl From<CoreView> for LocalView {
    fn from(v: CoreView) -> Self {
        LocalView {
            id: v.id,
            parent_id: v.parent_id,
            object_id: v.object_id,
            name: v.name,
            position: v.position,
            trashed: v.trashed,
        }
    }
}

impl From<LocalView> for CoreView {
    fn from(v: LocalView) -> Self {
        CoreView {
            id: v.id,
            parent_id: v.parent_id,
            object_id: v.object_id,
            name: v.name,
            position: v.position,
            trashed: v.trashed,
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

    /// Persist a document under `doc_id` (full snapshot).
    #[frb(sync)]
    pub fn save_doc(&self, doc_id: String, doc: &MicaDocument) {
        let doc_guard = doc.inner.lock().unwrap();
        let _ = self.inner.lock().unwrap().save_doc(&doc_id, &doc_guard);
    }

    // ── page tree (views) — P2-M3 ────────────────────────────────────────────

    /// All views (including trashed), ordered by position. The client builds the
    /// tree from `parent_id` and filters trash.
    #[frb(sync)]
    pub fn list_views(&self) -> Vec<LocalView> {
        self.inner
            .lock()
            .unwrap()
            .list_views()
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

    /// Permanently remove a view row (delete its document via [`Self::delete_doc`]).
    #[frb(sync)]
    pub fn purge_view(&self, id: String) {
        let _ = self.inner.lock().unwrap().purge_view(&id);
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
}
