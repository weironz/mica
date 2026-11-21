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

  test('Enter at the start of a heading keeps its format below', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'heading', text: 'Title', data: {
        'level': 2,
        'marks': marksToJson([Mark(0, 5, 'bold')]),
      }),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(0, 0),
    );
    c.splitAtCaret();
    expect(c.nodes.length, 2);
    // Empty paragraph above; the heading moved down INTACT (kind, level,
    // marks) — previously the format stranded on the empty upper line.
    expect(c.nodes[0].kind, 'paragraph');
    expect(c.nodes[0].text, '');
    expect(c.nodes[1].kind, 'heading');
    expect(c.nodes[1].text, 'Title');
    expect(c.nodes[1].data['level'], 2);
    expect(marksFromData(c.nodes[1].data).single.type, 'bold');
    // Caret stays at the start of the heading.
    expect(c.selection!.focus.node, 1);
    expect(c.selection!.focus.offset, 0);
  });

  test('IME newline at block start inserts a paragraph above too', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'todo', text: 'task', data: {'checked': true}),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(0, 0),
    );
    c.applyNewlineSplit('', 'task');
    expect(c.nodes.length, 2);
    expect(c.nodes[0].kind, 'paragraph');
    expect(c.nodes[1].kind, 'todo');
    expect(c.nodes[1].text, 'task');
    expect(c.nodes[1].data['checked'], true);
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

  group('gh#192: typing before a link keeps inserted text unmarked', () {
    test('typing Chinese prefix at offset 0 before a link shifts the link right', () {
      const url = 'https://github.com/weironz/mica';
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: url, data: {
          'marks': marksToJson([Mark(0, url.length, 'link', href: url)]),
        }),
      ]);
      c.selection = const DocSelection.collapsed(DocPosition(0, 0));
      c.setFocusedText('项目地址：$url', 5, 5);

      final node = c.nodes.single;
      expect(node.text, '项目地址：$url');
      final link = marksFromData(node.data).single;
      expect(link.type, 'link');
      expect(link.href, url);
      expect(node.text.substring(link.start, link.end), url);
      expect((link.start, link.end), (5, 5 + url.length));
    });

    test('typing matching first char before a link stays outside the link', () {
      const url = 'https://example.com';
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: url, data: {
          'marks': marksToJson([Mark(0, url.length, 'link', href: url)]),
        }),
      ]);
      c.selection = const DocSelection.collapsed(DocPosition(0, 0));
      c.setFocusedText('h$url', 1, 1);

      final node = c.nodes.single;
      final link = marksFromData(node.data).single;
      expect(node.text.substring(link.start, link.end), url);
      expect((link.start, link.end), (1, 1 + url.length));
    });

    test('IME preedit and commit before a link keeps committed text plain', () {
      const url = 'https://example.com';
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: url, data: {
          'marks': marksToJson([Mark(0, url.length, 'link', href: url)]),
        }),
      ]);
      c.selection = const DocSelection.collapsed(DocPosition(0, 0));
      c.setFocusedText('x$url', 1, 1);
      c.setFocusedText('xm$url', 2, 2);
      c.setFocusedText('项目地址：$url', 5, 5);

      final node = c.nodes.single;
      final link = marksFromData(node.data).single;
      expect(node.text.substring(link.start, link.end), url);
      expect((link.start, link.end), (5, 5 + url.length));
    });

    test('typing inside a link still extends it, and after it stays plain', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: 'click here', data: {
          'marks': marksToJson([Mark(0, 10, 'link', href: 'https://a.test')]),
        }),
      ]);
      c.selection = const DocSelection.collapsed(DocPosition(0, 5));
      c.setFocusedText('click right here', 11, 11);
      var link = marksFromData(c.nodes.single.data).single;
      expect(c.nodes.single.text.substring(link.start, link.end), 'click right here');

      c.selection = const DocSelection.collapsed(DocPosition(0, 16));
      c.setFocusedText('click right here now', 20, 20);
      link = marksFromData(c.nodes.single.data).single;
      expect(c.nodes.single.text.substring(link.start, link.end), 'click right here');
    });

    test('pasting unmarked text at a link trailing edge does not extend it', () {
      const url = 'https://a.test';
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: url, data: {
          'marks': marksToJson([Mark(0, url.length, 'link', href: url)]),
        }),
      ]);
      c.selection = const DocSelection.collapsed(DocPosition(0, url.length));
      c.insertTextAtCaret(' tail');
      final link = marksFromData(c.nodes.single.data).single;
      expect(c.nodes.single.text.substring(link.start, link.end), url);
    });
  });
}
