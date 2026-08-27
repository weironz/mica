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
      final order = rootDropOrder(views, [views.last]);
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
      final order = rootDropOrder(views, [views[3]]);
      expect(order.map((e) => e.id), ['first', 'second', 'third', 'deep']);
    });

    test('a multi-selection lands at the end together, in its own order', () {
      // Dropping a Ctrl-selected batch on the blank area below the tree. Each
      // dragged row has to be excluded from the roots it is being appended to,
      // or the ones already at the root appear twice — once in place and once
      // at the end — and the reorder call then names one id twice.
      final views = [
        v('a', position: '01'),
        v('b', position: '02'),
        v('c', position: '03'),
        v('deep', parent: 'b', position: '01'),
      ];
      final order = rootDropOrder(views, [views[2], views[3]]);
      expect(order.map((e) => e.id), ['a', 'b', 'c', 'deep']);
      expect(
        order.map((e) => e.id).toSet().length,
        order.length,
        reason: 'c was already a root; it must be listed once',
      );
    });

    test('a row already at the root moves to the end, not duplicated', () {
      final views = [
        v('a', position: '01'),
        v('b', position: '02'),
        v('c', position: '03'),
      ];
      final order = rootDropOrder(views, [views.first]);
      expect(order.map((e) => e.id), ['b', 'c', 'a']);
      expect(order.length, views.length, reason: 'a is listed once, not twice');
    });

    test('the root indicator and a depth-0 row line up', () {
      // The point of indenting the indicator is that "inside the last folder"
      // and "at the root" stop looking identical. That only works if the root
      // zone's line sits exactly where a top-level row's would.
      // DocumentListItem lays a row out as:
      //   2 + depth*16   content start
      //   + 18           the always-present expand column
      //   = the icon's left edge
      // The dot has to land on that last number, because the icon edge is what
      // the eye compares levels by. Spelled out rather than asserted as a
      // literal, so this fails loudly if the row's own layout moves.
      double iconLeftEdge(int depth) =>
          2 + depth * kTreeIndentUnit + kTreeExpandColumnWidth;
      for (final depth in [0, 1, 2, 5]) {
        expect(
          dropIndicatorInset(depth),
          iconLeftEdge(depth),
          reason: 'the dot must sit on the icon edge at depth $depth',
        );
      }
      expect(
        dropIndicatorInset(1) - dropIndicatorInset(0),
        kTreeIndentUnit,
        reason: 'one step must equal the tree\'s per-depth indent',
      );
    });

    test('children of other parents stay out of the root list', () {
      final views = [
        v('a', position: '01'),
        v('x', parent: 'a', position: '01'),
        v('y', parent: 'a', position: '02'),
        v('moving', parent: 'a', position: '03'),
      ];
      final order = rootDropOrder(views, [views.last]);
      expect(order.map((e) => e.id), ['a', 'moving']);
    });
  });

  /// AFFiNE's tree lets you drag LEFT under the last row of a subtree to pop the
  /// drop out one level at a time — Atlassian pragmatic-drag-and-drop's
  /// `reparent`. This is the arithmetic behind the dot's position.
  group('reparentLevelFor', () {
    // Pointing AT where a level's dot is drawn must select that level. Written
    // as `dropIndicatorInset(n)` rather than as the arithmetic, because the two
    // have to move together: when the dot shifted right by the expand column
    // and the aiming origin did not, the level flipped a whole column before
    // the dot got there.
    test('under the last row of the tree, dragging left reaches the root', () {
      int at(double x) =>
          reparentLevelFor(pointerX: x, rowDepth: 3, nextRowDepth: null);
      expect(
        at(dropIndicatorInset(3)),
        3,
        reason: 'stay a sibling of the row above',
      );
      expect(at(dropIndicatorInset(2)), 2);
      expect(at(dropIndicatorInset(1)), 1);
      expect(
        at(dropIndicatorInset(0)),
        0,
        reason: 'all the way out is the workspace root',
      );
    });

    test('cannot pop out past the next row', () {
      // …because the gap being pointed at is between this row and that one.
      int at(double x) =>
          reparentLevelFor(pointerX: x, rowDepth: 3, nextRowDepth: 2);
      expect(at(dropIndicatorInset(3)), 3);
      expect(at(dropIndicatorInset(2)), 2);
      expect(
        at(dropIndicatorInset(0)),
        2,
        reason: 'clamped at the next row"s depth, not 0',
      );
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
      // Past the midpoint between two dots it takes the further one, so the
      // mark never lags the pointer by a whole indent.
      int at(double x) =>
          reparentLevelFor(pointerX: x, rowDepth: 3, nextRowDepth: null);
      expect(at(dropIndicatorInset(1) + 0.6 * kTreeIndentUnit), 2);
      expect(at(dropIndicatorInset(1) + 0.4 * kTreeIndentUnit), 1);
    });
  });

}
