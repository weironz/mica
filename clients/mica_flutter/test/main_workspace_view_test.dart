// Widget-level coverage for WorkspaceView (lib/main_workspace_view.dart, part
// of the main.dart library): the sidebar tree it renders from seeded views,
// the loaded-empty / still-loading states of that tree, a row tap reaching
// onSelectView, a page switch repainting the title and the row highlight, and
// the host message surfacing as a banner.

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';
import 'package:mica_flutter/ui/page_tree_state.dart';

import 'support/title_workspace.dart';

DocumentView _page(String id, String name, {String position = '10'}) =>
    DocumentView(
      id: id,
      parentViewId: null,
      objectId: id,
      objectType: 'document',
      name: name,
      position: position,
    );

Finder _rowOf(String viewId) => find.byWidgetPredicate(
  (w) => w is DocumentListItem && w.view.id == viewId,
);

void main() {
  void wideViewport(WidgetTester tester) {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1440, 900);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
  }

  final titleField = find.byWidgetPredicate(
    (w) => w is TextField && w.focusNode?.debugLabel == 'PageTitle',
  );

  testWidgets('renders the workspace tree with the seeded views', (
    tester,
  ) async {
    wideViewport(tester);
    final a = _page('page-a', '甲');
    final b = _page('page-b', '乙', position: '20');
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('甲', id: 'page-a'),
        (_, _) async => true,
        views: [a, b],
      ),
    );
    await tester.pumpAndSettle();

    expect(_rowOf('page-a'), findsOneWidget);
    expect(_rowOf('page-b'), findsOneWidget);
    expect(find.text('甲'), findsWidgets);
    expect(find.text('乙'), findsOneWidget);
    // The workspace switcher names the selected workspace.
    expect(find.text('测试'), findsWidgets);
    // A populated tree is neither loading nor empty.
    expect(find.byType(PageTreeSkeleton), findsNothing);
    expect(find.text('暂无页面'), findsNothing);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a loaded workspace with no pages shows the empty state', (
    tester,
  ) async {
    wideViewport(tester);
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('当前页面'),
        (_, _) async => true,
        views: const [],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('暂无页面'), findsOneWidget);
    expect(find.byType(PageTreeSkeleton), findsNothing);
    expect(_rowOf('page-a'), findsNothing);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('the empty state offers the way out: new page creates', (
    tester,
  ) async {
    wideViewport(tester);
    final created = <String>[];
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('当前页面'),
        (_, _) async => true,
        views: const [],
        onCreateDocument: (name) async {
          created.add(name);
          return null; // a failure leaves the widget as it was — fine here.
        },
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('新建页面'));
    await tester.pumpAndSettle();
    expect(created, ['未命名页面']);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a pending tree shows the skeleton, not the false empty state', (
    tester,
  ) async {
    wideViewport(tester);
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('当前页面'),
        (_, _) async => true,
        views: const [],
        treePending: true,
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byType(PageTreeSkeleton), findsOneWidget);
    expect(find.text('暂无页面'), findsNothing);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('tapping a page row opens it through onSelectView', (
    tester,
  ) async {
    wideViewport(tester);
    final a = _page('page-a', '甲');
    final b = _page('page-b', '乙', position: '20');
    final opened = <String>[];
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('甲', id: 'page-a'),
        (_, _) async => true,
        views: [a, b],
        onSelectView: (view) async => opened.add(view.id),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(_rowOf('page-b'));
    await tester.pumpAndSettle();
    expect(opened, ['page-b']);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('switching pages repaints the title and the row highlight', (
    tester,
  ) async {
    wideViewport(tester);
    final a = _page('page-a', '甲');
    final b = _page('page-b', '乙', position: '20');
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('甲', id: 'page-a'),
        (_, _) async => true,
        views: [a, b],
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.widget<TextField>(titleField).controller!.text, '甲');
    expect(tester.widget<DocumentListItem>(_rowOf('page-a')).isSelected, isTrue);
    expect(
      tester.widget<DocumentListItem>(_rowOf('page-b')).isSelected,
      isFalse,
    );

    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('乙', id: 'page-b'),
        (_, _) async => true,
        views: [a, b],
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.widget<TextField>(titleField).controller!.text, '乙');
    expect(tester.widget<DocumentListItem>(_rowOf('page-b')).isSelected, isTrue);
    expect(
      tester.widget<DocumentListItem>(_rowOf('page-a')).isSelected,
      isFalse,
    );
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('a host message surfaces as the sidebar banner', (tester) async {
    wideViewport(tester);
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('甲', id: 'page-a'),
        (_, _) async => true,
        message: '同步失败，请重试',
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('同步失败，请重试'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
}
