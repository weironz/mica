// The breadcrumb and "copy path" describe the same path, and they used to
// describe it differently: the clipboard led with the workspace
// (`tools/AI工具/deepseek/reasonix`) while the breadcrumb started one level
// below it (`AI工具 › deepseek › reasonix`). The copy button sits immediately
// after the last crumb, so you were reading one path and about to copy another.
//
// Nothing failed while they disagreed — two renderers of the same idea, neither
// aware of the other, and only a human looking at both at once could notice. So
// the test that matters is not "the workspace appears" but "these two agree",
// asserted against one shared source (`pagePathSegments`).
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/main.dart';
import 'package:mica_flutter/ui/copy_button.dart';

DocumentView _view({
  required String id,
  required String name,
  String? parent,
  String type = 'document',
}) => DocumentView(
  id: id,
  parentViewId: parent,
  objectId: 'o_$id',
  objectType: type,
  name: name,
  position: '0000000010',
);

final _folder = _view(id: 'f1', name: '基础信息', type: 'folder');
final _page = _view(id: 'p1', name: 'BOI', parent: 'f1');
final _views = [_folder, _page];

Widget _host({required String? workspaceName, required DocumentView current}) =>
    MaterialApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      locale: const Locale('zh'),
      home: Scaffold(
        body: PageBreadcrumb(
          views: _views,
          current: current,
          workspaceName: workspaceName,
          onSelect: (_) async {},
          onCopyPath: () async => true,
          trailing: const SizedBox.shrink(),
        ),
      ),
    );

/// Every crumb label on screen, left to right.
///
/// Reads rendered text rather than the widget list, because "what you see" is
/// the whole point of the invariant — a segment hidden behind a null style or
/// an unbuilt branch would still be in the tree.
List<String> _rendered(WidgetTester tester) => tester
    .widgetList<Text>(find.byType(Text))
    .map((t) => t.data)
    .whereType<String>()
    .toList();

