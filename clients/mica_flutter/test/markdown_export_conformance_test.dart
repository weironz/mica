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
/// Two distinct causes are mixed in here and have NOT been separated yet — do
/// not assume every entry is a bug:
///
///  * The list/quote cluster (03, 04, 05, 15, 17, 20) differs in blank-line
///    and loose-list placement. Plausibly legitimate: Rust's gold is
///    `export_markdown` (whole-document export) while Dart's is
///    `selectionText` (copy-to-clipboard). Those are different products and
///    may be allowed to disagree on spacing.
///  * The inline cluster (11, 13, 16, 19, 21, 22, 23, 34) is more suspicious.
///    `34-link-title-escape` in particular loses a link title outright
///    (`[a](/u "C:\name")` → `[a](/u)`), which is the same data-loss shape as
///    the `render_span` link-dropping bug fixed on 2026-07-21.
///
/// Triage is tracked in `docs/rust-migration-assessment-2026-07-21.md`.
const _knownDivergent = {
  '03-lists.md', '04-todo.md', '05-quote.md', '10-mixed.md',
  '11-escapes-autolinks.md', '13-links.md', '15-list-items.md',
  '16-images.md', '17-quotes.md', '19-entities-defs.md',
  '20-item-containers.md', '21-html.md', '22-gfm.md', '23-math.md',
  '34-link-title-escape.md',
};

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
      if (_knownDivergent.contains(name)) {
        markTestSkipped('$name: known export divergence, pending triage');
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

      expect(got.trimRight(), goldFile.readAsStringSync().trimRight(),
          reason: 'EXPORT drift between the Dart mirror and the Rust engine in '
              '$name — the serializers disagree');
    });
  }
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
