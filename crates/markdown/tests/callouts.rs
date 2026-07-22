//! GFM alert (callout) behavior: the 5 standard types round-trip through a
//! `> [!TYPE]` marker reusing the flat quote model (`data.alert` on the group
//! head), render styled HTML, and degrade cleanly for anything non-standard.
//! The shared-fixture `24-callouts` pins the block shape; this file pins the
//! edges the fixture corpus can't express (HTML classes, degradation).

use mica_markdown::{export_html, export_markdown, import_markdown};

fn head_alert(md: &str) -> Option<String> {
  let payload = import_markdown(md, "root");
  let root = payload.blocks.iter().find(|b| b.id == payload.root_block_id).unwrap();
  let first = root.children.first()?;
  let block = payload.blocks.iter().find(|b| &b.id == first)?;
  block.data.get("alert").and_then(|v| v.as_str()).map(str::to_string)
}

#[test]
fn all_five_types_set_alert() {
  for (marker, want) in [
    ("NOTE", "note"),
    ("TIP", "tip"),
    ("IMPORTANT", "important"),
    ("WARNING", "warning"),
    ("CAUTION", "caution"),
  ] {
    let md = format!("> [!{marker}]\n> body\n");
    assert_eq!(head_alert(&md).as_deref(), Some(want), "marker {marker}");
  }
}

#[test]
fn type_is_case_insensitive() {
  assert_eq!(head_alert("> [!note]\n> x\n").as_deref(), Some("note"));
  assert_eq!(head_alert("> [!Warning]\n> x\n").as_deref(), Some("warning"));
}

#[test]
fn unknown_type_degrades_to_plain_quote() {
  // Not one of the 5 — stays literal blockquote text, no `data.alert`.
  assert_eq!(head_alert("> [!UNKNOWN]\n> x\n"), None);
  assert_eq!(head_alert("> [!NOTES]\n> x\n"), None);
  // Trailing content on the marker line disqualifies it (GitHub rule).
  assert_eq!(head_alert("> [!NOTE] and more\n> x\n"), None);
}

#[test]
fn marker_must_be_first_line() {
  // A `[!NOTE]` that is not the blockquote's opening line is just text.
  assert_eq!(head_alert("> lead in\n> [!NOTE]\n"), None);
}

#[test]
fn nested_blockquote_marker_stays_literal() {
  // Alerts are top-level (depth 1) only; `> > [!NOTE]` is a nested quote.
  assert_eq!(head_alert("> > [!NOTE]\n> > x\n"), None);
}

#[test]
fn body_less_alert_survives_round_trip() {
  // `> [!NOTE]` alone keeps its type via an empty quote head.
  assert_eq!(head_alert("> [!NOTE]\n").as_deref(), Some("note"));
  let exported = export_markdown(&import_markdown("> [!NOTE]\n", "root")).unwrap();
  assert!(exported.contains("[!NOTE]"), "marker lost on export: {exported:?}");
  assert_eq!(head_alert(&exported).as_deref(), Some("note"));
}

#[test]
fn export_normalizes_marker_to_uppercase() {
  let exported = export_markdown(&import_markdown("> [!tip]\n> body\n", "root")).unwrap();
  assert!(exported.contains("> [!TIP]"), "got: {exported:?}");
}

#[test]
fn html_carries_alert_classes_and_title() {
  let html = export_html(&import_markdown("> [!WARNING]\n> careful\n", "root")).unwrap();
  assert!(html.contains("markdown-alert markdown-alert-warning"), "classes: {html}");
  assert!(html.contains("markdown-alert-title"), "title row: {html}");
  assert!(html.contains(">Warning<"), "title text: {html}");
  assert!(html.contains("careful"), "body: {html}");
}

#[test]
fn plain_quote_html_unchanged() {
  let html = export_html(&import_markdown("> just a quote\n", "root")).unwrap();
  assert!(html.contains("<blockquote>"), "plain quote: {html}");
  assert!(!html.contains("markdown-alert"), "no alert chrome: {html}");
}

#[test]
fn alert_then_plain_quote_boundary() {
  let md = "> [!NOTE]\n> in the alert\n\n> not an alert\n";
  let payload = import_markdown(md, "root");
  let root = payload.blocks.iter().find(|b| b.id == payload.root_block_id).unwrap();
  let kinds: Vec<Option<&str>> = root
    .children
    .iter()
    .map(|id| {
      let b = payload.blocks.iter().find(|x| &x.id == id).unwrap();
      b.data.get("alert").and_then(|v| v.as_str())
    })
    .collect();
  assert_eq!(kinds, vec![Some("note"), None], "alert head then plain quote");
}
