//! Mica infinite whiteboard, mind map, flowchart, draw.io importer, orthogonal
//! connector router, 3-way semantic merge, and headless SVG renderer.
//!
//! Pure Rust, zero UI/browser dependency — shared identically by `mica-md`
//! HTML export, desktop FFI (`clients/mica_flutter/rust`), Web WASM
//! (`crates/board-wasm`), CLI, and MCP.

#[cfg(feature = "crdt")]
pub mod crdt;
pub mod drawio;
pub mod inflate;
pub mod layout;
pub mod merge;
pub mod model;
pub mod routing;
pub mod session;
pub mod svg;

use std::collections::HashMap;

pub use drawio::{DrawioError, import_drawio_bytes, import_drawio_str};
pub use layout::{layout_mindmap, markdown_to_mindmap, mindmap_to_markdown};
pub use merge::{extract_plain_text, merge_board_snapshots};
pub use model::{
  BOARD_SCHEMA_VERSION, BoardEdge, BoardNode, BoardSnapshot, BoardStroke, BoardSvgTheme, EdgeStyle,
  NodeStyle, tech_icon_svg_inner,
};
pub use routing::route_edge;
pub use session::{
  ActiveOverlay, BoardCommand, BoardPatch, BoardSession, BoardTool, PendingConnector, Rect,
  ResizeHandle, SnapGuide, SnapshotDelta, UiEffect,
};
pub use svg::render_board_svg;

/// Parse a `mica-board` JSON payload and render it to a standalone SVG string.
/// Returns `None` if `source` is not valid board JSON.
pub fn render_board_json_svg(source: &str, theme: Option<&BoardSvgTheme>) -> Option<String> {
  let snap = BoardSnapshot::parse_json(source).ok()?;
  Some(render_board_svg(&snap, theme))
}

/// Re-layout mind map trees inside a board JSON string and return updated compact JSON.
pub fn layout_mindmap_json(source: &str) -> Option<String> {
  let mut snap = BoardSnapshot::parse_json(source).ok()?;
  layout_mindmap(&mut snap);
  Some(snap.to_compact_json())
}

/// Compute routed polylines for every edge in a board JSON string.
/// Returns JSON object `{ "<edge_id>": [[x, y], ...], ... }`.
pub fn route_board_edges_json(source: &str) -> Option<String> {
  let snap = BoardSnapshot::parse_json(source).ok()?;
  let node_map: HashMap<&str, &BoardNode> = snap.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
  let mut routes: HashMap<&str, Vec<[f64; 2]>> = HashMap::with_capacity(snap.edges.len());
  for edge in &snap.edges {
    routes.insert(edge.id.as_str(), route_edge(edge, &snap.nodes, &node_map));
  }
  serde_json::to_string(&routes).ok()
}

/// 3-way semantic merge of board JSON strings. Falls back to `local` on parse error.
pub fn merge_board_json(base: &str, local: &str, remote: &str) -> String {
  let (Ok(b), Ok(l), Ok(r)) = (
    BoardSnapshot::parse_json(base),
    BoardSnapshot::parse_json(local),
    BoardSnapshot::parse_json(remote),
  ) else {
    return local.to_string();
  };
  merge_board_snapshots(&b, &l, &r).to_compact_json()
}

/// Extract searchable plain text from a `mica-board` JSON string.
pub fn extract_board_text_json(source: &str) -> String {
  match BoardSnapshot::parse_json(source) {
    Ok(snap) => extract_plain_text(&snap),
    Err(_) => String::new(),
  }
}

