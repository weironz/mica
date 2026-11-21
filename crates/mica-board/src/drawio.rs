//! Pure-Rust draw.io (mxGraph) importer supporting `.drawio`, `.xml`,
//! `.drawio.svg` (`content` attribute), and `.drawio.png` (`tEXt`/`zTXt` mxfile chunks).

use std::collections::HashMap;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use percent_encoding::percent_decode_str;

use crate::inflate::{inflate_raw, inflate_zlib};
use crate::model::{
  BOARD_SCHEMA_VERSION, BoardEdge, BoardNode, BoardSnapshot, EdgeStyle, NodeStyle,
};

#[derive(Debug, thiserror::Error)]
pub enum DrawioError {
  #[error("input does not contain a valid draw.io mxfile/mxGraphModel")]
  NotDrawio,
  #[error("XML parse error: {0}")]
  Xml(String),
  #[error("failed to decode compressed diagram: {0}")]
  Decode(String),
}

/// Import raw bytes from `.drawio`, `.xml`, `.drawio.svg`, or `.drawio.png`.
pub fn import_drawio_bytes(bytes: &[u8]) -> Result<BoardSnapshot, DrawioError> {
  // 1. PNG container (.drawio.png)
  if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
    let xml = extract_mxfile_from_png(bytes)?;
    return import_drawio_str(&xml);
  }
  let text = std::str::from_utf8(bytes).map_err(|_| DrawioError::NotDrawio)?;
  import_drawio_str(text)
}

/// Import text from `.drawio`, `.xml`, or `.drawio.svg`.
pub fn import_drawio_str(input: &str) -> Result<BoardSnapshot, DrawioError> {
  let trimmed = input.trim();
  // .drawio.svg stores HTML-escaped `<mxfile>` in the root `<svg content="...">` attribute.
  let xml_source = if trimmed.starts_with("<svg") || trimmed.contains("<svg ") {
    extract_mxfile_from_svg(trimmed)?
  } else {
    trimmed.to_string()
  };

  let doc = roxmltree::Document::parse(&xml_source).map_err(|e| DrawioError::Xml(e.to_string()))?;
  let root = doc.root_element();

  let mut diagram_models: Vec<(String, String)> = Vec::new();
  if root.has_tag_name("mxGraphModel") {
    diagram_models.push(("draw.io 导入".to_string(), xml_source.clone()));
  } else if root.has_tag_name("mxfile") {
    for child in root.children().filter(|n| n.has_tag_name("diagram")) {
      let page_name = child
        .attribute("name")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Page")
        .to_string();
      if let Some(inner_model) = child.children().find(|n| n.has_tag_name("mxGraphModel")) {
        let range = inner_model.range();
        diagram_models.push((page_name, xml_source[range].to_string()));
      } else if let Some(encoded) = child.text() {
        let decoded = decode_diagram_payload(encoded.trim())?;
        diagram_models.push((page_name, decoded));
      }
    }
  } else {
    return Err(DrawioError::NotDrawio);
  }

  if diagram_models.is_empty() {
    return Err(DrawioError::NotDrawio);
  }

  let mut all_nodes = Vec::new();
  let mut all_edges = Vec::new();
  let multi_page = diagram_models.len() > 1;
  let mut page_offset_x = 0.0;
  let mut title = diagram_models[0].0.clone();
  if title == "Page" || title == "Page-1" {
    title = "draw.io 导入".to_string();
  }

  for (page_idx, (page_name, model_xml)) in diagram_models.into_iter().enumerate() {
    let prefix = if multi_page {
      format!("p{page_idx}_")
    } else {
      String::new()
    };
    let (mut p_nodes, p_edges) = parse_mx_graph_model(&model_xml, &prefix, page_offset_x)?;
    if p_nodes.is_empty() && p_edges.is_empty() {
      continue;
    }

    let min_x = p_nodes.iter().map(|n| n.x).fold(f64::INFINITY, f64::min);
    let min_y = p_nodes.iter().map(|n| n.y).fold(f64::INFINITY, f64::min);
    let max_x = p_nodes
      .iter()
      .map(|n| n.x + n.w)
      .fold(f64::NEG_INFINITY, f64::max);
    let max_y = p_nodes
      .iter()
      .map(|n| n.y + n.h)
      .fold(f64::NEG_INFINITY, f64::max);

    if multi_page && min_x.is_finite() {
      all_nodes.push(BoardNode {
        id: format!("frame_p{page_idx}"),
        kind: "frame".into(),
        x: min_x - 32.0,
        y: min_y - 48.0,
        w: (max_x - min_x + 64.0).max(240.0),
        h: (max_y - min_y + 80.0).max(180.0),
        rot: 0.0,
        z: 0,
        parent_id: None,
        rank: page_idx as i32,
        collapsed: false,
        text: page_name,
        icon: None,
        style: NodeStyle {
          fill: "#F7F8FA".into(),
          stroke: "#C2C8D0".into(),
          stroke_width: 1.2,
          dashed: true,
          align: "left".into(),
          ..Default::default()
        },
      });
      page_offset_x = (max_x - min_x) + page_offset_x + 160.0;
    }

    all_nodes.append(&mut p_nodes);
    all_edges.extend(p_edges);
  }

  Ok(BoardSnapshot {
    version: BOARD_SCHEMA_VERSION,
    title,
    mode: "flowchart".into(),
    nodes: all_nodes,
    edges: all_edges,
    strokes: vec![],
  })
}

