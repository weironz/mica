import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'support/markdown_codec.dart';

/// Regression: Ctrl+A → copy → paste a page containing a table turned every
/// bold/inline-code cell into visible markdown source (`**你好**`). Cause:
/// the clipboard's text/html flavor emitted the cell's raw markdown —
/// `<td>**你好**</td>` — instead of real html marks, so the paste side's
/// `htmlToMarkdown` backslash-escaped the `**` (`\*\*你好\*\*`), which a table
/// cell renders as literal `**你好**`. The fix renders each cell through the
/// engine's Clipboard dialect (`renderClipboardHtml`), the same conversion the
/// cell-edit Ctrl+C uses. This pins that conversion end-to-end for the content
/// that broke.
void main() {
  setUpAll(useNativeMarkdownCodec);
  test('cell markdown -> copy html -> paste keeps MARKS, not literal source', () {
    for (final cellMd in [
      r'**bold** and `code`',
      r'**你好**',
      r'**真·GPU 掉卡**',
      r'`GPUMissing` 带外',
    ]) {
      // What the table copy writes into the clipboard html (the <td>). The
      // table comes from a stored document whose cells are legacy source
      // strings — the shape the regression shipped against.
      final table = TableData.fromBlock({
        'rows': [
          ['A', 'B'],
          [cellMd, 'x'],
        ],
      });
      final html = markdownCodec.renderClipboardHtml([
        (kind: 'table', text: '', data: table.toBlockData()),
      ]);
      // The cell must carry REAL html marks, never the raw markdown source.
      expect(html.contains('**'), isFalse,
          reason: 'raw markdown source in clipboard html for <<$cellMd>>: $html');
      expect(html.contains('&lt;strong'), isFalse,
          reason: 'escaped tags instead of marks for <<$cellMd>>: $html');

      // What the paste side turns that html back into.
      final md = htmlToMarkdown(html);

      // The bug: a backslash-escaped `\*` / `` \` `` in the round-tripped
      // markdown — which renders as literal source, not a mark.
      expect(md.contains(r'\*'), isFalse,
          reason: 'bold must not come back backslash-escaped for <<$cellMd>>: $md');
      expect(md.contains(r'\`'), isFalse,
          reason: 'code must not come back backslash-escaped for <<$cellMd>>: $md');

      // And the pasted cell text carries real marks, not literal `**`/`` ` ``.
      final blocks = markdownCodec.parseBlocks(md);
      final tableBlock = blocks.single;
      expect(tableBlock.kind, 'table');
      // Decoded through TableData, the same path the editor uses, rather than
      // casting the raw block data: a cell WITH marks is `{text, marks}` (the
      // shape paragraphs use and the engine emits), and only a mark-free cell
      // stays a plain string.
      final cell = TableData.fromBlock(tableBlock.data).rows[1].first;
      expect(cell.marks, isNotEmpty,
          reason: 'pasted cell <<${cell.text}>> must carry a mark');
    }
  });
}