//! Mobile / touch-friendly word selection rules, multi-tap counter, draggable
//! teardrop selection handles, and compact action bar (1:1 Rust port of
//! `main:clients/mica_flutter/lib/editor/touch_selection.dart`, implementing
//! `CAP-16-06`).
//!
//! Why CJK uses a 1-2 character rule on touch:
//! Chinese / Japanese / Korean scripts have no spaces between words, so a
//! naive word-boundary scan grabs the entire clause up to the next punctuation.
//! Selecting the tapped character plus one neighbor (grapheme/UTF-16 safe)
//! makes touch long-press and double-tap predictable and easy to refine.

use std::{cell::RefCell, rc::Rc};

use mica_editor::{DocPosition, DocSelection, EditorCommand, utf16::utf16_len};
use nsh_base::button;
use nsh_core::{
  element::{ElementRef, ResponderEvent, TouchClaim, clipboard_string, set_clipboard, text_measurer},
  geometry::px,
  prelude::*,
};

use crate::{
  editor_bridge::{MicaDocumentBridge, effective_block_kind},
  mica_theme::palette,
  text_geometry::{VisibleLayoutCache, offset_at_point},
};

pub use mica_editor::interaction::{
  TouchTapTracker, is_atomic_selection_kind, touch_selected_slice, touch_word_bounds,
};

/// Deferred single-touch tap captured on `on_press_down` so scrolling or
/// long-press gestures never focus the hidden IME sink (which would pop up the
/// mobile virtual keyboard while the user tries to scroll).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingTouchTap {
  pub local_x: f32,
  pub local_y: f32,
  pub page_x: f32,
  pub page_y: f32,
  pub alt: bool,
}

/// Only a clean single touch tap on editable content (not multi-tap word/block
/// selection and not opening a link) should raise the mobile soft keyboard.
pub fn should_request_keyboard_focus_on_touch_tap(click_count: u32, opened_link: bool) -> bool {
  click_count == 1 && !opened_link
}

/// Normalized pointer snapshot consumed by [`LightboxGesture`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightboxTouchPt {
  pub id: u32,
  pub page: (f32, f32),
  pub location: (f32, f32),
  pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LightboxGestureMode {
  Idle,
  Pan {
    pointer_id: u32,
    start_page: (f32, f32),
    start_pan: (f32, f32),
  },
  Pinch {
    ids: (u32, u32),
    start_dist: f32,
    start_mid: (f32, f32),
    start_zoom: f32,
    start_pan: (f32, f32),
  },
}

/// Multi-touch pinch-zoom + single-pointer pan state machine for the image and
/// Mermaid fullscreen lightbox (`CAP-16-07`).
///
/// Tracks pointers by their stable `id` so two-finger pinch never feeds
/// alternating coordinates into a single-finger pan delta.
#[derive(Clone, Copy, Debug)]
pub struct LightboxGesture {
  mode: LightboxGestureMode,
}

impl Default for LightboxGesture {
  fn default() -> Self {
    Self::new()
  }
}

impl LightboxGesture {
  pub const MIN_ZOOM: f32 = 0.25;
  pub const MAX_ZOOM: f32 = 6.0;

  pub fn new() -> Self {
    Self {
      mode: LightboxGestureMode::Idle,
    }
  }

  pub fn reset(&mut self) {
    self.mode = LightboxGestureMode::Idle;
  }

