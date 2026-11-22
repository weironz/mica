//! Headless deterministic SVG renderer for `BoardSnapshot`.
//! Produces standalone, `flutter_svg`- and `resvg`-safe SVG (inline attributes,
//! explicit `<polygon>` arrowheads, no `<foreignObject>`, no CSS selectors).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::model::{BoardEdge, BoardNode, BoardSnapshot, BoardSvgTheme, tech_icon_svg_inner};
use crate::routing::route_edge;

const PAD: f64 = 28.0;

pub fn render_board_svg(snap: &BoardSnapshot, theme: Option<&BoardSvgTheme>) -> String {
  let default_theme = BoardSvgTheme::default();
  let t = theme.unwrap_or(&default_theme);

  let hidden = collapsed_descendants(snap);
  let visible_nodes: Vec<&BoardNode> = snap
    .nodes
    .iter()
    .filter(|n| !hidden.contains(n.id.as_str()))
    .collect();

  if visible_nodes.is_empty() && snap.strokes.is_empty() {
    return format!(
      r#"<svg xmlns="http://www.w3.org/2000/svg" width="420" height="160" viewBox="0 0 420 160"><rect width="420" height="160" rx="10" fill="{}" stroke="{}" stroke-width="1.2"/><text x="210" y="85" text-anchor="middle" font-size="14" font-family="sans-serif" fill="{}">{}</text></svg>"#,
      esc(&t.canvas),
      esc(&t.border),
      esc(&t.subtle_text),
      esc(if snap.title.is_empty() {
        "空白画板 · 点击开始绘制"
      } else {
        &snap.title
      }),
    );
  }

  let mut min_x = f64::INFINITY;
  let mut min_y = f64::INFINITY;
  let mut max_x = f64::NEG_INFINITY;
  let mut max_y = f64::NEG_INFINITY;

  for n in &visible_nodes {
    min_x = min_x.min(n.x);
    min_y = min_y.min(n.y);
    max_x = max_x.max(n.x + n.w);
    max_y = max_y.max(n.y + n.h);
  }
  for s in &snap.strokes {
    for p in &s.points {
      min_x = min_x.min(p[0]);
      min_y = min_y.min(p[1]);
      max_x = max_x.max(p[0]);
      max_y = max_y.max(p[1]);
    }
  }

  let vx = (min_x - PAD).floor();
  let vy = (min_y - PAD).floor();
  let vw = ((max_x - min_x) + PAD * 2.0).max(240.0).ceil();
  let vh = ((max_y - min_y) + PAD * 2.0).max(140.0).ceil();

  let mut sorted_nodes = visible_nodes.clone();
  sorted_nodes.sort_by_key(|n| (n.kind != "frame", n.z));

  let node_map: HashMap<&str, &BoardNode> =
    visible_nodes.iter().map(|n| (n.id.as_str(), *n)).collect();

  let mut out = String::with_capacity(4096);
  let _ = write!(
    out,
    r#"<svg xmlns="http://www.w3.org/2000/svg" width="{vw}" height="{vh}" viewBox="{vx} {vy} {vw} {vh}">"#
  );
  let _ = write!(
    out,
    r#"<rect x="{vx}" y="{vy}" width="{vw}" height="{vh}" fill="{}"/>"#,
    esc(&t.canvas)
  );

  // 1. Frames first
  for n in sorted_nodes.iter().filter(|n| n.kind == "frame") {
    draw_node(&mut out, n, t);
  }

  // 2. Edges
  for edge in &snap.edges {
    if !node_map.contains_key(edge.from_id.as_str()) || !node_map.contains_key(edge.to_id.as_str())
    {
      continue;
    }
    draw_edge(&mut out, edge, &snap.nodes, &node_map, t);
  }

  // 3. Regular nodes
  for n in sorted_nodes.iter().filter(|n| n.kind != "frame") {
    draw_node(&mut out, n, t);
  }

  // 4. Freehand strokes
  for stroke in &snap.strokes {
    if stroke.points.len() < 2 {
      continue;
    }
    let mut d = String::new();
    let _ = write!(d, "M{:.1},{:.1}", stroke.points[0][0], stroke.points[0][1]);
    for p in &stroke.points[1..] {
      let _ = write!(d, " L{:.1},{:.1}", p[0], p[1]);
    }
    let _ = write!(
      out,
      r#"<path d="{d}" fill="none" stroke="{}" stroke-width="{:.1}" stroke-linecap="round" stroke-linejoin="round"/>"#,
      esc(&stroke.color),
      stroke.width.max(1.0)
    );
  }

  out.push_str("</svg>");
  out
}

