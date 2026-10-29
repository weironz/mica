//! Markdown → blocks → Markdown, printing whether the round trip is stable.
//! The same three calls the README's quick start shows, with the round-trip
//! assertion the engine is sold on.

use mica_md::{ParseMode, SerializeOptions, parse_body, serialize_body};

fn main() {
  let markdown = concat!(
    "---\ntitle: Demo\n---\n\n",
    "# Heading\n\n",
    "A paragraph with **bold**, *italic*, `code` and a [link](https://example.com).\n\n",
    "- list item\n- another item\n  - nested\n\n",
    "| A | B |\n| --- | --- |\n| 1 | 2 |\n\n",
    "> a quote\n\n",
    "```rust\nfn main() {}\n```\n"
  );

  let blocks = parse_body(markdown, ParseMode::Document);
  println!("parsed {} blocks:", blocks.len());
  for block in &blocks {
    println!("  {:<14} {:?}", block.kind, block.text.chars().take(30).collect::<String>());
  }

  let round = serialize_body(&blocks, &SerializeOptions::default());
  // The writer emits a normalized subset (this input's differences: nested
  // list indents, the quote's trailing bare `>`). Round-trip STABILITY is the
  // guarantee: a second pass is byte-identical to the first.
  let again = serialize_body(&parse_body(&round, ParseMode::Document), &SerializeOptions::default());
  println!("\nfirst pass identical to input: {}", round == markdown);
  println!("second pass identical to first: {}", round == again);
  println!("--- markdown out ---\n{round}");
}