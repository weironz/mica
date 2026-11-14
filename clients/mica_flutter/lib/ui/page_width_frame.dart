// Drag the page column's side edge to change the page width (gh#164).
//
// The column is centred, so moving one edge by d changes the width by 2d and
// both edges stay under the pointer's symmetry. The width snaps to the same
// 120 px steps as the Settings slider, and is saved through the same path on
// release (one stored value, not a second notion of page width). The Settings
// slider stays the precise / reset control; this is the direct one.
//
// The top stop means "full width": the column takes whatever the pane leaves
// once the sidebars have theirs, so it keeps adapting as they open and close.
// A fixed 1880 used to sit there, and on an ordinary window the last four or
// five stops all rendered as the same full-width column.
library;

import 'dart:math' as math;

import 'typography.dart';

import 'package:flutter/material.dart';

import '../l10n/app_localizations.dart';
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

/// Whether [width] is the "full width" stop rather than a pixel width. The
/// stored value stays the number [kPageWidthMax], so prefs, sync and older
/// clients need no new vocabulary — an old 1880 simply reads as full width.
bool isFullPageWidth(double width) => width >= kPageWidthMax;

/// The cap to lay the page column out with.
double pageColumnCap(double width) =>
    isFullPageWidth(width) ? double.infinity : width;

/// The px width to export with: full width exports as wide as the column is
/// right now (the room [PageWidthFrame] last measured).
double pageWidthForExport(double width) => isFullPageWidth(width)
    ? (PageWidthFrame.room.value ?? kPageWidthMax)
    : width;

/// The Settings readout for the chosen stop.
String pageWidthLabel(BuildContext context, double width) =>
    isFullPageWidth(width)
    ? AppLocalizations.of(context).settingsPageWidthFull
    : '${width.round()} px';

/// What this window actually draws, when that is not the chosen stop (or the
/// stop is full width, which has no fixed figure). Null when they agree or no
/// page is open. [room] is [PageWidthFrame.room].
String? pageWidthShownNote(BuildContext context, double width, double? room) {
  if (room == null) return null;
  final shown = math.min(pageColumnCap(width), room);
  if (!isFullPageWidth(width) && shown >= width) return null;
  return AppLocalizations.of(context).settingsPageWidthShown(shown.round());
}

const double _step = (kPageWidthMax - kPageWidthMin) / kPageWidthDivisions;

/// The width a drag lands on: the nearest Settings step inside the range.
double snapPageWidth(double raw) {
  final steps = ((raw - kPageWidthMin) / _step).round();
  return (kPageWidthMin + steps * _step).clamp(kPageWidthMin, kPageWidthMax);
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

  /// The widest the column can currently be drawn: the pane's width after the
  /// sidebars and the page margins. Null while no page is open. Settings reads
  /// it to show which stops this window can actually reach.
  static final ValueNotifier<double?> room = ValueNotifier(null);
  static Object? _roomOwner;

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
  double _room = double.infinity;
  int _hovered = 0; // which grip the pointer is over: -1 left, 1 right

  @override
  void dispose() {
    if (PageWidthFrame._roomOwner == this) {
      // Not synchronously: listeners may rebuild, and the tree is locked while
      // states are disposed. A frame mounted meanwhile takes ownership first.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (PageWidthFrame._roomOwner != this) return;
        PageWidthFrame._roomOwner = null;
        PageWidthFrame.room.value = null;
      });
    }
    super.dispose();
  }

  void _publish(double room) {
    _room = room;
    if (!room.isFinite) return;
    if (PageWidthFrame._roomOwner == this &&
        PageWidthFrame.room.value == room) {
      return;
    }
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      PageWidthFrame._roomOwner = this;
      PageWidthFrame.room.value = _room;
    });
  }

  /// The width actually drawn. A drag starts from here, not from the stored
  /// value: with 1520 stored and 1380 of room, starting at 1520 meant the
  /// first 70 px of an inward drag moved nothing.
  double _shown(double width) => math.min(pageColumnCap(width), _room);

  void _drag(int side, double delta) {
    _dx += delta;
    final raw = _start + 2 * side * _dx;
    final double live;
    if ((raw - _start).abs() < _step / 2) {
      live = widget.width;
    } else if (raw >= _room) {
      // Past what the pane can show, a wider number would look the same;
      // pulling to the edge means "as wide as it goes".
      live = kPageWidthMax;
    } else {
      live = snapPageWidth(raw);
    }
    setState(() => _live = live);
  }

  void _end() {
    final live = _live;
    if (live != null && live != widget.width) widget.onCommit(live);
    setState(() => _live = null);
  }

  String _readout(double live) {
    final px = '${_shown(live).round()} px';
    if (!isFullPageWidth(live)) return px;
    final l10n = Localizations.of<AppLocalizations>(context, AppLocalizations);
    return '${l10n?.settingsPageWidthFull ?? 'Full width'} · $px';
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
          _start = _shown(widget.width);
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
                ? MicaTheme.of(context).border.strong
                : Colors.transparent,
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        const g = PageWidthFrame.grip;
        _publish(
          widget.enabled ? constraints.maxWidth - 2 * g : constraints.maxWidth,
        );
        return _frame(context);
      },
    );
  }

  Widget _frame(BuildContext context) {
    final cap = pageColumnCap(_live ?? widget.width);
    if (!widget.enabled) {
      return ConstrainedBox(
        constraints: BoxConstraints(maxWidth: cap),
        child: widget.child,
      );
    }
    const g = PageWidthFrame.grip;
    final live = _live;
    return ConstrainedBox(
      constraints: BoxConstraints(maxWidth: cap + 2 * g),
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
                      _readout(live),
                      key: const ValueKey('page-width-readout'),
                      style: MicaTypography.caption,
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
