//! End-to-end deterministic tests and proptest invariants for `EditorSession`.

use std::collections::BTreeMap;

use mica_editor::*;
use proptest::prelude::*;
use serde_json::json;

fn caret(session: &EditorSession) -> DocPosition {
  match session.selection().expect("selection expected").normalized() {
    NormalizedSelection::Text { end, .. } => end,
    other => panic!("expected text selection, got {other:?}"),
  }
}

#[test]
fn from_markdown_and_export_roundtrip() {
  let md = "# Title\n\nHello **world** and `code`.\n\n- Item 1\n- Item 2\n";
  let session = EditorSession::from_markdown(md);
  assert!(session.blocks().len() >= 4);
  assert_eq!(session.blocks()[0].kind, "heading");
  assert_eq!(session.blocks()[0].text, "Title");
  let exported = session.export_markdown(&BTreeMap::new());
  assert!(exported.contains("# Title"));
  assert!(exported.contains("**world**"));
  assert!(exported.contains("- Item 1"));
}

#[test]
fn insert_text_split_and_backspace_merge_with_undo_redo() {
  let mut s = EditorSession::from_markdown("Hello world");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 5)),
  });
  let p = s.dispatch(EditorCommand::SplitBlock);
  assert!(p.content_changed);
  assert!(p.crdt_update.is_some());
  assert_eq!(s.blocks().len(), 2);
  assert_eq!(s.blocks()[0].text, "Hello");
  assert_eq!(s.blocks()[1].text, " world");
  assert_eq!(caret(&s), DocPosition::new(1, 0));

  let p2 = s.dispatch(EditorCommand::DeleteBackward);
  assert!(p2.content_changed);
  assert_eq!(s.blocks().len(), 1);
  assert_eq!(s.blocks()[0].text, "Hello world");
  assert_eq!(caret(&s), DocPosition::new(0, 5));

  assert!(s.can_undo());
  s.dispatch(EditorCommand::Undo);
  assert_eq!(s.blocks().len(), 2);
  assert!(s.can_redo());
  s.dispatch(EditorCommand::Redo);
  assert_eq!(s.blocks().len(), 1);
  assert_eq!(s.blocks()[0].text, "Hello world");
}

#[test]
fn ime_update_handles_cjk_composition_commit_and_input_rules() {
  let mut s = EditorSession::from_markdown("");
  let p1 = s.dispatch(EditorCommand::ImeUpdate {
    text: "## ".into(),
    base: 3,
    extent: 3,
    composing_valid: false,
    composing_collapsed: true,
    shift_pressed: false,
  });
  assert!(p1.content_changed);
  assert_eq!(s.blocks()[0].kind, "heading");
  assert_eq!(
    s.blocks()[0].data.get("level").and_then(|v| v.as_u64()),
    Some(2)
  );

  s.dispatch(EditorCommand::ImeUpdate {
    text: "nihao".into(),
    base: 5,
    extent: 5,
    composing_valid: true,
    composing_collapsed: false,
    shift_pressed: false,
  });
  s.dispatch(EditorCommand::ImeUpdate {
    text: "你好世界".into(),
    base: 4,
    extent: 4,
    composing_valid: false,
    composing_collapsed: true,
    shift_pressed: false,
  });
  s.dispatch(EditorCommand::CommitComposition);
  assert_eq!(s.blocks()[0].text, "你好世界");
  assert_eq!(caret(&s), DocPosition::new(0, 4));
}

#[test]
fn cross_block_marks_and_deletion_work_cleanly() {
  let mut s = EditorSession::from_markdown("First line\n\nSecond line");
  assert_eq!(s.blocks().len(), 2);
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::from_positions(
      DocPosition::new(0, 6),
      DocPosition::new(1, 6),
    )),
  });
  s.dispatch(EditorCommand::ToggleMark {
    kind: "bold".into(),
    href: None,
  });
  assert!(range_has_mark(&s.blocks()[0].marks(), 6, 10, "bold"));
  assert!(range_has_mark(&s.blocks()[1].marks(), 0, 6, "bold"));

  s.dispatch(EditorCommand::DeleteBackward);
  assert_eq!(s.blocks().len(), 1);
  assert_eq!(s.blocks()[0].text, "First  line");
}

