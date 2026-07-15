import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/editor/markdown.dart';

// The bug this pins: content copied from an LLM chat page (Gemini/ChatGPT)
// arrives as text/html, and htmlToMarkdown backslash-escaped EVERY `$` (and
// doubled every LaTeX `\`) — so a pasted `$\eta = 2$` could never become a
// math mark, while the SAME text pasted as plain text parsed fine. The user's
// primary math workflow (paste an LLM answer) silently produced dead source.
//
// The fix lets valid math runs (same Pandoc rules the parser uses) through the
// escaping verbatim; everything else keeps its protection.

List<Map<String, dynamic>> mathMarks(String markdown) {
  final blocks = markdownToBlocks(markdown);
  return [
    for (final b in blocks)
      if (b.data['marks'] is List)
        ...(b.data['marks'] as List).whereType<Map<String, dynamic>>().where(
          (m) => m['type'] == 'math',
        ),
  ];
}

void main() {
  test('a formula in pasted HTML survives into a math mark', () {
    const html =
        '<p>放大系数为 \$\\eta = 2 \\times \\frac{N - 1}{N}\$。'
        '当集群节点数 \$N = 256\$ 时，该系数逼近极限值 \$2\$。</p>';
    final md = htmlToMarkdown(html);
    expect(
      md,
      contains(r'$\eta = 2 \times \frac{N - 1}{N}$'),
      reason: 'the run passes through unescaped, single backslashes',
    );
    expect(md, isNot(contains(r'\$\eta')), reason: 'no escaped opener');

    final marks = mathMarks(md);
    expect(marks, hasLength(3));
    // And the LaTeX itself must be intact (no doubled backslashes).
    final blocks = markdownToBlocks(md);
    final text = blocks.first.text;
    final m = marks.first;
    expect(
      text.substring(m['start'] as int, m['end'] as int),
      r'\eta = 2 \times \frac{N - 1}{N}',
    );
  });

  test('the user actual Gemini bullet list parses end to end', () {
    const html =
        '<ul>'
        '<li><strong>带宽放大因子</strong>：对于包含 \$N\$ 个 GPU 节点的 Ring AllReduce 算法，'
        '每个 GPU 实际发送和接收的物理数据放大系数为 \$\\eta = 2 \\times \\frac{N - 1}{N}\$。</li>'
        '<li><strong>物理带宽上限</strong>：单向物理带宽为 \$B_{\\text{phy}} = 100 \\text{ GB/s}\$。</li>'
        '</ul>';
    final marks = mathMarks(htmlToMarkdown(html));
    expect(
      marks.length,
      greaterThanOrEqualTo(3),
      reason: 'N, eta-frac, and B_phy must all become math',
    );
  });

  test('currency keeps its protection — still escaped, still literal', () {
    const html = '<p>It costs \$5 and \$10 total.</p>';
    final md = htmlToMarkdown(html);
    expect(md, contains(r'\$5'), reason: 'no valid closer → escaped');
    expect(mathMarks(md), isEmpty);
    final text = markdownToBlocks(md).first.text;
    expect(text, contains('\$5 and \$10'), reason: 'dollars survive as text');
  });

  test('emphasis characters keep their protection', () {
    const html = '<p>a *not bold* and [not a link] b</p>';
    final md = htmlToMarkdown(html);
    expect(md, contains(r'\*not bold\*'));
    expect(md, contains(r'\[not a link\]'));
  });

  test(r'\(...\) LaTeX form also passes through', () {
    const html = '<p>能量 \\(E = mc^2\\) 守恒。</p>';
    final marks = mathMarks(htmlToMarkdown(html));
    expect(marks, hasLength(1));
  });

  test('a dollar formula inside <code> stays code, not math', () {
    const html = '<p>写成 <code>\$\\eta = 2\$</code> 才对。</p>';
    final md = htmlToMarkdown(html);
    final blocks = markdownToBlocks(md);
    final marks = (blocks.first.data['marks'] as List?) ?? const [];
    expect(
      marks.whereType<Map>().where((m) => m['type'] == 'math'),
      isEmpty,
      reason: 'code binds tighter than math',
    );
    expect(
      marks.whereType<Map>().where((m) => m['type'] == 'code'),
      isNotEmpty,
    );
  });
}