/// Create a starter board JSON for `"whiteboard"`, `"mindmap"`, or `"flowchart"`.
pub fn starter_board_json(template: &str) -> String {
  match template {
    "mindmap" => BoardSnapshot::template_mindmap().to_compact_json(),
    "flowchart" => BoardSnapshot::template_flowchart().to_compact_json(),
    _ => BoardSnapshot::template_whiteboard().to_compact_json(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn starter_templates_render_valid_svg() {
    for kind in ["whiteboard", "mindmap", "flowchart"] {
      let json = starter_board_json(kind);
      let svg = render_board_json_svg(&json, None).expect("svg");
      assert!(svg.starts_with("<svg "));
      assert!(svg.ends_with("</svg>"));
      assert!(!svg.contains("<foreignObject"));
    }
  }

  #[test]
  fn markdown_mindmap_roundtrip_and_layout() {
    let md = "# Mica 架构\n- 数据面\n  - Rust CRDT\n  - SQLite\n- 表现层\n  - Flutter Canvas\n";
    let snap = markdown_to_mindmap(md, None);
    assert_eq!(snap.mode, "mindmap");
    assert_eq!(snap.nodes.len(), 6);
    assert_eq!(snap.edges.len(), 5);
    // Children must be strictly to the right of root.
    let root = &snap.nodes[0];
    for child in &snap.nodes[1..] {
      assert!(child.x > root.x + root.w);
    }
    let exported = mindmap_to_markdown(&snap);
    assert!(exported.contains("# Mica 架构"));
    assert!(exported.contains("- Rust CRDT"));
    assert!(exported.contains("- Flutter Canvas"));
  }

  #[test]
  fn orthogonal_router_avoids_obstacle_and_keeps_waypoints() {
    let mut snap = BoardSnapshot::template_flowchart();
    // Place an obstacle right between f1 and f2.
    snap.nodes.push(BoardNode {
      id: "obs".into(),
      kind: "rect".into(),
      x: 200.0,
      y: 112.0,
      w: 180.0,
      h: 30.0,
      rot: 0.0,
      z: 9,
      parent_id: None,
      rank: 0,
      collapsed: false,
      text: "obstacle".into(),
      icon: None,
      style: NodeStyle::default(),
    });
    let node_map: HashMap<&str, &BoardNode> =
      snap.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let pts = route_edge(&snap.edges[0], &snap.nodes, &node_map);
    assert!(
      pts.len() >= 4,
      "expected routed bends around obstacle, got {pts:?}"
    );

    // Manual waypoints must be preserved.
    let mut manual_edge = snap.edges[0].clone();
    manual_edge.waypoints = vec![[120.0, 125.0]];
    let manual_pts = route_edge(&manual_edge, &snap.nodes, &node_map);
    assert!(manual_pts.iter().any(|p| (p[0] - 120.0).abs() < 1.0));

    // Orthogonal SVG bends must emit rounded quadratic corners (` Q`).
    let svg = render_board_svg(&snap, None);
    assert!(
      svg.contains(" Q"),
      "expected rounded orthogonal corners in SVG"
    );
  }

  #[test]
  fn import_drawio_uncompressed_and_svg_wrapper() {
    let mx = r#"<mxfile><diagram name="系统架构"><mxGraphModel><root>
      <mxCell id="0"/><mxCell id="1" parent="0"/>
      <mxCell id="c1" value="API 网关" style="rounded=1;fillColor=#dae8fc;strokeColor=#6c8ebf;" vertex="1" parent="1">
        <mxGeometry x="40" y="60" width="140" height="50" as="geometry"/>
      </mxCell>
      <mxCell id="c2" value="Postgres &lt;b&gt;主库&lt;/b&gt;" style="shape=cylinder3;fillColor=#d5e8d4;strokeColor=#82b366;" vertex="1" parent="1">
        <mxGeometry x="280" y="60" width="120" height="70" as="geometry"/>
      </mxCell>
      <mxCell id="e1" value="SQL" style="edgeStyle=orthogonalEdgeStyle;endArrow=classic;" edge="1" parent="1" source="c1" target="c2">
        <mxGeometry relative="1" as="geometry">
          <Array as="points"><mxPoint x="230" y="90"/></Array>
        </mxGeometry>
      </mxCell>
    </root></mxGraphModel></diagram></mxfile>"#;

    let snap = import_drawio_str(mx).expect("import mxfile");
    assert_eq!(snap.title, "系统架构");
    assert_eq!(snap.nodes.len(), 2);
    assert_eq!(snap.nodes[0].kind, "rounded");
    assert_eq!(snap.nodes[1].kind, "cylinder");
    assert_eq!(snap.nodes[1].text, "Postgres 主库");
    assert_eq!(snap.edges.len(), 1);
    assert_eq!(snap.edges[0].routing, "orthogonal");
    assert_eq!(snap.edges[0].label, "SQL");
    assert_eq!(snap.edges[0].waypoints, vec![[230.0, 90.0]]);

    // Also verify `.drawio.svg` wrapper extraction.
    let escaped = mx
      .replace('&', "&amp;")
      .replace('<', "&lt;")
      .replace('>', "&gt;")
      .replace('"', "&quot;");
    let svg_wrap = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" content="{escaped}"></svg>"#);
    let snap2 = import_drawio_str(&svg_wrap).expect("import drawio.svg");
    assert_eq!(snap2.nodes.len(), 2);
  }

  #[test]
  fn inflate_raw_fixed_huffman_and_stored_block() {
    // Stored raw DEFLATE block containing "hello mica"
    let payload = b"hello mica";
    let len = payload.len() as u16;
    let nlen = !len;
    let mut raw = vec![0x01]; // BFINAL=1, BTYPE=00 (stored)
    raw.extend_from_slice(&len.to_le_bytes());
    raw.extend_from_slice(&nlen.to_le_bytes());
    raw.extend_from_slice(payload);
    let decoded = inflate::inflate_raw(&raw).expect("inflate stored");
    assert_eq!(decoded, payload);
  }

  #[test]
  fn three_way_merge_preserves_independent_edits() {
    let base = BoardSnapshot::template_whiteboard();
    let mut local = base.clone();
    let mut remote = base.clone();
    // Local moves n1; remote edits n1's text & adds n3.
    local.nodes[0].x = 240.0;
    remote.nodes[0].text = "并发更新后的标题".into();
    remote.nodes.push(BoardNode {
      id: "n3".into(),
      kind: "diamond".into(),
      x: 500.0,
      y: 300.0,
      w: 120.0,
      h: 70.0,
      rot: 0.0,
      z: 3,
      parent_id: None,
      rank: 2,
      collapsed: false,
      text: "远端新增".into(),
      icon: None,
      style: NodeStyle::default(),
    });
    let merged = merge_board_snapshots(&base, &local, &remote);
    let n1 = merged.nodes.iter().find(|n| n.id == "n1").unwrap();
    assert!((n1.x - 240.0).abs() < 1e-6);
    assert_eq!(n1.text, "并发更新后的标题");
    assert!(merged.nodes.iter().any(|n| n.id == "n3"));
    assert!(extract_plain_text(&merged).contains("远端新增"));
  }

  #[cfg(feature = "crdt")]
  #[test]
  fn yrs_board_doc_merges_concurrent_move_and_text_edit() {
    let snap = BoardSnapshot::template_whiteboard();
    let doc_a = crdt::BoardDoc::from_snapshot(&snap, Some(101));
    let doc_b = crdt::BoardDoc::new(Some(202));
    doc_b.apply_update(&doc_a.encode_state_as_update()).unwrap();

    doc_a.move_node("n1", 333.0, 444.0);
    doc_b.set_node_text("n1", "Yrs 并发文本");

    let ua = doc_a.encode_state_as_update();
    let ub = doc_b.encode_state_as_update();
    doc_a.apply_update(&ub).unwrap();
    doc_b.apply_update(&ua).unwrap();

    let sa = doc_a.to_snapshot();
    let sb = doc_b.to_snapshot();
    assert_eq!(sa.nodes[0].x, 333.0);
    assert_eq!(sa.nodes[0].text, "Yrs 并发文本");
    assert_eq!(sa.nodes[0], sb.nodes[0]);
  }
}
