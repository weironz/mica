import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/preview_engine.dart';
import 'package:mica_flutter/editor/preview_raster.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

class _DelayedPreviewer extends RasterPreviewer {
  final jobs = <Completer<RasterPreview?>>[];
  final palettes = <MicaTokens>[];
  @override
  String get id => 'mermaid';
  @override
  Future<RasterPreview?> produce(
    String source,
    double width,
    MicaTokens tokens,
  ) {
    palettes.add(tokens);
    final job = Completer<RasterPreview?>();
    jobs.add(job);
    return job.future;
  }
}

void main() {
  Future<ui.Image> image(WidgetTester tester) async =>
      (await tester.runAsync(() => createTestImage(width: 4, height: 4)))!;

  for (final oldFirst in [true, false]) {
    testWidgets('theme switch rejects stale image, oldFirst=$oldFirst', (
      tester,
    ) async {
      final previewer = _DelayedPreviewer();
      final pipeline = RasterPreviewPipeline(
        previewers: [previewer],
        requestRebuild: (fn) => fn(),
      );
      addTearDown(pipeline.dispose);
      final oldImage = await image(tester);
      final newImage = await image(tester);
      pipeline.request('mermaid', 'graph TD; A-->B');
      pipeline.tokens = MicaTokens.dark_;
      pipeline.request('mermaid', 'graph TD; A-->B');
      expect(previewer.jobs, hasLength(2));
      if (oldFirst) {
        previewer.jobs[0].complete((image: oldImage, baseline: 1.0));
        await tester.pump();
        expect(pipeline.imagesOf('mermaid'), isEmpty);
        // An old completion must not clear the new request's pending state.
        pipeline.request('mermaid', 'graph TD; A-->B');
        expect(previewer.jobs, hasLength(2));
      }
      previewer.jobs[1].complete((image: newImage, baseline: 1.0));
      await tester.pump();
      if (!oldFirst) {
        previewer.jobs[0].complete((image: oldImage, baseline: 1.0));
        await tester.pump();
      }
      expect(pipeline.imagesOf('mermaid').values.single, same(newImage));
      expect(pipeline.baselinesOf('mermaid').values.single, 1);
      expect(oldImage.debugDisposed, isTrue);
      expect(newImage.debugDisposed, isFalse);
    });
  }

  testWidgets('switching back to the same palette still rejects old work', (
    tester,
  ) async {
    final previewer = _DelayedPreviewer();
    final pipeline = RasterPreviewPipeline(
      previewers: [previewer],
      requestRebuild: (fn) => fn(),
    );
    addTearDown(pipeline.dispose);
    final oldestImage = await image(tester);
    final currentImage = await image(tester);
    pipeline.request('mermaid', 'source');
    pipeline.tokens = MicaTokens.dark_;
    pipeline.request('mermaid', 'source');
    pipeline.tokens = MicaTokens.light;
    pipeline.request('mermaid', 'source');
    expect(previewer.palettes, [
      MicaTokens.light,
      MicaTokens.dark_,
      MicaTokens.light,
    ]);

    previewer.jobs.first.complete((image: oldestImage, baseline: 1.0));
    previewer.jobs[1].completeError(StateError('stale dark render'));
    await tester.pump();
    expect(oldestImage.debugDisposed, isTrue);
    expect(pipeline.imagesOf('mermaid'), isEmpty);
    pipeline.request('mermaid', 'source');
    expect(previewer.jobs, hasLength(3));

    previewer.jobs.last.complete((image: currentImage, baseline: 1.0));
    await tester.pump();
    expect(pipeline.imagesOf('mermaid')['source'], same(currentImage));
    expect(currentImage.debugDisposed, isFalse);
  });

  testWidgets('late image after disposal is released without rebuilding', (
    tester,
  ) async {
    final previewer = _DelayedPreviewer();
    var rebuilds = 0;
    final pipeline = RasterPreviewPipeline(
      previewers: [previewer],
      requestRebuild: (fn) {
        rebuilds++;
        fn();
      },
    );
    final lateImage = await image(tester);
    pipeline.request('mermaid', 'source');
    pipeline.dispose();
    previewer.jobs.single.complete((image: lateImage, baseline: 1.0));
    await tester.pump();
    expect(lateImage.debugDisposed, isTrue);
    expect(rebuilds, 0);
    expect(pipeline.images, isEmpty);
    pipeline.request('mermaid', 'other');
    expect(previewer.jobs, hasLength(1));
  });

  testWidgets('a completed failure is retried once in a new palette', (
    tester,
  ) async {
    final previewer = _DelayedPreviewer();
    final pipeline = RasterPreviewPipeline(
      previewers: [previewer],
      requestRebuild: (fn) => fn(),
    );
    addTearDown(pipeline.dispose);
    pipeline.request('mermaid', 'source');
    previewer.jobs.single.completeError(StateError('light render failed'));
    await tester.pump();
    pipeline.request('mermaid', 'source');
    expect(previewer.jobs, hasLength(1), reason: 'failure must not spin');
    pipeline.tokens = MicaTokens.dark_;
    pipeline.request('mermaid', 'source');
    expect(previewer.jobs, hasLength(2));
    final rendered = await image(tester);
    previewer.jobs.last.complete((image: rendered, baseline: 1.0));
    await tester.pump();
    expect(pipeline.imagesOf('mermaid')['source'], same(rendered));
    pipeline.request('mermaid', 'source');
    expect(previewer.jobs, hasLength(2));
  });

  testWidgets('failed old render cannot poison new theme cache', (
    tester,
  ) async {
    final previewer = _DelayedPreviewer();
    final pipeline = RasterPreviewPipeline(
      previewers: [previewer],
      requestRebuild: (fn) => fn(),
    );
    addTearDown(pipeline.dispose);
    pipeline.request('mermaid', 'source');
    pipeline.tokens = MicaTokens.dark_;
    previewer.jobs.single.completeError(StateError('old theme failed'));
    await tester.pump();
    pipeline.request('mermaid', 'source');
    expect(previewer.jobs, hasLength(2));
    previewer.jobs.last.complete(null);
    await tester.pump();
    pipeline.request('mermaid', 'source');
    expect(previewer.jobs, hasLength(2));
  });

  testWidgets('an unavailable renderer is retried after a pause, not failed', (
    tester,
  ) async {
    final previewer = _DelayedPreviewer();
    final pipeline = RasterPreviewPipeline(
      previewers: [previewer],
      requestRebuild: (fn) => fn(),
    );
    addTearDown(pipeline.dispose);
    final done = await image(tester);

    pipeline.request('mermaid', 'a');
    pipeline.request('mermaid', 'b');
    expect(previewer.jobs, hasLength(2));
    // The renderer went away with both in flight (another source crashed it,
    // or its code failed to load): neither source is to blame.
    previewer.jobs[0].completeError(const PreviewUnavailable());
    previewer.jobs[1].completeError(const PreviewUnavailable());
    await tester.pump();

    // Paused: asking again right away does not hammer the renderer.
    pipeline.request('mermaid', 'a');
    expect(previewer.jobs, hasLength(2));
    await tester.pump(const Duration(milliseconds: 999));
    expect(previewer.jobs, hasLength(2));

    // After the first pause (1 s) both are asked for again.
    await tester.pump(const Duration(milliseconds: 1));
    expect(previewer.jobs, hasLength(4));
    previewer.jobs[2].complete((image: done, baseline: null));
    previewer.jobs[3].complete(null); // this one genuinely cannot render
    await tester.pump();
    expect(pipeline.imagesOf('mermaid')['a'], same(done));

    // A null result is final: no retry, however long we wait.
    pipeline.request('mermaid', 'b');
    await tester.pump(const Duration(seconds: 60));
    expect(previewer.jobs, hasLength(4));
  });

  testWidgets('the pause doubles while the renderer stays unavailable', (
    tester,
  ) async {
    final previewer = _DelayedPreviewer();
    final pipeline = RasterPreviewPipeline(
      previewers: [previewer],
      requestRebuild: (fn) => fn(),
    );
    addTearDown(pipeline.dispose);

    pipeline.request('mermaid', 'a');
    previewer.jobs.last.completeError(const PreviewUnavailable());
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(previewer.jobs, hasLength(2)); // first retry after 1 s

    previewer.jobs.last.completeError(const PreviewUnavailable());
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 1999));
    expect(previewer.jobs, hasLength(2));
    await tester.pump(const Duration(milliseconds: 1));
    expect(previewer.jobs, hasLength(3)); // second retry after 2 s

    // Disposing cancels the pending retry instead of leaving a timer behind.
    previewer.jobs.last.completeError(const PreviewUnavailable());
    await tester.pump();
    pipeline.dispose();
    await tester.pump(const Duration(seconds: 10));
    expect(previewer.jobs, hasLength(3));
  });
}
