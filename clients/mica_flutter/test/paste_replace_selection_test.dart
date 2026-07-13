import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

// Regression: Ctrl+A then paste didn't replace the selection — the pasted
// blocks were appended and the selected content stayed. insertBlocksReplacing-
// Selection (the paste target) must delete the selection first.
EditorController _doc(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  return c;
}

typedef _Spec = ({String kind, String text, Map<String, dynamic> data});
_Spec _p(String text) => (kind: 'paragraph', text: text, data: <String, dynamic>{});

void main() {
  group('insertBlocksReplacingSelection', () {
    test('select-all (Ctrl+A) → paste replaces the whole document', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'heading', text: 'Title', data: {'level': 1}),
        EditorNode(id: 'b', kind: 'paragraph', text: 'body one'),
        EditorNode(id: 'c', kind: 'paragraph', text: 'body two'),
      ]);
      c.selection = DocSelection(
        anchor: const DocPosition(0, 0),
        focus: DocPosition(2, c.nodes[2].text.length),
      );
      c.insertBlocksReplacingSelection([_p('pasted one'), _p('pasted two')]);
      expect(c.nodes.map((n) => n.text).toList(), ['pasted one', 'pasted two']);
      // Leftover empty heading was overwritten, not left dangling above.
      expect(c.nodes.every((n) => n.kind == 'paragraph'), isTrue);
    });

    test('whole single block selected → replaced, not duplicated', () {
      final c = _doc([EditorNode(id: 'a', kind: 'paragraph', text: 'old')]);
      c.selection = DocSelection(
        anchor: const DocPosition(0, 0),
        focus: const DocPosition(0, 3),
      );
      c.insertBlocksReplacingSelection([_p('new')]);
      expect(c.nodes.length, 1);
      expect(c.nodes.single.text, 'new');
    });

    test('collapsed caret → inserts without deleting anything', () {
      final c = _doc([EditorNode(id: 'a', kind: 'paragraph', text: 'keep me')]);
      c.selection = const DocSelection.collapsed(DocPosition(0, 7));
      c.insertBlocksReplacingSelection([_p('added')]);
      expect(c.nodes.map((n) => n.text).toList(), ['keep me', 'added']);
    });

    test('partial in-block selection: the selection is deleted', () {
      final c = _doc([EditorNode(id: 'a', kind: 'paragraph', text: 'hello world')]);
      c.selection = DocSelection(
        anchor: const DocPosition(0, 6),
        focus: const DocPosition(0, 11),
      );
      c.insertBlocksReplacingSelection([_p('X')]);
      // "world" is gone; the remaining "hello " survives; paste follows.
      expect(c.nodes.map((n) => n.text).toList(), ['hello ', 'X']);
    });
  });
}
