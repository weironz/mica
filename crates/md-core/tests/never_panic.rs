//! The SDK surface ingests untrusted input on every path an embedder exposes:
//! Markdown to `parse_inline`, arbitrary JSON values to `decode_cell`, and raw
//! strings to the wire endpoints (which is where a network client lands).
//! A panic in any of them is a crash the embedder cannot catch — the property
//! here is simply: **never panic.** proptest shrinks any failure to a minimal
//! reproducer; copy that into a fixed regression test and fix the code.
//!
//! Case counts are kept at the parser file's level (256) so this stays a fast
//! regression gate, not a fuzzing run. The block parser has its own file
//! (`crates/markdown/tests/proptest_parse.rs`).

use mica_md::{decode_cell, parse_inline, wire};
use proptest::prelude::*;

/// Fragments that reach the inline scanner's deep branches: every delimiter
/// family, escape, entity, autolink shape, math form, raw HTML and CJK/emoji.
fn inlineish() -> impl Strategy<Value = String> {
  let token = prop_oneof![
    Just("*"),
    Just("**"),
    Just("_"),
    Just("__"),
    Just("~~"),
    Just("~"),
    Just("`"),
    Just("``"),
    Just("["),
    Just("]"),
    Just("("),
    Just(")"),
    Just("![").boxed(),
    Just("<"),
    Just(">"),
    Just("<https://a.b>"),
    Just("<u@e.com>"),
    Just("www.a.b"),
    Just("https://a.b/c"),
    Just("\\"),
    Just("\\*"),
    Just("$"),
    Just("$$"),
    Just("$x$"),
    Just("\\("),
    Just("\\)"),
    Just("&amp;"),
    Just("&#38;"),
    Just("|"),
    Just("\n"),
    Just("\r\n"),
    Just(" "),
    Just("\t"),
    Just("\u{0}"),
    Just("\u{feff}"),
    Just("重"),
    Just("\u{3000}"),
    Just("\u{1F600}"),
    Just("[^1]"),
    Just("[a](b \"t\")"),
    Just("<b>"),
    Just("</b>"),
    Just("<!--"),
  ];
  proptest::collection::vec(token, 0..96).prop_map(|parts| parts.concat())
}

/// An arbitrary JSON value, including the object shapes cells may carry.
fn cell_value() -> impl Strategy<Value = serde_json::Value> {
  let leaf = prop_oneof![
    Just(serde_json::Value::Null),
    any::<bool>().prop_map(serde_json::Value::Bool),
    any::<i64>().prop_map(|n| serde_json::Value::Number(n.into())),
    any::<f64>()
      .prop_filter("finite", |f| f.is_finite())
      .prop_map(|f| serde_json::Number::from_f64(f)
        .map_or(serde_json::Value::Null, serde_json::Value::Number)),
    // Text, including markdown-significant shapes.
    inlineish().prop_map(serde_json::Value::String),
  ];
  leaf.prop_recursive(2, 8, 4, |inner| {
    prop_oneof![
      proptest::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::Array),
      proptest::collection::btree_map(any::<String>(), inner, 0..4)
        .prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
    ]
  })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn parse_inline_never_panics(source in inlineish()) {
        let _ = parse_inline(&source);
    }

    #[test]
    fn decode_cell_never_panics(cell in cell_value()) {
        let _ = decode_cell(&cell);
    }

    /// The wire endpoints take raw strings from another language; a malformed
    /// one must be an `Err`, never a panic.
    #[test]
    fn wire_endpoints_never_panic(input in inlineish()) {
        let _ = wire::parse_body_json(&input, false);
        let _ = wire::parse_body_json(&input, true);
        let _ = wire::parse_inline_batch_json(&input);
        let _ = wire::decode_cells_json(&input);
        let _ = wire::encode_cells_json(&input);
        let _ = wire::serialize_body_json(&input, "{}");
        let _ = wire::serialize_inline_json(&input);
        let _ = wire::render_html_body_json(&input, "{}", "export");
        let _ = wire::render_html_body_json(&input, "{}", &input);
        let _ = wire::math_run_spans_json(&input);
        let _ = wire::parse_inline_math_json(&input);
    }
}