fn extract_mxfile_from_svg(svg: &str) -> Result<String, DrawioError> {
  let doc = roxmltree::Document::parse(svg).map_err(|e| DrawioError::Xml(e.to_string()))?;
  let content = doc
    .root_element()
    .attribute("content")
    .ok_or(DrawioError::NotDrawio)?;
  Ok(content.to_string())
}

fn extract_mxfile_from_png(bytes: &[u8]) -> Result<String, DrawioError> {
  let mut pos = 8usize;
  while pos + 12 <= bytes.len() {
    let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
    let kind = &bytes[pos + 4..pos + 8];
    let data_start = pos + 8;
    let data_end = data_start.saturating_add(len);
    if data_end + 4 > bytes.len() {
      break;
    }
    let data = &bytes[data_start..data_end];
    if kind == b"tEXt" {
      if let Some(nul) = data.iter().position(|&b| b == 0) {
        if &data[..nul] == b"mxfile" {
          let raw = String::from_utf8_lossy(&data[nul + 1..]);
          let decoded = percent_decode_str(&raw).decode_utf8_lossy().into_owned();
          return Ok(decoded);
        }
      }
    } else if kind == b"zTXt" {
      if let Some(nul) = data.iter().position(|&b| b == 0) {
        if &data[..nul] == b"mxfile" && nul + 2 < data.len() {
          let inflated =
            inflate_zlib(&data[nul + 2..]).map_err(|e| DrawioError::Decode(e.to_string()))?;
          let raw = String::from_utf8_lossy(&inflated);
          let decoded = percent_decode_str(&raw).decode_utf8_lossy().into_owned();
          return Ok(decoded);
        }
      }
    }
    pos = data_end + 4;
  }
  Err(DrawioError::NotDrawio)
}

fn decode_diagram_payload(encoded: &str) -> Result<String, DrawioError> {
  let compact: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
  if compact.starts_with("<mxGraphModel") {
    return Ok(compact);
  }
  let compressed = STANDARD
    .decode(&compact)
    .map_err(|e| DrawioError::Decode(e.to_string()))?;
  let inflated = inflate_raw(&compressed).map_err(|e| DrawioError::Decode(e.to_string()))?;
  let raw_str = String::from_utf8(inflated).map_err(|e| DrawioError::Decode(e.to_string()))?;
  let decoded = percent_decode_str(&raw_str)
    .decode_utf8()
    .map_err(|e| DrawioError::Decode(e.to_string()))?;
  Ok(decoded.into_owned())
}

struct RawCell {
  id: String,
  parent: Option<String>,
  value: String,
  style: HashMap<String, String>,
  vertex: bool,
  edge: bool,
  source: Option<String>,
  target: Option<String>,
  x: f64,
  y: f64,
  w: f64,
  h: f64,
  waypoints: Vec<[f64; 2]>,
}

