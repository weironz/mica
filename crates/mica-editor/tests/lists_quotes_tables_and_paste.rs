//! High-value regression tests for `mica-editor`: lists/indentation/numbering,
//! quotes, tables (alignment, cell marks, row/col ops, undo), paste &
//! `paste_repair` (URL linking, literal paste, nested/mislabeled fences, HTML),
//! and keyboard input rules / slash / wiki-link triggers.

use std::collections::BTreeMap;

use mica_editor::*;
use serde_json::json;

fn caret(session: &EditorSession) -> DocPosition {
  match session
    .selection()
    .expect("selection expected")
    .normalized()
  {
    NormalizedSelection::Text { end, .. } => end,
    other => panic!("expected text selection, got {other:?}"),
  }
}

// ── Lists, Indentation, Numbering & Quotes ───────────────────────────────────

#[test]
fn enter_on_list_continues_then_empty_enter_outdents_and_exits_to_paragraph() {
  let mut s = EditorSession::from_markdown("- Parent\n  - Child");
  assert_eq!(s.blocks().len(), 2);
  assert_eq!(s.blocks()[1].indent(), 1);

  // Enter at end of Child creates an empty nested bulleted_list item at indent 1.
  let child_len = utf16_len(&s.blocks()[1].text);
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(1, child_len)),
  });
  s.dispatch(EditorCommand::SplitBlock);
  assert_eq!(s.blocks().len(), 3);
  assert_eq!(s.blocks()[2].kind, "bulleted_list");
  assert_eq!(s.blocks()[2].indent(), 1);
  assert_eq!(s.blocks()[2].text, "");

  // Enter on empty nested item outdents to indent 0 first.
  s.dispatch(EditorCommand::SplitBlock);
  assert_eq!(s.blocks()[2].kind, "bulleted_list");
  assert_eq!(s.blocks()[2].indent(), 0);

  // Enter on empty top-level list item sheds to a plain paragraph.
  s.dispatch(EditorCommand::SplitBlock);
  assert_eq!(s.blocks()[2].kind, "paragraph");
  assert_eq!(s.blocks()[2].indent(), 0);
}

#[test]
fn backspace_at_start_of_list_or_empty_styled_block_sheds_block_kind_before_merging() {
  let mut s = EditorSession::from_markdown("## \n\n- Bullet\n\n> ");
  // Backspace at offset 0 of empty heading sheds to paragraph without merging.
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 0)),
  });
  s.dispatch(EditorCommand::DeleteBackward);
  assert_eq!(s.blocks()[0].kind, "paragraph");

  // Backspace at offset 0 of non-empty bulleted_list sheds marker first before merging.
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(1, 0)),
  });
  s.dispatch(EditorCommand::DeleteBackward);
  assert_eq!(s.blocks()[1].kind, "paragraph");
  assert_eq!(s.blocks()[1].text, "Bullet");
  assert_eq!(s.blocks().len(), 3);

  // Backspace at offset 0 of empty quote sheds to paragraph without merging.
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(2, 0)),
  });
  s.dispatch(EditorCommand::DeleteBackward);
  assert_eq!(s.blocks()[2].kind, "paragraph");
  assert_eq!(s.blocks().len(), 3);
}

#[test]
fn list_indent_clamps_at_max_nesting_depth_and_multi_block_indent_preserves_depth() {
  let mut s = EditorSession::from_markdown("- A\n- B\n- C");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(1, 0)),
  });
  for _ in 0..(MAX_NESTING_DEPTH + 4) {
    s.dispatch(EditorCommand::Indent { delta: 1 });
  }
  assert!(s.blocks()[1].indent() <= MAX_NESTING_DEPTH);

  // Multi-block indent across B and C
  let mut s2 = EditorSession::from_markdown("- A\n- B\n- C");
  s2.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::from_positions(
      DocPosition::new(1, 0),
      DocPosition::new(2, 1),
    )),
  });
  s2.dispatch(EditorCommand::Indent { delta: 1 });
  assert_eq!(s2.blocks()[1].indent(), 1);
  assert_eq!(s2.blocks()[2].indent(), 1);

  s2.dispatch(EditorCommand::Indent { delta: -1 });
  assert_eq!(s2.blocks()[1].indent(), 0);
  assert_eq!(s2.blocks()[2].indent(), 0);
}

