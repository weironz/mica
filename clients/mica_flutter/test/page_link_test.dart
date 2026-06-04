import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

EditorController _doc(String text, int caret) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load([EditorNode(id: 'a', kind: 'paragraph', text: text)]);
  c.selection = DocSelection(
    anchor: DocPosition(0, caret),
    focus: DocPosition(0, caret),
  );
  return c;
}

void main() {
  test('insertPageLink replaces the [[query with a linked title', () {
    final c = _doc('see [[gui please', 9); // caret after "[[gui"
    c.insertPageLink(4, 9, 'Guide', 'mica://page/v1');
    final node = c.nodes.single;
    expect(node.text, 'see Guide please');
    final link = marksFromData(node.data).single;
    expect(link.type, 'link');
    expect(link.href, 'mica://page/v1');
    expect(node.text.substring(link.start, link.end), 'Guide');
    // Caret lands right after the link.
    expect(c.selection!.focus.offset, 4 + 'Guide'.length);
  });

  test('insertPageLink shifts existing marks across the replacement', () {
    final c = _doc('bold [[x', 8);
    // "bold" is bold.
    c.nodes.single.data = {
      'marks': marksToJson([Mark(0, 4, 'bold')]),
    };
    c.insertPageLink(5, 8, 'Very Long Title', 'mica://page/v2');
    final node = c.nodes.single;
    expect(node.text, 'bold Very Long Title');
    final marks = marksFromData(node.data);
    final bold = marks.firstWhere((m) => m.type == 'bold');
    expect([bold.start, bold.end], [0, 4]); // untouched before the edit
    final link = marks.firstWhere((m) => m.type == 'link');
    expect(node.text.substring(link.start, link.end), 'Very Long Title');
  });

  test('setLinkRange edits and removes a link without touching text', () {
    final c = _doc('see Guide here', 0);
    c.nodes.single.data = {
      'marks': marksToJson([Mark(4, 9, 'link', href: 'https://old.example')]),
    };
    // Edit: same range, new href.
    c.setLinkRange(0, 4, 9, 'mica://page/v9');
    var link = marksFromData(c.nodes.single.data).single;
    expect(link.href, 'mica://page/v9');
    expect([link.start, link.end], [4, 9]);
    expect(c.nodes.single.text, 'see Guide here');
    // Remove: link mark gone, text intact.
    c.setLinkRange(0, 4, 9, null);
    expect(
      marksFromData(c.nodes.single.data).where((m) => m.type == 'link'),
      isEmpty,
    );
    expect(c.nodes.single.text, 'see Guide here');
  });

  test('insertPageLink refuses code blocks', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([EditorNode(id: 'a', kind: 'code_block', text: '[[x')]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 3),
      focus: DocPosition(0, 3),
    );
    c.insertPageLink(0, 3, 'Guide', 'mica://page/v1');
    expect(c.nodes.single.text, '[[x'); // unchanged
  });
}
