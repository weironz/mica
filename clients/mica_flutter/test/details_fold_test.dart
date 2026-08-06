// `<details>` renders as a fold instead of lines of HTML source.
//
// The invariant that matters most is the one that ISN'T about looks: the blocks
// are still raw `code_block`s holding their tags, so Markdown round-trip is
// byte-identical. Everything here is rendering; nothing writes to `node.text`.
// The bytes half of that claim is pinned on the authoritative side, in
// `crates/markdown/tests/details_fold.rs` — including that `data.collapsed`
// moves no byte of the exported document.
//
// Two forms, two mechanisms:
//
//   - TIGHT (one block, no blank lines) — the whole element is one raw block,
//     folded by drawing less of it.
//   - BLANK-LINE (what GitHub's docs recommend) — the element is N+2 blocks,
//     folded by ABSORBING the body and the `</details>` into the header's
//     layout as zero-height hidden entries. `_layouts` is indexed by node
//     position everywhere in render.dart, so hiding must not shift indices.
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/details_html.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
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

EditorNode _raw(String id, String text, {bool? collapsed}) => EditorNode(
  id: id,
  kind: 'code_block',
  text: text,
  data: {
    'language': 'html',
    'raw': true,
    if (collapsed != null) 'collapsed': collapsed,
  },
);

