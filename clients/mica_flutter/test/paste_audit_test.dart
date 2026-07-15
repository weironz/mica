import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

// Regression net for the 25 confirmed findings of the paste/copy adversarial
// audit: escaping fidelity, todo/math/footnote/code-language round trips,
// quote context on copy, Word mso-list corruption, task-list HTML, nested
// tables, and the atomic-block paste guard.

EditorController _doc(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  c.selection = DocSelection(
    anchor: const DocPosition(0, 0),
    focus: DocPosition(nodes.length - 1, nodes.last.text.length),
  );
  return c;
}

List<BlockSpec> _roundTrip(EditorController c) =>
    markdownToBlocks(htmlToMarkdown(c.selectionHtml()));

void main() {
  group('escaping — page text must not re-parse as structure', () {
    test('literal block-leader characters stay literal', () {
      for (final (html, want) in [
        ('<p>* Terms and conditions apply</p>', '* Terms and conditions apply'),
        ('<p># heading text</p>', '# heading text'),
        ('<p>&gt; be me</p>', '> be me'),
        ('<p>1. First point</p>', '1. First point'),
        ('<p>- dash start</p>', '- dash start'),
      ]) {
        final blocks = markdownToBlocks(htmlToMarkdown(html));
        expect(blocks, hasLength(1), reason: html);
        expect(blocks.single.kind, 'paragraph', reason: html);
        expect(blocks.single.text, want, reason: html);
      }
    });

    test('literal inline metacharacters stay literal', () {
      final blocks = markdownToBlocks(
        htmlToMarkdown('<p>match 5*3 and 4*2 or `--flag` and \$5</p>'),
      );
      expect(blocks.single.text, 'match 5*3 and 4*2 or `--flag` and \$5');
      expect(marksFromData(blocks.single.data), isEmpty,
          reason: 'no phantom emphasis/code/math from page-literal characters');
    });

    test('literal <table> text does not become a raw-HTML code block', () {
      final blocks = markdownToBlocks(
        htmlToMarkdown('<p>&lt;table&gt; needs &lt;tr&gt; rows</p>'),
      );
      expect(blocks.single.kind, 'paragraph');
      expect(blocks.single.text, '<table> needs <tr> rows');
    });

    test('newlines inside a heading collapse instead of splitting it', () {
      final blocks = markdownToBlocks(
        htmlToMarkdown('<h2>Getting\n    Started</h2>'),
      );
      expect(blocks, hasLength(1));
      expect(blocks.single.kind, 'heading');
      expect(blocks.single.text, 'Getting Started');
    });
  });

  group('mica → mica round trips (HTML flavor)', () {
    test('todo keeps its kind, checked state, and clean text', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'todo', text: 'buy milk', data: {'checked': true}),
        EditorNode(id: 'b', kind: 'todo', text: 'walk dog'),
      ]);
      final blocks = _roundTrip(c);
      expect(blocks.map((b) => b.kind), everyElement('todo'));
      expect(blocks[0].data['checked'], true);
      expect(blocks[1].data['checked'], isNot(true));
      expect(blocks.map((b) => b.text), ['buy milk', 'walk dog'],
          reason: 'no ☑/☐ glyph may leak into the text');
    });

    test('inline math keeps its mark (not demoted to plain text)', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: 'see E=mc^2 ok', data: {
          'marks': marksToJson([Mark(4, 10, 'math')]),
        }),
      ]);
      final blocks = _roundTrip(c);
      expect(blocks.single.text, 'see E=mc^2 ok');
      final math = marksFromData(blocks.single.data)
          .where((m) => m.type == 'math')
          .single;
      expect((math.start, math.end), (4, 10));
    });

    test('footnote reference keeps its label', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'paragraph', text: 'fact1 end', data: {
          'marks': marksToJson([Mark(4, 5, 'footnote', href: 'n1')]),
        }),
      ]);
      final blocks = _roundTrip(c);
      final fn = marksFromData(blocks.single.data)
          .where((m) => m.type == 'footnote')
          .single;
      expect(fn.href, 'n1');
    });

    test('math block survives as a math block', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'math_block', text: r'\frac{a}{b}'),
      ]);
      final blocks = _roundTrip(c);
      expect(blocks.single.kind, 'math_block');
      expect(blocks.single.text, r'\frac{a}{b}');
    });

    test('footnote definition keeps its label', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'footnote_def', text: 'the note',
            data: {'label': 'n1'}),
      ]);
      final blocks = _roundTrip(c);
      expect(blocks.single.kind, 'footnote_def');
      expect(blocks.single.data['label'], 'n1');
    });

    test('code block keeps its language', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'code_block', text: 'print(1)',
            data: {'language': 'py'}),
      ]);
      final blocks = _roundTrip(c);
      expect(blocks.single.kind, 'code_block');
      expect(blocks.single.data['language'], 'py');
      expect(blocks.single.text, 'print(1)');
    });

    test('heading + list INSIDE a quote keep the quote', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'heading', text: 'Title',
            data: {'level': 1, 'quote': 1}),
        EditorNode(id: 'b', kind: 'bulleted_list', text: 'item',
            data: {'quote': 1}),
      ]);
      final blocks = _roundTrip(c);
      expect(blocks[0].kind, 'heading');
      expect(blocks[0].data['quote'], 1);
      expect(blocks[1].kind, 'bulleted_list');
      expect(blocks[1].data['quote'], 1);
    });

    test('separate quote groups (qbreak) stay separate', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'quote', text: 'group one'),
        EditorNode(id: 'b', kind: 'quote', text: 'group two',
            data: {'qbreak': true}),
      ]);
      final blocks = _roundTrip(c);
      final quotes = blocks.where((b) => b.kind == 'quote').toList();
      expect(quotes, hasLength(2));
      expect(quotes[1].data['qbreak'], true,
          reason: 'two bars must not fuse into one on paste');
    });

    test('depth-2 quote keeps its nesting', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'quote', text: 'deep', data: {'quote': 2}),
      ]);
      final blocks = _roundTrip(c);
      expect(blocks.single.kind, 'quote');
      expect(blocks.single.data['quote'], 2);
    });

    test('ordered list starting at 5 still starts at 5', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'numbered_list', text: 'five',
            data: {'start': 5}),
        EditorNode(id: 'b', kind: 'numbered_list', text: 'six'),
      ]);
      expect(c.selectionHtml(), contains('<ol start="5">'));
      final blocks = _roundTrip(c);
      expect(blocks[0].data['start'], 5);
    });
  });

  group('external HTML shapes', () {
    test('GitHub task-list checkboxes become todos', () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        '<ul class="contains-task-list">'
        '<li class="task-list-item"><input type="checkbox" checked disabled> ship v2</li>'
        '<li class="task-list-item"><input type="checkbox" disabled> write docs</li>'
        '</ul>',
      ));
      expect(blocks.map((b) => b.kind), everyElement('todo'));
      expect(blocks[0].data['checked'], true);
      expect(blocks[0].text, 'ship v2');
      expect(blocks[1].data['checked'], isNot(true));
    });

    test('<pre> inside a list item becomes a code child, not glued lines', () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        '<ul><li>Run: <pre>npm install\nnpm start</pre></li></ul>',
      ));
      expect(blocks[0].kind, 'bulleted_list');
      expect(blocks[0].text, 'Run:');
      final code = blocks.where((b) => b.kind == 'code_block').single;
      expect(code.text, 'npm install\nnpm start');
      expect(code.data['li'], isNotNull,
          reason: 'the fence must attach as the item\'s child');
    });

    test('language class on <pre>/<code> lands in the fence info', () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        '<pre><code class="language-rust">fn main() {}</code></pre>',
      ));
      expect(blocks.single.kind, 'code_block');
      expect(blocks.single.data['language'], 'rust');
    });

    test('nested layout tables do not duplicate rows', () {
      final md = htmlToMarkdown(
        '<table><tr><td><table><tr><td>A</td><td>B</td></tr>'
        '<tr><td>C</td><td>D</td></tr></table></td></tr></table>',
      );
      expect(RegExp(r'^\| A \| B \|', multiLine: true).hasMatch(md), isFalse,
          reason: 'inner rows must not be emitted again as top-level rows');
    });

    test('<dl> terms and definitions get their own blocks', () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        '<dl><dt>HTTP</dt><dd>a protocol</dd><dt>FTP</dt><dd>a file protocol</dd></dl>',
      ));
      expect(blocks, hasLength(4));
      expect(blocks.map((b) => b.text),
          ['HTTP', 'a protocol', 'FTP', 'a file protocol']);
    });
  });

  group('Word mso-list flat paragraphs', () {
    String msoP(int level, String marker, String text) =>
        "<p class=MsoListParagraph style='mso-list:l0 level$level lfo1'>"
        "<span style='mso-list:Ignore'>$marker<span>&nbsp;</span></span>$text</p>";

    test('two consecutive nested items do NOT become a code block', () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        msoP(1, '1.', 'parent') + msoP(2, 'a.', 'kid one') + msoP(2, 'b.', 'kid two'),
      ));
      expect(blocks.where((b) => b.kind == 'code_block'), isEmpty);
      expect(
        [
          for (final b in blocks)
            if (b.kind == 'numbered_list') (b.text, (b.data['indent'] as int?) ?? 0)
        ],
        [('parent', 0), ('kid one', 1), ('kid two', 1)],
      );
    });

    test('a level2-only fragment starts at column 0 (no code, no false nest)',
        () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        '<p>Intro paragraph.</p>${msoP(2, '·', 'child one')}${msoP(2, '·', 'child two')}',
      ));
      expect(blocks.where((b) => b.kind == 'code_block'), isEmpty);
      expect(
        [
          for (final b in blocks)
            if (b.kind == 'bulleted_list') (b.text, (b.data['indent'] as int?) ?? 0)
        ],
        [('child one', 0), ('child two', 0)],
      );
    });

    test('a paragraph after the run is not absorbed into the last item', () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        msoP(1, '·', 'alpha') + '<p>plain tail</p>',
      ));
      expect(blocks.last.kind, 'paragraph');
      expect(blocks.last.text, 'plain tail');
    });
  });

  group('quote-context lists (emitter cap keeps them out of code)', () {
    test('a 3-deep list inside a blockquote never becomes a code block', () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        '<blockquote><ul><li>a<ul><li>b<ul><li>c</li></ul></li></ul></li></ul></blockquote>',
      ));
      expect(blocks.where((b) => b.kind == 'code_block'), isEmpty);
      expect(blocks.map((b) => b.text), containsAll(['a', 'b', 'c']));
    });
  });

  group('plain-text copy fidelity', () {
    test('numbered runs survive an interrupting nested bullet', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'numbered_list', text: 'alpha'),
        EditorNode(id: 'x', kind: 'bulleted_list', text: 'x', data: {'indent': 1}),
        EditorNode(id: 'b', kind: 'numbered_list', text: 'beta'),
      ]);
      expect(c.selectionPlainText(), '1. alpha\n\n  • x\n\n2. beta',
          reason: 'the clipboard numbers must match the rendered numbers');
    });

    test('returning shallower resets the deeper counter', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'numbered_list', text: 'a'),
        EditorNode(id: 'x', kind: 'numbered_list', text: 'x', data: {'indent': 1}),
        EditorNode(id: 'b', kind: 'numbered_list', text: 'b'),
        EditorNode(id: 'y', kind: 'numbered_list', text: 'y', data: {'indent': 1}),
      ]);
      expect(c.selectionPlainText(), '1. a\n\n  1. x\n\n2. b\n\n  1. y');
    });
  });

  test('paste onto an atomic block goes to a paragraph below, not its text',
      () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([
      EditorNode(id: 'img', kind: 'image', text: 'alt text',
          data: {'file_id': 'f1'}),
    ]);
    c.selection = const DocSelection(
      anchor: DocPosition(0, 0),
      focus: DocPosition(0, 0),
    );
    c.insertTextAtCaret('hello');
    expect(c.nodes[0].text, 'alt text', reason: 'alt must not be corrupted');
    expect(c.nodes[1].kind, 'paragraph');
    expect(c.nodes[1].text, 'hello');
  });
}
