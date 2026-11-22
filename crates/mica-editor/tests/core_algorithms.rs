//! Deterministic unit/integration tests for `mica-editor` pure algorithms:
//! UTF-16 helpers, inline marks & math atoms, selection reconciliation,
//! tables, keyboard input rules, paste heuristics, and LIS move_block diffing.

use mica_editor::*;
use serde_json::json;

#[test]
fn utf16_len_and_boundaries_handle_cjk_and_emoji_surrogates() {
  let s = "你好😀世界";
  assert_eq!(utf16_len(s), 6);
  assert_eq!(snap_utf16_boundary(s, 3, false), 2);
  assert_eq!(snap_utf16_boundary(s, 3, true), 4);
  assert_eq!(utf16_slice(s, 0, 2), "你好");
  assert_eq!(utf16_slice(s, 2, 4), "😀");
  assert_eq!(utf16_slice(s, 4, 6), "世界");
  assert_eq!(utf16_next_boundary(s, 2), 4);
  assert_eq!(utf16_prev_boundary(s, 4), 2);
}

#[test]
fn utf16_replace_and_diff_helpers_work_on_cjk() {
  let (out, s, e) = utf16_replace_range("你好世界", 2, 4, "Rust");
  assert_eq!(out, "你好Rust");
  assert_eq!((s, e), (2, 4));
  assert_eq!(common_prefix_utf16("你好世界", "你好Rust"), 2);
  assert_eq!(common_suffix_utf16("abc你好", "xyz你好", 0), 2);
  let (start, old_end) = diff_edit_range("你好世界", "你好Mica世界", None, None, None);
  assert_eq!((start, old_end), (2, 2));
  assert_eq!(inserted_text_utf16("你好世界", "你好Mica世界", None, None), "Mica");
}

#[test]
fn mark_apply_toggle_remove_and_normalize_merge_adjacent() {
  let m1 = apply_mark(&[], 0, 4, "bold", None, None);
  assert_eq!(m1.len(), 1);
  assert!(range_has_mark(&m1, 0, 4, "bold"));
  assert!(range_has_mark(&m1, 1, 3, "bold"));
  assert!(!range_has_mark(&m1, 0, 5, "bold"));

  let m2 = apply_mark(&m1, 4, 8, "bold", None, None);
  assert_eq!(m2.len(), 1);
  assert_eq!((m2[0].start, m2[0].end), (0, 8));

  let m3 = remove_mark(&m2, 3, 5, "bold");
  assert_eq!(m3.len(), 2);
  assert_eq!((m3[0].start, m3[0].end), (0, 3));
  assert_eq!((m3[1].start, m3[1].end), (5, 8));
}

#[test]
fn mark_shift_split_concat_preserve_integrity() {
  let marks = vec![
    EditorMark::new(2, 6, "bold"),
    EditorMark::new(7, 11, "link").with_href(Some("https://example.com".into())),
  ];
  let shifted = shift_marks(&marks, 0, 0, 3, 15, false);
  assert_eq!((shifted[0].start, shifted[0].end), (5, 9));
  assert_eq!((shifted[1].start, shifted[1].end), (10, 14));

  let (left, right) = split_marks(&marks, 4);
  assert_eq!(left.len(), 1);
  assert_eq!((left[0].start, left[0].end), (2, 4));
  assert_eq!(right.len(), 2);
  assert_eq!((right[0].start, right[0].end), (0, 2));
  assert_eq!((right[1].start, right[1].end), (3, 7));

  let joined = concat_marks(&left, &right, 4);
  assert!(marks_equal(&joined, &marks));
}

#[test]
fn math_atom_run_detection_and_caret_snapping() {
  let marks = vec![EditorMark::new(3, 8, "math")];
  assert_eq!(math_run_at(&marks, 5).map(|m| (m.start, m.end)), Some((3, 8)));
  assert_eq!(
    math_run_starting_at(&marks, 3).map(|m| (m.start, m.end)),
    Some((3, 8))
  );
  assert_eq!(
    math_run_ending_at(&marks, 8).map(|m| (m.start, m.end)),
    Some((3, 8))
  );
  assert_eq!(snap_out_of_math_run(&marks, 5, Some(true)), 8);
  assert_eq!(snap_out_of_math_run(&marks, 5, Some(false)), 3);
}

#[test]
fn link_click_hit_requires_interior_offset() {
  let mark = EditorMark::new(2, 6, "link").with_href(Some("https://mica.dev".into()));
  assert!(link_click_hit(&mark, 3));
  assert!(!link_click_hit(&mark, 2));
  assert!(!link_click_hit(&mark, 6));
}

#[test]
fn word_range_selects_latin_identifiers_and_cjk_segments() {
  assert_eq!(word_range_at("hello_world foo", 3), (0, 11));
  let (s, e) = word_range_at("测试 Rust代码 片段", 4);
  assert!(e > s);
}

#[test]
fn reconcile_selection_tracks_remote_insert_and_block_shift() {
  let old_snap = vec![
    ("b1".to_string(), "Hello world".to_string()),
    ("b2".to_string(), "Second line".to_string()),
  ];
  let new_blocks = vec![
    EditorBlock::new("b0", "paragraph", "Prepended block"),
    EditorBlock::new("b1", "paragraph", "Hey Hello world"),
    EditorBlock::new("b2", "paragraph", "Second line"),
  ];
  let old_sel = DocSelection::caret(0, 6);
  let reconciled = reconcile_selection(&old_snap, Some(&old_sel), &new_blocks).unwrap();
  assert_eq!(reconciled, DocSelection::caret(1, 10));
}

