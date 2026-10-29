// The ChatGPT double-fence paste repair: `unwrapNestedFences` strips the LLM's
// outer fence so the inner mermaid/code block renders and nothing swallows the
// rest of the document. This is a Dart PASTE heuristic (paste_repair.dart, D7),
// not a parser mirror, so it stays after S3.
//
// The CJK-friendly emphasis group that used to live here tested the Dart
// parser against Rust; it went with the mirror in S3 T3.9. Rust equivalent:
// crates/markdown/tests/cjk_emphasis.rs (`strong_closes_after_cjk_period`,
// `emphasis_between_cjk_and_punctuation`, `ascii_emphasis_is_unchanged`,
// `round_trip_cjk_bold`).
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/paste_repair.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'support/markdown_codec.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  group('unwrapNestedFences', () {
    List<(String, String?)> shape(String md) => [
          for (final b in markdownCodec.parseBlocks(unwrapNestedFences(md)))
            (b.kind, b.data['language'] as String?),
        ];

    test('the ChatGPT double-fence: inner mermaid becomes a real block, '
        'the rest is prose (not swallowed as code)', () {
      const src = '建议:\n\n'
          '```\n'
          '```mermaid\n'
          'flowchart LR\n'
          '  A --> B\n'
          '```\n'
          '```\n\n'
          '你们这张拓扑图里\n\n'
          '- 第一条';
      expect(shape(src), [
        ('paragraph', null),
        ('code_block', 'mermaid'),
        ('paragraph', null),
        ('bulleted_list', null),
      ]);
    });

    test('a normal single mermaid fence is untouched', () {
      const src = '```mermaid\nflowchart LR\n  A --> B\n```';
      expect(shape(src), [('code_block', 'mermaid')]);
    });

    test('a legit 4-outer / 3-inner wrapper is left alone (shows source)', () {
      const src = '````\n```mermaid\nA --> B\n```\n````\n\nafter';
      final blocks = markdownCodec.parseBlocks(unwrapNestedFences(src));
      expect(blocks.first.kind, 'code_block');
      expect(blocks.first.data['language'], isNot('mermaid'),
          reason: 'the 4-backtick wrapper is intentional literal source');
      expect(blocks.last.kind, 'paragraph');
      expect(blocks.last.text, 'after');
    });

    test('two adjacent independent code blocks are NOT merged', () {
      const src = '```js\na\n```\n```py\nb\n```';
      expect(shape(src), [('code_block', 'js'), ('code_block', 'py')]);
    });

    test('ordinary prose with no fences is returned unchanged', () {
      const src = 'hello\n\nworld';
      expect(unwrapNestedFences(src), src);
    });

    test('the 3-fence variant (no trailing outer close) still unwraps', () {
      const src = '```\n```mermaid\nA --> B\n```';
      expect(shape(src), [('code_block', 'mermaid')]);
    });
  });
}