fn collapsed_descendants(snap: &BoardSnapshot) -> HashSet<&str> {
  let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
  for n in &snap.nodes {
    if let Some(pid) = n.parent_id.as_deref() {
      children.entry(pid).or_default().push(n.id.as_str());
    }
  }
  let mut hidden = HashSet::new();
  let mut stack: Vec<&str> = Vec::new();
  for n in &snap.nodes {
    if n.collapsed {
      if let Some(kids) = children.get(n.id.as_str()) {
        stack.extend(kids.iter().copied());
      }
    }
  }
  while let Some(id) = stack.pop() {
    if hidden.insert(id) {
      if let Some(kids) = children.get(id) {
        stack.extend(kids.iter().copied());
      }
    }
  }
  hidden
}

fn adapt_fill<'a>(raw: &'a str, t: &'a BoardSvgTheme) -> &'a str {
  if raw == "transparent" || raw == "none" {
    return "none";
  }
  if t.dark && raw.eq_ignore_ascii_case("#FFFFFF") {
    return &t.surface;
  }
  raw
}

fn adapt_text_color<'a>(raw: &'a str, fill: &str, t: &'a BoardSvgTheme) -> &'a str {
  if t.dark && raw.eq_ignore_ascii_case("#1F2329") && (fill == t.surface || fill == "none") {
    return &t.text;
  }
  raw
}

