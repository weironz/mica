//! Block-mode marquee selection state, geometry hit-testing, and non-interactive
//! overlay (1:1 port of `main:clients/mica_flutter/lib/editor/block_marquee.dart`).
//!
//! Dragging a rectangle starting in the page's blank margins, the left 24px
//! block-handle gutter, or below the final block highlights every overlapping
//! top-level block as a contiguous range `(lo, hi)`.

use nsh_core::{color::Rgba, geometry::px, prelude::*};

use crate::mica_theme::palette;

pub use mica_editor::interaction::{
  MARQUEE_DRAG_SLOP, MARQUEE_GUTTER_WIDTH, MARQUEE_SCROLLBAR_STRIP, RectF, hit_block_range,
  starts_marquee_in_margin,
};

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

/// Non-interactive overlay painting the live Notion-style marquee drag rectangle.
/// Per-block selection pills are rendered authoritatively inside each virtual
/// list row (`block-selection-overlay`) so they scroll in lockstep and never
/// double-paint with misaligned ghost rims.
pub fn marquee_overlay_layer(
  state: BlockMarqueeState,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let active_rect = state.active_rect;

  view()
    .test_id("block-marquee-overlay")
    .absolute()
    .top(px(0.0))
    .left(px(0.0))
    .right(px(0.0))
    .bottom(px(0.0))
    .pointer_events(PointerEvents::None)
    .child(dynamic(move || {
      let live = active_rect.get();
      let p = palette(dark_mode.get());

      let mut layer = view()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .pointer_events(PointerEvents::None);

      if let Some(r) = live {
        let fill = Rgba {
          a: (p.selection_bg.a * 0.72).clamp(0.0, 1.0),
          ..p.selection_bg
        };
        layer = layer.child(
          view()
            .test_id("block-marquee-rect")
            .absolute()
            .left(px(r.left))
            .top(px(r.top))
            .w(px(r.width()))
            .h(px(r.height()))
            .rounded(px(2.0))
            .bg(fill)
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

    // Blank left and right margins (seamless all the way up to content_left / content_right)
    assert!(starts_marquee_in_margin(50.0, 100.0, w, c_left, c_right, c_bottom));
    assert!(starts_marquee_in_margin(192.0, 100.0, w, c_left, c_right, c_bottom));
    assert!(starts_marquee_in_margin(1006.0, 100.0, w, c_left, c_right, c_bottom));
    assert!(starts_marquee_in_margin(1050.0, 100.0, w, c_left, c_right, c_bottom));

    // Right 16px scrollbar strip excluded (> 1184)
    assert!(!starts_marquee_in_margin(1190.0, 100.0, w, c_left, c_right, c_bottom));

    // Left & right 24px gutters inside content column start marquee
    assert!(starts_marquee_in_margin(210.0, 100.0, w, c_left, c_right, c_bottom));
    assert!(starts_marquee_in_margin(990.0, 100.0, w, c_left, c_right, c_bottom));

    // Inside prose text body -> normal text selection (false)
    assert!(!starts_marquee_in_margin(350.0, 100.0, w, c_left, c_right, c_bottom));
    assert!(!starts_marquee_in_margin(950.0, 100.0, w, c_left, c_right, c_bottom));

    // Below content_bottom inside content column -> starts marquee (true)
    assert!(starts_marquee_in_margin(350.0, 480.0, w, c_left, c_right, c_bottom));
  }
}
