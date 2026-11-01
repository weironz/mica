//! Security behavior of the moved HTML/export pipeline, exercised through the
//! public API (the original `#[cfg(test)]` blocks tested private helpers; this
//! keeps the same behaviors covered without widening the engine's API).

use mica_md::compat::{export_html, export_markdown, import_markdown};

fn html(md: &str) -> String {
  export_html(&import_markdown(md, "root")).expect("export_html")
}

#[test]
fn script_urls_are_dropped_from_links_and_images() {
  for md in [
    "[x](javascript:alert(1))",
    "[x](JaVaScRiPt:alert(1))",
    "[x](vbscript:msgbox)",
    "[x](data:text/html,<script>alert(1)</script>)",
    "![x](javascript:alert(1))",
  ] {
    let out = html(md).to_ascii_lowercase();
    assert!(
      !out.contains("javascript:") && !out.contains("vbscript:") && !out.contains("data:text"),
      "{md} -> {out}"
    );
  }
  assert!(html("[x](https://e.com/a:b)").contains("href=\"https://e.com/a:b\""));
  assert!(html("[x](a/b:c)").contains("href=\"a/b:c\""));
  assert!(html("![x](data:image/png;base64,AAAA)").contains("src=\"data:image/png;base64,AAAA\""));
}

#[test]
fn a_bang_comment_close_does_not_hide_markup_from_the_stripper() {
  let out = html("<!-- --!><img src=x onerror=alert(2)> -->\n");
  assert!(!out.contains("onerror"), "{out}");
  let out = html("<!--><img src=x onerror=alert(3)>\n");
  assert!(!out.contains("onerror"), "{out}");
}

#[test]
fn tagfilter_keeps_non_ascii_text_intact() {
  let out = html("<details>\n<summary>点击展开 😀</summary>\n<script>x</script>é</details>");
  assert!(out.contains("点击展开 😀"), "{out}");
  assert!(out.contains("&lt;script"), "{out}");
  assert!(out.contains("é"), "{out}");
}

#[test]
fn event_handlers_are_stripped_from_raw_html_blocks() {
  let out = html("<div>\n<img src=x onerror=\"steal(document.cookie)\">\n</div>\n");
  assert!(!out.contains("onerror"), "{out}");
  assert!(!out.contains("steal("), "{out}");
  assert!(out.contains("<img"), "{out}");
}

#[test]
fn meta_refresh_is_dropped_from_the_exported_pipeline() {
  let out =
    html("<div>\n<meta http-equiv=\"refresh\" content=\"0;url=https://evil.example\">\n</div>\n");
  assert!(!out.to_ascii_lowercase().contains("http-equiv"), "{out}");
  assert!(!out.contains("evil.example"), "{out}");
}

#[test]
fn cycles_and_deep_chains_do_not_overflow() {
  let doc: mica_md::DocumentSnapshotPayload = serde_json::from_value(serde_json::json!({
    "schema_version": 1,
    "root_block_id": "root",
    "blocks": [
      {"id": "root", "type": "page", "text": "", "data": {}, "children": ["a"]},
      {"id": "a", "type": "quote", "text": "loop", "data": {}, "children": ["a", "root"]}
    ]
  }))
  .unwrap();
  assert!(export_html(&doc).unwrap().contains("loop"));
  assert!(export_markdown(&doc).unwrap().contains("loop"));
}

// ── Strict profile: no markup from the document reaches the output ────────

fn strict(md: &str) -> String {
  use mica_md::{HtmlOptions, HtmlProfile, ParseMode, parse_body, render_html_body};
  render_html_body(
    &parse_body(md, ParseMode::Document),
    &HtmlOptions {
      profile: HtmlProfile::Strict,
      ..Default::default()
    },
  )
}

#[test]
fn strict_shows_a_script_block_as_source() {
  let md = "<script>alert(1)</script>\n";
  assert!(
    html(md).contains("<script>"),
    "export keeps type-1 verbatim"
  );
  let out = strict(md);
  assert!(!out.contains("<script"), "{out}");
  assert!(
    out.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
    "{out}"
  );
  assert!(out.starts_with("<pre><code>"), "{out}");
}

#[test]
fn strict_escapes_inline_html_including_inside_table_cells() {
  let out = strict("a <b onclick=x>bold</b> c\n\n| h |\n| - |\n| <i>cell</i> |\n");
  assert!(!out.contains("<b") && !out.contains("<i>"), "{out}");
  assert!(out.contains("&lt;b onclick=x&gt;"), "{out}");
  assert!(out.contains("&lt;i&gt;cell&lt;/i&gt;"), "{out}");
}

#[test]
fn strict_matches_export_when_the_document_has_no_html() {
  let md = "# T\n\n**b** [l](https://x.y) `c`\n\n- a\n- b\n\n> q\n";
  let export = html(md);
  assert_eq!(strict(md), export);
  // And it does not leak into a later Export render (the flag is scoped).
  assert!(html("<b>x</b>\n").contains("<b>"));
}

#[test]
fn strict_is_on_the_wire() {
  let blocks = mica_md::wire::parse_body_json("<b>x</b>\n", false).unwrap();
  let out = mica_md::wire::render_html_body_json(&blocks, "{}", "strict").unwrap();
  assert!(!out.contains("<b>") && out.contains("&lt;b&gt;"), "{out}");
}