fn parse_mx_graph_model(
  xml: &str,
  id_prefix: &str,
  offset_x: f64,
) -> Result<(Vec<BoardNode>, Vec<BoardEdge>), DrawioError> {
  let doc = roxmltree::Document::parse(xml).map_err(|e| DrawioError::Xml(e.to_string()))?;
  let mut cells: Vec<RawCell> = Vec::new();
  let mut cell_by_id: HashMap<String, usize> = HashMap::new();

  for node in doc.descendants() {
    let (cell_el, override_id, override_value) = if node.has_tag_name("mxCell") {
      // Skip mxCell wrapped by <object>/<UserObject> since we handle the wrapper.
      if node
        .parent()
        .is_some_and(|p| p.has_tag_name("object") || p.has_tag_name("UserObject"))
      {
        continue;
      }
      (node, None, None)
    } else if node.has_tag_name("object") || node.has_tag_name("UserObject") {
      let Some(inner) = node.children().find(|c| c.has_tag_name("mxCell")) else {
        continue;
      };
      (inner, node.attribute("id"), node.attribute("label"))
    } else {
      continue;
    };

    let Some(id) = override_id.or_else(|| cell_el.attribute("id")) else {
      continue;
    };
    let value = override_value
      .or_else(|| cell_el.attribute("value"))
      .unwrap_or("");
    let style = parse_style(cell_el.attribute("style").unwrap_or(""));
    let vertex = cell_el.attribute("vertex") == Some("1");
    let edge = cell_el.attribute("edge") == Some("1");
    let parent = cell_el.attribute("parent").map(str::to_string);
    let source = cell_el.attribute("source").map(str::to_string);
    let target = cell_el.attribute("target").map(str::to_string);

    let (mut x, mut y, mut w, mut h) = (0.0, 0.0, 120.0, 56.0);
    let mut waypoints = Vec::new();
    if let Some(geom) = cell_el.children().find(|c| c.has_tag_name("mxGeometry")) {
      x = geom
        .attribute("x")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
      y = geom
        .attribute("y")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
      w = geom
        .attribute("width")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(120.0);
      h = geom
        .attribute("height")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(56.0);
      for arr in geom.children().filter(|c| c.has_tag_name("Array")) {
        if arr.attribute("as") == Some("points") {
          for pt in arr.children().filter(|c| c.has_tag_name("mxPoint")) {
            if let (Some(px), Some(py)) = (
              pt.attribute("x").and_then(|v| v.parse::<f64>().ok()),
              pt.attribute("y").and_then(|v| v.parse::<f64>().ok()),
            ) {
              waypoints.push([px + offset_x, py]);
            }
          }
        }
      }
    }

    let idx = cells.len();
    cell_by_id.insert(id.to_string(), idx);
    cells.push(RawCell {
      id: id.to_string(),
      parent,
      value: strip_html(value),
      style,
      vertex,
      edge,
      source,
      target,
      x,
      y,
      w,
      h,
      waypoints,
    });
  }

  // Resolve relative coordinates for vertices nested inside containers / swimlanes.
  let mut abs_pos: HashMap<String, (f64, f64)> = HashMap::new();
  for cell in &cells {
    if !cell.vertex {
      continue;
    }
    let mut ax = cell.x;
    let mut ay = cell.y;
    let mut cur_parent = cell.parent.as_deref();
    let mut guard = 0;
    while let Some(pid) = cur_parent {
      guard += 1;
      if guard > 32 || pid == "0" || pid == "1" {
        break;
      }
      if let Some(&pidx) = cell_by_id.get(pid) {
        let p = &cells[pidx];
        if p.vertex {
          ax += p.x;
          ay += p.y;
        }
        cur_parent = p.parent.as_deref();
      } else {
        break;
      }
    }
    abs_pos.insert(cell.id.clone(), (ax + offset_x, ay));
  }

  // Collect edge labels attached as child vertices of edges.
  let mut edge_child_labels: HashMap<String, String> = HashMap::new();
  for cell in &cells {
    if cell.vertex && !cell.value.is_empty() {
      if let Some(pid) = &cell.parent {
        if let Some(&pidx) = cell_by_id.get(pid) {
          if cells[pidx].edge {
            edge_child_labels.insert(pid.clone(), cell.value.clone());
          }
        }
      }
    }
  }

  let mut nodes = Vec::new();
  let mut edges = Vec::new();
  let mut z_counter = 1i32;

  for cell in &cells {
    if cell.vertex {
      // Skip child label vertices belonging to edges.
      if let Some(pid) = &cell.parent {
        if cell_by_id.get(pid).is_some_and(|&i| cells[i].edge) {
          continue;
        }
      }
      let (ax, ay) = abs_pos
        .get(&cell.id)
        .copied()
        .unwrap_or((cell.x + offset_x, cell.y));
      let (kind, icon) = map_drawio_shape(&cell.style);
      let fill = cell
        .style
        .get("fillColor")
        .filter(|c| *c != "none" && *c != "default")
        .cloned()
        .unwrap_or_else(|| {
          if kind == "text" {
            "transparent".into()
          } else {
            "#FFFFFF".into()
          }
        });
      let stroke = cell
        .style
        .get("strokeColor")
        .filter(|c| *c != "none" && *c != "default")
        .cloned()
        .unwrap_or_else(|| {
          if kind == "text" {
            "transparent".into()
          } else {
            "#646A73".into()
          }
        });
      let text_color = cell
        .style
        .get("fontColor")
        .filter(|c| *c != "default")
        .cloned()
        .unwrap_or_else(|| "#1F2329".into());
      let font_size = cell
        .style
        .get("fontSize")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(13.0)
        .clamp(9.0, 36.0);
      let stroke_width = cell
        .style
        .get("strokeWidth")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(1.5)
        .clamp(0.5, 6.0);
      let dashed = cell.style.get("dashed").is_some_and(|v| v == "1");
      let align = cell
        .style
        .get("align")
        .cloned()
        .unwrap_or_else(|| "center".into());

      let is_container = kind == "frame";
      nodes.push(BoardNode {
        id: format!("{id_prefix}{}", cell.id),
        kind,
        x: ax,
        y: ay,
        w: cell.w.max(36.0),
        h: cell.h.max(24.0),
        rot: cell
          .style
          .get("rotation")
          .and_then(|v| v.parse::<f64>().ok())
          .unwrap_or(0.0),
        z: if is_container { 0 } else { z_counter },
        parent_id: None,
        rank: z_counter,
        collapsed: false,
        text: cell.value.clone(),
        icon,
        style: NodeStyle {
          fill,
          stroke,
          stroke_width,
          dashed,
          font_size,
          text_color,
          align,
        },
      });
      z_counter += 1;
    } else if cell.edge {
      let (Some(src), Some(dst)) = (&cell.source, &cell.target) else {
        continue;
      };
      let routing = if cell.style.get("curved").is_some_and(|v| v == "1") {
        "curved"
      } else if cell
        .style
        .get("edgeStyle")
        .is_some_and(|s| s.contains("orthogonal") || s.contains("elbow"))
      {
        "orthogonal"
      } else {
        "straight"
      };
      let arrow_end = if cell.style.get("endArrow").is_some_and(|v| v == "none") {
        "none"
      } else {
        "arrow"
      };
      let arrow_start = if cell
        .style
        .get("startArrow")
        .is_some_and(|v| !v.is_empty() && v != "none")
      {
        "arrow"
      } else {
        "none"
      };
      let label = if !cell.value.is_empty() {
        cell.value.clone()
      } else {
        edge_child_labels.get(&cell.id).cloned().unwrap_or_default()
      };
      edges.push(BoardEdge {
        id: format!("{id_prefix}{}", cell.id),
        from_id: format!("{id_prefix}{src}"),
        to_id: format!("{id_prefix}{dst}"),
        from_anchor: anchor_from_xy(cell.style.get("exitX"), cell.style.get("exitY")),
        to_anchor: anchor_from_xy(cell.style.get("entryX"), cell.style.get("entryY")),
        routing: routing.into(),
        arrow_start: arrow_start.into(),
        arrow_end: arrow_end.into(),
        waypoints: cell.waypoints.clone(),
        label,
        style: EdgeStyle {
          stroke: cell
            .style
            .get("strokeColor")
            .filter(|c| *c != "default" && *c != "none")
            .cloned()
            .unwrap_or_else(|| "#646A73".into()),
          stroke_width: cell
            .style
            .get("strokeWidth")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(1.5)
            .clamp(0.8, 5.0),
          dashed: cell.style.get("dashed").is_some_and(|v| v == "1"),
        },
      });
    }
  }

  Ok((nodes, edges))
}

