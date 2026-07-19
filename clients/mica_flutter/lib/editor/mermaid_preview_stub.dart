import 'dart:ui' as ui;

import 'package:flutter_svg/flutter_svg.dart';

import '../src/rust/api/render.dart';
import 'mermaid_svg_inline.dart';

/// Non-web (desktop/mobile) mermaid rendering through the headless pure-Rust
/// `merman` engine — reached via OUR FFI (`api::render`, feature `render`) →
/// resvg-safe SVG → rasterized [ui.Image]. This is the SAME engine the HTML/PDF
/// export uses (`crates/markdown`), so a diagram looks identical on screen and
/// in an exported file, and the desktop ships mermaid exactly once (no separate
/// Dart merman package).
///
/// Mirrors the web path's contract (mermaid_preview_web.dart): identical
/// signature, the same "fill targetWidth at 2x device pixels, crisply" raster,
/// and a silent null on any failure — a failed preview leaves the ```mermaid
/// block on its highlighted source form (the dialect principle's degradation),
/// it never crashes. Rendering is fully OFFLINE: merman computes the diagram
/// locally, no network, no backend.
const bool mermaidAvailable = true;

/// Render mermaid [source] to a rasterized image. The FFI returns a `resvg-safe`
/// SVG (no `<foreignObject>` — plain shapes/text that flutter_svg decodes
/// faithfully), which becomes a [ui.Picture]; we scale it to fill [targetWidth]
/// at 2x device pixels (matching the web path so a diagram stays crisp when the
/// layout stretches it to the content width) and rasterize. Any failure (render
/// error / empty SVG / decode) resolves to null.
Future<ui.Image?> renderMermaid(String source, double targetWidth) async {
  try {
    // FFI merman render is async (runs on the bridge's worker thread, off the
    // build/layout pass that requested this preview), so no manual yield.
    final raw = await renderMermaidSvg(source: source);
    if (raw == null || raw.trim().isEmpty) return null;
    // flutter_svg ignores merman's <style> CSS; flatten it to inline styles
    // first so the diagram keeps its theme fills/strokes instead of going black.
    final svg = inlineMermaidCss(raw);

    final info = await vg.loadPicture(SvgStringLoader(svg), null);
    try {
      // resvg-safe SVG carries an explicit width/height, so the intrinsic size
      // is trustworthy; fall back to a sane box if a renderer ever omits it.
      final natW = info.size.width <= 0 ? 600.0 : info.size.width;
      final natH = info.size.height <= 0 ? 400.0 : info.size.height;
      // Scale to fill the display width at 2x — then clamp the ABSOLUTE
      // output dimensions. The old clamp bounded only the SCALE (0.5–8.0):
      // height was unbounded outright, and the 0.5 floor forced w >= natW/2,
      // so a tall/wide diagram (sequence charts easily reach natural heights
      // in the tens of thousands) allocated a hundreds-of-MB texture — or
      // exceeded the GPU max texture size (~16384) — in one Picture.toImage.
      // That was the single largest GPU allocation in the app (freeze-audit
      // CONFIRMED, double-verified). 4096px covers any real screen at 2x;
      // paint only stretches beyond it.
      const maxDim = 4096.0;
      var scale = ((targetWidth * 2) / natW).clamp(0.05, 8.0);
      if (natW * scale > maxDim) scale = maxDim / natW;
      if (natH * scale > maxDim) scale = maxDim / natH;
      final w = (natW * scale).round().clamp(1, 4096);
      final h = (natH * scale).round().clamp(1, 4096);

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
