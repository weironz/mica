use super::*;
use crate::model::{BoardEdge, BoardNode, BoardSnapshot, EdgeStyle, NodeStyle};
use crate::routing::{Side, anchor_point};

fn make_node(id: &str, kind: &str, x: f64, y: f64, w: f64, h: f64, z: i32) -> BoardNode {
  BoardNode {
    id: id.to_string(),
    kind: kind.to_string(),
    x,
    y,
    w,
    h,
    rot: 0.0,
    z,
    parent_id: None,
    rank: 0,
    collapsed: false,
    text: id.to_string(),
    icon: None,
    style: NodeStyle::default(),
  }
}

fn make_edge(id: &str, from_id: &str, to_id: &str, routing: &str) -> BoardEdge {
  BoardEdge {
    id: id.to_string(),
    from_id: from_id.to_string(),
    to_id: to_id.to_string(),
    from_anchor: "r".to_string(),
    to_anchor: "l".to_string(),
    routing: routing.to_string(),
    arrow_start: "none".to_string(),
    arrow_end: "arrow".to_string(),
    waypoints: Vec::new(),
    label: String::new(),
    style: EdgeStyle::default(),
  }
}

fn two_node_session() -> BoardSession {
  let snap = BoardSnapshot {
    version: 1,
    title: "测试画板".to_string(),
    mode: "whiteboard".to_string(),
    nodes: vec![
      make_node("a", "rounded", 100.0, 100.0, 120.0, 60.0, 1),
      make_node("b", "rounded", 320.0, 100.0, 120.0, 60.0, 2),
    ],
    edges: vec![make_edge("e1", "a", "b", "straight")],
    strokes: Vec::new(),
  };
  let mut s = BoardSession::new(snap);
  // Set identity viewport transform so screen == world in geometry tests.
  s.set_viewport(1.0, [0.0, 0.0]);
  s
}

fn pointer_down(x: f64, y: f64) -> BoardCommand {
  BoardCommand::PointerDown {
    screen_x: x,
    screen_y: y,
    button: 0,
    shift: false,
    alt: false,
    space: false,
    read_only: false,
    timestamp_ms: None,
  }
}

// ============================================================================
// 1. Construction, JSON & SVG round-trip
// ============================================================================

#[test]
fn from_json_valid_and_export_roundtrip() {
  let snap = BoardSnapshot::template_flowchart();
  let json = snap.to_compact_json();
  let session = BoardSession::from_json(&json);
  assert_eq!(session.snapshot().mode, "flowchart");
  assert_eq!(session.snapshot().nodes.len(), 4);
  assert_eq!(session.routed_edges().len(), 3);
  let exported = session.export_compact_json();
  let reparsed = BoardSnapshot::parse_json(&exported).expect("valid exported json");
  assert_eq!(reparsed, *session.snapshot());
}

#[test]
fn from_json_invalid_falls_back_to_whiteboard() {
  let session = BoardSession::from_json("{not valid json!!!");
  assert_eq!(session.snapshot().mode, "whiteboard");
  assert_eq!(session.snapshot().nodes.len(), 2);
}

#[test]
fn try_from_json_invalid_returns_err() {
  assert!(BoardSession::try_from_json("bad").is_err());
}

#[test]
fn render_svg_matches_snapshot() {
  let session = BoardSession::new(BoardSnapshot::template_mindmap());
  let svg = session.render_svg(None);
  assert!(svg.starts_with("<svg "));
  assert!(svg.ends_with("</svg>"));
  assert!(svg.contains("中心主题"));
}

#[test]
fn dispatch_json_roundtrip_and_invalid_cmd_error() {
  let mut s = two_node_session();
  let cmd_json = r#"{"type":"selectNode","id":"a","additive":false}"#;
  let patch_json = s.dispatch_json(cmd_json).expect("dispatch_json");
  let patch: BoardPatch = serde_json::from_str(&patch_json).expect("parse patch");
  assert_eq!(patch.active_overlay.selected_node_ids, vec!["a"]);
  assert!(s.dispatch_json(r#"{"type":"unknownOp"}"#).is_err());
}

#[test]
fn current_state_patch_hydrates_full_snapshot_and_routes() {
  let s = BoardSession::new(BoardSnapshot::template_mindmap());
  let patch = s.current_state_patch();
  assert!(patch.snapshot_changed);
  assert!(patch.rebuild_chrome);
  let delta = patch.delta.expect("hydration delta");
  assert!(delta.full_replace);
  assert_eq!(delta.upserted_nodes.len(), 4);
  assert_eq!(delta.upserted_edges.len(), 3);
  assert_eq!(delta.updated_routes.len(), 3);
}

// ============================================================================
// 2. Viewport & Coordinate Transforms
// ============================================================================

#[test]
fn screen_world_roundtrip_default_and_zoomed() {
  let mut s = two_node_session();
  s.set_viewport(1.5, [120.0, -45.0]);
  let world = [210.0, 88.0];
  let screen = s.world_to_screen(world);
  let back = s.screen_to_world(screen);
  assert!((back[0] - world[0]).abs() < 1e-9);
  assert!((back[1] - world[1]).abs() < 1e-9);
}

#[test]
fn set_viewport_clamps_zoom_to_0_2_through_3_0() {
  let mut s = two_node_session();
  s.set_viewport(0.01, [10.0, 20.0]);
  assert!((s.zoom() - 0.2).abs() < 1e-9);
  s.set_viewport(99.0, [10.0, 20.0]);
  assert!((s.zoom() - 3.0).abs() < 1e-9);
}

#[test]
fn set_viewport_noop_does_not_bump_revs() {
  let mut s = two_node_session();
  let vp_rev = s.viewport_rev();
  let act_rev = s.active_rev();
  let patch = s.set_viewport(1.0, [0.0, 0.0]);
  assert_eq!(patch.viewport_rev, vp_rev);
  assert_eq!(patch.active_rev, act_rev);
  assert!(!patch.rebuild_chrome);
}

#[test]
fn set_viewport_pan_only_does_not_rebuild_chrome() {
  let mut s = two_node_session();
  let vp_rev = s.viewport_rev();
  let patch = s.set_viewport(1.0, [40.0, -20.0]);
  assert_eq!(patch.viewport_rev, vp_rev + 1);
  assert!(!patch.rebuild_chrome);
}

#[test]
fn set_viewport_pan_while_editing_rebuilds_chrome() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::StartEditingNode {
    node_id: "a".to_string(),
  });
  let patch = s.set_viewport(1.0, [40.0, -20.0]);
  assert!(patch.rebuild_chrome);
}

#[test]
fn zoom_around_preserves_focal_world_point() {
  let mut s = two_node_session();
  s.set_viewport(1.0, [50.0, 30.0]);
  let focal = [400.0, 300.0];
  let world_before = s.screen_to_world(focal);
  s.zoom_around(1.5, focal);
  let world_after = s.screen_to_world(focal);
  assert!((world_before[0] - world_after[0]).abs() < 1e-9);
  assert!((world_before[1] - world_after[1]).abs() < 1e-9);
}

#[test]
fn wheel_zoom_via_command_clamps_and_keeps_anchor() {
  let mut s = two_node_session();
  let focal = [250.0, 180.0];
  let before = s.screen_to_world(focal);
  let patch = s.dispatch(BoardCommand::WheelZoom {
    factor: 1.12,
    focal_screen_x: focal[0],
    focal_screen_y: focal[1],
  });
  assert!(patch.rebuild_chrome);
  assert!((patch.zoom - 1.12).abs() < 1e-9);
  let after = s.screen_to_world(focal);
  assert!((before[0] - after[0]).abs() < 1e-9);
  assert!((before[1] - after[1]).abs() < 1e-9);
}

#[test]
fn fit_to_content_centers_nodes_and_clamps_zoom() {
  let mut s = two_node_session();
  let patch = s.fit_to_content(Some([1000.0, 700.0]));
  assert!((0.3..=1.35).contains(&patch.zoom));
  // Bounding box of a (100..220, 100..160) & b (320..440, 100..160): center = (270, 130).
  let screen_center = s.world_to_screen([270.0, 130.0]);
  assert!((screen_center[0] - 500.0).abs() < 1e-6);
  assert!((screen_center[1] - 350.0).abs() < 1e-6);
}

#[test]
fn fit_to_content_empty_board_resets_to_default_origin() {
  let empty = BoardSnapshot {
    version: 1,
    title: "空".into(),
    mode: "whiteboard".into(),
    nodes: vec![],
    edges: vec![],
    strokes: vec![],
  };
  let mut s = BoardSession::new(empty);
  s.fit_to_content(Some([1000.0, 500.0]));
  assert!((s.zoom() - 1.0).abs() < 1e-9);
  assert!((s.pan()[0] - 200.0).abs() < 1e-9);
  assert!((s.pan()[1] - 100.0).abs() < 1e-9);
}

#[test]
fn fit_to_content_tiny_viewport_is_noop() {
  let mut s = two_node_session();
  let before_rev = s.viewport_rev();
  let patch = s.fit_to_content(Some([20.0, 20.0]));
  assert_eq!(patch.viewport_rev, before_rev);
}

// ============================================================================
// 3. Hit Testing
// ============================================================================

#[test]
fn hit_test_node_respects_z_order() {
  let snap = BoardSnapshot {
    version: 1,
    title: "z".into(),
    mode: "whiteboard".into(),
    nodes: vec![
      make_node("low", "rect", 50.0, 50.0, 100.0, 100.0, 1),
      make_node("high", "rect", 80.0, 80.0, 100.0, 100.0, 5),
    ],
    edges: vec![],
    strokes: vec![],
  };
  let s = BoardSession::new(snap);
  assert_eq!(
    s.hit_test_node([100.0, 100.0]).map(|n| n.id.as_str()),
    Some("high")
  );
  assert_eq!(
    s.hit_test_node([60.0, 60.0]).map(|n| n.id.as_str()),
    Some("low")
  );
  assert!(s.hit_test_node([10.0, 10.0]).is_none());
}

