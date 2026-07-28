import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/highlight.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

// buildCodeSpan is a hand-written tokenizer the editor rebuilds for every code
// block on every keystroke (the Phase-1 painter cache always builds the span to
// compare it). It is memoized on (code, language, base, palette) — a complete
// key, since a code block carries no inline marks. These pin the memo: identical
// inputs reuse the instance (so no re-tokenize), any change misses, and the
// highlighted text is never corrupted by caching.

void main() {
  const base = TextStyle(fontSize: 14);

  test(
    'identical inputs return the very same cached span (no re-tokenize)',
    () {
      final a = buildCodeSpan(
        'int x = 1;\nreturn x;',
        'dart',
        base,
        MicaTokens.light.code,
      );
      final b = buildCodeSpan(
        'int x = 1;\nreturn x;',
        'dart',
        base,
        MicaTokens.light.code,
      );
      expect(
        identical(a, b),
        isTrue,
        reason: 'unchanged code must hit the memo, not tokenize again',
      );
    },
  );

  test('a change in code, language, or style misses the memo', () {
    final a = buildCodeSpan('int x = 1;', 'dart', base, MicaTokens.light.code);
    expect(
      identical(
        a,
        buildCodeSpan('int x = 2;', 'dart', base, MicaTokens.light.code),
      ),
      isFalse,
    );
    expect(
      identical(
        a,
        buildCodeSpan('int x = 1;', 'python', base, MicaTokens.light.code),
      ),
      isFalse,
    );
    expect(
      identical(
        a,
        buildCodeSpan(
          'int x = 1;',
          'dart',
          base.copyWith(fontSize: 16),
          MicaTokens.light.code,
        ),
      ),
      isFalse,
    );
  });

  test('the palette is part of the key — a theme switch must miss', () {
    // A span carries RESOLVED colours. Keyed without the palette, the first
    // code block tokenized in light mode would keep being served, in light
    // colours, on a dark page — and would go on doing that until its source
    // changed. Same defect shape as a content-hash cache that also cached the
    // 404: a key has to name everything the value depends on.
    const src = 'int x = 1;';
    final light = buildCodeSpan(src, 'dart', base, MicaTokens.light.code);
    final dark = buildCodeSpan(src, 'dart', base, MicaTokens.dark_.code);

    expect(identical(light, dark), isFalse, reason: 'the palette differs');

    Color? firstColour(TextSpan span) {
      if (span.style?.color != null) return span.style!.color;
      for (final child in span.children ?? const <InlineSpan>[]) {
        if (child is TextSpan) {
          final found = firstColour(child);
          if (found != null) return found;
        }
      }
      return null;
    }

    expect(
      firstColour(light),
      isNot(firstColour(dark)),
      reason: 'and the spans must actually be inked differently',
    );
  });

  test('memoization never corrupts the rendered text', () {
    const src = 'def f(a, b):\n    return a + b  # sum';
    final first = buildCodeSpan(src, 'python', base, MicaTokens.light.code);
    final second = buildCodeSpan(
      src,
      'python',
      base,
      MicaTokens.light.code,
    ); // served from memo
    expect(first.toPlainText(), src);
    expect(second.toPlainText(), src);
  });
}
