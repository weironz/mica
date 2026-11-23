//! 900×640 Knowledge Graph modal with deterministic pure-Rust Fruchterman–Reingold
//! layout and interactive `nsh_core::canvas` pan/zoom/hover/click-to-open.
//!
//! 1:1 parity with `main:clients/mica_flutter/lib/ui/page_graph_view.dart` and
//! `main:clients/mica_flutter/lib/ui/graph_dialog.dart`.

use std::{
  cell::RefCell,
  collections::{HashMap, HashSet},
  f32::consts::TAU,
  rc::Rc,
};

use mica_api_types::View;
use nsh_base::button;
use nsh_core::{
  canvas::{CanvasTextAlign, CanvasTextBaseline},
  geometry::{point, px, radians},
  prelude::*,
};
use uuid::Uuid;

use crate::{
  mica_icons,
  mica_theme::{MicaPalette, palette},
};

/// Fixed layout square matching Flutter `_canvas = 2000`.
pub const GRAPH_CANVAS_SIZE: f32 = 2000.0;
const GRAPH_SEED_RADIUS: f64 = 700.0;
const GRAPH_CLAMP_MIN: f64 = 40.0;
const GRAPH_CLAMP_MAX: f64 = 1960.0;
const GRAPH_HIT_RADIUS: f32 = 26.0;
const GRAPH_MAX_LABELS: usize = 30;
const GRAPH_MIN_ZOOM: f32 = 0.2;
const GRAPH_MAX_ZOOM: f32 = 4.0;

#[derive(Clone, Debug, PartialEq)]
pub struct GraphNode {
  pub id: Uuid,
  pub name: String,
  pub degree: usize,
  pub x: f32,
  pub y: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GraphEdge {
  pub source: Uuid,
  pub target: Uuid,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageGraphData {
  pub nodes: Vec<GraphNode>,
  pub edges: Vec<GraphEdge>,
  pub unlinked: usize,
}

impl PageGraphData {
  pub fn is_empty(&self) -> bool {
    self.nodes.is_empty()
  }
}

/// Scans `mica://page/{id}` and `[[Title]]` wiki links across active workspace pages,
/// computes node degrees, counts unlinked pages, and runs deterministic
/// Fruchterman–Reingold force-directed layout on `2000×2000`.
pub fn build_local_page_graph(
  views: &[View],
  docs: &HashMap<Uuid, String>,
) -> PageGraphData {
  let mut pages: Vec<&View> = views
    .iter()
    .filter(|v| v.object_type != "folder" && !v.is_deleted)
    .collect();
  // Deterministic input ordering so duplicate titles / seed order are stable.
  pages.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.object_id.cmp(&b.object_id)));

  let mut id_lookup: HashMap<Uuid, Uuid> = HashMap::with_capacity(pages.len() * 2);
  let mut title_lookup: HashMap<String, Uuid> = HashMap::with_capacity(pages.len());
  let mut page_names: HashMap<Uuid, String> = HashMap::with_capacity(pages.len());

  for v in &pages {
    id_lookup.insert(v.object_id, v.object_id);
    id_lookup.insert(v.id, v.object_id);
    page_names.entry(v.object_id).or_insert_with(|| v.name.clone());
    let norm = normalize_title(&v.name);
    if !norm.is_empty() {
      title_lookup.entry(norm).or_insert(v.object_id);
    }
  }

  let mut seen_edges: HashSet<(Uuid, Uuid)> = HashSet::new();
  let mut edges: Vec<GraphEdge> = Vec::new();
  let mut degrees: HashMap<Uuid, usize> = HashMap::new();

  for v in &pages {
    let src = v.object_id;
    let Some(md) = docs.get(&src) else {
      continue;
    };
    let mut targets: Vec<Uuid> = Vec::new();
    extract_mica_page_links(md, &id_lookup, &mut targets);
    extract_wiki_links(md, &title_lookup, &mut targets);

    for dst in targets {
      if dst == src {
        continue;
      }
      let canonical = if src < dst { (src, dst) } else { (dst, src) };
      if seen_edges.insert(canonical) {
        edges.push(GraphEdge {
          source: src,
          target: dst,
        });
        *degrees.entry(src).or_insert(0) += 1;
        *degrees.entry(dst).or_insert(0) += 1;
      }
    }
  }

