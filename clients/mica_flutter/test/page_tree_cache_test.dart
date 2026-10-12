import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/ui/page_tree_cache.dart';

DocumentView view(
  String id, {
  String? parent,
  String position = '0',
  String? name,
  bool folder = false,
}) => DocumentView(
  id: id,
  parentViewId: parent,
  objectId: id,
  objectType: folder ? 'folder' : 'document',
  name: name ?? id,
  position: position,
);

void main() {
  test('sorted siblings, depth and collapsed descendants are preserved', () {
    final cache = PageTreeCache()
      ..updateViews([
        view('last', position: '9'),
        view('child', parent: 'folder'),
        view('grandchild', parent: 'child'),
        view('folder', position: '1', folder: true),
      ]);
    expect(cache.visibleRows({}).map((r) => r.view.id), ['folder', 'last']);
    final rows = cache.visibleRows({'folder', 'child'});
    expect(rows.map((r) => r.view.id), [
      'folder',
      'child',
      'grandchild',
      'last',
    ]);
    expect(rows.map((r) => r.depth), [0, 1, 2, 0]);
    expect(rows.map((r) => r.hasChildren), [true, true, false, false]);
    expect(cache.visibleIndexOf('grandchild'), 2);
  });

  test(
    'only genuine orphans surface at root; cycles and duplicate IDs stay safe',
    () {
      final cache = PageTreeCache()
        ..updateViews([
          view('orphan', parent: 'missing'),
          view('orphan-child', parent: 'orphan'),
          view('cycle-a', parent: 'cycle-b'),
          view('cycle-b', parent: 'cycle-a'),
          view('root'),
          view('root'),
        ]);
      expect(
        cache
            .visibleRows({'orphan', 'cycle-a', 'cycle-b'})
            .map((r) => r.view.id),
        ['root', 'orphan'],
      );
    },
  );

  test(
    'unchanged source and expansion reuse rows across body/status rebuilds',
    () {
      final views = [
        view('folder', folder: true),
        view('child', parent: 'folder'),
      ];
      final cache = PageTreeCache();
      expect(cache.updateViews(views), isTrue);
      final rows = cache.visibleRows({'folder'});
      expect(cache.updateViews(List.of(views)), isFalse);
      expect(identical(cache.visibleRows({'folder'}), rows), isTrue);
      expect(cache.viewById('child'), same(views.last));
    },
  );

  test('expansion set mutation invalidates the visible snapshot', () {
    final cache = PageTreeCache()
      ..updateViews([
        view('folder', folder: true),
        view('child', parent: 'folder'),
      ]);
    final expanded = <String>{};
    final closed = cache.visibleRows(expanded);
    expanded.add('folder');
    final open = cache.visibleRows(expanded);
    expect(open.length, 2);
    expect(identical(open, closed), isFalse);
    expanded.clear();
    expect(cache.visibleRows(expanded).length, 1);
  });

  test('in-place list add/remove and replacement cannot leave stale rows', () {
    final views = [view('a')];
    final cache = PageTreeCache()..updateViews(views);
    final oldRows = cache.visibleRows({});
    views.add(view('b'));
    expect(cache.updateViews(views), isTrue);
    expect(cache.visibleRows({}).length, 2);
    views[0] = view('a', name: 'renamed', position: '9');
    expect(cache.updateViews(views), isTrue);
    expect(cache.visibleRows({}).map((r) => r.view.name), ['b', 'renamed']);
    expect(oldRows.first.view.name, 'a');
    views.removeAt(1);
    cache.updateViews(views);
    expect(cache.viewById('b'), isNull);
    expect(cache.visibleIndexOf('b'), isNull);
  });

  test('same-count reparent and reorder invalidate indices', () {
    final a = view('a');
    final b = view('b', position: '1');
    final cache = PageTreeCache()..updateViews([a, b]);
    cache.visibleRows({});
    cache.updateViews([a, view('b', parent: 'a')]);
    expect(cache.visibleRows({}).map((r) => r.view.id), ['a']);
    expect(cache.visibleIndexOf('b'), isNull);
    expect(cache.visibleRows({'a'}).map((r) => r.depth), [0, 1]);
    cache.updateViews([a, view('b', position: '-1')]);
    expect(cache.visibleRows({}).map((r) => r.view.id), ['b', 'a']);
    expect(cache.visibleIndexOf('a'), 1);
  });

  test('cached rows are immutable and source removal resets to empty', () {
    final cache = PageTreeCache()..updateViews([view('a')]);
    final rows = cache.visibleRows({});
    expect(() => rows.clear(), throwsUnsupportedError);
    cache.updateViews([]);
    expect(cache.visibleRows({}), isEmpty);
    expect(cache.viewById('a'), isNull);
  });
}
