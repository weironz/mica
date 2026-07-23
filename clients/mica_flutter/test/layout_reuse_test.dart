import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

// Phase 2 whole-layout reuse: an unchanged block skips all re-derivation and is
// just repositioned to its new Y. The controller mutates a node's text/data in
// place on the SAME instance (it reassigns the field), so these tests keep the
// EditorNode instances stable across pumps and mutate one — mirroring real
// edits — which is what actually drives the reuse fast-path. (painter_cache_test
// builds fresh nodes each pump and so only exercises the slow re-shape path.)

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

  double caretX(RenderDocument r, int node, int offset) =>
      r.caretRectFor(DocPosition(node, offset))!.left;
  double caretY(RenderDocument r, int node, int offset) =>
      r.caretRectFor(DocPosition(node, offset))!.top;

  testWidgets('a clean block below a grown edit reuses and repositions down',
      (tester) async {
    final n0 = EditorNode(id: 'a', kind: 'paragraph', text: 'x', data: {});
    final n1 = EditorNode(id: 'b', kind: 'paragraph', text: 'target', data: {});
    final nodes = [n0, n1];
    final r = await pump(tester, nodes);
    final yBefore = caretY(r, 1, 0);
    final xEndBefore = caretX(r, 1, 6); // end of "target"

    // Grow block 0 in place to three lines (same instance, new String — exactly
    // what the controller does). Block 1 is byte-identical → must be reused.
    n0.text = 'line one\nline two\nline three';
    await pump(tester, nodes);

    expect(caretY(r, 1, 0), greaterThan(yBefore + 20),
        reason: 'the survivor shifted down as the block above grew');
    expect(caretX(r, 1, 6), closeTo(xEndBefore, 0.01),
        reason: 'a reused block keeps its exact content geometry');
  });

  testWidgets('a clean block above an edit is left exactly where it was',
      (tester) async {
    final n0 = EditorNode(id: 'a', kind: 'paragraph', text: 'above', data: {});
    final n1 = EditorNode(id: 'b', kind: 'paragraph', text: 'y', data: {});
    final nodes = [n0, n1];
    final r = await pump(tester, nodes);
    final y0 = caretY(r, 0, 0);
    final x0 = caretX(r, 0, 5);

    n1.text = 'grew\nby\ntwo\nlines'; // edit below — must not move block 0
    await pump(tester, nodes);

    expect(caretY(r, 0, 0), closeTo(y0, 0.01));
    expect(caretX(r, 0, 5), closeTo(x0, 0.01));
  });

  testWidgets('an appearance change re-derives even identical text/data',
      (tester) async {
    final n0 = EditorNode(id: 'a', kind: 'paragraph', text: 'hello world', data: {});
    final nodes = [n0];
    final r = await pump(tester, nodes);
    final xEnd = caretX(r, 0, 11);

    // Same instances — only the appearance changes. The per-block identity
    // check can't see this; the whole-layout cache must be globally invalidated.
    await pump(tester, nodes, appearance: const EditorAppearance(fontScale: 1.6));
    expect(caretX(r, 0, 11), greaterThan(xEnd + 5),
        reason: 'larger font must widen the block, not reuse the 1.0 layout');
  });

  testWidgets('renumbering (delete an item above) is not stale-reused',
      (tester) async {
    // Three numbered items; their instances stay stable. Deleting the first
    // renumbers the rest — the ordinal is in the reuse key, so the survivors
    // must re-derive rather than paint a stale number. We can't read the ordinal
    // from outside, but a correct pass must not crash and must reposition item 3
    // up to where item 2 was.
    EditorNode item(String id, String t) =>
        EditorNode(id: id, kind: 'numbered_list', text: t, data: {});
    final a = item('a', 'one');
    final b = item('b', 'two');
    final c = item('c', 'three');
    final nodes = [a, b, c];
    final r = await pump(tester, nodes);
    final ySecond = caretY(r, 1, 0); // where item 'b' sits

    nodes.removeAt(0); // delete item 'a'; b,c stay same instances, renumber
    await pump(tester, nodes);

    // 'b' is now the first item, sitting where 'a' used to be (higher up).
    expect(caretY(r, 0, 0), lessThan(ySecond),
        reason: 'the list closed up after the delete');
    expect(r.caretRectFor(const DocPosition(2, 0)), isNull,
        reason: 'only two items remain');
  });
}
