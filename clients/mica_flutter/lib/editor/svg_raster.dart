import 'dart:ui' as ui;

import 'package:flutter_svg/flutter_svg.dart';
import 'package:xml/xml.dart';

import '../cjk_fonts.dart';

final _cjk = RegExp(
  r'[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]',
  unicode: true,
);
final _korean = RegExp(r'\p{Script=Hangul}', unicode: true);

String _withCjkFonts(String svg) {
  if (!_cjk.hasMatch(svg) || !svg.contains('<text')) return svg;
  final document = XmlDocument.parse(svg);
  for (final text in document.findAllElements('text')) {
    if (!_cjk.hasMatch(text.innerText)) continue;
    // flutter_svg passes a single family to ParagraphBuilder. CanvasKit can't
    // resolve system fonts; its async fallback can leave tofu in a cached
    // image. Use the same bundled faces as the editor for CJK SVG text.
    final font = _korean.hasMatch(text.innerText)
        ? bundledCjkFonts.last
        : bundledCjkFonts.first;
    for (final span in [text, ...text.findAllElements('tspan')]) {
      span.setAttribute('font-family', font);
      final style = span.getAttribute('style');
      if (style != null) {
        span.setAttribute('style', '$style;font-family:$font');
      }
    }
  }
  return document.toXmlString();
}

/// Keep the aspect ratio and bound both texture dimensions: a tall sequence
/// diagram (or wide formula) must not create a hundreds-of-MB texture.
Future<ui.Image?> rasterizeSvg(String svg, double targetPixelWidth) async {
  final info = await vg.loadPicture(SvgStringLoader(_withCjkFonts(svg)), null);
  try {
    final w = info.size.width;
    final h = info.size.height;
    if (!w.isFinite || !h.isFinite || w <= 0 || h <= 0) return null;
    const limit = 4096;
    var scale = (targetPixelWidth / w).clamp(0.0, 8.0);
    if (w * scale > limit) scale = limit / w;
    if (h * scale > limit) scale = limit / h;

    final recorder = ui.PictureRecorder();
    ui.Canvas(recorder)
      ..scale(scale)
      ..drawPicture(info.picture);
    final scaled = recorder.endRecording();
    try {
      return await scaled.toImage(
        (w * scale).ceil().clamp(1, limit),
        (h * scale).ceil().clamp(1, limit),
      );
    } finally {
      scaled.dispose();
    }
  } finally {
    info.picture.dispose();
  }
}
