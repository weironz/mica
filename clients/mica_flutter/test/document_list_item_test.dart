// Sidebar row (P4-2 follow-up): the Feishu/Notion-style page row — actions are
// hidden until the row is hovered (so names keep the full width at rest), and a
// single `⋯`/right-click menu holds rename/delete/collapse instead of three
// always-on icons.
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';

DocumentView _view({String name = 'A long page name that would truncate'}) =>
    DocumentView(
      id: 'v1',
      parentViewId: null,
      objectId: 'o1',
      objectType: 'document',
      name: name,
      position: '0000000010',
    );

Widget _host(DocumentListItem item) =>
    MaterialApp(home: Scaffold(body: SizedBox(width: 280, child: item)));

void main() {
  testWidgets('actions are hidden until the row is hovered', (tester) async {
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onRename: () {},
      onDelete: () {},
    )));

    // At rest: no ⋯ / + eating the name's width.
    expect(find.byIcon(Icons.more_horiz), findsNothing);
    expect(find.byIcon(Icons.add), findsNothing);
    expect(find.text('A long page name that would truncate'), findsOneWidget);

    // Hover the row → the two compact affordances fade in.
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    addTearDown(mouse.removePointer);
    await mouse.moveTo(tester.getCenter(find.byType(DocumentListItem)));
    await tester.pumpAndSettle();

    expect(find.byIcon(Icons.more_horiz), findsOneWidget);
    expect(find.byIcon(Icons.add), findsOneWidget);
  });

  testWidgets('⋯ opens a menu with rename/delete; delete fires onDelete',
      (tester) async {
    var renamed = false;
    var deleted = false;
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onRename: () => renamed = true,
      onDelete: () => deleted = true,
    )));

    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    addTearDown(mouse.removePointer);
    await mouse.moveTo(tester.getCenter(find.byType(DocumentListItem)));
    await tester.pumpAndSettle();

    await tester.tap(find.byIcon(Icons.more_horiz));
    await tester.pumpAndSettle();
    expect(find.text('重命名'), findsOneWidget);
    expect(find.text('删除'), findsOneWidget);
    expect(find.text('新建子页面'), findsOneWidget);

    await tester.tap(find.text('删除'));
    await tester.pumpAndSettle();
    expect(deleted, isTrue);
    expect(renamed, isFalse);
  });

  testWidgets('right-click anywhere on the row opens the same menu',
      (tester) async {
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onRename: () {},
      onDelete: () {},
    )));

    // Secondary (right) tap — no hover needed.
    final center = tester.getCenter(find.byType(DocumentListItem));
    final gesture =
        await tester.startGesture(center, kind: PointerDeviceKind.mouse, buttons: kSecondaryButton);
    await gesture.up();
    await tester.pumpAndSettle();
    expect(find.text('重命名'), findsOneWidget);
    expect(find.text('删除'), findsOneWidget);
  });

  testWidgets('a parent row offers collapse/expand in its menu', (tester) async {
    var toggled = false;
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(),
      depth: 0,
      hasChildren: true,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      onToggle: () => toggled = true,
      onPressed: () {},
      onCreateChild: () {},
      onRename: () {},
      onDelete: () {},
    )));

    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    addTearDown(mouse.removePointer);
    await mouse.moveTo(tester.getCenter(find.byType(DocumentListItem)));
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(Icons.more_horiz));
    await tester.pumpAndSettle();

    expect(find.text('收起子页面'), findsOneWidget);
    await tester.tap(find.text('收起子页面'));
    await tester.pumpAndSettle();
    expect(toggled, isTrue);
  });

  testWidgets('read-only rows expose no actions or menu', (tester) async {
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: false,
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onRename: () {},
      onDelete: () {},
    )));

    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    addTearDown(mouse.removePointer);
    await mouse.moveTo(tester.getCenter(find.byType(DocumentListItem)));
    await tester.pumpAndSettle();
    // canEdit == false → no ⋯ / + even on hover.
    expect(find.byIcon(Icons.more_horiz), findsNothing);
    expect(find.byIcon(Icons.add), findsNothing);
  });
}
