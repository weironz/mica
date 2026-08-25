import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/editor/table.dart';

/// Regression: Ctrl+A → copy → paste a page containing a table turned every
/// bold/inline-code cell into visible markdown source (`**你好**`). Cause:
/// `_tableHtml` (used by `selectionHtml`, the text/html copy flavor) emitted the
/// cell's raw markdown via `escapeHtml` — `<td>**你好**</td>` — instead of real
/// html marks, so the paste side's `htmlToMarkdown` backslash-escaped the `**`
/// (`\*\*你好\*\*`), which a table cell renders as literal `**你好**`. The fix
/// renders each cell through `inlineToHtml(parseInline(cell))`, the same
/// conversion `_copyTableArea` and the cell-edit Ctrl+C already use. This pins
/// that conversion end-to-end for the content that broke.
void main() {
  test('cell markdown -> copy html -> paste keeps MARKS, not literal source', () {
    for (final cellMd in [
      r'**bold** and `code`',
      r'**你好**',
      r'**真·GPU 掉卡**',
      r'`GPUMissing` 带外',
    ]) {
      // What the fixed table/cell copy writes into the <td> (and clipboard html).
      final parsed = parseInline(cellMd);
      final cellHtml = inlineToHtml(parsed.text, parsed.marks);

      // What the paste side turns that html back into.
      final md = htmlToMarkdown('<table><tr><td>$cellHtml</td></tr></table>');

      // The bug: a backslash-escaped `\*` / `` \` `` in the round-tripped
      // markdown — which renders as literal source, not a mark.
      expect(md.contains(r'\*'), isFalse,
          reason: 'bold must not come back backslash-escaped for <<$cellMd>>: $md');
      expect(md.contains(r'\`'), isFalse,
          reason: 'code must not come back backslash-escaped for <<$cellMd>>: $md');

      // And the pasted cell text carries real marks, not literal `**`/`` ` ``.
      final blocks = markdownToBlocks(md);
      final table = blocks.single;
      expect(table.kind, 'table');
      // Decoded through TableData, the same path the editor uses, rather than
      // casting the raw block data. That cast was `as String` and had been
      // failing since cells gained the unified stored form: a cell WITH marks
      // is now `{text, marks}` (the shape paragraphs use and Rust
      // `crates/markdown` emits), and only a mark-free cell stays a plain
      // string.
      //
      // So the crash was good news in disguise — getting a Map here is exactly
      // the marks surviving. The assertion below is unchanged; only the way the
      // text is obtained is.
      final cellText = TableData.fromBlock(table.data).rows.first.first;
      final marks = parseInline(cellText).marks;
      expect(marks, isNotEmpty,
          reason: 'pasted cell <<$cellText>> must parse back to a mark');
    }
  });
}
