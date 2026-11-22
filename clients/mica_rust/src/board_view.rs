//! Pure-Rust L3 infinite whiteboard view backed directly by `mica_board::BoardSession`
//! and `nsh_core::canvas` (`Painter`).

use std::{cell::RefCell, rc::Rc};

use mica_board::{BoardCommand, BoardSession};
use nsh_base::{button, theme};
use nsh_core::{
  geometry::{Bounds, Point, Size, px},
  prelude::*,
};

/// Render a full interactive whiteboard pane wired straight to `BoardSession`.
pub fn board_view(session: Rc<RefCell<BoardSession>>, rev: Signal<u64>) -> impl IntoElement {
  let s_add = session.clone();
  let s_child = session.clone();
  let s_layout = session.clone();
  let s_undo = session.clone();
  let s_redo = session.clone();
  let s_zoom_in = session.clone();
  let s_zoom_out = session.clone();
  let s_fit = session.clone();
  let s_paint = session.clone();
  let s_status = session.clone();

  view()
    .flex_1()
    .bg(move || theme().background)
    .child(
      view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(tool_btn("+ Node", "board-add-node", move || {
          {
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
          }
          rev.set(rev.get_untracked() + 1);
        }))
        .child(tool_btn("Mind Child", "board-add-child", move || {
          let changed = {
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
              true
            } else {
              false
            }
          };
          if changed {
            rev.set(rev.get_untracked() + 1);
          }
        }))
        .child(tool_btn("Auto Layout", "board-layout", move || {
          s_layout.borrow_mut().dispatch(BoardCommand::LayoutMindmap);
          rev.set(rev.get_untracked() + 1);
        }))
        .child(tool_btn("Undo", "board-undo", move || {
          s_undo.borrow_mut().dispatch(BoardCommand::Undo);
          rev.set(rev.get_untracked() + 1);
        }))
        .child(tool_btn("Redo", "board-redo", move || {
          s_redo.borrow_mut().dispatch(BoardCommand::Redo);
          rev.set(rev.get_untracked() + 1);
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
        .child(
          text(move || {
            rev.get();
            let s = s_status.borrow();
            format!(
              "Nodes: {} | Edges: {} | Zoom: {:.0}%",
              s.snapshot().nodes.len(),
              s.snapshot().edges.len(),
              s.zoom() * 100.0
            )
          })
          .text_xs()
          .text_color(move || theme().muted_foreground),
        ),
    )
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
        let node_stroke = hsla(218.0 / 360.0, 0.85, 0.55, 1.0);
        let sel_stroke = hsla(218.0 / 360.0, 0.95, 0.45, 1.0);
        for node in &snap.nodes {
          let rect = Bounds::new(
            Point::new(px(node.x as f32), px(node.y as f32)),
            Size::new(px(node.w as f32), px(node.h as f32)),
          );
          let selected = s.selected_node_ids().iter().any(|id| id == &node.id);
          p = p
            .begin_path()
            .round_rect(rect, px(8.0))
            .fill_color(node_fill)
            .fill()
            .stroke_color(if selected { sel_stroke } else { node_stroke })
            .stroke();
        }
        p.restore()
      })
      .flex_1(),
    )
}

fn tool_btn(
  label: &'static str,
  test_id: &'static str,
  on_click: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(test_id)
    .px_2()
    .py_1()
    .rounded(move || theme().radius)
    .bg(move || theme().secondary)
    .on_press(on_click)
    .child(
      text(label)
        .text_xs()
        .text_color(move || theme().secondary_foreground),
    )
}
