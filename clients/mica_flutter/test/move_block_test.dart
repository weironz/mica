import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

void main() {
  EditorController load(List<String> ids) {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([for (final id in ids) EditorNode(id: id, kind: 'paragraph', text: id)]);
    return c;
  }

  List<String> order(EditorController c) => [for (final n in c.nodes) n.id];

  test('moveBlock reorders and emits move_block at the final index', () async {
    final ops = <Map<String, dynamic>>[];
    final c = EditorController(
        rootBlockId: 'root',
        onOps: (batch) async => ops.addAll(batch.cast<Map<String, dynamic>>()));
    c.load([
      EditorNode(id: 'a', kind: 'paragraph', text: 'a'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'b'),
      EditorNode(id: 'c', kind: 'paragraph', text: 'c'),
    ]);
    // Drag a below c (insertion index 3).
    expect(c.moveBlock(0, 3), isTrue);
    expect(order(c), ['b', 'c', 'a']);
    await Future<void>.delayed(Duration.zero);
    final move = ops.singleWhere((o) => o['type'] == 'move_block');
    expect(move['block_id'], 'a');
    expect(move['index'], 2);
  });

  test('dropping a block onto its own slot is a no-op', () {
    final c = load(['a', 'b']);
    expect(c.moveBlock(0, 0), isFalse);
    expect(c.moveBlock(0, 1), isFalse); // directly after itself
    expect(order(c), ['a', 'b']);
  });

  test('move up and undo restores the order', () async {
    final c = load(['a', 'b', 'c']);
    expect(c.moveBlock(2, 0), isTrue);
    expect(order(c), ['c', 'a', 'b']);
    await Future<void>.delayed(Duration.zero);
    c.undo();
    expect(order(c), ['a', 'b', 'c']);
  });
}
