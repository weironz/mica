import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/model.dart';

/// The EXPORT direction, pinned across the two engines.
///
/// `markdown_conformance_test.dart` next door covers the import direction:
/// Dart's `markdownToBlocks` against Rust's `.blocks.json` gold. Nothing
/// covered the way back. Rust's own `fixtures_round_trip` exercises only
/// Rust's serializer, and the Dart side never called its serializer from a
/// conformance test at all — so `inlineToMarkdown`, `escapeBlockLeader` and
/// `escapeInline` had ZERO cross-engine coverage.
///
/// Both drifts that reached users came through that hole:
///
///   * `escapeBlockLeader` — a body line `===` exported unescaped, and on
///     re-import became a setext heading that swallowed the paragraph above it.
///   * `render_span` dropping a link whose range exactly coincided with a code
///     mark: `` [`useState`](url) `` exported as `` `useState` ``, URL gone.
///     Caught by accident, by Rust's round-trip assertion, not by any check
///     aimed at the Dart mirror.
///
/// Note what would NOT have helped: adding more `.md` fixtures. The corpus was
/// never the gap — the harness had no export edge. That is the lesson recorded
/// in `docs/lessons.md` §3.
///
/// Gold is `export_markdown(import_markdown(md))` from the Rust engine,
/// regenerated with `GEN_GOLD=1 cargo test -p mica-markdown --test conformance`.
/// Fixtures whose export output differs TODAY, listed so the other 17 can
/// start guarding immediately instead of waiting for a full triage.
///
/// This is a regression FLOOR in the style of `commonmark_scoreboard.rs`: the
/// set may only shrink. Removing a name and watching the test pass is how a
/// fix gets proven; adding one requires justifying why a new divergence is
/// acceptable, which it almost never is.
///
/// Triaged 2026-07-21: all 15 were REAL — the "maybe it is a legitimate
/// product difference between `export_markdown` and `selectionText`" theory
/// was wrong. Every one of them changes meaning when the copy is pasted back.
/// Five were fixed on the spot (11, 13, 16, 19, 34) by three lines in
/// `controller.dart`; the ten below are structural and still open:
///
/// Fixtures whose copy output differs from Rust's byte-for-byte but re-imports
/// to EXACTLY the same blocks — measured, not assumed.
///
/// These are not bugs, and byte-parity was the wrong bar for them.
/// `export_markdown` (whole-document export) and `selectionText`
/// (copy-to-clipboard) are different products; Rust ends every quote group
/// with a decorative bare `>` that is not in the source and not in the block
/// model, and Dart's output round-trips correctly without it. Adding it was
/// tried and reverted — it broke two long-standing tests that deliberately
/// assert the opposite ("a blank line would SPLIT the blockquote on re-parse
/// and shatter the quote bar").
///
/// So these still run: they just assert the invariant that actually matters
/// (copy → paste preserves the content) instead of an exact byte string.
const _cosmeticDivergence = {
  '05-quote.md', '10-mixed.md', '15-list-items.md',
  '17-quotes.md', '20-item-containers.md',
};

/// Fixtures whose copy output does NOT re-import to the same blocks — genuine,
/// still-open bugs.
///
/// EMPTY, and it should stay that way: every fixture's copy output now
/// re-imports to exactly the blocks it came from.
///
/// It held 15 entries when this harness was written. Do not add one back to
/// make a failure go away — a name here means the editor silently changes a
/// user's content on copy-paste, which is what all fifteen turned out to be.
///
/// Note for whoever hits a failure: the weaker check that compares only
/// kind + text once cleared three of these. Comparing `data` too is what
/// exposed them. Do not weaken it back.
const _knownBroken = <String>{};

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
    test('export conformance: $name', () {
      if (_knownBroken.contains(name)) {
        markTestSkipped('$name: known-broken export, see _knownBroken');
        return;
      }
      final goldFile = File(md.path.replaceAll(RegExp(r'\.md$'), '.md.gold'));
      expect(goldFile.existsSync(), isTrue,
          reason: 'missing export gold for $name — GEN_GOLD=1 on the Rust side');

      // Import with the Dart mirror, then serialize back with the Dart
      // serializer. Import parity is already asserted next door, so a failure
      // here isolates to the export half.
      final specs = markdownToBlocks(md.readAsStringSync());
      final got = _exportViaController(specs);
      if (got == null) {
        markTestSkipped('single empty block — not expressible as a selection');
        return;
      }

      // THE invariant, asserted for every fixture: what you copy is what you
      // get back. Byte-parity with Rust is a proxy for this; this is the thing
      // itself, and it is what a user loses when it breaks.
      expect(_canon(_shape(markdownToBlocks(got))), _canon(_shape(specs)),
          reason: 'copy → paste CHANGED the content in $name');

      if (_cosmeticDivergence.contains(name)) return;
      expect(got.trimRight(), goldFile.readAsStringSync().trimRight(),
          reason: 'EXPORT drift between the Dart mirror and the Rust engine in '
              '$name — the serializers disagree');
    });
  }
}

List<Map<String, dynamic>> _shape(List<BlockSpec> specs) => [
      for (final s in specs) {'kind': s.kind, 'text': s.text, 'data': s.data},
    ];

/// Canonical JSON so map ordering and int/double form cannot cause a false diff.
dynamic _canon(dynamic v) {
  if (v is Map) {
    final keys = v.keys.cast<String>().toList()..sort();
    return {for (final k in keys) k: _canon(v[k])};
  }
  if (v is List) return [for (final e in v) _canon(e)];
  if (v is num) return v.toDouble() == v.toInt() ? v.toInt() : v;
  return v;
}

/// Run the editor's real copy-as-Markdown path: load the blocks, select all,
/// serialize. This is deliberately the production path (`selectionText`) rather
/// than a test-only helper — the drifts live in the code users actually hit.
String? _exportViaController(List<BlockSpec> specs) {
  if (specs.isEmpty) return '';
  // A document that is one empty-text block cannot be expressed as a ranged
  // selection — anchor and focus land on the same offset and `selectionText`
  // returns '' by contract. Not comparable through the copy path; null asks
  // the caller to skip rather than report a phantom drift.
  if (specs.length == 1 && specs.first.text.isEmpty) return null;
  final nodes = [
    for (var i = 0; i < specs.length; i++)
      EditorNode(
        id: 'b$i',
        kind: specs[i].kind,
        text: specs[i].text,
        data: Map<String, dynamic>.of(specs[i].data),
      ),
  ];
  final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
  c.load(nodes);
  c.selection = DocSelection(
    anchor: const DocPosition(0, 0),
    focus: DocPosition(nodes.length - 1, nodes.last.text.length),
  );
  return c.selectionText();
}
