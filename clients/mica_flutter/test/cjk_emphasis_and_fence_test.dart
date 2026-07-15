import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';

// Two ChatGPT-paste fixes:
//  (1) CJK-friendly emphasis — a `**` closing right after a full-width period
//      (`日志。**它`) now pairs, matching markdown-cjk-friendly / Typora and
//      mica's own Rust engine. ASCII behavior is unchanged.
//  (2) unwrapNestedFences — the LLM "double-fence" wrapper is stripped at paste
//      time so the inner mermaid/code block renders and nothing swallows the
//      rest of the document.

({String text, List<Mark> marks}) _inline(String s) {
  final b = markdownToBlocks(s).single;
  return (text: b.text, marks: marksFromData(b.data));
}

List<Mark> _typed(String type, List<Mark> marks) =>
    marks.where((m) => m.type == type).toList();

void main() {
  group('CJK-friendly emphasis', () {
    test('bold closes after a full-width period', () {
      final r = _inline('结论：**不能只靠日志。**它是入口');
      expect(r.text, '结论：不能只靠日志。它是入口');
      final bold = _typed('bold', r.marks).single;
      expect(r.text.substring(bold.start, bold.end), '不能只靠日志。');
    });

    test('parseInline (live editor path) agrees', () {
      final p = parseInline('**加粗。**后面');
      expect(p.text, '加粗。后面');
      final bold = p.marks.where((m) => m.type == 'bold').single;
      expect(p.text.substring(bold.start, bold.end), '加粗。');
    });

    test('italic bounded by CJK punctuation', () {
      final r = _inline('这是*重点*。');
      final it = _typed('italic', r.marks).single;
      expect(r.text.substring(it.start, it.end), '重点');
    });

    test('underscore emphasis works across CJK boundaries', () {
      final r = _inline('下划线_强调_文字');
      final it = _typed('italic', r.marks).single;
      expect(r.text.substring(it.start, it.end), '强调');
    });

    test('ASCII emphasis is unchanged', () {
      expect(_typed('bold', _inline('**hello world**').marks), hasLength(1));
      expect(_typed('bold', _inline('foo**bar**baz').marks), hasLength(1));
      // snake_case stays literal (no emphasis).
      final snake = _inline('foo_bar_baz');
      expect(snake.text, 'foo_bar_baz');
      expect(snake.marks, isEmpty);
    });

    test('round-trip: exported CJK bold re-parses to bold', () {
      const src = '不能只靠日志。';
      final md = inlineToMarkdown('$src它', [Mark(0, src.length, 'bold')]);
      expect(md, '**不能只靠日志。**它');
      final back = _inline(md);
      final bold = _typed('bold', back.marks).single;
      expect(back.text.substring(bold.start, bold.end), src);
    });
  });

  group('unwrapNestedFences', () {
    List<(String, String?)> shape(String md) => [
          for (final b in markdownToBlocks(unwrapNestedFences(md)))
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
      final blocks = markdownToBlocks(unwrapNestedFences(src));
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
