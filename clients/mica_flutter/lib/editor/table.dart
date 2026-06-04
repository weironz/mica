/// Table model + GFM (pipe-table) parsing/serialization for the editor.
///
/// A table is a single block of kind `table`; its grid lives in the block's
/// `data` as `{ "rows": [[cell, …], …], "header": true }`. The main caret treats
/// the block as atomic — cells are edited through an overlay field (see
/// editor.dart) rather than the flat text-caret model, which keeps tables from
/// requiring a cell-addressing rewrite of the selection engine.
library;

class TableData {
  TableData(
    this.rows, {
    this.header = true,
    this.align = 'left',
    this.tableWidth = 1.0,
    List<double>? widths,
  }) : widths = _normalizeWidths(widths, rows.isEmpty ? 0 : rows.first.length);

  /// Rows of cells; every row has the same length (`columns`).
  final List<List<String>> rows;
  final bool header;

  /// Cell text alignment for the whole table: `left` | `center` | `right`.
  final String align;

  /// Per-column relative weights (sum normalized at layout). Length == columns.
  final List<double> widths;

  /// Overall table width as a fraction of the available content width
  /// (1.0 = full width). Dragging the table's right edge adjusts this.
  final double tableWidth;

  int get columns => rows.isEmpty ? 0 : rows.first.length;
  int get rowCount => rows.length;

  factory TableData.fromBlock(Map<String, dynamic> data) {
    final raw = data['rows'];
    final rows = <List<String>>[];
    if (raw is List) {
      for (final row in raw) {
        if (row is List) {
          rows.add([for (final cell in row) '$cell']);
        }
      }
    }
    if (rows.isEmpty) {
      return TableData.empty();
    }
    final width = rows.fold<int>(0, (m, r) => r.length > m ? r.length : m);
    for (final row in rows) {
      while (row.length < width) {
        row.add('');
      }
    }
    List<double>? widths;
    final rawWidths = data['widths'];
    if (rawWidths is List) {
      widths = [for (final w in rawWidths) (w as num).toDouble()];
    }
    final align = switch (data['align']) {
      'center' => 'center',
      'right' => 'right',
      _ => 'left',
    };
    final tw = data['width'];
    return TableData(
      rows,
      header: data['header'] != false,
      align: align,
      tableWidth: tw is num ? tw.toDouble().clamp(0.15, 1.0) : 1.0,
      widths: widths,
    );
  }

  factory TableData.empty() => TableData([
    ['Column 1', 'Column 2', 'Column 3'],
    ['', '', ''],
  ]);

  Map<String, dynamic> toBlockData() => {
    'rows': rows,
    'header': header,
    'align': align,
    'widths': widths,
    // Default-width tables omit the key (keeps parser conformance with the
    // Rust engine, which doesn't model table width).
    if (tableWidth != 1.0) 'width': tableWidth,
  };

  void insertRow(int at) {
    final i = at.clamp(0, rows.length);
    rows.insert(i, List<String>.filled(columns, ''));
  }

  void deleteRow(int at) {
    if (rows.length <= 1 || at < 0 || at >= rows.length) return;
    rows.removeAt(at);
  }

  void insertColumn(int at) {
    final i = at.clamp(0, columns);
    for (final row in rows) {
      row.insert(i.clamp(0, row.length), '');
    }
    widths.insert(i.clamp(0, widths.length), 1.0);
  }

  void deleteColumn(int at) {
    if (columns <= 1 || at < 0 || at >= columns) return;
    for (final row in rows) {
      if (at < row.length) row.removeAt(at);
    }
    if (at < widths.length) widths.removeAt(at);
  }

  static List<double> _normalizeWidths(List<double>? widths, int columns) {
    if (columns <= 0) return [];
    if (widths == null || widths.length != columns) {
      return List<double>.filled(columns, 1.0);
    }
    return [for (final w in widths) w <= 0 ? 1.0 : w];
  }
}

/// True if [lines] starting at [index] look like a GFM table (a `|`-row followed
/// by a `| --- |` separator row).
bool looksLikeGfmTable(List<String> lines, int index) {
  if (index + 1 >= lines.length) return false;
  final first = lines[index].trim();
  final sep = lines[index + 1].trim();
  if (!first.contains('|')) return false;
  // Separator: cells of only -, :, spaces.
  final sepCells = _splitRow(sep);
  if (sepCells.isEmpty) return false;
  return sepCells.every((c) => RegExp(r'^:?-{1,}:?$').hasMatch(c.trim()));
}

/// Parse a GFM table beginning at [index]; returns the table and the index just
/// past it.
({TableData table, int next}) parseGfmTable(List<String> lines, int index) {
  final header = _splitRow(lines[index]);
  var i = index + 2; // skip header + separator
  final body = <List<String>>[];
  while (i < lines.length && lines[i].trim().contains('|')) {
    body.add(_splitRow(lines[i]));
    i++;
  }
  final width = [header.length, for (final r in body) r.length]
      .fold<int>(1, (m, n) => n > m ? n : m);
  List<String> pad(List<String> r) =>
      [...r, for (var k = r.length; k < width; k++) ''];
  final rows = <List<String>>[pad(header), for (final r in body) pad(r)];
  return (table: TableData(rows, header: true), next: i);
}

/// Serialize a table to GFM pipe-table Markdown.
String tableToMarkdown(TableData table) {
  if (table.rows.isEmpty) return '';
  String row(List<String> cells) =>
      '| ${cells.map((c) => c.replaceAll('|', r'\|').replaceAll('\n', ' ').trim()).join(' | ')} |';
  final out = StringBuffer();
  out.writeln(row(table.rows.first));
  out.writeln('| ${List.filled(table.columns, '---').join(' | ')} |');
  for (final r in table.rows.skip(1)) {
    out.writeln(row(r));
  }
  return out.toString().trimRight();
}

List<String> _splitRow(String line) {
  var s = line.trim();
  if (s.startsWith('|')) s = s.substring(1);
  if (s.endsWith('|')) s = s.substring(0, s.length - 1);
  // Split on unescaped pipes.
  final cells = <String>[];
  final buffer = StringBuffer();
  for (var i = 0; i < s.length; i++) {
    final ch = s[i];
    if (ch == r'\' && i + 1 < s.length && s[i + 1] == '|') {
      buffer.write('|');
      i++;
    } else if (ch == '|') {
      cells.add(buffer.toString().trim());
      buffer.clear();
    } else {
      buffer.write(ch);
    }
  }
  cells.add(buffer.toString().trim());
  return cells;
}
