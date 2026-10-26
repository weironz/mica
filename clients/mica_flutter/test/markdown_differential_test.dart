// S0 differential harness: the same Markdown input through BOTH implementations.
//
// The Dart mirror (`lib/editor/markdown.dart`) and the Rust engine must agree on
// how an input becomes blocks and how those blocks serialize back. Today they
// are two implementations of one grammar; S3/S4 delete the Dart one. This test
// is the safety net for that deletion: every difference it finds is a behaviour
// change a user would see.
//
// S0 reaches Rust through the EXISTING desktop FFI (`MicaDocument`), because the
// S2 `MarkdownCodec` facade does not exist yet. That route goes through yrs, so
// marks come back sorted/merged by the CRDT. When the only difference is that
// normalization, the verdict is `crdt-normalization`, not a grammar drift —
// proven by pushing the Dart blocks through the same CRDT and re-comparing.
//
// Outputs (not committed — `build/` is ignored):
//   build/differential/report.json   every difference, one record per field
//   build/differential/summary.md    counts per category + first 10 examples
//
// The ratchet (`test/differential_baseline.json`, committed) records what was
// different on the day S0 landed. The assertion is subset-only: a difference may
// disappear, a NEW one fails the test. Regenerate deliberately, with a reason:
//
//   S0_WRITE_BASELINE=1 flutter test test/markdown_differential_test.dart
//
// @TestOn('vm') — native FFI, no web equivalent until S2.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/src/rust/api/document.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

import 'support/markdown_corpus.dart';
import 'support/native_build.dart';

typedef _Blocks = List<({String kind, String text, Map<String, dynamic> data})>;
typedef _Diff = ({
  String id,
  String category,
  String field,
  String dart,
  String rust,
});

const String _reportDir = 'build/differential';
const String _baselinePath = 'test/differential_baseline.json';

void main() {
  setUpAll(initCheckedNativeRust);
  tearDownAll(() async => RustLib.dispose());

  test('parse divergence is not counted as Rust own roundtrip failure', () {
    final sample = markdownCorpus(
      fuzzCount: 0,
    ).firstWhere((entry) => entry.id == 'spec:6');
    final diffs = _diffOne(sample.id, sample.markdown);
    expect(
      diffs.any((d) => d.category == 'kind'),
      isTrue,
      reason: 'this fixture must exercise different parser outputs',
    );
    expect(
      diffs.where((d) => d.category == 'roundtrip'),
      isEmpty,
      reason: 'both implementations round-trip their own distinct blocks',
    );
  });

  test('difference limit fails instead of reporting a truncated corpus', () {
    expect(() => _requireCompleteCorpus(50000), returnsNormally);
    expect(() => _requireCompleteCorpus(50001), throwsStateError);
  });

  test(
    'Dart mirror and Rust engine agree on every corpus input',
    timeout: const Timeout(Duration(minutes: 20)),
    () {
      final corpus = markdownCorpus();
      final diffs = <_Diff>[];

      for (final entry in corpus) {
        diffs.addAll(_diffOne(entry.id, entry.markdown));
        _requireCompleteCorpus(diffs.length);
      }

      final counts = _countByCategory(diffs);
      _writeReport(corpus.length, counts, diffs);
      _checkRatchet(counts, diffs);
    },
  );
}

void _requireCompleteCorpus(int differenceCount) {
  if (differenceCount > 50000) {
    throw StateError(
      'difference limit exceeded ($differenceCount): '
      'corpus incomplete; report and baseline were not written',
    );
  }
}

// ---------------------------------------------------------------------------
// One input
// ---------------------------------------------------------------------------