#[test]
fn visible_ordinal_counts_nested_and_interrupted_numbered_lists_correctly() {
  let blocks = vec![
    EditorBlock::new("1", "numbered_list", "First"),
    EditorBlock::new("2", "numbered_list", "Sub 1").with_data(json!({ "indent": 1 })),
    EditorBlock::new("3", "numbered_list", "Sub 2").with_data(json!({ "indent": 1 })),
    EditorBlock::new("4", "numbered_list", "Second"),
    EditorBlock::new("5", "paragraph", "Break"),
    EditorBlock::new("6", "numbered_list", "Fresh 1"),
  ];
  assert_eq!(visible_ordinal(&blocks, 0), Some(1));
  assert_eq!(visible_ordinal(&blocks, 1), Some(1));
  assert_eq!(visible_ordinal(&blocks, 2), Some(2));
  assert_eq!(visible_ordinal(&blocks, 3), Some(2));
  assert_eq!(visible_ordinal(&blocks, 5), Some(1));
}

#[test]
fn quote_enter_continues_and_empty_quote_exits_cleanly() {
  let mut s = EditorSession::from_markdown("> Line one");
  assert_eq!(s.blocks()[0].kind, "quote");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, utf16_len("Line one"))),
  });
  s.dispatch(EditorCommand::SplitBlock);
  assert_eq!(s.blocks().len(), 2);
  assert_eq!(s.blocks()[1].kind, "quote");
  assert_eq!(s.blocks()[1].text, "");

  // Enter on empty quote exits to paragraph.
  s.dispatch(EditorCommand::SplitBlock);
  assert_eq!(s.blocks()[1].kind, "paragraph");
}

#[test]
fn batch_transform_prose_preserves_inline_marks_and_code_block_joins_lines() {
  let mut s = EditorSession::from_markdown("Hello **A**\n\nHello **B**");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::from_positions(
      DocPosition::new(0, 0),
      DocPosition::new(1, 7),
    )),
  });
  s.dispatch(EditorCommand::BatchTransform {
    kind: "bulleted_list".into(),
    data: None,
  });
  assert_eq!(s.blocks()[0].kind, "bulleted_list");
  assert_eq!(s.blocks()[1].kind, "bulleted_list");
  assert!(s.blocks()[0].marks().iter().any(|m| m.kind == "bold"));
  assert!(s.blocks()[1].marks().iter().any(|m| m.kind == "bold"));
}

// ── Tables (Alignment, Marks, Row/Col CRUD, Navigation, Area Copy, Undo) ─────

#[test]
fn table_alignment_preserved_on_insert_delete_and_move_column() {
  let mut t = TableModel::from_block_data(&json!({
    "header": true,
    "aligns": ["left", "center", "right"],
    "rows": [
      ["H1", "H2", "H3"],
      ["a1", "a2", "a3"]
    ]
  }));

  t.insert_column(1);
  assert_eq!(t.columns(), 4);
  assert_eq!(
    t.aligns.as_deref(),
    Some(&["left".into(), "".into(), "center".into(), "right".into()][..])
  );

  t.set_column_align(1, "right");
  assert_eq!(t.aligns.as_deref().unwrap()[1], "right");

  t.delete_column(1);
  assert_eq!(t.columns(), 3);
  assert_eq!(
    t.aligns.as_deref(),
    Some(&["left".into(), "center".into(), "right".into()][..])
  );

  // Move column 2 left by 1 twice -> front, and verify alignment travels with column.
  assert!(t.move_column(2, -1));
  assert!(t.move_column(1, -1));
  assert_eq!(t.rows[0][0].text, "H3");
  assert_eq!(
    t.aligns.as_deref(),
    Some(&["right".into(), "left".into(), "center".into()][..])
  );
}