#[test]
fn hit_test_node_frame_title_bar_and_border_only() {
  let snap = BoardSnapshot {
    version: 1,
    title: "frame".into(),
    mode: "whiteboard".into(),
    nodes: vec![make_node("fr", "frame", 100.0, 100.0, 300.0, 200.0, 0)],
    edges: vec![],
    strokes: vec![],
  };
  let mut s = BoardSession::new(snap);
  s.set_viewport(1.0, [0.0, 0.0]);
  // Title bar (top 30px) hits frame.
  assert_eq!(
    s.hit_test_node([200.0, 115.0]).map(|n| n.id.as_str()),
    Some("fr")
  );
  // Border band (within outer +6 / inner -14) hits frame.
  assert_eq!(
    s.hit_test_node([105.0, 200.0]).map(|n| n.id.as_str()),
    Some("fr")
  );
  assert_eq!(
    s.hit_test_node([250.0, 298.0]).map(|n| n.id.as_str()),
    Some("fr")
  );
  // Deep interior misses frame so user can marquee/click inside.
  assert!(s.hit_test_node([250.0, 200.0]).is_none());
}

#[test]
fn hit_test_node_child_inside_frame_beats_frame() {
  let snap = BoardSnapshot {
    version: 1,
    title: "frame".into(),
    mode: "whiteboard".into(),
    nodes: vec![
      make_node("fr", "frame", 100.0, 100.0, 300.0, 200.0, 99),
      make_node("child", "rounded", 120.0, 105.0, 100.0, 50.0, 1),
    ],
    edges: vec![],
    strokes: vec![],
  };
  let s = BoardSession::new(snap);
  // Even though frame has higher z (99 > 1), frames sort below non-frames.
  assert_eq!(
    s.hit_test_node([150.0, 120.0]).map(|n| n.id.as_str()),
    Some("child")
  );
}

#[test]
fn hit_test_node_skips_collapsed_descendants() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  let child_pos = {
    let m1 = s.node_by_id("m1").unwrap();
    [m1.x + 10.0, m1.y + 10.0]
  };
  assert_eq!(
    s.hit_test_node(child_pos).map(|n| n.id.as_str()),
    Some("m1")
  );
  s.dispatch(BoardCommand::ToggleCollapse {
    node_id: "m0".to_string(),
  });
  assert!(s.hit_test_node(child_pos).is_none());
}

#[test]
fn hit_test_collapse_badge_hits_parent_only() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.set_viewport(1.0, [0.0, 0.0]);
  let m0 = s.node_by_id("m0").unwrap();
  let badge = [m0.x + m0.w + 10.0, m0.y + m0.h / 2.0];
  assert_eq!(s.hit_test_collapse_badge(badge).as_deref(), Some("m0"));
  // Leaf node m1 has no children => no collapse badge hit.
  let m1 = s.node_by_id("m1").unwrap();
  let leaf_badge = [m1.x + m1.w + 10.0, m1.y + m1.h / 2.0];
  assert!(s.hit_test_collapse_badge(leaf_badge).is_none());
}

#[test]
fn hit_test_collapse_badge_scales_with_zoom() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  let m0 = s.node_by_id("m0").unwrap();
  let offset_pt = [m0.x + m0.w + 10.0 + 18.0, m0.y + m0.h / 2.0];
  s.set_viewport(1.0, [0.0, 0.0]);
  assert!(s.hit_test_collapse_badge(offset_pt).is_none());
  // At 0.5x zoom, hit radius is 11 / 0.5 = 22px => now hits!
  s.set_viewport(0.5, [0.0, 0.0]);
  assert_eq!(s.hit_test_collapse_badge(offset_pt).as_deref(), Some("m0"));
}

#[test]
fn hit_test_anchor_port_on_selected_and_hovered_nodes() {
  let mut s = two_node_session();
  let right_a = anchor_point(s.node_by_id("a").unwrap(), Side::Right);
  // Neither selected nor hovered => miss.
  assert!(s.hit_test_anchor_port(right_a).is_none());
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  assert_eq!(
    s.hit_test_anchor_port(right_a),
    Some(("a".to_string(), "r".to_string()))
  );
  // Hovered node b should also expose ports.
  s.dispatch(BoardCommand::PointerHover {
    screen_x: 350.0,
    screen_y: 120.0,
  });
  let top_b = anchor_point(s.node_by_id("b").unwrap(), Side::Top);
  assert_eq!(
    s.hit_test_anchor_port(top_b),
    Some(("b".to_string(), "t".to_string()))
  );
}

#[test]
fn hit_test_anchor_port_ignores_frames() {
  let snap = BoardSnapshot {
    version: 1,
    title: "f".into(),
    mode: "whiteboard".into(),
    nodes: vec![make_node("fr", "frame", 100.0, 100.0, 200.0, 120.0, 0)],
    edges: vec![],
    strokes: vec![],
  };
  let mut s = BoardSession::new(snap);
  s.set_viewport(1.0, [0.0, 0.0]);
  s.dispatch(BoardCommand::SelectNode {
    id: "fr".into(),
    additive: false,
  });
  let right_fr = anchor_point(s.node_by_id("fr").unwrap(), Side::Right);
  assert!(s.hit_test_anchor_port(right_fr).is_none());
}

#[test]
fn hit_test_resize_handle_all_8_directions_single_selection_only() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  let cases = [
    ([100.0, 100.0], ResizeHandle::Nw),
    ([160.0, 100.0], ResizeHandle::N),
    ([220.0, 100.0], ResizeHandle::Ne),
    ([220.0, 130.0], ResizeHandle::E),
    ([220.0, 160.0], ResizeHandle::Se),
    ([160.0, 160.0], ResizeHandle::S),
    ([100.0, 160.0], ResizeHandle::Sw),
    ([100.0, 130.0], ResizeHandle::W),
  ];
  for (pt, expected) in cases {
    assert_eq!(
      s.hit_test_resize_handle(pt),
      Some(("a".to_string(), expected))
    );
  }
  // Multi-selection disables resize handles.
  s.dispatch(BoardCommand::SelectNode {
    id: "b".into(),
    additive: true,
  });
  assert!(s.hit_test_resize_handle([220.0, 160.0]).is_none());
}

#[test]
fn hit_test_edge_straight_and_orthogonal_routed_polyline() {
  let s = two_node_session();
  // Straight edge from (220, 130) to (320, 130).
  assert_eq!(
    s.hit_test_edge([270.0, 132.0]).map(|e| e.id.as_str()),
    Some("e1")
  );
  assert!(s.hit_test_edge([270.0, 160.0]).is_none());

  // Orthogonal routed edge in flowchart template.
  let mut flow = BoardSession::new(BoardSnapshot::template_flowchart());
  flow.set_viewport(1.0, [0.0, 0.0]);
  let pts = flow.routed_edges().get("fe1").expect("routed fe1").clone();
  let mid = [(pts[0][0] + pts[1][0]) / 2.0, (pts[0][1] + pts[1][1]) / 2.0];
  assert_eq!(flow.hit_test_edge(mid).map(|e| e.id.as_str()), Some("fe1"));
}

#[test]
fn hit_test_edge_curved_samples_bezier_not_control_polygon() {
  let snap = BoardSnapshot {
    version: 1,
    title: "curve".into(),
    mode: "whiteboard".into(),
    nodes: vec![
      make_node("a", "rounded", 0.0, 0.0, 100.0, 60.0, 1),
      make_node("b", "rounded", 300.0, 260.0, 100.0, 60.0, 2),
    ],
    edges: vec![make_edge("c1", "a", "b", "curved")],
    strokes: vec![],
  };
  let mut s = BoardSession::new(snap);
  s.set_viewport(1.0, [0.0, 0.0]);
  let pts = s.routed_edges().get("c1").unwrap().clone();
  assert_eq!(pts.len(), 4);
  // Evaluate midpoint t = 0.5 on the cubic bezier -> must hit!
  let mid = hit_test::cubic_at(pts[0], pts[1], pts[2], pts[3], 0.5);
  assert_eq!(s.hit_test_edge(mid).map(|e| e.id.as_str()), Some("c1"));
  // Control point cp1 is far off the actual curve -> must NOT hit!
  assert!(s.hit_test_edge(pts[1]).is_none());
}

// ============================================================================
// 4. Dragging, Smart Snapping & Panning
// ============================================================================

#[test]
fn drag_single_node_updates_position_and_coalesces_undo() {
  let mut s = two_node_session();
  let p_down = s.dispatch(pointer_down(150.0, 130.0));
  assert!(p_down.active_overlay.is_dragging);
  assert_eq!(p_down.active_overlay.dynamic_node_ids, vec!["a"]);

  s.pointer_move(170.0, 145.0, 170.0, 145.0, true);
  s.pointer_move(200.0, 180.0, 200.0, 180.0, true);
  let p_up = s.dispatch(BoardCommand::PointerUp);
  assert!(!p_up.active_overlay.is_dragging);
  assert!(p_up.active_overlay.dynamic_node_ids.is_empty());

  let a = s.node_by_id("a").unwrap();
  assert!((a.x - 150.0).abs() < 1e-6);
  assert!((a.y - 150.0).abs() < 1e-6);
  assert_eq!(s.undo_len(), 1);

  s.dispatch(BoardCommand::Undo);
  let a_after = s.node_by_id("a").unwrap();
  assert!((a_after.x - 100.0).abs() < 1e-6);
  assert!((a_after.y - 100.0).abs() < 1e-6);
}

#[test]
fn drag_zero_or_subthreshold_movement_does_not_push_undo() {
  let mut s = two_node_session();
  s.dispatch(pointer_down(150.0, 130.0));
  s.pointer_move(150.2, 130.1, 150.2, 130.1, true);
  s.dispatch(BoardCommand::PointerUp);
  assert!(!s.can_undo());
}

