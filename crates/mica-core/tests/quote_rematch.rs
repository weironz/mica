//! Re-anchoring an orphaned comment thread by its saved quote
//! (docs/comments-plan.md, Phase 2 ①).
//!
//! Two things are pinned here, and the second matters more than the first:
//!
//! 1. a thread whose anchor died over UNCHANGED text comes back — the case that
//!    actually happens, because `set_blocks` (every REST/MCP write, every version
//!    restore) rewrites each block's text object and kills every anchor on the
//!    document;
//! 2. the matcher REFUSES when it is not sure. A wrong anchor moves a discussion
//!    onto words nobody commented on, silently; an orphan is at least visibly an
//!    orphan. Every "stays None" test below is guarding that.

use mica_core::{Block, MicaDoc, QuoteIndex};

fn doc_with(blocks: &[(&str, &str)]) -> MicaDoc {
    let mut all = vec![
        Block::new("r", "page")
            .with_children(blocks.iter().map(|(id, _)| id.to_string()).collect()),
    ];
    for (id, text) in blocks {
        all.push(Block::new(*id, "paragraph").with_text(text.to_string()));
    }
    MicaDoc::from_blocks("r", &all)
}

fn index(doc: &MicaDoc) -> QuoteIndex {
    QuoteIndex::from_blocks(&doc.to_blocks())
}

#[test]
fn a_rewritten_document_kills_anchors_and_the_quote_brings_them_back() {
    // The motivating case end to end. `set_blocks` is what a REST/MCP write and a
    // version restore both do; the text is IDENTICAL afterwards.
    let mut doc = doc_with(&[("a", "the quick brown fox"), ("b", "second block")]);
    let anchor = doc.sticky_for_range("a", 4, "a", 9).expect("anchor 'quick'");
    assert_eq!(
        doc.resolve_range(&anchor).map(|r| (r.start_offset, r.end_offset)),
        Some((4, 9))
    );

    let blocks = doc.to_blocks();
    doc.set_blocks("r", &blocks);

    let dead = doc.resolve_range(&anchor).filter(|r| !r.is_empty());
    assert!(
        dead.is_none(),
        "set_blocks replaces every block's text object — the old anchor cannot survive"
    );

    let found = index(&doc).find("quick", Some("a")).expect("the quote is still there");
    assert_eq!(found.start_block, "a");
    assert_eq!((found.start_offset, found.end_offset), (4, 9));

    // And the recovered range yields a working anchor again.
    let fresh = doc
        .sticky_for_range(
            &found.start_block,
            found.start_offset,
            &found.end_block,
            found.end_offset,
        )
        .expect("re-anchors");
    let live = doc.resolve_range(&fresh).expect("resolves");
    assert_eq!((live.start_offset, live.end_offset), (4, 9));
}

#[test]
fn deleted_text_is_not_found_and_the_thread_stays_an_orphan() {
    // The case orphaning is FOR: the words are gone, so nothing may be matched.
    let doc = doc_with(&[("a", "the brown fox"), ("b", "second block")]);
    assert!(index(&doc).find("quick", Some("a")).is_none());
}

#[test]
fn the_original_block_wins_when_the_same_text_occurs_twice() {
    let doc = doc_with(&[("a", "see the note"), ("b", "see the note")]);
    let idx = index(&doc);
    assert_eq!(idx.find("see the note", Some("b")).unwrap().start_block, "b");
    assert_eq!(idx.find("see the note", Some("a")).unwrap().start_block, "a");
}

#[test]
fn two_matches_and_no_home_block_refuses_to_guess() {
    // The thread's block is gone entirely and the quote fits in two places —
    // picking one is a coin toss, so it stays an orphan.
    let doc = doc_with(&[("a", "see the note"), ("b", "see the note")]);
    assert!(index(&doc).find("see the note", Some("gone")).is_none());
}

