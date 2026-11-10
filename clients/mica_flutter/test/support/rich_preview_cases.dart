import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/mermaid_preview.dart';
import 'package:mica_flutter/editor/mermaid_theme.dart';
import 'package:mica_flutter/editor/preview_engine.dart';
import 'package:mica_flutter/editor/preview_raster.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void richPreviewCases() {
  test(
    'inline and display fractions rasterize with distinct baselines',
    () async {
      const inline = MathPreviewer(display: false);
      const display = MathPreviewer(display: true);
      const source = r'\frac{a}{b}';
      final preview = (await inline.produce(source, 400, MicaTokens.light))!;
      addTearDown(preview.image.dispose);
      final block = (await display.produce(source, 400, MicaTokens.dark_))!;
      addTearDown(block.image.dispose);
      expect(preview.baseline, inExclusiveRange(0, preview.image.height / 2));
      expect(block.baseline, greaterThan(preview.baseline!));
      expect(block.image.height, greaterThan(preview.image.height));
      final bytes = (await preview.image.toByteData(
        format: ui.ImageByteFormat.rawRgba,
      ))!;
      expect(bytes.buffer.asUint8List().any((byte) => byte != 0), isTrue);

      final dark = (await inline.produce(source, 400, MicaTokens.dark_))!;
      addTearDown(dark.image.dispose);
      final darkBytes = (await dark.image.toByteData(
        format: ui.ImageByteFormat.rawRgba,
      ))!;
      expect(dark.image.width, preview.image.width);
      expect(dark.image.height, preview.image.height);
      expect(darkBytes.buffer.asUint8List(), isNot(bytes.buffer.asUint8List()));
      expect(await inline.produce(r'\frac{x}', 400, MicaTokens.light), isNull);
    },
  );

  test(
    'Mermaid preserves host palettes and invalid source does not stop later renders',
    () async {
      const source = 'flowchart LR\n  A[开始] --> B[结束]';
      final light = mermaidThemeFor(MicaTokens.light);
      final dark = mermaidThemeFor(MicaTokens.dark_);
      final result = await Future.wait([
        mermaidSvg(source, light),
        mermaidSvg('not a diagram', light),
        mermaidSvg(source, dark),
      ]);
      expect(result[0], allOf(contains('<svg'), contains(light.canvas)));
      expect(result[1], isNull);
      expect(result[2], allOf(contains('<svg'), contains(dark.canvas)));
      final image = (await renderMermaid(source, 180, MicaTokens.dark_))!;
      addTearDown(image.dispose);
      expect(image.width, closeTo(360, 1));
      final bytes = (await image.toByteData(
        format: ui.ImageByteFormat.rawRgba,
      ))!;
      final pixels = bytes.buffer.asUint32List();
      expect(pixels.any((pixel) => pixel != pixels.first), isTrue);
    },
  );
}