#[test]
fn drag_multi_select_moves_all_selected_without_snap() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectAll);
  s.dispatch(pointer_down(150.0, 130.0));
  let patch = s.pointer_move(180.0, 170.0, 180.0, 170.0, false);
  assert!(patch.active_overlay.snap_guides.is_empty());
  assert_eq!(patch.active_overlay.dynamic_node_ids.len(), 2);
  s.dispatch(BoardCommand::PointerUp);
  assert!((s.node_by_id("a").unwrap().x - 130.0).abs() < 1e-6);
  assert!((s.node_by_id("b").unwrap().x - 350.0).abs() < 1e-6);
  assert!((s.node_by_id("b").unwrap().y - 140.0).abs() < 1e-6);
}

#[test]
fn drag_smart_snap_left_center_right_within_6px() {
  let snap = BoardSnapshot {
    version: 1,
    title: "snap".into(),
    mode: "whiteboard".into(),
    nodes: vec![
      make_node("a", "rounded", 100.0, 100.0, 100.0, 50.0, 1),
      make_node("b", "rounded", 250.0, 260.0, 100.0, 50.0, 2),
    ],
    edges: vec![],
    strokes: vec![],
  };
  let mut s = BoardSession::new(snap);
  s.set_viewport(1.0, [0.0, 0.0]);
  // Drag 'a' to x = 247 (within 3px of b.x = 250).
  s.dispatch(pointer_down(150.0, 125.0));
  let patch = s.pointer_move(297.0, 125.0, 297.0, 125.0, false);
  assert!((s.node_by_id("a").unwrap().x - 250.0).abs() < 1e-6);
  assert!(
    patch
      .active_overlay
      .snap_guides
      .iter()
      .any(|g| g.vertical && (g.coord - 250.0).abs() < 1e-6)
  );
}

#[test]
fn drag_smart_snap_top_center_bottom_within_6px() {
  let snap = BoardSnapshot {
    version: 1,
    title: "snap".into(),
    mode: "whiteboard".into(),
    nodes: vec![
      make_node("a", "rounded", 100.0, 100.0, 100.0, 50.0, 1),
      make_node("b", "rounded", 300.0, 200.0, 100.0, 50.0, 2),
    ],
    edges: vec![],
    strokes: vec![],
  };
  let mut s = BoardSession::new(snap);
  s.set_viewport(1.0, [0.0, 0.0]);
  // Move 'a' down by 96px -> raw y = 196, within 4px of b.y = 200.
  s.dispatch(pointer_down(150.0, 125.0));
  let patch = s.pointer_move(150.0, 221.0, 150.0, 221.0, false);
  assert!((s.node_by_id("a").unwrap().y - 200.0).abs() < 1e-6);
  assert!(
    patch
      .active_overlay
      .snap_guides
      .iter()
      .any(|g| !g.vertical && (g.coord - 200.0).abs() < 1e-6)
  );
}

#[test]
fn drag_smart_snap_no_stickiness_when_slowly_moving_past_guide() {
  let snap = BoardSnapshot {
    version: 1,
    title: "nostick".into(),
    mode: "whiteboard".into(),
    nodes: vec![
      make_node("a", "rounded", 100.0, 100.0, 100.0, 50.0, 1),
      make_node("b", "rounded", 100.0, 260.0, 100.0, 50.0, 2),
    ],
    edges: vec![],
    strokes: vec![],
  };
  let mut s = BoardSession::new(snap);
  s.set_viewport(1.0, [0.0, 0.0]);
  s.dispatch(pointer_down(150.0, 125.0));
  // Move 1px at a time for 10 steps (+10px total > 6px threshold).
  for step in 1..=10 {
    s.pointer_move(
      150.0 + step as f64,
      125.0,
      150.0 + step as f64,
      125.0,
      false,
    );
  }
  // Because snapping uses unsnapped origin + total_delta, it MUST escape the guide at +10px!
  assert!((s.node_by_id("a").unwrap().x - 110.0).abs() < 1e-6);
  assert!(s.snap_guides().is_empty());
}

#[test]
fn drag_alt_key_disables_smart_snapping() {
  let snap = BoardSnapshot {
    version: 1,
    title: "alt".into(),
    mode: "whiteboard".into(),
    nodes: vec![
      make_node("a", "rounded", 100.0, 100.0, 100.0, 50.0, 1),
      make_node("b", "rounded", 250.0, 260.0, 100.0, 50.0, 2),
    ],
    edges: vec![],
    strokes: vec![],
  };
  let mut s = BoardSession::new(snap);
  s.set_viewport(1.0, [0.0, 0.0]);
  s.dispatch(pointer_down(150.0, 125.0));
  let patch = s.pointer_move(297.0, 125.0, 297.0, 125.0, true);
  assert!((s.node_by_id("a").unwrap().x - 247.0).abs() < 1e-6);
  assert!(patch.active_overlay.snap_guides.is_empty());
}

#[test]
fn drag_does_not_bump_graph_rev_during_move_only_on_start_and_end() {
  let mut s = two_node_session();
  let r0 = s.graph_rev();
  let p_start = s.dispatch(pointer_down(150.0, 130.0));
  assert_eq!(p_start.graph_rev, r0 + 1);
  let p_move = s.pointer_move(180.0, 130.0, 180.0, 130.0, true);
  assert_eq!(p_move.graph_rev, r0 + 1);
  assert!(!p_move.rebuild_chrome);
  let p_end = s.dispatch(BoardCommand::PointerUp);
  assert_eq!(p_end.graph_rev, r0 + 2);
  assert!(p_end.rebuild_chrome);
}

#[test]
fn pan_with_middle_button_hand_tool_space_and_readonly() {
  let mut s = two_node_session();
  // Middle mouse button pan.
  s.dispatch(BoardCommand::PointerDown {
    screen_x: 100.0,
    screen_y: 100.0,
    button: 1,
    shift: false,
    alt: false,
    space: false,
    read_only: false,
    timestamp_ms: None,
  });
  let p = s.pointer_move(140.0, 125.0, 140.0, 125.0, false);
  assert_eq!(p.pan, [40.0, 25.0]);
  s.dispatch(BoardCommand::PointerUp);

  // Hand tool pan.
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Hand,
  });
  s.dispatch(pointer_down(200.0, 200.0));
  s.pointer_move(210.0, 190.0, 210.0, 190.0, false);
  assert_eq!(s.pan(), [50.0, 15.0]);
  s.dispatch(BoardCommand::PointerUp);

  // Spacebar pan even in Select tool.
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Select,
  });
  s.dispatch(BoardCommand::PointerDown {
    screen_x: 50.0,
    screen_y: 50.0,
    button: 0,
    shift: false,
    alt: false,
    space: true,
    read_only: false,
    timestamp_ms: None,
  });
  s.pointer_move(60.0, 70.0, 60.0, 70.0, false);
  assert_eq!(s.pan(), [60.0, 35.0]);
  s.dispatch(BoardCommand::PointerUp);

  // ReadOnly primary click pans instead of dragging nodes.
  s.dispatch(BoardCommand::PointerDown {
    screen_x: 150.0,
    screen_y: 130.0,
    button: 0,
    shift: false,
    alt: false,
    space: false,
    read_only: true,
    timestamp_ms: None,
  });
  assert!(s.active_overlay().is_panning);
  s.dispatch(BoardCommand::PointerUp);
}

// ============================================================================
// 5. 8-Direction Resizing
// ============================================================================

#[test]
fn resize_east_south_southeast_grows_and_clamps_minimums() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  // SE handle at (220, 160).
  s.dispatch(pointer_down(220.0, 160.0));
  assert!(s.active_overlay().is_resizing);
  s.pointer_move(260.0, 200.0, 260.0, 200.0, false);
  s.dispatch(BoardCommand::PointerUp);
  let (ax, ay, aw, ah) = {
    let a = s.node_by_id("a").unwrap();
    (a.x, a.y, a.w, a.h)
  };
  assert!((aw - 160.0).abs() < 1e-6);
  assert!((ah - 100.0).abs() < 1e-6);

  // Shrink below min_w=56, min_h=34 via SE handle.
  s.dispatch(pointer_down(ax + aw, ay + ah));
  s.pointer_move(ax + 10.0, ay + 5.0, ax + 10.0, ay + 5.0, false);
  s.dispatch(BoardCommand::PointerUp);
  let a = s.node_by_id("a").unwrap();
  assert!((a.w - 56.0).abs() < 1e-6);
  assert!((a.h - 34.0).abs() < 1e-6);
}

#[test]
fn resize_west_north_northwest_adjusts_origin_and_clamps_minimums() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  // NW handle at (100, 100), drag right-down by +100, +100 (should clamp to min_w=56, min_h=34 keeping right=220, bottom=160).
  s.dispatch(pointer_down(100.0, 100.0));
  s.pointer_move(200.0, 200.0, 200.0, 200.0, false);
  s.dispatch(BoardCommand::PointerUp);
  let a = s.node_by_id("a").unwrap();
  assert!((a.w - 56.0).abs() < 1e-6);
  assert!((a.h - 34.0).abs() < 1e-6);
  assert!((a.x + a.w - 220.0).abs() < 1e-6);
  assert!((a.y + a.h - 160.0).abs() < 1e-6);
}