  pub fn handle_touch(
    &mut self,
    phase: nsh_core::TouchPhase,
    points: &[LightboxTouchPt],
    canvas_size: (f32, f32),
    current_zoom: f32,
    current_pan: (f32, f32),
  ) -> Option<(f32, (f32, f32))> {
    let active: Vec<LightboxTouchPt> = match phase {
      nsh_core::TouchPhase::End | nsh_core::TouchPhase::Cancel => {
        points.iter().copied().filter(|p| !p.changed).collect()
      }
      _ => points.to_vec(),
    };

    if active.is_empty() {
      self.mode = LightboxGestureMode::Idle;
      return None;
    }

    if active.len() >= 2 {
      let a = active[0];
      let b = active[1];
      let dist = (a.page.0 - b.page.0).hypot(a.page.1 - b.page.1).max(8.0);
      let mid = (
        (a.location.0 + b.location.0) * 0.5,
        (a.location.1 + b.location.1) * 0.5,
      );

      let same_pair = match self.mode {
        LightboxGestureMode::Pinch { ids, .. } => {
          (ids.0 == a.id && ids.1 == b.id) || (ids.0 == b.id && ids.1 == a.id)
        }
        _ => false,
      };
      if !same_pair {
        self.mode = LightboxGestureMode::Pinch {
          ids: (a.id, b.id),
          start_dist: dist,
          start_mid: mid,
          start_zoom: current_zoom.max(0.01),
          start_pan: current_pan,
        };
        return None;
      }

      if let LightboxGestureMode::Pinch {
        start_dist,
        start_mid,
        start_zoom,
        start_pan,
        ..
      } = self.mode
      {
        let new_zoom = (start_zoom * (dist / start_dist)).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let ratio = new_zoom / start_zoom;
        let cx = canvas_size.0 * 0.5;
        let cy = canvas_size.1 * 0.5;
        // Keep the stage content point initially under `start_mid` pinned under `mid`.
        let pan_x = start_pan.0 * ratio + (mid.0 - cx) - (start_mid.0 - cx) * ratio;
        let pan_y = start_pan.1 * ratio + (mid.1 - cy) - (start_mid.1 - cy) * ratio;
        return Some((new_zoom, (pan_x, pan_y)));
      }
    }

    let p = active[0];
    match self.mode {
      LightboxGestureMode::Pan {
        pointer_id,
        start_page,
        start_pan,
      } if pointer_id == p.id => {
        let next_pan = (
          start_pan.0 + p.page.0 - start_page.0,
          start_pan.1 + p.page.1 - start_page.1,
        );
        Some((current_zoom, next_pan))
      }
      _ => {
        // Re-anchor cleanly whenever transitioning 0->1 or 2->1 fingers so lifting
        // one finger after pinch never produces a sudden jump.
        self.mode = LightboxGestureMode::Pan {
          pointer_id: p.id,
          start_page: p.page,
          start_pan: current_pan,
        };
        None
      }
    }
  }
}

/// Monotonic millisecond timestamp safe on both native desktop and `wasm32`.
pub fn now_ms() -> f64 {
  #[cfg(not(target_arch = "wasm32"))]
  {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
  }
  #[cfg(target_arch = "wasm32")]
  {
    js_sys::Date::now()
  }
}

/// Anchor coordinates `(edge_x, line_bottom_y)` in `paper_ref`-local pixels for
/// the start (left) and end (right) teardrop selection handles.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TouchHandleAnchors {
  pub start: Option<(f32, f32)>,
  pub end: Option<(f32, f32)>,
}

/// Snapshot of a mounted block row's geometry in `paper_ref`-local coordinates
/// taken at the start of a handle drag so `on_responder_move` hit-tests at 60fps
/// without awaiting async layout queries per frame.
#[derive(Clone, Debug)]
pub struct MeasuredBlockRow {
  pub block_idx: usize,
  pub block_id: String,
  pub non_prose: bool,
  pub row_top: f32,
  pub row_bottom: f32,
  pub text_left: f32,
  pub text_top: f32,
  pub text_right: f32,
  pub text_u16_len: u32,
}

/// Maps a target point `(target_x, target_y)` in `paper_ref`-local coordinates
/// (after scroll-delta adjustment) to the nearest [`DocPosition`] across visible
/// blocks — seamlessly crossing paragraphs, headings, lists, quotes, code blocks,
/// and atomic blocks.
pub fn resolve_doc_position_at_paper_point<F>(
  rows: &[MeasuredBlockRow],
  layouts: &VisibleLayoutCache,
  fixed_anchor: DocPosition,
  target_x: f32,
  target_y: f32,
  measure: &F,
) -> Option<DocPosition>
where
  F: Fn(&str, &str) -> Points,
{
  let first = rows.first()?;
  let last = rows.last()?;

  if target_y < first.row_top {
    return Some(DocPosition::new(first.block_idx, 0));
  }
  if target_y >= last.row_bottom {
    return Some(DocPosition::new(last.block_idx, last.text_u16_len));
  }

  // Find the row containing `target_y`, or the vertically closest row in inter-row gaps.
  let row = rows
    .iter()
    .find(|r| target_y >= r.row_top && target_y < r.row_bottom)
    .or_else(|| {
      rows.iter().min_by(|a, b| {
        let da = if target_y < a.row_top {
          a.row_top - target_y
        } else {
          target_y - a.row_bottom
        };
        let db = if target_y < b.row_top {
          b.row_top - target_y
        } else {
          target_y - b.row_bottom
        };
        da.total_cmp(&db)
      })
    })?;

  if row.non_prose {
    let off = if row.block_idx < fixed_anchor.node {
      0
    } else if row.block_idx > fixed_anchor.node {
      row.text_u16_len
    } else if target_x <= (row.text_left + row.text_right) * 0.5 {
      0
    } else {
      row.text_u16_len
    };
    return Some(DocPosition::new(row.block_idx, off));
  }

  let off = if let Some(info) = layouts.blocks.get(&row.block_id)
    && !info.lines.is_empty()
  {
    let local_x = target_x - row.text_left;
    let local_y = (target_y - row.text_top).max(0.0);
    if target_x <= row.text_left {
      offset_at_point(&info.lines, 0.0, local_y, &info.font, measure).min(row.text_u16_len)
    } else if row.text_right > row.text_left && target_x >= row.text_right {
      // Clamp to the end of the visual line at `local_y` so dragging to the
      // right margin always selects the final characters cleanly.
      offset_at_point(&info.lines, f32::MAX * 0.25, local_y, &info.font, measure)
        .min(row.text_u16_len)
    } else {
      offset_at_point(&info.lines, local_x, local_y, &info.font, measure).min(row.text_u16_len)
    }
  } else if row.block_idx < fixed_anchor.node {
    0
  } else {
    row.text_u16_len
  };

  Some(DocPosition::new(row.block_idx, off))
}

