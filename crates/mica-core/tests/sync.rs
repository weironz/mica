//! P2-M4: CRDT sync primitives — divergent replicas converge via state-vector
//! diff exchange, and merges are idempotent + order-independent.

use mica_core::{Block, MicaDoc};

fn para(id: &str, text: &str) -> Block {
    Block::new(id, "paragraph").with_text(text)
}

fn page(children: &[&str]) -> Block {
    Block::new("r", "page").with_children(children.iter().map(|s| s.to_string()).collect())
}

/// A fresh replica of a shared base, with its own actor id.
fn replica(base_state: &[u8], client_id: u64) -> MicaDoc {
    MicaDoc::from_update_with_client_id(base_state, Some(client_id)).expect("decode base")
}

fn base() -> Vec<u8> {
    MicaDoc::from_blocks_with_client_id("r", &[page(&["a"]), para("a", "Hello")], Some(1))
        .encode_state()
}

/// One round of mutual sync: each side sends the other only what its state
/// vector is missing, then applies it. After this both have all updates.
fn sync(a: &mut MicaDoc, b: &mut MicaDoc) {
    let a_sv = a.state_vector();
    let b_sv = b.state_vector();
    let a_to_b = a.encode_diff(&b_sv).unwrap();
    let b_to_a = b.encode_diff(&a_sv).unwrap();
    a.apply_update(&b_to_a).unwrap();
    b.apply_update(&a_to_b).unwrap();
}

#[test]
fn concurrent_edits_converge() {
    let state = base();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);

    // Concurrent, non-conflicting: A appends to a block's text, B adds a block.
    a.text_insert("a", 5, " from A");
    b.insert_block("r", 1, &para("b", "from B"));

    sync(&mut a, &mut b);

    assert_eq!(a.to_blocks(), b.to_blocks(), "replicas converge");
    let out = a.to_blocks();
    let block_a = out.iter().find(|x| x.id == "a").unwrap();
    assert_eq!(block_a.text, "Hello from A");
    assert!(out.iter().any(|x| x.id == "b" && x.text == "from B"));
    assert_eq!(out.iter().find(|x| x.id == "r").unwrap().children, vec!["a", "b"]);
}

#[test]
fn concurrent_text_into_same_block_converges() {
    let state = base();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);

    // Both insert different text at the SAME offset of the SAME block's Text.
    // yrs merges character-level; the actor ids give a deterministic order.
    a.text_insert("a", 5, "[A]");
    b.text_insert("a", 5, "[B]");

    sync(&mut a, &mut b);

    assert_eq!(a.to_blocks(), b.to_blocks(), "same block text converges");
    let text = a.to_blocks().into_iter().find(|x| x.id == "a").unwrap().text;
    assert!(text.contains("[A]") && text.contains("[B]"), "both edits survive: {text}");
    assert!(text.starts_with("Hello"));
}

#[test]
fn apply_is_idempotent() {
    let state = base();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);
    b.insert_block("r", 1, &para("b", "B"));

    let diff = b.encode_diff(&a.state_vector()).unwrap();
    a.apply_update(&diff).unwrap();
    let once = a.to_blocks();
    // Re-applying the same update changes nothing.
    a.apply_update(&diff).unwrap();
    assert_eq!(a.to_blocks(), once, "re-applying an update is a no-op");
}

#[test]
fn merge_is_order_independent() {
    let state = base();
    // Three actors each make one edit.
    let mut x = replica(&state, 10);
    let mut y = replica(&state, 20);
    let mut z = replica(&state, 30);
    x.text_insert("a", 5, "X");
    y.insert_block("r", 1, &para("b", "Y"));
    z.update_block_kind("a", "heading");

    let dx = x.encode_diff(&MicaDoc::from_update_with_client_id(&state, Some(99)).unwrap().state_vector()).unwrap();
    let dy = y.encode_diff(&MicaDoc::from_update_with_client_id(&state, Some(99)).unwrap().state_vector()).unwrap();
    let dz = z.encode_diff(&MicaDoc::from_update_with_client_id(&state, Some(99)).unwrap().state_vector()).unwrap();

    // Apply the three diffs onto two fresh replicas in DIFFERENT orders.
    let mut p = replica(&state, 40);
    let mut q = replica(&state, 50);
    for d in [&dx, &dy, &dz] {
        p.apply_update(d).unwrap();
    }
    for d in [&dz, &dx, &dy] {
        q.apply_update(d).unwrap();
    }
    assert_eq!(p.to_blocks(), q.to_blocks(), "merge order doesn't matter");
}
