//! flutter_rust_bridge surface for the offline document model (`mica-core`).
//!
//! `MicaDocument` is an opaque handle to a yrs-backed [`MicaDoc`]; Dart holds it
//! and calls edit operations. Blocks cross the boundary as JSON arrays — the
//! shape the editor already uses — so the editor binding (P2-M3) can adopt this
//! incrementally without a parallel block model.

use std::sync::Mutex;

use flutter_rust_bridge::frb;
use mica_core::{marks_from_data, Block, Mark, MicaDoc};

#[frb(opaque)]
pub struct MicaDocument {
    // crate-visible so the local store (api::store) can save/load it.
    pub(crate) inner: Mutex<MicaDoc>,
}

impl MicaDocument {
    /// Build a document from a root id and a JSON array of blocks.
    #[frb(sync)]
    pub fn from_blocks_json(root_id: String, blocks_json: String) -> MicaDocument {
        let blocks: Vec<Block> = serde_json::from_str(&blocks_json).unwrap_or_default();
        MicaDocument {
            inner: Mutex::new(MicaDoc::from_blocks(&root_id, &blocks)),
        }
    }

    /// Rebuild from an encoded yrs state (the local snapshot). Returns null if
    /// the bytes don't decode.
    #[frb(sync)]
    pub fn from_state(bytes: Vec<u8>) -> Option<MicaDocument> {
        MicaDoc::from_update(&bytes)
            .ok()
            .map(|d| MicaDocument { inner: Mutex::new(d) })
    }

    /// The document as a JSON array of blocks (tree order).
    #[frb(sync)]
    pub fn to_blocks_json(&self) -> String {
        serde_json::to_string(&self.inner.lock().unwrap().to_blocks())
            .unwrap_or_else(|_| "[]".into())
    }

    /// Encode the full document state (the base snapshot to persist locally).
    #[frb(sync)]
    pub fn encode_state(&self) -> Vec<u8> {
        self.inner.lock().unwrap().encode_state()
    }

    #[frb(sync)]
    pub fn root_block_id(&self) -> String {
        self.inner.lock().unwrap().root_block_id()
    }

    #[frb(sync)]
    pub fn insert_block_json(&self, parent_id: String, index: u32, block_json: String) {
        if let Ok(b) = serde_json::from_str::<Block>(&block_json) {
            self.inner
                .lock()
                .unwrap()
                .insert_block(&parent_id, index as usize, &b);
        }
    }

    #[frb(sync)]
    pub fn update_block_kind(&self, id: String, kind: String) {
        self.inner.lock().unwrap().update_block_kind(&id, &kind);
    }

    /// Mirror the editor's coarse `update_block` op: apply any subset of
    /// kind/text/data to a block in one call. Inline marks travel *inside* the
    /// editor's `data` (`data["marks"]`) — when `text` is given they are applied
    /// to the (replaced) text as yrs formatting; `set_block_data` then stores the
    /// non-marks props. This is the single chokepoint the desktop op stream funnels
    /// through, so the on-device yrs doc tracks every edit (P2-M3).
    #[frb(sync)]
    pub fn update_block(
        &self,
        id: String,
        kind: Option<String>,
        text: Option<String>,
        data_json: Option<String>,
    ) {
        let mut doc = self.inner.lock().unwrap();
        if let Some(k) = kind {
            doc.update_block_kind(&id, &k);
        }
        let data: Option<serde_json::Value> =
            data_json.as_deref().and_then(|s| serde_json::from_str(s).ok());
        if let Some(t) = text {
            // Text changed: set text + its marks together.
            let marks = data.as_ref().map(marks_from_data).unwrap_or_default();
            doc.set_block_text(&id, &t, &marks);
        } else if let Some(d) = &data {
            // Data-only update (e.g. a turn-into resetting data): reconcile the
            // marks to whatever `data` now says — clearing them if it has none.
            doc.set_block_marks(&id, &marks_from_data(d));
        }
        if let Some(d) = &data {
            doc.set_block_data(&id, d);
        }
    }

    #[frb(sync)]
    pub fn set_block_data_json(&self, id: String, data_json: String) {
        if let Ok(data) = serde_json::from_str(&data_json) {
            self.inner.lock().unwrap().set_block_data(&id, &data);
        }
    }

    #[frb(sync)]
    pub fn text_insert(&self, id: String, at: u32, text: String) {
        self.inner.lock().unwrap().text_insert(&id, at, &text);
    }

    #[frb(sync)]
    pub fn text_delete(&self, id: String, at: u32, len: u32) {
        self.inner.lock().unwrap().text_delete(&id, at, len);
    }

    #[frb(sync)]
    pub fn text_format(
        &self,
        id: String,
        start: u32,
        end: u32,
        ty: String,
        href: Option<String>,
        title: Option<String>,
    ) {
        let mark = Mark {
            start,
            end,
            ty,
            href,
            title,
        };
        self.inner.lock().unwrap().text_format(&id, &mark);
    }

    #[frb(sync)]
    pub fn delete_block(&self, id: String, bring_children: bool) {
        self.inner.lock().unwrap().delete_block(&id, bring_children);
    }

    #[frb(sync)]
    pub fn move_block(&self, id: String, new_parent: String, index: u32) {
        self.inner
            .lock()
            .unwrap()
            .move_block(&id, &new_parent, index as usize);
    }

    #[frb(sync)]
    pub fn split_block(&self, id: String, at: u32, new_id: String, new_kind: String) {
        self.inner
            .lock()
            .unwrap()
            .split_block(&id, at, &new_id, &new_kind);
    }

    #[frb(sync)]
    pub fn join_into_prev(&self, id: String) {
        self.inner.lock().unwrap().join_into_prev(&id);
    }
}
