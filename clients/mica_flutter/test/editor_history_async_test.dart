import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorNode _paragraph(String text) =>
    EditorNode(id: 'a', kind: 'paragraph', text: text);

void main() {
  test('typing during engine undo survives as a new local edit', () async {
    final undoResult = Completer<EditorHistoryState>();
    final enteredUndo = Completer<void>();
    final writes = <String>[];
    final controller = EditorController(
      rootBlockId: 'root',
      onOps: (ops) async {
        writes.add(ops.single['text'] as String);
      },
      onHistory: (action) async {
        if (action == HistoryAction.undo) {
          enteredUndo.complete();
          return undoResult.future;
        }
        return EditorHistoryState(
          nodes: [_paragraph('a')],
          canUndo: false,
          canRedo: false,
          undoDepth: 0,
          redoDepth: 0,
        );
      },
    );
    addTearDown(controller.dispose);
    controller.load([_paragraph('a')]);
    controller.selection = const DocSelection.collapsed(DocPosition(0, 1));
    controller.setFocusedText('ab', 2, 2);
    await controller.flushPending();

    final undo = controller.undo();
    await enteredUndo.future;
    controller.setFocusedText('ab🙂', 4, 4);
    undoResult.complete(
      EditorHistoryState(
        nodes: [_paragraph('a')],
        canUndo: false,
        canRedo: true,
        undoDepth: 0,
        redoDepth: 1,
      ),
    );
    await undo;

    expect(controller.nodes.single.text, 'a🙂');
    expect(controller.selection!.focus.offset, 3);
    expect(await controller.flushPending(), isTrue);
    expect(writes, ['ab', 'a🙂']);
    expect(controller.canUndo, isTrue);
    expect(controller.canRedo, isFalse);
  });

  test(
    'a failed burst commit blocks engine undo until it is durable',
    () async {
      var fail = true;
      var undoCalls = 0;
      final faults = <Object>[];
      final controller = EditorController(
        rootBlockId: 'root',
        onOps: (_) async {
          if (fail) throw StateError('disk full');
        },
        onOpFault: (error, _) => faults.add(error),
        onHistory: (action) async {
          if (action == HistoryAction.undo) undoCalls++;
          return EditorHistoryState(
            nodes: [_paragraph('a')],
            canUndo: false,
            canRedo: action == HistoryAction.undo,
            undoDepth: 0,
            redoDepth: action == HistoryAction.undo ? 1 : 0,
          );
        },
      );
      addTearDown(controller.dispose);
      controller.load([_paragraph('a')]);
      controller.selection = const DocSelection.collapsed(DocPosition(0, 1));
      controller.setFocusedText('ab', 2, 2);

      await controller.undo();
      expect(undoCalls, 0);
      expect(controller.nodes.single.text, 'ab');
      expect(controller.canUndo, isTrue);
      expect(faults, hasLength(1));

      fail = false;
      await controller.undo();
      expect(undoCalls, 1);
      expect(controller.nodes.single.text, 'a');
      expect(controller.canUndo, isFalse);
      expect(controller.canRedo, isTrue);
    },
  );

  test('blocked history persistence retains the same undo metadata', () async {
    var blocked = true;
    final controller = EditorController(
      rootBlockId: 'root',
      onOps: (_) async {},
      onHistory: (action) async {
        final pending = action == HistoryAction.undo && blocked;
        return EditorHistoryState(
          nodes: [_paragraph(pending ? 'ab' : 'a')],
          canUndo: pending,
          canRedo: action == HistoryAction.undo && !blocked,
          undoDepth: pending ? 1 : 0,
          redoDepth: action == HistoryAction.undo && !blocked ? 1 : 0,
          persistenceError: pending
              ? StateError('previous append failed')
              : null,
        );
      },
    );
    addTearDown(controller.dispose);
    controller.load([_paragraph('a')]);
    controller.selection = const DocSelection.collapsed(DocPosition(0, 1));
    controller.setFocusedText('ab', 2, 2);
    await controller.flushPending();

    await controller.undo();
    expect(controller.nodes.single.text, 'ab');
    expect(controller.canUndo, isTrue);
    expect(controller.canRedo, isFalse);
    expect(controller.opFaultCount, 1);

    blocked = false;
    await controller.undo();
    expect(controller.nodes.single.text, 'a');
    expect(controller.selection!.focus.offset, 1);
    expect(controller.canUndo, isFalse);
    expect(controller.canRedo, isTrue);
  });
}
