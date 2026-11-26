//! Feishu-Board-inspired infinite whiteboard, mind map, and flowchart editor backed
//! directly by `mica_board::BoardSession` and rendered via `nsh_core::canvas::Painter`.
//!
//! Layout & chrome parity (`board_editor_screen.dart` / `board_chrome.dart` / `board_painter.dart`):
//! - 50px top bar: editable title, Undo/Redo, Auto Layout, Import (draw.io/JSON & Markdown outline),
//!   Export (SVG / Markdown / JSON), node/edge count, primary `#3370FF` Done button.
//! - Left vertical 10-tool dock (`left: 14, top: 64`) with 12-shape & 20-tech-icon flyouts.
//! - Top-center contextual floating inspector for selected node(s) or edge.
//! - Bottom-right zoom pill (`-`, `{zoom}%`, `+`, `Fit`) & bottom-left shortcut hint pill.
//! - World-anchored inline node editor, edge label modal, Markdown outline import modal,
//!   unified `BoardCommand::KeyDown` + `UiEffect` loop, and 3-way semantic merge helper.

use std::{
  cell::RefCell,
  collections::{HashMap, HashSet},
  f32::consts::{PI, TAU},
  rc::Rc,
};

use mica_board::{
  BoardCommand, BoardEdge, BoardNode, BoardPatch, BoardSession, BoardSnapshot, BoardSvgTheme,
  BoardTool, PendingConnector, SnapGuide, UiEffect,
  session::{BOARD_SHAPES, BOARD_TECH_ICONS, GestureState},
  tech_icon_svg_inner,
};
use nsh_base::{button, input};
#[cfg(not(target_arch = "wasm32"))]
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::{
  canvas::{CanvasTextAlign, CanvasTextBaseline, LineCap, LineJoin},
  color::Rgba,
  element::{ImageSrc, KeyOptions, Svg, on_key_with, set_clipboard, svg},
  geometry::{point, px, radians, size},
  keymap::When,
  prelude::*,
};

use crate::{atomic_blocks::board_svg_theme, mica_icons};

// ============================================================================
// Exact Feishu-Inspired Palette (`BoardColors` from `board_models.dart`)
// ============================================================================

const fn rgb_const(hex: u32) -> Rgba { rgba_const(hex, 1.0) }
const fn rgba_const(hex: u32, a: f32) -> Rgba {
  Rgba { r: ((hex >> 16) & 0xff) as f32 / 255.0, g: ((hex >> 8) & 0xff) as f32 / 255.0, b: (hex & 0xff) as f32 / 255.0, a }
}

mod bc {
  use super::{Rgba, rgb_const, rgba_const};

  pub const BRAND: Rgba = rgb_const(0x3370FF);
  pub const BRAND_WASH_LIGHT: Rgba = rgba_const(0x2383E2, 0.14);
  pub const BRAND_WASH_DARK: Rgba = rgba_const(0x3B82F6, 0.22);
  pub const BRAND_MARQUEE: Rgba = rgba_const(0x2383E2, 0.10);
  pub const GUIDE: Rgba = rgba_const(0x3370FF, 0.65);
  pub const DANGER: Rgba = rgb_const(0xCF222E);
  pub const INK_LIGHT: Rgba = rgb_const(0x1F2329);
  pub const INK_DARK: Rgba = rgb_const(0xE6E9EF);
  pub const MUTED_LIGHT: Rgba = rgb_const(0x646A73);
  pub const MUTED_DARK: Rgba = rgb_const(0x9CA3AF);
  pub const BORDER_LIGHT: Rgba = rgb_const(0xE5E7EB);
  pub const BORDER_DARK: Rgba = rgb_const(0x343944);
  pub const DOT_LIGHT: Rgba = rgb_const(0xD5DBE3);
  pub const DOT_DARK: Rgba = rgb_const(0x343944);
  pub const CANVAS_LIGHT: Rgba = rgb_const(0xF7F8FA);
  pub const CANVAS_DARK: Rgba = rgb_const(0x16191E);
  pub const PANEL_LIGHT: Rgba = rgb_const(0xFFFFFF);
  pub const PANEL_DARK: Rgba = rgb_const(0x1F2329);
  pub const CHIP_LIGHT: Rgba = rgb_const(0xF0F2F5);
  pub const CHIP_DARK: Rgba = rgb_const(0x2A2F38);
  pub const SURFACE_DARK: Rgba = rgb_const(0x252830);
  pub const DEFAULT_STROKE: Rgba = rgb_const(0x8C959F);

  pub fn canvas(d: bool) -> Rgba { if d { CANVAS_DARK } else { CANVAS_LIGHT } }
  pub fn panel(d: bool) -> Rgba { if d { PANEL_DARK } else { PANEL_LIGHT } }
  pub fn border(d: bool) -> Rgba { if d { BORDER_DARK } else { BORDER_LIGHT } }
  pub fn ink(d: bool) -> Rgba { if d { INK_DARK } else { INK_LIGHT } }
  pub fn muted(d: bool) -> Rgba { if d { MUTED_DARK } else { MUTED_LIGHT } }
  pub fn chip(d: bool) -> Rgba { if d { CHIP_DARK } else { CHIP_LIGHT } }
  pub fn icon_hex(d: bool) -> &'static str { if d { "#D0D5DD" } else { "#374151" } }
  pub fn brand_wash(d: bool) -> Rgba { if d { BRAND_WASH_DARK } else { BRAND_WASH_LIGHT } }
}

const FILL_SWATCHES: &[(&str, &str, Rgba)] = &[
  ("#FFFFFF", "board-fill-white", rgb_const(0xFFFFFF)),
  ("#E8F3FF", "board-fill-blue", rgb_const(0xE8F3FF)),
  ("#EDF7ED", "board-fill-green", rgb_const(0xEDF7ED)),
  ("#FFF7CC", "board-fill-yellow", rgb_const(0xFFF7CC)),
  ("#FFF7E6", "board-fill-orange", rgb_const(0xFFF7E6)),
  ("#F5F0FF", "board-fill-purple", rgb_const(0xF5F0FF)),
  ("#FFF0F0", "board-fill-red", rgb_const(0xFFF0F0)),
  ("#E6F7F7", "board-fill-teal", rgb_const(0xE6F7F7)),
];

const STROKE_SWATCHES: &[(&str, &str, Rgba)] = &[
  ("#646A73", "board-stroke-gray", rgb_const(0x646A73)),
  ("#3370FF", "board-stroke-blue", rgb_const(0x3370FF)),
  ("#2DA44E", "board-stroke-green", rgb_const(0x2DA44E)),
  ("#D9822B", "board-stroke-orange", rgb_const(0xD9822B)),
  ("#8250DF", "board-stroke-purple", rgb_const(0x8250DF)),
  ("#CF222E", "board-stroke-red", rgb_const(0xCF222E)),
];

const DEFAULT_MD_OUTLINE: &str =
  "# 中心主题\n- 核心模块\n  - 数据模型\n  - 渲染引擎\n- 交付验收\n  - 自动化测试\n";

// ============================================================================
// Public 3-Way Merge Helpers & View Entry Points
// ============================================================================

/// 3-way semantic merge of board JSON snapshots (preserves concurrent local and
/// remote edits when saving embedded whiteboard blocks back into a document).
pub fn merge_board_snapshot_3way(base_json: &str, local_json: &str, remote_json: &str) -> String {
  mica_board::merge_board_json(base_json, local_json, remote_json)
}

/// Apply a 3-way semantic merge between `base_json`, the current `session`
/// state, and `remote_json`. Returns `true` if the session snapshot changed.
pub fn merge_session_with_remote(
  session: &mut BoardSession,
  base_json: &str,
  remote_json: &str,
) -> bool {
  let local_json = session.export_compact_json();
  let merged_json = mica_board::merge_board_json(base_json, &local_json, remote_json);
  if merged_json == local_json {
    return false;
  }
  if let Ok(snapshot) = BoardSnapshot::parse_json(&merged_json) {
    session.dispatch(BoardCommand::ReplaceSnapshot { snapshot, record_undo: false });
    return true;
  }
  false
}

/// Backward-compatible whiteboard view entry point used by `lib.rs`.
pub fn board_view(
  session: Rc<RefCell<BoardSession>>,
  rev: Signal<u64>,
  dark_mode: Signal<bool>,
  on_persist_board: Rc<dyn Fn(String)>,
) -> impl IntoElement {
  board_view_with_rev_and_close(session, rev, dark_mode, on_persist_board, None)
}

/// Full Feishu-style whiteboard view with `on_change` notification and optional
/// `on_close` callback (invoked by `#3370FF` `完成` and idle `Escape`).
pub fn board_view_with_close(
  session: Rc<RefCell<BoardSession>>,
  dark_mode: Signal<bool>,
  on_change: Rc<dyn Fn()>,
  on_close: Option<Rc<dyn Fn()>>,
) -> impl IntoElement {
  let rev = signal(0u64);
  let on_persist: Rc<dyn Fn(String)> = Rc::new(move |_| on_change());
  board_view_with_rev_and_close(session, rev, dark_mode, on_persist, on_close)
}

/// Whiteboard view accepting external `rev`, JSON persistence callback, and
/// optional `on_close` callback.
pub fn board_view_with_rev_and_close(
  session: Rc<RefCell<BoardSession>>,
  rev: Signal<u64>,
  dark_mode: Signal<bool>,
  on_persist_board: Rc<dyn Fn(String)>,
  on_close: Option<Rc<dyn Fn()>>,
) -> impl IntoElement {
  let (initial_title, initial_mode, initial_tool, initial_undo, initial_redo, initial_counts, initial_zoom, initial_insp) = {
    let s = session.borrow();
    let snap = s.snapshot();
    (
      snap.title.clone(),
      snap.mode.clone(),
      s.tool(),
      s.can_undo(),
      s.can_redo(),
      (snap.nodes.len(), snap.edges.len()),
      (s.zoom() * 100.0).round() as i32,
      compute_inspector_state(&s, false),
    )
  };
  let ctx = BoardCtx {
    session,
    rev,
    dark_mode,
    tool_sig: signal(initial_tool),
    board_mode_sig: signal(initial_mode),
    can_undo_sig: signal(initial_undo),
    can_redo_sig: signal(initial_redo),
    counts_sig: signal(initial_counts),
    zoom_pct_sig: signal(initial_zoom),
    inspector_state: signal(initial_insp),
    title_draft: signal(initial_title),
    editing_node_id: signal(None),
    editing_draft: signal(String::new()),
    editing_edge_id: signal(None),
    editing_edge_draft: signal(String::new()),
    show_shape_panel: signal(false),
    show_icon_panel: signal(false),
    show_import_menu: signal(false),
    show_export_menu: signal(false),
    show_md_import_modal: signal(false),
    md_import_draft: signal(DEFAULT_MD_OUTLINE.to_string()),
    active_shape_kind: signal("rounded".to_string()),
    active_icon_id: signal("server".to_string()),
    toast_msg: signal(None),
    last_svg_bytes: signal(0),
    viewport_size: signal((1040.0, 680.0)),
    did_initial_fit: signal(false),
    space_held: signal(false),
    last_move_ms: Rc::new(std::cell::Cell::new(0)),
    #[cfg(not(target_arch = "wasm32"))]
    mounted_at: std::time::Instant::now(),
    on_persist: on_persist_board,
    on_close,
  };

  register_board_shortcuts(&ctx);

  let (space_down, space_up, toast_sig) = (ctx.space_held, ctx.space_held, ctx.toast_msg);
  let (c_paint, c_layout, c_pdown) = (ctx.clone(), ctx.clone(), ctx.clone());
  let (c_pmove, c_pup, c_root_up, c_wheel) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());
  let (c_inline_cond, c_inline_body, c_insp) = (ctx.clone(), ctx.clone(), ctx.clone());
  let (c_edge_cond, c_edge_body) = (ctx.clone(), ctx.clone());
  let (c_md_cond, c_md_body) = (ctx.clone(), ctx.clone());
  let (c_imp_cond, c_imp_body) = (ctx.clone(), ctx.clone());
  let (c_exp_cond, c_exp_body) = (ctx.clone(), ctx.clone());

  view()
    .flex_1().w_full().h_full().relative().overflow_hidden()
    .bg(move || bc::canvas(dark_mode.get()))
    .on_mouse_up(move |_: &MouseEvent| c_root_up.finish_gesture_if_active())
    .on_key_down(move |ev: &KeyEvent| {
      if ev.key == " " || ev.code == "Space" { space_down.set(true); }
    })
    .on_key_up(move |ev: &KeyEvent| {
      if ev.key == " " || ev.code == "Space" { space_up.set(false); }
    })
    .child(build_top_bar(&ctx))
    .child(
      view()
        .flex_1().w_full().relative().overflow_hidden()
        .child(
          canvas(move || {
            c_paint.rev.get();
            paint_board(&c_paint.session.borrow(), c_paint.dark_mode.get(), c_paint.viewport_size.get())
          })
          .test_id("board-canvas")
          .on_layout(move |b: &Bounds<Points>| {
            let w = f32::from(b.size.width).max(320.0);
            let h = f32::from(b.size.height).max(240.0);
            let (pw, ph) = c_layout.viewport_size.get_untracked();
            if (pw - w).abs() > 1.0 || (ph - h).abs() > 1.0 {
              c_layout.viewport_size.set((w, h));
              c_layout.session.borrow_mut().dispatch(BoardCommand::SetViewportSize { width: w as f64, height: h as f64 });
            }
            if !c_layout.did_initial_fit.get_untracked() && w > 80.0 && h > 80.0 {
              c_layout.did_initial_fit.set(true);
              c_layout.dispatch_and_handle(BoardCommand::FitToContent { viewport: Some([w as f64, h as f64]) });
            }
          })
          .on_press_down(move |ev: &PressEvent| {
            c_pdown.close_popups();
            let patch = c_pdown.dispatch_and_handle(BoardCommand::PointerDown {
              screen_x: f32::from(ev.position.x) as f64,
              screen_y: f32::from(ev.position.y) as f64,
              button: 0,
              shift: ev.modifiers.shift,
              alt: ev.modifiers.alt,
              space: c_pdown.space_held.get_untracked(),
              read_only: false,
              timestamp_ms: Some(c_pdown.now_ms()),
            });
            if ev.clicks >= 2 && patch.ui_effects.is_empty() {
              let (sel_node, sel_edge) = {
                let s = c_pdown.session.borrow();
                (s.selected_node_ids().first().cloned(), s.selected_edge_id().map(str::to_owned))
              };
              if let Some(nid) = sel_node {
                c_pdown.dispatch_and_handle(BoardCommand::StartEditingNode { node_id: nid });
              } else if let Some(eid) = sel_edge {
                let lbl = c_pdown.session.borrow().snapshot().edges.iter()
                  .find(|e| e.id == eid).map(|e| e.label.clone()).unwrap_or_default();
                c_pdown.finish_gesture_if_active();
                c_pdown.editing_edge_draft.set(lbl);
                c_pdown.editing_edge_id.set(Some(eid));
                c_pdown.sync_chrome();
              }
            }
          })
          .on_mouse_move(move |ev: &MouseEvent| {
            let (sx, sy) = (f32::from(ev.position.x) as f64, f32::from(ev.position.y) as f64);
            if let Some(btn) = ev.pressed_button {
              let is_idle = matches!(c_pmove.session.borrow().gesture(), GestureState::Idle);
              if is_idle {
                if btn == MouseButton::Middle {
                  c_pmove.dispatch_and_handle(BoardCommand::PointerDown {
                    screen_x: sx, screen_y: sy, button: 1, shift: ev.modifiers.shift,
                    alt: ev.modifiers.alt, space: false, read_only: false, timestamp_ms: None,
                  });
                }
                return;
              }
              let now = c_pmove.now_ms();
              let prev = c_pmove.last_move_ms.get();
              if now > prev && now - prev < 8 {
                return;
              }
              c_pmove.last_move_ms.set(now);
              let changed = {
                let mut s = c_pmove.session.borrow_mut();
                let before = s.active_rev();
                s.dispatch(BoardCommand::PointerMove { screen_x: sx, screen_y: sy, alt: ev.modifiers.alt });
                s.active_rev() != before
              };
              if changed {
                c_pmove.bump();
              }
            } else {
              // Auto-recover if mouse button was released outside window or over a floating overlay.
              c_pmove.finish_gesture_if_active();
              let changed = {
                let mut s = c_pmove.session.borrow_mut();
                let before = s.active_rev();
                s.dispatch(BoardCommand::PointerHover { screen_x: sx, screen_y: sy });
                s.active_rev() != before
              };
              if changed {
                c_pmove.bump();
              }
            }
          })
          .on_mouse_up(move |_: &MouseEvent| c_pup.finish_gesture_if_active())
          .on_wheel(move |ev: &WheelEvent| {
            let dy = match ev.delta {
              WheelDelta::Points(pt) => f32::from(pt.y),
              WheelDelta::Lines(pt) => pt.y * 24.0,
            };
            c_wheel.dispatch_and_handle(BoardCommand::WheelZoom {
              factor: if dy < 0.0 { 1.12 } else { 1.0 / 1.12 },
              focal_screen_x: f32::from(ev.position.x) as f64,
              focal_screen_y: f32::from(ev.position.y) as f64,
            });
          })
          .flex_1().w_full().h_full(),
        )
        .child(build_left_dock(&ctx))
        .child(dynamic(move || {
          let Some(state) = c_insp.inspector_state.get() else {
            return view().into_element();
          };
          let dark = c_insp.dark_mode.get();
          build_contextual_inspector_for(&c_insp, &state, dark).into_element()
        }))
        .child(when(move || c_inline_cond.editing_node_id.get().is_some(), move || {
          build_world_inline_editor(&c_inline_body)
        }))
        .child(build_zoom_pill(&ctx))
        .child(build_shortcut_hint_pill(dark_mode))
        .child(when(move || toast_sig.get().is_some(), move || {
          view()
            .test_id("board-toast").absolute().bottom(px(58.0)).right(px(16.0)).z_index(40)
            .px_3().py_2().rounded(px(8.0)).bg(rgb_const(0x1F2329)).shadow_md()
            .child(text(move || toast_sig.get().unwrap_or_default()).text_size(px(12.0)).text_color(rgb_const(0xFFFFFF)))
        })),
    )
    .child(when(move || c_imp_cond.show_import_menu.get(), move || build_import_menu(&c_imp_body)))
    .child(when(move || c_exp_cond.show_export_menu.get(), move || build_export_menu(&c_exp_body)))
    .child(when(move || c_edge_cond.editing_edge_id.get().is_some(), move || build_edge_label_modal(&c_edge_body)))
    .child(when(move || c_md_cond.show_md_import_modal.get(), move || build_md_import_modal(&c_md_body)))
}

