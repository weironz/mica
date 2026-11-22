//! Infinite whiteboard & mindmap canvas backed directly by `mica_board::BoardSession`
//! and rendered via `nsh_core::canvas::Painter` with warm-gray dot grid, full pointer/wheel/
//! keyboard gestures, floating warm-gray toolbar, inline label editor, and right properties
//! inspector card.

use std::{cell::RefCell, collections::HashSet, f32::consts::TAU, rc::Rc};

use mica_board::{
  BoardCommand, BoardEdge, BoardNode, BoardSession, BoardSvgTheme, BoardTool, PendingConnector,
  SnapGuide, session::GestureState,
};
use nsh_base::{button, input};
use nsh_core::{
  canvas::{CanvasTextAlign, CanvasTextBaseline, LineCap, LineJoin},
  color::Rgba,
  element::{KeyOptions, on_key_with},
  geometry::{point, px, radians, size},
  keymap::When,
  prelude::*,
};

use crate::{
  mica_icons,
  mica_theme::{MicaPalette, palette},
};

struct FillSwatch {
  test_id: &'static str,
  label: &'static str,
  fill: &'static str,
  stroke: &'static str,
  preview: Rgba,
}

const FILL_SWATCHES: &[FillSwatch] = &[
  FillSwatch { test_id: "board-fill-white", label: "填充色：暖白", fill: "#FFFFFF", stroke: "#8C959F", preview: rgb_const(0xFFFFFF) },
  FillSwatch { test_id: "board-fill-yellow", label: "填充色：便签黄", fill: "#FEF3C7", stroke: "#D97706", preview: rgb_const(0xFEF3C7) },
  FillSwatch { test_id: "board-fill-sage", label: "填充色：鼠尾草绿", fill: "#DCFCE7", stroke: "#16A34A", preview: rgb_const(0xDCFCE7) },
  FillSwatch { test_id: "board-fill-sky", label: "填充色：晴空蓝", fill: "#DBEAFE", stroke: "#2563EB", preview: rgb_const(0xDBEAFE) },
  FillSwatch { test_id: "board-fill-lavender", label: "填充色：薰衣草紫", fill: "#EDE9FE", stroke: "#7C3AED", preview: rgb_const(0xEDE9FE) },
  FillSwatch { test_id: "board-fill-coral", label: "填充色：珊瑚红", fill: "#FEE2E2", stroke: "#DC2626", preview: rgb_const(0xFEE2E2) },
];

struct ShapeChoice {
  kind: &'static str,
  label: &'static str,
  test_id: &'static str,
  a11y: &'static str,
}

const SHAPE_CHOICES: &[ShapeChoice] = &[
  ShapeChoice { kind: "rounded", label: "圆角", test_id: "board-shape-rounded", a11y: "切换节点形状为圆角矩形" },
  ShapeChoice { kind: "rect", label: "矩形", test_id: "board-shape-rect", a11y: "切换节点形状为直角矩形" },
  ShapeChoice { kind: "sticky", label: "便签", test_id: "board-shape-sticky", a11y: "切换节点形状为便签" },
  ShapeChoice { kind: "diamond", label: "菱形", test_id: "board-shape-diamond", a11y: "切换节点形状为菱形" },
  ShapeChoice { kind: "ellipse", label: "椭圆", test_id: "board-shape-ellipse", a11y: "切换节点形状为椭圆" },
];

const fn rgb_const(hex: u32) -> Rgba {
  rgba_const(hex, 1.0)
}

const fn rgba_const(hex: u32, a: f32) -> Rgba {
  Rgba {
    r: ((hex >> 16) & 0xff) as f32 / 255.0,
    g: ((hex >> 8) & 0xff) as f32 / 255.0,
    b: (hex & 0xff) as f32 / 255.0,
    a,
  }
}

#[derive(Clone)]
struct BoardCtx {
  session: Rc<RefCell<BoardSession>>,
  rev: Signal<u64>,
  dark_mode: Signal<bool>,
  editing_node_id: Signal<Option<String>>,
  editing_draft: Signal<String>,
  last_svg_bytes: Signal<usize>,
  viewport_size: Signal<(f32, f32)>,
  space_held: Signal<bool>,
  on_persist: Rc<dyn Fn(String)>,
}

impl BoardCtx {
  fn bump(&self) {
    self.rev.set(self.rev.get_untracked() + 1);
  }

  fn dispatch_persist_if_changed(&self, cmd: BoardCommand) {
    let json = {
      let mut s = self.session.borrow_mut();
      let patch = s.dispatch(cmd);
      patch.snapshot_changed.then(|| s.export_compact_json())
    };
    self.bump();
    if let Some(j) = json {
      (self.on_persist)(j);
    }
  }

  fn dispatch_always_persist(&self, cmd: BoardCommand) {
    let json = {
      let mut s = self.session.borrow_mut();
      s.dispatch(cmd);
      s.export_compact_json()
    };
    self.bump();
    (self.on_persist)(json);
  }

  fn set_tool(&self, tool: BoardTool) {
    self.session.borrow_mut().dispatch(BoardCommand::SetTool { tool });
    self.bump();
  }

  fn add_node_in_view(&self, kind: &str, prefix: &str, fill: Option<&str>, stroke: Option<&str>) {
    let (vw, vh) = self.viewport_size.get_untracked();
    let json = {
      let mut s = self.session.borrow_mut();
      let count = s.snapshot().nodes.len();
      let center = s.screen_to_world([(vw as f64) * 0.45, (vh as f64) * 0.42]);
      let world_x = (center[0] + ((count % 4) as f64 - 1.5) * 185.0).max(-2000.0);
      let world_y = (center[1] + (((count / 4) % 3) as f64 - 1.0) * 115.0).max(-2000.0);
      s.dispatch(BoardCommand::AddNode {
        world_x,
        world_y,
        kind: kind.to_string(),
        icon: None,
        text: Some(format!("{prefix} {}", count + 1)),
        width: None,
        height: None,
        fill: fill.map(str::to_string),
        stroke: stroke.map(str::to_string),
        start_editing: false,
      });
      s.export_compact_json()
    };
    self.bump();
    (self.on_persist)(json);
  }

  fn add_mind_child(&self) {
    let next_json = {
      let mut s = self.session.borrow_mut();
      let parent_id = s
        .selected_node_ids()
        .first()
        .cloned()
        .or_else(|| s.snapshot().nodes.first().map(|n| n.id.clone()));
      parent_id.map(|pid| {
        s.dispatch(BoardCommand::AddMindChild { parent_id: pid, start_editing: false });
        s.export_compact_json()
      })
    };
    if let Some(json) = next_json {
      self.bump();
      (self.on_persist)(json);
    }
  }

  fn add_mind_sibling(&self) {
    let next_json = {
      let mut s = self.session.borrow_mut();
      let node_id = s
        .selected_node_ids()
        .first()
        .cloned()
        .or_else(|| s.snapshot().nodes.first().map(|n| n.id.clone()));
      node_id.map(|nid| {
        s.dispatch(BoardCommand::AddMindSibling { node_id: nid, start_editing: false });
        s.export_compact_json()
      })
    };
    if let Some(json) = next_json {
      self.bump();
      (self.on_persist)(json);
    }
  }

