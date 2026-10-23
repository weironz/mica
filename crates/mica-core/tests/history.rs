//! Personal editor history over real yrs replicas. Each sync exchanges encoded
//! diffs, including undo/redo's delete sets; no document snapshot is replayed.

use mica_core::{marks_from_data, Block, Mark, MicaDoc};
use serde_json::json;

fn paragraph(id: &str, text: &str) -> Block {
  Block::new(id, "paragraph").with_text(text)
}

fn base(text: &str) -> Vec<u8> {
  MicaDoc::from_blocks_with_client_id(
    "r",
    &[
      Block::new("r", "page").with_children(vec!["a".into()]),
      paragraph("a", text),
    ],
    Some(1),
  )
  .encode_state()
}

fn replicas(text: &str) -> (MicaDoc, MicaDoc) {
  let state = base(text);
  (
    MicaDoc::from_update_with_client_id(&state, Some(10)).unwrap(),
    MicaDoc::from_update_with_client_id(&state, Some(20)).unwrap(),
  )
}

fn block(doc: &MicaDoc, id: &str) -> Block {
  doc
    .to_blocks()
    .into_iter()
    .find(|block| block.id == id)
    .expect("block exists")
}

fn batch(doc: &mut MicaDoc, edit: impl FnOnce(&mut MicaDoc)) {
  doc.begin_history_batch();
  edit(doc);
  doc.end_history_batch();
}

fn sync(a: &mut MicaDoc, b: &mut MicaDoc) {
  let from_a = a.encode_diff(&b.state_vector()).unwrap();
  let from_b = b.encode_diff(&a.state_vector()).unwrap();
  a.apply_update(&from_b).unwrap();
  b.apply_update(&from_a).unwrap();
  assert_eq!(a.to_blocks(), b.to_blocks(), "encoded diffs converge");
  // Network replay must also be harmless for undo/redo updates.
  a.apply_update(&from_b).unwrap();
  b.apply_update(&from_a).unwrap();
  assert_eq!(a.to_blocks(), b.to_blocks(), "diff replay is idempotent");
}

fn mark(start: u32, end: u32, ty: &str) -> Mark {
  Mark {
    start,
    end,
    ty: ty.into(),
    href: None,
    title: None,
  }
}

#[test]
fn identical_coarse_update_retries_do_not_add_history() {
  let (mut a, mut b) = replicas("before");
  let before = a.to_blocks();
  let data = json!({
    "level": 2,
    "marks": [{"start": 0, "end": 7, "type": "bold"}],
  });
  let marks = marks_from_data(&data);
  batch(&mut a, |doc| {
    doc.update_block_kind("a", "heading");
    doc.set_block_text("a", "changed", &marks);
    doc.set_block_data("a", &data);
  });
  let after = a.to_blocks();
  let encoded = a.encode_state();
  assert_eq!(a.history_depths(), (1, 0));

  // The engine changed before an outbox failure; replaying the same dirty
  // coarse update after persistence recovers must remain a genuine no-op.
  batch(&mut a, |doc| {
    doc.update_block_kind("a", "heading");
    doc.set_block_text("a", "changed", &marks);
    doc.set_block_data("a", &data);
  });
  assert_eq!(a.history_depths(), (1, 0));
  assert_eq!(a.encode_state(), encoded);
  batch(&mut a, |doc| {
    doc.update_block_kind("a", "heading");
    doc.set_block_marks("a", &marks);
    doc.set_block_data("a", &data);
  });
  assert_eq!(a.history_depths(), (1, 0));
  assert_eq!(
    a.encode_state(),
    encoded,
    "same data-only marks are also a no-op"
  );
  sync(&mut a, &mut b);
  assert!(a.undo());
  assert_eq!(
    a.to_blocks(),
    before,
    "the first undo reverts the actual edit"
  );
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(a.to_blocks(), after);
  sync(&mut a, &mut b);
}

#[test]
fn ordinary_writes_and_reloaded_state_have_no_personal_history() {
  fn assert_send<T: Send>() {}
  assert_send::<MicaDoc>();

  let (mut a, _) = replicas("seed");
  a.reset_history();
  a.text_insert("a", 4, " initialized");
  assert!(!a.can_undo());
  assert!(!a.undo());
  assert!(!a.redo());

  batch(&mut a, |doc| doc.text_insert("a", 16, "!"));
  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "seed initialized");
  assert!(a.can_redo());
  a.reset_history();
  assert!(!a.can_undo());
  assert!(!a.can_redo());

  let loaded = MicaDoc::from_update(&a.encode_state()).unwrap();
  assert!(!loaded.can_undo());
  assert!(!loaded.can_redo());
  assert_eq!(loaded.to_blocks(), a.to_blocks());
}

