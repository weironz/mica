//! The web client's CRDT replica: mica-core's [`MicaDoc`] (yrs) behind
//! wasm-bindgen, with the method set and semantics of the desktop FFI
//! `MicaDocument` (`clients/mica_flutter/rust/src/api/document.rs`), so the
//! Dart adapters on both platforms are the same code over two transports.
//!
//! Until 2026-11-01 the browser ran yjs plus ~1,000 lines of Dart that
//! re-implemented these edit ops against it (props diffing, text splicing,
//! marks diffing). Every edit rule now has one definition, in mica-core.
//!
//! Panics: built with `panic = "abort"`, a panic is a wasm trap that surfaces
//! in JS as a thrown `RuntimeError`. The Dart wrapper turns that into the same
//! failure value the FFI returns. Updates reaching a browser have already been
//! applied by the server (which guards untrusted yrs input), so this is the
//! last line, not the first.
use mica_core::{Block, MicaDoc};
use wasm_bindgen::prelude::*;

/// One CRDT document replica.
#[wasm_bindgen]
pub struct MicaDocument {
  inner: MicaDoc,
}

#[wasm_bindgen]
impl MicaDocument {
  /// Decode a full state (lib0 v1). `undefined` when the bytes don't decode.
  /// The client id is random per replica, never pinned: every browser tab is
  /// its own CRDT actor (the web has no device identity to pin it to).
  #[wasm_bindgen(js_name = fromState)]
  pub fn from_state(bytes: &[u8]) -> Option<MicaDocument> {
    MicaDoc::from_update(bytes)
      .ok()
      .map(|inner| MicaDocument { inner })
  }

  /// A document built from a flat block list (JSON), rooted at `root_id`.
  /// `undefined` when the JSON is not a block list.
  #[wasm_bindgen(js_name = fromBlocksJson)]
  pub fn from_blocks_json(root_id: &str, blocks_json: &str) -> Option<MicaDocument> {
    let blocks: Vec<Block> = serde_json::from_str(blocks_json).ok()?;
    Some(MicaDocument {
      inner: MicaDoc::from_blocks(root_id, &blocks),
    })
  }

  #[wasm_bindgen(js_name = rootBlockId)]
  pub fn root_block_id(&self) -> String {
    self.inner.root_block_id()
  }

  #[wasm_bindgen(js_name = toBlocksJson)]
  pub fn to_blocks_json(&self) -> String {
    serde_json::to_string(&self.inner.to_blocks()).unwrap_or_else(|_| "[]".into())
  }

  /// Start recording which blocks change (mica-core `track_changes`).
  #[wasm_bindgen(js_name = trackChanges)]
  pub fn track_changes(&mut self) {
    self.inner.track_changes();
  }

  /// JSON array of the blocks whose content changed since the last call;
  /// `undefined` = read the whole document (`take_changed_block_ids`).
  #[wasm_bindgen(js_name = takeChangedBlockIds)]
  pub fn take_changed_block_ids(&mut self) -> Option<String> {
    self
      .inner
      .take_changed_block_ids()
      .map(|ids| serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into()))
  }

  /// The named blocks (JSON array of ids in, block list out); missing ids are
  /// left out.
  #[wasm_bindgen(js_name = blocksJson)]
  pub fn blocks_json(&self, ids_json: &str) -> String {
    let ids: Vec<String> = serde_json::from_str(ids_json).unwrap_or_default();
    serde_json::to_string(&self.inner.blocks_by_id(&ids)).unwrap_or_else(|_| "[]".into())
  }

  #[wasm_bindgen(js_name = encodeState)]
  pub fn encode_state(&self) -> Vec<u8> {
    self.inner.encode_state()
  }

  #[wasm_bindgen(js_name = stateVector)]
  pub fn state_vector(&self) -> Vec<u8> {
    self.inner.state_vector()
  }

  /// The delta from `state_vector` to now; empty when the vector is invalid
  /// (same as the FFI).
  #[wasm_bindgen(js_name = encodeDiffSince)]
  pub fn encode_diff_since(&self, state_vector: &[u8]) -> Vec<u8> {
    self.inner.encode_diff(state_vector).unwrap_or_default()
  }

  /// Merge a remote update; false when the bytes don't decode.
  #[wasm_bindgen(js_name = applyUpdate)]
  pub fn apply_update(&mut self, update: &[u8]) -> bool {
    self.inner.apply_update(update).is_ok()
  }

  #[wasm_bindgen(js_name = insertBlockJson)]
  pub fn insert_block_json(&mut self, parent_id: &str, index: u32, block_json: &str) {
    if let Ok(b) = serde_json::from_str::<Block>(block_json) {
      self.inner.insert_block(parent_id, index as usize, &b);
    }
  }

  #[wasm_bindgen(js_name = updateBlock)]
  pub fn update_block(
    &mut self,
    id: &str,
    kind: Option<String>,
    text: Option<String>,
    data_json: Option<String>,
  ) {
    let data: Option<serde_json::Value> = data_json
      .as_deref()
      .and_then(|s| serde_json::from_str(s).ok());
    self
      .inner
      .update_block_fields(id, kind.as_deref(), text.as_deref(), data.as_ref());
  }

  #[wasm_bindgen(js_name = deleteBlock)]
  pub fn delete_block(&mut self, id: &str, bring_children: bool) {
    self.inner.delete_block(id, bring_children);
  }

  #[wasm_bindgen(js_name = moveBlock)]
  pub fn move_block(&mut self, id: &str, new_parent: &str, index: u32) {
    self.inner.move_block(id, new_parent, index as usize);
  }

  #[wasm_bindgen(js_name = beginHistoryBatch)]
  pub fn begin_history_batch(&mut self) {
    self.inner.begin_history_batch();
  }

  #[wasm_bindgen(js_name = endHistoryBatch)]
  pub fn end_history_batch(&mut self) {
    self.inner.end_history_batch();
  }

  #[wasm_bindgen(js_name = resetHistory)]
  pub fn reset_history(&mut self) {
    self.inner.reset_history();
  }

  pub fn undo(&mut self) -> bool {
    self.inner.undo()
  }

  pub fn redo(&mut self) -> bool {
    self.inner.redo()
  }

  #[wasm_bindgen(js_name = canUndo)]
  pub fn can_undo(&self) -> bool {
    self.inner.can_undo()
  }

  #[wasm_bindgen(js_name = canRedo)]
  pub fn can_redo(&self) -> bool {
    self.inner.can_redo()
  }

  /// `[undo, redo]` stack depths.
  #[wasm_bindgen(js_name = historyDepths)]
  pub fn history_depths(&self) -> Vec<u32> {
    let (u, r) = self.inner.history_depths();
    vec![u, r]
  }
}

/// Bytes of this module's linear memory. Wasm memory only grows, so this is
/// the high-water mark; the browser suite opens and frees replicas in a loop
/// and expects it to stop growing (`web_doc_memory_test`).
#[wasm_bindgen(js_name = memoryBytes)]
pub fn memory_bytes() -> f64 {
  #[cfg(target_arch = "wasm32")]
  return (core::arch::wasm32::memory_size(0) * 65536) as f64;
  #[cfg(not(target_arch = "wasm32"))]
  0.0
}

/// Build identity, so the client can tell a stale bundle from this checkout.
#[wasm_bindgen(js_name = engineInfo)]
pub fn engine_info() -> String {
  serde_json::json!({
    "crate": "mica-doc-wasm",
    "source_fingerprint": env!("MICA_DOC_SOURCE_HASH"),
  })
  .to_string()
}
