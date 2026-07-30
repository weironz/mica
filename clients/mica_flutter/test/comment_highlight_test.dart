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
  /// [highlights] go through the WIDGET, not the render object's setter.
  ///
  /// That matters: `DocumentSurface.updateRenderObject` re-applies its own
  /// `commentHighlights` on every rebuild, so anything assigned imperatively is
  /// wiped by the next `pump()`. The tests below assert what was PAINTED, which
  /// requires a pump — so they have to arrive the way production sends them.
  Future<RenderDocument> pump(
    WidgetTester tester, {
    List<CommentHighlight> highlights = const [],
  }) async {
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
            commentHighlights: highlights,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
  }

  testWidgets('setting highlights repaints but NEVER relayouts', (
    tester,
  ) async {
    final render = await pump(tester);
    expect(render.debugNeedsPaint, isFalse, reason: 'settled before we start');

    render.commentHighlights = [
      (
        startBlock: 'a',
        endBlock: 'a',
        startOffset: 0,
        endOffset: 5,
        active: false,
      ),
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
      (
        startBlock: 'a',
        endBlock: 'a',
        startOffset: 0,
        endOffset: 11,
        active: true,
      ),
    ];
    await tester.pump();

    expect(render.caretRectFor(const DocPosition(0, 3)), before);
  });

  testWidgets('an identical list is a no-op (no needless repaint)', (
    tester,
  ) async {
    final render = await pump(tester);
    render.commentHighlights = [
      (
        startBlock: 'a',
        endBlock: 'a',
        startOffset: 0,
        endOffset: 5,
        active: false,
      ),
    ];
    await tester.pump();
    expect(render.debugNeedsPaint, isFalse);

    // Same values, new list instance → listEquals short-circuits.
    render.commentHighlights = [
      (
        startBlock: 'a',
        endBlock: 'a',
        startOffset: 0,
        endOffset: 5,
        active: false,
      ),
    ];
    expect(render.debugNeedsPaint, isFalse);
  });

  testWidgets('stale, out-of-range and unknown ranges paint without throwing', (
    tester,
  ) async {
    // The document can change between fetching threads and painting them, so a
    // range may no longer fit. It must degrade, not throw (a paint exception
    // would take the whole editor down).
    final render = await pump(tester);
    render.commentHighlights = [
      // Past the end of 'hello world' (11).
      (
        startBlock: 'a',
        endBlock: 'a',
        startOffset: 5,
        endOffset: 999,
        active: false,
      ),
      // Entirely past the end.
      (
        startBlock: 'a',
        endBlock: 'a',
        startOffset: 900,
        endOffset: 999,
        active: true,
      ),
      // Inverted.
      (
        startBlock: 'b',
        endBlock: 'b',
        startOffset: 5,
        endOffset: 1,
        active: false,
      ),
      // Zero-length.
      (
        startBlock: 'b',
        endBlock: 'b',
        startOffset: 2,
        endOffset: 2,
        active: false,
      ),
      // Negative (never sent, but must not crash).
      (
        startBlock: 'a',
        endBlock: 'a',
        startOffset: -3,
        endOffset: 4,
        active: false,
      ),
      // A block that is not in this document at all.
      (
        startBlock: 'does-not-exist',
        endBlock: 'does-not-exist',
        startOffset: 0,
        endOffset: 3,
        active: false,
      ),
    ];
    await tester.pump();

    // Still alive and still measuring the same geometry.
    expect(render.caretRectFor(const DocPosition(0, 3)), isNotNull);
    expect(tester.takeException(), isNull);
  });

  testWidgets('clearing the highlights repaints back to plain text', (
    tester,
  ) async {
    final render = await pump(tester);
    render.commentHighlights = [
      (
        startBlock: 'a',
        endBlock: 'a',
        startOffset: 0,
        endOffset: 5,
        active: false,
      ),
    ];
    await tester.pump();

    render.commentHighlights = const [];
    expect(render.debugNeedsPaint, isTrue);
    expect(render.debugNeedsLayout, isFalse);
    await tester.pump();
    expect(tester.takeException(), isNull);
  });

  // ── Cross-block ranges ────────────────────────────────────────────────────
  //
  // A comment can span blocks: the server anchor has always carried distinct
  // start/end blocks, and `_addCommentOnSelection` builds cross-block quotes. The
  // highlight type used to be one `blockId` plus two offsets, so `main.dart` had
  // to pass `startBlock` with the END block's offset. Two ways that went wrong:
  //   * a→b, startOffset 5, endOffset 1 → `to.clamp(from: 5, …)` = 5, an empty
  //     range: the comment was INVISIBLE;
  //   * a→b, startOffset 2, endOffset 9 → washed 'a'[2..9], a run whose length
  //     came from a different block, and never touched 'b'.
  // Only counting rectangles separates those from "correct", so these capture
  // what was painted instead of merely asserting nothing threw.

  /// The rects drawn in the comment wash colour during one paint.
  ///
  /// `TestRecordingCanvas` rather than a hand-rolled spy: a `noSuchMethod`
  /// forwarder cannot actually forward (`Object.noSuchMethod` throws), and this
  /// is the tool flutter_test ships for exactly this question.
  List<Rect> washRects(RenderDocument render, Color wash) {
    final canvas = _ViewportRecordingCanvas();
    render.paint(TestRecordingPaintingContext(canvas), Offset.zero);
    // `toARGB32()`, not `==`. `Paint.color` hands back a Color rebuilt from its
    // packed 32-bit form, and its float components are not bit-identical to the
    // token's — so `paint.color == token` is FALSE for two colours that print the
    // same and have the same ARGB32 and colour space. Comparing the packed value
    // is both the honest question ("is this the same colour?") and the one that
    // survives however Flutter stores it. Verified, not assumed: an identity
    // check on these two returned false while their ARGB32 matched exactly.
    final want = wash.toARGB32();
    return [
      for (final recorded in canvas.invocations)
        if (recorded.invocation.memberName == #drawRect &&
            (recorded.invocation.positionalArguments[1] as Paint).color
                    .toARGB32() ==
                want)
          recorded.invocation.positionalArguments[0] as Rect,
    ];
  }

  final wash = const EditorAppearance().tokens.editor.commentHighlight;

  testWidgets('a cross-block comment washes BOTH blocks, not one wrong run', (
    tester,
  ) async {
    // 'hello world'[6..] + 'second'[..3] — "world" plus "sec".
    final render = await pump(
      tester,
      highlights: const [
        (
          startBlock: 'a',
          startOffset: 6,
          endBlock: 'b',
          endOffset: 3,
          active: false,
        ),
      ],
    );

    final rects = washRects(render, wash);
    expect(
      rects.length,
      greaterThanOrEqualTo(2),
      reason: 'one run per block; the old code painted at most one',
    );
    // Two distinct vertical bands = two blocks, not one block twice.
    expect(
      rects.map((r) => r.top).toSet().length,
      2,
      reason: 'the wash must reach the second block',
    );
  });

  testWidgets('an inverted cross-block range is no longer invisible', (
    tester,
  ) async {
    // The exact shape that used to vanish: endOffset < startOffset, because the
    // two numbers were measured against different blocks.
    final render = await pump(
      tester,
      highlights: const [
        (
          startBlock: 'a',
          startOffset: 5,
          endBlock: 'b',
          endOffset: 1,
          active: false,
        ),
      ],
    );
    expect(
      washRects(render, wash),
      isNotEmpty,
      reason: 'a real comment spanning two blocks must be visible',
    );
  });

  testWidgets('a same-block range still washes only its own slice', (
    tester,
  ) async {
    final render = await pump(
      tester,
      highlights: const [
        (
          startBlock: 'a',
          startOffset: 0,
          endBlock: 'a',
          endOffset: 5,
          active: false,
        ),
      ],
    );
    final rects = washRects(render, wash);
    expect(rects, isNotEmpty);
    expect(
      rects.map((r) => r.top).toSet().length,
      1,
      reason: 'one block in, one band out — no bleed into the next block',
    );
  });

  testWidgets('an end block that no longer exists collapses to the start', (
    tester,
  ) async {
    // A stale anchor naming a deleted end block must not wash to the end of the
    // document. Degrade to the one block still worth trusting.
    final render = await pump(
      tester,
      highlights: const [
        (
          startBlock: 'a',
          startOffset: 0,
          endBlock: 'gone',
          endOffset: 4,
          active: false,
        ),
      ],
    );
    expect(tester.takeException(), isNull);
    final rects = washRects(render, wash);
    // `isNotEmpty` first, deliberately: asserting only "at most one band" would
    // pass on zero bands, which is how a test quietly stops testing anything.
    expect(rects, isNotEmpty, reason: 'the start block is still washable');
    expect(
      rects.map((r) => r.top).toSet().length,
      1,
      reason: 'never spill into blocks the anchor did not name',
    );
  });
}

/// `TestRecordingCanvas` answers every un-stubbed Canvas call with null, and
/// `RenderDocument.paint` starts by reading `getLocalClipBounds()` to decide what
/// is on screen — null there is a type error before any drawing happens. Report a
/// viewport big enough that nothing is culled, so the assertions are about the
/// highlight logic and not about scroll position.
class _ViewportRecordingCanvas extends TestRecordingCanvas {
  @override
  Rect getLocalClipBounds() => const Rect.fromLTWH(0, 0, 4000, 4000);
}
