//! Pins Markdown/HTML export as linear in the number of blocks.
//!
//! The exporters look every child up by id. That lookup was a linear scan of
//! the block list, so a document of B blocks exported in O(B²): 20k paragraphs
//! took 3.2s (release) — paid by export, share pages, MCP reads and every
//! `/ai/edits/preview` call. Thresholds are loose (seconds against ~0.1s
//! actual in a debug build) so they catch the quadratic coming back without
//! flaking on a loaded box: the old code needs ~30s here in debug.

#[test]
fn export_is_linear_in_block_count() {
  let md = "Some *normal* text with `code` and [a link](https://x.y).\n\n".repeat(20_000);
  let snap = mica_markdown::import_markdown(&md, "root");
  assert!(snap.blocks.len() > 20_000, "sanity: one block per paragraph");

  let started = std::time::Instant::now();
  mica_markdown::export_markdown(&snap).unwrap();
  mica_markdown::export_html(&snap).unwrap();
  let ms = started.elapsed().as_millis();
  assert!(ms < 5000, "exporting 20k paragraphs took {ms}ms — child lookup is a linear scan again");
}