// ============================================================================
// Board Context & Unified Command + UiEffect Loop
// ============================================================================

#[derive(Clone, PartialEq)]
enum InspectorState {
  Node {
    fill: String,
    stroke: String,
    dashed: bool,
    font_size_tenths: i32,
  },
  Edge {
    edge_id: String,
    routing: String,
    stroke: String,
    dashed: bool,
    arrow_end: String,
    label: String,
  },
}

fn compute_inspector_state(s: &BoardSession, editing_any: bool) -> Option<InspectorState> {
  if editing_any {
    return None;
  }
  let busy = matches!(
    s.gesture(),
    GestureState::Drag(_)
      | GestureState::Resize(_)
      | GestureState::Connector(_)
      | GestureState::Marquee(_)
  );
  if busy {
    return None;
  }
  if let Some(first) = s.selected_node_ids().first().and_then(|id| s.node_by_id(id)) {
    return Some(InspectorState::Node {
      fill: first.style.fill.clone(),
      stroke: first.style.stroke.clone(),
      dashed: first.style.dashed,
      font_size_tenths: (first.style.font_size * 10.0).round() as i32,
    });
  }
  if let Some(eid) = s.selected_edge_id()
    && let Some(edge) = s.snapshot().edges.iter().find(|e| e.id == eid)
  {
    return Some(InspectorState::Edge {
      edge_id: edge.id.clone(),
      routing: edge.routing.clone(),
      stroke: edge.style.stroke.clone(),
      dashed: edge.style.dashed,
      arrow_end: edge.arrow_end.clone(),
      label: edge.label.clone(),
    });
  }
  None
}

#[derive(Clone)]
struct BoardCtx {
  session: Rc<RefCell<BoardSession>>,
  rev: Signal<u64>,
  dark_mode: Signal<bool>,
  tool_sig: Signal<BoardTool>,
  board_mode_sig: Signal<String>,
  can_undo_sig: Signal<bool>,
  can_redo_sig: Signal<bool>,
  counts_sig: Signal<(usize, usize)>,
  zoom_pct_sig: Signal<i32>,
  inspector_state: Signal<Option<InspectorState>>,
  title_draft: Signal<String>,
  editing_node_id: Signal<Option<String>>,
  editing_draft: Signal<String>,
  editing_edge_id: Signal<Option<String>>,
  editing_edge_draft: Signal<String>,
  show_shape_panel: Signal<bool>,
  show_icon_panel: Signal<bool>,
  show_import_menu: Signal<bool>,
  show_export_menu: Signal<bool>,
  show_md_import_modal: Signal<bool>,
  md_import_draft: Signal<String>,
  active_shape_kind: Signal<String>,
  active_icon_id: Signal<String>,
  toast_msg: Signal<Option<String>>,
  last_svg_bytes: Signal<usize>,
  viewport_size: Signal<(f32, f32)>,
  did_initial_fit: Signal<bool>,
  space_held: Signal<bool>,
  last_move_ms: Rc<std::cell::Cell<u64>>,
  #[cfg(not(target_arch = "wasm32"))]
  mounted_at: std::time::Instant,
  on_persist: Rc<dyn Fn(String)>,
  on_close: Option<Rc<dyn Fn()>>,
}

impl BoardCtx {
  fn bump(&self) { self.rev.set(self.rev.get_untracked() + 1); }

  fn sync_chrome(&self) {
    let editing_any = self.editing_node_id.get_untracked().is_some()
      || self.editing_edge_id.get_untracked().is_some();
    let (tool, mode, can_undo, can_redo, counts, zoom_pct, insp) = {
      let s = self.session.borrow();
      let snap = s.snapshot();
      (
        s.tool(),
        snap.mode.clone(),
        s.can_undo(),
        s.can_redo(),
        (snap.nodes.len(), snap.edges.len()),
        (s.zoom() * 100.0).round() as i32,
        compute_inspector_state(&s, editing_any),
      )
    };
    if self.tool_sig.get_untracked() != tool { self.tool_sig.set(tool); }
    if self.board_mode_sig.get_untracked() != mode { self.board_mode_sig.set(mode); }
    if self.can_undo_sig.get_untracked() != can_undo { self.can_undo_sig.set(can_undo); }
    if self.can_redo_sig.get_untracked() != can_redo { self.can_redo_sig.set(can_redo); }
    if self.counts_sig.get_untracked() != counts { self.counts_sig.set(counts); }
    if self.zoom_pct_sig.get_untracked() != zoom_pct { self.zoom_pct_sig.set(zoom_pct); }
    if self.inspector_state.get_untracked() != insp { self.inspector_state.set(insp); }
  }

  fn finish_gesture_if_active(&self) {
    let active = !matches!(self.session.borrow().gesture(), GestureState::Idle);
    if active {
      self.dispatch_and_handle(BoardCommand::PointerUp);
    }
  }

  fn now_ms(&self) -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    { self.mounted_at.elapsed().as_millis() as u64 }
    #[cfg(target_arch = "wasm32")]
    { 0 }
  }

  fn close_popups(&self) {
    for sig in [self.show_shape_panel, self.show_icon_panel, self.show_import_menu, self.show_export_menu] {
      if sig.get_untracked() { sig.set(false); }
    }
  }

  fn show_toast(&self, msg: impl Into<String>) {
    let text_msg = msg.into();
    self.toast_msg.set(Some(text_msg.clone()));
    let toast = self.toast_msg;
    drop(set_timeout(std::time::Duration::from_secs(2), move || {
      if toast.get_untracked().as_deref() == Some(text_msg.as_str()) { toast.set(None); }
    }));
  }

  /// Single unified entry point for all `BoardCommand` dispatches and `UiEffect`s.
  fn dispatch_and_handle(&self, cmd: BoardCommand) -> BoardPatch {
    let (patch, next_json) = {
      let mut s = self.session.borrow_mut();
      let patch = s.dispatch(cmd);
      let json = patch.snapshot_changed.then(|| s.export_compact_json());
      (patch, json)
    };

    let overlay_edit = patch.active_overlay.editing_node_id.clone();
    if overlay_edit != self.editing_node_id.get_untracked() {
      if let Some(ref nid) = overlay_edit {
        let txt = self.session.borrow().node_by_id(nid).map(|n| n.text.clone()).unwrap_or_default();
        self.editing_draft.set(txt);
      }
      self.editing_node_id.set(overlay_edit);
    }
    if self.title_draft.get_untracked() != patch.active_overlay.title {
      self.title_draft.set(patch.active_overlay.title.clone());
    }
    if self.active_shape_kind.get_untracked() != patch.active_overlay.active_shape_kind {
      self.active_shape_kind.set(patch.active_overlay.active_shape_kind.clone());
    }
    if self.active_icon_id.get_untracked() != patch.active_overlay.active_icon_id {
      self.active_icon_id.set(patch.active_overlay.active_icon_id.clone());
    }

    for eff in &patch.ui_effects {
      match eff {
        UiEffect::StartInlineEdit { node_id, text } => {
          self.editing_draft.set(text.clone());
          self.editing_node_id.set(Some(node_id.clone()));
          self.editing_edge_id.set(None);
        }
        UiEffect::PromptEdgeLabel { edge_id, current_label } => {
          self.editing_edge_draft.set(current_label.clone());
          self.editing_edge_id.set(Some(edge_id.clone()));
          self.editing_node_id.set(None);
        }
        UiEffect::CloseEditor => self.close_and_save(),
      }
    }

    self.sync_chrome();
    self.bump();
    if let Some(json) = next_json { (self.on_persist)(json); }
    patch
  }

  fn style_nodes(&self, fill: Option<&str>, stroke: Option<&str>, dashed: Option<bool>, font_size: Option<f64>) {
    self.dispatch_and_handle(BoardCommand::UpdateSelectedNodesStyle {
      fill: fill.map(str::to_string), stroke: stroke.map(str::to_string), dashed, font_size,
      text_color: None, align: None, kind: None,
    });
  }

  fn style_edge(&self, routing: Option<&str>, stroke: Option<&str>, dashed: Option<bool>, arrow_end: Option<&str>) {
    self.dispatch_and_handle(BoardCommand::UpdateSelectedEdge {
      routing: routing.map(str::to_string), stroke: stroke.map(str::to_string), dashed,
      arrow_end: arrow_end.map(str::to_string), arrow_start: None,
    });
  }

  fn has_modal_or_edit_open(&self) -> bool {
    self.editing_node_id.get_untracked().is_some()
      || self.editing_edge_id.get_untracked().is_some()
      || self.show_md_import_modal.get_untracked()
  }

  fn add_node_in_view(&self, kind: &str, prefix: &str, fill: Option<&str>, stroke: Option<&str>) {
    let (vw, vh) = self.viewport_size.get_untracked();
    let (wx, wy, count) = {
      let s = self.session.borrow();
      let count = s.snapshot().nodes.len();
      let center = s.screen_to_world([(vw as f64) * 0.48, (vh as f64) * 0.45]);
      (
        (center[0] + ((count % 4) as f64 - 1.5) * 185.0).max(-2000.0),
        (center[1] + (((count / 4) % 3) as f64 - 1.0) * 115.0).max(-2000.0),
        count,
      )
    };
    self.dispatch_and_handle(BoardCommand::AddNode {
      world_x: wx, world_y: wy, kind: kind.to_string(), icon: None,
      text: Some(format!("{prefix} {}", count + 1)), width: None, height: None,
      fill: fill.map(str::to_string), stroke: stroke.map(str::to_string), start_editing: false,
    });
  }

  fn add_mind_child(&self, start_editing: bool) {
    let pid = {
      let s = self.session.borrow();
      s.selected_node_ids().first().cloned().or_else(|| s.snapshot().nodes.first().map(|n| n.id.clone()))
    };
    if let Some(parent_id) = pid {
      self.dispatch_and_handle(BoardCommand::AddMindChild { parent_id, start_editing });
    }
  }

  fn add_mind_sibling(&self, start_editing: bool) {
    let nid = {
      let s = self.session.borrow();
      s.selected_node_ids().first().cloned().or_else(|| s.snapshot().nodes.first().map(|n| n.id.clone()))
    };
    if let Some(node_id) = nid {
      self.dispatch_and_handle(BoardCommand::AddMindSibling { node_id, start_editing });
    }
  }

  fn start_editing_selected(&self) {
    let target = {
      let s = self.session.borrow();
      s.selected_node_ids().first().cloned().or_else(|| s.snapshot().nodes.first().map(|n| n.id.clone()))
    };
    if let Some(node_id) = target {
      self.dispatch_and_handle(BoardCommand::StartEditingNode { node_id });
    }
  }

  fn finish_editing(&self) {
    if self.editing_node_id.get_untracked().is_none() { return; }
    let json = {
      let mut s = self.session.borrow_mut();
      s.dispatch(BoardCommand::FinishEditing);
      s.export_compact_json()
    };
    self.editing_node_id.set(None);
    self.sync_chrome();
    self.bump();
    (self.on_persist)(json);
  }

  fn zoom_by(&self, factor: f64) {
    let (vw, vh) = self.viewport_size.get_untracked();
    self.dispatch_and_handle(BoardCommand::WheelZoom {
      factor, focal_screen_x: (vw as f64) * 0.5, focal_screen_y: (vh as f64) * 0.5,
    });
  }

  fn reset_zoom(&self) {
    let pan = self.session.borrow().pan();
    self.dispatch_and_handle(BoardCommand::SetViewport { zoom: 1.0, pan, viewport: None });
  }

  fn fit_to_content(&self) {
    let (vw, vh) = self.viewport_size.get_untracked();
    self.dispatch_and_handle(BoardCommand::FitToContent {
      viewport: Some([vw.max(320.0) as f64, vh.max(240.0) as f64]),
    });
  }

  fn close_and_save(&self) {
    self.close_popups();
    self.finish_editing();
    let title = self.title_draft.get_untracked();
    let json = {
      let mut s = self.session.borrow_mut();
      s.dispatch(BoardCommand::SetTitle { title });
      s.export_compact_json()
    };
    (self.on_persist)(json);
    if let Some(ref cb) = self.on_close { cb(); }
  }

  fn save_text_with_fallback(
    &self,
    file_name: String,
    filter_name: &'static str,
    ext: &'static str,
    dialog_title: &'static str,
    content: String,
    label: &'static str,
  ) {
    let ctx = self.clone();
    spawn(async move {
      #[cfg(target_arch = "wasm32")]
      {
        let _ = (filter_name, dialog_title);
        let mime = crate::transfer::infer_mime_type(&file_name, &[ext]);
        match crate::web_runtime::web_download_bytes(&file_name, mime, content.as_bytes()) {
          Ok(()) => ctx.show_toast(format!("已下载 {label}：{file_name}")),
          Err(_) => {
            set_clipboard(content);
            ctx.show_toast(format!("已复制 {label} 到剪贴板"));
          }
        }
      }
      #[cfg(not(target_arch = "wasm32"))]
      {
        let answer = pick_save_path(
          SaveDialog::new().file_name(file_name).filter(FileFilter::new(filter_name, [ext])).title(dialog_title),
        ).await;
        match answer {
          Ok(Some(path)) => {
            match std::fs::write(&path, content.as_bytes()) {
              Ok(()) => ctx.show_toast(format!("已导出 {label}：{}", path.display())),
              Err(e) => { set_clipboard(content); ctx.show_toast(format!("写入失败 ({e})，已复制 {label} 到剪贴板")); }
            }
          }
          Ok(None) => {}
          Err(_) => { set_clipboard(content); ctx.show_toast(format!("已复制 {label} 到剪贴板")); }
        }
      }
    });
  }

  fn export_svg(&self) {
    self.close_popups();
    let svg_theme: BoardSvgTheme = board_svg_theme(self.dark_mode.get_untracked());
    let (markup, title) = {
      let s = self.session.borrow();
      (
        mica_board::render_board_svg_with_source(s.snapshot(), Some(&svg_theme)),
        s.snapshot().title.clone(),
      )
    };
    self.last_svg_bytes.set(markup.len());
    self.save_text_with_fallback(
      format!("{}.svg", sanitize_filename(&title)), "SVG Vector", "svg", "导出高清矢量 SVG", markup, "SVG",
    );
  }

  fn export_markdown_outline(&self) {
    self.close_popups();
    let md = mica_board::mindmap_to_markdown(self.session.borrow().snapshot());
    if md.trim().is_empty() { self.show_toast("没有可导出的节点文本"); return; }
    set_clipboard(md);
    self.show_toast("已复制 Markdown 大纲到剪贴板");
  }

  fn export_json(&self) {
    self.close_popups();
    let (json, title) = {
      let s = self.session.borrow();
      (s.export_compact_json(), s.snapshot().title.clone())
    };
    self.save_text_with_fallback(
      format!("{}.mica-board.json", sanitize_filename(&title)), "Mica Board JSON", "json", "导出 .mica-board.json", json, "JSON",
    );
  }

  fn import_drawio_or_json(&self) {
    self.close_popups();
    let ctx = self.clone();
    spawn(async move {
      let picked = match crate::transfer::pick_import_file_bytes(
        "draw.io / Mica Board",
        &["drawio", "xml", "svg", "png", "json"],
        false,
      )
      .await
      {
        Ok(files) => files,
        Err(err) => {
          ctx.show_toast(err);
          return;
        }
      };
      let Some(file) = picked.into_iter().next() else { return; };
      let is_json = file
        .name
        .rsplit('.')
        .next()
        .filter(|_| file.name.contains('.'))
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
      let imported = if is_json {
        String::from_utf8(file.bytes).ok().and_then(|s| BoardSnapshot::parse_json(&s).ok())
          .ok_or_else(|| "JSON 格式无法识别为 Mica 画板".to_string())
      } else if let Some(snap) = mica_board::extract_board_snapshot_from_svg(&file.bytes) {
        Ok(snap)
      } else {
        mica_board::import_drawio_bytes(&file.bytes).map_err(|e| format!("导入 draw.io 失败：{e}"))
      };
      match imported {
        Ok(snap) => {
          let (n, e) = (snap.nodes.len(), snap.edges.len());
          ctx.dispatch_and_handle(BoardCommand::ReplaceSnapshot { snapshot: snap, record_undo: true });
          ctx.fit_to_content();
          ctx.show_toast(format!("已导入画板（{n} 个图形，{e} 条连线）"));
        }
        Err(msg) => ctx.show_toast(msg),
      }
    });
  }

  fn apply_markdown_outline(&self) {
    let md = self.md_import_draft.get_untracked();
    if md.trim().is_empty() { self.show_md_import_modal.set(false); return; }
    let title = self.session.borrow().snapshot().title.clone();
    let snap = mica_board::markdown_to_mindmap(&md, Some(&title));
    let n = snap.nodes.len();
    self.show_md_import_modal.set(false);
    self.dispatch_and_handle(BoardCommand::ReplaceSnapshot { snapshot: snap, record_undo: true });
    self.fit_to_content();
    self.show_toast(format!("已由 Markdown 大纲生成思维导图（{n} 节点）"));
  }
}