/// The block shape `crates/markdown` produces for GitHub's recommended source
/// (pinned there by `the_blank_line_form_is_an_opener_a_body_and_a_bare_closer`).
///
/// Indices: 0 before · 1 opener · 2 body · 3 closer · 4 after.
List<EditorNode> _blankLineForm({bool open = false, bool? collapsed}) => [
  EditorNode(id: 'before', kind: 'paragraph', text: 'before'),
  _raw(
    'open',
    '<details${open ? ' open' : ''}>\n<summary>点开看</summary>',
    collapsed: collapsed,
  ),
  EditorNode(id: 'body', kind: 'paragraph', text: '正文一段'),
  _raw('close', '</details>'),
  EditorNode(id: 'after', kind: 'paragraph', text: 'after'),
];

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
        _details('<details open>\n<summary>点开看</summary>\n正文一段\n</details>'),
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

    test('the blank-line form reads its default off the opening tag too', () {
      // Same trap as the tight form: reading the code-block default (line
      // count) would make the first click write the state it was already in.
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {})
        ..load(_blankLineForm());
      c.toggleCollapsed(1);
      expect(c.nodes[1].data['collapsed'], isFalse);

      final o = EditorController(rootBlockId: 'root', onOps: (_) async {})
        ..load(_blankLineForm(open: true));
      o.toggleCollapsed(1);
      expect(o.nodes[1].data['collapsed'], isTrue);
    });
  });

  group('parseDetailsOpenTag / isDetailsCloseTag read the split form', () {
    test('the shapes the parser actually emits', () {
      expect(parseDetailsOpenTag('<details>')!.summary, isEmpty);
      final withSummary = parseDetailsOpenTag(
        '<details>\n<summary>S</summary>',
      )!;
      expect(withSummary.summary, 'S');
      expect(withSummary.openByDefault, isFalse);
      expect(
        parseDetailsOpenTag(
          '<details open>\n<summary>S</summary>',
        )!.openByDefault,
        isTrue,
      );
      expect(isDetailsCloseTag('</details>'), isTrue);
      expect(isDetailsCloseTag('</DETAILS >\n'), isTrue);
    });

    test('refuses anything it cannot read as exactly the opening tags', () {
      // The tight form is a WHOLE element — parseDetailsBlock's job, not this
      // one's. Overlapping would make two renderers claim one node.
      expect(parseDetailsOpenTag(_tight), isNull);
      // Extra HTML the fold would silently swallow.
      expect(
        parseDetailsOpenTag('<details>\n<summary>S</summary>\n<p>x</p>'),
        isNull,
      );
      // A second line that is not a summary.
      expect(parseDetailsOpenTag('<details>\nplain body'), isNull);
      expect(parseDetailsOpenTag('<div>'), isNull);
      // A closer is not an opener, and vice versa.
      expect(parseDetailsOpenTag('</details>'), isNull);
      expect(isDetailsCloseTag('<details>'), isFalse);
      expect(isDetailsCloseTag('</details>\ntrailing'), isFalse);
    });

    test('detailsOpenByDefault answers for both forms, null for neither', () {
      expect(detailsOpenByDefault(_tight), isFalse);
      expect(detailsOpenByDefault(_open), isTrue);
      expect(detailsOpenByDefault('<details>\n<summary>S</summary>'), isFalse);
      expect(detailsOpenByDefault('<details open>'), isTrue);
      expect(detailsOpenByDefault('print(1)'), isNull);
    });
  });

  // ---------------------------------------------------------------------------
  // The blank-line form: folding a RANGE of blocks.
  // ---------------------------------------------------------------------------
  group('the blank-line form folds a range', () {
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

    testWidgets('collapsed: the body and the closing tag take no space', (
      tester,
    ) async {
      await pump(tester, _blankLineForm());
      final r = renderOf(tester);

      expect(r.debugDetailsHeaderAt(1), isNotNull);
      expect(r.debugTextAt(1), contains('点开看'));
      expect(r.debugTextAt(1), isNot(contains('<details')));
      for (final i in [2, 3]) {
        expect(
          r.debugHiddenAt(i),
          isTrue,
          reason: 'node $i must be folded away',
        );
        expect(r.debugBoxAt(i).$2, 0, reason: 'node $i must take no height');
      }
      // Index alignment is the whole reason hidden layouts exist rather than
      // skipped nodes: `after` must still be _layouts[4].
      expect(r.debugTextAt(4), contains('after'));
      expect(r.debugHiddenAt(4), isFalse);
      // …and the folded body must sit exactly on the header's bottom edge, so
      // there is no gap where the hidden blocks used to be.
      final header = r.debugBoxAt(1);
      expect(r.debugBoxAt(2).$1, header.$1 + header.$2);
    });

    testWidgets('expanded: the body renders as Markdown, the closer does not', (
      tester,
    ) async {
      await pump(tester, _blankLineForm(open: true));
      final r = renderOf(tester);

      expect(r.debugHiddenAt(2), isFalse);
      expect(r.debugTextAt(2), contains('正文一段'));
      expect(
        r.debugHiddenAt(3),
        isTrue,
        reason: 'the header stands for `</details>`; showing it too is noise',
      );
      expect(r.debugTextAt(3), isEmpty);
    });

    /// The whole point, and the round-trip red line on the client side: a click
    /// writes `data.collapsed` on the OPENING block and touches nothing else.
    testWidgets('clicking the header folds the range, touching only data', (
      tester,
    ) async {
      final ops = <Map<String, dynamic>>[];
      final nodes = _blankLineForm();
      final texts = nodes.map((n) => n.text).toList();
      await pump(tester, nodes, ops: ops);
      final origin = tester.getTopLeft(find.byType(MicaEditor));

      await tester.tapAt(
        origin + renderOf(tester).debugDetailsHeaderAt(1)!.center,
      );
      await tester.pump();

      final r = renderOf(tester);
      expect(r.debugHiddenAt(2), isFalse, reason: 'the first click expands');
      expect(r.debugTextAt(2), contains('正文一段'));
      expect(
        nodes.map((n) => n.text).toList(),
        texts,
        reason: 'round-trip: not one tag was rewritten',
      );
      expect(ops.single['type'], 'update_block');
      expect(ops.single['block_id'], 'open');
      expect(ops.single['data']['collapsed'], isFalse);
      expect(ops.single.containsKey('text'), isFalse);
    });

    testWidgets('typing in an expanded body does NOT bring the tags back', (
      tester,
    ) async {
      // The tight form declines whenever the selection is on ITS block, which is
      // right there. Reusing that rule for the range would make the tags flash
      // back the moment you put the caret in the body — the normal place to
      // type. Only the tags and what the fold HIDES trigger the escape hatch.
      await pump(tester, _blankLineForm(open: true));
      final origin = tester.getTopLeft(find.byType(MicaEditor));
      final body = renderOf(tester).debugBoxAt(2);
      final target = Offset(60, body.$1 + body.$2 / 2);
      expect(
        renderOf(tester).positionAt(target).node,
        2,
        reason:
            'precondition: the tap has to land IN the body, or this test '
            'proves nothing',
      );

      await tester.tapAt(origin + target);
      await tester.pump();

      final r = renderOf(tester);
      expect(r.debugTextAt(1), isNot(contains('<details')));
      expect(r.debugDetailsHeaderAt(1), isNotNull);
      expect(r.debugHiddenAt(3), isTrue);
    });

    testWidgets('the caret reaching the closing tag reveals the whole element', (
      tester,
    ) async {
      // The escape hatch: with the closer hidden there is otherwise NO way to
      // edit it. Arrow-left off the start of the block after the fold walks into
      // it, and the renderer then declines for the whole element.
      await pump(tester, _blankLineForm(open: true));
      final origin = tester.getTopLeft(find.byType(MicaEditor));
      final after = renderOf(tester).debugBoxAt(4);

      await tester.tapAt(origin + Offset(60, after.$1 + after.$2 / 2));
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.home);
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowLeft);
      await tester.pump();

      final r = renderOf(tester);
      expect(r.debugHiddenAt(3), isFalse);
      expect(r.debugTextAt(3), contains('</details>'));
      expect(
        r.debugDetailsHeaderAt(1),
        isNull,
        reason: 'the escape hatch shows the element whole, tags included',
      );
      expect(r.debugTextAt(1), contains('<details'));
    });

    testWidgets('an opener with no closer keeps rendering as source', (
      tester,
    ) async {
      await pump(tester, [
        _raw('open', '<details>\n<summary>点开看</summary>'),
        EditorNode(id: 'body', kind: 'paragraph', text: '正文一段'),
      ]);
      final r = renderOf(tester);
      expect(r.debugDetailsHeaderAt(0), isNull);
      expect(r.debugTextAt(0), contains('<details>'));
      expect(r.debugHiddenAt(1), isFalse);
    });

    testWidgets('a nested <details> is not paired by guesswork', (
      tester,
    ) async {
      await pump(tester, [
        _raw('o1', '<details>\n<summary>outer</summary>'),
        _raw('o2', '<details>\n<summary>inner</summary>'),
        EditorNode(id: 'body', kind: 'paragraph', text: 'x'),
        _raw('c1', '</details>'),
        _raw('c2', '</details>'),
      ]);
      final r = renderOf(tester);
      expect(
        r.debugDetailsHeaderAt(0),
        isNull,
        reason: 'the outer one cannot know which closer is its own',
      );
      // The inner one is unambiguous and still folds.
      expect(r.debugDetailsHeaderAt(1), isNotNull);
      for (var i = 0; i < 5; i++) {
        if (i == 2 || i == 3) continue;
        expect(r.debugHiddenAt(i), isFalse, reason: 'node $i');
      }
    });
  });

  // ---------------------------------------------------------------------------
  // A hidden node must be invisible to every index-keyed path, not just paint.
  // ---------------------------------------------------------------------------
  group('caret, hit testing and drag handles skip a folded range', () {
    Future<RenderDocument> pump(
      WidgetTester tester,
      List<EditorNode> nodes,
    ) async {
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
              onApplyOperations: (_) async {},
            ),
          ),
        ),
      );
      await tester.pump();
      return tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
    }

    testWidgets('no click anywhere lands the caret on a folded node', (
      tester,
    ) async {
      final r = await pump(tester, _blankLineForm());
      for (var y = 0.0; y < r.size.height; y += 2) {
        final p = r.positionAt(Offset(60, y));
        expect(
          r.debugHiddenAt(p.node),
          isFalse,
          reason: 'click at y=$y resolved to folded node ${p.node}',
        );
      }
    });

    testWidgets('clicking below a document that ENDS in a fold', (
      tester,
    ) async {
      // positionAt's "past the last block" fallback used to be the last layout
      // outright. When the document ends inside a collapsed fold that is the
      // hidden `</details>`, so clicking the empty space under the page parked
      // the caret in a block nobody can see.
      final r = await pump(tester, [
        EditorNode(id: 'before', kind: 'paragraph', text: 'before'),
        _raw('open', '<details>\n<summary>点开看</summary>'),
        EditorNode(id: 'body', kind: 'paragraph', text: '正文一段'),
        _raw('close', '</details>'),
      ]);
      expect(r.debugHiddenAt(3), isTrue, reason: 'precondition: fold is last');
      final p = r.positionAt(Offset(60, r.size.height - 1));
      expect(r.debugHiddenAt(p.node), isFalse, reason: 'landed on ${p.node}');
    });

    testWidgets('no hover anywhere reports a folded block', (tester) async {
      final r = await pump(tester, _blankLineForm());
      for (var y = 0.0; y < r.size.height; y += 2) {
        final b = r.blockAt(Offset(60, y));
        if (b == null) continue;
        expect(
          r.debugHiddenAt(b),
          isFalse,
          reason: 'hover at y=$y hit node $b',
        );
      }
    });

    testWidgets('no drop target is inside the folded range', (tester) async {
      // A zero-height node's midpoint is the fold's seam, so without a guard it
      // swallows every drop aimed at the block below — dropping a block INTO a
      // collapsed body, invisibly.
      final r = await pump(tester, _blankLineForm());
      for (var y = 0.0; y < r.size.height; y += 2) {
        final i = r.dropIndexAt(y);
        if (i >= 5) continue;
        expect(r.debugHiddenAt(i), isFalse, reason: 'drop at y=$y → node $i');
      }
    });

    testWidgets('Down off a fold header steps over a folded ATOMIC block', (
      tester,
    ) async {
      // The sharp edge in _stepToNode: it answers `DocPosition(i, 0)` outright
      // for an atomic node, no geometry consulted. A divider (or image) inside a
      // collapsed body is atomic, so Down off the header parked the caret ON it
      // — invisible, and one Backspace away from deleting a block the user
      // cannot see.
      final r = await pump(tester, [
        EditorNode(id: 'before', kind: 'paragraph', text: 'before'),
        _raw('open', '<details>\n<summary>点开看</summary>'),
        EditorNode(id: 'rule', kind: 'divider', text: '', data: {}),
        _raw('close', '</details>'),
        EditorNode(id: 'after', kind: 'paragraph', text: 'after'),
      ]);
      expect(
        r.debugHiddenAt(2),
        isTrue,
        reason: 'precondition: divider folded',
      );
      expect(r.positionBelow(const DocPosition(1, 0), null)!.node, 4);
    });

    testWidgets('Up off the block below a fold lands on the header', (
      tester,
    ) async {
      final r = await pump(tester, _blankLineForm());
      expect(
        r.positionAbove(const DocPosition(4, 0), null),
        const DocPosition(1, 0),
      );
      expect(r.positionBelow(const DocPosition(1, 0), null)!.node, 4);
    });

    testWidgets('a folded node has no caret rectangle to draw', (tester) async {
      // Remote cursors reach any node; drawing one on the seam would claim a
      // collaborator is editing a line nobody can see.
      final r = await pump(tester, _blankLineForm());
      expect(r.caretRectFor(const DocPosition(2, 0)), isNull);
      expect(r.caretRectFor(const DocPosition(3, 0)), isNull);
      expect(r.caretRectFor(const DocPosition(4, 0)), isNotNull);
    });
  });
}
