import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/ui/home_data.dart';

// The home screen's content is derived, not fetched: the shell already holds
// every workspace's views and the server has always sent `views.updated_at`.
// These pin the derivation rules, including the ones that only bite on real data
// — a null timestamp (local world / offline mirror) and folders vs pages.

RelativeTimeStrings _strings = RelativeTimeStrings(
  justNow: 'just now',
  minutesAgo: (n) => '${n}m ago',
  hoursAgo: (n) => '${n}h ago',
  yesterday: 'yesterday',
  daysAgo: (n) => '${n}d ago',
);

DocumentView _view(
  String id,
  String name, {
  bool folder = false,
  String? parent,
  String? icon,
  DateTime? updated,
}) {
  return DocumentView(
    id: id,
    parentViewId: parent,
    objectId: 'obj-$id',
    objectType: folder ? 'folder' : 'document',
    name: name,
    position: 'a',
    icon: icon,
    updatedAt: updated,
  );
}

void main() {
  group('relativeMeta', () {
    final now = DateTime(2026, 7, 27, 12, 0);

    test('buckets by distance, and goes absolute past a week', () {
      String at(Duration ago) =>
          relativeMeta(now.subtract(ago), _strings, now: now);
      expect(at(const Duration(seconds: 20)), 'just now');
      expect(at(const Duration(minutes: 5)), '5m ago');
      expect(at(const Duration(hours: 3)), '3h ago');
      expect(at(const Duration(days: 1)), 'yesterday');
      expect(at(const Duration(days: 3)), '3d ago');
      // 37 days ago as "37d ago" tells nobody anything — show the date.
      expect(at(const Duration(days: 37)), '2026/6/20');
    });

    test('a null timestamp yields empty, never a fake "just now"', () {
      // The local world and the offline page-tree mirror build views with no
      // mtime; inventing one would be a lie the user cannot check.
      expect(relativeMeta(null, _strings, now: now), '');
    });

    test('a clock skew into the future reads as just now, not negative', () {
      expect(
        relativeMeta(now.add(const Duration(minutes: 5)), _strings, now: now),
        'just now',
      );
    });
  });

  group('buildRecents', () {
    final now = DateTime(2026, 7, 27, 12, 0);

    test('newest first, across workspaces, with the workspace name attached', () {
      final recents = buildRecents(
        viewsByWorkspace: {
          'w1': [
            _view('a', 'Older', updated: now.subtract(const Duration(days: 2))),
            _view(
              'b',
              'Newest',
              updated: now.subtract(const Duration(minutes: 5)),
            ),
          ],
          'w2': [
            _view(
              'c',
              'Middle',
              updated: now.subtract(const Duration(hours: 4)),
            ),
          ],
        },
        workspaceNames: {'w1': 'Mine', 'w2': 'Team'},
        strings: _strings,
        now: now,
      );
      expect(recents.map((e) => e.name), ['Newest', 'Middle', 'Older']);
      expect(recents.first.workspaceName, 'Mine');
      expect(recents[1].workspaceName, 'Team');
      expect(recents.first.meta, '5m ago');
    });

    test('folders are excluded — this is about documents', () {
      final recents = buildRecents(
        viewsByWorkspace: {
          'w1': [
            _view('f', 'A folder', folder: true, updated: now),
            _view(
              'p',
              'A page',
              updated: now.subtract(const Duration(hours: 1)),
            ),
          ],
        },
        workspaceNames: {'w1': 'Mine'},
        strings: _strings,
        now: now,
      );
      expect(recents.map((e) => e.name), ['A page']);
    });

    test('pages without a timestamp sort last but are NOT dropped', () {
      final recents = buildRecents(
        viewsByWorkspace: {
          'w1': [
            _view('n', 'No mtime'),
            _view('t', 'Timed', updated: now.subtract(const Duration(days: 3))),
          ],
        },
        workspaceNames: {'w1': 'Mine'},
        strings: _strings,
        now: now,
      );
      expect(recents.map((e) => e.name), ['Timed', 'No mtime']);
      expect(
        recents.last.meta,
        '',
        reason: 'no mtime → no meta, not a fake one',
      );
    });

    test('honours the limit and carries the emoji through', () {
      final recents = buildRecents(
        viewsByWorkspace: {
          'w1': [
            for (var i = 0; i < 10; i++)
              _view(
                'p$i',
                'Page $i',
                icon: i == 0 ? '📗' : null,
                updated: now.subtract(Duration(minutes: i)),
              ),
          ],
        },
        workspaceNames: {'w1': 'Mine'},
        strings: _strings,
        limit: 3,
        now: now,
      );
      expect(recents.length, 3);
      expect(recents.first.icon, '📗');
      expect(recents[1].icon, isNull);
    });

    test('an unknown workspace id degrades to an empty label, not a crash', () {
      final recents = buildRecents(
        viewsByWorkspace: {
          'ghost': [_view('a', 'Orphan', updated: now)],
        },
        workspaceNames: const {},
        strings: _strings,
        now: now,
      );
      expect(recents.single.workspaceName, '');
    });
  });

  group('buildDirectories', () {
    String count(int n) => '$n items';

    test('top-level folders only, alphabetical, with child counts', () {
      final dirs = buildDirectories(
        viewsByWorkspace: {
          'w1': [
            _view('z', 'Zeta', folder: true),
            _view('a', 'Alpha', folder: true),
            _view('nested', 'Nested', folder: true, parent: 'a'),
            _view('p1', 'page', parent: 'a'),
            _view('p2', 'page2', parent: 'a'),
          ],
        },
        workspaceNames: {'w1': 'Mine'},
        formatChildCount: count,
      );
      // Nested folder excluded — the sidebar tree already shows the hierarchy.
      expect(dirs.map((e) => e.name), ['Alpha', 'Zeta']);
      // Alpha holds 3 children (nested folder + two pages).
      expect(dirs.first.meta, '3 items');
      expect(dirs.last.meta, '0 items');
    });

    test('pages are never listed as directories', () {
      final dirs = buildDirectories(
        viewsByWorkspace: {
          'w1': [_view('p', 'Just a page')],
        },
        workspaceNames: {'w1': 'Mine'},
        formatChildCount: count,
      );
      expect(dirs, isEmpty);
    });
  });

  group('countPages', () {
    test('counts pages only — a switcher must not call folders pages', () {
      expect(
        countPages([
          _view('a', 'page'),
          _view('b', 'folder', folder: true),
          _view('c', 'page2'),
        ]),
        2,
      );
    });

    test('null or empty is zero', () {
      expect(countPages(null), 0);
      expect(countPages(const []), 0);
    });
  });
}
