// S0 differential harness: the same Markdown input through BOTH implementations.
//
// The Dart mirror (`lib/editor/markdown.dart`) and the Rust engine must agree on
// how an input becomes blocks and how those blocks serialize back. Today they
// are two implementations of one grammar; S3/S4 delete the Dart one. This test
// is the safety net for that deletion: every difference it finds is a behaviour
// change a user would see.
//
// S2 T2.12 reaches Rust through the real MarkdownCodec facade. The raw engine
// no longer passes through yrs, so CRDT normalization is not a category here.
// Block parsing/serialization and inline parsing/serialization are compared.
//
// Outputs (not committed — `build/` is ignored):
//   build/differential/report.json   every difference, one record per field
//   build/differential/summary.md    counts per category + first 10 examples
//
// The S2 ratchet records full values for the raw codec protocol. Original S0
// keys/counts remain in fixtures/differential/s0_baseline.json; the transition
// audit explains every added/removed/changed field. A difference may disappear,
// but new keys OR changed values fail. Regenerate deliberately, with a reason:
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
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/src/rust/frb_generated.dart';

import 'support/markdown_corpus.dart';
import 'support/markdown_codec.dart';

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
  setUpAll(useNativeMarkdownCodec);
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

  test('ratchet detects same-key value changes beyond character 300', () {
    final long = 'x' * 400;
    final baseline = <String, dynamic>{'dart': long, 'rust': 'old'};
    final original = (
      id: 'same',
      category: 'text',
      field: 'same',
      dart: long,
      rust: 'old',
    );
    expect(_sameDifferenceValue(baseline, original), isTrue);
    expect(
      _sameDifferenceValue(baseline, (
        id: original.id,
        category: original.category,
        field: original.field,
        dart: '${long}changed',
        rust: original.rust,
      )),
      isFalse,
    );
    expect(
      _sameDifferenceValue(baseline, (
        id: original.id,
        category: original.category,
        field: original.field,
        dart: original.dart,
        rust: 'new',
      )),
      isFalse,
    );
  });

  test('description ratchet ignores only JSON object key order', () {
    final baseline = <String, dynamic>{
      'dart': '',
      'rust': 'quote:2:{"li":0,"quote":2}',
    };
    final actual = (
      id: 'x',
      category: 'block-count',
      field: 'block@1',
      dart: '',
      rust: 'quote:2:{"quote":2,"li":0}',
    );
    expect(_sameDifferenceValue(baseline, actual), isTrue);
    expect(
      _sameDifferenceValue(baseline, (
        id: actual.id,
        category: actual.category,
        field: actual.field,
        dart: '',
        rust: 'quote:2:{"quote":3,"li":0}',
      )),
      isFalse,
    );
    expect(
      _sameDifferenceValue(baseline, (
        id: actual.id,
        category: 'text',
        field: 'block@1.text',
        dart: '',
        rust: actual.rust,
      )),
      isFalse,
    );
    for (final changed in [
      'quote:2:{"quote":null,"li":0}',
      'quote:3:{"quote":2,"li":0}',
      'ordinary {"quote":2,"li":0}',
      'quote:2:{invalid}',
    ]) {
      expect(
        _sameDifferenceValue(baseline, (
          id: actual.id,
          category: actual.category,
          field: actual.field,
          dart: '',
          rust: changed,
        )),
        isFalse,
      );
    }
    expect(
      _sameDifferenceValue(
        {'dart': '', 'rust': 'table:0:{"rows":[1,2]}'},
        (
          id: 'x',
          category: 'block-count',
          field: 'block@1',
          dart: '',
          rust: 'table:0:{"rows":[2,1]}',
        ),
      ),
      isFalse,
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
      expect(counts['crdt-normalization'] ?? 0, 0);
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
  try {
    rustBlocks = _normalizeDartBlocks(markdownCodec.parseBlocks(markdown));
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

  out.addAll(_compareBlocks(id, dartBlocks!, rustBlocks!));
  out.addAll(_diffInline(id, markdown));

  // --- serialization -----------------------------------------------------
  // Both serializers get the SAME Dart parse result: the Rust facade receives
  // BlockSpec directly, and the Dart controller is loaded with those specs.
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
      final again = _normalizeDartBlocks(markdownCodec.parseBlocks(rustOwnMd));
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

List<_Diff> _diffInline(String id, String source) {
  try {
    final dart = parseInline(source);
    final rust = markdownCodec.parseInline(source);
    final out = <_Diff>[];
    if (dart.text != rust.text) {
      out.add((
        id: id,
        category: 'inline:text',
        field: 'inline.text',
        dart: _short(dart.text),
        rust: _short(rust.text),
      ));
    }
    final dartMarks = _sortMarks({'marks': marksToJson(dart.marks)})['marks'];
    final rustMarks = _sortMarks({'marks': marksToJson(rust.marks)})['marks'];
    if (_canon(dartMarks) != _canon(rustMarks)) {
      out.add((
        id: id,
        category: 'inline:marks',
        field: 'inline.marks',
        dart: _short(_canon(dartMarks)),
        rust: _short(_canon(rustMarks)),
      ));
    }
    final dartMd = inlineToMarkdown(dart.text, dart.marks);
    final rustMd = markdownCodec.serializeInline(dart.text, dart.marks);
    if (dartMd != rustMd) {
      out.add((
        id: id,
        category: 'inline:export',
        field: 'inline.markdown',
        dart: _short(dartMd),
        rust: _short(rustMd),
      ));
    }
    return out;
  } catch (error) {
    return [
      (
        id: id,
        category: 'crash',
        field: 'inline',
        dart: '',
        rust: _short('$error'),
      ),
    ];
  }
}

// ---------------------------------------------------------------------------
// Normalization (02-architecture §3.4)
// ---------------------------------------------------------------------------

_Blocks _normalizeDartBlocks(List<BlockSpec> specs) => [
  for (final s in specs) (kind: s.kind, text: s.text, data: _sortMarks(s.data)),
];

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
    '${b.kind}:${b.text.length}:${_canon(b.data)}';

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

String _rustExportMarkdown(_Blocks blocks) =>
    markdownCodec.serializeBlocks(blocks);

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
        'protocol': 's2-codec-full-values-v1',
        'counts': counts,
        'entries': (diffs.map(_entryKey).toSet().toList()..sort()),
        'values': {
          for (final d in diffs) _entryKey(d): {'dart': d.dart, 'rust': d.rust},
        },
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
  expect(baseline['protocol'], 's2-codec-full-values-v1');
  final values = baseline['values'] as Map<String, dynamic>;
  final changed = [
    for (final d in diffs)
      if (values.containsKey(_entryKey(d)) &&
          !_sameDifferenceValue(
            values[_entryKey(d)] as Map<String, dynamic>,
            d,
          ))
        _entryKey(d),
  ];
  expect(
    changed,
    isEmpty,
    reason: 'Existing difference values changed: ${changed.take(10)}',
  );

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

// Keep complete values in the report and ratchet. Truncation would miss a
// mutation beyond character 300 while retaining the same id/category/field.
String _short(String s) => s;

bool _sameDifferenceValue(Map<String, dynamic> expected, _Diff actual) {
  // Only block descriptions embed JSON inside a string. §3.4 allows object
  // key order differences there; text/Markdown/HTML remain byte-exact.
  String comparable(String value) {
    final description =
        (actual.category == 'block-count' &&
            RegExp(r'^block@\d+$').hasMatch(actual.field)) ||
        (actual.category == 'roundtrip' &&
            RegExp(r'^(dart:|rust:)?block@\d+$').hasMatch(actual.field));
    if (!description || value.isEmpty) return value;
    final match = RegExp(
      r'^([A-Za-z_][A-Za-z0-9_]*:\d+:)(\{[\s\S]*\})$',
    ).firstMatch(value);
    if (match == null) return value;
    try {
      final object = jsonDecode(match[2]!);
      if (object is! Map) return value;
      return '${match[1]}${_canon(object)}';
    } on FormatException {
      return value;
    }
  }

  return comparable(expected['dart'] as String) == comparable(actual.dart) &&
      comparable(expected['rust'] as String) == comparable(actual.rust);
}