/// Diameter of each teardrop handle knob and its 44px finger-friendly grab box.
pub const TOUCH_HANDLE_KNOB: f32 = 18.0;
pub const TOUCH_HANDLE_GRAB: f32 = 44.0;
/// Vertical lift mapping a finger sitting on the teardrop below the line back up
/// to the center of the text line so grabbing a handle never jumps down a line.
const HANDLE_GRAB_LIFT_Y: f32 = 24.0;

#[derive(Clone)]
struct ActiveHandleDrag {
  fixed: DocPosition,
  paper_origin_page: (f32, f32),
  scroll_start_y: f32,
  grab_offset_x: f32,
  grab_offset_y: f32,
  rows: Vec<MeasuredBlockRow>,
}

/// Inputs required to render and drive the draggable start/end teardrop handles
/// on mobile Web and narrow touch viewports (`CAP-16-06`).
#[derive(Clone)]
pub struct TouchHandlesProps {
  pub bridge: Rc<MicaDocumentBridge>,
  pub layouts: Rc<RefCell<VisibleLayoutCache>>,
  pub paper_ref: ElementRef,
  pub paper_bounds: Signal<(f32, f32)>,
  pub scroll_offset_y: Signal<f32>,
  pub autoscroll_speed: Signal<f32>,
  pub bubble_recalc_rev: Signal<u64>,
  pub has_text_selection: Signal<bool>,
  pub drag_anchor: Signal<Option<DocPosition>>,
  pub touch_selection_active: Signal<bool>,
  pub touch_handle_dragging: Signal<bool>,
  pub touch_handle_anchors: Signal<TouchHandleAnchors>,
  pub dark_mode: Signal<bool>,
}

