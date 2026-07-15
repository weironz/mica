import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

// An animated image swaps its frame ten-plus times a second. Going through the
// `images` setter would relayout the entire document every one of those times,
// for a picture whose box never moves — so [RenderDocument.replaceImage] is a
// paint-only path, and that is worth pinning: the cost of getting it wrong is
// invisible (it still looks right, it just melts a core on a long page).

ui.Image _image(int w, int h) {
  final recorder = ui.PictureRecorder();
  Canvas(recorder).drawRect(
    Rect.fromLTWH(0, 0, w.toDouble(), h.toDouble()),
    Paint()..color = const Color(0xFF112233),
  );
  return recorder.endRecording().toImageSync(w, h);
}

void main() {
  Future<RenderDocument> pumpImageDoc(
    WidgetTester tester,
    Map<String, ui.Image> images, {
    void Function(String key)? onImagePainted,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: DocumentSurface(
            nodes: [
              EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'}),
            ],
            selection: null,
            showCaret: false,
            caretOn: false,
            appearance: const EditorAppearance(),
            images: images,
            onImagePainted: onImagePainted,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
  }

  testWidgets('a same-size frame repaints without relayout', (tester) async {
    final first = _image(40, 40);
    final render = await pumpImageDoc(tester, {'f1': first});
    expect(render.debugNeedsPaint, isFalse, reason: 'settled before we start');

    final second = _image(40, 40);
    render.replaceImage('f1', second);
    expect(render.debugNeedsPaint, isTrue);
    expect(render.debugNeedsLayout, isFalse,
        reason: 'the box did not move — relayouting per frame is the bug');

    await tester.pump();
    first.dispose();
    second.dispose();
  });

  testWidgets('a frame that changes size falls back to a relayout',
      (tester) async {
    // Frames composite to the full image size, so this should not happen — but
    // painting a new size into a stale box would be silent corruption, and the
    // guard costs two integer compares.
    final first = _image(40, 40);
    final render = await pumpImageDoc(tester, {'f1': first});

    final bigger = _image(80, 80);
    render.replaceImage('f1', bigger);
    expect(render.debugNeedsLayout, isTrue);

    await tester.pump();
    first.dispose();
    bigger.dispose();
  });

  testWidgets('painting an image reports it — this is what keeps a GIF alive',
      (tester) async {
    // The editor stops loops nobody draws (deleted block, replaced source), and
    // this callback is the only signal it has that a picture is still on the
    // canvas.
    final painted = <String>[];
    final img = _image(40, 40);
    await pumpImageDoc(tester, {'f1': img}, onImagePainted: painted.add);
    expect(painted, contains('f1'));
    img.dispose();
  });

  testWidgets('an image that never decoded is not reported as painted',
      (tester) async {
    // A placeholder is not the picture: reporting here would keep a loop
    // running for something that isn't on screen.
    final painted = <String>[];
    await pumpImageDoc(tester, const {}, onImagePainted: painted.add);
    expect(painted, isEmpty);
  });
}