#[test]
fn remote_and_maintenance_writes_remain_untracked_inside_an_open_batch() {
  let (mut a, mut b) = replicas("ab");
  a.begin_history_batch();
  a.text_insert("a", 1, "L");
  b.text_insert("a", 2, "R");
  sync(&mut a, &mut b);
  a.set_block_prop("r", "title", &json!("renamed"));
  a.text_insert("a", 0, "[");
  a.end_history_batch();
  assert_eq!(block(&a, "a").text, "[aLbR");

  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "abR");
  assert_eq!(block(&a, "r").data["title"], "renamed");
  assert!(!a.can_undo(), "neither remote nor title is another step");
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(block(&a, "a").text, "[aLbR");
  sync(&mut a, &mut b);
}

#[test]
fn bulk_load_inside_a_batch_does_not_become_an_editor_edit() {
  let (mut a, _) = replicas("old");
  a.begin_history_batch();
  let mut blocks = a.to_blocks();
  blocks
    .iter_mut()
    .find(|block| block.id == "a")
    .unwrap()
    .text = "loaded".into();
  a.set_blocks("r", &blocks);
  a.end_history_batch();
  assert_eq!(block(&a, "a").text, "loaded");
  assert!(!a.can_undo());
}

#[test]
fn coarse_text_edits_undo_only_local_characters_after_remote_interleaving() {
  let (mut a, mut b) = replicas("abcd");
  batch(&mut a, |doc| doc.set_block_text("a", "aLbcd", &[]));
  batch(&mut b, |doc| doc.set_block_text("a", "abcRd", &[]));
  sync(&mut a, &mut b);
  assert_eq!(block(&a, "a").text, "aLbcRd");

  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "abcRd");
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(block(&a, "a").text, "aLbcRd");
  sync(&mut a, &mut b);
}

#[test]
fn repeated_characters_keep_remote_item_identity_and_its_mark() {
  let (mut a, mut b) = replicas("aaaa");
  batch(&mut a, |doc| doc.text_insert("a", 1, "a"));
  batch(&mut b, |doc| {
    doc.text_insert("a", 2, "a");
    doc.text_format("a", &mark(2, 3, "italic"));
  });
  sync(&mut a, &mut b);
  assert_eq!(block(&a, "a").text, "aaaaaa");
  assert_eq!(
    marks_from_data(&block(&a, "a").data),
    vec![mark(3, 4, "italic")]
  );

  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "aaaaa");
  assert_eq!(
    marks_from_data(&block(&a, "a").data),
    vec![mark(2, 3, "italic")]
  );
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(
    marks_from_data(&block(&a, "a").data),
    vec![mark(3, 4, "italic")]
  );
  sync(&mut a, &mut b);
}

#[test]
fn utf16_coarse_edits_and_undo_preserve_surrogate_pairs() {
  let (mut a, mut b) = replicas("甲🙂乙");
  batch(&mut a, |doc| doc.set_block_text("a", "甲🙂本乙", &[]));
  batch(&mut b, |doc| doc.set_block_text("a", "甲🙂乙🚀", &[]));
  sync(&mut a, &mut b);
  assert_eq!(block(&a, "a").text, "甲🙂本乙🚀");

  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "甲🙂乙🚀");
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(block(&a, "a").text, "甲🙂本乙🚀");
  sync(&mut a, &mut b);
}

#[test]
fn redo_does_not_resurrect_local_characters_deleted_by_a_remote_peer() {
  let (mut a, mut b) = replicas("x");
  batch(&mut a, |doc| doc.text_insert("a", 1, "abc"));
  sync(&mut a, &mut b);
  batch(&mut b, |doc| doc.text_delete("a", 1, 1));
  sync(&mut a, &mut b);
  assert_eq!(block(&a, "a").text, "xbc");

  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "x");
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(block(&a, "a").text, "xbc");
  sync(&mut a, &mut b);
}

#[test]
fn remote_edit_after_undo_preserves_redo_and_its_new_text() {
  let (mut a, mut b) = replicas("ab");
  batch(&mut a, |doc| doc.text_insert("a", 1, "L"));
  sync(&mut a, &mut b);
  assert!(a.undo());
  sync(&mut a, &mut b);
  batch(&mut b, |doc| doc.text_insert("a", 2, "R"));
  sync(&mut a, &mut b);
  assert!(a.can_redo(), "remote updates must not clear local redo");

  assert!(a.redo());
  assert_eq!(block(&a, "a").text, "aLbR");
  sync(&mut a, &mut b);
}

