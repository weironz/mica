import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart' show SearchResult;
import 'package:mica_flutter/ui/search_data.dart';

/// Two things here are easy to get subtly wrong, and both fail *quietly*:
/// highlighting that is case-sensitive while the server matches with `ILIKE`
/// (real hits render with nothing marked, so the row reads as a false positive),
/// and treating the query as a regex (a search for `50%` or `(` either crashes or
/// matches the wrong thing).
void main() {
  // A hit now says WHAT it is and WHERE it lives, and the dialog sends folders
  // somewhere different (locate in the tree) than pages (open). The dangerous
  // direction is a server too old to send `is_folder`: read that silence as
  // "folder" and every real page gets routed to the handler that refuses to
  // open it.
  group('SearchResult.fromJson', () {
    Map<String, dynamic> hit(Map<String, dynamic> extra) => {
      'view_id': 'v1',
      'object_id': 'o1',
      'name': '部署资料',
      'snippet': '',
      'title_match': true,
      ...extra,
    };

    test('a missing is_folder means PAGE, never folder', () {
      final r = SearchResult.fromJson(hit({}));
      expect(r.isFolder, isFalse);
      expect(r.parentViewId, isNull);
    });

    test('reads a folder hit and its parent', () {
      final r = SearchResult.fromJson(
        hit({'is_folder': true, 'parent_view_id': 'p9'}),
      );
      expect(r.isFolder, isTrue);
      expect(r.parentViewId, 'p9');
    });

    test('a null parent is the workspace root, not an error', () {
      final r = SearchResult.fromJson(hit({'parent_view_id': null}));
      expect(r.parentViewId, isNull);
    });
  });

  group('highlightRuns', () {
    String joined(List<HighlightRun> runs) =>
        runs.map((r) => r.hit ? '[${r.text}]' : r.text).join();

    test('marks every occurrence, keeping the rest intact', () {
      expect(
        joined(highlightRuns('alpha beta alpha', 'alpha')),
        '[alpha] beta [alpha]',
      );
    });

    test('is case-insensitive, matching the server ILIKE', () {
      // The whole point: the server returned this row for the query, so something
      // in it must be highlighted.
      expect(
        joined(highlightRuns('Alpha and ALPHA', 'alpha')),
        '[Alpha] and [ALPHA]',
      );
    });

    test(
      'a highlighted run keeps the document casing, not the query casing',
      () {
        final runs = highlightRuns('Alpha', 'alpha');

        expect(runs.single.text, 'Alpha');
        expect(runs.single.hit, isTrue);
      },
    );

    test('CJK substrings match without a tokenizer', () {
      // Matches the server: CJK search is substring, no extension, no analyzer.
      expect(joined(highlightRuns('光纤及模块清单', '模块')), '光纤及[模块]清单');
    });

    test('the query is a literal, never a regex', () {
      // `.` would match any character; `%` and `(` would be a pattern or an error.
      expect(joined(highlightRuns('a.b and axb', 'a.b')), '[a.b] and axb');
      expect(
        joined(highlightRuns('discount 50% off', '50%')),
        'discount [50%] off',
      );
      expect(joined(highlightRuns('f(x) = 1', '(')), 'f[(]x) = 1');
    });

    test('no match leaves one unmarked run', () {
      final runs = highlightRuns('alpha', 'zzz');

      expect(runs.length, 1);
      expect(runs.single.hit, isFalse);
      expect(runs.single.text, 'alpha');
    });

    test('an empty or blank query marks nothing', () {
      expect(highlightRuns('alpha', '').single.hit, isFalse);
      expect(highlightRuns('alpha', '   ').single.hit, isFalse);
    });

    test('empty text yields no runs', () {
      expect(highlightRuns('', 'alpha'), isEmpty);
    });

    test('adjacent matches do not merge or drop text', () {
      expect(joined(highlightRuns('aaaa', 'aa')), '[aa][aa]');
    });

    test('every run concatenates back to the original text', () {
      const text = 'Alpha beta ALPHA gamma';
      final runs = highlightRuns(text, 'alpha');

      expect(runs.map((r) => r.text).join(), text);
    });
  });

  group('moveSelection', () {
    test('an empty list has nothing to select', () {
      expect(moveSelection(current: -1, count: 0, delta: 1), -1);
      expect(moveSelection(current: 3, count: 0, delta: -1), -1);
    });

    test('with nothing selected, down takes the first and up the last', () {
      expect(moveSelection(current: -1, count: 5, delta: 1), 0);
      expect(moveSelection(current: -1, count: 5, delta: -1), 4);
    });

    test('moves one row at a time', () {
      expect(moveSelection(current: 1, count: 5, delta: 1), 2);
      expect(moveSelection(current: 1, count: 5, delta: -1), 0);
    });

    test('wraps at both ends instead of sticking', () {
      // Stopping dead at the edge reads as a stuck key.
      expect(moveSelection(current: 4, count: 5, delta: 1), 0);
      expect(moveSelection(current: 0, count: 5, delta: -1), 4);
    });

    test('a single result stays selected in both directions', () {
      expect(moveSelection(current: 0, count: 1, delta: 1), 0);
      expect(moveSelection(current: 0, count: 1, delta: -1), 0);
    });

    test('an index left over from a longer list is brought back in range', () {
      // The query changed and the result list shrank; the old index must not
      // survive as an out-of-range selection.
      expect(moveSelection(current: 9, count: 3, delta: 1), lessThan(3));
      expect(moveSelection(current: 9, count: 3, delta: -1), lessThan(3));
    });
  });

  // Workspace names are matched here, in memory, and never reach the server:
  // there is no endpoint for it, and with fifty workspaces a round trip per
  // keystroke would be the slowest part of the fastest half of the panel.
  group('matchingWorkspaces', () {
    final all = [
      (id: 'w1', name: '推理引擎'),
      (id: 'w2', name: '大语言模型'),
      (id: 'w3', name: 'AI开发框架'),
      (id: 'w4', name: 'Infra Notes'),
    ];

    test('matches a substring anywhere in the name', () {
      expect(
        matchingWorkspaces(workspaces: all, query: '语言').map((w) => w.id),
        ['w2'],
      );
    });

    test('is case-insensitive, like the server-side ILIKE for pages', () {
      // The two halves of this panel must agree on case, or a query finds the
      // page and misses the workspace of the same name.
      expect(
        matchingWorkspaces(workspaces: all, query: 'infra').map((w) => w.id),
        ['w4'],
      );
      expect(
        matchingWorkspaces(workspaces: all, query: 'INFRA').map((w) => w.id),
        ['w4'],
      );
    });

    test('an empty or blank query matches NOTHING, not everything', () {
      // The panel shows recents before anything is typed; dumping every
      // workspace there would bury them.
      expect(matchingWorkspaces(workspaces: all, query: ''), isEmpty);
      expect(matchingWorkspaces(workspaces: all, query: '   '), isEmpty);
    });

    test('keeps the caller order rather than re-ranking', () {
      // That order is the user's own arrangement of their workspaces.
      final many = matchingWorkspaces(
        workspaces: [
          (id: 'a', name: 'zzz project'),
          (id: 'b', name: 'project'),
        ],
        query: 'project',
      );
      expect(many.map((w) => w.id), ['a', 'b'], reason: 'input order kept');
    });

    test('caps the number of hits', () {
      final many = [
        for (var i = 0; i < 12; i++) (id: 'w$i', name: 'project $i'),
      ];
      expect(matchingWorkspaces(workspaces: many, query: 'project').length, 5);
      expect(
        matchingWorkspaces(workspaces: many, query: 'project', limit: 2).length,
        2,
      );
    });

    test('a non-positive limit yields nothing rather than everything', () {
      expect(matchingWorkspaces(workspaces: all, query: 'a', limit: 0), isEmpty);
    });

    test('the query is a literal, not a pattern', () {
      // Same trap `highlightRuns` guards: a workspace called "50% done" must be
      // findable by typing "50%", and "(" must not blow up.
      final odd = [
        (id: 'p', name: '50% done'),
        (id: 'q', name: 'notes (draft)'),
      ];
      expect(
        matchingWorkspaces(workspaces: odd, query: '50%').map((w) => w.id),
        ['p'],
      );
      expect(
        matchingWorkspaces(workspaces: odd, query: '(dr').map((w) => w.id),
        ['q'],
      );
    });
  });
}
