/// Table model + GFM (pipe-table) parsing/serialization for the editor.
///
/// A table is a single block of kind `table`; its grid lives in the block's
/// `data` as `{ "rows": [[cell, …], …], "header": true }`. The main caret treats
/// the block as atomic — cells are edited through an overlay field (see
/// editor.dart) rather than the flat text-caret model, which keeps tables from
/// requiring a cell-addressing rewrite of the selection engine.
library;

import 'markdown.dart' show isThematicBreak;

class TableData {
  TableData(
    this.rows, {
    this.header = true,
    this.align = 'left',
    this.tableWidth = 1.0,
    List<double>? widths,
    this.aligns,
  }) : widths = _normalizeWidths(widths, rows.isEmpty ? 0 : rows.first.length);

  /// Rows of cells; every row has the same length (`columns`).
  final List<List<String>> rows;
  final bool header;

  /// Cell text alignment for the whole table: `left` | `center` | `right`.
  final String align;

  /// GFM per-column alignment from the separator's colons (`''` | `left` |
  /// `center` | `right`); overrides [align] per column when present.
  final List<String>? aligns;

  /// Effective alignment for column [c].
  String alignFor(int c) {
    final a = aligns;
    if (a != null && c < a.length && a[c].isNotEmpty) return a[c];
    return align;
  }

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
          rows.add([for (final cell in row) _cellText(cell)]);
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
    final rawAligns = data['aligns'];
    return TableData(
      rows,
      header: data['header'] != false,
      align: align,
      tableWidth: tw is num ? tw.toDouble().clamp(0.15, 1.0) : 1.0,
      widths: widths,
      aligns: rawAligns is List ? [for (final a in rawAligns) '$a'] : null,
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
    if (aligns != null && aligns!.any((a) => a.isNotEmpty)) 'aligns': aligns,
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

  /// Coerce a stored cell to its text. A missing/absent cell must become an
  /// empty string — NOT the stringified placeholder a blind interpolation
  /// produces (`'$cell'` turns Dart `null` into "null", and on dart2js a
  /// JS `undefined` array hole into "undefined", which then renders and
  /// round-trips as that literal word). Real string cells pass through; any
  /// other JSON scalar (a number, say) keeps its textual form.
  static String _cellText(Object? cell) {
    if (cell == null) return '';
    if (cell is String) return cell;
    return '$cell';
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
  // The separator's cell count must MATCH the header row (GFM rule).
  if (sepCells.length != _splitRow(first).length) return false;
  return sepCells.every((c) => RegExp(r'^:?-{1,}:?$').hasMatch(c.trim()));
}

/// Parse a GFM table beginning at [index]; returns the table and the index just
/// past it.
({TableData table, int next}) parseGfmTable(List<String> lines, int index) {
  final header = _splitRow(lines[index]);
  // Column alignment from the separator's colons.
  final rawAligns = [
    for (final c in _splitRow(lines[index + 1]))
      switch ((c.trim().startsWith(':'), c.trim().endsWith(':'))) {
        (true, true) => 'center',
        (false, true) => 'right',
        (true, false) => 'left',
        _ => '',
      },
  ];
  var i = index + 2; // skip header + separator
  final body = <List<String>>[];
  while (i < lines.length) {
    final row = lines[i].trim();
    if (row.isEmpty) break;
    // A pipe-less line still belongs to the table unless it starts another
    // block (GFM: the table breaks at a blank line or a new block).
    if (!row.contains('|') && _startsBlock(row)) break;
    body.add(_splitRow(row));
    i++;
  }
  // The header defines the column count (GFM): longer rows truncate,
  // shorter ones pad.
  final width = header.isEmpty ? 1 : header.length;
  List<String> fit(List<String> r) => [
        for (var k = 0; k < width; k++) k < r.length ? r[k] : '',
      ];
  final rows = <List<String>>[fit(header), for (final r in body) fit(r)];
  return (
    table: TableData(
      rows,
      header: true,
      aligns: rawAligns.any((a) => a.isNotEmpty) ? fit(rawAligns) : null,
    ),
    next: i
  );
}

bool _startsBlock(String content) {
  if (content.startsWith('#') ||
      content.startsWith('>') ||
      content.startsWith('```') ||
      content.startsWith('~~~') ||
      content.startsWith('- ') ||
      content.startsWith('* ') ||
      content.startsWith('+ ')) {
    return true;
  }
  return isThematicBreak(content);
}

/// Serialize a table to GFM pipe-table Markdown.
String tableToMarkdown(TableData table) {
  if (table.rows.isEmpty) return '';
  String row(List<String> cells) =>
      '| ${cells.map((c) => c.replaceAll('|', r'\|').replaceAll('\n', ' ').trim()).join(' | ')} |';
  final out = StringBuffer();
  out.writeln(row(table.rows.first));
  final sep = [
    for (var c = 0; c < table.columns; c++)
      switch (table.aligns != null && c < table.aligns!.length
          ? table.aligns![c]
          : '') {
        'center' => ':---:',
        'right' => '---:',
        'left' => ':---',
        _ => '---',
      },
  ];
  out.writeln('| ${sep.join(' | ')} |');
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
