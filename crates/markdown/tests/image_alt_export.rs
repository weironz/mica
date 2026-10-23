//! Image alt marks stay inside the image; equal-range marks must not become outer links.
//! Mirrored by clients/mica_flutter/test/image_alt_export_test.dart.
use mica_markdown::{export_markdown, import_markdown};

#[test]
fn inline_image_alt_marks_and_real_outer_links_survive() {
  for (src, expected) in [
    // Issue #58's original short examples, with explicit image syntax.
    (r"pre ![\[x\](u)](v) post", r"pre ![\[x\](u)](v) post"),
    ("pre ![[a](u)](v) post", "pre ![[a](u)](v) post"),
    ("pre ![a [b] c](/img) post", "pre ![a \\[b\\] c](/img) post"),
    ("pre ![[x](/alt)](/img) post", "pre ![[x](/alt)](/img) post"),
    (
      "pre ![a [b](/alt) c](/img) post",
      "pre ![a [b](/alt) c](/img) post",
    ),
    (r"pre ![\[x\](u)](/img) post", r"pre ![\[x\](u)](/img) post"),
    (
      r"pre ![left \[ and right \]](/img) post",
      r"pre ![left \[ and right \]](/img) post",
    ),
    (r"pre ![\\\[x\]](/img) post", r"pre ![\\\[x\]](/img) post"),
    (
      "pre ![[x][alt]][image] post\n\n[alt]: /alt\n[image]: /img",
      "pre ![[x](/alt)](/img) post",
    ),
    (
      "pre ![**bold** *italic* `code`](/img) post",
      "pre ![**bold** *italic* `code`](/img) post",
    ),
    ("[![x](/img)](/outer)", "[![x](/img)](/outer)"),
    (
      "pre [![x](/img)](/outer) post",
      "pre [![x](/img)](/outer) post",
    ),
    (
      "pre ![[x](/alt)](/img \"image title\") post",
      "pre ![[x](/alt)](/img \"image title\") post",
    ),
    (
      "pre [![😀](/img \"image title\")](/outer \"link title\") post",
      "pre [![😀](/img \"image title\")](/outer \"link title\") post",
    ),
    (
      "😀 pre ![[😺](/alt)](/img) post",
      "😀 pre ![[😺](/alt)](/img) post",
    ),
  ] {
    let parsed = import_markdown(src, "root");
    let html = mica_markdown::export_html_document(&parsed, "test", 800).unwrap();
    let image_url = if src.contains("](v)") { "v" } else { "/img" };
    assert!(
      html.contains(&format!("<img src=\"{image_url}\"")),
      "{html}"
    );
    assert!(!html.contains("href=\"/alt\""), "{html}");
    assert!(!html.contains("href=\"u\""), "{html}");
    if src.contains("/outer") {
      assert!(html.contains("<a href=\"/outer\""), "{html}");
    }
    let md = export_markdown(&parsed).unwrap();
    assert_eq!(md.trim(), expected, "source: {src}");
    let again = import_markdown(&md, "root");
    assert_eq!(export_markdown(&again).unwrap(), md, "unstable: {src}");
    let first = parsed
      .blocks
      .iter()
      .find(|b| b.kind == "paragraph" && b.id != parsed.root_block_id)
      .unwrap();
    let second = again
      .blocks
      .iter()
      .find(|b| b.kind == "paragraph" && b.id != again.root_block_id)
      .unwrap();
    assert_eq!(second.text, first.text, "alt changed: {src}");
    assert_eq!(
      second.data["marks"], first.data["marks"],
      "marks changed: {src}"
    );
  }
}

#[test]
fn standalone_image_alt_links_flatten_without_becoming_outer_links() {
  for src in [
    "![[x](/alt)](/img)",
    "![[x][alt]][image]\n\n[alt]: /alt\n[image]: /img",
  ] {
    let parsed = import_markdown(src, "root");
    let image = parsed.blocks.iter().find(|b| b.id != "root").unwrap();
    assert_eq!(image.kind, "image", "{src}");
    assert_eq!(image.text, "x", "{src}");
    assert_eq!(image.data["url"], "/img", "{src}");
    assert_eq!(export_markdown(&parsed).unwrap().trim(), "![x](/img)");
  }
}

#[test]
fn image_blocks_escape_literal_alt_and_preserve_title() {
  for alt in [
    "a [b] c",
    "[x](/alt)",
    "😀 [😺] *literal*",
    r"\[x]",
    "[",
    "]",
    "",
  ] {
    let mut parsed = import_markdown("![placeholder](/img \"image title\")", "root");
    let image = parsed
      .blocks
      .iter_mut()
      .find(|b| b.kind == "image")
      .unwrap();
    image.text = alt.to_string();
    image.data["file_id"] = serde_json::json!("file");
    let assets = std::collections::BTreeMap::from([("file".into(), "assets/image.png".into())]);
    for (md, expected_url) in [
      (export_markdown(&parsed).unwrap(), "/img"),
      (
        mica_markdown::export_markdown_with_assets(&parsed, &assets).unwrap(),
        "assets/image.png",
      ),
    ] {
      let again = import_markdown(&md, "root");
      let image = again.blocks.iter().find(|b| b.kind == "image").unwrap();
      assert_eq!(image.text, alt, "{md}");
      assert_eq!(image.data["url"], expected_url);
      assert_eq!(image.data["title"], "image title");
    }
  }
}
