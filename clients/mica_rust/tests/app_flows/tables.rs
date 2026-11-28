use nsh_core::geometry::{px, size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[test]
fn regression_table_wysiwyg_chrome_handles_expand_bars_and_hscroll() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();

  let mut paste = nsh_core::PasteEvent::default();
  paste.text = "| 编号 | **待确认事项** | 责任方 |\n| --- | --- | --- |\n| IDC-01 | **单机柜可用功率** (kW/柜) | IDC |\n| NET-02 | Q3400-RA / 200G / 接入交换机准确型号与端口 | 网络供应商 |".into();
  app.paste("id:mica-editor-ime-sink", &paste).unwrap();
  app.wait_for("id:node-view-table").unwrap();
  app.wait_for("IDC-01").unwrap();
  app.wait_for("单机柜可用功率 (kW/柜)").unwrap();

  // 1. Clicking a header or body cell mounts the transparent WYSIWYG caret + IME proxy
  //    WITHOUT hiding/replacing the cell's rich rendered text!
  app.press("id:table-cell-0-0-1").unwrap();
  app.wait_for("id:table-cell-input-0-0-1").unwrap();
  app.wait_for("id:mica-table-active-caret").unwrap();
  app.wait_for("待确认事项").unwrap();

  // 2. Full-height column resize handles exist for inner borders AND right edge,
  //    and the horizontal scrollbar element is mounted.
  app.wait_for("id:table-col-resize-0-1").unwrap();
  app.wait_for("id:table-col-resize-0-2").unwrap();
  app.wait_for("id:table-col-resize-0-3").unwrap();
  app.wait_for("id:table-horizontal-scrollbar").unwrap();

  // 3. Legacy chrome: top-left corner menu, top column anchor menu, left row anchor menu,
  //    and right/bottom `+` expand bars all work!
  app.press("id:table-corner-handle").unwrap();
  app.wait_for("id:table-corner-menu").unwrap();
  app.press("id:table-autofit").unwrap();

  app.press("id:table-col-handle-1").unwrap();
  app.wait_for("id:table-col-menu").unwrap();
  app.press("id:table-col-align-center").unwrap();

  app.press("id:table-row-handle-1").unwrap();
  app.wait_for("id:table-row-menu").unwrap();
  app.press("id:table-row-insert-below").unwrap();
  // Original 3 rows -> 4 rows (`table-row-handle-3` now exists!)
  app.wait_for("id:table-row-handle-3").unwrap();

  // Bottom `+` bar appends a 5th row (`table-row-handle-4`), right `+` bar appends a 4th column (`table-col-handle-3`).
  app.press("id:table-add-row-bar").unwrap();
  app.wait_for("id:table-row-handle-4").unwrap();
  app.press("id:table-add-col-bar").unwrap();
  app.wait_for("id:table-col-handle-3").unwrap();

  // 4. Verify the table grid's left edge (`table-cell-0-0-0`) aligns flush with the
  //    page title's left edge (`page-title-input`) within 2.5px (the left row-handle
  //    gutter sits cleanly in the negative left margin).
  let nodes = app.screen().layout_nodes();
  let title_box = nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("page-title-input"))
    .expect("title input present")
    .bounds;
  let cell_box = nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("table-cell-0-0-0"))
    .expect("first table cell present")
    .bounds;
  assert!(
    (cell_box.origin.x.0 - title_box.origin.x.0).abs() <= 2.5,
    "table left edge ({}) must align flush with page left edge ({})",
    cell_box.origin.x.0,
    title_box.origin.x.0
  );

  // 5. Code block language picker closes cleanly on Escape AND on outside click.
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("id:slash-code").unwrap();
  app.press("id:slash-code").unwrap();
  app.wait_for("id:code-lang-btn").unwrap();
  app.press("id:code-lang-btn").unwrap();
  app.wait_for("id:code-lang-picker").unwrap();
  app.press_key("escape").unwrap();
  app
    .wait_for_gone("id:code-lang-picker", std::time::Duration::from_millis(800))
    .unwrap();
  app.press("id:code-lang-btn").unwrap();
  app.wait_for("id:code-lang-picker").unwrap();
  app.press("id:table-cell-0-0-0").unwrap();
  app
    .wait_for_gone("id:code-lang-picker", std::time::Duration::from_millis(800))
    .unwrap();
}



