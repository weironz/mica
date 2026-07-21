//! A link whose text is EXACTLY one code span must survive export.
//!
//! `render_span` picks the next mark by (start, widest) and the `code` branch
//! is terminal — it writes the literal span and never renders the marks nested
//! inside it. So when a code mark and a link mark covered the identical range,
//! whichever landed first in the mark list won, and ``[`a`](/x)`` exported as
//! `` `a` ``: the URL was silently gone. ``[`useState`](https://react.dev/…)``
//! is everywhere in technical writing, so this destroyed real content on any
//! export / copy-as-markdown round-trip.
//!
//! The fix makes terminal kinds (code/math/html/footnote) LOSE an exact-range
//! tie so the nestable mark wraps them. The autolink/bare-`www.` shorthands
//! write the plain text and discard inner marks, so they are now taken only
//! when there is nothing nested to lose.

fn round_trip(src: &str) -> String {
  let parsed = mica_markdown::import_markdown(src, "root");
  mica_markdown::export_markdown(&parsed).unwrap().trim().to_string()
}

#[test]
fn link_wrapping_a_whole_code_span_keeps_its_url() {
  assert_eq!(round_trip("[`a`](/x)\n"), "[`a`](/x)");
  assert_eq!(
    round_trip("[`useState`](https://react.dev/reference/react/useState)\n"),
    "[`useState`](https://react.dev/reference/react/useState)"
  );
}

#[test]
fn a_code_span_holding_a_bracket_still_keeps_its_url() {
  // The `]` lives inside the code span, so it must not close the link.
  assert_eq!(round_trip("[`a]b`](/x)\n"), "[`a]b`](/x)");
}

#[test]
fn partially_overlapping_code_and_link_were_never_broken() {
  // Guards the fix from over-reaching: these already worked.
  assert_eq!(round_trip("[a `b` c](/x)\n"), "[a `b` c](/x)");
  assert_eq!(round_trip("[`a` b](/x)\n"), "[`a` b](/x)");
  assert_eq!(round_trip("`a` and [b](/y)\n"), "`a` and [b](/y)");
}

#[test]
fn the_autolink_shorthand_still_applies_when_nothing_nests_inside() {
  // Guards the `inner.is_empty()` guard from over-reaching the other way:
  // a bare link with no inner marks must still write back in short form.
  assert_eq!(round_trip("<https://e.com/x>\n"), "<https://e.com/x>");
  assert_eq!(round_trip("www.e.com\n"), "www.e.com");
}

#[test]
fn a_code_formatted_autolink_keeps_both_marks() {
  // Text == href AND a code mark over it: the shorthand would drop the code,
  // so the bracketed form must win.
  assert_eq!(
    round_trip("[`https://e.com/x`](https://e.com/x)\n"),
    "[`https://e.com/x`](https://e.com/x)"
  );
}
