//! Command execution, gesture state machine transitions, mind-map actions,
//! styling, clipboard, undo/redo, and viewport transforms for `BoardSession`.

use std::collections::{HashMap, HashSet};

use crate::layout::layout_mindmap;
use crate::model::{BoardEdge, BoardNode, BoardSnapshot, BoardStroke, EdgeStyle, NodeStyle};
use crate::routing::{Side, anchor_point};

use super::BoardSession;
use super::catalog::{
  DEFAULT_PASTE_OFFSET, DOUBLE_CLICK_MS, DRAG_MOVE_THRESHOLD, FIT_ZOOM_MAX, FIT_ZOOM_MIN,
  MIN_NODE_H, MIN_NODE_W, MIND_BRANCH_PALETTE, NODE_PADDING_V, PEN_MIN_DIST, SNAP_THRESHOLD,
  SNAP_THRESHOLD_MIN, ZOOM_MAX, ZOOM_MIN, shape_meta, tech_icon_meta,
};
use super::hit_test::{dist, node_anchor_by_name};
use super::types::{
  ActiveStrokeState, BoardCommand, BoardPatch, BoardTool, DragState, GestureState, MarqueeState,
  PanState, PendingConnector, Rect, ResizeHandle, ResizeState, SnapGuide, SnapshotDelta, UiEffect,
};

impl BoardSession {
  pub(crate) fn execute_command(&mut self, cmd: BoardCommand) -> BoardPatch {
    match cmd {
      BoardCommand::PointerDown {
        screen_x,
        screen_y,
        button,
        shift,
        alt: _,
        space,
        read_only,
        timestamp_ms,
      } => self.handle_pointer_down(
        [screen_x, screen_y],
        button,
        shift,
        space,
        read_only,
        timestamp_ms,
      ),
      BoardCommand::PointerMove {
        screen_x,
        screen_y,
        alt,
      } => {
        let world = self.screen_to_world([screen_x, screen_y]);
        self.handle_pointer_move(world, [screen_x, screen_y], alt)
      }
      BoardCommand::PointerUp => self.handle_pointer_up(),
      BoardCommand::PointerHover { screen_x, screen_y } => {
        self.handle_pointer_hover([screen_x, screen_y])
      }
      BoardCommand::WheelZoom {
        factor,
        focal_screen_x,
        focal_screen_y,
      } => self.zoom_around_cmd(factor, [focal_screen_x, focal_screen_y]),
      BoardCommand::SetViewport {
        zoom,
        pan,
        viewport,
      } => {
        if let Some(vp) = viewport {
          self.viewport = vp;
        }
        self.set_viewport_cmd(zoom, pan)
      }
      BoardCommand::SetViewportSize { width, height } => {
        if width > 0.0 && height > 0.0 {
          self.viewport = [width, height];
        }
        self.noop_patch()
      }
      BoardCommand::FitToContent { viewport } => self.fit_to_content_cmd(viewport),
      BoardCommand::SetTool { tool } => self.set_tool_cmd(tool),
      BoardCommand::SetActiveShape { kind } => {
        self.active_shape_kind = kind;
        self.active_rev += 1;
        self.build_patch(false, None, true, Vec::new())
      }
      BoardCommand::SetActiveIcon { icon_id } => {
        self.active_icon_id = icon_id;
        self.active_rev += 1;
        self.build_patch(false, None, true, Vec::new())
      }
      BoardCommand::SetActiveRouting { routing } => {
        self.active_routing = routing;
        self.active_rev += 1;
        self.build_patch(false, None, true, Vec::new())
      }
      BoardCommand::SetPenStyle { color, width } => {
        if let Some(c) = color {
          self.active_pen_color = c;
        }
        if let Some(w) = width {
          self.active_pen_width = w.max(0.5);
        }
        self.active_rev += 1;
        self.build_patch(false, None, true, Vec::new())
      }
      BoardCommand::AddNode {
        world_x,
        world_y,
        kind,
        icon,
        text,
        width,
        height,
        fill,
        stroke,
        start_editing,
      } => self.add_node_cmd(
        [world_x, world_y],
        &kind,
        icon,
        text,
        width,
        height,
        fill,
        stroke,
        start_editing,
      ),
      BoardCommand::AddMindChild {
        parent_id,
        start_editing,
      } => self.add_mind_child_cmd(&parent_id, start_editing),
      BoardCommand::AddMindSibling {
        node_id,
        start_editing,
      } => self.add_mind_sibling_cmd(&node_id, start_editing),
      BoardCommand::ToggleCollapse { node_id } => self.toggle_collapse_cmd(&node_id),
      BoardCommand::LayoutMindmap => self.layout_mindmap_cmd(),
      BoardCommand::SelectNode { id, additive } => self.select_node_cmd(&id, additive),
      BoardCommand::SelectEdge { id } => self.select_edge_cmd(&id),
      BoardCommand::ClearSelection => self.clear_selection_cmd(),
      BoardCommand::SelectAll => self.select_all_cmd(),
      BoardCommand::StartEditingNode { node_id } => self.start_editing_node_cmd(&node_id),
      BoardCommand::SetNodeText {
        node_id,
        text,
        measured_height,
      } => self.set_node_text_cmd(&node_id, text, measured_height),
      BoardCommand::FinishEditing => self.finish_editing_cmd(),
      BoardCommand::SetTitle { title } => self.set_title_cmd(&title),
      BoardCommand::SetEdgeLabel { edge_id, label } => self.set_edge_label_cmd(&edge_id, label),
      BoardCommand::UpdateSelectedNodesStyle {
        fill,
        stroke,
        dashed,
        font_size,
        text_color,
        align,
        kind,
      } => self
        .update_selected_nodes_style_cmd(fill, stroke, dashed, font_size, text_color, align, kind),
      BoardCommand::UpdateSelectedEdge {
        routing,
        stroke,
        dashed,
        arrow_end,
        arrow_start,
      } => self.update_selected_edge_cmd(routing, stroke, dashed, arrow_end, arrow_start),
      BoardCommand::BringToFront => self.bring_to_front_cmd(),
      BoardCommand::SendToBack => self.send_to_back_cmd(),
      BoardCommand::CopySelection => {
        self.copy_selection_internal();
        self.noop_patch()
      }
      BoardCommand::PasteSelection { offset } => {
        self.paste_selection_cmd(offset.unwrap_or(DEFAULT_PASTE_OFFSET))
      }
      BoardCommand::DuplicateSelection => {
        self.copy_selection_internal();
        self.paste_selection_cmd(DEFAULT_PASTE_OFFSET)
      }
      BoardCommand::DeleteSelection => self.delete_selection_cmd(),
      BoardCommand::Undo => self.undo_cmd(),
      BoardCommand::Redo => self.redo_cmd(),
      BoardCommand::ReplaceSnapshot {
        snapshot,
        record_undo,
      } => self.replace_snapshot_cmd(snapshot, record_undo),
      BoardCommand::KeyDown {
        key,
        ctrl_or_meta,
        shift,
        read_only,
      } => self.handle_key_down(&key, ctrl_or_meta, shift, read_only),
    }
  }

  // --- Viewport Operations ---

  pub fn set_viewport(&mut self, next_zoom: f64, next_pan: [f64; 2]) -> BoardPatch {
    self.set_viewport_cmd(next_zoom, next_pan)
  }

  pub fn zoom_around(&mut self, factor: f64, focal_screen: [f64; 2]) -> BoardPatch {
    self.zoom_around_cmd(factor, focal_screen)
  }

  pub fn fit_to_content(&mut self, viewport: Option<[f64; 2]>) -> BoardPatch {
    self.fit_to_content_cmd(viewport)
  }

  fn set_viewport_cmd(&mut self, next_zoom: f64, next_pan: [f64; 2]) -> BoardPatch {
    let clamped = next_zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    if (clamped - self.zoom).abs() < 1e-12
      && (next_pan[0] - self.pan[0]).abs() < 1e-12
      && (next_pan[1] - self.pan[1]).abs() < 1e-12
    {
      return self.noop_patch();
    }
    let zoom_changed = (clamped - self.zoom).abs() > 1e-4;
    self.zoom = clamped;
    self.pan = next_pan;
    self.viewport_rev += 1;
    self.active_rev += 1;
    let rebuild_chrome = zoom_changed || self.editing_node_id.is_some();
    self.build_patch(false, None, rebuild_chrome, Vec::new())
  }

