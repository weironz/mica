import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/board/board_models.dart';
import 'package:mica_flutter/editor/board_preview.dart';
import 'package:mica_flutter/editor/model.dart';

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
}
