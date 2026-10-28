/// Table model for the editor.
///
/// A table is a single block of kind `table`; its grid lives in the block's
/// `data` as `{ "rows": [[cell, …], …], "header": true }`. The main caret treats
/// the block as atomic — cells are edited through an overlay field (see
/// editor.dart) rather than the flat text-caret model, which keeps tables from
/// requiring a cell-addressing rewrite of the selection engine.
///
/// In memory a cell is [TableCell] — plain text plus inline marks, the same
/// model a paragraph uses. The STORED form is still the engine's (a source
/// string or `{text, marks}`, docs/single-source/02-architecture.md §6.4):
/// decoding and encoding go through `markdownCodec`, so nothing in the editor
/// parses or writes Markdown syntax.
library;

import 'dart:collection';

import 'package:flutter/foundation.dart' show immutable;

import '../markdown/markdown_codec.dart'
    show InlineText, MarkdownCodecException, markdownCodec;
import 'marks.dart' show Mark, marksEqual, marksFromData, marksToJson;

/// One table cell: plain text plus inline marks over it — the same model a
/// paragraph uses. The STORED form is still the engine's (a source string or
/// `{text, marks}`, docs/single-source/02-architecture.md §6.4); decoding and
/// encoding go through [markdownCodec] in [TableData.fromBlock] /
/// [TableData.toBlockData], so nothing in the editor parses Markdown.
@immutable
class TableCell {
  const TableCell(this.text, [this.marks = const []]);

  static const empty = TableCell('');

  final String text;
  final List<Mark> marks;

  bool get isEmpty => text.isEmpty;

  TableCell copyWith({String? text, List<Mark>? marks}) =>
      TableCell(text ?? this.text, marks ?? this.marks);

  @override
  bool operator ==(Object other) =>
      other is TableCell && other.text == text && marksEqual(other.marks, marks);

  @override
  int get hashCode => Object.hash(
    text,
    Object.hashAll(marks.map((m) => (m.start, m.end, m.type, m.href, m.title))),
  );

