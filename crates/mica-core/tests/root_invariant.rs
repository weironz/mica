//! Regression: an empty root id must never erase a document's root.
//!
//! A client that seeded from a rootless local copy used to push
//! `meta.root = ""`. Because a yrs map insert is last-writer-wins, that erased
//! the root for every replica, and the document then failed to render for
//! everyone with `block not found: ` (an EMPTY id in the message is the
//! signature of this bug). It also self-perpetuated: each later write read the
//! empty root back and wrote it out again.

use mica_core::{Block, MicaDoc};

fn block(id: &str, children: Vec<&str>) -> Block {
    Block {
        id: id.to_string(),
        kind: "paragraph".to_string(),
        text: String::new(),
        data: serde_json::Value::Null,
        children: children.into_iter().map(str::to_string).collect(),
    }
}

#[test]
fn set_blocks_with_empty_root_keeps_the_existing_root() {
    let mut doc = MicaDoc::from_blocks("root_1", &[block("root_1", vec!["a"]), block("a", vec![])]);
    assert_eq!(doc.root_block_id(), "root_1");

    // A replica that doesn't know the root writes blocks anyway.
    doc.set_blocks("", &[block("a", vec![]), block("b", vec![])]);

    assert_eq!(
        doc.root_block_id(),
        "root_1",
        "an empty root id means 'unknown', not 'clear it'"
    );
}

#[test]
fn set_blocks_with_a_real_root_still_replaces_it() {
    let mut doc = MicaDoc::from_blocks("root_1", &[block("root_1", vec![])]);
    doc.set_blocks("root_2", &[block("root_2", vec![])]);
    assert_eq!(doc.root_block_id(), "root_2");
}

/// The empty root must not survive a round-trip through the wire encoding
/// either — this is the shape the server actually persists.
#[test]
fn the_kept_root_survives_encode_decode() {
    let mut doc = MicaDoc::from_blocks("root_1", &[block("root_1", vec![])]);
    doc.set_blocks("", &[block("a", vec![])]);
    let restored = MicaDoc::from_update(&doc.encode_state()).expect("decode");
    assert_eq!(restored.root_block_id(), "root_1");
}
