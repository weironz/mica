import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart';
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
  test('copy emits block-level Markdown markers', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'heading', text: 'Title', data: {'level': 2}),
      EditorNode(id: 'b', kind: 'bulleted_list', text: 'one'),
      EditorNode(id: 'c', kind: 'todo', text: 'do it', data: {'checked': true}),
      EditorNode(id: 'd', kind: 'quote', text: 'wise words'),
    ]);
    final md = c.selectionText();
    expect(md, contains('## Title'));
    expect(md, contains('- one'));
    expect(md, contains('- [x] do it'));
    expect(md, contains('> wise words'));
  });

  test('copy fences a fully-selected code block', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'code_block', text: 'print(1)', data: {'language': 'py'}),
    ]);
    expect(c.selectionText(), '```py\nprint(1)\n```');
  });

  test('copy keeps inline marks', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'paragraph', text: 'hi there', data: {
        'marks': marksToJson([Mark(0, 2, 'bold')]),
      }),
    ]);
    expect(c.selectionText(), '**hi** there');
  });

  test('a quote group copies as ONE blockquote and round-trips intact', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'quote', text: 'first line'),
      EditorNode(id: 'b', kind: 'quote', text: 'second line'),
      EditorNode(id: 'c', kind: 'quote', text: 'third line'),
    ]);
    final md = c.selectionText();
    expect(md, '> first line\n> second line\n> third line',
        reason: 'a blank line would SPLIT the blockquote on re-parse and '
            'shatter the quote bar');

    // Re-parsing folds the group into one multi-line quote block — still a
    // single continuous bar, which is the point.
    final blocks = markdownToBlocks(md);
    expect(blocks.map((b) => b.kind), ['quote']);
    expect(blocks.single.text, 'first line\nsecond line\nthird line');
    expect(blocks.where((b) => b.data['qbreak'] == true), isEmpty,
        reason: 'pasted back, the group must stay one continuous bar');
  });

  test('separate blockquotes (qbreak) keep their blank line apart', () {
    final c = _doc([
      EditorNode(id: 'a', kind: 'quote', text: 'group one'),
      EditorNode(id: 'b', kind: 'quote', text: 'group two', data: {'qbreak': true}),
    ]);
    expect(c.selectionText(), '> group one\n\n> group two');
  });

  test('partial single-block selection has no block marker', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([EditorNode(id: 'a', kind: 'heading', text: 'Title', data: {'level': 1})]);
    c.selection = DocSelection(
      anchor: const DocPosition(0, 0),
      focus: const DocPosition(0, 3), // "Tit"
    );
    expect(c.selectionText(), 'Tit');
  });

  // The clipboard's text/plain flavor: what a PLAIN editor (Notepad) reads —
  // Markdown syntax stripped, only rendered affordances (bullet/number/box) kept.
  group('selectionPlainText — no Markdown syntax', () {
    test('inline marks + block markers are gone; text/bullets rendered', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'heading', text: 'Title', data: {'level': 2}),
        EditorNode(id: 'b', kind: 'paragraph', text: 'hi there', data: {
          'marks': marksToJson([Mark(0, 2, 'bold'), Mark(3, 8, 'code')]),
        }),
        EditorNode(id: 'c', kind: 'bulleted_list', text: 'one'),
        EditorNode(id: 'd', kind: 'quote', text: 'wise words'),
      ]);
      final plain = c.selectionPlainText();
      expect(plain, isNot(contains('**')));
      expect(plain, isNot(contains('`')));
      expect(plain, isNot(contains('#')));
      expect(plain, isNot(contains('> ')));
      expect(plain, contains('Title'));
      expect(plain, contains('hi there')); // marks live off the text → already clean
      expect(plain, contains('• one')); // rendered bullet, not Markdown "- "
      expect(plain, contains('wise words'));
    });

    test('a link keeps its text and drops the URL', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: 'see docs', data: {
          'marks': marksToJson([Mark(4, 8, 'link', href: 'https://x.dev')]),
        }),
      ]);
      expect(c.selectionPlainText(), 'see docs');
    });

    test('a numbered list renders running numbers', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'numbered_list', text: 'first'),
        EditorNode(id: 'b', kind: 'numbered_list', text: 'second'),
      ]);
      expect(c.selectionPlainText(), '1. first\n\n2. second');
    });
  });

  // The clipboard's text/html flavor: Typora/Obsidian read this and convert it
  // back to formatted content, so copy→paste there keeps the formatting.
  group('selectionHtml — rich flavor', () {
    test('inline marks become HTML tags', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: 'hi there', data: {
          'marks': marksToJson([Mark(0, 2, 'bold'), Mark(3, 8, 'code')]),
        }),
      ]);
      final html = c.selectionHtml();
      expect(html, contains('<strong>hi</strong>'));
      expect(html, contains('<code>there</code>'));
    });

    test('a link becomes an anchor with href', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: 'see docs', data: {
          'marks': marksToJson([Mark(4, 8, 'link', href: 'https://x.dev')]),
        }),
      ]);
      expect(c.selectionHtml(), contains('<a href="https://x.dev">docs</a>'));
    });

    test('heading + a grouped bullet list + a grouped quote', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'heading', text: 'Title', data: {'level': 2}),
        EditorNode(id: 'b', kind: 'bulleted_list', text: 'one'),
        EditorNode(id: 'c', kind: 'bulleted_list', text: 'two'),
        EditorNode(id: 'd', kind: 'quote', text: 'q1'),
        EditorNode(id: 'e', kind: 'quote', text: 'q2'),
      ]);
      final html = c.selectionHtml();
      expect(html, contains('<h2>Title</h2>'));
      expect(html, contains('<ul><li>one</li><li>two</li></ul>'));
      expect(html, contains('<blockquote><p>q1</p><p>q2</p></blockquote>'));
    });

    test('entity-escapes reserved characters', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: 'a < b & c'),
      ]);
      expect(c.selectionHtml(), '<p>a &lt; b &amp; c</p>');
    });
  });
}
