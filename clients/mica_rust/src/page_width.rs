//! Draggable editor page-width frame (1:1 port of
//! `main:clients/mica_flutter/lib/ui/page_width_frame.dart`).
//!
//! The editor content column is centered, so dragging either 12px side grip by
//! `dx` changes the column width symmetrically by `2 * side * dx`. Widths snap
//! to 11 discrete 120px steps (`680 · 800 · … · 1880`), where `1880.0` is the
//! stored sentinel for full width (`全宽`). Double-clicking either grip resets
//! to [`PAGE_WIDTH_DEFAULT`] (`800.0`).

use std::rc::Rc;

use nsh_core::{color::Rgba, geometry::px, prelude::*};

use crate::mica_theme::palette;

const TRANSPARENT_RGBA: Rgba = Rgba {
  r: 0.0,
  g: 0.0,
  b: 0.0,
  a: 0.0,
};

/// Minimum selectable editor column width in logical px.
pub const PAGE_WIDTH_MIN: f32 = 680.0;
/// Maximum stored value — sentinel for "full width" (`全宽`).
pub const PAGE_WIDTH_MAX: f32 = 1880.0;
/// Default readable column width (and double-click reset target).
pub const PAGE_WIDTH_DEFAULT: f32 = 800.0;
/// Step increment between discrete width stops (`(1880 - 680) / 10`).
pub const PAGE_WIDTH_STEP: f32 = 120.0;
/// Hit target width of each left/right resize grip.
pub const PAGE_WIDTH_GRIP: f32 = 12.0;
/// Minimum available container width before side resize grips activate.
pub const PAGE_WIDTH_MIN_ROOM_FOR_GRIPS: f32 = 600.0;

/// Returns `true` when `w` represents the "full width" (`全宽`) stop.
pub fn is_full_width(w: f32) -> bool {
  w >= PAGE_WIDTH_MAX
}

/// Returns the layout cap for a stored page width (`INFINITY` for full width).
pub fn page_column_cap(w: f32) -> f32 {
  if is_full_width(w) {
    f32::INFINITY
  } else {
    w
  }
}

/// Snaps a raw pixel width to the nearest discrete 120px stop in
/// `[PAGE_WIDTH_MIN, PAGE_WIDTH_MAX]`.
pub fn snap_page_width(raw: f32) -> f32 {
  let steps = ((raw - PAGE_WIDTH_MIN) / PAGE_WIDTH_STEP).round();
  (PAGE_WIDTH_MIN + steps * PAGE_WIDTH_STEP).clamp(PAGE_WIDTH_MIN, PAGE_WIDTH_MAX)
}

/// Computes the actual rendered column width given the stored width and
/// available pane `room` (excluding the two 12px grips).
pub fn rendered_column_width(stored: f32, room: f32) -> f32 {
  if !room.is_finite() || room <= 0.0 {
    return if is_full_width(stored) {
      PAGE_WIDTH_MAX
    } else {
      stored.clamp(PAGE_WIDTH_MIN, PAGE_WIDTH_MAX)
    };
  }
  page_column_cap(stored).min(room).max(0.0)
}

/// Computes the live snapped width during a symmetric edge drag (1:1 Flutter
/// `_PageWidthFrameState._drag`).
///
/// - `stored`: persisted width before the drag began
/// - `start_rendered`: rendered column width at drag start
/// - `room`: available width inside the frame (excluding grips)
/// - `side`: `-1.0` for the left grip, `+1.0` for the right grip
/// - `dx`: pointer delta in px from drag start (`current_x - start_x`)
pub fn compute_drag_width(stored: f32, start_rendered: f32, room: f32, side: f32, dx: f32) -> f32 {
  let raw = start_rendered + 2.0 * side * dx;
  if (raw - start_rendered).abs() < PAGE_WIDTH_STEP / 2.0 {
    stored
  } else if raw >= room {
    PAGE_WIDTH_MAX
  } else {
    snap_page_width(raw)
  }
}

/// Formats the floating readout pill shown while dragging a page-width grip.
pub fn width_readout_label(live_w: f32, rendered_w: f32) -> String {
  let px_val = rendered_w.round().max(0.0) as i32;
  if is_full_width(live_w) {
    format!("全宽 · {px_val} px")
  } else {
    format!("{px_val} px")
  }
}

