//! Block-mode marquee selection state, geometry hit-testing, and non-interactive
//! overlay (1:1 port of `main:clients/mica_flutter/lib/editor/block_marquee.dart`).
//!
//! Dragging a rectangle starting in the page's blank margins, the left 24px
//! block-handle gutter, or below the final block highlights every overlapping
//! top-level block as a contiguous range `(lo, hi)`.

use nsh_core::{color::Rgba, geometry::px, prelude::*};

use crate::mica_theme::palette;

/// Width of the right-edge scrollbar strip excluded from starting a marquee.
pub const MARQUEE_SCROLLBAR_STRIP: f32 = 16.0;
/// Width of each `PageWidthFrame` side resize grip excluded from marquee starts.
pub const MARQUEE_GRIP_WIDTH: f32 = 12.0;
/// Width of the left block-handle gutter inside the content column.
pub const MARQUEE_GUTTER_WIDTH: f32 = 24.0;
/// Minimum pointer travel (px) before a press upgrades into a live marquee drag.
pub const MARQUEE_DRAG_SLOP: f32 = 4.0;

/// Axis-aligned rectangle in container-local logical pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RectF {
  pub left: f32,
  pub top: f32,
  pub right: f32,
  pub bottom: f32,
}

impl RectF {
  /// Constructs a rectangle from explicit edges.
  pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
    Self {
      left,
      top,
      right,
      bottom,
    }
  }

  /// Constructs a normalized rectangle from two arbitrary corner points.
  pub fn from_points(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
    Self {
      left: x0.min(x1),
      top: y0.min(y1),
      right: x0.max(x1),
      bottom: y0.max(y1),
    }
  }

  /// Horizontal span (clamped to `>= 0.0`).
  pub fn width(self) -> f32 {
    (self.right - self.left).max(0.0)
  }

  /// Vertical span (clamped to `>= 0.0`).
  pub fn height(self) -> f32 {
    (self.bottom - self.top).max(0.0)
  }

  /// Returns `true` when `self` and `other` have non-empty interior overlap
  /// (1:1 Dart `Rect.overlaps`).
  pub fn intersects(self, other: RectF) -> bool {
    self.left < other.right
      && self.right > other.left
      && self.top < other.bottom
      && self.bottom > other.top
  }

  /// Expands every edge outward by `delta` logical pixels.
  pub fn inflate(self, delta: f32) -> Self {
    Self {
      left: self.left - delta,
      top: self.top - delta,
      right: self.right + delta,
      bottom: self.bottom + delta,
    }
  }
}

/// Returns the contiguous top-level block index range `(lo, hi)` (inclusive)
/// whose rectangles intersect `marquee`, or `None` when no block overlaps.
pub fn hit_block_range(marquee: RectF, block_rects: &[(usize, RectF)]) -> Option<(usize, usize)> {
  let mut lo: Option<usize> = None;
  let mut hi: usize = 0;
  for &(idx, rect) in block_rects {
    if !rect.intersects(marquee) {
      continue;
    }
    lo = Some(lo.map_or(idx, |cur| cur.min(idx)));
    hi = hi.max(idx);
  }
  lo.map(|start| (start, hi))
}

/// Returns `true` if a pointer press at `(local_x, local_y)` inside a pane of
/// width `container_w` should begin a block-mode marquee selection rather than
/// normal text selection, scrollbar dragging, or page-width resizing.
///
/// Matches Flutter `EditorMarqueeRegion._startsMarquee`:
/// - Excludes the right `16px` scrollbar strip (`local_x > container_w - 16`).
/// - Excludes the left & right `12px` page-width grips adjacent to
///   `[content_left, content_right]`.
/// - Accepts presses in the blank left/right margins outside the grips.
/// - Accepts presses inside the left `24px` gutter of the content column.
/// - Accepts presses below `content_bottom` within the content column.
pub fn starts_marquee_in_margin(
  local_x: f32,
  local_y: f32,
  container_w: f32,
  content_left: f32,
  content_right: f32,
  content_bottom: f32,
) -> bool {
  if local_x < 0.0 || local_y < 0.0 || container_w <= 0.0 {
    return false;
  }
  // 1. Right-edge scrollbar strip must always scroll.
  if local_x > container_w - MARQUEE_SCROLLBAR_STRIP {
    return false;
  }
  // 2. Blank margins outside the page-width frame (which extends 12px on each side).
  let page_left = content_left - MARQUEE_GRIP_WIDTH;
  let page_right = content_right + MARQUEE_GRIP_WIDTH;
  if local_x < page_left || local_x > page_right {
    return true;
  }
  // 3. The 12px page-width resize grips belong exclusively to `PageWidthFrame`.
  let on_grip = local_x < content_left || local_x > content_right;
  if on_grip {
    return false;
  }
  // 4. Inside the content column: left 24px block-handle gutter starts a marquee.
  if local_x < content_left + MARQUEE_GUTTER_WIDTH {
    return true;
  }
  // 5. Blank space below the last block starts a marquee.
  local_y >= content_bottom
}

