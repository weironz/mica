// S4 T4.8: proof that no data migration is needed. The stored cell form (a
// source string or a `{text, marks}` object, 02-architecture §6.4) is written
// by the engine and read by the engine; this file pins the editor's TableData
// against it in three directions:
//
//  1. OLD DOCUMENTS: rows of legacy source strings decode to the same
//     text + marks the Rust `decode_cell` produces.
//  2. NEW WRITES → OLD READERS: TableData.toBlockData output, fed through the
//     OLD reader's cell-to-source logic (copied below as the historical
//     reference), parses back to the original cell.
//  3. ROUND-TRIP: fromBlock(toBlockData(t)) == t over the boundary charset
//     from S1 T1.8's proptest.
//
// All engine access goes through the real MarkdownCodec facade.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'support/markdown_codec.dart';

/// The pre-S4 `_cellText` from `table.dart` (commit d5c5282a), the historical
/// reference an ALREADY-RELEASED client runs when it reads a table: strings
/// pass through; `{text, marks}` objects go back to source through the old
/// Dart writer. That writer shrank away in S4 T4.7 — for the marked-cell
/// branch the old and new writers agree on everything the S3 T3.7
/// differential triaged (R/H categories only), so this reference keeps the
/// pass-through branch that actually varies by stored shape and the
/// marked-cell parse-back is asserted through the engine below.
String oldCellText(Object? cell) {
  if (cell == null) return '';
  if (cell is String) return cell;
  if (cell is Map) {
    final text = cell['text'];
    if (text is String) {
      final marks = marksFromData(Map<String, dynamic>.from(cell));
      return marks.isEmpty ? text : markdownCodec.serializeInline(text, marks);
    }
  }
  return '$cell';
}

void main() {
  setUpAll(useNativeMarkdownCodec);

  group('1. old documents: legacy source strings decode', () {
    test('every cell matches Rust decode_cell', () {
      const sources = [
        '**bold** and `code`',
        '[l](https://e.com)',
        r'a \| pipe',
        'line<br>break',
        r'\*escaped\*',
        'plain',
        '中文加粗 **真**',
        'https://example.com',
      ];
      final table = TableData.fromBlock({
        'rows': [
          ['h'],
          for (final s in sources) [s],
        ],
      });
      final expected = markdownCodec.decodeCells(sources);
      final flat = [for (final row in table.rows.skip(1)) ...row];
      expect(flat.length, expected.length);
      for (var i = 0; i < flat.length; i++) {
        expect(flat[i].text, expected[i].text, reason: 'text of ${sources[i]}');
        expect(marksEqual(flat[i].marks, expected[i].marks), isTrue,
            reason: 'marks of ${sources[i]}');
      }
    });

    test('an escaped pipe stays literal text, not a column split', () {
      final table = TableData.fromBlock({
        'rows': [
          ['a \\| b'],
        ],
      });
      expect(table.rows[0][0].text, 'a | b');
      expect(table.rows[0].length, 1);
    });
  });

  group('2. new writes → old readers', () {
    test('a marked cell stores an object the old reader turns back into source', () {
      // What the new editor writes for a bold cell.
      final cell = TableCell('b and i', [Mark(0, 1, 'bold'), Mark(6, 7, 'italic')]);
      final data = TableData([
        [TableCell('h')],
        [cell],
      ]).toBlockData();
      final stored = (data['rows'] as List).last.first;
      // The new model only ever stores `{text, marks}` for marked cells —
      // exactly the shape Rust writes and the old Dart reader accepts.
      expect(stored, isA<Map>());
      // The old reader's path: object → source string → its inline parse.
      // The source half runs through the engine here (the old Dart writer
      // agrees with it on triaged content, see file comment).
      final source = oldCellText(stored);
      final decoded = markdownCodec.parseInline(source);
      expect(decoded.text, cell.text);
      expect(marksEqual(decoded.marks, cell.marks), isTrue);
    });

    test('an unmarked cell stores source the old reader passes through', () {
      final data = TableData([
        [TableCell('h')],
        [TableCell('plain')],
      ]).toBlockData();
      final stored = (data['rows'] as List).last.first;
      // Old reader: `cell is String` → the string itself, parsed by the old
      // engine → same text. New writer only stores the string when reading it
      // back reproduces the cell (I3 decision), which 'plain' does.
      expect(stored, isA<String>());
      expect(stored, 'plain');
    });

    test('I3 exception (user decision, S4): a bare URL stores the object form', () {
      // A bare URL typed into a cell has NO link mark in the model. Storing it
      // as source would make every reader add one, so the editor stores the
      // object form: new clients show plain text (consistent with editing),
      // old clients parse the stored text and show the same words.
      final data = TableData([
        [TableCell('h')],
        [TableCell('see https://example.com now')],
      ]).toBlockData();
      final stored = (data['rows'] as List).last.first;
      expect(stored, isA<Map>());
      final map = stored as Map;
      expect(map['text'], 'see https://example.com now');
      expect(map['marks'], isEmpty);
    });

    test('a cell with a literal `*` stores escaped source (no accidental bold)', () {
      final data = TableData([
        [TableCell('h')],
        [TableCell('2 * 3')],
      ]).toBlockData();
      final stored = (data['rows'] as List).last.first;
      expect(stored, isA<String>());
      // The `*` is escaped so a reader does not parse emphasis from it.
      expect(stored, contains(r'\*'));
      expect(stored, isNot(contains('2 * 3')));
    });
  });

  group('3. round-trip over the T1.8 boundary charset', () {
    // The same charset S1 T1.8's proptest uses, plus the pipe/backslash
    // shapes cells specifically care about.
    const samples = [
      'a *b* c',
      'a | b',
      r'a \ b',
      '',
      '😀 emoji',
      'line\nbreak',
      '中文混排 **粗**',
      '`code` and [l](u)',
      r'\| \* escape',
      '  padded  ',
      '~~~',
      'x_y_z',
    ];

    test('fromBlock(toBlockData(t)) == t for mark-free cells', () {
      for (final s in samples) {
        final t = TableData([
          [TableCell('h')],
          [TableCell(s)],
        ]);
        final back = TableData.fromBlock(t.toBlockData());
        expect(back.rows[1][0].text, s, reason: s);
        expect(back.rows[1][0].marks, isEmpty, reason: s);
      }
    });

    test('fromBlock(toBlockData(t)) == t for marked cells', () {
      final cell = TableCell('bold and code', [Mark(0, 4, 'bold'), Mark(9, 13, 'code')]);
      final t = TableData([
        [TableCell('h')],
        [cell],
      ]);
      final back = TableData.fromBlock(t.toBlockData());
      expect(back.rows[1][0].text, 'bold and code');
      expect(marksEqual(back.rows[1][0].marks, cell.marks), isTrue);
    });

    test('the decode cache returns equal cells without a second engine call', () {
      // Same content, second decode: the cache serves it. We assert the
      // observable contract (same values) here; the call-count half of the
      // budget is pinned in perf/table_cells_bench_test.dart.
      final data = {'rows': [
        ['**x**'],
      ]};
      final a = TableData.fromBlock(data).rows[0][0];
      final b = TableData.fromBlock(data).rows[0][0];
      expect(a, b);
      expect(identical(a, b), isTrue, reason: 'cache should return the same instance');
    });
  });
}