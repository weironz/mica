import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/model.dart';
import 'support/markdown_codec.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  String copy(String source, {int first = 0}) {
    final blocks = markdownToBlocks(source);
    final nodes = [
      for (var i = 0; i < blocks.length; i++)
        EditorNode(
          id: 'b$i',
          kind: blocks[i].kind,
          text: blocks[i].text,
          data: Map<String, dynamic>.of(blocks[i].data),
        ),
    ];
    final controller = EditorController(
      rootBlockId: 'root',
      onOps: (_) async {},
    );
    try {
      controller.load(nodes);
      controller.selection = DocSelection(
        anchor: DocPosition(first, 0),
        focus: DocPosition(nodes.length - 1, nodes.last.text.length),
      );
      return controller.selectionText();
    } finally {
      controller.dispose();
    }
  }

  test('copy uses visible ordinals, even when selection starts mid-list', () {
    expect(copy('1. one\n2. two\n3. three'), '1. one\n2. two\n3. three');
    expect(copy('1. one\n2. two\n3. three', first: 1), '2. two\n3. three');
  });

  test('nested lists and separate runs keep independent numbering', () {
    const source =
        '1. outer\n    1. inner\n    2. next\n2. outer two\n\n'
        'paragraph\n\n3. three\n4. four';
    expect(copy(source), source);
  });

  test(
    'wide ordinals and list container children preserve parsed structure',
    () {
      for (final source in [
        '100. outer\n     1. inner\n     2. next\n101. outer two',
        '1. first\n\n    child paragraph\n\n2. second',
        '999999999. first\n1. second',
      ]) {
        final copied = copy(source);
        Object shape(String markdown) => markdownToBlocks(
          markdown,
        ).map((b) => {'kind': b.kind, 'text': b.text, 'data': b.data}).toList();
        expect(shape(copied), shape(source), reason: copied);
      }
    },
  );
}
