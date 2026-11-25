//! UI-framework-agnostic editor interaction rules & geometry primitives (L2).
//!
//! Keeps high-value editor interaction invariants — touch CJK/Latin word bounds,
//! touch multi-tap escalation, block marquee hit-testing and margin rules,
//! atomic vs character-selectable block classification, and edge auto-scroll
//! velocity — out of any single UI shell so future shell rewrites inherit them
//! directly with zero re-implementation drift.

use crate::utf16::{utf16_len, utf16_slice};

// ── Block Selection Classification ───────────────────────────────────────────

/// Returns `true` for atomic blocks that select as a whole during cross-block
/// dragging rather than character-by-character.
///
/// Code blocks are intentionally excluded so dragging through a code snippet
/// selects characters continuously without stalling.
pub fn is_atomic_selection_kind(kind: &str) -> bool {
  matches!(
    kind,
    "board" | "mermaid" | "math" | "math_block" | "image" | "table" | "divider"
  )
}

// ── Touch Word Selection & Multi-Tap Counter (CAP-16-06) ─────────────────────

fn is_wide_ideograph(ch: char) -> bool {
  let c = ch as u32;
  (0x3400..=0x9FFF).contains(&c)
    || (0x3040..=0x30FF).contains(&c)
    || (0xAC00..=0xD7A3).contains(&c)
    || (0xF900..=0xFAFF).contains(&c)
    || (0x20000..=0x2FA1F).contains(&c)
}

fn is_wide_punctuation(ch: char) -> bool {
  let c = ch as u32;
  (0x2000..=0x206F).contains(&c)
    || (0x3000..=0x303F).contains(&c)
    || (0xFE30..=0xFE4F).contains(&c)
    || (0xFF00..=0xFF0F).contains(&c)
    || (0xFF1A..=0xFF20).contains(&c)
    || (0xFF3B..=0xFF40).contains(&c)
    || (0xFF5B..=0xFF65).contains(&c)
}

/// Computes the UTF-16 `(start, end)` range selected by a touch double-tap or
/// long-press at UTF-16 unit offset `glyph_u16`:
/// - Wide CJK / kana / hangul: tapped char + next wide char (or previous wide
///   char at the end of a run, or just that 1 char if isolated).
/// - Wide CJK punctuation (`，。、“”（）【】`): selected alone.
/// - Latin words & numbers: full contiguous word.
/// - Surrogate-pair safe: never splits emoji or CJK Ext-B characters.
pub fn touch_word_bounds(text: &str, glyph_u16: u32) -> (u32, u32) {
  let total_u16 = utf16_len(text);
  if total_u16 == 0 {
    return (0, 0);
  }
  let g = glyph_u16.min(total_u16.saturating_sub(1));

  let mut chars: Vec<(char, u32, u32)> = Vec::with_capacity(text.len());
  let mut cur = 0u32;
  for ch in text.chars() {
    let next = cur + ch.len_utf16() as u32;
    chars.push((ch, cur, next));
    cur = next;
  }

  let at = chars
    .iter()
    .position(|&(_, s, e)| g >= s && g < e)
    .unwrap_or(chars.len() - 1);
  let (ch, s, e) = chars[at];

  if is_wide_punctuation(ch) {
    return (s, e);
  }
  if is_wide_ideograph(ch) {
    if at + 1 < chars.len() && is_wide_ideograph(chars[at + 1].0) {
      return (s, chars[at + 1].2);
    }
    if at > 0 && is_wide_ideograph(chars[at - 1].0) {
      return (chars[at - 1].1, e);
    }
    return (s, e);
  }
  let is_latin_word = |c: char| !is_wide_ideograph(c) && (c.is_alphanumeric() || c == '_');
  if is_latin_word(ch) {
    let mut lo = at;
    while lo > 0 && is_latin_word(chars[lo - 1].0) {
      lo -= 1;
    }
    let mut hi = at;
    while hi + 1 < chars.len() && is_latin_word(chars[hi + 1].0) {
      hi += 1;
    }
    return (chars[lo].1, chars[hi].2);
  }
  (s, e)
}

