// The embedded benchmark sample must still be the fixture directory: Chrome
// cannot read files, so the sample is a copy, and a copy that silently goes
// stale would make the VM/Chrome numbers incomparable.
//
// @TestOn('vm') — it reads the fixture directory.
@TestOn('vm')
library;

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'support/markdown_bench_inputs.dart';
import 'support/markdown_corpus.dart';

void main() {
  test('embedded benchmark sample matches the conformance fixtures', () {
    final dir = Directory('$markdownFixtureRoot/conformance');
    final files = dir
        .listSync()
        .whereType<File>()
        .where((f) => f.path.endsWith('.md'))
        .toList()
      ..sort((a, b) => a.path.compareTo(b.path));
    final onDisk = files.map((f) => f.readAsStringSync()).join();
    expect(markdownFixtureSample, onDisk,
        reason: 'regenerate test/support/markdown_bench_inputs.dart from '
            'crates/markdown/tests/fixtures/conformance/*.md');
  });
}