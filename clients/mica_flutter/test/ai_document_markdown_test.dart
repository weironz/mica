import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'support/markdown_codec.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  test(
    'document Markdown reads every block without changing the selection',
    () {
      final controller = EditorController(
        rootBlockId: 'root',
        onOps: (_) async {},
      );
      addTearDown(controller.dispose);
      controller.load([
        EditorNode(id: 'h', kind: 'heading', text: '配置', data: {'level': 2}),
        EditorNode(
          id: 'c',
          kind: 'code_block',
          text: 'lookup: vault',
          data: {'language': 'yaml'},
        ),
        EditorNode(
          id: 'i',
          kind: 'image',
          text: '拓扑',
          data: {'file_id': 'asset'},
        ),
        EditorNode(id: 'd', kind: 'divider', text: ''),
      ]);
      const selection = DocSelection(
        anchor: DocPosition(0, 1),
        focus: DocPosition(0, 2),
      );
      controller.selection = selection;
      final markdown = controller.documentMarkdown(
        imageUrls: {'asset': 'https://example.test/image.png'},
      );
      expect(markdown, contains('## 配置'));
      expect(markdown, contains('```yaml\nlookup: vault\n```'));
      expect(markdown, contains('![拓扑](https://example.test/image.png)'));
      expect(markdown, endsWith('---'));
      expect(controller.selection, same(selection));
      controller.selection = null;
      expect(
        controller.documentMarkdown(
          imageUrls: {'asset': 'https://example.test/image.png'},
        ),
        markdown,
      );
    },
  );

  test('empty editor body serializes without indexing a missing node', () {
    final controller = EditorController(
      rootBlockId: 'root',
      onOps: (_) async {},
    );
    addTearDown(controller.dispose);
    expect(controller.documentMarkdown(), '');
  });
}
