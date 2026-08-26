import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/main.dart';

/// Dropping on the sidebar tree's root zone.
///
/// The zone exists because every other drop target takes its parent from the
/// ROW it sits on, so with a nested row at the bottom of the tree there was no
/// way to say "below all of this, at the workspace root".
///
/// What is tested is the ORDER handed to `onReorderViews`, not the gesture:
/// dropping is visible the moment you do it, dropping into the wrong PLACE is a
/// silent reorder of somebody's sidebar.
DocumentView v(String id, {String? parent, required String position}) =>
    DocumentView(
      id: id,
      parentViewId: parent,
      objectId: 'obj-$id',
      objectType: 'document',
      name: id,
      position: position,
    );

void main() {
  group('rootDropOrder', () {
    test('a nested row lands at the END of the root list', () {
      final views = [
        v('a', position: '01'),
        v('b', position: '02'),
        v('deep', parent: 'b', position: '01'),
      ];
      final order = rootDropOrder(views, views.last);
      expect(order.map((e) => e.id), ['a', 'b', 'deep']);
    });

    test('root children keep POSITION order, not list order', () {
      // The caller hands over `widget.views`, which is not sorted.
      final views = [
        v('second', position: '02'),
        v('third', position: '03'),
        v('first', position: '01'),
        v('deep', parent: 'first', position: '01'),
      ];
      final order = rootDropOrder(views, views[3]);
      expect(order.map((e) => e.id), ['first', 'second', 'third', 'deep']);
    });

    test('a row already at the root moves to the end, not duplicated', () {
      final views = [
        v('a', position: '01'),
        v('b', position: '02'),
        v('c', position: '03'),
      ];
      final order = rootDropOrder(views, views.first);
      expect(order.map((e) => e.id), ['b', 'c', 'a']);
      expect(order.length, views.length, reason: 'a is listed once, not twice');
    });

    test('the root indicator and a depth-0 row line up', () {
      // The point of indenting the indicator is that "inside the last folder"
      // and "at the root" stop looking identical. That only works if the root
      // zone's line sits exactly where a top-level row's would.
      expect(dropIndicatorInset(0), 2);
      expect(dropIndicatorInset(1), 18);
      expect(dropIndicatorInset(2), 34);
      expect(
        dropIndicatorInset(1) - dropIndicatorInset(0),
        16,
        reason: 'one step must equal DocumentListItem\'s per-depth indent',
      );
    });

    test('children of other parents stay out of the root list', () {
      final views = [
        v('a', position: '01'),
        v('x', parent: 'a', position: '01'),
        v('y', parent: 'a', position: '02'),
        v('moving', parent: 'a', position: '03'),
      ];
      final order = rootDropOrder(views, views.last);
      expect(order.map((e) => e.id), ['a', 'moving']);
    });
  });

  /// AFFiNE's tree lets you drag LEFT under the last row of a subtree to pop the
  /// drop out one level at a time — Atlassian pragmatic-drag-and-drop's
  /// `reparent`. This is the arithmetic behind the dot's position.
  group('reparentLevelFor', () {
    test('under the last row of the tree, dragging left reaches the root', () {
      // A row at depth 3 with nothing after it: every level 0..3 is reachable.
      int at(double x) =>
          reparentLevelFor(pointerX: x, rowDepth: 3, nextRowDepth: null);
      expect(at(2 + 3 * 16), 3, reason: 'stay a sibling of the row above');
      expect(at(2 + 2 * 16), 2);
      expect(at(2 + 1 * 16), 1);
      expect(at(2), 0, reason: 'all the way out is the workspace root');
    });

    test('cannot pop out past the next row', () {
      // …because the gap being pointed at is between this row and that one.
      int at(double x) =>
          reparentLevelFor(pointerX: x, rowDepth: 3, nextRowDepth: 2);
      expect(at(2 + 3 * 16), 3);
      expect(at(2 + 2 * 16), 2);
      expect(at(2), 2, reason: 'clamped at the next row"s depth, not 0');
      expect(at(-200), 2, reason: 'dragging far left cannot escape the clamp');
    });

    test('an expanded folder offers no choice at all', () {
      // The next row is its own first child, so the gap is inside the folder
      // and there is exactly one level it can mean.
      for (final x in [-100.0, 0.0, 50.0, 500.0]) {
        expect(
          reparentLevelFor(pointerX: x, rowDepth: 1, nextRowDepth: 2),
          1,
          reason: 'x=$x must not move the level',
        );
      }
    });

    test('dragging right past the row"s own level does not nest deeper', () {
      // Going deeper is what the `into` zone is for; this gesture only pops out.
      expect(
        reparentLevelFor(pointerX: 999, rowDepth: 2, nextRowDepth: null),
        2,
      );
    });

    test('the level snaps to the nearest step, not the one it passed', () {
      // Half a step right of level 1 rounds up to 2, so the dot never lags the
      // pointer by a whole indent.
      expect(
        reparentLevelFor(pointerX: 2 + 1.6 * 16, rowDepth: 3, nextRowDepth: null),
        2,
      );
      expect(
        reparentLevelFor(pointerX: 2 + 1.4 * 16, rowDepth: 3, nextRowDepth: null),
        1,
      );
    });
  });

}
