import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cjk_fonts.dart';

void main() {
  tearDown(() => debugDefaultTargetPlatformOverride = null);

  test(
    'Windows leads with the crisp system font (微软雅黑), not the bundled one',
    () {
      debugDefaultTargetPlatformOverride = TargetPlatform.windows;
      final f = cjkFontFallback;
      expect(f.first, 'Microsoft YaHei UI');
      expect(f, isNot(contains('CJKFallback')));
      expect(f, isNot(contains('NotoSansSC')));
      expect(f, contains('Malgun Gothic'));
      expect(f, contains('MingLiU-ExtB'));
      expect(usesBundledCjkFonts, isFalse);
    },
  );

  test('macOS leads with PingFang SC (苹方)', () {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    expect(cjkFontFallback.first, 'PingFang SC');
    expect(cjkFontFallback.last, 'CJKFallback');
  });

  test('Linux leads with Noto Sans CJK SC', () {
    debugDefaultTargetPlatformOverride = TargetPlatform.linux;
    expect(cjkFontFallback.first, 'Noto Sans CJK SC');
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