fn sanitize_filename(raw: &str) -> String {
  let cleaned: String = raw.trim().chars()
    .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
    .collect();
  if cleaned.is_empty() { "未命名画板".to_string() } else { cleaned }
}

// ============================================================================
// Keyboard Routing -> `BoardCommand::KeyDown`
// ============================================================================

fn bind_key_cmd(ctx: &BoardCtx, binding: &str, key: &'static str, ctrl_or_meta: bool, shift: bool) {
  let c = ctx.clone();
  on_key_with(binding, KeyOptions { when: When::NotEditing, ..KeyOptions::default() }, move |_| {
    if !c.has_modal_or_edit_open() {
      c.dispatch_and_handle(BoardCommand::KeyDown { key: key.to_string(), ctrl_or_meta, shift, read_only: false });
    }
  });
}

fn register_board_shortcuts(ctx: &BoardCtx) {
  for (binding, key) in [
    ("v", "v"), ("h", "h"), ("s", "s"), ("t", "t"), ("l", "l"), ("p", "p"), ("f", "f"), ("m", "m"),
    ("tab", "tab"), ("return", "enter"), ("delete", "delete"), ("backspace", "backspace"),
  ] {
    bind_key_cmd(ctx, binding, key, false, false);
  }
  for (binding, key, shift) in [
    ("mod+z", "z", false), ("shift+mod+z", "z", true), ("mod+y", "y", false),
    ("mod+c", "c", false), ("mod+v", "v", false), ("mod+d", "d", false), ("mod+a", "a", false),
  ] {
    bind_key_cmd(ctx, binding, key, true, shift);
  }

  let c_esc = ctx.clone();
  on_key_with("escape", KeyOptions { when: When::Always, pass_through: true, ..KeyOptions::default() }, move |_| {
    if c_esc.show_md_import_modal.get_untracked() { c_esc.show_md_import_modal.set(false); return; }
    if c_esc.editing_edge_id.get_untracked().is_some() {
      c_esc.editing_edge_id.set(None);
      c_esc.sync_chrome();
      return;
    }
    if c_esc.show_shape_panel.get_untracked() || c_esc.show_icon_panel.get_untracked()
      || c_esc.show_import_menu.get_untracked() || c_esc.show_export_menu.get_untracked()
    {
      c_esc.close_popups();
      return;
    }
    c_esc.dispatch_and_handle(BoardCommand::KeyDown { key: "escape".into(), ctrl_or_meta: false, shift: false, read_only: false });
  });
}

// ============================================================================
// Shared Chrome Primitives & 50px Top Bar
// ============================================================================

fn floating_panel(dm: Signal<bool>, tid: &'static str, radius: f32) -> View {
  view()
    .test_id(tid).rounded(px(radius)).border_1()
    .border_color(move || bc::border(dm.get()))
    .bg(move || bc::panel(dm.get()))
    .shadow_md()
}

fn build_top_bar(ctx: &BoardCtx) -> impl IntoElement {
  let (dm, title_draft) = (ctx.dark_mode, ctx.title_draft);
  let (mode_sig, can_undo_sig, can_redo_sig, counts_sig) = (
    ctx.board_mode_sig,
    ctx.can_undo_sig,
    ctx.can_redo_sig,
    ctx.counts_sig,
  );
  let (c_title_sub, c_undo, c_redo, c_layout) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());
  let (c_imp_btn, c_exp_btn, c_svg_quick, c_done) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());

  view()
    .test_id("board-top-bar").w_full().h(px(50.0)).px(px(14.0))
    .flex_row().items_center().gap_2().border_b_1()
    .border_color(move || bc::border(dm.get())).bg(move || bc::panel(dm.get()))
    .child(dynamic(move || {
      let glyph = if mode_sig.get() == "mindmap" { ICON_MINDMAP } else { ICON_DRAW };
      dock_svg(19.0, "#3370FF", glyph)
    }))
    .child(
      input()
        .test_id("board-title-input").w(px(220.0)).px_2().py_1().rounded(px(6.0))
        .bg(move || bc::panel(dm.get())).text_size(px(14.0)).font_weight(FontWeight::SEMIBOLD)
        .text_color(move || bc::ink(dm.get())).placeholder("未命名画板")
        .value(move || title_draft.get())
        .on_change(move |v: &str| title_draft.set(v.to_owned()))
        .on_submit(move |v: &str| { c_title_sub.dispatch_and_handle(BoardCommand::SetTitle { title: v.to_owned() }); }),
    )
    .child(
      button().test_id("board-undo").accessibility_label("撤销 (Ctrl+Z)")
        .w(px(30.0)).h(px(30.0)).items_center().justify_center().rounded(px(6.0))
        .hover(move |s| s.bg(bc::chip(dm.get_untracked())))
        .on_press(move || { c_undo.dispatch_and_handle(BoardCommand::Undo); })
        .child(dynamic(move || {
          mica_icons::undo(if can_undo_sig.get() { bc::icon_hex(dm.get()) } else { "#9CA3AF" })
        })),
    )
    .child(
      button().test_id("board-redo").accessibility_label("重做 (Ctrl+Shift+Z)")
        .w(px(30.0)).h(px(30.0)).items_center().justify_center().rounded(px(6.0))
        .hover(move |s| s.bg(bc::chip(dm.get_untracked())))
        .on_press(move || { c_redo.dispatch_and_handle(BoardCommand::Redo); })
        .child(dynamic(move || {
          mica_icons::redo(if can_redo_sig.get() { bc::icon_hex(dm.get()) } else { "#9CA3AF" })
        })),
    )
    .child(
      button().test_id("board-layout").accessibility_label("整理导图 Auto Layout")
        .px_2().h(px(30.0)).flex_row().items_center().gap_1().rounded(px(6.0))
        .hover(move |s| s.bg(bc::chip(dm.get_untracked())))
        .on_press(move || { c_layout.dispatch_and_handle(BoardCommand::LayoutMindmap); })
        .child(dock_svg(15.0, "#3370FF", ICON_WAND))
        .child(text("Auto Layout").text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(move || bc::ink(dm.get()))),
    )
    .child(
      button().test_id("board-import-btn").accessibility_label("导入画板或 Markdown 大纲")
        .px_2().h(px(30.0)).flex_row().items_center().gap_1().rounded(px(6.0))
        .hover(move |s| s.bg(bc::chip(dm.get_untracked())))
        .on_press(move || {
          let next = !c_imp_btn.show_import_menu.get_untracked();
          c_imp_btn.close_popups();
          c_imp_btn.show_import_menu.set(next);
        })
        .child(dynamic(move || dock_svg(15.0, bc::icon_hex(dm.get()), ICON_UPLOAD)))
        .child(text("导入").text_size(px(12.5)).text_color(move || bc::ink(dm.get()))),
    )
    .child(
      button().test_id("board-export-btn").accessibility_label("导出画板")
        .px_2().h(px(30.0)).flex_row().items_center().gap_1().rounded(px(6.0))
        .hover(move |s| s.bg(bc::chip(dm.get_untracked())))
        .on_press(move || {
          let next = !c_exp_btn.show_export_menu.get_untracked();
          c_exp_btn.close_popups();
          c_exp_btn.show_export_menu.set(next);
        })
        .child(dynamic(move || dock_svg(15.0, bc::icon_hex(dm.get()), ICON_DOWNLOAD)))
        .child(text("导出").text_size(px(12.5)).text_color(move || bc::ink(dm.get()))),
    )
    .child(
      button().test_id("board-export-svg").accessibility_label("导出高清矢量 SVG")
        .px_2().h(px(28.0)).items_center().justify_center().rounded(px(6.0))
        .bg(move || bc::chip(dm.get())).hover(move |s| s.bg(bc::brand_wash(dm.get_untracked())))
        .on_press(move || c_svg_quick.export_svg())
        .child(text("SVG").text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(move || bc::ink(dm.get()))),
    )
    .child(view().flex_1())
    .child(
      text(move || {
        let (nodes, edges) = counts_sig.get();
        format!("{nodes} 图形 · {edges} 连线")
      })
      .text_size(px(12.0)).text_color(move || bc::muted(dm.get())),
    )
    .child(
      button().test_id("board-done-btn").accessibility_label("完成并保存画板")
        .px(px(16.0)).py(px(6.0)).rounded(px(6.0)).bg(bc::BRAND)
        .on_press(move || c_done.close_and_save())
        .child(text("完成").text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(rgb_const(0xFFFFFF))),
    )
}

fn build_import_menu(ctx: &BoardCtx) -> impl IntoElement + use<> {
  let (dm, c_drawio, c_md) = (ctx.dark_mode, ctx.clone(), ctx.clone());
  floating_panel(dm, "board-import-menu", 10.0)
    .absolute().top(px(52.0)).left(px(400.0)).w(px(290.0)).z_index(45).p_1().gap_1()
    .child(menu_row_btn(dm, "board-import-drawio", "导入 draw.io (.drawio/.xml/.svg/.png) 或 JSON", move || c_drawio.import_drawio_or_json()))
    .child(menu_row_btn(dm, "board-import-md", "从 Markdown 大纲生成思维导图", move || {
      c_md.close_popups();
      c_md.show_md_import_modal.set(true);
    }))
}

fn build_export_menu(ctx: &BoardCtx) -> impl IntoElement + use<> {
  let (dm, c_svg, c_md, c_json) = (ctx.dark_mode, ctx.clone(), ctx.clone(), ctx.clone());
  floating_panel(dm, "board-export-menu", 10.0)
    .absolute().top(px(52.0)).left(px(465.0)).w(px(220.0)).z_index(45).p_1().gap_1()
    .child(menu_row_btn(dm, "board-export-svg-item", "导出高清矢量 SVG", move || c_svg.export_svg()))
    .child(menu_row_btn(dm, "board-export-md", "复制为 Markdown 大纲", move || c_md.export_markdown_outline()))
    .child(menu_row_btn(dm, "board-export-json", "导出 .mica-board.json", move || c_json.export_json()))
}

fn menu_row_btn(dm: Signal<bool>, tid: &'static str, label: &'static str, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid).w_full().px_3().py_2().rounded(px(6.0)).items_start()
    .hover(move |s| s.bg(bc::chip(dm.get_untracked()))).on_press(on_click)
    .child(text(label).text_size(px(12.5)).text_color(move || bc::ink(dm.get())))
}

// ============================================================================
// Left Vertical 10-Tool Dock + 12-Shape & 20-Tech-Icon Flyouts
// ============================================================================

fn build_left_dock(ctx: &BoardCtx) -> impl IntoElement {
  let (dm, show_shapes, show_icons) = (ctx.dark_mode, ctx.show_shape_panel, ctx.show_icon_panel);
  let (c_sel, c_hand, c_shape, c_icon) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());
  let (c_sticky, c_text, c_conn, c_pen) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());
  let (c_frame, c_mind, c_shape_fly, c_icon_fly) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());

  view()
    .test_id("board-left-dock-wrap").absolute().left(px(14.0)).top(px(14.0)).z_index(25)
    .flex_row().items_start().gap_2()
    .child(
      floating_panel(dm, "board-left-dock", 12.0).py_2().px(px(6.0)).gap(px(2.0)).items_center()
        .child(dock_tool_btn(ctx, "board-tool-select", "选择 / 框选 (V)", ICON_CURSOR, |t, sp, ip| t == BoardTool::Select && !sp && !ip, move || {
          c_sel.close_popups(); c_sel.dispatch_and_handle(BoardCommand::SetTool { tool: BoardTool::Select });
        }))
        .child(dock_tool_btn(ctx, "board-tool-hand", "抓手平移 (H / 按住空格)", ICON_HAND, |t, _, _| t == BoardTool::Hand, move || {
          c_hand.close_popups(); c_hand.dispatch_and_handle(BoardCommand::SetTool { tool: BoardTool::Hand });
        }))
        .child(view().w(px(24.0)).h(px(1.0)).my(px(4.0)).bg(move || bc::border(dm.get())))
        .child(dock_tool_btn(ctx, "board-tool-shape", "图形库（白板 / 流程图）", ICON_SHAPES, |t, sp, _| t == BoardTool::Shape || sp, move || {
          let next = !c_shape.show_shape_panel.get_untracked(); c_shape.close_popups(); c_shape.show_shape_panel.set(next);
        }))
        .child(dock_tool_btn(ctx, "board-tool-icon", "架构图标库（20 个技术组件）", ICON_SERVER, |t, _, ip| t == BoardTool::Icon || ip, move || {
          let next = !c_icon.show_icon_panel.get_untracked(); c_icon.close_popups(); c_icon.show_icon_panel.set(next);
        }))
        .child(dock_tool_btn(ctx, "board-add-sticky", "便签 (S)", ICON_STICKY, |t, _, _| t == BoardTool::Sticky, move || {
          c_sticky.close_popups(); c_sticky.add_node_in_view("sticky", "便签", Some("#FFF7CC"), Some("#E0B422"));
        }))
        .child(dock_tool_btn(ctx, "board-tool-text", "文本 (T)", ICON_TEXT, |t, _, _| t == BoardTool::Text, move || {
          c_text.close_popups(); c_text.dispatch_and_handle(BoardCommand::SetTool { tool: BoardTool::Text });
        }))
        .child(dock_tool_btn(ctx, "board-tool-connector", "连线 / 箭头 (L)", ICON_CONNECTOR, |t, _, _| t == BoardTool::Connector, move || {
          c_conn.close_popups(); c_conn.dispatch_and_handle(BoardCommand::SetTool { tool: BoardTool::Connector });
        }))
        .child(dock_tool_btn(ctx, "board-tool-pen", "自由画笔 (P)", ICON_PEN, |t, _, _| t == BoardTool::Pen, move || {
          c_pen.close_popups(); c_pen.dispatch_and_handle(BoardCommand::SetTool { tool: BoardTool::Pen });
        }))
        .child(dock_tool_btn(ctx, "board-tool-frame", "分组容器 Frame (F)", ICON_FRAME, |t, _, _| t == BoardTool::Frame, move || {
          c_frame.close_popups(); c_frame.dispatch_and_handle(BoardCommand::SetTool { tool: BoardTool::Frame });
        }))
        .child(view().w(px(24.0)).h(px(1.0)).my(px(4.0)).bg(move || bc::border(dm.get())))
        .child(dock_tool_btn(ctx, "board-add-node", "插入思维导图 (M)", ICON_MINDMAP, |t, _, _| t == BoardTool::Mindmap, move || {
          c_mind.close_popups(); c_mind.add_node_in_view("mind_root", "中心主题", Some("#3370FF"), Some("#245BDB"));
        })),
    )
    .child(when(move || show_shapes.get(), move || build_shape_flyout(&c_shape_fly)))
    .child(when(move || show_icons.get(), move || build_tech_icon_flyout(&c_icon_fly)))
}

