import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/ui/overview_data.dart';

// The overview shows what is inside one container. Two rules carry weight: only
// DIRECT children (otherwise it duplicates the sidebar tree), and folders before
// pages (a container mixed alphabetically into its siblings reads as a flat pile).
// The trail walk also has to survive a cycle — the DB forbids one, but a stale
// client cache is not the DB, and a hung UI is worse than a short trail.

DocumentView _v(
  String id,
  String name, {
  bool folder = false,
  String? parent,
  String? icon,
}) => DocumentView(
  id: id,
  parentViewId: parent,
  objectId: 'obj-$id',
  objectType: folder ? 'folder' : 'document',
  name: name,
  position: 'a',
  icon: icon,
);

String _pageMeta(DocumentView v) => 'page:${v.name}';
String _folderMeta(int n) => '$n items';

void main() {
  final tree = [
    _v('root-page', 'Zebra page'),
    _v('f1', 'Beta folder', folder: true, icon: '📘'),
    _v('f2', 'Alpha folder', folder: true),
    _v('child1', 'Inside beta', parent: 'f1'),
    _v('child2', 'Also inside beta', parent: 'f1'),
    _v('deep', 'Deep folder', folder: true, parent: 'f1'),
    _v('deepest', 'Deep page', parent: 'deep'),
  ];

  group('buildOverviewItems', () {
    test('root: folders first (alphabetical), then pages', () {
      final items = buildOverviewItems(
        views: tree,
        parentViewId: null,
        formatPageMeta: _pageMeta,
        formatFolderMeta: _folderMeta,
      );
      expect(items.map((i) => i.name), [
        'Alpha folder',
        'Beta folder',
        'Zebra page',
      ]);
      expect(items.first.isFolder, isTrue);
      expect(items.last.isFolder, isFalse);
    });

    test('only DIRECT children — nested content is not flattened in', () {
      final items = buildOverviewItems(
        views: tree,
        parentViewId: 'f1',
        formatPageMeta: _pageMeta,
        formatFolderMeta: _folderMeta,
      );
      // 'deepest' lives under 'deep', so it must not appear here.
      expect(items.map((i) => i.name), [
        'Deep folder',
        'Also inside beta',
        'Inside beta',
      ]);
    });

    test('folder meta counts children, page meta uses the page formatter', () {
      final items = buildOverviewItems(
        views: tree,
        parentViewId: null,
        formatPageMeta: _pageMeta,
        formatFolderMeta: _folderMeta,
      );
      final beta = items.firstWhere((i) => i.name == 'Beta folder');
      expect(beta.meta, '3 items'); // two pages + one nested folder
      expect(beta.childCount, 3);
      final page = items.firstWhere((i) => i.name == 'Zebra page');
      expect(page.meta, 'page:Zebra page');
      expect(page.childCount, 0, reason: 'a page holds nothing');
    });

    test('carries the emoji, and an empty container yields an empty list', () {
      final items = buildOverviewItems(
        views: tree,
        parentViewId: null,
        formatPageMeta: _pageMeta,
        formatFolderMeta: _folderMeta,
      );
      expect(items.firstWhere((i) => i.name == 'Beta folder').icon, '📘');
      expect(items.firstWhere((i) => i.name == 'Alpha folder').icon, isNull);

      expect(
        buildOverviewItems(
          views: tree,
          parentViewId: 'f2',
          formatPageMeta: _pageMeta,
          formatFolderMeta: _folderMeta,
        ),
        isEmpty,
      );
    });
  });

  group('overviewTrail', () {
    test('root-first path down to the view', () {
      expect(overviewTrail(tree, 'deep').map((v) => v.name), [
        'Beta folder',
        'Deep folder',
      ]);
      expect(overviewTrail(tree, 'f1').map((v) => v.name), ['Beta folder']);
    });

    test('null or unknown id yields an empty trail, not a crash', () {
      expect(overviewTrail(tree, null), isEmpty);
      expect(overviewTrail(tree, 'ghost'), isEmpty);
    });

    test('a parent cycle terminates instead of hanging', () {
      // Only reachable from a corrupted/stale cache — but a spin here freezes the
      // whole app, so the walk must be defensive rather than trusting.
      final cyclic = [
        _v('a', 'A', folder: true, parent: 'b'),
        _v('b', 'B', folder: true, parent: 'a'),
      ];
      final trail = overviewTrail(cyclic, 'a');
      expect(trail.length, lessThanOrEqualTo(2));
      expect(trail.map((v) => v.name), contains('A'));
    });
  });
}
