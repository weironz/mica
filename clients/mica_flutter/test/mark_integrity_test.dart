import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

EditorController _doc(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  return c;
}

void main() {
  test('deleting a selected link does not bleed onto following text', () {
    // "Guide rest": link over "Guide"; select it and delete.
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'Guide rest', data: {
        'marks': marksToJson([Mark(0, 5, 'link', href: 'https://x')]),
      }),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(0, 5),
    );
    c.deleteSelection();
    expect(c.nodes.single.text, ' rest');
    expect(marksFromData(c.nodes.single.data), isEmpty);
  });

  test('cross-node deleteSelection keeps the tail marks, drops the cut', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'head LINK', data: {
        'marks': marksToJson([Mark(5, 9, 'link', href: 'https://a')]),
      }),
      EditorNode(id: 'b', kind: 'paragraph', text: 'tail BOLD!', data: {
        'marks': marksToJson([Mark(5, 9, 'bold')]),
      }),
    ]);
    // Select from before LINK to after "tail " in the second node.
    c.selection = const DocSelection(
      anchor: DocPosition(0, 5),
      focus: DocPosition(1, 5),
    );
    c.deleteSelection();
    final node = c.nodes.single;
    expect(node.text, 'head BOLD!');
    final marks = marksFromData(node.data);
    expect(marks.where((m) => m.type == 'link'), isEmpty);
    final bold = marks.single;
    expect(node.text.substring(bold.start, bold.end), 'BOLD');
  });

  test('IME diff bias: deleting linked text that repeats its neighbor', () {
    // "AAA" with a link on the first "AA"; selecting [0,2) and deleting
    // aligns the naive diff on the wrong "A" — the selection bias fixes it.
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'AAA', data: {
        'marks': marksToJson([Mark(0, 2, 'link', href: 'https://x')]),
      }),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(0, 2),
    );
    c.setFocusedText('A', 0, 0);
    expect(c.nodes.single.text, 'A');
    expect(marksFromData(c.nodes.single.data), isEmpty);
  });

  test('mergeBackward carries the absorbed node marks shifted', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'left ', data: {
        'marks': marksToJson([Mark(0, 4, 'bold')]),
      }),
      EditorNode(id: 'b', kind: 'paragraph', text: 'LINK here', data: {
        'marks': marksToJson([Mark(0, 4, 'link', href: 'https://b')]),
      }),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(1, 0),
      focus: DocPosition(1, 0),
    );
    expect(c.mergeBackward(), isTrue);
    final node = c.nodes.single;
    expect(node.text, 'left LINK here');
    final marks = marksFromData(node.data);
    final link = marks.firstWhere((m) => m.type == 'link');
    expect(node.text.substring(link.start, link.end), 'LINK');
    final bold = marks.firstWhere((m) => m.type == 'bold');
    expect(node.text.substring(bold.start, bold.end), 'left');
  });

  test('splitAtCaret divides a mark spanning the split point', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'boldtext', data: {
        'marks': marksToJson([Mark(0, 8, 'bold')]),
      }),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 4),
      focus: DocPosition(0, 4),
    );
    c.splitAtCaret();
    expect(c.nodes.length, 2);
    final first = marksFromData(c.nodes[0].data).single;
    expect(c.nodes[0].text.substring(first.start, first.end), 'bold');
    final second = marksFromData(c.nodes[1].data).single;
    expect(c.nodes[1].text.substring(second.start, second.end), 'text');
  });
}
