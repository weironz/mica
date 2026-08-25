//! Table cells carry Markdown inline source (code spans, emphasis, links).
//! Importing a table, then re-exporting it (Markdown and HTML), must preserve
//! those marks — and a missing/`null` stored cell must export as empty, never
//! as the literal word "null".

use mica_markdown::{export_html, export_markdown, import_markdown, payload_from_value};
use serde_json::json;

#[test]
fn inline_code_in_a_cell_round_trips() {
  let md = "| A | B |\n| --- | --- |\n| `code` | **bold** |";
  let snap = import_markdown(md, "root");

  // Markdown round-trip keeps the code span and the emphasis verbatim.
  let back = export_markdown(&snap).unwrap();
  assert!(back.contains("| `code` | **bold** |"), "got:\n{back}");

  // HTML render turns the inline source into real <code>/<strong>.
  let html = export_html(&snap).unwrap();
  assert!(html.contains("<td><code>code</code></td>"), "got:\n{html}");
  assert!(html.contains("<td><strong>bold</strong></td>"), "got:\n{html}");
}

#[test]
fn a_null_cell_exports_empty_not_the_literal_null() {
  // A stored grid with a `null` cell (a missing value) must not leak the word
  // "null" into the exported Markdown or HTML.
  let payload = payload_from_value(json!({
    "schema_version": 1,
    "root_block_id": "root",
    "blocks": [
      { "id": "root", "type": "page", "children": ["t"] },
      {
        "id": "t",
        "type": "table",
        "data": {
          "rows": [["A", "B"], ["`code`", null]],
          "header": true,
          "widths": [1.0, 1.0]
        },
        "children": []
      }
    ]
  }))
  .unwrap();

  let md = export_markdown(&payload).unwrap();
  assert!(!md.contains("null"), "markdown leaked null:\n{md}");
  assert!(md.contains("| `code` |  |"), "got:\n{md}");

  let html = export_html(&payload).unwrap();
  assert!(!html.contains("null"), "html leaked null:\n{html}");
  assert!(html.contains("<td><code>code</code></td>"), "got:\n{html}");
}

#[test]
fn rich_cells_import_as_the_unified_marks_form() {
  // A marked cell upgrades from raw source to the {text, marks} object that
  // paragraphs use; a mark-free cell stays a plain string (no bloat).
  let md = "| plain | **bold** |\n| --- | --- |\n| a | `x` |";
  let snap = import_markdown(md, "root");
  let table = snap.blocks.iter().find(|b| b.kind == "table").unwrap();
  let rows = table.data.get("rows").and_then(|v| v.as_array()).unwrap();

  // Mark-free header cell "plain" stays a string.
  assert!(rows[0][0].is_string(), "mark-free cell must stay a string: {:?}", rows[0][0]);
  // "**bold**" becomes {text:"bold", marks:[bold 0..4]}.
  let bold = &rows[0][1];
  assert_eq!(bold.get("text").and_then(|v| v.as_str()), Some("bold"));
  let marks = bold.get("marks").and_then(|v| v.as_array()).unwrap();
  assert_eq!(marks.len(), 1);
  assert_eq!(marks[0].get("type").and_then(|v| v.as_str()), Some("bold"));
  assert_eq!(marks[0].get("start").and_then(|v| v.as_u64()), Some(0));
  assert_eq!(marks[0].get("end").and_then(|v| v.as_u64()), Some(4));
}

#[test]
fn rich_cell_round_trips_read_write_read() {
  // Nested emphasis, a link, an escaped pipe inside a code span, and an empty
  // cell — the read → write → read invariant must be stable and mark-preserving.
  let md = "| A | B |\n| --- | --- |\n\
            | **bold *it*** | [t](/u) |\n\
            | b `\\|` az |  |";
  let snap = import_markdown(md, "root");
  let back = export_markdown(&snap).unwrap();

  // Every construct survives the serializer verbatim (GFM-canonical).
  assert!(back.contains("| **bold *it*** | [t](/u) |"), "got:\n{back}");
  assert!(back.contains("| b `\\|` az |  |"), "escaped pipe in code lost:\n{back}");

  // Second cycle is a fixed point (the round-trip invariant).
  let snap2 = import_markdown(&back, "root");
  let back2 = export_markdown(&snap2).unwrap();
  assert_eq!(back, back2, "round-trip is not idempotent");

  // The stored form is unified marks, and HTML renders the marks.
  let html = export_html(&snap).unwrap();
  assert!(html.contains("<strong>bold <em>it</em></strong>"), "nested marks:\n{html}");
  assert!(html.contains("<a href=\"/u\">t</a>"), "link:\n{html}");
  assert!(html.contains("<code>|</code>"), "escaped pipe code span:\n{html}");
}
