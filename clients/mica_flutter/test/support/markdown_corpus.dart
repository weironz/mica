// The Markdown inputs the S0 differential harness and the S3/S4 safety net run
// on, plus the performance corpus.
//
// Every entry carries a stable id so a report line names its source. Paths are
// relative to `clients/mica_flutter` (where `flutter test` runs).
//
// Corpus sources, and one correction to the S0 plan:
//
//   1. `crates/markdown/tests/fixtures/commonmark-spec-0.31.2.json` — 652
//   2. `crates/markdown/tests/fixtures/gfm-extensions.json`          — 24
//   3. `golden-master.json` is NOT a corpus of inputs: it stores the expected
//      `DocumentSnapshotPayload` for keys `spec-*`, `gfm-*`, `fixture-*`,
//      `paste`, `vhost-user`, `adv-*`. Its only inputs not already covered by
//      (1), (2), (4) and (5) are the 29 adversarial cases, which live in
//      `crates/markdown/tests/golden_master.rs::adversarial_inputs`. Those 29
//      are reproduced here under `golden:adv-*`; `golden_master_covers_corpus`
//      below proves the two sets add up to exactly the JSON's keys (a new
//      adversarial case on the Rust side therefore fails the check until it is
//      added here).
//   4. `crates/markdown/tests/fixtures/conformance/*.md` — 35 today. The S0
//      plan says 105; that is the number of FILES in the directory (each case
//      has `.md`, `.md.gold` and `.blocks.json`), not fixtures.
//   5. `fixtures/paste.md` and `tests/data/*.md` — 2
//   6. seeded random concatenations of Markdown tokens

import 'dart:convert';
import 'dart:io';

typedef CorpusEntry = ({String id, String markdown});

/// Fixture root, relative to `clients/mica_flutter`.
const String markdownFixtureRoot = '../../crates/markdown/tests/fixtures';

/// Every Markdown input the differential and the benchmarks run on.
///
/// Deterministic for a given `fuzzCount`/`seed`: the fuzz corpus is the only
/// generated part and its generator is fixed.
List<CorpusEntry> markdownCorpus({
  int fuzzCount = 5000,
  int seed = 20261026,
}) {
  final root = Directory(markdownFixtureRoot);
  if (!root.existsSync()) {
    throw StateError('markdown fixtures not found at ${root.absolute.path}');
  }
  final entries = <CorpusEntry>[];

  // 1. CommonMark 0.31.2 spec examples.
  for (final (i, c) in _readJsonArray('$markdownFixtureRoot/'
          'commonmark-spec-0.31.2.json')
      .indexed) {
    final example = c['example'];
    final n = example is num ? example.toInt() : i;
    entries.add((
      id: 'spec:$n',
      markdown: (c['markdown'] as String?) ?? '',
    ));
  }

  // 2. GFM extension cases. No `example` number in this file; the Rust corpus
  //    uses the array index, and so do we (ids only need to be stable).
  for (final (i, c) in
      _readJsonArray('$markdownFixtureRoot/gfm-extensions.json').indexed) {
    entries.add((
      id: 'gfm:$i',
      markdown: (c['markdown'] as String?) ?? '',
    ));
  }

  // 3. Golden-master inputs that no other source provides (adversarial cases).
  for (final (id, markdown) in _adversarialInputs()) {
    entries.add((id: 'golden:$id', markdown: markdown));
  }

  // 4. Curated conformance fixtures, sorted by file name (stable order).
  final confDir = Directory('$markdownFixtureRoot/conformance');
  final confFiles = confDir
      .listSync()
      .whereType<File>()
      .where((f) => f.path.endsWith('.md'))
      .toList()
    ..sort((a, b) => a.path.compareTo(b.path));
  for (final file in confFiles) {
    final name = file.uri.pathSegments.last;
    entries.add((id: 'conf:$name', markdown: file.readAsStringSync()));
  }

  // 5. Whole files: the paste incident fixture and the large-file fixture.
  entries.add((
    id: 'file:paste.md',
    markdown: File('$markdownFixtureRoot/paste.md').readAsStringSync(),
  ));
  final dataDir = Directory('$markdownFixtureRoot/../data');
  if (dataDir.existsSync()) {
    final files = dataDir
        .listSync()
        .whereType<File>()
        .where((f) => f.path.endsWith('.md'))
        .toList()
      ..sort((a, b) => a.path.compareTo(b.path));
    for (final file in files) {
      entries.add((
        id: 'file:${file.uri.pathSegments.last}',
        markdown: file.readAsStringSync(),
      ));
    }
  }

  // 6. Seeded fuzz: 1–40 tokens joined verbatim.
  entries.addAll(fuzzCorpus(count: fuzzCount, seed: seed));

  return entries;
}

/// The token alphabet the fuzz corpus draws from. Kept in one place so the
/// benchmark and the differential see the same shapes.
const List<String> fuzzTokens = [
  '**', '*', '_', '__', '~~', '~', '`', '```', '~~~', r'$', r'$$',
  r'\(', r'\)', '[', ']', '(', ')', '![', '<', '>', 'http://x.y',
  '# ', '## ', '- ', '* ', '1. ', '2) ', '> ', '>> ', '| a | b |',
  '|---|---|', '---', '***', '[^1]', '[^1]: ', '[x]: /u', '\n', '\n\n',
  '  \n', '\\\n', '\t', '    ', '中文', '😀', '𠀀', 'é', '&amp;', '&#123;',
  r'\*', '\\', 'a', 'b c',
];

