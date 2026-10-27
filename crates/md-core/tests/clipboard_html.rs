//! Literal Clipboard dialect outputs from Dart selectionHtml/inlineToHtml.
use mica_md::wire;
use serde_json::{Value, json};

fn html(blocks: Value) -> String {
  wire::render_html_body_json(&blocks.to_string(), "{}", "clipboard").unwrap()
}

#[test]
fn source_math_mermaid_footnotes_and_unicode() {
  assert_eq!(
    html(json!([
      {"type":"paragraph","text":"😀 x note","data":{"marks":[
        {"start":3,"end":4,"type":"math"},
        {"start":5,"end":9,"type":"footnote","href":"a\"b"}
      ]}},
      {"type":"math_block","text":"x<y & z"},
      {"type":"code_block","text":"graph LR; A-->B","data":{"language":"mermaid"}},
      {"type":"footnote_def","text":"body","data":{"label":"a\"b"}}
    ])),
    concat!(
      "<p>😀 <span data-mica-math=\"1\">$x$</span> <sup data-mica-footnote=\"a&quot;b\">note</sup></p>",
      "<p><span data-mica-math=\"1\">$$x&lt;y &amp; z$$</span></p>",
      "<pre><code class=\"language-mermaid\">graph LR; A--&gt;B</code></pre>",
      "<p data-mica-fndef=\"a&quot;b\">body</p>"
    )
  );
}

#[test]
fn nested_lists_quotes_tasks_and_tables() {
  assert_eq!(
    html(json!([
      {"type":"numbered_list","text":"five","data":{"start":5}},
      {"type":"todo","text":"done","data":{"indent":1,"checked":true}},
      {"type":"numbered_list","text":"six"},
      {"type":"quote","text":"q","data":{"quote":2}},
      {"type":"quote","text":"new","data":{"qbreak":true}},
      {"type":"table","data":{"rows":[["**h**"],[{"text":"x","marks":[{"type":"code","start":0,"end":1}]}]]}}
    ])),
    concat!(
      "<ol start=\"5\"><li>five<ul><li><input type=\"checkbox\" checked disabled> done</li></ul></li><li>six</li></ol>",
      "<blockquote><blockquote><p>q</p></blockquote></blockquote><blockquote><p>new</p></blockquote>",
      "<table><tr><th><strong>h</strong></th></tr><tr><td><code>x</code></td></tr></table>"
    )
  );
}

#[test]
fn supplied_image_sources_and_escaped_raw_html() {
  let blocks = json!([
    {"type":"image","text":"\"alt&","data":{"file_id":"a","url":"old"}},
    {"type":"paragraph","text":"<b>x</b>","data":{"marks":[{"start":0,"end":8,"type":"html"}]}},
    {"type":"code_block","text":"<x>&","data":{"language":"html","raw":true}},
    {"type":"divider"}
  ]);
  assert_eq!(
    wire::render_html_body_json(
      &blocks.to_string(),
      r#"{"a":"https://x/y?a=1&b=2"}"#,
      "clipboard"
    )
    .unwrap(),
    concat!(
      "<p><img src=\"https://x/y?a=1&amp;b=2\" alt=\"&quot;alt&amp;\"></p>",
      "<p>&lt;b&gt;x&lt;/b&gt;</p><pre><code class=\"language-html\">&lt;x&gt;&amp;</code></pre><hr>"
    )
  );
}

#[test]
fn dangerous_urls_cannot_survive_clipboard_attributes() {
  for url in [
    "javascript:alert(1)",
    "java\tscript:x",
    "vbscript:x",
    "data:text/html,x",
  ] {
    let out = html(json!([
      {"type":"image","text":"a","data":{"url":url}},
      {"type":"paragraph","text":"x","data":{"marks":[{"type":"link","start":0,"end":1,"href":url}]}}
    ]));
    assert_eq!(
      out, "<p><img src=\"\" alt=\"a\"></p><p><a href=\"\">x</a></p>",
      "{url}"
    );
  }
}