fn dock_tool_btn(
  ctx: &BoardCtx,
  tid: &'static str,
  tooltip_text: &'static str,
  svg_inner: &'static str,
  is_active: impl Fn(BoardTool, bool, bool) -> bool + Copy + 'static,
  on_click: impl Fn() + 'static,
) -> impl IntoElement {
  let (dm, tool_sig, sp, ip) = (ctx.dark_mode, ctx.tool_sig, ctx.show_shape_panel, ctx.show_icon_panel);
  button()
    .test_id(tid).accessibility_label(tooltip_text)
    .w(px(36.0)).h(px(36.0)).items_center().justify_center().rounded(px(8.0))
    .bg(move || {
      if is_active(tool_sig.get(), sp.get(), ip.get()) { bc::brand_wash(dm.get()) } else { rgba_const(0x000000, 0.0) }
    })
    .hover(move |s| s.bg(bc::chip(dm.get_untracked()))).on_press(on_click)
    .child(dynamic(move || {
      let col = if is_active(tool_sig.get(), sp.get(), ip.get()) { "#3370FF" } else { bc::icon_hex(dm.get()) };
      dock_svg(19.0, col, svg_inner)
    }))
}

fn build_shape_flyout(ctx: &BoardCtx) -> impl IntoElement + use<> {
  let (dm, active_shape) = (ctx.dark_mode, ctx.active_shape_kind);
  let build_group = |category: &'static str| {
    let mut grid = view().w_full().flex_row().flex_wrap().gap(px(6.0));
    for meta in BOARD_SHAPES.iter().filter(|m| m.category == category) {
      let c_pick = ctx.clone();
      let (kind, label, tid) = (meta.kind, meta.label, format!("board-shape-{}", meta.kind));
      grid = grid.child(
        button()
          .test_id(&tid).accessibility_label(label)
          .w(px(80.0)).py(px(7.0)).px(px(4.0)).gap(px(4.0)).items_center().rounded(px(8.0)).border_1()
          .border_color(move || if active_shape.get() == kind { bc::BRAND } else { rgba_const(0x000000, 0.0) })
          .bg(move || if active_shape.get() == kind { bc::brand_wash(dm.get()) } else { bc::chip(dm.get()) })
          .on_press(move || {
            c_pick.dispatch_and_handle(BoardCommand::SetActiveShape { kind: kind.to_string() });
            c_pick.dispatch_and_handle(BoardCommand::SetTool { tool: BoardTool::Shape });
            c_pick.show_shape_panel.set(false);
          })
          .child(mini_shape_svg(kind, meta.default_fill, meta.default_stroke))
          .child(text(label).text_size(px(11.0)).text_color(move || bc::ink(dm.get()))),
      );
    }
    grid
  };

  floating_panel(dm, "board-shape-flyout", 12.0).w(px(280.0)).p_3().gap_2()
    .child(text("基础图形（选后点击画布放置）").text_size(px(11.0)).text_color(move || bc::muted(dm.get())))
    .child(build_group("basic"))
    .child(text("流程图标准图形").text_size(px(11.0)).text_color(move || bc::muted(dm.get())))
    .child(build_group("flowchart"))
}

fn build_tech_icon_flyout(ctx: &BoardCtx) -> impl IntoElement + use<> {
  let (dm, active_icon) = (ctx.dark_mode, ctx.active_icon_id);
  let mut grid = view().w_full().flex_row().flex_wrap().gap(px(6.0));
  for meta in BOARD_TECH_ICONS {
    let c_pick = ctx.clone();
    let (id, label, tid) = (meta.id, meta.label, format!("board-tech-icon-{}", meta.id));
    grid = grid.child(
      button()
        .test_id(&tid).accessibility_label(label)
        .w(px(68.0)).py(px(7.0)).px(px(4.0)).gap(px(4.0)).items_center().rounded(px(8.0)).border_1()
        .border_color(move || if active_icon.get() == id { bc::BRAND } else { rgba_const(0x000000, 0.0) })
        .bg(move || if active_icon.get() == id { bc::brand_wash(dm.get()) } else { bc::chip(dm.get()) })
        .on_press(move || {
          c_pick.dispatch_and_handle(BoardCommand::SetActiveIcon { icon_id: id.to_string() });
          c_pick.dispatch_and_handle(BoardCommand::SetTool { tool: BoardTool::Icon });
          c_pick.show_icon_panel.set(false);
        })
        .child(mini_tech_icon_svg(id, meta.default_stroke))
        .child(text(label).text_size(px(10.5)).text_color(move || bc::ink(dm.get()))),
    );
  }
  floating_panel(dm, "board-icon-flyout", 12.0).w(px(312.0)).max_h(px(420.0)).p_3().gap_2()
    .child(text("技术架构图标库（选后点击画布放置）").text_size(px(11.0)).text_color(move || bc::muted(dm.get())))
    .child(scroll().w_full().max_h(px(368.0)).child(grid))
}

// ============================================================================
// Top-Center Contextual Floating Inspector (Node Mode + Edge Mode)
// ============================================================================

fn build_contextual_inspector_for(ctx: &BoardCtx, state: &InspectorState, dark: bool) -> impl IntoElement + use<> {
  let body = match state {
    InspectorState::Node { fill, stroke, dashed, font_size_tenths } => {
      build_node_inspector_row(ctx, fill, stroke, *dashed, (*font_size_tenths as f64) / 10.0, dark).into_element()
    }
    InspectorState::Edge { edge_id, routing, stroke, dashed, arrow_end, label } => {
      build_edge_inspector_row(ctx, edge_id, routing, stroke, *dashed, arrow_end, label, dark).into_element()
    }
  };
  view()
    .test_id("board-inspector-wrap").absolute().top(px(14.0)).left(px(68.0)).right(px(68.0)).z_index(24)
    .pointer_events(PointerEvents::BoxNone).flex_row().justify_center()
    .child(
      floating_panel(ctx.dark_mode, "board-inspector-card", 10.0).px(px(10.0)).py(px(6.0))
        .child(body),
    )
}

fn build_node_inspector_row(
  ctx: &BoardCtx,
  fill: &str,
  stroke: &str,
  dashed: bool,
  fs: f64,
  dark: bool,
) -> impl IntoElement {
  let bcol = bc::border(dark);
  let mut row = view().flex_row().flex_wrap().items_center().gap(px(5.0));

  for &(hex, tid, col) in FILL_SWATCHES {
    let c = ctx.clone();
    row = row.child(color_dot_btn(tid, hex, col, false, fill.eq_ignore_ascii_case(hex), move || {
      c.style_nodes(Some(hex), None, None, None);
    }));
  }
  row = row.child(v_sep(bcol));
  for &(hex, tid, col) in STROKE_SWATCHES {
    let c = ctx.clone();
    row = row.child(color_dot_btn(tid, hex, col, true, stroke.eq_ignore_ascii_case(hex), move || {
      c.style_nodes(None, Some(hex), None, None);
    }));
  }

  let (c_dash, c_fdec, c_finc) = (ctx.clone(), ctx.clone(), ctx.clone());
  let (c_child, c_sib, c_edit) = (ctx.clone(), ctx.clone(), ctx.clone());
  let (c_front, c_back, c_dup, c_del) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());

  row
    .child(v_sep(bcol))
    .child(insp_chip("board-node-dashed", if dashed { "实线" } else { "虚线" }, "切换虚线边框", dashed, false, dark, move || {
      c_dash.style_nodes(None, None, Some(!dashed), None);
    }))
    .child(insp_chip("board-font-dec", "A-", "缩小字号", false, false, dark, move || {
      c_fdec.style_nodes(None, None, None, Some((fs - 1.5).clamp(10.0, 28.0)));
    }))
    .child(insp_chip("board-font-inc", "A+", "增大字号", false, false, dark, move || {
      c_finc.style_nodes(None, None, None, Some((fs + 1.5).clamp(10.0, 28.0)));
    }))
    .child(v_sep(bcol))
    .child(insp_chip("board-add-child", "+子", "添加子节点 (Tab)", false, false, dark, move || c_child.add_mind_child(false)))
    .child(insp_chip("board-add-sibling", "+同级", "添加同级节点 (Enter)", false, false, dark, move || c_sib.add_mind_sibling(false)))
    .child(insp_chip("board-edit-label", "编辑文字", "编辑选中节点文字", false, false, dark, move || c_edit.start_editing_selected()))
    .child(v_sep(bcol))
    .child(insp_chip("board-front", "置顶", "置于顶层", false, false, dark, move || { c_front.dispatch_and_handle(BoardCommand::BringToFront); }))
    .child(insp_chip("board-back", "置底", "置于底层", false, false, dark, move || { c_back.dispatch_and_handle(BoardCommand::SendToBack); }))
    .child(insp_chip("board-duplicate", "克隆", "克隆 (Ctrl+D)", false, false, dark, move || { c_dup.dispatch_and_handle(BoardCommand::DuplicateSelection); }))
    .child(insp_chip("board-delete", "删除", "删除 (Delete)", false, true, dark, move || { c_del.dispatch_and_handle(BoardCommand::DeleteSelection); }))
}

#[allow(clippy::too_many_arguments)]
fn build_edge_inspector_row(
  ctx: &BoardCtx,
  edge_id: &str,
  routing: &str,
  stroke: &str,
  dashed: bool,
  arrow_end: &str,
  label: &str,
  dark: bool,
) -> impl IntoElement {
  let bcol = bc::border(dark);
  let mut row = view().flex_row().flex_wrap().items_center().gap(px(5.0));

  for (rkey, rlabel, tid) in [("orthogonal", "折线", "board-edge-ortho"), ("curved", "曲线", "board-edge-curved"), ("straight", "直线", "board-edge-straight")] {
    let (c, active) = (ctx.clone(), routing == rkey);
    row = row.child(
      button().test_id(tid).accessibility_label(rlabel).px_2().py(px(4.0)).rounded(px(6.0))
        .bg(if active { bc::BRAND } else { bc::chip(dark) })
        .on_press(move || c.style_edge(Some(rkey), None, None, None))
        .child(text(rlabel).text_size(px(12.0)).text_color(if active { rgb_const(0xFFFFFF) } else { bc::ink(dark) })),
    );
  }
  row = row.child(v_sep(bcol));
  for &(hex, tid, col) in STROKE_SWATCHES {
    let c = ctx.clone();
    row = row.child(color_dot_btn(tid, hex, col, false, stroke.eq_ignore_ascii_case(hex), move || {
      c.style_edge(None, Some(hex), None, None);
    }));
  }

  let (has_arrow, eid, elbl) = (arrow_end == "arrow", edge_id.to_owned(), label.to_owned());
  let (c_dash, c_arr, c_lbl, c_del) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());

  row
    .child(v_sep(bcol))
    .child(insp_chip("board-edge-dashed", if dashed { "实线" } else { "虚线" }, "切换虚线", dashed, false, dark, move || {
      c_dash.style_edge(None, None, Some(!dashed), None);
    }))
    .child(insp_chip("board-edge-arrow", if has_arrow { "隐藏箭头" } else { "显示箭头" }, "切换终点箭头", has_arrow, false, dark, move || {
      c_arr.style_edge(None, None, None, Some(if has_arrow { "none" } else { "arrow" }));
    }))
    .child(insp_chip("board-edge-edit-label", "编辑标签", "编辑连线标签", false, false, dark, move || {
      c_lbl.editing_edge_draft.set(elbl.clone());
      c_lbl.editing_edge_id.set(Some(eid.clone()));
      c_lbl.sync_chrome();
    }))
    .child(insp_chip("board-edge-delete", "删除连线", "删除选中连线", false, true, dark, move || {
      c_del.dispatch_and_handle(BoardCommand::DeleteSelection);
    }))
}

fn color_dot_btn(tid: &'static str, a11y: &'static str, col: Rgba, outlined: bool, selected: bool, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid).accessibility_label(a11y).w(px(18.0)).h(px(18.0)).rounded_full()
    .bg(if outlined { rgba_const(0x000000, 0.0) } else { col }).border_2()
    .border_color(if selected { bc::BRAND } else if outlined { col } else { rgb_const(0xD0D5DD) })
    .on_press(on_click)
}

fn insp_chip(tid: &'static str, label: &'static str, a11y: &'static str, active: bool, danger: bool, dark: bool, on_click: impl Fn() + 'static) -> impl IntoElement {
  let fg = if danger { bc::DANGER } else if active { bc::BRAND } else { bc::ink(dark) };
  let bg = if active { bc::brand_wash(dark) } else { rgba_const(0x000000, 0.0) };
  button()
    .test_id(tid).accessibility_label(a11y).px(px(6.0)).h(px(26.0)).items_center().justify_center().rounded(px(6.0))
    .bg(bg).hover(move |s| s.bg(bc::chip(dark))).on_press(on_click)
    .child(text(label).text_size(px(11.5)).text_color(fg))
}

fn v_sep(col: Rgba) -> impl IntoElement {
  view().w(px(1.0)).h(px(18.0)).mx(px(1.0)).bg(col)
}

// ============================================================================
// World-Anchored Inline Node Editor & Modals (Edge Label / Markdown Import)
// ============================================================================

fn build_world_inline_editor(ctx: &BoardCtx) -> impl IntoElement + use<> {
  let (dm, rev, draft, vp, edit_id_sig) = (ctx.dark_mode, ctx.rev, ctx.editing_draft, ctx.viewport_size, ctx.editing_node_id);
  let (s_pos, c_change, c_submit, c_done) = (ctx.session.clone(), ctx.clone(), ctx.clone(), ctx.clone());

  let node_box = move || -> (f32, f32, f32, Rgba, Rgba) {
    rev.get();
    let (dark, (vw, vh)) = (dm.get(), vp.get());
    let s = s_pos.borrow();
    let (zoom, [px0, py0]) = (s.zoom() as f32, s.pan());
    if let Some(node) = edit_id_sig.get().as_deref().and_then(|id| s.node_by_id(id)) {
      let sx = (node.x as f32 * zoom + px0 as f32).clamp(64.0, (vw - 260.0).max(64.0));
      let sy = (node.y as f32 * zoom + py0 as f32).clamp(12.0, (vh - 72.0).max(12.0));
      let sw = (node.w as f32 * zoom).clamp(220.0, 420.0);
      let adapted_white = dark && node.style.fill.eq_ignore_ascii_case("#FFFFFF");
      let bg = if adapted_white || node.style.fill == "transparent" {
        if dark { bc::SURFACE_DARK } else { bc::PANEL_LIGHT }
      } else {
        parse_hex_rgba(&node.style.fill).unwrap_or_else(|| bc::panel(dark))
      };
      let fg = if dark && node.style.text_color.eq_ignore_ascii_case("#1F2329") && (adapted_white || node.style.fill == "transparent") {
        bc::INK_DARK
      } else {
        parse_hex_rgba(&node.style.text_color).unwrap_or_else(|| bc::ink(dark))
      };
      (sx, sy, sw, bg, fg)
    } else {
      (80.0, 64.0, 280.0, bc::panel(dark), bc::ink(dark))
    }
  };
  let (nb_l, nb_t, nb_w, nb_bg, nb_fg) = (node_box.clone(), node_box.clone(), node_box.clone(), node_box.clone(), node_box);

  view()
    .test_id("board-inline-edit-bar").absolute()
    .left(move || px(nb_l().0)).top(move || px(nb_t().1)).w(move || px(nb_w().2)).z_index(30)
    .flex_row().items_center().gap_2().px_2().py_1().rounded(px(8.0))
    .border_2().border_color(bc::BRAND).bg(move || nb_bg().3).shadow_md()
    .child(
      input()
        .test_id("board-node-text-input").flex_1().px_2().py_1().rounded(px(4.0))
        .bg(rgba_const(0x000000, 0.0)).text_size(px(13.5)).text_color(move || nb_fg().4)
        .placeholder("输入节点文字...").auto_focus(true).value(move || draft.get())
        .on_change(move |v: &str| {
          draft.set(v.to_owned());
          if let Some(nid) = c_change.editing_node_id.get_untracked() {
            c_change.session.borrow_mut().dispatch(BoardCommand::SetNodeText {
              node_id: nid, text: v.to_owned(), measured_height: None,
            });
            c_change.bump();
          }
        })
        .on_submit(move |_: &str| c_submit.finish_editing()),
    )
    .child(
      button().test_id("board-edit-done").accessibility_label("完成节点文字编辑")
        .px_2().py_1().rounded(px(6.0)).bg(bc::BRAND).on_press(move || c_done.finish_editing())
        .child(text("完成").text_size(px(12.0)).font_weight(FontWeight::MEDIUM).text_color(rgb_const(0xFFFFFF))),
    )
}

#[allow(clippy::too_many_arguments)]
fn modal_overlay(
  dm: Signal<bool>,
  tid: &'static str,
  width: f32,
  title: &'static str,
  body: impl IntoElement,
  cancel_tid: &'static str,
  confirm_tid: &'static str,
  confirm_label: &'static str,
  on_cancel: impl Fn() + 'static,
  on_confirm: impl Fn() + 'static,
) -> impl IntoElement {
  view()
    .test_id(tid).absolute().top(px(0.0)).left(px(0.0)).right(px(0.0)).bottom(px(0.0)).z_index(50)
    .bg(rgba_const(0x000000, 0.35)).items_center().justify_center()
    .child(
      floating_panel(dm, "board-modal-card", 12.0).w(px(width)).p_4().gap_3()
        .child(text(title).text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).text_color(move || bc::ink(dm.get())))
        .child(body)
        .child(
          view().w_full().flex_row().justify_end().gap_2()
            .child(
              button().test_id(cancel_tid).px_3().py(px(6.0)).rounded(px(6.0))
                .hover(move |s| s.bg(bc::chip(dm.get_untracked()))).on_press(on_cancel)
                .child(text("取消").text_size(px(13.0)).text_color(move || bc::muted(dm.get()))),
            )
            .child(
              button().test_id(confirm_tid).px_3().py(px(6.0)).rounded(px(6.0)).bg(bc::BRAND).on_press(on_confirm)
                .child(text(confirm_label).text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(rgb_const(0xFFFFFF))),
            ),
        ),
    )
}