#[test]
fn a_new_local_batch_after_undo_clears_the_redo_branch() {
  let (mut a, mut b) = replicas("ab");
  batch(&mut a, |doc| doc.text_insert("a", 1, "L"));
  assert!(a.undo());
  batch(&mut a, |doc| doc.text_insert("a", 2, "N"));
  assert!(!a.can_redo());
  assert!(!a.redo());
  assert_eq!(block(&a, "a").text, "abN");
  sync(&mut a, &mut b);
}

#[test]
fn remote_property_and_kind_overrides_survive_undo_and_redo() {
  let (mut a, mut b) = replicas("text");
  a.set_block_data("a", &json!({"level": 1, "checked": false}));
  sync(&mut a, &mut b);
  batch(&mut a, |doc| {
    doc.update_block_kind("a", "heading");
    doc.set_block_data("a", &json!({"level": 2, "checked": true}));
  });
  sync(&mut a, &mut b);
  batch(&mut b, |doc| {
    doc.update_block_kind("a", "code");
    doc.set_block_data(
      "a",
      &json!({"level": 2, "checked": false, "language": "rust"}),
    );
  });
  sync(&mut a, &mut b);

  assert!(a.undo());
  let undone = block(&a, "a");
  assert_eq!(undone.kind, "code");
  assert_eq!(
    undone.data,
    json!({"level": 1, "checked": false, "language": "rust"})
  );
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(block(&a, "a").kind, "code");
  assert_eq!(
    block(&a, "a").data,
    json!({"level": 2, "checked": false, "language": "rust"})
  );
  sync(&mut a, &mut b);
}

#[test]
fn formatting_undo_keeps_remote_formatting() {
  let (mut a, mut b) = replicas("abcd");
  batch(&mut a, |doc| doc.text_format("a", &mark(0, 2, "bold")));
  batch(&mut b, |doc| doc.text_format("a", &mark(2, 4, "italic")));
  sync(&mut a, &mut b);
  assert!(a.undo());
  assert_eq!(
    marks_from_data(&block(&a, "a").data),
    vec![mark(2, 4, "italic")]
  );
  sync(&mut a, &mut b);
  assert!(a.redo());
  let marks = marks_from_data(&block(&a, "a").data);
  assert!(marks.contains(&mark(0, 2, "bold")));
  assert!(marks.contains(&mark(2, 4, "italic")));
  sync(&mut a, &mut b);
}

#[test]
fn split_insert_move_delete_and_props_are_one_atomic_history_step() {
  let state = MicaDoc::from_blocks_with_client_id(
    "r",
    &[
      Block::new("r", "page").with_children(vec!["a".into(), "z".into(), "remote".into()]),
      paragraph("a", "abcd"),
      paragraph("z", "deleted"),
      paragraph("remote", "remote"),
    ],
    Some(1),
  )
  .encode_state();
  let mut a = MicaDoc::from_update_with_client_id(&state, Some(10)).unwrap();
  let mut b = MicaDoc::from_update_with_client_id(&state, Some(20)).unwrap();
  let mut before = a.to_blocks();
  before
    .iter_mut()
    .find(|block| block.id == "remote")
    .unwrap()
    .text = "remote!".into();
  batch(&mut a, |doc| {
    doc.split_block("a", 2, "tail", "paragraph");
    doc.insert_block("r", 2, &paragraph("new", "inserted"));
    doc.move_block("new", "tail", 0);
    doc.update_block_kind("tail", "heading");
    doc.set_block_data("tail", &json!({"level": 2}));
    doc.delete_block("z", false);
  });
  batch(&mut b, |doc| doc.text_insert("remote", 6, "!"));
  sync(&mut a, &mut b);
  let after = a.to_blocks();
  assert_eq!(block(&a, "r").children, vec!["a", "tail", "remote"]);
  assert_eq!(block(&a, "tail").children, vec!["new"]);

  assert!(a.undo());
  assert_eq!(a.to_blocks(), before);
  assert!(
    !a.can_undo(),
    "all internal transactions form one history step"
  );
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(a.to_blocks(), after);
  sync(&mut a, &mut b);
}

fn move_replicas(nested: bool) -> (MicaDoc, MicaDoc) {
  let mut blocks = vec![
    Block::new("r", "page").with_children(if nested {
      vec!["left".into(), "right".into()]
    } else {
      vec!["a".into(), "b".into(), "c".into()]
    }),
    paragraph("a", "a"),
    paragraph("b", "b"),
    paragraph("c", "c"),
  ];
  if nested {
    blocks.extend([
      paragraph("left", "left").with_children(vec!["a".into(), "b".into(), "c".into()]),
      paragraph("right", "right"),
    ]);
  }
  let state = MicaDoc::from_blocks_with_client_id("r", &blocks, Some(1)).encode_state();
  (
    MicaDoc::from_update_with_client_id(&state, Some(10)).unwrap(),
    MicaDoc::from_update_with_client_id(&state, Some(20)).unwrap(),
  )
}

