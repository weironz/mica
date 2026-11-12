import 'package:flutter/material.dart';

/// Paints the specified shadow outside the existing Material surface only.
/// No wrapper/padding changes dialog or menu constraints, and the clipped-out
/// interior prevents the foreground shape painter from tinting text/content.
class MicaPanelShape extends RoundedRectangleBorder {
  const MicaPanelShape({required this.shadow, super.side, super.borderRadius});
  final BoxShadow shadow;

  @override
  void paint(Canvas canvas, Rect rect, {TextDirection? textDirection}) {
    final path = getOuterPath(rect, textDirection: textDirection);
    canvas.save();
    final extent =
        shadow.blurRadius * 3 +
        shadow.offset.distance +
        shadow.spreadRadius.abs() +
        4;
    canvas.clipPath(
      Path.combine(
        PathOperation.difference,
        Path()..addRect(rect.inflate(extent)),
        path,
      ),
    );
    canvas.drawPath(path.shift(shadow.offset), shadow.toPaint());
    canvas.restore();
    super.paint(canvas, rect, textDirection: textDirection);
  }

  @override
  MicaPanelShape copyWith({
    BorderSide? side,
    BorderRadiusGeometry? borderRadius,
    BoxShadow? shadow,
  }) => MicaPanelShape(
    side: side ?? this.side,
    borderRadius: borderRadius ?? this.borderRadius,
    shadow: shadow ?? this.shadow,
  );
  @override
  ShapeBorder scale(double t) => MicaPanelShape(
    side: side.scale(t),
    borderRadius: borderRadius * t,
    shadow: shadow.scale(t),
  );
  @override
  ShapeBorder? lerpFrom(ShapeBorder? a, double t) => a is MicaPanelShape
      ? MicaPanelShape(
          side: BorderSide.lerp(a.side, side, t),
          borderRadius: BorderRadiusGeometry.lerp(
            a.borderRadius,
            borderRadius,
            t,
          )!,
          shadow: BoxShadow.lerp(a.shadow, shadow, t)!,
        )
      : super.lerpFrom(a, t);
  @override
  ShapeBorder? lerpTo(ShapeBorder? b, double t) =>
      b is MicaPanelShape ? b.lerpFrom(this, t) : super.lerpTo(b, t);
  @override
  bool operator ==(Object other) =>
      other is MicaPanelShape &&
      other.side == side &&
      other.borderRadius == borderRadius &&
      other.shadow == shadow;
  @override
  int get hashCode => Object.hash(side, borderRadius, shadow);
}

/// The two-pixel wash stays outside the one-pixel focused outline. Its layout
/// dimensions remain those of OutlineInputBorder, including floating-label gaps.
class MicaFocusBorder extends OutlineInputBorder {
  const MicaFocusBorder({
    required this.wash,
    super.borderSide,
    super.borderRadius,
    super.gapPadding,
  });
  final Color wash;
  @override
  void paint(
    Canvas canvas,
    Rect rect, {
    double? gapStart,
    double gapExtent = 0,
    double gapPercentage = 0,
    TextDirection? textDirection,
  }) {
    OutlineInputBorder(
      borderRadius: borderRadius + BorderRadius.circular(2),
      borderSide: BorderSide(color: wash, width: 2),
      gapPadding: gapPadding,
    ).paint(
      canvas,
      rect.inflate(2),
      gapStart: gapStart == null ? null : gapStart + 2,
      gapExtent: gapExtent,
      gapPercentage: gapPercentage,
      textDirection: textDirection,
    );
    super.paint(
      canvas,
      rect,
      gapStart: gapStart,
      gapExtent: gapExtent,
      gapPercentage: gapPercentage,
      textDirection: textDirection,
    );
  }

  @override
  MicaFocusBorder copyWith({
    BorderSide? borderSide,
    BorderRadius? borderRadius,
    double? gapPadding,
    Color? wash,
  }) => MicaFocusBorder(
    wash: wash ?? this.wash,
    borderSide: borderSide ?? this.borderSide,
    borderRadius: borderRadius ?? this.borderRadius,
    gapPadding: gapPadding ?? this.gapPadding,
  );
}
