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
      onCreateChildFolder: () {},
      onRename: () {},
      onDelete: () {},
    )));

    // At rest: no ⋯ / + eating the name's width.
    expect(find.byIcon(Icons.more_horiz), findsNothing);
    expect(find.byIcon(Icons.add), findsNothing);
    expect(find.text('A long page name that would truncate'), findsOneWidget);

    // Hover the row → the menu affordance fades in. The `+` quick-add is
    // folder-only (a page is a leaf), so it stays absent on a document row.
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    addTearDown(mouse.removePointer);
    await mouse.moveTo(tester.getCenter(find.byType(DocumentListItem)));
    await tester.pumpAndSettle();

    expect(find.byIcon(Icons.more_horiz), findsOneWidget);
    expect(find.byIcon(Icons.add), findsNothing);
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
      onCreateChildFolder: () {},
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
    // A page is a leaf — no child-create entries on a document row.
    expect(find.text('新建子页面'), findsNothing);
    expect(find.text('新建子文件夹'), findsNothing);

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
      onCreateChildFolder: () {},
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
      onCreateChildFolder: () {},
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

    expect(find.text('收起子项'), findsOneWidget);
    await tester.tap(find.text('收起子项'));
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
      onCreateChildFolder: () {},
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

  // ── F4: folder rows ─────────────────────────────────────────────────────────

  DocumentView folderView() => const DocumentView(
        id: 'f1',
        parentViewId: null,
        objectId: 'o-folder',
        objectType: 'folder',
        name: 'Chapter',
        position: '0000000010',
      );

  testWidgets('a folder row shows a folder icon and clicking it expands (not open)',
      (tester) async {
    var toggled = false;
    var opened = false;
    await tester.pumpWidget(_host(DocumentListItem(
      view: folderView(),
      depth: 0,
      hasChildren: true,
      revealToggle: false,
      isCollapsed: true,
      isSelected: false,
      canEdit: true,
      onToggle: () => toggled = true,
      onPressed: () => opened = true, // navigate/open — must NOT fire for a folder
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onDelete: () {},
    )));

    // Folder icon, not the document icon.
    expect(find.byIcon(Icons.folder_outlined), findsOneWidget);
    expect(find.byIcon(Icons.description_outlined), findsNothing);

    // Clicking the row expands it in place instead of opening an editor.
    await tester.tap(find.byType(DocumentListItem));
    await tester.pumpAndSettle();
    expect(toggled, isTrue, reason: 'folder click toggles expand');
    expect(opened, isFalse, reason: 'folder click never opens an editor');
  });

  testWidgets('the row menu offers "新建子文件夹" and fires onCreateChildFolder',
      (tester) async {
    var childFolder = false;
    await tester.pumpWidget(_host(DocumentListItem(
      view: folderView(),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () => childFolder = true,
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

    expect(find.text('新建子页面'), findsOneWidget);
    expect(find.text('新建子文件夹'), findsOneWidget);
    await tester.tap(find.text('新建子文件夹'));
    await tester.pumpAndSettle();
    expect(childFolder, isTrue);
  });
}
