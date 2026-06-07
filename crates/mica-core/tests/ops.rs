//! P2-M1: editor-intent operations on MicaDoc.

use mica_core::{Block, MicaDoc};
use serde_json::{json, Value};

fn para(id: &str, text: &str) -> Block {
    Block::new(id, "paragraph").with_text(text)
}

fn page(children: &[&str]) -> Block {
    Block::new("r", "page").with_children(children.iter().map(|s| s.to_string()).collect())
}

/// Build a doc rooted at "r" from blocks.
fn doc(blocks: Vec<Block>) -> MicaDoc {
    MicaDoc::from_blocks("r", &blocks)
}

fn get<'a>(blocks: &'a [Block], id: &str) -> &'a Block {
    blocks.iter().find(|b| b.id == id).expect("block present")
}

fn ids(blocks: &[Block]) -> Vec<String> {
    blocks.iter().map(|b| b.id.clone()).collect()
}

#[test]
fn insert_block_into_parent() {
    let mut d = doc(vec![page(&["a"]), para("a", "one")]);
    d.insert_block("r", 1, &para("b", "two"));
    let out = d.to_blocks();
    assert_eq!(get(&out, "r").children, vec!["a", "b"]);
    assert_eq!(get(&out, "b").text, "two");
}

#[test]
fn insert_block_index_clamped_and_ordered() {
    let mut d = doc(vec![page(&["a"]), para("a", "one")]);
    d.insert_block("r", 0, &para("b", "two")); // before a
    assert_eq!(get(&d.to_blocks(), "r").children, vec!["b", "a"]);
}

#[test]
fn update_kind_and_data() {
    let mut d = doc(vec![page(&["a"]), para("a", "Title")]);
    d.update_block_kind("a", "heading");
    d.set_block_data("a", &json!({"level": 1}));
    let out = d.to_blocks();
    assert_eq!(get(&out, "a").kind, "heading");
    assert_eq!(get(&out, "a").data, json!({"level": 1}));
}

#[test]
fn text_insert_delete() {
    let mut d = doc(vec![page(&["a"]), para("a", "Hello")]);
    d.text_insert("a", 5, " world");
    assert_eq!(get(&d.to_blocks(), "a").text, "Hello world");
    d.text_delete("a", 0, 6); // drop "Hello "
    assert_eq!(get(&d.to_blocks(), "a").text, "world");
}

#[test]
fn text_insert_before_marks_shifts_them() {
    // bold over "world"; inserting at the front must keep bold on "world".
    let a = para("a", "world").with_data(json!({"marks": [{"start": 0, "end": 5, "type": "bold"}]}));
    let mut d = doc(vec![page(&["a"]), a]);
    d.text_insert("a", 0, "hello ");
    let out = d.to_blocks();
    assert_eq!(get(&out, "a").text, "hello world");
    // bold should now cover "world" = [6,11] (CRDT formatting moved with the text)
    assert_eq!(get(&out, "a").data, json!({"marks": [{"start": 6, "end": 11, "type": "bold"}]}));
}

#[test]
fn text_format_adds_mark() {
    let mut d = doc(vec![page(&["a"]), para("a", "Hello")]);
    d.text_format("a", &mica_core::Mark { start: 0, end: 5, ty: "italic".into(), href: None, title: None });
    assert_eq!(get(&d.to_blocks(), "a").data, json!({"marks": [{"start": 0, "end": 5, "type": "italic"}]}));
}

#[test]
fn delete_block_bring_children_up() {
    let mut d = doc(vec![
        page(&["a"]),
        Block::new("a", "list").with_children(vec!["b".into(), "c".into()]),
        para("b", "b"),
        para("c", "c"),
    ]);
    d.delete_block("a", true);
    let out = d.to_blocks();
    assert_eq!(get(&out, "r").children, vec!["b", "c"]); // a's children spliced into root
    assert!(!ids(&out).contains(&"a".to_string()));
}

#[test]
fn delete_block_without_bring_children() {
    let mut d = doc(vec![
        page(&["a", "z"]),
        Block::new("a", "list").with_children(vec!["b".into()]),
        para("b", "b"),
        para("z", "z"),
    ]);
    d.delete_block("a", false);
    let out = d.to_blocks();
    assert_eq!(get(&out, "r").children, vec!["z"]); // a removed, its child not promoted
    assert!(!ids(&out).contains(&"a".to_string()));
}

