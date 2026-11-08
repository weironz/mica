// Drag the page column's side edge to change the page width (gh#164).
//
// The column is centred, so moving one edge by d changes the width by 2d and
// both edges stay under the pointer's symmetry. The width snaps to the same
// 120 px steps as the Settings slider, and is saved through the same path on
// release (one stored value, not a second notion of page width). The Settings
// slider stays the precise / reset control; this is the direct one.
library;

import 'package:flutter/material.dart';

import 'theme_tokens.dart';

/// Editor page (content column) width, as 11 discrete steps like AppFlowy —
/// `680 · 800 · … · 1880` (min + max + 10 divisions, step 120). The DEFAULT and
/// the Settings "reset" both go to [kPageWidthDefault] = 800px, the readable
/// column width the note-app field converges on (Obsidian 700, Notion ~708,
/// AFFiNE ~720-800) — NOT AppFlowy's reset-to-full-width. See research 2026-07.
const double kPageWidthMin = 680;
const double kPageWidthMax = 1880;
const double kPageWidthDefault = 800;
const int kPageWidthDivisions = 10;

/// The width a drag lands on: the nearest Settings step inside the range.
double snapPageWidth(double raw) {
  const step = (kPageWidthMax - kPageWidthMin) / kPageWidthDivisions;
  final steps = ((raw - kPageWidthMin) / step).round();
  return (kPageWidthMin + steps * step).clamp(kPageWidthMin, kPageWidthMax);
}

/// [child] capped at [width], with a resize grip just outside each side edge.
/// Takes [grip] extra px on each side, so a host keeping its layout should
/// give up that much padding. [enabled] false (narrow panes, touch) renders
/// the plain capped column.
class PageWidthFrame extends StatefulWidget {
  const PageWidthFrame({
    required this.width,
    required this.onCommit,
    required this.child,
    this.enabled = true,
    super.key,
  });

  static const double grip = 12;

  final double width;
  final ValueChanged<double> onCommit;
  final Widget child;
  final bool enabled;

  @override
  State<PageWidthFrame> createState() => _PageWidthFrameState();
}

class _PageWidthFrameState extends State<PageWidthFrame> {
  /// The width while a drag is under way; null otherwise.
  double? _live;
  double _start = 0;
  double _dx = 0;
  int _hovered = 0; // which grip the pointer is over: -1 left, 1 right

  void _drag(int side, double delta) {
    _dx += delta;
    setState(() => _live = snapPageWidth(_start + 2 * side * _dx));
  }

  void _end() {
    final live = _live;
    if (live != null && live != widget.width) widget.onCommit(live);
    setState(() => _live = null);
  }

  Widget _grip(int side) {
    final active = _live != null || _hovered == side;
    return MouseRegion(
      cursor: SystemMouseCursors.resizeLeftRight,
      onEnter: (_) => setState(() => _hovered = side),
      onExit: (_) => setState(() => _hovered = 0),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onHorizontalDragStart: (_) {
          _start = widget.width;
          _dx = 0;
          setState(() => _live = widget.width);
        },
        onHorizontalDragUpdate: (d) => _drag(side, d.delta.dx),
        onHorizontalDragEnd: (_) => _end(),
        onHorizontalDragCancel: () => setState(() => _live = null),
        child: Center(
          child: Container(
            width: 2,
            color: active
                ? MicaTheme.of(context).accent.primary.withValues(alpha: 0.5)
                : Colors.transparent,
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final width = _live ?? widget.width;
    if (!widget.enabled) {
      return ConstrainedBox(
        constraints: BoxConstraints(maxWidth: width),
        child: widget.child,
      );
    }
    const g = PageWidthFrame.grip;
    final live = _live;
    return ConstrainedBox(
      constraints: BoxConstraints(maxWidth: width + 2 * g),
      child: Stack(
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: g),
            child: widget.child,
          ),
          Positioned(left: 0, top: 0, bottom: 0, width: g, child: _grip(-1)),
          Positioned(right: 0, top: 0, bottom: 0, width: g, child: _grip(1)),
          if (live != null)
            Positioned(
              top: 0,
              left: 0,
              right: 0,
              child: Center(
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    color: MicaTheme.of(context).surface.overlay,
                    borderRadius: BorderRadius.circular(6),
                  ),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 8,
                      vertical: 2,
                    ),
                    child: Text(
                      '${live.round()} px',
                      key: const ValueKey('page-width-readout'),
                      style: const TextStyle(fontSize: 12),
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