fn parse_style(raw: &str) -> HashMap<String, String> {
  let mut map = HashMap::new();
  for token in raw.split(';') {
    let t = token.trim();
    if t.is_empty() {
      continue;
    }
    if let Some((k, v)) = t.split_once('=') {
      map.insert(k.trim().to_string(), v.trim().to_string());
    } else {
      map.insert(t.to_string(), "1".to_string());
    }
  }
  map
}

fn map_drawio_shape(style: &HashMap<String, String>) -> (String, Option<String>) {
  if style.contains_key("text") {
    return ("text".into(), None);
  }
  if style.contains_key("swimlane") || style.contains_key("group") {
    return ("frame".into(), None);
  }
  if style.contains_key("rhombus") {
    return ("diamond".into(), None);
  }
  if style.contains_key("ellipse") {
    return ("ellipse".into(), None);
  }
  if style.contains_key("triangle") {
    return ("triangle".into(), None);
  }
  if style.contains_key("cloud") {
    return ("cloud".into(), None);
  }
  if let Some(shape) = style.get("shape") {
    let s = shape.to_ascii_lowercase();
    if s.contains("cylinder") || s.contains("database") || s.contains("rds") || s.contains("dynamo")
    {
      return ("cylinder".into(), None);
    }
    if s.contains("parallelogram") {
      return ("parallelogram".into(), None);
    }
    if s.contains("document") {
      return ("document".into(), None);
    }
    if s.contains("hexagon") {
      return ("hexagon".into(), None);
    }
    if s.contains("note") {
      return ("sticky".into(), None);
    }
    if s.contains("actor") || s.contains("user") {
      return ("icon_card".into(), Some("user".into()));
    }
    if s.contains("server") || s.contains("ec2") || s.contains("compute") {
      return ("icon_card".into(), Some("server".into()));
    }
    if s.contains("cloud") {
      return ("cloud".into(), None);
    }
    if s.contains("queue") || s.contains("sqs") || s.contains("kafka") {
      return ("icon_card".into(), Some("queue".into()));
    }
    if s.contains("cache") || s.contains("redis") {
      return ("icon_card".into(), Some("cache".into()));
    }
    if s.contains("gateway") || s.contains("router") {
      return ("icon_card".into(), Some("gateway".into()));
    }
    if s.contains("lock") || s.contains("shield") || s.contains("firewall") {
      return ("icon_card".into(), Some("firewall".into()));
    }
    if s.contains("container") || s.contains("docker") || s.contains("pod") || s.contains("k8s") {
      return ("icon_card".into(), Some("container".into()));
    }
  }
  if style.get("rounded").is_some_and(|v| v == "1") {
    if style
      .get("arcSize")
      .and_then(|v| v.parse::<f64>().ok())
      .is_some_and(|a| a >= 40.0)
    {
      return ("pill".into(), None);
    }
    return ("rounded".into(), None);
  }
  ("rect".into(), None)
}

