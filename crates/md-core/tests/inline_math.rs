//! `parse_inline_math` parity with Dart `parseInlineMath`
//! (clients/mica_flutter/lib/editor/marks.dart), including UTF-16 offsets.

use mica_md::{InlineText, parse_inline_math};

fn marked(text: &str) -> InlineText {
  parse_inline_math(text)
}

#[test]
fn a_lone_dollar_formula_becomes_one_marked_run() {
  let r = marked("$E = mc^2$");
  assert_eq!(r.text, "E = mc^2");
  assert_eq!(r.marks.len(), 1);
  assert_eq!(r.marks[0].kind, "math");
  assert_eq!((r.marks[0].start, r.marks[0].end), (0, r.text.len()));
}

#[test]
fn mid_prose_formula_keeps_its_position() {
  let r = marked("see $x^2$ here");
  assert_eq!(r.text, "see x^2 here");
  assert_eq!(r.marks.len(), 1);
  let m = &r.marks[0];
  assert_eq!(&r.text[m.start..m.end], "x^2");
}

#[test]
fn paren_form_is_recognized() {
  let r = marked(r"\(a+b\) and \(c\)");
  assert_eq!(r.text, "a+b and c");
  assert_eq!(r.marks.len(), 2);
  assert_eq!(r.marks[0].kind, "math");
}

#[test]
fn two_dollar_formulas_on_one_line() {
  let r = marked("$a$ and $b$");
  assert_eq!(r.text, "a and b");
  assert_eq!(r.marks.len(), 2);
}

#[test]
fn bare_dollars_prices_are_not_math() {
  let r = marked("price is $5 and $10");
  assert!(r.marks.is_empty());
  assert_eq!(r.text, "price is $5 and $10");
}

#[test]
fn a_display_dollar_marker_is_left_alone() {
  let r = marked("$$E=mc^2$$");
  assert!(r.marks.is_empty());
  assert_eq!(r.text, "$$E=mc^2$$");
}

#[test]
fn plain_text_passes_through_verbatim() {
  let r = marked("no formulas at all");
  assert!(r.marks.is_empty());
  assert_eq!(r.text, "no formulas at all");
}

#[test]
fn utf16_offsets_after_chinese_and_emoji() {
  // "中文" = 2 units, "😀" = 2 units, then the formula.
  let r = marked("中文😀 $x$");
  assert_eq!(r.text, "中文😀 x");
  let m = &r.marks[0];
  assert_eq!((m.start, m.end), (5, 6));
  // Slice by UTF-16 units (Rust byte indexing would reject this offset).
  let units: Vec<u16> = r.text.encode_utf16().collect();
  assert_eq!(String::from_utf16(&units[m.start..m.end]).unwrap(), "x");
  // Non-BMP inside the formula is measured in UTF-16 too.
  let r = marked("$a😀b$");
  assert_eq!(r.text, "a😀b");
  assert_eq!((r.marks[0].start, r.marks[0].end), (0, 4));
}

#[test]
fn empty_paren_form_is_not_a_mark() {
  let r = marked(r"\(\) x");
  assert!(r.marks.is_empty(), "empty formula must stay text: {r:?}");
}
