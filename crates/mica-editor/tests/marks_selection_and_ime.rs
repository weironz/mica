//! High-value regression tests for `mica-editor`: inline marks, UTF-16 / CJK /
//! emoji boundaries, selection normalization, IME composition, and remote CRDT
//! selection reconciliation (ported from `main` Flutter editor regressions).

use std::collections::BTreeMap;

use mica_editor::*;
use serde_json::json;

fn caret(session: &EditorSession) -> DocPosition {
  match session.selection().expect("selection expected").normalized() {
    NormalizedSelection::Text { end, .. } => end,
    other => panic!("expected text selection, got {other:?}"),
  }
}

// ── Marks & Math Atom Regressions ────────────────────────────────────────────

#[test]
fn overlapping_different_mark_kinds_coexist_without_clobbering() {
  let m = apply_mark(&[], 0, 10, "bold", None, None);
  let m = apply_mark(&m, 4, 12, "italic", None, None);
  let m = apply_mark(&m, 2, 8, "link", Some("https://mica.dev"), None);

  assert!(range_has_mark(&m, 0, 10, "bold"));
  assert!(range_has_mark(&m, 4, 12, "italic"));
  assert!(range_has_mark(&m, 2, 8, "link"));
  assert!(!range_has_mark(&m, 0, 2, "link"));
  assert!(!range_has_mark(&m, 10, 12, "bold"));
}

#[test]
fn adjacent_links_merge_only_when_href_matches() {
  let same = apply_mark(&[], 0, 4, "link", Some("https://a.test"), None);
  let same = apply_mark(&same, 4, 8, "link", Some("https://a.test"), None);
  assert_eq!(same.len(), 1);
  assert_eq!((same[0].start, same[0].end), (0, 8));

  let diff = apply_mark(&[], 0, 4, "link", Some("https://a.test"), None);
  let diff = apply_mark(&diff, 4, 8, "link", Some("https://b.test"), None);
  assert_eq!(diff.len(), 2);
  assert_eq!(diff[0].href.as_deref(), Some("https://a.test"));
  assert_eq!(diff[1].href.as_deref(), Some("https://b.test"));
}

#[test]
fn toggle_mark_on_partial_overlap_fills_first_then_removes_all() {
  let m0 = apply_mark(&[], 0, 4, "bold", None, None);
  let m1 = apply_or_remove_mark(&m0, 2, 8, "bold", None, None, true);
  assert!(range_has_mark(&m1, 0, 8, "bold"));
  assert_eq!(m1.len(), 1);

  let m2 = apply_or_remove_mark(&m1, 2, 6, "bold", None, None, false);
  assert_eq!(m2.len(), 2);
  assert_eq!((m2[0].start, m2[0].end), (0, 2));
  assert_eq!((m2[1].start, m2[1].end), (6, 8));
}

#[test]
fn shift_marks_handles_insert_at_boundaries_and_interior_deletion() {
  let marks = vec![
    EditorMark::new(4, 10, "bold"),
    EditorMark::new(12, 16, "link").with_href(Some("https://mica.dev".into())),
  ];

  // Insert 3 chars strictly inside bold [4, 10): bold expands to [4, 13), link shifts to [15, 19).
  let ins_mid = shift_marks(&marks, 6, 6, 3, 25, false);
  assert_eq!((ins_mid[0].start, ins_mid[0].end), (4, 13));
  assert_eq!((ins_mid[1].start, ins_mid[1].end), (15, 19));

  // Insert 2 chars at bold's leading edge (4): `pin_at_start=true` expands left,
  // `pin_at_start=false` shifts bold right.
  let shift_right = shift_marks(&marks, 4, 4, 2, 25, false);
  assert_eq!((shift_right[0].start, shift_right[0].end), (6, 12));
  let pin_start = shift_marks(&marks, 4, 4, 2, 25, true);
  assert_eq!((pin_start[0].start, pin_start[0].end), (4, 12));

  // Link NEVER expands at its leading edge even when `pin_at_start=true`.
  let link_start = shift_marks(&marks, 12, 12, 2, 25, true);
  assert_eq!((link_start[1].start, link_start[1].end), (14, 18));

  // Delete strictly inside bold [5, 8) (delta = -3) shrinks it to [4, 7).
  let del_mid = shift_marks(&marks, 5, 8, -3, 25, false);
  assert_eq!((del_mid[0].start, del_mid[0].end), (4, 7));

  // Delete spanning all of bold [2, 11) (delta = -9) drops bold and shifts link left by 9.
  let del_all = shift_marks(&marks, 2, 11, -9, 25, false);
  assert_eq!(del_all.len(), 1);
  assert_eq!(del_all[0].kind, "link");
  assert_eq!((del_all[0].start, del_all[0].end), (3, 7));
}

