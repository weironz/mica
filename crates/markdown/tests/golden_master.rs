//! Byte-exact golden master for the block parser — the safety net for the
//! state-struct/handler decomposition of `import_markdown_inner`
//! (docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §8).
//!
//! The conformance suite (tests/fixtures/conformance/) pins a LOSSY projection
//! of the parse — root children only, marks sorted, nested blocks omitted.
//! A refactor of the block scanner can regress exactly the parts that
//! projection drops (nested block payloads, mark emission order, data shapes).
//! This test instead snapshots the FULL `DocumentSnapshotPayload` of every
//! corpus input, both entry points, and fails on any byte difference.
//!
//! Regenerate after an INTENTIONAL grammar change:
//!   UPDATE_GOLDEN=1 cargo test -p mica-markdown --test golden_master
//!
//! The corpus: the 652 CommonMark 0.31.2 spec examples, the GFM extension
//! cases, every curated fixture under tests/fixtures/conformance/ + paste.md,
//! and a set of adversarial inputs encoding the parser's past incidents
//! (NBSP-indent infinite loop, fragment front-matter swallow, …).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use mica_markdown::{DocumentSnapshotPayload, import_markdown, import_markdown_fragment};

fn fixtures_dir(relative: &str) -> std::path::PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// The parser's historical incidents, kept executable: each entry is an input
/// that once broke this engine (see the git log around it), so a refactor that
/// regresses any of them fails by name.
fn adversarial_inputs() -> Vec<(String, String)> {
  vec![
    ("adv-empty", String::new()),
    ("adv-only-blanks", "\n\n\n  \n\t\n".to_string()),
    ("adv-lone-hr", "---\n".to_string()),
    (
      "adv-front-matter-unterminated",
      "---\ntitle: swallowed\n".to_string(),
    ),
    (
      "adv-nbsp-indent-list",
      "- a\n\u{00a0}- b\n\u{00a0}  - c\n".to_string(),
    ),
    ("adv-nbsp-paragraph", "a\n\u{00a0}b\n".to_string()),
    (
      "adv-crlf",
      "# T\r\n\r\npara\r\n\r\n- one\r\n- two\r\n".to_string(),
    ),
    ("adv-lone-cr", "a\rb\r".to_string()),
    (
      "adv-tab-columns",
      ">\tfoo\n-\t\tbar\n\n*\tqux\n".to_string(),
    ),
    ("adv-unterminated-fence", "```\nno close\n".to_string()),
    (
      "adv-fence-in-list",
      "- item\n\n  ```\n  code\n\n  still code\n".to_string(),
    ),
    ("adv-deep-nesting", "- a\n".repeat(30)),
    ("adv-empty-quote-group", ">\n> \n\ntext\n".to_string()),
    ("adv-alert-bodyless", "> [!NOTE]\n\nafter\n".to_string()),
    (
      "adv-alert-title-run",
      "> [!TIP]\n> bold start\n> more\n".to_string(),
    ),
    (
      "adv-lazy-continuation",
      "> quote\nlazy\n\n- item\nlazy too\n".to_string(),
    ),
    (
      "adv-loose-tight-mix",
      "- a\n\n- b\n- c\n\n- d\n".to_string(),
    ),
    (
      "adv-marker-change",
      "- a\n+ b\n* c\n\n1. d\n1) e\n".to_string(),
    ),
    (
      "adv-indent-code-in-list",
      "- a\n\n    indented? no: list item para\n\n    - nested\n".to_string(),
    ),
    (
      "adv-top-code-after-list",
      "- a\n\n    code? no: continuation\n\n    ```\n    fence in item\n".to_string(),
    ),
    ("adv-quote-interrupts", "para\n> quote\npara2\n".to_string()),
    (
      "adv-quote-def-mixed",
      "[a]: /x\n\n> [a]: /y\n\n[a]\n".to_string(),
    ),
    (
      "adv-setext-multi",
      "one\ntwo\n===\n\nthree\n---\n".to_string(),
    ),
    ("adv-table-no-scan", "| a | b |\nno separator\n".to_string()),
    ("adv-ol-start", "5. five\n6. six\n\n1) one\n".to_string()),
    (
      "adv-html-block-7",
      "<!-- comment\nstill comment\n-->\n\ntext\n".to_string(),
    ),
    (
      "adv-footnote-def",
      "text[^1]\n\n[^1]: note body\n    continued\n".to_string(),
    ),
    (
      "adv-nested-quote-depth",
      ">>> deep\n>> deeper\n> base\n".to_string(),
    ),
    (
      "adv-item-children-para",
      "- code follows\n\n  ```\n  x\n  ```\n\n  para after code\n".to_string(),
    ),
  ]
  .into_iter()
  .map(|(id, s)| (id.to_string(), s))
  .collect()
}

fn spec_cases(file: &str, prefix: &str) -> Vec<(String, String)> {
  let raw = fs::read_to_string(fixtures_dir(file)).expect("spec file");
  let value: serde_json::Value = serde_json::from_str(&raw).expect("spec json");
  value
    .as_array()
    .expect("spec array")
    .iter()
    .enumerate()
    .map(|(i, case)| {
      // The CommonMark spec carries an `example` number; the GFM extension
      // file does not, so fall back to the array index — ids only need to be
      // STABLE, not meaningful.
      let n = case
        .get("example")
        .and_then(|v| v.as_u64())
        .unwrap_or(i as u64);
      let md = case.get("markdown").and_then(|v| v.as_str()).unwrap_or("");
      (format!("{prefix}-{n}"), md.to_string())
    })
    .collect()
}

