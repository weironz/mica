// Finger text selection: what a double-tap / long-press grabs, and the two
// drag handles + the cut/copy/paste/select-all menu shown over a selection
// made by touch (instead of the mouse's formatting bar).
//
// The editor owns the gestures and the selection; this file only holds the
// pure word rule and the overlay widgets, so the editor library stays inside
// its size ratchet and the rule is testable without a widget tree.
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import 'model.dart' show DocPosition, wordBoundsAt;

/// An ideographic grapheme: CJK ideographs (incl. compatibility and, via
/// their high surrogate, Ext-B and beyond), kana, hangul. These are the
/// scripts written without spaces, where "the word" has no boundary to find.
bool _wide(String g) {
  final c = g.codeUnitAt(0);
  return (c >= 0x3400 && c <= 0x9FFF) ||
      (c >= 0x3040 && c <= 0x30FF) ||
      (c >= 0xAC00 && c <= 0xD7A3) ||
      (c >= 0xF900 && c <= 0xFAFF) ||
      (c >= 0xD840 && c <= 0xD87F);
}

/// Punctuation and symbols outside ASCII (，。、“”…—（）【】《》 and the
/// full-width forms): selected on their own, never joined to a neighbour.
bool _mark(String g) {
  final c = g.codeUnitAt(0);
  return (c >= 0x2000 && c <= 0x206F) ||
      (c >= 0x3000 && c <= 0x303F) ||
      (c >= 0xFE30 && c <= 0xFE4F) ||
      (c >= 0xFF00 && c <= 0xFF0F) ||
      (c >= 0xFF1A && c <= 0xFF20) ||
      (c >= 0xFF3B && c <= 0xFF40) ||
      (c >= 0xFF5B && c <= 0xFF65);
}

/// What a finger's double-tap or long-press selects around [glyph], the
/// offset of the character UNDER the finger (not the nearest caret boundary,
/// which is half a glyph off either way).
///
/// Latin words and numbers: the whole word, as with the mouse. Runs of CJK
/// (and other wide script): never the whole run, which reached to the next
/// punctuation mark and so selected a different amount from one tap to the
/// next. A finger gets the character it touched plus the one after it — or
/// the one before it at the end of a run, or just that one character when it
/// stands alone. Always one or two characters, decided by the text alone.
///
/// Grapheme-safe: an emoji or a CJK Ext-B character is never split. Returns
/// `(start, end)`; `start == end` means nothing to select.
(int, int) touchWordBounds(String text, int glyph) {
  if (text.isEmpty) return (0, 0);
  final g = glyph.clamp(0, text.length - 1);
  final graphemes = text.characters.toList();
  // Index of the grapheme that holds code unit g, and the offsets around it.
  var at = 0, start = 0;
  for (; at < graphemes.length; at++) {
    final end = start + graphemes[at].length;
    if (g < end) break;
    start = end;
  }
  if (at >= graphemes.length) return wordBoundsAt(text, g);
  final len = graphemes[at].length;
  if (_mark(graphemes[at])) return (start, start + len);
  if (!_wide(graphemes[at])) return wordBoundsAt(text, start);
  if (at + 1 < graphemes.length && _wide(graphemes[at + 1])) {
    return (start, start + len + graphemes[at + 1].length);
  }
  if (at > 0 && _wide(graphemes[at - 1])) {
    return (start - graphemes[at - 1].length, start + len);
  }
  return (start, start + len);
}

/// Diameter of a handle's knob, and the finger-sized area around it that
/// grabs it. The area hangs BELOW the line only, so a tap on the selected text
/// itself still reaches the editor (a triple-tap must land on the canvas).
const double touchHandleKnob = 20;
const double touchHandleGrab = 44;

/// One selection end's handle: a teardrop under the line whose point touches
/// the selection edge (Android's shape — the point faces the selected text).
/// Lay it out with its top-left at [touchHandleOrigin].
class TouchSelectionHandle extends StatelessWidget {
  const TouchSelectionHandle({
    super.key,
    required this.isStart,
    required this.color,
    required this.onDragStart,
    required this.onDragUpdate,
    required this.onDragEnd,
  });

  final bool isStart;
  final Color color;
  final GestureDragStartCallback onDragStart;
  final GestureDragUpdateCallback onDragUpdate;
  final VoidCallback onDragEnd;

