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
