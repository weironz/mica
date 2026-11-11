import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../cjk_fonts.dart';

Future<void>? _nativeFonts;

Future<void> _loadNativeFonts() async {
  for (final (family, path) in const [
    ('NotoSansSC', 'fonts/NotoSansSC-Regular.otf'),
    ('CJKFallback', 'fonts/DroidSansFallback.ttf'),
  ]) {
    await (FontLoader(family)..addFont(rootBundle.load(path))).load();
  }
}

/// Loads retained native fallbacks, and requests the Web's first visible Noto shards.
///
/// The span styles match the Material theme. Web loading is asynchronous;
/// text and cached SVG previews repaint when the engine announces new fonts.
Future<void> warmUpFonts() async {
  if (usesBundledCjkFonts) await (_nativeFonts ??= _loadNativeFonts());
  for (final span in cjkFontWarmupSpans()) {
    final painter = TextPainter(text: span, textDirection: TextDirection.ltr)
      ..layout();
    painter.dispose();
  }
  // Also warm the icon font used by the editor's painted toolbars.
  for (final icon in [Icons.content_copy, Icons.wrap_text, Icons.add]) {
    final painter = TextPainter(
      text: TextSpan(
        text: String.fromCharCode(icon.codePoint),
        style: TextStyle(
          fontFamily: icon.fontFamily,
          package: icon.fontPackage,
          fontSize: 16,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    painter.dispose();
  }
}

/// The exact CJK samples used by [warmUpFonts], exposed for regression tests.
@visibleForTesting
Iterable<TextSpan> cjkFontWarmupSpans() sync* {
  for (final text in const ['中文字体预热示例', '繁體字預熱']) {
    yield TextSpan(
      text: text,
      style: TextStyle(fontSize: 16, fontFamilyFallback: cjkFontFallback),
    );
  }
}
