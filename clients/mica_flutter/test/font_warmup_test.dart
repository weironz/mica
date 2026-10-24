import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cjk_fonts.dart';
import 'package:mica_flutter/ui/font_warmup.dart';

void main() {
  test('CJK warm-up uses the same bundled fallback as the UI theme', () {
    final spans = cjkFontWarmupSpans().toList();

    expect(spans.map((span) => span.text), ['中文字体预热示例', '繁體字預熱']);
    for (final span in spans) {
      expect(span.style?.fontFamilyFallback, cjkFontFallback);
    }
  });
}
