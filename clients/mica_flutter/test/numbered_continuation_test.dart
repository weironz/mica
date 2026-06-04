import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';

void main() {
  test('an interrupting bullet list keeps ordered numbering via data.start', () {
    // The rustfs-docs shape: `1.` with nested bullets, then `2.`.
    final blocks = markdownToBlocks(
        '1. 请审阅三种安装启动模式：\n'
        '   - [SNSD](https://a)\n'
        '   - [SNMD](https://b)\n'
        '   - MNMD（当前文档）\n'
        '2. 预安装检查清单\n');
    final second = blocks.last;
    expect(second.kind, 'numbered_list');
    expect(second.data['start'], 2, reason: 'the broken run resumes at 2');

    // Same-level bullets between (2-space indent = siblings, spec rule).
    final flat = markdownToBlocks('1. one\n- bullet\n2. two\n');
    expect(flat.last.data['start'], 2);

    // A code block between items also keeps the next number.
    final fenced = markdownToBlocks('1. one\n\n```\nx\n```\n\n2. two\n');
    expect(fenced.last.data['start'], 2);
  });
}
