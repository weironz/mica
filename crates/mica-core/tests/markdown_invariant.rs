//! P2-M1: the round-trip invariant extends through the yrs model.
//!
//! For any markdown, exporting it after a trip through `MicaDoc` must equal
//! exporting it directly — i.e. `md → blocks → MicaDoc → blocks → md` ==
//! `md → blocks → md`. This ties the CRDT document model to `crates/markdown`,
//! the markdown authority, and is the §10 red-line "round-trip is invariant".

use mica_core::{Block as CoreBlock, MicaDoc};
use mica_markdown::{export_markdown, import_markdown, Block as MdBlock, DocumentSnapshotPayload};

fn md_blocks_to_core(blocks: &[MdBlock]) -> Vec<CoreBlock> {
    // markdown::Block and mica_core::Block share an identical serde shape.
    serde_json::from_value(serde_json::to_value(blocks).unwrap()).unwrap()
}

fn core_blocks_to_md(blocks: &[CoreBlock]) -> Vec<MdBlock> {
    serde_json::from_value(serde_json::to_value(blocks).unwrap()).unwrap()
}

/// Export markdown after routing the document through MicaDoc.
fn via_mica(md: &str) -> String {
    let payload = import_markdown(md, "root");
    let core = md_blocks_to_core(&payload.blocks);
    let doc = MicaDoc::from_blocks(&payload.root_block_id, &core);
    let out = doc.to_blocks();
    let payload2 = DocumentSnapshotPayload {
        schema_version: 1,
        root_block_id: payload.root_block_id.clone(),
        blocks: core_blocks_to_md(&out),
    };
    export_markdown(&payload2).unwrap()
}

/// Export markdown directly (the markdown crate's own normalization).
fn direct(md: &str) -> String {
    export_markdown(&import_markdown(md, "root")).unwrap()
}

fn assert_invariant(md: &str) {
    assert_eq!(
        via_mica(md),
        direct(md),
        "markdown round-trip via MicaDoc diverged for input:\n{md}"
    );
}

#[test]
fn headings_and_paragraphs() {
    assert_invariant("# Heading 1\n\n## Heading 2\n\nSome body text here.\n");
}

#[test]
fn inline_emphasis() {
    assert_invariant("This is **bold**, this is *italic*, and this is `code`.\n");
}

#[test]
fn links() {
    assert_invariant("Visit [Mica](https://mica.dev) for more.\n");
}

#[test]
fn unordered_list() {
    assert_invariant("- one\n- two\n- three\n");
}

#[test]
fn ordered_list() {
    assert_invariant("1. first\n2. second\n3. third\n");
}

#[test]
fn blockquote_and_code_block() {
    assert_invariant("> a quote\n\n```rust\nfn main() {}\n```\n");
}

#[test]
fn mixed_document() {
    assert_invariant(
        "# Title\n\nIntro with **bold** and a [link](https://example.com).\n\n\
         - list item one\n- list item two\n\n> blockquote\n",
    );
}