List<_Diff> _diffOne(String id, String markdown) {
  final out = <_Diff>[];

  // --- parse -------------------------------------------------------------
  _Blocks? dartBlocks;
  _Blocks? rustBlocks;
  Object? dartError;
  Object? rustError;
  try {
    dartBlocks = _normalizeDartBlocks(markdownToBlocks(markdown));
  } catch (error) {
    dartError = error;
  }
  MicaDocument? rustDoc;
  try {
    rustDoc = MicaDocument.fromMarkdown(markdown: markdown);
    rustBlocks = _normalizeRustBlocks(rustDoc);
  } catch (error) {
    rustError = error;
  }
  if (dartError != null || rustError != null) {
    return [
      (
        id: id,
        category: 'crash',
        field: dartError != null ? 'dart' : 'rust',
        dart: _short(dartError?.toString() ?? ''),
        rust: _short(rustError?.toString() ?? ''),
      ),
    ];
  }

  var blockDiffs = _compareBlocks(id, dartBlocks!, rustBlocks!);
  // A difference that disappears after the Dart result makes the same CRDT
  // round trip is normalization, not grammar.
  if (blockDiffs.isNotEmpty) {
    final viaCrdt = _roundTripThroughCrdt(dartBlocks);
    if (viaCrdt != null && _compareBlocks(id, viaCrdt, rustBlocks).isEmpty) {
      blockDiffs = [
        for (final d in blockDiffs)
          (
            id: d.id,
            category: 'crdt-normalization',
            field: d.field,
            dart: d.dart,
            rust: d.rust,
          ),
      ];
    }
  }
  out.addAll(blockDiffs);

  // --- serialization -----------------------------------------------------
  // Both serializers get the SAME blocks: the Dart parse result. Rust is fed
  // through `fromBlocksJson` (no Markdown re-parse), the Dart controller is
  // loaded with the same specs.
  String? dartMd;
  String? rustMd;
  try {
    dartMd = _dartDocumentMarkdown(dartBlocks);
  } catch (error) {
    out.add((
      id: id,
      category: 'crash',
      field: 'dart-serialize',
      dart: _short(error.toString()),
      rust: '',
    ));
  }
  try {
    rustMd = _rustExportMarkdown(dartBlocks);
  } catch (error) {
    out.add((
      id: id,
      category: 'crash',
      field: 'rust-serialize',
      dart: '',
      rust: _short(error.toString()),
    ));
  }
  if (dartMd != null && rustMd != null && dartMd != rustMd) {
    out.add((
      id: id,
      category: 'export',
      field: 'markdown',
      dart: _short(dartMd),
      rust: _short(rustMd),
    ));
  }

  // --- round trip --------------------------------------------------------
  // Each side's own output must re-parse to its own blocks.
  if (dartMd != null) {
    try {
      final again = _normalizeDartBlocks(markdownToBlocks(dartMd));
      final rt = _compareBlocks(id, again, dartBlocks);
      out.addAll([
        for (final d in rt)
          (
            id: d.id,
            category: 'roundtrip',
            field: 'dart:${d.field}',
            dart: d.dart,
            rust: d.rust,
          ),
      ]);
    } catch (error) {
      out.add((
        id: id,
        category: 'roundtrip',
        field: 'dart:crash',
        dart: _short(error.toString()),
        rust: '',
      ));
    }
  }
  {
    try {
      // The export comparison above deliberately serializes Dart's blocks on
      // both sides. Rust's OWN round trip must start from its own parse result;
      // otherwise a parse disagreement is counted again as a round-trip error.
      final rustOwnMd = _rustExportMarkdown(rustBlocks);
      final again = _normalizeRustBlocks(
        MicaDocument.fromMarkdown(markdown: rustOwnMd),
      );
      final rt = _compareBlocks(id, again, rustBlocks);
      out.addAll([
        for (final d in rt)
          (
            id: d.id,
            category: 'roundtrip',
            field: 'rust:${d.field}',
            dart: d.dart,
            rust: d.rust,
          ),
      ]);
    } catch (error) {
      out.add((
        id: id,
        category: 'roundtrip',
        field: 'rust:crash',
        dart: '',
        rust: _short(error.toString()),
      ));
    }
  }

  return out;
}

// ---------------------------------------------------------------------------
// Normalization (02-architecture §3.4)
// ---------------------------------------------------------------------------

_Blocks _normalizeDartBlocks(List<BlockSpec> specs) => [
  for (final s in specs) (kind: s.kind, text: s.text, data: _sortMarks(s.data)),
];

_Blocks _normalizeRustBlocks(MicaDocument doc) {
  final blocks = (jsonDecode(doc.toBlocksJson()) as List)
      .cast<Map<String, dynamic>>();
  final byId = {for (final b in blocks) b['id'] as String: b};
  final root = byId[doc.rootBlockId()];
  if (root == null) return const [];
  final children = (root['children'] as List? ?? const []).cast<String>();
  final out = <({String kind, String text, Map<String, dynamic> data})>[];
  for (final childId in children) {
    final block = byId[childId];
    if (block == null) continue;
    out.add((
      kind: (block['type'] as String?) ?? '',
      text: (block['text'] as String?) ?? '',
      data: _sortMarks(_dataMap(block['data'])),
    ));
  }
  return out;
}

Map<String, dynamic> _dataMap(Object? raw) {
  if (raw is Map) {
    return {
      for (final entry in raw.entries)
        entry.key as String: _canonicalValue(entry.value),
    };
  }
  return <String, dynamic>{};
}