fn build_edge_label_modal(ctx: &BoardCtx) -> impl IntoElement + use<> {
  let (dm, draft) = (ctx.dark_mode, ctx.editing_edge_draft);
  let (c_cancel, c_ok, c_sub) = (ctx.clone(), ctx.clone(), ctx.clone());
  let commit = move |c: &BoardCtx| {
    if let Some(edge_id) = c.editing_edge_id.get_untracked() {
      let label = c.editing_edge_draft.get_untracked();
      c.editing_edge_id.set(None);
      c.dispatch_and_handle(BoardCommand::SetEdgeLabel { edge_id, label });
    }
  };
  let field = input()
    .test_id("board-edge-label-input").w_full().px_3().py_2().rounded(px(8.0)).border_1()
    .border_color(move || bc::border(dm.get())).bg(move || bc::canvas(dm.get()))
    .text_size(px(13.0)).text_color(move || bc::ink(dm.get()))
    .placeholder("例如：是 / 否 / HTTP 请求").auto_focus(true)
    .value(move || draft.get()).on_change(move |v: &str| draft.set(v.to_owned()))
    .on_submit(move |_: &str| commit(&c_sub));

  modal_overlay(
    dm, "board-edge-label-modal", 360.0, "编辑连线文字", field,
    "board-edge-label-cancel", "board-edge-label-confirm", "确定",
    move || {
      c_cancel.editing_edge_id.set(None);
      c_cancel.sync_chrome();
    },
    move || commit(&c_ok),
  )
}

fn build_md_import_modal(ctx: &BoardCtx) -> impl IntoElement + use<> {
  let (dm, draft, c_cancel, c_ok) = (ctx.dark_mode, ctx.md_import_draft, ctx.clone(), ctx.clone());
  let field = input()
    .test_id("board-md-import-input").w_full().h(px(200.0)).p_3().multiline(true).rounded(px(8.0)).border_1()
    .border_color(move || bc::border(dm.get())).bg(move || bc::canvas(dm.get()))
    .text_size(px(13.0)).text_color(move || bc::ink(dm.get()))
    .placeholder("粘贴 # 标题 或 - 缩进列表...").auto_focus(true)
    .value(move || draft.get()).on_change(move |v: &str| draft.set(v.to_owned()));

  modal_overlay(
    dm, "board-md-import-modal", 480.0, "从 Markdown 大纲生成思维导图", field,
    "board-md-import-cancel", "board-md-import-confirm", "生成导图",
    move || c_cancel.show_md_import_modal.set(false), move || c_ok.apply_markdown_outline(),
  )
}

// ============================================================================
// Bottom-Right Zoom Pill & Bottom-Left Shortcut Hint Pill
// ============================================================================

fn build_zoom_pill(ctx: &BoardCtx) -> impl IntoElement {
  let (dm, zoom_pct) = (ctx.dark_mode, ctx.zoom_pct_sig);
  let (c_out, c_reset, c_in, c_fit) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());

  floating_panel(dm, "board-zoom-pill", 10.0)
    .absolute().right(px(16.0)).bottom(px(16.0)).z_index(22)
    .flex_row().items_center().gap(px(2.0)).px(px(6.0)).py(px(4.0))
    .child(zoom_btn(dm, "board-zoom-out", "缩小", "-", move || c_out.zoom_by(1.0 / 1.15)))
    .child(
      button().test_id("board-zoom-reset").accessibility_label("重置缩放为 100%")
        .px(px(6.0)).h(px(28.0)).items_center().justify_center().rounded(px(6.0))
        .hover(move |s| s.bg(bc::chip(dm.get_untracked()))).on_press(move || c_reset.reset_zoom())
        .child(
          text(move || format!("{}%", zoom_pct.get()))
            .text_size(px(12.0)).font_weight(FontWeight::MEDIUM).text_color(move || bc::ink(dm.get())),
        ),
    )
    .child(zoom_btn(dm, "board-zoom-in", "放大", "+", move || c_in.zoom_by(1.15)))
    .child(
      button().test_id("board-fit").accessibility_label("自适应视图")
        .px(px(6.0)).h(px(28.0)).items_center().justify_center().rounded(px(6.0))
        .hover(move |s| s.bg(bc::chip(dm.get_untracked()))).on_press(move || c_fit.fit_to_content())
        .child(text("Fit").text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(move || bc::ink(dm.get()))),
    )
}

fn zoom_btn(dm: Signal<bool>, tid: &'static str, a11y: &'static str, glyph: &'static str, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid).accessibility_label(a11y).w(px(28.0)).h(px(28.0)).items_center().justify_center().rounded(px(6.0))
    .hover(move |s| s.bg(bc::chip(dm.get_untracked()))).on_press(on_click)
    .child(text(glyph).text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(move || bc::ink(dm.get())))
}

fn build_shortcut_hint_pill(dm: Signal<bool>) -> impl IntoElement {
  view()
    .test_id("board-shortcut-hint").absolute().left(px(16.0)).bottom(px(16.0)).z_index(20)
    .px(px(10.0)).py(px(5.0)).rounded(px(8.0)).border_1()
    .border_color(move || bc::border(dm.get()))
    .bg(move || Rgba { a: 0.92, ..bc::panel(dm.get()) })
    .child(
      text("双击编辑文字 · 导图 Tab 加子节点 / Enter 加同级 · 空格+拖拽平移 · 滚轮缩放")
        .text_size(px(11.0)).text_color(move || bc::muted(dm.get())),
    )
}

// ============================================================================
// Infinite Canvas 2D Painter (1:1 Parity with `board_painter.dart`)
// ============================================================================

fn visible_content_bounds(session: &BoardSession) -> Option<(f64, f64, f64, f64)> {
  let snap = session.snapshot();
  let hidden = session.collapsed_descendants();
  let mut min_x = f64::INFINITY;
  let mut min_y = f64::INFINITY;
  let mut max_x = f64::NEG_INFINITY;
  let mut max_y = f64::NEG_INFINITY;
  for n in &snap.nodes {
    if hidden.contains(&n.id) {
      continue;
    }
    min_x = min_x.min(n.x);
    min_y = min_y.min(n.y);
    max_x = max_x.max(n.x + n.w);
    max_y = max_y.max(n.y + n.h);
  }
  for s in &snap.strokes {
    for pt in &s.points {
      min_x = min_x.min(pt[0]);
      min_y = min_y.min(pt[1]);
      max_x = max_x.max(pt[0]);
      max_y = max_y.max(pt[1]);
    }
  }
  if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
    Some((min_x, min_y, max_x, max_y))
  } else {
    None
  }
}

/// Natural height/width aspect ratio of an embedded board's content (including snug margins),
/// used to size inline board cards cleanly without clipping or excessive empty whitespace.
pub fn board_natural_aspect(json_src: &str) -> f32 {
  let payload = if json_src.trim().is_empty() {
    mica_board::starter_board_json("mindmap")
  } else {
    json_src.to_string()
  };
  let session = BoardSession::from_json(&payload);
  let Some((min_x, min_y, max_x, max_y)) = visible_content_bounds(&session) else {
    return 0.52;
  };
  let cw = (max_x - min_x).max(160.0) + 48.0;
  let ch = (max_y - min_y).max(120.0) + 48.0;
  ((ch / cw) as f32).clamp(0.36, 0.85)
}

/// Paint a read-only embedded board preview snugly fitted to `viewport` (`(w, h)`)
/// using the exact same native 2D canvas painter as the full whiteboard editor
/// (tight 20px margins instead of fullscreen-toolbar margins, crisp text, icons, and dot grid).
pub fn paint_board_preview(json_src: &str, dark: bool, viewport: (f32, f32)) -> Painter {
  let payload = if json_src.trim().is_empty() {
    mica_board::starter_board_json("mindmap")
  } else {
    json_src.to_string()
  };
  let mut session = BoardSession::from_json(&payload);
  let (vw, vh) = (viewport.0.max(200.0) as f64, viewport.1.max(140.0) as f64);
  if let Some((min_x, min_y, max_x, max_y)) = visible_content_bounds(&session) {
    let content_w = (max_x - min_x).max(120.0);
    let content_h = (max_y - min_y).max(80.0);
    let pad = 20.0_f64;
    let avail_w = (vw - pad * 2.0).max(80.0);
    let avail_h = (vh - pad * 2.0).max(80.0);
    let zoom = (avail_w / content_w).min(avail_h / content_h).clamp(0.15, 1.8);
    let cx = (min_x + max_x) * 0.5;
    let cy = (min_y + max_y) * 0.5;
    let pan = [vw * 0.5 - cx * zoom, vh * 0.5 - cy * zoom];
    session.set_viewport(zoom, pan);
  } else {
    session.set_viewport(1.0, [vw * 0.15, vh * 0.15]);
  }
  paint_board(&session, dark, (vw as f32, vh as f32))
}

fn paint_board(s: &BoardSession, dark: bool, viewport: (f32, f32)) -> Painter {
  let snap = s.snapshot();
  let [pan_x, pan_y] = s.pan();
  let zoom = s.zoom().clamp(0.1, 8.0) as f32;
  let inv_z = 1.0 / zoom;
  let (vw, vh) = (viewport.0.max(320.0), viewport.1.max(240.0));

  let mut p = Painter::new()
    .fill_color(bc::canvas(dark))
    .fill_rect(px(0.0), px(0.0), px(vw), px(vh));
  p = paint_dot_grid(p, pan_x as f32, pan_y as f32, zoom, vw, vh, dark);
  p = p.save().translate(px(pan_x as f32), px(pan_y as f32)).scale(zoom, zoom);

  let hidden = s.collapsed_descendants();
  let mut visible_nodes: Vec<&BoardNode> = snap.nodes.iter().filter(|n| !hidden.contains(&n.id)).collect();
  visible_nodes.sort_by_key(|n| (n.kind != "frame", n.z));
  let visible_ids: HashSet<&str> = visible_nodes.iter().map(|n| n.id.as_str()).collect();

  for node in visible_nodes.iter().filter(|n| n.kind == "frame") { p = paint_node(p, node, dark); }
  let sel_edge_id = s.selected_edge_id();
  for edge in &snap.edges {
    if !visible_ids.contains(edge.from_id.as_str()) || !visible_ids.contains(edge.to_id.as_str()) { continue; }
    if let Some(pts) = s.routed_edges().get(&edge.id) && pts.len() >= 2 {
      p = paint_edge(p, edge, pts, sel_edge_id == Some(edge.id.as_str()), dark);
    }
  }
  for node in visible_nodes.iter().filter(|n| n.kind != "frame") { p = paint_node(p, node, dark); }
  p = paint_strokes(p, s);
  p = paint_collapse_badges(p, snap, hidden, dark);
  p = paint_overlays(p, s, &visible_nodes, inv_z);
  p.restore()
}

/// Renders the infinite dot grid in a single batched dashed-stroke path
/// (~30 horizontal dashed segments -> 1 Direct2D geometry + 1 brush per frame)
/// instead of thousands of individual `fill_rect` COM geometries.
fn paint_dot_grid(mut p: Painter, pan_x: f32, pan_y: f32, zoom: f32, vw: f32, vh: f32, dark: bool) -> Painter {
  let mut world_step = 28.0f32;
  while world_step * zoom < 14.0 { world_step *= 2.0; }
  let mut step = world_step * zoom;
  if !step.is_finite() || step < 8.0 { return p; }
  while (vh / step) > 120.0 { step *= 2.0; }
  let (start_x, start_y) = (pan_x.rem_euclid(step), pan_y.rem_euclid(step));
  if !start_x.is_finite() || !start_y.is_finite() { return p; }
  let dot_size = (1.6 * zoom).clamp(1.2, 2.4);
  let half = dot_size * 0.5;
  let gap = (step - dot_size).max(1.0);
  p = p
    .save()
    .stroke_color(if dark { bc::DOT_DARK } else { bc::DOT_LIGHT })
    .line_width(px(dot_size))
    .line_cap(LineCap::Butt)
    .line_dash(&[px(dot_size), px(gap)])
    .begin_path();
  let x0 = start_x - half;
  let x1 = vw + step;
  let mut y = start_y;
  while y <= vh + step {
    p = p.move_to(px(x0), px(y)).line_to(px(x1), px(y));
    y += step;
  }
  p.stroke().restore()
}

fn paint_edge(mut p: Painter, edge: &BoardEdge, pts: &[[f64; 2]], selected: bool, dark: bool) -> Painter {
  let stroke_col = if selected { bc::BRAND } else { parse_hex_rgba(&edge.style.stroke).unwrap_or(bc::MUTED_LIGHT) };
  let sw = (edge.style.stroke_width as f32).max(1.2) + if selected { 0.8 } else { 0.0 };
  p = p.save().stroke_color(stroke_col).line_width(px(sw)).line_cap(LineCap::Round).line_join(LineJoin::Round);
  if edge.style.dashed { p = p.line_dash(&[px(6.0), px(4.0)]); }

  let (mid, start_dir, end_dir) = if edge.routing == "curved" && pts.len() == 4 {
    p = p.begin_path().move_to(px(pts[0][0] as f32), px(pts[0][1] as f32))
      .bezier_curve_to(point(px(pts[1][0] as f32), px(pts[1][1] as f32)), point(px(pts[2][0] as f32), px(pts[2][1] as f32)), point(px(pts[3][0] as f32), px(pts[3][1] as f32)))
      .stroke();
    let mid = [
      0.125 * pts[0][0] + 0.375 * pts[1][0] + 0.375 * pts[2][0] + 0.125 * pts[3][0],
      0.125 * pts[0][1] + 0.375 * pts[1][1] + 0.375 * pts[2][1] + 0.125 * pts[3][1],
    ];
    (mid, [pts[0][0] - pts[1][0], pts[0][1] - pts[1][1]], [pts[3][0] - pts[2][0], pts[3][1] - pts[2][1]])
  } else {
    p = if edge.routing == "orthogonal" && pts.len() >= 3 {
      paint_rounded_orthogonal(p, pts, 10.0)
    } else {
      let mut q = p.begin_path().move_to(px(pts[0][0] as f32), px(pts[0][1] as f32));
      for pt in &pts[1..] { q = q.line_to(px(pt[0] as f32), px(pt[1] as f32)); }
      q.stroke()
    };
    let n = pts.len();
    (polyline_midpoint(pts), [pts[0][0] - pts[1][0], pts[0][1] - pts[1][1]], [pts[n - 1][0] - pts[n - 2][0], pts[n - 1][1] - pts[n - 2][1]])
  };

  p = p.line_dash(&[]);
  if let Some(&tip) = pts.last() { p = paint_edge_marker(p, &edge.arrow_end, tip, end_dir, stroke_col); }
  p = paint_edge_marker(p, &edge.arrow_start, pts[0], start_dir, stroke_col);
  if !edge.label.trim().is_empty() { p = paint_edge_label(p, edge.label.trim(), mid, stroke_col, dark); }
  p.restore()
}

fn paint_rounded_orthogonal(mut p: Painter, pts: &[[f64; 2]], radius: f64) -> Painter {
  p = p.begin_path().move_to(px(pts[0][0] as f32), px(pts[0][1] as f32));
  for i in 1..pts.len() - 1 {
    let (prev, curr, next) = (pts[i - 1], pts[i], pts[i + 1]);
    let (v1, v2) = ([prev[0] - curr[0], prev[1] - curr[1]], [next[0] - curr[0], next[1] - curr[1]]);
    let (d1, d2) = (v1[0].hypot(v1[1]), v2[0].hypot(v2[1]));
    if d1 < 1e-3 || d2 < 1e-3 { continue; }
    let r = radius.min(d1 * 0.5).min(d2 * 0.5);
    let before = [curr[0] + (v1[0] / d1) * r, curr[1] + (v1[1] / d1) * r];
    let after = [curr[0] + (v2[0] / d2) * r, curr[1] + (v2[1] / d2) * r];
    p = p.line_to(px(before[0] as f32), px(before[1] as f32))
      .quadratic_curve_to(point(px(curr[0] as f32), px(curr[1] as f32)), point(px(after[0] as f32), px(after[1] as f32)));
  }
  let last = pts[pts.len() - 1];
  p.line_to(px(last[0] as f32), px(last[1] as f32)).stroke()
}