#[test]
fn list_indent_outdent_and_todo_toggle() {
  let mut s = EditorSession::from_markdown("- [ ] Task A\n- [ ] Task B");
  assert_eq!(s.blocks().len(), 2);
  s.dispatch(EditorCommand::ToggleTodo { block_index: 0 });
  assert_eq!(
    s.blocks()[0].data.get("checked").and_then(|v| v.as_bool()),
    Some(true)
  );

  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(1, 0)),
  });
  s.dispatch(EditorCommand::Indent { delta: 1 });
  assert_eq!(s.blocks()[1].indent(), 1);

  s.dispatch(EditorCommand::Indent { delta: -1 });
  assert_eq!(s.blocks()[1].indent(), 0);
}

#[test]
fn transform_and_batch_transform_into_code_block() {
  let mut s = EditorSession::from_markdown("line 1\n\nline 2");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::from_positions(
      DocPosition::new(0, 0),
      DocPosition::new(1, 6),
    )),
  });
  s.dispatch(EditorCommand::BatchTransform {
    kind: "code_block".into(),
    data: Some(json!({ "language": "rust" })),
  });
  assert_eq!(s.blocks().len(), 1);
  assert_eq!(s.blocks()[0].kind, "code_block");
  assert_eq!(s.blocks()[0].text, "line 1\nline 2");
  assert_eq!(
    s.blocks()[0].data.get("language").and_then(|v| v.as_str()),
    Some("rust")
  );
}

#[test]
fn table_ops_through_editor_session_and_undo() {
  let mut s = EditorSession::from_markdown("| H1 | H2 |\n| --- | --- |\n| A | B |");
  assert_eq!(s.blocks()[0].kind, "table");
  s.dispatch(EditorCommand::TableOp {
    block_index: 0,
    op: TableOp::SetCell {
      row: 1,
      col: 0,
      cell: TableCell::new("Updated", vec![EditorMark::new(0, 7, "bold")]),
    },
  });
  s.dispatch(EditorCommand::TableOp {
    block_index: 0,
    op: TableOp::InsertRow { at: 2 },
  });
  let tbl = TableModel::from_block_data(&s.blocks()[0].data);
  assert_eq!(tbl.row_count(), 3);
  assert_eq!(tbl.rows[1][0].text, "Updated");

  s.dispatch(EditorCommand::Undo);
  let tbl_after_undo = TableModel::from_block_data(&s.blocks()[0].data);
  assert_eq!(tbl_after_undo.row_count(), 2);
}

#[test]
fn paste_markdown_copy_cut_and_set_link() {
  let mut s = EditorSession::from_markdown("Prefix ");
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 7)),
  });
  s.dispatch(EditorCommand::Paste {
    input: PasteInput::Markdown {
      markdown: "Hello **Rust**".into(),
      plain: "Hello Rust".into(),
      rich: true,
    },
  });
  assert!(s.blocks()[0].text.contains("Hello Rust"));
  let rust_start = utf16_len(
    &s.blocks()[0].text[..s.blocks()[0].text.find("Rust").expect("Rust present")],
  );
  assert!(range_has_mark(
    &s.blocks()[0].marks(),
    rust_start,
    rust_start + 4,
    "bold"
  ));

  s.dispatch(EditorCommand::SetLink {
    block_index: 0,
    start: 0,
    end: 6,
    href: Some("https://mica.dev".into()),
    text: None,
  });
  assert!(range_has_mark(&s.blocks()[0].marks(), 0, 6, "link"));

  let total_u16 = utf16_len(&s.blocks()[0].text);
  s.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::from_positions(
      DocPosition::new(0, 0),
      DocPosition::new(0, total_u16),
    )),
  });
  let copy_patch = s.dispatch(EditorCommand::CopySelection {
    image_urls: BTreeMap::new(),
  });
  let clip = copy_patch.clipboard_out.expect("clipboard payload");
  assert!(clip.markdown.contains("[Prefix](https://mica.dev)"));
  assert!(clip.markdown.contains("**Rust**"));
  assert!(clip.html.contains("<strong>"));
}