  fn zoom_around_cmd(&mut self, factor: f64, focal_screen: [f64; 2]) -> BoardPatch {
    if !factor.is_finite() || factor <= 0.0 {
      return self.noop_patch();
    }
    let next_zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
    if (next_zoom - self.zoom).abs() < 1e-12 {
      return self.noop_patch();
    }
    let world_focal = self.screen_to_world(focal_screen);
    let next_pan = [
      focal_screen[0] - world_focal[0] * next_zoom,
      focal_screen[1] - world_focal[1] * next_zoom,
    ];
    self.set_viewport_cmd(next_zoom, next_pan)
  }

  fn fit_to_content_cmd(&mut self, viewport: Option<[f64; 2]>) -> BoardPatch {
    if let Some(vp) = viewport {
      self.viewport = vp;
    }
    let [vw, vh] = self.viewport;
    if vw < 40.0 || vh < 40.0 {
      return self.noop_patch();
    }
    let visible: Vec<&BoardNode> = self
      .snapshot
      .nodes
      .iter()
      .filter(|n| !self.collapsed_descendants.contains(&n.id))
      .collect();
    if visible.is_empty() && self.snapshot.strokes.is_empty() {
      return self.set_viewport_cmd(1.0, [vw * 0.2, vh * 0.2]);
    }
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for n in visible {
      min_x = min_x.min(n.x);
      min_y = min_y.min(n.y);
      max_x = max_x.max(n.x + n.w);
      max_y = max_y.max(n.y + n.h);
    }
    for s in &self.snapshot.strokes {
      for p in &s.points {
        min_x = min_x.min(p[0]);
        min_y = min_y.min(p[1]);
        max_x = max_x.max(p[0]);
        max_y = max_y.max(p[1]);
      }
    }
    if !min_x.is_finite() || !min_y.is_finite() {
      return self.set_viewport_cmd(1.0, [vw * 0.2, vh * 0.2]);
    }
    let content_w = (max_x - min_x).max(120.0);
    let content_h = (max_y - min_y).max(80.0);
    let avail_w = (vw - 160.0).max(160.0);
    let avail_h = (vh - 140.0).max(120.0);
    let next_zoom = (avail_w / content_w)
      .min(avail_h / content_h)
      .clamp(FIT_ZOOM_MIN, FIT_ZOOM_MAX);
    let cx = (min_x + max_x) / 2.0;
    let cy = (min_y + max_y) / 2.0;
    let next_pan = [vw / 2.0 - cx * next_zoom, vh / 2.0 - cy * next_zoom];
    self.set_viewport_cmd(next_zoom, next_pan)
  }

  // --- Pointer State Machine ---

  fn handle_pointer_down(
    &mut self,
    screen: [f64; 2],
    button: u8,
    shift: bool,
    space: bool,
    read_only: bool,
    timestamp_ms: Option<u64>,
  ) -> BoardPatch {
    let world = self.screen_to_world(screen);

    // Middle mouse button (1), Hand tool, Spacebar, or readOnly => pan.
    if button == 1 || self.tool == BoardTool::Hand || space || read_only {
      self.gesture = GestureState::Pan(PanState {
        last_screen: screen,
      });
      self.active_rev += 1;
      return self.build_patch(false, None, false, Vec::new());
    }
    if button != 0 {
      return self.noop_patch();
    }

    // Close active inline editor when clicking outside the editing node.
    let mut pre_delta: Option<SnapshotDelta> = None;
    if let Some(edit_id) = self.editing_node_id.clone() {
      if let Some(editing) = self.node_by_id(&edit_id) {
        if Rect::from_node(editing).contains(world) {
          return self.noop_patch();
        }
      }
      pre_delta = self.finish_editing_internal();
    }

    // Creation tools.
    match self.tool {
      BoardTool::Pen => {
        let mut patch = self.begin_pen_stroke(world);
        merge_pre_delta(&mut patch, pre_delta);
        return patch;
      }
      BoardTool::Shape => {
        let kind = self.active_shape_kind.clone();
        let mut patch = self.add_node_cmd(world, &kind, None, None, None, None, None, None, true);
        merge_pre_delta(&mut patch, pre_delta);
        return patch;
      }
      BoardTool::Icon => {
        let meta = tech_icon_meta(&self.active_icon_id);
        let mut patch = self.add_node_cmd(
          world,
          "icon_card",
          Some(meta.id.to_string()),
          Some(meta.label.to_string()),
          None,
          None,
          Some(meta.default_fill.to_string()),
          Some(meta.default_stroke.to_string()),
          true,
        );
        merge_pre_delta(&mut patch, pre_delta);
        return patch;
      }
      BoardTool::Sticky => {
        let mut patch =
          self.add_node_cmd(world, "sticky", None, None, None, None, None, None, true);
        merge_pre_delta(&mut patch, pre_delta);
        return patch;
      }
      BoardTool::Text => {
        let mut patch = self.add_node_cmd(world, "text", None, None, None, None, None, None, true);
        merge_pre_delta(&mut patch, pre_delta);
        return patch;
      }
      BoardTool::Frame => {
        let mut patch =
          self.add_node_cmd(world, "frame", None, None, None, None, None, None, false);
        merge_pre_delta(&mut patch, pre_delta);
        return patch;
      }
      BoardTool::Mindmap => {
        let mut patch = self.insert_mindmap_at(world);
        merge_pre_delta(&mut patch, pre_delta);
        return patch;
      }
      BoardTool::Select | BoardTool::Connector | BoardTool::Hand => {}
    }

    // 1. Mind-map collapse badge click.
    if let Some(collapse_id) = self.hit_test_collapse_badge(world) {
      let mut patch = self.toggle_collapse_cmd(&collapse_id);
      merge_pre_delta(&mut patch, pre_delta);
      return patch;
    }

    // 2. Connector port drag (works in both Select & Connector modes).
    if let Some((node_id, side)) = self.hit_test_anchor_port(world) {
      let mut patch = self.begin_connector_drag(&node_id, &side, world);
      merge_pre_delta(&mut patch, pre_delta);
      return patch;
    }

    // 3. Resize handle drag.
    if let Some((node_id, handle)) = self.hit_test_resize_handle(world) {
      let mut patch = self.begin_resize(&node_id, handle, world);
      merge_pre_delta(&mut patch, pre_delta);
      return patch;
    }

    // 4. Hit node.
    if let Some(hit_id) = self.hit_test_node(world).map(|n| n.id.clone()) {
      if self.tool == BoardTool::Connector {
        let mut patch = self.begin_connector_drag(&hit_id, "r", world);
        merge_pre_delta(&mut patch, pre_delta);
        return patch;
      }
      let is_double = self.check_and_record_tap(&hit_id, timestamp_ms);
      if is_double {
        let mut patch = self.start_editing_node_cmd(&hit_id);
        merge_pre_delta(&mut patch, pre_delta);
        return patch;
      }
      let mut patch = self.begin_node_drag(&hit_id, shift, world);
      merge_pre_delta(&mut patch, pre_delta);
      return patch;
    }

    // 5. Hit edge.
    if let Some((edge_id, edge_label)) = self
      .hit_test_edge(world)
      .map(|e| (e.id.clone(), e.label.clone()))
    {
      let is_double = self.check_and_record_tap(&edge_id, timestamp_ms);
      let mut patch = self.select_edge_cmd(&edge_id);
      if is_double {
        self.editing_edge_id = Some(edge_id.clone());
        patch.active_overlay.editing_edge_id = Some(edge_id.clone());
        patch.ui_effects.push(UiEffect::PromptEdgeLabel {
          edge_id,
          current_label: edge_label,
        });
      }
      merge_pre_delta(&mut patch, pre_delta);
      return patch;
    }

    // 6. Empty canvas -> rubber-band marquee selection.
    self.last_tap = None;
    let had_selection = !self.selected_node_ids.is_empty() || self.selected_edge_id.is_some();
    self.selected_node_ids.clear();
    self.selected_edge_id = None;
    self.gesture = GestureState::Marquee(MarqueeState {
      start_world: world,
      current_world: world,
      rect: Rect::from_points(world, world),
    });
    self.active_rev += 1;
    let mut patch = self.build_patch(
      false,
      None,
      had_selection || pre_delta.is_some(),
      Vec::new(),
    );
    merge_pre_delta(&mut patch, pre_delta);
    patch
  }

  fn check_and_record_tap(&mut self, target_id: &str, timestamp_ms: Option<u64>) -> bool {
    let Some(ts) = timestamp_ms else {
      self.last_tap = None;
      return false;
    };
    let is_double = matches!(
      &self.last_tap,
      Some((prev_id, prev_ts)) if prev_id == target_id && ts >= *prev_ts && ts - *prev_ts < DOUBLE_CLICK_MS
    );
    self.last_tap = Some((target_id.to_string(), ts));
    is_double
  }