/// Renders the two draggable teardrop selection handles (`touch-handle-start`
/// and `touch-handle-end`) whenever a touch/narrow selection is active.
///
/// Each handle uses `TouchClaim::on_start().block_native(true)` + `keep_touches(true)`
/// so dragging a handle on iOS Safari / Android Chrome never gets hijacked by
/// browser vertical scrolling.
pub fn touch_selection_handles_layer(props: TouchHandlesProps) -> impl IntoElement {
  let drag_state: Rc<RefCell<Option<ActiveHandleDrag>>> = Rc::new(RefCell::new(None));
  let measure = Rc::new(text_measurer());

  let visible = computed(move || {
    if !props.has_text_selection.get() || props.drag_anchor.get().is_some() {
      return false;
    }
    let pw = props.paper_bounds.get().0;
    let narrow = pw > 0.0 && pw < 640.0;
    props.touch_selection_active.get() || narrow
  });

  let make_handle = move |is_start: bool| {
    let tid = if is_start {
      "touch-handle-start"
    } else {
      "touch-handle-end"
    };
    let label = if is_start {
      "拖动调整选区起点"
    } else {
      "拖动调整选区终点"
    };

    let br_start = props.bridge.clone();
    let br_move = props.bridge.clone();
    let layouts_start = props.layouts.clone();
    let layouts_move = props.layouts.clone();
    let drag_start = drag_state.clone();
    let drag_move = drag_state.clone();
    let drag_end_r = drag_state.clone();
    let drag_end_t = drag_state.clone();
    let measure_move = measure.clone();
    let pref_start = props.paper_ref;
    let anchors_sig = props.touch_handle_anchors;
    let paper_bounds = props.paper_bounds;
    let scroll_offset_y = props.scroll_offset_y;
    let autoscroll_speed = props.autoscroll_speed;
    let bubble_recalc_rev = props.bubble_recalc_rev;
    let dragging_sig = props.touch_handle_dragging;
    let dark_mode = props.dark_mode;

    let pos_sig = computed(move || {
      let a = anchors_sig.get();
      if is_start { a.start } else { a.end }
    });

    when(move || visible.get() && pos_sig.get().is_some(), move || {
      let br_s = br_start.clone();
      let br_m = br_move.clone();
      let lay_s = layouts_start.clone();
      let lay_m = layouts_move.clone();
      let ds = drag_start.clone();
      let dm = drag_move.clone();
      let der = drag_end_r.clone();
      let det = drag_end_t.clone();
      let mm = measure_move.clone();

      let finish_drag = move |slot: &Rc<RefCell<Option<ActiveHandleDrag>>>| {
        *slot.borrow_mut() = None;
        dragging_sig.set(false);
        autoscroll_speed.set(0.0);
        bubble_recalc_rev.update(|v| *v = v.wrapping_add(1));
      };
      let finish_r = finish_drag;
      let finish_t = finish_drag;

      let mut knob = view()
        .w(px(TOUCH_HANDLE_KNOB))
        .h(px(TOUCH_HANDLE_KNOB))
        .bg(move || palette(dark_mode.get()).accent)
        .border_1()
        .border_color(move || palette(dark_mode.get()).surface_base)
        .pointer_events(PointerEvents::None);

      if is_start {
        // Teardrop pointing top-right toward the selection start edge
        knob = knob
          .rounded_tl(px(TOUCH_HANDLE_KNOB * 0.5))
          .rounded_bl(px(TOUCH_HANDLE_KNOB * 0.5))
          .rounded_br(px(TOUCH_HANDLE_KNOB * 0.5))
          .rounded_tr(px(2.0));
      } else {
        // Teardrop pointing top-left toward the selection end edge
        knob = knob
          .rounded_tr(px(TOUCH_HANDLE_KNOB * 0.5))
          .rounded_br(px(TOUCH_HANDLE_KNOB * 0.5))
          .rounded_bl(px(TOUCH_HANDLE_KNOB * 0.5))
          .rounded_tl(px(2.0));
      }

      view()
        .test_id(tid)
        .label(label)
        .absolute()
        .left(move || {
          let (ex, _) = pos_sig.get().unwrap_or((0.0, 0.0));
          let x = if is_start {
            ex - TOUCH_HANDLE_GRAB
          } else {
            ex
          };
          px(x)
        })
        .top(move || {
          let (_, ey) = pos_sig.get().unwrap_or((0.0, 0.0));
          px((ey - 2.0).max(0.0))
        })
        .w(px(TOUCH_HANDLE_GRAB))
        .h(px(TOUCH_HANDLE_GRAB))
        .z_index(36)
        .items_start()
        .justify_start()
        .claim_touches(TouchClaim::on_start().block_native(true))
        .keep_touches(true)
        .on_responder_start(move |e: &ResponderEvent| {
          let Some(sel) = br_s.selection() else {
            return;
          };
          let fixed = if is_start {
            sel.end_pos()
          } else {
            sel.start_pos()
          };
          dragging_sig.set(true);

          // Approximate grab offset from handle anchor to touch point so initial
          // contact never jumps the selection edge onto the next line.
          let anchor_local = pos_sig.get_untracked().unwrap_or((0.0, 0.0));
          let scroll_start_y = scroll_offset_y.get_untracked();
          let start_page = (e.page.x.0, e.page.y.0);
          let handle_origin_local_x = if is_start {
            anchor_local.0 - TOUCH_HANDLE_GRAB
          } else {
            anchor_local.0
          };
          let handle_origin_local_y = (anchor_local.1 - 2.0).max(0.0);
          let approx_paper_origin = (
            start_page.0 - (handle_origin_local_x + e.location.x.0),
            start_page.1 - (handle_origin_local_y + e.location.y.0),
          );
          let grab_offset_x = (start_page.0 - approx_paper_origin.0) - anchor_local.0;
          let grab_offset_y = HANDLE_GRAB_LIFT_Y;

          *ds.borrow_mut() = Some(ActiveHandleDrag {
            fixed,
            paper_origin_page: approx_paper_origin,
            scroll_start_y,
            grab_offset_x,
            grab_offset_y,
            rows: Vec::new(),
          });

          // Snapshot mounted rows & text boxes asynchronously once at drag start.
          let row_refs = lay_s.borrow().snapshot_row_refs();
          let text_refs_map: std::collections::HashMap<String, ElementRef> =
            lay_s.borrow().text_refs.clone();
          let br_snap = br_s.clone();
          let ds_snap = ds.clone();
          spawn(async move {
            let Some(pm) = pref_start.measure().await else {
              return;
            };
            let mut measured = Vec::with_capacity(row_refs.len());
            for (bid, idx, rref) in row_refs {
              let Some(block) = br_snap.block_at(idx) else {
                continue;
              };
              let Some(rm) = rref.measure().await else {
                continue;
              };
              let row_top = rm.page.y.0 - pm.page.y.0;
              let row_bottom = row_top + rm.frame.size.height.0;
              if row_bottom <= row_top {
                continue;
              }
              let tm = if let Some(tref) = text_refs_map.get(&bid).copied() {
                tref.measure().await
              } else {
                None
              };
              let (text_left, text_top, text_right) = if let Some(t) = tm {
                let tl = t.page.x.0 - pm.page.x.0;
                let tt = t.page.y.0 - pm.page.y.0;
                (tl, tt, tl + t.frame.size.width.0)
              } else {
                let rl = rm.page.x.0 - pm.page.x.0;
                (rl + 24.0, row_top, rl + (rm.frame.size.width.0 - 24.0).max(24.0))
              };
              measured.push(MeasuredBlockRow {
                block_idx: idx,
                block_id: bid,
                non_prose: is_atomic_selection_kind(effective_block_kind(&block)),
                row_top,
                row_bottom,
                text_left,
                text_top,
                text_right,
                text_u16_len: utf16_len(&block.text),
              });
            }
            measured.sort_by_key(|r| r.block_idx);
            if let Some(active) = ds_snap.borrow_mut().as_mut() {
              active.paper_origin_page = (pm.page.x.0, pm.page.y.0);
              active.rows = measured;
            }
          });
        })
        .on_responder_move(move |e: &ResponderEvent| {
          let state_opt = dm.borrow().clone();
          let Some(state) = state_opt else {
            return;
          };
          let scroll_delta = scroll_offset_y.get_untracked() - state.scroll_start_y;
          let local_x = e.page.x.0 - state.paper_origin_page.0 - state.grab_offset_x;
          let local_y_vp = e.page.y.0 - state.paper_origin_page.1 - state.grab_offset_y;
          let target_y = local_y_vp + scroll_delta;

          let (_, vh) = paper_bounds.get_untracked();
          autoscroll_speed.set(crate::editor_positioning::edge_autoscroll_speed(
            e.page.y.0 - state.paper_origin_page.1,
            vh,
          ));

          if state.rows.is_empty() {
            return;
          }
          // Drop the `VisibleLayoutCache` read borrow BEFORE `set_selection`,
          // because `set_selection` synchronously triggers reactive overlays
          // and effects that read/update layout caches.
          let focus_pos = {
            let cache = lay_m.borrow();
            resolve_doc_position_at_paper_point(
              &state.rows,
              &cache,
              state.fixed,
              local_x,
              target_y,
              &|f, t| mm(f, t),
            )
          };
          if let Some(focus_pos) = focus_pos {
            let next = DocSelection::from_positions(state.fixed, focus_pos);
            if br_m.selection().as_ref() != Some(&next) {
              br_m.set_selection(next);
            }
          }
        })
        .on_responder_release(move |_: &ResponderEvent| finish_r(&der))
        .on_responder_terminate(move |_: &ResponderEvent| finish_t(&det))
        .child(
          view()
            .w_full()
            .h_full()
            .pt(px(2.0))
            .flex_row()
            .justify_start()
            .items_start()
            .pointer_events(PointerEvents::None)
            .child(if is_start {
              view()
                .w_full()
                .flex_row()
                .justify_end()
                .child(knob)
            } else {
              view()
                .w_full()
                .flex_row()
                .justify_start()
                .child(knob)
            }),
        )
    })
  };

  view()
    .test_id("touch-selection-handles-layer")
    .absolute()
    .inset_0()
    .pointer_events(PointerEvents::BoxNone)
    .child(make_handle(true))
    .child(make_handle(false))
}