/// Returns the UTF-16 slice selected by [`touch_word_bounds`].
pub fn touch_selected_slice(text: &str, glyph_u16: u32) -> &str {
  let (s, e) = touch_word_bounds(text, glyph_u16);
  utf16_slice(text, s, e)
}

/// Counts repeated touch taps (since Web backends often report `clicks = 1`
/// for touch pointers): taps within `400ms` and `30px` escalate `1 -> 2 -> 3`.
#[derive(Clone, Debug, Default)]
pub struct TouchTapTracker {
  count: u32,
  last_pos: Option<(f32, f32)>,
  last_ms: f64,
}

impl TouchTapTracker {
  pub const WINDOW_MS: f64 = 400.0;
  pub const TOUCH_SLOP_PX: f32 = 30.0;

  pub fn bump(&mut self, x: f32, y: f32, now_ms: f64) -> u32 {
    let near = self.last_pos.is_some_and(|(lx, ly)| {
      let dx = x - lx;
      let dy = y - ly;
      (dx * dx + dy * dy).sqrt() <= Self::TOUCH_SLOP_PX
    });
    let soon = self.last_ms > 0.0 && (now_ms - self.last_ms) <= Self::WINDOW_MS;
    self.count = if near && soon {
      (self.count % 3) + 1
    } else {
      1
    };
    self.last_pos = Some((x, y));
    self.last_ms = now_ms;
    self.count
  }

  pub fn reset(&mut self) {
    self.count = 0;
    self.last_pos = None;
    self.last_ms = 0.0;
  }
}

// ── Block-Mode Marquee Geometry (CAP-02-05 / CAP-02-06) ──────────────────────

/// Width of the right-edge scrollbar strip excluded from starting a marquee.
pub const MARQUEE_SCROLLBAR_STRIP: f32 = 16.0;
/// Width of the left/right block-handle gutters inside the content column.
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
  pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
    Self {
      left,
      top,
      right,
      bottom,
    }
  }

  pub fn from_points(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
    Self {
      left: x0.min(x1),
      top: y0.min(y1),
      right: x0.max(x1),
      bottom: y0.max(y1),
    }
  }

  pub fn width(self) -> f32 {
    (self.right - self.left).max(0.0)
  }

  pub fn height(self) -> f32 {
    (self.bottom - self.top).max(0.0)
  }

  /// Returns `true` when `self` and `other` have non-empty interior overlap.
  pub fn intersects(self, other: RectF) -> bool {
    self.left < other.right
      && self.right > other.left
      && self.top < other.bottom
      && self.bottom > other.top
  }

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
/// normal text selection or scrollbar dragging.
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
  if local_x > container_w - MARQUEE_SCROLLBAR_STRIP {
    return false;
  }
  if local_x < content_left || local_x > content_right {
    return true;
  }
  if local_x < content_left + MARQUEE_GUTTER_WIDTH
    || local_x > content_right - MARQUEE_GUTTER_WIDTH
  {
    return true;
  }
  local_y >= content_bottom
}

// ── Edge Auto-Scroll Velocity (CAP-02-03) ────────────────────────────────────

