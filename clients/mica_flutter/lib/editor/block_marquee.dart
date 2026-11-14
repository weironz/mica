import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../ui/autoscroll.dart';
import '../ui/theme_tokens.dart';
import 'controller.dart';
import 'model.dart';
import 'render.dart';

/// Bridges a marquee drawn in the page's blank margins to the editor under it.
class EditorMarqueeHook {
  Object? _owner;
  void Function(Rect global)? _onSelect;
  VoidCallback? _onEnd;
  VoidCallback? _onClear;
  double? Function()? _contentBottom;
  bool Function(Offset global)? _inGutter;

  bool get isBound => _owner != null;

  /// The region took the current press; the editor must not also start a
  /// text drag from it (the blank below the last block is inside the editor).
  bool claimed = false;

  /// Global y where the last block ends, or null when unknown.
  double? contentBottom() => _contentBottom?.call();

  /// Whether [global] is in the editor's handle gutter but not on a handle.
  bool inGutter(Offset global) => _inGutter?.call(global) ?? false;

  void bind(
    Object owner, {
    required void Function(Rect global) onSelect,
    required VoidCallback onEnd,
    required VoidCallback onClear,
    required double? Function() contentBottom,
    required bool Function(Offset global) inGutter,
  }) {
    _owner = owner;
    _contentBottom = contentBottom;
    _inGutter = inGutter;
    _onSelect = onSelect;
    _onEnd = onEnd;
    _onClear = onClear;
  }

  /// A replaced editor is disposed after its successor has bound, so only the
  /// current owner may clear the binding.
  void unbind(Object owner) {
    if (!identical(_owner, owner)) return;
    _owner = null;
    _contentBottom = null;
    _inGutter = null;
    _onSelect = null;
    _onEnd = null;
    _onClear = null;
  }

  void select(Rect global) => _onSelect?.call(global);
  void end() => _onEnd?.call();
  void clear() => _onClear?.call();
}

