//! Regression & contract tests for inline highlight (`==text==` / `<mark>`).

use mica_md::{
  HtmlNode, HtmlOptions, HtmlProfile, compat, html_to_markdown, parse_inline, render_html,
  schema::marks,
};
use serde_json::json;

#[test]
fn parses_and_round_trips_inline_highlight() {
  let md = "这是 ==核心重点== 和 ==key point==。";
  let doc = compat::import_markdown(md, "root");
  assert_eq!(doc.blocks.len(), 2); // root + 1 paragraph
  assert_eq!(doc.blocks[1].text, "这是 核心重点 和 key point。");

  let parsed = parse_inline("这是 ==核心重点== 和 ==key point==。");
  assert_eq!(parsed.marks.len(), 2);
  assert!(parsed.marks.iter().all(|m| m.kind == marks::HIGHLIGHT));

  assert_eq!(compat::export_markdown(&doc).unwrap(), md);
  assert_eq!(
    compat::export_html(&doc).unwrap(),
    "<p>这是 <mark>核心重点</mark> 和 <mark>key point</mark>。</p>"
  );
  let clipboard_opts = HtmlOptions {
    profile: HtmlProfile::Clipboard,
    ..Default::default()
  };
  assert_eq!(
    render_html(&doc, &clipboard_opts).unwrap(),
    "<p>这是 <mark>核心重点</mark> 和 <mark>key point</mark>。</p>"
  );
  assert!(mica_markdown::static_page_css(760).contains("mark{"));
}

#[test]
fn parses_cjk_without_spaces_and_nested_marks() {
  let md = "中文==重点内容==后面，以及 **==粗体高亮==** 与 ==[链接](https://example.com)==。\n";
  let doc = compat::import_markdown(md, "root");
  assert_eq!(doc.blocks.len(), 2);
  assert_eq!(
    doc.blocks[1].text,
    "中文重点内容后面，以及 粗体高亮 与 链接。"
  );

  let html = compat::export_html(&doc).unwrap();
  assert!(html.contains("<mark>重点内容</mark>"), "html={html}");
  assert!(
    html.contains("<strong><mark>粗体高亮</mark></strong>"),
    "html={html}"
  );
  assert!(
    html.contains("<mark><a href=\"https://example.com\">链接</a></mark>"),
    "html={html}"
  );

  let exported = compat::export_markdown(&doc).unwrap();
  let reimported = compat::import_markdown(&exported, "root");
  assert_eq!(reimported.blocks[1].text, doc.blocks[1].text);
  assert_eq!(compat::export_html(&reimported).unwrap(), html);
}

#[test]
fn does_not_misparse_prose_comparisons_triple_equals_or_code_spans() {
  let cases = [
    "if a == b and c == d then ok",
    "strict === comparison === here",
    "single =not highlight= here",
    "spaces == not highlight == here",
    "code `==not highlight==` stays code",
    "escaped \\==literal== stays plain",
  ];
  for input in cases {
    let parsed = parse_inline(input);
    assert!(
      parsed.marks.iter().all(|m| m.kind != marks::HIGHLIGHT),
      "unexpected highlight in `{input}`: {:?}",
      parsed.marks
    );
  }
}

#[test]
fn literal_double_equals_in_plain_text_round_trips_without_becoming_highlight() {
  let md = "escaped \\==literal== and a == b and x===y\n";
  let doc = compat::import_markdown(md, "root");
  assert_eq!(doc.blocks.len(), 2);
  assert_eq!(
    doc.blocks[1].text,
    "escaped ==literal== and a == b and x===y"
  );

  let exported = compat::export_markdown(&doc).unwrap();
  let reimported = compat::import_markdown(&exported, "root");
  assert_eq!(reimported.blocks[1].text, doc.blocks[1].text);
  assert!(
    !compat::export_html(&reimported).unwrap().contains("<mark>"),
    "literal == turned into <mark>: exported={exported}"
  );
}

#[test]
fn pastes_html_mark_tag_into_highlight_markdown() {
  let body = HtmlNode::from_json(&json!({
    "e": "body",
    "a": {},
    "c": [
      {
        "e": "p",
        "a": {},
        "c": [
          {"t": "Check "},
          {"e": "mark", "a": {}, "c": [{"t": "highlighted"}]},
          {"t": " and "},
          {"e": "strong", "a": {}, "c": [
            {"e": "mark", "a": {}, "c": [{"t": "both"}]}
          ]},
          {"t": "."}
        ]
      }
    ]
  }));
  let md = html_to_markdown(&body);
  assert_eq!(md.trim(), "Check ==highlighted== and **==both==**.");
}
