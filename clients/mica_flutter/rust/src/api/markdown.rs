//! Desktop codec: synchronous forwards to the shared Markdown wire API.
use flutter_rust_bridge::frb;

#[frb(sync)]
pub fn md_parse_body(markdown: String, fragment: bool) -> Result<String, String> {
  mica_md::wire::parse_body_json(&markdown, fragment).map_err(|e| e.to_string())
}

#[frb(sync)]
pub fn md_parse_inline_batch(sources_json: String) -> Result<String, String> {
  mica_md::wire::parse_inline_batch_json(&sources_json).map_err(|e| e.to_string())
}

#[frb(sync)]
pub fn md_decode_cells(cells_json: String) -> Result<String, String> {
  mica_md::wire::decode_cells_json(&cells_json).map_err(|e| e.to_string())
}

#[frb(sync)]
pub fn md_encode_cells(inlines_json: String) -> Result<String, String> {
  mica_md::wire::encode_cells_json(&inlines_json).map_err(|e| e.to_string())
}

#[frb(sync)]
pub fn md_serialize_body(blocks_json: String, assets_json: String) -> Result<String, String> {
  mica_md::wire::serialize_body_json(&blocks_json, &assets_json).map_err(|e| e.to_string())
}

#[frb(sync)]
pub fn md_serialize_inline(inline_json: String) -> Result<String, String> {
  mica_md::wire::serialize_inline_json(&inline_json).map_err(|e| e.to_string())
}

#[frb(sync)]
pub fn md_render_html_body(
  blocks_json: String,
  image_srcs_json: String,
  profile: String,
) -> Result<String, String> {
  mica_md::wire::render_html_body_json(&blocks_json, &image_srcs_json, &profile)
    .map_err(|e| e.to_string())
}

#[frb(sync)]
pub fn md_math_run_spans(source: String) -> String {
  mica_md::wire::math_run_spans_json(&source)
}

#[frb(sync)]
pub fn md_parse_inline_math(source: String) -> String {
  mica_md::wire::parse_inline_math_json(&source)
}

#[frb(sync)]
pub fn md_engine_info() -> String {
  mica_md::wire::engine_info_json()
}
