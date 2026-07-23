import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

// RenderDocument keeps each block's shaped TextPainter across layout passes and
// reuses it when the block's rendered span + width are unchanged (the layout-
// side virtualization from docs/editor-virtualization-plan.md, Phase 1). The
// cost of getting reuse wrong is silent geometry corruption — a stale painter
// answering caret/hit-test queries for text it no longer holds. These pin the
// two ways that could happen: an edited block must re-shape, and an appearance
// change must re-shape, even though the block id (the cache key) never changes.

EditorNode _p(String id, String text) =>
    EditorNode(id: id, kind: 'paragraph', text: text, data: const {});

void main() {
  Future<RenderDocument> pump(
    WidgetTester tester,
    List<EditorNode> nodes, {
    EditorAppearance appearance = const EditorAppearance(),
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: DocumentSurface(
            nodes: nodes,
            selection: null,
            showCaret: false,
            caretBlink: ValueNotifier(false),
            appearance: appearance,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
  }

  double caretX(RenderDocument r, int node, int offset) {
    final rect = r.caretRectFor(DocPosition(node, offset));
    expect(rect, isNotNull, reason: 'text position must have a caret rect');
    return rect!.left;
  }

  testWidgets('an edited block re-shapes — its cached painter is not reused',
      (tester) async {
    final r = await pump(tester, [_p('a', 'hi'), _p('b', 'xy')]);
    final shortEnd = caretX(r, 0, 2); // caret after "hi"
    final bBefore = caretX(r, 1, 2); // caret in the untouched block

    // Same block id 'a', longer text: the cache key is unchanged, so a naive
    // id-only cache would answer offset 13 against the stale "hi" painter.
    await pump(tester, [_p('a', 'hi there vast'), _p('b', 'xy')]);
    final longEnd = caretX(r, 0, 13); // caret after the new, longer text

    expect(longEnd, greaterThan(shortEnd + 20),
        reason: 'the block must have re-shaped to the longer text');
    // The untouched block stayed one line above it — its geometry is stable.
    expect(caretX(r, 1, 2), closeTo(bBefore, 0.01));
  });

  testWidgets('changing appearance re-shapes — font scale is in the fingerprint',
      (tester) async {
    final r = await pump(tester, [_p('a', 'hello world')]);
    final atOne = caretX(r, 0, 11);

    await pump(tester, [_p('a', 'hello world')],
        appearance: const EditorAppearance(fontScale: 1.6));
    final atBig = caretX(r, 0, 11);

    expect(atBig, greaterThan(atOne + 5),
        reason: 'larger font must widen the block, not reuse the 1.0 painter');
  });

  testWidgets('surviving many edit cycles never uses a disposed painter',
      (tester) async {
    // Exercises store / reuse / replace / prune repeatedly. Any use-after-
    // dispose or double-free throws and fails the test; the final geometry
    // also has to be correct (proves the last shaping actually happened).
    final r = await pump(tester, [_p('a', 'seed'), _p('b', 'two')]);
    for (var i = 0; i < 12; i++) {
      await pump(tester, [
        _p('a', 'edit number $i here'),
        _p('b', 'two'),
      ]);
    }
    // Block 'b' was byte-identical every pass (pure reuse); still correct.
    expect(r.caretRectFor(const DocPosition(1, 3)), isNotNull);
    final wide = caretX(r, 0, 'edit number 11 here'.length);
    final narrow = caretX(r, 0, 4);
    expect(wide, greaterThan(narrow));
  });

  testWidgets('deleting a block prunes it without disturbing the survivor',
      (tester) async {
    final r = await pump(tester, [_p('a', 'alpha'), _p('b', 'beta')]);
    final aX = caretX(r, 0, 5);

    // 'b' is gone; 'a' remains and must still answer correctly (its cached
    // painter survives the prune of 'b').
    await pump(tester, [_p('a', 'alpha')]);
    expect(caretX(r, 0, 5), closeTo(aX, 0.01));
    // The old index 1 no longer exists.
    expect(r.caretRectFor(const DocPosition(1, 0)), isNull);
  });
}