  @override
  String toString() => 'TableCell(${text.length} chars, ${marks.length} marks)';
}

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
  final List<List<TableCell>> rows;
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

  /// Bound of the content-keyed decode cache. Layout calls [TableData.fromBlock]
  /// on every relayout, so decoded cells are cached; 4096 entries covers a
  /// large table plus the cells of the pages around it.
  static const int _cacheLimit = 4096;

  /// Source string → decoded cell. Insertion order doubles as LRU order.
  static final LinkedHashMap<String, TableCell> _sourceCache =
      LinkedHashMap<String, TableCell>();

  /// [TableCell] → its stored form (a source string or a `{text, marks}`
  /// object). Value equality on [TableCell] makes this a content cache too.
  static final LinkedHashMap<TableCell, Object> _storedCache =
      LinkedHashMap<TableCell, Object>();

  factory TableData.fromBlock(Map<String, dynamic> data) {
    final raw = data['rows'];
    final shapes = <List<Object?>>[];
    if (raw is List) {
      for (final row in raw) {
        if (row is List) shapes.add(row);
      }
    }
    final decoded = _decodeCells([for (final row in shapes) ...row]);
    var at = 0;
    final rows = <List<TableCell>>[];
    for (final row in shapes) {
      rows.add(decoded.sublist(at, at + row.length));
      at += row.length;
    }
    if (rows.isEmpty) {
      return TableData.empty();
    }
    final width = rows.fold<int>(0, (m, r) => r.length > m ? r.length : m);
    for (final row in rows) {
      while (row.length < width) {
        row.add(TableCell.empty);
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
    [TableCell('Column 1'), TableCell('Column 2'), TableCell('Column 3')],
    [TableCell.empty, TableCell.empty, TableCell.empty],
  ]);

  Map<String, dynamic> toBlockData() {
    final encoded = _encodeCells([for (final row in rows) ...row]);
    var at = 0;
    final outRows = <List<Object>>[];
    for (final row in rows) {
      outRows.add(encoded.sublist(at, at + row.length));
      at += row.length;
    }
    return {
      'rows': outRows,
      'header': header,
      'align': align,
      'widths': widths,
      if (aligns != null && aligns!.any((a) => a.isNotEmpty)) 'aligns': aligns,
      // Default-width tables omit the key (keeps parser conformance with the
      // Rust engine, which doesn't model table width).
      if (tableWidth != 1.0) 'width': tableWidth,
    };
  }

  void insertRow(int at) {
    final i = at.clamp(0, rows.length);
    rows.insert(i, List<TableCell>.filled(columns, TableCell.empty));
  }

  void deleteRow(int at) {
    if (rows.length <= 1 || at < 0 || at >= rows.length) return;
    rows.removeAt(at);
  }

  void insertColumn(int at) {
    final i = at.clamp(0, columns);
    for (final row in rows) {
      row.insert(i.clamp(0, row.length), TableCell.empty);
    }
    widths.insert(i.clamp(0, widths.length), 1.0);
    aligns?.insert(i.clamp(0, aligns!.length), '');
  }

  void deleteColumn(int at) {
    if (columns <= 1 || at < 0 || at >= columns) return;
    for (final row in rows) {
      if (at < row.length) row.removeAt(at);
    }
    if (at < widths.length) widths.removeAt(at);
    if (aligns != null && at < aligns!.length) aligns!.removeAt(at);
  }

  /// Stored cells → [TableCell]. `{text, marks}` objects decode locally (they
  /// already ARE text + marks); only source STRINGS need the engine, and those
  /// are cached by content — layout calls [fromBlock] on every relayout.
  static List<TableCell> _decodeCells(List<Object?> cells) {
    final out = List<TableCell?>.filled(cells.length, null);
    final stringAt = <int>[];
    final strings = <String>[];
    for (var i = 0; i < cells.length; i++) {
      final cell = cells[i];
      if (cell == null) {
        out[i] = TableCell.empty;
      } else if (cell is String) {
        stringAt.add(i);
        strings.add(cell);
      } else if (cell is Map) {
        final text = cell['text'];
        out[i] = text is String
            ? TableCell(text, marksFromData(Map<String, dynamic>.from(cell)))
            : TableCell.empty;
      } else {
        out[i] = TableCell('$cell');
      }
    }
    if (stringAt.isNotEmpty) {
      final decoded = _decodeSources(strings);
      for (var k = 0; k < stringAt.length; k++) {
        out[stringAt[k]] = decoded[k];
      }
    }
    return out.cast<TableCell>();
  }

  /// Decode source strings, deduplicated, one engine call per cache misses.
  /// The batch is capped so one call's wire JSON stays small: decoding 500
  /// cells in one batch spent ~10 ms in the Dart-side jsonDecode of the
  /// single result array (many small decodes are cheaper than one big one).
  static const int _batchSize = 64;

  static List<TableCell> _decodeSources(List<String> sources) {
    final out = List<TableCell?>.filled(sources.length, null);
    final unique = <String>[];
    final positions = <String, List<int>>{};
    for (var i = 0; i < sources.length; i++) {
      final source = sources[i];
      final hit = _sourceCache.remove(source);
      if (hit != null) {
        _sourceCache[source] = hit; // LRU bump
        out[i] = hit;
        continue;
      }
      final at = positions[source];
      if (at == null) {
        positions[source] = [i];
        unique.add(source);
      } else {
        at.add(i);
      }
    }
    for (var start = 0; start < unique.length; start += _batchSize) {
      final end = (start + _batchSize).clamp(0, unique.length);
      final batch = unique.sublist(start, end);
      final fresh = _decodeViaEngine(batch);
      if (fresh == null) {
        // Engine unavailable (02 §5.2): show the source as literal text.
        // Not cached, so a later call with a working engine still decodes it.
        for (var k = 0; k < batch.length; k++) {
          final cell = TableCell(batch[k]);
          for (final i in positions[batch[k]]!) {
            out[i] = cell;
          }
        }
        continue;
      }
      for (var k = 0; k < batch.length; k++) {
        final cell = TableCell(fresh[k].text, fresh[k].marks);
        _remember(_sourceCache, batch[k], cell);
        for (final i in positions[batch[k]]!) {
          out[i] = cell;
        }
      }
    }
    return out.cast<TableCell>();
  }

  /// [TableCell] → stored form. Cached by content; only new cells reach the
  /// engine, so a width drag over a 500-cell table makes zero engine calls.
  static List<Object> _encodeCells(List<TableCell> cells) {
    final out = List<Object?>.filled(cells.length, null);
    final missed = <int>[];
    for (var i = 0; i < cells.length; i++) {
      final hit = _storedCache.remove(cells[i]);
      if (hit != null) {
        _storedCache[cells[i]] = hit; // LRU bump
        out[i] = hit;
      } else {
        missed.add(i);
      }
    }
    if (missed.isEmpty) return out.cast<Object>();
    final pending = <InlineText>[
      for (final i in missed) (text: cells[i].text, marks: cells[i].marks),
    ];
    List<Object>? encoded;
    try {
      encoded = markdownCodec.encodeCells(pending);
    } on MarkdownCodecException {
      encoded = null;
    } on StateError {
      encoded = null; // codec never initialized: same degradation
    }
    if (encoded == null) {
      // Object form is always correct, just less compact — and readers take it
      // as-is, so nothing is lost.
      for (var k = 0; k < missed.length; k++) {
        final cell = cells[missed[k]];
        final stored = _storedObject(cell);
        out[missed[k]] = stored;
        _remember(_storedCache, cell, stored);
      }
      return out.cast<Object>();
    }
    // I3 exception (G1 #4, decided in S4 T4.8 option b): a mark-free cell is
    // stored as Markdown source, which a reader parses back. Store that source
    // only when re-reading it reproduces the cell EXACTLY; otherwise (a bare
    // URL / e-mail gains a `link` mark, leading/trailing spaces are trimmed,
    // …) store the object form, so editing and re-opening agree.
    final stringAt = <int>[];
    final strings = <String>[];
    for (var k = 0; k < missed.length; k++) {
      final value = encoded[k];
      if (value is String) {
        stringAt.add(missed[k]);
        strings.add(value);
      } else {
        out[missed[k]] = value;
      }
    }
    if (strings.isNotEmpty) {
      final decoded = _decodeSources(strings);
      for (var k = 0; k < strings.length; k++) {
        final i = stringAt[k];
        final cell = cells[i];
        out[i] = decoded[k] == cell ? strings[k] : _storedObject(cell);
      }
    }
    for (var k = 0; k < missed.length; k++) {
      _remember(_storedCache, cells[missed[k]], out[missed[k]]!);
    }
    return out.cast<Object>();
  }

  static Map<String, dynamic> _storedObject(TableCell cell) => {
    'text': cell.text,
    'marks': marksToJson(cell.marks),
  };

  static List<InlineText>? _decodeViaEngine(List<String> sources) {
    try {
      return markdownCodec.decodeCells(sources);
    } on MarkdownCodecException {
      return null;
    } on StateError {
      return null;
    }
  }

  static void _remember<K, V>(LinkedHashMap<K, V> cache, K key, V value) {
    cache.remove(key);
    cache[key] = value;
    while (cache.length > _cacheLimit) {
      cache.remove(cache.keys.first);
    }
  }

  static List<double> _normalizeWidths(List<double>? widths, int columns) {
    if (columns <= 0) return [];
    if (widths == null || widths.length != columns) {
      return List<double>.filled(columns, 1.0);
    }
    return [for (final w in widths) w <= 0 ? 1.0 : w];
  }
}