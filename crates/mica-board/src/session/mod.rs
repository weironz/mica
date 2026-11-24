//! Pure-Rust headless interactive state machine `BoardSession` for Mica's
//! infinite whiteboard, mind map, and flowchart editor.
//!
//! Zero UI / browser dependencies. Exposed via Serde JSON commands & patches
//! for direct passthrough over desktop FFI (`clients/mica_flutter/rust`) and
//! Web WASM (`crates/board-wasm`).

pub mod catalog;
mod hit_test;
mod ops;
pub mod types;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use crate::model::{BoardEdge, BoardNode, BoardSnapshot, BoardSvgTheme};
use crate::routing::route_edge;
use crate::svg::render_board_svg;

pub use catalog::{BOARD_SHAPES, BOARD_TECH_ICONS, ShapeMeta, TechIconMeta};
use catalog::{DEFAULT_PAN, DEFAULT_VIEWPORT, MAX_HISTORY};
pub use types::{
  ActiveOverlay, ActiveStrokeState, BoardCommand, BoardPatch, BoardTool, DragState, GestureState,
  MarqueeState, PanState, PendingConnector, Rect, ResizeHandle, ResizeState, SnapGuide,
  SnapshotDelta, UiEffect,
};

/// Headless interactive whiteboard & mind map session.
#[derive(Debug, Clone)]
pub struct BoardSession {
  pub(crate) snapshot: BoardSnapshot,
  pub(crate) node_index: HashMap<String, usize>,
  pub(crate) edges_by_node: HashMap<String, Vec<usize>>,
  pub(crate) collapsed_descendants: HashSet<String>,
  pub(crate) routed_edges: HashMap<String, Vec<[f64; 2]>>,
  pub(crate) id_counter: u64,

  // Viewport state.
  pub(crate) zoom: f64,
  pub(crate) pan: [f64; 2],
  pub(crate) viewport: [f64; 2],

  // Tool & creation defaults.
  pub(crate) tool: BoardTool,
  pub(crate) active_shape_kind: String,
  pub(crate) active_icon_id: String,
  pub(crate) active_routing: String,
  pub(crate) active_pen_color: String,
  pub(crate) active_pen_width: f64,

  // Selection & editing state.
  pub(crate) selected_node_ids: Vec<String>,
  pub(crate) selected_edge_id: Option<String>,
  pub(crate) hovered_node_id: Option<String>,
  pub(crate) editing_node_id: Option<String>,
  pub(crate) editing_edge_id: Option<String>,
  pub(crate) editing_before_snapshot: Option<BoardSnapshot>,

  // Active gesture & smart alignment guides.
  pub(crate) gesture: GestureState,
  pub(crate) snap_guides: Vec<SnapGuide>,
  pub(crate) last_tap: Option<(String, u64)>,

  // In-memory clipboard.
  pub(crate) clipboard_nodes: Vec<BoardNode>,
  pub(crate) clipboard_edges: Vec<BoardEdge>,

  // 100-step gesture-coalesced undo/redo history.
  pub(crate) undo_stack: Vec<BoardSnapshot>,
  pub(crate) redo_stack: Vec<BoardSnapshot>,

  // Two-layer monotonic dirty versions.
  pub(crate) graph_rev: u64,
  pub(crate) viewport_rev: u64,
  pub(crate) active_rev: u64,
}

impl BoardSession {
  /// Create a new interactive session from a `BoardSnapshot`.
  pub fn new(snapshot: BoardSnapshot) -> Self {
    let mut session = Self {
      snapshot,
      node_index: HashMap::new(),
      edges_by_node: HashMap::new(),
      collapsed_descendants: HashSet::new(),
      routed_edges: HashMap::new(),
      id_counter: 1000,
      zoom: 1.0,
      pan: DEFAULT_PAN,
      viewport: DEFAULT_VIEWPORT,
      tool: BoardTool::Select,
      active_shape_kind: "rounded".to_string(),
      active_icon_id: "server".to_string(),
      active_routing: "orthogonal".to_string(),
      active_pen_color: "#3370FF".to_string(),
      active_pen_width: 2.6,
      selected_node_ids: Vec::new(),
      selected_edge_id: None,
      hovered_node_id: None,
      editing_node_id: None,
      editing_edge_id: None,
      editing_before_snapshot: None,
      gesture: GestureState::Idle,
      snap_guides: Vec::new(),
      last_tap: None,
      clipboard_nodes: Vec::new(),
      clipboard_edges: Vec::new(),
      undo_stack: Vec::new(),
      redo_stack: Vec::new(),
      graph_rev: 0,
      viewport_rev: 0,
      active_rev: 0,
    };
    session.rebuild_indices_and_routes();
    session
  }

