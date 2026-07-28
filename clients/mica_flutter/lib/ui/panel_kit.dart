/// Layout primitives the settings-style screens are built from: the small
/// tracked section label, the white card it sits above, and the dashed
/// choose-a-file zone.
///
/// Separate from `status_kit.dart` on purpose — that file is about *states*
/// (nothing here yet / this broke / this is progressing). These are plain
/// containers with no opinion about what is inside them.
///
/// **All copy arrives already localized** (same contract as `status_kit.dart`):
/// nothing here holds a user-facing string, so the arb files stay the single
/// source of wording.
library;

import 'package:flutter/material.dart';

import 'theme_tokens.dart';

/// Card border.
Color _cardBorder(BuildContext context) => MicaTheme.of(context).border.subtle;

/// Dashed pick-zone: a hair cooler than the card border, on an almost-white fill.
Color _zoneBorder(BuildContext context) => MicaTheme.of(context).border.normal;
Color _zoneFill(BuildContext context) => MicaTheme.of(context).surface.raised;

/// The small tracked label above a section.
///
/// Replaces a `titleMedium` + accent-coloured icon row. The point of the eyebrow
/// treatment is that it does NOT compete with the content underneath — a 16px
/// semibold title with a blue glyph reads as a heading of equal weight to
/// everything it labels.
class MicaEyebrow extends StatelessWidget {
  const MicaEyebrow(this.label, {this.icon, super.key});

  /// Already-localized label.
  final String label;

  /// Optional leading glyph, muted rather than accent-coloured.
  final IconData? icon;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        if (icon case final i?) ...[
          Icon(i, size: 13, color: MicaTheme.of(context).text.faint),
          const SizedBox(width: 6),
        ],
        Text(
          label,
          style: TextStyle(
            fontSize: 12,
            fontWeight: FontWeight.w600,
            letterSpacing: 0.6,
            color: MicaTheme.of(context).text.faint,
          ),
        ),
      ],
    );
  }
}

/// A white content card: hairline border, 14px radius, generous padding.
///
/// Radius is **14, not the mockups' 12** — the design system pins 8/14/16 and the
/// mockups violate their own ladder (recorded in `docs/design-adoption.md`).
class MicaCard extends StatelessWidget {
  const MicaCard({
    required this.child,
    this.padding = const EdgeInsets.all(18),
    super.key,
  });

  final Widget child;
  final EdgeInsetsGeometry padding;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      decoration: BoxDecoration(
        color: MicaTheme.of(context).surface.base,
        border: Border.all(color: _cardBorder(context)),
        borderRadius: BorderRadius.circular(14),
      ),
      padding: padding,
      child: child,
    );
  }
}

/// The dashed "choose a file" zone.
///
/// **A click target, not a drop target.** The mockup draws a drag-and-drop well
/// captioned 「把 .zip 或 .md 拖到这里」, but nothing in the app accepts a drop —
/// that needs a new dependency (`desktop_drop`) and a CLAUDE.md exemption.
/// Shipping the drag caption before the drop works would be a control that lies,
/// so this takes [onTap] and no drop handler, and the caller must pass copy that
/// says *choose*, not *drag*. Enforced by the API shape rather than by discipline:
/// there is nowhere to attach a drop listener.
class MicaPickZone extends StatelessWidget {
  const MicaPickZone({
    required this.icon,
    required this.title,
    required this.onTap,
    this.subtitle,
    super.key,
  });

  final IconData icon;

  /// Already-localized. Must describe choosing, not dragging.
  final String title;
  final String? subtitle;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      painter: _DashedBorderPainter(_zoneBorder(context)),
      child: Material(
        color: _zoneFill(context),
        borderRadius: BorderRadius.circular(14),
        child: InkWell(
          borderRadius: BorderRadius.circular(14),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 22),
            child: Column(
              children: [
                Container(
                  width: 44,
                  height: 44,
                  decoration: BoxDecoration(
                    color: MicaTheme.of(context).accent.wash,
                    borderRadius: BorderRadius.circular(12),
                  ),
                  alignment: Alignment.center,
                  child: Icon(
                    icon,
                    size: 20,
                    color: MicaTheme.of(context).accent.primary,
                  ),
                ),
                const SizedBox(height: 11),
                Text(
                  title,
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontSize: 14,
                    fontWeight: FontWeight.w600,
                    color: MicaTheme.of(context).text.primary,
                  ),
                ),
                if (subtitle case final s?) ...[
                  const SizedBox(height: 6),
                  Text(
                    s,
                    textAlign: TextAlign.center,
                    style: TextStyle(
                      fontSize: 12.5,
                      height: 1.6,
                      color: MicaTheme.of(context).text.faint,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// 1.5px dashed rounded rectangle.
///
/// Painted rather than composed: Flutter has no dashed `BorderSide`, and the
/// alternatives are a stack of clipped slivers or this. The painter is the one
/// that stays correct when the box is resized, and it walks the rounded outline so
/// the dashes follow the corners instead of breaking at them.
class _DashedBorderPainter extends CustomPainter {
  const _DashedBorderPainter(this.color);

  /// Passed in, not read here: a painter has no BuildContext, so the token
  /// lookup has to happen in the widget that builds it.
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final path = Path()
      ..addRRect(
        RRect.fromRectAndRadius(Offset.zero & size, const Radius.circular(14)),
      );
    final paint = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5
      ..color = color;

    const dash = 6.0;
    const gap = 4.0;
    for (final metric in path.computeMetrics()) {
      var distance = 0.0;
      while (distance < metric.length) {
        final end = (distance + dash).clamp(0.0, metric.length);
        canvas.drawPath(metric.extractPath(distance, end), paint);
        distance = end + gap;
      }
    }
  }

  @override
  bool shouldRepaint(_DashedBorderPainter oldDelegate) =>
      oldDelegate.color != color;
}
