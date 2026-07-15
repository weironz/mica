import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/model.dart';

// Paste/copy STRUCTURE fidelity: nested list levels survive every leg of the
// clipboard trip (mica → HTML → markdown → blocks), and blockquote conversion
// keeps block structure while dropping decorative italics (mica quotes are
// upright — bar + muted ink, no slant).

EditorController _doc(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  c.selection = DocSelection(
    anchor: const DocPosition(0, 0),
    focus: DocPosition(nodes.length - 1, nodes.last.text.length),
  );
  return c;
}

List<(String, int)> _listShape(List<BlockSpec> blocks) => [
      for (final b in blocks)
        if (b.kind == 'bulleted_list' || b.kind == 'numbered_list')
          (b.text, (b.data['indent'] as int?) ?? 0),
    ];

void main() {
  group('selectionHtml — nested list levels become nested <ul>/<ol>', () {
    test('indent levels nest inside the parent <li>', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'bulleted_list', text: 'a'),
        EditorNode(id: 'b', kind: 'bulleted_list', text: 'b', data: {'indent': 1}),
        EditorNode(id: 'c', kind: 'bulleted_list', text: 'c', data: {'indent': 2}),
        EditorNode(id: 'd', kind: 'bulleted_list', text: 'd'),
      ]);
      expect(
        c.selectionHtml(),
        '<ul><li>a<ul><li>b<ul><li>c</li></ul></li></ul></li><li>d</li></ul>',
      );
    });

    test('ordered sublist under a bullet keeps both kinds', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'bulleted_list', text: 'a'),
        EditorNode(id: 'b', kind: 'numbered_list', text: 'b', data: {'indent': 1}),
        EditorNode(id: 'c', kind: 'numbered_list', text: 'c', data: {'indent': 1}),
      ]);
      expect(
        c.selectionHtml(),
        '<ul><li>a<ol><li>b</li><li>c</li></ol></li></ul>',
      );
    });

    test('a selection that STARTS on a nested item emits the sublist bare', () {
      final c = _doc([
        EditorNode(id: 'b', kind: 'bulleted_list', text: 'b', data: {'indent': 1}),
        EditorNode(id: 'c', kind: 'bulleted_list', text: 'c'),
      ]);
      expect(c.selectionHtml(), '<ul><li>b</li></ul><ul><li>c</li></ul>');
    });
  });

  group('mica → mica round-trip (the HTML flavor wins on paste)', () {
    test('3-level unordered list survives copy → htmlToMarkdown → parse', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'bulleted_list', text: 'a'),
        EditorNode(id: 'b', kind: 'bulleted_list', text: 'b', data: {'indent': 1}),
        EditorNode(id: 'c', kind: 'bulleted_list', text: 'c', data: {'indent': 2}),
        EditorNode(id: 'd', kind: 'bulleted_list', text: 'd'),
      ]);
      final blocks = markdownToBlocks(htmlToMarkdown(c.selectionHtml()));
      expect(_listShape(blocks), [('a', 0), ('b', 1), ('c', 2), ('d', 0)]);
    });

    test('nested ORDERED list survives the same trip', () {
      final c = _doc([
        EditorNode(id: 'a', kind: 'numbered_list', text: 'a'),
        EditorNode(id: 'b', kind: 'numbered_list', text: 'b', data: {'indent': 1}),
        EditorNode(id: 'c', kind: 'numbered_list', text: 'c', data: {'indent': 1}),
      ]);
      final blocks = markdownToBlocks(htmlToMarkdown(c.selectionHtml()));
      expect(_listShape(blocks), [('a', 0), ('b', 1), ('c', 1)]);
    });
  });

  group('htmlToMarkdown — external nested-list shapes', () {
    test('standard <ul><li><ul> nesting keeps levels', () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        '<ul><li>a<ul><li>b<ul><li>c</li></ul></li></ul></li><li>d</li></ul>',
      ));
      expect(_listShape(blocks), [('a', 0), ('b', 1), ('c', 2), ('d', 0)]);
    });

    test('nested <ol>: child indent reaches the 3-wide `1. ` content column',
        () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        '<ol><li>a<ol><li>b</li></ol></li></ol>',
      ));
      expect(_listShape(blocks), [('a', 0), ('b', 1)],
          reason: 'the old fixed 2-space indent fell short of `1. ` and the '
              'child parsed as a top-level item');
    });

    test('"sibling" nesting (<ul> directly inside <ul> — Word/old editors) '
        'still nests instead of vanishing', () {
      final blocks = markdownToBlocks(htmlToMarkdown(
        '<ul><li>a</li><ul><li>b</li></ul><li>c</li></ul>',
      ));
      expect(_listShape(blocks), [('a', 0), ('b', 1), ('c', 0)]);
    });

    test('Word flat mso-list paragraphs: levels + bullet/number detection', () {
      const word = '''
<p class=MsoListParagraph style='text-indent:-18.0pt;mso-list:l0 level1 lfo1'>
<span style='mso-list:Ignore'>·<span>&nbsp;</span></span>alpha</p>
<p class=MsoListParagraph style='text-indent:-18.0pt;mso-list:l0 level2 lfo1'>
<span style='mso-list:Ignore'>·<span>&nbsp;</span></span>beta</p>
<p class=MsoListParagraph style='text-indent:-18.0pt;mso-list:l1 level1 lfo2'>
<span style='mso-list:Ignore'>1.<span>&nbsp;</span></span>uno</p>
<p>plain tail</p>''';
      final blocks = markdownToBlocks(htmlToMarkdown(word));
      expect(_listShape(blocks), [('alpha', 0), ('beta', 1), ('uno', 0)]);
      expect(blocks.where((b) => b.kind == 'numbered_list').single.text, 'uno');
      expect(blocks.last.kind, 'paragraph',
          reason: 'the paragraph after the list must NOT be lazily absorbed '
              'into the last item');
      expect(blocks.last.text, 'plain tail');
      expect(blocks.any((b) => b.text.contains('·')), isFalse,
          reason: 'the mso-list:Ignore marker glyph is presentation, not text');
    });
  });

  group('htmlToMarkdown — blockquote structure + no italics', () {
    test('multi-paragraph quote keeps its lines (was flattened to one)', () {
      final md = htmlToMarkdown(
        '<blockquote><p>first</p><p>second</p></blockquote>',
      );
      expect(md, '> first\n>\n> second');
      final blocks = markdownToBlocks(md);
      expect(blocks.map((b) => b.kind), everyElement('quote'));
    });

    test('nested blockquote gains a second marker', () {
      final md = htmlToMarkdown(
        '<blockquote><p>outer</p><blockquote><p>inner</p></blockquote></blockquote>',
      );
      expect(md, contains('> outer'));
      expect(md, contains('> > inner'));
    });

    test('decorative italics inside a quote are dropped — tag and style', () {
      final md = htmlToMarkdown(
        '<blockquote><p><i>slanted</i> and '
        '<span style="font-style:italic">styled</span></p></blockquote>',
      );
      expect(md, '> slanted and styled',
          reason: 'sources style quote text italic as decoration; mica '
              'quotes are upright, so no *emphasis* may be synthesized');
    });

    test('bold/links inside a quote are KEPT — only italic is decoration', () {
      final md = htmlToMarkdown(
        '<blockquote><p><b>strong</b> <a href="https://x.io">link</a></p></blockquote>',
      );
      expect(md, '> **strong** [link](https://x.io)');
    });

    test('italic OUTSIDE a quote still converts', () {
      final md = htmlToMarkdown(
        '<blockquote><p><i>a</i></p></blockquote><p><i>b</i></p>',
      );
      expect(md, contains('> a'));
      expect(md, contains('*b*'));
    });

    test('a list inside a quote keeps items on their own quoted lines', () {
      final md = htmlToMarkdown(
        '<blockquote><ul><li>one</li><li>two</li></ul></blockquote>',
      );
      expect(md, contains('> - one'));
      expect(md, contains('> - two'));
    });
  });

  group('plain-markdown text paste (no HTML flavor)', () {
    test('2-space nested bullets parse with levels', () {
      final blocks = markdownToBlocks('- a\n  - b\n    - c');
      expect(_listShape(blocks), [('a', 0), ('b', 1), ('c', 2)]);
    });

    test('tab-indented nested bullets parse with levels', () {
      final blocks = markdownToBlocks('- a\n\t- b');
      expect(_listShape(blocks), [('a', 0), ('b', 1)]);
    });
  });
}