  let mut nodes: Vec<GraphNode> = Vec::new();
  let mut unlinked = 0usize;
  let mut seen_pages: HashSet<Uuid> = HashSet::with_capacity(pages.len());

  for v in &pages {
    if !seen_pages.insert(v.object_id) {
      continue;
    }
    let degree = degrees.get(&v.object_id).copied().unwrap_or(0);
    if degree == 0 {
      unlinked += 1;
    } else {
      nodes.push(GraphNode {
        id: v.object_id,
        name: page_names
          .get(&v.object_id)
          .cloned()
          .unwrap_or_else(|| v.name.clone()),
        degree,
        x: GRAPH_CANVAS_SIZE * 0.5,
        y: GRAPH_CANVAS_SIZE * 0.5,
      });
    }
  }

  // Degree descending, then name, then UUID so hubs seed apart and top-30 labels pick hubs.
  nodes.sort_by(|a, b| {
    b.degree
      .cmp(&a.degree)
      .then_with(|| a.name.cmp(&b.name))
      .then_with(|| a.id.cmp(&b.id))
  });

  layout_page_graph(&mut nodes, &edges);

  PageGraphData {
    nodes,
    edges,
    unlinked,
  }
}

/// Deterministic Fruchterman–Reingold placement on `2000×2000`.
///
/// - Circle seed: center `(1000, 1000)`, radius `700`.
/// - Ideal edge length: `k = 0.9 * sqrt(4_000_000 / n)`.
/// - Iterations: `300` (`120` when `n > 300`).
/// - Coordinates clamped to `[40, 1960]`.
pub fn layout_page_graph(nodes: &mut [GraphNode], edges: &[GraphEdge]) {
  let n = nodes.len();
  if n == 0 {
    return;
  }

  let canvas = GRAPH_CANVAS_SIZE as f64;
  let center = canvas * 0.5;
  let mut index: HashMap<Uuid, usize> = HashMap::with_capacity(n);
  let mut pos: Vec<(f64, f64)> = Vec::with_capacity(n);

  for (i, node) in nodes.iter().enumerate() {
    index.insert(node.id, i);
    let angle = std::f64::consts::TAU * (i as f64) / (n as f64);
    pos.push((
      center + angle.cos() * GRAPH_SEED_RADIUS,
      center + angle.sin() * GRAPH_SEED_RADIUS,
    ));
  }

  let k = 0.9 * ((canvas * canvas) / (n as f64)).sqrt();
  let iterations = if n > 300 { 120 } else { 300 };
  let mut temperature = canvas / 10.0;
  let cooling = temperature / (iterations as f64 + 1.0);

  let mut disp: Vec<(f64, f64)> = vec![(0.0, 0.0); n];
  for _ in 0..iterations {
    disp.fill((0.0, 0.0));

    // Repulsion: every pair pushes apart.
    for i in 0..n {
      for j in (i + 1)..n {
        let mut dx = pos[i].0 - pos[j].0;
        let mut dy = pos[i].1 - pos[j].1;
        let mut dist = dx.hypot(dy);
        if dist < 0.01 {
          dx = 0.01 * (i + 1) as f64;
          dy = 0.01 * (j + 1) as f64;
          dist = dx.hypot(dy);
        }
        let force = (k * k) / dist;
        let push_x = (dx / dist) * force;
        let push_y = (dy / dist) * force;
        disp[i].0 += push_x;
        disp[i].1 += push_y;
        disp[j].0 -= push_x;
        disp[j].1 -= push_y;
      }
    }

    // Attraction: linked pages pull together.
    for edge in edges {
      let Some(&a) = index.get(&edge.source) else {
        continue;
      };
      let Some(&b) = index.get(&edge.target) else {
        continue;
      };
      if a == b {
        continue;
      }
      let dx = pos[a].0 - pos[b].0;
      let dy = pos[a].1 - pos[b].1;
      let dist = dx.hypot(dy);
      if dist < 0.01 {
        continue;
      }
      let force = (dist * dist) / k;
      let pull_x = (dx / dist) * force;
      let pull_y = (dy / dist) * force;
      disp[a].0 -= pull_x;
      disp[a].1 -= pull_y;
      disp[b].0 += pull_x;
      disp[b].1 += pull_y;
    }

    // Move capped by cooling temperature and clamp to canvas bounds.
    for i in 0..n {
      let (dx, dy) = disp[i];
      let dist = dx.hypot(dy);
      if dist < 0.01 {
        continue;
      }
      let step = dist.min(temperature);
      pos[i].0 = (pos[i].0 + (dx / dist) * step).clamp(GRAPH_CLAMP_MIN, GRAPH_CLAMP_MAX);
      pos[i].1 = (pos[i].1 + (dy / dist) * step).clamp(GRAPH_CLAMP_MIN, GRAPH_CLAMP_MAX);
    }

    temperature -= cooling;
  }

  for (node, (x, y)) in nodes.iter_mut().zip(pos) {
    node.x = x as f32;
    node.y = y as f32;
  }
}