#[test]
fn resize_northeast_and_southwest_handles() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  // NE handle at (220, 100): move to (250, 80) -> w=150, h=80, y=80.
  s.dispatch(pointer_down(220.0, 100.0));
  s.pointer_move(250.0, 80.0, 250.0, 80.0, false);
  s.dispatch(BoardCommand::PointerUp);
  let a = s.node_by_id("a").unwrap();
  assert!((a.x - 100.0).abs() < 1e-6);
  assert!((a.y - 80.0).abs() < 1e-6);
  assert!((a.w - 150.0).abs() < 1e-6);
  assert!((a.h - 80.0).abs() < 1e-6);

  // SW handle at (100, 160): move to (80, 190) -> x=80, w=170, h=110.
  s.dispatch(pointer_down(100.0, 160.0));
  s.pointer_move(80.0, 190.0, 80.0, 190.0, false);
  s.dispatch(BoardCommand::PointerUp);
  let a = s.node_by_id("a").unwrap();
  assert!((a.x - 80.0).abs() < 1e-6);
  assert!((a.w - 170.0).abs() < 1e-6);
  assert!((a.h - 110.0).abs() < 1e-6);
}

#[test]
fn resize_east_west_north_south_cardinal_handles() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  // Note: anchor port hit has priority over resize handle at cardinal midpoints (N/E/S/W),
  // so test cardinal resize handles on a frame node (frames have no anchor ports!).
  let frame_patch = s.dispatch(BoardCommand::AddNode {
    world_x: 500.0,
    world_y: 500.0,
    kind: "frame".into(),
    icon: None,
    text: None,
    width: Some(200.0),
    height: Some(100.0),
    fill: None,
    stroke: None,
    start_editing: false,
  });
  let fid = frame_patch.active_overlay.selected_node_ids[0].clone();
  // Frame rect: x=400, y=450, w=200, h=100.
  // E handle: (600, 500) -> +40 => w=240.
  s.dispatch(pointer_down(600.0, 500.0));
  s.pointer_move(640.0, 500.0, 640.0, 500.0, false);
  s.dispatch(BoardCommand::PointerUp);
  assert!((s.node_by_id(&fid).unwrap().w - 240.0).abs() < 1e-6);

  // S handle: (520, 550) -> +20 => h=120.
  s.dispatch(pointer_down(520.0, 550.0));
  s.pointer_move(520.0, 570.0, 520.0, 570.0, false);
  s.dispatch(BoardCommand::PointerUp);
  assert!((s.node_by_id(&fid).unwrap().h - 120.0).abs() < 1e-6);

  // W handle: (400, 510) -> -30 => x=370, w=270.
  s.dispatch(pointer_down(400.0, 510.0));
  s.pointer_move(370.0, 510.0, 370.0, 510.0, false);
  s.dispatch(BoardCommand::PointerUp);
  assert!((s.node_by_id(&fid).unwrap().x - 370.0).abs() < 1e-6);
  assert!((s.node_by_id(&fid).unwrap().w - 270.0).abs() < 1e-6);

  // N handle: (505, 450) -> -15 => y=435, h=135.
  s.dispatch(pointer_down(505.0, 450.0));
  s.pointer_move(505.0, 435.0, 505.0, 435.0, false);
  s.dispatch(BoardCommand::PointerUp);
  assert!((s.node_by_id(&fid).unwrap().y - 435.0).abs() < 1e-6);
  assert!((s.node_by_id(&fid).unwrap().h - 135.0).abs() < 1e-6);
}

#[test]
fn resize_updates_incident_edge_routes_and_pushes_single_undo() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  s.dispatch(pointer_down(220.0, 160.0)); // SE
  let p = s.pointer_move(260.0, 160.0, 260.0, 160.0, false);
  let delta = p.delta.expect("resize delta");
  let route = delta
    .updated_routes
    .get("e1")
    .expect("incident edge e1 rerouted");
  assert!((route[0][0] - 260.0).abs() < 1e-6);
  s.dispatch(BoardCommand::PointerUp);
  assert_eq!(s.undo_len(), 1);
}

#[test]
fn resize_noop_without_movement_does_not_push_undo() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  s.dispatch(pointer_down(220.0, 160.0));
  s.dispatch(BoardCommand::PointerUp);
  assert!(!s.can_undo());
}

// ============================================================================
// 6. Marquee Selection & Selection Commands
// ============================================================================

#[test]
fn marquee_drag_selects_overlapping_visible_nodes() {
  let mut s = two_node_session();
  s.dispatch(pointer_down(50.0, 50.0));
  let p = s.pointer_move(250.0, 200.0, 250.0, 200.0, false);
  assert!(p.active_overlay.marquee_rect.is_some());
  assert_eq!(p.active_overlay.selected_node_ids, vec!["a"]);

  let p2 = s.pointer_move(400.0, 200.0, 400.0, 200.0, false);
  assert_eq!(p2.active_overlay.selected_node_ids, vec!["a", "b"]);

  let p_up = s.dispatch(BoardCommand::PointerUp);
  assert!(p_up.active_overlay.marquee_rect.is_none());
  assert!(p_up.rebuild_chrome);
}

#[test]
fn marquee_ignores_collapsed_descendants() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.set_viewport(1.0, [0.0, 0.0]);
  s.dispatch(BoardCommand::ToggleCollapse {
    node_id: "m0".into(),
  });
  s.dispatch(pointer_down(-500.0, -500.0));
  s.pointer_move(2000.0, 2000.0, 2000.0, 2000.0, false);
  s.dispatch(BoardCommand::PointerUp);
  assert_eq!(s.selected_node_ids(), &["m0"]);
}

#[test]
fn select_node_additive_toggles_membership() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  s.dispatch(BoardCommand::SelectNode {
    id: "b".into(),
    additive: true,
  });
  assert_eq!(s.selected_node_ids(), &["a", "b"]);
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: true,
  });
  assert_eq!(s.selected_node_ids(), &["b"]);
}

#[test]
fn select_all_skips_collapsed_descendants() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.dispatch(BoardCommand::ToggleCollapse {
    node_id: "m0".into(),
  });
  s.dispatch(BoardCommand::SelectAll);
  assert_eq!(s.selected_node_ids(), &["m0"]);
}

#[test]
fn pointer_hover_updates_hovered_node_and_skips_during_drag() {
  let mut s = two_node_session();
  let p = s.dispatch(BoardCommand::PointerHover {
    screen_x: 150.0,
    screen_y: 120.0,
  });
  assert_eq!(p.active_overlay.hovered_node_id.as_deref(), Some("a"));
  // Start dragging 'a'; hover events must be ignored while dragging.
  s.dispatch(pointer_down(150.0, 120.0));
  let p_ignored = s.dispatch(BoardCommand::PointerHover {
    screen_x: 350.0,
    screen_y: 120.0,
  });
  assert_eq!(
    p_ignored.active_overlay.hovered_node_id.as_deref(),
    Some("a")
  );
  s.dispatch(BoardCommand::PointerUp);
}

// ============================================================================
// 7. Connector Creation
// ============================================================================

#[test]
fn connector_drag_from_anchor_port_snaps_to_nearest_target_anchor() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  // Right anchor of 'a' is (220, 130).
  let p_down = s.dispatch(pointer_down(220.0, 130.0));
  assert!(p_down.active_overlay.pending_connector.is_some());

  // Move near top of 'b' (b: 320..440, 100..160; top anchor = (380, 100)).
  let p_move = s.pointer_move(380.0, 105.0, 380.0, 105.0, false);
  let pc = p_move.active_overlay.pending_connector.unwrap();
  assert_eq!(pc.target_node_id.as_deref(), Some("b"));
  assert_eq!(pc.target_anchor.as_deref(), Some("t"));
  assert!((pc.current_world[0] - 380.0).abs() < 1e-6);
  assert!((pc.current_world[1] - 100.0).abs() < 1e-6);

  let p_up = s.dispatch(BoardCommand::PointerUp);
  assert!(p_up.active_overlay.pending_connector.is_none());
  assert_eq!(s.snapshot().edges.len(), 2);
  let new_edge = s.snapshot().edges.last().unwrap();
  assert_eq!(new_edge.from_id, "a");
  assert_eq!(new_edge.to_id, "b");
  assert_eq!(new_edge.from_anchor, "r");
  assert_eq!(new_edge.to_anchor, "t");
  assert_eq!(s.selected_edge_id(), Some(new_edge.id.as_str()));
  assert!(s.routed_edges().contains_key(&new_edge.id));
}

#[test]
fn connector_drag_in_connector_tool_mode_starts_from_right_anchor() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Connector,
  });
  s.dispatch(pointer_down(150.0, 130.0));
  let pc = s
    .active_overlay()
    .pending_connector
    .expect("pending connector");
  assert_eq!(pc.from_node_id, "a");
  assert_eq!(pc.from_anchor, "r");
}

#[test]
fn connector_drag_ignores_self_and_frame_targets() {
  let snap = BoardSnapshot {
    version: 1,
    title: "conn".into(),
    mode: "whiteboard".into(),
    nodes: vec![
      make_node("a", "rounded", 100.0, 100.0, 120.0, 60.0, 1),
      make_node("fr", "frame", 300.0, 80.0, 200.0, 160.0, 0),
    ],
    edges: vec![],
    strokes: vec![],
  };
  let mut s = BoardSession::new(snap);
  s.set_viewport(1.0, [0.0, 0.0]);
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  s.dispatch(pointer_down(220.0, 130.0));
  // Over self: no target.
  let p_self = s.pointer_move(150.0, 130.0, 150.0, 130.0, false);
  assert!(
    p_self
      .active_overlay
      .pending_connector
      .unwrap()
      .target_node_id
      .is_none()
  );
  // Over frame title bar: still no target.
  let p_fr = s.pointer_move(350.0, 95.0, 350.0, 95.0, false);
  assert!(
    p_fr
      .active_overlay
      .pending_connector
      .unwrap()
      .target_node_id
      .is_none()
  );
  s.dispatch(BoardCommand::PointerUp);
  assert!(s.snapshot().edges.is_empty());
}

#[test]
fn connector_drag_released_on_empty_canvas_cancels_cleanly() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  s.dispatch(pointer_down(220.0, 130.0));
  s.pointer_move(260.0, 300.0, 260.0, 300.0, false);
  let p_up = s.dispatch(BoardCommand::PointerUp);
  assert!(!p_up.snapshot_changed);
  assert_eq!(s.snapshot().edges.len(), 1);
  assert!(!s.can_undo());
}

