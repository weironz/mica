import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

// Comment highlights are PAINT-ONLY, and that is the load-bearing property: a
// comment lives outside the document (Postgres, not the CRDT), so it must not be
// able to move a single glyph — otherwise commenting would change the text's
// layout and, through it, what round-trips. These pin that, plus the stale-range
// safety (the doc can change between fetching threads and drawing them).

void main() {
  Future<RenderDocument> pump(WidgetTester tester) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: DocumentSurface(
            nodes: [
              EditorNode(id: 'a', kind: 'paragraph', text: 'hello world'),
              EditorNode(id: 'b', kind: 'paragraph', text: 'second'),
            ],
            selection: null,
            showCaret: false,
            caretBlink: ValueNotifier(false),
            appearance: const EditorAppearance(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
  }

  testWidgets('setting highlights repaints but NEVER relayouts', (tester) async {
    final render = await pump(tester);
    expect(render.debugNeedsPaint, isFalse, reason: 'settled before we start');

    render.commentHighlights = [
      (blockId: 'a', startOffset: 0, endOffset: 5, active: false),
    ];

    expect(render.debugNeedsPaint, isTrue);
    expect(
      render.debugNeedsLayout,
      isFalse,
      reason: 'a comment must not be able to move a glyph',
    );
    await tester.pump();
  });

  testWidgets('a highlight does not shift caret geometry', (tester) async {
    final render = await pump(tester);
    final before = render.caretRectFor(const DocPosition(0, 3));
    expect(before, isNotNull);

    render.commentHighlights = [
      (blockId: 'a', startOffset: 0, endOffset: 11, active: true),
    ];
    await tester.pump();

    expect(render.caretRectFor(const DocPosition(0, 3)), before);
  });

  testWidgets('an identical list is a no-op (no needless repaint)',
      (tester) async {
    final render = await pump(tester);
    render.commentHighlights = [
      (blockId: 'a', startOffset: 0, endOffset: 5, active: false),
    ];
    await tester.pump();
    expect(render.debugNeedsPaint, isFalse);

    // Same values, new list instance → listEquals short-circuits.
    render.commentHighlights = [
      (blockId: 'a', startOffset: 0, endOffset: 5, active: false),
    ];
    expect(render.debugNeedsPaint, isFalse);
  });

  testWidgets('stale, out-of-range and unknown ranges paint without throwing',
      (tester) async {
    // The document can change between fetching threads and painting them, so a
    // range may no longer fit. It must degrade, not throw (a paint exception
    // would take the whole editor down).
    final render = await pump(tester);
    render.commentHighlights = [
      // Past the end of 'hello world' (11).
      (blockId: 'a', startOffset: 5, endOffset: 999, active: false),
      // Entirely past the end.
      (blockId: 'a', startOffset: 900, endOffset: 999, active: true),
      // Inverted.
      (blockId: 'b', startOffset: 5, endOffset: 1, active: false),
      // Zero-length.
      (blockId: 'b', startOffset: 2, endOffset: 2, active: false),
      // Negative (never sent, but must not crash).
      (blockId: 'a', startOffset: -3, endOffset: 4, active: false),
      // A block that is not in this document at all.
      (blockId: 'does-not-exist', startOffset: 0, endOffset: 3, active: false),
    ];
    await tester.pump();

    // Still alive and still measuring the same geometry.
    expect(render.caretRectFor(const DocPosition(0, 3)), isNotNull);
    expect(tester.takeException(), isNull);
  });

  testWidgets('clearing the highlights repaints back to plain text',
      (tester) async {
    final render = await pump(tester);
    render.commentHighlights = [
      (blockId: 'a', startOffset: 0, endOffset: 5, active: false),
    ];
    await tester.pump();

    render.commentHighlights = const [];
    expect(render.debugNeedsPaint, isTrue);
    expect(render.debugNeedsLayout, isFalse);
    await tester.pump();
    expect(tester.takeException(), isNull);
  });
}
