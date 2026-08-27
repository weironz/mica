// Sidebar row (P4-2 follow-up): the Feishu/Notion-style page row — actions are
// hidden until the row is hovered (so names keep the full width at rest), and a
// single `⋯`/right-click menu holds rename/delete/collapse instead of three
// always-on icons.
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/main.dart';
// main.dart imports this but does not re-export it; the multi-select tests
// below drive the row exactly as the tree does, which means folding each click
// through the real function rather than a stand-in.
import 'package:mica_flutter/ui/tree_selection.dart';

DocumentView _view({String name = 'A long page name that would truncate'}) =>
    DocumentView(
      id: 'v1',
      parentViewId: null,
      objectId: 'o1',
      objectType: 'document',
      name: name,
      position: '0000000010',
    );

// The row reads its tooltips/menu labels through `context.l10n`, which is
// `AppLocalizations.of(context)!` — without the delegates every test here dies
// on that null check, so the host must localize exactly like the real app.
Widget _host(DocumentListItem item) => MaterialApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      // Pinned: the menu assertions below name their labels in Chinese, so the
      // host must not drift to whatever locale the test binding defaults to.
      locale: const Locale('zh'),
      home: Scaffold(body: SizedBox(width: 280, child: item)),
    );

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
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
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
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () => renamed = true,
      onClone: () {},
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

  testWidgets('the row menu offers 创建副本 and fires onClone', (tester) async {
    var cloned = false;
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () => cloned = true,
      onDelete: () {},
    )));

    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    addTearDown(mouse.removePointer);
    await mouse.moveTo(tester.getCenter(find.byType(DocumentListItem)));
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(Icons.more_horiz));
    await tester.pumpAndSettle();

    expect(find.text('创建副本'), findsOneWidget);
    await tester.tap(find.text('创建副本'));
    await tester.pumpAndSettle();
    expect(cloned, isTrue);
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
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
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
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () => toggled = true,
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
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
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
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
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () => toggled = true,
      onPressed: () => opened = true, // navigate/open — must NOT fire for a folder
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
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
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () => childFolder = true,
      onRename: () {},
      onClone: () {},
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

  testWidgets('isRenaming shows an editable field seeded with the name; '
      'Enter commits', (tester) async {
    String? submitted;
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(name: 'Old name'),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      isRenaming: true,
      onRenameSubmit: (v) => submitted = v,
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
      onDelete: () {},
    )));
    await tester.pumpAndSettle();

    // The name is an editable field (not a static Text), seeded with the current
    // name so typing replaces it.
    expect(find.byType(TextField), findsOneWidget);
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller?.text,
      'Old name',
    );

    await tester.enterText(find.byType(TextField), 'New name');
    await tester.testTextInput.receiveAction(TextInputAction.done); // Enter
    await tester.pumpAndSettle();
    expect(submitted, 'New name');
  });

  testWidgets('the name barely moves when the rename field opens', (
    tester,
  ) async {
    // Entering rename swaps a Text for a TextField in the same slot, so any
    // contentPadding on the field shows up as the title visibly jumping right.
    // It used to jump 10px; the remaining ~4 is the outline border's own inset,
    // which reads as deliberate padding rather than a lurch. Pinned because the
    // regression is invisible in code review — you only see it on screen.
    Widget host(bool renaming) => _host(DocumentListItem(
          view: _view(name: '欢迎'),
          depth: 0,
          hasChildren: false,
          revealToggle: false,
          isCollapsed: false,
          isSelected: false,
          canEdit: true,
          isRenaming: renaming,
          onRenameSubmit: (_) {},
          onRenameCancel: () {},
          onToggle: () {},
          onPressed: () {},
          onCreateChild: () {},
          onCreateChildFolder: () {},
          onRename: () {},
          onClone: () {},
          onDelete: () {},
        ));

    await tester.pumpWidget(host(false));
    final restingX = tester.getTopLeft(find.text('欢迎')).dx;

    await tester.pumpWidget(host(true));
    await tester.pumpAndSettle();
    final editingX = tester.getTopLeft(find.byType(EditableText)).dx;

    expect(
      editingX - restingX,
      lessThanOrEqualTo(4.0),
      reason: 'the title must not lurch sideways when you start renaming',
    );
  });

  testWidgets('commit fires exactly once on blur (click-away)', (tester) async {
    var submits = 0;
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(name: 'X'),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      isRenaming: true,
      onRenameSubmit: (_) => submits++,
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
      onDelete: () {},
    )));
    await tester.pumpAndSettle();
    FocusManager.instance.primaryFocus?.unfocus(); // click-away
    await tester.pumpAndSettle();
    expect(submits, 1);
  });

  // Esc must back out of the inline field (the create-then-name flow) without
  // committing — otherwise the only exits are Enter and blur, and blur COMMITS,
  // so a mistyped new page name would have no way out.
  testWidgets('Esc cancels the inline field without committing',
      (tester) async {
    var cancels = 0;
    var submits = 0;
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(name: 'Untitled'),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      isRenaming: true,
      onRenameSubmit: (_) => submits++,
      onRenameCancel: () => cancels++,
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
      onDelete: () {},
    )));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'typed but unwanted');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(cancels, 1, reason: 'Esc should cancel');
    expect(submits, 0, reason: 'Esc must not commit');
  });

  // Pins the "rename is F2, never double-click" decision (docs/shortcuts.md).
  // Registering an onDoubleTap on the row would put a DoubleTapGestureRecognizer
  // in the arena, and it calls hold() on the first tap — so EVERY single click
  // (open a page, expand a folder: the sidebar's hot path) would stall for
  // kDoubleTapTimeout before doing anything. The other tap tests here settle,
  // which advances past that timeout and would NOT catch it; this one must not.
  testWidgets('a single click acts at once — no double-tap tax on the row',
      (tester) async {
    var opened = false;
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () => opened = true,
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
      onDelete: () {},
    )));

    await tester.tap(find.byType(DocumentListItem));
    await tester.pump(); // deliberately no settle: zero time advanced
    expect(
      opened,
      isTrue,
      reason: 'onTap must resolve on pointer-up, not after kDoubleTapTimeout',
    );
  });

  testWidgets('a folder row shows the quick create-folder icon on hover',
      (tester) async {
    await tester.pumpWidget(_host(DocumentListItem(
      view: folderView(),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
      onDelete: () {},
    )));
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    addTearDown(mouse.removePointer);
    await mouse.moveTo(tester.getCenter(find.byType(DocumentListItem)));
    await tester.pumpAndSettle();
    // A folder row has both quick-add affordances: child page + child folder.
    expect(find.byIcon(Icons.add), findsOneWidget);
    expect(find.byIcon(Icons.create_new_folder_outlined), findsOneWidget);
  });

  // ── Ctrl/Shift multi-selection ───────────────────────────────────────────
  //
  // What these cover is the ROW's half: that a modified click is reported as a
  // selection gesture instead of opening the page, and that a right-click inside
  // a selection swaps the menu.
  //
  // What they deliberately do NOT claim to cover is accumulation across clicks.
  // That was the bug this feature shipped with, it lived in the shell's State,
  // and a widget test driving one row can only exercise a stand-in for it — the
  // first attempt here did exactly that and passed with the bug reintroduced.
  // The fix was to move the accumulation into `TreeSelection`, where
  // tree_selection_test.dart tests it directly.

  testWidgets('Ctrl-click reports a toggle, not an open', (tester) async {
    var opened = false;
    var toggled = false;
    bool? sawExtend;
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(name: '甲'),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: true,
      isRenaming: false,
      onSelectClick: ({required extendRange}) {
        toggled = true;
        sawExtend = extendRange;
      },
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () => opened = true,
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
      onDelete: () {},
    )));

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.tap(find.text('甲'));
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pumpAndSettle();

    expect(toggled, isTrue);
    expect(sawExtend, isFalse, reason: 'Ctrl toggles one row, it is not a range');
    expect(opened, isFalse, reason: 'Ctrl-click must not open the page');
  });

  testWidgets('Shift-click asks for a range; a plain click asks for neither',
      (tester) async {
    var opened = false;
    var plainTaps = 0;
    bool? sawExtend;
    Widget row() => _host(DocumentListItem(
          view: _view(name: '甲'),
          depth: 0,
          hasChildren: false,
          revealToggle: false,
          isCollapsed: false,
          isSelected: false,
          canEdit: true,
          isRenaming: false,
          onSelectClick: ({required extendRange}) => sawExtend = extendRange,
          onPlainTap: () => plainTaps++,
          onRenameSubmit: (_) {},
          onRenameCancel: () {},
          onToggle: () {},
          onPressed: () => opened = true,
          onCreateChild: () {},
          onCreateChildFolder: () {},
          onRename: () {},
          onClone: () {},
          onDelete: () {},
        ));

    await tester.pumpWidget(row());
    await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
    await tester.tap(find.text('甲'));
    await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
    await tester.pumpAndSettle();
    expect(sawExtend, isTrue);
    expect(plainTaps, 0, reason: 'a modified click is not a plain one');

    // Unmodified: the row opens, and the tree is told to drop its selection.
    await tester.tap(find.text('甲'));
    await tester.pumpAndSettle();
    expect(opened, isTrue);
    expect(plainTaps, 1);
  });

  testWidgets('the row reports every Ctrl-click, so a selection can build up',
      (tester) async {
    // The row's contribution only: it must report each modified click rather
    // than swallowing the second one. Whether the RESULT accumulates is
    // TreeSelection's job and is tested there — see the note above.
    var selection = <String>{};
    Widget rowFor(String id) => _host(DocumentListItem(
          view: DocumentView(
            id: id,
            parentViewId: null,
            objectId: 'o$id',
            objectType: 'document',
            name: id,
            position: '01',
          ),
          depth: 0,
          hasChildren: false,
          revealToggle: false,
          isCollapsed: false,
          isSelected: false,
          isMultiSelected: selection.contains(id),
          canEdit: true,
          isRenaming: false,
          onSelectClick: ({required extendRange}) {
            selection = selectionAfterToggle(selection, id);
          },
          onRenameSubmit: (_) {},
          onRenameCancel: () {},
          onToggle: () {},
          onPressed: () {},
          onCreateChild: () {},
          onCreateChildFolder: () {},
          onRename: () {},
          onClone: () {},
          onDelete: () {},
        ));

    for (final id in ['甲', '乙']) {
      await tester.pumpWidget(rowFor(id));
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.tap(find.text(id));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();
    }

    expect(selection, {'甲', '乙'});
  });

  testWidgets('a multi-selected row swaps the whole menu for the batch one',
      (tester) async {
    var deleted = false;
    var batchRan = false;
    await tester.pumpWidget(_host(DocumentListItem(
      view: _view(name: '甲'),
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      isMultiSelected: true,
      canEdit: true,
      isRenaming: false,
      batchActions: () => [
        PopupMenuItem<VoidCallback>(
          value: () => batchRan = true,
          child: const Text('删除 3 项'),
        ),
      ],
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: () {},
      onPressed: () {},
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onRename: () {},
      onClone: () {},
      onDelete: () => deleted = true,
    )));

    await tester.tap(find.text('甲'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();

    expect(find.text('删除 3 项'), findsOneWidget);
    expect(
      find.text('重命名'),
      findsNothing,
      reason: 'the per-row entries are replaced, not appended to — otherwise '
          '"重命名" (this row) sits next to "删除 3 项" (all of them)',
    );

    await tester.tap(find.text('删除 3 项'));
    await tester.pumpAndSettle();
    expect(batchRan, isTrue);
    expect(deleted, isFalse, reason: 'the batch entry carries its own action');
  });
}
