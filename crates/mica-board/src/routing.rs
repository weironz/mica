//! Connector anchor selection and obstacle-avoiding orthogonal A* routing.
//! Preserves user-specified `manual_waypoints` while routing around obstacles.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::model::{BoardEdge, BoardNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
  Top,
  Right,
  Bottom,
  Left,
}

impl Side {
  pub fn parse(s: &str) -> Option<Self> {
    match s {
      "t" | "top" => Some(Self::Top),
      "r" | "right" => Some(Self::Right),
      "b" | "bottom" => Some(Self::Bottom),
      "l" | "left" => Some(Self::Left),
      _ => None,
    }
  }

  pub fn normal(self) -> (f64, f64) {
    match self {
      Self::Top => (0.0, -1.0),
      Self::Right => (1.0, 0.0),
      Self::Bottom => (0.0, 1.0),
      Self::Left => (-1.0, 0.0),
    }
  }
}

pub fn anchor_point(node: &BoardNode, side: Side) -> [f64; 2] {
  match side {
    Side::Top => [node.x + node.w / 2.0, node.y],
    Side::Right => [node.x + node.w, node.y + node.h / 2.0],
    Side::Bottom => [node.x + node.w / 2.0, node.y + node.h],
    Side::Left => [node.x, node.y + node.h / 2.0],
  }
}

pub fn pick_auto_sides(from: &BoardNode, to: &BoardNode) -> (Side, Side) {
  let fx = from.x + from.w / 2.0;
  let fy = from.y + from.h / 2.0;
  let tx = to.x + to.w / 2.0;
  let ty = to.y + to.h / 2.0;
  let dx = tx - fx;
  let dy = ty - fy;
  if dx.abs() >= dy.abs() {
    if dx >= 0.0 {
      (Side::Right, Side::Left)
    } else {
      (Side::Left, Side::Right)
    }
  } else if dy >= 0.0 {
    (Side::Bottom, Side::Top)
  } else {
    (Side::Top, Side::Bottom)
  }
}

pub fn resolve_endpoints(
  edge: &BoardEdge,
  from: &BoardNode,
  to: &BoardNode,
) -> ([f64; 2], Side, [f64; 2], Side) {
  let (auto_f, auto_t) = pick_auto_sides(from, to);
  let sf = Side::parse(&edge.from_anchor).unwrap_or(auto_f);
  let st = Side::parse(&edge.to_anchor).unwrap_or(auto_t);
  (anchor_point(from, sf), sf, anchor_point(to, st), st)
}

