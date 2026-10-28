// Table cells store inline-Markdown source; serializing goes through the
// engine now (the Dart GFM writer died with the mirror in S3 T3.6). The Rust
// half of the code-span cases is crates/markdown/tests/table_inline_code.rs;
// this file pins the editor-side model (TableData) feeding the engine.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';
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

    // The cell keeps its raw inline-code source verbatim (cells store Markdown
    // source; the backticks are NOT a stray "undefined"/"null" placeholder).
    expect(table.rows[1][0], '`code`');

    // Re-serializing the table preserves the code span.
    expect(_serialize(table), md);

    // And the cell's Markdown parses to a code mark over "code".
    final parsed = parseInline(table.rows[1][0]);
    expect(parsed.text, 'code');
    expect(parsed.marks.single.type, 'code');
  });

  test('other inline marks in a cell round-trip too (bold/italic/strike/link)',
      () {
    const md = '| A | B |\n| --- | --- |\n| **b** *i* | ~~s~~ [t](u) |';
    final table = TableData.fromBlock(
      markdownCodec.parseBlocks(md).singleWhere((b) => b.kind == 'table').data,
    );
    expect(table.rows[1][0], '**b** *i*');
    expect(table.rows[1][1], '~~s~~ [t](u)');
    expect(_serialize(table), md);

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
    expect(_serialize(table), '| A | B |\n| --- | --- |\n| `code` |  |');
  });
}