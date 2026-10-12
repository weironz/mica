import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

import 'support/title_workspace.dart';

void main() {
  testWidgets('large nested tree stays cheap with only active ancestors open', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1440, 900);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final views = [
      for (var i = 0; i < 588; i++)
        DocumentView(
          id: 'folder-$i',
          parentViewId: i < 53 ? null : 'folder-${i % 53}',
          objectId: 'folder-$i',
          objectType: 'folder',
          name: '目录 $i',
          position: i.toString().padLeft(5, '0'),
        ),
      for (var i = 0; i < 3080; i++)
        DocumentView(
          id: 'page-$i',
          parentViewId: 'folder-${53 + i % 535}',
          objectId: 'page-$i',
          objectType: 'document',
          name: '嵌套页面 $i',
          position: i.toString().padLeft(5, '0'),
        ),
    ];
    final watch = Stopwatch()..start();
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('嵌套页面 0', id: 'page-0'),
        (_, _) async => true,
        views: views,
      ),
    );
    await tester.pumpAndSettle();
    final initialMs = watch.elapsedMilliseconds;
    final delegate =
        tester.widget<SliverList>(find.byType(SliverList)).delegate
            as SliverChildBuilderDelegate;
    expect(delegate.childCount, lessThan(100));
    watch.reset();
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('嵌套页面 1', id: 'page-1'),
        (_, _) async => true,
        views: views,
      ),
    );
    await tester.pumpAndSettle();
    final navigationMs = watch.elapsedMilliseconds;
    watch.reset();
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('嵌套页面 1', id: 'page-1'),
        (_, _) async => true,
        views: views,
        message: '同步状态更新',
      ),
    );
    await tester.pumpAndSettle();
    // ignore: avoid_print
    print(
      'nested-tree initial_ms=$initialMs navigation_ms=$navigationMs '
      'status_ms=${watch.elapsedMilliseconds} visible_rows=${delegate.childCount}',
    );
    expect(find.byType(DocumentListItem).evaluate().length, lessThan(60));
    expect(tester.takeException(), isNull);
  });

  testWidgets('reordering keyed tree rows preserves each row state', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1440, 900);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    const a = DocumentView(
      id: 'a',
      parentViewId: null,
      objectId: 'a',
      objectType: 'document',
      name: '甲',
      position: '1',
    );
    const b = DocumentView(
      id: 'b',
      parentViewId: null,
      objectId: 'b',
      objectType: 'document',
      name: '乙',
      position: '2',
    );
    final bootstrap = titleBootstrap('甲', id: 'a');
    await tester.pumpWidget(
      titleWorkspace(bootstrap, (_, _) async => true, views: [a, b]),
    );
    await tester.pumpAndSettle();
    final row = find.byWidgetPredicate(
      (w) => w is DocumentListItem && w.view.id == 'b',
    );
    final state = tester.state(row);
    await tester.pumpWidget(
      titleWorkspace(
        bootstrap,
        (_, _) async => true,
        views: [
          a,
          const DocumentView(
            id: 'b',
            parentViewId: null,
            objectId: 'b',
            objectType: 'document',
            name: '乙',
            position: '0',
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.state(row), same(state));
    expect(
      tester.getTopLeft(row).dy,
      lessThan(
        tester
            .getTopLeft(
              find.byWidgetPredicate(
                (w) => w is DocumentListItem && w.view.id == 'a',
              ),
            )
            .dy,
      ),
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('large page tree builds only viewport rows on navigation', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1440, 900);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final views = [
      for (var i = 0; i < 3668; i++)
        DocumentView(
          id: 'page-$i',
          parentViewId: null,
          objectId: 'page-$i',
          objectType: 'document',
          name: '页面 $i',
          position: i.toString().padLeft(5, '0'),
        ),
    ];
    final watch = Stopwatch()..start();
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('页面 0', id: 'page-0'),
        (_, _) async => true,
        views: views,
        onSearch: (_) async => const [
          SearchResult(
            viewId: 'page-3000',
            objectId: 'page-3000',
            name: '页面 3000',
            snippet: '',
            titleMatch: true,
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();
    final initialMs = watch.elapsedMilliseconds;
    final sliver = tester.widget<SliverList>(find.byType(SliverList));
    final delegate = sliver.delegate;
    expect(delegate, isA<SliverChildBuilderDelegate>());
    expect((delegate as SliverChildBuilderDelegate).childCount, views.length);
    final mountedRows = find.byType(DocumentListItem).evaluate().length;
    expect(mountedRows, greaterThan(0));
    expect(mountedRows, lessThan(60));
    expect(delegate.findIndexByKey(const ValueKey('page-3000')), 3000);
    final wrappedRow = find.byWidgetPredicate(
      (w) => w is KeyedSubtree && w.key == const ValueKey('page-0'),
    );
    expect(tester.getSize(wrappedRow).height, 46);
    // ignore: avoid_print
    print(
      'large-tree initial_ms=$initialMs delegate=${delegate.runtimeType} '
      'mounted_rows=$mountedRows',
    );
    watch.reset();
    await tester.pumpWidget(
      titleWorkspace(
        titleBootstrap('页面 1', id: 'page-1'),
        (_, _) async => true,
        views: views,
        onSearch: (_) async => const [
          SearchResult(
            viewId: 'page-3000',
            objectId: 'page-3000',
            name: '页面 3000',
            snippet: '',
            titleMatch: true,
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();
    // ignore: avoid_print
    print('large-tree navigation_ms=${watch.elapsedMilliseconds}');
    expect(find.byType(DocumentListItem).evaluate().length, lessThan(60));
    final list = find.byType(CustomScrollView).first;
    final scroll = tester.widget<CustomScrollView>(list).controller!;
    await tester.tap(find.byIcon(Icons.search).first);
    await tester.pumpAndSettle();
    await tester.enterText(
      find.descendant(
        of: find.byType(Dialog),
        matching: find.byType(TextField),
      ),
      '3000',
    );
    await tester.pump(const Duration(milliseconds: 400));
    await tester.pumpAndSettle();
    await tester.tap(find.text('页面 3000'));
    await tester.pumpAndSettle();
    final located = find.byWidgetPredicate(
      (w) => w is DocumentListItem && w.view.id == 'page-3000',
    );
    expect(located, findsOneWidget);
    final rect = tester.getRect(list);
    final targetRect = tester.getRect(located);
    expect(targetRect.top, greaterThanOrEqualTo(rect.top));
    expect(targetRect.bottom, lessThanOrEqualTo(rect.bottom));
    scroll.jumpTo(scroll.position.maxScrollExtent);
    await tester.pumpAndSettle();
    expect(find.text('页面 3667'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