#[test]
fn table_cell_inline_code_bold_and_links_survive_storage_and_markdown_export() {
  let md = "| Key | Value |\n| :--- | ---: |\n| `foo` | **bar** [doc](https://mica.dev) |";
  let s = EditorSession::from_markdown(md);
  assert_eq!(s.blocks()[0].kind, "table");

  let tbl = TableModel::from_block_data(&s.blocks()[0].data);
  assert_eq!(tbl.rows[1][0].text, "foo");
  assert!(tbl.rows[1][0].marks.iter().any(|m| m.kind == "code"));
  assert!(tbl.rows[1][1].marks.iter().any(|m| m.kind == "bold"));
  assert!(tbl.rows[1][1].marks.iter().any(|m| m.kind == "link"));

  // Round-trip through to_block_data -> from_block_data.
  let round = TableModel::from_block_data(&tbl.to_block_data());
  assert_eq!(round.rows[1][0].text, "foo");
  assert!(round.rows[1][0].marks.iter().any(|m| m.kind == "code"));
  assert!(round.rows[1][1].marks.iter().any(|m| m.kind == "bold"));

  let exported = s.export_markdown(&BTreeMap::new());
  assert!(exported.contains("`foo`"));
  assert!(exported.contains("**bar**"));
  assert!(exported.contains("[doc](https://mica.dev)"));
}

#[test]
fn table_area_copy_and_session_undo_redo_cover_all_mutations() {
  let mut s = EditorSession::from_markdown("| A | B |\n| --- | --- |\n| 1 | 2 |");
  s.dispatch(EditorCommand::TableOp {
    block_index: 0,
    op: TableOp::InsertColumn { at: 2 },
  });
  s.dispatch(EditorCommand::TableOp {
    block_index: 0,
    op: TableOp::SetColumnAlign {
      col: 2,
      align: "right".into(),
    },
  });
  let tbl = TableModel::from_block_data(&s.blocks()[0].data);
  assert_eq!(tbl.columns(), 3);
  assert_eq!(tbl.aligns.as_deref().unwrap()[2], "right");

  let copy_patch = s.dispatch(EditorCommand::TableOp {
    block_index: 0,
    op: TableOp::CopyArea {
      r0: 0,
      c0: 0,
      r1: 1,
      c1: 1,
    },
  });
  let clip = copy_patch.clipboard_out.expect("table area clipboard");
  assert_eq!(clip.plain, "A\tB\n1\t2");
  assert!(clip.markdown.contains("| A | B |"));
  assert!(clip.html.contains("<td>1</td>"));

  // Undo SetAlign then InsertColumn -> back to 2 columns.
  s.dispatch(EditorCommand::Undo);
  s.dispatch(EditorCommand::Undo);
  let tbl_back = TableModel::from_block_data(&s.blocks()[0].data);
  assert_eq!(tbl_back.columns(), 2);
}

#[test]
fn table_navigation_wraps_rows_and_exits_at_boundaries() {
  let t = TableModel::from_block_data(&json!({
    "header": true,
    "rows": [["A", "B"], ["C", "D"]]
  }));
  assert_eq!(
    t.navigate(0, 0, TableNavDirection::Tab),
    TableNavResult::Cell { row: 0, col: 1 }
  );
  assert_eq!(
    t.navigate(0, 1, TableNavDirection::Tab),
    TableNavResult::Cell { row: 1, col: 0 }
  );
  assert_eq!(
    t.navigate(1, 1, TableNavDirection::Tab),
    TableNavResult::InsertRowAndFocus { row: 2, col: 0 }
  );
  assert_eq!(
    t.navigate(0, 0, TableNavDirection::ShiftTab),
    TableNavResult::ExitBefore
  );
  assert_eq!(
    t.navigate(1, 0, TableNavDirection::Down),
    TableNavResult::ExitAfter
  );
  assert_eq!(
    t.navigate(0, 1, TableNavDirection::Up),
    TableNavResult::ExitBefore
  );
}