/// Reactive state for block-mode marquee selection.
#[derive(Clone)]
pub struct BlockMarqueeState {
  /// Live drag rectangle in container-local coordinates while dragging.
  pub active_rect: Signal<Option<RectF>>,
  /// Contiguous selected top-level block indices `lo..=hi` (inclusive).
  pub selected_range: Signal<Option<(usize, usize)>>,
  /// Bounding boxes of selected blocks to paint `6px`-rounded selection tints.
  pub highlighted_rects: Signal<Vec<RectF>>,
}

impl BlockMarqueeState {
  /// Creates an idle marquee state with no active drag or selected blocks.
  pub fn new() -> Self {
    Self {
      active_rect: signal(None),
      selected_range: signal(None),
      highlighted_rects: signal(Vec::new()),
    }
  }

  /// Clears any active marquee drag and selected block highlight.
  pub fn clear(&self) {
    if self.active_rect.get_untracked().is_some() {
      self.active_rect.set(None);
    }
    if self.selected_range.get_untracked().is_some() {
      self.selected_range.set(None);
    }
    if !self.highlighted_rects.get_untracked().is_empty() {
      self.highlighted_rects.set(Vec::new());
    }
  }

  /// Returns `true` while dragging a marquee or while blocks are block-selected.
  pub fn is_active(&self) -> bool {
    self.active_rect.get().is_some() || self.selected_range.get().is_some()
  }

  /// Untracked variant of [`Self::is_active`] for event handlers.
  pub fn is_active_untracked(&self) -> bool {
    self.active_rect.get_untracked().is_some() || self.selected_range.get_untracked().is_some()
  }

  /// Returns the currently selected contiguous top-level block range, if any.
  pub fn selected_blocks(&self) -> Option<(usize, usize)> {
    self.selected_range.get()
  }

  /// Updates the live drag rectangle and recomputes hit blocks + highlight rects.
  pub fn update_drag(&self, rect: RectF, block_rects: &[(usize, RectF)]) {
    self.active_rect.set(Some(rect));
    let range = hit_block_range(rect, block_rects);
    self.selected_range.set(range);
    let boxes = match range {
      Some((lo, hi)) => block_rects
        .iter()
        .filter(|(idx, _)| *idx >= lo && *idx <= hi)
        .map(|(_, r)| r.inflate(2.0))
        .collect(),
      None => Vec::new(),
    };
    self.highlighted_rects.set(boxes);
  }

  /// Ends the live rectangle drag while retaining the block selection highlight.
  pub fn end_drag(&self) {
    if self.active_rect.get_untracked().is_some() {
      self.active_rect.set(None);
    }
  }
}

impl Default for BlockMarqueeState {
  fn default() -> Self {
    Self::new()
  }
}

