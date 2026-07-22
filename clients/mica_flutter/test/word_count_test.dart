import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/word_count.dart';

void main() {
  group('countBlocks', () {
    test('empty document counts nothing', () {
      expect(countBlocks(const []), DocCounts.zero);
      expect(countBlocks(const ['']), DocCounts.zero);
      expect(countBlocks(const ['   \n\t ']), DocCounts.zero);
    });

    test('plain English: whitespace-separated words', () {
      expect(countBlocks(const ['hello world']), const DocCounts(2, 10));
      // Leading/trailing/collapsed whitespace does not inflate the word count.
      expect(countBlocks(const ['  the   quick brown  ']),
          const DocCounts(3, 13));
    });

    test('punctuation counts as chars but breaks/does not add words', () {
      // "hello, world!" -> 2 words; chars = 12 (comma + bang counted, spaces not)
      expect(countBlocks(const ['hello, world!']), const DocCounts(2, 12));
      // A hyphen splits a Latin run into two words (whitespace-free but broken).
      expect(countBlocks(const ['re-run']), const DocCounts(2, 6));
    });

    test('CJK: each ideograph is one word and one char', () {
      expect(countBlocks(const ['你好世界']), const DocCounts(4, 4));
      // CJK punctuation counts as a char, not a word.
      expect(countBlocks(const ['你好，世界。']), const DocCounts(4, 6));
    });

    test('mixed CJK + Latin', () {
      // 3 CJK words + 1 Latin word ("Flutter"); chars = 3 + 7 = 10.
      expect(countBlocks(const ['中文和Flutter']), const DocCounts(4, 10));
    });

    test('accented Latin / Cyrillic count as word chars, not CJK', () {
      expect(countBlocks(const ['café résumé']), const DocCounts(2, 10));
      expect(countBlocks(const ['привет мир']), const DocCounts(2, 9));
    });

    test('block boundaries break words and newlines are not chars', () {
      // Two blocks, one word each; no char spans the boundary.
      expect(countBlocks(const ['foo', 'bar']), const DocCounts(2, 6));
      // A Latin run is closed at end-of-block even without trailing space.
      expect(countBlocks(const ['abc', '123']), const DocCounts(2, 6));
    });

    test('digits form words', () {
      expect(countBlocks(const ['version 12 beta']), const DocCounts(3, 13));
    });
  });
}
