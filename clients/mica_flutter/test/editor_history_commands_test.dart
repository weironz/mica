import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorNode _p(String id, String text) =>
    EditorNode(id: id, kind: 'paragraph', text: text);

void main() {
  test('failed structural commits must save before engine undo', () async {
    var fail = false;
    var undos = 0;
    final saved = <String>[];
    final controller = EditorController(
      rootBlockId: 'root',
      onOps: (ops) async {
        if (fail) throw StateError('not applied');
        saved.add(ops.single['text'] as String);
      },
      onHistory: (action) async {
        if (action == HistoryAction.undo) undos++;
        return EditorHistoryState(
          nodes: [_p('a', 'A'), _p('b', 'b')],
          canUndo: action == HistoryAction.undo,
          canRedo: action == HistoryAction.undo,
          undoDepth: action == HistoryAction.undo ? 1 : 0,
          redoDepth: action == HistoryAction.undo ? 1 : 0,
        );
      },
    );
    addTearDown(controller.dispose);
    controller.load([_p('a', 'a'), _p('b', 'b')]);
    controller.selection = const DocSelection.collapsed(DocPosition(0, 0));
    controller.setBlockText('a', 'A');
    await controller.flushPending();
    controller.selection = const DocSelection.collapsed(DocPosition(1, 0));
    fail = true;
    controller.setBlockText('b', 'B');
    await pumpEventQueue();

    await controller.undo();
    expect(undos, 0);
    expect(controller.nodes.map((n) => n.text), ['A', 'B']);
    fail = false;
    await controller.undo();
    expect(saved, ['A', 'B']);
    expect(undos, 1);
    expect(controller.nodes.map((n) => n.text), ['A', 'b']);
    expect(controller.selection!.focus.node, 1);
    expect(controller.canUndo, isTrue);
  });

  for (final useBackspace in [false, true]) {
    test('deleting the sole atomic block is one undo ($useBackspace)', () async {
      final batches = <List<DocOp>>[];
      final controller = EditorController(
        rootBlockId: 'root',
        onOps: (ops) async => batches.add(ops),
      );
      addTearDown(controller.dispose);
      controller.load([EditorNode(id: 'table', kind: 'table', text: '')]);
      controller.selection = const DocSelection.collapsed(DocPosition(0, 0));
      if (useBackspace) {
        expect(controller.mergeBackward(), isTrue);
      } else {
        controller.deleteNode(0);
      }
      await controller.flushPending();
      expect(batches, hasLength(1));
      expect(batches.single.map((op) => op['type']), [
        'delete_block',
        'insert_block',
      ]);
      expect(controller.nodes.single.kind, 'paragraph');
      await controller.undo();
      expect(controller.nodes.single.id, 'table');
      expect(controller.nodes.single.kind, 'table');
      expect(controller.canUndo, isFalse);
    });
  }

  test('pasting over a range restores the entire range in one undo', () async {
    final batches = <List<DocOp>>[];
    final controller = EditorController(
      rootBlockId: 'root',
      onOps: (ops) async => batches.add(ops),
    );
    addTearDown(controller.dispose);
    controller.load([_p('a', 'one'), _p('b', 'two')]);
    controller.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(1, 3),
    );
    controller.insertBlocksReplacingSelection([
      (kind: 'paragraph', text: 'replacement', data: <String, dynamic>{}),
    ]);
    await controller.flushPending();
    expect(batches, hasLength(1));
    expect(controller.nodes.single.text, 'replacement');
    await controller.undo();
    expect(controller.nodes.map((n) => n.id), ['a', 'b']);
    expect(controller.nodes.map((n) => n.text), ['one', 'two']);
    expect(controller.canUndo, isFalse);
  });

  test('non-keyboard structure changes wait for engine history too', () async {
    final gate = Completer<EditorHistoryState>();
    final entered = Completer<void>();
    var writes = 0;
    final controller = EditorController(
      rootBlockId: 'root',
      onOps: (_) async => writes++,
      onHistory: (action) async {
        if (action == HistoryAction.undo) {
          entered.complete();
          return gate.future;
        }
        return EditorHistoryState(
          nodes: [_p('a', 'a')],
          canUndo: false,
          canRedo: false,
        );
      },
    );
    addTearDown(controller.dispose);
    controller.load([_p('a', 'a')]);
    controller.selection = const DocSelection.collapsed(DocPosition(0, 1));
    controller.setBlockText('a', 'ab');
    await controller.flushPending();
    final undo = controller.undo();
    await entered.future;
    controller.splitAtCaret();
    controller.setSelectedBlocksKind('heading');
    controller.insertParagraphAtTop('unexpected');
    expect(controller.nodes, hasLength(1));
    expect(controller.nodes.single.kind, 'paragraph');
    gate.complete(
      EditorHistoryState(
        nodes: [_p('a', 'a')],
        canUndo: false,
        canRedo: true,
        undoDepth: 0,
        redoDepth: 1,
      ),
    );
    await undo;
    expect(writes, 1);
    expect(controller.nodes.single.text, 'a');
  });

  test('standalone undo preserves a remotely overwritten move', () async {
    final controller = EditorController(rootBlockId: 'root', onOps: (_) async {});
    addTearDown(controller.dispose);
    controller.load([for (final id in ['a', 'b', 'c', 'd']) _p(id, id)]);
    controller.moveBlock(0, 3);
    await controller.flushPending();
    controller.reconcile([for (final id in ['b', 'c', 'd', 'a']) _p(id, id)]);
    await controller.undo();
    expect(controller.nodes.map((n) => n.id), ['b', 'c', 'd', 'a']);
    await controller.redo();
    expect(controller.nodes.map((n) => n.id), ['b', 'c', 'd', 'a']);
  });
}
