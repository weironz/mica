import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorController _fresh(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  return c;
}

void main() {
  test('typing `# ` on an existing heading re-levels it', () {
    final c = _fresh([
      EditorNode(id: 'h', kind: 'heading', text: 'My Title', data: {'level': 2}),
    ]);
    // Type "# " at the start of the H2: text becomes "# My Title".
    c.selection = DocSelection.collapsed(const DocPosition(0, 0));
    c.nodes[0].text = '# My Title';
    c.selection = DocSelection.collapsed(const DocPosition(0, 2));
    expect(c.applyInputRules(), isTrue);
    expect(c.nodes[0].kind, 'heading');
    expect(c.nodes[0].data['level'], 1, reason: 'H2 re-levels to H1');
    expect(c.nodes[0].text, 'My Title', reason: 'the marker is consumed');

    // And deeper: "### " on the (now) H1.
    c.nodes[0].text = '### My Title';
    c.selection = DocSelection.collapsed(const DocPosition(0, 4));
    expect(c.applyInputRules(), isTrue);
    expect(c.nodes[0].data['level'], 3);
  });

  test('Backspace at a heading start rises over an empty line, format kept', () {
    final c = _fresh([
      EditorNode(id: 'p1', kind: 'paragraph', text: 'first line'),
      EditorNode(id: 'p2', kind: 'paragraph', text: ''),
      EditorNode(id: 'h', kind: 'heading', text: 'My Title', data: {'level': 2}),
    ]);
    c.selection = DocSelection.collapsed(const DocPosition(2, 0));

    // The empty line above is consumed; the heading moves up UNCHANGED.
    expect(c.mergeBackward(), isTrue);
    expect(c.nodes.length, 2);
    expect(c.nodes[1].kind, 'heading');
    expect(c.nodes[1].text, 'My Title');
    expect(c.nodes[1].data['level'], 2, reason: 'format survives');
    expect(c.selection!.focus, const DocPosition(1, 0));

    // Against a NON-empty line, the standard text merge applies.
    expect(c.mergeBackward(), isTrue);
    expect(c.nodes.length, 1);
    expect(c.nodes[0].text, 'first lineMy Title');
  });

  test('Backspace at a list item start sheds the marker, then merges', () {
    final c = _fresh([
      EditorNode(id: 'p', kind: 'paragraph', text: ''),
      EditorNode(id: 'li', kind: 'numbered_list', text: 'item one'),
      EditorNode(id: 'td', kind: 'todo', text: 'task', data: {'checked': true}),
    ]);

    // Numbered item: first Backspace strips the number IN PLACE.
    c.selection = DocSelection.collapsed(const DocPosition(1, 0));
    expect(c.mergeBackward(), isTrue);
    expect(c.nodes.length, 3, reason: 'no structural change yet');
    expect(c.nodes[1].kind, 'paragraph');
    expect(c.nodes[1].text, 'item one');
    expect(c.selection!.focus, const DocPosition(1, 0));

    // Second Backspace: now a paragraph, it rises over the empty line.
    expect(c.mergeBackward(), isTrue);
    expect(c.nodes.length, 2);
    expect(c.nodes[0].kind, 'paragraph');
    expect(c.nodes[0].text, 'item one');

    // Todo: the checkbox state goes with the marker.
    c.selection = DocSelection.collapsed(const DocPosition(1, 0));
    expect(c.mergeBackward(), isTrue);
    expect(c.nodes[1].kind, 'paragraph');
    expect(c.nodes[1].data.containsKey('checked'), isFalse);
  });

  test('```yaml⏎ — first-line language tag becomes the block language', () {
    final c = _fresh([
      EditorNode(id: 'c', kind: 'code_block', text: 'yaml'),
    ]);
    // The IME inserted "yaml\n"; insertCodeNewline sees caret 5.
    c.selection = DocSelection.collapsed(const DocPosition(0, 4));
    c.insertCodeNewline(5);
    expect(c.nodes[0].data['language'], 'yaml');
    expect(c.nodes[0].text, '', reason: 'the tag line is consumed');
    expect(c.selection!.focus, const DocPosition(0, 0));

    // Plain text that is NOT a language stays a soft newline.
    c.nodes[0].text = 'hello';
    c.selection = DocSelection.collapsed(const DocPosition(0, 5));
    c.insertCodeNewline(6);
    expect(c.nodes[0].text, 'hello\n');
    expect(c.nodes[0].data['language'], 'yaml', reason: 'unchanged');
  });

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

  test('second Enter inside a pasted multi-line quote exits in place', () {
    // Paste folds a quote group into ONE multi-line block. Enter at the end
    // of its first line splits off a block LEADING with the old soft break
    // — a "barred empty line". The next Enter must turn THAT line into a
    // plain paragraph (caret staying put), not stack paragraphs above it.
    final c = _fresh([
      EditorNode(id: 'q', kind: 'quote', text: 'a\nb'),
    ]);
    // First Enter at the end of line "a" (caret offset 1).
    c.selection = DocSelection.collapsed(const DocPosition(0, 1));
    c.applyNewlineSplit('a', '\nb');
    expect(c.nodes.map((n) => n.text), ['a', '\nb']);
    expect(c.nodes[1].kind, 'quote');
    expect(c.selection!.focus, const DocPosition(1, 0),
        reason: 'caret sits on the barred empty line');

    // Second Enter on that empty line.
    c.applyNewlineSplit('', '\nb');
    expect(c.nodes.map((n) => n.kind), ['quote', 'paragraph', 'quote'],
        reason: 'the empty line leaves the quote as a plain paragraph');
    expect(c.nodes.map((n) => n.text), ['a', '', 'b']);
    expect(c.selection!.focus, const DocPosition(1, 0),
        reason: 'the caret STAYS on the now-plain line');
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
