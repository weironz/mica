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
}
