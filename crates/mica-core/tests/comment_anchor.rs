//! Comment anchors must survive concurrent edits — the whole reason they are yrs
//! sticky indexes instead of character offsets (docs/comments-plan.md).
//!
//! A stored offset is only correct against the version it was taken from: insert
//! a word ahead of it and the highlight slides silently onto the wrong text,
//! which is exactly the failure a comment feature cannot have. These pin the
//! properties the feature leans on, including what happens when the anchored text
//! is deleted (an orphan).

use mica_core::{Block, MicaDoc};

fn doc_with(text: &str) -> MicaDoc {
    MicaDoc::from_blocks(
        "r",
        &[
            Block::new("r", "page").with_children(vec!["a".into(), "b".into()]),
            Block::new("a", "paragraph").with_text(text.to_string()),
            Block::new("b", "paragraph").with_text("second block".to_string()),
        ],
    )
}

#[test]
fn an_anchor_resolves_where_it_was_taken() {
    let doc = doc_with("hello world");
    // "world" — offsets 6..11.
    let anchor = doc.sticky_for_range("a", 6, "a", 11).expect("anchor");
    let range = doc.resolve_range(&anchor).expect("resolves");
    assert_eq!(range.start_block, "a");
    assert_eq!(range.end_block, "a");
    assert_eq!((range.start_offset, range.end_offset), (6, 11));
    assert!(!range.is_empty());
}

#[test]
fn text_inserted_before_the_anchor_shifts_it() {
    // THE point of sticky indexes: a stored offset would still say 6..11 and the
    // highlight would land on "o wor".
    let mut doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).expect("anchor");
    doc.text_insert("a", 0, ">> "); // 3 UTF-16 units ahead of the anchor
    let range = doc.resolve_range(&anchor).expect("still resolves");
    assert_eq!(
        (range.start_offset, range.end_offset),
        (9, 14),
        "anchor must follow its text"
    );
}

#[test]
fn text_inserted_inside_the_anchor_extends_it() {
    let mut doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).expect("anchor");
    doc.text_insert("a", 8, "XX"); // inside "world"
    let range = doc.resolve_range(&anchor).expect("resolves");
    assert_eq!((range.start_offset, range.end_offset), (6, 13));
}

#[test]
fn typing_immediately_outside_the_anchor_is_not_swallowed() {
    // Assoc::After on the start / Assoc::Before on the end make the highlight hug
    // the selection: text typed at either edge stays OUT of the comment.
    let mut doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).expect("anchor");
    doc.text_insert("a", 11, "!!"); // just past the end
    let range = doc.resolve_range(&anchor).expect("resolves");
    assert_eq!(
        (range.start_offset, range.end_offset),
        (6, 11),
        "an insert at the end edge must not extend the comment"
    );
}

#[test]
fn deleting_the_anchored_text_orphans_the_thread() {
    let mut doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "a", 11).expect("anchor");
    doc.text_delete("a", 6, 5); // delete "world"
    // Either outcome is an orphan, and the caller must treat them alike: yrs
    // keeps tombstones, so the indexes may still resolve with the range collapsed
    // to zero length rather than failing outright.
    match doc.resolve_range(&anchor) {
        None => {}
        Some(range) => assert!(
            range.is_empty(),
            "a deleted range must collapse, got {range:?}"
        ),
    }
}

#[test]
fn an_anchor_can_span_two_blocks() {
    let doc = doc_with("hello world");
    let anchor = doc.sticky_for_range("a", 6, "b", 6).expect("anchor");
    let range = doc.resolve_range(&anchor).expect("resolves");
    assert_eq!(range.start_block, "a");
    assert_eq!(range.end_block, "b");
    assert_eq!((range.start_offset, range.end_offset), (6, 6));
    // Different blocks → not empty even though the offsets match.
    assert!(!range.is_empty());
}

#[test]
fn an_unknown_block_or_out_of_range_offset_yields_no_anchor() {
    let doc = doc_with("hello world");
    assert!(doc.sticky_for_range("nope", 0, "a", 1).is_none());
    assert!(doc.sticky_for_range("a", 0, "nope", 1).is_none());
    // 11 is the end of "hello world" (valid); 12 is past it.
    assert!(doc.sticky_for_range("a", 0, "a", 11).is_some());
    assert!(doc.sticky_for_range("a", 0, "a", 12).is_none());
    assert!(doc.sticky_for_range("a", 99, "a", 99).is_none());
}

#[test]
fn corrupt_anchor_bytes_resolve_to_none_rather_than_panicking() {
    // The bytes come back from Postgres; a truncated/garbage row must not take the
    // request down (and must not be guessed at either).
    let doc = doc_with("hello world");
    let mut anchor = doc.sticky_for_range("a", 6, "a", 11).expect("anchor");
    anchor.start_sticky = vec![0xff, 0x00, 0x13, 0x37];
    assert!(doc.resolve_range(&anchor).is_none());
    anchor.start_sticky.clear();
    assert!(doc.resolve_range(&anchor).is_none());
}
