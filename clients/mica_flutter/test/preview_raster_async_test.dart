import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/preview_raster.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

class _DelayedPreviewer extends RasterPreviewer {
  final jobs = <Completer<ui.Image?>>[];
  final palettes = <MicaTokens>[];
  @override
  String get id => 'mermaid';
  @override
  Future<ui.Image?> produce(String source, double width, MicaTokens tokens) {
    palettes.add(tokens);
    final job = Completer<ui.Image?>();
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
        previewer.jobs[0].complete(oldImage);
        await tester.pump();
        expect(pipeline.imagesOf('mermaid'), isEmpty);
        // An old completion must not clear the new request's pending state.
        pipeline.request('mermaid', 'graph TD; A-->B');
        expect(previewer.jobs, hasLength(2));
      }
      previewer.jobs[1].complete(newImage);
      await tester.pump();
      if (!oldFirst) {
        previewer.jobs[0].complete(oldImage);
        await tester.pump();
      }
      expect(pipeline.imagesOf('mermaid').values.single, same(newImage));
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

    previewer.jobs.first.complete(oldestImage);
    previewer.jobs[1].completeError(StateError('stale dark render'));
    await tester.pump();
    expect(oldestImage.debugDisposed, isTrue);
    expect(pipeline.imagesOf('mermaid'), isEmpty);
    pipeline.request('mermaid', 'source');
    expect(previewer.jobs, hasLength(3));

    previewer.jobs.last.complete(currentImage);
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
    previewer.jobs.single.complete(lateImage);
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
    previewer.jobs.last.complete(rendered);
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
}
