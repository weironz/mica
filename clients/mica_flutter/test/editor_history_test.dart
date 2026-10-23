import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorNode _p(String id, String text, [Map<String, dynamic>? data]) =>
    EditorNode(id: id, kind: 'paragraph', text: text, data: data ?? {});

void main() {
  test(
    'a remote property change does not hide the local text inverse',
    () async {
      final batches = <List<DocOp>>[];
      final c = EditorController(
        rootBlockId: 'root',
        onOps: (ops) async => batches.add(ops),
      );
      addTearDown(c.dispose);
      c.load([_p('a', 'hello')]);
      c.selection = const DocSelection.collapsed(DocPosition(0, 5));
      c.setFocusedText('hello!', 6, 6);
      await c.flushPending();
      c.reconcile([
        _p('a', 'hello!', {'color': 'remote'}),
      ]);
      c.undo();
      await pumpEventQueue();
      expect(c.nodes.single.text, 'hello');
      expect(c.nodes.single.data['color'], 'remote');
      expect(batches.last.single['data'], {'color': 'remote'});
      c.redo();
      await pumpEventQueue();
      expect(c.nodes.single.text, 'hello!');
      expect(c.nodes.single.data['color'], 'remote');
    },
  );

  test(
    'remote reconcile during a typing burst preserves its inverse',
    () async {
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
      addTearDown(c.dispose);
      c.load([_p('a', 'local'), _p('b', 'before')]);
      c.selection = const DocSelection.collapsed(DocPosition(0, 5));
      c.setFocusedText('local!', 6, 6);
      c.reconcile([_p('a', 'local'), _p('b', 'remote')]);
      c.undo();
      await pumpEventQueue();
      expect(c.nodes.map((n) => n.text), ['local', 'remote']);
      c.redo();
      await pumpEventQueue();
      expect(c.nodes.map((n) => n.text), ['local!', 'remote']);
    },
  );

  test(
    'engine undo waits for the burst commit and emits no coarse restore',
    () async {
      final commit = Completer<void>();
      final events = <String>[];
      final c = EditorController(
        rootBlockId: 'root',
        onOps: (ops) async {
          events.add('commit');
          await commit.future;
          events.add('durable');
        },
        onHistory: (action) async {
          events.add(action.name);
          return EditorHistoryState(
            nodes: [
              _p('foreign', 'remote'),
              _p('a', action == HistoryAction.redo ? 'local!' : 'local'),
            ],
            canUndo: action == HistoryAction.redo,
            canRedo: action == HistoryAction.undo,
          );
        },
      );
      addTearDown(c.dispose);
      c.load([_p('a', 'local')]);
      c.selection = const DocSelection.collapsed(DocPosition(0, 5));
      c.setFocusedText('local!', 6, 6);
      c.undo();
      await pumpEventQueue();
      expect(events, ['reset', 'commit']);
      commit.complete();
      await pumpEventQueue();
      expect(events, ['reset', 'commit', 'durable', 'undo']);
      expect(c.nodes.map((n) => n.text), ['remote', 'local']);
      expect(c.canRedo, isTrue);
      c.redo();
      await pumpEventQueue();
      expect(events.last, 'redo');
      expect(events.where((e) => e == 'commit'), hasLength(1));
      expect(c.nodes.last.text, 'local!');
    },
  );

  test(
    'selection metadata follows engine depths when foreign items are skipped',
    () async {
      var undos = 0;
      final c = EditorController(
        rootBlockId: 'root',
        onOps: (_) async {},
        onHistory: (action) async {
          if (action == HistoryAction.undo) undos++;
          return EditorHistoryState(
            nodes: [
              _p('a', undos < 2 ? 'A' : ''),
              _p('b', ''),
              _p('c', 'remote'),
            ],
            canUndo: undos < 2,
            canRedo: undos > 0,
            undoDepth: undos == 1 ? 1 : 0,
            redoDepth: undos,
          );
        },
      );
      addTearDown(c.dispose);
      c.load([_p('a', ''), _p('b', ''), _p('c', '')]);
      for (var i = 0; i < 3; i++) {
        c.selection = DocSelection.collapsed(DocPosition(i, 0));
        c.setFocusedText(['A', 'B', 'C'][i], 1, 1);
        await c.flushPending();
      }
      c.undo();
      await pumpEventQueue();
      expect(
        c.selection!.focus.node,
        1,
        reason: 'engine skipped c and reverted b',
      );
      expect(c.nodes.last.text, 'remote');
      c.undo();
      await pumpEventQueue();
      expect(
        c.selection!.focus.node,
        0,
        reason: 'the remaining item belongs to a',
      );
      expect(c.canUndo, isFalse);
    },
  );

  test(
    'engine history failure stays observable and retains the step',
    () async {
      var fail = true;
      final faults = <Object>[];
      final c = EditorController(
        rootBlockId: 'root',
        onOps: (_) async {},
        onOpFault: (error, _) => faults.add(error),
        onHistory: (action) async {
          if (action == HistoryAction.undo && fail) throw StateError('disk');
          return EditorHistoryState(
            nodes: [_p('a', 'x')],
            canUndo: false,
            canRedo: true,
          );
        },
      );
      addTearDown(c.dispose);
      c.load([_p('a', 'x')]);
      c.selection = const DocSelection.collapsed(DocPosition(0, 1));
      c.setFocusedText('xy', 2, 2);
      await c.flushPending();
      c.undo();
      await pumpEventQueue();
      expect(faults, hasLength(1));
      expect(c.nodes.single.text, 'xy');
      expect(c.canUndo, isTrue);
      fail = false;
      c.undo();
      await pumpEventQueue();
      expect(c.nodes.single.text, 'x');
    },
  );
}
