// The arithmetic behind "locate this page in the sidebar".
//
// It exists as a pure function because the first implementation was wrong in a
// way nothing caught: it asked `Scrollable.ensureVisible` for the row's
// BuildContext through a GlobalKey. The tree is a lazy `ListView`, so a row
// below the fold has no element and no context — the call did nothing for
// precisely the rows that needed scrolling, and worked only for ones already on
// screen. It shipped, and the user reported "看着没效果".

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart' show treeRevealOffset;

void main() {
  group('treeRevealOffset', () {
    // A tall tree: 400 rows of 38px in a 600px pane.
    const rowExtent = 38.0;
    const viewport = 600.0;
    const maxScroll = 400 * rowExtent - viewport;

    test('centres a row in the middle of a long tree', () {
      final offset = treeRevealOffset(
        index: 200,
        rowExtent: rowExtent,
        viewportHeight: viewport,
        maxScrollExtent: maxScroll,
      );
      // The row's own top, minus half a viewport, plus half a row.
      expect(
        offset,
        closeTo(200 * rowExtent - viewport / 2 + rowExtent / 2, 0.01),
      );
      // And it really is inside the pane afterwards.
      final rowTop = 200 * rowExtent - offset;
      expect(rowTop, greaterThan(0));
      expect(rowTop + rowExtent, lessThan(viewport));
    });

    test('the first rows clamp to the top instead of a negative offset', () {
      // Centring row 0 wants a negative scroll; a scrollable refuses that, and
      // an unclamped value would either throw or snap somewhere arbitrary.
      for (final index in [0, 1, 5]) {
        final offset = treeRevealOffset(
          index: index,
          rowExtent: rowExtent,
          viewportHeight: viewport,
          maxScrollExtent: maxScroll,
        );
        expect(offset, 0.0, reason: 'row $index is already visible at the top');
      }
    });

    test('the last rows clamp to the bottom', () {
      final offset = treeRevealOffset(
        index: 399,
        rowExtent: rowExtent,
        viewportHeight: viewport,
        maxScrollExtent: maxScroll,
      );
      expect(offset, maxScroll);
    });

    test('a tree shorter than the pane never scrolls', () {
      // maxScrollExtent <= min: there is nowhere to go, and asking for a
      // centred position would be asking to scroll a list that does not.
      final offset = treeRevealOffset(
        index: 3,
        rowExtent: rowExtent,
        viewportHeight: viewport,
        maxScrollExtent: 0,
      );
      expect(offset, 0.0);
    });

    test('respects a non-zero minimum scroll extent', () {
      final offset = treeRevealOffset(
        index: 0,
        rowExtent: rowExtent,
        viewportHeight: viewport,
        maxScrollExtent: 500,
        minScrollExtent: 120,
      );
      expect(offset, 120.0);
    });

    test('the result is always inside the scrollable range', () {
      for (var i = 0; i < 400; i++) {
        final offset = treeRevealOffset(
          index: i,
          rowExtent: rowExtent,
          viewportHeight: viewport,
          maxScrollExtent: maxScroll,
        );
        expect(offset, greaterThanOrEqualTo(0));
        expect(offset, lessThanOrEqualTo(maxScroll));
      }
    });
  });
}