  fn start_editing_selected(&self) {
    let picked = {
      let mut s = self.session.borrow_mut();
      let target = s
        .selected_node_ids()
        .first()
        .cloned()
        .or_else(|| s.snapshot().nodes.first().map(|n| n.id.clone()));
      target.map(|nid| {
        let txt = s.node_by_id(&nid).map(|n| n.text.clone()).unwrap_or_default();
        s.dispatch(BoardCommand::StartEditingNode { node_id: nid.clone() });
        (nid, txt)
      })
    };
    if let Some((nid, txt)) = picked {
      self.editing_draft.set(txt);
      self.editing_node_id.set(Some(nid));
      self.bump();
    }
  }

  fn finish_editing(&self) {
    if self.editing_node_id.get_untracked().is_none() {
      return;
    }
    let json = {
      let mut s = self.session.borrow_mut();
      s.dispatch(BoardCommand::FinishEditing);
      s.export_compact_json()
    };
    self.editing_node_id.set(None);
    self.bump();
    (self.on_persist)(json);
  }

  fn delete_selection(&self) {
    let editing = self.editing_node_id.get_untracked();
    let json = {
      let mut s = self.session.borrow_mut();
      let patch = s.dispatch(BoardCommand::DeleteSelection);
      patch.snapshot_changed.then(|| s.export_compact_json())
    };
    if let Some(eid) = editing
      && self.session.borrow().node_by_id(&eid).is_none()
    {
      self.editing_node_id.set(None);
    }
    self.bump();
    if let Some(j) = json {
      (self.on_persist)(j);
    }
  }

  fn zoom_by(&self, factor: f64) {
    let (vw, vh) = self.viewport_size.get_untracked();
    self.session.borrow_mut().dispatch(BoardCommand::WheelZoom {
      factor,
      focal_screen_x: (vw as f64) * 0.5,
      focal_screen_y: (vh as f64) * 0.5,
    });
    self.bump();
  }

  fn fit_to_content(&self) {
    let (vw, vh) = self.viewport_size.get_untracked();
    self.session.borrow_mut().dispatch(BoardCommand::FitToContent {
      viewport: Some([vw.max(320.0) as f64, vh.max(240.0) as f64]),
    });
    self.bump();
  }

  fn export_svg(&self) {
    let dark = self.dark_mode.get_untracked();
    let svg_theme = if dark {
      BoardSvgTheme {
        dark: true,
        canvas: "#191919".into(),
        surface: "#242424".into(),
        text: "#D4D4D4".into(),
        subtle_text: "#949494".into(),
        border: "#3F3F3F".into(),
        accent: "#60A5FA".into(),
      }
    } else {
      BoardSvgTheme {
        dark: false,
        canvas: "#FAFAF8".into(),
        surface: "#FFFFFF".into(),
        text: "#37352F".into(),
        subtle_text: "#5F5E5B".into(),
        border: "#D3D2CE".into(),
        accent: "#2563EB".into(),
      }
    };
    let svg_len = self.session.borrow().render_svg(Some(&svg_theme)).len();
    self.last_svg_bytes.set(svg_len);
  }
}

