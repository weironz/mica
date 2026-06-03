import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/model.dart';

void main() {
  test('markdownToBlocks parses ![alt](url) into an image block', () {
    final blocks = markdownToBlocks('![a cat](https://x.io/cat.png)');
    expect(blocks.length, 1);
    expect(blocks.first.kind, 'image');
    expect(blocks.first.text, 'a cat');
    expect(blocks.first.data['url'], 'https://x.io/cat.png');
  });

  test('image among other blocks', () {
    final blocks = markdownToBlocks('# Title\n\n![](https://x.io/a.jpg)\n\nbody');
    expect(blocks.map((b) => b.kind).toList(),
        ['heading', 'image', 'paragraph']);
  });

  test('copying an image node yields markdown', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([
      EditorNode(id: 'a', kind: 'paragraph', text: 'before'),
      EditorNode(
        id: 'b',
        kind: 'image',
        text: 'cat',
        data: {'url': 'https://x.io/cat.png'},
      ),
    ]);
    c.selection = DocSelection(
      anchor: const DocPosition(0, 0),
      focus: const DocPosition(1, 0),
    );
    expect(c.selectionText(), contains('![cat](https://x.io/cat.png)'));
  });

  test('image vs link are distinguished by the ! prefix', () {
    // A standalone link line stays a paragraph (with a link mark), not an image.
    final link = markdownToBlocks('[docs](https://x.io)');
    expect(link.single.kind, 'paragraph');
    expect(link.single.text, 'docs');
    expect((link.single.data['marks'] as List).single['type'], 'link');

    // The same target with a ! is an image block.
    final img = markdownToBlocks('![docs](https://x.io)');
    expect(img.single.kind, 'image');
    expect(img.single.data['url'], 'https://x.io');
  });

  test('inline image is not mistaken for a link', () {
    // A paragraph mixing a link and an inline image: only the link becomes a
    // mark; the image stays literal (no inline-image marks), never a broken link.
    final blocks = markdownToBlocks('see [d](https://a) and ![i](https://b) end');
    final p = blocks.single;
    expect(p.kind, 'paragraph');
    final marks = (p.data['marks'] as List?) ?? [];
    expect(marks.where((m) => m['type'] == 'link').length, 1);
    expect(p.text, contains('![i](https://b)'));
  });

  test('setImageSource swaps an external url for our file_id', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([
      EditorNode(id: 'b', kind: 'image', text: 'cat', data: {
        'url': 'https://x.io/cat.png',
      }),
    ]);
    c.setImageSource('b', fileId: 'file-123', name: 'cat.png');
    final data = c.nodes.single.data;
    expect(data['file_id'], 'file-123');
    expect(data['name'], 'cat.png');
    expect(data.containsKey('url'), isFalse);
  });
}
