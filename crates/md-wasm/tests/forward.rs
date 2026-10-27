//! These tests invoke the actual exports, not a parallel binding implementation.
use mica_md::wire;
use mica_md_wasm as binding;

const INLINE: &str = r#"{"text":"😀 bold 𠀀","marks":[{"start":3,"end":7,"type":"bold"}]}"#;

#[test]
fn parse_body_forwards_both_modes_and_conformance_fixtures() {
  for entry in std::fs::read_dir(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/markdown/tests/fixtures/conformance"
  ))
  .unwrap()
  {
    let path = entry.unwrap().path();
    if path.extension().is_some_and(|ext| ext == "md") {
      let source = std::fs::read_to_string(path).unwrap();
      for fragment in [false, true] {
        assert_eq!(
          binding::parse_body(&source, fragment).unwrap(),
          wire::parse_body_json(&source, fragment).unwrap()
        );
      }
    }
  }
  for fragment in [false, true] {
    let source = "---\ntitle: metadata\n---\n\n# 😀 *heading*\n\n|a|b|\n|-|-|\n|**x**|𠀀|";
    assert_eq!(
      binding::parse_body(source, fragment).unwrap(),
      wire::parse_body_json(source, fragment).unwrap()
    );
  }
}

#[test]
fn parse_inline_batch_forwards() {
  let sources = r#"["**bold** and 😀", "$x$ 𠀀", "[link](https://example.org)", ""]"#;
  assert_eq!(
    binding::parse_inline_batch(sources).unwrap(),
    wire::parse_inline_batch_json(sources).unwrap()
  );
}

#[test]
fn decode_cells_forwards() {
  let cells = r#"["**bold**", {"text":"😀","marks":[]}, null, "a\\|b", ""]"#;
  assert_eq!(
    binding::decode_cells(cells).unwrap(),
    wire::decode_cells_json(cells).unwrap()
  );
}

#[test]
fn encode_cells_forwards() {
  let inlines = format!("[{INLINE}]");
  assert_eq!(
    binding::encode_cells(&inlines).unwrap(),
    wire::encode_cells_json(&inlines).unwrap()
  );
}

#[test]
fn serialize_body_forwards() {
  let blocks = wire::parse_body_json("# Heading\n\n![alt](url)\n\n* item", false).unwrap();
  let assets = r#"{"image":"images/local.png"}"#;
  assert_eq!(
    binding::serialize_body(&blocks, assets).unwrap(),
    wire::serialize_body_json(&blocks, assets).unwrap()
  );
}

#[test]
fn serialize_inline_forwards() {
  assert_eq!(
    binding::serialize_inline(INLINE).unwrap(),
    wire::serialize_inline_json(INLINE).unwrap()
  );
}

#[test]
fn render_html_body_forwards_both_profiles() {
  let blocks = wire::parse_body_json(
    "# 😀\n\n$x$ and **bold**\n\n```mermaid\ngraph TD; A-->B\n```",
    false,
  )
  .unwrap();
  for profile in ["export", "clipboard"] {
    assert_eq!(
      binding::render_html_body(&blocks, "{}", profile).unwrap(),
      wire::render_html_body_json(&blocks, "{}", profile).unwrap()
    );
  }
}

#[test]
fn math_run_spans_forwards_utf16() {
  let source = "😀 $x$ and 𠀀 \\(y\\)";
  assert_eq!(
    binding::math_run_spans(source),
    wire::math_run_spans_json(source)
  );
}

#[test]
fn parse_inline_math_forwards() {
  for source in ["😀 $x$ and 𠀀 \\(y\\)", "$$block$$", "", "no math"] {
    assert_eq!(
      binding::parse_inline_math(source),
      wire::parse_inline_math_json(source)
    );
  }
}

#[test]
fn engine_info_preserves_metadata_and_stamps_binding_sources() {
  let mut expected: serde_json::Value = serde_json::from_str(&wire::engine_info_json()).unwrap();
  expected["source_fingerprint"] = env!("MICA_MD_SOURCE_HASH").into();
  assert_eq!(binding::engine_info(), expected.to_string());
}
