//! `parse_inline`: text plus marks. Mark offsets are UTF-16 code units over
//! the PARSED text (`inline.text`, delimiters already stripped) — the
//! coordinate system JS/Dart editors use. This prints each marked run and
//! verifies the offsets against `str::encode_utf16`, the conversion a Rust
//! embedder needs to slice the text by byte range.

use mica_md::parse_inline;

fn main() {
  let source = "run `kubectl` on 中文 text **now**";
  let inline = parse_inline(source);
  println!("source: {:?}", source);
  println!("text:   {:?}", inline.text);
  println!("marks:  {:?}\n", inline.marks);

  // UTF-16 unit → byte offset map over the CLEAN text (one entry per char
  // boundary, in order). Rust `str` slicing needs bytes; marks carry units.
  let text = &inline.text;
  let mut boundary: Vec<(usize, usize)> = vec![(0, 0)];
  let mut units = 0usize;
  for (byte, ch) in text.char_indices() {
    units += ch.len_utf16();
    boundary.push((units, byte + ch.len_utf8()));
  }
  let at = |unit: usize| {
    boundary
      .iter()
      .find(|(u, _)| *u == unit)
      .map(|(_, b)| *b)
      .unwrap_or(text.len())
  };

  for mark in &inline.marks {
    let start = at(mark.start);
    let end = at(mark.end);
    println!(
      "  {:<6} utf16 [{}..{}) = byte [{}..{}) = {:?}",
      mark.kind,
      mark.start,
      mark.end,
      start,
      end,
      &text[start..end]
    );
  }

  // The CJK text proves the unit/byte split matters: `中文` is 3 bytes per
  // char but 1 UTF-16 unit, so a byte-offset reader would slice mid-glyph.
  println!(
    "\ntext bytes {} vs utf16 units {} (they diverge on CJK/emoji)",
    text.len(),
    text.encode_utf16().count()
  );
}