//! WebAssembly surface for `mica-board` — loaded lazily on demand when a page
//! renders or edits a `mica-board` block (0 B on normal Markdown pages).
use mica_board::{BoardSvgTheme, import_drawio_bytes, markdown_to_mindmap, mindmap_to_markdown};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn render_board_svg(source: &str, theme_json: Option<String>) -> Option<String> {
  let theme = theme_json
    .as_deref()
    .and_then(|raw| serde_json::from_str::<BoardSvgTheme>(raw).ok());
  mica_board::render_board_json_svg(source, theme.as_ref())
}

#[wasm_bindgen]
pub fn layout_mindmap(source: &str) -> Option<String> {
  mica_board::layout_mindmap_json(source)
}

#[wasm_bindgen]
pub fn route_board_edges(source: &str) -> Option<String> {
  mica_board::route_board_edges_json(source)
}

#[wasm_bindgen]
pub fn merge_board(base: &str, local: &str, remote: &str) -> String {
  mica_board::merge_board_json(base, local, remote)
}

#[wasm_bindgen]
pub fn import_drawio(bytes: &[u8]) -> Result<String, JsValue> {
  import_drawio_bytes(bytes)
    .map(|snap| snap.to_compact_json())
    .map_err(|e| JsValue::from_str(&e.to_string()))
}

#[wasm_bindgen]
pub fn markdown_to_mindmap_json(markdown: &str, title: Option<String>) -> String {
  markdown_to_mindmap(markdown, title.as_deref()).to_compact_json()
}

#[wasm_bindgen]
pub fn mindmap_json_to_markdown(source: &str) -> Option<String> {
  let snap = mica_board::BoardSnapshot::parse_json(source).ok()?;
  Some(mindmap_to_markdown(&snap))
}

#[wasm_bindgen]
pub fn starter_board(template: &str) -> String {
  mica_board::starter_board_json(template)
}