/// Compute polyline points for an edge.
/// - `straight`: `[start, ...waypoints, end]`
/// - `orthogonal`: respects `edge.waypoints` if present, otherwise runs an
///   obstacle-avoiding orthogonal A* router over a corridor grid built from node bounds.
/// - `curved`: returns `[start, cp1, cp2, end]` (caller draws cubic bezier).
pub fn route_edge(
  edge: &BoardEdge,
  nodes: &[BoardNode],
  node_map: &HashMap<&str, &BoardNode>,
) -> Vec<[f64; 2]> {
  let (Some(from), Some(to)) = (
    node_map.get(edge.from_id.as_str()),
    node_map.get(edge.to_id.as_str()),
  ) else {
    return Vec::new();
  };
  let (start, sf, end, st) = resolve_endpoints(edge, from, to);

  if edge.routing == "straight" {
    let mut pts = Vec::with_capacity(edge.waypoints.len() + 2);
    pts.push(start);
    pts.extend_from_slice(&edge.waypoints);
    pts.push(end);
    return pts;
  }

  if edge.routing == "curved" && edge.waypoints.is_empty() {
    let dist = ((end[0] - start[0]).hypot(end[1] - start[1]) * 0.42).clamp(28.0, 160.0);
    let nf = sf.normal();
    let nt = st.normal();
    return vec![
      start,
      [start[0] + nf.0 * dist, start[1] + nf.1 * dist],
      [end[0] + nt.0 * dist, end[1] + nt.1 * dist],
      end,
    ];
  }

  // Orthogonal routing: preserve manual waypoints if user dragged bends.
  if !edge.waypoints.is_empty() {
    let mut pts = Vec::with_capacity(edge.waypoints.len() + 4);
    pts.push(start);
    let mut prev = start;
    for &wp in &edge.waypoints {
      if (prev[0] - wp[0]).abs() > 0.5 && (prev[1] - wp[1]).abs() > 0.5 {
        pts.push([wp[0], prev[1]]);
      }
      pts.push(wp);
      prev = wp;
    }
    if (prev[0] - end[0]).abs() > 0.5 && (prev[1] - end[1]).abs() > 0.5 {
      pts.push([prev[0], end[1]]);
    }
    pts.push(end);
    return simplify_orthogonal(pts);
  }

  // Collect obstacles (non-frame nodes within bounding window, excluding from/to).
  let min_x = start[0].min(end[0]) - 100.0;
  let max_x = start[0].max(end[0]) + 100.0;
  let min_y = start[1].min(end[1]) - 100.0;
  let max_y = start[1].max(end[1]) + 100.0;

  let obstacles: Vec<[f64; 4]> = nodes
    .iter()
    .filter(|n| {
      n.id != from.id
        && n.id != to.id
        && n.kind != "frame"
        && n.x + n.w >= min_x
        && n.x <= max_x
        && n.y + n.h >= min_y
        && n.y <= max_y
    })
    .map(|n| [n.x - 4.0, n.y - 4.0, n.x + n.w + 4.0, n.y + n.h + 4.0])
    .collect();

  // Pick the largest exit/entry stub that stays clear of obstacles.
  let nf = sf.normal();
  let nt = st.normal();
  let pick_stub = |pt: [f64; 2], n: (f64, f64)| -> [f64; 2] {
    for m in [16.0, 10.0, 5.0, 2.0, 0.5] {
      let cand = [pt[0] + n.0 * m, pt[1] + n.1 * m];
      if !segment_hits_any(pt, cand, &obstacles) {
        return cand;
      }
    }
    [pt[0] + n.0 * 4.0, pt[1] + n.1 * 4.0]
  };
  let s_out = pick_stub(start, nf);
  let e_out = pick_stub(end, nt);

  // Fast path: clean 2-bend / Z-shape if no obstacle blocks it.
  if let Some(simple) = try_simple_orthogonal(start, s_out, e_out, end, sf, st, &obstacles) {
    return simplify_orthogonal(simple);
  }

  // Corridor grid coordinates from endpoints + obstacle bounds.
  let mut xs = vec![
    s_out[0],
    e_out[0],
    (s_out[0] + e_out[0]) / 2.0,
    from.x - 16.0,
    from.x + from.w + 16.0,
    to.x - 16.0,
    to.x + to.w + 16.0,
  ];
  let mut ys = vec![
    s_out[1],
    e_out[1],
    (s_out[1] + e_out[1]) / 2.0,
    from.y - 16.0,
    from.y + from.h + 16.0,
    to.y - 16.0,
    to.y + to.h + 16.0,
  ];
  for o in obstacles.iter().take(24) {
    xs.push(o[0] - 8.0);
    xs.push(o[2] + 8.0);
    ys.push(o[1] - 8.0);
    ys.push(o[3] + 8.0);
  }
  dedup_sorted(&mut xs);
  dedup_sorted(&mut ys);

  let Some(si) = xs.iter().position(|&v| (v - s_out[0]).abs() < 1e-3) else {
    return simplify_orthogonal(vec![start, s_out, [s_out[0], e_out[1]], e_out, end]);
  };
  let Some(sj) = ys.iter().position(|&v| (v - s_out[1]).abs() < 1e-3) else {
    return simplify_orthogonal(vec![start, s_out, [s_out[0], e_out[1]], e_out, end]);
  };
  let Some(ei) = xs.iter().position(|&v| (v - e_out[0]).abs() < 1e-3) else {
    return simplify_orthogonal(vec![start, s_out, [s_out[0], e_out[1]], e_out, end]);
  };
  let Some(ej) = ys.iter().position(|&v| (v - e_out[1]).abs() < 1e-3) else {
    return simplify_orthogonal(vec![start, s_out, [s_out[0], e_out[1]], e_out, end]);
  };

  if let Some(mid) = astar_corridor(&xs, &ys, (si, sj), (ei, ej), &obstacles) {
    let mut pts = Vec::with_capacity(mid.len() + 2);
    pts.push(start);
    pts.extend(mid);
    pts.push(end);
    return simplify_orthogonal(pts);
  }

  simplify_orthogonal(vec![start, s_out, [s_out[0], e_out[1]], e_out, end])
}

