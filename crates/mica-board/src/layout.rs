//! Mind map variable-height tree layout and bidirectional Markdown outline conversion.

use std::collections::{HashMap, HashSet};

use crate::model::{
  BOARD_SCHEMA_VERSION, BoardEdge, BoardNode, BoardSnapshot, EdgeStyle, NodeStyle,
};

const HORIZONTAL_GAP: f64 = 68.0;
const VERTICAL_GAP: f64 = 18.0;

const BRANCH_PALETTE: &[(&str, &str)] = &[
  ("#E8F3FF", "#3370FF"),
  ("#EDF7ED", "#2DA44E"),
  ("#FFF7E6", "#D9822B"),
  ("#F5F0FF", "#8250DF"),
  ("#FFF0F0", "#CF222E"),
  ("#E6F7F7", "#0E8A8A"),
];

/// Re-layout every mind map tree (`mind_root` + descendants via `parent_id`)
/// cleanly without overlaps. Collapsed nodes hide their descendants' height.
pub fn layout_mindmap(snap: &mut BoardSnapshot) {
  let mut children: HashMap<String, Vec<usize>> = HashMap::new();
  let mut id_to_idx: HashMap<String, usize> = HashMap::new();
  for (i, n) in snap.nodes.iter().enumerate() {
    id_to_idx.insert(n.id.clone(), i);
  }
  for (i, n) in snap.nodes.iter().enumerate() {
    if let Some(pid) = &n.parent_id {
      if id_to_idx.contains_key(pid) && pid != &n.id {
        children.entry(pid.clone()).or_default().push(i);
      }
    }
  }
  for list in children.values_mut() {
    list.sort_by(|&a, &b| {
      snap.nodes[a]
        .rank
        .cmp(&snap.nodes[b].rank)
        .then_with(|| snap.nodes[a].id.cmp(&snap.nodes[b].id))
    });
  }

  let roots: Vec<usize> = snap
    .nodes
    .iter()
    .enumerate()
    .filter(|(_, n)| {
      n.kind == "mind_root"
        || (n.kind == "mind_node"
          && n
            .parent_id
            .as_ref()
            .is_none_or(|p| !id_to_idx.contains_key(p)))
    })
    .map(|(i, _)| i)
    .collect();

  let mut visited = HashSet::new();
  for root_idx in roots {
    if !visited.insert(root_idx) {
      continue;
    }
    let mut subtree_h: HashMap<usize, f64> = HashMap::new();
    compute_subtree_height(
      root_idx,
      &snap.nodes,
      &children,
      &mut subtree_h,
      &mut HashSet::new(),
    );
    let root_x = snap.nodes[root_idx].x;
    let root_center_y = snap.nodes[root_idx].y + snap.nodes[root_idx].h / 2.0;
    let total_h = subtree_h
      .get(&root_idx)
      .copied()
      .unwrap_or(snap.nodes[root_idx].h);
    place_subtree(
      root_idx,
      root_x,
      root_center_y - total_h / 2.0,
      &mut snap.nodes,
      &children,
      &subtree_h,
      &mut visited,
    );
  }
}

fn compute_subtree_height(
  idx: usize,
  nodes: &[BoardNode],
  children: &HashMap<String, Vec<usize>>,
  memo: &mut HashMap<usize, f64>,
  stack: &mut HashSet<usize>,
) -> f64 {
  if !stack.insert(idx) {
    return nodes[idx].h;
  }
  let node = &nodes[idx];
  let own_h = node.h.max(36.0);
  if node.collapsed {
    stack.remove(&idx);
    memo.insert(idx, own_h);
    return own_h;
  }
  let Some(kids) = children.get(&node.id) else {
    stack.remove(&idx);
    memo.insert(idx, own_h);
    return own_h;
  };
  if kids.is_empty() {
    stack.remove(&idx);
    memo.insert(idx, own_h);
    return own_h;
  }
  let mut sum = 0.0;
  for (i, &k) in kids.iter().enumerate() {
    if i > 0 {
      sum += VERTICAL_GAP;
    }
    sum += compute_subtree_height(k, nodes, children, memo, stack);
  }
  let h = own_h.max(sum);
  stack.remove(&idx);
  memo.insert(idx, h);
  h
}

