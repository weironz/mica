import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorController _fresh(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  return c;
}

void main() {
  // ---------------------------------------------------------------------------
  // Quote continues on Enter
  // ---------------------------------------------------------------------------
  test('Enter inside a quote stays a quote (carries depth)', () {
    final c = _fresh([EditorNode(id: 'q', kind: 'quote', text: 'first line')]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 10));
    c.splitAtCaret();
    expect(c.nodes.length, 2);
    expect(c.nodes[1].kind, 'quote', reason: 'the new line continues the quote');
    expect(c.nodes[1].quoteDepth >= 1, isTrue);
    expect(c.selection!.focus.node, 1);
  });

  test('Enter at the end of a group-opening quote keeps the bar continuous',
      () {
    // The first block of a SECOND blockquote group carries qbreak. Pressing
    // Enter at its end must NOT hand qbreak (or the parent marks) to the
    // continuation line — that severed the bar exactly at the split.
    final c = _fresh([
      EditorNode(id: 'q1', kind: 'quote', text: 'group one'),
      EditorNode(
        id: 'q2',
        kind: 'quote',
        text: 'group two opener',
        data: {'qbreak': true, 'marks': []},
      ),
    ]);
    c.selection = DocSelection.collapsed(
        DocPosition(1, c.nodes[1].text.length));
    c.applyNewlineSplit('group two opener', '');

    expect(c.nodes[2].kind, 'quote');
    expect(c.nodes[2].data['qbreak'], isNull,
        reason: 'the continuation stays in the same group — no bar break');
    expect(c.nodes[2].data['marks'], isNull);
    expect(c.nodes[1].data['qbreak'], true,
        reason: 'the opener itself keeps starting its group');
  });

  test('Enter on an empty quote line exits to a paragraph', () {
    final c = _fresh([EditorNode(id: 'q', kind: 'quote', text: '')]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 0));
    c.splitAtCaret();
    expect(c.nodes.length, 1, reason: 'no new block; the quote becomes a para');
    expect(c.nodes[0].kind, 'paragraph');
  });

  test('IME newline path also continues the quote', () {
    final c = _fresh([EditorNode(id: 'q', kind: 'quote', text: 'abcd')]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 2));
    c.applyNewlineSplit('ab', 'cd');
    expect(c.nodes.length, 2);
    expect(c.nodes[0].kind, 'quote');
    expect(c.nodes[1].kind, 'quote');
    expect(c.nodes[1].text, 'cd');
  });

  // ---------------------------------------------------------------------------
  // Code block Enter auto-indent
  // ---------------------------------------------------------------------------
  test('code newline copies the previous line indentation', () {
    final c = _fresh([
      EditorNode(id: 'c', kind: 'code_block', text: '    foo'),
    ]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 7));
    // The IME inserted a '\n' at offset 7 → caret 8 in the new (newline) text.
    c.insertCodeNewline(8);
    expect(c.nodes[0].text, '    foo\n    ',
        reason: 'the new line copies the 4-space indent');
    expect(c.selection!.focus.offset, '    foo\n    '.length);
  });

  test('code newline mid-text keeps the tail and indents it', () {
    final c = _fresh([
      EditorNode(id: 'c', kind: 'code_block', text: '\tif x:\n\t\tbody'),
    ]);
    // Break right after the colon: '\tif x:' then newline.
    const breakAt = 6; // before '\n'
    c.selection = const DocSelection.collapsed(DocPosition(0, breakAt));
    c.insertCodeNewline(breakAt + 1);
    expect(c.nodes[0].text, '\tif x:\n\t\n\t\tbody',
        reason: 'a tab-indented line begets a tab-indented blank line');
  });

  test('code newline on an unindented line adds no indent', () {
    final c = _fresh([
      EditorNode(id: 'c', kind: 'code_block', text: 'foo'),
    ]);
    c.selection = const DocSelection.collapsed(DocPosition(0, 3));
    c.insertCodeNewline(4);
    expect(c.nodes[0].text, 'foo\n');
  });
}
