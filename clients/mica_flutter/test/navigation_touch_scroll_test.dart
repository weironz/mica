import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/main.dart';
import 'package:mica_flutter/ui/navigation_draggable.dart';

import 'support/title_workspace.dart';

void main() {
  final bootstrap = titleBootstrap('当前页面');
  final views = [
    for (var i = 0; i < 40; i++)
      DocumentView.fromJson({
        'id': 'page-$i',
        'object_id': 'page-$i',
        'object_type': 'document',
        'name': '目录 $i',
        'position': i.toString().padLeft(4, '0'),
      }),
  ];
  final entries = [
    for (var i = 0; i < 20; i++)
      WorkspaceEntry(
        origin: 'local',
        role: 'owner',
        workspace: Workspace(
          id: 'ws-$i',
          name: '工作区 $i',
          ownerId: 'user',
          role: 'owner',
        ),
      ),
  ];

  Future<void> openDrawer(WidgetTester tester) async {
    await tester.tap(find.byIcon(Icons.menu));
    await tester.pumpAndSettle();
  }

  for (final workspaces in [false, true]) {
    testWidgets(
      '${workspaces ? 'workspace menu' : 'page tree'} swipes scroll without reordering',
      (tester) async {
        var reordered = 0;
        tester.view.devicePixelRatio = 1;
        tester.view.physicalSize = const Size(390, 844);
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        await tester.pumpWidget(
          titleWorkspace(
            bootstrap,
            (_, _) async => true,
            views: views,
            entries: entries,
            onReorderViews: (_, _) async {
              reordered++;
            },
            onReorderWorkspaces: (_) async {
              reordered++;
            },
          ),
        );
        await openDrawer(tester);
        if (workspaces) {
          await tester.tap(find.byIcon(Icons.unfold_more));
          await tester.pumpAndSettle();
        }
        final row = find.text(workspaces ? '工作区 2' : '目录 2');
        final start = tester.getCenter(row);
        final scrollable = tester.state<ScrollableState>(
          find.ancestor(of: row, matching: find.byType(Scrollable)).first,
        );
        await tester.dragFrom(start, const Offset(0, -160));
        await tester.pumpAndSettle();
        expect(scrollable.position.pixels, greaterThan(80));
        expect(reordered, 0);
        expect(find.byType(DragTarget<WorkspaceEntry>), findsNothing);
        expect(tester.takeException(), isNull);
      },
    );
  }

  for (final kind in [PointerDeviceKind.touch, PointerDeviceKind.mouse]) {
    testWidgets('$kind can still reorder after its intended activation', (
      tester,
    ) async {
      var dropped = false;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Column(
              children: [
                const NavigationDraggable<String>(
                  data: 'row',
                  feedback: Text('dragging'),
                  child: SizedBox(
                    width: 250,
                    height: 80,
                    child: Text('source'),
                  ),
                ),
                DragTarget<String>(
                  onAcceptWithDetails: (_) => dropped = true,
                  builder: (_, _, _) => const SizedBox(
                    width: 250,
                    height: 80,
                    child: Text('target'),
                  ),
                ),
              ],
            ),
          ),
        ),
      );
      final gesture = await tester.startGesture(
        tester.getCenter(find.text('source')),
        kind: kind,
      );
      if (kind == PointerDeviceKind.touch) {
        await tester.pump(kLongPressTimeout + const Duration(milliseconds: 1));
      }
      await gesture.moveBy(const Offset(0, 25));
      await tester.pump();
      expect(find.text('dragging'), findsOneWidget);
      await gesture.moveTo(tester.getCenter(find.text('target')));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      expect(dropped, isTrue);
      expect(tester.takeException(), isNull);
    });
  }
}
