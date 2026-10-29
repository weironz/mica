//! The SDK's public API, pinned: every item `mica_md` exports keeps its exact
//! signature. Any change fails to COMPILE here, which forces the changer to
//! also update this file — making the diff reviewable at a glance. Companion of
//! `mica-markdown`'s `facade_surface.rs` (which pins the adapter's own surface).
//!
//! Note `compat::` items are pinned too: the Mica adapter re-exports them
//! wholesale, so they are load-bearing even though the SDK docs steer new
//! embedders to the primitives.

use std::collections::BTreeMap;

use mica_md::wire::BlockJson;

#[test]
fn engine_primitives_keep_their_signatures() {
  // Parsing.
  let _: fn(&str, &mica_md::ParseOptions) -> mica_md::Document = mica_md::parse;
  let _: fn(&str, mica_md::ParseMode) -> Vec<mica_md::Block> = mica_md::parse_body;
  let _: fn(&str) -> mica_md::InlineText = mica_md::parse_inline;
  let _: fn(&str) -> mica_md::InlineText = mica_md::parse_inline_math;
  let _: fn(&str) -> Vec<(usize, usize)> = mica_md::math_run_spans;

  // Table cells.
  let _: fn(&serde_json::Value) -> mica_md::InlineText = mica_md::decode_cell;
  let _: fn(&mica_md::InlineText) -> serde_json::Value = mica_md::encode_cell;

  // Serialization.
  let _: fn(&mica_md::Document, &mica_md::SerializeOptions) -> Result<String, mica_md::MdError> =
    mica_md::serialize;
  let _: fn(&[mica_md::Block], &mica_md::SerializeOptions) -> String = mica_md::serialize_body;
  let _: fn(&str, &[mica_md::Mark]) -> String = mica_md::serialize_inline;

  // HTML.
  let _: fn(&mica_md::Document, &mica_md::HtmlOptions) -> Result<String, mica_md::MdError> =
    mica_md::render_html;
  let _: fn(&[mica_md::Block], &mica_md::HtmlOptions) -> String = mica_md::render_html_body;
}

#[test]
fn compat_entries_keep_their_signatures() {
  use mica_md::compat;
  let _: fn(&str, &str) -> mica_md::DocumentSnapshotPayload = compat::import_markdown;
  let _: fn(&str, &str) -> mica_md::DocumentSnapshotPayload = compat::import_markdown_fragment;
  let _: fn(&mica_md::DocumentSnapshotPayload) -> Result<String, mica_md::MdError> =
    compat::export_markdown;
  let _: fn(
    &mica_md::DocumentSnapshotPayload,
    &BTreeMap<String, String>,
  ) -> Result<String, mica_md::MdError> = compat::export_markdown_with_assets;
  let _: fn(&mica_md::DocumentSnapshotPayload) -> Result<String, mica_md::MdError> =
    compat::export_html;
  let _: fn(&mut mica_md::DocumentSnapshotPayload, &BTreeMap<String, String>) =
    compat::set_image_srcs;
  let _: fn(&str) -> (Option<String>, &str) = compat::split_front_matter;
}

#[test]
fn wire_json_endpoints_keep_their_signatures() {
  use mica_md::wire;
  let _: fn(&str, bool) -> Result<String, mica_md::MdError> = wire::parse_body_json;
  let _: fn(&str) -> Result<String, mica_md::MdError> = wire::parse_inline_batch_json;
  let _: fn(&str) -> Result<String, mica_md::MdError> = wire::decode_cells_json;
  let _: fn(&str) -> Result<String, mica_md::MdError> = wire::encode_cells_json;
  let _: fn(&str, &str) -> Result<String, mica_md::MdError> = wire::serialize_body_json;
  let _: fn(&str) -> Result<String, mica_md::MdError> = wire::serialize_inline_json;
  let _: fn(&str) -> String = wire::math_run_spans_json;
  let _: fn(&str) -> String = wire::parse_inline_math_json;
  let _: fn() -> String = wire::engine_info_json;
  // render_html_body_json carries its profile argument positionally.
  let _: fn(&str, &str, &str) -> Result<String, mica_md::MdError> = wire::render_html_body_json;
}

#[test]
fn model_types_keep_their_shapes() {
  // Constructors and builder methods.
  let _ = mica_md::Block::new("a", "paragraph")
    .with_text("t")
    .with_data(serde_json::Value::Null)
    .with_children(Vec::new());
  let _ = mica_md::DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id: "root".to_string(),
    blocks: Vec::new(),
  };
  let mark = mica_md::Mark {
    start: 0,
    end: 1,
    kind: "bold".to_string(),
    href: None,
    title: None,
  };
  let _: mica_md::InlineText = mica_md::InlineText {
    text: String::new(),
    marks: vec![mark],
  };
  let _ = mica_md::MdError::UnsupportedSchemaVersion(0);
  let _ = mica_md::MdError::BlockNotFound("x".to_string());
  let _ = mica_md::MdError::InvalidInput("y".to_string());
  let _: i32 = mica_md::SCHEMA_VERSION;

  // Options are plain data with public fields.
  let _ = mica_md::ParseOptions::default().root_id;
  let _ = mica_md::SerializeOptions {
    assets: BTreeMap::new(),
  }
  .assets;
  let _ = mica_md::HtmlOptions {
    image_srcs: BTreeMap::new(),
    profile: mica_md::HtmlProfile::Clipboard,
  }
  .profile;
  let _ = mica_md::HtmlProfile::Export;

  // Wire structs are deserializable (they ARE the cross-language format).
  let _: Result<BlockJson, _> = serde_json::from_str("{}");
  let _: mica_md::wire::InlineJson = mica_md::wire::InlineJson {
    text: String::new(),
    marks: Vec::new(),
  };

  // Mermaid renderers behind the `render` feature; the feature-off stubs must
  // keep the same signature (they return None).
  let _ = mica_md::render_mermaid_svg("graph TD; A--\u{2192}B;", None);
  let _ = mica_md::render_mermaid_svg_with_id("graph TD; A--\u{2192}B;", "id", None);
  let theme = mica_md::MermaidHostTheme {
    dark: false,
    canvas: String::new(),
    surface: String::new(),
    surface_alt: String::new(),
    text: String::new(),
    subtle_text: String::new(),
    border: String::new(),
    line: String::new(),
    error: String::new(),
    warning: String::new(),
    success: String::new(),
  };
  let _ = mica_md::render_mermaid_svg("graph TD; A--\u{2192}B;", Some(&theme));

  // ID strategies (`Uuid` exists only with the default `uuid` feature).
  #[cfg(feature = "uuid")]
  let _ = mica_md::BlockIds::Uuid;
  let _ = mica_md::BlockIds::Sequential {
    prefix: "b".to_string(),
  };
}

#[test]
fn schema_constants_are_complete_tables() {
  // Spot-check the public constant tables; full equality with SCHEMA.md is
  // schema_doc_sync.rs's job.
  assert!(mica_md::schema::kinds::ALL.contains(&"paragraph"));
  assert!(mica_md::schema::marks::ALL.contains(&"bold"));
  assert!(mica_md::schema::data_keys::ALL.contains(&"marks"));
}

#[test]
fn properties_module_keeps_its_surface() {
  use mica_md::properties;
  let _: fn(&str) -> Vec<properties::Property> = properties::parse_properties;
  let _: fn(&str, &str, &properties::PropertyValue) -> String = properties::upsert_property;
  let _: fn(&str, &str) -> String = properties::remove_property;
  let _ = properties::PropertyValue::Text(String::new());
}