  /// Parse a board JSON string into a `BoardSession`.
  /// Falls back to `BoardSnapshot::template_whiteboard()` if JSON is invalid.
  pub fn from_json(json: &str) -> Self {
    let snap =
      BoardSnapshot::parse_json(json).unwrap_or_else(|_| BoardSnapshot::template_whiteboard());
    Self::new(snap)
  }

  /// Strict JSON constructor that returns a parse error on invalid JSON.
  pub fn try_from_json(json: &str) -> Result<Self, serde_json::Error> {
    let snap = BoardSnapshot::parse_json(json)?;
    Ok(Self::new(snap))
  }

  /// Export the current snapshot as compact JSON.
  pub fn export_compact_json(&self) -> String {
    self.snapshot.to_compact_json()
  }

  /// Render the current snapshot to a standalone SVG string.
  pub fn render_svg(&self, theme: Option<&BoardSvgTheme>) -> String {
    render_board_svg(&self.snapshot, theme)
  }

  /// Dispatch a typed command and return the resulting `BoardPatch`.
  pub fn dispatch(&mut self, cmd: BoardCommand) -> BoardPatch {
    self.execute_command(cmd)
  }

  /// Dispatch a JSON-encoded `BoardCommand` and return a JSON-encoded `BoardPatch`.
  pub fn dispatch_json(&mut self, cmd_json: &str) -> Result<String, String> {
    let cmd: BoardCommand =
      serde_json::from_str(cmd_json).map_err(|e| format!("invalid board command json: {e}"))?;
    let patch = self.dispatch(cmd);
    serde_json::to_string(&patch).map_err(|e| format!("failed to serialize board patch: {e}"))
  }

  /// High-frequency pointer-move fast path avoiding command JSON allocation.
  pub fn pointer_move(
    &mut self,
    world_x: f64,
    world_y: f64,
    screen_x: f64,
    screen_y: f64,
    alt_pressed: bool,
  ) -> BoardPatch {
    self.handle_pointer_move([world_x, world_y], [screen_x, screen_y], alt_pressed)
  }

  /// Snapshot of full current state as a hydration `BoardPatch`.
  pub fn current_state_patch(&self) -> BoardPatch {
    self.build_patch(true, Some(self.full_replace_delta()), true, Vec::new())
  }

  // --- Coordinate Conversions ---

  pub fn screen_to_world(&self, screen: [f64; 2]) -> [f64; 2] {
    [
      (screen[0] - self.pan[0]) / self.zoom,
      (screen[1] - self.pan[1]) / self.zoom,
    ]
  }

  pub fn world_to_screen(&self, world: [f64; 2]) -> [f64; 2] {
    [
      world[0] * self.zoom + self.pan[0],
      world[1] * self.zoom + self.pan[1],
    ]
  }

  // --- Read-Only Accessors ---

  pub fn snapshot(&self) -> &BoardSnapshot {
    &self.snapshot
  }

  pub fn routed_edges(&self) -> &HashMap<String, Vec<[f64; 2]>> {
    &self.routed_edges
  }

  pub fn collapsed_descendants(&self) -> &HashSet<String> {
    &self.collapsed_descendants
  }

  pub fn zoom(&self) -> f64 {
    self.zoom
  }

  pub fn pan(&self) -> [f64; 2] {
    self.pan
  }

  pub fn viewport(&self) -> [f64; 2] {
    self.viewport
  }

  pub fn tool(&self) -> BoardTool {
    self.tool
  }

  pub fn graph_rev(&self) -> u64 {
    self.graph_rev
  }

  pub fn viewport_rev(&self) -> u64 {
    self.viewport_rev
  }

  pub fn active_rev(&self) -> u64 {
    self.active_rev
  }

  pub fn selected_node_ids(&self) -> &[String] {
    &self.selected_node_ids
  }

  pub fn selected_edge_id(&self) -> Option<&str> {
    self.selected_edge_id.as_deref()
  }

  pub fn hovered_node_id(&self) -> Option<&str> {
    self.hovered_node_id.as_deref()
  }

  pub fn editing_node_id(&self) -> Option<&str> {
    self.editing_node_id.as_deref()
  }

  pub fn editing_edge_id(&self) -> Option<&str> {
    self.editing_edge_id.as_deref()
  }

  pub fn snap_guides(&self) -> &[SnapGuide] {
    &self.snap_guides
  }

  pub fn gesture(&self) -> &GestureState {
    &self.gesture
  }

  pub fn can_undo(&self) -> bool {
    !self.undo_stack.is_empty()
  }

  pub fn can_redo(&self) -> bool {
    !self.redo_stack.is_empty()
  }