#[test]
fn split_and_concat_marks_roundtrip_with_cjk_and_emoji_offsets() {
  let text = "你好😀世界Rust";
  // UTF-16 offsets: 你(0..1) 好(1..2) 😀(2..4) 世(4..5) 界(5..6) R(6..7) u(7..8) s(8..9) t(9..10)
  assert_eq!(utf16_len(text), 10);
  let marks = vec![
    EditorMark::new(0, 6, "bold"),
    EditorMark::new(4, 10, "italic"),
  ];
  let (left, right) = split_marks(&marks, 4);
  let rejoined = concat_marks(&left, &right, 4);
  assert!(marks_equal(&rejoined, &marks));
}

#[test]
fn math_atom_backspace_and_forward_delete_remove_entire_formula_atom() {
  let mut s = EditorSession::from_markdown("See $a+b$ end");
  let math = s.blocks()[0]
    .marks()
    .into_iter()
    .find(|m| m.kind == "math")
    .expect("inline math mark expected");

  // Backspace right at end of math atom removes the entire atom.
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, math.end)),
  });
  s.dispatch(EditorCommand::DeleteBackward);
  assert!(!s.blocks()[0].text.contains("a+b"));
  assert!(s.blocks()[0].marks().iter().all(|m| m.kind != "math"));

  // Undo restores the math atom cleanly.
  s.dispatch(EditorCommand::Undo);
  assert!(s.blocks()[0].text.contains("a+b"));
  assert!(s.blocks()[0].marks().iter().any(|m| m.kind == "math"));

  // Forward Delete right at start of math atom also removes the entire atom.
  let math2 = s.blocks()[0]
    .marks()
    .into_iter()
    .find(|m| m.kind == "math")
    .expect("inline math mark after undo");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, math2.start)),
  });
  s.dispatch(EditorCommand::DeleteForward);
  assert!(!s.blocks()[0].text.contains("a+b"));
}

#[test]
fn set_inline_math_empty_source_deletes_atom_and_nonempty_updates_it() {
  let mut s = EditorSession::from_markdown("F = $m a$!");
  let mk = s.blocks()[0]
    .marks()
    .into_iter()
    .find(|m| m.kind == "math")
    .unwrap();
  s.dispatch(EditorCommand::SetInlineMath {
    block_index: 0,
    start: mk.start,
    end: mk.end,
    source: r"\frac{dp}{dt}".into(),
  });
  assert!(s.blocks()[0].text.contains(r"\frac{dp}{dt}"));

  let mk2 = s.blocks()[0]
    .marks()
    .into_iter()
    .find(|m| m.kind == "math")
    .unwrap();
  s.dispatch(EditorCommand::SetInlineMath {
    block_index: 0,
    start: mk2.start,
    end: mk2.end,
    source: "".into(),
  });
  assert!(s.blocks()[0].marks().iter().all(|m| m.kind != "math"));
}

// ── Backwards Selection, ActiveMarks & Cross-Block Editing ───────────────────

