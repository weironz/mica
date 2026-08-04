//! P2-M4: CRDT sync primitives — divergent replicas converge via state-vector
//! diff exchange, and merges are idempotent + order-independent.

use mica_core::{Block, MicaDoc};
use serde_json::json;

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

/// Two people typing into ONE paragraph, through the path the editor actually
/// uses.
///
/// The tests above call `text_insert` — the fine-grained API, which always
/// interleaved correctly. The editor's hot path does not touch it: typing
/// emits `update_block` carrying the block's WHOLE new text, which lands in
/// [`MicaDoc::set_block_text`]. That used to mean "remove the entire Text
/// range, insert the new string", and a whole-range replacement is not
/// something a CRDT can interleave — both replicas' inserts survived while
/// only one of the two deletions had anything to delete, so `"Hello"` plus a
/// character at each end converged on **`"AHelloHelloB"`**: the paragraph
/// duplicated. (Measured, 2026-08-04, before the fix.)
///
/// `set_text_and_marks` now writes the minimal splice
/// (`text_diff::utf16_text_diff`), which puts the two writers on disjoint
/// ranges. Convergence was never the broken part — what they converged ON was.
#[test]
fn concurrent_typing_in_one_block_interleaves_not_duplicates() {
    let state = base();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);

    // Each editor sends its block's full new text, exactly as `update_block`
    // does: A typed at the front, B typed at the end.
    a.set_block_text("a", "AHello", &[]);
    b.set_block_text("a", "HelloB", &[]);

    sync(&mut a, &mut b);

    assert_eq!(a.to_blocks(), b.to_blocks(), "replicas converge");
    let text = a.to_blocks().into_iter().find(|x| x.id == "a").unwrap().text;
    assert_eq!(text, "AHelloB", "both edits, once each");

    // The fine-grained path agrees — the two are no longer two behaviours.
    let mut c = replica(&state, 10);
    let mut d = replica(&state, 20);
    c.text_insert("a", 0, "A");
    d.text_insert("a", 5, "B");
    sync(&mut c, &mut d);
    assert_eq!(
        c.to_blocks().into_iter().find(|x| x.id == "a").unwrap().text,
        text,
        "coarse update_block and text_insert now land the same thing",
    );
}

/// The same, with the edits INSIDE the word and with CJK — the offsets are
/// UTF-16 units, and a splice computed in the wrong unit would cut a character
/// in half rather than merge.
#[test]
fn concurrent_typing_survives_cjk_and_interior_edits() {
    let state = MicaDoc::from_blocks_with_client_id(
        "r",
        &[page(&["a"]), para("a", "笔记软件")],
        Some(1),
    )
    .encode_state();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);

    a.set_block_text("a", "笔记好软件", &[]); // insert at 2
    b.set_block_text("a", "笔记软件很棒", &[]); // append at 4

    sync(&mut a, &mut b);

    assert_eq!(a.to_blocks(), b.to_blocks());
    let text = a.to_blocks().into_iter().find(|x| x.id == "a").unwrap().text;
    assert_eq!(text, "笔记好软件很棒");
}

/// A concurrent edit must not be swallowed by the OTHER replica's re-format.
/// Marks are still written coarsely (clear-all + re-apply), so this pins that
/// the coarse half cannot eat the fine half's text.
#[test]
fn a_concurrent_format_does_not_eat_a_concurrent_keystroke() {
    let state = base();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);

    // A bolds the whole word without changing a character; B types.
    // Built the way the editor does: marks arrive inside the block's `data`.
    let bold = mica_core::marks_from_data(&json!({
        "marks": [{"start": 0, "end": 5, "type": "bold"}]
    }));
    a.set_block_text("a", "Hello", &bold);
    b.set_block_text("a", "Hello!", &[]);

    sync(&mut a, &mut b);

    assert_eq!(a.to_blocks(), b.to_blocks());
    let out = a.to_blocks().into_iter().find(|x| x.id == "a").unwrap();
    assert_eq!(out.text, "Hello!", "the keystroke survives the re-format");
}

#[test]
fn concurrent_props_fields_converge() {
    // A block whose data already has one prop key.
    let state = MicaDoc::from_blocks_with_client_id(
        "r",
        &[
            page(&["a"]),
            Block::new("a", "paragraph")
                .with_text("x")
                .with_data(json!({ "indent": 1 })),
        ],
        Some(1),
    )
    .encode_state();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);

    // Concurrent edits to DIFFERENT props fields of the SAME block. With the old
    // whole-blob string props this was last-write-wins (one edit lost); with the
    // field-level MapRef (P2-M4.7) both survive.
    a.set_block_data("a", &json!({ "indent": 1, "checked": true }));
    b.set_block_data("a", &json!({ "indent": 1, "level": 2 }));

    sync(&mut a, &mut b);

    assert_eq!(a.to_blocks(), b.to_blocks(), "replicas converge");
    let data = a.to_blocks().into_iter().find(|x| x.id == "a").unwrap().data;
    assert_eq!(data["indent"], 1, "untouched field kept");
    assert_eq!(data["checked"], true, "A's field survives");
    assert_eq!(data["level"], 2, "B's field survives");
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

/// Marks of a block, as `{start,end,type}` triples, for readable assertions.
fn marks_of(d: &MicaDoc, id: &str) -> Vec<(u32, u32, String)> {
    let b = d.to_blocks().into_iter().find(|x| x.id == id).unwrap();
    mica_core::marks_from_data(&b.data)
        .into_iter()
        .map(|m| (m.start, m.end, m.ty))
        .collect()
}

