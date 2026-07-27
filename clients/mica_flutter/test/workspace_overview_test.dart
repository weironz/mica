import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/workspace_overview.dart';

// The overview owns no data and no mode, so what is worth pinning is the wiring:
// every tap reaches the host with the right id, the mode toggle reports instead
// of switching itself (the host persists it), a missing emoji falls back to a
// glyph that still says folder-vs-page, and an empty workspace renders an answer
// rather than a blank pane.

const _strings = WorkspaceOverviewStrings(
  sectionLabel: '全部内容',
  cardsModeLabel: '卡片',
  listModeLabel: '目录',
  moreActionsLabel: '更多操作',
  emptyTitle: '这个工作区还是空的',
  emptyBody: '新建页面开始写作。',
  emptyActionLabel: '新建页面',
);

WorkspaceItem item({
  required String id,
  String? icon,
  String name = '产品思考',
  bool isFolder = false,
  String meta = '昨天更新',
  int childCount = 0,
}) {
  return (
    id: id,
    icon: icon,
    name: name,
    isFolder: isFolder,
    meta: meta,
    childCount: childCount,
  );
}

Widget host({
  required List<WorkspaceItem> items,
  WorkspaceOverviewMode mode = WorkspaceOverviewMode.cards,
  void Function(WorkspaceOverviewMode)? onModeChanged,
  void Function(String)? onOpen,
  void Function(String)? onMore,
  VoidCallback? onEmptyAction,
  double width = 900,
}) {
  return MaterialApp(
    home: Scaffold(
      body: Center(
        child: SizedBox(
          width: width,
          child: WorkspaceOverview(
            items: items,
            mode: mode,
            onModeChanged: onModeChanged ?? (_) {},
            onOpen: onOpen ?? (_) {},
            onMore: onMore,
            onEmptyAction: onEmptyAction,
            strings: _strings,
          ),
        ),
      ),
    ),
  );
}