fn paint_edge_marker(p: Painter, marker: &str, tip: [f64; 2], dir: [f64; 2], color: Rgba) -> Painter {
  match marker {
    "arrow" => paint_arrow_head(p, tip, dir, color),
    "dot" => p.fill_color(color).begin_path().arc(px(tip[0] as f32), px(tip[1] as f32), px(3.5), radians(0.0), radians(TAU)).fill(),
    "diamond" => {
      let len = dir[0].hypot(dir[1]);
      if len < 1e-3 { return p; }
      let (ux, uy) = (dir[0] / len, dir[1] / len);
      let (mx, my) = (tip[0] - ux * 5.0, tip[1] - uy * 5.0);
      p.fill_color(color).begin_path()
        .move_to(px(tip[0] as f32), px(tip[1] as f32))
        .line_to(px((mx - uy * 4.0) as f32), px((my + ux * 4.0) as f32))
        .line_to(px((tip[0] - ux * 10.0) as f32), px((tip[1] - uy * 10.0) as f32))
        .line_to(px((mx + uy * 4.0) as f32), px((my - ux * 4.0) as f32))
        .close_path().fill()
    }
    _ => p,
  }
}

fn paint_arrow_head(p: Painter, tip: [f64; 2], dir: [f64; 2], color: Rgba) -> Painter {
  let len = dir[0].hypot(dir[1]);
  if len < 1e-3 { return p; }
  let (ux, uy) = (dir[0] / len, dir[1] / len);
  let (bx, by) = (tip[0] - ux * 9.0, tip[1] - uy * 9.0);
  p.fill_color(color).begin_path()
    .move_to(px(tip[0] as f32), px(tip[1] as f32))
    .line_to(px((bx - uy * 4.5) as f32), px((by + ux * 4.5) as f32))
    .line_to(px((bx + uy * 4.5) as f32), px((by - ux * 4.5) as f32))
    .close_path().fill()
}

fn paint_edge_label(p: Painter, label: &str, mid: [f64; 2], border: Rgba, dark: bool) -> Painter {
  let est_w = (label.chars().map(|c| if c.is_ascii() { 7.2 } else { 13.0 }).sum::<f32>() + 12.0).max(28.0);
  let rect = Bounds::new(point(px(mid[0] as f32 - est_w * 0.5), px(mid[1] as f32 - 10.0)), size(px(est_w), px(20.0)));
  p.fill_color(if dark { bc::SURFACE_DARK } else { rgb_const(0xFFFFFF) })
    .stroke_color(Rgba { a: 0.6, ..border }).line_width(px(0.8))
    .begin_path().round_rect(rect, px(4.0)).fill().stroke()
    .fill_color(bc::ink(dark)).font("12px system-ui")
    .text_align(CanvasTextAlign::Center).text_baseline(CanvasTextBaseline::Middle)
    .fill_text(label, px(mid[0] as f32), px(mid[1] as f32))
}

fn paint_node(mut p: Painter, node: &BoardNode, dark: bool) -> Painter {
  let (x, y, w, h) = (node.x as f32, node.y as f32, node.w.max(8.0) as f32, node.h.max(8.0) as f32);
  let (cx, cy) = (x + w * 0.5, y + h * 0.5);
  let adapted_white = dark && node.style.fill.eq_ignore_ascii_case("#FFFFFF");
  let fill_opt = if node.style.fill == "transparent" || node.style.fill == "none" {
    None
  } else if adapted_white {
    Some(bc::SURFACE_DARK)
  } else {
    parse_hex_rgba(&node.style.fill).or(Some(rgb_const(0xFFFFFF)))
  };
  let stroke_opt = if node.style.stroke == "transparent" || node.style.stroke == "none" {
    None
  } else {
    parse_hex_rgba(&node.style.stroke).or(Some(bc::DEFAULT_STROKE))
  };
  let sw = (node.style.stroke_width as f32).max(0.0);

  p = p.save();
  if node.rot.abs() > 0.1 {
    p = p.translate(px(cx), px(cy)).rotate(radians((node.rot as f32).to_radians())).translate(px(-cx), px(-cy));
  }

  if node.kind == "cylinder" {
    p = paint_cylinder(p, x, y, w, h, fill_opt, stroke_opt, sw, node.style.dashed);
  } else if node.kind == "frame" {
    let rect = Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
    if let Some(fill) = fill_opt {
      p = p.fill_color(Rgba { a: if dark { 0.18 } else { 0.70 }, ..fill }).begin_path().round_rect(rect, px(10.0)).fill();
    }
    if let Some(stroke) = stroke_opt && sw > 0.0 {
      p = p.stroke_color(stroke).line_width(px(sw)).line_join(LineJoin::Round);
      if node.style.dashed { p = p.line_dash(&[px(6.0), px(4.0)]); }
      p = p.begin_path().round_rect(rect, px(10.0)).stroke().line_dash(&[]);
    }
  } else {
    let elevated = matches!(node.kind.as_str(), "sticky" | "rounded" | "mind_root" | "mind_node");
    if let Some(fill) = fill_opt {
      if elevated {
        // Crisp lightweight offset drop shadow (avoids per-node Direct2D Gaussian
        // blur effect pipelines and nested command lists during 60-120fps drag).
        let is_sticky = node.kind == "sticky";
        let (dy, alpha) = match (is_sticky, dark) {
          (true, true) => (3.5, 0.24),
          (true, false) => (3.0, 0.08),
          (false, true) => (2.0, 0.16),
          (false, false) => (1.8, 0.05),
        };
        p = build_node_shape_path(p.fill_color(rgba_const(0x000000, alpha)), node, x, y + dy, w, h).fill();
      }
      p = build_node_shape_path(p.fill_color(fill), node, x, y, w, h).fill();
    }
    if let Some(stroke) = stroke_opt && sw > 0.0 {
      p = p.stroke_color(stroke).line_width(px(sw)).line_join(LineJoin::Round);
      if node.style.dashed { p = p.line_dash(&[px(6.0), px(4.0)]); }
      p = build_node_shape_path(p, node, x, y, w, h).stroke().line_dash(&[]);
      if node.kind == "sticky" {
        let fold = 14.0f32.min(w * 0.25).min(h * 0.35);
        p = p.fill_color(rgba_const(0x000000, 0.06)).begin_path()
          .move_to(px(x + w - fold), px(y + h)).line_to(px(x + w - fold), px(y + h - fold)).line_to(px(x + w), px(y + h - fold)).close_path().fill()
          .stroke_color(stroke).line_width(px(1.0)).begin_path()
          .move_to(px(x + w - fold), px(y + h)).line_to(px(x + w - fold), px(y + h - fold)).line_to(px(x + w), px(y + h - fold)).stroke();
      }
    }
  }

  let mut text_top_shift = 0.0f32;
  if let Some(icon_id) = node.icon.as_deref() && !icon_id.is_empty() {
    p = paint_tech_icon_on_canvas(p, icon_id, cx - 13.0, y + 11.0, 26.0, stroke_opt.unwrap_or(bc::BRAND));
    text_top_shift = 15.0;
  }
  if !node.text.is_empty() {
    let text_col = if dark && node.style.text_color.eq_ignore_ascii_case("#1F2329") && (adapted_white || fill_opt.is_none()) {
      bc::INK_DARK
    } else {
      parse_hex_rgba(&node.style.text_color).unwrap_or_else(|| bc::ink(dark))
    };
    p = paint_node_text(p, node, x, y, w, h, text_top_shift, text_col);
  }
  p.restore()
}

#[allow(clippy::too_many_arguments)]
fn paint_cylinder(mut p: Painter, x: f32, y: f32, w: f32, h: f32, fill_opt: Option<Rgba>, stroke_opt: Option<Rgba>, sw: f32, dashed: bool) -> Painter {
  let (rx, ry) = (w * 0.5, (h * 0.16).clamp(6.0, 14.0));
  let (cx, top_y, bot_y) = (x + rx, y + ry, y + h - ry);
  let body = |q: Painter| {
    q.begin_path().move_to(px(x), px(top_y))
      .ellipse(point(px(cx), px(top_y)), size(px(rx), px(ry)), radians(0.0), radians(PI), radians(TAU), false)
      .line_to(px(x + w), px(bot_y))
      .ellipse(point(px(cx), px(bot_y)), size(px(rx), px(ry)), radians(0.0), radians(0.0), radians(PI), false)
      .close_path()
  };
  if let Some(fill) = fill_opt { p = body(p.fill_color(fill)).fill(); }
  if let Some(stroke) = stroke_opt && sw > 0.0 {
    p = p.stroke_color(stroke).line_width(px(sw)).line_join(LineJoin::Round);
    if dashed { p = p.line_dash(&[px(6.0), px(4.0)]); }
    p = body(p).stroke().line_dash(&[]);
  }
  if let Some(fill) = fill_opt {
    p = p.fill_color(fill).begin_path()
      .ellipse(point(px(cx), px(top_y)), size(px(rx), px(ry)), radians(0.0), radians(0.0), radians(TAU), false).fill();
  }
  if let Some(stroke) = stroke_opt && sw > 0.0 {
    p = p.stroke_color(stroke).line_width(px(sw)).begin_path()
      .ellipse(point(px(cx), px(top_y)), size(px(rx), px(ry)), radians(0.0), radians(0.0), radians(TAU), false).stroke();
  }
  p
}

fn build_node_shape_path(p: Painter, node: &BoardNode, x: f32, y: f32, w: f32, h: f32) -> Painter {
  let rect = Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
  let (cx, cy) = (x + w * 0.5, y + h * 0.5);
  match node.kind.as_str() {
    "rect" => p.begin_path().rect(rect),
    "pill" => p.begin_path().round_rect(rect, px((h * 0.5).min(w * 0.5))),
    "ellipse" => p.begin_path().ellipse(point(px(cx), px(cy)), size(px(w * 0.5), px(h * 0.5)), radians(0.0), radians(0.0), radians(TAU), false),
    "diamond" => p.begin_path().move_to(px(cx), px(y)).line_to(px(x + w), px(cy)).line_to(px(cx), px(y + h)).line_to(px(x), px(cy)).close_path(),
    "parallelogram" => {
      let sk = (w * 0.14).min(24.0);
      p.begin_path().move_to(px(x + sk), px(y)).line_to(px(x + w), px(y)).line_to(px(x + w - sk), px(y + h)).line_to(px(x), px(y + h)).close_path()
    }
    "hexagon" => {
      let sk = (w * 0.16).min(24.0);
      p.begin_path().move_to(px(x + sk), px(y)).line_to(px(x + w - sk), px(y)).line_to(px(x + w), px(cy))
        .line_to(px(x + w - sk), px(y + h)).line_to(px(x + sk), px(y + h)).line_to(px(x), px(cy)).close_path()
    }
    "triangle" => p.begin_path().move_to(px(cx), px(y)).line_to(px(x + w), px(y + h)).line_to(px(x), px(y + h)).close_path(),
    "sticky" => {
      let fold = 14.0f32.min(w * 0.25).min(h * 0.35);
      p.begin_path().move_to(px(x), px(y)).line_to(px(x + w), px(y)).line_to(px(x + w), px(y + h - fold))
        .line_to(px(x + w - fold), px(y + h)).line_to(px(x), px(y + h)).close_path()
    }
    "document" => {
      let wave = (h * 0.14).clamp(6.0, 14.0);
      let by = y + h - wave;
      p.begin_path().move_to(px(x), px(y)).line_to(px(x + w), px(y)).line_to(px(x + w), px(by))
        .quadratic_curve_to(point(px(x + w * 0.75), px(by - wave)), point(px(cx), px(by)))
        .quadratic_curve_to(point(px(x + w * 0.25), px(by + wave)), point(px(x), px(by))).close_path()
    }
    "cloud" => p.begin_path().move_to(px(x + w * 0.25), px(y + h * 0.82))
      .bezier_curve_to(point(px(x + w * 0.04), px(y + h * 0.82)), point(px(x + w * 0.04), px(y + h * 0.45)), point(px(x + w * 0.22), px(y + h * 0.42)))
      .bezier_curve_to(point(px(x + w * 0.24), px(y + h * 0.12)), point(px(x + w * 0.58), px(y + h * 0.10)), point(px(x + w * 0.68), px(y + h * 0.30)))
      .bezier_curve_to(point(px(x + w * 0.96), px(y + h * 0.30)), point(px(x + w * 0.98), px(y + h * 0.82)), point(px(x + w * 0.78), px(y + h * 0.82)))
      .close_path(),
    "text" => p.begin_path().round_rect(rect, px(4.0)),
    _ => p.begin_path().round_rect(rect, px(if node.kind == "mind_root" { 10.0 } else { 8.0 })),
  }
}

#[allow(clippy::too_many_arguments)]
fn paint_node_text(mut p: Painter, node: &BoardNode, x: f32, y: f32, w: f32, h: f32, top_shift: f32, text_col: Rgba) -> Painter {
  if node.kind == "frame" {
    let title = node.text.lines().next().unwrap_or("");
    return p.fill_color(text_col).font("600 12.5px system-ui")
      .text_align(CanvasTextAlign::Left).text_baseline(CanvasTextBaseline::Middle)
      .fill_text_max_width(title, px(x + 12.0), px(y + 15.0), px((w - 24.0).max(24.0)));
  }
  let fs = (node.style.font_size as f32).clamp(9.0, 32.0);
  let font_spec = if node.kind == "mind_root" { format!("600 {fs:.1}px system-ui") } else { format!("{fs:.1}px system-ui") };
  let pad_x = if node.kind == "diamond" { 22.0 } else { 10.0 };
  let max_w = (w - pad_x * 2.0).max(24.0);
  let lines = wrap_text_lines(&node.text, max_w, fs);
  let line_h = fs * 1.36;
  let start_y = (y + h * 0.5 + top_shift - (lines.len() as f32) * line_h * 0.5).clamp(y + 8.0, (y + h - 8.0).max(y + 8.0)) + line_h * 0.5;
  let (align, tx) = match node.style.align.as_str() {
    "left" => (CanvasTextAlign::Left, x + pad_x),
    "right" => (CanvasTextAlign::Right, x + w - pad_x),
    _ => (CanvasTextAlign::Center, x + w * 0.5),
  };
  p = p.fill_color(text_col).font(&font_spec).text_align(align).text_baseline(CanvasTextBaseline::Middle);
  for (i, line) in lines.iter().enumerate() {
    p = p.fill_text_max_width(line, px(tx), px(start_y + (i as f32) * line_h), px(max_w));
  }
  p
}

/// Smooth quadratic-Bezier-midpoint freehand pen strokes (1:1 with `board_painter.dart`).
fn paint_strokes(mut p: Painter, s: &BoardSession) -> Painter {
  p = p.save().line_cap(LineCap::Round).line_join(LineJoin::Round);
  let draw_one = |pts: &[[f64; 2]], color: &str, width: f64, mut q: Painter| -> Painter {
    if pts.len() < 2 { return q; }
    q = q.stroke_color(parse_hex_rgba(color).unwrap_or(bc::BRAND)).line_width(px((width as f32).max(1.2)))
      .begin_path().move_to(px(pts[0][0] as f32), px(pts[0][1] as f32));
    for i in 1..pts.len() - 1 {
      let (p0, p1) = (pts[i], pts[i + 1]);
      let mid = [(p0[0] + p1[0]) * 0.5, (p0[1] + p1[1]) * 0.5];
      q = q.quadratic_curve_to(point(px(p0[0] as f32), px(p0[1] as f32)), point(px(mid[0] as f32), px(mid[1] as f32)));
    }
    let last = pts[pts.len() - 1];
    q.line_to(px(last[0] as f32), px(last[1] as f32)).stroke()
  };
  for stroke in &s.snapshot().strokes { p = draw_one(&stroke.points, &stroke.color, stroke.width, p); }
  if let GestureState::Stroke(active) = s.gesture() {
    p = draw_one(&active.stroke.points, &active.stroke.color, active.stroke.width, p);
  }
  p.restore()
}

/// Mind-map collapse/expand badges (`r=8` circle at `(x + w + 10, y + h / 2)`).
fn paint_collapse_badges(mut p: Painter, snap: &BoardSnapshot, hidden: &HashSet<String>, dark: bool) -> Painter {
  let mut child_count: HashMap<&str, usize> = HashMap::new();
  for n in &snap.nodes {
    if let Some(pid) = n.parent_id.as_deref() { *child_count.entry(pid).or_insert(0) += 1; }
  }
  for n in &snap.nodes {
    if hidden.contains(&n.id) || (n.kind != "mind_root" && n.kind != "mind_node") { continue; }
    let Some(&count) = child_count.get(n.id.as_str()) else { continue; };
    if count == 0 { continue; }
    let (cx, cy) = ((n.x + n.w + 10.0) as f32, (n.y + n.h * 0.5) as f32);
    let bg = if n.collapsed { bc::BRAND } else if dark { bc::CHIP_DARK } else { rgb_const(0xFFFFFF) };
    let fg = if n.collapsed { rgb_const(0xFFFFFF) } else { bc::BRAND };
    let label = if n.collapsed { count.to_string() } else { "-".to_string() };
    p = p.fill_color(bg).stroke_color(bc::BRAND).line_width(px(1.2))
      .begin_path().arc(px(cx), px(cy), px(8.0), radians(0.0), radians(TAU)).fill().stroke()
      .fill_color(fg).font("600 11px system-ui")
      .text_align(CanvasTextAlign::Center).text_baseline(CanvasTextBaseline::Middle)
      .fill_text(&label, px(cx), px(cy));
  }
  p
}

