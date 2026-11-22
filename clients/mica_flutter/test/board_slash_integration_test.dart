import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/board/board_models.dart';
import 'package:mica_flutter/editor/board_inline_layout.dart';
import 'package:mica_flutter/editor/board_preview.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void main() {
  test('mica-board code_block acts as unit block in preview mode and yields starter JSON', () {
    final previewNode = EditorNode(
      id: 'b1',
      kind: 'code_block',
      text: starterEmbeddedBoardJson('mindmap'),
      data: const {'language': 'mica-board'},
    );
    expect(previewNode.isUnitBlock, isTrue);

    final sourceNode = EditorNode(
      id: 'b2',
      kind: 'code_block',
      text: starterEmbeddedBoardJson('flowchart'),
      data: const {'language': 'mica-board', 'view': 'code'},
    );
    expect(sourceNode.isUnitBlock, isFalse);

    for (final kind in const ['whiteboard', 'mindmap', 'flowchart']) {
      final snap = BoardSnapshot.tryParseJson(starterEmbeddedBoardJson(kind));
      expect(snap, isNotNull);
      expect(snap!.nodes, isNotEmpty);
    }

    expect(const BoardPreviewer().id, 'board');
  });

  test('inline board frame respects natural width, requested width, and 8-handle hit testing', () {
    final frame = computeBoardFrame(
      maxWidth: 800,
      gutter: 28,
      pixels: const Size(1000, 500),
      y: 100,
      naturalWidth: 500,
    );
    expect(frame.rect.left, 28);
    expect(frame.rect.top, 108);
    expect(frame.rect.width, 500);
    expect(frame.rect.height, 250);
    expect(frame.boxHeight, 250 + kBoardFramePadV * 2);

    // Every one of the 8 handles hits at its center and returns a resize cursor.
    for (final (handle, center) in boardHandleCenters(frame.rect)) {
      expect(hitTestBoardHandle(frame.rect, center), handle);
      expect(boardResizeCursor(handle), isNot(SystemMouseCursors.basic));
    }
    expect(hitTestBoardHandle(frame.rect, frame.rect.center), isNull);

    // Requested width clamps cleanly between kBoardMinWidth and maxW.
    final shrunk = computeBoardFrame(
      maxWidth: 800,
      gutter: 28,
      pixels: const Size(1000, 500),
      y: 100,
      requestedWidth: 50,
    );
    expect(shrunk.rect.width, kBoardMinWidth);

    final enlarged = computeBoardFrame(
      maxWidth: 800,
      gutter: 28,
      pixels: const Size(1000, 500),
      y: 100,
      requestedWidth: 2000,
    );
    expect(enlarged.rect.width, boardMaxWidthFor(800, 28));
  });

  test('computeResizedBoardWidth enlarges and shrinks proportionally from all handles', () {
    const start = Offset(528, 358);
    // Drag bottom-right outward by +120px X -> enlarges from 500 to 620.
    expect(
      computeResizedBoardWidth(
        handle: BoardResizeHandle.se,
        startLocal: start,
        currentLocal: start + const Offset(120, 60),
        startWidth: 500,
        aspect: 0.5,
        maxWidth: 764,
      ),
      620,
    );
    // Drag bottom-right inward by -200px X -> shrinks from 500 to 300.
    expect(
      computeResizedBoardWidth(
        handle: BoardResizeHandle.se,
        startLocal: start,
        currentLocal: start + const Offset(-200, -100),
        startWidth: 500,
        aspect: 0.5,
        maxWidth: 764,
      ),
      300,
    );
    // Drag bottom edge downward by +50px (aspect 0.5 -> +100px width).
    expect(
      computeResizedBoardWidth(
        handle: BoardResizeHandle.s,
        startLocal: start,
        currentLocal: start + const Offset(0, 50),
        startWidth: 500,
        aspect: 0.5,
        maxWidth: 764,
      ),
      600,
    );
    // Drag left edge leftward by -80px -> widens to 580.
    expect(
      computeResizedBoardWidth(
        handle: BoardResizeHandle.w,
        startLocal: start,
        currentLocal: start + const Offset(-80, 0),
        startWidth: 500,
        aspect: 0.5,
        maxWidth: 764,
      ),
      580,
    );
  });

  test('EditorController.previewBoardWidth previews locally and setBoardWidth commits once', () async {
    final ops = <List<Map<String, dynamic>>>[];
    final ctl = EditorController(
      rootBlockId: 'root',
      onOps: (batch) async => ops.add(batch),
    );
    ctl.nodes.addAll([
      EditorNode(
        id: 'b1',
        kind: 'code_block',
        text: starterEmbeddedBoardJson('whiteboard'),
        data: const {'language': 'mica-board'},
      ),
      EditorNode(
        id: 'c1',
        kind: 'code_block',
        text: 'print(1)',
        data: const {'language': 'python'},
      ),
    ]);

    ctl.previewBoardWidth(0, 420);
    await Future<void>.delayed(Duration.zero);
    expect(ctl.nodes[0].data['width'], 420);
    expect(ops, isEmpty);

    ctl.setBoardWidth(0, 580);
    await Future<void>.delayed(Duration.zero);
    expect(ctl.nodes[0].data['width'], 580);
    expect(ops.length, 1);
    expect(ops.single.single['type'], 'update_block');
    expect((ops.single.single['data'] as Map)['width'], 580);

    // Non-board code blocks are untouched.
    ctl.setBoardWidth(1, 300);
    await Future<void>.delayed(Duration.zero);
    expect(ctl.nodes[1].data['width'], isNull);
    expect(ops.length, 1);
    ctl.dispose();
  });

  test('paintBoardFrame and paintBoardResizeChrome render cleanly in light and dark themes', () {
    for (final tokens in const [MicaTokens.light, MicaTokens.dark_]) {
      final rec = ui.PictureRecorder();
      final canvas = Canvas(rec);
      const rect = Rect.fromLTWH(28, 16, 480, 260);
      paintBoardFrame(canvas, rect, tokens);
      paintBoardResizeChrome(
        canvas,
        rect,
        tokens,
        resizing: true,
        activeHandle: BoardResizeHandle.se,
      );
      rec.endRecording().dispose();
    }
  });

  test('clampBoardPan allows free inline panning while keeping content reachable', () {
    const rect = Rect.fromLTWH(28, 16, 400, 200);
    const content = Size(400, 200);
    expect(clampBoardPan(const Offset(120, -60), rect, content), const Offset(120, -60));
    expect(clampBoardPan(const Offset(999, -999), rect, content), const Offset(340, -170));
  });
}
