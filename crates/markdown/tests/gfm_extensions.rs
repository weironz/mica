//! GFM extension scoreboard: the 24 official examples from the extension
//! sections of the GFM spec (tables, task list items, strikethrough,
//! extended autolinks, disallowed raw HTML), vendored in
//! fixtures/gfm-extensions.json. Same contract as the CommonMark
//! scoreboard: a regression floor, not a gate.

use std::collections::BTreeMap;

use mica_markdown::{export_html, import_markdown};
use serde_json::Value;

const BASELINE_PASS: usize = 24; // 24/24 — GFM extensions complete, 2026-06-05

fn normalize(html: &str) -> String {
  html.trim_end().to_string()
}

#[test]
fn gfm_extension_scoreboard() {
  let spec_path = format!(
    "{}/tests/fixtures/gfm-extensions.json",
    env!("CARGO_MANIFEST_DIR")
  );
  let examples: Vec<Value> =
    serde_json::from_str(&std::fs::read_to_string(spec_path).unwrap()).unwrap();

  let mut per_section: BTreeMap<String, (usize, usize)> = BTreeMap::new();
  let mut passed = 0usize;
  for ex in &examples {
    let md = ex["markdown"].as_str().unwrap();
    let expected = ex["html"].as_str().unwrap();
    let section = ex["section"].as_str().unwrap().to_string();
    let got = export_html(&import_markdown(md, "root")).unwrap_or_default();
    let ok = normalize(&got) == normalize(expected);
    let entry = per_section.entry(section).or_insert((0, 0));
    entry.1 += 1;
    if ok {
      entry.0 += 1;
      passed += 1;
    } else if std::env::var("GFM_VERBOSE").is_ok() {
      println!("FAIL [{}]\nMD {:?}\nWANT {:?}\nGOT {:?}\n", ex["section"], md, expected, got);
    }
  }
  println!("GFM extensions: {passed}/{}", examples.len());
  for (sec, (ok, n)) in &per_section {
    println!("  {sec}: {ok}/{n}");
  }
  assert!(passed >= BASELINE_PASS, "GFM regressed: {passed} < {BASELINE_PASS}");
}
