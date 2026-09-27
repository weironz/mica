import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

/// Navigation rows scroll on touch; holding a row starts a reorder drag.
/// Choose by the actual pointer, so a touchscreen laptop still supports both
/// touch scrolling and immediate mouse dragging.
class NavigationDraggable<T extends Object> extends Draggable<T> {
  const NavigationDraggable({
    super.key,
    required super.child,
    required super.feedback,
    super.data,
    super.childWhenDragging,
    super.dragAnchorStrategy,
    super.onDragStarted,
    super.onDragEnd,
    super.onDraggableCanceled,
    super.onDragCompleted,
  });

  @override
  MultiDragGestureRecognizer createRecognizer(
    GestureMultiDragStartCallback onStart,
  ) => _NavigationDragRecognizer()..onStart = onStart;
}

class _NavigationDragRecognizer extends DelayedMultiDragGestureRecognizer {
  final _mouse = ImmediateMultiDragGestureRecognizer();

  @override
  void addAllowedPointer(PointerDownEvent event) {
    if (event.kind == PointerDeviceKind.mouse) {
      _mouse
        ..gestureSettings = gestureSettings
        ..onStart = onStart
        ..addPointer(event);
    } else {
      super.addAllowedPointer(event);
    }
  }

  @override
  void dispose() {
    _mouse.dispose();
    super.dispose();
  }
}
