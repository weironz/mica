//! I3/I4: table-cell encode/decode is an identity for representable values.

use mica_md::{Block, InlineText, Mark, decode_cell, encode_cell, parse_inline, serialize_inline};
use proptest::prelude::*;

fn inline_from(text: &str, marks: Vec<Mark>) -> InlineText {
  InlineText {
    text: text.to_string(),
    marks,
  }
}

proptest! {
  #![proptest_config(ProptestConfig::with_cases(512))]

  #[test]
  fn cell_encode_decode_is_identity_for_marked_text(
    text in "[a-z *_`~|\\\\\\[\\]()$\n中😀]{0,24}",
    ranges in prop::collection::vec((0usize..24, 0usize..24), 0..3),
  ) {
    // EXCLUSION: a mark-free cell is stored as inline SOURCE and re-parsed on
    // read, so the inline parser canonicalizes edge whitespace and whitespace
    // adjacent to a newline (existing behavior). Those values are not
    // representable in the storage format; everything else is covered exactly.
    prop_assume!(text.trim() == text);
    prop_assume!(!text.contains(" \n") && !text.contains("\n ") && !text.contains('\t'));
    // Non-overlapping, ascending ranges; the kind rotates through the
    // marks a cell can carry. `link` gets a real destination so serialization
    // never has to invent one.
    let mut marks = Vec::new();
    let mut cursor = 0usize;
    for (i, (a, b)) in ranges.into_iter().enumerate() {
      let (mut lo, mut hi) = (a.min(b), a.max(b));
      if hi <= lo { hi = (lo + 1).min(24); }
      if lo < cursor { lo = cursor; }
      if hi <= lo { continue; }
      cursor = hi;
      let kind = ["bold", "italic", "code", "strike", "link"][i % 5];
      marks.push(Mark {
        start: lo,
        end: hi,
        kind: kind.to_string(),
        href: (kind == "link").then(|| "https://x.y".to_string()),
        title: None,
      });
    }
    let cell = inline_from(&text, marks);
    let decoded = decode_cell(&encode_cell(&cell));
    prop_assert_eq!(decoded, cell);
  }
}

#[test]
fn plain_cells_escape_literal_markup() {
  for text in ["a *b* c", "a | b", "a \\ b", "", "😀", "line\nbreak"] {
    let cell = inline_from(text, Vec::new());
    let decoded = decode_cell(&encode_cell(&cell));
    assert_eq!(decoded.text, text, "plain cell text changed: {text:?}");
    assert!(
      decoded.marks.is_empty(),
      "plain cell gained marks: {text:?}"
    );
  }
}

#[test]
fn inline_serialize_parse_round_trips() {
  let text = "some bold and code and a link";
  let marks = vec![
    Mark {
      start: 5,
      end: 9,
      kind: "bold".into(),
      href: None,
      title: None,
    },
    Mark {
      start: 14,
      end: 18,
      kind: "code".into(),
      href: None,
      title: None,
    },
    Mark {
      start: 23,
      end: 27,
      kind: "link".into(),
      href: Some("https://x.y".into()),
      title: None,
    },
  ];
  let md = serialize_inline(text, &marks);
  let parsed = parse_inline(&md);
  assert_eq!(parsed.text, text);
  assert_eq!(parsed.marks, marks);
}

#[test]
fn blocks_are_the_public_migration_type() {
  let block = Block::new("b", "paragraph").with_text("x");
  assert_eq!(block.kind, "paragraph");
}
