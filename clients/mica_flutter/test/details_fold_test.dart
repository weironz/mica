// `<details>` renders as a fold instead of four lines of HTML source.
//
// The invariant that matters most is the one that ISN'T about looks: the block
// is still a raw `code_block` holding its tags, so Markdown round-trip is
// byte-identical. Everything here is rendering; nothing writes to `node.text`.
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/details_html.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

const _tight = '<details>\n<summary>点开看</summary>\n正文一段\n</details>';
const _open = '<details open>\n<summary>点开看</summary>\n正文一段\n</details>';

EditorNode _details(String text) => EditorNode(
  id: 'd',
  kind: 'code_block',
  text: text,
  data: {'language': 'html', 'raw': true},
);

void main() {
  group('parseDetailsBlock reads the whole element or nothing', () {
    test('the tight form, summary and body split out', () {
      final shape = parseDetailsBlock(_tight)!;
      expect(shape.summary, '点开看');
      expect(shape.body, '正文一段');
      expect(shape.openByDefault, isFalse);
    });

    test('`open` is the default expanded state', () {
      expect(
        parseDetailsBlock('<details open>\nx\n</details>')!.openByDefault,
        isTrue,
      );
      expect(
        parseDetailsBlock('<DETAILS OPEN>\nx\n</DETAILS>')!.openByDefault,
        isTrue,
      );
    });

    test('no summary is allowed — the renderer supplies a label', () {
      final shape = parseDetailsBlock('<details>\njust body\n</details>')!;
      expect(shape.summary, isEmpty);
      expect(shape.body, 'just body');
    });

    // Everything it does not recognize keeps rendering as source, which is
    // never wrong — only plain. These are the refusals worth pinning, because
    // a loose parser here would fold something it cannot fold back.
    test('refuses what it cannot read whole', () {
      // Nested: the fold would not know where the inner element ends.
      expect(
        parseDetailsBlock('<details>\n<details>\na\n</details>\n</details>'),
        isNull,
      );
      // Unterminated — the blank-line form's OPENING block looks exactly like
      // this, and folding it would hide nothing while claiming to fold.
      expect(parseDetailsBlock('<details>\n<summary>x</summary>'), isNull);
      // Not a details element at all.
      expect(parseDetailsBlock('<div>\nx\n</div>'), isNull);
      // Trailing junk after the close tag.
      expect(
        parseDetailsBlock('<details>\nx\n</details>\n<p>after</p>'),
        isNull,
      );
    });
  });

  group('the fold in the editor', () {
    Future<void> pump(
      WidgetTester tester,
      List<EditorNode> nodes, {
      List<Map<String, dynamic>>? ops,
    }) async {
      await tester.pumpWidget(
        MaterialApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          locale: const Locale('zh'),
          home: Scaffold(
            body: MicaEditor(
              rootBlockId: 'root',
              nodes: nodes,
              version: 0,
              canEdit: true,
              onApplyOperations: (batch) async => ops?.addAll(batch),
            ),
          ),
        ),
      );
      await tester.pump();
    }

    RenderDocument renderOf(WidgetTester tester) =>
        tester.renderObject<RenderDocument>(find.byType(DocumentSurface));

    testWidgets('a plain <details> draws folded: summary yes, body no', (
      tester,
    ) async {
      await pump(tester, [_details(_tight)]);
      final r = renderOf(tester);

      // Claimed by the fold renderer, not the text pipeline — the source form
      // would show `<details>` as the first line.
      expect(r.debugTextAt(0), isNot(contains('<details>')));
      expect(r.debugTextAt(0), contains('点开看'));
      expect(r.debugTextAt(0), isNot(contains('正文一段')));
      expect(r.debugDetailsHeaderAt(0), isNotNull);
    });

    testWidgets('<details open> draws expanded', (tester) async {
      await pump(tester, [
        _details(
          '<details open>\n<summary>点开看</summary>\n正文一段\n</details>',
        ),
      ]);
      expect(renderOf(tester).debugTextAt(0), contains('正文一段'));
    });

    /// The whole point. A click on the header flips the fold and writes only
    /// `data.collapsed` — `node.text` keeps every tag, so the Markdown that
    /// comes back out is the Markdown that went in.
    testWidgets('clicking the header folds, and touches only data.collapsed', (
      tester,
    ) async {
      final ops = <Map<String, dynamic>>[];
      final node = _details(_tight);
      await pump(tester, [node], ops: ops);
      final r = renderOf(tester);
      final origin = tester.getTopLeft(find.byType(MicaEditor));

      await tester.tapAt(origin + r.debugDetailsHeaderAt(0)!.center);
      await tester.pump();

      expect(
        renderOf(tester).debugTextAt(0),
        contains('正文一段'),
        reason: 'the first click must expand a block that started folded',
      );
      expect(node.text, _tight, reason: 'round-trip: the tags are untouched');
      expect(ops.single['type'], 'update_block');
      expect(ops.single['data']['collapsed'], isFalse);
      expect(ops.single.containsKey('text'), isFalse);
    });

    /// The escape hatch: there is no other way to edit the tags once the fold
    /// hides them.
    testWidgets('the caret in the block brings the raw source back', (
      tester,
    ) async {
      await pump(tester, [
        EditorNode(id: 'p', kind: 'paragraph', text: 'above'),
        // Expanded, so there IS a body row to click; a folded block is header
        // and nothing else.
        _details(_open),
      ]);
      final r = renderOf(tester);
      final origin = tester.getTopLeft(find.byType(MicaEditor));
      // Click the fold's BODY area (below the header) — that reaches the text
      // pipeline and parks the caret in the block.
      final header = r.debugDetailsHeaderAt(1)!;
      await tester.tapAt(origin + Offset(header.center.dx, header.bottom + 2));
      await tester.pump();

      expect(renderOf(tester).debugTextAt(1), contains('<summary>'));
    });

    /// Mermaid also claims `code_block`. Before 2026-08-03 the registry was a
    /// kind→renderer Map, so registering a second renderer for a kind silently
    /// replaced the first — this is the test that would have caught it.
    testWidgets('registering two renderers for code_block keeps both', (
      tester,
    ) async {
      await pump(tester, [
        _details(_tight),
        EditorNode(
          id: 'c',
          kind: 'code_block',
          text: 'print(1)',
          data: {'language': 'python'},
        ),
      ]);
      final r = renderOf(tester);
      // The fold claimed its node…
      expect(r.debugDetailsHeaderAt(0), isNotNull);
      // …and an ordinary code block still went through the text pipeline.
      expect(r.debugDetailsHeaderAt(1), isNull);
      expect(r.debugTextAt(1), contains('print(1)'));
    });
  });

  group('toggleCollapsed knows the details default', () {
    /// Reading the code-block default (line count) here would make the first
    /// click write `collapsed: true` on a block that was already folded — a
    /// click that visibly does nothing.
    test('the first click on a folded <details> expands it', () {
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {})
        ..load([_details(_tight)]);
      c.toggleCollapsed(0);
      expect(c.nodes.single.data['collapsed'], isFalse);
      expect(c.nodes.single.text, _tight);
    });

    test('the first click on <details open> folds it', () {
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {})
        ..load([_details(_open)]);
      c.toggleCollapsed(0);
      expect(c.nodes.single.data['collapsed'], isTrue);
    });
  });
}