/// Compact touch / narrow-viewport selection action bar (`剪切 / 复制 / 粘贴 / 全选`,
/// `CAP-16-06`). Uses generous 34px hit targets so fingers can tap accurately
/// without triggering accidental drag or horizontal overflow.
pub fn touch_selection_bar_view(
  bridge: Rc<MicaDocumentBridge>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let b_cut = bridge.clone();
  let b_copy = bridge.clone();
  let b_paste = bridge.clone();
  let b_all = bridge;

  let action_btn = move |tid: &'static str, label: &'static str, on_act: Rc<dyn Fn()>| {
    button()
      .test_id(tid)
      .h(px(34.0))
      .px_3()
      .items_center()
      .justify_center()
      .rounded(px(6.0))
      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
      .on_press(move || on_act())
      .child(
        text(label)
          .text_size(px(13.0))
          .font_weight(FontWeight::MEDIUM)
          .text_color(move || palette(dark_mode.get()).text_primary),
      )
  };

  view()
    .test_id("touch-selection-bar")
    .h(px(40.0))
    .px(px(4.0))
    .flex_row()
    .items_center()
    .gap(px(2.0))
    .rounded(px(10.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .child(action_btn(
      "touch-sel-cut",
      "剪切",
      Rc::new(move || {
        if let Some(payload) = b_cut.copy_selection() {
          set_clipboard(payload.plain);
          b_cut.dispatch(EditorCommand::DeleteBackward);
          b_cut.set_status_flash("已剪切所选内容");
        }
      }),
    ))
    .child(action_btn(
      "touch-sel-copy",
      "复制",
      Rc::new(move || {
        if let Some(payload) = b_copy.copy_selection() {
          set_clipboard(payload.plain);
          b_copy.clear_selection();
          b_copy.set_status_flash("已复制所选内容");
        }
      }),
    ))
    .child(action_btn(
      "touch-sel-paste",
      "粘贴",
      Rc::new(move || {
        let br = b_paste.clone();
        spawn(async move {
          let s = clipboard_string().await;
          if !s.is_empty() {
            br.paste_clipboard(&s, "", false);
          }
        });
      }),
    ))
    .child(action_btn(
      "touch-sel-all",
      "全选",
      Rc::new(move || {
        b_all.select_all();
      }),
    ))
}

#[cfg(test)]
mod tests {
  use super::*;
  use nsh_core::TextLine;

  fn fixed_measure(_font: &str, text: &str) -> Points {
    let w: f32 = text
      .chars()
      .map(|ch| if ch.is_ascii() { 8.0 } else { 16.0 })
      .sum();
    Points(w)
  }

  #[test]
  fn touch_word_bounds_handles_cjk_latin_punctuation_and_emoji() {
    // CJK mid-run selects 2 chars (not the entire sentence!)
    let s = "泰国算力交付清单，共32台";
    assert_eq!(touch_selected_slice(s, 0), "泰国");
    assert_eq!(touch_selected_slice(s, 2), "算力");
    // End of CJK run grabs the previous wide char too
    assert_eq!(touch_selected_slice(s, 7), "清单");
    // Full-width punctuation stands alone
    assert_eq!(touch_selected_slice(s, 8), "，");
    // Isolated single CJK char stays 1 char
    assert_eq!(touch_selected_slice(s, 9), "共");
    // ASCII number grabs whole run
    assert_eq!(touch_selected_slice(s, 10), "32");
    // Latin word grabs full word
    let en = "hello world";
    assert_eq!(touch_selected_slice(en, 1), "hello");
    // Emoji surrogate pair is never split
    let em = "🚀火箭";
    assert_eq!(touch_selected_slice(em, 0), "🚀");
    assert_eq!(touch_selected_slice(em, 1), "🚀");
    assert_eq!(touch_selected_slice(em, 2), "火箭");
  }

  #[test]
  fn touch_tap_tracker_escalates_double_and_triple_tap_within_window_and_slop() {
    let mut tracker = TouchTapTracker::default();
    assert_eq!(tracker.bump(100.0, 200.0, 1000.0), 1);
    assert_eq!(tracker.bump(110.0, 205.0, 1250.0), 2);
    assert_eq!(tracker.bump(108.0, 202.0, 1500.0), 3);
    // Wraps cleanly after 3
    assert_eq!(tracker.bump(108.0, 202.0, 1700.0), 1);
    // Far tap resets to 1
    assert_eq!(tracker.bump(200.0, 202.0, 1800.0), 1);
    // Slow tap (>400ms) resets to 1
    assert_eq!(tracker.bump(202.0, 202.0, 2300.0), 1);
  }

  #[test]
  fn resolve_doc_position_crosses_prose_code_block_and_atomic_blocks_without_stalling() {
    let mut layouts = VisibleLayoutCache::default();
    layouts.update_lines(
      "p0",
      vec![TextLine {
        text: "hello world".into(),
        start: 0,
        end: 11,
        x: Points(0.0),
        y: Points(0.0),
        width: Points(88.0),
        height: Points(24.0),
        ascender: Points(16.0),
        descender: Points(4.0),
        cap_height: Points(12.0),
        x_height: Points(8.0),
      }],
      "16px sans-serif".into(),
      24.0,
    );
    layouts.update_lines(
      "c1",
      vec![
        TextLine {
          text: "let x = 1;\n".into(),
          start: 0,
          end: 11,
          x: Points(0.0),
          y: Points(0.0),
          width: Points(80.0),
          height: Points(22.0),
          ascender: Points(14.0),
          descender: Points(4.0),
          cap_height: Points(10.0),
          x_height: Points(7.0),
        },
        TextLine {
          text: "println!(x);".into(),
          start: 11,
          end: 23,
          x: Points(0.0),
          y: Points(22.0),
          width: Points(96.0),
          height: Points(22.0),
          ascender: Points(14.0),
          descender: Points(4.0),
          cap_height: Points(10.0),
          x_height: Points(7.0),
        },
      ],
      "14px monospace".into(),
      22.0,
    );
    layouts.update_lines(
      "p2",
      vec![TextLine {
        text: "tail line".into(),
        start: 0,
        end: 9,
        x: Points(0.0),
        y: Points(0.0),
        width: Points(72.0),
        height: Points(24.0),
        ascender: Points(16.0),
        descender: Points(4.0),
        cap_height: Points(12.0),
        x_height: Points(8.0),
      }],
      "16px sans-serif".into(),
      24.0,
    );

    let rows = vec![
      MeasuredBlockRow {
        block_idx: 0,
        block_id: "p0".into(),
        non_prose: false,
        row_top: 0.0,
        row_bottom: 30.0,
        text_left: 40.0,
        text_top: 3.0,
        text_right: 340.0,
        text_u16_len: 11,
      },
      MeasuredBlockRow {
        block_idx: 1,
        block_id: "c1".into(),
        non_prose: false,
        row_top: 34.0,
        row_bottom: 110.0,
        text_left: 54.0,
        text_top: 54.0,
        text_right: 326.0,
        text_u16_len: 23,
      },
      MeasuredBlockRow {
        block_idx: 2,
        block_id: "p2".into(),
        non_prose: false,
        row_top: 114.0,
        row_bottom: 144.0,
        text_left: 40.0,
        text_top: 117.0,
        text_right: 340.0,
        text_u16_len: 9,
      },
    ];

    let anchor = DocPosition::new(0, 2);
    // Mid code block second line: character-level selection inside code block
    let in_code = resolve_doc_position_at_paper_point(&rows, &layouts, anchor, 54.0 + 32.0, 54.0 + 30.0, &fixed_measure)
      .expect("hits code block");
    assert_eq!(in_code, DocPosition::new(1, 15));

    // Past right edge of tail paragraph: selects full line end (never drops trailing chars)
    let tail_end = resolve_doc_position_at_paper_point(&rows, &layouts, anchor, 380.0, 128.0, &fixed_measure)
      .expect("hits tail end");
    assert_eq!(tail_end, DocPosition::new(2, 9));

    // Below last block: clamps to end of last block
    let below = resolve_doc_position_at_paper_point(&rows, &layouts, anchor, 100.0, 220.0, &fixed_measure)
      .expect("below clamps to end");
    assert_eq!(below, DocPosition::new(2, 9));
  }

  #[test]
  fn pending_touch_tap_only_focuses_ime_on_clean_single_tap_not_link_or_multitap() {
    assert!(should_request_keyboard_focus_on_touch_tap(1, false));
    assert!(!should_request_keyboard_focus_on_touch_tap(1, true));
    assert!(!should_request_keyboard_focus_on_touch_tap(2, false));
    assert!(!should_request_keyboard_focus_on_touch_tap(3, false));
    assert!(!should_request_keyboard_focus_on_touch_tap(0, false));
  }

  #[test]
  fn lightbox_gesture_handles_pan_pinch_interleaved_moves_and_lift_without_jumps() {
    use nsh_core::TouchPhase;
    let mut g = LightboxGesture::new();
    let canvas = (800.0, 600.0);

    let pt = |id: u32, px: f32, py: f32, changed: bool| LightboxTouchPt {
      id,
      page: (px, py),
      location: (px - 100.0, py - 50.0), // arbitrary offset stage origin (100, 50)
      changed,
    };

    // 1. Single-finger start + pan
    assert_eq!(
      g.handle_touch(TouchPhase::Start, &[pt(1, 500.0, 350.0, true)], canvas, 1.0, (0.0, 0.0)),
      None
    );
    let (z1, p1) = g
      .handle_touch(TouchPhase::Move, &[pt(1, 540.0, 330.0, true)], canvas, 1.0, (0.0, 0.0))
      .expect("single-finger move pans");
    assert_eq!(z1, 1.0);
    assert!((p1.0 - 40.0).abs() < 1e-4 && (p1.1 + 20.0).abs() < 1e-4);

    // 2. Second finger lands -> enters Pinch with zero jump
    assert_eq!(
      g.handle_touch(
        TouchPhase::Start,
        &[pt(1, 540.0, 330.0, false), pt(2, 640.0, 330.0, true)],
        canvas,
        z1,
        p1,
      ),
      None
    );

    // 3. Interleaved moves: first only finger 1 reports moved (distance doubles 100 -> 200),
    // then only finger 2 reports moved (still 200 apart). Must NOT jump back and forth!
    let (z2, p2) = g
      .handle_touch(
        TouchPhase::Move,
        &[pt(1, 440.0, 330.0, true), pt(2, 640.0, 330.0, false)],
        canvas,
        z1,
        p1,
      )
      .expect("pinch move updates zoom");
    assert!((z2 - 2.0).abs() < 1e-4, "expected 2x zoom, got {z2}");

    // Interleaved finger 2 move that leaves distance at 200px around shifted midpoint
    let (z3, p3) = g
      .handle_touch(
        TouchPhase::Move,
        &[pt(1, 440.0, 330.0, false), pt(2, 640.0, 330.0, true)],
        canvas,
        z2,
        p2,
      )
      .expect("stable two-finger hold keeps zoom and pan");
    assert!((z3 - 2.0).abs() < 1e-4);
    assert!((p3.0 - p2.0).abs() < 1e-4 && (p3.1 - p2.1).abs() < 1e-4);

    // 4. Off-center pinch preserves focus point: zoom centered at stage mid stays pinned
    let mut g2 = LightboxGesture::new();
    let mid_stage_a = (450.0, 350.0); // stage coords (350, 300), canvas center = (400, 300) => -50 on x
    let mid_stage_b = (550.0, 350.0);
    assert_eq!(
      g2.handle_touch(
        TouchPhase::Start,
        &[
          LightboxTouchPt { id: 1, page: mid_stage_a, location: (350.0, 300.0), changed: false },
          LightboxTouchPt { id: 2, page: mid_stage_b, location: (450.0, 300.0), changed: true },
        ],
        canvas,
        1.0,
        (0.0, 0.0),
      ),
      None
    );
    // Double separation around stage center (400, 300): start_mid=(400,300), so pan stays (0,0)
    let (zc, pc) = g2
      .handle_touch(
        TouchPhase::Move,
        &[
          LightboxTouchPt { id: 1, page: (400.0, 350.0), location: (300.0, 300.0), changed: true },
          LightboxTouchPt { id: 2, page: (600.0, 350.0), location: (500.0, 300.0), changed: false },
        ],
        canvas,
        1.0,
        (0.0, 0.0),
      )
      .unwrap();
    assert!((zc - 2.0).abs() < 1e-4);
    assert!(pc.0.abs() < 1e-4 && pc.1.abs() < 1e-4);

    // 5. Lifting finger 2 after pinch re-anchors to finger 1 cleanly (None on lift), then pans smoothly
    assert_eq!(
      g.handle_touch(
        TouchPhase::End,
        &[pt(1, 440.0, 330.0, false), pt(2, 640.0, 330.0, true)],
        canvas,
        z3,
        p3,
      ),
      None
    );
    let (z4, p4) = g
      .handle_touch(TouchPhase::Move, &[pt(1, 460.0, 345.0, true)], canvas, z3, p3)
      .expect("single finger resumes panning cleanly from p3");
    assert!((z4 - z3).abs() < 1e-4);
    assert!((p4.0 - (p3.0 + 20.0)).abs() < 1e-4);
    assert!((p4.1 - (p3.1 + 15.0)).abs() < 1e-4);

    // 6. Pinch clamp check (min 0.25, max 6.0)
    let (_, _) = (0.25_f32, 6.0_f32);
    let (z_max, _) = g2
      .handle_touch(
        TouchPhase::Move,
        &[
          LightboxTouchPt { id: 1, page: (0.0, 350.0), location: (-100.0, 300.0), changed: true },
          LightboxTouchPt { id: 2, page: (2000.0, 350.0), location: (1900.0, 300.0), changed: false },
        ],
        canvas,
        2.0,
        pc,
      )
      .unwrap();
    assert_eq!(z_max, LightboxGesture::MAX_ZOOM);

    // 7. Lifting final finger returns Idle
    assert_eq!(
      g.handle_touch(TouchPhase::End, &[pt(1, 460.0, 345.0, true)], canvas, z4, p4),
      None
    );
  }
}