#[test]
fn slash_command_code_language_inline_math_and_move_blocks() {
  let mut s = EditorSession::from_markdown("/code");
  s.dispatch(EditorCommand::ApplySlashCommand {
    start: 0,
    end: 5,
    kind: "code_block".into(),
    data: json!({ "language": "rust" }),
  });
  assert_eq!(s.blocks()[0].kind, "code_block");
  s.dispatch(EditorCommand::SetCodeLanguage {
    block_index: 0,
    language: Some("toml".into()),
  });
  assert_eq!(
    s.blocks()[0].data.get("language").and_then(|v| v.as_str()),
    Some("toml")
  );

  let mut m = EditorSession::from_markdown("Value $x$ end");
  let math_start = m.blocks()[0]
    .marks()
    .iter()
    .find(|mk| mk.kind == "math")
    .map(|mk| (mk.start, mk.end))
    .expect("inline math mark");
  m.dispatch(EditorCommand::SetInlineMath {
    block_index: 0,
    start: math_start.0,
    end: math_start.1,
    source: "x^2+y^2".into(),
  });
  assert!(m.blocks()[0].text.contains("x^2+y^2"));

  let mut mv = EditorSession::from_markdown("One\n\nTwo\n\nThree");
  let last_id = mv.blocks()[2].id.clone();
  mv.dispatch(EditorCommand::MoveBlocks {
    block_ids: vec![last_id.clone()],
    target_index: 0,
  });
  assert_eq!(mv.blocks()[0].id, last_id);
  assert_eq!(mv.blocks()[0].text, "Three");
}

#[test]
fn find_replace_and_replace_all_in_one_undo_batch() {
  let mut s = EditorSession::from_markdown("foo bar foo\n\nanother foo");
  let find_p = s.dispatch(EditorCommand::FindReplace {
    op: FindReplaceOp::FindAll {
      query: "foo".into(),
      case_sensitive: false,
    },
  });
  assert_eq!(find_p.find_matches.as_ref().map(Vec::len), Some(3));

  let rep_p = s.dispatch(EditorCommand::FindReplace {
    op: FindReplaceOp::ReplaceAll {
      query: "foo".into(),
      replacement: "baz".into(),
      case_sensitive: false,
    },
  });
  assert_eq!(rep_p.replaced_count, Some(3));
  assert_eq!(s.blocks()[0].text, "baz bar baz");
  assert_eq!(s.blocks()[1].text, "another baz");

  s.dispatch(EditorCommand::Undo);
  assert_eq!(s.blocks()[0].text, "foo bar foo");
  assert_eq!(s.blocks()[1].text, "another foo");
}

#[test]
fn crdt_two_peer_remote_merge_preserves_local_caret_and_undo() {
  let mut peer_a = EditorSession::from_markdown_with_client_id("Alpha\n\nBeta", Some(101));
  let initial_state = peer_a.encode_state();
  let peer_b_doc =
    mica_core::MicaDoc::from_update_with_client_id(&initial_state, Some(202)).unwrap();
  let mut peer_b = EditorSession::from_doc(peer_b_doc);

  peer_a.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(0, 5)),
  });
  let p_a = peer_a.dispatch(EditorCommand::InsertText {
    text: " One".into(),
  });
  let upd_a = p_a.crdt_update.expect("peer A crdt diff");

  peer_b.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(1, 4)),
  });
  let p_b = peer_b.dispatch(EditorCommand::InsertText {
    text: " Two".into(),
  });
  let upd_b = p_b.crdt_update.expect("peer B crdt diff");

  let merged_a = peer_a.dispatch(EditorCommand::ApplyRemoteUpdate { update: upd_b });
  let merged_b = peer_b.dispatch(EditorCommand::ApplyRemoteUpdate { update: upd_a });
  assert!(merged_a.content_changed);
  assert!(merged_b.content_changed);

  assert_eq!(peer_a.blocks()[0].text, "Alpha One");
  assert_eq!(peer_a.blocks()[1].text, "Beta Two");
  assert_eq!(peer_b.blocks()[0].text, "Alpha One");
  assert_eq!(peer_b.blocks()[1].text, "Beta Two");

  peer_a.dispatch(EditorCommand::Undo);
  assert_eq!(peer_a.blocks()[0].text, "Alpha");
  assert_eq!(peer_a.blocks()[1].text, "Beta Two");
}