#[test]
fn connector_drag_uses_active_routing_and_routes_new_edge() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SetActiveRouting {
    routing: "curved".into(),
  });
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  s.dispatch(pointer_down(220.0, 130.0));
  s.pointer_move(325.0, 130.0, 325.0, 130.0, false);
  s.dispatch(BoardCommand::PointerUp);
  let new_edge = s.snapshot().edges.last().unwrap();
  assert_eq!(new_edge.routing, "curved");
  assert_eq!(s.routed_edges().get(&new_edge.id).unwrap().len(), 4);
}

// ============================================================================
// 8. Freehand Pen Strokes
// ============================================================================

#[test]
fn pen_stroke_samples_points_with_min_distance_3() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Pen,
  });
  s.dispatch(pointer_down(10.0, 10.0));
  // Move only 2px (< 3.0) -> ignored!
  let rev_before = s.active_rev();
  let p_skip = s.pointer_move(12.0, 10.0, 12.0, 10.0, false);
  assert_eq!(p_skip.active_rev, rev_before);
  // Move to (14.0, 10.0) (dist=4 >= 3.0) -> sampled!
  let p_add = s.pointer_move(14.0, 10.0, 14.0, 10.0, false);
  assert_eq!(p_add.active_rev, rev_before + 1);
  let st = p_add.active_overlay.active_stroke.unwrap();
  assert_eq!(st.points.len(), 2);
}

#[test]
fn pen_stroke_single_point_discarded_without_undo() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Pen,
  });
  s.dispatch(pointer_down(10.0, 10.0));
  let p_up = s.dispatch(BoardCommand::PointerUp);
  assert!(!p_up.snapshot_changed);
  assert!(s.snapshot().strokes.is_empty());
  assert!(!s.can_undo());
}

#[test]
fn pen_stroke_commits_multi_point_stroke_with_custom_style_and_undo() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SetPenStyle {
    color: Some("#CF222E".into()),
    width: Some(4.0),
  });
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Pen,
  });
  s.dispatch(pointer_down(0.0, 0.0));
  s.pointer_move(10.0, 5.0, 10.0, 5.0, false);
  s.pointer_move(25.0, 15.0, 25.0, 15.0, false);
  let p_up = s.dispatch(BoardCommand::PointerUp);
  assert!(p_up.snapshot_changed);
  assert_eq!(s.snapshot().strokes.len(), 1);
  assert_eq!(s.snapshot().strokes[0].color, "#CF222E");
  assert!((s.snapshot().strokes[0].width - 4.0).abs() < 1e-6);
  assert_eq!(s.snapshot().strokes[0].points.len(), 3);
  assert_eq!(s.undo_len(), 1);
  s.dispatch(BoardCommand::Undo);
  assert!(s.snapshot().strokes.is_empty());
}

#[test]
fn pen_stroke_reflected_in_active_overlay_during_drawing() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Pen,
  });
  s.dispatch(pointer_down(5.0, 5.0));
  assert!(s.active_overlay().active_stroke.is_some());
  s.dispatch(BoardCommand::PointerUp);
  assert!(s.active_overlay().active_stroke.is_none());
}

// ============================================================================
// 9. Creation Tools & Inline Editing
// ============================================================================

#[test]
fn add_node_defaults_for_shape_sticky_text_frame_icon() {
  let mut s = two_node_session();
  let p_text = s.dispatch(BoardCommand::AddNode {
    world_x: 200.0,
    world_y: 200.0,
    kind: "text".into(),
    icon: None,
    text: None,
    width: None,
    height: None,
    fill: None,
    stroke: None,
    start_editing: false,
  });
  let tid = &p_text.active_overlay.selected_node_ids[0];
  let tn = s.node_by_id(tid).unwrap();
  assert_eq!(tn.style.fill, "transparent");
  assert_eq!(tn.style.stroke, "transparent");
  assert_eq!(tn.style.align, "left");
  assert_eq!(tn.text, "输入文字");

  let p_frame = s.dispatch(BoardCommand::AddNode {
    world_x: 300.0,
    world_y: 300.0,
    kind: "frame".into(),
    icon: None,
    text: None,
    width: None,
    height: None,
    fill: None,
    stroke: None,
    start_editing: false,
  });
  let fid = &p_frame.active_overlay.selected_node_ids[0];
  let fn_node = s.node_by_id(fid).unwrap();
  assert_eq!(fn_node.z, 0);
  assert!(fn_node.style.dashed);
  assert!((fn_node.w - 380.0).abs() < 1e-6);
  assert!((fn_node.h - 240.0).abs() < 1e-6);
}

#[test]
fn pointer_down_creation_tools_shape_icon_sticky_text_frame_mindmap() {
  let mut s = two_node_session();
  // Shape tool
  s.dispatch(BoardCommand::SetActiveShape {
    kind: "diamond".into(),
  });
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Shape,
  });
  let p_shape = s.dispatch(pointer_down(500.0, 500.0));
  assert_eq!(s.tool(), BoardTool::Select);
  assert!(matches!(
    p_shape.ui_effects.first(),
    Some(UiEffect::StartInlineEdit { .. })
  ));
  s.dispatch(BoardCommand::FinishEditing);

  // Icon tool
  s.dispatch(BoardCommand::SetActiveIcon {
    icon_id: "database".into(),
  });
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Icon,
  });
  let p_icon = s.dispatch(pointer_down(600.0, 600.0));
  let icon_id = p_icon.active_overlay.selected_node_ids[0].clone();
  let icon_node = s.node_by_id(&icon_id).unwrap();
  assert_eq!(icon_node.kind, "icon_card");
  assert_eq!(icon_node.icon.as_deref(), Some("database"));
  assert_eq!(icon_node.text, "数据库");
  s.dispatch(BoardCommand::FinishEditing);

  // Sticky tool
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Sticky,
  });
  let p_sticky = s.dispatch(pointer_down(700.0, 700.0));
  let sid = p_sticky.active_overlay.selected_node_ids[0].clone();
  assert_eq!(s.node_by_id(&sid).unwrap().kind, "sticky");
  s.dispatch(BoardCommand::FinishEditing);

  // Frame tool (does NOT auto-enter inline editing)
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Frame,
  });
  let p_frame = s.dispatch(pointer_down(800.0, 800.0));
  assert!(p_frame.ui_effects.is_empty());
  assert!(s.editing_node_id().is_none());

  // Mindmap tool inserts root + 2 children in 1 undo step.
  let count_before = s.snapshot().nodes.len();
  let undo_before = s.undo_len();
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Mindmap,
  });
  let p_mm = s.dispatch(pointer_down(900.0, 900.0));
  assert_eq!(s.snapshot().nodes.len(), count_before + 3);
  assert_eq!(s.undo_len(), undo_before + 1);
  let root_id = &p_mm.active_overlay.selected_node_ids[0];
  assert_eq!(s.node_by_id(root_id).unwrap().kind, "mind_root");
}

#[test]
fn double_click_node_starts_inline_edit_and_emits_ui_effect() {
  let mut s = two_node_session();
  let first = s.dispatch(BoardCommand::PointerDown {
    screen_x: 150.0,
    screen_y: 130.0,
    button: 0,
    shift: false,
    alt: false,
    space: false,
    read_only: false,
    timestamp_ms: Some(1000),
  });
  assert!(first.ui_effects.is_empty());
  s.dispatch(BoardCommand::PointerUp);

  let second = s.dispatch(BoardCommand::PointerDown {
    screen_x: 150.0,
    screen_y: 130.0,
    button: 0,
    shift: false,
    alt: false,
    space: false,
    read_only: false,
    timestamp_ms: Some(1200),
  });
  assert_eq!(s.editing_node_id(), Some("a"));
  assert_eq!(
    second.ui_effects,
    vec![UiEffect::StartInlineEdit {
      node_id: "a".into(),
      text: "a".into(),
    }]
  );
}

#[test]
fn double_click_edge_emits_prompt_edge_label_effect() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::PointerDown {
    screen_x: 270.0,
    screen_y: 130.0,
    button: 0,
    shift: false,
    alt: false,
    space: false,
    read_only: false,
    timestamp_ms: Some(1000),
  });
  s.dispatch(BoardCommand::PointerUp);
  let second = s.dispatch(BoardCommand::PointerDown {
    screen_x: 270.0,
    screen_y: 130.0,
    button: 0,
    shift: false,
    alt: false,
    space: false,
    read_only: false,
    timestamp_ms: Some(1150),
  });
  assert_eq!(s.selected_edge_id(), Some("e1"));
  assert_eq!(s.editing_edge_id(), Some("e1"));
  assert_eq!(
    second.ui_effects,
    vec![UiEffect::PromptEdgeLabel {
      edge_id: "e1".into(),
      current_label: String::new(),
    }]
  );
}

#[test]
fn set_node_text_auto_grows_height_with_measured_height_and_icon_extra() {
  let mut s = two_node_session();
  let p_icon = s.dispatch(BoardCommand::AddNode {
    world_x: 300.0,
    world_y: 300.0,
    kind: "icon_card".into(),
    icon: Some("server".into()),
    text: Some("Srv".into()),
    width: None,
    height: Some(86.0),
    fill: None,
    stroke: None,
    start_editing: true,
  });
  let nid = p_icon.active_overlay.selected_node_ids[0].clone();
  // measured_height = 70 => needed = max(82, 70 + 16 + 30) = 116 > 86 + 1 => grows to 116.
  s.dispatch(BoardCommand::SetNodeText {
    node_id: nid.clone(),
    text: "Line1\nLine2\nLine3".into(),
    measured_height: Some(70.0),
  });
  let n = s.node_by_id(&nid).unwrap();
  assert_eq!(n.text, "Line1\nLine2\nLine3");
  assert!((n.h - 116.0).abs() < 1e-6);
}