fn draw_node(out: &mut String, n: &BoardNode, t: &BoardSvgTheme) {
  let fill = adapt_fill(&n.style.fill, t);
  let stroke = if n.style.stroke == "transparent" || n.style.stroke == "none" {
    "none"
  } else {
    &n.style.stroke
  };
  let sw = n.style.stroke_width.max(0.0);
  let dash = if n.style.dashed {
    r#" stroke-dasharray="6,4""#
  } else {
    ""
  };
  let (x, y, w, h) = (n.x, n.y, n.w, n.h);
  let cx = x + w / 2.0;
  let cy = y + h / 2.0;

  if n.rot.abs() > 0.1 {
    let _ = write!(
      out,
      r#"<g transform="rotate({:.1} {cx:.1} {cy:.1})">"#,
      n.rot
    );
  }

  match n.kind.as_str() {
    "rect" => {
      let _ = write!(
        out,
        r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#
      );
    }
    "pill" => {
      let rx = (h / 2.0).min(w / 2.0);
      let _ = write!(
        out,
        r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="{rx:.1}" ry="{rx:.1}" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#
      );
    }
    "ellipse" => {
      let _ = write!(
        out,
        r#"<ellipse cx="{cx:.1}" cy="{cy:.1}" rx="{:.1}" ry="{:.1}" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#,
        w / 2.0,
        h / 2.0
      );
    }
    "diamond" => {
      let _ = write!(
        out,
        r#"<polygon points="{cx:.1},{y:.1} {:.1},{cy:.1} {cx:.1},{:.1} {x:.1},{cy:.1}" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#,
        x + w,
        y + h
      );
    }
    "parallelogram" => {
      let sk = (w * 0.14).min(24.0);
      let _ = write!(
        out,
        r#"<polygon points="{:.1},{y:.1} {:.1},{y:.1} {:.1},{:.1} {x:.1},{:.1}" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#,
        x + sk,
        x + w,
        x + w - sk,
        y + h,
        y + h
      );
    }
    "hexagon" => {
      let sk = (w * 0.16).min(24.0);
      let _ = write!(
        out,
        r#"<polygon points="{:.1},{y:.1} {:.1},{y:.1} {:.1},{cy:.1} {:.1},{:.1} {:.1},{:.1} {x:.1},{cy:.1}" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#,
        x + sk,
        x + w - sk,
        x + w,
        x + w - sk,
        y + h,
        x + sk,
        y + h
      );
    }
    "triangle" => {
      let _ = write!(
        out,
        r#"<polygon points="{cx:.1},{y:.1} {:.1},{:.1} {x:.1},{:.1}" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#,
        x + w,
        y + h,
        y + h
      );
    }
    "cylinder" => {
      let ry = (h * 0.16).clamp(6.0, 14.0);
      let top_y = y + ry;
      let bot_y = y + h - ry;
      let rx = w / 2.0;
      let _ = write!(
        out,
        r#"<path d="M{x:.1},{top_y:.1} A{rx:.1},{ry:.1} 0 0 1 {:.1},{top_y:.1} L{:.1},{bot_y:.1} A{rx:.1},{ry:.1} 0 0 1 {x:.1},{bot_y:.1} Z" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#,
        x + w,
        x + w
      );
      let _ = write!(
        out,
        r#"<ellipse cx="{cx:.1}" cy="{top_y:.1}" rx="{rx:.1}" ry="{ry:.1}" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"/>"#
      );
    }
    "document" => {
      let wave = (h * 0.14).clamp(6.0, 14.0);
      let by = y + h - wave;
      let _ = write!(
        out,
        r#"<path d="M{x:.1},{y:.1} L{:.1},{y:.1} L{:.1},{by:.1} Q{:.1},{:.1} {cx:.1},{by:.1} Q{:.1},{:.1} {x:.1},{by:.1} Z" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#,
        x + w,
        x + w,
        x + w * 0.75,
        by - wave,
        x + w * 0.25,
        by + wave
      );
    }
    "cloud" => {
      let _ = write!(
        out,
        r#"<path d="M{:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} Z" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#,
        x + w * 0.25,
        y + h * 0.82,
        x + w * 0.04,
        y + h * 0.82,
        x + w * 0.04,
        y + h * 0.45,
        x + w * 0.22,
        y + h * 0.42,
        x + w * 0.24,
        y + h * 0.12,
        x + w * 0.58,
        y + h * 0.10,
        x + w * 0.68,
        y + h * 0.30,
        x + w * 0.96,
        y + h * 0.30,
        x + w * 0.98,
        y + h * 0.82,
        x + w * 0.78,
        y + h * 0.82,
        x + w * 0.60,
        y + h * 0.86,
        x + w * 0.40,
        y + h * 0.86,
        x + w * 0.25,
        y + h * 0.82
      );
    }
    "sticky" => {
      let fold = 14.0;
      let _ = write!(
        out,
        r#"<path d="M{x:.1},{y:.1} L{:.1},{y:.1} L{:.1},{:.1} L{:.1},{:.1} L{x:.1},{:.1} Z" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"/>"#,
        x + w,
        x + w,
        y + h - fold,
        x + w - fold,
        y + h,
        y + h
      );
      let _ = write!(
        out,
        r#"<path d="M{:.1},{:.1} L{:.1},{:.1} L{:.1},{:.1}" fill="none" stroke="{stroke}" stroke-width="1"/>"#,
        x + w - fold,
        y + h,
        x + w - fold,
        y + h - fold,
        x + w,
        y + h - fold
      );
    }
    "text" => {
      if fill != "none" || stroke != "none" {
        let _ = write!(
          out,
          r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="4" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#
        );
      }
    }
    "frame" => {
      let _ = write!(
        out,
        r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="10" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#
      );
      let tc = adapt_text_color(&n.style.text_color, fill, t);
      if !n.text.trim().is_empty() {
        let _ = write!(
          out,
          r#"<text x="{:.1}" y="{:.1}" font-size="12.5" font-weight="600" font-family="sans-serif" fill="{tc}">{}</text>"#,
          x + 12.0,
          y + 20.0,
          esc(n.text.lines().next().unwrap_or(""))
        );
      }
      if n.rot.abs() > 0.1 {
        out.push_str("</g>");
      }
      return;
    }
    _ => {
      // rounded, mind_root, mind_node, icon_card
      let rx = if n.kind == "mind_root" { 10.0 } else { 8.0 };
      let _ = write!(
        out,
        r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="{rx}" ry="{rx}" fill="{fill}" stroke="{stroke}" stroke-width="{sw:.1}"{dash}/>"#
      );
    }
  }

  // Optional tech architecture icon glyph
  let mut text_offset_y = 0.0;
  if let Some(icon_name) = n.icon.as_deref() {
    if let Some(inner) = tech_icon_svg_inner(icon_name) {
      let icon_size = 24.0;
      let ix = cx - icon_size / 2.0;
      let iy = y + 10.0;
      let icon_stroke = if stroke == "none" { &t.accent } else { stroke };
      let _ = write!(
        out,
        r#"<g transform="translate({ix:.1},{iy:.1})" fill="none" stroke="{icon_stroke}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{inner}</g>"#
      );
      text_offset_y = 14.0;
    }
  }

  // Node text lines
  if !n.text.is_empty() {
    let fs = n.style.font_size.clamp(9.0, 32.0);
    let tc = adapt_text_color(&n.style.text_color, fill, t);
    let max_text_w = (w - 20.0).max(24.0);
    let lines = wrap_lines(&n.text, max_text_w, fs);
    let line_h = fs * 1.38;
    let total_h = lines.len() as f64 * line_h;
    let start_y = cy + text_offset_y - total_h / 2.0 + fs * 0.85;
    let (anchor, tx) = match n.style.align.as_str() {
      "left" => ("start", x + 10.0),
      "right" => ("end", x + w - 10.0),
      _ => ("middle", cx),
    };
    let weight = if n.kind == "mind_root" {
      r#" font-weight="600""#
    } else {
      ""
    };
    for (i, line) in lines.iter().enumerate() {
      let ly = start_y + i as f64 * line_h;
      let _ = write!(
        out,
        r#"<text x="{tx:.1}" y="{ly:.1}" text-anchor="{anchor}" font-size="{fs:.1}"{weight} font-family="sans-serif" fill="{tc}">{}</text>"#,
        esc(line)
      );
    }
  }

  if n.rot.abs() > 0.1 {
    out.push_str("</g>");
  }
}

