use wasm_bindgen::prelude::*;

#[cfg(feature = "preview-math")]
#[wasm_bindgen]
pub fn render_math(source: &str, display: bool, color: &str) -> Option<String> {
  let math = mica_md::render::render_math_svg(source, display, Some(color))?;
  serde_json::to_string(&math).ok()
}

#[cfg(feature = "preview-mermaid")]
#[wasm_bindgen]
pub fn render_mermaid(source: &str, theme: &str) -> Option<String> {
  let theme = serde_json::from_str(theme).ok()?;
  mica_md::render_mermaid_svg(source, Some(&theme))
}
