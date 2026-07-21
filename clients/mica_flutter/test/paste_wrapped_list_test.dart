// A pasted list whose items sit inside wrapper elements.
//
// Reported as "I copy this and paste it, and the list content is gone". It
// was: the surrounding paragraphs arrived and the whole numbered list, nested
// bullets and all, simply was not there.
//
// The cause is in the source markup, not in ours. Per the spec an `<ol>`/`<ul>`
// may only contain `<li>`, but Google's AI overview emits
// `<ol><div data-bfc><li>…</li></div>…</ol>`, and the HTML parser keeps that
// shape. `_list` walked DIRECT children and skipped anything that was not an
// `li`, so every item was behind a skipped `<div>`.
//
// The markup below is the real clipboard structure, reduced to the parts that
// matter (Google's class/jsaction/data-ved noise removed). Verified against the
// actual 31KB clipboard capture: before the fix that produced 4 blocks and no
// list; after, 12 blocks with the nesting intact.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/editor/markdown.dart';

void main() {
  test('items wrapped in <div> inside <ol> still become a list', () {
    const html = '''
<p>您可以按照以下排查步骤逐步解决该问题：</p>
<ol>
  <div data-bfc=""><li><span>临时恢复</span>：重启应用程序即可。</li></div>
  <div data-bfc=""><li><span>关闭 GSP 固件功能</span>：可以通过关闭来规避：
    <ul>
      <div data-bfc=""><li>远程连接您的 Linux 系统。</li></div>
      <div data-bfc=""><li>执行命令关闭 GSP。</li></div>
    </ul>
  </li></div>
  <div data-bfc=""><li>驱动与固件版本升级。</li></div>
</ol>
<p>如果报错仍然无法消除…</p>''';

    final blocks = markdownToBlocks(htmlToMarkdown(html));
    final kinds = [for (final b in blocks) b.kind];

    expect(kinds.where((k) => k == 'numbered_list').length, 3,
        reason: 'every item was behind a wrapper div and vanished');
    expect(kinds.where((k) => k == 'bulleted_list').length, 2);

    // The sub-list must stay a sub-list: unwrapping must not hoist the nested
    // items up to the top level.
    final nested = blocks.where((b) => b.kind == 'bulleted_list');
    expect(nested.every((b) => (b.data['indent'] ?? 0) == 1), isTrue,
        reason: 'nested bullets belong under their numbered item');

    // The paragraphs around the list were never the problem — guard that the
    // fix did not disturb them.
    expect(kinds.first, 'paragraph');
    expect(kinds.last, 'paragraph');
  });

  test('a plain, spec-correct list is unchanged', () {
    const html = '<ul><li>one</li><li>two<ul><li>deep</li></ul></li></ul>';
    final blocks = markdownToBlocks(htmlToMarkdown(html));
    expect(blocks.length, 3);
    expect(blocks.map((b) => b.data['indent'] ?? 0).toList(), [0, 0, 1]);
  });
}
