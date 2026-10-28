// Table cells are text + marks in memory; the stored form (source string or
// `{text, marks}` object) is produced by the engine. The Rust half of the
// code-span cases is crates/markdown/tests/table_inline_code.rs; this file
// pins the editor-side model (TableData) feeding the engine.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'support/markdown_codec.dart';

String _serialize(TableData table) => markdownCodec.serializeBlocks([
      (kind: 'table', text: '', data: table.toBlockData()),
    ]);

void main() {
  setUpAll(useNativeMarkdownCodec);
  test('a cell with inline code survives import → model → markdown round-trip',
      () {
    const md = '| A | B |\n| --- | --- |\n| `code` | text |';
    final blocks = markdownCodec.parseBlocks(md);
    final spec = blocks.singleWhere((b) => b.kind == 'table');
    final table = TableData.fromBlock(spec.data);

    // The cell's clean text and its code mark (the decoded form — no stray
    // "undefined"/"null" placeholder can appear here).
    final cell = table.rows[1][0];
    expect(cell.text, 'code');
    expect(cell.marks.single.type, 'code');

    // Re-serializing the table preserves the code span.
    expect(_serialize(table), md);
  });

  test('other inline marks in a cell round-trip too (bold/italic/strike/link)',
      () {
    const md = '| A | B |\n| --- | --- |\n| **b** *i* | ~~s~~ [t](u) |';
    final table = TableData.fromBlock(
      markdownCodec.parseBlocks(md).singleWhere((b) => b.kind == 'table').data,
    );
    expect(table.rows[1][0].text, 'b i');
    expect(table.rows[1][0].marks.map((m) => m.type).toSet(), {'bold', 'italic'});
    expect(table.rows[1][1].marks.map((m) => m.type).toSet(), {'strike', 'link'});
    expect(_serialize(table), md);
  });

  test('a missing cell coerces to empty — never the literal "null"/"undefined"',
      () {
    // The bug: a blind `'$cell'` interpolation stringifies a JSON null to
    // "null" (and a dart2js JS `undefined` array hole to "undefined"), which
    // then renders and round-trips as that literal word inside the cell.
    final table = TableData.fromBlock({
      'rows': [
        ['A', 'B'],
        ['`code`', null],
      ],
      'header': true,
    });
    expect(table.rows[1][1], TableCell.empty);
    expect(table.rows[1][0].text, 'code');
    expect(_serialize(table), '| A | B |\n| --- | --- |\n| `code` |  |');
  });
}