#[test]
fn move_block_reparents() {
    let mut d = doc(vec![page(&["a", "b"]), para("a", "a"), para("b", "b")]);
    d.move_block("b", "a", 0);
    let out = d.to_blocks();
    assert_eq!(get(&out, "r").children, vec!["a"]);
    assert_eq!(get(&out, "a").children, vec!["b"]);
}

#[test]
fn split_block_divides_text_and_marks() {
    // "HelloWorld" bold over all; split at 5.
    let a = para("a", "HelloWorld").with_data(json!({"marks": [{"start": 0, "end": 10, "type": "bold"}]}));
    let mut d = doc(vec![page(&["a"]), a]);
    d.split_block("a", 5, "n", "paragraph");
    let out = d.to_blocks();
    assert_eq!(get(&out, "r").children, vec!["a", "n"]);
    assert_eq!(get(&out, "a").text, "Hello");
    assert_eq!(get(&out, "a").data, json!({"marks": [{"start": 0, "end": 5, "type": "bold"}]}));
    assert_eq!(get(&out, "n").text, "World");
    assert_eq!(get(&out, "n").data, json!({"marks": [{"start": 0, "end": 5, "type": "bold"}]}));
}

#[test]
fn split_moves_children_to_tail() {
    let mut d = doc(vec![
        page(&["a"]),
        Block::new("a", "paragraph").with_text("HelloWorld").with_children(vec!["c".into()]),
        para("c", "child"),
    ]);
    d.split_block("a", 5, "n", "paragraph");
    let out = d.to_blocks();
    assert_eq!(get(&out, "a").children, Vec::<String>::new());
    assert_eq!(get(&out, "n").children, vec!["c"]);
}

#[test]
fn join_into_prev_merges_text_and_marks() {
    // a="Hello" (bold), b="World" (italic). Join b into a → "HelloWorld".
    let a = para("a", "Hello").with_data(json!({"marks": [{"start": 0, "end": 5, "type": "bold"}]}));
    let b = para("b", "World").with_data(json!({"marks": [{"start": 0, "end": 5, "type": "italic"}]}));
    let mut d = doc(vec![page(&["a", "b"]), a, b]);
    d.join_into_prev("b");
    let out = d.to_blocks();
    assert_eq!(get(&out, "r").children, vec!["a"]);
    assert_eq!(get(&out, "a").text, "HelloWorld");
    assert_eq!(
        get(&out, "a").data,
        json!({"marks": [
            {"start": 0, "end": 5, "type": "bold"},
            {"start": 5, "end": 10, "type": "italic"},
        ]})
    );
    assert!(!ids(&out).contains(&"b".to_string()));
}

#[test]
fn ops_survive_encode_decode() {
    let mut d = doc(vec![page(&["a"]), para("a", "Hello")]);
    d.text_insert("a", 5, " world");
    d.insert_block("r", 1, &para("b", "second"));
    let restored = MicaDoc::from_update(&d.encode_state()).expect("decode");
    let out = restored.to_blocks();
    assert_eq!(get(&out, "a").text, "Hello world");
    assert_eq!(get(&out, "r").children, vec!["a", "b"]);
    let _ = Value::Null; // silence unused import in some configs
}

#[test]
fn set_block_marks_clears_then_reapplies() {
    // a="HelloWorld" bold over [0,5). Data-only update with empty marks must
    // clear the bold; a fresh italic over [5,10) must then stick.
    let a = para("a", "HelloWorld")
        .with_data(json!({"marks": [{"start": 0, "end": 5, "type": "bold"}]}));
    let mut d = doc(vec![page(&["a"]), a]);
    // Clear all marks (the turn-into-reset case): no marks remain.
    d.set_block_marks("a", &[]);
    assert!(get(&d.to_blocks(), "a").data.get("marks").is_none());
    // Re-apply a different mark, surviving encode/decode.
    d.set_block_marks(
        "a",
        &[mica_core::Mark { start: 5, end: 10, ty: "italic".into(), href: None, title: None }],
    );
    let restored = MicaDoc::from_update(&d.encode_state()).expect("decode");
    assert_eq!(
        get(&restored.to_blocks(), "a").data,
        json!({"marks": [{"start": 5, "end": 10, "type": "italic"}]})
    );
}
