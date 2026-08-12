// Where a click opens a link, and where it just puts the caret down.
//
// Reported 2026-08-12: on a line that STARTS with a link there was no way to
// click into position 0 — every attempt opened the URL — so you could not put
// the caret there to press Enter and make room above it. The renderer snaps a
// click to the nearest caret boundary, and offset 0 is inside the mark by plain
// containment, so the left margin and the first glyph both counted as the link.

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';

Mark _link(int start, int end) =>
    Mark(start, end, 'link', href: 'https://example.test');

void main() {
  group('linkClickHit', () {
    test('the caret slot BEFORE the link is not a hit', () {
      // THE regression: a line beginning with a link, clicked at its very
      // start. This offset has to stay available or there is no way to type
      // above such a line.
      expect(linkClickHit(_link(0, 20), 0), isFalse);
    });

    test('inside the link is a hit', () {
      final m = _link(0, 20);
      for (final offset in [1, 5, 19]) {
        expect(linkClickHit(m, offset), isTrue, reason: 'offset $offset');
      }
    });

    test('the end boundary is not a hit — that slot is after the link', () {
      expect(linkClickHit(_link(0, 20), 20), isFalse);
      expect(linkClickHit(_link(0, 20), 21), isFalse);
    });

    test('a link in the MIDDLE of a line keeps the same rule', () {
      final m = _link(6, 12);
      expect(linkClickHit(m, 5), isFalse, reason: 'before it');
      expect(linkClickHit(m, 6), isFalse, reason: 'its own start slot');
      expect(linkClickHit(m, 7), isTrue);
      expect(linkClickHit(m, 11), isTrue);
      expect(linkClickHit(m, 12), isFalse, reason: 'its end slot');
    });

    test('a ONE-character link stays clickable', () {
      // The exception that keeps the rule honest: start is the only interior
      // slot such a link has, so excluding it would make it unopenable.
      final m = _link(3, 4);
      expect(linkClickHit(m, 3), isTrue);
      expect(linkClickHit(m, 4), isFalse);
    });

    test('non-link marks are never a hit', () {
      expect(linkClickHit(Mark(0, 5, 'bold'), 2), isFalse);
      expect(linkClickHit(Mark(0, 5, 'code'), 2), isFalse);
    });

    test('a link mark with no href is not clickable', () {
      // Marks are persisted data; a malformed one must not become a click that
      // opens nothing.
      expect(linkClickHit(Mark(0, 5, 'link'), 2), isFalse);
    });
  });
}