  pub(crate) fn handle_pointer_move(
    &mut self,
    world: [f64; 2],
    screen: [f64; 2],
    alt_pressed: bool,
  ) -> BoardPatch {
    match &mut self.gesture {
      GestureState::Idle => self.noop_patch(),
      GestureState::Pan(pan_state) => {
        let dx = screen[0] - pan_state.last_screen[0];
        let dy = screen[1] - pan_state.last_screen[1];
        pan_state.last_screen = screen;
        let next_pan = [self.pan[0] + dx, self.pan[1] + dy];
        self.set_viewport_cmd(self.zoom, next_pan)
      }
      GestureState::Stroke(_) => self.update_pen_stroke(world),
      GestureState::Connector(_) => self.update_connector_drag(world),
      GestureState::Resize(_) => self.update_resize_to(world),
      GestureState::Drag(_) => self.update_node_drag_to(world, !alt_pressed),
      GestureState::Marquee(_) => self.update_marquee(world),
    }
  }

  fn handle_pointer_up(&mut self) -> BoardPatch {
    let gesture = std::mem::take(&mut self.gesture);
    match gesture {
      GestureState::Idle => self.noop_patch(),
      GestureState::Pan(_) => {
        self.active_rev += 1;
        self.build_patch(false, None, false, Vec::new())
      }
      GestureState::Stroke(st) => self.end_pen_stroke(st),
      GestureState::Connector(pc) => self.end_connector_drag(pc),
      GestureState::Resize(rs) => self.end_resize(rs),
      GestureState::Drag(ds) => self.end_node_drag(ds),
      GestureState::Marquee(_) => {
        self.active_rev += 1;
        self.build_patch(false, None, true, Vec::new())
      }
    }
  }

  fn handle_pointer_hover(&mut self, screen: [f64; 2]) -> BoardPatch {
    if matches!(self.gesture, GestureState::Drag(_) | GestureState::Pan(_)) {
      return self.noop_patch();
    }
    let world = self.screen_to_world(screen);
    let hit = self.hit_test_node(world).map(|n| n.id.clone());
    if self.hovered_node_id == hit {
      return self.noop_patch();
    }
    self.hovered_node_id = hit;
    self.active_rev += 1;
    self.build_patch(false, None, false, Vec::new())
  }

  // --- Dragging with Smart Alignment Guides ---

