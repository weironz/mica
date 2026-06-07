import 'dart:ui' as ui;

import 'package:flutter_svg/flutter_svg.dart';
import 'package:merman/merman.dart';

import 'mermaid_svg_inline.dart';

/// Non-web (desktop/mobile) mermaid rendering through the headless pure-Rust
/// `merman` engine (FFI, no browser/JS) → SVG → rasterized [ui.Image].
///
/// Mirrors the web path's contract (mermaid_preview_web.dart): identical
/// signature, the same "fill targetWidth at 2x device pixels, crisply" raster,
/// and a silent null on any failure — a failed preview leaves the ```mermaid
/// block on its highlighted source form (the dialect principle's degradation),
/// it never crashes. Rendering is fully OFFLINE: merman computes the diagram
/// locally, no network, no backend.
const bool mermaidAvailable = true;

/// Process-wide engine, opened lazily on first render. Opening loads the
/// bundled native library once; a null result (load failed on this platform)
/// makes [renderMermaid] degrade rather than throw.
Merman? _engine;
bool _engineTried = false;

Merman? _openEngine() {
  if (_engineTried) return _engine;
  _engineTried = true;
  try {
    _engine = Merman.open();
  } catch (_) {
    _engine = null;
  }
  return _engine;
}

/// Render mermaid [source] to a rasterized image. merman emits a `resvg-safe`
/// SVG (no `<foreignObject>` — plain shapes/text that flutter_svg decodes
/// faithfully), which becomes a [ui.Picture]; we scale it to fill [targetWidth]
/// at 2x device pixels (matching the web path so a diagram stays crisp when the
/// layout stretches it to the content width) and rasterize. Any failure (engine
/// load, mermaid syntax via [MermanException], SVG decode) resolves to null.
Future<ui.Image?> renderMermaid(String source, double targetWidth) async {
  final engine = _openEngine();
  if (engine == null) return null;

  // Yield before the synchronous, CPU-bound merman render so it never runs
  // inside the build/layout pass that requested this preview.
  await Future<void>.delayed(Duration.zero);

  try {
    final raw = engine.renderSvg(
      source,
      optionsJson: '{"svg":{"pipeline":"resvg-safe"}}',
    );
    if (raw.trim().isEmpty) return null;
    // flutter_svg ignores merman's <style> CSS; flatten it to inline styles
    // first so the diagram keeps its theme fills/strokes instead of going black.
    final svg = inlineMermaidCss(raw);

    final info = await vg.loadPicture(SvgStringLoader(svg), null);
    try {
      // resvg-safe SVG carries an explicit width/height, so the intrinsic size
      // is trustworthy; fall back to a sane box if a renderer ever omits it.
      final natW = info.size.width <= 0 ? 600.0 : info.size.width;
      final natH = info.size.height <= 0 ? 400.0 : info.size.height;
      // Scale to fill the display width at 2x; cap so a tiny diagram on a huge
      // page doesn't allocate an absurd canvas (same clamp as the web path).
      final scale = ((targetWidth * 2) / natW).clamp(0.5, 8.0);
      final w = (natW * scale).round();
      final h = (natH * scale).round();

      final recorder = ui.PictureRecorder();
      ui.Canvas(recorder)
        ..scale(scale)
        ..drawPicture(info.picture);
      final scaled = recorder.endRecording();
      try {
        return await scaled.toImage(w, h);
      } finally {
        scaled.dispose();
      }
    } finally {
      info.picture.dispose();
    }
  } catch (_) {
    return null;
  }
}
