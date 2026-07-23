import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/highlight.dart';

// buildCodeSpan is a hand-written tokenizer the editor rebuilds for every code
// block on every keystroke (the Phase-1 painter cache always builds the span to
// compare it). It is now memoized on (code, language, base) — a complete key,
// since a code block carries no inline marks. These pin the memo: identical
// inputs reuse the instance (so no re-tokenize), any change misses, and the
// highlighted text is never corrupted by caching.

void main() {
  const base = TextStyle(fontSize: 14);

  test('identical inputs return the very same cached span (no re-tokenize)', () {
    final a = buildCodeSpan('int x = 1;\nreturn x;', 'dart', base);
    final b = buildCodeSpan('int x = 1;\nreturn x;', 'dart', base);
    expect(identical(a, b), isTrue,
        reason: 'unchanged code must hit the memo, not tokenize again');
  });

  test('a change in code, language, or style misses the memo', () {
    final a = buildCodeSpan('int x = 1;', 'dart', base);
    expect(identical(a, buildCodeSpan('int x = 2;', 'dart', base)), isFalse);
    expect(identical(a, buildCodeSpan('int x = 1;', 'python', base)), isFalse);
    expect(
        identical(a, buildCodeSpan('int x = 1;', 'dart', base.copyWith(fontSize: 16))),
        isFalse);
  });

  test('memoization never corrupts the rendered text', () {
    const src = 'def f(a, b):\n    return a + b  # sum';
    final first = buildCodeSpan(src, 'python', base);
    final second = buildCodeSpan(src, 'python', base); // served from memo
    expect(first.toPlainText(), src);
    expect(second.toPlainText(), src);
  });
}
