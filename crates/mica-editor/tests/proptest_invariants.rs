//! High-signal property-based tests (`proptest`) enforcing structural,
//! UTF-16, mark-algebra, undo/redo, and two-peer CRDT convergence invariants
//! in `mica-editor`.

use std::collections::BTreeMap;

use mica_editor::*;
use proptest::prelude::*;

fn unicode_text_strategy() -> impl Strategy<Value = String> {
  prop::collection::vec(
    prop_oneof![
      Just('a'),
      Just('z'),
      Just(' '),
      Just('你'),
      Just('好'),
      Just('😀'),
      Just('🚀'),
      Just('\u{0301}'),
    ],
    0..24,
  )
  .prop_map(|chars| chars.into_iter().collect())
}

#[derive(Debug, Clone)]
enum FuzzOp {
  Insert(String),
  Split,
  Backspace,
  ForwardDelete,
  ToggleBold,
  Indent(i32),
  MoveCaret(usize, usize),
  Undo,
  Redo,
}

fn fuzz_op_strategy() -> impl Strategy<Value = FuzzOp> {
  prop_oneof![
    3 => prop_oneof![Just("a"), Just("你"), Just("😀"), Just(" "), Just("Rust")]
      .prop_map(|s| FuzzOp::Insert(s.to_string())),
    1 => Just(FuzzOp::Split),
    2 => Just(FuzzOp::Backspace),
    1 => Just(FuzzOp::ForwardDelete),
    1 => Just(FuzzOp::ToggleBold),
    1 => prop_oneof![Just(1), Just(-1)].prop_map(FuzzOp::Indent),
    2 => (0usize..8, 0usize..32).prop_map(|(b, o)| FuzzOp::MoveCaret(b, o)),
    1 => Just(FuzzOp::Undo),
    1 => Just(FuzzOp::Redo),
  ]
}

fn assert_session_invariants(s: &EditorSession) {
  assert!(!s.blocks().is_empty(), "document must never become empty");
  for blk in s.blocks() {
    let len = utf16_len(&blk.text);
    let marks = blk.marks();
    for mk in &marks {
      assert!(
        mk.start < mk.end && mk.end <= len,
        "invalid mark {mk:?} for text len {len} ({:?})",
        blk.text
      );
      assert_eq!(
        snap_utf16_boundary(&blk.text, mk.start, false),
        mk.start,
        "mark start split surrogate pair"
      );
      assert_eq!(
        snap_utf16_boundary(&blk.text, mk.end, false),
        mk.end,
        "mark end split surrogate pair"
      );
    }
  }
  if let Some(sel) = s.selection() {
    match sel.normalized() {
      NormalizedSelection::Text { start, end } => {
        assert!(start.node < s.blocks().len());
        assert!(end.node < s.blocks().len());
        let s_text = &s.blocks()[start.node].text;
        let e_text = &s.blocks()[end.node].text;
        assert!(start.offset <= utf16_len(s_text));
        assert!(end.offset <= utf16_len(e_text));
        assert_eq!(
          snap_utf16_boundary(s_text, start.offset, false),
          start.offset
        );
        assert_eq!(snap_utf16_boundary(e_text, end.offset, false), end.offset);
      }
      NormalizedSelection::BlockAtom { block_index }
      | NormalizedSelection::TableArea { block_index, .. } => {
        assert!(block_index < s.blocks().len());
      }
    }
  }
}

fn apply_fuzz_op(s: &mut EditorSession, op: FuzzOp) -> Option<Vec<u8>> {
  let patch = match op {
    FuzzOp::Insert(text) => s.dispatch(EditorCommand::InsertText { text }),
    FuzzOp::Split => s.dispatch(EditorCommand::SplitBlock),
    FuzzOp::Backspace => s.dispatch(EditorCommand::DeleteBackward),
    FuzzOp::ForwardDelete => s.dispatch(EditorCommand::DeleteForward),
    FuzzOp::ToggleBold => s.dispatch(EditorCommand::ToggleMark {
      kind: "bold".into(),
      href: None,
    }),
    FuzzOp::Indent(delta) => s.dispatch(EditorCommand::Indent { delta }),
    FuzzOp::MoveCaret(bi_raw, off_raw) => {
      let bi = bi_raw % s.blocks().len();
      let text = &s.blocks()[bi].text;
      let len = utf16_len(text);
      let off = if len == 0 {
        0
      } else {
        snap_utf16_boundary(text, (off_raw as u32) % (len + 1), false)
      };
      s.dispatch(EditorCommand::SetSelection {
        selection: Some(DocSelection::caret(bi, off)),
      })
    }
    FuzzOp::Undo => s.dispatch(EditorCommand::Undo),
    FuzzOp::Redo => s.dispatch(EditorCommand::Redo),
  };
  patch.crdt_update
}