/// Deterministic fuzz corpus. `Random(seed)` is specified by the Dart SDK
/// (a fixed algorithm), so the same seed gives the same inputs on VM and web.
List<CorpusEntry> fuzzCorpus({int count = 5000, int seed = 20261026}) {
  final random = _Lcg(seed);
  final out = <CorpusEntry>[];
  for (var n = 0; n < count; n++) {
    final parts = random.nextInt(40) + 1;
    final buffer = StringBuffer();
    for (var p = 0; p < parts; p++) {
      buffer.write(fuzzTokens[random.nextInt(fuzzTokens.length)]);
    }
    out.add((id: 'fuzz:$n', markdown: buffer.toString()));
  }
  return out;
}

/// A tiny self-contained LCG instead of `dart:math`'s `Random`.
///
/// `Random(seed)` would tie every id/input pair in this file to the Dart SDK's
/// generator; a differential baseline is committed to git and must not move
/// when the SDK does. This generator is the same on every platform and is
/// stable forever.
class _Lcg {
  _Lcg(int seed) : _state = seed & 0x7fffffff;
  int _state;
  int nextInt(int max) {
    _state = (_state * 1103515245 + 12345) & 0x7fffffff;
    return _state % max;
  }
}

List<Map<String, dynamic>> _readJsonArray(String path) {
  final decoded = jsonDecode(File(path).readAsStringSync());
  return (decoded as List).cast<Map<String, dynamic>>();
}

/// The adversarial inputs from `crates/markdown/tests/golden_master.rs`.
///
/// Duplicated deliberately: the Rust list is the source of truth, but reading
/// it from Dart would mean parsing Rust source. `golden_master_covers_corpus`
/// pins the KEY SET, so a new case there fails a test here instead of silently
/// dropping out of the differential.
List<(String, String)> _adversarialInputs() => [
      ('adv-empty', ''),
      ('adv-only-blanks', '\n\n\n  \n\t\n'),
      ('adv-lone-hr', '---\n'),
      ('adv-front-matter-unterminated', '---\ntitle: swallowed\n'),
      ('adv-nbsp-indent-list', '- a\n - b\n   - c\n'),
      ('adv-nbsp-paragraph', 'a\n b\n'),
      ('adv-crlf', '# T\r\n\r\npara\r\n\r\n- one\r\n- two\r\n'),
      ('adv-lone-cr', 'a\rb\r'),
      ('adv-tab-columns', '>\tfoo\n-\t\tbar\n\n*\tqux\n'),
      ('adv-unterminated-fence', '```\nno close\n'),
      ('adv-fence-in-list', '- item\n\n  ```\n  code\n\n  still code\n'),
      ('adv-deep-nesting', '- a\n' * 30),
      ('adv-empty-quote-group', '>\n> \n\ntext\n'),
      ('adv-alert-bodyless', '> [!NOTE]\n\nafter\n'),
      ('adv-alert-title-run', '> [!TIP]\n> bold start\n> more\n'),
      ('adv-lazy-continuation', '> quote\nlazy\n\n- item\nlazy too\n'),
      ('adv-loose-tight-mix', '- a\n\n- b\n- c\n\n- d\n'),
      ('adv-marker-change', '- a\n+ b\n* c\n\n1. d\n1) e\n'),
      (
        'adv-indent-code-in-list',
        '- a\n\n    indented? no: list item para\n\n    - nested\n'
      ),
      (
        'adv-top-code-after-list',
        '- a\n\n    code? no: continuation\n\n    ```\n    fence in item\n'
      ),
      ('adv-quote-interrupts', 'para\n> quote\npara2\n'),
      ('adv-quote-def-mixed', '[a]: /x\n\n> [a]: /y\n\n[a]\n'),
      ('adv-setext-multi', 'one\ntwo\n===\n\nthree\n---\n'),
      ('adv-table-no-scan', '| a | b |\nno separator\n'),
      ('adv-ol-start', '5. five\n6. six\n\n1) one\n'),
      ('adv-html-block-7', '<!-- comment\nstill comment\n-->\n\ntext\n'),
      ('adv-footnote-def', 'text[^1]\n\n[^1]: note body\n    continued\n'),
      ('adv-nested-quote-depth', '>>> deep\n>> deeper\n> base\n'),
      (
        'adv-item-children-para',
        '- code follows\n\n  ```\n  x\n  ```\n\n  para after code\n'
      ),
    ];

/// Keys in `golden-master.json`. Used by the coverage test.
Set<String> goldenMasterKeys() =>
    _readJsonObjectKeys('$markdownFixtureRoot/golden-master.json');

/// The corpus ids converted to the naming `golden-master.json` uses. The
/// symmetric difference with [goldenMasterKeys] must be empty.
Set<String> goldenMasterCoveredKeys(List<CorpusEntry> entries) {
  final keys = <String>{};
  for (final entry in entries) {
    final id = entry.id;
    if (id.startsWith('fuzz:')) continue;
    final colon = id.indexOf(':');
    final prefix = id.substring(0, colon);
    final rest = id.substring(colon + 1);
    switch (prefix) {
      case 'spec':
        keys.add('spec-$rest');
      case 'gfm':
        keys.add('gfm-$rest');
      case 'golden':
        keys.add(rest);
      case 'conf':
        keys.add('fixture-${rest.replaceAll(RegExp(r'\.md$'), '')}');
      case 'file':
        keys.add(rest == 'paste.md' ? 'paste' : 'vhost-user');
    }
  }
  return keys;
}

Set<String> _readJsonObjectKeys(String path) {
  final decoded = jsonDecode(File(path).readAsStringSync());
  return (decoded as Map<String, dynamic>).keys.toSet();
}