fn place_subtree(
  idx: usize,
  x: f64,
  top_y: f64,
  nodes: &mut [BoardNode],
  children: &HashMap<String, Vec<usize>>,
  subtree_h: &HashMap<usize, f64>,
  visited: &mut HashSet<usize>,
) {
  let total_h = subtree_h.get(&idx).copied().unwrap_or(nodes[idx].h);
  let own_h = nodes[idx].h.max(36.0);
  let own_w = nodes[idx].w.max(80.0);
  let id = nodes[idx].id.clone();
  let collapsed = nodes[idx].collapsed;

  nodes[idx].x = x;
  nodes[idx].y = top_y + (total_h - own_h) / 2.0;

  if collapsed {
    return;
  }
  let Some(kids) = children.get(&id) else {
    return;
  };
  let kids_sum: f64 = kids
    .iter()
    .enumerate()
    .map(|(i, k)| {
      (if i > 0 { VERTICAL_GAP } else { 0.0 }) + subtree_h.get(k).copied().unwrap_or(nodes[*k].h)
    })
    .sum();
  let mut cur_y = top_y + (total_h - kids_sum) / 2.0;
  let child_x = x + own_w + HORIZONTAL_GAP;
  for &k in kids {
    if !visited.insert(k) {
      continue;
    }
    let kh = subtree_h.get(&k).copied().unwrap_or(nodes[k].h);
    place_subtree(k, child_x, cur_y, nodes, children, subtree_h, visited);
    cur_y += kh + VERTICAL_GAP;
  }
}

/// Convert a Markdown heading / bullet list outline into a laid-out mind map `BoardSnapshot`.
pub fn markdown_to_mindmap(markdown: &str, title: Option<&str>) -> BoardSnapshot {
  let mut items: Vec<(usize, String)> = Vec::new();
  let mut heading_base: usize = 0;

  for raw in markdown.lines() {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with("```") {
      continue;
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
      let hashes = 1 + rest.chars().take_while(|&c| c == '#').count();
      let text = trimmed[hashes..].trim();
      if !text.is_empty() {
        let level = hashes.saturating_sub(1);
        heading_base = level + 1;
        items.push((level, text.to_string()));
      }
      continue;
    }
    let indent_spaces: usize = raw
      .chars()
      .take_while(|&c| c == ' ' || c == '\t')
      .map(|c| if c == '\t' { 2 } else { 1 })
      .sum();
    let after = raw.trim_start();
    let bullet_text = after
      .strip_prefix("- ")
      .or_else(|| after.strip_prefix("* "))
      .or_else(|| after.strip_prefix("+ "));
    if let Some(text) = bullet_text {
      let clean = text.trim();
      if !clean.is_empty() {
        let level = heading_base + (indent_spaces / 2);
        items.push((level, clean.to_string()));
      }
    }
  }

  if items.is_empty() {
    let mut snap = BoardSnapshot::template_mindmap();
    if let Some(t) = title {
      if !t.trim().is_empty() {
        snap.title = t.trim().to_string();
        if let Some(root) = snap.nodes.first_mut() {
          root.text = t.trim().to_string();
        }
      }
    }
    return snap;
  }

  let min_level = items.iter().map(|(l, _)| *l).min().unwrap_or(0);
  let single_root = items.iter().filter(|(l, _)| *l == min_level).count() == 1;

  let mut nodes = Vec::new();
  let mut edges = Vec::new();
  let mut stack: Vec<(usize, String, usize)> = Vec::new(); // (depth, node_id, branch_color_idx)
  let mut rank_per_parent: HashMap<String, i32> = HashMap::new();

  let root_title = if single_root {
    items[0].1.clone()
  } else {
    title
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .unwrap_or("思维导图")
      .to_string()
  };

  let (rw, rh) = estimate_node_size(&root_title, true);
  nodes.push(BoardNode {
    id: "m0".into(),
    kind: "mind_root".into(),
    x: 80.0,
    y: 180.0,
    w: rw,
    h: rh,
    rot: 0.0,
    z: 1,
    parent_id: None,
    rank: 0,
    collapsed: false,
    text: root_title.clone(),
    icon: None,
    style: NodeStyle {
      fill: "#3370FF".into(),
      stroke: "#245BDB".into(),
      stroke_width: 1.8,
      font_size: 15.0,
      text_color: "#FFFFFF".into(),
      align: "center".into(),
      dashed: false,
    },
  });
  stack.push((0, "m0".into(), 0));

  let start_idx = if single_root { 1 } else { 0 };
  let mut top_branch_idx = 0usize;

  for (idx, (raw_level, text)) in items.into_iter().enumerate().skip(start_idx) {
    let depth = if single_root {
      (raw_level - min_level).max(1)
    } else {
      raw_level - min_level + 1
    };
    while stack.len() > 1 && stack.last().is_some_and(|(d, _, _)| *d >= depth) {
      stack.pop();
    }
    let (_, parent_id, parent_color) = stack.last().cloned().unwrap_or((0, "m0".into(), 0));
    let color_idx = if parent_id == "m0" {
      let c = top_branch_idx % BRANCH_PALETTE.len();
      top_branch_idx += 1;
      c
    } else {
      parent_color
    };
    let (fill, stroke) = BRANCH_PALETTE[color_idx];
    let node_id = format!("m{}", idx + 1);
    let rank = {
      let r = rank_per_parent.entry(parent_id.clone()).or_insert(0);
      let v = *r;
      *r += 1;
      v
    };
    let (w, h) = estimate_node_size(&text, false);
    nodes.push(BoardNode {
      id: node_id.clone(),
      kind: "mind_node".into(),
      x: 0.0,
      y: 0.0,
      w,
      h,
      rot: 0.0,
      z: (nodes.len() + 1) as i32,
      parent_id: Some(parent_id.clone()),
      rank,
      collapsed: false,
      text,
      icon: None,
      style: NodeStyle {
        fill: if depth == 1 {
          fill.into()
        } else {
          "#FFFFFF".into()
        },
        stroke: stroke.into(),
        stroke_width: 1.5,
        align: "left".into(),
        ..Default::default()
      },
    });
    edges.push(BoardEdge {
      id: format!("me{}", edges.len() + 1),
      from_id: parent_id,
      to_id: node_id.clone(),
      from_anchor: "r".into(),
      to_anchor: "l".into(),
      routing: "curved".into(),
      arrow_start: "none".into(),
      arrow_end: "none".into(),
      waypoints: vec![],
      label: String::new(),
      style: EdgeStyle {
        stroke: stroke.into(),
        stroke_width: 1.7,
        dashed: false,
      },
    });
    stack.push((depth, node_id, color_idx));
  }

  let mut snap = BoardSnapshot {
    version: BOARD_SCHEMA_VERSION,
    title: root_title,
    mode: "mindmap".into(),
    nodes,
    edges,
    strokes: vec![],
  };
  layout_mindmap(&mut snap);
  snap
}

