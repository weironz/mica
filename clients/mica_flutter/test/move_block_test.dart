import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

void main() {
  EditorController load(List<String> ids) {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([
      for (final id in ids) EditorNode(id: id, kind: 'paragraph', text: id),
    ]);
    return c;
  }

  List<String> order(EditorController c) => [for (final n in c.nodes) n.id];

  test('moveBlock reorders and emits move_block at the final index', () async {
    final ops = <Map<String, dynamic>>[];
    final c = EditorController(
      rootBlockId: 'root',
      onOps: (batch) async => ops.addAll(batch.cast<Map<String, dynamic>>()),
    );
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

  // ── 2026-10-21 review P1-3 ───────────────────────────────────────────────
  //
  // The case above only checked the LOCAL order — which is why undoing a drag
  // passed while the backend kept the moved order. These replay every emitted
  // op the way the backends do (a move removes, then inserts at `index` in what
  // remains) and require the backend to end up where the editor shows.

  List<String> replay(List<String> server, List<Map<String, dynamic>> ops) {
    final out = [...server];
    for (final op in ops) {
      switch (op['type']) {
        case 'delete_block':
          out.remove(op['block_id']);
        case 'insert_block':
          out.insert(op['index'] as int, (op['block'] as Map)['id'] as String);
        case 'move_block':
          out.remove(op['block_id']);
          out.insert(
            (op['index'] as int).clamp(0, out.length),
            op['block_id'] as String,
          );
      }
    }
    return out;
  }

  test('undoing a drag moves the block back on the backend too', () async {
    final ops = <Map<String, dynamic>>[];
    final c = EditorController(
      rootBlockId: 'root',
      onOps: (batch) async => ops.addAll(batch.cast<Map<String, dynamic>>()),
    );
    c.load([
      for (final id in ['a', 'b', 'c', 'd'])
        EditorNode(id: id, kind: 'paragraph', text: id),
    ]);
    expect(c.moveBlock(0, 4), isTrue); // a to the bottom
    await Future<void>.delayed(Duration.zero);
    c.undo();
    await Future<void>.delayed(Duration.zero);
    expect(order(c), ['a', 'b', 'c', 'd']);
    expect(
      replay(['a', 'b', 'c', 'd'], ops),
      order(c),
      reason: 'the backend kept the dragged order',
    );
    expect(
      ops.where((o) => o['type'] == 'move_block').length,
      2,
      reason: 'the drag, then ONE move to undo it — not one per block',
    );
  });

  test(
    'any sequence of drags, undos and redos keeps the backend in step',
    () async {
      final ids = [for (var i = 0; i < 8; i++) 'n$i'];
      for (var seed = 0; seed < 40; seed++) {
        final ops = <Map<String, dynamic>>[];
        final c = EditorController(
          rootBlockId: 'root',
          onOps: (batch) async =>
              ops.addAll(batch.cast<Map<String, dynamic>>()),
        );
        c.load([
          for (final id in ids) EditorNode(id: id, kind: 'paragraph', text: id),
        ]);
        var x = seed * 7919 + 13;
        int next(int n) => (x = (x * 1103515245 + 12345) & 0x7fffffff) % n;
        for (var step = 0; step < 12; step++) {
          switch (next(3)) {
            case 0:
              c.moveBlock(next(ids.length), next(ids.length + 1));
            case 1:
              c.undo();
            case 2:
              c.redo();
          }
          await Future<void>.delayed(Duration.zero);
          expect(replay(ids, ops), order(c), reason: 'seed $seed step $step');
        }
      }
    },
  );
}
