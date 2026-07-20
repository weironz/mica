//! Does `text_format` survive a mark that runs past the end of the text?
//!
//! `text_insert` and `text_delete` both clamp their offsets against
//! `t.len(&txn)`; `text_format` does not — it only rejects `end <= start` and
//! then hands `(start, len)` straight to yrs. Marks arrive from Dart as UTF-16
//! offsets computed against a snapshot of the text, so a stale or concurrently
//! shortened block yields `end > len`.
//!
//! This matters far past one bad call: every FFI entry point holds a
//! `Mutex<MicaDoc>` and unwraps it, so a panic in here POISONS the lock and
//! every later read panics too — the document goes permanently blank.

use mica_core::marks::Mark;
use mica_core::{Block, MicaDoc};

fn doc_with(text: &str) -> MicaDoc {
    MicaDoc::from_blocks(
        "r",
        &[
            Block::new("r", "page").with_children(vec!["a".into()]),
            Block::new("a", "paragraph").with_text(text.to_string()),
        ],
    )
}

#[test]
fn format_past_end_of_text_does_not_panic() {
    let mut doc = doc_with("hi"); // len 2 in UTF-16
    doc.text_format(
        "a",
        &Mark { start: 0, end: 99, ty: "bold".into(), href: None, title: None },
    );
    // Surviving at all is the assertion; also prove the doc is still usable
    // (a poisoned/corrupt doc is what the caller actually suffers).
    let blocks = doc.to_blocks();
    assert!(blocks.iter().any(|b| b.id == "a"), "block survives: {blocks:?}");
}

#[test]
fn format_starting_past_end_does_not_panic() {
    let mut doc = doc_with("hi");
    doc.text_format(
        "a",
        &Mark { start: 50, end: 60, ty: "bold".into(), href: None, title: None },
    );
    assert!(doc.to_blocks().iter().any(|b| b.id == "a"));
}

#[test]
fn format_past_end_on_cjk_does_not_panic() {
    // CJK is 1 UTF-16 unit per char but 3 bytes — the offset units are exactly
    // where these two representations get confused.
    let mut doc = doc_with("中文"); // len 2 in UTF-16, 6 bytes
    doc.text_format(
        "a",
        &Mark { start: 1, end: 40, ty: "italic".into(), href: None, title: None },
    );
    assert!(doc.to_blocks().iter().any(|b| b.id == "a"));
}
