//! A real page must not make the importer allocate without bound.
//!
//! A 19.6 KB, 175-line page from a public wiki (`kvm/vhost-user.md`) drove
//! `import_markdown` to request **1.6 GB in a single allocation**. Rust aborts
//! on allocation failure, and an abort inside a `cdylib` takes the host process
//! with it: the desktop app vanished mid-import having written 155 of ~840
//! pages, and Windows logged 0xc0000409 twice at the same offset.
//!
//! Same family as `nested_bracket_perf.rs` — `import_markdown` runs on
//! user-supplied markdown (import + MCP), so a page that costs unbounded
//! resources is a real availability bug, not a curiosity. That file guards
//! TIME; this one guards MEMORY.
//!
//! Two things shaped this test:
//!
//! - **It cannot be caught.** `catch_unwind` handles panics; allocation failure
//!   aborts. There is no guard-the-call-site fix — the parser has to not ask
//!   for the memory.
//! - **The input is unremarkable.** No enormous line, no deep nesting, nothing
//!   an input-size limit would reject. The fixture is kept verbatim rather than
//!   reduced: two attempts at a hand-written minimal case did NOT reproduce, so
//!   the trigger is not yet understood, and a "simplified" repro would be
//!   testing a guess instead of the failure.

/// The first 42 lines — the shortest prefix of the page that still blows up,
/// found by bisection.
const BLOWUP: &str = include_str!("data/vhost-user-42.md");

#[test]
fn a_real_page_imports_without_a_runaway_allocation() {
    let payload = mica_markdown::import_markdown(BLOWUP, "root");
    // Reaching here at all IS the assertion: the failure mode is an abort, not
    // a wrong answer. The block count only guards the opposite over-correction
    // — a parser that "fixes" this by returning nothing.
    assert!(
        payload.blocks.len() > 5,
        "the page should still parse into real blocks, got {}",
        payload.blocks.len()
    );
}

/// The trigger, reduced: an ordered list, a blank line, then a paragraph
/// indented with **U+00A0 (no-break space)** rather than ASCII spaces.
///
/// This is why two hand-written "8 spaces after a list" cases failed to
/// reproduce — the page is CJK, and whatever produced it indented with NBSP.
/// `cat -A` on the fixture shows `M-BM-` (0xC2 0xA0) eight times where the eye
/// sees leading whitespace.
#[test]
fn a_paragraph_indented_with_no_break_spaces_after_a_list() {
    let src = "1. a\n2. b\n\n\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}x\n";
    let payload = mica_markdown::import_markdown(src, "root");
    assert!(!payload.blocks.is_empty());
    let out = mica_markdown::export_markdown(&payload).expect("export should succeed");
    assert!(!out.is_empty());
}

/// The same content through export, so a fix that merely moves the cost from
/// import to export does not read as green.
#[test]
fn the_same_page_round_trips() {
    let payload = mica_markdown::import_markdown(BLOWUP, "root");
    let out = mica_markdown::export_markdown(&payload).expect("export should succeed");
    assert!(!out.is_empty());
}
