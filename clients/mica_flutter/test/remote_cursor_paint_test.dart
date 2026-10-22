import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

const _remoteColor = Color(0xFF123ABC);

class _ClippedCanvas extends TestRecordingCanvas {
  _ClippedCanvas(this.clip);

  final Rect clip;

  @override
  Rect getLocalClipBounds() => clip;
}

int _remoteShapes(RenderDocument render, Rect clip) {
  final canvas = _ClippedCanvas(clip);
  render.paint(TestRecordingPaintingContext(canvas), Offset.zero);
  return canvas.invocations.where((recorded) {
    if (recorded.invocation.memberName != #drawRRect) return false;
    final paint = recorded.invocation.positionalArguments[1] as Paint;
    return paint.color.toARGB32() == _remoteColor.toARGB32();
  }).length;
}

void main() {
  testWidgets('remote cursor outside the paint viewport is culled', (
    tester,
  ) async {
    final nodes = [
      for (var i = 0; i < 80; i++)
        EditorNode(id: 'b$i', kind: 'paragraph', text: 'line $i'),
    ];
    final scroll = ScrollController();
    addTearDown(scroll.dispose);
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: SingleChildScrollView(
            controller: scroll,
            child: DocumentSurface(
              nodes: nodes,
              selection: null,
              showCaret: false,
              caretBlink: ValueNotifier(false),
              appearance: const EditorAppearance(),
              remoteCursors: const [
                (
                  blockId: 'b15',
                  offset: 0,
                  color: _remoteColor,
                  label: 'near',
                ),
                (
                  blockId: 'b79',
                  offset: 0,
                  color: _remoteColor,
                  label: 'other',
                ),
              ],
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final render = tester.renderObject<RenderDocument>(
      find.byType(DocumentSurface),
    );
    final remoteY = render.caretRectFor(const DocPosition(79, 0))!.top;
    final nearY = render.caretRectFor(const DocPosition(15, 0))!.top;
    expect(remoteY, greaterThan(700));
    expect(nearY, greaterThan(100));
    expect(nearY, lessThan(700));

    // The recording canvas does not clip invocations itself: any remote shape
    // here means we wasted paint work on a cursor far below the viewport.
    expect(_remoteShapes(render, const Rect.fromLTWH(0, 0, 800, 100)), 0);
    // What the app actually hands paint: the scroll viewport is a repaint
    // boundary, so it clips with a LAYER and the canvas reports no clip at all
    // (±1e9). Culling has to come from the viewport itself (#62) — reading the
    // canvas alone, this painted every block of the document.
    expect(
      _remoteShapes(render, const Rect.fromLTRB(-1e9, -1e9, 1e9, 1e9)),
      2,
      reason: 'only the on-screen cursor paints (bar + flag); the far-below '
          'one is culled even with an unclipped canvas',
    );
    // Scroll the real viewport down to it; then it paints.
    scroll.jumpTo(remoteY - 100);
    await tester.pump();
    expect(
      _remoteShapes(render, Rect.fromLTWH(0, remoteY - 20, 800, 100)),
      2,
      reason: 'the visible cursor still paints its bar and name flag',
    );
  });
}
