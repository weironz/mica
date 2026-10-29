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