#[test]
fn backwards_selection_toggle_mark_copy_and_delete_behave_identically_to_forward() {
  let mut s = EditorSession::from_markdown("Alpha Beta Gamma");
  // Backwards selection over "Beta" [10 -> 6]
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::from_positions(
      DocPosition::new(0, 10),
      DocPosition::new(0, 6),
    )),
  });
  s.dispatch(EditorCommand::ToggleMark {
    kind: "bold".into(),
    href: None,
  });
  assert!(range_has_mark(&s.blocks()[0].marks(), 6, 10, "bold"));

  let clip = s
    .dispatch(EditorCommand::CopySelection {
      image_urls: BTreeMap::new(),
    })
    .clipboard_out
    .expect("clipboard payload");
  assert_eq!(clip.plain, "Beta");
  assert_eq!(clip.markdown, "**Beta**");

  s.dispatch(EditorCommand::CutSelection {
    image_urls: BTreeMap::new(),
  });
  assert_eq!(s.blocks()[0].text, "Alpha  Gamma");
  assert_eq!(caret(&s), DocPosition::new(0, 6));
}

#[test]
fn active_marks_reports_formatting_at_caret_and_across_multi_block_selection() {
  let s = EditorSession::from_markdown("**Hello** plain\n\n**World**");
  let inside_bold = active_marks_for_selection(s.blocks(), Some(&DocSelection::caret(0, 2)));
  assert!(inside_bold.kinds.contains("bold"));
  assert!(!inside_bold.kinds.contains("italic"));

  let on_plain = active_marks_for_selection(s.blocks(), Some(&DocSelection::caret(0, 8)));
  assert!(!on_plain.kinds.contains("bold"));

  // Selection covering bold + plain must NOT report bold across whole range.
  let mixed = active_marks_for_selection(
    s.blocks(),
    Some(&DocSelection::from_positions(
      DocPosition::new(0, 0),
      DocPosition::new(1, 5),
    )),
  );
  assert!(!mixed.kinds.contains("bold"));

  // Selection covering only bold in block 1 reports bold.
  let all_bold = active_marks_for_selection(
    s.blocks(),
    Some(&DocSelection::from_positions(
      DocPosition::new(1, 0),
      DocPosition::new(1, 5),
    )),
  );
  assert!(all_bold.kinds.contains("bold"));
}

#[test]
fn cross_block_insert_text_replaces_spanning_selection_and_merges_tail() {
  let mut s = EditorSession::from_markdown("One two\n\nMiddle block\n\nThree four");
  assert_eq!(s.blocks().len(), 3);
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::from_positions(
      DocPosition::new(0, 4),
      DocPosition::new(2, 6),
    )),
  });
  s.dispatch(EditorCommand::InsertText {
    text: "REPLACED ".into(),
  });
  assert_eq!(s.blocks().len(), 1);
  assert_eq!(s.blocks()[0].text, "One REPLACED four");
  assert_eq!(caret(&s), DocPosition::new(0, 13));

  s.dispatch(EditorCommand::Undo);
  assert_eq!(s.blocks().len(), 3);
  assert_eq!(s.blocks()[1].text, "Middle block");
}

#[test]
fn set_link_updates_href_and_label_text_and_removes_when_href_none() {
  let mut s = EditorSession::from_markdown("Click here now");
  s.dispatch(EditorCommand::SetLink {
    block_index: 0,
    start: 6,
    end: 10,
    href: Some("https://mica.dev".into()),
    text: Some("Mica Docs".into()),
  });
  assert_eq!(s.blocks()[0].text, "Click Mica Docs now");
  assert!(range_has_mark(&s.blocks()[0].marks(), 6, 15, "link"));

  s.dispatch(EditorCommand::SetLink {
    block_index: 0,
    start: 6,
    end: 15,
    href: None,
    text: None,
  });
  assert!(!range_has_mark(&s.blocks()[0].marks(), 6, 15, "link"));
}

// ── IME Composition & Input-Rule Safety ──────────────────────────────────────

