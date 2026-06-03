import 'package:flutter/material.dart';

/// The Mica wordmark glyph: three stacked rhombus "sheets" — a nod to mica, the
/// layered mineral, and to the editor's stacked blocks. Drawn in-house (no asset).
class MicaLogo extends StatelessWidget {
  const MicaLogo({this.size = 24, super.key});

  final double size;

  @override
  Widget build(BuildContext context) {
    return SizedBox.square(
      dimension: size,
      child: CustomPaint(painter: _MicaLogoPainter()),
    );
  }
}

class _MicaLogoPainter extends CustomPainter {
  // Bottom → top: light to deep blue, so the stack reads as layered sheets.
  static const _layers = [
    Color(0xFF93C5FD),
    Color(0xFF3B82F6),
    Color(0xFF1D4ED8),
  ];

  @override
  void paint(Canvas canvas, Size size) {
    final w = size.width;
    final h = size.height;
    final cx = w / 2;
    final hw = w * 0.44; // half width of a sheet
    final hh = h * 0.20; // half height of a sheet
    final gap = h * 0.165; // vertical offset between layers

    Path diamond(double cy) => Path()
      ..moveTo(cx, cy - hh)
      ..lineTo(cx + hw, cy)
      ..lineTo(cx, cy + hh)
      ..lineTo(cx - hw, cy)
      ..close();

    // Paint bottom-most first so upper sheets overlap it.
    final baseCy = h * 0.5 + gap;
    for (var i = 0; i < _layers.length; i++) {
      canvas.drawPath(
        diamond(baseCy - i * gap),
        Paint()
          ..color = _layers[i]
          ..isAntiAlias = true,
      );
    }
  }

  @override
  bool shouldRepaint(covariant _MicaLogoPainter oldDelegate) => false;
}
