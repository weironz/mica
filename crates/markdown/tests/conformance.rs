//! Shared-fixture conformance tests. The gold `.blocks.json` files are the
//! single source of truth for the markdown grammar: this test pins the Rust
//! engine to them, and the Dart mirror's
//! `test/markdown_conformance_test.dart` pins the client to the SAME files —
//! any grammar drift between the two implementations fails one of the sides.
//!
//! Regenerate golds after an intentional grammar change:
//! `GEN_GOLD=1 cargo test -p mica-markdown --test conformance`

use mica_markdown::import_markdown;
use serde_json::{Value, json};

fn fixture_dir() -> std::path::PathBuf {
  std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/conformance")
}

/// Parse a fixture into the language-neutral gold shape: the root's children
/// in order, each as `{kind, text, data}`.
fn parse_to_gold(markdown: &str) -> Value {
  let payload = import_markdown(markdown, "root");
  let root = payload
    .blocks
    .iter()
    .find(|b| b.id == payload.root_block_id)
    .expect("root block");
  let blocks: Vec<Value> = root
    .children
    .iter()
    .filter_map(|id| payload.blocks.iter().find(|b| &b.id == id))
    .map(|b| {
      // Language-neutral data shape: absent data is `{}`, and marks are
      // sorted by range — array order is semantically irrelevant and the
      // two implementations emit it differently.
      let mut data = if b.data.is_null() { json!({}) } else { b.data.clone() };
      if let Some(marks) = data.get_mut("marks").and_then(Value::as_array_mut) {
        marks.sort_by_key(|m| {
          (
            m.get("start").and_then(Value::as_u64).unwrap_or(0),
            m.get("end").and_then(Value::as_u64).unwrap_or(0),
            m.get("type").and_then(Value::as_str).unwrap_or("").to_string(),
          )
        });
      }
      json!({"kind": b.kind, "text": b.text, "data": data})
    })
    .collect();
  Value::Array(blocks)
}

#[test]
fn fixtures_match_gold() {
  let dir = fixture_dir();
  let generate = std::env::var("GEN_GOLD").is_ok();
  let mut checked = 0;
  let mut entries: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
  entries.sort_by_key(|e| e.file_name());
  for entry in entries {
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("md") {
      continue;
    }
    let markdown = std::fs::read_to_string(&path).unwrap();
    let got = parse_to_gold(&markdown);
    let gold_path = path.with_extension("blocks.json");
    if generate {
      std::fs::write(&gold_path, serde_json::to_string_pretty(&got).unwrap()).unwrap();
      continue;
    }
    let gold: Value =
      serde_json::from_str(&std::fs::read_to_string(&gold_path).unwrap_or_else(|_| {
        panic!("missing gold for {path:?} — run with GEN_GOLD=1 to create")
      }))
      .unwrap();
    assert_eq!(got, gold, "grammar drift in {path:?}");
    checked += 1;
  }
  if !generate {
    assert!(checked >= 10, "expected at least 10 fixtures, found {checked}");
  }
}

/// Round-trip stability: export(import(md)) parses back to the same blocks.
#[test]
fn fixtures_round_trip() {
  let dir = fixture_dir();
  for entry in std::fs::read_dir(&dir).unwrap().flatten() {
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("md") {
      continue;
    }
    let markdown = std::fs::read_to_string(&path).unwrap();
    let first = parse_to_gold(&markdown);
    let exported =
      mica_markdown::export_markdown(&import_markdown(&markdown, "root")).unwrap();
    let second = parse_to_gold(&exported);
    assert_eq!(first, second, "round-trip drift in {path:?}");
  }
}