#[test]
fn ime_preedit_suppresses_input_rules_until_commit_and_esc_leaves_no_residue() {
  let mut s = EditorSession::from_markdown("前缀");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 2)),
  });

  // User starts typing pinyin while composing: no input rule or duplicate commit.
  s.dispatch(EditorCommand::ImeUpdate {
    text: "前缀ni'hao".into(),
    base: 8,
    extent: 8,
    composing_valid: true,
    composing_collapsed: false,
    shift_pressed: false,
  });
  assert_eq!(s.blocks()[0].text, "前缀ni'hao");

  // User presses Esc in IME -> IME reverts buffer back to "前缀" without deleting prefix.
  s.dispatch(EditorCommand::ImeUpdate {
    text: "前缀".into(),
    base: 2,
    extent: 2,
    composing_valid: false,
    composing_collapsed: true,
    shift_pressed: false,
  });
  s.dispatch(EditorCommand::CommitComposition);
  assert_eq!(s.blocks()[0].text, "前缀");
  assert_eq!(caret(&s), DocPosition::new(0, 2));

  // Subsequent CJK + ASCII mixed input commits cleanly without duplication.
  s.dispatch(EditorCommand::ImeUpdate {
    text: "前缀zhongwen".into(),
    base: 10,
    extent: 10,
    composing_valid: true,
    composing_collapsed: false,
    shift_pressed: false,
  });
  s.dispatch(EditorCommand::ImeUpdate {
    text: "前缀中文Rust".into(),
    base: 8,
    extent: 8,
    composing_valid: false,
    composing_collapsed: true,
    shift_pressed: false,
  });
  s.dispatch(EditorCommand::CommitComposition);
  assert_eq!(s.blocks()[0].text, "前缀中文Rust");
  assert_eq!(caret(&s), DocPosition::new(0, 8));
}

#[test]
fn ime_composition_suppresses_heading_rule_while_active() {
  let mut s = EditorSession::from_markdown("");
  s.dispatch(EditorCommand::ImeUpdate {
    text: "# ".into(),
    base: 2,
    extent: 2,
    composing_valid: true,
    composing_collapsed: false,
    shift_pressed: false,
  });
  // While composing is open, paragraph must NOT prematurely convert to heading.
  assert_eq!(s.blocks()[0].kind, "paragraph");
}

#[test]
fn ime_newline_in_code_block_preserves_leading_indentation() {
  let mut s = EditorSession::from_markdown("```rust\n    let x = 1;\n```");
  assert_eq!(s.blocks()[0].kind, "code_block");
  let len = utf16_len(&s.blocks()[0].text);
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, len)),
  });
  s.dispatch(EditorCommand::SplitBlock);
  assert_eq!(s.blocks().len(), 1);
  assert!(
    s.blocks()[0].text.ends_with("\n    "),
    "expected 4-space auto-indent, got {:?}",
    s.blocks()[0].text
  );
}

// ── Remote CRDT Selection Remapping & Undo Isolation ─────────────────────────

#[test]
fn reconcile_selection_clamps_safely_when_remote_deletes_focused_block() {
  let old_snap = vec![
    ("b0".to_string(), "Keep me".to_string()),
    ("b1".to_string(), "Deleted by peer".to_string()),
    ("b2".to_string(), "Tail".to_string()),
  ];
  let new_blocks = vec![
    EditorBlock::new("b0", "paragraph", "Keep me"),
    EditorBlock::new("b2", "paragraph", "Tail"),
  ];
  let old_sel = DocSelection::caret(1, 5);
  let reconciled = reconcile_selection(&old_snap, Some(&old_sel), &new_blocks)
    .expect("reconciled selection should remain valid");
  match reconciled.normalized() {
    NormalizedSelection::Text { start, end } => {
      assert!(start.node < new_blocks.len());
      assert!(end.node < new_blocks.len());
      assert!(end.offset <= utf16_len(&new_blocks[end.node].text));
    }
    _ => panic!("expected text selection"),
  }
}

