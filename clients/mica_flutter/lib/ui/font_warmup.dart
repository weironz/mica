import 'package:flutter/material.dart';

import '../cjk_fonts.dart';

/// Loads the CJK fallback before the first document paints on Flutter Web.
///
/// The custom editor would otherwise briefly paint `.notdef` while the bundled
/// font becomes available. The span styles must match the Material theme: a
/// default-style painter warms the engine's remote family and leaves the first
/// themed CJK label to reshape when that download finishes.
void warmUpFonts() {
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
