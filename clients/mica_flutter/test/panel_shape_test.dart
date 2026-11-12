import 'dart:ui' as ui;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';
import 'package:mica_flutter/ui/panel_shape.dart';

void main() {
  test(
    'zero elevation panels paint an outside shadow without tinting content',
    () async {
      final recorder = ui.PictureRecorder();
      final canvas = Canvas(recorder);
      canvas.drawColor(Colors.white, BlendMode.src);
      MicaTokens.light.dialogShape.paint(
        canvas,
        const Rect.fromLTWH(50, 50, 100, 100),
      );
      final pic = recorder.endRecording();
      final image = await pic.toImage(240, 240);
      try {
        final bytes = (await image.toByteData(
          format: ui.ImageByteFormat.rawRgba,
        ))!.buffer.asUint8List();
        int red(int x, int y) => bytes[(y * 240 + x) * 4];
        expect(
          red(100, 100),
          255,
          reason: 'foreground shadow must never tint the dialog interior',
        );
        expect(
          red(100, 160),
          lessThan(255),
          reason: 'the requested outside blur must actually paint',
        );
      } finally {
        image.dispose();
        pic.dispose();
      }
    },
  );
  test('theme panels keep their border and shadow during interpolation', () {
    final a = MicaTokens.light.dialogShape, b = MicaTokens.dark_.dialogShape;
    final mid = ShapeBorder.lerp(a, b, 0.5)! as MicaPanelShape;
    expect(mid.shadow.blurRadius, 24);
    expect(mid.shadow.offset, const Offset(0, 8));
    expect(mid.side.width, 1);
    final theme = MicaTokens.light.toMaterialTheme();
    expect(theme.splashFactory, NoSplash.splashFactory);
    expect(theme.dialogTheme.elevation, 0);
    expect(theme.popupMenuTheme.elevation, 0);
    expect(theme.inputDecorationTheme.focusedBorder, isA<MicaFocusBorder>());
    expect(theme.inputDecorationTheme.focusedBorder!.borderSide.width, 1);
  });
}