#[test]
fn reconcile_selection_shifts_cross_block_range_when_peer_inserts_above_and_edits_prefix() {
  let old_snap = vec![
    ("b1".to_string(), "Hello world".to_string()),
    ("b2".to_string(), "Second line".to_string()),
  ];
  let new_blocks = vec![
    EditorBlock::new("b0", "paragraph", "Inserted above"),
    EditorBlock::new("b1", "paragraph", ">>> Hello world"),
    EditorBlock::new("b2", "paragraph", "Second line"),
  ];
  let old_sel = DocSelection::from_positions(DocPosition::new(0, 6), DocPosition::new(1, 6));
  let reconciled = reconcile_selection(&old_snap, Some(&old_sel), &new_blocks).unwrap();
  assert_eq!(
    reconciled,
    DocSelection::from_positions(DocPosition::new(1, 10), DocPosition::new(2, 6))
  );
}

#[test]
fn crdt_remote_edit_does_not_corrupt_local_undo_or_revert_peer_text() {
  let mut peer_a = EditorSession::from_markdown_with_client_id("Alpha\n\nBeta", Some(1));
  let initial_state = peer_a.encode_state();
  let mut peer_b = EditorSession::from_doc(
    mica_core::MicaDoc::from_update_with_client_id(&initial_state, Some(2)).expect("peer B init"),
  );

  // Peer A edits block 0.
  peer_a.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 5)),
  });
  let patch_a = peer_a.dispatch(EditorCommand::InsertText {
    text: " LocalA".into(),
  });
  peer_a.dispatch(EditorCommand::FlushHistoryBatch);

  // Peer B concurrently edits block 1.
  peer_b.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(1, 4)),
  });
  let patch_b = peer_b.dispatch(EditorCommand::InsertText {
    text: " RemoteB".into(),
  });

  // Peer A receives Peer B's remote diff.
  peer_a.dispatch(EditorCommand::ApplyRemoteUpdate {
    update: patch_b.crdt_update.unwrap(),
  });
  assert_eq!(peer_a.blocks()[0].text, "Alpha LocalA");
  assert_eq!(peer_a.blocks()[1].text, "Beta RemoteB");

  // Peer A undoes its OWN edit: Peer B's text MUST survive intact!
  peer_a.dispatch(EditorCommand::Undo);
  assert_eq!(peer_a.blocks()[0].text, "Alpha");
  assert_eq!(peer_a.blocks()[1].text, "Beta RemoteB");

  // And Peer B applying Peer A's update converges cleanly.
  peer_b.dispatch(EditorCommand::ApplyRemoteUpdate {
    update: patch_a.crdt_update.unwrap(),
  });
  assert_eq!(peer_b.blocks()[0].text, "Alpha LocalA");
  assert_eq!(peer_b.blocks()[1].text, "Beta RemoteB");
}

#[test]
fn delete_selected_blocks_on_whole_document_keeps_non_empty_paragraph() {
  let mut s = EditorSession::from_markdown("First\n\nSecond");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::from_positions(
      DocPosition::new(0, 0),
      DocPosition::new(1, 6),
    )),
  });
  s.dispatch(EditorCommand::DeleteSelectedBlocks);
  assert_eq!(s.blocks().len(), 1);
  assert_eq!(s.blocks()[0].kind, "paragraph");
  assert_eq!(s.blocks()[0].text, "");
}

#[test]
fn append_empty_paragraph_adds_trailing_editable_block_after_atomic_or_code_block() {
  let mut s = EditorSession::from_markdown("```rust\nfn main() {}\n```");
  assert_eq!(s.blocks().len(), 1);
  assert_eq!(s.blocks()[0].kind, "code_block");

  s.dispatch(EditorCommand::AppendEmptyParagraph);
  assert_eq!(s.blocks().len(), 2);
  assert_eq!(s.blocks()[1].kind, "paragraph");
  assert_eq!(s.blocks()[1].text, "");
  assert_eq!(caret(&s), DocPosition::new(1, 0));

  // Calling again when trailing block is already an empty paragraph is idempotent.
  s.dispatch(EditorCommand::AppendEmptyParagraph);
  assert_eq!(s.blocks().len(), 2);
}