/// Non-interactive overlay painting:
/// 1. `6px`-rounded `selection_bg` overlays over every highlighted block box,
/// 2. The live marquee rectangle (`0.5x` `selection_bg` fill + `0.7x` `accent`
///    1px border).
pub fn marquee_overlay_layer(
  state: BlockMarqueeState,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let active_rect = state.active_rect;
  let highlighted_rects = state.highlighted_rects;

  view()
    .test_id("block-marquee-overlay")
    .absolute()
    .top(px(0.0))
    .left(px(0.0))
    .right(px(0.0))
    .bottom(px(0.0))
    .pointer_events(PointerEvents::None)
    .child(dynamic(move || {
      let rects = highlighted_rects.get();
      let live = active_rect.get();
      let p = palette(dark_mode.get());

      let mut layer = view()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .pointer_events(PointerEvents::None);

      for rect in rects {
        layer = layer.child(
          view()
            .absolute()
            .left(px(rect.left))
            .top(px(rect.top))
            .w(px(rect.width()))
            .h(px(rect.height()))
            .rounded(px(6.0))
            .bg(p.selection_bg)
            .pointer_events(PointerEvents::None),
        );
      }

      if let Some(r) = live {
        let fill = Rgba {
          a: (p.selection_bg.a * 0.5).clamp(0.0, 1.0),
          ..p.selection_bg
        };
        let stroke = Rgba {
          a: 0.7,
          ..p.accent
        };
        layer = layer.child(
          view()
            .test_id("block-marquee-rect")
            .absolute()
            .left(px(r.left))
            .top(px(r.top))
            .w(px(r.width()))
            .h(px(r.height()))
            .bg(fill)
            .border_1()
            .border_color(stroke)
            .pointer_events(PointerEvents::None),
        );
      }

      layer.into_element()
    }))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rect_normalization_and_intersection() {
    let r = RectF::from_points(120.0, 200.0, 40.0, 50.0);
    assert_eq!(r, RectF::new(40.0, 50.0, 120.0, 200.0));
    assert_eq!(r.width(), 80.0);
    assert_eq!(r.height(), 150.0);

    // Overlapping
    assert!(r.intersects(RectF::new(100.0, 100.0, 160.0, 160.0)));
    // Touching edges only -> Dart Rect.overlaps semantics = false
    assert!(!r.intersects(RectF::new(120.0, 50.0, 200.0, 200.0)));
    assert!(!r.intersects(RectF::new(40.0, 200.0, 120.0, 250.0)));
    // Disjoint
    assert!(!r.intersects(RectF::new(200.0, 300.0, 260.0, 350.0)));
  }

  #[test]
  fn hit_block_range_returns_contiguous_bounds() {
    let blocks = vec![
      (0, RectF::new(100.0, 0.0, 700.0, 30.0)),
      (1, RectF::new(100.0, 40.0, 700.0, 70.0)),
      (2, RectF::new(100.0, 80.0, 700.0, 110.0)),
      (3, RectF::new(100.0, 120.0, 700.0, 150.0)),
    ];

    // Drag across blocks 1 and 2
    let marquee = RectF::from_points(80.0, 50.0, 250.0, 95.0);
    assert_eq!(hit_block_range(marquee, &blocks), Some((1, 2)));

    // Drag across block 0 and block 3 (even if only 0 and 3 touched, contiguous lo..=hi)
    let narrow_miss_middle = vec![
      (0, RectF::new(100.0, 0.0, 700.0, 30.0)),
      (1, RectF::new(500.0, 40.0, 700.0, 70.0)),
      (2, RectF::new(100.0, 80.0, 700.0, 110.0)),
    ];
    let left_strip = RectF::from_points(90.0, 10.0, 150.0, 90.0);
    assert_eq!(hit_block_range(left_strip, &narrow_miss_middle), Some((0, 2)));

    // Miss all blocks
    let miss = RectF::from_points(10.0, 10.0, 50.0, 90.0);
    assert_eq!(hit_block_range(miss, &blocks), None);
  }

  #[test]
  fn starts_marquee_margin_grip_gutter_scrollbar_rules() {
    // Container 1200px wide, centered 800px content column => [200, 1000], grips [188, 200) & (1000, 1012]
    let w = 1200.0;
    let c_left = 200.0;
    let c_right = 1000.0;
    let c_bottom = 450.0;

    // Blank left and right margins
    assert!(starts_marquee_in_margin(50.0, 100.0, w, c_left, c_right, c_bottom));
    assert!(starts_marquee_in_margin(1050.0, 100.0, w, c_left, c_right, c_bottom));

    // Right 16px scrollbar strip excluded (> 1184)
    assert!(!starts_marquee_in_margin(1190.0, 100.0, w, c_left, c_right, c_bottom));

    // 12px page-width resize grips excluded
    assert!(!starts_marquee_in_margin(192.0, 100.0, w, c_left, c_right, c_bottom));
    assert!(!starts_marquee_in_margin(1006.0, 100.0, w, c_left, c_right, c_bottom));

    // Left 24px gutter inside content column starts marquee
    assert!(starts_marquee_in_margin(210.0, 100.0, w, c_left, c_right, c_bottom));

    // Inside prose text body -> normal text selection (false)
    assert!(!starts_marquee_in_margin(350.0, 100.0, w, c_left, c_right, c_bottom));

    // Below content_bottom inside content column -> starts marquee (true)
    assert!(starts_marquee_in_margin(350.0, 480.0, w, c_left, c_right, c_bottom));
  }
}
