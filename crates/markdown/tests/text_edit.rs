use mica_markdown::{
  Block, block_text_matches, export_markdown, import_markdown, markdown_find_text,
  replace_block_text,
};
use serde_json::json;

fn body(markdown: &str) -> Block {
  let doc = import_markdown(markdown, "root");
  doc.blocks.into_iter().find(|b| b.id != "root").unwrap()
}

#[test]
fn markdown_find_decodes_inline_and_list_syntax_without_joining_blocks() {
  assert_eq!(
    markdown_find_text("run `omp --version` now").as_deref(),
    Some("run omp --version now")
  );
  assert_eq!(
    markdown_find_text("- **pending**: login").as_deref(),
    Some("pending: login")
  );
  assert_eq!(markdown_find_text("2. second").as_deref(), Some("second"));
  assert_eq!(markdown_find_text("one\n\ntwo"), None);
  assert_eq!(markdown_find_text("unchanged"), None);
}

#[test]
fn replacement_splices_utf16_marks_without_touching_other_data() {
  let block = body("😀 **before** and `tool` [after](https://example.test)");
  let data = json!({
    "marks": block.data["marks"], "indent": 2, "custom": {"keep": true}
  });
  let mut block = block.with_data(data);
  block.children = vec!["child".into()];
  let edited = replace_block_text(&block, "before", "长😀text", true);
  assert_eq!(edited.text, "😀 长😀text and tool after");
  assert_eq!(edited.id, block.id);
  assert_eq!(edited.children, block.children);
  assert_eq!(edited.data["custom"], block.data["custom"]);
  assert_eq!(edited.data["indent"], 2);
  assert_eq!(
    edited.data["marks"][0],
    json!({"start": 3, "end": 10, "type": "bold"})
  );
  let code = edited.data["marks"]
    .as_array()
    .unwrap()
    .iter()
    .find(|m| m["type"] == "code")
    .unwrap();
  assert_eq!(
    (code["start"].as_u64(), code["end"].as_u64()),
    (Some(15), Some(19))
  );
}

#[test]
fn replacement_markdown_and_literal_formats_have_distinct_storage() {
  let block = body("set old now");
  let rich = replace_block_text(&block, "old", "`DEEPSEEK_API_KEY`", true);
  assert_eq!(rich.text, "set DEEPSEEK_API_KEY now");
  assert_eq!(
    rich.data["marks"][0],
    json!({"type": "code", "start": 4, "end": 20})
  );
  let plain = replace_block_text(&block, "old", "`DEEPSEEK_API_KEY`", false);
  assert_eq!(plain.text, "set `DEEPSEEK_API_KEY` now");
  assert!(plain.data.get("marks").is_none());
}

#[test]
fn matches_cross_inline_boundaries_and_multiple_edits_keep_offsets() {
  let block = body("😀 a `long` z and a `long` z **tail**");
  assert_eq!(block_text_matches(&block, "a long z"), 2);
  let edited = replace_block_text(&block, "a long z", "*短*", true);
  assert_eq!(edited.text, "😀 短 and 短 tail");
  assert_eq!(
    edited.data["marks"],
    json!([
      {"start":3,"end":4,"type":"italic"},
      {"start":9,"end":10,"type":"italic"},
      {"start":11,"end":15,"type":"bold"}
    ])
  );
}

#[test]
fn partial_and_enclosing_marks_keep_only_the_intended_ranges() {
  let block = body("**abCD**ef [whole sentence](https://example.test)");
  let edited = replace_block_text(&block, "CDef", "X", true);
  assert_eq!(edited.text, "abX whole sentence");
  assert_eq!(
    edited.data["marks"][0],
    json!({"start":0,"end":2,"type":"bold"})
  );
  let edited = replace_block_text(&block, "whole", "`new`", true);
  let marks = edited.data["marks"].as_array().unwrap();
  assert!(
    marks
      .iter()
      .any(|m| m["type"] == "link" && m["href"] == "https://example.test")
  );
  assert!(marks.iter().any(|m| m["type"] == "code"));
  let exact = replace_block_text(&body("`old`"), "old", "**new**", true);
  assert_eq!(
    exact.data["marks"],
    json!([{"type":"bold", "start":0, "end":3}])
  );
}

#[test]
fn table_cells_cover_legacy_and_structured_values_and_keep_alignment() {
  let block = Block::new("table", "table").with_data(json!({
    "align": ["left", "right"],
    "rows": [
      ["**old** value", {"text":"old value","marks":[{"type":"code","start":0,"end":3}]}],
      [null, "other"]
    ]
  }));
  assert_eq!(block_text_matches(&block, "old value"), 2);
  let edited = replace_block_text(&block, "old value", "`key_name`", true);
  assert_eq!(block_text_matches(&edited, "key_name"), 2);
  assert_eq!(edited.data["align"], block.data["align"]);
  assert_eq!(edited.data["rows"][1], block.data["rows"][1]);
  for cell in edited.data["rows"][0].as_array().unwrap() {
    assert_eq!(cell["text"], "key_name");
    assert_eq!(cell["marks"], json!([{"start":0,"end":8,"type":"code"}]));
  }
  let literal = replace_block_text(&edited, "key_name", "`literal`", false);
  assert_eq!(literal.data["rows"][0][0]["text"], "`literal`");
}

#[test]
fn code_blocks_stay_literal_and_no_matches_leave_blocks_unchanged() {
  let block = body("```sh\nold\n```");
  let edited = replace_block_text(&block, "old", "`new_key`", true);
  assert_eq!(edited.text.trim(), "`new_key`");
  assert_eq!(edited.data, block.data);
  let block = body("**a**");
  assert_eq!(replace_block_text(&block, "missing", "new", true), block);
}

#[test]
fn rich_table_replacement_roundtrips_without_escaped_code_delimiters() {
  let mut doc = import_markdown("| a | b |\n| - | - |\n| old | **tail** |", "root");
  for block in &mut doc.blocks {
    *block = replace_block_text(block, "old", "`key_name`", true);
  }
  let markdown = export_markdown(&doc).unwrap();
  assert!(markdown.contains("`key_name`"), "{markdown}");
  assert!(!markdown.contains("\\`"), "{markdown}");
  let roundtrip = import_markdown(&markdown, "root");
  let table = roundtrip.blocks.iter().find(|b| b.kind == "table").unwrap();
  assert_eq!(block_text_matches(table, "key_name"), 1);
}
