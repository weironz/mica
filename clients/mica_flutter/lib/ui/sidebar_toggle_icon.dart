import 'package:flutter/material.dart';

/// The panel glyph used by the sidebar toggles: a rounded window outline with
/// a divider marking which edge the panel lives on.
///
/// Drawn rather than taken from Material, whose `view_sidebar` fills the panel
/// with a heavy block and reads as a different weight from the rest of the
/// outlined toolbar icons.
class SidebarToggleIcon extends StatelessWidget {
  const SidebarToggleIcon({super.key, this.right = false, this.size = 18});

  /// Panel on the right edge (the tools pane) instead of the left.
  final bool right;
  final double size;

  @override
  Widget build(BuildContext context) {
    final icon = IconTheme.of(context);
    return SizedBox.square(
      dimension: size,
      child: CustomPaint(
        painter: _PanelPainter(
          color: icon.color ?? Theme.of(context).colorScheme.onSurface,
          right: right,
        ),
      ),
    );
  }
}

class _PanelPainter extends CustomPainter {
  const _PanelPainter({required this.color, required this.right});

  final Color color;
  final bool right;

  @override
  void paint(Canvas canvas, Size size) {
    final stroke = size.width / 12;
    final paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = stroke
      ..strokeCap = StrokeCap.round;
    final frame = Rect.fromLTWH(
      size.width * 0.1,
      size.height * 0.16,
      size.width * 0.8,
      size.height * 0.68,
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(frame, Radius.circular(size.width * 0.17)),
      paint,
    );
    final x = right
        ? frame.right - frame.width * 0.34
        : frame.left + frame.width * 0.34;
    canvas.drawLine(Offset(x, frame.top), Offset(x, frame.bottom), paint);
  }

  @override
  bool shouldRepaint(_PanelPainter old) =>
      old.color != color || old.right != right;
}
