//! I8: every wire offset is UTF-16, and each wire function has an error path.

use mica_md::{MdError, ParseMode, parse_body, wire};

#[test]
fn parse_body_json_matches_parse_body() {
  let md = "# T\n\nSome **bold** and `code`.";
  let json = wire::parse_body_json(md, false).expect("parse_body_json");
  let wire_blocks: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
  let blocks = parse_body(md, ParseMode::Document);
  assert_eq!(wire_blocks.len(), blocks.len());
  for (value, block) in wire_blocks.iter().zip(blocks) {
    assert_eq!(value["type"], block.kind);
    assert_eq!(value["text"], block.text);
    let mut data = if block.data.is_null() {
      serde_json::json!({})
    } else {
      block.data
    };
    // §3.4 comparisons normalize mark order: the wire keeps the parser's
    // emission order (it is semantic input to the serializer's tie-break),
    // so both sides sort before comparing here.
    let sort_marks = |data: &mut serde_json::Value| {
      if let Some(marks) = data
        .get_mut("marks")
        .and_then(serde_json::Value::as_array_mut)
      {
        marks.sort_by_key(|m| {
          (
            m["start"].as_u64().unwrap_or(0),
            m["end"].as_u64().unwrap_or(0),
            m["type"].as_str().unwrap_or("").to_string(),
          )
        });
      }
    };
    let mut wire_data = value["data"].clone();
    sort_marks(&mut data);
    sort_marks(&mut wire_data);
    assert_eq!(wire_data, data);
  }
}

