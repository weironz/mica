// Editor-side image behavior. The PARSER assertions that used to live here
// (`![alt](url)` → image block, image vs link, inline images inside a
// paragraph) went with the Dart mirror in S3 T3.9; their Rust equivalents:
//   * crates/markdown/tests/image_alt_export.rs — standalone images flatten
//     alt links, image blocks escape literal alt and keep the title;
//   * crates/markdown/tests/inline_invertibility.rs
//     `image_blocks_preserve_destinations_and_titles`;
//   * crates/markdown/tests/fixtures/conformance/16-images.md (+ .blocks.json).
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'support/markdown_codec.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
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