#[test]
fn casing_and_re_wrapping_do_not_stop_an_exact_match() {
    let doc = doc_with(&[("a", "The Quick\tBrown Fox")]);
    let found = index(&doc)
        .find("quick brown fox", Some("a"))
        .expect("case and whitespace kind are noise");
    assert_eq!((found.start_offset, found.end_offset), (4, 19));
}

#[test]
fn a_lightly_edited_quote_still_matches() {
    // "anchors" → "anchor", one deletion inside a long quote: still the same
    // sentence to a reader, so the thread should follow it.
    let doc = doc_with(&[("a", "comment anchor survives concurrent edits")]);
    let found = index(&doc)
        .find("comment anchors survives concurrent edits", Some("a"))
        .expect("within the edit budget");
    assert_eq!(found.start_block, "a");
    assert_eq!(found.start_offset, 0);
}

#[test]
fn a_heavily_rewritten_quote_is_refused() {
    let doc = doc_with(&[("a", "comment anchor survives concurrent edits")]);
    assert!(
        index(&doc)
            .find("footnotes are rendered at the bottom", Some("a"))
            .is_none(),
        "past the edit budget it is a different sentence"
    );
}

#[test]
fn a_short_quote_is_never_fuzzy_matched() {
    // "toots" is one edit from "tools" — 20% of the quote, inside the edit
    // budget, and still nowhere near enough evidence. Short quotes get the exact
    // tier only, or any short word would re-anchor onto any near-rhyme.
    let doc = doc_with(&[("a", "a box of tools")]);
    assert!(index(&doc).find("toots", Some("a")).is_none());
    assert!(index(&doc).find("fox", Some("a")).is_none());
}

#[test]
fn two_equally_good_fuzzy_places_are_refused() {
    // Both blocks are one insertion away from the quote, so neither is better.
    let doc = doc_with(&[
        ("a", "release the build artifact"),
        ("b", "release the build artifact"),
    ]);
    assert!(
        index(&doc)
            .find("release the build artifacts", Some("gone"))
            .is_none(),
        "the same near-match twice is ambiguous"
    );
}

#[test]
fn a_quote_spanning_blocks_is_matched_across_them() {
    // The client joins a multi-block selection with '\n'; the index joins blocks
    // with a boundary, so the same quote matches back across the same blocks.
    let doc = doc_with(&[("a", "first line"), ("b", "second line")]);
    let found = index(&doc)
        .find("line\nsecond", Some("a"))
        .expect("crosses the block boundary");
    assert_eq!((found.start_block.as_str(), found.start_offset), ("a", 6));
    assert_eq!((found.end_block.as_str(), found.end_offset), ("b", 6));
}

#[test]
fn a_truncated_quote_matches_its_prefix() {
    // The client cuts a long selection at 300 chars and appends '…'; with the
    // ellipsis in the pattern the exact tier could never hit.
    let doc = doc_with(&[("a", "a long paragraph that was cut short in the quote")]);
    let found = index(&doc)
        .find("a long paragraph that was cut…", Some("a"))
        .expect("the ellipsis is a truncation marker, not text");
    assert_eq!((found.start_offset, found.end_offset), (0, 29));
}

#[test]
fn an_empty_quote_matches_nothing() {
    // Quote is optional on the create endpoint — an empty one must not match the
    // first character of the document.
    let doc = doc_with(&[("a", "some text")]);
    assert!(index(&doc).find("", Some("a")).is_none());
    assert!(index(&doc).find("   ", Some("a")).is_none());
}

#[test]
fn offsets_are_utf16_like_everything_else_in_the_anchor_path() {
    // An emoji is 2 UTF-16 units; a range whose offsets were counted in chars
    // would anchor one unit short and the highlight would cut a surrogate pair.
    let doc = doc_with(&[("a", "🎉🎉 party time")]);
    let found = index(&doc).find("party", Some("a")).expect("found");
    assert_eq!((found.start_offset, found.end_offset), (5, 10));
    // And the offsets are usable as an anchor (they are in range for yrs).
    assert!(doc
        .sticky_for_range("a", found.start_offset, "a", found.end_offset)
        .is_some());
}