fn try_simple_orthogonal(
  start: [f64; 2],
  s_out: [f64; 2],
  e_out: [f64; 2],
  end: [f64; 2],
  sf: Side,
  st: Side,
  obstacles: &[[f64; 4]],
) -> Option<Vec<[f64; 2]>> {
  let horiz_start = matches!(sf, Side::Left | Side::Right);
  let horiz_end = matches!(st, Side::Left | Side::Right);
  let candidates: Vec<Vec<[f64; 2]>> = if horiz_start && horiz_end {
    let mx = (s_out[0] + e_out[0]) / 2.0;
    vec![vec![
      start,
      s_out,
      [mx, s_out[1]],
      [mx, e_out[1]],
      e_out,
      end,
    ]]
  } else if !horiz_start && !horiz_end {
    let my = (s_out[1] + e_out[1]) / 2.0;
    vec![vec![
      start,
      s_out,
      [s_out[0], my],
      [e_out[0], my],
      e_out,
      end,
    ]]
  } else if horiz_start {
    vec![vec![start, s_out, [e_out[0], s_out[1]], e_out, end]]
  } else {
    vec![vec![start, s_out, [s_out[0], e_out[1]], e_out, end]]
  };

  for cand in candidates {
    let clear = cand
      .windows(2)
      .all(|w| !segment_hits_any(w[0], w[1], obstacles));
    if clear {
      return Some(cand);
    }
  }
  None
}

#[derive(Copy, Clone)]
struct State {
  f: f64,
  g: f64,
  i: usize,
  j: usize,
  dir: u8, // 0=none, 1=horiz, 2=vert
}

impl PartialEq for State {
  fn eq(&self, other: &Self) -> bool {
    self.f.to_bits() == other.f.to_bits()
  }
}
impl Eq for State {}
impl PartialOrd for State {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    Some(self.cmp(other))
  }
}
impl Ord for State {
  fn cmp(&self, other: &Self) -> Ordering {
    other.f.total_cmp(&self.f)
  }
}

