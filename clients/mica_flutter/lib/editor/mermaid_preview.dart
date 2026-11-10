import 'dart:ui' as ui;

import '../ui/theme_tokens.dart';
import 'mermaid_svg_inline.dart';
import 'mermaid_theme.dart';
import 'preview_engine.dart';
import 'svg_raster.dart';

/// The same merman engine as HTML export, via native FFI or the Web worker.
Future<ui.Image?> renderMermaid(
  String source,
  double targetWidth,
  MicaTokens tokens,
) async {
  try {
    final raw = await mermaidSvg(source, mermaidThemeFor(tokens));
    if (raw == null || raw.trim().isEmpty) return null;
    // flutter_svg ignores merman's <style> CSS. Inline it on both platforms.
    final svg = inlineMermaidCss(raw, background: cssHex(tokens.surface.base));
    return await rasterizeSvg(svg, targetWidth * 2);
  } catch (_) {
    return null;
  }
}
