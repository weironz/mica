//! I6: the schema constants cover everything the engine emits.

use std::collections::BTreeSet;
use std::path::Path;

use mica_md::schema::{data_keys, kinds, marks};
use mica_md::{ParseOptions, parse};

fn corpus() -> Vec<(String, String)> {
  let mut out = Vec::new();
  let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../markdown/tests/fixtures");
  for (file, prefix) in [
    ("commonmark-spec-0.31.2.json", "spec"),
    ("gfm-extensions.json", "gfm"),
  ] {
    let raw = std::fs::read_to_string(base.join(file)).expect("spec file");
    let cases: Vec<serde_json::Value> = serde_json::from_str(&raw).expect("spec json");
    for (i, case) in cases.iter().enumerate() {
      out.push((
        format!("{prefix}:{i}"),
        case["markdown"].as_str().unwrap_or("").to_string(),
      ));
    }
  }
  let conf = base.join("conformance");
  let mut paths: Vec<_> = std::fs::read_dir(conf)
    .expect("conformance")
    .filter_map(|e| e.ok())
    .map(|e| e.path())
    .filter(|p| p.extension().is_some_and(|x| x == "md"))
    .collect();
  paths.sort();
  for path in paths {
    out.push((
      format!("conf:{}", path.file_name().unwrap().to_string_lossy()),
      std::fs::read_to_string(&path).unwrap(),
    ));
  }
  out
}

/// Inputs for schema corners the shared corpus does not exercise.
const EXTRA: &[&str] = &[
  "- [x] done\n- [ ] todo\n",
  "> [!WARNING]\n> careful\n",
  "text[^1]\n\n[^1]: note\n",
  "![alt](a.png \"t\")\n",
  "| a | b |\n| :- | -: |\n| 1 | 2 |\n",
  "```rust\nfn main() {}\n```\n",
  "<div>\nraw\n</div>\n",
  "$$\nx^2\n$$\n",
  "---\ntags: [a]\n---\nbody\n",
  "a $x$ b\n",
  "<span>html</span>\n",
];

#[test]
fn every_emitted_kind_key_and_mark_is_in_the_schema() {
  let kinds_set: BTreeSet<&str> = kinds::ALL.iter().copied().collect();
  let keys_set: BTreeSet<&str> = data_keys::ALL.iter().copied().collect();
  let marks_set: BTreeSet<&str> = marks::ALL.iter().copied().collect();
  let aliases: BTreeSet<&str> = kinds::ALIASES.iter().map(|(alias, _)| *alias).collect();

  let mut seen_kinds = BTreeSet::new();
  let mut seen_keys = BTreeSet::new();
  let mut seen_marks = BTreeSet::new();

  let mut inputs = corpus();
  inputs.extend(
    EXTRA
      .iter()
      .enumerate()
      .map(|(i, md)| (format!("extra:{i}"), md.to_string())),
  );
  for (id, markdown) in inputs {
    for block in parse(&markdown, &ParseOptions::default()).blocks {
      assert!(
        kinds_set.contains(block.kind.as_str()) || aliases.contains(block.kind.as_str()),
        "{id}: unknown kind {:?}",
        block.kind
      );
      seen_kinds.insert(block.kind.clone());
      if let Some(data) = block.data.as_object() {
        for key in data.keys() {
          assert!(
            keys_set.contains(key.as_str()),
            "{id}: unknown data key {key:?}"
          );
          seen_keys.insert(key.clone());
        }
        for mark in data
          .get("marks")
          .and_then(serde_json::Value::as_array)
          .into_iter()
          .flatten()
        {
          if let Some(kind) = mark.get("type").and_then(serde_json::Value::as_str) {
            assert!(marks_set.contains(kind), "{id}: unknown mark {kind:?}");
            seen_marks.insert(kind.to_string());
          }
        }
      }
    }
  }

  // Reverse direction: no dead constants. Items that exist only for the editor
  // or for compatibility are listed here rather than silently never seen.
  let editor_only_kinds = ["page"];
  for kind in kinds::ALL {
    assert!(
      seen_kinds.contains(*kind) || editor_only_kinds.contains(kind),
      "kind constant {kind:?} never appears in the corpus (dead entry or missing input)"
    );
  }
  let adapter_only_keys = ["title", "file_id", "name", "widths", "width", "aligns"];
  for key in data_keys::ALL {
    assert!(
      seen_keys.contains(*key) || adapter_only_keys.contains(key),
      "data key constant {key:?} never appears in the corpus"
    );
  }
  for mark in marks::ALL {
    assert!(
      seen_marks.contains(*mark),
      "mark constant {mark:?} never observed"
    );
  }
}
