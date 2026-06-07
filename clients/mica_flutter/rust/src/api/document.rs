//! flutter_rust_bridge surface for the offline document model (`mica-core`).
//!
//! `MicaDocument` is an opaque handle to a yrs-backed [`MicaDoc`]; Dart holds it
//! and calls edit operations. Blocks cross the boundary as JSON arrays — the
//! shape the editor already uses — so the editor binding (P2-M3) can adopt this
//! incrementally without a parallel block model.

use std::sync::Mutex;

use flutter_rust_bridge::frb;
use mica_core::{Block, Mark, MicaDoc};

#[frb(opaque)]
pub struct MicaDocument {
    inner: Mutex<MicaDoc>,
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