fn anchor_from_xy(x: Option<&String>, y: Option<&String>) -> String {
  let (Some(xs), Some(ys)) = (x, y) else {
    return "auto".into();
  };
  let (Ok(xv), Ok(yv)) = (xs.parse::<f64>(), ys.parse::<f64>()) else {
    return "auto".into();
  };
  if yv <= 0.1 {
    "t".into()
  } else if yv >= 0.9 {
    "b".into()
  } else if xv <= 0.1 {
    "l".into()
  } else if xv >= 0.9 {
    "r".into()
  } else {
    "auto".into()
  }
}

fn strip_html(raw: &str) -> String {
  let replaced = raw
    .replace("<br>", "\n")
    .replace("<br/>", "\n")
    .replace("<br />", "\n")
    .replace("</div>", "\n")
    .replace("</p>", "\n");
  let mut out = String::with_capacity(replaced.len());
  let mut in_tag = false;
  for ch in replaced.chars() {
    match ch {
      '<' => in_tag = true,
      '>' => in_tag = false,
      _ if !in_tag => out.push(ch),
      _ => {}
    }
  }
  out
    .replace("&nbsp;", " ")
    .replace("&amp;", "&")
    .replace("&lt;", "<")
    .replace("&gt;", ">")
    .replace("&quot;", "\"")
    .replace("&#39;", "'")
    .trim()
    .to_string()
}
