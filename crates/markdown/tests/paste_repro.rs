//! LLM-output-shaped paste, pinned end to end.
//!
//! Written while chasing a desktop panic (poisoned mutex → blank page) that a
//! user hit after pasting ChatGPT output into a page. It did NOT reproduce the
//! panic — the engine handles this content fine — so the root cause lies on the
//! rich-paste path (HTML→Markdown in Dart, then per-block FFI writes), not here.
//!
//! The fixture stays because the shape is worth pinning and nothing else pinned
//! it: nested fences (a ```markdown block whose body is markdown), CJK prose
//! mixed with ASCII, box-drawing/arrow glyphs in code blocks, and marks over
//! CJK — where UTF-16 and byte offsets are easiest to confuse.

const PASTE: &str = include_str!("fixtures/paste.md");

#[test]
fn import_the_pasted_chatgpt_output() {
    let payload = mica_markdown::import_markdown(PASTE, "root");
    eprintln!("blocks = {}", payload.blocks.len());
    assert!(!payload.blocks.is_empty());
}

#[test]
fn round_trip_the_pasted_chatgpt_output() {
    let payload = mica_markdown::import_markdown(PASTE, "root");
    let out = mica_markdown::export_markdown(&payload).expect("export");
    // Re-import the export: the round-trip invariant (CLAUDE.md principle #4).
    let again = mica_markdown::import_markdown(&out, "root");
    assert_eq!(
        payload.blocks.len(),
        again.blocks.len(),
        "round-trip changed the block count"
    );
}

#[test]
fn export_html_of_the_pasted_chatgpt_output() {
    let payload = mica_markdown::import_markdown(PASTE, "root");
    let html = mica_markdown::export_html_document(&payload, "代码审查", 800).expect("html");
    assert!(!html.is_empty());
}

/// Marks are UTF-16 offsets into `text`; CJK is where those get confused with
/// byte offsets. Assert every mark stays inside its block.
#[test]
fn marks_stay_within_their_block_text() {
    let payload = mica_markdown::import_markdown(PASTE, "root");
    for b in &payload.blocks {
        let len = b.text.encode_utf16().count() as u64;
        let Some(marks) = b.data.get("marks").and_then(|m| m.as_array()) else {
            continue;
        };
        for m in marks {
            let end = m.get("end").and_then(|v| v.as_u64()).unwrap_or(0);
            assert!(
                end <= len,
                "mark ends at {end} but block {} has {len} UTF-16 units: {:?}",
                b.id,
                b.text
            );
        }
    }
}
