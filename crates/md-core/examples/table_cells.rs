//! Table cells: the stored shape (source string or `{text, marks}` object)
//! versus the decoded model, via `decode_cell` / `encode_cell`. Shows the
//! escaping rule that keeps a literal `*` from re-reading as emphasis.

use mica_md::{decode_cell, encode_cell};

fn main() {
  // A marked cell stores the object shape.
  let marked = encode_cell(&mica_md::InlineText {
    text: "bold".to_string(),
    marks: vec![mica_md::Mark {
      start: 0,
      end: 4,
      kind: "bold".to_string(),
      href: None,
      title: None,
    }],
  });
  println!("marked cell stored as: {marked}");
  let back = decode_cell(&marked);
  println!("  decodes to: {:?}\n", back);

  // An unmarked cell with a literal `*` stores ESCAPED source — otherwise a
  // reader would parse `2 * 3 * 4` back into emphasis.
  let starred = encode_cell(&mica_md::InlineText {
    text: "2 * 3 * 4".to_string(),
    marks: Vec::new(),
  });
  println!("cell with a literal * stored as: {starred}");
  let back = decode_cell(&starred);
  println!("  decodes to: {:?} (no marks: {:?})", back.text, back.marks);

  // And the plain case: source in, source out.
  let plain = encode_cell(&mica_md::InlineText {
    text: "just text".to_string(),
    marks: Vec::new(),
  });
  println!("\nplain cell stored as: {plain}");
}