#[test]
fn ai_block_edits_apply_atomically_and_reject_stale_snapshot() {
  let mut s = EditorSession::from_markdown("First\n\nSecond");
  let snap = s.capture_ai_snapshot();
  let second_id = s.blocks()[1].id.clone();

  let preview = AiEditPreview {
    edits: vec![AiBlockEdit {
      block_ids: vec![second_id],
      before: "Second".into(),
      after: "Improved Second".into(),
      replacement: vec![BlockSpec {
        kind: "paragraph".into(),
        text: "Improved Second".into(),
        data: json!({}),
      }],
    }],
  };
  let p = s.dispatch(EditorCommand::ApplyAiEdits {
    snapshot: snap.clone(),
    preview: preview.clone(),
  });
  assert!(p.error.is_none());
  assert_eq!(s.blocks()[1].text, "Improved Second");

  let stale = s.dispatch(EditorCommand::ApplyAiEdits {
    snapshot: snap,
    preview,
  });
  assert!(stale.error.is_some());
}

#[test]
fn dispatch_json_and_atomic_blocks_work_over_wire_format() {
  let mut s = EditorSession::from_markdown("Intro");
  let res = s
    .dispatch_json(r#"{"cmd":"insert_divider"}"#)
    .expect("valid json command");
  let patch: EditorPatch = serde_json::from_str(&res).expect("valid patch json");
  assert!(patch.content_changed);
  assert!(s.blocks().iter().any(|b| b.kind == "divider"));

  s.dispatch(EditorCommand::InsertImage {
    file_id: Some("f1".into()),
    url: None,
    name: Some("pic.png".into()),
    alt: "Diagram".into(),
    align: Some("center".into()),
  });
  assert!(s.blocks().iter().any(|b| b.kind == "image"));
}

#[derive(Debug, Clone)]
enum FuzzOp {
  Insert(String),
  Split,
  Backspace,
  DeleteFwd,
  ToggleBold,
  Indent(i32),
  Undo,
  Redo,
}

fn fuzz_op_strategy() -> impl Strategy<Value = FuzzOp> {
  prop_oneof![
    "[a-zA-Z0-9 你好😀]{1,6}".prop_map(FuzzOp::Insert),
    Just(FuzzOp::Split),
    Just(FuzzOp::Backspace),
    Just(FuzzOp::DeleteFwd),
    Just(FuzzOp::ToggleBold),
    prop_oneof![Just(FuzzOp::Indent(1)), Just(FuzzOp::Indent(-1))],
    Just(FuzzOp::Undo),
    Just(FuzzOp::Redo),
  ]
}

proptest! {
  #![proptest_config(ProptestConfig::with_cases(48))]

  #[test]
  fn arbitrary_edit_sequences_preserve_non_empty_doc_and_valid_utf16_selection(
    ops in prop::collection::vec(fuzz_op_strategy(), 1..40)
  ) {
    let mut s = EditorSession::from_markdown("Start 开始");
    for op in ops {
      match op {
        FuzzOp::Insert(text) => { s.dispatch(EditorCommand::InsertText { text }); }
        FuzzOp::Split => { s.dispatch(EditorCommand::SplitBlock); }
        FuzzOp::Backspace => { s.dispatch(EditorCommand::DeleteBackward); }
        FuzzOp::DeleteFwd => { s.dispatch(EditorCommand::DeleteForward); }
        FuzzOp::ToggleBold => { s.dispatch(EditorCommand::ToggleMark { kind: "bold".into(), href: None }); }
        FuzzOp::Indent(delta) => { s.dispatch(EditorCommand::Indent { delta }); }
        FuzzOp::Undo => { s.dispatch(EditorCommand::Undo); }
        FuzzOp::Redo => { s.dispatch(EditorCommand::Redo); }
      }
      prop_assert!(!s.blocks().is_empty());
      if let Some(sel) = s.selection() {
        match sel.normalized() {
          NormalizedSelection::Text { start, end } => {
            prop_assert!(start.node < s.blocks().len());
            prop_assert!(end.node < s.blocks().len());
            prop_assert!(start.offset <= utf16_len(&s.blocks()[start.node].text));
            prop_assert!(end.offset <= utf16_len(&s.blocks()[end.node].text));
          }
          NormalizedSelection::BlockAtom { block_index }
          | NormalizedSelection::TableArea { block_index, .. } => {
            prop_assert!(block_index < s.blocks().len());
          }
        }
      }
      for b in s.blocks() {
        let len = utf16_len(&b.text);
        for m in b.marks() {
          prop_assert!(m.start <= m.end && m.end <= len);
        }
      }
    }
    prop_assert!(!s.encode_state().is_empty());
    let _md = s.export_markdown(&BTreeMap::new());
  }
}
