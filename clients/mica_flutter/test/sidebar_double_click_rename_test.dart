import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/main.dart';
import 'package:mica_flutter/ui/navigation_draggable.dart';

import 'support/title_workspace.dart';

Future<void> mouseClick(
  WidgetTester tester,
  Finder row, {
  Offset? position,
}) async {
  // tester.tap stamps every event with zero, even after pump advances time.
  // Supply actual test-frame timestamps so time/slop assertions are meaningful.
  final click = await tester.createGesture(kind: PointerDeviceKind.mouse);
  final stamp = Duration(
    milliseconds: tester.binding.clock.now().millisecondsSinceEpoch,
  );
  await click.down(position ?? tester.getCenter(row), timeStamp: stamp);
  await click.up(timeStamp: stamp + const Duration(milliseconds: 1));
}

Future<void> doubleClick(WidgetTester tester, Finder row) async {
  await mouseClick(tester, row);
  await tester.pump(const Duration(milliseconds: 80));
  await mouseClick(tester, row);
  await tester.pumpAndSettle();
}

void main() {
  final current = titleBootstrap('当前页面');
  final target = titleBootstrap('待重命名页面', id: 'page-b');
  final row = find.byWidgetPredicate(
    (w) => w is DocumentListItem && w.view.id == target.view.id,
  );

  Future<void> workspace(
    WidgetTester tester,
    Future<bool> Function(DocumentView, String) rename,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1280, 900);
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(
      titleWorkspace(current, rename, views: [current.view, target.view]),
    );
    await tester.pumpAndSettle();
  }

  testWidgets('double click uses the real rename dialog for the clicked page', (
    tester,
  ) async {
    final saves = <(String, String)>[];
    await workspace(tester, (view, name) async {
      saves.add((view.id, name));
      return true;
    });
    await doubleClick(tester, row);
    expect(find.byType(AlertDialog), findsOneWidget);
    final field = find.descendant(
      of: find.byType(AlertDialog),
      matching: find.byType(TextField),
    );
    final controller = tester.widget<TextField>(field).controller!;
    expect(controller.text, target.view.name);
    expect(
      controller.selection,
      TextSelection(baseOffset: 0, extentOffset: target.view.name.length),
    );
    await tester.enterText(field, '  快速改名  ');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(saves, [(target.view.id, '快速改名')]);
    expect(find.byType(AlertDialog), findsNothing);
  });

  testWidgets('clicking another page breaks a double-click pair', (
    tester,
  ) async {
    await workspace(tester, (_, _) async => true);
    await mouseClick(tester, row);
    await tester.pump(const Duration(milliseconds: 80));
    final other = find.byWidgetPredicate(
      (w) => w is DocumentListItem && w.view.id == current.view.id,
    );
    await mouseClick(tester, other);
    await tester.pump(const Duration(milliseconds: 80));
    await mouseClick(tester, row);
    await tester.pumpAndSettle();
    expect(find.byType(AlertDialog), findsNothing);
  });

  for (final action in ['cancel', 'escape', 'blank', 'unchanged']) {
    testWidgets('double click rename $action does not save', (tester) async {
      var saves = 0;
      await workspace(tester, (_, _) async {
        saves++;
        return true;
      });
      await doubleClick(tester, row);
      expect(find.byType(AlertDialog), findsOneWidget);
      if (action == 'cancel') {
        await tester.tap(find.text('取消'));
      } else if (action == 'escape') {
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      } else {
        if (action == 'blank') {
          await tester.enterText(
            find.descendant(
              of: find.byType(AlertDialog),
              matching: find.byType(TextField),
            ),
            '   ',
          );
        }
        await tester.tap(find.text('保存'));
      }
      await tester.pumpAndSettle();
      expect(saves, 0);
      expect(find.byType(AlertDialog), findsNothing);
    });
  }

  Future<void> host(
    WidgetTester tester, {
    bool canEdit = true,
    bool folder = false,
    bool draggable = false,
    required VoidCallback rename,
    required VoidCallback open,
    VoidCallback? select,
  }) async {
    final item = DocumentListItem(
      view: folder
          ? DocumentView.fromJson({
              'id': 'folder',
              'object_id': 'folder',
              'object_type': 'folder',
              'name': '文件夹',
              'position': '0',
            })
          : target.view,
      depth: 0,
      hasChildren: false,
      revealToggle: false,
      isCollapsed: false,
      isSelected: false,
      canEdit: canEdit,
      isRenaming: false,
      onRenameSubmit: (_) {},
      onRenameCancel: () {},
      onToggle: open,
      onPressed: open,
      onCreateChild: () {},
      onCreateChildFolder: () {},
      onClone: () {},
      onRename: rename,
      onDelete: () {},
      onSelectClick: select == null
          ? null
          : ({required extendRange}) => select(),
    );
    await tester.pumpWidget(
      MaterialApp(
        locale: const Locale('zh'),
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: SizedBox(
            width: 280,
            child: draggable
                ? NavigationDraggable<String>(
                    data: 'row',
                    feedback: const Text('dragging'),
                    child: item,
                  )
                : item,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
  }

  for (final key in [
    LogicalKeyboardKey.controlLeft,
    LogicalKeyboardKey.shiftLeft,
    LogicalKeyboardKey.metaLeft,
  ]) {
    testWidgets('$key rapid clicks both select, never rename', (tester) async {
      var selections = 0;
      var renames = 0;
      var opens = 0;
      await host(
        tester,
        rename: () => renames++,
        open: () => opens++,
        select: () => selections++,
      );
      await tester.sendKeyDownEvent(key);
      await doubleClick(tester, find.byType(DocumentListItem));
      await tester.sendKeyUpEvent(key);
      expect(selections, 2);
      expect(renames, 0);
      expect(opens, 0);
      // Releasing a modifier must not pair with its preceding click.
      await mouseClick(tester, find.byType(DocumentListItem));
      expect(opens, 1);
      expect(renames, 0);
    });
  }

  for (final mode in ['viewer', 'folder', 'touch']) {
    testWidgets('$mode rapid clicks retain their ordinary action', (
      tester,
    ) async {
      var opens = 0;
      var renames = 0;
      await host(
        tester,
        canEdit: mode != 'viewer',
        folder: mode == 'folder',
        rename: () => renames++,
        open: () => opens++,
      );
      for (var i = 0; i < 2; i++) {
        if (mode == 'touch') {
          await tester.tap(find.byType(DocumentListItem));
        } else {
          await mouseClick(tester, find.byType(DocumentListItem));
        }
        await tester.pump(const Duration(milliseconds: 80));
      }
      expect(opens, 2);
      expect(renames, 0);
    });
  }

  testWidgets('slow or distant clicks do not form a rename pair', (
    tester,
  ) async {
    var renames = 0;
    var opens = 0;
    await host(tester, rename: () => renames++, open: () => opens++);
    final row = find.byType(DocumentListItem);
    await mouseClick(tester, row);
    await tester.pump(kDoubleTapTimeout + const Duration(milliseconds: 1));
    await mouseClick(tester, row);
    await tester.pump(const Duration(milliseconds: 80));
    await mouseClick(
      tester,
      row,
      position: tester.getTopLeft(row) + const Offset(5, 15),
    );
    expect(opens, 3);
    expect(renames, 0);
  });

  testWidgets('drag cancellation clears the preceding click pair', (
    tester,
  ) async {
    var renames = 0;
    await host(tester, draggable: true, rename: () => renames++, open: () {});
    final row = find.byType(DocumentListItem);
    await mouseClick(tester, row);
    await tester.pump(const Duration(milliseconds: 80));
    final drag = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await drag.down(
      tester.getCenter(row),
      timeStamp: Duration(
        milliseconds: tester.binding.clock.now().millisecondsSinceEpoch,
      ),
    );
    await drag.moveBy(const Offset(0, 40));
    await tester.pump();
    expect(find.text('dragging'), findsOneWidget);
    await drag.up();
    await tester.pump(const Duration(milliseconds: 80));
    await mouseClick(tester, row);
    expect(renames, 0);
  });
}
