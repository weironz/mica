// Geometry for the editor's floating chrome (hover tooltips today).
//
// A pure function in its own file because `RenderDocument` cannot be constructed
// by a test, and "does the bubble end up somewhere visible?" is exactly the kind
// of arithmetic that is wrong only at the edges.

import 'dart:ui';

/// Where a hover tooltip's bubble goes: above [anchor] when there is room inside
/// [visible], flipped below it when there isn't.
///
/// The flip is the point. The bubble used to be placed at `anchor.top - height -
/// gap` with **no clamping at all**, so a code block scrolled up against the top
/// of the viewport put its toolbar tooltips above the visible area — drawn, paid
/// for, and invisible. Hovering the Copy button showed the button highlight and
/// nothing else, which reads as a broken tooltip rather than a clipped one.
///
/// [visible] is the paint-time clip rect (`Canvas.getLocalClipBounds()`), i.e.
/// the same coordinate space [anchor] is already in. Using the RenderBox's own
/// `size` instead would be wrong: its height is the whole *document*, so "is
/// there room above" would be measured against the top of the document rather
/// than the top of what the user can actually see.
Rect tooltipRect({
  required Rect anchor,
  required Size bubble,
  required Rect visible,
  double gap = 4,
  double margin = 4,
}) {
  final left = (anchor.center.dx - bubble.width / 2).clamp(
    visible.left + margin,
    // The inner clamp keeps a bubble wider than the viewport pinned at the left
    // edge instead of producing an inverted range (which would throw).
    (visible.right - bubble.width - margin).clamp(
      visible.left + margin,
      double.infinity,
    ),
  );

  final above = anchor.top - bubble.height - gap;
  final below = anchor.bottom + gap;
  // Prefer above; flip below only when above would land outside. If neither fits
  // (a viewport shorter than the bubble) the clamp keeps it on screen.
  var top = above >= visible.top + margin ? above : below;
  top = top.clamp(
    visible.top + margin,
    (visible.bottom - bubble.height - margin).clamp(
      visible.top + margin,
      double.infinity,
    ),
  );

  return Rect.fromLTWH(left, top, bubble.width, bubble.height);
}