/// Export a mind map (or any board) to a clean Markdown outline.
pub fn mindmap_to_markdown(snap: &BoardSnapshot) -> String {
  let mut children: HashMap<&str, Vec<&BoardNode>> = HashMap::new();
  let id_set: HashSet<&str> = snap.nodes.iter().map(|n| n.id.as_str()).collect();
  let mut roots: Vec<&BoardNode> = Vec::new();

  for node in &snap.nodes {
    if let Some(pid) = node.parent_id.as_deref() {
      if id_set.contains(pid) && pid != node.id {
        children.entry(pid).or_default().push(node);
        continue;
      }
    }
    roots.push(node);
  }
  for kids in children.values_mut() {
    kids.sort_by_key(|n| (n.rank, n.id.as_str()));
  }
  roots.sort_by(|a, b| {
    let a_root = a.kind == "mind_root";
    let b_root = b.kind == "mind_root";
    b_root
      .cmp(&a_root)
      .then_with(|| a.y.total_cmp(&b.y))
      .then_with(|| a.x.total_cmp(&b.x))
  });

  let mut out = String::new();
  let mut visited = HashSet::new();
  for root in roots {
    write_outline(root, 0, &children, &mut visited, &mut out);
  }
  out.trim_end().to_string()
}

fn write_outline<'a>(
  node: &'a BoardNode,
  depth: usize,
  children: &HashMap<&str, Vec<&'a BoardNode>>,
  visited: &mut HashSet<&'a str>,
  out: &mut String,
) {
  if !visited.insert(node.id.as_str()) {
    return;
  }
  let line = node.text.replace('\n', " ");
  let clean = line.trim();
  if !clean.is_empty() {
    if depth == 0 {
      out.push_str("# ");
      out.push_str(clean);
      out.push('\n');
    } else {
      for _ in 0..(depth - 1) {
        out.push_str("  ");
      }
      out.push_str("- ");
      out.push_str(clean);
      out.push('\n');
    }
  }
  if let Some(kids) = children.get(node.id.as_str()) {
    for kid in kids {
      write_outline(kid, depth + 1, children, visited, out);
    }
  }
}

fn estimate_node_size(text: &str, is_root: bool) -> (f64, f64) {
  let mut max_units: f64 = 0.0;
  let mut lines = 0usize;
  for line in text.lines() {
    lines += 1;
    let units: f64 = line
      .chars()
      .map(|c| if c.is_ascii() { 0.58 } else { 1.05 })
      .sum();
    if units > max_units {
      max_units = units;
    }
  }
  lines = lines.max(1);
  let font = if is_root { 15.0 } else { 13.5 };
  let w = (max_units * font + 34.0).clamp(if is_root { 140.0 } else { 112.0 }, 260.0);
  let wrapped_lines = ((max_units * font) / (w - 24.0)).ceil().max(1.0) as usize;
  let total_lines = lines.max(wrapped_lines);
  let h = (total_lines as f64 * (font * 1.4) + 20.0).max(if is_root { 48.0 } else { 40.0 });
  (w, h)
}