void main() {
  testWidgets('the breadcrumb reads exactly what copy path copies', (
    tester,
  ) async {
    await tester.pumpWidget(_host(workspaceName: 'greenstor', current: _page));

    final copied = pagePathSegments(
      workspaceName: 'greenstor',
      view: _page,
      views: _views,
    );

    expect(copied, ['greenstor', '基础信息', 'BOI']);
    // The assertion the fix exists for. Not `contains('greenstor')` — that
    // passes for a breadcrumb that shows the workspace in the wrong place, or
    // twice, or that has drifted from the clipboard in some other segment.
    expect(_rendered(tester), copied);
  });

  testWidgets('a page at the workspace root still agrees', (tester) async {
    final loose = _view(id: 'p2', name: '散页');
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: PageBreadcrumb(
            views: [loose],
            current: loose,
            workspaceName: 'greenstor',
            onSelect: (_) async {},
            onCopyPath: () async => true,
            trailing: const SizedBox.shrink(),
          ),
        ),
      ),
    );

    expect(
      pagePathSegments(
        workspaceName: 'greenstor',
        view: loose,
        views: [loose],
      ),
      ['greenstor', '散页'],
    );
    expect(_rendered(tester), ['greenstor', '散页']);
  });

  // Local (offline) workspaces reach the breadcrumb with no name to show. The
  // rule is "show nothing", not "show an empty first segment followed by a
  // chevron pointing at it".
  testWidgets('no workspace name means no leading segment', (tester) async {
    await tester.pumpWidget(_host(workspaceName: null, current: _page));
    expect(_rendered(tester), ['基础信息', 'BOI']);
  });

  testWidgets('a blank workspace name is treated as none', (tester) async {
    await tester.pumpWidget(_host(workspaceName: '   ', current: _page));
    expect(_rendered(tester), ['基础信息', 'BOI']);
  });

  // The copy button reports where the click was, instead of throwing a black
  // bar across the bottom of the window to say the expected thing happened.
  group('the copy button confirms itself', () {
    Future<void> pumpCopy(WidgetTester tester, Future<bool> Function() onCopy) =>
        tester.pumpWidget(
          MaterialApp(
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            locale: const Locale('zh'),
            home: Scaffold(
              body: InlineCopyButton(onCopy: onCopy, tooltip: 'copy'),
            ),
          ),
        );

    testWidgets('a successful copy turns the icon into a check', (tester) async {
      await pumpCopy(tester, () async => true);

      expect(find.byIcon(Icons.content_copy_outlined), findsOneWidget);
      await tester.tap(find.byType(IconButton));
      await tester.pump();

      expect(find.byIcon(Icons.check), findsOneWidget);
      expect(find.byIcon(Icons.content_copy_outlined), findsNothing);
    });

    /// It must go back on its own. A button stuck on "done" stops being a
    /// button you can press again — and the next copy would look like nothing
    /// happened.
    testWidgets('the check reverts by itself', (tester) async {
      await pumpCopy(tester, () async => true);
      await tester.tap(find.byType(IconButton));
      await tester.pump();
      expect(find.byIcon(Icons.check), findsOneWidget);

      await tester.pump(const Duration(seconds: 2));
      expect(find.byIcon(Icons.content_copy_outlined), findsOneWidget);
    });

    /// A refused clipboard must NOT look like a success. The check is the only
    /// signal the user gets here, so it has to mean exactly one thing.
    testWidgets('a failed copy leaves the icon alone', (tester) async {
      await pumpCopy(tester, () async => false);
      await tester.tap(find.byType(IconButton));
      await tester.pump();

      expect(find.byIcon(Icons.check), findsNothing);
      expect(find.byIcon(Icons.content_copy_outlined), findsOneWidget);
    });
  });

  // The search results list shows the same path beside each hit. That makes
  // THREE places describing one thing (breadcrumb, copy path, search row) —
  // which is how this went wrong the first time, so they share one walk and
  // this pins the relationship rather than each one's output separately.
  group('ancestorPathSegments is the same walk, minus the leaf', () {
    test('a hit trail is its page path with the page taken off', () {
      final full = pagePathSegments(
        workspaceName: 'greenstor',
        view: _page,
        views: _views,
      )!;
      final trail = ancestorPathSegments(
        workspaceName: 'greenstor',
        parentViewId: _page.parentViewId,
        views: _views,
        startedAt: _page.id,
      );

      expect(trail, full.sublist(0, full.length - 1));
      expect(trail, ['greenstor', '基础信息']);
    });

    test('a root-level hit still names the workspace', () {
      final loose = _view(id: 'p9', name: '散页');
      expect(
        ancestorPathSegments(
          workspaceName: 'greenstor',
          parentViewId: null,
          views: [loose],
          startedAt: loose.id,
        ),
        ['greenstor'],
      );
    });

    // Local (offline) mode has no workspace name and a root-level page has no
    // folders: the trail is empty and the row draws nothing.
    test('nothing to say yields empty, not [""]', () {
      expect(
        ancestorPathSegments(
          workspaceName: null,
          parentViewId: null,
          views: const [],
        ),
        isEmpty,
      );
      expect(
        ancestorPathSegments(
          workspaceName: '  ',
          parentViewId: null,
          views: const [],
        ),
        isEmpty,
      );
    });
  });

  group('pagePathSegments', () {
    test('no open page means no path, not a half one', () {
      expect(
        pagePathSegments(workspaceName: 'ws', view: null, views: _views),
        isNull,
      );
      expect(
        pagePathSegments(workspaceName: null, view: _page, views: _views),
        isNull,
      );
    });

    // A parent that is not in `views` (the tree is loaded lazily) must stop the
    // walk, not invent a segment or silently drop the workspace.
    test('an unloaded parent truncates instead of inventing a gap', () {
      final orphan = _view(id: 'p3', name: '孤儿', parent: 'missing');
      expect(
        pagePathSegments(
          workspaceName: 'greenstor',
          view: orphan,
          views: [orphan],
        ),
        ['greenstor', '孤儿'],
      );
    });

    // The tree forbids cycles; walking it should still not be the thing that
    // hangs if one ever appears.
    test('a cycle terminates', () {
      final a = _view(id: 'a', name: 'A', parent: 'b', type: 'folder');
      final b = _view(id: 'b', name: 'B', parent: 'a', type: 'folder');
      expect(
        pagePathSegments(workspaceName: 'ws', view: a, views: [a, b]),
        ['ws', 'B', 'A'],
      );
    });
  });
}
