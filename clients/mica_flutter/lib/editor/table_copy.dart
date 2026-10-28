// The rich-flavor half of table-area copy: turn the selected cell rectangle
// into engine blocks so `MarkdownCodec.renderClipboardHtml` can build the
// `<table>` (cells included) in the Clipboard dialect. The plain-TSV half
// stays at the call site — it is a flat string join with no engine involved.
library;

import 'table.dart';

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