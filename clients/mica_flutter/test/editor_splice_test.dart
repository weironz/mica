import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

// Regression for issue #131: plain text replacement never remapped marks, so a
// paste at the start of a paragraph left bold over the wrong characters and a
// cross-block inline-math paste only touched the focus block. The single
// document-range splice primitive is the seam.
EditorController _doc(List<EditorNode> nodes, {List<List<DocOp>>? ops}) {
  final c = EditorController(
    rootBlockId: 'root',
    onOps: (batch) async => ops?.add(batch),
  );
  c.load(nodes);
  return c;
}

void main() {
  test('cross-block splice clips tail marks at the surviving junction', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'alpha'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'beta', data: {
        'marks': marksToJson([
          Mark(0, 2, 'link', href: '/deleted', title: 'gone'),
          Mark(1, 4, 'bold'),
        ]),
      }),
    ]);
    c.spliceRange(const DocSelection(anchor: DocPosition(0, 2), focus: DocPosition(1, 2)), 'X');
    expect(c.nodes.single.text, 'alXta');
    final marks = marksFromData(c.nodes.single.data);
    expect(marks.map((m) => m.type), ['bold']);
    expect((marks.single.start, marks.single.end), (3, 5));
  });

  test('inserting text before a mark pushes the mark right', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'abcd', data: {
        'marks': marksToJson([Mark(2, 4, 'bold')]),
      }),
    ]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 0));
    c.insertTextAtCaret('X');

    expect(c.nodes.single.text, 'Xabcd');
    final bold = marksFromData(c.nodes.single.data).single;
    expect(c.nodes.single.text.substring(bold.start, bold.end), 'cd');
  });

  test('a document-range splice replaces a cross-block selection', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'alpha'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'beta'),
    ]);
    const range = DocSelection(
      anchor: DocPosition(0, 2),
      focus: DocPosition(1, 2),
    );
    c.spliceRange(range, 'x', inserts: [Mark(0, 1, 'math')]);

    expect(c.nodes.length, 1);
    expect(c.nodes.single.text, 'alxta');
    final math = marksFromData(c.nodes.single.data).single;
    expect(math.type, 'math');
    expect(math.start, 2);
    expect(math.end, 3);
    expect(c.selection!.focus.offset, 3);
  });

  test('IME newline remaps marks by the actual replaced range', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'abcdef', data: {
        'marks': marksToJson([
          Mark(4, 6, 'bold'),
          Mark(2, 4, 'link', href: 'https://x'),
        ]),
      }),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 2),
      focus: DocPosition(0, 4),
    );
    c.applyNewlineSplit('ab', 'ef');

    expect(c.nodes.length, 2);
    expect(c.nodes[0].text, 'ab');
    expect(c.nodes[1].text, 'ef');
    expect(marksFromData(c.nodes[0].data), isEmpty);
    final after = marksFromData(c.nodes[1].data);
    expect(after.length, 1, reason: 'the link over the deleted text must die');
    expect(after.single.type, 'bold');
    expect((after.single.start, after.single.end), (0, 2));
  });

  test('spliceRange emits one batch covering the merged node', () async {
    final batches = <List<DocOp>>[];
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'alpha'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'beta'),
      EditorNode(id: 'c', kind: 'paragraph', text: 'tail'),
    ], ops: batches);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(1, 4),
    );
    c.spliceRange(
      c.selection!,
      'X',
      inserts: [Mark(0, 1, 'bold')],
    );
    await c.flushPending();
    expect(c.nodes.map((n) => n.text), ['X', 'tail']);
    expect(batches.expand((b) => b).map((op) => op['type']), contains('delete_block'));
  });
}