#[nsh_core::prelude::component]
fn TableCellMidCaretHarness() -> impl nsh_core::prelude::IntoElement {
  use std::{cell::RefCell, rc::Rc};
  use nsh_core::prelude::*;

  let md = "\
| DGX OS 大版本 | 底层 Ubuntu 版本 | 典型适配硬件与定位 |\n\
| --- | --- | --- |\n\
| **DGX OS 7** | **Ubuntu 24.04 LTS** (7.6.0 对应 24.04.5) | 当前 DGX B200 / B300 生产主力基线 |\n";
  let bridge = mica_rust::editor_bridge::MicaDocumentBridge::from_markdown(md);
  let layouts = Rc::new(RefCell::new(mica_rust::text_geometry::VisibleLayoutCache::default()));
  let dark_mode = signal(false);
  let page_width = signal(880.0_f32);
  let env = mica_rust::block_views::BlockRenderEnv {
    bridge: bridge.clone(),
    dark_mode,
    page_width,
    layouts,
    drag_anchor: signal(None),
    last_click: signal((0, 0, 0.0, 0)),
    lightbox: signal(None),
    image_ctx: mica_rust::image_blocks::ImageContext::offline_default(),
    focused_code_block: signal(None),
    focused_table_cell: signal(None),
    table_selection: signal(None),
    table_drag_anchor: signal(None),
    marquee_selected: signal(None),
    body_blank_marquee_ready: signal(false),
    marquee_origin_page: signal(None),
    editor_focused: signal(false),
    caret_blink_on: signal(true),
    sink_clear_rev: signal(0),
    inline_math_edit: signal(None),
    inner_drag_hit: Rc::new(std::cell::Cell::new(None)),
    last_pointer_is_touch: signal(false),
    touch_selection_active: signal(false),
    touch_tap_tracker: Rc::new(RefCell::new(mica_rust::touch_selection::TouchTapTracker::default())),
  };
  let structure_rev = bridge.structure_rev_signal();
  let br_count = bridge.clone();
  let br_keys = bridge.clone();

  view()
    .flex_1()
    .w_full()
    .h_full()
    .pt(px(140.0))
    .pl(px(180.0))
    .child(
      virtual_list(
        move || {
          structure_rev.get();
          br_count.block_len()
        },
        move |at| mica_rust::block_views::render_virtual_block_row(env.clone(), at),
      )
      .keys(move || {
        structure_rev.get();
        br_keys.block_ids()
      })
      .estimated_item_size(px(120.0))
      .flex_1()
      .w_full()
      .h_full(),
    )
}

