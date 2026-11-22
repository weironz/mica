//! WebAssembly surface for `mica-board` — loaded lazily on demand when a page
//! renders or edits a `mica-board` block (0 B on normal Markdown pages).
use mica_board::{
  BoardSession, BoardSvgTheme, import_drawio_bytes, markdown_to_mindmap, mindmap_to_markdown,
};
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

/// Stateful headless board interaction session for the Web canvas.
#[wasm_bindgen]
pub struct WasmBoardSession {
  inner: BoardSession,
}

#[wasm_bindgen]
impl WasmBoardSession {
  #[wasm_bindgen(constructor)]
  pub fn new(snapshot_json: &str) -> Self {
    Self {
      inner: BoardSession::from_json(snapshot_json),
    }
  }

  #[wasm_bindgen(js_name = currentStatePatchJson)]
  pub fn current_state_patch_json(&self) -> String {
    serde_json::to_string(&self.inner.current_state_patch()).unwrap_or_else(|_| "{}".into())
  }

  #[wasm_bindgen(js_name = dispatchJson)]
  pub fn dispatch_json(&mut self, cmd_json: &str) -> Result<String, JsValue> {
    self
      .inner
      .dispatch_json(cmd_json)
      .map_err(|e| JsValue::from_str(&e))
  }

  #[wasm_bindgen(js_name = pointerMoveJson)]
  pub fn pointer_move_json(
    &mut self,
    world_x: f64,
    world_y: f64,
    screen_x: f64,
    screen_y: f64,
    alt_pressed: bool,
  ) -> String {
    let patch = self
      .inner
      .pointer_move(world_x, world_y, screen_x, screen_y, alt_pressed);
    serde_json::to_string(&patch).unwrap_or_else(|_| "{}".into())
  }

  #[wasm_bindgen(js_name = exportCompactJson)]
  pub fn export_compact_json(&self) -> String {
    self.inner.export_compact_json()
  }

  #[wasm_bindgen(js_name = renderSvg)]
  pub fn render_svg(&self, theme_json: Option<String>) -> String {
    let theme = theme_json
      .as_deref()
      .and_then(|raw| serde_json::from_str::<BoardSvgTheme>(raw).ok());
    self.inner.render_svg(theme.as_ref())
  }
}
