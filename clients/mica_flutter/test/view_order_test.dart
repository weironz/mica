import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/view_order.dart';

typedef _View = ({String id, String? parent});

List<String> _ordered(List<_View> views) => orderViewsParentFirst(
  views,
  idOf: (view) => view.id,
  parentOf: (view) => view.parent,
).map((view) => view.id).toList();

void main() {
  test('parents precede descendants even when the input is reversed', () {
    expect(
      _ordered([
        (id: 'leaf', parent: 'folder'),
        (id: 'folder', parent: 'root'),
        (id: 'root', parent: null),
      ]),
      ['root', 'folder', 'leaf'],
    );
  });

  test('root and sibling order stay stable and input stays unchanged', () {
    final views = <_View>[
      (id: 'b', parent: 'root'),
      (id: 'other', parent: null),
      (id: 'root', parent: null),
      (id: 'a', parent: 'root'),
    ];
    final before = List<_View>.of(views);
    expect(_ordered(views), ['other', 'root', 'b', 'a']);
    expect(views, before);
  });

  test('orphan roots and their children are retained after normal roots', () {
    expect(
      _ordered([
        (id: 'child', parent: 'orphan'),
        (id: 'orphan', parent: 'missing'),
        (id: 'root', parent: null),
      ]),
      ['root', 'orphan', 'child'],
    );
  });

  test('empty workspace produces an empty order', () {
    expect(_ordered([]), isEmpty);
  });
}