proptest! {
  #![proptest_config(ProptestConfig::with_cases(96))]

  #[test]
  fn utf16_slice_and_split_never_panic_on_arbitrary_unicode(
    s in unicode_text_strategy(),
    raw_a in 0u32..40,
    raw_b in 0u32..40,
  ) {
    let len = utf16_len(&s);
    let a = snap_utf16_boundary(&s, raw_a.min(len), false);
    let b = snap_utf16_boundary(&s, raw_b.min(len), true);
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };

    let sliced = utf16_slice(&s, lo, hi);
    let sliced_len = utf16_len(sliced);
    let (l, r) = utf16_split_at(&s, lo);
    prop_assert_eq!(format!("{l}{r}"), s);
    prop_assert!(sliced_len <= len);
  }

  #[test]
  fn mark_split_and_concat_are_lossless_and_normalized(
    raw_ranges in prop::collection::vec((0u32..30, 0u32..30, prop_oneof![Just("bold"), Just("italic"), Just("code")]), 0..12),
    split_at in 0u32..30,
  ) {
    let mut marks = Vec::new();
    for (a, b, kind) in raw_ranges {
      let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
      if lo < hi {
        marks = apply_mark(&marks, lo, hi, kind, None, None);
      }
    }
    let normalized = normalize_marks(marks);
    for w in normalized.windows(2) {
      if w[0].kind == w[1].kind {
        prop_assert!(w[0].end < w[1].start, "same-kind marks must be merged/non-overlapping: {normalized:?}");
      }
    }
    let (left, right) = split_marks(&normalized, split_at);
    let rejoined = concat_marks(&left, &right, split_at);
    prop_assert!(marks_equal(&rejoined, &normalized));
  }

  #[test]
  fn random_session_commands_preserve_all_document_and_selection_invariants(
    ops in prop::collection::vec(fuzz_op_strategy(), 1..40),
  ) {
    let mut s = EditorSession::from_markdown("初始 Hello 😀\n\n- 列表项");
    for op in ops {
      let _ = apply_fuzz_op(&mut s, op);
      assert_session_invariants(&s);
    }
    // Markdown export must always succeed without panicking.
    let _md = s.export_markdown(&BTreeMap::new());
  }

  #[test]
  fn two_peer_concurrent_edits_converge_and_keep_valid_selections(
    ops_a in prop::collection::vec(
      prop_oneof![
        Just(FuzzOp::Insert("A".into())),
        Just(FuzzOp::Insert("你".into())),
        Just(FuzzOp::Backspace),
        Just(FuzzOp::ToggleBold),
        (0usize..4, 0usize..20).prop_map(|(b, o)| FuzzOp::MoveCaret(b, o)),
      ],
      1..15,
    ),
    ops_b in prop::collection::vec(
      prop_oneof![
        Just(FuzzOp::Insert("B".into())),
        Just(FuzzOp::Insert("好".into())),
        Just(FuzzOp::Backspace),
        Just(FuzzOp::ToggleBold),
        (0usize..4, 0usize..20).prop_map(|(b, o)| FuzzOp::MoveCaret(b, o)),
      ],
      1..15,
    ),
  ) {
    let mut peer_a = EditorSession::from_markdown_with_client_id("Hello world\n\nSecond line", Some(11));
    let init = peer_a.encode_state();
    let mut peer_b = EditorSession::from_doc(
      mica_core::MicaDoc::from_update_with_client_id(&init, Some(22)).expect("peer B init"),
    );

    let mut updates_a = Vec::new();
    for op in ops_a {
      if let Some(u) = apply_fuzz_op(&mut peer_a, op) {
        updates_a.push(u);
      }
      assert_session_invariants(&peer_a);
    }
    let mut updates_b = Vec::new();
    for op in ops_b {
      if let Some(u) = apply_fuzz_op(&mut peer_b, op) {
        updates_b.push(u);
      }
      assert_session_invariants(&peer_b);
    }

    // Exchange all diffs in both directions.
    for u in &updates_b {
      peer_a.dispatch(EditorCommand::ApplyRemoteUpdate { update: u.clone() });
      assert_session_invariants(&peer_a);
    }
    for u in &updates_a {
      peer_b.dispatch(EditorCommand::ApplyRemoteUpdate { update: u.clone() });
      assert_session_invariants(&peer_b);
    }

    let texts_a: Vec<String> = peer_a.blocks().iter().map(|b| b.text.clone()).collect();
    let texts_b: Vec<String> = peer_b.blocks().iter().map(|b| b.text.clone()).collect();
    prop_assert_eq!(texts_a, texts_b, "two CRDT peers must converge to identical block texts");
  }
}