#[test]
fn finish_editing_coalesces_undo_and_relayouts_mindmap_node() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.dispatch(BoardCommand::StartEditingNode {
    node_id: "m1".into(),
  });
  let y_m2_before = s.node_by_id("m2").unwrap().y;
  s.dispatch(BoardCommand::SetNodeText {
    node_id: "m1".into(),
    text: "多行\n分支\n文本\n很长".into(),
    measured_height: Some(120.0),
  });
  s.dispatch(BoardCommand::SetNodeText {
    node_id: "m1".into(),
    text: "多行\n分支\n文本\n很长!".into(),
    measured_height: Some(120.0),
  });
  let p_fin = s.dispatch(BoardCommand::FinishEditing);
  assert!(p_fin.active_overlay.editing_node_id.is_none());
  // Multiple set_node_text calls coalesce into a single undo entry!
  assert_eq!(s.undo_len(), 1);
  // Mindmap re-layout should have shifted m2 vertically to avoid overlap with taller m1.
  let y_m2_after = s.node_by_id("m2").unwrap().y;
  assert!((y_m2_after - y_m2_before).abs() > 1.0);
}

#[test]
fn pointer_down_inside_editing_node_keeps_editing_outside_finishes() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::StartEditingNode {
    node_id: "a".into(),
  });
  // Click inside 'a' (100..220, 100..160) -> keeps editing!
  s.dispatch(pointer_down(150.0, 130.0));
  assert_eq!(s.editing_node_id(), Some("a"));
  // Click outside on empty canvas -> finishes editing!
  s.dispatch(pointer_down(500.0, 500.0));
  assert!(s.editing_node_id().is_none());
}

#[test]
fn set_title_and_set_edge_label_record_undo() {
  let mut s = two_node_session();
  let p_empty = s.dispatch(BoardCommand::SetTitle {
    title: "   ".into(),
  });
  assert!(!p_empty.snapshot_changed);
  let p_title = s.dispatch(BoardCommand::SetTitle {
    title: "架构设计图".into(),
  });
  assert!(p_title.snapshot_changed);
  assert_eq!(s.snapshot().title, "架构设计图");

  let p_label = s.dispatch(BoardCommand::SetEdgeLabel {
    edge_id: "e1".into(),
    label: "HTTP/2".into(),
  });
  assert!(p_label.snapshot_changed);
  assert_eq!(s.snapshot().edges[0].label, "HTTP/2");
  assert_eq!(s.undo_len(), 2);
}

// ============================================================================
// 10. Mind Map Workflow: Tab, Enter, Collapse, Layout, Cascade Delete
// ============================================================================

#[test]
fn mindmap_tab_adds_child_and_enters_inline_edit() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.dispatch(BoardCommand::SelectNode {
    id: "m1".into(),
    additive: false,
  });
  let patch = s.dispatch(BoardCommand::KeyDown {
    key: "Tab".into(),
    ctrl_or_meta: false,
    shift: false,
    read_only: false,
  });
  assert_eq!(s.snapshot().nodes.len(), 5);
  assert_eq!(s.snapshot().edges.len(), 4);
  let new_id = patch.active_overlay.selected_node_ids[0].clone();
  assert_eq!(
    s.node_by_id(&new_id).unwrap().parent_id.as_deref(),
    Some("m1")
  );
  assert_eq!(s.editing_node_id(), Some(new_id.as_str()));
  assert!(matches!(
    patch.ui_effects.first(),
    Some(UiEffect::StartInlineEdit { node_id, .. }) if node_id == &new_id
  ));
}

#[test]
fn mindmap_enter_adds_sibling_on_mind_node_and_edits_normal_node() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.dispatch(BoardCommand::SelectNode {
    id: "m1".into(),
    additive: false,
  });
  let p = s.dispatch(BoardCommand::KeyDown {
    key: "Enter".into(),
    ctrl_or_meta: false,
    shift: false,
    read_only: false,
  });
  let sib_id = p.active_overlay.selected_node_ids[0].clone();
  assert_eq!(
    s.node_by_id(&sib_id).unwrap().parent_id.as_deref(),
    Some("m0")
  );
  assert_eq!(s.editing_node_id(), Some(sib_id.as_str()));

  // On a regular whiteboard node, Enter starts inline editing instead of adding sibling.
  let mut wb = two_node_session();
  wb.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  let p_wb = wb.dispatch(BoardCommand::KeyDown {
    key: "Enter".into(),
    ctrl_or_meta: false,
    shift: false,
    read_only: false,
  });
  assert_eq!(wb.snapshot().nodes.len(), 2);
  assert_eq!(p_wb.active_overlay.editing_node_id.as_deref(), Some("a"));
}

#[test]
fn mindmap_branch_palette_cycles_on_root_and_inherits_on_deeper_nodes() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  // Add 4th child under root m0 (index 3 -> purple #F5F0FF / #8250DF).
  let p4 = s.dispatch(BoardCommand::AddMindChild {
    parent_id: "m0".into(),
    start_editing: false,
  });
  let c4_id = p4.active_overlay.selected_node_ids[0].clone();
  let c4 = s.node_by_id(&c4_id).unwrap();
  assert_eq!(c4.style.fill, "#F5F0FF");
  assert_eq!(c4.style.stroke, "#8250DF");

  // Add grandchild under c4 -> fill="#FFFFFF", stroke inherits "#8250DF".
  let pg = s.dispatch(BoardCommand::AddMindChild {
    parent_id: c4_id,
    start_editing: false,
  });
  let gc_id = pg.active_overlay.selected_node_ids[0].clone();
  let gc = s.node_by_id(&gc_id).unwrap();
  assert_eq!(gc.style.fill, "#FFFFFF");
  assert_eq!(gc.style.stroke, "#8250DF");
}

#[test]
fn mindmap_collapse_badge_click_toggles_and_hides_descendants() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.set_viewport(1.0, [0.0, 0.0]);
  let badge = {
    let m0 = s.node_by_id("m0").unwrap();
    [m0.x + m0.w + 10.0, m0.y + m0.h / 2.0]
  };
  s.dispatch(pointer_down(badge[0], badge[1]));
  assert!(s.node_by_id("m0").unwrap().collapsed);
  assert_eq!(s.collapsed_descendants().len(), 3);
  assert!(s.collapsed_descendants().contains("m1"));
  assert!(s.collapsed_descendants().contains("m2"));
  assert!(s.collapsed_descendants().contains("m3"));

  // Clicking badge again expands.
  let badge2 = {
    let m0 = s.node_by_id("m0").unwrap();
    [m0.x + m0.w + 10.0, m0.y + m0.h / 2.0]
  };
  s.dispatch(pointer_down(badge2[0], badge2[1]));
  assert!(!s.node_by_id("m0").unwrap().collapsed);
  assert!(s.collapsed_descendants().is_empty());
}

#[test]
fn mindmap_add_child_uncollapses_collapsed_parent() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.dispatch(BoardCommand::ToggleCollapse {
    node_id: "m0".into(),
  });
  assert!(s.node_by_id("m0").unwrap().collapsed);
  s.dispatch(BoardCommand::AddMindChild {
    parent_id: "m0".into(),
    start_editing: false,
  });
  assert!(!s.node_by_id("m0").unwrap().collapsed);
  assert!(s.collapsed_descendants().is_empty());
}

#[test]
fn mindmap_layout_command_removes_overlaps_and_reroutes() {
  let mut snap = BoardSnapshot::template_mindmap();
  // Scramble all child positions on top of root.
  for n in &mut snap.nodes[1..] {
    n.x = 80.0;
    n.y = 180.0;
  }
  let mut s = BoardSession::new(snap);
  let patch = s.dispatch(BoardCommand::LayoutMindmap);
  assert!(patch.snapshot_changed);
  let root = s.node_by_id("m0").unwrap();
  for id in ["m1", "m2", "m3"] {
    let c = s.node_by_id(id).unwrap();
    assert!(c.x > root.x + root.w);
  }
  assert!(s.node_by_id("m1").unwrap().y < s.node_by_id("m2").unwrap().y);
  assert!(s.node_by_id("m2").unwrap().y < s.node_by_id("m3").unwrap().y);
}

#[test]
fn delete_selection_cascades_collapsed_and_expanded_descendants_and_edges() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  // Add a grandchild under m1, then collapse m1, then delete m1!
  let pg = s.dispatch(BoardCommand::AddMindChild {
    parent_id: "m1".into(),
    start_editing: false,
  });
  let gc_id = pg.active_overlay.selected_node_ids[0].clone();
  s.dispatch(BoardCommand::ToggleCollapse {
    node_id: "m1".into(),
  });
  assert!(s.collapsed_descendants().contains(&gc_id));

  s.dispatch(BoardCommand::SelectNode {
    id: "m1".into(),
    additive: false,
  });
  let p_del = s.dispatch(BoardCommand::DeleteSelection);
  let delta = p_del.delta.expect("delete delta");
  assert!(delta.removed_node_ids.contains(&"m1".to_string()));
  assert!(delta.removed_node_ids.contains(&gc_id));
  assert!(s.node_by_id("m1").is_none());
  assert!(s.node_by_id(&gc_id).is_none());
  // All incident edges removed too.
  assert!(
    !s.snapshot()
      .edges
      .iter()
      .any(|e| e.to_id == "m1" || e.from_id == "m1")
  );
  assert_eq!(s.snapshot().nodes.len(), 3); // m0, m2, m3 remain
}

// ============================================================================
// 11. Styling, Edge Style, Z-Order, Clipboard & Keyboard
// ============================================================================