/// Observes the page's blank space — beside the page column, and below the
/// body — and turns a mouse drag that starts there into a marquee rectangle.
///
/// A [Listener], so it never competes in the gesture arena with the editor or
/// the page-width grips. The rectangle's origin is pinned to the content, so
/// it stretches correctly while the page auto-scrolls under it.
class EditorMarqueeRegion extends StatefulWidget {
  const EditorMarqueeRegion({
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
  final EditorMarqueeHook hook;
  final VoidCallback onOutsidePointerDown;
  final Widget child;

  @override
  State<EditorMarqueeRegion> createState() => _EditorMarqueeRegionState();
}

class _EditorMarqueeRegionState extends State<EditorMarqueeRegion> {
  static const double _slop = 4;
  static const double _scrollbarStrip = 16;

  int? _pointer;
  Offset? _origin;
  double _originScroll = 0;
  Offset? _current;
  bool _dragging = false;
  Rect? _paintRect;
  Timer? _autoScroll;

  @override
  void dispose() {
    _autoScroll?.cancel();
    super.dispose();
  }

  Rect? _globalRect(BuildContext? context) {
    final object = context?.findRenderObject();
    if (object is! RenderBox || !object.attached || !object.hasSize) {
      return null;
    }
    return object.localToGlobal(Offset.zero) & object.size;
  }

  ScrollableState? get _scrollable {
    final context = widget.editorSurfaceKey.currentContext;
    return context == null ? null : Scrollable.maybeOf(context);
  }

  bool _startsMarquee(Offset p, Rect body, Rect? page) {
    final region = _globalRect(context);
    if (page == null || region == null) return false;
    // The page's scrollbar hugs the pane's right edge; dragging its thumb
    // must only scroll.
    if (p.dx > region.right - _scrollbarStrip) return false;
    // Left of the text is mostly the editor's own handle gutter: it looks like
    // margin, so it starts a marquee too — except on a block's drag handle.
    if (p.dy >= body.top && widget.hook.inGutter(p)) return true;
    // Beside the page at any height, the title rows included: a marquee is
    // usually drawn from the top-left corner down.
    if (p.dx < page.left || p.dx > page.right) return true;
    final onGrip = p.dx < body.left || p.dx > body.right;
    return !onGrip && p.dy >= (widget.hook.contentBottom() ?? body.bottom);
  }

  void _down(PointerDownEvent event) {
    final body = _globalRect(widget.editorSurfaceKey.currentContext);
    if (body == null) return;
    if (!body.contains(event.position)) widget.onOutsidePointerDown();
    if (!widget.enabled ||
        _pointer != null ||
        !widget.hook.isBound ||
        event.kind != PointerDeviceKind.mouse ||
        event.buttons & kPrimaryButton == 0 ||
        !_startsMarquee(
          event.position,
          body,
          _globalRect(widget.pageFrameKey.currentContext),
        )) {
      return;
    }
    _pointer = event.pointer;
    widget.hook.claimed = true;
    _origin = event.position;
    _current = event.position;
    _originScroll = _scrollable?.position.pixels ?? 0;
  }

  void _move(PointerMoveEvent event) {
    if (event.pointer != _pointer) return;
    _current = event.position;
    if (!_dragging) {
      if ((event.position - _origin!).distance < _slop) return;
      _dragging = true;
    }
    _update();
    _autoScroll ??= Timer.periodic(
      const Duration(milliseconds: 16),
      (_) => _tick(),
    );
  }

  void _up(PointerUpEvent event) {
    if (event.pointer != _pointer) return;
    if (_dragging) {
      _current = event.position;
      _update();
      widget.hook.end();
    } else {
      widget.hook.clear();
    }
    _reset();
  }

  void _cancel(PointerCancelEvent event) {
    if (event.pointer != _pointer) return;
    if (_dragging) widget.hook.end();
    _reset();
  }

  void _reset() {
    _autoScroll?.cancel();
    _autoScroll = null;
    _pointer = null;
    widget.hook.claimed = false;
    _origin = null;
    _current = null;
    _dragging = false;
    if (_paintRect != null) setState(() => _paintRect = null);
  }

  /// The marquee in global coordinates, its origin carried along by however
  /// far the page has scrolled since the press.
  Rect? get _marquee {
    final origin = _origin;
    final current = _current;
    if (!_dragging || origin == null || current == null) return null;
    final scrolled =
        (_scrollable?.position.pixels ?? _originScroll) - _originScroll;
    return Rect.fromPoints(origin.translate(0, -scrolled), current);
  }

  void _update() {
    final rect = _marquee;
    final box = context.findRenderObject();
    if (rect == null || box is! RenderBox || !box.hasSize) return;
    widget.hook.select(rect);
    setState(() {
      _paintRect = Rect.fromPoints(
        box.globalToLocal(rect.topLeft),
        box.globalToLocal(rect.bottomRight),
      );
    });
  }

  void _tick() {
    final scrollable = _scrollable;
    final current = _current;
    final viewport = scrollable?.context.findRenderObject();
    if (!mounted ||
        scrollable == null ||
        current == null ||
        viewport is! RenderBox ||
        !viewport.hasSize) {
      return;
    }
    final velocity = edgeAutoScrollVelocity(
      viewport.globalToLocal(current).dy,
      viewport.size.height,
    );
    if (velocity == 0) return;
    final position = scrollable.position;
    final next = (position.pixels + velocity).clamp(
      position.minScrollExtent,
      position.maxScrollExtent,
    );
    if (next == position.pixels) return;
    position.jumpTo(next);
    _update();
  }

  @override
  Widget build(BuildContext context) {
    final rect = _paintRect;
    final tint = MicaTheme.of(context).editor.selection;
    return Listener(
      behavior: HitTestBehavior.translucent,
      onPointerDown: _down,
      onPointerMove: _move,
      onPointerUp: _up,
      onPointerCancel: _cancel,
      child: Stack(
        fit: StackFit.passthrough,
        children: [
          widget.child,
          if (rect != null)
            Positioned.fromRect(
              rect: rect,
              child: IgnorePointer(
                child: DecoratedBox(
                  key: const ValueKey('block-marquee-rect'),
                  decoration: BoxDecoration(
                    color: tint.withValues(alpha: tint.a * 0.5),
                    border: Border.all(
                      color: tint.withValues(alpha: (tint.a * 3).clamp(0, 1)),
                    ),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// The editor half of the marquee: the whole blocks the rectangle covers.
///
/// The blocks are also written into the controller as one document range, so
/// copy reuses the normal clipboard path. Any selection or content change that
/// did not come from here ends block mode.
class BlockMarqueeSelection extends ChangeNotifier {
  BlockMarqueeSelection(this._controller, this._render) {
    _controller.addListener(_sync);
  }

  final EditorController _controller;
  final RenderDocument? Function() _render;
  ({int lo, int hi})? _range;
  DocSelection? _applied;
  int _revision = -1;
  bool _applying = false;

  ({int lo, int hi})? get range => _range;
  bool get active => _range != null;

  double? contentBottom() {
    final r = _render();
    if (r == null || !r.attached || !r.hasSize) return null;
    for (var i = r.blockCount - 1; i >= 0; i--) {
      final box = r.blockBox(i);
      if (box != null) return r.localToGlobal(box.bottomLeft).dy;
    }
    return null;
  }

  bool inGutter(Offset global) {
    final r = _render();
    if (r == null || !r.attached || !r.hasSize) return false;
    final local = r.globalToLocal(global);
    return local.dx >= 0 &&
        local.dx < EditorTheme.gutter &&
        local.dy >= 0 &&
        local.dy < r.size.height &&
        r.dragHandleAt(local) == null;
  }

  @override
  void dispose() {
    _controller.removeListener(_sync);
    super.dispose();
  }

  void selectRect(Rect global) {
    final r = _render();
    if (r == null || !r.attached || !r.hasSize) return;
    final local = Rect.fromPoints(
      r.globalToLocal(global.topLeft),
      r.globalToLocal(global.bottomRight),
    );
    int? lo;
    var hi = 0;
    for (var i = 0; i < r.blockCount; i++) {
      final box = r.blockBox(i);
      if (box == null || !box.overlaps(local)) continue;
      lo ??= i;
      hi = i;
    }
    final next = lo == null ? null : (lo: lo, hi: hi);
    if (next == _range && next != null) return;
    _range = next;
    _apply(
      next == null
          ? null
          : DocSelection(
              anchor: DocPosition(next.lo, 0),
              focus: DocPosition(
                next.hi,
                _controller.nodes[next.hi].text.length,
              ),
            ),
    );
    notifyListeners();
  }

  void clear() {
    final range = _range;
    if (range == null) return;
    _range = null;
    _controller.setSelection(DocSelection.collapsed(DocPosition(range.lo, 0)));
    notifyListeners();
  }

  /// Block-mode keys: Escape leaves it, Delete / Backspace / cut remove the
  /// blocks whole. Null lets every other key fall through to the editor.
  KeyEventResult? handleKey(KeyEvent event, {required bool Function() copy}) {
    final range = _range;
    if (range == null) return null;
    final key = event.logicalKey;
    final keyboard = HardwareKeyboard.instance;
    final accel = keyboard.isControlPressed || keyboard.isMetaPressed;
    if (key == LogicalKeyboardKey.escape) {
      clear();
      return KeyEventResult.handled;
    }
    if (accel && key == LogicalKeyboardKey.keyC) {
      copy();
      return KeyEventResult.handled;
    }
    final cut = accel && key == LogicalKeyboardKey.keyX;
    if (cut && !copy()) return KeyEventResult.handled;
    if (cut ||
        key == LogicalKeyboardKey.delete ||
        key == LogicalKeyboardKey.backspace) {
      _range = null;
      _controller.removeBlocks(range.lo, range.hi);
      notifyListeners();
      return KeyEventResult.handled;
    }
    return null;
  }

  void _apply(DocSelection? selection) {
    _applying = true;
    _controller.setSelection(selection);
    _applying = false;
    _applied = _controller.selection;
    _revision = _controller.contentRevision;
  }

  void _sync() {
    if (_applying || _range == null) return;
    if (_controller.selection == _applied &&
        _controller.contentRevision == _revision) {
      return;
    }
    _range = null;
    notifyListeners();
  }
}

/// Paints the whole-block tint over every block in [selection]'s range.
class BlockMarqueeHighlight extends StatelessWidget {
  const BlockMarqueeHighlight(
    this.selection,
    this.render,
    this.color, {
    super.key,
  });

  final BlockMarqueeSelection selection;
  final RenderDocument? Function() render;
  final Color color;

  @override
  Widget build(BuildContext context) => IgnorePointer(
    child: CustomPaint(painter: _HighlightPainter(selection, render, color)),
  );
}

class _HighlightPainter extends CustomPainter {
  _HighlightPainter(this.selection, this.render, this.color)
    : super(repaint: selection);

  final BlockMarqueeSelection selection;
  final RenderDocument? Function() render;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final range = selection.range;
    final r = render();
    if (range == null || r == null || !r.hasSize) return;
    final paint = Paint()..color = color;
    for (var i = range.lo; i <= range.hi && i < r.blockCount; i++) {
      final box = r.blockBox(i);
      if (box == null) continue;
      canvas.drawRRect(
        RRect.fromRectAndRadius(box.inflate(2), const Radius.circular(6)),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(_HighlightPainter old) =>
      old.selection != selection || old.color != color;
}
