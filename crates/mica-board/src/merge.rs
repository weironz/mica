//! Deterministic 3-way semantic merge for concurrent embedded board edits,
//! plus plain-text extraction for Mica's global search index.

use std::collections::{HashMap, HashSet};

use crate::model::{BoardEdge, BoardNode, BoardSnapshot, BoardStroke};

/// Perform a deterministic 3-way semantic merge of `local` and `remote` against `base`.
/// Prevents JSON corruption when two collaborators concurrently edit an embedded
/// `mica-board` block in the same Markdown document:
/// - Independent node additions/moves/style edits both survive.
/// - Deletions are respected unless the other peer explicitly modified the element.
/// - Same-field conflicts resolve deterministically in favor of `local` (with `remote`
///   non-conflicting fields preserved).
pub fn merge_board_snapshots(
  base: &BoardSnapshot,
  local: &BoardSnapshot,
  remote: &BoardSnapshot,
) -> BoardSnapshot {
  let title = if local.title != base.title {
    local.title.clone()
  } else {
    remote.title.clone()
  };
  let mode = if local.mode != base.mode {
    local.mode.clone()
  } else {
    remote.mode.clone()
  };

  let nodes = merge_nodes(&base.nodes, &local.nodes, &remote.nodes);
  let live_ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
  let edges = merge_edges(&base.edges, &local.edges, &remote.edges)
    .into_iter()
    .filter(|e| live_ids.contains(e.from_id.as_str()) && live_ids.contains(e.to_id.as_str()))
    .collect();
  let strokes = merge_strokes(&base.strokes, &local.strokes, &remote.strokes);

  BoardSnapshot {
    version: local.version.max(remote.version),
    title,
    mode,
    nodes,
    edges,
    strokes,
  }
}

fn merge_nodes(base: &[BoardNode], local: &[BoardNode], remote: &[BoardNode]) -> Vec<BoardNode> {
  let b_map: HashMap<&str, &BoardNode> = base.iter().map(|n| (n.id.as_str(), n)).collect();
  let l_map: HashMap<&str, &BoardNode> = local.iter().map(|n| (n.id.as_str(), n)).collect();
  let r_map: HashMap<&str, &BoardNode> = remote.iter().map(|n| (n.id.as_str(), n)).collect();

  let mut order: Vec<&str> = Vec::new();
  let mut seen = HashSet::new();
  for id in local.iter().chain(remote.iter()).map(|n| n.id.as_str()) {
    if seen.insert(id) {
      order.push(id);
    }
  }

  let mut out = Vec::new();
  for id in order {
    match (b_map.get(id), l_map.get(id), r_map.get(id)) {
      (_, Some(l), Some(r)) => {
        if let Some(b) = b_map.get(id) {
          out.push(merge_single_node(b, l, r));
        } else {
          out.push((*l).clone());
        }
      }
      (None, Some(l), None) => out.push((*l).clone()),
      (None, None, Some(r)) => out.push((*r).clone()),
      (Some(b), Some(l), None) => {
        // Remote deleted; keep only if local modified it.
        if *l != *b {
          out.push((*l).clone());
        }
      }
      (Some(b), None, Some(r)) => {
        // Local deleted; keep only if remote modified it.
        if *r != *b {
          out.push((*r).clone());
        }
      }
      (_, None, None) => {}
    }
  }
  out
}