  fn begin_node_drag(&mut self, hit_id: &str, additive: bool, world: [f64; 2]) -> BoardPatch {
    if !self.selected_node_ids.iter().any(|id| id == hit_id) {
      self.selected_edge_id = None;
      if additive {
        if let Some(pos) = self.selected_node_ids.iter().position(|id| id == hit_id) {
          self.selected_node_ids.remove(pos);
        } else {
          self.selected_node_ids.push(hit_id.to_string());
        }
      } else {
        self.selected_node_ids.clear();
        self.selected_node_ids.push(hit_id.to_string());
      }
    }
    let mut start_positions = HashMap::new();
    for id in &self.selected_node_ids {
      if let Some(n) = self.node_by_id(id) {
        start_positions.insert(id.clone(), [n.x, n.y]);
      }
    }
    self.snap_guides.clear();
    self.gesture = GestureState::Drag(DragState {
      start_positions,
      total_delta: [0.0, 0.0],
      last_world: Some(world),
      before_snapshot: self.snapshot.clone(),
    });
    // Exclude dragged nodes from static layer once at drag start.
    self.graph_rev += 1;
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  fn update_node_drag_to(&mut self, world: [f64; 2], snap: bool) -> BoardPatch {
    if self.selected_node_ids.is_empty() {
      return self.noop_patch();
    }
    let world_delta = match &mut self.gesture {
      GestureState::Drag(ds) => {
        let delta = match ds.last_world {
          Some(prev) => [world[0] - prev[0], world[1] - prev[1]],
          None => [0.0, 0.0],
        };
        ds.last_world = Some(world);
        ds.total_delta[0] += delta[0];
        ds.total_delta[1] += delta[1];
        ds.total_delta
      }
      _ => return self.noop_patch(),
    };

    // Apply total_delta to original start positions (unsnapped) so slow movement
    // never sticks to an alignment guide.
    let start_positions = match &self.gesture {
      GestureState::Drag(ds) => ds.start_positions.clone(),
      _ => return self.noop_patch(),
    };
    for id in &self.selected_node_ids {
      if let (Some(&idx), Some(&origin)) = (self.node_index.get(id), start_positions.get(id)) {
        let n = &mut self.snapshot.nodes[idx];
        n.x = origin[0] + world_delta[0];
        n.y = origin[1] + world_delta[1];
      }
    }

    if snap && self.selected_node_ids.len() == 1 {
      let moving_id = self.selected_node_ids[0].clone();
      self.apply_smart_snap(&moving_id);
    } else {
      self.snap_guides.clear();
    }

    let moved_ids = self.selected_node_ids.clone();
    let updated_routes = self.reroute_incident_edges(&moved_ids);
    let upserted_nodes: Vec<BoardNode> = moved_ids
      .iter()
      .filter_map(|id| self.node_by_id(id).cloned())
      .collect();
    let delta = SnapshotDelta {
      upserted_nodes,
      updated_routes,
      ..Default::default()
    };

    // High-frequency active-only update: graph_rev stays unchanged, rebuild_chrome = false.
    self.active_rev += 1;
    self.build_patch(true, Some(delta), false, Vec::new())
  }

  fn apply_smart_snap(&mut self, moving_id: &str) {
    let Some(&moving_idx) = self.node_index.get(moving_id) else {
      self.snap_guides.clear();
      return;
    };
    let moving = self.snapshot.nodes[moving_idx].clone();
    let threshold = (SNAP_THRESHOLD / self.zoom).clamp(SNAP_THRESHOLD_MIN, SNAP_THRESHOLD);
    let mut snap_dx: Option<f64> = None;
    let mut snap_dy: Option<f64> = None;
    let mut best_dx = threshold;
    let mut best_dy = threshold;
    let mut v_guide: Option<SnapGuide> = None;
    let mut h_guide: Option<SnapGuide> = None;

    let m_xs = [moving.x, moving.x + moving.w / 2.0, moving.x + moving.w];
    let m_ys = [moving.y, moving.y + moving.h / 2.0, moving.y + moving.h];

    for other in &self.snapshot.nodes {
      if other.id == moving.id || self.collapsed_descendants.contains(&other.id) {
        continue;
      }
      // Match corresponding edges only (left-left, center-center, right-right;
      // top-top, center-center, bottom-bottom) to avoid bumpy false snaps.
      let o_xs = [other.x, other.x + other.w / 2.0, other.x + other.w];
      for i in 0..3 {
        let d = (o_xs[i] - m_xs[i]).abs();
        if d < best_dx {
          best_dx = d;
          snap_dx = Some(o_xs[i] - m_xs[i]);
          v_guide = Some(SnapGuide {
            vertical: true,
            coord: o_xs[i],
            start: moving.y.min(other.y) - 16.0,
            end: (moving.y + moving.h).max(other.y + other.h) + 16.0,
          });
        }
      }
      let o_ys = [other.y, other.y + other.h / 2.0, other.y + other.h];
      for i in 0..3 {
        let d = (o_ys[i] - m_ys[i]).abs();
        if d < best_dy {
          best_dy = d;
          snap_dy = Some(o_ys[i] - m_ys[i]);
          h_guide = Some(SnapGuide {
            vertical: false,
            coord: o_ys[i],
            start: moving.x.min(other.x) - 16.0,
            end: (moving.x + moving.w).max(other.x + other.w) + 16.0,
          });
        }
      }
    }

    let mut guides = Vec::new();
    let target = &mut self.snapshot.nodes[moving_idx];
    if let Some(dx) = snap_dx {
      target.x += dx;
      if let Some(g) = v_guide {
        guides.push(g);
      }
    }
    if let Some(dy) = snap_dy {
      target.y += dy;
      if let Some(g) = h_guide {
        guides.push(g);
      }
    }
    self.snap_guides = guides;
  }

  fn end_node_drag(&mut self, ds: DragState) -> BoardPatch {
    self.snap_guides.clear();
    let moved = ds.total_delta[0].hypot(ds.total_delta[1]) > DRAG_MOVE_THRESHOLD;
    if moved {
      self.push_undo(Some(ds.before_snapshot));
    }
    let updated_routes = self.reroute_orthogonal_edges();
    let delta = if !updated_routes.is_empty() {
      Some(SnapshotDelta {
        updated_routes,
        ..Default::default()
      })
    } else {
      None
    };
    self.graph_rev += 1;
    self.active_rev += 1;
    self.build_patch(delta.is_some(), delta, true, Vec::new())
  }

  // --- 8-Direction Resizing ---

  fn begin_resize(&mut self, node_id: &str, handle: ResizeHandle, world: [f64; 2]) -> BoardPatch {
    self.gesture = GestureState::Resize(ResizeState {
      node_id: node_id.to_string(),
      handle,
      last_world: Some(world),
      before_snapshot: self.snapshot.clone(),
    });
    self.graph_rev += 1;
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  fn update_resize_to(&mut self, world: [f64; 2]) -> BoardPatch {
    let (node_id, handle, delta) = match &mut self.gesture {
      GestureState::Resize(rs) => {
        let d = match rs.last_world {
          Some(prev) => [world[0] - prev[0], world[1] - prev[1]],
          None => [0.0, 0.0],
        };
        rs.last_world = Some(world);
        (rs.node_id.clone(), rs.handle, d)
      }
      _ => return self.noop_patch(),
    };
    let Some(&idx) = self.node_index.get(&node_id) else {
      return self.noop_patch();
    };
    let n = &mut self.snapshot.nodes[idx];
    match handle {
      ResizeHandle::E => {
        n.w = (n.w + delta[0]).max(MIN_NODE_W);
      }
      ResizeHandle::S => {
        n.h = (n.h + delta[1]).max(MIN_NODE_H);
      }
      ResizeHandle::Se => {
        n.w = (n.w + delta[0]).max(MIN_NODE_W);
        n.h = (n.h + delta[1]).max(MIN_NODE_H);
      }
      ResizeHandle::W => {
        let nw = (n.w - delta[0]).max(MIN_NODE_W);
        n.x += n.w - nw;
        n.w = nw;
      }
      ResizeHandle::N => {
        let nh = (n.h - delta[1]).max(MIN_NODE_H);
        n.y += n.h - nh;
        n.h = nh;
      }
      ResizeHandle::Nw => {
        let nw = (n.w - delta[0]).max(MIN_NODE_W);
        let nh = (n.h - delta[1]).max(MIN_NODE_H);
        n.x += n.w - nw;
        n.y += n.h - nh;
        n.w = nw;
        n.h = nh;
      }
      ResizeHandle::Ne => {
        let nh = (n.h - delta[1]).max(MIN_NODE_H);
        n.y += n.h - nh;
        n.h = nh;
        n.w = (n.w + delta[0]).max(MIN_NODE_W);
      }
      ResizeHandle::Sw => {
        let nw = (n.w - delta[0]).max(MIN_NODE_W);
        n.x += n.w - nw;
        n.w = nw;
        n.h = (n.h + delta[1]).max(MIN_NODE_H);
      }
    }
    let updated_node = n.clone();
    let updated_routes = self.reroute_incident_edges(std::slice::from_ref(&node_id));
    let snapshot_delta = SnapshotDelta {
      upserted_nodes: vec![updated_node],
      updated_routes,
      ..Default::default()
    };
    self.active_rev += 1;
    self.build_patch(true, Some(snapshot_delta), false, Vec::new())
  }

  fn end_resize(&mut self, rs: ResizeState) -> BoardPatch {
    if rs.before_snapshot != self.snapshot {
      self.push_undo(Some(rs.before_snapshot));
    }
    let updated_routes = self.reroute_orthogonal_edges();
    let delta = if !updated_routes.is_empty() {
      Some(SnapshotDelta {
        updated_routes,
        ..Default::default()
      })
    } else {
      None
    };
    self.graph_rev += 1;
    self.active_rev += 1;
    self.build_patch(delta.is_some(), delta, true, Vec::new())
  }

  // --- Marquee Selection ---

  fn update_marquee(&mut self, world: [f64; 2]) -> BoardPatch {
    let rect = match &mut self.gesture {
      GestureState::Marquee(m) => {
        m.current_world = world;
        m.rect = Rect::from_points(m.start_world, world);
        m.rect
      }
      _ => return self.noop_patch(),
    };
    self.selected_node_ids.clear();
    self.selected_edge_id = None;
    for n in &self.snapshot.nodes {
      if self.collapsed_descendants.contains(&n.id) {
        continue;
      }
      if rect.overlaps(&Rect::from_node(n)) {
        self.selected_node_ids.push(n.id.clone());
      }
    }
    self.active_rev += 1;
    self.build_patch(false, None, false, Vec::new())
  }

  // --- Connector Drawing ---

  fn begin_connector_drag(
    &mut self,
    from_id: &str,
    from_anchor: &str,
    world: [f64; 2],
  ) -> BoardPatch {
    let Some(from_node) = self.node_by_id(from_id) else {
      return self.noop_patch();
    };
    let from_world = node_anchor_by_name(from_node, from_anchor);
    self.gesture = GestureState::Connector(PendingConnector {
      from_node_id: from_id.to_string(),
      from_anchor: from_anchor.to_string(),
      from_world,
      current_world: world,
      target_node_id: None,
      target_anchor: None,
    });
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  fn update_connector_drag(&mut self, world: [f64; 2]) -> BoardPatch {
    let from_id = match &self.gesture {
      GestureState::Connector(pc) => pc.from_node_id.clone(),
      _ => return self.noop_patch(),
    };
    let snap_target = self.hit_test_node(world).and_then(|hit| {
      if hit.id == from_id || hit.kind == "frame" {
        return None;
      }
      let mut best_side = "l";
      let mut best_pt = anchor_point(hit, Side::Left);
      let mut best_dist = f64::INFINITY;
      for (side_str, side) in [
        ("t", Side::Top),
        ("r", Side::Right),
        ("b", Side::Bottom),
        ("l", Side::Left),
      ] {
        let pt = anchor_point(hit, side);
        let d = dist(pt, world);
        if d < best_dist {
          best_dist = d;
          best_side = side_str;
          best_pt = pt;
        }
      }
      Some((hit.id.clone(), best_side.to_string(), best_pt))
    });

    if let GestureState::Connector(pc) = &mut self.gesture {
      if let Some((tid, tside, tpt)) = snap_target {
        pc.target_node_id = Some(tid);
        pc.target_anchor = Some(tside);
        pc.current_world = tpt;
      } else {
        pc.target_node_id = None;
        pc.target_anchor = None;
        pc.current_world = world;
      }
    }
    self.active_rev += 1;
    self.build_patch(false, None, false, Vec::new())
  }

  fn end_connector_drag(&mut self, pc: PendingConnector) -> BoardPatch {
    if let Some(target_id) = pc.target_node_id {
      self.push_undo(None);
      let edge_id = self.next_id("e");
      let edge = BoardEdge {
        id: edge_id.clone(),
        from_id: pc.from_node_id,
        to_id: target_id,
        from_anchor: pc.from_anchor,
        to_anchor: pc.target_anchor.unwrap_or_else(|| "auto".to_string()),
        routing: self.active_routing.clone(),
        arrow_start: "none".to_string(),
        arrow_end: "arrow".to_string(),
        waypoints: Vec::new(),
        label: String::new(),
        style: EdgeStyle::default(),
      };
      self.snapshot.edges.push(edge.clone());
      self.rebuild_indices();
      let mut updated_routes = HashMap::new();
      if let Some(pts) = self.reroute_single_edge(&edge_id) {
        updated_routes.insert(edge_id.clone(), pts);
      }
      self.selected_node_ids.clear();
      self.selected_edge_id = Some(edge_id);
      self.graph_rev += 1;
      self.active_rev += 1;
      let delta = SnapshotDelta {
        upserted_edges: vec![edge],
        updated_routes,
        ..Default::default()
      };
      return self.build_patch(true, Some(delta), true, Vec::new());
    }
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  // --- Freehand Pen Strokes ---

  fn begin_pen_stroke(&mut self, world: [f64; 2]) -> BoardPatch {
    let before_snapshot = self.snapshot.clone();
    let stroke = BoardStroke {
      id: self.next_id("s"),
      color: self.active_pen_color.clone(),
      width: self.active_pen_width,
      points: vec![world],
    };
    self.gesture = GestureState::Stroke(ActiveStrokeState {
      stroke,
      before_snapshot,
    });
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  fn update_pen_stroke(&mut self, world: [f64; 2]) -> BoardPatch {
    let added = match &mut self.gesture {
      GestureState::Stroke(st) => {
        let should_add = st
          .stroke
          .points
          .last()
          .is_none_or(|&last| dist(last, world) >= PEN_MIN_DIST);
        if should_add {
          st.stroke.points.push(world);
        }
        should_add
      }
      _ => return self.noop_patch(),
    };
    if !added {
      return self.noop_patch();
    }
    self.active_rev += 1;
    self.build_patch(false, None, false, Vec::new())
  }

  fn end_pen_stroke(&mut self, st: ActiveStrokeState) -> BoardPatch {
    if st.stroke.points.len() >= 2 {
      self.push_undo(Some(st.before_snapshot));
      let stroke = st.stroke;
      self.snapshot.strokes.push(stroke.clone());
      self.graph_rev += 1;
      self.active_rev += 1;
      let delta = SnapshotDelta {
        upserted_strokes: vec![stroke],
        ..Default::default()
      };
      return self.build_patch(true, Some(delta), true, Vec::new());
    }
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  // --- Creation Helpers ---

  #[allow(clippy::too_many_arguments)]
  fn add_node_cmd(
    &mut self,
    world_center: [f64; 2],
    kind: &str,
    icon: Option<String>,
    text: Option<String>,
    width: Option<f64>,
    height: Option<f64>,
    fill: Option<String>,
    stroke: Option<String>,
    start_editing: bool,
  ) -> BoardPatch {
    self.push_undo(None);
    let node = self.create_node_struct(world_center, kind, icon, text, width, height, fill, stroke);
    let node_id = node.id.clone();
    self.snapshot.nodes.push(node.clone());
    self.rebuild_indices();
    self.selected_node_ids.clear();
    self.selected_node_ids.push(node_id.clone());
    self.selected_edge_id = None;
    self.tool = BoardTool::Select;
    self.graph_rev += 1;
    self.active_rev += 1;

    let mut effects = Vec::new();
    if start_editing {
      self.editing_edge_id = None;
      self.editing_node_id = Some(node_id.clone());
      self.editing_before_snapshot = Some(self.snapshot.clone());
      effects.push(UiEffect::StartInlineEdit {
        node_id,
        text: node.text.clone(),
      });
    }

    let delta = SnapshotDelta {
      upserted_nodes: vec![node],
      collapsed_descendants: self.sorted_collapsed_descendants(),
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, effects)
  }

  #[allow(clippy::too_many_arguments)]
  fn create_node_struct(
    &mut self,
    world_center: [f64; 2],
    kind: &str,
    icon: Option<String>,
    text: Option<String>,
    width: Option<f64>,
    height: Option<f64>,
    fill: Option<String>,
    stroke: Option<String>,
  ) -> BoardNode {
    let meta = shape_meta(kind);
    let w = width.unwrap_or(match kind {
      "frame" => 380.0,
      "text" => 180.0,
      "icon_card" => 128.0,
      _ => meta.default_w,
    });
    let h = height.unwrap_or(match kind {
      "frame" => 240.0,
      "text" => 40.0,
      "icon_card" => 86.0,
      _ => meta.default_h,
    });
    let max_z = self.snapshot.nodes.iter().map(|n| n.z).max().unwrap_or(0);
    let default_text = match kind {
      "frame" => "分区 Frame",
      "text" => "输入文字",
      "sticky" => "便签内容",
      "mind_root" => "中心主题",
      _ => meta.label,
    };
    let default_fill = match kind {
      "text" => "transparent",
      "frame" => "#F7F8FA",
      "mind_root" => "#3370FF",
      _ => meta.default_fill,
    };
    let default_stroke = match kind {
      "text" => "transparent",
      "frame" => "#C2C8D0",
      "mind_root" => "#245BDB",
      _ => meta.default_stroke,
    };
    let align = if matches!(kind, "sticky" | "text" | "frame") {
      "left"
    } else {
      "center"
    };
    BoardNode {
      id: self.next_id("n"),
      kind: kind.to_string(),
      x: world_center[0] - w / 2.0,
      y: world_center[1] - h / 2.0,
      w,
      h,
      rot: 0.0,
      z: if kind == "frame" { 0 } else { max_z + 1 },
      parent_id: None,
      rank: 0,
      collapsed: false,
      text: text.unwrap_or_else(|| default_text.to_string()),
      icon,
      style: NodeStyle {
        fill: fill.unwrap_or_else(|| default_fill.to_string()),
        stroke: stroke.unwrap_or_else(|| default_stroke.to_string()),
        dashed: kind == "frame",
        text_color: if kind == "mind_root" {
          "#FFFFFF".to_string()
        } else {
          "#1F2329".to_string()
        },
        align: align.to_string(),
        ..Default::default()
      },
    }
  }

  fn insert_mindmap_at(&mut self, world_center: [f64; 2]) -> BoardPatch {
    self.push_undo(None);
    let root = self.create_node_struct(
      world_center,
      "mind_root",
      None,
      None,
      None,
      None,
      None,
      None,
    );
    let root_id = root.id.clone();
    self.snapshot.nodes.push(root);
    self.rebuild_indices();
    let c1 = self.push_mind_child_internal(&root_id);
    let c2 = self.push_mind_child_internal(&root_id);
    layout_mindmap(&mut self.snapshot);
    self.rebuild_indices_and_routes();
    self.selected_node_ids.clear();
    self.selected_node_ids.push(root_id.clone());
    self.selected_edge_id = None;
    self.tool = BoardTool::Select;
    self.graph_rev += 1;
    self.active_rev += 1;

    let mut upserted_nodes = Vec::new();
    if let Some(n) = self.node_by_id(&root_id).cloned() {
      upserted_nodes.push(n);
    }
    let mut upserted_edges = Vec::new();
    for pair in [c1, c2].into_iter().flatten() {
      if let Some(n) = self.node_by_id(&pair.0).cloned() {
        upserted_nodes.push(n);
      }
      if let Some(e) = self.snapshot.edges.iter().find(|e| e.id == pair.1).cloned() {
        upserted_edges.push(e);
      }
    }
    let delta = SnapshotDelta {
      upserted_nodes,
      upserted_edges,
      updated_routes: self.routed_edges.clone(),
      collapsed_descendants: self.sorted_collapsed_descendants(),
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  // --- Mind Map Workflow ---

  fn push_mind_child_internal(&mut self, parent_id: &str) -> Option<(String, String)> {
    let &parent_idx = self.node_index.get(parent_id)?;
    self.snapshot.nodes[parent_idx].collapsed = false;
    let parent_kind = self.snapshot.nodes[parent_idx].kind.clone();
    let parent_has_parent = self.snapshot.nodes[parent_idx].parent_id.is_some();
    let parent_x = self.snapshot.nodes[parent_idx].x;
    let parent_y = self.snapshot.nodes[parent_idx].y;
    let parent_w = self.snapshot.nodes[parent_idx].w;
    let parent_stroke = self.snapshot.nodes[parent_idx].style.stroke.clone();

    let existing_kids: Vec<&BoardNode> = self
      .snapshot
      .nodes
      .iter()
      .filter(|n| n.parent_id.as_deref() == Some(parent_id))
      .collect();
    let next_rank = existing_kids.iter().map(|n| n.rank + 1).max().unwrap_or(0);
    let (fill, stroke) = if parent_kind == "mind_root" || !parent_has_parent {
      let (f, s) = MIND_BRANCH_PALETTE[existing_kids.len() % MIND_BRANCH_PALETTE.len()];
      (f.to_string(), s.to_string())
    } else {
      ("#FFFFFF".to_string(), parent_stroke)
    };

    let child_id = self.next_id("m");
    let edge_id = self.next_id("me");
    let z = self.snapshot.nodes.len() as i32 + 2;
    let child = BoardNode {
      id: child_id.clone(),
      kind: "mind_node".to_string(),
      x: parent_x + parent_w + 68.0,
      y: parent_y,
      w: 148.0,
      h: 40.0,
      rot: 0.0,
      z,
      parent_id: Some(parent_id.to_string()),
      rank: next_rank,
      collapsed: false,
      text: "子主题".to_string(),
      icon: None,
      style: NodeStyle {
        fill,
        stroke: stroke.clone(),
        stroke_width: 1.5,
        align: "left".to_string(),
        ..Default::default()
      },
    };
    let edge = BoardEdge {
      id: edge_id.clone(),
      from_id: parent_id.to_string(),
      to_id: child_id.clone(),
      from_anchor: "r".to_string(),
      to_anchor: "l".to_string(),
      routing: "curved".to_string(),
      arrow_start: "none".to_string(),
      arrow_end: "none".to_string(),
      waypoints: Vec::new(),
      label: String::new(),
      style: EdgeStyle {
        stroke,
        stroke_width: 1.7,
        dashed: false,
      },
    };
    self.snapshot.nodes.push(child);
    self.snapshot.edges.push(edge);
    self.rebuild_indices();
    Some((child_id, edge_id))
  }

  fn add_mind_child_cmd(&mut self, parent_id: &str, start_editing: bool) -> BoardPatch {
    if !self.node_index.contains_key(parent_id) {
      return self.noop_patch();
    }
    self.push_undo(None);
    let Some((child_id, edge_id)) = self.push_mind_child_internal(parent_id) else {
      return self.noop_patch();
    };
    layout_mindmap(&mut self.snapshot);
    self.rebuild_indices_and_routes();
    self.selected_node_ids.clear();
    self.selected_node_ids.push(child_id.clone());
    self.selected_edge_id = None;
    self.graph_rev += 1;
    self.active_rev += 1;

    let mut effects = Vec::new();
    if start_editing {
      self.editing_edge_id = None;
      self.editing_node_id = Some(child_id.clone());
      self.editing_before_snapshot = Some(self.snapshot.clone());
      let text = self
        .node_by_id(&child_id)
        .map(|n| n.text.clone())
        .unwrap_or_default();
      effects.push(UiEffect::StartInlineEdit {
        node_id: child_id,
        text,
      });
    }

    let upserted_edges = self
      .snapshot
      .edges
      .iter()
      .find(|e| e.id == edge_id)
      .cloned()
      .into_iter()
      .collect();
    let delta = SnapshotDelta {
      upserted_nodes: self.snapshot.nodes.clone(),
      upserted_edges,
      updated_routes: self.routed_edges.clone(),
      collapsed_descendants: self.sorted_collapsed_descendants(),
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, effects)
  }

  fn add_mind_sibling_cmd(&mut self, node_id: &str, start_editing: bool) -> BoardPatch {
    let Some(node) = self.node_by_id(node_id) else {
      return self.noop_patch();
    };
    let target_parent = match node.parent_id.as_deref() {
      Some(pid) if self.node_index.contains_key(pid) => pid.to_string(),
      _ => node_id.to_string(),
    };
    self.add_mind_child_cmd(&target_parent, start_editing)
  }

  fn toggle_collapse_cmd(&mut self, node_id: &str) -> BoardPatch {
    let Some(&idx) = self.node_index.get(node_id) else {
      return self.noop_patch();
    };
    self.push_undo(None);
    self.snapshot.nodes[idx].collapsed = !self.snapshot.nodes[idx].collapsed;
    layout_mindmap(&mut self.snapshot);
    self.rebuild_indices_and_routes();
    self
      .selected_node_ids
      .retain(|id| !self.collapsed_descendants.contains(id));
    self.graph_rev += 1;
    self.active_rev += 1;
    let delta = SnapshotDelta {
      upserted_nodes: self.snapshot.nodes.clone(),
      updated_routes: self.routed_edges.clone(),
      collapsed_descendants: self.sorted_collapsed_descendants(),
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  fn layout_mindmap_cmd(&mut self) -> BoardPatch {
    self.push_undo(None);
    layout_mindmap(&mut self.snapshot);
    self.rebuild_indices_and_routes();
    self.graph_rev += 1;
    self.active_rev += 1;
    let delta = SnapshotDelta {
      upserted_nodes: self.snapshot.nodes.clone(),
      updated_routes: self.routed_edges.clone(),
      collapsed_descendants: self.sorted_collapsed_descendants(),
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  // --- Selection, Tool, Inline Editing ---

  fn set_tool_cmd(&mut self, next: BoardTool) -> BoardPatch {
    if self.tool == next {
      return self.noop_patch();
    }
    self.tool = next;
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  fn select_node_cmd(&mut self, id: &str, additive: bool) -> BoardPatch {
    if !self.node_index.contains_key(id) {
      return self.noop_patch();
    }
    self.selected_edge_id = None;
    if additive {
      if let Some(pos) = self.selected_node_ids.iter().position(|x| x == id) {
        self.selected_node_ids.remove(pos);
      } else {
        self.selected_node_ids.push(id.to_string());
      }
    } else {
      self.selected_node_ids.clear();
      self.selected_node_ids.push(id.to_string());
    }
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  fn select_edge_cmd(&mut self, id: &str) -> BoardPatch {
    if !self.snapshot.edges.iter().any(|e| e.id == id) {
      return self.noop_patch();
    }
    self.selected_node_ids.clear();
    self.selected_edge_id = Some(id.to_string());
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  fn clear_selection_cmd(&mut self) -> BoardPatch {
    if self.selected_node_ids.is_empty() && self.selected_edge_id.is_none() {
      return self.noop_patch();
    }
    self.selected_node_ids.clear();
    self.selected_edge_id = None;
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  fn select_all_cmd(&mut self) -> BoardPatch {
    self.selected_node_ids = self
      .snapshot
      .nodes
      .iter()
      .filter(|n| !self.collapsed_descendants.contains(&n.id))
      .map(|n| n.id.clone())
      .collect();
    self.selected_edge_id = None;
    self.active_rev += 1;
    self.build_patch(false, None, true, Vec::new())
  }

  fn start_editing_node_cmd(&mut self, node_id: &str) -> BoardPatch {
    let Some(node) = self.node_by_id(node_id) else {
      return self.noop_patch();
    };
    let text = node.text.clone();
    self.editing_edge_id = None;
    self.editing_node_id = Some(node_id.to_string());
    self.selected_node_ids.clear();
    self.selected_node_ids.push(node_id.to_string());
    self.selected_edge_id = None;
    self.editing_before_snapshot = Some(self.snapshot.clone());
    self.graph_rev += 1;
    self.active_rev += 1;
    self.build_patch(
      false,
      None,
      true,
      vec![UiEffect::StartInlineEdit {
        node_id: node_id.to_string(),
        text,
      }],
    )
  }

  fn set_node_text_cmd(
    &mut self,
    node_id: &str,
    next_text: String,
    measured_height: Option<f64>,
  ) -> BoardPatch {
    let Some(&idx) = self.node_index.get(node_id) else {
      return self.noop_patch();
    };
    let n = &mut self.snapshot.nodes[idx];
    let min_h: f64 = match n.kind.as_str() {
      "mind_root" => 48.0,
      "mind_node" => 40.0,
      "icon_card" => 82.0,
      _ => 44.0,
    };
    let icon_extra = if n.icon.is_some() { 30.0 } else { 0.0 };
    let mut new_h = n.h;
    if let Some(mh) = measured_height {
      let needed_h = min_h.max(mh + NODE_PADDING_V * 2.0 + icon_extra);
      if needed_h > n.h + 1.0 {
        new_h = needed_h;
      }
    }
    let text_changed = n.text != next_text;
    let height_changed = (new_h - n.h).abs() > 1e-6;
    if !text_changed && !height_changed {
      return self.noop_patch();
    }
    n.text = next_text;
    n.h = new_h;
    let updated_node = n.clone();
    let updated_routes = self.reroute_incident_edges(&[node_id.to_string()]);
    self.active_rev += 1;
    // If not currently in inline edit mode, also bump graph_rev so static layer updates immediately.
    if self.editing_node_id.as_deref() != Some(node_id) {
      self.graph_rev += 1;
    }
    let delta = SnapshotDelta {
      upserted_nodes: vec![updated_node],
      updated_routes,
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  fn finish_editing_internal(&mut self) -> Option<SnapshotDelta> {
    if self.editing_node_id.is_none() && self.editing_edge_id.is_none() {
      return None;
    }
    let edited_is_mind = self
      .editing_node_id
      .as_deref()
      .and_then(|id| self.node_by_id(id))
      .is_some_and(|n| n.kind == "mind_root" || n.kind == "mind_node");
    self.editing_node_id = None;
    self.editing_edge_id = None;
    if let Some(before) = self.editing_before_snapshot.take() {
      if before != self.snapshot {
        self.push_undo(Some(before));
      }
    }
    let delta = if edited_is_mind {
      layout_mindmap(&mut self.snapshot);
      self.rebuild_indices_and_routes();
      Some(SnapshotDelta {
        upserted_nodes: self.snapshot.nodes.clone(),
        updated_routes: self.routed_edges.clone(),
        collapsed_descendants: self.sorted_collapsed_descendants(),
        ..Default::default()
      })
    } else {
      let updated_routes = self.reroute_orthogonal_edges();
      if updated_routes.is_empty() {
        None
      } else {
        Some(SnapshotDelta {
          updated_routes,
          ..Default::default()
        })
      }
    };
    self.graph_rev += 1;
    self.active_rev += 1;
    delta
  }

  fn finish_editing_cmd(&mut self) -> BoardPatch {
    if self.editing_node_id.is_none() && self.editing_edge_id.is_none() {
      return self.noop_patch();
    }
    let delta = self.finish_editing_internal();
    self.build_patch(delta.is_some(), delta, true, Vec::new())
  }

  fn set_title_cmd(&mut self, title: &str) -> BoardPatch {
    let trimmed = title.trim();
    if trimmed.is_empty() || trimmed == self.snapshot.title {
      return self.noop_patch();
    }
    self.push_undo(None);
    self.snapshot.title = trimmed.to_string();
    self.active_rev += 1;
    self.build_patch(true, None, true, Vec::new())
  }

  fn set_edge_label_cmd(&mut self, edge_id: &str, label: String) -> BoardPatch {
    let Some(idx) = self.snapshot.edges.iter().position(|e| e.id == edge_id) else {
      return self.noop_patch();
    };
    self.push_undo(None);
    self.snapshot.edges[idx].label = label;
    if self.editing_edge_id.as_deref() == Some(edge_id) {
      self.editing_edge_id = None;
    }
    let updated_edge = self.snapshot.edges[idx].clone();
    self.graph_rev += 1;
    self.active_rev += 1;
    let delta = SnapshotDelta {
      upserted_edges: vec![updated_edge],
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  // --- Styling, Z-Order, Clipboard, Delete ---

  #[allow(clippy::too_many_arguments)]
  fn update_selected_nodes_style_cmd(
    &mut self,
    fill: Option<String>,
    stroke: Option<String>,
    dashed: Option<bool>,
    font_size: Option<f64>,
    text_color: Option<String>,
    align: Option<String>,
    kind: Option<String>,
  ) -> BoardPatch {
    if self.selected_node_ids.is_empty() {
      return self.noop_patch();
    }
    self.push_undo(None);
    let mut upserted_nodes = Vec::new();
    for id in &self.selected_node_ids {
      let Some(&idx) = self.node_index.get(id) else {
        continue;
      };
      let n = &mut self.snapshot.nodes[idx];
      if let Some(ref f) = fill {
        n.style.fill = f.clone();
        // Auto-contrast text color for dark fills (#3370FF / #1F2329).
        if f.eq_ignore_ascii_case("#3370FF") || f.eq_ignore_ascii_case("#1F2329") {
          n.style.text_color = "#FFFFFF".to_string();
        } else if n.style.text_color.eq_ignore_ascii_case("#FFFFFF") && text_color.is_none() {
          n.style.text_color = "#1F2329".to_string();
        }
      }
      if let Some(ref s) = stroke {
        n.style.stroke = s.clone();
      }
      if let Some(d) = dashed {
        n.style.dashed = d;
      }
      if let Some(fs) = font_size {
        n.style.font_size = fs;
      }
      if let Some(ref tc) = text_color {
        n.style.text_color = tc.clone();
      }
      if let Some(ref al) = align {
        n.style.align = al.clone();
      }
      if let Some(ref k) = kind {
        n.kind = k.clone();
      }
      upserted_nodes.push(n.clone());
    }
    self.graph_rev += 1;
    self.active_rev += 1;
    let delta = SnapshotDelta {
      upserted_nodes,
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  fn update_selected_edge_cmd(
    &mut self,
    routing: Option<String>,
    stroke: Option<String>,
    dashed: Option<bool>,
    arrow_end: Option<String>,
    arrow_start: Option<String>,
  ) -> BoardPatch {
    let Some(edge_id) = self.selected_edge_id.clone() else {
      return self.noop_patch();
    };
    let Some(idx) = self.snapshot.edges.iter().position(|e| e.id == edge_id) else {
      return self.noop_patch();
    };
    self.push_undo(None);
    let e = &mut self.snapshot.edges[idx];
    if let Some(r) = routing {
      e.routing = r;
      e.waypoints.clear();
    }
    if let Some(s) = stroke {
      e.style.stroke = s;
    }
    if let Some(d) = dashed {
      e.style.dashed = d;
    }
    if let Some(ae) = arrow_end {
      e.arrow_end = ae;
    }
    if let Some(as_) = arrow_start {
      e.arrow_start = as_;
    }
    let updated_edge = e.clone();
    let mut updated_routes = HashMap::new();
    if let Some(pts) = self.reroute_single_edge(&edge_id) {
      updated_routes.insert(edge_id, pts);
    }
    self.graph_rev += 1;
    self.active_rev += 1;
    let delta = SnapshotDelta {
      upserted_edges: vec![updated_edge],
      updated_routes,
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  fn bring_to_front_cmd(&mut self) -> BoardPatch {
    if self.selected_node_ids.is_empty() {
      return self.noop_patch();
    }
    self.push_undo(None);
    let mut max_z = self.snapshot.nodes.iter().map(|n| n.z).max().unwrap_or(0);
    let mut upserted_nodes = Vec::new();
    for id in &self.selected_node_ids {
      if let Some(&idx) = self.node_index.get(id) {
        max_z += 1;
        self.snapshot.nodes[idx].z = max_z;
        upserted_nodes.push(self.snapshot.nodes[idx].clone());
      }
    }
    self.graph_rev += 1;
    self.active_rev += 1;
    let delta = SnapshotDelta {
      upserted_nodes,
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  fn send_to_back_cmd(&mut self) -> BoardPatch {
    if self.selected_node_ids.is_empty() {
      return self.noop_patch();
    }
    self.push_undo(None);
    let mut min_z = self.snapshot.nodes.iter().map(|n| n.z).min().unwrap_or(1);
    let mut upserted_nodes = Vec::new();
    for id in &self.selected_node_ids {
      if let Some(&idx) = self.node_index.get(id) {
        min_z -= 1;
        self.snapshot.nodes[idx].z = min_z;
        upserted_nodes.push(self.snapshot.nodes[idx].clone());
      }
    }
    self.graph_rev += 1;
    self.active_rev += 1;
    let delta = SnapshotDelta {
      upserted_nodes,
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  fn copy_selection_internal(&mut self) {
    if self.selected_node_ids.is_empty() {
      return;
    }
    let selected_set: HashSet<&str> = self.selected_node_ids.iter().map(String::as_str).collect();
    self.clipboard_nodes = self
      .selected_node_ids
      .iter()
      .filter_map(|id| self.node_by_id(id).cloned())
      .collect();
    self.clipboard_edges = self
      .snapshot
      .edges
      .iter()
      .filter(|e| {
        selected_set.contains(e.from_id.as_str()) && selected_set.contains(e.to_id.as_str())
      })
      .cloned()
      .collect();
  }

  fn paste_selection_cmd(&mut self, offset: [f64; 2]) -> BoardPatch {
    if self.clipboard_nodes.is_empty() {
      return self.noop_patch();
    }
    self.push_undo(None);
    let clip_nodes = self.clipboard_nodes.clone();
    let clip_edges = self.clipboard_edges.clone();
    let mut id_map: HashMap<String, String> = HashMap::new();
    let mut new_ids: Vec<String> = Vec::with_capacity(clip_nodes.len());
    let mut upserted_nodes = Vec::with_capacity(clip_nodes.len());
    let mut upserted_edges = Vec::new();
    let base_z = self.snapshot.nodes.iter().map(|n| n.z).max().unwrap_or(0);

    for (offset_idx, src) in clip_nodes.into_iter().enumerate() {
      let nid = self.next_id("n");
      id_map.insert(src.id.clone(), nid.clone());
      let z = base_z + (offset_idx as i32) + 1;
      let node = BoardNode {
        id: nid.clone(),
        kind: src.kind,
        x: src.x + offset[0],
        y: src.y + offset[1],
        w: src.w,
        h: src.h,
        rot: src.rot,
        z,
        parent_id: None,
        rank: 0,
        collapsed: false,
        text: src.text,
        icon: src.icon,
        style: src.style,
      };
      self.snapshot.nodes.push(node.clone());
      upserted_nodes.push(node);
      new_ids.push(nid);
    }

    for src_edge in clip_edges {
      if let (Some(nf), Some(nt)) = (id_map.get(&src_edge.from_id), id_map.get(&src_edge.to_id)) {
        let eid = self.next_id("e");
        let edge = BoardEdge {
          id: eid,
          from_id: nf.clone(),
          to_id: nt.clone(),
          from_anchor: src_edge.from_anchor,
          to_anchor: src_edge.to_anchor,
          routing: src_edge.routing,
          arrow_start: src_edge.arrow_start,
          arrow_end: src_edge.arrow_end,
          waypoints: Vec::new(),
          label: src_edge.label,
          style: src_edge.style,
        };
        self.snapshot.edges.push(edge.clone());
        upserted_edges.push(edge);
      }
    }

    self.rebuild_indices();
    let mut updated_routes = HashMap::new();
    for e in &upserted_edges {
      if let Some(pts) = self.reroute_single_edge(&e.id) {
        updated_routes.insert(e.id.clone(), pts);
      }
    }
    self.selected_node_ids = new_ids;
    self.selected_edge_id = None;
    self.graph_rev += 1;
    self.active_rev += 1;
    let delta = SnapshotDelta {
      upserted_nodes,
      upserted_edges,
      updated_routes,
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  fn delete_selection_cmd(&mut self) -> BoardPatch {
    if self.selected_node_ids.is_empty() && self.selected_edge_id.is_none() {
      return self.noop_patch();
    }
    self.push_undo(None);
    let mut removed_node_ids: Vec<String> = Vec::new();
    let mut removed_edge_ids: Vec<String> = Vec::new();

    if let Some(eid) = self.selected_edge_id.take() {
      if self.snapshot.edges.iter().any(|e| e.id == eid) {
        self.snapshot.edges.retain(|e| e.id != eid);
        removed_edge_ids.push(eid);
      }
    }

    if !self.selected_node_ids.is_empty() {
      // Transitive cascade: delete selected nodes and all their mind-map descendants
      // (including collapsed descendants).
      let mut to_delete: HashSet<String> = self.selected_node_ids.iter().cloned().collect();
      let mut grew = true;
      while grew {
        grew = false;
        for n in &self.snapshot.nodes {
          if let Some(pid) = &n.parent_id {
            if to_delete.contains(pid) && to_delete.insert(n.id.clone()) {
              grew = true;
            }
          }
        }
      }
      for n in &self.snapshot.nodes {
        if to_delete.contains(&n.id) {
          removed_node_ids.push(n.id.clone());
        }
      }
      for e in &self.snapshot.edges {
        if (to_delete.contains(&e.from_id) || to_delete.contains(&e.to_id))
          && !removed_edge_ids.contains(&e.id)
        {
          removed_edge_ids.push(e.id.clone());
        }
      }
      self.snapshot.nodes.retain(|n| !to_delete.contains(&n.id));
      self
        .snapshot
        .edges
        .retain(|e| !to_delete.contains(&e.from_id) && !to_delete.contains(&e.to_id));
      if let Some(edit_id) = &self.editing_node_id {
        if to_delete.contains(edit_id) {
          self.editing_node_id = None;
          self.editing_before_snapshot = None;
        }
      }
      if let Some(hid) = &self.hovered_node_id {
        if to_delete.contains(hid) {
          self.hovered_node_id = None;
        }
      }
      self.selected_node_ids.clear();
    }

    self.rebuild_indices_and_routes();
    self.graph_rev += 1;
    self.active_rev += 1;
    let delta = SnapshotDelta {
      removed_node_ids,
      removed_edge_ids,
      updated_routes: self.routed_edges.clone(),
      collapsed_descendants: self.sorted_collapsed_descendants(),
      ..Default::default()
    };
    self.build_patch(true, Some(delta), true, Vec::new())
  }

  // --- Undo / Redo / Snapshot Replacement ---

  fn undo_cmd(&mut self) -> BoardPatch {
    let Some(prev) = self.undo_stack.pop() else {
      return self.noop_patch();
    };
    self.editing_node_id = None;
    self.editing_edge_id = None;
    self.editing_before_snapshot = None;
    self.gesture = GestureState::Idle;
    self.snap_guides.clear();
    self.redo_stack.push(self.snapshot.clone());
    self.snapshot = prev;
    self.rebuild_indices_and_routes();
    self
      .selected_node_ids
      .retain(|id| self.node_index.contains_key(id));
    self.selected_edge_id = None;
    self.graph_rev += 1;
    self.active_rev += 1;
    self.build_patch(true, Some(self.full_replace_delta()), true, Vec::new())
  }

  fn redo_cmd(&mut self) -> BoardPatch {
    let Some(next) = self.redo_stack.pop() else {
      return self.noop_patch();
    };
    self.editing_node_id = None;
    self.editing_edge_id = None;
    self.editing_before_snapshot = None;
    self.gesture = GestureState::Idle;
    self.snap_guides.clear();
    self.undo_stack.push(self.snapshot.clone());
    self.snapshot = next;
    self.rebuild_indices_and_routes();
    self
      .selected_node_ids
      .retain(|id| self.node_index.contains_key(id));
    self.selected_edge_id = None;
    self.graph_rev += 1;
    self.active_rev += 1;
    self.build_patch(true, Some(self.full_replace_delta()), true, Vec::new())
  }

  fn replace_snapshot_cmd(&mut self, next: BoardSnapshot, record_undo: bool) -> BoardPatch {
    if record_undo {
      self.push_undo(None);
    }
    self.editing_node_id = None;
    self.editing_edge_id = None;
    self.editing_before_snapshot = None;
    self.gesture = GestureState::Idle;
    self.snap_guides.clear();
    self.selected_node_ids.clear();
    self.selected_edge_id = None;
    self.hovered_node_id = None;
    self.snapshot = next;
    self.rebuild_indices_and_routes();
    self.graph_rev += 1;
    self.active_rev += 1;
    self.build_patch(true, Some(self.full_replace_delta()), true, Vec::new())
  }

  // --- Keyboard Shortcuts ---

  fn handle_key_down(
    &mut self,
    key: &str,
    ctrl_or_meta: bool,
    shift: bool,
    read_only: bool,
  ) -> BoardPatch {
    let lower = key.to_ascii_lowercase();

    // While inline editing, Escape or Ctrl/Meta+Enter finishes editing.
    if self.editing_node_id.is_some() {
      if lower == "escape" || (lower == "enter" && ctrl_or_meta) {
        return self.finish_editing_cmd();
      }
      return self.noop_patch();
    }

    if ctrl_or_meta {
      return match lower.as_str() {
        "z" if shift => self.redo_cmd(),
        "z" => self.undo_cmd(),
        "y" => self.redo_cmd(),
        "c" => {
          self.copy_selection_internal();
          self.noop_patch()
        }
        "v" => self.paste_selection_cmd(DEFAULT_PASTE_OFFSET),
        "d" => {
          self.copy_selection_internal();
          self.paste_selection_cmd(DEFAULT_PASTE_OFFSET)
        }
        "a" => self.select_all_cmd(),
        _ => self.noop_patch(),
      };
    }

    if read_only {
      if lower == "escape" {
        return self.build_patch(false, None, false, vec![UiEffect::CloseEditor]);
      }
      return self.noop_patch();
    }

    match lower.as_str() {
      "tab" if self.selected_node_ids.len() == 1 => {
        let sel = self.selected_node_ids[0].clone();
        self.add_mind_child_cmd(&sel, true)
      }
      "enter" if self.selected_node_ids.len() == 1 => {
        let sel_id = self.selected_node_ids[0].clone();
        let is_mind = self
          .node_by_id(&sel_id)
          .is_some_and(|n| n.kind == "mind_root" || n.kind == "mind_node");
        if is_mind {
          self.add_mind_sibling_cmd(&sel_id, true)
        } else {
          self.start_editing_node_cmd(&sel_id)
        }
      }
      "delete" | "backspace" => self.delete_selection_cmd(),
      "escape" => {
        if !self.selected_node_ids.is_empty() || self.selected_edge_id.is_some() {
          self.clear_selection_cmd()
        } else if self.tool != BoardTool::Select {
          self.set_tool_cmd(BoardTool::Select)
        } else {
          self.build_patch(false, None, false, vec![UiEffect::CloseEditor])
        }
      }
      "v" => self.set_tool_cmd(BoardTool::Select),
      "h" => self.set_tool_cmd(BoardTool::Hand),
      "s" => self.set_tool_cmd(BoardTool::Sticky),
      "t" => self.set_tool_cmd(BoardTool::Text),
      "l" => self.set_tool_cmd(BoardTool::Connector),
      "p" => self.set_tool_cmd(BoardTool::Pen),
      "f" => self.set_tool_cmd(BoardTool::Frame),
      "m" => self.set_tool_cmd(BoardTool::Mindmap),
      _ => self.noop_patch(),
    }
  }
}

fn merge_pre_delta(patch: &mut BoardPatch, pre_delta: Option<SnapshotDelta>) {
  let Some(pre) = pre_delta else {
    return;
  };
  patch.snapshot_changed = true;
  match &mut patch.delta {
    Some(dst) => {
      dst.full_replace |= pre.full_replace;
      for n in pre.upserted_nodes {
        if !dst.upserted_nodes.iter().any(|x| x.id == n.id) {
          dst.upserted_nodes.push(n);
        }
      }
      for (k, v) in pre.updated_routes {
        dst.updated_routes.entry(k).or_insert(v);
      }
    }
    None => {
      patch.delta = Some(pre);
    }
  }
}
