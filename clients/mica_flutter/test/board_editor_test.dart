import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/board/board_controller.dart';
import 'package:mica_flutter/board/board_editor_screen.dart';
import 'package:mica_flutter/board/board_models.dart';

void main() {
  test('BoardController undo/redo, smart snap, resize, connector, and mindmap flow', () {
    final ctrl = BoardController(BoardSnapshot.starter('mindmap'));
    expect(ctrl.snapshot.nodes.length, 4);
    expect(ctrl.snapshot.edges.length, 3);
    expect(ctrl.canUndo, isFalse);

    // Add mind child & verify undo/redo.
    final child = ctrl.addMindChild('m1');
    expect(child, isNotNull);
    expect(ctrl.snapshot.nodes.length, 5);
    expect(ctrl.canUndo, isTrue);

    // Collapse m1 hides its child from visible descendants.
    ctrl.toggleCollapse('m1');
    expect(collapsedDescendants(ctrl.snapshot), contains(child!.id));
    ctrl.toggleCollapse('m1');
    expect(collapsedDescendants(ctrl.snapshot), isEmpty);

    // Undo restores prior collapse state & addition cleanly.
    ctrl.undo();
    expect(collapsedDescendants(ctrl.snapshot), contains(child.id));
    ctrl.redo();
    expect(collapsedDescendants(ctrl.snapshot), isEmpty);

    // Duplicate selection.
    ctrl.selectNode('m1');
    final beforeDup = ctrl.snapshot.nodes.length;
    ctrl.duplicateSelection();
    expect(ctrl.snapshot.nodes.length, beforeDup + 1);

    // Drag with smart alignment snapping: slow 2px steps MUST smoothly snap
    // near the guide and then break free without getting stuck in quicksand.
    final added = ctrl.addNodeAt(const Offset(100, 900), kind: 'diamond');
    final neighbor = ctrl.addNodeAt(const Offset(400, 903), kind: 'cylinder');
    final startY = added.y;
    ctrl.beginNodeDrag(added.id);
    ctrl.updateNodeDrag(const Offset(0, 1));
    // Within +-6 px of neighbor's center Y -> snaps & emits horizontal guide.
    expect(ctrl.activeGuides.isNotEmpty, isTrue);
    // Continue moving slowly (+2 px per frame x 20 frames = +40 px total).
    // Must escape the alignment line cleanly instead of staying trapped!
    for (var i = 0; i < 20; i++) {
      ctrl.updateNodeDrag(const Offset(0, 2));
    }
    expect(added.y, closeTo(startY + 41, 0.5));
    expect(ctrl.activeGuides, isEmpty);
    ctrl.endNodeDrag();

    // Connector drag creates edge between added & neighbor.
    final edgeBefore = ctrl.snapshot.edges.length;
    ctrl.beginConnectorDrag(added.id, 'r', nodeAnchorOffset(added, 'r'));
    ctrl.updateConnectorDrag(neighbor.center);
    ctrl.endConnectorDrag();
    expect(ctrl.snapshot.edges.length, edgeBefore + 1);

    // JSON round-trip preserves everything.
    final parsed = BoardSnapshot.tryParseJson(ctrl.snapshot.toCompactJson());
    expect(parsed, isNotNull);
    expect(parsed!.nodes.length, ctrl.snapshot.nodes.length);
    expect(parsed.edges.length, ctrl.snapshot.edges.length);
  });

  testWidgets('BoardEditorScreen renders toolbar, shape/icon libraries, inspector, and saves JSON', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1440, 900);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    String? savedResult;
    final initial = BoardSnapshot.starter('flowchart').toCompactJson();

    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) => Scaffold(
            body: ElevatedButton(
              onPressed: () async {
                savedResult = await showBoardEditorDialog(
                  context,
                  initialJson: initial,
                );
              },
              child: const Text('Open'),
            ),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();

    // Top bar & left dock rendered.
    expect(find.text('完成'), findsOneWidget);
    expect(find.text('整理导图'), findsOneWidget);

    // Open Shape library popover.
    await tester.tap(find.byIcon(Icons.category_outlined));
    await tester.pump();
    expect(find.text('流程图标准图形'), findsOneWidget);
    expect(find.text('判断条件'), findsOneWidget);

    // Open Tech Icon library popover.
    await tester.tap(find.byIcon(Icons.dns_outlined));
    await tester.pump();
    expect(find.text('技术架构图标库（选后点击画布放置）'), findsOneWidget);
    expect(find.text('数据库'), findsOneWidget);

    // Pick Database icon & tap canvas to place it.
    await tester.tap(find.text('数据库'));
    await tester.pump();
    await tester.tapAt(const Offset(700, 450));
    await tester.pump();

    // Finish & verify updated JSON contains the newly placed icon_card node.
    await tester.tap(find.text('完成'));
    await tester.pumpAndSettle();

    expect(savedResult, isNotNull);
    final savedSnap = BoardSnapshot.tryParseJson(savedResult!);
    expect(savedSnap, isNotNull);
    expect(savedSnap!.nodes.any((n) => n.icon == 'database'), isTrue);
  });
}
