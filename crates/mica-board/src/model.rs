use serde::{Deserialize, Serialize};

pub const BOARD_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoardSnapshot {
  #[serde(default = "default_version")]
  pub version: u32,
  #[serde(default = "default_title")]
  pub title: String,
  #[serde(default = "default_mode")]
  pub mode: String, // "whiteboard" | "mindmap" | "flowchart"
  #[serde(default)]
  pub nodes: Vec<BoardNode>,
  #[serde(default)]
  pub edges: Vec<BoardEdge>,
  #[serde(default)]
  pub strokes: Vec<BoardStroke>,
}

fn default_version() -> u32 {
  BOARD_SCHEMA_VERSION
}
fn default_title() -> String {
  "未命名画板".to_string()
}
fn default_mode() -> String {
  "whiteboard".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoardNode {
  pub id: String,
  /// One of: rect, rounded, ellipse, diamond, pill, parallelogram, cylinder,
  /// document, cloud, hexagon, triangle, sticky, text, frame, mind_root,
  /// mind_node, icon_card
  #[serde(default = "default_node_kind")]
  pub kind: String,
  pub x: f64,
  pub y: f64,
  pub w: f64,
  pub h: f64,
  #[serde(default)]
  pub rot: f64,
  #[serde(default)]
  pub z: i32,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub parent_id: Option<String>,
  #[serde(default)]
  pub rank: i32,
  #[serde(default)]
  pub collapsed: bool,
  #[serde(default)]
  pub text: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub icon: Option<String>,
  #[serde(default)]
  pub style: NodeStyle,
}

fn default_node_kind() -> String {
  "rounded".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeStyle {
  #[serde(default = "default_fill")]
  pub fill: String,
  #[serde(default = "default_stroke")]
  pub stroke: String,
  #[serde(default = "default_stroke_width")]
  pub stroke_width: f64,
  #[serde(default)]
  pub dashed: bool,
  #[serde(default = "default_font_size")]
  pub font_size: f64,
  #[serde(default = "default_text_color")]
  pub text_color: String,
  #[serde(default = "default_align")]
  pub align: String, // "left" | "center" | "right"
}

fn default_fill() -> String {
  "#FFFFFF".to_string()
}
fn default_stroke() -> String {
  "#8C959F".to_string()
}
fn default_stroke_width() -> f64 {
  1.5
}
fn default_font_size() -> f64 {
  13.5
}
fn default_text_color() -> String {
  "#1F2329".to_string()
}
fn default_align() -> String {
  "center".to_string()
}

impl Default for NodeStyle {
  fn default() -> Self {
    Self {
      fill: default_fill(),
      stroke: default_stroke(),
      stroke_width: default_stroke_width(),
      dashed: false,
      font_size: default_font_size(),
      text_color: default_text_color(),
      align: default_align(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoardEdge {
  pub id: String,
  pub from_id: String,
  pub to_id: String,
  /// "auto" | "t" | "r" | "b" | "l"
  #[serde(default = "default_anchor")]
  pub from_anchor: String,
  #[serde(default = "default_anchor")]
  pub to_anchor: String,
  /// "curved" | "orthogonal" | "straight"
  #[serde(default = "default_routing")]
  pub routing: String,
  /// "none" | "arrow" | "diamond" | "dot"
  #[serde(default = "default_arrow_start")]
  pub arrow_start: String,
  #[serde(default = "default_arrow_end")]
  pub arrow_end: String,
  #[serde(default)]
  pub waypoints: Vec<[f64; 2]>,
  #[serde(default)]
  pub label: String,
  #[serde(default)]
  pub style: EdgeStyle,
}

fn default_anchor() -> String {
  "auto".to_string()
}
fn default_routing() -> String {
  "orthogonal".to_string()
}
fn default_arrow_start() -> String {
  "none".to_string()
}
fn default_arrow_end() -> String {
  "arrow".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EdgeStyle {
  #[serde(default = "default_edge_stroke")]
  pub stroke: String,
  #[serde(default = "default_stroke_width")]
  pub stroke_width: f64,
  #[serde(default)]
  pub dashed: bool,
}

fn default_edge_stroke() -> String {
  "#646A73".to_string()
}

impl Default for EdgeStyle {
  fn default() -> Self {
    Self {
      stroke: default_edge_stroke(),
      stroke_width: default_stroke_width(),
      dashed: false,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoardStroke {
  pub id: String,
  #[serde(default = "default_ink_color")]
  pub color: String,
  #[serde(default = "default_ink_width")]
  pub width: f64,
  #[serde(default)]
  pub points: Vec<[f64; 2]>,
}

fn default_ink_color() -> String {
  "#3370FF".to_string()
}
fn default_ink_width() -> f64 {
  2.4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardSvgTheme {
  pub dark: bool,
  pub canvas: String,
  pub surface: String,
  pub text: String,
  pub subtle_text: String,
  pub border: String,
  pub accent: String,
}

impl Default for BoardSvgTheme {
  fn default() -> Self {
    Self {
      dark: false,
      canvas: "#F7F8FA".to_string(),
      surface: "#FFFFFF".to_string(),
      text: "#1F2329".to_string(),
      subtle_text: "#646A73".to_string(),
      border: "#8C959F".to_string(),
      accent: "#3370FF".to_string(),
    }
  }
}

impl BoardSnapshot {
  pub fn parse_json(raw: &str) -> Result<Self, serde_json::Error> {
    serde_json::from_str(raw)
  }

  pub fn to_compact_json(&self) -> String {
    serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
  }

  /// Blank whiteboard with a starter sticky note and frame.
  pub fn template_whiteboard() -> Self {
    Self {
      version: BOARD_SCHEMA_VERSION,
      title: "未命名画板".to_string(),
      mode: "whiteboard".to_string(),
      nodes: vec![
        BoardNode {
          id: "n1".into(),
          kind: "rounded".into(),
          x: 120.0,
          y: 120.0,
          w: 180.0,
          h: 64.0,
          rot: 0.0,
          z: 1,
          parent_id: None,
          rank: 0,
          collapsed: false,
          text: "核心想法".into(),
          icon: None,
          style: NodeStyle {
            fill: "#E8F3FF".into(),
            stroke: "#3370FF".into(),
            stroke_width: 1.8,
            ..Default::default()
          },
        },
        BoardNode {
          id: "n2".into(),
          kind: "sticky".into(),
          x: 380.0,
          y: 112.0,
          w: 190.0,
          h: 80.0,
          rot: 0.0,
          z: 2,
          parent_id: None,
          rank: 1,
          collapsed: false,
          text: "双击任意图形编辑文字\n拖边缘圆点拉出连线".into(),
          icon: None,
          style: NodeStyle {
            fill: "#FFF7CC".into(),
            stroke: "#E0B422".into(),
            stroke_width: 1.4,
            align: "left".into(),
            ..Default::default()
          },
        },
      ],
      edges: vec![BoardEdge {
        id: "e1".into(),
        from_id: "n1".into(),
        to_id: "n2".into(),
        from_anchor: "r".into(),
        to_anchor: "l".into(),
        routing: "curved".into(),
        arrow_start: "none".into(),
        arrow_end: "arrow".into(),
        waypoints: vec![],
        label: String::new(),
        style: EdgeStyle::default(),
      }],
      strokes: vec![],
    }
  }

  /// Starter mind map with root + 3 branches laid out cleanly.
  pub fn template_mindmap() -> Self {
    let mut snap = Self {
      version: BOARD_SCHEMA_VERSION,
      title: "思维导图".to_string(),
      mode: "mindmap".to_string(),
      nodes: vec![
        BoardNode {
          id: "m0".into(),
          kind: "mind_root".into(),
          x: 80.0,
          y: 180.0,
          w: 168.0,
          h: 52.0,
          rot: 0.0,
          z: 1,
          parent_id: None,
          rank: 0,
          collapsed: false,
          text: "中心主题".into(),
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
        },
        mind_child("m1", "m0", 0, "分支主题 1", "#E8F3FF", "#3370FF"),
        mind_child("m2", "m0", 1, "分支主题 2", "#EDF7ED", "#2DA44E"),
        mind_child("m3", "m0", 2, "分支主题 3", "#FFF7E6", "#D9822B"),
      ],
      edges: vec![
        mind_edge("me1", "m0", "m1", "#3370FF"),
        mind_edge("me2", "m0", "m2", "#2DA44E"),
        mind_edge("me3", "m0", "m3", "#D9822B"),
      ],
      strokes: vec![],
    };
    crate::layout::layout_mindmap(&mut snap);
    snap
  }

  /// Starter flowchart (pill -> process -> decision -> branches).
  pub fn template_flowchart() -> Self {
    Self {
      version: BOARD_SCHEMA_VERSION,
      title: "流程图".to_string(),
      mode: "flowchart".to_string(),
      nodes: vec![
        BoardNode {
          id: "f1".into(),
          kind: "pill".into(),
          x: 220.0,
          y: 60.0,
          w: 140.0,
          h: 46.0,
          rot: 0.0,
          z: 1,
          parent_id: None,
          rank: 0,
          collapsed: false,
          text: "开始".into(),
          icon: None,
          style: NodeStyle {
            fill: "#EDF7ED".into(),
            stroke: "#2DA44E".into(),
            ..Default::default()
          },
        },
        BoardNode {
          id: "f2".into(),
          kind: "rounded".into(),
          x: 200.0,
          y: 156.0,
          w: 180.0,
          h: 54.0,
          rot: 0.0,
          z: 2,
          parent_id: None,
          rank: 1,
          collapsed: false,
          text: "处理请求".into(),
          icon: None,
          style: NodeStyle {
            fill: "#E8F3FF".into(),
            stroke: "#3370FF".into(),
            ..Default::default()
          },
        },
        BoardNode {
          id: "f3".into(),
          kind: "diamond".into(),
          x: 210.0,
          y: 262.0,
          w: 160.0,
          h: 84.0,
          rot: 0.0,
          z: 3,
          parent_id: None,
          rank: 2,
          collapsed: false,
          text: "校验通过？".into(),
          icon: None,
          style: NodeStyle {
            fill: "#FFF7E6".into(),
            stroke: "#D9822B".into(),
            ..Default::default()
          },
        },
        BoardNode {
          id: "f4".into(),
          kind: "cylinder".into(),
          x: 215.0,
          y: 400.0,
          w: 150.0,
          h: 68.0,
          rot: 0.0,
          z: 4,
          parent_id: None,
          rank: 3,
          collapsed: false,
          text: "写入数据库".into(),
          icon: None,
          style: NodeStyle {
            fill: "#F5F0FF".into(),
            stroke: "#8250DF".into(),
            ..Default::default()
          },
        },
      ],
      edges: vec![
        flow_edge("fe1", "f1", "b", "f2", "t", ""),
        flow_edge("fe2", "f2", "b", "f3", "t", ""),
        flow_edge("fe3", "f3", "b", "f4", "t", "是"),
      ],
      strokes: vec![],
    }
  }
}

fn mind_child(
  id: &str,
  parent: &str,
  rank: i32,
  text: &str,
  fill: &str,
  stroke: &str,
) -> BoardNode {
  BoardNode {
    id: id.into(),
    kind: "mind_node".into(),
    x: 340.0,
    y: 120.0 + (rank as f64) * 76.0,
    w: 150.0,
    h: 42.0,
    rot: 0.0,
    z: rank + 2,
    parent_id: Some(parent.into()),
    rank,
    collapsed: false,
    text: text.into(),
    icon: None,
    style: NodeStyle {
      fill: fill.into(),
      stroke: stroke.into(),
      stroke_width: 1.5,
      align: "left".into(),
      ..Default::default()
    },
  }
}

fn mind_edge(id: &str, from: &str, to: &str, stroke: &str) -> BoardEdge {
  BoardEdge {
    id: id.into(),
    from_id: from.into(),
    to_id: to.into(),
    from_anchor: "r".into(),
    to_anchor: "l".into(),
    routing: "curved".into(),
    arrow_start: "none".into(),
    arrow_end: "none".into(),
    waypoints: vec![],
    label: String::new(),
    style: EdgeStyle {
      stroke: stroke.into(),
      stroke_width: 1.8,
      dashed: false,
    },
  }
}

fn flow_edge(id: &str, from: &str, fa: &str, to: &str, ta: &str, label: &str) -> BoardEdge {
  BoardEdge {
    id: id.into(),
    from_id: from.into(),
    to_id: to.into(),
    from_anchor: fa.into(),
    to_anchor: ta.into(),
    routing: "orthogonal".into(),
    arrow_start: "none".into(),
    arrow_end: "arrow".into(),
    waypoints: vec![],
    label: label.into(),
    style: EdgeStyle::default(),
  }
}

/// Built-in lightweight technical architecture icon glyph (24x24 viewBox SVG inner elements).
pub fn tech_icon_svg_inner(name: &str) -> Option<&'static str> {
  Some(match name {
    "server" => {
      r#"<rect x="3" y="4" width="18" height="6" rx="1.5"/><rect x="3" y="14" width="18" height="6" rx="1.5"/><circle cx="7" cy="7" r="1"/><circle cx="7" cy="17" r="1"/>"#
    }
    "database" => {
      r#"<ellipse cx="12" cy="5.5" rx="7.5" ry="2.8"/><path d="M4.5 5.5v13c0 1.55 3.36 2.8 7.5 2.8s7.5-1.25 7.5-2.8v-13"/><path d="M4.5 12c0 1.55 3.36 2.8 7.5 2.8s7.5-1.25 7.5-2.8"/>"#
    }
    "cache" => r#"<polygon points="13,2 4,14 11,14 10,22 20,10 13,10"/>"#,
    "queue" => {
      r#"<rect x="3" y="5" width="4" height="14" rx="1"/><rect x="10" y="5" width="4" height="14" rx="1"/><rect x="17" y="5" width="4" height="14" rx="1"/>"#
    }
    "gateway" => {
      r#"<circle cx="12" cy="12" r="8"/><path d="M4 12h16M12 4a12 12 0 0 1 0 16M12 4a12 12 0 0 0 0 16"/>"#
    }
    "cloud" => {
      r#"<path d="M7 18h10.5a4 4 0 0 0 .7-7.93A5.5 5.5 0 0 0 7.4 8.2 3.8 3.8 0 0 0 7 18z"/>"#
    }
    "browser" => {
      r#"<rect x="3" y="4" width="18" height="16" rx="2"/><line x1="3" y1="9" x2="21" y2="9"/><circle cx="6.5" cy="6.5" r="0.8"/><circle cx="9.5" cy="6.5" r="0.8"/>"#
    }
    "mobile" => {
      r#"<rect x="6.5" y="2.5" width="11" height="19" rx="2"/><line x1="10.5" y1="18.5" x2="13.5" y2="18.5"/>"#
    }
    "user" => r#"<circle cx="12" cy="8" r="3.5"/><path d="M5 20c0-3.5 3.1-6 7-6s7 2.5 7 6"/>"#,
    "lock" => {
      r#"<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V7.5a4 4 0 0 1 8 0V11"/>"#
    }
    "api" => r#"<path d="M8 7L4 12l4 5M16 7l4 5-4 5M13.5 5l-3 14"/>"#,
    "container" => {
      r#"<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9L12 3z"/><path d="M12 12l8-4.5M12 12v9M12 12L4 7.5"/>"#
    }
    "storage" => {
      r#"<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M3 10h18M8 14.5h3"/>"#
    }
    "ai" => r#"<path d="M12 3l2.2 5.8L20 11l-5.8 2.2L12 19l-2.2-5.8L4 11l5.8-2.2L12 3z"/>"#,
    "monitor" => {
      r#"<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M7 12l2.5-3 2.5 2 3-4 2 2M8 20h8M12 16v4"/>"#
    }
    "cdn" => {
      r#"<circle cx="12" cy="12" r="3"/><circle cx="5" cy="6" r="2"/><circle cx="19" cy="6" r="2"/><circle cx="5" cy="18" r="2"/><circle cx="19" cy="18" r="2"/><path d="M7 7.5l3 2.5M17 7.5l-3 2.5M7 16.5l3-2.5M17 16.5l-3-2.5"/>"#
    }
    "lb" => {
      r#"<circle cx="5" cy="12" r="2.2"/><circle cx="19" cy="6" r="2.2"/><circle cx="19" cy="12" r="2.2"/><circle cx="19" cy="18" r="2.2"/><path d="M7.2 12H16.8M7.2 11.2L16.8 6.8M7.2 12.8L16.8 17.2"/>"#
    }
    "firewall" => r#"<path d="M12 3l7 3v5.5c0 4.5-3 8-7 9.5-4-1.5-7-5-7-9.5V6l7-3z"/>"#,
    "worker" => {
      r#"<circle cx="12" cy="12" r="3.2"/><path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M18.4 5.6l-2.1 2.1M7.7 16.3l-2.1 2.1"/>"#
    }
    "doc" => r#"<path d="M6 3h8l4 4v14H6V3z"/><path d="M14 3v4h4M9 11h6M9 15h6"/>"#,
    _ => return None,
  })
}