fn paint_overlays(mut p: Painter, s: &BoardSession, visible_nodes: &[&BoardNode], inv_z: f32) -> Painter {
  let (accent, selected_ids, editing_id) = (bc::BRAND, s.selected_node_ids(), s.editing_node_id());

  for node in visible_nodes {
    if !selected_ids.contains(&node.id) || editing_id == Some(node.id.as_str()) { continue; }
    let pad = 2.0 * inv_z;
    let halo = Bounds::new(point(px(node.x as f32 - pad), px(node.y as f32 - pad)), size(px(node.w as f32 + pad * 2.0), px(node.h as f32 + pad * 2.0)));
    p = p.stroke_color(accent).line_width(px(1.8 * inv_z)).begin_path().rect(halo).stroke();
  }

  // All 8 resize handles ONLY on single selection & not editing
  if selected_ids.len() == 1 && editing_id.is_none()
    && let Some(node) = visible_nodes.iter().find(|n| n.id == selected_ids[0])
  {
    p = paint_8_resize_handles(p, node, inv_z, accent);
  }

  // 4 anchor ports on single-selected AND hovered non-frame nodes
  let mut port_targets: Vec<&str> = Vec::with_capacity(2);
  if selected_ids.len() == 1 { port_targets.push(selected_ids[0].as_str()); }
  if let Some(hid) = s.hovered_node_id() && !port_targets.contains(&hid) { port_targets.push(hid); }
  for tid in port_targets {
    if editing_id != Some(tid)
      && let Some(node) = visible_nodes.iter().find(|n| n.id == tid)
      && node.kind != "frame"
    {
      p = paint_anchor_ports(p, node, inv_z, accent);
    }
  }

  for guide in s.snap_guides() { p = paint_snap_guide(p, guide, inv_z, bc::GUIDE); }
  if let GestureState::Connector(pc) = s.gesture() { p = paint_pending_connector(p, pc, inv_z, accent); }
  if let GestureState::Marquee(m) = s.gesture() {
    let rect = Bounds::new(point(px(m.rect.x as f32), px(m.rect.y as f32)), size(px(m.rect.w as f32), px(m.rect.h as f32)));
    p = p.save().fill_color(bc::BRAND_MARQUEE).stroke_color(accent).line_width(px(1.2 * inv_z))
      .begin_path().rect(rect).fill().stroke().restore();
  }
  p
}

fn paint_8_resize_handles(mut p: Painter, node: &BoardNode, inv_z: f32, accent: Rgba) -> Painter {
  let (hs, half) = (6.5 * inv_z, 3.25 * inv_z);
  let (x, y, w, h) = (node.x as f32, node.y as f32, node.w as f32, node.h as f32);
  let (cx, cy) = (x + w * 0.5, y + h * 0.5);
  for (hx, hy) in [(x, y), (cx, y), (x + w, y), (x + w, cy), (x + w, y + h), (cx, y + h), (x, y + h), (x, cy)] {
    let hr = Bounds::new(point(px(hx - half), px(hy - half)), size(px(hs), px(hs)));
    p = p.fill_color(rgb_const(0xFFFFFF)).stroke_color(accent).line_width(px(1.4 * inv_z)).begin_path().rect(hr).fill().stroke();
  }
  p
}

fn paint_anchor_ports(mut p: Painter, node: &BoardNode, inv_z: f32, accent: Rgba) -> Painter {
  let r = 4.2 * inv_z;
  for (ax, ay) in [
    ((node.x + node.w * 0.5) as f32, node.y as f32),
    ((node.x + node.w) as f32, (node.y + node.h * 0.5) as f32),
    ((node.x + node.w * 0.5) as f32, (node.y + node.h) as f32),
    (node.x as f32, (node.y + node.h * 0.5) as f32),
  ] {
    p = p.fill_color(accent).stroke_color(rgb_const(0xFFFFFF)).line_width(px(1.5 * inv_z))
      .begin_path().arc(px(ax), px(ay), px(r), radians(0.0), radians(TAU)).fill().stroke();
  }
  p
}

fn paint_snap_guide(p: Painter, g: &SnapGuide, inv_z: f32, color: Rgba) -> Painter {
  let (x1, y1, x2, y2) = if g.vertical { (g.coord as f32, g.start as f32, g.coord as f32, g.end as f32) } else { (g.start as f32, g.coord as f32, g.end as f32, g.coord as f32) };
  p.save().stroke_color(color).line_width(px(1.0 * inv_z)).line_dash(&[px(4.5 * inv_z), px(3.5 * inv_z)])
    .begin_path().move_to(px(x1), px(y1)).line_to(px(x2), px(y2)).stroke().restore()
}

fn paint_pending_connector(p: Painter, pc: &PendingConnector, inv_z: f32, accent: Rgba) -> Painter {
  let (from, to) = (pc.from_world, pc.current_world);
  let p = p.save().stroke_color(accent).line_width(px(2.0 * inv_z))
    .begin_path().move_to(px(from[0] as f32), px(from[1] as f32)).line_to(px(to[0] as f32), px(to[1] as f32)).stroke();
  paint_arrow_head(p, to, [to[0] - from[0], to[1] - from[1]], accent).restore()
}

/// Paint any of the 20 technical architecture vector icons directly onto `Painter` (24×24 space).
fn paint_tech_icon_on_canvas(mut p: Painter, id: &str, left: f32, top: f32, box_size: f32, color: Rgba) -> Painter {
  let scale = box_size / 24.0;
  p = p.save().translate(px(left), px(top)).scale(scale, scale)
    .stroke_color(color).fill_color(color).line_width(px(1.8)).line_cap(LineCap::Round).line_join(LineJoin::Round);
  let rr = |x: f32, y: f32, w: f32, h: f32| Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
  p = match id {
    "server" => p.begin_path().round_rect(rr(3.0, 4.0, 18.0, 6.0), px(1.5)).round_rect(rr(3.0, 14.0, 18.0, 6.0), px(1.5)).stroke()
      .begin_path().arc(px(7.0), px(7.0), px(1.1), radians(0.0), radians(TAU)).arc(px(7.0), px(17.0), px(1.1), radians(0.0), radians(TAU)).fill(),
    "database" => p.begin_path().ellipse(point(px(12.0), px(5.5)), size(px(7.5), px(2.7)), radians(0.0), radians(0.0), radians(TAU), false)
      .move_to(px(4.5), px(5.5)).line_to(px(4.5), px(18.5)).move_to(px(19.5), px(5.5)).line_to(px(19.5), px(18.5)).stroke()
      .begin_path().ellipse(point(px(12.0), px(18.5)), size(px(7.5), px(2.7)), radians(0.0), radians(0.0), radians(PI), false).stroke()
      .begin_path().ellipse(point(px(12.0), px(12.0)), size(px(7.5), px(2.7)), radians(0.0), radians(0.0), radians(PI), false).stroke(),
    "cache" => p.begin_path().move_to(px(13.0), px(2.0)).line_to(px(4.0), px(14.0)).line_to(px(11.0), px(14.0)).line_to(px(10.0), px(22.0)).line_to(px(20.0), px(10.0)).line_to(px(13.0), px(10.0)).close_path().stroke(),
    "queue" => p.begin_path().round_rect(rr(3.0, 5.0, 4.0, 14.0), px(1.0)).round_rect(rr(10.0, 5.0, 4.0, 14.0), px(1.0)).round_rect(rr(17.0, 5.0, 4.0, 14.0), px(1.0)).stroke(),
    "gateway" => p.begin_path().arc(px(12.0), px(12.0), px(8.0), radians(0.0), radians(TAU)).move_to(px(4.0), px(12.0)).line_to(px(20.0), px(12.0)).stroke()
      .begin_path().ellipse(point(px(12.0), px(12.0)), size(px(4.0), px(8.0)), radians(0.0), radians(0.0), radians(TAU), false).stroke(),
    "cloud" => p.begin_path().move_to(px(7.0), px(18.0)).line_to(px(17.5), px(18.0))
      .bezier_curve_to(point(px(21.0), px(18.0)), point(px(21.5), px(11.0)), point(px(17.5), px(10.5)))
      .bezier_curve_to(point(px(16.5), px(5.5)), point(px(9.0), px(5.5)), point(px(7.6), px(9.5)))
      .bezier_curve_to(point(px(3.5), px(10.0)), point(px(3.5), px(18.0)), point(px(7.0), px(18.0))).close_path().stroke(),
    "browser" => p.begin_path().round_rect(rr(3.0, 4.0, 18.0, 16.0), px(2.0)).move_to(px(3.0), px(9.0)).line_to(px(21.0), px(9.0)).stroke()
      .begin_path().arc(px(6.5), px(6.5), px(0.9), radians(0.0), radians(TAU)).arc(px(9.5), px(6.5), px(0.9), radians(0.0), radians(TAU)).fill(),
    "mobile" => p.begin_path().round_rect(rr(6.5, 2.5, 11.0, 19.0), px(2.0)).move_to(px(10.5), px(18.5)).line_to(px(13.5), px(18.5)).stroke(),
    "user" => p.begin_path().arc(px(12.0), px(8.0), px(3.5), radians(0.0), radians(TAU)).stroke()
      .begin_path().ellipse(point(px(12.0), px(20.0)), size(px(7.0), px(6.0)), radians(0.0), radians(PI), radians(TAU), false).stroke(),
    "lock" => p.begin_path().round_rect(rr(5.0, 11.0, 14.0, 10.0), px(2.0)).stroke()
      .begin_path().ellipse(point(px(12.0), px(11.0)), size(px(4.0), px(6.5)), radians(0.0), radians(PI), radians(TAU), false).stroke(),
    "api" => p.begin_path().move_to(px(8.0), px(7.0)).line_to(px(4.0), px(12.0)).line_to(px(8.0), px(17.0))
      .move_to(px(16.0), px(7.0)).line_to(px(20.0), px(12.0)).line_to(px(16.0), px(17.0)).move_to(px(13.5), px(5.0)).line_to(px(10.5), px(19.0)).stroke(),
    "container" => p.begin_path().move_to(px(12.0), px(3.0)).line_to(px(20.0), px(7.5)).line_to(px(20.0), px(16.5)).line_to(px(12.0), px(21.0)).line_to(px(4.0), px(16.5)).line_to(px(4.0), px(7.5)).close_path()
      .move_to(px(12.0), px(12.0)).line_to(px(20.0), px(7.5)).move_to(px(12.0), px(12.0)).line_to(px(4.0), px(7.5)).move_to(px(12.0), px(12.0)).line_to(px(12.0), px(21.0)).stroke(),
    "storage" => p.begin_path().round_rect(rr(3.0, 5.0, 18.0, 14.0), px(2.0)).move_to(px(3.0), px(10.0)).line_to(px(21.0), px(10.0)).move_to(px(8.0), px(14.5)).line_to(px(11.5), px(14.5)).stroke(),
    "ai" => p.begin_path().move_to(px(12.0), px(3.0)).line_to(px(14.2), px(8.8)).line_to(px(20.0), px(11.0)).line_to(px(14.2), px(13.2)).line_to(px(12.0), px(19.0)).line_to(px(9.8), px(13.2)).line_to(px(4.0), px(11.0)).line_to(px(9.8), px(8.8)).close_path().stroke(),
    "monitor" => p.begin_path().round_rect(rr(3.0, 4.0, 18.0, 12.0), px(2.0))
      .move_to(px(6.5), px(12.0)).line_to(px(9.5), px(9.0)).line_to(px(12.0), px(11.0)).line_to(px(15.0), px(7.0)).line_to(px(17.5), px(9.0))
      .move_to(px(8.0), px(20.0)).line_to(px(16.0), px(20.0)).move_to(px(12.0), px(16.0)).line_to(px(12.0), px(20.0)).stroke(),
    "cdn" => {
      let mut q = p.begin_path().arc(px(12.0), px(12.0), px(2.8), radians(0.0), radians(TAU)).stroke();
      for (cx, cy) in [(5.0f32, 6.0), (19.0, 6.0), (5.0, 18.0), (19.0, 18.0)] {
        q = q.begin_path().arc(px(cx), px(cy), px(1.8), radians(0.0), radians(TAU))
          .move_to(px(cx + (12.0 - cx) * 0.25), px(cy + (12.0 - cy) * 0.25))
          .line_to(px(cx + (12.0 - cx) * 0.65), px(cy + (12.0 - cy) * 0.65)).stroke();
      }
      q
    }
    "lb" => {
      let mut q = p.begin_path().arc(px(5.0), px(12.0), px(2.2), radians(0.0), radians(TAU)).stroke();
      for ny in [6.0f32, 12.0, 18.0] {
        q = q.begin_path().arc(px(19.0), px(ny), px(2.0), radians(0.0), radians(TAU)).move_to(px(7.2), px(12.0)).line_to(px(17.0), px(ny)).stroke();
      }
      q
    }
    "firewall" => p.begin_path().move_to(px(12.0), px(3.0)).line_to(px(19.0), px(6.0)).line_to(px(19.0), px(11.5))
      .bezier_curve_to(point(px(19.0), px(16.0)), point(px(16.0), px(19.5)), point(px(12.0), px(21.0)))
      .bezier_curve_to(point(px(8.0), px(19.5)), point(px(5.0), px(16.0)), point(px(5.0), px(11.5)))
      .line_to(px(5.0), px(6.0)).close_path().stroke(),
    "worker" => p.begin_path().arc(px(12.0), px(12.0), px(3.4), radians(0.0), radians(TAU)).stroke()
      .line_width(px(1.4)).begin_path().arc(px(12.0), px(12.0), px(7.2), radians(0.0), radians(TAU)).stroke(),
    "doc" => p.begin_path().move_to(px(6.0), px(3.0)).line_to(px(14.0), px(3.0)).line_to(px(18.0), px(7.0)).line_to(px(18.0), px(21.0)).line_to(px(6.0), px(21.0)).close_path()
      .move_to(px(9.0), px(11.5)).line_to(px(15.0), px(11.5)).move_to(px(9.0), px(15.5)).line_to(px(15.0), px(15.5)).stroke(),
    _ => p.begin_path().round_rect(rr(4.0, 4.0, 16.0, 16.0), px(3.0)).stroke(),
  };
  p.restore()
}

// ============================================================================
// Dock & Flyout Inline SVG Helpers
// ============================================================================

const ICON_CURSOR: &str = r##"<path d="M5 3.5L16.5 11L11.2 12.2L8.8 17L5 3.5Z"/>"##;
const ICON_HAND: &str = r##"<path d="M7 11V6.5A1 1 0 0 1 9 6.5V10M9 9.5V5A1 1 0 0 1 11 5V10M11 9.5V5.8A1 1 0 0 1 13 5.8V10M13 10V7.5A1 1 0 0 1 15 7.5V12.5A4.5 4.5 0 0 1 10.5 17H9.5A4 4 0 0 1 5.5 13V10.5A1 1 0 0 1 7.5 10.5"/>"##;
const ICON_SHAPES: &str = r##"<rect x="3" y="3" width="6.5" height="6.5" rx="1.2"/><circle cx="14.2" cy="6.2" r="3.2"/><path d="M6.2 11.8L9.6 17.5H2.8Z"/><rect x="11.2" y="11.5" width="6" height="6" rx="1"/>"##;
const ICON_SERVER: &str = r##"<rect x="3" y="3.8" width="14" height="5" rx="1.2"/><rect x="3" y="11.2" width="14" height="5" rx="1.2"/><circle cx="6.2" cy="6.3" r="0.8"/><circle cx="6.2" cy="13.7" r="0.8"/>"##;
const ICON_STICKY: &str = r##"<path d="M4 3.5H16V12.5L12.5 16H4Z"/><path d="M12.5 16V12.5H16"/>"##;
const ICON_TEXT: &str = r##"<path d="M4.5 5.5H15.5M10 5.5V15.5M7.5 15.5H12.5"/>"##;
const ICON_CONNECTOR: &str = r##"<circle cx="4.8" cy="15.2" r="1.8"/><circle cx="15.2" cy="4.8" r="1.8"/><path d="M6.2 13.8L13.8 6.2"/>"##;
const ICON_PEN: &str = r##"<path d="M3.5 14.5C5.5 9.5 7.5 13.5 10 9.5C12.5 5.5 14.5 11.5 16.5 6.5"/>"##;
const ICON_FRAME: &str = r##"<path d="M3.5 6.5V3.5H6.5M13.5 3.5H16.5V6.5M16.5 13.5V16.5H13.5M6.5 16.5H3.5V13.5"/>"##;
const ICON_MINDMAP: &str = r##"<rect x="2.5" y="7.8" width="4.8" height="4.4" rx="1"/><rect x="12.5" y="3.5" width="5" height="3.8" rx="1"/><rect x="12.5" y="12.7" width="5" height="3.8" rx="1"/><path d="M7.3 10H10V5.4H12.5M10 10V14.6H12.5"/>"##;
const ICON_DRAW: &str = r##"<path d="M13.8 4.2L15.8 6.2L7.2 14.8L4.2 15.8L5.2 12.8Z"/>"##;
const ICON_WAND: &str = r##"<path d="M4 16L13 7M12 4V6M16 8H14M14.8 5.2L13.4 6.6M8 4.5V6"/>"##;
const ICON_UPLOAD: &str = r##"<path d="M10 13V4.5M6.5 8L10 4.5L13.5 8M4 15.5H16"/>"##;
const ICON_DOWNLOAD: &str = r##"<path d="M10 4.5V13M6.5 9.5L10 13L13.5 9.5M4 15.5H16"/>"##;