/// Computes continuous 60fps vertical edge auto-scroll velocity (in px per 16ms
/// tick; negative scrolls up, positive scrolls down, `0.0` inside the safe band)
/// when dragging a text selection, touch handle, or marquee box near/past the
/// top or bottom edge of the editor viewport.
pub fn edge_autoscroll_speed(cy: f32, viewport_h: f32) -> f32 {
  if viewport_h <= 1.0 {
    return 0.0;
  }
  let zone = 56.0_f32.min(viewport_h * 0.25);
  const MIN_SPEED: f32 = 10.0;
  const MAX_SPEED: f32 = 38.0;
  const RAMP: f32 = 96.0;
  let ramp = |past: f32| {
    let t = (past / RAMP).clamp(0.0, 1.0);
    MIN_SPEED + t * (MAX_SPEED - MIN_SPEED)
  };
  if cy < zone {
    -ramp(zone - cy)
  } else if cy > viewport_h - zone {
    ramp(cy - (viewport_h - zone))
  } else {
    0.0
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn touch_word_bounds_handles_cjk_latin_punctuation_and_emoji() {
    let s = "泰国算力交付清单，共32台";
    assert_eq!(touch_selected_slice(s, 0), "泰国");
    assert_eq!(touch_selected_slice(s, 2), "算力");
    assert_eq!(touch_selected_slice(s, 7), "清单");
    assert_eq!(touch_selected_slice(s, 8), "，");
    assert_eq!(touch_selected_slice(s, 9), "共");
    assert_eq!(touch_selected_slice(s, 10), "32");
    assert_eq!(touch_selected_slice("hello world", 1), "hello");
    assert_eq!(touch_selected_slice("🚀火箭", 0), "🚀");
    assert_eq!(touch_selected_slice("🚀火箭", 1), "🚀");
    assert_eq!(touch_selected_slice("🚀火箭", 2), "火箭");
  }

  #[test]
  fn touch_tap_tracker_escalates_within_window_and_slop() {
    let mut tracker = TouchTapTracker::default();
    assert_eq!(tracker.bump(100.0, 200.0, 1000.0), 1);
    assert_eq!(tracker.bump(110.0, 205.0, 1250.0), 2);
    assert_eq!(tracker.bump(108.0, 202.0, 1500.0), 3);
    assert_eq!(tracker.bump(108.0, 202.0, 1700.0), 1);
    assert_eq!(tracker.bump(200.0, 202.0, 1800.0), 1);
    assert_eq!(tracker.bump(202.0, 202.0, 2300.0), 1);
  }

  #[test]
  fn marquee_geometry_and_margin_rules_are_shell_independent() {
    let r = RectF::from_points(120.0, 200.0, 40.0, 50.0);
    assert_eq!(r, RectF::new(40.0, 50.0, 120.0, 200.0));
    assert!(r.intersects(RectF::new(100.0, 100.0, 160.0, 160.0)));
    assert!(!r.intersects(RectF::new(120.0, 50.0, 200.0, 200.0)));

    let blocks = vec![
      (0, RectF::new(100.0, 0.0, 700.0, 30.0)),
      (1, RectF::new(500.0, 40.0, 700.0, 70.0)),
      (2, RectF::new(100.0, 80.0, 700.0, 110.0)),
    ];
    assert_eq!(
      hit_block_range(RectF::from_points(90.0, 10.0, 150.0, 90.0), &blocks),
      Some((0, 2))
    );

    assert!(starts_marquee_in_margin(50.0, 100.0, 1200.0, 200.0, 1000.0, 450.0));
    assert!(!starts_marquee_in_margin(1190.0, 100.0, 1200.0, 200.0, 1000.0, 450.0));
    assert!(!starts_marquee_in_margin(350.0, 100.0, 1200.0, 200.0, 1000.0, 450.0));
    assert!(starts_marquee_in_margin(350.0, 480.0, 1200.0, 200.0, 1000.0, 450.0));
  }

  #[test]
  fn atomic_selection_excludes_code_blocks_and_autoscroll_ramps() {
    assert!(!is_atomic_selection_kind("code_block"));
    assert!(!is_atomic_selection_kind("paragraph"));
    assert!(is_atomic_selection_kind("table"));
    assert!(is_atomic_selection_kind("board"));
    assert_eq!(edge_autoscroll_speed(300.0, 600.0), 0.0);
    assert!(edge_autoscroll_speed(-40.0, 600.0) < edge_autoscroll_speed(50.0, 600.0));
    assert!(edge_autoscroll_speed(650.0, 600.0) > edge_autoscroll_speed(550.0, 600.0));
  }
}