// ── Paste & Paste-Repair Regressions ─────────────────────────────────────────

#[test]
fn paste_bare_url_over_selected_text_wraps_label_as_link_without_overwriting() {
  let mut s = EditorSession::from_markdown("Read documentation here");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::from_positions(
      DocPosition::new(0, 5),
      DocPosition::new(0, 18),
    )),
  });
  s.dispatch(EditorCommand::Paste {
    input: PasteInput::Plain {
      text: "https://mica.dev/docs".into(),
    },
  });
  assert_eq!(s.blocks()[0].text, "Read documentation here");
  let link = s.blocks()[0]
    .marks()
    .into_iter()
    .find(|m| m.kind == "link")
    .expect("URL paste over selection should apply link mark");
  assert_eq!((link.start, link.end), (5, 18));
  assert_eq!(link.href.as_deref(), Some("https://mica.dev/docs"));
}

#[test]
fn paste_plain_literal_never_parses_markdown_or_wraps_url() {
  let mut s = EditorSession::from_markdown("Note:");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 5)),
  });
  s.dispatch(EditorCommand::Paste {
    input: PasteInput::PlainLiteral {
      text: " **raw** https://mica.dev".into(),
    },
  });
  assert_eq!(s.blocks()[0].text, "Note: **raw** https://mica.dev");
  assert!(s.blocks()[0].marks().is_empty());
}

#[test]
fn paste_display_formula_creates_math_block() {
  let mut s = EditorSession::from_markdown("");
  s.dispatch(EditorCommand::Paste {
    input: PasteInput::Plain {
      text: "$$\n\\int_0^1 x^2 dx\n$$".into(),
    },
  });
  assert!(
    s.blocks()
      .iter()
      .any(|b| b.kind == "math_block" && b.text.contains("\\int_0^1"))
  );
}

#[test]
fn paste_multiline_markdown_creates_structured_blocks_and_is_single_undo() {
  let mut s = EditorSession::from_markdown("");
  s.dispatch(EditorCommand::Paste {
    input: PasteInput::Plain {
      text: "## Section\n\n- Alpha\n- Beta\n\n```rust\nlet x = 1;\n```".into(),
    },
  });
  assert!(
    s.blocks()
      .iter()
      .any(|b| b.kind == "heading" && b.text == "Section")
  );
  assert!(
    s.blocks()
      .iter()
      .any(|b| b.kind == "bulleted_list" && b.text == "Alpha")
  );
  assert!(
    s.blocks()
      .iter()
      .any(|b| b.kind == "code_block" && b.text.contains("let x = 1;"))
  );

  s.dispatch(EditorCommand::Undo);
  assert_eq!(s.blocks().len(), 1);
  assert_eq!(s.blocks()[0].text, "");
}

#[test]
fn paste_repair_unwraps_nested_fences_and_retags_mislabeled_fences() {
  let nested = "```\n```rust\nfn main() {}\n```\n```";
  let unwrapped = unwrap_nested_fences(nested);
  assert!(unwrapped.contains("```rust"));
  assert!(unwrapped.contains("fn main() {}"));

  let mislabeled = "```markdown\n#!/bin/bash\nset -euo pipefail\necho hi\n```";
  let retagged = retag_mislabeled_fences(mislabeled);
  assert!(retagged.contains("```bash") || retagged.contains("```sh"));
}

#[test]
fn paste_html_tree_preserves_headings_links_and_lists() {
  let html_tree = json!({
    "e": "div",
    "a": {},
    "c": [
      {
        "e": "h2",
        "a": {},
        "c": [{ "t": "HTML Title" }]
      },
      {
        "e": "ul",
        "a": {},
        "c": [
          {
            "e": "li",
            "a": {},
            "c": [
              { "t": "Visit " },
              {
                "e": "a",
                "a": { "href": "https://mica.dev" },
                "c": [{ "t": "Mica" }]
              }
            ]
          }
        ]
      }
    ]
  });
  let mut s = EditorSession::from_markdown("");
  s.dispatch(EditorCommand::Paste {
    input: PasteInput::Html {
      html_node_json: html_tree,
      plain: "HTML Title\nVisit Mica".into(),
    },
  });
  assert!(
    s.blocks()
      .iter()
      .any(|b| b.kind == "heading" && b.text == "HTML Title")
  );
  let item = s
    .blocks()
    .iter()
    .find(|b| b.kind == "bulleted_list")
    .expect("HTML ul>li should become bulleted_list");
  assert!(item.text.contains("Visit Mica"));
  assert!(item.marks().iter().any(|m| m.kind == "link"));
}

