import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import '../e2e/board_spike_canvas.dart';

void main() {
  testWidgets('BoardSpikeCanvas isolates static layer during drag and auto-grows on edit', (
    tester,
  ) async {
    final controller = BoardSpikeController(nodeCount: 500);
    expect(controller.nodes.length, 500);
    expect(controller.edges.length, greaterThan(400));

    await tester.pumpWidget(
      MaterialApp(home: BoardSpikeCanvas(controller: controller)),
    );
    await tester.pump();

    final initialStaticPaints = controller.staticPaintCount;
    final initialPicRebuilds = controller.pictureRebuildCount;
    expect(initialStaticPaints, greaterThanOrEqualTo(1));
    expect(initialPicRebuilds, 1);

    // Begin dragging n0: static picture rebuilds ONCE to exclude n0.
    controller.beginDrag('n0');
    await tester.pump();
    final afterBeginStatic = controller.staticPaintCount;
    final afterBeginPic = controller.pictureRebuildCount;
    expect(afterBeginPic, initialPicRebuilds + 1);

    // 20 drag move updates MUST NOT repaint the static layer or rebuild picture.
    final activeBefore = controller.activePaintCount;
    for (var i = 0; i < 20; i++) {
      controller.updateDrag(const Offset(5, 3));
      await tester.pump();
    }
    expect(controller.staticPaintCount, afterBeginStatic);
    expect(controller.pictureRebuildCount, afterBeginPic);
    expect(controller.activePaintCount, greaterThanOrEqualTo(activeBefore + 20));

    controller.endDrag();
    await tester.pump();

    // Auto-grow height when multiline text is entered.
    final n0 = controller.nodeById['n0']!;
    final hBefore = n0.height;
    controller.startEditing('n0');
    await tester.pump();
    controller.updateEditingText(
      'n0',
      '${n0.text}\n新增第一行\n新增第二行\n新增第三行自动撑高',
    );
    await tester.pump();
    expect(n0.height, greaterThan(hBefore));
    controller.finishEditing();
    await tester.pump();
    expect(controller.editingNodeId, isNull);
  });
}
