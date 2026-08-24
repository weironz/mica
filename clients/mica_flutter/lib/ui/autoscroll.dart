import 'dart:async';

import 'package:flutter/widgets.dart';

/// Edge auto-scroll velocity for a drag inside a scrollable.
///
/// Given the pointer's vertical position [dy] within a viewport of
/// [viewportHeight], returns pixels-per-tick to scroll: negative near the top
/// (scroll up, reveal earlier content), positive near the bottom, zero in the
/// slack middle. Speed ramps linearly with how far into the [zone] the pointer
/// has pushed, so a small overshoot creeps and a hard push races.
///
/// Pure and framework-free so the direction/zone/clamp rules can be tested
/// without a live drag; the widget wires it to a ticking timer + ScrollController.
double edgeAutoScrollVelocity(
  double dy,
  double viewportHeight, {
  double zone = 60,
  double maxStep = 14,
}) {
  // A viewport shorter than two zones would make the top and bottom bands
  // overlap; clamp the effective zone so the midpoint stays neutral.
  final effectiveZone = zone.clamp(0.0, viewportHeight / 2);
  if (effectiveZone <= 0) {
    return 0;
  }
  if (dy < effectiveZone) {
    final depth = ((effectiveZone - dy) / effectiveZone).clamp(0.0, 1.0);
    return -maxStep * depth;
  }
  final bottomEdge = viewportHeight - effectiveZone;
  if (dy > bottomEdge) {
    final depth = ((dy - bottomEdge) / effectiveZone).clamp(0.0, 1.0);
    return maxStep * depth;
  }
  return 0;
}

/// Wraps a scrollable whose items can be dragged, and scrolls it while a drag
/// rests near its top/bottom edge.
///
/// Reads the pointer from the RAW event stream of this region, NOT from
/// `Draggable.onDragUpdate`. That distinction is the whole point of this
/// widget:
///
/// `Draggable` guards `onDragUpdate` with `if (mounted)` — its own
/// documentation says the callback "will only be called while this widget is
/// still mounted". Auto-scrolling a list drags the grabbed row out of the
/// viewport, the list recycles it, and its `Draggable` unmounts. From that
/// moment the position updates STOP, while the pointer is still down and still
/// in the edge zone. Anything driven by that callback then keeps scrolling on
/// the last position it ever saw, all the way to the end, and moving the
/// pointer back to the middle cannot stop it — nothing is listening any more.
/// It only stops on release, because `onDraggableCanceled` is NOT
/// mounted-guarded.
///
/// A [Listener] here survives all of that: pointer routing is fixed when the
/// pointer goes DOWN, so every move for that pointer keeps arriving at this
/// region's render object for as long as the REGION is mounted — no matter
/// which child unmounted, and even once the pointer has left the region.
class DragAutoScrollRegion extends StatefulWidget {
  const DragAutoScrollRegion({
    super.key,
    required this.enabled,
    required this.controller,
    required this.child,
    this.zone = 60,
    this.maxStep = 14,
    this.tick = const Duration(milliseconds: 16),
  });

  /// A drag is in progress. Pointer moves are ignored otherwise, so an ordinary
  /// press-and-move over the list (a scrollbar drag, a text selection) does not
  /// start scrolling on its own.
  final bool enabled;
  final ScrollController controller;
  final Widget child;
  final double zone;
  final double maxStep;
  final Duration tick;

  @override
  State<DragAutoScrollRegion> createState() => _DragAutoScrollRegionState();
}

class _DragAutoScrollRegionState extends State<DragAutoScrollRegion> {
  Timer? _timer;
  Offset? _pointer;

  @override
  void didUpdateWidget(DragAutoScrollRegion old) {
    super.didUpdateWidget(old);
    if (!widget.enabled) {
      _stop();
    } else if (!old.enabled) {
      // The move that STARTS a drag arrives while `enabled` is still false —
      // `onDragStarted` runs during that same event, so this widget has not
      // rebuilt yet. Waiting for the next move to notice means a drag that
      // begins already inside the edge zone sits there doing nothing until the
      // hand twitches again.
      _ensureTimer();
    }
  }

  @override
  void dispose() {
    _stop();
    super.dispose();
  }

  void _stop() {
    _timer?.cancel();
    _timer = null;
    // Cleared with the timer: a stale pointer from the previous drag would let
    // the next one start scrolling before it had moved anywhere.
    _pointer = null;
  }

  void _ensureTimer() {
    if (_pointer == null) return;
    _timer ??= Timer.periodic(widget.tick, (_) => _tick());
  }

  void _onMove(PointerMoveEvent event) {
    // Recorded even when no drag is running, so the drag-start rebuild has a
    // position to work from (see didUpdateWidget).
    //
    // Stores WHERE the pointer is, not how fast that made us scroll once: the
    // tick re-derives velocity from the live position, so resting outside the
    // edge zone stops on the next frame even with no further events.
    _pointer = event.position;
    if (widget.enabled) _ensureTimer();
  }

  void _tick() {
    final box = context.findRenderObject() as RenderBox?;
    final pointer = _pointer;
    if (!widget.enabled ||
        box == null ||
        !box.hasSize ||
        pointer == null ||
        !widget.controller.hasClients) {
      _stop();
      return;
    }
    final velocity = edgeAutoScrollVelocity(
      box.globalToLocal(pointer).dy,
      box.size.height,
      zone: widget.zone,
      maxStep: widget.maxStep,
    );
    if (velocity == 0) {
      _stop();
      return;
    }
    final position = widget.controller.position;
    final next = (position.pixels + velocity).clamp(
      position.minScrollExtent,
      position.maxScrollExtent,
    );
    // Already at an end: keep the timer, the pointer may come back off the edge.
    if (next != position.pixels) widget.controller.jumpTo(next);
  }

  @override
  Widget build(BuildContext context) {
    return Listener(onPointerMove: _onMove, child: widget.child);
  }
}
