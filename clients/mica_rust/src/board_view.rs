//! Infinite whiteboard & mindmap canvas backed directly by `mica_board::BoardSession`
//! and rendered via `nsh_core::canvas::Painter` with full pointer/wheel gestures
//! and inline text editing overlay.

use std::{cell::RefCell, rc::Rc};

use mica_board::{BoardCommand, BoardSession, BoardTool};
use nsh_base::{button, input, theme};
use nsh_core::{
  color::hsla,
  geometry::{point, px, size},
  prelude::*,
};

pub fn board_view(
  session: Rc<RefCell<BoardSession>>,
  rev: Signal<u64>,
  on_persist_board: Rc<dyn Fn(String)>,
) -> impl IntoElement {
  let editing_node_id: Signal<Option<String>> = signal(None);
  let editing_draft: Signal<String> = signal(String::new());
  let last_svg_bytes: Signal<usize> = signal(0);

  let s_sel = session.clone();
  let s_hand = session.clone();
  let s_conn = session.clone();
  let s_sticky = session.clone();
  let s_add = session.clone();
  let s_child = session.clone();
  let s_sibling = session.clone();
  let s_layout = session.clone();
  let s_edit_btn = session.clone();
  let s_undo = session.clone();
  let s_redo = session.clone();
  let s_zoom_in = session.clone();
  let s_zoom_out = session.clone();
  let s_fit = session.clone();
  let s_svg = session.clone();
  let s_status = session.clone();
  let s_paint = session.clone();
  let s_pdown = session.clone();
  let s_pmove = session.clone();
  let s_pup = session.clone();
  let s_wheel = session.clone();
  let s_edit_apply = session.clone();
  let s_edit_finish = session.clone();

  let p_add = on_persist_board.clone();
  let p_child = on_persist_board.clone();
  let p_sib = on_persist_board.clone();
  let p_layout = on_persist_board.clone();
  let p_undo = on_persist_board.clone();
  let p_redo = on_persist_board.clone();
  let p_pup = on_persist_board.clone();
  let p_edit = on_persist_board.clone();

  view()
    .flex_1()
    .bg(move || theme().background)
    .child(
      view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(tool_btn("Select", "board-tool-select", move || {
          s_sel
            .borrow_mut()
            .dispatch(BoardCommand::SetTool { tool: BoardTool::Select });
          rev.set(rev.get_untracked() + 1);
        }))
        .child(tool_btn("Hand", "board-tool-hand", move || {
          s_hand
            .borrow_mut()
            .dispatch(BoardCommand::SetTool { tool: BoardTool::Hand });
          rev.set(rev.get_untracked() + 1);
        }))
        .child(tool_btn("Connector", "board-tool-connector", move || {
          s_conn
            .borrow_mut()
            .dispatch(BoardCommand::SetTool { tool: BoardTool::Connector });
          rev.set(rev.get_untracked() + 1);
        }))
        .child(tool_btn("+ Sticky", "board-add-sticky", move || {
          let json = {
            let mut s = s_sticky.borrow_mut();
            let count = s.snapshot().nodes.len() as f64;
            s.dispatch(BoardCommand::AddNode {
              world_x: 120.0 + (count % 4.0) * 190.0,
              world_y: 120.0 + (count / 4.0).floor() * 110.0,
              kind: "sticky".into(),
              icon: None,
              text: Some(format!("Note {}", count as usize + 1)),
              width: None,
              height: None,
              fill: None,
              stroke: None,
              start_editing: false,
            });
            s.export_compact_json()
          };
          rev.set(rev.get_untracked() + 1);
          p_add(json);
        }))
        .child(tool_btn("+ Node", "board-add-node", move || {
          let json = {
            let mut s = s_add.borrow_mut();
            let count = s.snapshot().nodes.len() as f64;
            s.dispatch(BoardCommand::AddNode {
              world_x: 80.0 + (count % 5.0) * 180.0,
              world_y: 80.0 + (count / 5.0).floor() * 100.0,
              kind: "rounded".into(),
              icon: None,
              text: Some(format!("Node {}", count as usize + 1)),
              width: None,
              height: None,
              fill: None,
              stroke: None,
              start_editing: false,
            });
            s.export_compact_json()
          };
          rev.set(rev.get_untracked() + 1);
          on_persist_board(json);
        }))
        .child(tool_btn("Mind Child", "board-add-child", move || {
          let next_json = {
            let mut s = s_child.borrow_mut();
            let parent_id = s
              .selected_node_ids()
              .first()
              .cloned()
              .or_else(|| s.snapshot().nodes.first().map(|n| n.id.clone()));
            if let Some(parent_id) = parent_id {
              s.dispatch(BoardCommand::AddMindChild {
                parent_id,
                start_editing: false,
              });
              Some(s.export_compact_json())
            } else {
              None
            }
          };
          if let Some(json) = next_json {
            rev.set(rev.get_untracked() + 1);
            p_child(json);
          }
        }))
        .child(tool_btn("Mind Sibling", "board-add-sibling", move || {
          let next_json = {
            let mut s = s_sibling.borrow_mut();
            let node_id = s
              .selected_node_ids()
              .first()
              .cloned()
              .or_else(|| s.snapshot().nodes.first().map(|n| n.id.clone()));
            if let Some(node_id) = node_id {
              s.dispatch(BoardCommand::AddMindSibling {
                node_id,
                start_editing: false,
              });
              Some(s.export_compact_json())
            } else {
              None
            }
          };
          if let Some(json) = next_json {
            rev.set(rev.get_untracked() + 1);
            p_sib(json);
          }
        }))
        .child(tool_btn("Edit Label", "board-edit-label", move || {
          let picked = {
            let mut s = s_edit_btn.borrow_mut();
            let target = s
              .selected_node_ids()
              .first()
              .cloned()
              .or_else(|| s.snapshot().nodes.first().map(|n| n.id.clone()));
            target.map(|nid| {
              let txt = s
                .snapshot()
                .nodes
                .iter()
                .find(|n| n.id == nid)
                .map(|n| n.text.clone())
                .unwrap_or_default();
              s.dispatch(BoardCommand::StartEditingNode {
                node_id: nid.clone(),
              });
              (nid, txt)
            })
          };
          if let Some((nid, txt)) = picked {
            editing_draft.set(txt);
            editing_node_id.set(Some(nid));
            rev.set(rev.get_untracked() + 1);
          }
        }))
        .child(tool_btn("Auto Layout", "board-layout", move || {
          let json = {
            let mut s = s_layout.borrow_mut();
            s.dispatch(BoardCommand::LayoutMindmap);
            s.export_compact_json()
          };
          rev.set(rev.get_untracked() + 1);
          p_layout(json);
        }))
        .child(tool_btn("Undo", "board-undo", move || {
          let json = {
            let mut s = s_undo.borrow_mut();
            s.dispatch(BoardCommand::Undo);
            s.export_compact_json()
          };
          rev.set(rev.get_untracked() + 1);
          p_undo(json);
        }))
        .child(tool_btn("Redo", "board-redo", move || {
          let json = {
            let mut s = s_redo.borrow_mut();
            s.dispatch(BoardCommand::Redo);
            s.export_compact_json()
          };
          rev.set(rev.get_untracked() + 1);
          p_redo(json);
        }))
        .child(tool_btn("Zoom +", "board-zoom-in", move || {
          s_zoom_in.borrow_mut().dispatch(BoardCommand::WheelZoom {
            factor: 1.2,
            focal_screen_x: 400.0,
            focal_screen_y: 300.0,
          });
          rev.set(rev.get_untracked() + 1);
        }))
        .child(tool_btn("Zoom -", "board-zoom-out", move || {
          s_zoom_out.borrow_mut().dispatch(BoardCommand::WheelZoom {
            factor: 0.8,
            focal_screen_x: 400.0,
            focal_screen_y: 300.0,
          });
          rev.set(rev.get_untracked() + 1);
        }))
        .child(tool_btn("Fit", "board-fit", move || {
          s_fit.borrow_mut().dispatch(BoardCommand::FitToContent {
            viewport: Some([800.0, 560.0]),
          });
          rev.set(rev.get_untracked() + 1);
        }))
        .child(tool_btn("Export SVG", "board-export-svg", move || {
          let svg_len = s_svg.borrow().render_svg(None).len();
          last_svg_bytes.set(svg_len);
        }))
        .child(
          text(move || {
            rev.get();
            let svg_b = last_svg_bytes.get();
            let s = s_status.borrow();
            let svg_tag = if svg_b > 0 {
              " | SVG Ready"
            } else {
              ""
            };
            format!(
              "Nodes: {} | Edges: {} | Zoom: {:.0}%{svg_tag}",
              s.snapshot().nodes.len(),
              s.snapshot().edges.len(),
              s.zoom() * 100.0
            )
          })
          .text_xs()
          .text_color(move || theme().muted_foreground),
        ),
    )
    .child(when(move || editing_node_id.get().is_some(), move || {
      let s_a = s_edit_apply.clone();
      let s_f = s_edit_finish.clone();
      let p_e = p_edit.clone();
      view()
        .test_id("board-inline-edit-bar")
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(move || theme().primary)
        .bg(move || theme().card)
        .child(
          text("Node Text:")
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
        .child(
          input()
            .test_id("board-node-text-input")
            .flex_1()
            .px_2()
            .py_1()
            .border_1()
            .border_color(move || theme().primary)
            .rounded(move || theme().radius)
            .text_color(move || theme().foreground)
            .value(move || editing_draft.get())
            .on_change(move |v: &str| {
              editing_draft.set(v.to_owned());
              if let Some(nid) = editing_node_id.get_untracked() {
                s_a.borrow_mut().dispatch(BoardCommand::SetNodeText {
                  node_id: nid,
                  text: v.to_owned(),
                  measured_height: None,
                });
                rev.set(rev.get_untracked() + 1);
              }
            }),
        )
        .child(tool_btn("Done", "board-edit-done", move || {
          let json = {
            let mut s = s_f.borrow_mut();
            s.dispatch(BoardCommand::FinishEditing);
            s.export_compact_json()
          };
          editing_node_id.set(None);
          rev.set(rev.get_untracked() + 1);
          p_e(json);
        }))
    }))
    .child(
      canvas(move || {
        rev.get();
        let s = s_paint.borrow();
        let snap = s.snapshot();
        let [px_off, py_off] = s.pan();
        let z = s.zoom() as f32;

        let mut p = Painter::new()
          .save()
          .translate(px(px_off as f32), px(py_off as f32))
          .scale(z, z);

        let edge_color = hsla(215.0 / 360.0, 0.16, 0.52, 0.9);
        p = p.stroke_color(edge_color);
        for edge in &snap.edges {
          if let Some(pts) = s.routed_edges().get(&edge.id)
            && pts.len() >= 2
          {
            p = p
              .begin_path()
              .move_to(px(pts[0][0] as f32), px(pts[0][1] as f32));
            for pt in &pts[1..] {
              p = p.line_to(px(pt[0] as f32), px(pt[1] as f32));
            }
            p = p.stroke();
          }
        }

        let node_fill = hsla(215.0 / 360.0, 0.9, 0.95, 1.0);
        let sticky_fill = hsla(48.0 / 360.0, 0.96, 0.88, 1.0);
        let node_stroke = hsla(218.0 / 360.0, 0.85, 0.55, 1.0);
        let sel_stroke = hsla(218.0 / 360.0, 0.95, 0.45, 1.0);

        for node in &snap.nodes {
          let selected = s.selected_node_ids().contains(&node.id);
          let fill = if node.kind == "sticky" {
            sticky_fill
          } else {
            node_fill
          };
          let rect = Bounds::new(
            point(px(node.x as f32), px(node.y as f32)),
            size(px(node.w as f32), px(node.h as f32)),
          );
          p = p
            .fill_color(fill)
            .stroke_color(if selected { sel_stroke } else { node_stroke })
            .begin_path()
            .round_rect(rect, px(8.0))
            .fill()
            .stroke();
        }

        p.restore()
      })
      .test_id("board-canvas")
      .on_press_down(move |ev: &PressEvent| {
        let double_click_target = {
          let mut s = s_pdown.borrow_mut();
          s.dispatch(BoardCommand::PointerDown {
            screen_x: f32::from(ev.position.x) as f64,
            screen_y: f32::from(ev.position.y) as f64,
            button: 0,
            shift: ev.modifiers.shift,
            alt: ev.modifiers.alt,
            space: false,
            read_only: false,
            timestamp_ms: None,
          });
          if ev.clicks >= 2 {
            s.selected_node_ids().first().cloned().map(|nid| {
              let txt = s
                .snapshot()
                .nodes
                .iter()
                .find(|n| n.id == nid)
                .map(|n| n.text.clone())
                .unwrap_or_default();
              s.dispatch(BoardCommand::StartEditingNode {
                node_id: nid.clone(),
              });
              (nid, txt)
            })
          } else {
            None
          }
        };
        if let Some((nid, txt)) = double_click_target {
          editing_draft.set(txt);
          editing_node_id.set(Some(nid));
        }
        rev.set(rev.get_untracked() + 1);
      })
      .on_mouse_move(move |ev: &MouseEvent| {
        if ev.pressed_button.is_some() {
          s_pmove.borrow_mut().dispatch(BoardCommand::PointerMove {
            screen_x: f32::from(ev.position.x) as f64,
            screen_y: f32::from(ev.position.y) as f64,
            alt: ev.modifiers.alt,
          });
          rev.set(rev.get_untracked() + 1);
        }
      })
      .on_mouse_up(move |_: &MouseEvent| {
        let json = {
          let mut s = s_pup.borrow_mut();
          let patch = s.dispatch(BoardCommand::PointerUp);
          patch.snapshot_changed.then(|| s.export_compact_json())
        };
        rev.set(rev.get_untracked() + 1);
        if let Some(j) = json {
          p_pup(j);
        }
      })
      .on_wheel(move |ev: &WheelEvent| {
        let dy = match ev.delta {
          WheelDelta::Points(pt) => f32::from(pt.y),
          WheelDelta::Lines(pt) => pt.y * 24.0,
        };
        let factor = if dy < 0.0 { 1.12 } else { 0.89 };
        s_wheel.borrow_mut().dispatch(BoardCommand::WheelZoom {
          factor,
          focal_screen_x: f32::from(ev.position.x) as f64,
          focal_screen_y: f32::from(ev.position.y) as f64,
        });
        rev.set(rev.get_untracked() + 1);
      })
      .flex_1()
      .w_full()
      .h_full(),
    )
}

fn tool_btn(label: &'static str, tid: &'static str, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid)
    .px_2()
    .py_1()
    .border_1()
    .border_color(move || theme().border)
    .rounded(move || theme().radius)
    .bg(move || theme().secondary)
    .on_press(on_click)
    .child(
      text(label)
        .text_xs()
        .text_color(move || theme().secondary_foreground),
    )
}