#[test]
fn regression_table_cell_mid_click_places_caret_in_middle_and_drag_selects_substring() {
  use nsh_core::{
    EventKind, EventPayload, MouseButton, MouseEvent, PayloadValue, PressEvent,
    geometry::point,
    keymap::Modifiers,
  };

  let mut config = EngineConfig::new(Root::of::<TableCellMidCaretHarness>());
  config.viewport = size(px(1100.0), px(700.0));
  let mut app = Headless::start(config).unwrap();

  app.wait_for("id:table-cell-0-1-1").unwrap();
  let cell_node = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("table-cell-0-1-1"))
    .expect("target table cell must exist");
  let cell_id = cell_node.id;
  let cell_bounds = cell_node.bounds;

  // 1. Click near the start/middle of the cell (local x=48px inside a >200px cell)
  //    in a table offset far from window (0, 0) (`pl=180, pt=140`).
  //    Previously, `on_press_down` subtracted parent-local origin from `page_position`
  //    AND `on_press` snapped the caret back to end-of-cell on mouse-up!
  app
    .press_at("id:table-cell-0-1-1", point(px(48.0), px(18.0)))
    .unwrap();
  app.wait_for("id:mica-table-active-caret").unwrap();

  // Simulate the Windows RichEdit `set_value` -> `EM_SETSEL(end, end)` spurious
  // initial `SelectionChange` echo right after mount: caret must NOT jump to end!
  let input_id = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("table-cell-input-0-1-1"))
    .expect("active table cell input proxy must exist")
    .id;
  app.engine().control().dispatch_event(
    input_id,
    EventKind::SelectionChange,
    EventPayload::Map(vec![
      ("start", PayloadValue::Number(36.0)),
      ("end", PayloadValue::Number(36.0)),
    ]),
  );
  app.settle();

  let caret_bounds = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("mica-table-active-caret"))
    .expect("active table caret must be rendered")
    .bounds;
  let caret_local_x = caret_bounds.origin.x.0 - cell_bounds.origin.x.0;
  assert!(
    caret_local_x > 16.0 && caret_local_x < 96.0,
    "clicking at local x=48 must place caret near the clicked character and resist RichEdit end-echo (got local x={caret_local_x:.1}, cell width={:.1})",
    cell_bounds.size.width.0
  );

  // 2. Desktop left-button drag within the same cell from local x=48 to x=148:
  //    dispatch the exact desktop mouse sequence (`PressDown` -> `MouseMove(Left)` -> `MouseUp` -> `Press`).
  //    Must highlight a substring inside the cell and KEEP it selected after release!
  let start_local = point(px(48.0), px(18.0));
  let start_page = point(
    cell_bounds.origin.x + start_local.x,
    cell_bounds.origin.y + start_local.y,
  );
  let end_local = point(px(148.0), px(18.0));
  let end_page = point(
    cell_bounds.origin.x + end_local.x,
    cell_bounds.origin.y + end_local.y,
  );
  let control = app.engine().control();
  control.dispatch_event(
    cell_id,
    EventKind::PressDown,
    EventPayload::Press(PressEvent {
      position: start_local,
      page_position: start_page,
      modifiers: Modifiers::default(),
      clicks: 1,
    }),
  );
  control.dispatch_event(
    cell_id,
    EventKind::MouseMove,
    EventPayload::Mouse(MouseEvent::new(
      end_local,
      end_page,
      Modifiers::default(),
      Some(MouseButton::Left),
    )),
  );
  control.dispatch_event(
    cell_id,
    EventKind::MouseUp,
    EventPayload::Mouse(MouseEvent::released(
      end_local,
      end_page,
      Modifiers::default(),
      MouseButton::Left,
    )),
  );
  // Desktop also delivers `on_press` when mouse-up occurs inside the same view.
  control.dispatch_event(cell_id, EventKind::Press, EventPayload::None);

  app.wait_for("id:mica-table-cell-selection-rect").unwrap();
  let sel_rect = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("mica-table-cell-selection-rect"))
    .expect("intra-cell selection highlight must remain visible after drag release")
    .bounds;
  assert!(
    sel_rect.size.width.0 >= 50.0,
    "intra-cell drag selection must highlight dragged substring (got width={:.1})",
    sel_rect.size.width.0
  );
  let sel_local_x = sel_rect.origin.x.0 - cell_bounds.origin.x.0;
  assert!(
    sel_local_x > 16.0 && sel_local_x < 96.0,
    "intra-cell selection highlight must start near drag anchor x=48 (got local x={sel_local_x:.1})"
  );
}

