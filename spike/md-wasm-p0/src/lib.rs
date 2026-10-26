//! P0 spike: the existing Mica Markdown engine behind a wasm-bindgen edge.
//!
//! Only `parseBody` exists — enough to answer T2.1's size / latency / speed
//! questions. The production crate (`crates/md-wasm`, T2.3) forwards the whole
//! `mica_md::wire` table instead.

use serde_json::{Map, Value};
use wasm_bindgen::prelude::*;

/// `import_markdown` → root block children → `[BlockJson]` (02 §3.2).
#[wasm_bindgen(js_name = parseBody)]
pub fn parse_body(markdown: &str) -> Result<String, JsError> {
    let doc = mica_markdown::import_markdown(markdown, "root");
    let root = doc
        .blocks
        .iter()
        .find(|block| block.id == doc.root_block_id)
        .ok_or_else(|| JsError::new("root block missing"))?;

    let index: std::collections::HashMap<_, _> =
        doc.blocks.iter().map(|block| (block.id.as_str(), block)).collect();
    let mut out = Vec::with_capacity(root.children.len());
    for id in &root.children {
        let Some(block) = index.get(id.as_str()) else {
            continue;
        };
        let mut obj = Map::new();
        obj.insert("type".into(), Value::String(block.kind.clone()));
        obj.insert("text".into(), Value::String(block.text.clone()));
        // `data` is an object when present; empty when absent (02 §3.2).
        let data = if block.data.is_object() {
            block.data.clone()
        } else {
            Value::Object(Map::new())
        };
        obj.insert("data".into(), data);
        out.push(Value::Object(obj));
    }
    serde_json::to_string(&out).map_err(|e| JsError::new(&e.to_string()))
}