#[test]
fn undo_and_redo_preserve_a_remote_occurrence_moved_in_the_same_parent() {
  let (mut a, mut b) = move_replicas(false);
  batch(&mut a, |doc| doc.move_block("a", "r", 2));
  sync(&mut a, &mut b);
  batch(&mut b, |doc| doc.move_block("a", "r", 1));
  sync(&mut a, &mut b);
  assert_eq!(block(&a, "r").children, vec!["b", "a", "c"]);

  assert!(a.undo());
  assert_eq!(block(&a, "r").children, vec!["b", "a", "c"]);
  assert_eq!(a.history_depths(), (0, 1), "repair adds no history item");
  sync(&mut a, &mut b);
  a.redo();
  assert_eq!(block(&a, "r").children, vec!["b", "a", "c"]);
  sync(&mut a, &mut b);
}

#[test]
fn undo_keeps_a_remote_cross_parent_move_but_reverts_text_in_the_same_batch() {
  let (mut a, mut b) = move_replicas(true);
  batch(&mut a, |doc| {
    doc.move_block("a", "left", 2);
    doc.text_insert("a", 1, " local");
  });
  sync(&mut a, &mut b);
  batch(&mut b, |doc| {
    doc.move_block("a", "right", 0);
    doc.text_insert("a", 7, " remote");
  });
  sync(&mut a, &mut b);

  assert!(a.undo());
  assert_eq!(block(&a, "left").children, vec!["b", "c"]);
  assert_eq!(block(&a, "right").children, vec!["a"]);
  assert_eq!(block(&a, "a").text, "a remote");
  assert_eq!(a.history_depths(), (0, 1), "repair adds no history item");
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(block(&a, "left").children, vec!["b", "c"]);
  assert_eq!(block(&a, "right").children, vec!["a"]);
  assert_eq!(block(&a, "a").text, "a local remote");
  assert_eq!(a.history_depths(), (1, 0));
  sync(&mut a, &mut b);
}

#[test]
fn undo_keeps_the_remote_position_after_a_cross_parent_move_back() {
  let (mut a, mut b) = move_replicas(true);
  batch(&mut a, |doc| doc.move_block("a", "right", 0));
  sync(&mut a, &mut b);
  batch(&mut b, |doc| doc.move_block("a", "left", 1));
  sync(&mut a, &mut b);

  assert!(a.undo());
  assert_eq!(block(&a, "left").children, vec!["b", "a", "c"]);
  assert!(block(&a, "right").children.is_empty());
  sync(&mut a, &mut b);
  a.redo();
  assert_eq!(block(&a, "left").children, vec!["b", "a", "c"]);
  assert!(block(&a, "right").children.is_empty());
  sync(&mut a, &mut b);
}

#[test]
fn redo_preserves_a_remote_occurrence_created_after_undo() {
  let (mut a, mut b) = move_replicas(false);
  batch(&mut a, |doc| doc.move_block("a", "r", 2));
  sync(&mut a, &mut b);
  assert!(a.undo());
  assert_eq!(block(&a, "r").children, vec!["a", "b", "c"]);
  sync(&mut a, &mut b);
  batch(&mut b, |doc| doc.move_block("a", "r", 1));
  sync(&mut a, &mut b);

  assert!(a.redo());
  assert_eq!(block(&a, "r").children, vec!["b", "a", "c"]);
  sync(&mut a, &mut b);
}

#[test]
fn merge_restores_text_marks_and_children_on_undo() {
  let state = MicaDoc::from_blocks_with_client_id(
    "r",
    &[
      Block::new("r", "page").with_children(vec!["a".into(), "b".into()]),
      paragraph("a", "one"),
      paragraph("b", "🙂two")
        .with_children(vec!["child".into()])
        .with_data(json!({"marks": [{"start": 0, "end": 2, "type": "bold"}]})),
      paragraph("child", "child"),
    ],
    Some(1),
  )
  .encode_state();
  let mut a = MicaDoc::from_update_with_client_id(&state, Some(10)).unwrap();
  let mut b = MicaDoc::from_update_with_client_id(&state, Some(20)).unwrap();
  let before = a.to_blocks();
  batch(&mut a, |doc| doc.join_into_prev("b"));
  assert_eq!(block(&a, "a").text, "one🙂two");
  assert_eq!(block(&a, "a").children, vec!["child"]);
  assert_eq!(
    marks_from_data(&block(&a, "a").data),
    vec![mark(3, 5, "bold")]
  );
  let after = a.to_blocks();
  sync(&mut a, &mut b);
  assert!(a.undo());
  assert_eq!(a.to_blocks(), before);
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(a.to_blocks(), after);
  sync(&mut a, &mut b);
}

