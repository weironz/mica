// Pins the inline scanner as NON-quadratic on unclosed / unmatched delimiters
// (GitHub issue #61). The fix pre-scans and memoises "next occurrence"
// positions ([_ScanMemo] in lib/editor/marks.dart), so every closer lookup is
// O(1) after one linear build. This file is the Dart mirror of the Rust-side
// perf test for the same issue; style follows nested_bracket_perf_test.dart.
//
// Shapes guarded, each previously O(N) PER OPENER and O(N²) per parse:
//
//   1. `\(` with no closing `\)` — indexOf ran to the end at every opener.
//   2. `_a` repeats — every word boundary re-ran the GFM autolink/email scan
//      over the whole tail.
//   3. `$` with no valid closer — the Pandoc closer search walked the tail.
//   4. Unclosed `<` — the autolink/raw-HTML closer search ran to the end.
//   5. Unclosed backtick runs — _codeSpanClose rescanned the tail per run.
//   6. Many marks to render — the per-span min-over-all-marks pick in
//      _renderSpan was O(M) per span, O(M²) per export.
//
// Thresholds are deliberately loose (seconds against ~millisecond actuals) so
// they catch a quadratic return without flaking on a loaded CI box.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';

int parseMs(String src) {
  final sw = Stopwatch()..start();
  final parsed = parseInline(src);
  sw.stop();
  expect(parsed.text.isNotEmpty, isTrue, reason: 'sanity: it should parse');
  return sw.elapsedMilliseconds;
}

void main() {
  test('unclosed \\( openers are linear', () {
    final ms = parseMs(r'\(' * 20000);
    expect(
      ms < 3000,
      isTrue,
      reason:
          '20000 unclosed \\( took ${ms}ms — indexOf is rescans the tail '
          'per opener again (was O(N²), see issue #61)',
    );
  });

  test('repeated _a email/autolink probes are linear', () {
    // `_` is a GFM autolink word boundary, so every `a` is probed; there is
    // no `@` anywhere, and the old regex walked the tail at every boundary.
    for (final n in [5000, 20000]) {
      final ms = parseMs('_a' * n + '_');
      expect(
        ms < 3000,
        isTrue,
        reason:
            '${n}x _a took ${ms}ms — the extended-autolink scan is '
            'running to end-of-input per boundary again (issue #61)',
      );
    }
  });

  test('unclosed \$ math openers are linear', () {
    // Each `$x ` opens inline math by the Pandoc rules; the only later `$`
    // has a space before or a digit after it, so nothing closes. The old
    // closer search walked to end of input per opener.
    final ms = parseMs('\$x ' * 8000);
    expect(
      ms < 3000,
      isTrue,
      reason:
          '8000 unclosed \$ took ${ms}ms — _findMathCloser rescans the '
          'tail per opener again (issue #61)',
    );
  });

  test('unclosed < autolink openers are linear', () {
    // Every `<x ` starts an autolink attempt with no `>` anywhere; the old
    // indexOf scanned the tail for each.
    final ms = parseMs('<a ' * 8000);
    expect(
      ms < 3000,
      isTrue,
      reason:
          '8000 unclosed < took ${ms}ms — the > lookup rescans the tail '
          'per opener again (issue #61)',
    );
  });

  test('unclosed backtick runs are linear', () {
    // Three-backtick openers close on another run of exactly three; here the
    // interleaved single backticks never match, so each run scanned the tail.
    final ms = parseMs('```a`' * 8000);
    expect(
      ms < 3000,
      isTrue,
      reason:
          '8000 unclosed backtick runs took ${ms}ms — _codeSpanClose '
          'rescans the tail per run again (issue #61)',
    );
  });

  test('mathRunSpans over unclosed \\( is linear', () {
    // The HTML paste path runs mathRunSpans over the whole fetched document;
    // an unclosed `\(` used to cost a full tail scan at every opener.
    final src = r'\(' * 20000;
    final sw = Stopwatch()..start();
    final spans = mathRunSpans(src);
    final ms = (sw..stop()).elapsedMilliseconds;
    expect(spans, isEmpty, reason: 'sanity: nothing ever closes');
    expect(
      ms < 3000,
      isTrue,
      reason: 'mathRunSpans over 20000 unclosed \\( took ${ms}ms (issue #61)',
    );
  });

  test('inline export with many marks is not quadratic', () {
    // M adjacent same-kind marks: the per-span min-over-all-marks pick in
    // _renderSpan was O(M) per span. Merged marks (this is how the editor's
    // own mark lists look after normalization) must round-trip verbatim.
    final text = 'ab' * 8000;
    final marks = [
      for (var k = 0; k < 8000; k++) Mark(2 * k, 2 * k + 2, 'bold'),
    ];
    final sw = Stopwatch()..start();
    final rendered = inlineToMarkdown(text, marks);
    final ms = (sw..stop()).elapsedMilliseconds;
    expect(rendered, '**ab**' * 8000, reason: 'render output is unchanged');
    expect(
      ms < 3000,
      isTrue,
      reason:
          'rendering 8000 marks took ${ms}ms — the per-span pick is '
          'scanning all marks again (was O(M²), issue #61)',
    );
  });

  test('inline export with nested marks is not quadratic', () {
    // Deeply overlapping marks exercise the outermost-first recursion: each
    // level picks from the surviving marks. Exact-output equivalence against
    // the old naive per-span scan is differential-fuzzed in
    // render_span_equivalence_test.dart; here only speed is pinned.
    const depth = 300;
    final text = 'x' * (2 * depth);
    final marks = [
      for (var k = 0; k < depth; k++) Mark(k, 2 * depth - k, 'bold'),
    ];
    final sw = Stopwatch()..start();
    final rendered = inlineToMarkdown(text, marks);
    final ms = (sw..stop()).elapsedMilliseconds;
    expect(rendered.isNotEmpty, isTrue, reason: 'sanity: it renders');
    expect(
      ms < 3000,
      isTrue,
      reason: 'rendering $depth nested marks took ${ms}ms (issue #61)',
    );
  });

  test('InlineToHtml with many marks is not quadratic', () {
    final text = 'ab' * 8000;
    final marks = [
      for (var k = 0; k < 8000; k++) Mark(2 * k, 2 * k + 2, 'italic'),
    ];
    final sw = Stopwatch()..start();
    final rendered = inlineToHtml(text, marks);
    final ms = (sw..stop()).elapsedMilliseconds;
    expect(rendered, '<em>ab</em>' * 8000, reason: 'HTML output is unchanged');
    expect(
      ms < 3000,
      isTrue,
      reason: 'rendering 8000 marks to HTML took ${ms}ms (issue #61)',
    );
  });
}