/// Every input the parser is pinned against, with a stable id.
fn corpus() -> Vec<(String, String)> {
  let mut all: Vec<(String, String)> = Vec::new();
  all.extend(spec_cases(
    "tests/fixtures/commonmark-spec-0.31.2.json",
    "spec",
  ));
  all.extend(spec_cases("tests/fixtures/gfm-extensions.json", "gfm"));
  let conf = fixtures_dir("tests/fixtures/conformance");
  let mut paths: Vec<_> = fs::read_dir(&conf)
    .expect("conformance dir")
    .filter_map(|e| e.ok())
    .map(|e| e.path())
    .filter(|p| p.extension().is_some_and(|x| x == "md"))
    .collect();
  paths.sort();
  for p in paths {
    let stem = p.file_stem().unwrap().to_string_lossy().to_string();
    all.push((format!("fixture-{stem}"), fs::read_to_string(&p).unwrap()));
  }
  all.push((
    "paste".to_string(),
    fs::read_to_string(fixtures_dir("tests/fixtures/paste.md")).unwrap(),
  ));
  all.push((
    "vhost-user".to_string(),
    fs::read_to_string(fixtures_dir("tests/data/vhost-user-42.md")).unwrap(),
  ));
  all.extend(adversarial_inputs());
  all
}

/// Snapshot one input through BOTH entry points: the document path parses
/// front matter, the fragment path must not (a leading `---` there is body,
/// not a fence header — that distinction once swallowed content).
///
/// Block ids are RANDOM UUIDs at parse time (`block_<uuid>`), so a raw dump
/// would differ run-to-run. They are also not parse semantics — consumers
/// walk the tree. Renumber them by first appearance (`b0`, `b1`, …, the root
/// by its given id) so the snapshot pins the STRUCTURE, not the dice.
fn snapshot(md: &str) -> serde_json::Value {
  let canon = |payload: &DocumentSnapshotPayload| {
    let mut value = serde_json::to_value(payload).expect("serialize payload");
    let mut ids: Vec<(String, String)> =
      vec![(payload.root_block_id.clone(), payload.root_block_id.clone())];
    if let Some(blocks) = value.get_mut("blocks").and_then(|v| v.as_array_mut()) {
      for b in blocks.iter() {
        if let Some(id) = b.get("id").and_then(|v| v.as_str()).map(str::to_string) {
          let next = format!("b{}", ids.len());
          if !ids.iter().any(|(from, _)| *from == id) {
            ids.push((id, next));
          }
        }
      }
      let remap = |s: &str| -> String {
        ids
          .iter()
          .find(|(from, _)| from == s)
          .map(|(_, to)| to.clone())
          .unwrap_or_else(|| s.to_string())
      };
      for b in blocks.iter_mut() {
        if let Some(id) = b.get("id").and_then(|v| v.as_str()).map(str::to_string) {
          *b.get_mut("id").unwrap() = serde_json::Value::String(remap(&id));
        }
        if let Some(children) = b.get_mut("children").and_then(|v| v.as_array_mut()) {
          for c in children {
            if let Some(s) = c.as_str().map(str::to_string) {
              *c = serde_json::Value::String(remap(&s));
            }
          }
        }
      }
    }
    value
  };
  let full = import_markdown(md, "root");
  let frag = import_markdown_fragment(md, "root");
  serde_json::json!({
    "full": canon(&full),
    "fragment": canon(&frag),
  })
}

#[test]
fn golden_master() {
  let path = fixtures_dir("tests/fixtures/golden-master.json");
  let corpus = corpus();
  assert!(!corpus.is_empty(), "corpus must not be empty");

  let current: BTreeMap<String, serde_json::Value> = corpus
    .iter()
    .map(|(id, md)| (id.clone(), snapshot(md)))
    .collect();

  if std::env::var("UPDATE_GOLDEN").is_ok() {
    let pretty = format!(
      "{:#}\n",
      serde_json::Value::Object(current.into_iter().collect(),)
    );
    // serde_json's Map for `json!`/`Value::Object` is BTree-backed (no
    // preserve_order feature in this workspace), so the file is deterministic.
    fs::write(&path, pretty).expect("write golden");
    panic!(
      "golden master REGENERATED ({} inputs) — review the diff and commit it",
      corpus.len()
    );
  }

  let raw = fs::read_to_string(&path).expect(
    "golden master missing — run: UPDATE_GOLDEN=1 cargo test -p mica-markdown --test golden_master",
  );
  let golden: serde_json::Value = serde_json::from_str(&raw).expect("golden json");
  let golden_map = golden.as_object().expect("golden object");

  let mut missing = 0;
  let mut changed = Vec::new();
  for (id, snap) in &current {
    match golden_map.get(id) {
      None => {
        missing += 1;
        if changed.len() < 20 {
          changed.push(id.clone());
        }
      }
      Some(old) if old != snap && changed.len() < 20 => {
        changed.push(format!("{id} (output differs)"));
      }
      _ => {}
    }
  }
  let extra: Vec<&String> = golden_map
    .keys()
    .filter(|k| !current.contains_key(k.as_str()))
    .collect();

  assert!(
    changed.is_empty() && missing == 0 && extra.is_empty(),
    "golden master mismatch: {} changed/missing {:?}, {} extra: {:?} (of {} inputs)",
    changed.len() + missing as usize,
    changed,
    extra.len(),
    extra.iter().take(5).collect::<Vec<_>>(),
    current.len(),
  );
}
