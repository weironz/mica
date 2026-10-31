//! Synchronous Web bindings to the frozen Markdown wire contract.
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = parseBody)]
pub fn parse_body(markdown: &str, fragment: bool) -> Result<String, JsError> {
  mica_md::wire::parse_body_json(markdown, fragment).map_err(js_error)
}

#[wasm_bindgen(js_name = parseInlineBatch)]
pub fn parse_inline_batch(sources_json: &str) -> Result<String, JsError> {
  mica_md::wire::parse_inline_batch_json(sources_json).map_err(js_error)
}

#[wasm_bindgen(js_name = decodeCells)]
pub fn decode_cells(cells_json: &str) -> Result<String, JsError> {
  mica_md::wire::decode_cells_json(cells_json).map_err(js_error)
}

#[wasm_bindgen(js_name = encodeCells)]
pub fn encode_cells(inlines_json: &str) -> Result<String, JsError> {
  mica_md::wire::encode_cells_json(inlines_json).map_err(js_error)
}

#[wasm_bindgen(js_name = serializeBody)]
pub fn serialize_body(blocks_json: &str, assets_json: &str) -> Result<String, JsError> {
  mica_md::wire::serialize_body_json(blocks_json, assets_json).map_err(js_error)
}

#[wasm_bindgen(js_name = serializeInline)]
pub fn serialize_inline(inline_json: &str) -> Result<String, JsError> {
  mica_md::wire::serialize_inline_json(inline_json).map_err(js_error)
}

#[wasm_bindgen(js_name = renderHtmlBody)]
pub fn render_html_body(
  blocks_json: &str,
  image_srcs_json: &str,
  profile: &str,
) -> Result<String, JsError> {
  mica_md::wire::render_html_body_json(blocks_json, image_srcs_json, profile).map_err(js_error)
}

#[wasm_bindgen(js_name = mathRunSpans)]
pub fn math_run_spans(source: &str) -> String {
  mica_md::wire::math_run_spans_json(source)
}

#[wasm_bindgen(js_name = parseInlineMath)]
pub fn parse_inline_math(source: &str) -> String {
  mica_md::wire::parse_inline_math_json(source)
}

#[wasm_bindgen(js_name = properties)]
pub fn properties(request_json: &str) -> Result<String, JsError> {
  mica_md::wire::properties_json(request_json).map_err(js_error)
}

#[wasm_bindgen(js_name = engineInfo)]
pub fn engine_info() -> String {
  // The wire shape is shared; each running artifact identifies its own sources.
  let mut info: serde_json::Value =
    serde_json::from_str(&mica_md::wire::engine_info_json()).expect("engine info JSON");
  info["source_fingerprint"] = env!("MICA_MD_SOURCE_HASH").into();
  info.to_string()
}

fn js_error(error: mica_md::MdError) -> JsError {
  JsError::new(&error.to_string())
}
