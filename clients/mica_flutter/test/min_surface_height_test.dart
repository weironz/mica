import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

// The canvas claims EditorTheme.minSurfaceHeight whether or not the text needs
// it, so a blank page still offers a big click-to-write area. That floor is
// padding BELOW the text, which means anything the PAGE stacks under the canvas
// is pushed down by all of it: a three-line page with one backlink drew its
// text, ~420px of nothing, and then the backlinks panel stranded halfway down
// the window ("悬在页面中央"). The floor is a knob on the appearance so the page
// can drop it when it has something to put there.

void main() {
  Future<RenderDocument> pump(
    WidgetTester tester,
    List<EditorNode> nodes, {
    EditorAppearance appearance = const EditorAppearance(),
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            child: DocumentSurface(
              nodes: nodes,
              selection: null,
              showCaret: false,
              caretBlink: ValueNotifier(false),
              appearance: appearance,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
  }

  List<EditorNode> shortPage() => [
    EditorNode(id: 'a', kind: 'paragraph', text: 'one'),
    EditorNode(id: 'b', kind: 'paragraph', text: 'two'),
  ];

  testWidgets('a short page still reserves the click-to-write floor', (
    tester,
  ) async {
    final r = await pump(tester, shortPage());
    expect(r.size.height, EditorTheme.minSurfaceHeight);
  });

  testWidgets('dropping the floor lets a short page hug its content', (
    tester,
  ) async {
    final r = await pump(
      tester,
      shortPage(),
      appearance: const EditorAppearance(minSurfaceHeight: 0),
    );
    // Content + the bottom pad that keeps a click-to-write strip below the
    // text — and nowhere near the 420 that stranded the panel.
    expect(r.size.height, lessThan(EditorTheme.minSurfaceHeight));
    expect(r.size.height, greaterThan(EditorTheme.bottomPad));
  });

  testWidgets('a long page is unaffected by the floor either way', (
    tester,
  ) async {
    final long = [
      for (var i = 0; i < 40; i++)
        EditorNode(id: 'p$i', kind: 'paragraph', text: 'line $i'),
    ];
    final withFloor = (await pump(tester, long)).size.height;
    final without = (await pump(
      tester,
      long,
      appearance: const EditorAppearance(minSurfaceHeight: 0),
    )).size.height;
    expect(withFloor, greaterThan(EditorTheme.minSurfaceHeight));
    expect(without, withFloor);
  });

  testWidgets('the trailing pad is trimmed for a panel below', (tester) async {
    // Peers that stack a link panel under the body keep 36-48px between the
    // last line and the panel (Obsidian 48 / Logseq 44 / Roam 36). Ours was
    // 96 (this pad) + 28 (the panel's own margin) with no rule in between.
    //
    // Both heights are read straight out of pump(): the two pumps reuse one
    // render object, so holding on to the first RenderDocument would just hand
    // back the second measurement.
    final withPad = (await pump(
      tester,
      shortPage(),
      appearance: const EditorAppearance(minSurfaceHeight: 0),
    )).size.height;
    final trimmed = (await pump(
      tester,
      shortPage(),
      appearance: const EditorAppearance(minSurfaceHeight: 0, bottomPad: 24),
    )).size.height;

    expect(withPad - trimmed, EditorTheme.bottomPad - 24);
    // What the panel actually sees: the canvas's leftover pad below the last
    // line, plus the 16px the panel puts above its own rule.
    final content = withPad - EditorTheme.bottomPad;
    expect(trimmed - content + 16, inInclusiveRange(36, 48));
  });

  test('the floor takes part in appearance equality', () {
    // The render object skips relayout when the appearance compares equal, so a
    // floor left out of == would change nothing until something else did.
    expect(
      const EditorAppearance(),
      isNot(const EditorAppearance(minSurfaceHeight: 0)),
    );
    expect(
      const EditorAppearance(),
      isNot(const EditorAppearance(bottomPad: 24)),
    );
  });
}
