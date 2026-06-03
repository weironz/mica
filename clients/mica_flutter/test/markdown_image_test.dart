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
}
