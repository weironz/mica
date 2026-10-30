// Keep an overlay on a spot inside the editor canvas — without a follower
// layer, so the overlay can hold tooltips.
//
// The bars that belong to a spot in the document (format bar, link bar, slash
// menu, table cell editor) live in the app Overlay but must move with the
// canvas when the host scrolls, and the editor does not own that scrollable.
// They used `CompositedTransformFollower` for this, which tracks the target
// through any transform — but only at COMPOSITING time. A `Tooltip` is an
// `OverlayPortal` that asks its anchor for a paint transform during LAYOUT,
// meets the follower layer with no transform yet, and asserts. So the bars ran
// with tooltips switched off (2026-08-12).
//
// Flutter's own advice (#188894) is `OverlayPortal.overlayChildLayoutBuilder`,
// which places the overlay during layout from the anchor's paint transform.
// That means rebuilding the four bars around OverlayPortal controllers instead
// of the OverlayEntry lifecycle they have. This takes the same idea at a
// smaller cost: the bar keeps its OverlayEntry, and this render object reads
// where the canvas is at PAINT time — every layout of the frame is done by
// then, scrolling included — and paints the bar there. It repaints on each
// frame while mounted, so a scroll that never rebuilds the bar still moves
// it. Nothing between the bar and the Overlay is a follower layer, so a
// tooltip's layout-time transform query just works.
//
// The one approximation: a tooltip measured during layout sees the offset of
// the PREVIOUS paint. A tooltip appears on hover, after the bar has painted,
// so it lands in the right place.
import 'package:flutter/rendering.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

/// Paints [child] at [offset] in the coordinates of the box [target] returns.
/// Renders nothing while [target] gives no attached box.
class CanvasFollower extends SingleChildRenderObjectWidget {
  const CanvasFollower({
    super.key,
    required this.target,
    required this.offset,
    super.child,
  });

  final RenderBox? Function() target;
  final Offset offset;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      RenderCanvasFollower(target: target, offset: offset);

  @override
  void updateRenderObject(
    BuildContext context,
    RenderCanvasFollower renderObject,
  ) {
    renderObject
      ..target = target
      ..offset = offset;
  }
}

class RenderCanvasFollower extends RenderProxyBox {
  RenderCanvasFollower({
    required RenderBox? Function() target,
    required Offset offset,
  }) : _target = target,
       _offset = offset;

  RenderBox? Function() _target;
  set target(RenderBox? Function() value) {
    _target = value;
    markNeedsPaint();
  }

  Offset _offset;
  set offset(Offset value) {
    if (value == _offset) return;
    _offset = value;
    markNeedsPaint();
  }

  /// Where the child was last painted, relative to this box. Null while the
  /// target is missing (nothing painted, nothing hit).
  Offset? _shift;

  int? _frameCallback;

  @override
  bool get isRepaintBoundary => true;

  @override
  void attach(PipelineOwner owner) {
    super.attach(owner);
    _scheduleRepaint();
  }

  @override
  void detach() {
    final id = _frameCallback;
    if (id != null) SchedulerBinding.instance.cancelFrameCallbackWithId(id);
    _frameCallback = null;
    super.detach();
  }

  /// Repaint on every frame that happens anyway (never schedules one): the
  /// canvas can move without anything here changing.
  void _scheduleRepaint() {
    _frameCallback = SchedulerBinding.instance.scheduleFrameCallback((_) {
      _frameCallback = null;
      if (!attached) return;
      markNeedsPaint();
      _scheduleRepaint();
    }, scheduleNewFrame: false);
  }

  Offset? _computeShift() {
    final target = _target();
    if (target == null || !target.attached || !target.hasSize) return null;
    final here = localToGlobal(Offset.zero);
    return target.localToGlobal(_offset) - here;
  }

  @override
  void paint(PaintingContext context, Offset offset) {
    _shift = _computeShift();
    final child = this.child;
    final shift = _shift;
    if (child == null || shift == null) return;
    context.paintChild(child, offset + shift);
  }

  @override
  void applyPaintTransform(RenderBox child, Matrix4 transform) {
    final shift = _shift ?? _computeShift() ?? Offset.zero;
    transform.translateByDouble(shift.dx, shift.dy, 0, 1);
  }

  /// The child is painted outside this box's own bounds, so the usual
  /// "is the point inside me" test would drop every tap on it.
  @override
  bool hitTest(BoxHitTestResult result, {required Offset position}) {
    final child = this.child;
    final shift = _shift;
    if (child == null || shift == null) return false;
    return result.addWithPaintOffset(
      offset: shift,
      position: position,
      hitTest: (result, transformed) =>
          child.hitTest(result, position: transformed),
    );
  }
}
