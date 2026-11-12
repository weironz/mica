import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cjk_fonts.dart';

void main() {
  tearDown(() => debugDefaultTargetPlatformOverride = null);

  test('Windows D uses bundled Noto with system rare-glyph fallback', () {
    debugDefaultTargetPlatformOverride = TargetPlatform.windows;
    final f = cjkFontFallback;
    expect(f, [
      'NotoSansSC',
      'Microsoft YaHei UI',
      'Microsoft YaHei',
      'Microsoft JhengHei UI',
      'Yu Gothic UI',
      'Malgun Gothic',
      'MingLiU-ExtB',
      'SimSun-ExtB',
    ]);
    expect(f, isNot(contains('CJKFallback')));
    expect(nativeCjkFontAssets.toList(), [
      ('NotoSansSC', 'fonts/NotoSansSC-Regular.otf'),
    ]);
    expect(f, contains('Malgun Gothic'));
    expect(f, contains('MingLiU-ExtB'));
    expect(usesBundledCjkFonts, isTrue);
    expect(svgCjkFontFamily(korean: false), 'NotoSansSC');
    expect(svgCjkFontFamily(korean: true), 'Malgun Gothic');
  });

  test('macOS leads with PingFang SC (苹方)', () {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    expect(cjkFontFallback, [
      'PingFang SC',
      'PingFang TC',
      'Hiragino Sans',
      'NotoSansSC',
      'CJKFallback',
    ]);
    expect(cjkFontFallback.last, 'CJKFallback');
  });

  test('Linux leads with Noto Sans CJK SC', () {
    debugDefaultTargetPlatformOverride = TargetPlatform.linux;
    expect(cjkFontFallback, [
      'Noto Sans CJK SC',
      'Source Han Sans SC',
      'WenQuanYi Micro Hei',
      'NotoSansSC',
      'CJKFallback',
    ]);
    expect(cjkFontFallback.last, 'CJKFallback');
  });

  test('other native platforms keep the bundled font safety net', () {
    for (final p in TargetPlatform.values.where(
      (p) => p != TargetPlatform.windows,
    )) {
      debugDefaultTargetPlatformOverride = p;
      expect(cjkFontFallback.last, 'CJKFallback', reason: '$p keeps the tail');
      expect(
        cjkFontFallback.length,
        greaterThan(1),
        reason: '$p prepends system CJK fonts',
      );
      expect(usesBundledCjkFonts, isTrue);
    }
  });
}
