// Guards against re-introducing EAGER nested-link detection in the Dart mirror.
//
// The CommonMark "a link's text may not contain a link" rule needs a recursive
// re-parse of every bracket label. Computing it up-front for EVERY '[' made even
// plain bracket nesting exponential (depth 24 ≈ 12s), and this runs on the
// editor's PASTE path — a UI freeze, not just a slow test. It is now computed
// lazily, so brackets matching no link form cost nothing.
//
// WHAT THIS TEST DOES **NOT** COVER — stated plainly so it is not mistaken for a
// clean bill of health: nested *links* (`[[[a](/u)](/u)](/u)`) are STILL
// exponential, because each level forces the check, gets rejected, and the span
// is re-parsed on the way back down. That residual is pre-existing and tracked
// in docs/code-review-2026-07-20.md; it is NOT fixed by the laziness pinned
// here. No assertion is made about that shape, because asserting it would mean
// encoding the bad behavior as "expected".
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';

void main() {
  test('plain bracket nesting is not exponential', () {
    const depth = 24;
    final src = '${'[' * depth}a${']' * depth}';
    final sw = Stopwatch()..start();
    final parsed = parseInline(src);
    sw.stop();
    expect(parsed.text.isNotEmpty, isTrue, reason: 'sanity: it still parses');
    expect(
      sw.elapsedMilliseconds < 3000,
      isTrue,
      reason: 'depth-$depth PLAIN nesting took ${sw.elapsedMilliseconds}ms — the '
          'nested-link check is eager again (exponential even for non-links); '
          'keep it lazy',
    );
  });
}