fn dock_svg(size_px: f32, color: &str, inner: &str) -> Svg {
  let markup = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">{inner}</svg>"##
  );
  svg(ImageSrc::svg_markup(&markup)).w(px(size_px)).h(px(size_px))
}

fn mini_shape_svg(kind: &str, fill: &str, stroke: &str) -> Svg {
  let body = match kind {
    "rect" => format!(r#"<rect x="2" y="2" width="30" height="18" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>"#),
    "ellipse" => format!(r#"<ellipse cx="17" cy="11" rx="15" ry="9" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>"#),
    "diamond" => format!(r#"<polygon points="17,1.5 32.5,11 17,20.5 1.5,11" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>"#),
    "pill" => format!(r#"<rect x="2" y="2" width="30" height="18" rx="9" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>"#),
    "triangle" => format!(r#"<polygon points="17,2 32,20 2,20" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>"#),
    "parallelogram" => format!(r#"<polygon points="7,2 32,2 27,20 2,20" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>"#),
    "hexagon" => format!(r#"<polygon points="8,2 26,2 32,11 26,20 8,20 2,11" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>"#),
    "cylinder" => format!(r#"<path d="M3,5 A14,3.5 0 0 1 31,5 L31,17 A14,3.5 0 0 1 3,17 Z" fill="{fill}" stroke="{stroke}" stroke-width="1.4"/><ellipse cx="17" cy="5" rx="14" ry="3.5" fill="{fill}" stroke="{stroke}" stroke-width="1.4"/>"#),
    "document" => format!(r#"<path d="M2,2 H32 V17 Q24.5,14 17,17 Q9.5,20 2,17 Z" fill="{fill}" stroke="{stroke}" stroke-width="1.4"/>"#),
    "cloud" => format!(r#"<path d="M9,18 H26 C31,18 31,11 26,10 C25,5 15,5 13,9 C6,9 5,18 9,18 Z" fill="{fill}" stroke="{stroke}" stroke-width="1.4"/>"#),
    "sticky" => format!(r#"<path d="M3,2 H31 V15 L25,20 H3 Z" fill="{fill}" stroke="{stroke}" stroke-width="1.4"/><path d="M25,20 V15 H31" fill="none" stroke="{stroke}" stroke-width="1.2"/>"#),
    _ => format!(r#"<rect x="2" y="2" width="30" height="18" rx="4" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>"#),
  };
  let markup = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 34 22">{body}</svg>"#);
  svg(ImageSrc::svg_markup(&markup)).w(px(34.0)).h(px(22.0))
}

fn mini_tech_icon_svg(id: &str, stroke: &str) -> Svg {
  let inner = tech_icon_svg_inner(id).unwrap_or(r#"<rect x="4" y="4" width="16" height="16" rx="3"/>"#);
  let markup = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="{stroke}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{inner}</svg>"##
  );
  svg(ImageSrc::svg_markup(&markup)).w(px(24.0)).h(px(24.0))
}

// ============================================================================
// Pure Geometry & Text Wrapping Helpers
// ============================================================================

fn polyline_midpoint(pts: &[[f64; 2]]) -> [f64; 2] {
  let total: f64 = pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
  if total < 1e-3 { return pts[0]; }
  let (half, mut acc) = (total * 0.5, 0.0);
  for w in pts.windows(2) {
    let seg = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
    if acc + seg >= half && seg > 1e-3 {
      let r = (half - acc) / seg;
      return [w[0][0] + (w[1][0] - w[0][0]) * r, w[0][1] + (w[1][1] - w[0][1]) * r];
    }
    acc += seg;
  }
  pts[pts.len() / 2]
}

fn wrap_text_lines(text: &str, max_width: f32, font_size: f32) -> Vec<String> {
  let mut lines = Vec::new();
  for para in text.split('\n') {
    if para.is_empty() { lines.push(String::new()); continue; }
    let (mut cur, mut cur_w) = (String::new(), 0.0f32);
    for ch in para.chars() {
      let cw = if ch.is_ascii() { font_size * 0.56 } else { font_size * 1.02 };
      if cur_w + cw > max_width && !cur.is_empty() { lines.push(std::mem::take(&mut cur)); cur_w = 0.0; }
      cur.push(ch);
      cur_w += cw;
    }
    if !cur.is_empty() { lines.push(cur); }
  }
  if lines.is_empty() { lines.push(String::new()); }
  lines
}

fn parse_hex_rgba(raw: &str) -> Option<Rgba> {
  let hex = raw.trim().strip_prefix('#')?;
  let (r, g, b, a) = match hex.len() {
    3 => (u8::from_str_radix(&hex[0..1], 16).ok()? * 17, u8::from_str_radix(&hex[1..2], 16).ok()? * 17, u8::from_str_radix(&hex[2..3], 16).ok()? * 17, 255),
    6 => (u8::from_str_radix(&hex[0..2], 16).ok()?, u8::from_str_radix(&hex[2..4], 16).ok()?, u8::from_str_radix(&hex[4..6], 16).ok()?, 255),
    8 => (u8::from_str_radix(&hex[0..2], 16).ok()?, u8::from_str_radix(&hex[2..4], 16).ok()?, u8::from_str_radix(&hex[4..6], 16).ok()?, u8::from_str_radix(&hex[6..8], 16).ok()?),
    _ => return None,
  };
  Some(Rgba { r: f32::from(r) / 255.0, g: f32::from(g) / 255.0, b: f32::from(b) / 255.0, a: f32::from(a) / 255.0 })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn merge_session_with_remote_applies_three_way_merge() {
    let base = BoardSnapshot::template_whiteboard();
    let base_json = base.to_compact_json();
    let mut session = BoardSession::new(base.clone());
    let n0_id = base.nodes[0].id.clone();
    session.dispatch(BoardCommand::SetNodeText {
      node_id: n0_id.clone(), text: "本地修改标题".into(), measured_height: None,
    });
    let mut remote = base.clone();
    remote.nodes[0].x = 420.0;
    assert!(merge_session_with_remote(&mut session, &base_json, &remote.to_compact_json()));
    let n0 = session.node_by_id(&n0_id).unwrap();
    assert_eq!(n0.text, "本地修改标题");
    assert!((n0.x - 420.0).abs() < 1e-6);
  }

  #[test]
  fn sanitize_filename_strips_illegal_path_chars() {
    assert_eq!(sanitize_filename("架构/设计:v1?"), "架构_设计_v1_");
    assert_eq!(sanitize_filename("   "), "未命名画板");
  }

  #[test]
  fn board_swatches_and_catalog_match_feishu_parity() {
    assert_eq!(FILL_SWATCHES.len(), 8);
    assert_eq!(STROKE_SWATCHES.len(), 6);
    assert_eq!(BOARD_SHAPES.len(), 12);
    assert_eq!(BOARD_TECH_ICONS.len(), 20);
    for icon in BOARD_TECH_ICONS {
      assert!(tech_icon_svg_inner(icon.id).is_some(), "missing svg for {}", icon.id);
    }
  }

  #[test]
  fn paint_board_renders_all_node_kinds_and_overlays_without_panicking() {
    let mut session = BoardSession::new(BoardSnapshot::template_flowchart());
    session.dispatch(BoardCommand::AddNode {
      world_x: 100.0, world_y: 100.0, kind: "icon_card".into(), icon: Some("database".into()),
      text: Some("Postgres".into()), width: None, height: None,
      fill: Some("#F5F0FF".into()), stroke: Some("#8250DF".into()), start_editing: false,
    });
    session.dispatch(BoardCommand::AddNode {
      world_x: 200.0, world_y: 200.0, kind: "frame".into(), icon: None,
      text: Some("分组".into()), width: None, height: None, fill: None, stroke: None, start_editing: false,
    });
    let mut mind_session = BoardSession::new(BoardSnapshot::template_mindmap());
    let root_id = mind_session.snapshot().nodes[0].id.clone();
    mind_session.dispatch(BoardCommand::SelectNode { id: root_id.clone(), additive: false });
    let _ = paint_board(&session, false, (1280.0, 800.0));
    let _ = paint_board(&session, true, (1280.0, 800.0));
    let _ = paint_board(&mind_session, false, (1280.0, 800.0));
    mind_session.dispatch(BoardCommand::ToggleCollapse { node_id: root_id });
    let _ = paint_board(&mind_session, true, (1280.0, 800.0));
  }

  #[test]
  fn paint_board_uses_batched_dot_grid_and_no_gpu_shadow_blur() {
    use nsh_core::canvas::CanvasOp;
    let mut s = BoardSession::from_json(&mica_board::starter_board_json("whiteboard"));
    s.dispatch(BoardCommand::FitToContent { viewport: Some([1920.0, 1080.0]) });
    let drawing = paint_board(&s, false, (1920.0, 1080.0)).finish();
    assert!(
      drawing.ops.len() < 300,
      "1920x1080 dot grid + starter board must stay compact (got {} ops; old FillRect loop emitted 1600..4800)",
      drawing.ops.len()
    );
    assert!(
      !drawing.ops.iter().any(|op| matches!(op, CanvasOp::ShadowBlur(_))),
      "paint_board must not emit Direct2D GPU ShadowBlur effect ops per frame"
    );
  }

  #[test]
  fn hover_and_drag_emit_zero_inspector_churn_and_recover_stale_gestures() {
    use nsh_core::{
      ApproxTextEngine, Core, EventKind, EventPayload, MountItem, MouseButton, MouseEvent,
      PressEvent, ViewKind,
      element::{NoEvent, Root, Scope},
      geometry::{point, px, size},
      keymap::Modifiers,
    };

    struct BoardHarness;
    thread_local! {
      static TEST_SESS: RefCell<Option<Rc<RefCell<BoardSession>>>> = const { RefCell::new(None) };
    }
    impl nsh_core::element::Component for BoardHarness {
      type Props = ();
      type Event = NoEvent;
      fn view(_: Self::Props, _: Scope<Self::Event>) -> impl IntoElement {
        let json = mica_board::starter_board_json("whiteboard");
        let sess = Rc::new(RefCell::new(BoardSession::from_json(&json)));
        let rev = signal(0u64);
        let dm = signal(false);
        TEST_SESS.with(|s| *s.borrow_mut() = Some(sess.clone()));
        board_view_with_rev_and_close(sess, rev, dm, Rc::new(|_| {}), None)
      }
    }

    let mut core = Core::new(ApproxTextEngine, size(px(1280.0), px(820.0)));
    core.mount_root(&Root::of::<BoardHarness>());
    let batch0 = core.commit().unwrap();
    let canvas_id = batch0
      .items
      .iter()
      .find_map(|item| match item {
        MountItem::Create { id, kind: ViewKind::Canvas, .. } => Some(*id),
        _ => None,
      })
      .expect("board-canvas node id");
    core.component_event(
      canvas_id,
      EventKind::Layout,
      &EventPayload::Layout(Bounds::new(point(px(0.0), px(50.0)), size(px(1280.0), px(770.0)))),
    );
    let _ = core.commit();

    let sess = TEST_SESS.with(|s| s.borrow().clone().unwrap());
    let (n0_screen, n0_bottom_port, n1_screen) = {
      let s = sess.borrow();
      let n0 = &s.snapshot().nodes[0];
      let n1 = &s.snapshot().nodes[1];
      let p0 = s.world_to_screen([n0.x + n0.w * 0.5, n0.y + n0.h * 0.5]);
      let pb = s.world_to_screen([n0.x + n0.w * 0.5, n0.y + n0.h]);
      let p1 = s.world_to_screen([n1.x + n1.w * 0.5, n1.y + n1.h * 0.5]);
      ((p0[0] as f32, p0[1] as f32), (pb[0] as f32, pb[1] as f32), (p1[0] as f32, p1[1] as f32))
    };

    // 1. Click node 0, then drag connector from bottom port to node 1 (selects new edge).
    core.component_event(
      canvas_id,
      EventKind::PressDown,
      &EventPayload::Press(PressEvent {
        position: point(px(n0_screen.0), px(n0_screen.1)),
        page_position: point(px(n0_screen.0), px(n0_screen.1 + 50.0)),
        modifiers: Modifiers::default(),
        clicks: 1,
      }),
    );
    let _ = core.commit();
    core.component_event(
      canvas_id,
      EventKind::MouseUp,
      &EventPayload::Mouse(MouseEvent::released(
        point(px(n0_screen.0), px(n0_screen.1)),
        point(px(n0_screen.0), px(n0_screen.1 + 50.0)),
        Modifiers::default(),
        MouseButton::Left,
      )),
    );
    let _ = core.commit();

    core.component_event(
      canvas_id,
      EventKind::PressDown,
      &EventPayload::Press(PressEvent {
        position: point(px(n0_bottom_port.0), px(n0_bottom_port.1)),
        page_position: point(px(n0_bottom_port.0), px(n0_bottom_port.1 + 50.0)),
        modifiers: Modifiers::default(),
        clicks: 1,
      }),
    );
    let _ = core.commit();
    for step in 1..=8 {
      let t = step as f32 / 8.0;
      let x = n0_bottom_port.0 + (n1_screen.0 - n0_bottom_port.0) * t;
      let y = n0_bottom_port.1 + (n1_screen.1 - n0_bottom_port.1) * t;
      core.component_event(
        canvas_id,
        EventKind::MouseMove,
        &EventPayload::Mouse(MouseEvent::new(
          point(px(x), px(y)),
          point(px(x), px(y + 50.0)),
          Modifiers::default(),
          Some(MouseButton::Left),
        )),
      );
      if let Some(b) = core.commit() {
        let creates = b.items.iter().filter(|i| matches!(i, MountItem::Create { .. })).count();
        assert_eq!(creates, 0, "PointerMove during connector drag must never rebuild UI chrome/inspector");
      }
    }
    core.component_event(
      canvas_id,
      EventKind::MouseUp,
      &EventPayload::Mouse(MouseEvent::released(
        point(px(n1_screen.0), px(n1_screen.1)),
        point(px(n1_screen.0), px(n1_screen.1 + 50.0)),
        Modifiers::default(),
        MouseButton::Left,
      )),
    );
    let _ = core.commit();
    assert!(sess.borrow().selected_edge_id().is_some(), "connector drag must select created edge");

    // 2. Repeatedly hover over and away from node 0 while edge inspector is open:
    //    must emit ZERO UI node creates/deletes (canvas repaint only).
    for round in 0..6 {
      let (hx, hy) = if round % 2 == 0 { n0_screen } else { (5.0, 5.0) };
      core.component_event(
        canvas_id,
        EventKind::MouseMove,
        &EventPayload::Mouse(MouseEvent::new(
          point(px(hx), px(hy)),
          point(px(hx), px(hy + 50.0)),
          Modifiers::default(),
          None,
        )),
      );
      let b = core.commit().expect("hover change repaints canvas");
      let creates = b.items.iter().filter(|i| matches!(i, MountItem::Create { .. })).count();
      let deletes = b.items.iter().filter(|i| matches!(i, MountItem::Delete { .. })).count();
      assert_eq!(creates, 0, "hover round {round} must not recreate inspector/dock views");
      assert_eq!(deletes, 0, "hover round {round} must not delete inspector/dock views");
    }

    // 3. Stale gesture auto-recovery: start drag on node 1 (avoids double-tap on node 0),
    //    miss MouseUp (released outside canvas), next MouseMove with no pressed button
    //    must finish gesture cleanly back to Idle.
    core.component_event(
      canvas_id,
      EventKind::PressDown,
      &EventPayload::Press(PressEvent {
        position: point(px(n1_screen.0), px(n1_screen.1)),
        page_position: point(px(n1_screen.0), px(n1_screen.1 + 50.0)),
        modifiers: Modifiers::default(),
        clicks: 1,
      }),
    );
    let _ = core.commit();
    assert!(matches!(sess.borrow().gesture(), GestureState::Drag(_)));

    core.component_event(
      canvas_id,
      EventKind::MouseMove,
      &EventPayload::Mouse(MouseEvent::new(
        point(px(n1_screen.0 + 10.0), px(n1_screen.1 + 10.0)),
        point(px(n1_screen.0 + 10.0), px(n1_screen.1 + 60.0)),
        Modifiers::default(),
        None,
      )),
    );
    let _ = core.commit();
    assert!(
      matches!(sess.borrow().gesture(), GestureState::Idle),
      "MouseMove with no pressed button must recover stale Drag gesture back to Idle"
    );
  }
}