fn normalize_title(raw: &str) -> String {
  raw.trim().to_lowercase()
}

fn extract_mica_page_links(
  md: &str,
  id_lookup: &HashMap<Uuid, Uuid>,
  out: &mut Vec<Uuid>,
) {
  const PREFIX: &str = "mica://page/";
  let mut rest = md;
  while let Some(idx) = rest.find(PREFIX) {
    let after = &rest[idx + PREFIX.len()..];
    if after.len() >= 36
      && let Ok(parsed) = Uuid::parse_str(&after[..36])
      && let Some(&canonical) = id_lookup.get(&parsed)
    {
      out.push(canonical);
    }
    rest = after;
  }
}

fn extract_wiki_links(
  md: &str,
  title_lookup: &HashMap<String, Uuid>,
  out: &mut Vec<Uuid>,
) {
  let mut rest = md;
  while let Some(start) = rest.find("[[") {
    let after = &rest[start + 2..];
    let Some(end) = after.find("]]") else {
      break;
    };
    let inner = &after[..end];
    if !inner.contains('\n') && !inner.contains('[') {
      // Support [[Title]], [[Title|Alias]], and [[Title#Heading]].
      let base = inner
        .split('|')
        .next()
        .unwrap_or(inner)
        .split('#')
        .next()
        .unwrap_or(inner);
      let norm = normalize_title(base);
      if let Some(&target) = title_lookup.get(&norm) {
        out.push(target);
      }
    }
    rest = &after[end + 2..];
  }
}

fn node_radius(degree: usize) -> f32 {
  (4.0 + (degree as f32).sqrt() * 3.0).min(22.0)
}

/// Nearest node to `(wx, wy)` in world coordinates within hit radius.
pub fn nearest_node(nodes: &[GraphNode], wx: f32, wy: f32, hit_radius: f32) -> Option<Uuid> {
  let mut best: Option<Uuid> = None;
  let mut best_dist = f32::INFINITY;
  for node in nodes {
    let d = (node.x - wx).hypot(node.y - wy);
    if d < best_dist {
      best_dist = d;
      best = Some(node.id);
    }
  }
  (best_dist <= hit_radius).then_some(best).flatten()
}

#[derive(Clone, Copy)]
struct DragPanState {
  start_screen: (f32, f32),
  start_pan: (f32, f32),
  moved: bool,
}

fn fit_viewport_transform(vp_w: f32, vp_h: f32) -> (f32, (f32, f32)) {
  let w = vp_w.max(200.0);
  let h = vp_h.max(200.0);
  let zoom = ((w.min(h) - 48.0) / GRAPH_CANVAS_SIZE).clamp(GRAPH_MIN_ZOOM, 1.0);
  let pan_x = (w - GRAPH_CANVAS_SIZE * zoom) * 0.5;
  let pan_y = (h - GRAPH_CANVAS_SIZE * zoom) * 0.5;
  (zoom, (pan_x, pan_y))
}

