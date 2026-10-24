import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

void main() {
  late EditorController controller;
  late List<List<DocOp>> batches;
  setUp(() {
    batches = [];
    controller = EditorController(
      rootBlockId: 'root',
      onOps: (ops) async => batches.add(ops),
    );
    controller.load([
      EditorNode(id: 'a', kind: 'bulleted_list', text: 'first'),
      EditorNode(
        id: 'b',
        kind: 'bulleted_list',
        text: 'second',
        data: {'indent': 1},
      ),
      EditorNode(id: 'c', kind: 'bulleted_list', text: 'third'),
    ]);
  });
  tearDown(() => controller.dispose());

  const forward = DocSelection(
    anchor: DocPosition(0, 2),
    focus: DocPosition(2, 3),
  );

  for (final type in ['bold', 'italic', 'strike', 'code', 'link']) {
    test('cross-list $type respects endpoints and commits one batch', () async {
      controller.selection = forward;
      controller.toggleMark(
        type,
        href: type == 'link' ? 'https://example.test' : null,
      );
      await controller.flushPending();
      expect(batches, hasLength(1));
      expect(batches.single.map((op) => op['block_id']), ['a', 'b', 'c']);
      for (final (index, start, end) in [(0, 2, 5), (1, 0, 6), (2, 0, 3)]) {
        final mark = marksFromData(controller.nodes[index].data).single;
        expect((mark.start, mark.end, mark.type), (start, end, type));
        if (type == 'link') expect(mark.href, 'https://example.test');
      }
      expect(
        controller.nodes.map((node) => node.kind),
        everyElement('bulleted_list'),
      );
      expect(controller.nodes[1].data['indent'], 1);
      expect(controller.nodes.map((node) => node.text), [
        'first',
        'second',
        'third',
      ]);
      expect(controller.selection, forward);
    });
  }

  test('backwards selection toggles the entire range together', () async {
    controller.selection = DocSelection(
      anchor: forward.focus,
      focus: forward.anchor,
    );
    controller.nodes.first.data['marks'] = marksToJson([Mark(2, 5, 'bold')]);
    controller.toggleMark('bold');
    await controller.flushPending();
    expect(
      controller.nodes.map((node) => marksFromData(node.data).single.type),
      everyElement('bold'),
    );
    controller.toggleMark('bold');
    await controller.flushPending();
    expect(
      controller.nodes.map((node) => marksFromData(node.data)),
      everyElement(isEmpty),
    );
    expect(controller.selection?.anchor, forward.focus);
  });

  test(
    'skips code, atomic blocks, empty lines and an unselected end block',
    () async {
      controller.load([
        EditorNode(id: 'a', kind: 'paragraph', text: 'first'),
        EditorNode(id: 'code', kind: 'code_block', text: 'source'),
        EditorNode(id: 'table', kind: 'table', text: ''),
        EditorNode(id: 'image', kind: 'image', text: 'alt'),
        EditorNode(id: 'empty', kind: 'bulleted_list', text: ''),
        EditorNode(
          id: 'b',
          kind: 'todo',
          text: 'second',
          data: {'checked': true},
        ),
        EditorNode(id: 'c', kind: 'paragraph', text: 'untouched'),
      ]);
      controller.selection = const DocSelection(
        anchor: DocPosition(0, 1),
        focus: DocPosition(6, 0),
      );
      controller.toggleMark('italic');
      await controller.flushPending();
      expect(batches.single.map((op) => op['block_id']), ['a', 'b']);
      for (final index in [1, 2, 3, 4, 6]) {
        expect(controller.nodes[index].data.containsKey('marks'), isFalse);
      }
      expect(controller.nodes[5].data['checked'], isTrue);
    },
  );

  test('multiblock formatting is one undo and redo step', () async {
    controller.selection = forward;
    controller.toggleMark('bold');
    await controller.flushPending();
    controller.undo();
    await pumpEventQueue();
    expect(
      controller.nodes.map((node) => marksFromData(node.data)),
      everyElement(isEmpty),
    );
    expect(controller.canUndo, isFalse);
    controller.redo();
    await pumpEventQueue();
    expect(
      controller.nodes.map((node) => marksFromData(node.data).single.type),
      everyElement('bold'),
    );
  });

  test('collapsed ranges and missing link targets do not commit', () async {
    controller.selection = const DocSelection.collapsed(DocPosition(0, 1));
    controller.toggleMark('bold');
    controller.selection = forward;
    controller.toggleMark('link');
    await controller.flushPending();
    expect(batches, isEmpty);
  });

  test('removing marks keeps formatting outside partial endpoints', () async {
    for (final node in controller.nodes) {
      node.data['marks'] = marksToJson([Mark(0, node.text.length, 'bold')]);
    }
    controller.selection = forward;
    controller.toggleMark('bold');
    await controller.flushPending();
    expect(marksToJson(marksFromData(controller.nodes.first.data)), [
      Mark(0, 2, 'bold').toJson(),
    ]);
    expect(marksFromData(controller.nodes[1].data), isEmpty);
    expect(marksToJson(marksFromData(controller.nodes.last.data)), [
      Mark(3, 5, 'bold').toJson(),
    ]);
  });
}