#[test]
fn table_model_crud_alignment_navigation_and_area_copy() {
  let mut table = TableModel::from_block_data(&json!({
    "header": true,
    "aligns": ["left", "center", "right"],
    "rows": [
      ["**A**", "B", "C"],
      ["1", "2", "3"],
      ["4", "5", "6"]
    ]
  }));
  assert_eq!(table.row_count(), 3);
  assert_eq!(table.columns(), 3);
  assert_eq!(table.rows[0][0].text, "A");
  assert!(range_has_mark(&table.rows[0][0].marks, 0, 1, "bold"));

  assert!(table.move_column(0, 1));
  assert_eq!(table.rows[0][1].text, "A");
  assert_eq!(
    table.aligns.as_deref(),
    Some(&["center".into(), "left".into(), "right".into()][..])
  );

  assert!(!table.move_row(0, 1));
  assert!(table.move_row(1, 1));
  assert_eq!(table.rows[2][0].text, "2");

  assert_eq!(
    table.navigate(2, 2, TableNavDirection::Tab),
    TableNavResult::InsertRowAndFocus { row: 3, col: 0 }
  );
  assert_eq!(
    table.navigate(0, 0, TableNavDirection::ShiftTab),
    TableNavResult::ExitBefore
  );

  let clip = table.copy_area(0, 0, 1, 1);
  assert!(clip.plain.contains('\t'));
  assert!(clip.markdown.contains('|'));
  assert!(clip.html.contains("<table"));

  let encoded = table.to_block_data();
  let decoded = TableModel::from_block_data(&encoded);
  assert_eq!(decoded.row_count(), table.row_count());
  assert_eq!(decoded.columns(), table.columns());
  assert_eq!(decoded.aligns, table.aligns);
}

#[test]
fn input_rules_match_headings_lists_quotes_code_and_inline_marks() {
  let blk = |text: &str| EditorBlock::new("b1", "paragraph", text);
  assert!(matches!(
    match_input_rule(&blk("## "), 3),
    Some(InputRuleMatch::ConvertBlock { ref kind, .. }) if kind == "heading"
  ));
  assert!(matches!(
    match_input_rule(&blk("- "), 2),
    Some(InputRuleMatch::ConvertBlock { ref kind, .. }) if kind == "bulleted_list"
  ));
  assert!(matches!(
    match_input_rule(&blk("1. "), 3),
    Some(InputRuleMatch::ConvertBlock { ref kind, .. }) if kind == "numbered_list"
  ));
  assert!(matches!(
    match_input_rule(&blk("- [ ] "), 6),
    Some(InputRuleMatch::ConvertBlock { ref kind, .. }) if kind == "todo"
  ));
  assert!(matches!(
    match_input_rule(&blk("> "), 2),
    Some(InputRuleMatch::ConvertBlock { ref kind, .. }) if kind == "quote"
  ));
  assert!(matches!(
    match_input_rule(&blk("---"), 3),
    Some(InputRuleMatch::InsertDivider)
  ));
  assert!(matches!(
    match_input_rule(&blk("```"), 3),
    Some(InputRuleMatch::ConvertBlock { ref kind, .. }) if kind == "code_block"
  ));
  assert!(matches!(
    match_input_rule(&blk("hi **bold**"), 11),
    Some(InputRuleMatch::InlineMark { ref text, .. }) if text == "hi bold"
  ));
  assert!(matches!(
    match_input_rule(&blk("see `code`"), 10),
    Some(InputRuleMatch::InlineMark { ref text, .. }) if text == "see code"
  ));
  assert!(matches!(
    match_input_rule(&blk("x $a+b$"), 7),
    Some(InputRuleMatch::InlineMark { ref text, .. }) if text == "x a+b"
  ));

  assert_eq!(
    detect_slash_trigger(&blk("/head"), 5),
    Some((0, 5, "head".into()))
  );
  assert_eq!(
    detect_page_link_trigger(&blk("see [[架构"), 8),
    Some((4, 8, "架构".into()))
  );
}

#[test]
fn paste_heuristics_detect_urls_formulas_and_fence_repairs() {
  assert!(looks_like_url("https://github.com/weironz/mica"));
  assert!(!looks_like_url("not a url"));
  assert!(looks_like_image_url("https://cdn.example.com/a.png"));
  assert_eq!(pasted_formula_source("$$\nE = mc^2\n$$"), Some("E = mc^2".to_string()));
  assert_eq!(normalize_line_endings("a\r\nb\rc"), "a\nb\nc");

  let retagged = retag_mislabeled_fences("```markdown\n{\"a\": 1}\n```");
  assert!(retagged.contains("```json") || retagged.contains("{\"a\": 1}"));
}

#[test]
fn lis_minimal_move_ops_moves_only_out_of_order_blocks() {
  let old_ids = vec!["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()];
  let new_ids = vec!["d".to_string(), "a".to_string(), "b".to_string(), "c".to_string()];
  let old_blocks: Vec<EditorBlock> = old_ids
    .iter()
    .map(|id| EditorBlock::new(id, "paragraph", id))
    .collect();
  let target = compute_move_blocks_target(&old_blocks, &["d".to_string()], 0).unwrap();
  let kept = in_order_ids(&old_ids, &target);
  assert_eq!(kept.len(), 3);
  let ids: Vec<String> = target.into_iter().map(|b| b.id).collect();
  assert_eq!(ids, new_ids);
}