/// Centered `900×640` Knowledge Graph modal (`关系图`).
pub fn graph_dialog_modal(
  open: Signal<bool>,
  graph: Signal<PageGraphData>,
  current_doc: Signal<Option<Uuid>>,
  dark_mode: Signal<bool>,
  on_open_page: Rc<dyn Fn(Uuid)>,
  on_close: Rc<dyn Fn()>,
) -> impl IntoElement {
  when(move || open.get(), {
    let on_open_page = on_open_page.clone();
    let on_close = on_close.clone();
    move || {
      let (init_zoom, init_pan) = fit_viewport_transform(900.0, 586.0);
      let zoom = signal(init_zoom);
      let pan = signal(init_pan);
      let viewport_size = signal((900.0_f32, 586.0_f32));
      let has_fitted = Rc::new(RefCell::new(false));
      let hovered_id: Signal<Option<Uuid>> = signal(None);
      let drag_state: Rc<RefCell<Option<DragPanState>>> = Rc::new(RefCell::new(None));

      let close_bg = on_close.clone();
      let close_btn = on_close.clone();
      let open_cb = on_open_page.clone();

      view()
        .test_id("graph-modal-backdrop")
        .absolute()
        .inset_0()
        .z_index(60)
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("graph-backdrop-dismiss")
            .accessibility_label("关闭关系图弹窗")
            .absolute()
            .inset_0()
            .on_press(move || close_bg()),
        )
        .child(
          view()
            .test_id("graph-dialog")
            .w(px(900.0))
            .h(px(640.0))
            .max_w(relative(0.92))
            .max_h(relative(0.90))
            .rounded(px(12.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .overflow_hidden()
            .shadow_md()
            .z_index(61)
            .child(
              // Header: 关系图 + ×
              view()
                .h(px(52.0))
                .px(px(20.0))
                .flex_row()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .child(
                  view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(dynamic(move || {
                      mica_icons::graph_nodes(palette(dark_mode.get()).text_muted_hex)
                    }))
                    .child(
                      text("关系图")
                        .text_size(px(16.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || palette(dark_mode.get()).text_primary),
                    ),
                )
                .child(
                  button()
                    .test_id("graph-dialog-close")
                    .accessibility_label("关闭关系图")
                    .w(px(28.0))
                    .h(px(28.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || close_btn())
                    .child(dynamic(move || {
                      mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                    })),
                ),
            )
            .child(
              // Body: Empty state OR Interactive Canvas
              view()
                .flex_1()
                .min_h(px(0.0))
                .relative()
                .child(dynamic({
                  let open_cb = open_cb.clone();
                  let has_fitted = has_fitted.clone();
                  let drag_state = drag_state.clone();
                  move || {
                    let data = graph.get();
                    if data.is_empty() {
                      return empty_graph_view(data.unlinked, dark_mode).into_element();
                    }
                    interactive_graph_body(
                      graph,
                      current_doc,
                      dark_mode,
                      zoom,
                      pan,
                      viewport_size,
                      has_fitted.clone(),
                      hovered_id,
                      drag_state.clone(),
                      open_cb.clone(),
                    )
                    .into_element()
                  }
                })),
            ),
        )
    }
  })
}

fn empty_graph_view(unlinked: usize, dark_mode: Signal<bool>) -> impl IntoElement {
  let heading = if unlinked > 0 {
    format!("还没有页面互相链接（{unlinked} 个页面）")
  } else {
    "还没有页面互相链接".to_string()
  };

  view()
    .test_id("graph-empty-state")
    .flex_1()
    .w_full()
    .h_full()
    .items_center()
    .justify_center()
    .p(px(32.0))
    .gap_2()
    .child(
      view()
        .w(px(48.0))
        .h(px(48.0))
        .rounded(px(10.0))
        .bg(move || palette(dark_mode.get()).surface_sunken)
        .items_center()
        .justify_center()
        .child(dynamic(move || {
          mica_icons::graph_nodes(palette(dark_mode.get()).text_faint_hex)
        })),
    )
    .child(
      text(heading)
        .text_size(px(14.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_muted),
    )
    .child(
      text("在页面里输入 [[ 链接到另一个页面，连线就会出现在这里。")
        .text_size(px(12.5))
        .text_color(move || palette(dark_mode.get()).text_faint),
    )
}

#[allow(clippy::too_many_arguments)]
fn interactive_graph_body(
  graph: Signal<PageGraphData>,
  current_doc: Signal<Option<Uuid>>,
  dark_mode: Signal<bool>,
  zoom: Signal<f32>,
  pan: Signal<(f32, f32)>,
  viewport_size: Signal<(f32, f32)>,
  has_fitted: Rc<RefCell<bool>>,
  hovered_id: Signal<Option<Uuid>>,
  drag_state: Rc<RefCell<Option<DragPanState>>>,
  on_open_page: Rc<dyn Fn(Uuid)>,
) -> impl IntoElement {
  let drag_down = drag_state.clone();
  let drag_move = drag_state.clone();
  let drag_up = drag_state.clone();
  let drag_leave = drag_state;

  let hit_at_screen = move |sx: f32, sy: f32| -> Option<Uuid> {
    let z = zoom.get_untracked().max(GRAPH_MIN_ZOOM);
    let (px_off, py_off) = pan.get_untracked();
    let wx = (sx - px_off) / z;
    let wy = (sy - py_off) / z;
    // Scale hit radius slightly when zoomed out so small dots remain easy to click.
    let eff_radius = GRAPH_HIT_RADIUS.max(14.0 / z);
    let data = graph.get_untracked();
    nearest_node(&data.nodes, wx, wy, eff_radius)
  };

  let hit_move = hit_at_screen;
  let hit_up = hit_at_screen;

  let zoom_by = move |factor: f32| {
    let (vw, vh) = viewport_size.get_untracked();
    let fx = vw * 0.5;
    let fy = vh * 0.5;
    let old_z = zoom.get_untracked();
    let new_z = (old_z * factor).clamp(GRAPH_MIN_ZOOM, GRAPH_MAX_ZOOM);
    let (ox, oy) = pan.get_untracked();
    let wx = (fx - ox) / old_z;
    let wy = (fy - oy) / old_z;
    zoom.set(new_z);
    pan.set((fx - wx * new_z, fy - wy * new_z));
  };

  let zoom_out = zoom_by;
  let zoom_in = zoom_by;
  let zoom_reset = move || {
    let (vw, vh) = viewport_size.get_untracked();
    let (z, p) = fit_viewport_transform(vw, vh);
    zoom.set(z);
    pan.set(p);
  };

  view()
    .flex_1()
    .w_full()
    .h_full()
    .relative()
    .overflow_hidden()
    .child(
      canvas(move || {
        let data = graph.get();
        let dark = dark_mode.get();
        let current = current_doc.get();
        let hovered = hovered_id.get();
        let z = zoom.get();
        let p = pan.get();
        let vp = viewport_size.get();
        paint_graph(&data, current, hovered, dark, z, p, vp)
      })
      .test_id("graph-canvas")
      .w_full()
      .h_full()
      .on_layout(move |b: &Bounds<Points>| {
        let w = f32::from(b.size.width).max(200.0);
        let h = f32::from(b.size.height).max(200.0);
        let (pw, ph) = viewport_size.get_untracked();
        if (pw - w).abs() > 1.0 || (ph - h).abs() > 1.0 {
          viewport_size.set((w, h));
        }
        let mut fitted = has_fitted.borrow_mut();
        if !*fitted {
          *fitted = true;
          let (z, p) = fit_viewport_transform(w, h);
          zoom.set(z);
          pan.set(p);
        }
      })
      .on_press_down(move |ev: &PressEvent| {
        let sx = f32::from(ev.position.x);
        let sy = f32::from(ev.position.y);
        *drag_down.borrow_mut() = Some(DragPanState {
          start_screen: (sx, sy),
          start_pan: pan.get_untracked(),
          moved: false,
        });
      })
      .on_mouse_move(move |ev: &MouseEvent| {
        let sx = f32::from(ev.position.x);
        let sy = f32::from(ev.position.y);
        if ev.pressed_button.is_some() {
          let mut guard = drag_move.borrow_mut();
          if let Some(st) = guard.as_mut() {
            let dx = sx - st.start_screen.0;
            let dy = sy - st.start_screen.1;
            if dx.hypot(dy) > 4.0 {
              st.moved = true;
            }
            pan.set((st.start_pan.0 + dx, st.start_pan.1 + dy));
          }
        } else {
          let next = hit_move(sx, sy);
          if next != hovered_id.get_untracked() {
            hovered_id.set(next);
          }
        }
      })
      .on_mouse_up(move |ev: &MouseEvent| {
        let st = drag_up.borrow_mut().take();
        if let Some(st) = st
          && !st.moved
        {
          let sx = f32::from(ev.position.x);
          let sy = f32::from(ev.position.y);
          if let Some(node_id) = hit_up(sx, sy) {
            (on_open_page)(node_id);
          }
        }
      })
      .on_mouse_leave(move |_: &MouseEvent| {
        *drag_leave.borrow_mut() = None;
        if hovered_id.get_untracked().is_some() {
          hovered_id.set(None);
        }
      })
      .on_wheel(move |ev: &WheelEvent| {
        let dy = match ev.delta {
          WheelDelta::Points(pt) => f32::from(pt.y),
          WheelDelta::Lines(pt) => pt.y * 24.0,
        };
        let factor = if dy < 0.0 { 1.12 } else { 0.89 };
        let fx = f32::from(ev.position.x);
        let fy = f32::from(ev.position.y);
        let old_z = zoom.get_untracked();
        let new_z = (old_z * factor).clamp(GRAPH_MIN_ZOOM, GRAPH_MAX_ZOOM);
        let (ox, oy) = pan.get_untracked();
        let wx = (fx - ox) / old_z;
        let wy = (fy - oy) / old_z;
        zoom.set(new_z);
        pan.set((fx - wx * new_z, fy - wy * new_z));
      }),
    )
    .child(when(move || graph.get().unlinked > 0, move || {
      view()
        .test_id("graph-unlinked-note")
        .absolute()
        .left(px(12.0))
        .bottom(px(12.0))
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .child(
          text(move || format!("另有 {} 个页面没有任何链接，未画出", graph.get().unlinked))
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).text_faint),
        )
    }))
    .child(
      // Floating bottom-right zoom controls: - / 重置 / +
      view()
        .test_id("graph-zoom-controls")
        .absolute()
        .right(px(12.0))
        .bottom(px(12.0))
        .flex_row()
        .items_center()
        .gap_1()
        .px_1()
        .py_1()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .shadow_sm()
        .child(zoom_btn(
          dark_mode,
          "graph-zoom-out",
          "缩小关系图",
          "-",
          move || zoom_out(0.84),
        ))
        .child(
          button()
            .test_id("graph-zoom-reset")
            .accessibility_label("重置关系图缩放与位置")
            .px_2()
            .h(px(26.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(zoom_reset)
            .child(
              text("重置")
                .text_size(px(12.0))
                .text_color(move || palette(dark_mode.get()).text_primary),
            ),
        )
        .child(zoom_btn(
          dark_mode,
          "graph-zoom-in",
          "放大关系图",
          "+",
          move || zoom_in(1.18),
        )),
    )
}

fn zoom_btn(
  dark_mode: Signal<bool>,
  tid: &'static str,
  a11y: &'static str,
  label: &'static str,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(a11y)
    .w(px(26.0))
    .h(px(26.0))
    .items_center()
    .justify_center()
    .rounded(px(6.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_press)
    .child(
      text(label)
        .text_size(px(14.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_primary),
    )
}

fn paint_graph(
  data: &PageGraphData,
  current: Option<Uuid>,
  hovered: Option<Uuid>,
  dark: bool,
  zoom: f32,
  pan: (f32, f32),
  vp: (f32, f32),
) -> Painter {
  let pal: MicaPalette = palette(dark);
  let mut p = Painter::new()
    .fill_color(pal.surface_overlay)
    .fill_rect(px(0.0), px(0.0), px(vp.0.max(200.0)), px(vp.1.max(200.0)));

  let positions: HashMap<Uuid, (f32, f32)> = data
    .nodes
    .iter()
    .map(|n| (n.id, (n.x, n.y)))
    .collect();

  p = p.save().translate(px(pan.0), px(pan.1)).scale(zoom, zoom);

  // 1. Unlit edges first, then hovered incident edges on top (1.6px text_primary).
  let mut lit_edges: Vec<((f32, f32), (f32, f32))> = Vec::new();
  p = p.stroke_color(pal.border_normal).line_width(px(1.0));
  for edge in &data.edges {
    let (Some(&a), Some(&b)) = (positions.get(&edge.source), positions.get(&edge.target)) else {
      continue;
    };
    let is_lit = hovered.is_some_and(|h| edge.source == h || edge.target == h);
    if is_lit {
      lit_edges.push((a, b));
    } else {
      p = p
        .begin_path()
        .move_to(px(a.0), px(a.1))
        .line_to(px(b.0), px(b.1))
        .stroke();
    }
  }
  if !lit_edges.is_empty() {
    p = p.stroke_color(pal.text_primary).line_width(px(1.6));
    for (a, b) in lit_edges {
      p = p
        .begin_path()
        .move_to(px(a.0), px(a.1))
        .line_to(px(b.0), px(b.1))
        .stroke();
    }
  }

  // 2. Nodes + current-page outer ring + top-30/current/hovered labels.
  for (i, node) in data.nodes.iter().enumerate() {
    let radius = node_radius(node.degree);
    let is_current = current == Some(node.id);
    let is_hovered = hovered == Some(node.id);

    let fill = if is_current || is_hovered {
      pal.text_primary
    } else {
      pal.text_muted
    };
    p = p
      .fill_color(fill)
      .begin_path()
      .arc(px(node.x), px(node.y), px(radius), radians(0.0), radians(TAU))
      .fill();

    if is_current {
      p = p
        .stroke_color(pal.text_primary)
        .line_width(px(1.5))
        .begin_path()
        .arc(
          px(node.x),
          px(node.y),
          px(radius + 4.0),
          radians(0.0),
          radians(TAU),
        )
        .stroke();
    }

    if (i < GRAPH_MAX_LABELS || is_hovered || is_current) && !node.name.is_empty() {
      let label = truncate_label(&node.name, 18);
      p = p
        .fill_color(pal.text_primary)
        .font("11px system-ui")
        .text_align(CanvasTextAlign::Center)
        .text_baseline(CanvasTextBaseline::Top)
        .fill_text(&label, px(node.x), px(node.y + radius + 4.0));
    }
  }

  // Ensure point() import is used cleanly for future bounding extensions if needed.
  let _ = point(px(0.0), px(0.0));
  p.restore()
}

fn truncate_label(name: &str, max_chars: usize) -> String {
  let mut chars = name.chars();
  let prefix: String = chars.by_ref().take(max_chars).collect();
  if chars.next().is_some() {
    format!("{prefix}…")
  } else {
    prefix
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn make_page_view(id: Uuid, name: &str, deleted: bool) -> View {
    View {
      id: Uuid::new_v4(),
      workspace_id: Uuid::nil(),
      parent_view_id: None,
      object_id: id,
      object_type: "page".to_string(),
      name: name.to_string(),
      icon: None,
      position: "a0".to_string(),
      is_deleted: deleted,
      created_by: Uuid::nil(),
      created_at: chrono::Utc::now(),
      updated_at: chrono::Utc::now(),
      state_bytes: None,
    }
  }

  #[test]
  fn build_local_page_graph_parses_wiki_and_mica_links_and_counts_unlinked() {
    let hub = Uuid::new_v4();
    let alpha = Uuid::new_v4();
    let beta = Uuid::new_v4();
    let orphan = Uuid::new_v4();
    let deleted = Uuid::new_v4();

    let mut folder = make_page_view(Uuid::new_v4(), "Folder", false);
    folder.object_type = "folder".to_string();

    let views = vec![
      folder,
      make_page_view(hub, "架构总览", false),
      make_page_view(alpha, "Alpha 设计", false),
      make_page_view(beta, "Beta 方案", false),
      make_page_view(orphan, "孤立笔记", false),
      make_page_view(deleted, "已删除", true),
    ];

    let mut docs = HashMap::new();
    docs.insert(
      hub,
      "参考 [[Alpha 设计]] 和 [[Beta 方案|别名]]，以及自链 [[架构总览]] 和 [[已删除]]。".to_string(),
    );
    docs.insert(
      alpha,
      format!("回到 [总览](mica://page/{hub}) 与 [Beta](mica://page/{beta})"),
    );
    docs.insert(beta, "纯文本无出链".to_string());
    docs.insert(orphan, "没有任何链接".to_string());

    let graph = build_local_page_graph(&views, &docs);

    assert_eq!(graph.unlinked, 1);
    assert_eq!(graph.nodes.len(), 3);
    // Edges: (hub, alpha), (hub, beta), (alpha, beta) — duplicate (alpha, hub) collapsed.
    assert_eq!(graph.edges.len(), 3);
    for node in &graph.nodes {
      assert_eq!(node.degree, 2);
      assert!((40.0..=1960.0).contains(&node.x));
      assert!((40.0..=1960.0).contains(&node.y));
    }
  }

  #[test]
  fn fruchterman_reingold_layout_is_deterministic_and_bounded() {
    let a = Uuid::from_u128(1);
    let b = Uuid::from_u128(2);
    let c = Uuid::from_u128(3);
    let d = Uuid::from_u128(4);

    let views = vec![
      make_page_view(a, "A", false),
      make_page_view(b, "B", false),
      make_page_view(c, "C", false),
      make_page_view(d, "D", false),
    ];
    let mut docs = HashMap::new();
    docs.insert(a, "[[B]] [[C]]".to_string());
    docs.insert(b, "[[C]]".to_string());
    docs.insert(c, "[[D]]".to_string());

    let g1 = build_local_page_graph(&views, &docs);
    let g2 = build_local_page_graph(&views, &docs);

    assert_eq!(g1.nodes.len(), 4);
    assert_eq!(g1.unlinked, 0);
    // Deterministic across runs.
    assert_eq!(g1.nodes, g2.nodes);
    for node in &g1.nodes {
      assert!(node.x.is_finite() && node.y.is_finite());
      assert!((40.0..=1960.0).contains(&node.x));
      assert!((40.0..=1960.0).contains(&node.y));
    }
    // Highest-degree node (C, degree 3) comes first.
    assert_eq!(g1.nodes[0].id, c);
    assert_eq!(g1.nodes[0].degree, 3);
  }

  #[test]
  fn nearest_node_hit_detection_respects_radius() {
    let a = Uuid::from_u128(10);
    let b = Uuid::from_u128(20);
    let nodes = vec![
      GraphNode { id: a, name: "A".into(), degree: 2, x: 100.0, y: 100.0 },
      GraphNode { id: b, name: "B".into(), degree: 1, x: 200.0, y: 200.0 },
    ];
    assert_eq!(nearest_node(&nodes, 110.0, 110.0, 26.0), Some(a));
    assert_eq!(nearest_node(&nodes, 150.0, 150.0, 26.0), None);
  }
}
