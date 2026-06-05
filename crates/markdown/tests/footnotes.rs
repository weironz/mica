//! GFM footnotes, both directions. A `[^label]` inline reference strips to its
//! bare label under a `footnote` mark; a `[^label]: …` line at the document
//! start is a `footnote_def` block. Import→export must be identity for every
//! shape — numeric and non-numeric labels, multiple references to one
//! definition, and an undefined reference degrading to literal text. HTML
//! export uses the GFM shape: superscript reference links plus a trailing
//! <section class="footnotes"> with backlinks.

use mica_markdown::{export_html, export_markdown, import_markdown};

/// import→export, asserting the Markdown comes back unchanged (trailing
/// whitespace aside).
fn round_trip(md: &str) {
  let snap = import_markdown(md, "root");
  let back = export_markdown(&snap).unwrap();
  assert_eq!(back.trim_end(), md.trim_end(), "round-trip drift");
}

#[test]
fn numeric_reference_and_definition_round_trip() {
  round_trip("Here is a note.[^1]\n\n[^1]: The definition.");
}

#[test]
fn non_numeric_label_round_trips() {
  round_trip("See the spec.[^gfm-spec]\n\n[^gfm-spec]: General Format for Markdown.");
}

#[test]
fn multiple_references_to_one_definition_round_trip() {
  round_trip("First[^a] and again[^a].\n\n[^a]: shared note.");
}

#[test]
fn undefined_reference_degrades_to_literal() {
  // No definition exists, but the reference still round-trips verbatim — the
  // bare label is the block text and `[^x]` is restored from the mark.
  round_trip("A dangling reference[^missing] stays put.");
}

#[test]
fn reference_strips_to_bare_label_with_a_footnote_mark() {
  let snap = import_markdown("Text[^1] more.", "root");
  let para = snap
    .blocks
    .iter()
    .find(|b| b.kind == "paragraph" && !b.text.is_empty())
    .unwrap();
  // The bracket+caret syntax is gone; only the label survives in the text.
  assert_eq!(para.text, "Text1 more.");
  let marks = para.data["marks"].as_array().unwrap();
  let fnote = marks.iter().find(|m| m["type"] == "footnote").unwrap();
  assert_eq!(fnote["href"], "1");
}

#[test]
fn definition_carries_label_and_inline_content() {
  let snap = import_markdown("[^n]: see **here**.", "root");
  let def = snap.blocks.iter().find(|b| b.kind == "footnote_def").unwrap();
  assert_eq!(def.data["label"], "n");
  // The bold marker is parsed to an inline mark, not left literal.
  assert_eq!(def.text, "see here.");
  let marks = def.data["marks"].as_array().unwrap();
  assert!(marks.iter().any(|m| m["type"] == "bold"));
}

#[test]
fn html_export_uses_gfm_reference_and_section() {
  let snap = import_markdown("Note.[^1]\n\n[^1]: The body.", "root");
  let html = export_html(&snap).unwrap();
  // Reference: superscript backlink-anchored to the definition.
  assert!(
    html.contains("<sup id=\"fnref-1\"><a href=\"#fn-1\">1</a></sup>"),
    "got:\n{html}"
  );
  // Definition: trailing section with the body and a backref to the reference.
  assert!(html.contains("<section class=\"footnotes\">"), "got:\n{html}");
  assert!(html.contains("<li id=\"fn-1\">"), "got:\n{html}");
  assert!(html.contains("href=\"#fnref-1\""), "got:\n{html}");
  assert!(html.contains("The body."), "got:\n{html}");
}

#[test]
fn multiline_definition_joins_indented_continuation() {
  // A continuation line indented 4 columns is part of the same definition.
  let md = "[^long]: first line\n    second line";
  let snap = import_markdown(md, "root");
  let def = snap.blocks.iter().find(|b| b.kind == "footnote_def").unwrap();
  assert_eq!(def.text, "first line\nsecond line");
  let back = export_markdown(&snap).unwrap();
  assert_eq!(back.trim_end(), md);
}
