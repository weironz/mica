import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';

// Ctrl+F now opens an in-page find bar (was: global workspace search). These
// lock in the match enumerator behind it: case-insensitive, non-overlapping,
// across nodes in document order, with no range errors on short text.
void main() {
  group('findTextMatches', () {
    test('finds a single match with node/start/end', () {
      final m = findTextMatches(['hello world'], 'world');
      expect(m.length, 1);
      expect((m.single.node, m.single.start, m.single.end), (0, 6, 11));
    });

    test('is case-insensitive', () {
      final m = findTextMatches(['Hello WORLD'], 'world');
      expect(m.single.start, 6);
    });

    test('matches across multiple nodes in document order', () {
      final m = findTextMatches(['foo bar', 'baz', 'bar bar'], 'bar');
      expect(m.map((e) => (e.node, e.start)).toList(), [
        (0, 4),
        (2, 0),
        (2, 4),
      ]);
    });

    test('non-overlapping', () {
      final m = findTextMatches(['aaaa'], 'aa');
      expect(m.map((e) => e.start).toList(), [0, 2]);
    });

    test('empty query → no matches', () {
      expect(findTextMatches(['anything'], ''), isEmpty);
    });

    test('no match → empty', () {
      expect(findTextMatches(['abc', 'def'], 'xyz'), isEmpty);
    });

    test('query longer than text → empty (no range error)', () {
      expect(findTextMatches(['hi'], 'hello'), isEmpty);
    });

    test('match at the very end', () {
      final m = findTextMatches(['see the end'], 'end');
      expect((m.single.start, m.single.end), (8, 11));
    });

    test('CJK query matches, non-overlapping', () {
      final m = findTextMatches(['你好世界，世界'], '世界');
      expect(m.map((e) => e.start).toList(), [2, 5]);
    });
  });
}
