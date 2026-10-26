import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/properties.dart';

void main() {
  test('unmarked punctuation keeps the plain serializer contract', () {
    const text = '<]`\n;';
    expect(inlineToMarkdown(text, []), text);
    expect(inlineToMarkdown('&amp;', []), '&amp;amp;');
  });
  test('Unicode indentation uses the same columns for list code', () {
    for (final whitespace in [' ', '\t', '\u00a0', '\u1680', '\u2003', '\u202f', '\u205f', '\u3000']) {
      final blocks = markdownToBlocks('- a\n${whitespace * 6}b');
      expect(blocks.last.kind, 'code_block');
      expect(blocks.last.text.endsWith('b'), isTrue);
    }
  });

  test('unclosed math keeps following lines', () {
    for (final opener in [r'$$', r'\[']) {
      final block = markdownToBlocks('$opener\nsecret\nmore').single;
      expect(block.text, '${opener == r'\[' ? '[' : opener}\nsecret\nmore');
    }
  });

  test('property lists quote every comma', () {
    for (final item in ['a,b', 'a, b', 'a,,b', ',a', 'a,']) {
      final value = PropList([item]);
      expect(parseProperties(upsertProperty('', 'tags', value)), [Property('tags', value)]);
    }
  });
  test('code padding preserves whitespace payloads and ranges', () {
    for (final input in ['`  \t  `', 'x `  \t  ` y', '`\t `', '` \t`', '`  \u2003  `', '`\u2003`', '`   `']) {
      final first = markdownToBlocks(input).single;
      final exported = inlineToMarkdown(first.text, marksFromData(first.data));
      final again = markdownToBlocks(exported).single;
      expect(again.text, first.text, reason: input);
      expect(again.data, first.data, reason: input);
    }
  });
  test('inline exports preserve entities math strike and empty links', () {
    for (final input in ['&amp;amp;', r'\(a$b\)', '~~`a~~b`~~', 'a[](/u)b', '[](/u)', 'a[](/u)', '[](/u)a', '**a[](/u)b**', 'a[](/u)[](/v)b', r'[x](a\)b)', r'[x](a\\b)', r'[x](u "a\\")', '[x](a&amp;amp;b)']) {
      final first = markdownToBlocks(input).single;
      final exported = inlineToMarkdown(first.text, marksFromData(first.data));
      final again = markdownToBlocks(exported).single;
      expect(again.text, first.text, reason: '$input -> $exported');
      expect(again.data, first.data, reason: '$input -> $exported');
    }
  });
  test('quoted fences share top-level rules', () {
    for (final input in ['> ````\n> ```\n> ````', '> ~~~\n> x\n> ~~~']) {
      final code = [
        for (final b in markdownToBlocks(input))
          if (b.kind == 'code_block') b,
      ];
      expect(code, hasLength(1), reason: input);
      expect(code.single.data['quote'], 1);
    }
  });

  test('copied code block fences never conflict with content', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    const codeText = '````\n```\n````';
    c.load([
      EditorNode(id: 'b0', kind: 'code_block', text: codeText, data: {'language': 'rust'}),
    ]);
    c.selection = DocSelection(
      anchor: const DocPosition(0, 0),
      focus: DocPosition(0, codeText.length),
    );
    final md = c.selectionText();
    expect(md.startsWith('`````rust'), isTrue, reason: md);
    expect(md.endsWith('`````'), isTrue, reason: md);
    final again = markdownToBlocks(md);
    expect(again.single.kind, 'code_block');
    expect(again.single.text, codeText);
  });
  test('zero-width link at a shorthand start survives export', () {
    final plain = markdownToBlocks('[](x)www.example.com').single;
    expect(
      inlineToMarkdown(plain.text, marksFromData(plain.data)),
      '[](x)www.example.com',
    );
    String exportedFor(String input) {
      final first = markdownToBlocks(input).single;
      return inlineToMarkdown(first.text, marksFromData(first.data));
    }

    int links(String markdown) => marksFromData(
          markdownToBlocks(markdown).single.data,
        ).where((m) => m.type == 'link').length;

    const angle = '[](x)<https://e.com>';
    expect(links(angle), 2);
    final exported = exportedFor(angle);
    expect(links(exported), 2, reason: exported);
    expect(exported, contains('https://e.com'));
  });
}
