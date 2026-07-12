// Sidebar tree opens COLLAPSED and remembers what the user expanded. A nested
// page is revealed by expanding its ancestor chain — ancestorIds is that walk.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

void main() {
  DocumentView v(String id, String? parent) => DocumentView(
        id: id,
        parentViewId: parent,
        objectId: 'o$id',
        objectType: 'document',
        name: id,
        position: '0000000010',
      );

  // root ─ a ─ b   (+ a sibling subtree x ─ y)
  final views = [
    v('root', null),
    v('a', 'root'),
    v('b', 'a'),
    v('x', null),
    v('y', 'x'),
  ];

  test('ancestorIds walks the whole parent chain', () {
    expect(ancestorIds(views, 'b'), {'a', 'root'});
    expect(ancestorIds(views, 'a'), {'root'});
    expect(ancestorIds(views, 'y'), {'x'});
  });

  test('a root node has no ancestors', () {
    expect(ancestorIds(views, 'root'), isEmpty);
    expect(ancestorIds(views, 'x'), isEmpty);
  });

  test('an unknown node has no ancestors', () {
    expect(ancestorIds(views, 'ghost'), isEmpty);
  });

  test('a corrupt parent cycle is safe (does not hang)', () {
    final cyclic = [v('A', 'B'), v('B', 'A')];
    expect(ancestorIds(cyclic, 'A'), {'B', 'A'});
  });
}
