import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/preview_raster.dart';

// The REAL RasterPreviewPipeline — not a replica — must deliver a baseline
// alongside every math raster it captures. Inline atoms sit formulas on the
// text baseline with it; without one they degrade to middle alignment, which
// reads visibly wrong next to text ("x" floating above the line).
//
// Probed before building: through this exact host tree (Column +
// RepaintBoundary), \frac{a}{b} at fontSize 18 reports ≈19.94 from the top —
// within 0.004px of a bare layout, so the wrapping is transparent.

void main() {
  testWidgets('capturing a math raster also files its baseline', (
    tester,
  ) async {
    StateSetter? rebuild;
    final pipeline = RasterPreviewPipeline(
      previewers: const [MathPreviewer()],
      // The widget may not be pumped yet when request() defers its first
      // callback; run the mutation regardless and rebuild when we can.
      requestRebuild: (fn) => rebuild == null ? fn() : rebuild!(fn),
    );

    await tester.pumpWidget(
      MaterialApp(
        home: StatefulBuilder(
          builder: (context, setState) {
            rebuild = setState;
            return Stack(
              clipBehavior: Clip.hardEdge,
              children: [
                const SizedBox.expand(),
                Positioned(
                  left: -100000,
                  top: 0,
                  child: pipeline.offstageHost(),
                ),
              ],
            );
          },
        ),
      ),
    );

    pipeline.request('math', r'\frac{a}{b}');
    // request() defers everything post-frame; pump until the capture lands
    // (bounded — the pipeline itself gives up after 20 retries).
    for (var i = 0; i < 25 && pipeline.imagesOf('math').isEmpty; i++) {
      await tester.pump();
      // toImage is async; let it complete.
      await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    }

    final img = pipeline.imagesOf('math')[r'\frac{a}{b}'];
    expect(img, isNotNull, reason: 'the raster itself must still capture');
    expect(img!.width, greaterThan(0));

    final baseline = pipeline.baselinesOf('math')[r'\frac{a}{b}'];
    expect(baseline, isNotNull, reason: 'baseline must ride along');
    expect(
      baseline!,
      closeTo(19.94, 2.0),
      reason:
          'top-to-baseline of \\frac{a}{b} at 18pt, from the pre-build probe',
    );
    // And it must be inside the image (logical height = px / pixelRatio).
    expect(baseline, lessThanOrEqualTo(img.height / EditorTheme_mathDpr));
  });
}

// EditorTheme.mathPixelRatio without importing render.dart's part-of tangle.
const double EditorTheme_mathDpr = 2.0;