#[test]
fn ai_edit_rejects_stale_snapshot_and_undoes_atomically() {
  let mut s = EditorSession::from_markdown("Intro\n\nOld body\n\nFooter");
  let snap = s.capture_ai_snapshot();
  let preview = AiEditPreview {
    edits: vec![AiBlockEdit {
      block_ids: vec![s.blocks()[1].id.clone()],
      before: "Old body".into(),
      after: "New body".into(),
      replacement: vec![
        BlockSpec::new("paragraph", "New body part 1", json!({})),
        BlockSpec::new("paragraph", "New body part 2", json!({})),
      ],
    }],
  };

  // Apply on matching snapshot succeeds and undoes in a single Undo step.
  let p = s.dispatch(EditorCommand::ApplyAiEdits {
    snapshot: snap.clone(),
    preview: preview.clone(),
  });
  assert!(p.content_changed);
  assert_eq!(s.blocks().len(), 4);
  assert_eq!(s.blocks()[1].text, "New body part 1");
  assert_eq!(s.blocks()[2].text, "New body part 2");

  s.dispatch(EditorCommand::Undo);
  assert_eq!(s.blocks().len(), 3);
  assert_eq!(s.blocks()[1].text, "Old body");

  // Mutate document so snapshot becomes stale; ApplyAiEdits must reject cleanly.
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(1, 8)),
  });
  s.dispatch(EditorCommand::InsertText {
    text: " edited".into(),
  });
  let stale_patch = s.dispatch(EditorCommand::ApplyAiEdits {
    snapshot: snap,
    preview,
  });
  assert!(!stale_patch.content_changed);
  assert_eq!(s.blocks()[1].text, "Old body edited");
}

#[test]
fn set_block_text_updates_raw_source_without_running_prose_input_rules() {
  let mut s = EditorSession::from_markdown("```mermaid\ngraph TD\n  A --> B\n```");
  let p = s.dispatch(EditorCommand::SetBlockText {
    block_index: 0,
    text: "# not a heading\n- not a list".into(),
  });
  assert!(p.content_changed);
  assert_eq!(s.blocks()[0].kind, "code_block");
  assert_eq!(s.blocks()[0].text, "# not a heading\n- not a list");
  assert_eq!(
    s.blocks()[0].data.get("language").and_then(|v| v.as_str()),
    Some("mermaid")
  );
  s.dispatch(EditorCommand::Undo);
  assert!(s.blocks()[0].text.contains("A --> B"));
}

#[test]
fn insert_divider_and_image_always_preserve_trailing_editable_paragraph() {
  let mut s = EditorSession::from_markdown("Hello");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 5)),
  });
  s.dispatch(EditorCommand::InsertDivider);
  assert!(s.blocks().iter().any(|b| b.kind == "divider"));
  assert_eq!(s.blocks().last().unwrap().kind, "paragraph");

  s.dispatch(EditorCommand::InsertImage {
    file_id: Some("img-1".into()),
    url: Some("https://cdn.example.com/a.png".into()),
    name: Some("a.png".into()),
    alt: "diagram".into(),
    align: Some("center".into()),
  });
  assert!(s.blocks().iter().any(|b| b.kind == "image"));
  assert_eq!(s.blocks().last().unwrap().kind, "paragraph");

  let exported = s.export_markdown(&BTreeMap::from([(
    "img-1".to_string(),
    "assets/a.png".to_string(),
  )]));
  assert!(exported.contains("---"));
  assert!(exported.contains("![diagram]"));
}

#[test]
fn heading_button_target_toggles_between_heading_level_and_paragraph() {
  assert_eq!(
    heading_button_target(Some("paragraph"), 0, 2),
    ("heading".to_string(), json!({ "level": 2 }))
  );
  assert_eq!(
    heading_button_target(Some("heading"), 2, 2),
    ("paragraph".to_string(), json!({}))
  );
  assert_eq!(
    heading_button_target(Some("heading"), 2, 3),
    ("heading".to_string(), json!({ "level": 3 }))
  );
}
