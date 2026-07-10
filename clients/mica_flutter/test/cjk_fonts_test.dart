import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cjk_fonts.dart';

void main() {
  tearDown(() => debugDefaultTargetPlatformOverride = null);

  test('Windows leads with the crisp system font (微软雅黑), not the bundled one', () {
    debugDefaultTargetPlatformOverride = TargetPlatform.windows;
    final f = cjkFontFallback;
    expect(f.first, 'Microsoft YaHei UI');
    // The washed-out bundled DroidSansFallback is no longer the SOLE fallback,
    // only the last-resort tail.
    expect(f, isNot(equals(const ['CJKFallback'])));
    expect(f.last, 'CJKFallback');
  });

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

  test('every platform keeps the bundled font as a tofu safety net', () {
    for (final p in TargetPlatform.values) {
      debugDefaultTargetPlatformOverride = p;
      expect(cjkFontFallback.last, 'CJKFallback', reason: '$p keeps the tail');
      expect(cjkFontFallback.length, greaterThan(1),
          reason: '$p prepends system CJK fonts');
    }
  });
}
