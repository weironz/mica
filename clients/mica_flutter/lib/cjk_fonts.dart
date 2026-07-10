import 'package:flutter/foundation.dart';

/// The CJK font fallback chain, per platform.
///
/// The base UI/prose font (Roboto, or the user's pick) carries no CJK glyphs, so
/// Chinese/Japanese/Korean text resolves through this fallback list. On desktop
/// we name the crisp SYSTEM CJK fonts — Windows 微软雅黑 (Microsoft YaHei),
/// macOS 苹方 (PingFang SC), Linux Noto CJK — the same "defer to the OS font"
/// approach AppFlowy uses. The old bundled `DroidSansFallback` (family
/// `CJKFallback`) is a single-weight, spindly pre-Noto face; leaving it as the
/// SOLE fallback is what made Chinese look thin/washed-out ("虚"). It stays only
/// as the last-resort tail so a glyph never renders as tofu.
///
/// Web keeps only the bundled family: Flutter-web CanvasKit cannot resolve
/// system fonts by name, and its on-demand Noto download would break offline
/// mode + flash `.notdef` boxes on the custom-painted editor.
List<String> get cjkFontFallback {
  if (kIsWeb) return const ['CJKFallback'];
  switch (defaultTargetPlatform) {
    case TargetPlatform.windows:
      return const [
        'Microsoft YaHei UI',
        'Microsoft YaHei',
        'Microsoft JhengHei UI', // Traditional
        'CJKFallback',
      ];
    case TargetPlatform.macOS:
      return const [
        'PingFang SC',
        'PingFang TC',
        'Hiragino Sans',
        'CJKFallback',
      ];
    case TargetPlatform.linux:
      return const [
        'Noto Sans CJK SC',
        'Source Han Sans SC',
        'WenQuanYi Micro Hei',
        'CJKFallback',
      ];
    default: // iOS / Android — the system CJK font is already good.
      return const [
        'PingFang SC',
        'Microsoft YaHei UI',
        'Noto Sans CJK SC',
        'CJKFallback',
      ];
  }
}
