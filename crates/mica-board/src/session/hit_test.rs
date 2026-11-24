//! Geometry hit-testing for `BoardSession`:
//! collapse badges, connector anchor ports, 8-direction resize handles,
//! nodes (preserving frame 30px title-bar + border rule), and routed/curved edges.

use std::collections::{HashMap, HashSet};

use crate::model::{BoardEdge, BoardNode};
use crate::routing::{Side, anchor_point, route_edge};

use super::BoardSession;
use super::catalog::{
  ANCHOR_PORT_HIT_RADIUS, COLLAPSE_BADGE_HIT_RADIUS, EDGE_HIT_TOLERANCE, FRAME_INNER_DEFLATE,
  FRAME_OUTER_INFLATE, FRAME_TITLE_BAR_H, RESIZE_HANDLE_HIT_RADIUS,
};
use super::types::{Rect, ResizeHandle};

impl BoardSession {
  /// Hit-test mind-map collapse/expand badges (`(x + w + 10, y + h / 2)`, radius `11 / zoom`).
  pub fn hit_test_collapse_badge(&self, world: [f64; 2]) -> Option<String> {
    let mut has_children: HashSet<&str> = HashSet::new();
    for n in &self.snapshot.nodes {
      if let Some(pid) = n.parent_id.as_deref() {
        has_children.insert(pid);
      }
    }
    let hit_radius = COLLAPSE_BADGE_HIT_RADIUS / self.zoom;
    for n in &self.snapshot.nodes {
      if self.collapsed_descendants.contains(&n.id) || !has_children.contains(n.id.as_str()) {
        continue;
      }
      if n.kind != "mind_root" && n.kind != "mind_node" {
        continue;
      }
      let badge_center = [n.x + n.w + 10.0, n.y + n.h / 2.0];
      if dist(world, badge_center) <= hit_radius {
        return Some(n.id.clone());
      }
    }
    None
  }

  /// Hit-test 4-port connector anchors (`t`, `r`, `b`, `l`) on selected or hovered non-frame nodes.
  pub fn hit_test_anchor_port(&self, world: [f64; 2]) -> Option<(String, String)> {
    let mut candidates: Vec<&str> = Vec::with_capacity(self.selected_node_ids.len() + 1);
    for id in &self.selected_node_ids {
      if !candidates.contains(&id.as_str()) {
        candidates.push(id.as_str());
      }
    }
    if let Some(hid) = self.hovered_node_id.as_deref() {
      if !candidates.contains(&hid) {
        candidates.push(hid);
      }
    }
    let hit_radius = ANCHOR_PORT_HIT_RADIUS / self.zoom;
    for id in candidates {
      let Some(&idx) = self.node_index.get(id) else {
        continue;
      };
      if self.collapsed_descendants.contains(id) {
        continue;
      }
      let n = &self.snapshot.nodes[idx];
      if n.kind == "frame" {
        continue;
      }
      for (side_str, side) in [
        ("t", Side::Top),
        ("r", Side::Right),
        ("b", Side::Bottom),
        ("l", Side::Left),
      ] {
        let pt = anchor_point(n, side);
        if dist(world, pt) <= hit_radius {
          return Some((n.id.clone(), side_str.to_string()));
        }
      }
    }
    None
  }

  /// Hit-test 8-direction resize handles when exactly 1 node is selected.
  pub fn hit_test_resize_handle(&self, world: [f64; 2]) -> Option<(String, ResizeHandle)> {
    if self.selected_node_ids.len() != 1 {
      return None;
    }
    let id = &self.selected_node_ids[0];
    if self.collapsed_descendants.contains(id) {
      return None;
    }
    let &idx = self.node_index.get(id)?;
    let n = &self.snapshot.nodes[idx];
    let tol = RESIZE_HANDLE_HIT_RADIUS / self.zoom;
    let handles = [
      (ResizeHandle::Nw, [n.x, n.y]),
      (ResizeHandle::N, [n.x + n.w / 2.0, n.y]),
      (ResizeHandle::Ne, [n.x + n.w, n.y]),
      (ResizeHandle::E, [n.x + n.w, n.y + n.h / 2.0]),
      (ResizeHandle::Se, [n.x + n.w, n.y + n.h]),
      (ResizeHandle::S, [n.x + n.w / 2.0, n.y + n.h]),
      (ResizeHandle::Sw, [n.x, n.y + n.h]),
      (ResizeHandle::W, [n.x, n.y + n.h / 2.0]),
    ];
    for (handle, pt) in handles {
      if dist(world, pt) <= tol {
        return Some((n.id.clone(), handle));
      }
    }
    None
  }