#[test]
fn parse_inline_batch_and_cell_functions_round_trip() {
  let inline = wire::parse_inline_batch_json(r#"["a **b**","😀 **c**"]"#).unwrap();
  let parsed: Vec<serde_json::Value> = serde_json::from_str(&inline).unwrap();
  assert_eq!(parsed.len(), 2);
  assert_eq!(parsed[1]["text"], "😀 c");

  let cells = serde_json::json!(["**bold**", {"text": "x", "marks": [{"start": 0, "end": 1, "type": "bold"}]}]);
  let decoded = wire::decode_cells_json(&cells.to_string()).unwrap();
  let decoded: Vec<serde_json::Value> = serde_json::from_str(&decoded).unwrap();
  assert_eq!(decoded[0]["text"], "bold");
  assert_eq!(decoded[1]["marks"][0]["type"], "bold");

  let encoded = wire::encode_cells_json(&decoded_json()).unwrap();
  let encoded: Vec<serde_json::Value> = serde_json::from_str(&encoded).unwrap();
  assert!(encoded[0].is_string(), "mark-free cell stays a string");
}

fn decoded_json() -> String {
  serde_json::json!([
    {"text": "plain", "marks": []},
    {"text": "x", "marks": [{"start": 0, "end": 1, "type": "bold"}]}
  ])
  .to_string()
}

#[test]
fn serialize_body_and_inline_render_markdown() {
  let blocks = serde_json::json!([
    {"type": "heading", "text": "T", "data": {"level": 1}},
    {"type": "paragraph", "text": "a b", "data": {"marks": [{"start": 0, "end": 1, "type": "bold"}]}}
  ]);
  let md = wire::serialize_body_json(&blocks.to_string(), "{}").unwrap();
  assert_eq!(md, "# T\n\n**a** b");
  let inline = wire::serialize_inline_json(
    &serde_json::json!({"text": "a b", "marks": [{"start": 0, "end": 1, "type": "bold"}]})
      .to_string(),
  )
  .unwrap();
  assert_eq!(inline, "**a** b");
}

#[test]
fn render_html_body_profiles_and_errors() {
  let blocks = serde_json::json!([{"type": "paragraph", "text": "hi", "data": {}}]);
  let html = wire::render_html_body_json(&blocks.to_string(), "{}", "export").unwrap();
  assert!(html.contains("hi"));
  let clipboard = wire::render_html_body_json(&blocks.to_string(), "{}", "clipboard").unwrap();
  assert_eq!(html, clipboard, "S1 clipboard profile equals export");

  assert!(wire::render_html_body_json(&blocks.to_string(), "{}", "nope").is_err());
  assert!(wire::parse_inline_batch_json("not json").is_err());
  assert!(wire::decode_cells_json("{}").is_err());
  assert!(wire::encode_cells_json("{}").is_err());
  assert!(wire::serialize_body_json("{}", "{}").is_err());
  assert!(wire::serialize_inline_json("null").is_err());
}

#[test]
fn offsets_are_utf16_for_non_bmp_text() {
  // "😀" is 2 UTF-16 units; "𠀀" is also 2. The `$x$` run starts after them.
  let source = "😀𠀀 $x$";
  let spans: Vec<[usize; 2]> = serde_json::from_str(&wire::math_run_spans_json(source)).unwrap();
  assert_eq!(spans, vec![[5, 8]]);
  let inline: serde_json::Value =
    serde_json::from_str(&wire::parse_inline_math_json(source)).unwrap();
  assert_eq!(inline["text"], "😀𠀀 x");
  assert_eq!(inline["marks"][0]["start"], 5);
  assert_eq!(inline["marks"][0]["end"], 6);

  let parsed: serde_json::Value =
    serde_json::from_str(&wire::parse_inline_batch_json(r#"["😀**b**"]"#).unwrap()).unwrap();
  let marks = parsed[0]["marks"].as_array().unwrap();
  assert_eq!(marks[0]["start"], 2);
  assert_eq!(marks[0]["end"], 3);
}

#[test]
fn engine_info_reports_the_schema_and_a_fingerprint() {
  let info: serde_json::Value = serde_json::from_str(&wire::engine_info_json()).unwrap();
  assert_eq!(info["schema_version"], 1);
  assert_eq!(info["crate_version"], env!("CARGO_PKG_VERSION"));
  assert!(
    info["source_fingerprint"]
      .as_str()
      .is_some_and(|f| f.len() == 16)
  );
}

#[test]
fn wire_public_signatures_are_frozen() {
  let _: fn(&str, bool) -> Result<String, MdError> = wire::parse_body_json;
  let _: fn(&str) -> Result<String, MdError> = wire::parse_inline_batch_json;
  let _: fn(&str) -> Result<String, MdError> = wire::decode_cells_json;
  let _: fn(&str) -> Result<String, MdError> = wire::encode_cells_json;
  let _: fn(&str, &str) -> Result<String, MdError> = wire::serialize_body_json;
  let _: fn(&str) -> Result<String, MdError> = wire::serialize_inline_json;
  let _: fn(&str, &str, &str) -> Result<String, MdError> = wire::render_html_body_json;
  let _: fn(&str) -> String = wire::math_run_spans_json;
  let _: fn(&str) -> String = wire::parse_inline_math_json;
  let _: fn() -> String = wire::engine_info_json;
}

#[test]
fn block_json_marks_keep_parser_order_and_extra_data_survives() {
  // An enclosing emphasis is appended after the inner code mark by the parser,
  // so the raw order is `code` then `bold`. The wire PRESERVES that order:
  // the serializer's tie-break reads it (an equal-range mark emitted later is
  // the outer one). Re-sorting here corrupted deep-nesting round-trips.
  let json = wire::parse_body_json("**a `b`**", false).unwrap();
  let blocks: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
  let marks = blocks[0]["data"]["marks"].as_array().unwrap();
  let types: Vec<&str> = marks.iter().map(|m| m["type"].as_str().unwrap()).collect();
  assert_eq!(
    types,
    vec!["code", "bold"],
    "parser emission order is semantic"
  );
}

#[test]
fn encode_cells_keeps_mark_order_and_extra_fields() {
  let inlines = serde_json::json!([{
    "text": "abcdef",
    "marks": [
      {"start": 4, "end": 6, "type": "bold"},
      {"start": 0, "end": 1, "type": "italic"},
      {"start": 2, "end": 3, "type": "link", "href": "https://x.y", "title": "t"}
    ]
  }]);
  let out = wire::encode_cells_json(&inlines.to_string()).unwrap();
  let cells: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
  let marks = cells[0]["marks"].as_array().unwrap();
  let starts: Vec<u64> = marks.iter().map(|m| m["start"].as_u64().unwrap()).collect();
  assert_eq!(starts, vec![4, 0, 2], "cell encoding preserves input order");
  assert_eq!(marks[2]["href"], "https://x.y");
  assert_eq!(marks[2]["title"], "t");
}

#[test]
fn wire_round_trip_matches_native_export_on_equal_range_ties() {
  // Deep nesting (only the innermost 32 levels parse) produces marks that
  // share an exact range with different types (image/link/image). The
  // serializer's tie-break relies on the parser's emission order, so the
  // wire round-trip must not reorder marks before serializing: sorting by
  // (start, end, type) put the link second among equal-range marks and the
  // re-export wrapped the image in a link instead of the other way round.
  let root = env!("CARGO_MANIFEST_DIR");
  let md = std::fs::read_to_string(format!(
    "{root}/../markdown/tests/fixtures/conformance/43-inline-nesting-budget.md"
  ))
  .expect("fixture 43 must be reachable from md-core tests");
  let parsed = wire::parse_body_json(&md, false).unwrap();
  let back = wire::serialize_body_json(&parsed, "{}").unwrap();
  let native =
    mica_md::compat::export_markdown(&mica_md::compat::import_markdown(&md, "root")).unwrap();
  assert_eq!(back, native);
}
