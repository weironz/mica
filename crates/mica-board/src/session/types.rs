//! Serde-friendly command, patch, overlay, geometry, and gesture types for `BoardSession`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::{BoardEdge, BoardNode, BoardSnapshot, BoardStroke};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoardTool {
  #[default]
  Select,
  Hand,
  Shape,
  Icon,
  Sticky,
  Text,
  Connector,
  Pen,
  Frame,
  Mindmap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResizeHandle {
  Nw,
  N,
  Ne,
  E,
  Se,
  S,
  Sw,
  W,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
  pub x: f64,
  pub y: f64,
  pub w: f64,
  pub h: f64,
}

impl Rect {
  pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
    Self { x, y, w, h }
  }

  pub fn from_node(n: &BoardNode) -> Self {
    Self {
      x: n.x,
      y: n.y,
      w: n.w,
      h: n.h,
    }
  }

  pub fn from_points(a: [f64; 2], b: [f64; 2]) -> Self {
    let min_x = a[0].min(b[0]);
    let min_y = a[1].min(b[1]);
    let max_x = a[0].max(b[0]);
    let max_y = a[1].max(b[1]);
    Self {
      x: min_x,
      y: min_y,
      w: max_x - min_x,
      h: max_y - min_y,
    }
  }

  pub fn right(&self) -> f64 {
    self.x + self.w
  }

  pub fn bottom(&self) -> f64 {
    self.y + self.h
  }

  pub fn inflate(&self, delta: f64) -> Self {
    Self {
      x: self.x - delta,
      y: self.y - delta,
      w: self.w + delta * 2.0,
      h: self.h + delta * 2.0,
    }
  }

  pub fn deflate(&self, delta: f64) -> Self {
    self.inflate(-delta)
  }

  pub fn contains(&self, p: [f64; 2]) -> bool {
    if self.w <= 0.0 || self.h <= 0.0 {
      return false;
    }
    p[0] >= self.x && p[0] <= self.x + self.w && p[1] >= self.y && p[1] <= self.y + self.h
  }

  pub fn overlaps(&self, other: &Self) -> bool {
    if self.right() <= other.x || other.right() <= self.x {
      return false;
    }
    if self.bottom() <= other.y || other.bottom() <= self.y {
      return false;
    }
    true
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapGuide {
  pub vertical: bool,
  pub coord: f64,
  pub start: f64,
  pub end: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingConnector {
  pub from_node_id: String,
  pub from_anchor: String,
  pub from_world: [f64; 2],
  pub current_world: [f64; 2],
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub target_node_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub target_anchor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DragState {
  pub start_positions: HashMap<String, [f64; 2]>,
  pub total_delta: [f64; 2],
  pub last_world: Option<[f64; 2]>,
  pub before_snapshot: BoardSnapshot,
}

#[derive(Debug, Clone)]
pub struct ResizeState {
  pub node_id: String,
  pub handle: ResizeHandle,
  pub last_world: Option<[f64; 2]>,
  pub before_snapshot: BoardSnapshot,
}

#[derive(Debug, Clone)]
pub struct MarqueeState {
  pub start_world: [f64; 2],
  pub current_world: [f64; 2],
  pub rect: Rect,
}

#[derive(Debug, Clone)]
pub struct ActiveStrokeState {
  pub stroke: BoardStroke,
  pub before_snapshot: BoardSnapshot,
}

#[derive(Debug, Clone)]
pub struct PanState {
  pub last_screen: [f64; 2],
}

#[derive(Debug, Clone, Default)]
pub enum GestureState {
  #[default]
  Idle,
  Pan(PanState),
  Drag(DragState),
  Resize(ResizeState),
  Marquee(MarqueeState),
  Connector(PendingConnector),
  Stroke(ActiveStrokeState),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UiEffect {
  StartInlineEdit {
    #[serde(rename = "nodeId")]
    node_id: String,
    text: String,
  },
  PromptEdgeLabel {
    #[serde(rename = "edgeId")]
    edge_id: String,
    #[serde(rename = "currentLabel")]
    current_label: String,
  },
  CloseEditor,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDelta {
  #[serde(default)]
  pub full_replace: bool,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub upserted_nodes: Vec<BoardNode>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub removed_node_ids: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub upserted_edges: Vec<BoardEdge>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub removed_edge_ids: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub upserted_strokes: Vec<BoardStroke>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub removed_stroke_ids: Vec<String>,
  #[serde(default, skip_serializing_if = "HashMap::is_empty")]
  pub updated_routes: HashMap<String, Vec<[f64; 2]>>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub collapsed_descendants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveOverlay {
  pub selected_node_ids: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub selected_edge_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub hovered_node_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub editing_node_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub editing_edge_id: Option<String>,
  pub tool: BoardTool,
  pub active_shape_kind: String,
  pub active_icon_id: String,
  pub active_routing: String,
  pub is_dragging: bool,
  pub is_resizing: bool,
  pub is_panning: bool,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub snap_guides: Vec<SnapGuide>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub marquee_rect: Option<Rect>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pending_connector: Option<PendingConnector>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub active_stroke: Option<BoardStroke>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub dynamic_node_ids: Vec<String>,
  pub can_undo: bool,
  pub can_redo: bool,
  pub title: String,
  pub mode: String,
  pub node_count: usize,
  pub edge_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardPatch {
  pub graph_rev: u64,
  pub viewport_rev: u64,
  pub active_rev: u64,
  pub rebuild_chrome: bool,
  pub zoom: f64,
  pub pan: [f64; 2],
  pub snapshot_changed: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub delta: Option<SnapshotDelta>,
  pub active_overlay: ActiveOverlay,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub ui_effects: Vec<UiEffect>,
}

fn default_true() -> bool {
  true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BoardCommand {
  /// Pointer press on canvas. `button`: 0 = primary, 1 = middle, 2 = secondary.
  PointerDown {
    #[serde(rename = "screenX")]
    screen_x: f64,
    #[serde(rename = "screenY")]
    screen_y: f64,
    #[serde(default)]
    button: u8,
    #[serde(default)]
    shift: bool,
    #[serde(default)]
    alt: bool,
    #[serde(default)]
    space: bool,
    #[serde(default, rename = "readOnly")]
    read_only: bool,
    #[serde(default, rename = "timestampMs")]
    timestamp_ms: Option<u64>,
  },
  PointerMove {
    #[serde(rename = "screenX")]
    screen_x: f64,
    #[serde(rename = "screenY")]
    screen_y: f64,
    #[serde(default)]
    alt: bool,
  },
  PointerUp,
  PointerHover {
    #[serde(rename = "screenX")]
    screen_x: f64,
    #[serde(rename = "screenY")]
    screen_y: f64,
  },
  WheelZoom {
    factor: f64,
    #[serde(rename = "focalScreenX")]
    focal_screen_x: f64,
    #[serde(rename = "focalScreenY")]
    focal_screen_y: f64,
  },
  SetViewport {
    zoom: f64,
    pan: [f64; 2],
    #[serde(default)]
    viewport: Option<[f64; 2]>,
  },
  SetViewportSize {
    width: f64,
    height: f64,
  },
  FitToContent {
    #[serde(default)]
    viewport: Option<[f64; 2]>,
  },
  SetTool {
    tool: BoardTool,
  },
  SetActiveShape {
    kind: String,
  },
  SetActiveIcon {
    #[serde(rename = "iconId")]
    icon_id: String,
  },
  SetActiveRouting {
    routing: String,
  },
  SetPenStyle {
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    width: Option<f64>,
  },
  AddNode {
    #[serde(rename = "worldX")]
    world_x: f64,
    #[serde(rename = "worldY")]
    world_y: f64,
    kind: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
    #[serde(default)]
    fill: Option<String>,
    #[serde(default)]
    stroke: Option<String>,
    #[serde(default, rename = "startEditing")]
    start_editing: bool,
  },
  AddMindChild {
    #[serde(rename = "parentId")]
    parent_id: String,
    #[serde(default, rename = "startEditing")]
    start_editing: bool,
  },
  AddMindSibling {
    #[serde(rename = "nodeId")]
    node_id: String,
    #[serde(default, rename = "startEditing")]
    start_editing: bool,
  },
  ToggleCollapse {
    #[serde(rename = "nodeId")]
    node_id: String,
  },
  LayoutMindmap,
  SelectNode {
    id: String,
    #[serde(default)]
    additive: bool,
  },
  SelectEdge {
    id: String,
  },
  ClearSelection,
  SelectAll,
  StartEditingNode {
    #[serde(rename = "nodeId")]
    node_id: String,
  },
  SetNodeText {
    #[serde(rename = "nodeId")]
    node_id: String,
    text: String,
    #[serde(default, rename = "measuredHeight")]
    measured_height: Option<f64>,
  },
  FinishEditing,
  SetTitle {
    title: String,
  },
  SetEdgeLabel {
    #[serde(rename = "edgeId")]
    edge_id: String,
    label: String,
  },
  UpdateSelectedNodesStyle {
    #[serde(default)]
    fill: Option<String>,
    #[serde(default)]
    stroke: Option<String>,
    #[serde(default)]
    dashed: Option<bool>,
    #[serde(default, rename = "fontSize")]
    font_size: Option<f64>,
    #[serde(default, rename = "textColor")]
    text_color: Option<String>,
    #[serde(default)]
    align: Option<String>,
    #[serde(default)]
    kind: Option<String>,
  },
  UpdateSelectedEdge {
    #[serde(default)]
    routing: Option<String>,
    #[serde(default)]
    stroke: Option<String>,
    #[serde(default)]
    dashed: Option<bool>,
    #[serde(default, rename = "arrowEnd")]
    arrow_end: Option<String>,
    #[serde(default, rename = "arrowStart")]
    arrow_start: Option<String>,
  },
  BringToFront,
  SendToBack,
  CopySelection,
  PasteSelection {
    #[serde(default)]
    offset: Option<[f64; 2]>,
  },
  DuplicateSelection,
  DeleteSelection,
  Undo,
  Redo,
  ReplaceSnapshot {
    snapshot: BoardSnapshot,
    #[serde(default = "default_true", rename = "recordUndo")]
    record_undo: bool,
  },
  KeyDown {
    key: String,
    #[serde(default, rename = "ctrlOrMeta")]
    ctrl_or_meta: bool,
    #[serde(default)]
    shift: bool,
    #[serde(default, rename = "readOnly")]
    read_only: bool,
  },
}