/// Marks compare by `(start, end, type)`; other data keys by value.
Map<String, dynamic> _sortMarks(Map<String, dynamic> data) {
  final marks = data['marks'];
  if (marks is! List) return data;
  final sorted = [...marks]
    ..sort((a, b) {
      final ma = a as Map;
      final mb = b as Map;
      final s = (ma['start'] as num? ?? 0).compareTo(mb['start'] as num? ?? 0);
      if (s != 0) return s;
      final e = (ma['end'] as num? ?? 0).compareTo(mb['end'] as num? ?? 0);
      if (e != 0) return e;
      return (ma['type'] as String? ?? '').compareTo(
        mb['type'] as String? ?? '',
      );
    });
  return {
    ...data,
    'marks': [for (final m in sorted) _canonicalValue(m)],
  };
}

Object? _canonicalValue(Object? v) {
  if (v is Map) {
    final keys = v.keys.cast<String>().toList()..sort();
    return {for (final k in keys) k: _canonicalValue(v[k])};
  }
  if (v is List) return [for (final e in v) _canonicalValue(e)];
  if (v is num && v.toDouble() == v.toInt()) return v.toInt();
  return v;
}

String _canon(Object? v) => jsonEncode(_canonicalValue(v));

/// Push Dart blocks through the same yrs document the Rust side uses, then
/// read them back. If this matches the Rust parse, the difference was CRDT
/// normalization (mark merge/order, id minting), not grammar.
_Blocks? _roundTripThroughCrdt(_Blocks blocks) {
  try {
    final ids = [for (var i = 0; i < blocks.length; i++) 'd$i'];
    final json = jsonEncode([
      {
        'id': 'root',
        'type': 'page',
        'text': '',
        'data': <String, dynamic>{},
        'children': ids,
      },
      for (var i = 0; i < blocks.length; i++)
        {
          'id': ids[i],
          'type': blocks[i].kind,
          'text': blocks[i].text,
          'data': blocks[i].data,
          'children': <String>[],
        },
    ]);
    return _normalizeRustBlocks(
      MicaDocument.fromBlocksJson(rootId: 'root', blocksJson: json),
    );
  } catch (_) {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

List<_Diff> _compareBlocks(String id, _Blocks dart, _Blocks rust) {
  final out = <_Diff>[];
  final count = dart.length > rust.length ? dart.length : rust.length;
  for (var i = 0; i < count; i++) {
    if (i >= dart.length || i >= rust.length) {
      out.add((
        id: id,
        category: 'block-count',
        field: 'block@$i',
        dart: _short(i < dart.length ? _describe(dart[i]) : ''),
        rust: _short(i < rust.length ? _describe(rust[i]) : ''),
      ));
      continue;
    }
    final d = dart[i];
    final r = rust[i];
    if (d.kind != r.kind) {
      out.add((
        id: id,
        category: 'kind',
        field: 'block@$i.kind',
        dart: d.kind,
        rust: r.kind,
      ));
    }
    if (d.text != r.text) {
      out.add((
        id: id,
        category: 'text',
        field: 'block@$i.text',
        dart: _short(d.text),
        rust: _short(r.text),
      ));
    }
    final dKeys = d.data.keys.toSet();
    final rKeys = r.data.keys.toSet();
    for (final key in {...dKeys, ...rKeys}) {
      final dv = d.data[key];
      final rv = r.data[key];
      if (_canon(dv) == _canon(rv)) continue;
      out.add((
        id: id,
        category: key == 'marks' ? 'marks' : 'data:$key',
        field: 'block@$i.data.$key',
        dart: _short(_canon(dv)),
        rust: _short(_canon(rv)),
      ));
    }
  }
  return out;
}

String _describe(({String kind, String text, Map<String, dynamic> data}) b) =>
    '${b.kind}:${b.text.length}:${jsonEncode(b.data)}';

// ---------------------------------------------------------------------------
// Serialization through each implementation
// ---------------------------------------------------------------------------

String _dartDocumentMarkdown(_Blocks blocks) {
  final controller = EditorController(rootBlockId: 'root', onOps: (_) async {});
  try {
    controller.load([
      for (var i = 0; i < blocks.length; i++)
        EditorNode(
          id: 'd$i',
          kind: blocks[i].kind,
          text: blocks[i].text,
          data: Map<String, dynamic>.from(blocks[i].data),
        ),
    ]);
    return controller.documentMarkdown();
  } finally {
    controller.dispose();
  }
}

String _rustExportMarkdown(_Blocks blocks) {
  final ids = [for (var i = 0; i < blocks.length; i++) 'd$i'];
  final json = jsonEncode([
    {
      'id': 'root',
      'type': 'page',
      'text': '',
      'data': <String, dynamic>{},
      'children': ids,
    },
    for (var i = 0; i < blocks.length; i++)
      {
        'id': ids[i],
        'type': blocks[i].kind,
        'text': blocks[i].text,
        'data': blocks[i].data,
        'children': <String>[],
      },
  ]);
  return MicaDocument.fromBlocksJson(
    rootId: 'root',
    blocksJson: json,
  ).exportMarkdown();
}

// ---------------------------------------------------------------------------
// Report + ratchet
// ---------------------------------------------------------------------------

Map<String, int> _countByCategory(List<_Diff> diffs) {
  final counts = <String, int>{};
  for (final d in diffs) {
    counts[d.category] = (counts[d.category] ?? 0) + 1;
  }
  return {for (final k in counts.keys.toList()..sort()) k: counts[k]!};
}

String _entryKey(_Diff d) => '${d.id}|${d.category}|${d.field}';

void _writeReport(int corpusSize, Map<String, int> counts, List<_Diff> diffs) {
  final dir = Directory(_reportDir)..createSync(recursive: true);
  final report = [
    for (final d in diffs)
      {
        'id': d.id,
        'category': d.category,
        'field': d.field,
        'dart': d.dart,
        'rust': d.rust,
      },
  ];
  File(
    '${dir.path}/report.json',
  ).writeAsStringSync(const JsonEncoder.withIndent(' ').convert(report));

  final byCategory = <String, List<_Diff>>{};
  for (final d in diffs) {
    (byCategory[d.category] ??= []).add(d);
  }
  final summary = StringBuffer()
    ..writeln('# Differential report')
    ..writeln()
    ..writeln('Corpus: $corpusSize inputs.')
    ..writeln()
    ..writeln('| category | count |')
    ..writeln('| --- | --- |');
  for (final entry in counts.entries) {
    summary.writeln('| ${entry.key} | ${entry.value} |');
  }
  summary.writeln();
  summary.writeln('First 10 differences per category:');
  for (final entry in byCategory.entries) {
    summary
      ..writeln()
      ..writeln('## ${entry.key} (${entry.value.length})');
    for (final d in entry.value.take(10)) {
      summary
        ..writeln()
        ..writeln('- `${d.id}` `${d.field}`')
        ..writeln('  - dart: `${d.dart}`')
        ..writeln('  - rust: `${d.rust}`');
    }
  }
  File('${dir.path}/summary.md').writeAsStringSync(summary.toString());
  // ignore: avoid_print
  print('differential: wrote ${dir.path}/report.json and summary.md');
}

void _checkRatchet(Map<String, int> counts, List<_Diff> diffs) {
  final baselineFile = File(_baselinePath);
  final writeMode = Platform.environment['S0_WRITE_BASELINE'] == '1';

  if (writeMode) {
    baselineFile.writeAsStringSync(
      const JsonEncoder.withIndent(' ').convert({
        'counts': counts,
        'entries': (diffs.map(_entryKey).toSet().toList()..sort()),
      }),
    );
    // ignore: avoid_print
    print('differential: wrote ratchet baseline to $_baselinePath');
    return;
  }

  expect(
    baselineFile.existsSync(),
    isTrue,
    reason:
        'missing $_baselinePath — generate it once with '
        'S0_WRITE_BASELINE=1 flutter test test/markdown_differential_test.dart',
  );
  final baseline =
      jsonDecode(baselineFile.readAsStringSync()) as Map<String, dynamic>;
  final baseCounts = (baseline['counts'] as Map).cast<String, int>();
  final baseEntries = (baseline['entries'] as List).cast<String>().toSet();

  final added = <String>[
    for (final d in diffs)
      if (!baseEntries.contains(_entryKey(d))) _entryKey(d),
  ]..sort();
  final grown = <String>[
    for (final entry in counts.entries)
      if (entry.value > (baseCounts[entry.key] ?? 0))
        '${entry.key}: ${baseCounts[entry.key] ?? 0} -> ${entry.value}',
  ];

  expect(
    added,
    isEmpty,
    reason:
        'NEW differences against the Rust engine. A difference is only '
        'acceptable if it is recorded in $_baselinePath with a reason; the '
        'first few: ${added.take(10).join(', ')}',
  );
  expect(
    grown,
    isEmpty,
    reason: 'category counts grew past the S0 baseline: ${grown.join('; ')}',
  );
}

String _short(String s, [int max = 300]) =>
    s.length <= max ? s : '${s.substring(0, max)}…(${s.length} chars)';
