//! I3/I4: table-cell encode/decode and inline serialize/parse invariants.
//!
//! I3 is checked over values the storage format can represent exactly. A
//! mark-free cell is stored as inline SOURCE and re-parsed on read, so the
//! inline parser's whitespace canonicalization and autolink detection are part
//! of the format; the lossy cases are pinned explicitly at the bottom instead
//! of being silently skipped.

use mica_md::{Block, InlineText, Mark, decode_cell, encode_cell, parse_inline, serialize_inline};
use proptest::prelude::*;

fn inline_from(text: &str, marks: Vec<Mark>) -> InlineText {
  InlineText {
    text: text.to_string(),
    marks,
  }
}

/// UTF-16 boundaries of every char boundary in `text`, including 0 and len.
fn utf16_boundaries(text: &str) -> Vec<usize> {
  let mut out = vec![0usize];
  let mut units = 0usize;
  for c in text.chars() {
    units += c.len_utf16();
    out.push(units);
  }
  out
}

fn marked(text: &str, ranges: Vec<(usize, usize)>, kinds: &[usize]) -> InlineText {
  let boundaries = utf16_boundaries(text);
  let chars = boundaries.len() - 1;
  let names = ["bold", "italic", "code", "strike", "link"];
  let mut marks = Vec::new();
  let mut cursor = 0usize; // char index, non-overlapping, at least one gap
  for (i, (a, b)) in ranges.into_iter().enumerate() {
    if cursor >= chars {
      break;
    }
    let lo = a.min(b).min(chars).max(cursor);
    let hi = a.max(b).min(chars);
    if lo >= hi {
      continue;
    }
    let kind = names[kinds.get(i).copied().unwrap_or(0) % names.len()];
    marks.push(Mark {
      start: boundaries[lo],
      end: boundaries[hi],
      kind: kind.to_string(),
      href: (kind == "link").then(|| "https://x.y".to_string()),
      title: None,
    });
    cursor = hi + 1; // one char gap keeps adjacent same-kind marks distinct
  }
  inline_from(text, marks)
}

/// Whether every mark covers a run the inline writer can delimit: no newline,
/// and no whitespace at either edge (emphasis/code delimiters would not form).
fn mark_slices_are_representable(text: &str, marks: &[Mark]) -> bool {
  let boundaries = utf16_boundaries(text);
  for mark in marks {
    let Some(lo) = boundaries.iter().position(|&x| x == mark.start) else {
      return false;
    };
    let Some(hi) = boundaries.iter().position(|&x| x == mark.end) else {
      return false;
    };
    let slice: String = text.chars().skip(lo).take(hi.saturating_sub(lo)).collect();
    if slice.contains('\n') {
      return false;
    }
    if slice.chars().next().is_none_or(|c| c.is_whitespace())
      || slice.chars().next_back().is_none_or(|c| c.is_whitespace())
    {
      return false;
    }
  }
  true
}

proptest! {
  #![proptest_config(ProptestConfig::with_cases(512))]

  /// I3: a MARKED cell goes through `{text, marks}` and must survive exactly.
  #[test]
  fn marked_cell_encode_decode_is_identity(
    text in "[a-z *_`~|\\\\\\[\\]()$\n中😀]{0,24}",
    ranges in prop::collection::vec((0usize..10, 0usize..10), 0..3),
    kinds in prop::collection::vec(0usize..5, 0..3),
  ) {
    let cell = marked(&text, ranges, &kinds);
    prop_assume!(!cell.marks.is_empty());
    prop_assert_eq!(decode_cell(&encode_cell(&cell)), cell);
  }

  /// I4: non-overlapping marks round-trip through inline Markdown.
  ///
  /// Excluded (existing inline-format limitations, not this refactor's choice):
  /// - edge whitespace / whitespace adjacent to a newline (canonicalized);
  /// - Markdown-significant punctuation inside a marked run — the writer does
  ///   not re-delimit emphasis when a mark covers only punctuation, e.g.
  ///   `**$**` after an alphanumeric is not left-flanking (fixed cases cover
  ///   escaping, this property covers the representable space);
  /// - bare URLs/emails in an unmarked run (the reader autolinks them).
  #[test]
  fn inline_serialize_parse_round_trips(
    text in "[a-z0-9中😀]{1,6}(\\n[a-z0-9中😀]{1,6}| [a-z0-9中😀]{1,6}){0,2}",
    ranges in prop::collection::vec((0usize..10, 0usize..10), 0..3),
    kinds in prop::collection::vec(0usize..5, 0..3),
  ) {
    prop_assume!(text.trim() == text);
    prop_assume!(!text.contains(" \n") && !text.contains("\n "));
    prop_assume!(!text.contains("://") && !text.contains("www.") && !text.contains('@'));
    let source = marked(&text, ranges, &kinds);
    prop_assume!(!source.text.is_empty());
    prop_assume!(mark_slices_are_representable(&source.text, &source.marks));
    let md = serialize_inline(&source.text, &source.marks);
    let parsed = parse_inline(&md);
    prop_assert_eq!(parsed.text, source.text);
    prop_assert_eq!(parsed.marks, source.marks);
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
fn fixed_inline_round_trip() {
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

/// Pins the CURRENT loss for a mark-free cell — the storage format is inline
/// source, so the reader re-parses it. The `{text, marks: []}` object would be
/// lossless in Rust's decoder, but the legacy Dart table path
/// (`TableData._cellText` → `parseInline`) re-parses an empty-marks object's
/// text too, so it is NOT a proven compatibility escape. These autolink and
/// whitespace cases are the approved I3 exceptions in
/// docs/single-source/02-architecture.md §2.2 (G1 decision, 2026-10-27).
#[test]
fn empty_marks_cells_document_the_current_loss() {
  let url = decode_cell(&encode_cell(&inline_from(
    "https://example.com",
    Vec::new(),
  )));
  assert_eq!(url.text, "https://example.com");
  assert_eq!(url.marks.len(), 1, "autolink detection adds a link mark");
  assert_eq!(url.marks[0].kind, "link");

  let trailing = decode_cell(&encode_cell(&inline_from("x ", Vec::new())));
  assert_eq!(trailing.text, "x", "trailing whitespace is canonicalized");
}

#[test]
fn blocks_are_the_public_migration_type() {
  let block = Block::new("b", "paragraph").with_text("x");
  assert_eq!(block.kind, "paragraph");
}