fn marks(v: serde_json::Value) -> Vec<mica_core::Mark> {
    mica_core::marks_from_data(&json!({ "marks": v }))
}

/// Two people formatting DIFFERENT words at the same time must both win.
///
/// The coarse writer could not do this: it said "clear the formatting of the
/// whole block, then replay my marks", an operation over the entire text, so
/// whichever update merged second erased the other's bold outright. Nothing
/// about the text changed in either edit — this is purely the formatting half.
#[test]
fn concurrent_formatting_of_different_words_both_survive() {
    let state = base();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);
    a.set_block_text("a", "Hello world", &[]);
    sync(&mut a, &mut b);

    a.set_block_text(
        "a",
        "Hello world",
        &marks(json!([{"start": 0, "end": 5, "type": "bold"}])),
    );
    b.set_block_text(
        "a",
        "Hello world",
        &marks(json!([{"start": 6, "end": 11, "type": "italic"}])),
    );
    sync(&mut a, &mut b);

    assert_eq!(a.to_blocks(), b.to_blocks(), "replicas converge");
    let mut got = marks_of(&a, "a");
    got.sort();
    assert_eq!(
        got,
        vec![
            (0, 5, "bold".to_string()),
            (6, 11, "italic".to_string()),
        ],
        "both formats survive"
    );
}

/// Removing a mark is now expressible over just its own range, so an un-bold
/// no longer takes an unrelated concurrent format with it.
///
/// A un-bolds everything; B, who still sees it all bold, italicises the second
/// word. The un-bold must land AND the italic must survive it.
#[test]
fn an_unbold_does_not_erase_a_concurrent_italic() {
    let state = base();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);
    let bold_all = marks(json!([{"start": 0, "end": 11, "type": "bold"}]));
    a.set_block_text("a", "Hello world", &bold_all);
    sync(&mut a, &mut b);

    a.set_block_text("a", "Hello world", &[]);
    b.set_block_text(
        "a",
        "Hello world",
        &marks(json!([
            {"start": 0, "end": 11, "type": "bold"},
            {"start": 6, "end": 11, "type": "italic"},
        ])),
    );
    sync(&mut a, &mut b);

    assert_eq!(a.to_blocks(), b.to_blocks(), "replicas converge");
    let got = marks_of(&a, "a");
    assert!(
        !got.iter().any(|(_, _, t)| t == "bold"),
        "the un-bold lands: {got:?}"
    );
    assert!(
        got.contains(&(6, 11, "italic".to_string())),
        "the concurrent italic survives it: {got:?}"
    );
}

/// Un-bolding ONE word leaves the other bold — the removal is scoped to the
/// range it was asked for, not to the block.
#[test]
fn unbolding_one_word_leaves_the_other_bold() {
    let state = base();
    let mut a = replica(&state, 10);
    a.set_block_text(
        "a",
        "Hello world",
        &marks(json!([{"start": 0, "end": 11, "type": "bold"}])),
    );
    a.set_block_text(
        "a",
        "Hello world",
        &marks(json!([{"start": 6, "end": 11, "type": "bold"}])),
    );
    assert_eq!(marks_of(&a, "a"), vec![(6, 11, "bold".to_string())]);
}

/// Formatting and typing at the same time, in the same block: neither is lost.
/// The text splice and the format delta touch disjoint ranges by construction.
#[test]
fn a_concurrent_format_and_keystroke_both_land() {
    let state = base();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);
    a.set_block_text("a", "Hello world", &[]);
    sync(&mut a, &mut b);

    a.set_block_text(
        "a",
        "Hello world",
        &marks(json!([{"start": 0, "end": 5, "type": "bold"}])),
    );
    b.set_block_text("a", "Hello world!", &[]);
    sync(&mut a, &mut b);

    assert_eq!(a.to_blocks(), b.to_blocks(), "replicas converge");
    let out = a.to_blocks().into_iter().find(|x| x.id == "a").unwrap();
    assert_eq!(out.text, "Hello world!", "the keystroke survives");
    assert_eq!(
        marks_of(&a, "a"),
        vec![(0, 5, "bold".to_string())],
        "and so does the bold"
    );
}

/// A removal must not be undone by someone else's unrelated formatting.
///
/// The coarse writer re-stated the block's ENTIRE formatting on every change,
/// so B — who was only bolding the third word — also re-asserted the bold on
/// the first word that A had just removed, resurrecting it.
#[test]
fn a_concurrent_format_elsewhere_does_not_resurrect_a_removed_bold() {
    let state = base();
    let mut a = replica(&state, 10);
    let mut b = replica(&state, 20);
    a.set_block_text(
        "a",
        "one two three",
        &marks(json!([{"start": 0, "end": 3, "type": "bold"}])),
    );
    sync(&mut a, &mut b);

    // A un-bolds "one"; B, still seeing it bold, bolds "three" as well.
    a.set_block_text("a", "one two three", &[]);
    b.set_block_text(
        "a",
        "one two three",
        &marks(json!([
            {"start": 0, "end": 3, "type": "bold"},
            {"start": 8, "end": 13, "type": "bold"},
        ])),
    );
    sync(&mut a, &mut b);

    assert_eq!(a.to_blocks(), b.to_blocks(), "replicas converge");
    let got = marks_of(&a, "a");
    assert_eq!(
        got,
        vec![(8, 13, "bold".to_string())],
        "the removal holds and only the new bold is there: {got:?}"
    );
}
