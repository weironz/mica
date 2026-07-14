import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/rich_paste_stub.dart'
    show clipboardHtmlIsDataTable;
import 'package:mica_flutter/editor/table.dart';

EditorController _doc(List<EditorNode> nodes) {
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  return c;
}

EditorNode _table3x3() => EditorNode(
      id: 't',
      kind: 'table',
      text: '',
      data: TableData([
        ['h1', 'h2', 'h3'],
        ['a1', 'a2', 'a3'],
        ['b1', 'b2', 'b3'],
      ]).toBlockData(),
    );

void main() {
  group('clearTableCells — Delete/cut on an area selection', () {
    test('blanks exactly the inclusive rectangle', () {
      final c = _doc([_table3x3()]);
      c.clearTableCells(0, 1, 1, 2, 2); // rows 1–2 × cols 1–2
      final t = TableData.fromBlock(c.nodes[0].data);
      expect(t.rows[0], ['h1', 'h2', 'h3']); // header untouched
      expect(t.rows[1], ['a1', '', '']);
      expect(t.rows[2], ['b1', '', '']);
    });

    test('out-of-range bounds clamp instead of throwing', () {
      final c = _doc([_table3x3()]);
      c.clearTableCells(0, -5, -5, 99, 99); // whole table
      final t = TableData.fromBlock(c.nodes[0].data);
      expect(t.rows.expand((r) => r).every((s) => s.isEmpty), isTrue);
    });

    test('non-table node is a no-op', () {
      final c = _doc([EditorNode(id: 'p', kind: 'paragraph', text: 'x')]);
      c.clearTableCells(0, 0, 0, 1, 1); // must not throw
      expect(c.nodes[0].text, 'x');
    });
  });

  test('resetTableColumnWidths returns every weight to 1.0 (auto-fit mode)', () {
    final table = TableData([
      ['a', 'b'],
      ['c', 'd'],
    ], widths: [3.0, 1.0]);
    final c = _doc([
      EditorNode(id: 't', kind: 'table', text: '', data: table.toBlockData()),
    ]);
    c.resetTableColumnWidths(0);
    final t = TableData.fromBlock(c.nodes[0].data);
    expect(t.widths, [1.0, 1.0]);
  });

  test('insertParagraphAfter appends a paragraph and lands the caret in it', () {
    final c = _doc([_table3x3()]);
    c.insertParagraphAfter(0);
    expect(c.nodes.length, 2);
    expect(c.nodes[1].kind, 'paragraph');
    expect(c.nodes[1].text, '');
    expect(c.selection?.focus, const DocPosition(1, 0));
  });

  group('Excel/Sheets HTML table → Markdown table (paste path)', () {
    test('an Excel-style styled table becomes a GFM pipe table', () {
      // Shape mirrors what Excel puts in CF_HTML (post header-strip): styled
      // spans, explicit widths, no <thead>.
      const excel = '''
<html><body>
<table border=0 cellpadding=0 cellspacing=0 width=192 style='border-collapse:collapse;width:144pt'>
 <tr height=19 style='height:14.4pt'>
  <td height=19 width=64 style='height:14.4pt;width:48pt'>Name</td>
  <td width=64 style='width:48pt'>Qty</td>
  <td width=64 style='width:48pt'>Price</td>
 </tr>
 <tr height=19 style='height:14.4pt'>
  <td height=19 style='height:14.4pt'>Apple</td>
  <td align=right>3</td>
  <td align=right>1.5</td>
 </tr>
</table>
</body></html>''';
      final md = htmlToMarkdown(excel);
      expect(md, contains('| Name | Qty | Price |'));
      expect(md, contains('| --- | --- | --- |'));
      expect(md, contains('| Apple | 3 | 1.5 |'));
    });

    test('bold inside a table cell survives as **bold**', () {
      final md = htmlToMarkdown(
        '<table><tr><td><b>total</b></td><td>42</td></tr></table>',
      );
      expect(md, contains('**total**'));
    });
  });

  group('clipboardHtmlIsDataTable — table-vs-image paste arbitration', () {
    test('an Excel-style data table qualifies', () {
      expect(
        clipboardHtmlIsDataTable(
          "<table border=0><tr><td>Name</td><td>Qty</td></tr></table>",
        ),
        isTrue,
      );
    });

    test('an image wrapped in a layout table (Word/Outlook) does NOT — '
        'the bitmap flavor must win or the image is lost', () {
      expect(
        clipboardHtmlIsDataTable(
          '<table><tr><td><img src="file:///C:/temp/clip_image001.png"></td></tr></table>',
        ),
        isFalse,
      );
    });

    test('a bare <table substring in another tag does not qualify', () {
      expect(clipboardHtmlIsDataTable('<tablet-frame>x</tablet-frame>'), isFalse);
    });

    test('plain paragraphs do not qualify', () {
      expect(clipboardHtmlIsDataTable('<p>hello</p>'), isFalse);
    });
  });
}
