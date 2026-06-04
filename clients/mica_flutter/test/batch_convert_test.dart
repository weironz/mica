import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

EditorController _doc(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  c.selection = DocSelection(
    anchor: const DocPosition(0, 0),
    focus: DocPosition(nodes.length - 1, nodes.last.text.length),
  );
  return c;
}

void main() {
  test('converts every selected block to a bulleted list (marks kept)', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'one', data: {
        'marks': marksToJson([Mark(0, 3, 'bold')]),
      }),
      EditorNode(id: 'b', kind: 'paragraph', text: 'two'),
      EditorNode(id: 'c', kind: 'heading', text: 'three', data: {'level': 1}),
    ]);
    c.setSelectedBlocksKind('bulleted_list');
    expect(c.nodes.map((n) => n.kind).toList(),
        ['bulleted_list', 'bulleted_list', 'bulleted_list']);
    // Inline marks survive the conversion.
    expect(marksFromData(c.nodes[0].data).single.type, 'bold');
  });

  test('todo conversion carries checked data', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'x'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'y'),
    ]);
    c.setSelectedBlocksKind('todo', data: {'checked': false});
    expect(c.nodes.every((n) => n.kind == 'todo'), isTrue);
    expect(c.nodes[0].data['checked'], false);
  });

  test('code block merges selected lines into one block', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'line1'),
      EditorNode(id: 'b', kind: 'paragraph', text: 'line2'),
      EditorNode(id: 'c', kind: 'paragraph', text: 'line3'),
    ]);
    c.setSelectedBlocksKind('code_block');
    expect(c.nodes.length, 1);
    expect(c.nodes.single.kind, 'code_block');
    expect(c.nodes.single.text, 'line1\nline2\nline3');
  });

  test('atomic blocks are skipped during conversion', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'x'),
      EditorNode(id: 'b', kind: 'image', text: '', data: {'file_id': 'f'}),
      EditorNode(id: 'c', kind: 'paragraph', text: 'y'),
    ]);
    c.setSelectedBlocksKind('quote');
    expect(c.nodes[0].kind, 'quote');
    expect(c.nodes[1].kind, 'image'); // untouched
    expect(c.nodes[2].kind, 'quote');
  });
}
