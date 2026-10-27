// The rich-flavor half of table-area copy: turn the selected cell rectangle
// into engine blocks so `MarkdownCodec.renderClipboardHtml` can build the
// `<table>` (cells included) in the Clipboard dialect. The plain-TSV half
// stays at the call site — it is a flat string join with no engine involved.
library;

import 'marks.dart';
import 'table.dart';

/// A clipboard table cell in its stored form: inline Markdown parsed to
/// `{text, marks}` when it carries formatting, plain string otherwise —
/// the same encoding `TableData._encodeCell` stores, so the engine's
/// Clipboard dialect sees exactly what a copied whole table would.
Object cellClipboardMarkdown(String source) {
  final parsed = parseInline(source);
  if (parsed.marks.isEmpty) return source;
  return {'text': parsed.text, 'marks': marksToJson(parsed.marks)};
}

/// The selected rectangle of [table] as one `table` block's data map.
Map<String, dynamic> tableAreaData(
  TableData table, {
  required int r0,
  required int c0,
  required int r1,
  required int c1,
}) {
  return {
    'rows': [
      for (var r = r0; r <= r1 && r < table.rows.length; r++)
        if (r >= 0)
          [
            for (var c = c0; c <= c1 && c < table.rows[r].length; c++)
              if (c >= 0) cellClipboardMarkdown(table.rows[r][c]),
          ],
    ],
  };
}