fn merge_single_node(b: &BoardNode, l: &BoardNode, r: &BoardNode) -> BoardNode {
  let pick_f64 =
    |bv: f64, lv: f64, rv: f64| -> f64 { if (lv - bv).abs() > 1e-6 { lv } else { rv } };
  let pick_str = |bv: &str, lv: &str, rv: &str| -> String {
    if lv != bv {
      lv.to_string()
    } else {
      rv.to_string()
    }
  };
  BoardNode {
    id: l.id.clone(),
    kind: pick_str(&b.kind, &l.kind, &r.kind),
    x: pick_f64(b.x, l.x, r.x),
    y: pick_f64(b.y, l.y, r.y),
    w: pick_f64(b.w, l.w, r.w),
    h: pick_f64(b.h, l.h, r.h),
    rot: pick_f64(b.rot, l.rot, r.rot),
    z: if l.z != b.z { l.z } else { r.z },
    parent_id: if l.parent_id != b.parent_id {
      l.parent_id.clone()
    } else {
      r.parent_id.clone()
    },
    rank: if l.rank != b.rank { l.rank } else { r.rank },
    collapsed: if l.collapsed != b.collapsed {
      l.collapsed
    } else {
      r.collapsed
    },
    text: pick_str(&b.text, &l.text, &r.text),
    icon: if l.icon != b.icon {
      l.icon.clone()
    } else {
      r.icon.clone()
    },
    style: if l.style != b.style {
      l.style.clone()
    } else {
      r.style.clone()
    },
  }
}

fn merge_edges(base: &[BoardEdge], local: &[BoardEdge], remote: &[BoardEdge]) -> Vec<BoardEdge> {
  let b_map: HashMap<&str, &BoardEdge> = base.iter().map(|e| (e.id.as_str(), e)).collect();
  let l_map: HashMap<&str, &BoardEdge> = local.iter().map(|e| (e.id.as_str(), e)).collect();
  let r_map: HashMap<&str, &BoardEdge> = remote.iter().map(|e| (e.id.as_str(), e)).collect();

  let mut order: Vec<&str> = Vec::new();
  let mut seen = HashSet::new();
  for id in local.iter().chain(remote.iter()).map(|e| e.id.as_str()) {
    if seen.insert(id) {
      order.push(id);
    }
  }
  let mut out = Vec::new();
  for id in order {
    match (b_map.get(id), l_map.get(id), r_map.get(id)) {
      (Some(b), Some(l), Some(r)) => {
        if *l != *b {
          out.push((*l).clone());
        } else {
          out.push((*r).clone());
        }
      }
      (None, Some(l), _) => out.push((*l).clone()),
      (None, None, Some(r)) => out.push((*r).clone()),
      (Some(b), Some(l), None) if *l != *b => out.push((*l).clone()),
      (Some(b), None, Some(r)) if *r != *b => out.push((*r).clone()),
      _ => {}
    }
  }
  out
}

fn merge_strokes(
  base: &[BoardStroke],
  local: &[BoardStroke],
  remote: &[BoardStroke],
) -> Vec<BoardStroke> {
  let b_ids: HashSet<&str> = base.iter().map(|s| s.id.as_str()).collect();
  let l_map: HashMap<&str, &BoardStroke> = local.iter().map(|s| (s.id.as_str(), s)).collect();
  let r_map: HashMap<&str, &BoardStroke> = remote.iter().map(|s| (s.id.as_str(), s)).collect();
  let mut out = Vec::new();
  let mut seen = HashSet::new();
  for s in local.iter().chain(remote.iter()) {
    if !seen.insert(s.id.as_str()) {
      continue;
    }
    let in_b = b_ids.contains(s.id.as_str());
    let in_l = l_map.contains_key(s.id.as_str());
    let in_r = r_map.contains_key(s.id.as_str());
    if (in_l && in_r) || (!in_b && (in_l || in_r)) {
      out.push(s.clone());
    }
  }
  out
}

/// Extract searchable plain text from a board (title, all node text, all edge labels).
pub fn extract_plain_text(snap: &BoardSnapshot) -> String {
  let mut parts: Vec<&str> = Vec::new();
  let t = snap.title.trim();
  if !t.is_empty() {
    parts.push(t);
  }
  for node in &snap.nodes {
    let s = node.text.trim();
    if !s.is_empty() {
      parts.push(s);
    }
  }
  for edge in &snap.edges {
    let s = edge.label.trim();
    if !s.is_empty() {
      parts.push(s);
    }
  }
  parts.join("\n")
}