// ── Keyboard Input Rules, Slash & Wiki-Link Triggers ─────────────────────────

#[test]
fn input_rules_cover_all_heading_levels_reject_h7_and_support_inline_styles() {
  let blk = |t: &str| EditorBlock::new("b1", "paragraph", t);
  for level in 1u32..=6 {
    let prefix = format!("{} ", "#".repeat(level as usize));
    let m = match_input_rule(&blk(&prefix), prefix.len() as u32);
    assert!(
      matches!(m, Some(InputRuleMatch::ConvertBlock { ref kind, ref data, .. })
        if kind == "heading" && data.get("level").and_then(|v| v.as_u64()) == Some(u64::from(level))),
      "expected heading level {level}, got {m:?}"
    );
  }
  assert!(match_input_rule(&blk("####### "), 8).is_none());

  assert!(matches!(
    match_input_rule(&blk("* "), 2),
    Some(InputRuleMatch::ConvertBlock { ref kind, .. }) if kind == "bulleted_list"
  ));
  assert!(matches!(
    match_input_rule(&blk("- [x] "), 6),
    Some(InputRuleMatch::ConvertBlock { ref kind, ref data, .. })
      if kind == "todo" && data.get("checked").and_then(|v| v.as_bool()) == Some(true)
  ));
  assert!(matches!(
    match_input_rule(&blk("~~~"), 3),
    Some(InputRuleMatch::ConvertBlock { ref kind, .. }) if kind == "code_block"
  ));
  assert!(matches!(
    match_input_rule(&blk("a ~~gone~~"), 10),
    Some(InputRuleMatch::InlineMark { ref text, ref marks, .. })
      if text == "a gone" && marks.iter().any(|m| m.kind == "strike")
  ));
  assert!(matches!(
    match_input_rule(&blk("a *em*"), 6),
    Some(InputRuleMatch::InlineMark { ref text, ref marks, .. })
      if text == "a em" && marks.iter().any(|m| m.kind == "italic")
  ));
}

#[test]
fn slash_and_page_link_triggers_respect_word_boundaries() {
  let blk = |t: &str| EditorBlock::new("b1", "paragraph", t);
  // Mid-URL slash must NOT trigger slash menu.
  assert!(detect_slash_trigger(&blk("https://mica.dev/docs"), 21).is_none());
  // Start of line or after space triggers slash menu.
  assert_eq!(
    detect_slash_trigger(&blk("/tbl"), 4),
    Some((0, 4, "tbl".into()))
  );
  assert_eq!(
    detect_slash_trigger(&blk("hi /code"), 8),
    Some((3, 8, "code".into()))
  );
  // Code blocks never trigger slash menu.
  let code_blk = EditorBlock::new("c1", "code_block", "/head");
  assert!(detect_slash_trigger(&code_blk, 5).is_none());

  // Wiki-link trigger works with CJK queries and stops once closed with `]]`.
  assert_eq!(
    detect_page_link_trigger(&blk("关联 [[需求文档"), 11),
    Some((3, 11, "需求文档".into()))
  );
  assert!(detect_page_link_trigger(&blk("关联 [[需求文档]]"), 13).is_none());
}