/// Wraps `child` in a centered, edge-draggable page-width frame matching
/// Flutter's `PageWidthFrame`.
///
/// The child element is mounted once inside a stable container so resizing
/// across narrow/wide thresholds never remounts `rich_editor()`.
pub fn page_width_frame<C: IntoElement + 'static>(
  width: Signal<f32>,
  dark_mode: Signal<bool>,
  on_commit: Rc<dyn Fn(f32)>,
  child: C,
) -> impl IntoElement {
  let outer_width = signal(0.0_f32);
  let live_width: Signal<Option<f32>> = signal(None);
  let hovered_side = signal(0_i8);
  let drag_start_x = signal(0.0_f32);
  let drag_start_rendered = signal(PAGE_WIDTH_DEFAULT);
  let last_click_ms = signal(0.0_f64);

  let grips_enabled = computed(move || outer_width.get() >= PAGE_WIDTH_MIN_ROOM_FOR_GRIPS);
  let room_sig = computed(move || {
    let outer = outer_width.get();
    if outer <= 0.0 {
      0.0
    } else if outer >= PAGE_WIDTH_MIN_ROOM_FOR_GRIPS {
      (outer - 2.0 * PAGE_WIDTH_GRIP).max(0.0)
    } else {
      outer
    }
  });

  let build_grip = move |side: f32, commit_cb: Rc<dyn Fn(f32)>| {
    let side_i8: i8 = if side < 0.0 { -1 } else { 1 };
    let test_id = if side < 0.0 {
      "page-width-grip-left"
    } else {
      "page-width-grip-right"
    };

    view()
      .test_id(test_id)
      .w(move || {
        if grips_enabled.get() {
          px(PAGE_WIDTH_GRIP)
        } else {
          px(0.0)
        }
      })
      .flex_shrink_0()
      .items_center()
      .justify_center()
      .cursor(Cursor::ColResize)
      .on_mouse_enter(move |_: &MouseEvent| hovered_side.set(side_i8))
      .on_mouse_leave(move |_: &MouseEvent| {
        if hovered_side.get_untracked() == side_i8 {
          hovered_side.set(0);
        }
      })
      .claim_touches(TouchClaim::on_start())
      .keep_touches(true)
      .on_responder_grant(move |e: &ResponderEvent| {
        if !grips_enabled.get_untracked() {
          return;
        }
        let stored = width.get_untracked();
        let avail = room_sig.get_untracked();
        drag_start_x.set(f32::from(e.page.x));
        drag_start_rendered.set(rendered_column_width(stored, avail));
        live_width.set(Some(stored));
      })
      .on_responder_move(move |e: &ResponderEvent| {
        if live_width.get_untracked().is_none() {
          return;
        }
        let dx = f32::from(e.page.x) - drag_start_x.get_untracked();
        let stored = width.get_untracked();
        let start_r = drag_start_rendered.get_untracked();
        let avail = room_sig.get_untracked().max(1.0);
        let next = compute_drag_width(stored, start_r, avail, side, dx);
        live_width.set(Some(next));
      })
      .on_responder_release(move |e: &ResponderEvent| {
        let moved = (f32::from(e.page.x) - drag_start_x.get_untracked()).abs();
        let finished = live_width.get_untracked();
        live_width.set(None);

        if moved < 4.0 {
          let now = e.timestamp;
          let prev = last_click_ms.get_untracked();
          if prev > 0.0 && now >= prev && (now - prev) <= 350.0 {
            last_click_ms.set(0.0);
            if (width.get_untracked() - PAGE_WIDTH_DEFAULT).abs() > 0.5 {
              (commit_cb)(PAGE_WIDTH_DEFAULT);
            }
          } else {
            last_click_ms.set(now);
          }
          return;
        }

        last_click_ms.set(0.0);
        if let Some(target) = finished
          && (target - width.get_untracked()).abs() > 0.5
        {
          (commit_cb)(target);
        }
      })
      .on_responder_terminate(move |_: &ResponderEvent| {
        live_width.set(None);
      })
      .child(
        view()
          .w(px(2.0))
          .h_full()
          .bg(move || {
            let active =
              grips_enabled.get() && (live_width.get().is_some() || hovered_side.get() == side_i8);
            if active {
              palette(dark_mode.get()).border_strong
            } else {
              TRANSPARENT_RGBA
            }
          }),
      )
  };

  let left_grip = build_grip(-1.0, on_commit.clone());
  let right_grip = build_grip(1.0, on_commit);

  view()
    .test_id("page-width-frame")
    .relative()
    .flex_1()
    .w_full()
    .min_h(px(0.0))
    .overflow_hidden()
    .items_center()
    .on_layout(move |b: &Bounds<Points>| {
      let w = f32::from(b.size.width).max(0.0);
      if (outer_width.get_untracked() - w).abs() > 0.5 {
        outer_width.set(w);
      }
    })
    .child(
      view()
        .relative()
        .flex_1()
        .w_full()
        .min_h(px(0.0))
        .overflow_hidden()
        .max_w(move || {
          let eff = live_width.get().unwrap_or_else(|| width.get());
          if is_full_width(eff) {
            Length::from(relative(1.0))
          } else {
            let extra = if grips_enabled.get() {
              2.0 * PAGE_WIDTH_GRIP
            } else {
              0.0
            };
            Length::from(px(eff + extra))
          }
        })
        .flex_row()
        .items_stretch()
        .child(left_grip)
        .child(view().flex_1().min_w(px(0.0)).min_h(px(0.0)).overflow_hidden().child(child))
        .child(right_grip)
        .child(when(
          move || live_width.get().is_some(),
          move || {
            view()
              .absolute()
              .top(px(4.0))
              .left(px(0.0))
              .right(px(0.0))
              .items_center()
              .pointer_events(PointerEvents::None)
              .child(
                view()
                  .test_id("page-width-readout")
                  .bg(move || palette(dark_mode.get()).surface_overlay)
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).border_normal)
                  .rounded(px(6.0))
                  .px_2()
                  .py_1()
                  .child(
                    text(move || {
                      let lw = live_width.get().unwrap_or_else(|| width.get());
                      let avail = room_sig.get();
                      width_readout_label(lw, rendered_column_width(lw, avail))
                    })
                    .text_size(px(12.0))
                    .text_color(move || palette(dark_mode.get()).text_muted),
                  ),
              )
          },
        )),
    )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn full_width_sentinel() {
    assert!(!is_full_width(680.0));
    assert!(!is_full_width(1760.0));
    assert!(is_full_width(1880.0));
    assert!(is_full_width(2400.0));
  }

  #[test]
  fn snap_width_stops_and_clamps() {
    assert_eq!(snap_page_width(400.0), 680.0);
    assert_eq!(snap_page_width(680.0), 680.0);
    assert_eq!(snap_page_width(739.0), 680.0);
    assert_eq!(snap_page_width(741.0), 800.0);
    assert_eq!(snap_page_width(800.0), 800.0);
    assert_eq!(snap_page_width(920.0), 920.0);
    assert_eq!(snap_page_width(1880.0), 1880.0);
    assert_eq!(snap_page_width(2500.0), 1880.0);
  }

  #[test]
  fn drag_width_symmetric_deadzone_and_full_width() {
    // Small drag (< 60px width delta => < 30px pointer dx) preserves stored width.
    assert_eq!(compute_drag_width(800.0, 800.0, 1400.0, 1.0, 29.0), 800.0);
    assert_eq!(compute_drag_width(800.0, 800.0, 1400.0, -1.0, -29.0), 800.0);

    // Dragging right grip outward by +60px widens by +120px -> 920px.
    assert_eq!(compute_drag_width(800.0, 800.0, 1400.0, 1.0, 60.0), 920.0);
    // Dragging left grip outward by -60px widens by +120px -> 920px.
    assert_eq!(compute_drag_width(800.0, 800.0, 1400.0, -1.0, -60.0), 920.0);
    // Dragging right grip inward by -60px narrows by -120px -> 680px.
    assert_eq!(compute_drag_width(800.0, 800.0, 1400.0, 1.0, -60.0), 680.0);

    // Dragging past available room snaps to full-width sentinel (1880).
    assert_eq!(compute_drag_width(1040.0, 1040.0, 1200.0, 1.0, 100.0), PAGE_WIDTH_MAX);
    // Stored > room (e.g. 1520 stored, 1100 rendered): dragging inward starts from rendered 1100.
    assert_eq!(compute_drag_width(1520.0, 1100.0, 1100.0, 1.0, -90.0), 920.0);
  }

  #[test]
  fn readout_label_formatting() {
    assert_eq!(width_readout_label(800.0, 800.0), "800 px");
    assert_eq!(width_readout_label(1880.0, 1342.4), "全宽 · 1342 px");
    assert_eq!(width_readout_label(1880.0, 1200.0), "全宽 · 1200 px");
  }
}
