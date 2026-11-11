import 'package:flutter/foundation.dart';

const bundledCjkFonts = ['NotoSansSC', 'CJKFallback'];

/// Windows uses its standard CJK fonts; Web downloads engine-selected shards.
bool get usesBundledCjkFonts =>
    !kIsWeb && defaultTargetPlatform != TargetPlatform.windows;

/// The CJK font fallback chain, per platform.
///
/// The base UI/prose font (Roboto, or the user's pick) carries no CJK glyphs, so
/// Chinese/Japanese/Korean text resolves through this fallback list. On desktop
/// we name the crisp SYSTEM CJK fonts — Windows 微软雅黑 (Microsoft YaHei),
/// macOS 苹方 (PingFang SC), Linux Noto CJK — the same "defer to the OS font"
/// approach AppFlowy uses. The old bundled `DroidSansFallback` (family
/// `CJKFallback`) is a single-weight, spindly pre-Noto face; leaving it as the
/// SOLE fallback is what made Chinese look thin/washed-out ("虚"). It stays only
/// as a last-resort tail on platforms without guaranteed system CJK fonts.
/// Neither bundled face covers every Unicode extension character.
///
/// Web lets CanvasKit choose Noto shards from the same-origin mirror. Glyphs
/// not downloaded before going offline may remain missing until reconnected.
List<String> get cjkFontFallback {
  if (kIsWeb) return const [];
  // Desktop leads with the OS CJK font; the two bundled families are only the
  // last-resort tail. Windows ships the system families below.
  const tail = bundledCjkFonts;
  switch (defaultTargetPlatform) {
    case TargetPlatform.windows:
      return const [
        'Microsoft YaHei UI',
        'Microsoft YaHei',
        'Microsoft JhengHei UI', // Traditional
        'Yu Gothic UI',
        'Malgun Gothic',
        'MingLiU-ExtB',
        'SimSun-ExtB',
      ];
    case TargetPlatform.macOS:
      return const ['PingFang SC', 'PingFang TC', 'Hiragino Sans', ...tail];
    case TargetPlatform.linux:
      return const [
        'Noto Sans CJK SC',
        'Source Han Sans SC',
        'WenQuanYi Micro Hei',
        ...tail,
      ];
    default: // iOS / Android — the system CJK font is already good.
      return const [
        'PingFang SC',
        'Microsoft YaHei UI',
        'Noto Sans CJK SC',
        ...tail,
      ];
  }
}
