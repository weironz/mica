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

/// A link whose LABEL is itself a URL.
///
/// This is what the clipboard hands back when you copy a link out of Mica and
/// paste it in again — the HTML flavor round-trips as `[url](url)` — and it
/// used to come out as literal text with only the parenthesised half turned
/// into a link (measured 2026-08-04: `text` was the whole `[…](…)` source and
/// the sole link mark sat on offsets 25–47).
///
/// The cause was the nested-link guard: CommonMark forbids a link inside a
/// link, and the guard asked "does this label parse to something with a link
/// mark". A GFM *extended autolink* produces exactly that mark, so a bare-URL
/// label counted as a nested link and the whole construct was refused. But the
/// nesting rule is about explicit link CONSTRUCTS — an autolink inside a label
/// never actually happens, because once the label is a link's text nothing
/// autolinks inside it. GitHub renders this as an ordinary link, and so do we
/// now.
#[test]
fn a_url_shaped_label_is_still_a_link() {
  use mica_markdown::import_markdown;

  let one = |md: &str| {
    let snap = import_markdown(md, "root");
    let b = snap
      .blocks
      .iter()
      .find(|b| b.id != "root")
      .expect("one block")
      .clone();
    let marks = b.data.get("marks").cloned().unwrap_or_default();
    (b.text, marks.to_string())
  };

  let (text, marks) = one("[https://github.com/a/b](https://github.com/a/b)\n");
  assert_eq!(
    text, "https://github.com/a/b",
    "the label becomes the link TEXT, not literal markdown source"
  );
  assert!(
    marks.contains("\"start\":0") && marks.contains("\"end\":22"),
    "one link mark spanning the whole label, got {marks}"
  );
  assert!(marks.contains("https://github.com/a/b"), "href kept: {marks}");

  // Differing label and href still works, and the label is not autolinked into
  // a second, nested link.
  let (text2, marks2) = one("[https://a.example](https://b.example)\n");
  assert_eq!(text2, "https://a.example");
  assert!(marks2.contains("https://b.example"), "href wins: {marks2}");
  assert_eq!(marks2.matches("\"type\"").count(), 1, "exactly one mark: {marks2}");

  // The rule it must NOT break: a genuinely nested EXPLICIT link is still
  // refused, per CommonMark.
  let (nested, _) = one("[a [b](/c) d](/e)\n");
  assert!(
    nested.contains('['),
    "an explicit link inside a label still refuses the outer link, got {nested:?}"
  );

  // And a bare URL on its own still autolinks — the flag is off only inside
  // the label check.
  let (bare, bare_marks) = one("see https://x.example here\n");
  assert_eq!(bare, "see https://x.example here");
  assert!(bare_marks.contains("https://x.example"), "{bare_marks}");
}

/// A single `~` between word characters is a RANGE, not a strikethrough.
///
/// Reported 2026-08-04: "1个汉字 ≈ 1~2个 Token，英文…1个单词 ≈ 1~2个 Token"
/// came back as "…≈ 1<del>2个 Token，…≈ 1</del>2个 Token" — the two tildes of
/// two separate ranges paired up with each other.
///
/// GitHub does exactly the same (verified against its own renderer), so this is
/// a DELIBERATE divergence, not a conformance fix. It is justified by the thing
/// GitHub does not have: Mica is WYSIWYG, so the tildes are consumed into a
/// mark and the writer cannot recover them by editing the text — while on
/// GitHub the source stays visible and escapable. See the note at the `~`
/// branch in `parse_inline_memo`.
///
/// **The same table exists in Dart** (`test/tilde_range_test.dart`). Rust is the
/// authority and Dart is the mirror; a case added here belongs there too.
#[test]
fn a_single_tilde_between_word_characters_is_a_range() {
  let one = |md: &str| {
    let snap = import_markdown(md, "root");
    let b = snap
      .blocks
      .into_iter()
      .find(|b| b.id != "root")
      .expect("one block");
    let marks = b.data.get("marks").cloned().unwrap_or(Value::Null);
    (b.text, marks.to_string())
  };

  // The report, verbatim.
  let (text, marks) = one("中文 1个汉字 ≈ 1~2个 Token，英文 1个单词 ≈ 1~2个 Token");
  assert!(text.contains("1~2个 Token，"), "tildes survive: {text:?}");
  assert!(!marks.contains("strike"), "no strike mark: {marks}");

  // Same shape with CJK numerals — `一~二` is the same range written the
  // other way, and the CJK flanking amendment must not let it through.
  let (text, marks) = one("大约 一~二 个，再来 一~二 个");
  assert!(!marks.contains("strike"), "CJK range is not strike: {marks} {text:?}");

  // What must KEEP working: a single tilde with word boundaries around it,
  // which is the GFM spec's own single-tilde form.
  let (_, marks) = one("Hello, ~there~ world!");
  assert!(marks.contains("strike"), "spaced single tilde still strikes: {marks}");

  // And the double tilde is untouched in every position, intraword included.
  let (_, marks) = one("~~Hi~~ and a1~~b~~c2");
  assert!(marks.contains("strike"), "double tilde still strikes: {marks}");
}