  @override
  Widget build(BuildContext context) {
    const r = Radius.circular(touchHandleKnob / 2);
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      dragStartBehavior: DragStartBehavior.down,
      onPanStart: onDragStart,
      onPanUpdate: onDragUpdate,
      onPanEnd: (_) => onDragEnd(),
      onPanCancel: onDragEnd,
      child: SizedBox(
        width: touchHandleGrab,
        height: touchHandleGrab,
        child: Align(
          alignment: isStart ? Alignment.topRight : Alignment.topLeft,
          child: Container(
            key: ValueKey(isStart ? 'touch-handle-start' : 'touch-handle-end'),
            width: touchHandleKnob,
            height: touchHandleKnob,
            decoration: BoxDecoration(
              color: color,
              borderRadius: BorderRadius.only(
                topLeft: isStart ? r : Radius.zero,
                topRight: isStart ? Radius.zero : r,
                bottomLeft: r,
                bottomRight: r,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Canvas-local top-left for a handle whose selection edge is [caret] (the
/// caret rect at that end): the knob's pointed corner sits on the caret's
/// bottom, left of it for the start handle and right of it for the end.
Offset touchHandleOrigin(Rect caret, {required bool isStart}) => isStart
    ? Offset(caret.left - touchHandleGrab + 1, caret.bottom)
    : Offset(caret.left, caret.bottom);

/// Height reserved above the selection for [TouchSelectionMenu].
const double touchMenuHeight = 44;

/// The text menu over a finger selection: a row of plain text buttons
/// (剪切 / 复制 / 粘贴 / 全选), the platform's own selection menu shape — not
/// the formatting toolbar a mouse selection gets.
class TouchSelectionMenu extends StatelessWidget {
  const TouchSelectionMenu({super.key, required this.items});

  /// (label, action) in display order.
  final List<(String, VoidCallback)> items;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return ExcludeFocus(
      child: Material(
        key: const ValueKey('touch-selection-menu'),
        elevation: 6,
        color: scheme.inverseSurface,
        borderRadius: BorderRadius.circular(8),
        child: SizedBox(
          height: touchMenuHeight - 6,
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final (label, action) in items)
                TextButton(
                  onPressed: action,
                  style: TextButton.styleFrom(
                    foregroundColor: scheme.onInverseSurface,
                    padding: const EdgeInsets.symmetric(horizontal: 14),
                    minimumSize: const Size(48, touchMenuHeight - 6),
                    shape: const RoundedRectangleBorder(),
                  ),
                  child: Text(label),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Tracks one handle drag: which end stays put, and how far the finger sat
/// from the edge it grabbed — the finger is under the line, the edge on it,
/// so following the finger itself would jump the edge down a line.
class TouchHandleDrag {
  TouchHandleDrag({required this.fixed, required this.grabOffset});

  /// The selection end that does not move (the other handle's end).
  final DocPosition fixed;

  /// Finger minus the grabbed edge's line-center, canvas-local.
  final Offset grabOffset;

  /// Where on the canvas the moving edge should go for a finger at [finger].
  Offset target(Offset finger) => finger - grabOffset;
}

/// Counts repeated taps for word (2) / block (3) selection. The platform
/// GestureDetector reports double-taps without a position and has no
/// triple-tap, so the editor counts taps itself: one within [window] and the
/// slop of the previous one escalates the count, anything else restarts at 1.
/// Stamps are pointer-event times (binding clock, not wall clock), so the
/// window is deterministic under the test fake-async clock.
class MultiTapCounter {
  static const Duration window = Duration(milliseconds: 400);

  /// A mouse lands where it was; a finger lands a fingertip off, so its
  /// second tap gets a fingertip of room (Flutter's own double-tap slop is
  /// larger still, but that would merge taps on neighbouring words).
  static const double mouseSlop = 12, touchSlop = 30;

  int _count = 0;
  Offset? _lastPos;
  Duration? _lastStamp;

  /// End the sequence (a drag, or a tap that consumed it).
  void reset() => _count = 0;

  /// Record a tap at [pos] stamped [now]; returns the running count.
  int bump(Offset pos, Duration now, {required bool touch}) {
    final lastPos = _lastPos, last = _lastStamp;
    final near =
        lastPos != null &&
        (lastPos - pos).distance <= (touch ? touchSlop : mouseSlop);
    final soon = last != null && (now - last) <= window;
    _count = (near && soon) ? _count + 1 : 1;
    _lastPos = pos;
    _lastStamp = now;
    return _count;
  }
}

/// The pointers whose long-press selects a word: a finger or a pen. Never the
/// mouse — holding its button still for a moment and then dragging must stay a
/// drag-select, which a mouse long-press would win the gesture arena from.
const Set<PointerDeviceKind> touchLongPressDevices = {
  PointerDeviceKind.touch,
  PointerDeviceKind.stylus,
};
