import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/preview_raster.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

/// A preview is a BITMAP, so the ink colour it was captured with is baked into
/// it — unlike every other colour in the app, it cannot be re-read from the
/// palette at paint time. The cache is keyed by source string alone, so without
/// this invalidation a formula captured in light mode keeps being painted onto a
/// dark page, where dark ink on a dark background is simply not there.
///
/// This is the same shape as the bug where a content-hash image cache happily
/// served a cached 404 (docs/lessons.md): a key that does not mention everything
/// the value depends on.
void main() {
  RasterPreviewPipeline makePipeline() => RasterPreviewPipeline(
    previewers: const [MathPreviewer()],
    requestRebuild: (fn) => fn(),
  );

  /// `createTestImage` decodes for real, so it has to run OUTSIDE the fake-async
  /// zone — awaited directly inside `testWidgets` it never completes and the
  /// test sits there until the 10-minute timeout.
  Future<ui.Image> testImage(WidgetTester tester) async =>
      (await tester.runAsync(() => createTestImage(width: 4, height: 4)))!;

  testWidgets('switching the palette drops every cached raster', (
    tester,
  ) async {
    final pipeline = makePipeline();
    addTearDown(pipeline.dispose);

    pipeline.imagesOf('math')['x^2'] = await testImage(tester);
    pipeline.baselinesOf('math')['x^2'] = 12.5;
    expect(pipeline.imagesOf('math'), isNotEmpty);

    pipeline.tokens = MicaTokens.dark_;

    expect(
      pipeline.imagesOf('math'),
      isEmpty,
      reason: 'a raster inked for the light palette must not be reused',
    );
    expect(
      pipeline.baselinesOf('math'),
      isEmpty,
      reason: 'a baseline describes a raster that no longer exists',
    );
    // The textures are disposed at the next frame boundary, never mid-frame:
    // the frame in flight may still be painting them.
    await tester.pump();
  });

  testWidgets('setting the same palette keeps the cache', (tester) async {
    // Otherwise every rebuild that re-assigns the same tokens would throw the
    // rasters away and re-capture them, which is a per-frame stutter, not a
    // theme switch.
    final pipeline = makePipeline();
    addTearDown(pipeline.dispose);

    pipeline.imagesOf('math')['x^2'] = await testImage(tester);

    pipeline.tokens = MicaTokens.light;

    expect(pipeline.imagesOf('math'), isNotEmpty);
    expect(pipeline.tokens, same(MicaTokens.light));
  });

  testWidgets('the offstage formula takes its ink from the palette handed in', (
    tester,
  ) async {
    // The whole point of threading tokens into buildOffstage: the same source
    // must produce different pixels per palette.
    const previewer = MathPreviewer();
    expect(previewer.buildOffstage('x^2', MicaTokens.light), isNotNull);
    expect(previewer.buildOffstage('x^2', MicaTokens.dark_), isNotNull);
    expect(
      MicaTokens.light.text.primary,
      isNot(MicaTokens.dark_.text.primary),
      reason: 'if these were equal the invalidation above would prove nothing',
    );
  });
}
