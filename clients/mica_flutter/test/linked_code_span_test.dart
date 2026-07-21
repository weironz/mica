// A link whose text is EXACTLY one code span must survive export.
//
// `_renderSpan` picks the next mark by (start, widest) and the `code` branch is
// terminal — it writes the literal span and never renders the marks nested
// inside it. So when a code mark and a link mark covered the identical range,
// whichever came first in the mark list won, and ``[`a`](/x)`` exported as
// `` `a` ``: the URL was silently gone. ``[`useState`](https://react.dev/…)``
// is everywhere in technical writing, so this destroyed real content on any
// copy-as-markdown / export round-trip.
//
// The fix makes terminal kinds (code/math/html/footnote) LOSE an exact-range
// tie so the nestable mark wraps them. The autolink/bare-`www.` shorthands
// write the plain text and discard inner marks, so they are now taken only
// when there is nothing nested to lose.
//
// Both engines had the same bug; `crates/markdown/tests/linked_code_span.rs`
// is the Rust half of this pair and asserts the identical strings.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';

/// import → export, at the inline layer where the bug lived.
String roundTrip(String src) {
  final blocks = markdownToBlocks(src);
  expect(blocks.length, 1, reason: 'these cases are all one paragraph');
  return inlineToMarkdown(blocks.first.text, marksFromData(blocks.first.data));
}

void main() {
  test('a link wrapping a whole code span keeps its url', () {
    expect(roundTrip('[`a`](/x)'), '[`a`](/x)');
    expect(
      roundTrip('[`useState`](https://react.dev/reference/react/useState)'),
      '[`useState`](https://react.dev/reference/react/useState)',
    );
  });

  test('a code span holding a bracket still keeps its url', () {
    // The `]` lives inside the code span, so it must not close the link.
    expect(roundTrip('[`a]b`](/x)'), '[`a]b`](/x)');
  });

  test('partially overlapping code and link were never broken', () {
    // Guards the fix from over-reaching: these already worked.
    expect(roundTrip('[a `b` c](/x)'), '[a `b` c](/x)');
    expect(roundTrip('[`a` b](/x)'), '[`a` b](/x)');
    expect(roundTrip('`a` and [b](/y)'), '`a` and [b](/y)');
  });

  test('the autolink shorthand still applies when nothing nests inside', () {
    // Guards the `inner.isEmpty` guard from over-reaching the other way: a
    // bare link with no inner marks must still write back in short form.
    expect(roundTrip('<https://e.com/x>'), '<https://e.com/x>');
    expect(roundTrip('www.e.com'), 'www.e.com');
  });

  test('a code-formatted autolink keeps both marks', () {
    // Text == href AND a code mark over it: the shorthand would drop the code,
    // so the bracketed form must win.
    expect(
      roundTrip('[`https://e.com/x`](https://e.com/x)'),
      '[`https://e.com/x`](https://e.com/x)',
    );
  });
}
