import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/cell_edit_controller.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

void main() {
  late EditorController c;
  setUp(() {
    c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([
      EditorNode(
        id: 'p',
        kind: 'paragraph',
        text: 'run a`b then `literal`',
        data: {
          'marks': marksToJson([Mark(4, 7, 'code')]),
        },
      ),
      EditorNode(
        id: 'code',
        kind: 'code_block',
        text: '# comment\n``` literal',
        data: {'language': 'bash'},
      ),
    ]);
  });
  tearDown(() => c.dispose());

  for (final range in [(4, 7), (5, 7), (0, 22)]) {
    test('plain/rich flavors for inline code selection $range', () {
      final end = range.$2.clamp(0, c.nodes.first.text.length);
      c.selection = DocSelection(
        anchor: DocPosition(0, range.$1),
        focus: DocPosition(0, end),
      );
      final text = c.nodes.first.text.substring(range.$1, end);
      expect(c.selectionClipboardText(), text);
      final pasted = markdownToBlocks(htmlToMarkdown(c.selectionHtml())).single;
      expect(pasted.text, text);
      expect(
        marksFromData(pasted.data).where((m) => m.type == 'code'),
        isNotEmpty,
      );
      // Reversed selections must use the same slices.
      c.selection = DocSelection(
        anchor: DocPosition(0, end),
        focus: DocPosition(0, range.$1),
      );
      expect(c.selectionClipboardText(), text);
    });
  }

  test('cross-block copy has literal code, rich paste retains code blocks', () {
    c.selection = DocSelection(
      anchor: const DocPosition(0, 0),
      focus: DocPosition(1, c.nodes[1].text.length),
    );
    expect(
      c.selectionClipboardText(),
      'run a`b then `literal`\n\n# comment\n``` literal',
    );
    final blocks = markdownToBlocks(htmlToMarkdown(c.selectionHtml()));
    expect(blocks.map((b) => b.kind), ['paragraph', 'code_block']);
    expect(blocks.last.text, c.nodes.last.text);
    expect(marksFromData(blocks.first.data).single.type, 'code');
  });

  test('table atomic selection emits clean TSV and rich table marks', () {
    c.load([
      EditorNode(
        id: 't',
        kind: 'table',
        text: '',
        data: {
          'rows': [
            ['`code`', '**bold**'],
            [r'\`literal\`', '中文'],
          ],
        },
      ),
    ]);
    c.collapseTo(const DocPosition(0, 0));
    expect(c.selectionClipboardText(), 'code\tbold\n`literal`\t中文');
    expect(c.selectionHtml(), contains('<code>code</code>'));
    expect(c.selectionHtml(), contains('<strong>bold</strong>'));
  });

  test('collapsed text copies nothing; atomic image still copies URL', () {
    c.collapseTo(const DocPosition(0, 3));
    expect(c.selectionClipboardText(), isEmpty);
    c.load([
      EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f'}),
    ]);
    c.collapseTo(const DocPosition(0, 0));
    expect(
      c.selectionClipboardText(imageUrls: {'f': 'https://test/image'}),
      'https://test/image',
    );
  });

  test(
    'hard-break markers disappear but code and LaTeX source stay literal',
    () {
      const source = 'a\\\nb';
      c.load([EditorNode(id: 'p', kind: 'paragraph', text: source)]);
      c.selection = const DocSelection(
        anchor: DocPosition(0, 0),
        focus: DocPosition(0, 4),
      );
      expect(c.selectionClipboardText(), 'a\nb');
      c.selection = const DocSelection(
        anchor: DocPosition(0, 0),
        focus: DocPosition(0, 2),
      );
      expect(c.selectionClipboardText(), 'a');
      c.nodes.first.data = {
        'marks': marksToJson([Mark(0, 4, 'code')]),
      };
      c.selection = const DocSelection(
        anchor: DocPosition(0, 0),
        focus: DocPosition(0, 4),
      );
      expect(c.selectionClipboardText(), source);
      const latex = 'a &= b \\\\\nc &= d';
      c.load([EditorNode(id: 'm', kind: 'math_block', text: latex)]);
      c.collapseTo(const DocPosition(0, 0));
      expect(c.selectionClipboardText(), latex);
    },
  );

  test(
    'cell selection clips actual code marks and preserves literal syntax',
    () {
      final cell = CellEditController(r'run `a` then \`literal\`');
      addTearDown(cell.dispose);
      cell.selection = TextSelection(
        baseOffset: 0,
        extentOffset: cell.text.length,
      );
      expect(cell.selection.textInside(cell.text), 'run a then `literal`');
      final html = cell.selectionHtml();
      expect(html, 'run <code>a</code> then `literal`');
      final parsed = parseInline(htmlToMarkdown(html));
      expect(parsed.text, cell.text);
      expect(parsed.marks.single.type, 'code');
      cell.selection = const TextSelection(baseOffset: 4, extentOffset: 5);
      expect(cell.selectionHtml(), '<code>a</code>');
    },
  );
}
