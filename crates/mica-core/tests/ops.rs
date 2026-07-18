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

/// P2 §6: the local→cloud migration replays the local block tree onto the
/// *freshly-created cloud doc's root* (strategy (c), docs/phase2-offline-crdt.md
/// §7.1). The key invariant: it never rewrites `meta.root`, so there is no
/// concurrent LWW collision and no orphaned subtree — every block stays
/// reachable from the cloud root. This mirrors exactly what `buildMigrationOps`
/// emits (update the seeded cloud root with the local root's kind/text, then
/// insert the local children under it).
#[test]
fn migration_replays_onto_cloud_root_without_meta_collision() {
    // A just-created cloud doc: one block which *is* the root (a paragraph), as
    // `create_document` seeds it.
    let cloud_root = "block_cloud";
    let mut d = MicaDoc::from_blocks(cloud_root, &[Block::new(cloud_root, "paragraph")]);
    assert_eq!(d.root_block_id(), cloud_root);

    // Replay a local tree: page "Title" → [p1, list L → image I(file_id rewritten)].
    d.update_block_kind(cloud_root, "page");
    d.set_block_text(cloud_root, "Title", &[]);
    d.insert_block(cloud_root, 0, &para("p1", "hello"));
    d.insert_block(cloud_root, 1, &Block::new("L", "list"));
    d.insert_block(
        "L",
        0,
        &Block::new("I", "image").with_data(json!({"file_id": "uuid-123", "name": "pic.png"})),
    );

    // The cloud root id is untouched — meta.root was never rewritten.
    assert_eq!(d.root_block_id(), cloud_root);

    let out = d.to_blocks();
    assert_eq!(get(&out, cloud_root).kind, "page");
    assert_eq!(get(&out, cloud_root).text, "Title");
    assert_eq!(get(&out, cloud_root).children, vec!["p1", "L"]);
    assert_eq!(get(&out, "L").children, vec!["I"]);
    // The reconciled image file_id (sha256→UUID) and other props survive.
    assert_eq!(get(&out, "I").data, json!({"file_id": "uuid-123", "name": "pic.png"}));

    // No orphans: `to_blocks` is a DFS from the root, appending any unreachable
    // block last. The full DFS order covering every block proves reachability.
    assert_eq!(ids(&out), vec!["block_cloud", "p1", "L", "I"]);
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

#[test]
fn set_blocks_reverts_content_as_forward_ops() {
    // v1: page[a, b] with a="one", b="two".
    let mut d = doc(vec![page(&["a", "b"]), para("a", "one"), para("b", "two")]);
    let v1_blocks = d.to_blocks();

    // Edit to v2: change a, delete b, add c → page[a, c].
    d.set_block_text("a", "one edited", &[]);
    d.delete_block("b", false);
    d.insert_block("r", 1, &para("c", "three"));
    // A second client sitting at v2 (shares d's history up to here).
    let replica_v2 = MicaDoc::from_update(&d.encode_state()).expect("v2 decode");

    // Restore d to v1 as FORWARD ops, capturing the update it produces.
    let sv = d.state_vector();
    d.set_blocks("r", &v1_blocks);
    let restore_update = d.encode_diff(&sv).expect("restore diff");

    // The restored doc equals v1 exactly (b came back, c gone, a reverted).
    assert_eq!(d.to_blocks(), v1_blocks, "doc reverted to v1");

    // Crucially, the restore is a normal update, not a reset: a replica at v2
    // that merely applies it ALSO converges to v1 (CRDT-safe under concurrency).
    let mut replica = replica_v2;
    replica.apply_update(&restore_update).expect("apply restore");
    assert_eq!(replica.to_blocks(), v1_blocks, "replica converges to v1");
}
