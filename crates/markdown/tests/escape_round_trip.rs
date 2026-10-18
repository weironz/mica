//! Paragraph / quote text that merely LOOKS like block syntax must survive
//! export → import unchanged (CLAUDE.md principle #4: round-trip is an
//! invariant).
//!
//! Each case is text a user can type in the editor. Before the fix:
//! - `[TODO]` exported as `\[TODO\]`, which re-imports as DISPLAY MATH;
//! - leader escaping ran on the first line only, so after a hard break
//!   (`\` + newline, the editor's storage form) `- item` became a list;
//! - `1)`, a tab after the marker, and quote text were never escaped.

use mica_markdown::{export_markdown, import_markdown};

/// One block of `kind` holding exactly `text`, exported and re-imported.
fn round_trip(kind: &str, text: &str) -> Vec<(String, String)> {
  let mut payload = import_markdown("placeholder\n", "root");
  let block = payload
    .blocks
    .iter_mut()
    .find(|b| b.kind == "paragraph")
    .expect("a paragraph");
  block.kind = kind.to_string();
  block.text = text.to_string();
  let md = export_markdown(&payload).expect("export");
  import_markdown(&md, "root")
    .blocks
    .into_iter()
    .filter(|b| b.id != "root" && b.kind != "page")
    .map(|b| (b.kind, b.text))
    .collect()
}

fn assert_stable(kind: &str, text: &str) {
  assert_eq!(
    round_trip(kind, text),
    vec![(kind.to_string(), text.to_string())],
    "{kind} {text:?} changed on round trip"
  );
}

#[test]
fn a_bracketed_paragraph_does_not_become_math() {
  for text in ["[TODO]", "[1]", "[x]", "[a]\\\n[b]", "["] {
    assert_stable("paragraph", text);
  }
}

#[test]
fn every_line_of_a_paragraph_keeps_its_leader_escaped() {
  for text in [
    "line1\\\n- item",
    "line1\\\n# heading",
    "line1\\\n> quote",
    "line1\\\n1. one",
    "line1\\\n+ plus",
  ] {
    assert_stable("paragraph", text);
  }
}

#[test]
fn rarer_leaders_are_escaped_too() {
  for text in ["1) x", "-\tx", "#\tx", ">x", "#", "+", "2."] {
    assert_stable("paragraph", text);
  }
}

#[test]
fn quote_text_keeps_its_leader_escaped() {
  for text in ["- a", "# a", "1. a"] {
    assert_stable("quote", text);
  }
}