#[test]
fn dart_thirty_fixture_port() {
  let fixture: Value =
    serde_json::from_str(include_str!("fixtures/clipboard_html_port.json")).unwrap();
  let cases = fixture["cases"].as_array().unwrap();
  assert_eq!(cases.len(), 30, "must replay all Dart full-selection cases");
  for c in cases {
    assert_eq!(
      html(c["blocks"].clone()),
      c["html"].as_str().unwrap(),
      "{}",
      c["id"]
    );
  }
}

#[test]
fn all_nested_mark_kinds_keep_dart_ties_and_titles() {
  assert_eq!(
    html(json!([{"type":"paragraph","text":"abc","data":{"marks":[
      {"start":0,"end":3,"type":"bold"},
      {"start":0,"end":3,"type":"italic"},
      {"start":1,"end":2,"type":"strike"}
    ]}}])),
    "<p><em><strong>a<s>b</s>c</strong></em></p>"
  );
  // HTML clipboard historically omits link titles; Export still keeps them.
  assert_eq!(
    html(json!([{"type":"paragraph","text":"abc","data":{"marks":[
      {"start":0,"end":3,"type":"code"},
      {"start":0,"end":3,"type":"link","href":"/u?a=1&b=2","title":"t"}
    ]}}])),
    "<p><a href=\"/u?a=1&amp;b=2\"><code>abc</code></a></p>"
  );
  assert_eq!(
    html(json!([{"type":"paragraph","text":"icon","data":{"marks":[
      {"start":0,"end":4,"type":"image","href":"/i.png"}
    ]}}])),
    "<p><img src=\"/i.png\" alt=\"icon\"></p>"
  );
}

#[test]
fn single_empty_text_selection_matches_the_old_collapsed_caret() {
  // Old selectionHtml has no slice for a collapsed caret on a textual block;
  // full selection of the sole empty code node is that same caret. An empty
  // code node within a non-collapsed multi-block selection still emits <pre>.
  assert_eq!(
    html(json!([{"type":"code_block","text":"","data":{"language":"a~~"}}])),
    ""
  );
  assert_eq!(
    html(json!([
      {"type":"code_block","text":"","data":{"language":"a~~"}},
      {"type":"paragraph","text":"tail"}
    ])),
    "<pre><code class=\"language-a~~\"></code></pre><p>tail</p>"
  );
}

#[test]
fn export_rich_feature_exception_is_explicit_and_clipboard_stays_source() {
  let blocks = json!([{"type":"paragraph","text":"x","data":{"marks":[
    {"start":0,"end":1,"type":"math"}
  ]}}]);
  let export = wire::render_html_body_json(&blocks.to_string(), "{}", "export").unwrap();
  let clipboard = html(blocks);
  assert_eq!(clipboard, "<p><span data-mica-math=\"1\">$x$</span></p>");
  if cfg!(feature = "render") {
    assert!(
      export.contains("<svg"),
      "Export must keep its rich renderer: {export}"
    );
    assert!(!clipboard.contains("<svg"));
  } else {
    assert_eq!(export, "<p><span class=\"math\">x</span></p>");
  }
}

#[test]
fn invalid_schema_and_missing_root_are_reported_by_typed_api() {
  let mut doc = mica_md::DocumentSnapshotPayload {
    schema_version: 2,
    root_block_id: "r".into(),
    blocks: vec![],
  };
  let opts = mica_md::HtmlOptions {
    profile: mica_md::HtmlProfile::Clipboard,
    ..Default::default()
  };
  assert!(matches!(
    mica_md::render_html(&doc, &opts),
    Err(mica_md::MdError::UnsupportedSchemaVersion(2))
  ));
  doc.schema_version = 1;
  assert!(matches!(
    mica_md::render_html(&doc, &opts),
    Err(mica_md::MdError::BlockNotFound(_))
  ));
}