#[test]
fn update_selected_nodes_style_auto_contrasts_dark_fills_and_reverts() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  // Dark brand blue -> white text.
  s.dispatch(BoardCommand::UpdateSelectedNodesStyle {
    fill: Some("#3370FF".into()),
    stroke: None,
    dashed: None,
    font_size: None,
    text_color: None,
    align: None,
    kind: None,
  });
  assert_eq!(s.node_by_id("a").unwrap().style.text_color, "#FFFFFF");

  // Dark ink -> stays white text.
  s.dispatch(BoardCommand::UpdateSelectedNodesStyle {
    fill: Some("#1F2329".into()),
    stroke: None,
    dashed: None,
    font_size: None,
    text_color: None,
    align: None,
    kind: None,
  });
  assert_eq!(s.node_by_id("a").unwrap().style.text_color, "#FFFFFF");

  // Light fill -> automatically reverts text back to #1F2329.
  s.dispatch(BoardCommand::UpdateSelectedNodesStyle {
    fill: Some("#FFF7CC".into()),
    stroke: None,
    dashed: None,
    font_size: None,
    text_color: None,
    align: None,
    kind: None,
  });
  assert_eq!(s.node_by_id("a").unwrap().style.text_color, "#1F2329");
}

#[test]
fn update_selected_nodes_style_explicit_text_color_overrides_auto_contrast() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  s.dispatch(BoardCommand::UpdateSelectedNodesStyle {
    fill: Some("#3370FF".into()),
    stroke: Some("#000000".into()),
    dashed: Some(true),
    font_size: Some(18.0),
    text_color: Some("#FFEE00".into()),
    align: Some("right".into()),
    kind: Some("hexagon".into()),
  });
  let a = s.node_by_id("a").unwrap();
  assert_eq!(a.style.text_color, "#FFEE00");
  assert_eq!(a.style.stroke, "#000000");
  assert!(a.style.dashed);
  assert!((a.style.font_size - 18.0).abs() < 1e-6);
  assert_eq!(a.style.align, "right");
  assert_eq!(a.kind, "hexagon");
}

#[test]
fn update_selected_edge_routing_clears_waypoints_and_reroutes() {
  let mut snap = BoardSnapshot::template_flowchart();
  snap.edges[0].waypoints = vec![[150.0, 120.0]];
  let mut s = BoardSession::new(snap);
  s.dispatch(BoardCommand::SelectEdge { id: "fe1".into() });
  let patch = s.dispatch(BoardCommand::UpdateSelectedEdge {
    routing: Some("curved".into()),
    stroke: Some("#CF222E".into()),
    dashed: Some(true),
    arrow_end: Some("diamond".into()),
    arrow_start: Some("dot".into()),
  });
  let e = &s.snapshot().edges[0];
  assert_eq!(e.routing, "curved");
  assert!(e.waypoints.is_empty());
  assert_eq!(e.style.stroke, "#CF222E");
  assert!(e.style.dashed);
  assert_eq!(e.arrow_end, "diamond");
  assert_eq!(e.arrow_start, "dot");
  let route = patch
    .delta
    .unwrap()
    .updated_routes
    .get("fe1")
    .unwrap()
    .clone();
  assert_eq!(route.len(), 4);
}

#[test]
fn bring_to_front_and_send_to_back_adjust_z_order() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  s.dispatch(BoardCommand::BringToFront);
  assert!(s.node_by_id("a").unwrap().z > s.node_by_id("b").unwrap().z);

  s.dispatch(BoardCommand::SendToBack);
  assert!(s.node_by_id("a").unwrap().z < s.node_by_id("b").unwrap().z);
}

#[test]
fn copy_paste_duplicate_remaps_internal_edges_and_offsets_nodes() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectAll);
  s.dispatch(BoardCommand::DuplicateSelection);
  assert_eq!(s.snapshot().nodes.len(), 4);
  assert_eq!(s.snapshot().edges.len(), 2);
  let pasted_ids = s.selected_node_ids().to_vec();
  assert_eq!(pasted_ids.len(), 2);
  assert!(!pasted_ids.contains(&"a".to_string()));
  assert!(!pasted_ids.contains(&"b".to_string()));
  let p0 = s.node_by_id(&pasted_ids[0]).unwrap();
  assert!((p0.x - 128.0).abs() < 1e-6);
  assert!((p0.y - 128.0).abs() < 1e-6);
  // Cloned edge must connect the two newly pasted nodes!
  let cloned_edge = s.snapshot().edges.last().unwrap();
  assert_eq!(cloned_edge.from_id, pasted_ids[0]);
  assert_eq!(cloned_edge.to_id, pasted_ids[1]);
}

#[test]
fn keyboard_shortcuts_tools_select_all_copy_paste_delete_escape_close_editor() {
  let mut s = two_node_session();
  for (key, expected) in [
    ("h", BoardTool::Hand),
    ("s", BoardTool::Sticky),
    ("t", BoardTool::Text),
    ("l", BoardTool::Connector),
    ("p", BoardTool::Pen),
    ("f", BoardTool::Frame),
    ("m", BoardTool::Mindmap),
    ("v", BoardTool::Select),
  ] {
    s.dispatch(BoardCommand::KeyDown {
      key: key.into(),
      ctrl_or_meta: false,
      shift: false,
      read_only: false,
    });
    assert_eq!(s.tool(), expected);
  }

  // Ctrl+A -> selects all 2 nodes.
  s.dispatch(BoardCommand::KeyDown {
    key: "a".into(),
    ctrl_or_meta: true,
    shift: false,
    read_only: false,
  });
  assert_eq!(s.selected_node_ids().len(), 2);

  // Escape #1 -> clears selection.
  let p1 = s.dispatch(BoardCommand::KeyDown {
    key: "Escape".into(),
    ctrl_or_meta: false,
    shift: false,
    read_only: false,
  });
  assert!(s.selected_node_ids().is_empty());
  assert!(p1.ui_effects.is_empty());

  // Switch tool to Pen, Escape #2 -> returns to Select tool.
  s.dispatch(BoardCommand::SetTool {
    tool: BoardTool::Pen,
  });
  let p2 = s.dispatch(BoardCommand::KeyDown {
    key: "Escape".into(),
    ctrl_or_meta: false,
    shift: false,
    read_only: false,
  });
  assert_eq!(s.tool(), BoardTool::Select);
  assert!(p2.ui_effects.is_empty());

  // Escape #3 (nothing selected, Select tool) -> CloseEditor!
  let p3 = s.dispatch(BoardCommand::KeyDown {
    key: "Escape".into(),
    ctrl_or_meta: false,
    shift: false,
    read_only: false,
  });
  assert_eq!(p3.ui_effects, vec![UiEffect::CloseEditor]);
}

#[test]
fn keyboard_shortcuts_readonly_only_allows_escape_close() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  s.dispatch(BoardCommand::KeyDown {
    key: "Delete".into(),
    ctrl_or_meta: false,
    shift: false,
    read_only: true,
  });
  assert_eq!(s.snapshot().nodes.len(), 2);

  let p_esc = s.dispatch(BoardCommand::KeyDown {
    key: "Escape".into(),
    ctrl_or_meta: false,
    shift: false,
    read_only: true,
  });
  assert_eq!(p_esc.ui_effects, vec![UiEffect::CloseEditor]);
}

// ============================================================================
// 12. Undo / Redo & Replace Snapshot
// ============================================================================

#[test]
fn undo_redo_100_step_cap_and_redo_cleared_on_new_mutation() {
  let mut s = two_node_session();
  for i in 0..120 {
    s.dispatch(BoardCommand::SetTitle {
      title: format!("Title {i}"),
    });
  }
  // Capped at 100 steps.
  assert_eq!(s.undo_len(), 100);
  assert_eq!(s.snapshot().title, "Title 119");

  // Undo 5 times, verify redo works via Ctrl+Shift+Z and Ctrl+Y.
  for _ in 0..5 {
    s.dispatch(BoardCommand::KeyDown {
      key: "z".into(),
      ctrl_or_meta: true,
      shift: false,
      read_only: false,
    });
  }
  assert_eq!(s.snapshot().title, "Title 114");
  assert_eq!(s.redo_len(), 5);

  s.dispatch(BoardCommand::KeyDown {
    key: "z".into(),
    ctrl_or_meta: true,
    shift: true,
    read_only: false,
  });
  assert_eq!(s.snapshot().title, "Title 115");

  s.dispatch(BoardCommand::KeyDown {
    key: "y".into(),
    ctrl_or_meta: true,
    shift: false,
    read_only: false,
  });
  assert_eq!(s.snapshot().title, "Title 116");

  // New mutation clears redo stack.
  s.dispatch(BoardCommand::SetTitle {
    title: "Branch".into(),
  });
  assert!(!s.can_redo());

  // Undo all remaining 98 steps cleanly to empty stack.
  while s.can_undo() {
    s.dispatch(BoardCommand::Undo);
  }
  assert_eq!(s.undo_len(), 0);
  // Extra undo is safe no-op.
  let rev = s.graph_rev();
  let p = s.dispatch(BoardCommand::Undo);
  assert_eq!(p.graph_rev, rev);
}

#[test]
fn undo_redo_restores_collapsed_descendants_and_routes() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.dispatch(BoardCommand::ToggleCollapse {
    node_id: "m0".into(),
  });
  assert_eq!(s.collapsed_descendants().len(), 3);
  s.dispatch(BoardCommand::Undo);
  assert!(s.collapsed_descendants().is_empty());
  assert_eq!(s.routed_edges().len(), 3);
  s.dispatch(BoardCommand::Redo);
  assert_eq!(s.collapsed_descendants().len(), 3);
}

#[test]
fn replace_snapshot_with_and_without_undo() {
  let mut s = two_node_session();
  let next = BoardSnapshot::template_flowchart();
  s.dispatch(BoardCommand::ReplaceSnapshot {
    snapshot: next.clone(),
    record_undo: true,
  });
  assert_eq!(s.snapshot().mode, "flowchart");
  assert!(s.can_undo());
  s.dispatch(BoardCommand::Undo);
  assert_eq!(s.snapshot().mode, "whiteboard");

  s.dispatch(BoardCommand::ReplaceSnapshot {
    snapshot: next,
    record_undo: false,
  });
  assert_eq!(s.snapshot().mode, "flowchart");
  assert!(!s.can_undo());
}

