//! P2-M1: yrs document model round-trip invariants.
//!
//! These prove `blocks → MicaDoc → blocks` preserves text, inline marks (incl.
//! overlapping + links), block attrs, and tree order — and that the same holds
//! across a yrs encode/decode. The marks↔Text-delta mapping (the §12 risk) is
//! the thing under test.

use mica_core::{Block, MicaDoc};
use serde_json::json;

/// Build a doc from blocks, read it back, and assert equality.
fn roundtrip(root: &str, blocks: Vec<Block>) -> Vec<Block> {
    let doc = MicaDoc::from_blocks(root, &blocks);
    doc.to_blocks()
}

/// Same, but force the data through a yrs encode → decode first.
fn roundtrip_encoded(root: &str, blocks: Vec<Block>) -> Vec<Block> {
    let doc = MicaDoc::from_blocks(root, &blocks);
    let bytes = doc.encode_state();
    let restored = MicaDoc::from_update(&bytes).expect("decode");
    assert_eq!(restored.root_block_id(), root, "root id survives encode/decode");
    restored.to_blocks()
}

fn para(id: &str, text: &str) -> Block {
    Block::new(id, "paragraph").with_text(text)
}

#[test]
fn plain_text_block() {
    let blocks = vec![Block::new("r", "page").with_children(vec!["a".into()]), para("a", "Hello, world")];
    assert_eq!(roundtrip("r", blocks.clone()), blocks);
    assert_eq!(roundtrip_encoded("r", blocks.clone()), blocks);
}

#[test]
fn simple_marks() {
    // "Hello world" with bold over "Hello", italic over "world".
    let a = para("a", "Hello world").with_data(json!({
        "marks": [
            {"start": 0, "end": 5, "type": "bold"},
            {"start": 6, "end": 11, "type": "italic"},
        ]
    }));
    let blocks = vec![Block::new("r", "page").with_children(vec!["a".into()]), a];
    assert_eq!(roundtrip("r", blocks.clone()), blocks);
    assert_eq!(roundtrip_encoded("r", blocks.clone()), blocks);
}

#[test]
fn overlapping_marks_recombine() {
    // bold over all of "abcdef", italic over "cd" → yrs splits into 3 runs that
    // must recombine into bold[0,6] + italic[2,4].
    let a = para("a", "abcdef").with_data(json!({
        "marks": [
            {"start": 0, "end": 6, "type": "bold"},
            {"start": 2, "end": 4, "type": "italic"},
        ]
    }));
    let blocks = vec![Block::new("r", "page").with_children(vec!["a".into()]), a];
    assert_eq!(roundtrip("r", blocks.clone()), blocks);
}

#[test]
fn link_mark_keeps_href_and_title() {
    let a = para("a", "see here").with_data(json!({
        "marks": [
            {"start": 4, "end": 8, "type": "link", "href": "https://mica.dev", "title": "Mica"},
        ]
    }));
    let blocks = vec![Block::new("r", "page").with_children(vec!["a".into()]), a];
    assert_eq!(roundtrip("r", blocks.clone()), blocks);
    assert_eq!(roundtrip_encoded("r", blocks.clone()), blocks);
}

#[test]
fn utf16_offsets_align_with_dart_indices() {
    // Chinese chars are 1 UTF-16 unit each; an emoji is a surrogate pair (2).
    // Marks use Dart UTF-16 string indices, so the Doc's Utf16 offset kind must
    // keep "a😀b" → bold over "😀" = [1,3].
    let a = para("a", "a😀b").with_data(json!({
        "marks": [ {"start": 1, "end": 3, "type": "bold"} ]
    }));
    let b = para("b", "你好世界").with_data(json!({
        "marks": [ {"start": 2, "end": 4, "type": "italic"} ]
    }));
    let blocks = vec![
        Block::new("r", "page").with_children(vec!["a".into(), "b".into()]),
        a,
        b,
    ];
    assert_eq!(roundtrip("r", blocks.clone()), blocks);
    assert_eq!(roundtrip_encoded("r", blocks.clone()), blocks);
}

#[test]
fn block_attrs_preserved_alongside_marks() {
    // A heading with both a block attr (level) and an inline mark.
    let h = Block::new("a", "heading").with_text("Title").with_data(json!({
        "level": 2,
        "marks": [ {"start": 0, "end": 5, "type": "code"} ]
    }));
    let blocks = vec![Block::new("r", "page").with_children(vec!["a".into()]), h];
    assert_eq!(roundtrip("r", blocks.clone()), blocks);
    assert_eq!(roundtrip_encoded("r", blocks.clone()), blocks);
}

#[test]
fn nested_tree_dfs_order() {
    let blocks = vec![
        Block::new("r", "page").with_children(vec!["a".into(), "b".into()]),
        para("a", "first"),
        para("b", "second").with_children(vec!["c".into()]),
        para("c", "nested under b"),
    ];
    // DFS from r: r, a, b, c — matches input order.
    assert_eq!(roundtrip("r", blocks.clone()), blocks);
    assert_eq!(roundtrip_encoded("r", blocks.clone()), blocks);
}

#[test]
fn block_with_no_marks_has_null_data() {
    let blocks = vec![Block::new("r", "page").with_children(vec!["a".into()]), para("a", "plain")];
    let out = roundtrip("r", blocks);
    // data stays Null (no attrs, no marks) — not an empty object.
    assert_eq!(out[1].data, serde_json::Value::Null);
}
