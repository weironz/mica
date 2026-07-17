import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';

/// Regression: copying formatted text OUT of a table cell and pasting it into
/// the body dropped the formatting and showed the literal markdown source
/// (`**bold**`). Cause: a cell is a Flutter TextField, and its default Ctrl+C
/// puts only the cell's raw markdown on the clipboard as text/plain — no
/// text/html — so the paste path's single-line plain branch inserted it
/// verbatim (Typora worked because it re-parses markdown; mica does not for a
/// lone plain line). The fix has the cell's own Ctrl+C write a text/html flavor
/// too, via `inlineToHtml(parseInline(selectedCellMarkdown))`. This pins the two
/// ends of that flavor: the html mica writes, and the marks mica reads back.
void main() {
  test('cell Ctrl+C html flavor round-trips bold + inline code as MARKS', () {
    // What the fixed cell copy computes for a selection of the cell's markdown.
    const selectedCellMd = r'**bold** and `code`';
    final parsed = parseInline(selectedCellMd);
    final richHtml = inlineToHtml(parsed.text, parsed.marks);
    expect(richHtml, contains('<strong>bold</strong>'));
    expect(richHtml, contains('<code>code</code>'));

    // What the paste path does with that clipboard html.
    final md = htmlToMarkdown(richHtml);
    final blocks = markdownToBlocks(md);
    expect(blocks.length, 1);
    final block = blocks.single;
    expect(block.kind, 'paragraph');
    // The marks must survive as REAL marks — not as literal `**`/`` ` `` in text.
    expect(block.text, 'bold and code');
    final marks = marksFromData(block.data);
    expect(marks.map((m) => m.type).toSet(), {'bold', 'code'});
  });
}
