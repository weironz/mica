// Guards the S0 corpus itself: ids are unique, every source is present, the
// golden-master coverage arithmetic holds, and the embedded benchmark sample
// still matches the fixture files.
//
// @TestOn('vm') because the consistency check reads the fixture directory.
@TestOn('vm')
library;

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'support/markdown_corpus.dart';

void main() {
  final corpus = markdownCorpus(fuzzCount: 20);

  test('corpus ids are unique', () {
    final ids = corpus.map((e) => e.id).toList();
    expect(ids.toSet().length, ids.length);
  });

  test('every source is represented', () {
    int count(String prefix) => corpus.where((e) => e.id.startsWith(prefix)).length;
    expect(count('spec:'), 652);
    expect(count('gfm:'), 24);
    expect(count('golden:'), 29);
    expect(count('conf:'), Directory('$markdownFixtureRoot/conformance')
        .listSync()
        .whereType<File>()
        .where((f) => f.path.endsWith('.md'))
        .length);
    expect(count('file:'), 2);
    expect(count('fuzz:'), 20);
  });

  test('golden-master.json keys are exactly the corpus inputs', () {
    final covered = goldenMasterCoveredKeys(corpus);
    final expected = goldenMasterKeys();
    expect(covered.difference(expected), isEmpty,
        reason: 'corpus ids with no golden-master key');
    expect(expected.difference(covered), isEmpty,
        reason: 'golden-master inputs missing from the corpus — a new '
            'adversarial case in crates/markdown/tests/golden_master.rs must '
            'be added to test/support/markdown_corpus.dart');
  });

  test('fuzz corpus is deterministic for a seed', () {
    final a = fuzzCorpus(count: 50, seed: 7);
    final b = fuzzCorpus(count: 50, seed: 7);
    expect([for (final e in a) e.markdown],
        [for (final e in b) e.markdown]);
    final c = fuzzCorpus(count: 50, seed: 8);
    expect([for (final e in a) e.markdown],
        isNot([for (final e in c) e.markdown]));
  });
}