// Pins inline parsing as NON-exponential in bracket nesting.
//
// CommonMark's "a link's text may not contain a link" rule needs a recursive
// re-parse of every bracket label. Two things keep that from exploding, and
// this file guards both:
//
// 1. LAZINESS — the check only gates the two link forms, so brackets matching
//    no link form never pay it. Guards `[[[[a]]]]` (was ~12s at depth 24).
// 2. A SHARED MEMO — without it, a label that DOES contain a link is parsed
//    twice per level (once for the check, once by the scan falling through the
//    rejected brackets): T(n) = 2*T(n-1). Guards `[[[a](/u)](/u)](/u)`.
//
// Both shapes are asserted because fixing only the first still left a real
// freeze vector: this runs on the editor's PASTE path.
//
// Thresholds are deliberately loose (seconds against a ~0-4ms actual) so they
// catch a return to exponential without flaking on a loaded CI box.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';

int parseMs(String src) {
  final sw = Stopwatch()..start();
  final parsed = parseInline(src);
  sw.stop();
  expect(parsed.text.isNotEmpty, isTrue, reason: 'sanity: it should still parse');
  return sw.elapsedMilliseconds;
}

void main() {
  test('plain bracket nesting is not exponential', () {
    const depth = 24;
    final ms = parseMs('${'[' * depth}a${']' * depth}');
    expect(ms < 3000, isTrue,
        reason: 'depth-$depth PLAIN nesting took ${ms}ms — the nested-link '
            'check is eager again (exponential even for non-links)');
  });

  // `[![[![…a](u)](u)](u)](u)`: the shared memo covered only the link CHECK;
  // an image label was parsed by `addLink` through a FRESH cache, so every
  // link check below an image re-ran from scratch — P(k) = 2·P(k-2). Mirrors
  // the Rust case in crates/markdown/tests/nested_bracket_perf.rs (P1-7).
  test('alternating link/image nesting is not exponential', () {
    for (final k in [20, 40, 60]) {
      final open = [for (var d = 0; d < k; d++) d.isEven ? '[' : '!['].join();
      final ms = parseMs('${open}a${'](u)' * k}');
      expect(ms < 3000, isTrue,
          reason: 'depth-$k ALTERNATING nesting took ${ms}ms — image labels '
              'are parsed outside the shared label cache again');
    }
  });

  // Each recognized label is a recursive call: thousands of nested images were
  // a stack overflow. Past `kMaxInlineNesting` the openers stay literal.
  test('deep image nesting stays within the nesting budget', () {
    for (final unit in ['![', '[']) {
      final parsed = parseInline('${unit * 5000}a${'](u)' * 5000}');
      expect(parsed.text, contains('a'));
      expect(
          parsed.marks.where((m) => m.type == 'image').length,
          lessThanOrEqualTo(kMaxInlineNesting));
    }
  });

  test('nested-link nesting is not exponential', () {
    const depth = 24;
    final ms = parseMs('${'[' * depth}a${'](/u)' * depth}');
    expect(ms < 3000, isTrue,
        reason: 'depth-$depth NESTED-LINK nesting took ${ms}ms — the '
            '_labelHasLinkCached memo is no longer shared across recursive '
            'parses, so each level is re-parsed twice again');
  });
}