  pub fn undo_len(&self) -> usize {
    self.undo_stack.len()
  }

  pub fn redo_len(&self) -> usize {
    self.redo_stack.len()
  }

  pub fn node_by_id(&self, id: &str) -> Option<&BoardNode> {
    self.node_index.get(id).map(|&i| &self.snapshot.nodes[i])
  }

  pub fn dynamic_node_ids(&self) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    match &self.gesture {
      GestureState::Drag(_) => {
        for id in &self.selected_node_ids {
          if !ids.contains(id) {
            ids.push(id.clone());
          }
        }
      }
      GestureState::Resize(rs) => {
        ids.push(rs.node_id.clone());
      }
      _ => {}
    }
    if let Some(edit_id) = &self.editing_node_id {
      if !ids.contains(edit_id) {
        ids.push(edit_id.clone());
      }
    }
    ids
  }

  // --- Internal Index, Routing, History & Patch Helpers ---

  pub(crate) fn next_id(&mut self, prefix: &str) -> String {
    loop {
      self.id_counter += 1;
      let cand = format!("{prefix}{}", self.id_counter);
      let taken = self.node_index.contains_key(&cand)
        || self.snapshot.edges.iter().any(|e| e.id == cand)
        || self.snapshot.strokes.iter().any(|s| s.id == cand);
      if !taken {
        return cand;
      }
    }
  }

  pub(crate) fn rebuild_indices(&mut self) {
    self.node_index.clear();
    self.edges_by_node.clear();
    for (idx, n) in self.snapshot.nodes.iter().enumerate() {
      self.node_index.insert(n.id.clone(), idx);
    }
    for (idx, e) in self.snapshot.edges.iter().enumerate() {
      self
        .edges_by_node
        .entry(e.from_id.clone())
        .or_default()
        .push(idx);
      if e.to_id != e.from_id {
        self
          .edges_by_node
          .entry(e.to_id.clone())
          .or_default()
          .push(idx);
      }
    }
    self.collapsed_descendants = compute_collapsed_descendants(&self.snapshot);
  }

  pub(crate) fn reroute_all_edges(&mut self) {
    self.routed_edges.clear();
    let node_map: HashMap<&str, &BoardNode> = self
      .snapshot
      .nodes
      .iter()
      .map(|n| (n.id.as_str(), n))
      .collect();
    for edge in &self.snapshot.edges {
      let pts = route_edge(edge, &self.snapshot.nodes, &node_map);
      self.routed_edges.insert(edge.id.clone(), pts);
    }
  }

  pub(crate) fn reroute_orthogonal_edges(&mut self) -> HashMap<String, Vec<[f64; 2]>> {
    let node_map: HashMap<&str, &BoardNode> = self
      .snapshot
      .nodes
      .iter()
      .map(|n| (n.id.as_str(), n))
      .collect();
    let mut updated = HashMap::new();
    for edge in &self.snapshot.edges {
      if edge.routing == "orthogonal" {
        let pts = route_edge(edge, &self.snapshot.nodes, &node_map);
        self.routed_edges.insert(edge.id.clone(), pts.clone());
        updated.insert(edge.id.clone(), pts);
      }
    }
    updated
  }

  pub(crate) fn reroute_incident_edges(
    &mut self,
    node_ids: &[String],
  ) -> HashMap<String, Vec<[f64; 2]>> {
    let mut edge_indices: Vec<usize> = Vec::new();
    for nid in node_ids {
      if let Some(list) = self.edges_by_node.get(nid) {
        for &ei in list {
          if !edge_indices.contains(&ei) {
            edge_indices.push(ei);
          }
        }
      }
    }
    let node_map: HashMap<&str, &BoardNode> = self
      .snapshot
      .nodes
      .iter()
      .map(|n| (n.id.as_str(), n))
      .collect();
    let mut updated = HashMap::new();
    for ei in edge_indices {
      if let Some(edge) = self.snapshot.edges.get(ei) {
        let pts = route_edge(edge, &self.snapshot.nodes, &node_map);
        self.routed_edges.insert(edge.id.clone(), pts.clone());
        updated.insert(edge.id.clone(), pts);
      }
    }
    updated
  }

  pub(crate) fn reroute_single_edge(&mut self, edge_id: &str) -> Option<Vec<[f64; 2]>> {
    let edge = self.snapshot.edges.iter().find(|e| e.id == edge_id)?;
    let node_map: HashMap<&str, &BoardNode> = self
      .snapshot
      .nodes
      .iter()
      .map(|n| (n.id.as_str(), n))
      .collect();
    let pts = route_edge(edge, &self.snapshot.nodes, &node_map);
    self.routed_edges.insert(edge_id.to_string(), pts.clone());
    Some(pts)
  }

  pub(crate) fn rebuild_indices_and_routes(&mut self) {
    self.rebuild_indices();
    self.reroute_all_edges();
  }

  pub(crate) fn push_undo(&mut self, pre_state: Option<BoardSnapshot>) {
    let snap = pre_state.unwrap_or_else(|| self.snapshot.clone());
    self.undo_stack.push(snap);
    if self.undo_stack.len() > MAX_HISTORY {
      self.undo_stack.remove(0);
    }
    self.redo_stack.clear();
  }

  pub(crate) fn sorted_collapsed_descendants(&self) -> Vec<String> {
    let mut list: Vec<String> = self.collapsed_descendants.iter().cloned().collect();
    list.sort();
    list
  }

  pub(crate) fn full_replace_delta(&self) -> SnapshotDelta {
    SnapshotDelta {
      full_replace: true,
      upserted_nodes: self.snapshot.nodes.clone(),
      removed_node_ids: Vec::new(),
      upserted_edges: self.snapshot.edges.clone(),
      removed_edge_ids: Vec::new(),
      upserted_strokes: self.snapshot.strokes.clone(),
      removed_stroke_ids: Vec::new(),
      updated_routes: self.routed_edges.clone(),
      collapsed_descendants: self.sorted_collapsed_descendants(),
    }
  }

  pub(crate) fn active_overlay(&self) -> ActiveOverlay {
    let (is_dragging, is_resizing, is_panning, marquee_rect, pending_connector, active_stroke) =
      match &self.gesture {
        GestureState::Idle => (false, false, false, None, None, None),
        GestureState::Pan(_) => (false, false, true, None, None, None),
        GestureState::Drag(_) => (true, false, false, None, None, None),
        GestureState::Resize(_) => (false, true, false, None, None, None),
        GestureState::Marquee(m) => (false, false, false, Some(m.rect), None, None),
        GestureState::Connector(pc) => (false, false, false, None, Some(pc.clone()), None),
        GestureState::Stroke(st) => (false, false, false, None, None, Some(st.stroke.clone())),
      };

    ActiveOverlay {
      selected_node_ids: self.selected_node_ids.clone(),
      selected_edge_id: self.selected_edge_id.clone(),
      hovered_node_id: self.hovered_node_id.clone(),
      editing_node_id: self.editing_node_id.clone(),
      editing_edge_id: self.editing_edge_id.clone(),
      tool: self.tool,
      active_shape_kind: self.active_shape_kind.clone(),
      active_icon_id: self.active_icon_id.clone(),
      active_routing: self.active_routing.clone(),
      is_dragging,
      is_resizing,
      is_panning,
      snap_guides: self.snap_guides.clone(),
      marquee_rect,
      pending_connector,
      active_stroke,
      dynamic_node_ids: self.dynamic_node_ids(),
      can_undo: self.can_undo(),
      can_redo: self.can_redo(),
      title: self.snapshot.title.clone(),
      mode: self.snapshot.mode.clone(),
      node_count: self.snapshot.nodes.len(),
      edge_count: self.snapshot.edges.len(),
    }
  }

  pub(crate) fn build_patch(
    &self,
    snapshot_changed: bool,
    delta: Option<SnapshotDelta>,
    rebuild_chrome: bool,
    ui_effects: Vec<UiEffect>,
  ) -> BoardPatch {
    BoardPatch {
      graph_rev: self.graph_rev,
      viewport_rev: self.viewport_rev,
      active_rev: self.active_rev,
      rebuild_chrome,
      zoom: self.zoom,
      pan: self.pan,
      snapshot_changed,
      delta,
      active_overlay: self.active_overlay(),
      ui_effects,
    }
  }

  pub(crate) fn noop_patch(&self) -> BoardPatch {
    self.build_patch(false, None, false, Vec::new())
  }
}

/// Compute set of node ids hidden beneath any collapsed mind-map ancestor.
pub fn compute_collapsed_descendants(snap: &BoardSnapshot) -> HashSet<String> {
  let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
  for n in &snap.nodes {
    if let Some(pid) = n.parent_id.as_deref() {
      children.entry(pid).or_default().push(n.id.as_str());
    }
  }
  let mut hidden: HashSet<String> = HashSet::new();
  let mut stack: Vec<&str> = Vec::new();
  for n in &snap.nodes {
    if n.collapsed {
      if let Some(kids) = children.get(n.id.as_str()) {
        stack.extend(kids.iter().copied());
      }
    }
  }
  while let Some(id) = stack.pop() {
    if hidden.insert(id.to_string()) {
      if let Some(kids) = children.get(id) {
        stack.extend(kids.iter().copied());
      }
    }
  }
  hidden
}