  /// Hit-test topmost visible node. Frames are bottom-sorted and only hit on their
  /// 30px title bar or outer/inner border band so clicks inside reach child nodes.
  pub fn hit_test_node(&self, world: [f64; 2]) -> Option<&BoardNode> {
    let mut ordered: Vec<&BoardNode> = self
      .snapshot
      .nodes
      .iter()
      .filter(|n| !self.collapsed_descendants.contains(&n.id))
      .collect();
    ordered.sort_by(|a, b| {
      let af = if a.kind == "frame" { 0u8 } else { 1u8 };
      let bf = if b.kind == "frame" { 0u8 } else { 1u8 };
      af.cmp(&bf).then_with(|| a.z.cmp(&b.z))
    });
    for &n in ordered.iter().rev() {
      let rect = Rect::from_node(n);
      if n.kind == "frame" {
        let outer = rect.inflate(FRAME_OUTER_INFLATE / self.zoom);
        let inner = rect.deflate(FRAME_INNER_DEFLATE / self.zoom);
        let title_bar = Rect::new(n.x, n.y, n.w, FRAME_TITLE_BAR_H);
        if title_bar.contains(world) || (outer.contains(world) && !inner.contains(world)) {
          return Some(n);
        }
      } else if rect.contains(world) {
        return Some(n);
      }
    }
    None
  }

  /// Hit-test visible edges using routed polylines and cubic curve sampling.
  pub fn hit_test_edge(&self, world: [f64; 2]) -> Option<&BoardEdge> {
    let tol = EDGE_HIT_TOLERANCE / self.zoom;
    let node_map: HashMap<&str, &BoardNode> = self
      .snapshot
      .nodes
      .iter()
      .map(|n| (n.id.as_str(), n))
      .collect();

    for edge in self.snapshot.edges.iter().rev() {
      if self.collapsed_descendants.contains(&edge.from_id)
        || self.collapsed_descendants.contains(&edge.to_id)
      {
        continue;
      }
      if !node_map.contains_key(edge.from_id.as_str())
        || !node_map.contains_key(edge.to_id.as_str())
      {
        continue;
      }
      let fallback;
      let pts: &[[f64; 2]] = if let Some(cached) = self.routed_edges.get(&edge.id) {
        cached.as_slice()
      } else {
        fallback = route_edge(edge, &self.snapshot.nodes, &node_map);
        fallback.as_slice()
      };
      if pts.len() < 2 {
        continue;
      }
      if edge.routing == "curved" && edge.waypoints.is_empty() && pts.len() == 4 {
        const STEPS: usize = 16;
        let mut prev = pts[0];
        for i in 1..=STEPS {
          let t = (i as f64) / (STEPS as f64);
          let cur = cubic_at(pts[0], pts[1], pts[2], pts[3], t);
          if dist_to_segment(world, prev, cur) <= tol {
            return Some(edge);
          }
          prev = cur;
        }
      } else {
        for seg in pts.windows(2) {
          if dist_to_segment(world, seg[0], seg[1]) <= tol {
            return Some(edge);
          }
        }
      }
    }
    None
  }
}

pub(super) fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
  (a[0] - b[0]).hypot(a[1] - b[1])
}

pub(super) fn cubic_at(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], t: f64) -> [f64; 2] {
  let mt = 1.0 - t;
  let w0 = mt * mt * mt;
  let w1 = 3.0 * mt * mt * t;
  let w2 = 3.0 * mt * t * t;
  let w3 = t * t * t;
  [
    p0[0] * w0 + p1[0] * w1 + p2[0] * w2 + p3[0] * w3,
    p0[1] * w0 + p1[1] * w1 + p2[1] * w2 + p3[1] * w3,
  ]
}

pub(super) fn dist_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
  let abx = b[0] - a[0];
  let aby = b[1] - a[1];
  let len2 = abx * abx + aby * aby;
  if len2 < 1e-4 {
    return dist(p, a);
  }
  let t = (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / len2).clamp(0.0, 1.0);
  let proj = [a[0] + abx * t, a[1] + aby * t];
  dist(p, proj)
}

pub(super) fn node_anchor_by_name(node: &BoardNode, side: &str) -> [f64; 2] {
  match Side::parse(side) {
    Some(s) => anchor_point(node, s),
    None => [node.x + node.w / 2.0, node.y + node.h / 2.0],
  }
}