void main() {
  testWidgets('card mode renders every item with its name and meta', (
    tester,
  ) async {
    await tester.pumpWidget(
      host(
        items: [
          item(id: 'a', icon: '📗', name: '产品思考', meta: '248 个页面'),
          item(id: 'b', icon: '📘', name: '基础设施', meta: '96 个页面'),
        ],
      ),
    );

    expect(find.byKey(const ValueKey('workspaceOverview.card.a')), findsOne);
    expect(find.byKey(const ValueKey('workspaceOverview.card.b')), findsOne);
    expect(find.text('产品思考'), findsOne);
    expect(find.text('248 个页面'), findsOne);
    expect(find.text('基础设施'), findsOne);
    expect(find.text('96 个页面'), findsOne);
    // No list rows while in card mode.
    expect(find.byKey(const ValueKey('workspaceOverview.row.a')), findsNothing);
  });

  testWidgets('list mode renders every item as a directory row', (
    tester,
  ) async {
    await tester.pumpWidget(
      host(
        mode: WorkspaceOverviewMode.list,
        items: [
          item(id: 'a', name: '部署', isFolder: true, childCount: 4),
          item(id: 'b', name: '服务器清单', meta: '2026-07-02'),
        ],
      ),
    );

    expect(find.byKey(const ValueKey('workspaceOverview.row.a')), findsOne);
    expect(find.byKey(const ValueKey('workspaceOverview.row.b')), findsOne);
    expect(find.text('部署'), findsOne);
    // A folder trails its child count; a page trails its formatted meta.
    expect(find.text('4'), findsOne);
    expect(find.text('2026-07-02'), findsOne);
    expect(find.byKey(const ValueKey('workspaceOverview.card.a')), findsNothing);
  });

  testWidgets('the toggle reports the mode instead of switching itself', (
    tester,
  ) async {
    final reported = <WorkspaceOverviewMode>[];
    await tester.pumpWidget(
      host(
        items: [item(id: 'a')],
        onModeChanged: reported.add,
      ),
    );

    await tester.tap(find.byKey(const ValueKey('workspaceOverview.mode.list')));
    await tester.pump();

    expect(reported, [WorkspaceOverviewMode.list]);
    // Still card mode: the host owns the value and has not pushed a new one.
    expect(find.byKey(const ValueKey('workspaceOverview.card.a')), findsOne);

    await tester.tap(
      find.byKey(const ValueKey('workspaceOverview.mode.cards')),
    );
    await tester.pump();
    // Fires even for the already-selected segment — a swallowed tap would hide
    // a failed preference write.
    expect(reported, [WorkspaceOverviewMode.list, WorkspaceOverviewMode.cards]);
  });

  testWidgets('tapping an item opens it by id, in both modes', (tester) async {
    final opened = <String>[];
    await tester.pumpWidget(
      host(
        items: [item(id: 'first'), item(id: 'second', name: '基础设施')],
        onOpen: opened.add,
      ),
    );

    await tester.tap(
      find.byKey(const ValueKey('workspaceOverview.card.second')),
    );
    expect(opened, ['second']);

    await tester.pumpWidget(
      host(
        mode: WorkspaceOverviewMode.list,
        items: [item(id: 'first'), item(id: 'second', name: '基础设施')],
        onOpen: opened.add,
      ),
    );
    await tester.tap(
      find.byKey(const ValueKey('workspaceOverview.row.first')),
    );
    expect(opened, ['second', 'first']);
  });

  testWidgets('the overflow affordance fires onMore, not onOpen', (
    tester,
  ) async {
    final opened = <String>[];
    final more = <String>[];
    await tester.pumpWidget(
      host(items: [item(id: 'a')], onOpen: opened.add, onMore: more.add),
    );

    await tester.tap(find.byKey(const ValueKey('workspaceOverview.more.a')));
    expect(more, ['a']);
    expect(opened, isEmpty);
  });

  testWidgets('no overflow affordance is drawn without an onMore host', (
    tester,
  ) async {
    await tester.pumpWidget(host(items: [item(id: 'a')]));
    expect(find.byKey(const ValueKey('workspaceOverview.more.a')), findsNothing);
  });

  testWidgets('a null icon falls back to a folder or page glyph, never emoji', (
    tester,
  ) async {
    await tester.pumpWidget(
      host(
        items: [
          item(id: 'folder', name: '部署', isFolder: true, childCount: 2),
          item(id: 'page', name: '服务器清单'),
        ],
      ),
    );

    Finder glyphIn(String id, IconData icon) => find.descendant(
      of: find.byKey(ValueKey('workspaceOverview.card.$id')),
      matching: find.byIcon(icon),
    );

    expect(glyphIn('folder', Icons.folder_outlined), findsOne);
    expect(glyphIn('page', Icons.description_outlined), findsOne);
    expect(glyphIn('folder', Icons.description_outlined), findsNothing);
    expect(glyphIn('page', Icons.folder_outlined), findsNothing);

    final glyph = tester.widget<Icon>(
      glyphIn('page', Icons.description_outlined),
    );
    expect(glyph.color, const Color(0xFF9AA4AF));
  });

  testWidgets('a user emoji wins over the fallback glyph', (tester) async {
    await tester.pumpWidget(
      host(items: [item(id: 'a', icon: '📗', isFolder: true)]),
    );
    expect(find.text('📗'), findsOne);
    expect(find.byIcon(Icons.folder_outlined), findsNothing);
  });

  testWidgets('an empty workspace states what happened and one next step', (
    tester,
  ) async {
    var actions = 0;
    await tester.pumpWidget(
      host(items: const [], onEmptyAction: () => actions++),
    );

    expect(find.byKey(const ValueKey('workspaceOverview.empty')), findsOne);
    expect(find.text('这个工作区还是空的'), findsOne);
    expect(find.text('新建页面开始写作。'), findsOne);

    await tester.tap(
      find.byKey(const ValueKey('workspaceOverview.emptyAction')),
    );
    expect(actions, 1);
  });

  testWidgets('the empty state stays copy-only without an action host', (
    tester,
  ) async {
    await tester.pumpWidget(host(items: const []));
    expect(find.byKey(const ValueKey('workspaceOverview.empty')), findsOne);
    expect(
      find.byKey(const ValueKey('workspaceOverview.emptyAction')),
      findsNothing,
    );
  });

  testWidgets('the card grid reflows 3 -> 2 -> 1 as the pane narrows', (
    tester,
  ) async {
    final items = [
      item(id: 'a', name: 'A'),
      item(id: 'b', name: 'B'),
      item(id: 'c', name: 'C'),
    ];
    double dy(String id) =>
        tester.getTopLeft(find.byKey(ValueKey('workspaceOverview.card.$id'))).dy;
    double cardWidth(String id) =>
        tester.getSize(find.byKey(ValueKey('workspaceOverview.card.$id'))).width;

    await tester.pumpWidget(host(items: items, width: 900));
    expect(dy('a'), dy('b'));
    expect(dy('a'), dy('c'));

    await tester.pumpWidget(host(items: items, width: 600));
    expect(dy('a'), dy('b'));
    expect(dy('c'), greaterThan(dy('a')));
    expect(cardWidth('a'), lessThan(400));

    await tester.pumpWidget(host(items: items, width: 380));
    expect(dy('b'), greaterThan(dy('a')));
    expect(dy('c'), greaterThan(dy('b')));
    expect(cardWidth('a'), 380);
  });
}