#[test]
fn delete_selected_edge_and_undo_restores_edge() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectEdge { id: "e1".into() });
  let p_del = s.dispatch(BoardCommand::DeleteSelection);
  assert!(s.snapshot().edges.is_empty());
  assert!(s.selected_edge_id().is_none());
  assert_eq!(p_del.delta.unwrap().removed_edge_ids, vec!["e1"]);
  s.dispatch(BoardCommand::Undo);
  assert_eq!(s.snapshot().edges.len(), 1);
  assert!(s.routed_edges().contains_key("e1"));
}

#[test]
fn key_escape_and_ctrl_enter_finish_inline_editing() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::StartEditingNode {
    node_id: "a".into(),
  });
  s.dispatch(BoardCommand::SetNodeText {
    node_id: "a".into(),
    text: "edited".into(),
    measured_height: None,
  });
  s.dispatch(BoardCommand::KeyDown {
    key: "Escape".into(),
    ctrl_or_meta: false,
    shift: false,
    read_only: false,
  });
  assert!(s.editing_node_id().is_none());
  assert_eq!(s.node_by_id("a").unwrap().text, "edited");

  s.dispatch(BoardCommand::StartEditingNode {
    node_id: "a".into(),
  });
  s.dispatch(BoardCommand::KeyDown {
    key: "Enter".into(),
    ctrl_or_meta: true,
    shift: false,
    read_only: false,
  });
  assert!(s.editing_node_id().is_none());
}

#[test]
fn add_mind_sibling_on_root_adds_child_instead() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  let before = s.snapshot().nodes.len();
  let p = s.dispatch(BoardCommand::AddMindSibling {
    node_id: "m0".into(),
    start_editing: false,
  });
  assert_eq!(s.snapshot().nodes.len(), before + 1);
  let new_id = &p.active_overlay.selected_node_ids[0];
  assert_eq!(
    s.node_by_id(new_id).unwrap().parent_id.as_deref(),
    Some("m0")
  );
}

#[test]
fn drag_smart_snap_zoom_clamps_threshold_between_2_5_and_6() {
  let snap = BoardSnapshot {
    version: 1,
    title: "zoom_snap".into(),
    mode: "whiteboard".into(),
    nodes: vec![
      make_node("a", "rounded", 100.0, 100.0, 100.0, 50.0, 1),
      make_node("b", "rounded", 250.0, 260.0, 100.0, 50.0, 2),
    ],
    edges: vec![],
    strokes: vec![],
  };
  let mut s = BoardSession::new(snap);
  // At 3.0x zoom, threshold = (6 / 3).clamp(2.5, 6) = 2.5px.
  s.set_viewport(3.0, [0.0, 0.0]);
  s.dispatch(pointer_down(450.0, 375.0)); // world (150, 125)
  // Move to raw x = 246 (4px away from 250 > 2.5px) -> should NOT snap.
  s.pointer_move(296.0, 125.0, 888.0, 375.0, false);
  assert!((s.node_by_id("a").unwrap().x - 246.0).abs() < 1e-6);
  // Move to raw x = 248 (2px away from 250 < 2.5px) -> SHOULD snap to 250!
  s.pointer_move(298.0, 125.0, 894.0, 375.0, false);
  assert!((s.node_by_id("a").unwrap().x - 250.0).abs() < 1e-6);
}

#[test]
fn paste_with_empty_clipboard_is_noop() {
  let mut s = two_node_session();
  let rev = s.graph_rev();
  let p = s.dispatch(BoardCommand::PasteSelection { offset: None });
  assert!(!p.snapshot_changed);
  assert_eq!(p.graph_rev, rev);
}

#[test]
fn copy_paste_custom_offset_and_clears_parent_id() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.dispatch(BoardCommand::SelectNode {
    id: "m1".into(),
    additive: false,
  });
  let orig_x = s.node_by_id("m1").unwrap().x;
  let orig_y = s.node_by_id("m1").unwrap().y;
  s.dispatch(BoardCommand::CopySelection);
  let p = s.dispatch(BoardCommand::PasteSelection {
    offset: Some([50.0, -20.0]),
  });
  let new_id = &p.active_overlay.selected_node_ids[0];
  let pasted = s.node_by_id(new_id).unwrap();
  assert!(pasted.parent_id.is_none());
  assert!((pasted.x - (orig_x + 50.0)).abs() < 1e-6);
  assert!((pasted.y - (orig_y - 20.0)).abs() < 1e-6);
}

#[test]
fn dispatch_json_covers_all_core_commands() {
  let mut s = two_node_session();
  let cmds = [
    r#"{"type":"setViewport","zoom":1.2,"pan":[10.0,20.0]}"#,
    r#"{"type":"setTool","tool":"sticky"}"#,
    r#"{"type":"addNode","worldX":200.0,"worldY":200.0,"kind":"sticky"}"#,
    r#"{"type":"setTitle","title":"JSON测试"}"#,
    r##"{"type":"updateSelectedNodesStyle","fill":"#3370FF"}"##,
    r#"{"type":"bringToFront"}"#,
    r#"{"type":"sendToBack"}"#,
    r#"{"type":"duplicateSelection"}"#,
    r#"{"type":"deleteSelection"}"#,
    r#"{"type":"undo"}"#,
    r#"{"type":"redo"}"#,
    r#"{"type":"fitToContent","viewport":[1280.0,800.0]}"#,
  ];
  for c in cmds {
    let out = s
      .dispatch_json(c)
      .unwrap_or_else(|e| panic!("failed {c}: {e}"));
    let _: BoardPatch = serde_json::from_str(&out).expect("valid patch json");
  }
}

#[test]
fn clear_selection_noop_when_empty_and_clears_when_populated() {
  let mut s = two_node_session();
  let r0 = s.active_rev();
  let p0 = s.dispatch(BoardCommand::ClearSelection);
  assert_eq!(p0.active_rev, r0);
  s.dispatch(BoardCommand::SelectEdge { id: "e1".into() });
  let p1 = s.dispatch(BoardCommand::ClearSelection);
  assert!(p1.active_overlay.selected_edge_id.is_none());
  assert!(p1.rebuild_chrome);
}

#[test]
fn set_viewport_size_and_fit_to_content_uses_stored_viewport() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SetViewportSize {
    width: 800.0,
    height: 600.0,
  });
  assert_eq!(s.viewport(), [800.0, 600.0]);
  s.dispatch(BoardCommand::FitToContent { viewport: None });
  let sc = s.world_to_screen([270.0, 130.0]);
  assert!((sc[0] - 400.0).abs() < 1e-6);
  assert!((sc[1] - 300.0).abs() < 1e-6);
}

#[test]
fn pointer_down_on_node_with_shift_adds_to_selection_and_drags_group() {
  let mut s = two_node_session();
  s.dispatch(BoardCommand::SelectNode {
    id: "a".into(),
    additive: false,
  });
  s.dispatch(BoardCommand::PointerDown {
    screen_x: 350.0,
    screen_y: 130.0,
    button: 0,
    shift: true,
    alt: false,
    space: false,
    read_only: false,
    timestamp_ms: None,
  });
  assert_eq!(s.selected_node_ids(), &["a", "b"]);
  s.pointer_move(370.0, 150.0, 370.0, 150.0, false);
  s.dispatch(BoardCommand::PointerUp);
  assert!((s.node_by_id("a").unwrap().x - 120.0).abs() < 1e-6);
  assert!((s.node_by_id("b").unwrap().x - 340.0).abs() < 1e-6);
}

#[test]
fn orthogonal_edge_reroutes_around_dragged_obstacle_on_pointer_up() {
  let mut s = BoardSession::new(BoardSnapshot::template_flowchart());
  s.set_viewport(1.0, [0.0, 0.0]);
  // Add an obstacle off to the side, then drag it right between f1 and f2 (x=200..380, y=112..142).
  let p = s.dispatch(BoardCommand::AddNode {
    world_x: 600.0,
    world_y: 127.0,
    kind: "rect".into(),
    icon: None,
    text: Some("obs".into()),
    width: Some(180.0),
    height: Some(30.0),
    fill: None,
    stroke: None,
    start_editing: false,
  });
  let obs_id = p.active_overlay.selected_node_ids[0].clone();
  let fe1_before = s.routed_edges().get("fe1").unwrap().len();
  // Drag obs from center (600, 127) to (290, 127).
  s.dispatch(pointer_down(600.0, 127.0));
  s.pointer_move(290.0, 127.0, 290.0, 127.0, true);
  let p_up = s.dispatch(BoardCommand::PointerUp);
  let fe1_after = s.routed_edges().get("fe1").unwrap().len();
  assert!(fe1_after >= 4 && fe1_after >= fe1_before);
  assert!(p_up.delta.unwrap().updated_routes.contains_key("fe1"));
  assert_eq!(s.node_by_id(&obs_id).unwrap().x, 200.0);
}

#[test]
fn set_node_text_outside_edit_mode_bumps_graph_rev() {
  let mut s = two_node_session();
  let r0 = s.graph_rev();
  let p = s.dispatch(BoardCommand::SetNodeText {
    node_id: "a".into(),
    text: "direct update".into(),
    measured_height: None,
  });
  assert_eq!(p.graph_rev, r0 + 1);
  assert_eq!(s.node_by_id("a").unwrap().text, "direct update");
}

#[test]
fn toggle_collapse_deselects_newly_hidden_descendant() {
  let mut s = BoardSession::new(BoardSnapshot::template_mindmap());
  s.dispatch(BoardCommand::SelectNode {
    id: "m1".into(),
    additive: false,
  });
  s.dispatch(BoardCommand::ToggleCollapse {
    node_id: "m0".into(),
  });
  assert!(s.selected_node_ids().is_empty());
}