fn draw_edge(
  out: &mut String,
  edge: &BoardEdge,
  all_nodes: &[BoardNode],
  node_map: &HashMap<&str, &BoardNode>,
  t: &BoardSvgTheme,
) {
  let pts = route_edge(edge, all_nodes, node_map);
  if pts.len() < 2 {
    return;
  }
  let stroke = &edge.style.stroke;
  let sw = edge.style.stroke_width.max(1.0);
  let dash = if edge.style.dashed {
    r#" stroke-dasharray="6,4""#
  } else {
    ""
  };

  let (d, mid, start_dir, end_dir) = if edge.routing == "curved" && pts.len() == 4 {
    let d = format!(
      "M{:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
      pts[0][0], pts[0][1], pts[1][0], pts[1][1], pts[2][0], pts[2][1], pts[3][0], pts[3][1]
    );
    let mid = [
      0.125 * pts[0][0] + 0.375 * pts[1][0] + 0.375 * pts[2][0] + 0.125 * pts[3][0],
      0.125 * pts[0][1] + 0.375 * pts[1][1] + 0.375 * pts[2][1] + 0.125 * pts[3][1],
    ];
    let sd = [pts[0][0] - pts[1][0], pts[0][1] - pts[1][1]];
    let ed = [pts[3][0] - pts[2][0], pts[3][1] - pts[2][1]];
    (d, mid, sd, ed)
  } else {
    let d = if edge.routing == "orthogonal" && pts.len() >= 3 {
      build_rounded_orthogonal_svg_path(&pts, 10.0)
    } else {
      let mut s = String::new();
      let _ = write!(s, "M{:.1},{:.1}", pts[0][0], pts[0][1]);
      for p in &pts[1..] {
        let _ = write!(s, " L{:.1},{:.1}", p[0], p[1]);
      }
      s
    };
    let mid = polyline_midpoint(&pts);
    let sd = [pts[0][0] - pts[1][0], pts[0][1] - pts[1][1]];
    let n = pts.len();
    let ed = [pts[n - 1][0] - pts[n - 2][0], pts[n - 1][1] - pts[n - 2][1]];
    (d, mid, sd, ed)
  };

  let _ = write!(
    out,
    r#"<path d="{d}" fill="none" stroke="{stroke}" stroke-width="{sw:.1}" stroke-linecap="round" stroke-linejoin="round"{dash}/>"#
  );

  if edge.arrow_end == "arrow" {
    draw_arrow(out, *pts.last().unwrap(), end_dir, stroke);
  }
  if edge.arrow_start == "arrow" {
    draw_arrow(out, pts[0], start_dir, stroke);
  }

  if !edge.label.trim().is_empty() {
    let lbl = edge.label.trim();
    let est_w = (lbl
      .chars()
      .map(|c| if c.is_ascii() { 7.0 } else { 13.0 })
      .sum::<f64>()
      + 14.0)
      .max(28.0);
    let _ = write!(
      out,
      r#"<rect x="{:.1}" y="{:.1}" width="{est_w:.1}" height="20" rx="4" fill="{}" stroke="{stroke}" stroke-width="0.8"/>"#,
      mid[0] - est_w / 2.0,
      mid[1] - 10.0,
      esc(&t.surface)
    );
    let _ = write!(
      out,
      r#"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="11.5" font-family="sans-serif" fill="{}">{}</text>"#,
      mid[0],
      mid[1] + 4.0,
      esc(&t.text),
      esc(lbl)
    );
  }
}

