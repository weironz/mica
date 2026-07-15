// Real-device proof that an animated image actually MOVES on the editor canvas.
//
// The unit tests (test/image_animator_test.dart) drive the loop on a fake
// FrameSource, because a fake clock is what timing assertions need — but a fake
// clock is also why they can never answer the only question that matters to a
// user: does the picture change? A real ui.Codec's frames arrive on futures a
// plain `flutter test` never completes, so the whole editor path (fetch →
// decode → animate → repaint the canvas) can only be exercised here, on a
// runner with a real engine and a real clock.
//
// This samples the actual raster, twice, and insists the pixels differ. Nothing
// weaker is honest: "an animator was created" would pass with a frozen picture.
//
//   flutter test integration_test/gif_animation_test.dart -d windows
import 'dart:convert';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mica_flutter/editor/editor.dart';

// 3 frames — solid red, then green, then blue — 80ms each, loops forever.
final kAnimatedGif = base64Decode(
  'R0lGODlhBAAEAIEAAP8AAAAAAAAAAAAAACH/C05FVFNDQVBFMi4wAwEAAAAh+QQACAAAACwAAAAA'
  'BAAEAAAICQABCBxIsCCAgAAh+QQBCAABACwAAAAABAAEAIEA/wAAAAAAAAAAAAAICQABCBxIsCCA'
  'gAAh+QQBCAABACwAAAAABAAEAIEAAP8AAAAAAAAAAAAICQABCBxIsCCAgAA7',
);

// A still 2x2 red PNG: the control. If "the pixels changed" also fires for
// this, the GIF assertion below proves nothing about animation.
final kStillPng = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP8zwACTGCSAQANHQED'
  'gslx/wAAAABJRU5ErkJggg==',
);

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  final boundary = GlobalKey();

  Future<void> pumpImage(WidgetTester tester, Uint8List bytes) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: RepaintBoundary(
            key: boundary,
            child: MicaEditor(
              rootBlockId: 'root',
              nodes: [
                EditorNode(
                  id: 'i',
                  kind: 'image',
                  text: '',
                  data: const {'file_id': 'pic'},
                ),
              ],
              version: 0,
              canEdit: true,
              reHostImages: false,
              onApplyOperations: (_) async {},
              onLoadImageBytes: (_) async => bytes,
            ),
          ),
        ),
      ),
    );
    // Give the fetch + decode a real moment to land.
    for (var i = 0; i < 20; i++) {
      await tester.pump(const Duration(milliseconds: 20));
    }
  }

  /// The canvas as raw pixels, right now.
  Future<Uint8List> raster(WidgetTester tester) async {
    final render =
        boundary.currentContext!.findRenderObject()! as RenderRepaintBoundary;
    late Uint8List pixels;
    await tester.runAsync(() async {
      final image = await render.toImage();
      final data = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
      pixels = Uint8List.fromList(data!.buffer.asUint8List());
      image.dispose();
    });
    return pixels;
  }

  int differingBytes(Uint8List a, Uint8List b) {
    if (a.length != b.length) return a.length;
    var n = 0;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) n++;
    }
    return n;
  }

  bool hasInk(Uint8List px) {
    // The image paints onto a white matte; a page that never drew the picture
    // is uniform. Any solid red/green/blue block breaks that up.
    for (var i = 0; i < px.length; i += 4) {
      if (px[i] != px[0] || px[i + 1] != px[1] || px[i + 2] != px[2]) {
        return true;
      }
    }
    return false;
  }

  testWidgets('a GIF advances frames on the canvas', (tester) async {
    await pumpImage(tester, kAnimatedGif);
    final first = await raster(tester);
    expect(hasInk(first), isTrue,
        reason: 'the picture must be on the canvas before we can watch it move');

    // Two frames' worth of real time — solid red should have become green/blue.
    for (var i = 0; i < 12; i++) {
      await tester.pump(const Duration(milliseconds: 20));
    }
    final later = await raster(tester);

    expect(differingBytes(first, later), greaterThan(0),
        reason: 'the GIF is frozen on frame 0 — the animator never ran');
  });

  testWidgets('a still image does not change — the control', (tester) async {
    await pumpImage(tester, kStillPng);
    final first = await raster(tester);
    expect(hasInk(first), isTrue);

    for (var i = 0; i < 12; i++) {
      await tester.pump(const Duration(milliseconds: 20));
    }
    final later = await raster(tester);

    expect(differingBytes(first, later), 0,
        reason: 'a still image must never repaint differently — if this fails, '
            'the GIF assertion above is measuring something else');
  });
}
