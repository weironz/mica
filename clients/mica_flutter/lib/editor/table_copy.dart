// Table-area copy: both clipboard flavors of a selected cell rectangle — the
// plain TSV and the engine's Clipboard-dialect `<table>` (cells included).
library;

import '../markdown/markdown_codec.dart' show markdownCodec, withEngineOrNull;
import 'table.dart';

/// Both flavors of the selected rectangle: tab-separated plain text (pastes
/// back into Excel/Sheets as cells) and the Clipboard-dialect `<table>` (pastes
/// into Mica/Typora as a table). [html] is empty when the engine is down, so
/// the TSV still lands.
({String plain, String html}) tableAreaClipboard(
  TableData table, {
  required int r0,
  required int c0,
  required int r1,
  required int c1,
}) {
  final plain = [
    for (var r = r0; r <= r1 && r < table.rows.length; r++)
      if (r >= 0)
        [
          for (var c = c0; c <= c1 && c < table.rows[r].length; c++)
            if (c >= 0)
              table.rows[r][c].text.replaceAll('\t', ' ').replaceAll('\n', ' '),
        ].join('\t'),
  ].join('\n');
  final data = tableAreaData(table, r0: r0, c0: c0, r1: r1, c1: c1);
  final html = withEngineOrNull(
    () => markdownCodec.renderClipboardHtml([(kind: 'table', text: '', data: data)]),
  );
  return (plain: plain, html: html ?? '');
}

/// The selected rectangle of [table] as one `table` block's data map. Cells are
/// encoded by the same [TableData.toBlockData] a whole-table copy uses, so the
/// engine's Clipboard dialect sees exactly what it would there.
Map<String, dynamic> tableAreaData(
  TableData table, {
  required int r0,
  required int c0,
  required int r1,
  required int c1,
}) {
  final rows = <List<TableCell>>[
    for (var r = r0; r <= r1 && r < table.rows.length; r++)
      if (r >= 0)
        [
          for (var c = c0; c <= c1 && c < table.rows[r].length; c++)
            if (c >= 0) table.rows[r][c],
        ],
  ];
  return {'rows': TableData(rows).toBlockData()['rows']};
}