fn build_rounded_orthogonal_svg_path(pts: &[[f64; 2]], radius: f64) -> String {
  let mut d = String::new();
  let _ = write!(d, "M{:.1},{:.1}", pts[0][0], pts[0][1]);
  for i in 1..pts.len() - 1 {
    let prev = pts[i - 1];
    let curr = pts[i];
    let next = pts[i + 1];
    let v1 = [prev[0] - curr[0], prev[1] - curr[1]];
    let v2 = [next[0] - curr[0], next[1] - curr[1]];
    let d1 = v1[0].hypot(v1[1]);
    let d2 = v2[0].hypot(v2[1]);
    if d1 < 1e-3 || d2 < 1e-3 {
      continue;
    }
    let r = radius.min(d1 / 2.0).min(d2 / 2.0);
    let before = [curr[0] + (v1[0] / d1) * r, curr[1] + (v1[1] / d1) * r];
    let after = [curr[0] + (v2[0] / d2) * r, curr[1] + (v2[1] / d2) * r];
    let _ = write!(
      d,
      " L{:.1},{:.1} Q{:.1},{:.1} {:.1},{:.1}",
      before[0], before[1], curr[0], curr[1], after[0], after[1]
    );
  }
  let last = pts[pts.len() - 1];
  let _ = write!(d, " L{:.1},{:.1}", last[0], last[1]);
  d
}

fn draw_arrow(out: &mut String, tip: [f64; 2], dir: [f64; 2], color: &str) {
  let len = dir[0].hypot(dir[1]);
  if len < 1e-3 {
    return;
  }
  let ux = dir[0] / len;
  let uy = dir[1] / len;
  let size = 9.0;
  let half = 4.5;
  let bx = tip[0] - ux * size;
  let by = tip[1] - uy * size;
  let p1x = bx - uy * half;
  let p1y = by + ux * half;
  let p2x = bx + uy * half;
  let p2y = by - ux * half;
  let _ = write!(
    out,
    r#"<polygon points="{:.1},{:.1} {p1x:.1},{p1y:.1} {p2x:.1},{p2y:.1}" fill="{color}"/>"#,
    tip[0], tip[1]
  );
}

fn polyline_midpoint(pts: &[[f64; 2]]) -> [f64; 2] {
  let total: f64 = pts
    .windows(2)
    .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
    .sum();
  if total < 1e-3 {
    return pts[0];
  }
  let half = total / 2.0;
  let mut acc = 0.0;
  for w in pts.windows(2) {
    let seg = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
    if acc + seg >= half && seg > 1e-3 {
      let r = (half - acc) / seg;
      return [
        w[0][0] + (w[1][0] - w[0][0]) * r,
        w[0][1] + (w[1][1] - w[0][1]) * r,
      ];
    }
    acc += seg;
  }
  pts[pts.len() / 2]
}

fn wrap_lines(text: &str, max_width: f64, font_size: f64) -> Vec<String> {
  let mut lines = Vec::new();
  for para in text.split('\n') {
    if para.is_empty() {
      lines.push(String::new());
      continue;
    }
    let mut cur = String::new();
    let mut cur_w = 0.0;
    for ch in para.chars() {
      let cw = if ch.is_ascii() {
        font_size * 0.56
      } else {
        font_size * 1.02
      };
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

fn esc(s: &str) -> String {
  s.replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
    .replace('\'', "&#39;")
}
