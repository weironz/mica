import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';

/// The Dart markdown mirror (editor hot paths: paste, AI insert) pinned to
/// the SAME gold fixtures as the Rust engine
/// (`crates/markdown/tests/fixtures/conformance`, regenerated there with
/// `GEN_GOLD=1 cargo test -p mica-markdown --test conformance`). Any grammar
/// drift between the two implementations fails here or there.
void main() {
  final dir = Directory('../../crates/markdown/tests/fixtures/conformance');

  test('fixture directory is reachable', () {
    expect(dir.existsSync(), isTrue,
        reason: 'expected shared fixtures at ${dir.absolute.path}');
  });

  final mdFiles = dir
      .listSync()
      .whereType<File>()
      .where((f) => f.path.endsWith('.md'))
      .toList()
    ..sort((a, b) => a.path.compareTo(b.path));

  for (final md in mdFiles) {
    final name = md.uri.pathSegments.last;
    test('conformance: $name', () {
      final goldFile =
          File(md.path.replaceAll(RegExp(r'\.md$'), '.blocks.json'));
      expect(goldFile.existsSync(), isTrue,
          reason: 'missing gold for $name — GEN_GOLD=1 on the Rust side');
      final gold = jsonDecode(goldFile.readAsStringSync()) as List<dynamic>;

      final specs = markdownToBlocks(md.readAsStringSync());
      final got = [
        for (final s in specs)
          {'kind': s.kind, 'text': s.text, 'data': _sortMarks(s.data)},
      ];

      // Compare via canonical JSON (keys sorted recursively) so map-order
      // and int/double representation differences don't matter.
      expect(
        const JsonEncoder.withIndent('  ').convert(_canon(got)),
        const JsonEncoder.withIndent('  ').convert(_canon(gold)),
        reason: 'grammar drift between Dart mirror and Rust engine in $name',
      );
    });
  }
}

/// Mark array order is semantically irrelevant and differs between the two
/// implementations — sort by range like the Rust gold generator does.
Map<String, dynamic> _sortMarks(Map<String, dynamic> data) {
  final marks = data['marks'];
  if (marks is! List) return data;
  final sorted = [...marks]..sort((a, b) {
      final ma = a as Map, mb = b as Map;
      final s = ((ma['start'] as num).compareTo(mb['start'] as num));
      if (s != 0) return s;
      final e = ((ma['end'] as num).compareTo(mb['end'] as num));
      if (e != 0) return e;
      return (ma['type'] as String).compareTo(mb['type'] as String);
    });
  return {...data, 'marks': sorted};
}

dynamic _canon(dynamic v) {
  if (v is Map) {
    final keys = v.keys.cast<String>().toList()..sort();
    return {for (final k in keys) k: _canon(v[k])};
  }
  if (v is List) return [for (final e in v) _canon(e)];
  if (v is num) return v.toDouble() == v.toInt() ? v.toInt() : v;
  return v;
}
