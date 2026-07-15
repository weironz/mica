import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';

// The Dart mirror of crates/markdown/tests/math_code_span.rs. Rust is
// authoritative (CLAUDE.md #2), so these assert the SAME answers: a math opener
// must not scan through a code span, because CommonMark 0.31.2 §6.1 gives code
// spans higher precedence than every inline construct but HTML tags and
// autolinks — the `$` inside one is literal and cannot close an earlier `$`.
//
// Before the fix, `spent $5, config `$HOME` dir` produced a math run of
// "5, config `", eating the code span's own backtick from the middle.

List<String> mathRuns(String src) {
  final r = parseInline(src);
  return [
    for (final m in r.marks)
      if (m.type == 'math') r.text.substring(m.start, m.end),
  ];
}

List<String> codeRuns(String src) {
  final r = parseInline(src);
  return [
    for (final m in r.marks)
      if (m.type == 'code') r.text.substring(m.start, m.end),
  ];
}

void main() {
  test('math does not eat through a code span', () {
    const src = 'spent \$5, config `\$HOME` dir';
    expect(mathRuns(src), isEmpty);
    expect(codeRuns(src), ['\$HOME']);
  });

  test('code span content survives verbatim', () {
    const src = 'run `echo \$PATH` now';
    expect(parseInline(src).text, 'run echo \$PATH now');
    expect(codeRuns(src), ['echo \$PATH']);
    expect(mathRuns(src), isEmpty);
  });

  test('real math still parses', () {
    expect(mathRuns(r'coef $\eta = 2 \times \frac{N-1}{N}$ ok'), [
      r'\eta = 2 \times \frac{N-1}{N}',
    ]);
  });

  test('a closer after a code span still closes; backticks stay verbatim', () {
    // Stepping over a span is not dropping it. Math content is raw LaTeX and is
    // never re-parsed as markdown, so the backticks stay.
    expect(mathRuns('\$a `x` b\$ tail'), ['a `x` b']);
  });

  test('an unclosed backtick is literal and does not block math', () {
    // Per CommonMark an unclosed run is ordinary text — it must not make the
    // scan bail, or a stray backtick would cost us real math.
    expect(mathRuns('\$a ` b\$ tail'), ['a ` b']);
  });
}