#[test]
fn remote_deletion_of_a_locally_inserted_block_cannot_be_resurrected() {
  let (mut a, mut b) = replicas("base");
  batch(&mut a, |doc| {
    doc.insert_block("r", 1, &paragraph("new", "local"))
  });
  sync(&mut a, &mut b);
  batch(&mut b, |doc| doc.delete_block("new", false));
  sync(&mut a, &mut b);
  assert!(
    !a.undo(),
    "the entire local insertion was already deleted remotely"
  );
  assert!(!a.can_undo());
  assert!(!a.redo());
  assert!(!a.to_blocks().iter().any(|block| block.id == "new"));
  sync(&mut a, &mut b);
}

#[test]
fn a_slow_coarse_operation_stays_in_one_explicit_batch() {
  let (mut a, _) = replicas("before");
  let before = a.to_blocks();
  a.begin_history_batch();
  a.update_block_kind("a", "heading");
  std::thread::sleep(std::time::Duration::from_millis(550));
  a.set_block_text("a", "after", &[]);
  a.set_block_data("a", &json!({"level": 2}));
  a.end_history_batch();
  assert!(a.undo());
  assert_eq!(a.to_blocks(), before);
  assert!(
    !a.can_undo(),
    "the default 500ms timeout must not split a batch"
  );
}

#[test]
fn nested_batches_share_a_group_but_separate_batches_do_not_merge() {
  let (mut a, _) = replicas("");
  a.end_history_batch();
  a.begin_history_batch();
  a.text_insert("a", 0, "a");
  a.begin_history_batch();
  a.text_insert("a", 1, "b");
  a.end_history_batch();
  assert!(!a.undo(), "a batch must close before undo");
  a.text_insert("a", 2, "c");
  a.end_history_batch();
  batch(&mut a, |doc| doc.text_insert("a", 3, "d"));
  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "abc");
  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "");
  assert!(!a.can_undo());
  assert!(a.redo());
  assert_eq!(block(&a, "a").text, "abc");
  assert!(a.redo());
  assert_eq!(block(&a, "a").text, "abcd");
}

#[test]
fn undo_depths_account_for_skipped_remote_overwritten_batches() {
  let (mut a, mut b) = replicas("a");
  assert_eq!(a.history_depths(), (0, 0));
  batch(&mut a, |doc| doc.text_insert("a", 0, "1"));
  batch(&mut a, |doc| doc.text_insert("a", 0, "2"));
  batch(&mut a, |doc| doc.update_block_kind("a", "heading"));
  assert_eq!(a.history_depths(), (3, 0));
  sync(&mut a, &mut b);
  batch(&mut b, |doc| doc.update_block_kind("a", "code"));
  sync(&mut a, &mut b);

  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "1a");
  assert_eq!(block(&a, "a").kind, "code");
  assert_eq!(a.history_depths(), (1, 1), "one undo popped two items");
  assert!(
    a.can_undo(),
    "a boolean alone cannot describe the skipped item"
  );
  sync(&mut a, &mut b);
  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "a");
  assert_eq!(a.history_depths(), (0, 2));
  sync(&mut a, &mut b);
  assert!(a.redo());
  assert_eq!(a.history_depths(), (1, 1));
  assert!(a.redo());
  assert_eq!(block(&a, "a").text, "21a");
  assert_eq!(block(&a, "a").kind, "code");
  assert_eq!(a.history_depths(), (2, 0));
  sync(&mut a, &mut b);
  a.reset_history();
  assert_eq!(a.history_depths(), (0, 0));
}

#[test]
fn reset_clears_both_stacks_and_closes_the_previous_session_batch() {
  let (mut a, _) = replicas("");
  batch(&mut a, |doc| doc.text_insert("a", 0, "a"));
  batch(&mut a, |doc| doc.text_insert("a", 1, "b"));
  assert!(a.undo());
  assert!(a.can_undo() && a.can_redo());
  a.begin_history_batch();
  a.reset_history();
  a.text_insert("a", 1, "untracked");
  a.end_history_batch();
  assert!(!a.can_undo());
  assert!(!a.can_redo());
  batch(&mut a, |doc| doc.text_insert("a", 10, "!"));
  assert!(a.undo());
  assert_eq!(block(&a, "a").text, "auntracked");
}
