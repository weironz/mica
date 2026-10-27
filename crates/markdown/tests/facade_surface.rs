//! The facade must keep every item it exported before the md-core split, with
//! the same signature: app-core re-exports this crate wholesale
//! (`pub use mica_markdown::*`), so a dropped or changed item breaks callers
//! we cannot see from here. Every line below fails to COMPILE on a mismatch.

use std::collections::BTreeMap;

use mica_markdown::*;

#[test]
fn every_pre_split_item_keeps_its_signature() {
  // Functions.
  let _: fn(&str, &str) -> DocumentSnapshotPayload = import_markdown;
  let _: fn(&str, &str) -> DocumentSnapshotPayload = import_markdown_fragment;
  let _: fn(&DocumentSnapshotPayload) -> DocumentOperationResult<String> = export_markdown;
  let _: fn(
    &DocumentSnapshotPayload,
    &BTreeMap<String, String>,
  ) -> DocumentOperationResult<String> = export_markdown_with_assets;
  let _: fn(&DocumentSnapshotPayload) -> DocumentOperationResult<String> = export_html;
  let _: fn(&DocumentSnapshotPayload, &str, u32) -> DocumentOperationResult<String> =
    export_html_document;
  let _: fn(&mut DocumentSnapshotPayload, &BTreeMap<String, String>) = set_image_srcs;
  let _: fn(&DocumentSnapshotPayload) -> Option<&str> = document_title;
  let _: fn(&mut DocumentSnapshotPayload, &str) -> bool = set_document_title;
  let _: fn(&str, &str) -> String = with_page_title;
  let _: fn(&str, &str) -> String = strip_page_title;
  let _: fn(&DocumentSnapshotPayload, &str) -> Option<usize> = block_index;
  let _: fn(&DocumentSnapshotPayload, &str, &str) -> bool = is_descendant;
  let _: fn(serde_json::Value) -> DocumentOperationResult<DocumentSnapshotPayload> =
    payload_from_value;
  let _: fn(&str) -> Vec<(usize, usize)> = math_run_spans;
  let _: fn(&str, Option<&MermaidHostTheme>) -> Option<String> = render_mermaid_svg;
  let _: fn(&str, &str, Option<&MermaidHostTheme>) -> Option<String> = render_mermaid_svg_with_id;

  // types / constants / methods.
  let _: &str = SAFE_HTML_CSP;
  let _: [(char, &str, &str); 4] = mica_markdown::JSON_ESCAPE_COLLISIONS;
  let _: fn(&str) -> Option<(&'static str, &'static str)> = mangled_latex_collision;
  let _: fn(&str, &str) -> String = mangled_latex_message;
  let _ = DocumentOperationError::BlockNotFound("x".to_string());
  let _: DocumentOperationResult<String> = Err(DocumentOperationError::UnsupportedSchemaVersion(0));
  let _ = Block::new("a", "paragraph")
    .with_text("t")
    .with_data(serde_json::Value::Null)
    .with_children(Vec::new());
  let _ = DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id: "root".to_string(),
    blocks: Vec::new(),
  };
  let _ = MermaidHostTheme {
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

  // properties (public module).
  use mica_markdown::properties::{
    Property, PropertyValue, parse_properties, remove_property, upsert_property,
  };
  let _: fn(&str) -> Vec<Property> = parse_properties;
  let _: fn(&str, &str, &PropertyValue) -> String = upsert_property;
  let _: fn(&str, &str) -> String = remove_property;
  let _ = PropertyValue::Text("t".to_string());
  let _ = Property {
    key: "k".to_string(),
    value: PropertyValue::Checkbox(true),
  };

  // text_edit.
  let _: fn(&str) -> Option<String> = markdown_find_text;
  let _: fn(&Block, &str) -> usize = block_text_matches;
  let _: fn(&Block, usize) -> String = block_text_preview;
  let _: fn(&Block, &str, &str, bool) -> Block = replace_block_text;
}
