import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/ui/trash_data.dart';

/// Deleting a folder used to list one row wearing a page icon, with nothing
/// saying it was a folder and nothing saying how much came back with it — while
/// restore and purge are both subtree-wide on the server.
void main() {
  DocumentView page(String id, {String? parent, String name = 'p'}) =>
      DocumentView(
        id: id,
        parentViewId: parent,
        objectId: 'obj-$id',
        objectType: 'document',
        name: name,
        position: id,
      );

  DocumentView folder(String id, {String? parent, String name = 'f'}) =>
      DocumentView(
        id: id,
        parentViewId: parent,
        objectId: 'obj-$id',
        objectType: 'folder',
        name: name,
        position: id,
      );

  group('roots only', () {
    test('a child whose parent is also deleted is not its own row', () {
      // Restoring the folder brings the page back with it, so a separate row for
      // the page would offer a restore that means nothing alone.
      final entries = buildTrashEntries(
        deleted: [
          folder('f1'),
          page('p1', parent: 'f1'),
        ],
        live: const [],
      );

      expect(entries.length, 1);
      expect(entries.single.view.id, 'f1');
    });

    test('a page deleted on its own is a row', () {
      final entries = buildTrashEntries(
        deleted: [page('p1', parent: 'liveFolder')],
        live: [folder('liveFolder', name: '产品思考')],
      );

      expect(entries.length, 1);
      expect(entries.single.view.id, 'p1');
    });

    test('server order is preserved, not re-sorted', () {
      final entries = buildTrashEntries(
        deleted: [page('c'), page('a'), page('b')],
        live: const [],
      );

      expect([for (final e in entries) e.view.id], ['c', 'a', 'b']);
    });
  });

  group('subtree counts', () {
    test('counts descendants by kind, excluding the root itself', () {
      // f1 / [p1, p2, f2 / [p3]]
      final entries = buildTrashEntries(
        deleted: [
          folder('f1'),
          page('p1', parent: 'f1'),
          page('p2', parent: 'f1'),
          folder('f2', parent: 'f1'),
          page('p3', parent: 'f2'),
        ],
        live: const [],
      );

      final e = entries.single;
      expect(e.pages, 3, reason: 'p1, p2, p3 — the root folder is not a page');
      expect(e.folders, 1, reason: 'f2 only — the root is not counted');
    });

    test('a lone page reports an empty subtree', () {
      final e = buildTrashEntries(deleted: [page('p1')], live: const []).single;

      expect(e.pages, 0);
      expect(e.folders, 0);
    });

    test('a parent cycle terminates instead of hanging', () {
      // Not reachable through the UI, but a bad row must not wedge the dialog.
      final entries = buildTrashEntries(
        deleted: [
          folder('a', parent: 'b'),
          folder('b', parent: 'a'),
        ],
        live: const [],
      );

      // Both parents are deleted, so neither is a root — the point of this test
      // is that it returns at all.
      expect(entries, isEmpty);
    });
  });

  group('restore path', () {
    test('names the live ancestor chain, outermost first', () {
      final entries = buildTrashEntries(
        deleted: [page('p1', parent: 'inner')],
        live: [
          folder('outer', name: '产品思考'),
          folder('inner', parent: 'outer', name: '留存'),
        ],
      );

      expect(entries.single.path, '产品思考 / 留存');
    });

    test('the workspace root has no path', () {
      final e = buildTrashEntries(deleted: [page('p1')], live: const []).single;

      expect(e.path, '');
    });

    test('an unresolvable ancestor ends the walk rather than guessing', () {
      // The server re-parents an orphaned root to the top level; inventing a path
      // we cannot see would be worse than saying nothing.
      final e = buildTrashEntries(
        deleted: [page('p1', parent: 'gone')],
        live: const [],
      ).single;

      expect(e.path, '');
    });

    test('a cycle among live views does not hang the walk', () {
      final e = buildTrashEntries(
        deleted: [page('p1', parent: 'a')],
        live: [
          folder('a', parent: 'b', name: 'A'),
          folder('b', parent: 'a', name: 'B'),
        ],
      ).single;

      expect(e.path.split(' / ').length, lessThanOrEqualTo(2));
    });
  });
}
