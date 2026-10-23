// Mirrors crates/markdown/tests/image_alt_export.rs.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

void main() {
  group('image alt export', () {
    const cases = [
      // Issue #58's original short examples, with explicit image syntax.
      (r'pre ![\[x\](u)](v) post', r'pre ![\[x\](u)](v) post'),
      ('pre ![[a](u)](v) post', 'pre ![[a](u)](v) post'),
      ('pre ![a [b] c](/img) post', r'pre ![a \[b\] c](/img) post'),
      ('pre ![[x](/alt)](/img) post', 'pre ![[x](/alt)](/img) post'),
      ('pre ![a [b](/alt) c](/img) post', 'pre ![a [b](/alt) c](/img) post'),
      (r'pre ![\[x\](u)](/img) post', r'pre ![\[x\](u)](/img) post'),
      (
        r'pre ![left \[ and right \]](/img) post',
        r'pre ![left \[ and right \]](/img) post',
      ),
      (r'pre ![\\\[x\]](/img) post', r'pre ![\\\[x\]](/img) post'),
      (
        'pre ![[x][alt]][image] post\n\n[alt]: /alt\n[image]: /img',
        'pre ![[x](/alt)](/img) post',
      ),
      (
        'pre ![**bold** *italic* `code`](/img) post',
        'pre ![**bold** *italic* `code`](/img) post',
      ),
      ('[![x](/img)](/outer)', '[![x](/img)](/outer)'),
      ('pre [![x](/img)](/outer) post', 'pre [![x](/img)](/outer) post'),
      (
        'pre ![[x](/alt)](/img "image title") post',
        'pre ![[x](/alt)](/img "image title") post',
      ),
      (
        'pre [![😀](/img "image title")](/outer "link title") post',
        'pre [![😀](/img "image title")](/outer "link title") post',
      ),
      ('😀 pre ![[😺](/alt)](/img) post', '😀 pre ![[😺](/alt)](/img) post'),
    ];
    for (final (src, expected) in cases) {
      test(src, () {
        final block = markdownToBlocks(src).single;
        expect(block.kind, 'paragraph');
        final marks = marksFromData(block.data);
        final md = inlineToMarkdown(block.text, marks);
        expect(md, expected);
        final again = markdownToBlocks(md).single;
        expect(again.text, block.text);
        expect(marksToJson(marksFromData(again.data)), marksToJson(marks));
        expect(inlineToMarkdown(again.text, marksFromData(again.data)), md);
        final html = inlineToHtml(block.text, marks);
        final imageUrl = src.contains('](v)') ? 'v' : '/img';
        expect(html, contains('<img src="$imageUrl"'));
        expect(html, isNot(contains('href="/alt"')));
        expect(html, isNot(contains('href="u"')));
        if (src.contains('/outer')) {
          expect(html, contains('<a href="/outer"><img'));
        }
      });
    }

    for (final src in [
      '![[x](/alt)](/img)',
      '![[x][alt]][image]\n\n[alt]: /alt\n[image]: /img',
    ]) {
      test('standalone image flattens alt links: $src', () {
        final image = markdownToBlocks(src).single;
        expect(image.kind, 'image');
        expect(image.text, 'x');
        expect(image.data['url'], '/img');
      });
    }

    for (final alt in [
      'a [b] c',
      '[x](/alt)',
      '😀 [😺] *literal*',
      r'\[x]',
      '[',
      ']',
      '',
    ]) {
      test('image block escapes literal alt: $alt', () {
        final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
        addTearDown(c.dispose);
        c.load([
          EditorNode(
            id: 'image',
            kind: 'image',
            text: alt,
            data: {'url': '/img', 'title': 'image title'},
          ),
          EditorNode(id: 'after', kind: 'paragraph', text: 'after'),
        ]);
        c.selection = const DocSelection(
          anchor: DocPosition(0, 0),
          focus: DocPosition(1, 5),
        );
        final md = c.selectionText();
        expect(md, startsWith('![${escapeInline(alt)}](/img "image title")'));
        final image = markdownToBlocks(md).first;
        expect(image.kind, 'image');
        expect(image.text, alt);
        expect(image.data['url'], '/img');
        expect(image.data['title'], 'image title');
      });
    }
  });
}