#[test]
fn regression_table_default_fits_page_live_column_drag_and_right_edge_resize() {
  use nsh_core::geometry::point;

  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();

  // Paste a 5-column table with long Chinese/English content (same class as the bug report).
  let mut paste = nsh_core::PasteEvent::default();
  paste.text = concat!(
    "| Fortinet 公告编号 | CVE 编号 | 严重级别 / CVSSv3 | 漏洞类型与受影响组件 | 风险描述 |\n",
    "| --- | --- | --- | --- | --- |\n",
    "| **FG-IR-26-268** | CVE-2026-126254 | **Critical (9.6)** | 数值截断错误 (CWE-197) / BGP Daemon | 已认证 iBGP 邻居可发送特制报文，触发路由属性异常 |\n",
    "| **FG-IR-26-267** | CVE-2026-126253 | **Critical (9.3)** | 认证绕过 (CWE-305) / OSPF Daemon | 相邻网段未认证攻击者可通过特制报文绕过认证 |\n",
  )
  .into();
  app.paste("id:mica-editor-ime-sink", &paste).unwrap();
  app.wait_for("id:node-view-table").unwrap();
  app.wait_for("id:table-col-resize-0-5").unwrap();
  app.settle();

  let bounds_of = |app: &Headless, tid: &str| {
    app
      .screen()
      .layout_nodes()
      .into_iter()
      .find(|n| n.test_id.as_deref() == Some(tid))
      .unwrap_or_else(|| panic!("{tid} must exist in layout tree"))
      .bounds
  };
  let total_cols_w = |app: &Headless| -> f32 {
    (0..5)
      .map(|c| bounds_of(app, &format!("table-cell-0-0-{c}")).size.width.0)
      .sum()
  };

  // 1. Default 5-column table MUST fit within page column width (no horizontal overflow / scrollbar).
  let title_box = bounds_of(&app, "page-title-input");
  let initial_total_w = total_cols_w(&app);
  let initial_sb = bounds_of(&app, "table-horizontal-scrollbar");
  assert!(
    initial_total_w <= title_box.size.width.0 + 3.0,
    "default 5-col table width ({initial_total_w}) must fit page width ({})",
    title_box.size.width.0
  );
  assert!(
    initial_sb.size.height.0 <= 1.5,
    "default page-fit table must not show horizontal scrollbar (got height {})",
    initial_sb.size.height.0
  );

  // 2. Dragging an inner column divider (`table-col-resize-0-2`) MUST update cell widths
  //    in REAL TIME while the pointer is still held down (before release)!
  let col1_before = bounds_of(&app, "table-cell-0-0-1").size.width.0;
  let col2_before = bounds_of(&app, "table-cell-0-0-2").size.width.0;
  app
    .drag_and_hold("id:table-col-resize-0-2", point(px(45.0), px(0.0)))
    .unwrap();
  let col1_mid = bounds_of(&app, "table-cell-0-0-1").size.width.0;
  let col2_mid = bounds_of(&app, "table-cell-0-0-2").size.width.0;
  assert!(
    col1_mid > col1_before + 30.0 && col2_mid < col2_before - 30.0,
    "column widths must update live mid-drag before release: col1 {col1_before}->{col1_mid}, col2 {col2_before}->{col2_mid}"
  );
  app.drop_held().unwrap();
  app.settle();
  let col1_after = bounds_of(&app, "table-cell-0-0-1").size.width.0;
  assert!(
    (col1_after - col1_mid).abs() <= 4.0,
    "persisted column width after release ({col1_after}) must match live drag width ({col1_mid})"
  );

  // 3. Dragging the RIGHTMOST edge (`table-col-resize-0-5`) right MUST widen the last column
  //    and total table width past page width LIVE during drag, revealing the horizontal scrollbar,
  //    and persist after release!
  let col0_before_right = bounds_of(&app, "table-cell-0-0-0").size.width.0;
  let col4_before_right = bounds_of(&app, "table-cell-0-0-4").size.width.0;
  let total_before_right = total_cols_w(&app);
  app
    .drag_and_hold("id:table-col-resize-0-5", point(px(180.0), px(0.0)))
    .unwrap();
  let col0_mid_right = bounds_of(&app, "table-cell-0-0-0").size.width.0;
  let col4_mid_right = bounds_of(&app, "table-cell-0-0-4").size.width.0;
  let total_mid_right = total_cols_w(&app);
  let sb_mid_right = bounds_of(&app, "table-horizontal-scrollbar");
  assert!(
    col4_mid_right > col4_before_right + 140.0,
    "dragging rightmost edge right must live-widen last column: {col4_before_right}->{col4_mid_right}"
  );
  assert!(
    (col0_mid_right - col0_before_right).abs() <= 2.0,
    "dragging rightmost edge right must keep earlier columns stable: {col0_before_right}->{col0_mid_right}"
  );
  assert!(
    total_mid_right > title_box.size.width.0 + 140.0,
    "dragging rightmost edge right must allow table to exceed page width ({total_mid_right} vs {})",
    title_box.size.width.0
  );
  assert!(
    sb_mid_right.size.height.0 >= 7.0,
    "horizontal scrollbar must appear live once dragged past page width (got height {})",
    sb_mid_right.size.height.0
  );

  app.drop_held().unwrap();
  app.settle();
  let total_after_right = total_cols_w(&app);
  let sb_after_right = bounds_of(&app, "table-horizontal-scrollbar");
  assert!(
    total_after_right > total_before_right + 140.0,
    "widened table width must persist after releasing rightmost edge: {total_before_right}->{total_after_right}"
  );
  assert!(
    sb_after_right.size.height.0 >= 7.0,
    "horizontal scrollbar must remain visible after releasing overflow drag"
  );

  // 4. Dragging the rightmost edge back left shrinks the table back within page width.
  app
    .drag_and_hold("id:table-col-resize-0-5", point(px(-220.0), px(0.0)))
    .unwrap();
  let total_mid_narrow = total_cols_w(&app);
  assert!(
    total_mid_narrow < total_after_right - 160.0,
    "dragging rightmost edge left must live-narrow table: {total_after_right}->{total_mid_narrow}"
  );
  app.drop_held().unwrap();
  app.settle();
  let total_after_narrow = total_cols_w(&app);
  assert!(
    total_after_narrow <= title_box.size.width.0 + 2.0,
    "narrowed table must fit back inside page width ({total_after_narrow} vs {})",
    title_box.size.width.0
  );
}


