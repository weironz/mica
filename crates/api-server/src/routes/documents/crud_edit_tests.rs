use super::*;
use mica_app_core::documents::{apply_operations, export_markdown, import_markdown};
use serde_json::json;

fn request(find: &str, replace: &str) -> UpdateMarkdownRequest {
  serde_json::from_value(json!({"mode":"find_replace", "find":find, "replace":replace})).unwrap()
}

#[test]
fn find_replace_edits_inline_table_and_list_markdown_without_rebuilding_blocks() {
  for (markdown, find) in [
    (
      "run `omp --version`, confirmed",
      "run `omp --version`, confirmed",
    ),
    (
      "- **pending**: account login",
      "- **pending**: account login",
    ),
    (
      "| key | value |\n| - | - |\n| kind | full executable |",
      "full executable",
    ),
  ] {
    let current = import_markdown(markdown, "root");
    let ids: Vec<_> = current.blocks.iter().map(|b| b.id.clone()).collect();
    let ops = markdown_update_ops(&current, &request(find, "`new_key`"), Uuid::nil()).unwrap();
    assert!(
      ops
        .iter()
        .all(|op| matches!(op, DocumentOperation::UpdateBlock { .. }))
    );
    let updated = apply_operations(current, &ops).unwrap();
    assert_eq!(
      updated
        .blocks
        .iter()
        .map(|b| b.id.clone())
        .collect::<Vec<_>>(),
      ids
    );
    let markdown = export_markdown(&updated).unwrap();
    assert!(markdown.contains("`new_key`"), "{markdown}");
    assert!(!markdown.contains("\\`"), "{markdown}");
  }
}

#[test]
fn unique_counts_formatted_plain_and_table_matches_together() {
  let current = import_markdown("**old**\n\nold\n\n| a |\n| - |\n| old |", "root");
  let mut req = request("old", "new");
  req.unique = true;
  let err = markdown_update_ops(&current, &req, Uuid::nil()).unwrap_err();
  assert!(err.contains("3 time(s)"), "{err}");
  req.unique = false;
  assert_eq!(
    markdown_update_ops(&current, &req, Uuid::nil())
      .unwrap()
      .len(),
    3
  );
}

#[test]
fn find_replace_rejects_cross_block_matches_and_invalid_formats() {
  let current = import_markdown("one\n\ntwo", "root");
  let err = markdown_update_ops(&current, &request("one\n\ntwo", "x"), Uuid::nil()).unwrap_err();
  assert!(err.contains("one block or table cell"), "{err}");
  let mut req = request("one", "x");
  req.replacement_format = "typo".into();
  assert!(
    markdown_update_ops(&current, &req, Uuid::nil())
      .unwrap_err()
      .contains("replacement_format")
  );
}

#[test]
fn find_replace_supports_explicit_literal_replacement() {
  let current = import_markdown("old", "root");
  let mut req = request("old", "`key_name`");
  req.replacement_format = "plain_text".into();
  let ops = markdown_update_ops(&current, &req, Uuid::nil()).unwrap();
  let updated = apply_operations(current, &ops).unwrap();
  assert_eq!(
    updated.blocks.iter().find(|b| b.id != "root").unwrap().text,
    "`key_name`"
  );
}

#[test]
fn outline_maps_unicode_previews_and_types_to_exact_block_ids() {
  let current = import_markdown(
    "# heading\n\n- 列表 😀 item\n\n| key |\n| - |\n| value |",
    "root",
  );
  let outline = outline_from_payload(&current, 9);
  assert_eq!(outline.blocks.len(), outline.block_ids.len());
  for (block, id) in outline.blocks.iter().zip(&outline.block_ids) {
    assert_eq!(&block.block_id, id);
    assert!(!block.preview.is_empty());
    assert!(block.preview.chars().count() <= 40);
  }
  assert_eq!(outline.blocks[1].kind, "bulleted_list");
  assert_eq!(outline.blocks[1].preview, "列表 😀 item");
  assert_eq!(outline.blocks[2].kind, "table");
  assert_eq!(outline.blocks[2].preview, "key value");
}
