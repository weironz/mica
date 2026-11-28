//! Root-level dark-pill hover tooltip overlay for icon buttons.
//!
//! Mounted once at the root of the application shell (`z_index(120)`,
//! `PointerEvents::None`) so tooltips are never clipped by `overflow_hidden()`
//! containers (such as the breadcrumb row or editor viewport) and automatically
//! clamp inside the viewport bounds.

use nsh_core::{
  color::{rgb, rgba, white},
  geometry::{Bounds, Points, px},
  prelude::*,
};

#[derive(Clone, Debug, PartialEq)]
struct TooltipEntry {
  label: String,
  anchor_x: f32,
  anchor_top: f32,
  anchor_bottom: f32,
  token: u64,
}

/// Shared tooltip state installed once at the app root via `provide_context`.
#[derive(Clone, Copy)]
pub struct TooltipHost {
  entry: Signal<Option<TooltipEntry>>,
  next_token: Signal<u64>,
  viewport_w: Signal<f32>,
}

/// Install a root [`TooltipHost`] into the reactive context tree.
pub fn install_tooltip_host(viewport_w: Signal<f32>) -> TooltipHost {
  let host = TooltipHost {
    entry: signal(None),
    next_token: signal(0u64),
    viewport_w,
  };
  provide_context(host);
  host
}

/// Immediately dismiss any active hover tooltip (e.g., when a button is clicked).
pub fn hide_tooltip() {
  if let Some(host) = use_context::<TooltipHost>()
    && host.entry.get_untracked().is_some()
  {
    host.entry.set(None);
  }
}

/// Estimate the single-line rendered pixel width of a `12px` tooltip pill.
pub fn estimate_tooltip_width(label: &str) -> f32 {
  let text_w: f32 = label
    .chars()
    .map(|ch| {
      if ch.is_ascii() {
        if ch.is_ascii_uppercase() { 8.0 } else { 7.0 }
      } else {
        12.5
      }
    })
    .sum();
  (text_w + 18.0).max(36.0)
}

/// Compute clamped `(left, top)` window coordinates for a tooltip pill anchored
/// to a trigger box `[anchor_top..anchor_bottom]` centered at `anchor_x`.
///
/// Places the pill cleanly ABOVE the trigger when there is vertical headroom
/// (`anchor_top >= 36px`, matching the editor header toolbar style) and flips
/// BELOW for triggers in the top `34px` titlebar.
pub fn compute_tooltip_pos(
  label: &str,
  anchor_x: f32,
  anchor_top: f32,
  anchor_bottom: f32,
  viewport_w: f32,
) -> (f32, f32) {
  let est_w = estimate_tooltip_width(label);
  let max_left = (viewport_w - est_w - 8.0).max(8.0);
  let left = (anchor_x - est_w * 0.5).clamp(8.0, max_left);
  let top = if anchor_top >= 36.0 {
    (anchor_top - 30.0).max(4.0)
  } else {
    anchor_bottom + 6.0
  };
  (left, top)
}

/// Wrap any icon button element with a static hover tooltip label.
pub fn with_tooltip<E: IntoElement + 'static>(el: E, label: &'static str) -> impl IntoElement {
  with_dynamic_tooltip(el, move || label)
}

/// Wrap any icon button element with a dynamic hover tooltip label (e.g., `"收藏"` / `"取消收藏"`).
pub fn with_dynamic_tooltip<E, F, S>(el: E, label_fn: F) -> impl IntoElement
where
  E: IntoElement + 'static,
  S: AsRef<str>,
  F: Fn() -> S + 'static,
{
  let host = use_context::<TooltipHost>();
  let box_size = signal((28.0_f32, 28.0_f32));
  let my_token = signal(0u64);

  view()
    .flex_shrink_0()
    .on_layout(move |b: &Bounds<Points>| {
      let (w, h) = (b.size.width.0, b.size.height.0);
      if w > 0.0 && h > 0.0 {
        let (pw, ph) = box_size.get_untracked();
        if (pw - w).abs() > 0.5 || (ph - h).abs() > 0.5 {
          box_size.set((w, h));
        }
      }
    })
    .on_mouse_enter(move |e: &MouseEvent| {
      let Some(host) = host else {
        return;
      };
      let raw = label_fn();
      let label = raw.as_ref();
      if label.trim().is_empty() {
        return;
      }
      let (w, h) = box_size.get_untracked();
      let origin_x = e.page_position.x.0 - e.position.x.0;
      let origin_y = e.page_position.y.0 - e.position.y.0;
      let tok = host.next_token.get_untracked().wrapping_add(1);
      host.next_token.set(tok);
      my_token.set(tok);
      host.entry.set(Some(TooltipEntry {
        label: label.to_string(),
        anchor_x: origin_x + w * 0.5,
        anchor_top: origin_y,
        anchor_bottom: origin_y + h,
        token: tok,
      }));
    })
    .on_mouse_leave(move |_: &MouseEvent| {
      let Some(host) = host else {
        return;
      };
      let tok = my_token.get_untracked();
      if host.entry.get_untracked().is_some_and(|cur| cur.token == tok) {
        host.entry.set(None);
      }
    })
    .child(el)
}

/// Root-level non-interactive overlay rendering the active dark tooltip pill.
pub fn tooltip_overlay(host: TooltipHost, dark_mode: Signal<bool>) -> impl IntoElement {
  when(move || host.entry.get().is_some(), move || {
    let Some(cur) = host.entry.get() else {
      return view().into_element();
    };
    let vw = host.viewport_w.get().max(320.0);
    let (left, top) = compute_tooltip_pos(
      &cur.label,
      cur.anchor_x,
      cur.anchor_top,
      cur.anchor_bottom,
      vw,
    );
    let dark = dark_mode.get();
    view()
      .test_id("hover-tooltip")
      .absolute()
      .left(px(left))
      .top(px(top))
      .z_index(120)
      .pointer_events(PointerEvents::None)
      .px(px(9.0))
      .py(px(4.0))
      .rounded(px(6.0))
      .bg(if dark {
        rgba(0x26262AF5)
      } else {
        rgba(0x111111F0)
      })
      .border_1()
      .border_color(if dark {
        rgb(0x3F3F46)
      } else {
        rgba(0x111111F0)
      })
      .shadow_sm()
      .child(
        text(cur.label)
          .text_size(px(12.0))
          .font_weight(FontWeight::MEDIUM)
          .text_color(white()),
      )
      .into_element()
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tooltip_places_above_editor_header_and_flips_below_titlebar() {
    let (l_above, t_above) = compute_tooltip_pos("收藏", 900.0, 42.0, 70.0, 1280.0);
    assert!(t_above < 42.0, "editor header tooltip should sit above button");
    assert!(l_above > 850.0 && l_above < 900.0);

    let (_l_below, t_below) = compute_tooltip_pos("应用菜单", 20.0, 2.0, 32.0, 1280.0);
    assert!(t_below > 32.0, "top titlebar tooltip should flip below button");
  }

  #[test]
  fn tooltip_clamps_near_left_and_right_viewport_edges() {
    let (l_left, _) = compute_tooltip_pos("应用菜单", 6.0, 42.0, 70.0, 1280.0);
    assert!((l_left - 8.0).abs() < 0.01);

    let (l_right, _) = compute_tooltip_pos("切换右侧大纲与页面信息边栏", 1270.0, 42.0, 70.0, 1280.0);
    let est_w = estimate_tooltip_width("切换右侧大纲与页面信息边栏");
    assert!(l_right + est_w <= 1280.0 - 7.5);
  }
}