fn astar_corridor(
  xs: &[f64],
  ys: &[f64],
  start: (usize, usize),
  goal: (usize, usize),
  obstacles: &[[f64; 4]],
) -> Option<Vec<[f64; 2]>> {
  let mut heap = BinaryHeap::new();
  let mut best: HashMap<(usize, usize, u8), f64> = HashMap::new();
  let mut prev: HashMap<(usize, usize, u8), (usize, usize, u8)> = HashMap::new();

  let h0 = (xs[start.0] - xs[goal.0]).abs() + (ys[start.1] - ys[goal.1]).abs();
  heap.push(State {
    f: h0,
    g: 0.0,
    i: start.0,
    j: start.1,
    dir: 0,
  });
  best.insert((start.0, start.1, 0), 0.0);

  let mut expansions = 0usize;
  let mut found: Option<(usize, usize, u8)> = None;

  while let Some(cur) = heap.pop() {
    if (cur.i, cur.j) == goal {
      found = Some((cur.i, cur.j, cur.dir));
      break;
    }
    expansions += 1;
    if expansions > 1600 {
      break;
    }
    if cur.g
      > best
        .get(&(cur.i, cur.j, cur.dir))
        .copied()
        .unwrap_or(f64::INFINITY)
        + 1e-6
    {
      continue;
    }
    let p0 = [xs[cur.i], ys[cur.j]];
    let mut neighbors = Vec::with_capacity(4);
    if cur.i > 0 {
      neighbors.push((cur.i - 1, cur.j, 1u8));
    }
    if cur.i + 1 < xs.len() {
      neighbors.push((cur.i + 1, cur.j, 1u8));
    }
    if cur.j > 0 {
      neighbors.push((cur.i, cur.j - 1, 2u8));
    }
    if cur.j + 1 < ys.len() {
      neighbors.push((cur.i, cur.j + 1, 2u8));
    }

    for (ni, nj, ndir) in neighbors {
      let p1 = [xs[ni], ys[nj]];
      if segment_hits_any(p0, p1, obstacles) {
        continue;
      }
      let seg_len = (p1[0] - p0[0]).abs() + (p1[1] - p0[1]).abs();
      let bend_penalty = if cur.dir != 0 && cur.dir != ndir {
        24.0
      } else {
        0.0
      };
      let ng = cur.g + seg_len + bend_penalty;
      let key = (ni, nj, ndir);
      if ng < best.get(&key).copied().unwrap_or(f64::INFINITY) - 1e-6 {
        best.insert(key, ng);
        prev.insert(key, (cur.i, cur.j, cur.dir));
        let h = (xs[ni] - xs[goal.0]).abs() + (ys[nj] - ys[goal.1]).abs();
        heap.push(State {
          f: ng + h,
          g: ng,
          i: ni,
          j: nj,
          dir: ndir,
        });
      }
    }
  }

  let mut cur = found?;
  let mut pts = Vec::new();
  loop {
    pts.push([xs[cur.0], ys[cur.1]]);
    if let Some(&p) = prev.get(&cur) {
      cur = p;
    } else {
      break;
    }
  }
  pts.reverse();
  Some(pts)
}

fn segment_hits_any(a: [f64; 2], b: [f64; 2], obstacles: &[[f64; 4]]) -> bool {
  let sx0 = a[0].min(b[0]);
  let sx1 = a[0].max(b[0]);
  let sy0 = a[1].min(b[1]);
  let sy1 = a[1].max(b[1]);
  for o in obstacles {
    if sx1 > o[0] && sx0 < o[2] && sy1 > o[1] && sy0 < o[3] {
      return true;
    }
  }
  false
}

fn dedup_sorted(vals: &mut Vec<f64>) {
  vals.sort_by(f64::total_cmp);
  let mut out: Vec<f64> = Vec::with_capacity(vals.len());
  for v in vals.drain(..) {
    if out.last().is_none_or(|&prev| (v - prev).abs() > 1.0) {
      out.push(v);
    }
  }
  *vals = out;
}

fn simplify_orthogonal(pts: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
  let mut out: Vec<[f64; 2]> = Vec::with_capacity(pts.len());
  for p in pts {
    if out
      .last()
      .is_some_and(|l| (l[0] - p[0]).abs() < 0.5 && (l[1] - p[1]).abs() < 0.5)
    {
      continue;
    }
    if out.len() >= 2 {
      let a = out[out.len() - 2];
      let b = out[out.len() - 1];
      let colinear_x = (a[0] - b[0]).abs() < 0.5 && (b[0] - p[0]).abs() < 0.5;
      let colinear_y = (a[1] - b[1]).abs() < 0.5 && (b[1] - p[1]).abs() < 0.5;
      if colinear_x || colinear_y {
        out.pop();
      }
    }
    out.push(p);
  }
  out
}