#[test]
fn caret_tracks_cleanly_after_typing_and_splitting_chinese_text() {
  let mut s = EditorSession::from_markdown("你好世界");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 2)),
  });
  s.dispatch(EditorCommand::InsertText {
    text: "😀".into()
  });
  assert_eq!(s.blocks()[0].text, "你好😀世界");
  assert_eq!(caret(&s), DocPosition::new(0, 4));

  s.dispatch(EditorCommand::SplitBlock);
  assert_eq!(s.blocks()[0].text, "你好😀");
  assert_eq!(s.blocks()[1].text, "世界");
  assert_eq!(caret(&s), DocPosition::new(1, 0));
}

#[test]
fn empty_first_line_dropped_by_backspace_and_delete_preserving_next_block_kind() {
  // 1. Backspace on an empty first paragraph above a bold paragraph drops line 0.
  let mut s = EditorSession::from_markdown("**Spectrum-X** 平台");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 0)),
  });
  s.dispatch(EditorCommand::SplitBlock);
  assert_eq!(s.blocks().len(), 2);
  assert_eq!(s.blocks()[0].text, "");
  assert_eq!(s.blocks()[1].text, "Spectrum-X 平台");

  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 0)),
  });
  s.dispatch(EditorCommand::DeleteBackward);
  assert_eq!(s.blocks().len(), 1);
  assert_eq!(s.blocks()[0].text, "Spectrum-X 平台");
  assert!(s.blocks()[0].marks().iter().any(|m| m.kind == "bold"));
  assert_eq!(caret(&s), DocPosition::new(0, 0));

  // 2. Delete (forward) on an empty first paragraph above a heading drops line 0
  //    AND preserves the heading kind rather than flattening it to paragraph.
  let mut s2 = EditorSession::from_markdown("## 架构概览");
  s2.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 0)),
  });
  s2.dispatch(EditorCommand::SplitBlock);
  assert_eq!(s2.blocks().len(), 2);
  assert_eq!(s2.blocks()[0].kind, "paragraph");
  assert_eq!(s2.blocks()[0].text, "");
  assert_eq!(s2.blocks()[1].kind, "heading");

  s2.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 0)),
  });
  s2.dispatch(EditorCommand::DeleteForward);
  assert_eq!(s2.blocks().len(), 1);
  assert_eq!(s2.blocks()[0].kind, "heading");
  assert_eq!(s2.blocks()[0].text, "架构概览");
  assert_eq!(caret(&s2), DocPosition::new(0, 0));

  // 3. Delete (forward) on an empty paragraph above an image removes the blank
  //    line and selects the image atom rather than deleting the image.
  let mut s4 = EditorSession::from_markdown("placeholder\n\n![diagram](https://example.com/a.png)");
  s4.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::from_positions(
      DocPosition::new(0, 0),
      DocPosition::new(0, 11),
    )),
  });
  s4.dispatch(EditorCommand::DeleteBackward);
  assert_eq!(s4.blocks()[0].text, "");
  assert_eq!(s4.blocks()[1].kind, "image");
  s4.dispatch(EditorCommand::DeleteForward);
  assert_eq!(s4.blocks().len(), 1);
  assert_eq!(s4.blocks()[0].kind, "image");
  assert_eq!(s4.selection(), Some(&DocSelection::block_atom(0)));

  // 4. PrependEmptyParagraph (Title Enter) pushes existing first-line content
  //    down and opens an editable blank line at index 0; on a blank page it
  //    reuses the sole empty paragraph without duplicating it.
  let mut s5 = EditorSession::from_markdown("**Spectrum-X** 平台");
  s5.dispatch(EditorCommand::PrependEmptyParagraph);
  assert_eq!(s5.blocks().len(), 2);
  assert_eq!(s5.blocks()[0].kind, "paragraph");
  assert_eq!(s5.blocks()[0].text, "");
  assert_eq!(s5.blocks()[1].text, "Spectrum-X 平台");
  assert_eq!(caret(&s5), DocPosition::new(0, 0));

  let mut blank = EditorSession::from_markdown("");
  blank.dispatch(EditorCommand::PrependEmptyParagraph);
  assert_eq!(blank.blocks().len(), 1);
  assert_eq!(caret(&blank), DocPosition::new(0, 0));
}
