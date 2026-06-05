import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/table.dart';

void main() {
  test('a cell with inline code survives import → model → markdown round-trip',
      () {
    const md = '| A | B |\n| --- | --- |\n| `code` | text |';
    final blocks = markdownToBlocks(md);
    final spec = blocks.singleWhere((b) => b.kind == 'table');
    final table = TableData.fromBlock(spec.data);

    // The cell keeps its raw inline-code source verbatim (cells store Markdown
    // source; the backticks are NOT a stray "undefined"/"null" placeholder).
    expect(table.rows[1][0], '`code`');

    // Re-serializing the table preserves the code span.
    expect(tableToMarkdown(table), md);

    // And the cell's Markdown parses to a code mark over "code".
    final parsed = parseInline(table.rows[1][0]);
    expect(parsed.text, 'code');
    expect(parsed.marks.single.type, 'code');
  });

  test('other inline marks in a cell round-trip too (bold/italic/strike/link)',
      () {
    const md =
        '| A | B |\n| --- | --- |\n| **b** *i* | ~~s~~ [t](u) |';
    final table = TableData.fromBlock(
      markdownToBlocks(md).singleWhere((b) => b.kind == 'table').data,
    );
    expect(table.rows[1][0], '**b** *i*');
    expect(table.rows[1][1], '~~s~~ [t](u)');
    expect(tableToMarkdown(table), md);

    final c1 = parseInline(table.rows[1][1]);
    expect(c1.marks.map((m) => m.type).toSet(), {'strike', 'link'});
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
    expect(table.rows[1][1], '');
    expect(table.rows[1][0], '`code`');
    expect(tableToMarkdown(table), '| A | B |\n| --- | --- |\n| `code` |  |');
  });
}