/// Interactive infinite whiteboard & mindmap view backed by `mica_board::BoardSession`.
pub fn board_view(
  session: Rc<RefCell<BoardSession>>,
  rev: Signal<u64>,
  dark_mode: Signal<bool>,
  on_persist_board: Rc<dyn Fn(String)>,
) -> impl IntoElement {
  let ctx = BoardCtx {
    session,
    rev,
    dark_mode,
    editing_node_id: signal(None),
    editing_draft: signal(String::new()),
    last_svg_bytes: signal(0),
    viewport_size: signal((1040.0, 720.0)),
    space_held: signal(false),
    on_persist: on_persist_board,
  };

  register_board_shortcuts(&ctx);

  let space_down = ctx.space_held;
  let space_up = ctx.space_held;
  let (c_paint, c_layout, c_pdown) = (ctx.clone(), ctx.clone(), ctx.clone());
  let (c_pmove, c_pup, c_wheel) = (ctx.clone(), ctx.clone(), ctx.clone());
  let (c_inline, c_bar) = (ctx.clone(), ctx.clone());
  let (c_insp_cond, c_insp_body) = (ctx.clone(), ctx.clone());

  view()
    .flex_1()
    .relative()
    .overflow_hidden()
    .bg(move || canvas_bg(dark_mode.get()))
    .on_key_down(move |ev: &KeyEvent| {
      if ev.key == " " || ev.code == "Space" {
        space_down.set(true);
      }
    })
    .on_key_up(move |ev: &KeyEvent| {
      if ev.key == " " || ev.code == "Space" {
        space_up.set(false);
      }
    })
    .child(
      canvas(move || {
        c_paint.rev.get();
        let dark = c_paint.dark_mode.get();
        let vp = c_paint.viewport_size.get();
        let s = c_paint.session.borrow();
        paint_board(&s, dark, vp)
      })
      .test_id("board-canvas")
      .on_layout(move |b: &Bounds<Points>| {
        let w = f32::from(b.size.width).max(320.0);
        let h = f32::from(b.size.height).max(240.0);
        let (pw, ph) = c_layout.viewport_size.get_untracked();
        if (pw - w).abs() > 1.0 || (ph - h).abs() > 1.0 {
          c_layout.viewport_size.set((w, h));
          c_layout.session.borrow_mut().dispatch(BoardCommand::SetViewportSize {
            width: w as f64,
            height: h as f64,
          });
        }
      })
      .on_press_down(move |ev: &PressEvent| {
        if c_pdown.editing_node_id.get_untracked().is_some() {
          c_pdown.finish_editing();
        }
        let dbl = {
          let mut s = c_pdown.session.borrow_mut();
          s.dispatch(BoardCommand::PointerDown {
            screen_x: f32::from(ev.position.x) as f64,
            screen_y: f32::from(ev.position.y) as f64,
            button: 0,
            shift: ev.modifiers.shift,
            alt: ev.modifiers.alt,
            space: c_pdown.space_held.get_untracked(),
            read_only: false,
            timestamp_ms: None,
          });
          if ev.clicks >= 2 {
            s.selected_node_ids().first().cloned().map(|nid| {
              let txt = s.node_by_id(&nid).map(|n| n.text.clone()).unwrap_or_default();
              s.dispatch(BoardCommand::StartEditingNode { node_id: nid.clone() });
              (nid, txt)
            })
          } else {
            None
          }
        };
        if let Some((nid, txt)) = dbl {
          c_pdown.editing_draft.set(txt);
          c_pdown.editing_node_id.set(Some(nid));
        }
        c_pdown.bump();
      })
      .on_mouse_move(move |ev: &MouseEvent| {
        let (sx, sy) = (f32::from(ev.position.x) as f64, f32::from(ev.position.y) as f64);
        if let Some(btn) = ev.pressed_button {
          let mut s = c_pmove.session.borrow_mut();
          if btn == MouseButton::Middle && matches!(s.gesture(), GestureState::Idle) {
            s.dispatch(BoardCommand::PointerDown {
              screen_x: sx,
              screen_y: sy,
              button: 1,
              shift: ev.modifiers.shift,
              alt: ev.modifiers.alt,
              space: false,
              read_only: false,
              timestamp_ms: None,
            });
          } else {
            s.dispatch(BoardCommand::PointerMove {
              screen_x: sx,
              screen_y: sy,
              alt: ev.modifiers.alt,
            });
          }
          drop(s);
          c_pmove.bump();
        } else {
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
      .on_mouse_up(move |_: &MouseEvent| {
        c_pup.dispatch_persist_if_changed(BoardCommand::PointerUp);
      })
      .on_wheel(move |ev: &WheelEvent| {
        let dy = match ev.delta {
          WheelDelta::Points(pt) => f32::from(pt.y),
          WheelDelta::Lines(pt) => pt.y * 24.0,
        };
        c_wheel.session.borrow_mut().dispatch(BoardCommand::WheelZoom {
          factor: if dy < 0.0 { 1.12 } else { 0.89 },
          focal_screen_x: f32::from(ev.position.x) as f64,
          focal_screen_y: f32::from(ev.position.y) as f64,
        });
        c_wheel.bump();
      })
      .flex_1()
      .w_full()
      .h_full(),
    )
    .child(build_floating_toolbar(&ctx))
    .child(when(move || c_inline.editing_node_id.get().is_some(), move || {
      build_inline_editor_bar(&c_bar)
    }))
    .child(when(
      move || {
        c_insp_cond.rev.get();
        !c_insp_cond.session.borrow().selected_node_ids().is_empty()
      },
      move || build_inspector_card(&c_insp_body),
    ))
    .child(build_viewport_dock(&ctx))
}

fn bind_board_key(ctx: &BoardCtx, key: &str, action: impl Fn(&BoardCtx) + 'static) {
  let c = ctx.clone();
  let opts = KeyOptions { when: When::NotEditing, ..KeyOptions::default() };
  on_key_with(key, opts, move |_| {
    if c.editing_node_id.get_untracked().is_none() {
      action(&c);
    }
  });
}

fn register_board_shortcuts(ctx: &BoardCtx) {
  bind_board_key(ctx, "tab", BoardCtx::add_mind_child);
  bind_board_key(ctx, "return", BoardCtx::add_mind_sibling);
  bind_board_key(ctx, "delete", BoardCtx::delete_selection);
  bind_board_key(ctx, "backspace", BoardCtx::delete_selection);
  bind_board_key(ctx, "mod+z", |c| c.dispatch_persist_if_changed(BoardCommand::Undo));
  bind_board_key(ctx, "mod+y", |c| c.dispatch_persist_if_changed(BoardCommand::Redo));
  bind_board_key(ctx, "shift+mod+z", |c| c.dispatch_persist_if_changed(BoardCommand::Redo));

  let c_esc = ctx.clone();
  on_key_with(
    "escape",
    KeyOptions { when: When::Always, pass_through: true, ..KeyOptions::default() },
    move |_| {
      if c_esc.editing_node_id.get_untracked().is_some() {
        c_esc.finish_editing();
      } else {
        c_esc.session.borrow_mut().dispatch(BoardCommand::ClearSelection);
        c_esc.bump();
      }
    },
  );
}

fn floating_card(dark_mode: Signal<bool>, tid: &'static str, radius: f32) -> View {
  view()
    .test_id(tid)
    .rounded(px(radius))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
}

fn build_floating_toolbar(ctx: &BoardCtx) -> impl IntoElement {
  let dm = ctx.dark_mode;
  let (c_sel, c_hand, c_conn) = (ctx.clone(), ctx.clone(), ctx.clone());
  let (c_sticky, c_node, c_diamond) = (ctx.clone(), ctx.clone(), ctx.clone());
  let (c_child, c_sibling, c_layout) = (ctx.clone(), ctx.clone(), ctx.clone());
  let (c_edit, c_delete, c_undo, c_redo) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());

  view()
    .test_id("board-floating-toolbar-wrap")
    .absolute().top(px(12.0)).left(px(12.0)).right(px(12.0)).z_index(20)
    .pointer_events(PointerEvents::BoxNone)
    .flex_row().justify_center()
    .child(
      floating_card(dm, "board-floating-toolbar", 10.0)
        .max_w(relative(1.0)).flex_row().flex_wrap().items_center().justify_center().gap_1().px_2().py_1().shadow_sm()
        .child(mode_tool_btn(ctx, BoardTool::Select, "Select", "board-tool-select", "选择工具", move || c_sel.set_tool(BoardTool::Select)))
        .child(mode_tool_btn(ctx, BoardTool::Hand, "Hand", "board-tool-hand", "抓手平移工具", move || c_hand.set_tool(BoardTool::Hand)))
        .child(mode_tool_btn(ctx, BoardTool::Connector, "Connector", "board-tool-connector", "连线工具", move || c_conn.set_tool(BoardTool::Connector)))
        .child(toolbar_divider(dm))
        .child(action_btn(dm, "+ Sticky", "board-add-sticky", "添加便签节点", move || c_sticky.add_node_in_view("sticky", "Note", Some("#FEF3C7"), Some("#D97706"))))
        .child(action_btn(dm, "+ Node", "board-add-node", "添加圆角节点", move || c_node.add_node_in_view("rounded", "Node", None, None)))
        .child(action_btn(dm, "+ Diamond", "board-add-diamond", "添加菱形判断节点", move || c_diamond.add_node_in_view("diamond", "Decision", Some("#DBEAFE"), Some("#2563EB"))))
        .child(action_btn(dm, "+ Child", "board-add-child", "添加思维导图子节点 (Tab)", move || c_child.add_mind_child()))
        .child(action_btn(dm, "+ Sibling", "board-add-sibling", "添加思维导图同级节点 (Enter)", move || c_sibling.add_mind_sibling()))
        .child(toolbar_divider(dm))
        .child(action_btn(dm, "Auto Layout", "board-layout", "自动整理思维导图布局", move || c_layout.dispatch_always_persist(BoardCommand::LayoutMindmap)))
        .child(action_btn(dm, "Edit Label", "board-edit-label", "编辑选中节点文字", move || c_edit.start_editing_selected()))
        .child(icon_action_btn(dm, "board-delete", "删除选中节点 (Delete)", move || c_delete.delete_selection(), move |p| mica_icons::trash_icon(p.text_muted_hex)))
        .child(toolbar_divider(dm))
        .child(icon_action_btn(dm, "board-undo", "撤销画板操作 (Ctrl+Z)", move || c_undo.dispatch_always_persist(BoardCommand::Undo), move |p| mica_icons::undo(p.text_muted_hex)))
        .child(icon_action_btn(dm, "board-redo", "重做画板操作 (Ctrl+Y)", move || c_redo.dispatch_always_persist(BoardCommand::Redo), move |p| mica_icons::redo(p.text_muted_hex))),
    )
}

fn build_inline_editor_bar(ctx: &BoardCtx) -> impl IntoElement + use<> {
  let (dm, draft) = (ctx.dark_mode, ctx.editing_draft);
  let (c_change, c_submit, c_done) = (ctx.clone(), ctx.clone(), ctx.clone());

  floating_card(dm, "board-inline-edit-bar", 10.0)
    .absolute().top(px(62.0)).left(px(16.0)).w(px(380.0)).z_index(25)
    .flex_row().items_center().gap_2().px_3().py_2()
    .border_color(move || palette(dm.get()).accent)
    .shadow_md()
    .child(text("节点文字").text_size(px(12.0)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_muted))
    .child(
      input()
        .test_id("board-node-text-input")
        .flex_1().px_2().py_1().rounded(px(6.0))
        .bg(move || palette(dm.get()).surface_sunken)
        .text_size(px(13.0))
        .text_color(move || palette(dm.get()).text_primary)
        .placeholder("输入节点文字...")
        .auto_focus(true)
        .value(move || draft.get())
        .on_change(move |v: &str| {
          draft.set(v.to_owned());
          if let Some(nid) = c_change.editing_node_id.get_untracked() {
            c_change.session.borrow_mut().dispatch(BoardCommand::SetNodeText { node_id: nid, text: v.to_owned(), measured_height: None });
            c_change.bump();
          }
        })
        .on_submit(move |_: &str| c_submit.finish_editing()),
    )
    .child(
      button()
        .test_id("board-edit-done")
        .accessibility_label("完成节点文字编辑")
        .px_3().py_1().rounded(px(6.0))
        .bg(move || palette(dm.get()).accent)
        .on_press(move || c_done.finish_editing())
        .child(text("Done").text_size(px(12.0)).font_weight(FontWeight::MEDIUM).text_color(rgb_const(0xFFFFFF))),
    )
}

fn build_inspector_card(ctx: &BoardCtx) -> impl IntoElement + use<> {
  let dm = ctx.dark_mode;
  let c_dyn = ctx.clone();

  floating_card(dm, "board-inspector-card", 12.0)
    .absolute().top(px(64.0)).right(px(14.0)).w(px(218.0)).z_index(20)
    .p_3().gap_3().shadow_md()
    .child(dynamic(move || {
      c_dyn.rev.get();
      let dark = c_dyn.dark_mode.get();
      let p = palette(dark);
      let (sel_count, active_fill, active_kind, coords) = summarize_selection(&c_dyn.session.borrow());

      let mut swatch_row = view().flex_row().flex_wrap().items_center().gap_2();
      for sw in FILL_SWATCHES {
        let c_sw = c_dyn.clone();
        let active = active_fill.eq_ignore_ascii_case(sw.fill);
        let (fill_s, stroke_s) = (sw.fill, sw.stroke);
        swatch_row = swatch_row.child(
          button()
            .test_id(sw.test_id).accessibility_label(sw.label)
            .w(px(24.0)).h(px(24.0)).rounded_full().bg(sw.preview)
            .border_2().border_color(if active { p.accent } else { p.border_strong })
            .on_press(move || {
              c_sw.dispatch_persist_if_changed(BoardCommand::UpdateSelectedNodesStyle {
                fill: Some(fill_s.into()), stroke: Some(stroke_s.into()), dashed: None,
                font_size: None, text_color: Some("#1F2329".into()), align: None, kind: None,
              });
            }),
        );
      }

      let mut shape_row = view().flex_row().flex_wrap().items_center().gap_1();
      for sh in SHAPE_CHOICES {
        let c_sh = c_dyn.clone();
        let active = active_kind == sh.kind
          || (sh.kind == "rounded" && matches!(active_kind.as_str(), "mind_root" | "mind_node"));
        let kind_s = sh.kind;
        shape_row = shape_row.child(
          button()
            .test_id(sh.test_id).accessibility_label(sh.a11y)
            .px_2().py_1().rounded(px(6.0))
            .bg(if active { p.accent_soft } else { p.surface_sunken })
            .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
            .on_press(move || {
              c_sh.dispatch_persist_if_changed(BoardCommand::UpdateSelectedNodesStyle {
                fill: None, stroke: None, dashed: None, font_size: None,
                text_color: None, align: None, kind: Some(kind_s.into()),
              });
            })
            .child(
              text(sh.label).text_size(px(11.5))
                .font_weight(if active { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                .text_color(if active { p.accent } else { p.text_primary }),
            ),
        );
      }

      let (c_edit, c_del) = (c_dyn.clone(), c_dyn.clone());
      view()
        .w_full().gap_3()
        .child(
          view().w_full().flex_row().items_center().justify_between()
            .child(text("节点属性").text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(p.text_primary))
            .child(view().px_2().py_1().rounded(px(6.0)).bg(p.surface_sunken).child(text(format!("已选 {sel_count} 项")).text_size(px(11.0)).text_color(p.text_muted))),
        )
        .child(view().gap_1().child(text("填充颜色").text_size(px(11.0)).text_color(p.text_faint)).child(swatch_row))
        .child(view().gap_1().child(text("节点形状").text_size(px(11.0)).text_color(p.text_faint)).child(shape_row))
        .child(view().w_full().px_2().py_1().rounded(px(6.0)).bg(p.surface_sunken).child(text(coords).text_size(px(11.0)).text_color(p.text_muted)))
        .child(
          view().w_full().flex_row().items_center().gap_2()
            .child(
              button()
                .test_id("board-inspector-edit").accessibility_label("编辑选中节点文字")
                .flex_1().py_1().items_center().justify_center().rounded(px(6.0)).bg(p.surface_sunken)
                .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
                .on_press(move || c_edit.start_editing_selected())
                .child(text("编辑文字").text_size(px(12.0)).text_color(p.text_primary)),
            )
            .child(
              button()
                .test_id("board-inspector-delete").accessibility_label("删除选中的节点")
                .px_2().py_1().flex_row().items_center().gap_1().rounded(px(6.0)).bg(p.surface_sunken)
                .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
                .on_press(move || c_del.delete_selection())
                .child(mica_icons::trash_icon(if dark { "#F87171" } else { "#DC2626" }))
                .child(text("删除").text_size(px(12.0)).text_color(p.danger)),
            ),
        )
    }))
}

fn summarize_selection(s: &BoardSession) -> (usize, String, String, String) {
  let ids = s.selected_node_ids();
  let count = ids.len();
  let first = ids.first().and_then(|id| s.node_by_id(id));
  let fill = first.map(|n| n.style.fill.clone()).unwrap_or_else(|| "#FFFFFF".into());
  let kind = first.map(|n| n.kind.clone()).unwrap_or_else(|| "rounded".into());
  let summary = if count == 1
    && let Some(n) = first
  {
    format!("X {:.0} · Y {:.0}  |  {:.0}×{:.0}", n.x, n.y, n.w, n.h)
  } else if count > 1 {
    let (mut min_x, mut min_y, mut max_x, mut max_y) =
      (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for id in ids {
      if let Some(n) = s.node_by_id(id) {
        min_x = min_x.min(n.x);
        min_y = min_y.min(n.y);
        max_x = max_x.max(n.x + n.w);
        max_y = max_y.max(n.y + n.h);
      }
    }
    if min_x.is_finite() {
      format!("X {:.0} · Y {:.0}  |  {:.0}×{:.0}", min_x, min_y, (max_x - min_x).max(0.0), (max_y - min_y).max(0.0))
    } else {
      "多选节点".into()
    }
  } else {
    String::new()
  };
  (count, fill, kind, summary)
}

fn build_viewport_dock(ctx: &BoardCtx) -> impl IntoElement {
  let (dm, rev, last_svg_bytes) = (ctx.dark_mode, ctx.rev, ctx.last_svg_bytes);
  let (c_zout, c_fit, c_zin, c_svg) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone());
  let (s_zoom, s_status) = (ctx.session.clone(), ctx.session.clone());

  floating_card(dm, "board-viewport-dock", 10.0)
    .absolute().bottom(px(14.0)).left(px(14.0)).z_index(20)
    .flex_row().items_center().gap_1().px_2().py_1().shadow_sm()
    .child(zoom_step_btn(dm, "board-zoom-out", "缩小画板视图", "-", move || c_zout.zoom_by(0.84)))
    .child(
      button()
        .test_id("board-fit").accessibility_label("重置并适配画板全部内容")
        .px_2().h(px(26.0)).items_center().justify_center().rounded(px(6.0))
        .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
        .on_press(move || c_fit.fit_to_content())
        .child(
          text(move || { rev.get(); format!("{:.0}% · Fit", s_zoom.borrow().zoom() * 100.0) })
            .text_size(px(11.5)).font_weight(FontWeight::MEDIUM)
            .text_color(move || palette(dm.get()).text_primary),
        ),
    )
    .child(zoom_step_btn(dm, "board-zoom-in", "放大画板视图", "+", move || c_zin.zoom_by(1.18)))
    .child(toolbar_divider(dm))
    .child(action_btn(dm, "Export SVG", "board-export-svg", "导出画板为矢量 SVG", move || c_svg.export_svg()))
    .child(toolbar_divider(dm))
    .child(
      view().px_1().child(
        text(move || {
          rev.get();
          let svg_tag = if last_svg_bytes.get() > 0 { " · SVG Ready" } else { "" };
          let s = s_status.borrow();
          format!("Nodes: {} · Edges: {}{svg_tag}", s.snapshot().nodes.len(), s.snapshot().edges.len())
        })
        .text_size(px(11.0))
        .text_color(move || palette(dm.get()).text_faint),
      ),
    )
}

fn zoom_step_btn(dm: Signal<bool>, tid: &'static str, a11y: &'static str, glyph: &'static str, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid).accessibility_label(a11y)
    .w(px(26.0)).h(px(26.0)).items_center().justify_center().rounded(px(6.0))
    .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(text(glyph).text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(move || palette(dm.get()).text_primary))
}

fn toolbar_divider(dark_mode: Signal<bool>) -> impl IntoElement {
  view().w(px(1.0)).h(px(18.0)).mx_1().bg(move || palette(dark_mode.get()).border_normal)
}

fn mode_tool_btn(ctx: &BoardCtx, tool: BoardTool, label: &'static str, tid: &'static str, a11y: &'static str, on_click: impl Fn() + 'static) -> impl IntoElement {
  let (dm, rev) = (ctx.dark_mode, ctx.rev);
  let (s_bg, s_fg) = (ctx.session.clone(), ctx.session.clone());
  button()
    .test_id(tid).accessibility_label(a11y)
    .px_2().h(px(26.0)).items_center().justify_center().rounded(px(6.0))
    .bg(move || { rev.get(); let p = palette(dm.get()); if s_bg.borrow().tool() == tool { p.accent_soft } else { p.surface_overlay } })
    .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(
      text(label).text_size(px(12.0)).font_weight(FontWeight::MEDIUM)
        .text_color(move || { rev.get(); let p = palette(dm.get()); if s_fg.borrow().tool() == tool { p.accent } else { p.text_primary } }),
    )
}

fn action_btn(dm: Signal<bool>, label: &'static str, tid: &'static str, a11y: &'static str, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid).accessibility_label(a11y)
    .px_2().h(px(26.0)).items_center().justify_center().rounded(px(6.0))
    .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(text(label).text_size(px(12.0)).text_color(move || palette(dm.get()).text_primary))
}

fn icon_action_btn(dm: Signal<bool>, tid: &'static str, a11y: &'static str, on_click: impl Fn() + 'static, icon_fn: impl Fn(MicaPalette) -> Svg + 'static) -> impl IntoElement {
  button()
    .test_id(tid).accessibility_label(a11y)
    .w(px(26.0)).h(px(26.0)).items_center().justify_center().rounded(px(6.0))
    .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(dynamic(move || icon_fn(palette(dm.get()))))
}

// ============================================================================
// Infinite Canvas 2D Painter (Warm-Gray Dot Grid + Edges + Shapes + Overlays)
// ============================================================================

fn canvas_bg(dark: bool) -> Rgba {
  if dark { rgb_const(0x191919) } else { rgb_const(0xFAFAF8) }
}

fn paint_board(s: &BoardSession, dark: bool, viewport: (f32, f32)) -> Painter {
  let pal = palette(dark);
  let snap = s.snapshot();
  let [pan_x, pan_y] = s.pan();
  let zoom = s.zoom().clamp(0.1, 8.0) as f32;
  let inv_z = 1.0 / zoom;
  let (vw, vh) = (viewport.0.max(320.0), viewport.1.max(240.0));

  let mut p = Painter::new()
    .fill_color(canvas_bg(dark))
    .fill_rect(px(0.0), px(0.0), px(vw), px(vh));
  p = paint_dot_grid(p, pan_x as f32, pan_y as f32, zoom, vw, vh, dark);

  p = p.save().translate(px(pan_x as f32), px(pan_y as f32)).scale(zoom, zoom);

  let hidden = s.collapsed_descendants();
  let mut visible_nodes: Vec<&BoardNode> =
    snap.nodes.iter().filter(|n| !hidden.contains(&n.id)).collect();
  visible_nodes.sort_by_key(|n| (n.kind != "frame", n.z));
  let visible_ids: HashSet<&str> = visible_nodes.iter().map(|n| n.id.as_str()).collect();

  for node in visible_nodes.iter().filter(|n| n.kind == "frame") {
    p = paint_node(p, node, dark, &pal);
  }
  let sel_edge_id = s.selected_edge_id();
  for edge in &snap.edges {
    if !visible_ids.contains(edge.from_id.as_str()) || !visible_ids.contains(edge.to_id.as_str()) {
      continue;
    }
    if let Some(pts) = s.routed_edges().get(&edge.id)
      && pts.len() >= 2
    {
      p = paint_edge(p, edge, pts, sel_edge_id == Some(edge.id.as_str()), &pal);
    }
  }
  for node in visible_nodes.iter().filter(|n| n.kind != "frame") {
    p = paint_node(p, node, dark, &pal);
  }
  p = paint_strokes(p, s);
  p = paint_overlays(p, s, &visible_nodes, inv_z, &pal);
  p.restore()
}

fn paint_dot_grid(
  mut p: Painter,
  pan_x: f32,
  pan_y: f32,
  zoom: f32,
  vw: f32,
  vh: f32,
  dark: bool,
) -> Painter {
  let mut world_step = 24.0f32;
  while world_step * zoom < 14.0 {
    world_step *= 2.0;
  }
  while world_step * zoom > 72.0 && world_step > 6.0 {
    world_step *= 0.5;
  }
  let mut step = world_step * zoom;
  if step < 6.0 {
    return p;
  }
  while (vw / step) * (vh / step) > 4800.0 {
    step *= 2.0;
  }
  let (start_x, start_y) = (pan_x.rem_euclid(step), pan_y.rem_euclid(step));
  let dot_size = if zoom >= 1.4 { 2.2 } else { 1.8 };
  let half = dot_size * 0.5;
  p = p.fill_color(if dark { rgba_const(0x525252, 0.55) } else { rgba_const(0xC8C7C2, 0.85) });

  let mut y = start_y;
  while y <= vh + step {
    let mut x = start_x;
    while x <= vw + step {
      p = p.fill_rect(px(x - half), px(y - half), px(dot_size), px(dot_size));
      x += step;
    }
    y += step;
  }
  p
}

fn paint_edge(
  mut p: Painter,
  edge: &BoardEdge,
  pts: &[[f64; 2]],
  selected: bool,
  pal: &MicaPalette,
) -> Painter {
  let stroke_col = if selected {
    pal.accent
  } else {
    parse_hex_rgba(&edge.style.stroke).unwrap_or(pal.text_faint)
  };
  let sw = (edge.style.stroke_width as f32).max(1.2) + if selected { 1.0 } else { 0.0 };

  p = p
    .save()
    .stroke_color(stroke_col)
    .line_width(px(sw))
    .line_cap(LineCap::Round)
    .line_join(LineJoin::Round);
  if edge.style.dashed {
    p = p.line_dash(&[px(6.0), px(4.0)]);
  }

  let (mid, start_dir, end_dir) = if edge.routing == "curved" && pts.len() == 4 {
    p = p
      .begin_path()
      .move_to(px(pts[0][0] as f32), px(pts[0][1] as f32))
      .bezier_curve_to(
        point(px(pts[1][0] as f32), px(pts[1][1] as f32)),
        point(px(pts[2][0] as f32), px(pts[2][1] as f32)),
        point(px(pts[3][0] as f32), px(pts[3][1] as f32)),
      )
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
      for pt in &pts[1..] {
        q = q.line_to(px(pt[0] as f32), px(pt[1] as f32));
      }
      q.stroke()
    };
    let n = pts.len();
    (polyline_midpoint(pts), [pts[0][0] - pts[1][0], pts[0][1] - pts[1][1]], [pts[n - 1][0] - pts[n - 2][0], pts[n - 1][1] - pts[n - 2][1]])
  };

  p = p.line_dash(&[]);
  if let Some(&tip) = pts.last() {
    p = paint_edge_marker(p, &edge.arrow_end, tip, end_dir, stroke_col);
  }
  p = paint_edge_marker(p, &edge.arrow_start, pts[0], start_dir, stroke_col);
  if !edge.label.trim().is_empty() {
    p = paint_edge_label(p, edge.label.trim(), mid, stroke_col, pal);
  }
  p.restore()
}

fn paint_rounded_orthogonal(mut p: Painter, pts: &[[f64; 2]], radius: f64) -> Painter {
  p = p.begin_path().move_to(px(pts[0][0] as f32), px(pts[0][1] as f32));
  for i in 1..pts.len() - 1 {
    let (prev, curr, next) = (pts[i - 1], pts[i], pts[i + 1]);
    let (v1, v2) = ([prev[0] - curr[0], prev[1] - curr[1]], [next[0] - curr[0], next[1] - curr[1]]);
    let (d1, d2) = (v1[0].hypot(v1[1]), v2[0].hypot(v2[1]));
    if d1 < 1e-3 || d2 < 1e-3 {
      continue;
    }
    let r = radius.min(d1 * 0.5).min(d2 * 0.5);
    let before = [curr[0] + (v1[0] / d1) * r, curr[1] + (v1[1] / d1) * r];
    let after = [curr[0] + (v2[0] / d2) * r, curr[1] + (v2[1] / d2) * r];
    p = p
      .line_to(px(before[0] as f32), px(before[1] as f32))
      .quadratic_curve_to(point(px(curr[0] as f32), px(curr[1] as f32)), point(px(after[0] as f32), px(after[1] as f32)));
  }
  let last = pts[pts.len() - 1];
  p.line_to(px(last[0] as f32), px(last[1] as f32)).stroke()
}

fn paint_edge_marker(p: Painter, marker: &str, tip: [f64; 2], dir: [f64; 2], color: Rgba) -> Painter {
  match marker {
    "arrow" => paint_arrow_head(p, tip, dir, color),
    "dot" => p
      .fill_color(color)
      .begin_path()
      .arc(px(tip[0] as f32), px(tip[1] as f32), px(3.5), radians(0.0), radians(TAU))
      .fill(),
    "diamond" => {
      let len = dir[0].hypot(dir[1]);
      if len < 1e-3 {
        return p;
      }
      let (ux, uy) = (dir[0] / len, dir[1] / len);
      let (mx, my) = (tip[0] - ux * 5.0, tip[1] - uy * 5.0);
      p.fill_color(color)
        .begin_path()
        .move_to(px(tip[0] as f32), px(tip[1] as f32))
        .line_to(px((mx - uy * 4.0) as f32), px((my + ux * 4.0) as f32))
        .line_to(px((tip[0] - ux * 10.0) as f32), px((tip[1] - uy * 10.0) as f32))
        .line_to(px((mx + uy * 4.0) as f32), px((my - ux * 4.0) as f32))
        .close_path()
        .fill()
    }
    _ => p,
  }
}

fn paint_arrow_head(p: Painter, tip: [f64; 2], dir: [f64; 2], color: Rgba) -> Painter {
  let len = dir[0].hypot(dir[1]);
  if len < 1e-3 {
    return p;
  }
  let (ux, uy) = (dir[0] / len, dir[1] / len);
  let (bx, by) = (tip[0] - ux * 9.5, tip[1] - uy * 9.5);
  p.fill_color(color)
    .begin_path()
    .move_to(px(tip[0] as f32), px(tip[1] as f32))
    .line_to(px((bx - uy * 4.6) as f32), px((by + ux * 4.6) as f32))
    .line_to(px((bx + uy * 4.6) as f32), px((by - ux * 4.6) as f32))
    .close_path()
    .fill()
}

fn paint_edge_label(p: Painter, label: &str, mid: [f64; 2], border: Rgba, pal: &MicaPalette) -> Painter {
  let est_w = (label.chars().map(|c| if c.is_ascii() { 7.2 } else { 13.0 }).sum::<f32>() + 14.0).max(28.0);
  let rect = Bounds::new(point(px(mid[0] as f32 - est_w * 0.5), px(mid[1] as f32 - 10.0)), size(px(est_w), px(20.0)));
  p.fill_color(pal.surface_overlay)
    .stroke_color(border)
    .line_width(px(0.9))
    .begin_path()
    .round_rect(rect, px(4.0))
    .fill()
    .stroke()
    .fill_color(pal.text_primary)
    .font("11.5px system-ui")
    .text_align(CanvasTextAlign::Center)
    .text_baseline(CanvasTextBaseline::Middle)
    .fill_text(label, px(mid[0] as f32), px(mid[1] as f32))
}

fn paint_node(mut p: Painter, node: &BoardNode, dark: bool, pal: &MicaPalette) -> Painter {
  let (x, y, w, h) = (node.x as f32, node.y as f32, node.w.max(8.0) as f32, node.h.max(8.0) as f32);
  let (cx, cy) = (x + w * 0.5, y + h * 0.5);

  let adapted_white = dark && node.style.fill.eq_ignore_ascii_case("#FFFFFF");
  let fill_opt = if node.style.fill == "transparent" || node.style.fill == "none" {
    None
  } else if adapted_white {
    Some(pal.surface_overlay)
  } else {
    parse_hex_rgba(&node.style.fill).or(Some(pal.surface_overlay))
  };
  let stroke_opt = if node.style.stroke == "transparent" || node.style.stroke == "none" {
    None
  } else {
    parse_hex_rgba(&node.style.stroke).or(Some(pal.border_strong))
  };
  let sw = (node.style.stroke_width as f32).max(0.0);

  p = p.save();
  if node.rot.abs() > 0.1 {
    p = p.translate(px(cx), px(cy)).rotate(radians((node.rot as f32).to_radians())).translate(px(-cx), px(-cy));
  }

  let elevated = matches!(node.kind.as_str(), "sticky" | "rounded" | "mind_root" | "mind_node");
  if let Some(fill) = fill_opt {
    if elevated {
      let is_sticky = node.kind == "sticky";
      let shadow_col = rgba_const(0x000000, match (is_sticky, dark) {
        (true, true) => 0.32,
        (true, false) => 0.11,
        (false, true) => 0.22,
        (false, false) => 0.06,
      });
      p = p
        .save()
        .shadow(shadow_col, px(if is_sticky { 10.0 } else { 5.0 }), point(px(0.0), px(if is_sticky { 4.0 } else { 2.0 })))
        .fill_color(fill);
      p = build_node_shape_path(p, node, x, y, w, h).fill().restore();
    } else {
      p = build_node_shape_path(p.fill_color(fill), node, x, y, w, h).fill();
    }
  }

  if let Some(stroke) = stroke_opt
    && sw > 0.0
  {
    p = p.stroke_color(stroke).line_width(px(sw)).line_join(LineJoin::Round);
    if node.style.dashed {
      p = p.line_dash(&[px(6.0), px(4.0)]);
    }
    p = build_node_shape_path(p, node, x, y, w, h).stroke().line_dash(&[]);

    if node.kind == "sticky" {
      let fold = 14.0f32.min(w * 0.25).min(h * 0.35);
      p = p
        .fill_color(rgba_const(0x000000, 0.06))
        .begin_path()
        .move_to(px(x + w - fold), px(y + h))
        .line_to(px(x + w - fold), px(y + h - fold))
        .line_to(px(x + w), px(y + h - fold))
        .close_path()
        .fill()
        .stroke_color(stroke)
        .line_width(px(1.0))
        .begin_path()
        .move_to(px(x + w - fold), px(y + h))
        .line_to(px(x + w - fold), px(y + h - fold))
        .line_to(px(x + w), px(y + h - fold))
        .stroke();
    } else if node.kind == "cylinder" {
      let ry = (h * 0.16).clamp(6.0, 14.0);
      p = p
        .begin_path()
        .ellipse(point(px(cx), px(y + ry)), size(px(w * 0.5), px(ry)), radians(0.0), radians(0.0), radians(TAU), false)
        .stroke();
    }
  }

  if !node.text.is_empty() {
    let text_col = if dark && node.style.text_color.eq_ignore_ascii_case("#1F2329") && (adapted_white || fill_opt.is_none()) {
      pal.text_primary
    } else {
      parse_hex_rgba(&node.style.text_color).unwrap_or(pal.text_primary)
    };
    p = paint_node_text(p, node, x, y, w, h, text_col);
  }
  p.restore()
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
      p.begin_path()
        .move_to(px(x + sk), px(y))
        .line_to(px(x + w - sk), px(y))
        .line_to(px(x + w), px(cy))
        .line_to(px(x + w - sk), px(y + h))
        .line_to(px(x + sk), px(y + h))
        .line_to(px(x), px(cy))
        .close_path()
    }
    "triangle" => p.begin_path().move_to(px(cx), px(y)).line_to(px(x + w), px(y + h)).line_to(px(x), px(y + h)).close_path(),
    "sticky" => {
      let fold = 14.0f32.min(w * 0.25).min(h * 0.35);
      p.begin_path()
        .move_to(px(x), px(y))
        .line_to(px(x + w), px(y))
        .line_to(px(x + w), px(y + h - fold))
        .line_to(px(x + w - fold), px(y + h))
        .line_to(px(x), px(y + h))
        .close_path()
    }
    "document" => {
      let wave = (h * 0.14).clamp(6.0, 14.0);
      let by = y + h - wave;
      p.begin_path()
        .move_to(px(x), px(y))
        .line_to(px(x + w), px(y))
        .line_to(px(x + w), px(by))
        .quadratic_curve_to(point(px(x + w * 0.75), px(by - wave)), point(px(cx), px(by)))
        .quadratic_curve_to(point(px(x + w * 0.25), px(by + wave)), point(px(x), px(by)))
        .close_path()
    }
    "cloud" => p
      .begin_path()
      .move_to(px(x + w * 0.25), px(y + h * 0.82))
      .bezier_curve_to(point(px(x + w * 0.04), px(y + h * 0.82)), point(px(x + w * 0.04), px(y + h * 0.45)), point(px(x + w * 0.22), px(y + h * 0.42)))
      .bezier_curve_to(point(px(x + w * 0.24), px(y + h * 0.12)), point(px(x + w * 0.58), px(y + h * 0.10)), point(px(x + w * 0.68), px(y + h * 0.30)))
      .bezier_curve_to(point(px(x + w * 0.96), px(y + h * 0.30)), point(px(x + w * 0.98), px(y + h * 0.82)), point(px(x + w * 0.78), px(y + h * 0.82)))
      .close_path(),
    "text" => p.begin_path().round_rect(rect, px(4.0)),
    "frame" => p.begin_path().round_rect(rect, px(10.0)),
    _ => p.begin_path().round_rect(rect, px(if node.kind == "mind_root" { 10.0 } else { 8.0 })),
  }
}

fn paint_node_text(mut p: Painter, node: &BoardNode, x: f32, y: f32, w: f32, h: f32, text_col: Rgba) -> Painter {
  if node.kind == "frame" {
    let title = node.text.lines().next().unwrap_or("");
    return p
      .fill_color(text_col)
      .font("600 12.5px system-ui")
      .text_align(CanvasTextAlign::Left)
      .text_baseline(CanvasTextBaseline::Middle)
      .fill_text_max_width(title, px(x + 12.0), px(y + 16.0), px((w - 24.0).max(24.0)));
  }
  let fs = (node.style.font_size as f32).clamp(9.0, 32.0);
  let font_spec = if node.kind == "mind_root" { format!("600 {fs:.1}px system-ui") } else { format!("{fs:.1}px system-ui") };
  let pad_x = if node.kind == "diamond" { 22.0 } else { 12.0 };
  let max_w = (w - pad_x * 2.0).max(24.0);
  let lines = wrap_text_lines(&node.text, max_w, fs);
  let line_h = fs * 1.36;
  let start_y = y + h * 0.5 - (lines.len() as f32) * line_h * 0.5 + line_h * 0.5;
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

fn paint_strokes(mut p: Painter, s: &BoardSession) -> Painter {
  p = p.save().line_cap(LineCap::Round).line_join(LineJoin::Round);
  let draw_one = |pts: &[[f64; 2]], color: &str, width: f64, mut q: Painter| -> Painter {
    if pts.len() < 2 {
      return q;
    }
    q = q
      .stroke_color(parse_hex_rgba(color).unwrap_or(rgb_const(0x2563EB)))
      .line_width(px((width as f32).max(1.2)))
      .begin_path()
      .move_to(px(pts[0][0] as f32), px(pts[0][1] as f32));
    for pt in &pts[1..] {
      q = q.line_to(px(pt[0] as f32), px(pt[1] as f32));
    }
    q.stroke()
  };
  for stroke in &s.snapshot().strokes {
    p = draw_one(&stroke.points, &stroke.color, stroke.width, p);
  }
  if let GestureState::Stroke(active) = s.gesture() {
    p = draw_one(&active.stroke.points, &active.stroke.color, active.stroke.width, p);
  }
  p.restore()
}

fn paint_overlays(
  mut p: Painter,
  s: &BoardSession,
  visible_nodes: &[&BoardNode],
  inv_z: f32,
  pal: &MicaPalette,
) -> Painter {
  let accent = pal.accent;
  let selected_ids = s.selected_node_ids();
  let is_connector_tool = s.tool() == BoardTool::Connector;

  if let Some(hid) = s.hovered_node_id()
    && !selected_ids.iter().any(|id| id == hid)
    && let Some(node) = visible_nodes.iter().find(|n| n.id == hid)
  {
    let pad = 2.5 * inv_z;
    let rect = Bounds::new(point(px(node.x as f32 - pad), px(node.y as f32 - pad)), size(px(node.w as f32 + pad * 2.0), px(node.h as f32 + pad * 2.0)));
    p = p
      .stroke_color(Rgba { a: 0.45, ..accent })
      .line_width(px(1.5 * inv_z))
      .begin_path()
      .round_rect(rect, px(9.0 * inv_z.min(1.2)))
      .stroke();
    if is_connector_tool || node.kind != "frame" {
      p = paint_anchor_ports(p, node, inv_z, accent);
    }
  }

  for node in visible_nodes {
    if !selected_ids.contains(&node.id) {
      continue;
    }
    let pad = 3.0 * inv_z;
    let halo = Bounds::new(point(px(node.x as f32 - pad), px(node.y as f32 - pad)), size(px(node.w as f32 + pad * 2.0), px(node.h as f32 + pad * 2.0)));
    p = p.stroke_color(accent).line_width(px(2.0 * inv_z)).begin_path().round_rect(halo, px(8.0)).stroke();

    let hs = 8.0 * inv_z;
    let half = hs * 0.5;
    for (cx, cy) in [
      (node.x as f32, node.y as f32),
      ((node.x + node.w) as f32, node.y as f32),
      ((node.x + node.w) as f32, (node.y + node.h) as f32),
      (node.x as f32, (node.y + node.h) as f32),
    ] {
      let hr = Bounds::new(point(px(cx - half), px(cy - half)), size(px(hs), px(hs)));
      p = p
        .fill_color(rgb_const(0xFFFFFF))
        .stroke_color(accent)
        .line_width(px(1.6 * inv_z))
        .begin_path()
        .round_rect(hr, px(2.0 * inv_z))
        .fill()
        .stroke();
    }
    if is_connector_tool {
      p = paint_anchor_ports(p, node, inv_z, accent);
    }
  }

  for guide in s.snap_guides() {
    p = paint_snap_guide(p, guide, inv_z, accent);
  }
  if let GestureState::Connector(pc) = s.gesture() {
    p = paint_pending_connector(p, pc, inv_z, accent);
  }
  if let GestureState::Marquee(m) = s.gesture() {
    let rect = Bounds::new(point(px(m.rect.x as f32), px(m.rect.y as f32)), size(px(m.rect.w as f32), px(m.rect.h as f32)));
    p = p
      .save()
      .fill_color(Rgba { a: 0.12, ..accent })
      .stroke_color(accent)
      .line_width(px(1.4 * inv_z))
      .line_dash(&[px(4.0 * inv_z), px(3.0 * inv_z)])
      .begin_path()
      .rect(rect)
      .fill()
      .stroke()
      .restore();
  }
  p
}

fn paint_anchor_ports(mut p: Painter, node: &BoardNode, inv_z: f32, accent: Rgba) -> Painter {
  let r = 4.0 * inv_z;
  for (ax, ay) in [
    ((node.x + node.w * 0.5) as f32, node.y as f32),
    ((node.x + node.w) as f32, (node.y + node.h * 0.5) as f32),
    ((node.x + node.w * 0.5) as f32, (node.y + node.h) as f32),
    (node.x as f32, (node.y + node.h * 0.5) as f32),
  ] {
    p = p
      .fill_color(rgb_const(0xFFFFFF))
      .stroke_color(accent)
      .line_width(px(1.4 * inv_z))
      .begin_path()
      .arc(px(ax), px(ay), px(r), radians(0.0), radians(TAU))
      .fill()
      .stroke();
  }
  p
}

fn paint_snap_guide(p: Painter, g: &SnapGuide, inv_z: f32, accent: Rgba) -> Painter {
  let (x1, y1, x2, y2) = if g.vertical {
    (g.coord as f32, g.start as f32, g.coord as f32, g.end as f32)
  } else {
    (g.start as f32, g.coord as f32, g.end as f32, g.coord as f32)
  };
  p.save()
    .stroke_color(accent)
    .line_width(px(1.0 * inv_z))
    .line_dash(&[px(4.0 * inv_z), px(4.0 * inv_z)])
    .begin_path()
    .move_to(px(x1), px(y1))
    .line_to(px(x2), px(y2))
    .stroke()
    .restore()
}

fn paint_pending_connector(p: Painter, pc: &PendingConnector, inv_z: f32, accent: Rgba) -> Painter {
  let (from, to) = (pc.from_world, pc.current_world);
  let p = p
    .save()
    .stroke_color(accent)
    .line_width(px(1.8 * inv_z))
    .line_dash(&[px(5.0 * inv_z), px(4.0 * inv_z)])
    .begin_path()
    .move_to(px(from[0] as f32), px(from[1] as f32))
    .line_to(px(to[0] as f32), px(to[1] as f32))
    .stroke()
    .line_dash(&[]);
  paint_arrow_head(p, to, [to[0] - from[0], to[1] - from[1]], accent).restore()
}

// ============================================================================
// Pure Geometry & Text Wrapping Helpers
// ============================================================================

fn polyline_midpoint(pts: &[[f64; 2]]) -> [f64; 2] {
  let total: f64 = pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
  if total < 1e-3 {
    return pts[0];
  }
  let half = total * 0.5;
  let mut acc = 0.0;
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
    if para.is_empty() {
      lines.push(String::new());
      continue;
    }
    let (mut cur, mut cur_w) = (String::new(), 0.0f32);
    for ch in para.chars() {
      let cw = if ch.is_ascii() { font_size * 0.56 } else { font_size * 1.02 };
      if cur_w + cw > max_width && !cur.is_empty() {
        lines.push(std::mem::take(&mut cur));
        cur_w = 0.0;
      }
      cur.push(ch);
      cur_w += cw;
    }
    if !cur.is_empty() {
      lines.push(cur);
    }
  }
  if lines.is_empty() {
    lines.push(String::new());
  }
  lines
}

fn parse_hex_rgba(raw: &str) -> Option<Rgba> {
  let hex = raw.trim().strip_prefix('#')?;
  let (r, g, b, a) = match hex.len() {
    3 => (
      u8::from_str_radix(&hex[0..1], 16).ok()? * 17,
      u8::from_str_radix(&hex[1..2], 16).ok()? * 17,
      u8::from_str_radix(&hex[2..3], 16).ok()? * 17,
      255,
    ),
    6 => (
      u8::from_str_radix(&hex[0..2], 16).ok()?,
      u8::from_str_radix(&hex[2..4], 16).ok()?,
      u8::from_str_radix(&hex[4..6], 16).ok()?,
      255,
    ),
    8 => (
      u8::from_str_radix(&hex[0..2], 16).ok()?,
      u8::from_str_radix(&hex[2..4], 16).ok()?,
      u8::from_str_radix(&hex[4..6], 16).ok()?,
      u8::from_str_radix(&hex[6..8], 16).ok()?,
    ),
    _ => return None,
  };
  Some(Rgba {
    r: f32::from(r) / 255.0,
    g: f32::from(g) / 255.0,
    b: f32::from(b) / 255.0,
    a: f32::from(a) / 255.0,
  })
}
