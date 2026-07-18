//! `export_html_document` wraps the HTML fragment in a standalone, self-
//! contained page; `set_image_srcs` swaps content-addressed `file_id`s for the
//! `data:` URIs an export embeds. Both feed the download endpoint and the FFI
//! local export, so they must stay pure and platform-agnostic.

use std::collections::BTreeMap;

use mica_markdown::{export_html_document, import_markdown, set_image_srcs};

#[test]
fn wraps_fragment_in_standalone_document() {
  let snapshot = import_markdown("# Hello\n\nA paragraph with **bold**.", "root");
  let html = export_html_document(&snapshot, "My Page").expect("export");

  // A real, self-contained HTML5 file: doctype, charset, embedded style, no
  // external stylesheet/script requests.
  assert!(html.starts_with("<!doctype html>"), "has doctype: {html}");
  assert!(html.contains("<meta charset=\"utf-8\">"));
  assert!(html.contains("<style>"), "carries its own CSS");
  assert!(!html.contains("<link"), "no external stylesheet");
  assert!(!html.contains("<script"), "no scripts");
  // Title reaches both <title> and the <h1>.
  assert!(html.contains("<title>My Page</title>"));
  assert!(html.contains("<h1>My Page</h1>"));
  // The body fragment is present.
  assert!(html.contains("<strong>bold</strong>"));
}

#[test]
fn escapes_title() {
  let snapshot = import_markdown("body", "root");
  let html = export_html_document(&snapshot, "a <b> & \"c\"").expect("export");
  assert!(html.contains("<title>a &lt;b&gt; &amp; "), "title escaped: {html}");
  assert!(!html.contains("<title>a <b>"), "no raw angle brackets in title");
}

#[test]
fn set_image_srcs_rewrites_matching_file_id() {
  // An image block carrying a content-addressed file_id (as cloud/local pages
  // store it) — export must be able to point it at an embedded data: URI.
  let mut snapshot = import_markdown("![alt](placeholder)", "root");
  // Force a known file_id onto the single image block.
  let img = snapshot
    .blocks
    .iter_mut()
    .find(|b| b.kind == "image")
    .expect("an image block");
  if let serde_json::Value::Object(map) = &mut img.data {
    map.insert("file_id".into(), serde_json::Value::String("abc123".into()));
  }

  let mut srcs = BTreeMap::new();
  srcs.insert("abc123".to_string(), "data:image/png;base64,Zm9v".to_string());
  srcs.insert("unused".to_string(), "data:image/png;base64,YmFy".to_string());
  set_image_srcs(&mut snapshot, &srcs);

  let html = export_html_document(&snapshot, "T").expect("export");
  assert!(
    html.contains("src=\"data:image/png;base64,Zm9v\""),
    "image src rewritten to the embedded data URI: {html}"
  );
}

#[test]
fn set_image_srcs_leaves_unmatched_blocks_untouched() {
  let mut snapshot = import_markdown("![alt](https://example.com/x.png)", "root");
  let before = snapshot.clone();
  // A map that matches no block's file_id.
  let mut srcs = BTreeMap::new();
  srcs.insert("nomatch".to_string(), "data:image/png;base64,Zm9v".to_string());
  set_image_srcs(&mut snapshot, &srcs);
  assert_eq!(snapshot, before, "no file_id match → snapshot unchanged");
}
