import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/model.dart';

EditorController _doc(List<EditorNode> nodes, {int focus = 0}) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  c.selection = DocSelection(
    anchor: DocPosition(focus, 0),
    focus: DocPosition(focus, 0),
  );
  return c;
}

EditorNode _li(String id, String text, {int indent = 0, String kind = 'bulleted_list'}) {
  return EditorNode(
    id: id,
    kind: kind,
    text: text,
    data: {if (indent > 0) 'indent': indent},
  );
}

void main() {
  test('Tab indents one level under the previous item, clamped', () {
    final c = _doc([_li('a', 'parent'), _li('b', 'child')], focus: 1);
    expect(c.indentSelection(1), isTrue);
    expect(c.nodes[1].indent, 1);
    // Can't go two levels deeper than the parent.
    expect(c.indentSelection(1), isFalse);
    expect(c.nodes[1].indent, 1);
  });

  test('first list item cannot indent (no list above)', () {
    final c = _doc([_li('a', 'only')], focus: 0);
    expect(c.indentSelection(1), isFalse);
    expect(c.nodes[0].indent, 0);
  });

  test('Shift+Tab outdents and removes the data key at level 0', () {
    final c = _doc([_li('a', 'p'), _li('b', 'c', indent: 1)], focus: 1);
    expect(c.indentSelection(-1), isTrue);
    expect(c.nodes[1].indent, 0);
    expect(c.nodes[1].data.containsKey('indent'), isFalse);
  });

  test('multi-line selection indents every covered list item', () {
    final c = _doc([
      _li('a', 'parent'),
      _li('b', 'one'),
      _li('c', 'two', kind: 'todo'),
      EditorNode(id: 'd', kind: 'paragraph', text: 'not a list'),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(1, 0),
      focus: DocPosition(3, 0),
    );
    expect(c.indentSelection(1), isTrue);
    expect(c.nodes[1].indent, 1);
    expect(c.nodes[2].indent, 1); // todo nests too (chained clamp)
    expect(c.nodes[3].data.containsKey('indent'), isFalse); // paragraph untouched
  });

  test('Enter on a nested todo keeps the level, unchecks', () {
    final c = _doc([
      _li('a', 'parent'),
      _li('b', 'task', indent: 1, kind: 'todo'),
    ], focus: 1);
    c.nodes[1].data = {'indent': 1, 'checked': true};
    c.selection = const DocSelection(
      anchor: DocPosition(1, 4),
      focus: DocPosition(1, 4),
    );
    c.splitAtCaret();
    expect(c.nodes[2].kind, 'todo');
    expect(c.nodes[2].indent, 1);
    expect(c.nodes[2].data['checked'], false);
  });

  test('markdownToBlocks parses nesting; copy prefix round-trips it', () {
    final specs = markdownToBlocks('- a\n  - b\n    1. c\n- d');
    expect([for (final s in specs) s.data['indent'] ?? 0], [0, 1, 2, 0]);
    final c = _doc([
      for (final s in specs)
        EditorNode(id: s.text, kind: s.kind, text: s.text, data: {...s.data}),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(3, 1),
    );
    // Assert the INVARIANT (the copy re-parses to the same nesting), not one
    // exact byte string. The old assertion pinned a blank line between every
    // item — which is what makes a list LOOSE in CommonMark — and so
    // contradicted this test's own name. Items of one kind now stay tight;
    // a kind change (bullets → numbers) still gets its blank, because without
    // it the two lists merge on paste.
    final copied = c.selectionText();
    expect(copied, '- a\n    - b\n\n        1. c\n\n- d');
    expect(
      [for (final s in markdownToBlocks(copied)) s.data['indent'] ?? 0],
      [0, 1, 2, 0],
      reason: 'copy → paste must preserve the nesting it started with',
    );
  });
}
