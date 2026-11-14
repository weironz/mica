import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

/// Bridges mouse drags that begin outside the page column back into its editor.
class EditorMarginSelectionHook {
  void Function(Offset)? _onStart;
  void Function(Offset, Offset)? _onUpdate;
  void Function(Offset)? _onEnd;
  VoidCallback? _onCancel;
  int? _pointer;
  Offset? _lastPosition;

  void bind({
    required void Function(Offset) onStart,
    required void Function(Offset, Offset) onUpdate,
    required void Function(Offset) onEnd,
    required VoidCallback onCancel,
  }) {
    _onStart = onStart;
    _onUpdate = onUpdate;
    _onEnd = onEnd;
    _onCancel = onCancel;
  }

  void unbind() {
    _pointer = null;
    _lastPosition = null;
    _onStart = null;
    _onUpdate = null;
    _onEnd = null;
    _onCancel = null;
  }

  void pointerDown(
    PointerDownEvent event, {
    required bool enabled,
    required Rect? editorRect,
    required Rect? pageRect,
  }) {
    if (!enabled ||
        _pointer != null ||
        _onStart == null ||
        event.kind != PointerDeviceKind.mouse ||
        event.buttons & kPrimaryButton == 0 ||
        editorRect == null ||
        pageRect == null) {
      return;
    }
    final position = event.position;
    if (position.dy < editorRect.top ||
        position.dy >= editorRect.bottom ||
        (position.dx >= pageRect.left && position.dx <= pageRect.right)) {
      return;
    }
    _pointer = event.pointer;
    _lastPosition = position;
    _onStart!(position);
  }

  void pointerMove(PointerMoveEvent event) {
    if (_pointer != event.pointer) return;
    if (event.buttons & kPrimaryButton == 0) {
      _finish(event.position);
      return;
    }
    final previous = _lastPosition ?? event.position;
    _lastPosition = event.position;
    _onUpdate?.call(event.position, event.position - previous);
  }

  void pointerUp(PointerUpEvent event) {
    if (_pointer != event.pointer) return;
    final previous = _lastPosition ?? event.position;
    _onUpdate?.call(event.position, event.position - previous);
    _finish(event.position);
  }

  void pointerCancel(PointerCancelEvent event) {
    if (_pointer != event.pointer) return;
    _pointer = null;
    _lastPosition = null;
    _onCancel?.call();
  }

  void _finish(Offset position) {
    _pointer = null;
    _lastPosition = null;
    _onEnd?.call(position);
  }
}

/// Observes the page's side margins and forwards mouse drags to its editor.
class EditorMarginSelectionRegion extends StatelessWidget {
  const EditorMarginSelectionRegion({
    required this.enabled,
    required this.editorSurfaceKey,
    required this.pageFrameKey,
    required this.hook,
    required this.onOutsidePointerDown,
    required this.child,
    super.key,
  });

  final bool enabled;
  final GlobalKey editorSurfaceKey;
  final GlobalKey pageFrameKey;
  final EditorMarginSelectionHook hook;
  final VoidCallback onOutsidePointerDown;
  final Widget child;

  Rect? _globalRect(GlobalKey key) {
    final object = key.currentContext?.findRenderObject();
    if (object is! RenderBox || !object.attached) return null;
    return Rect.fromPoints(
      object.localToGlobal(Offset.zero),
      object.localToGlobal(Offset(object.size.width, object.size.height)),
    );
  }

  @override
  Widget build(BuildContext context) => Listener(
    behavior: HitTestBehavior.translucent,
    onPointerDown: (event) {
      final editorRect = _globalRect(editorSurfaceKey);
      if (editorRect == null) return;
      if (!editorRect.contains(event.position)) onOutsidePointerDown();
      hook.pointerDown(
        event,
        enabled: enabled,
        editorRect: editorRect,
        pageRect: _globalRect(pageFrameKey),
      );
    },
    onPointerMove: hook.pointerMove,
    onPointerUp: hook.pointerUp,
    onPointerCancel: hook.pointerCancel,
    child: child,
  );
}
