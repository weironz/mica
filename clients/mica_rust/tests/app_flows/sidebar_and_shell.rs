use nsh_core::geometry::{px, size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[test]
fn regression_recycle_bin_hidden_copy_path_appearance_and_board_click_edit() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Recycle Bin must NOT be rendered in the sidebar footer (lives in Settings -> 回收站 only).
  assert!(
    app.press("id:toggle-recycle-bin").is_err(),
    "sidebar footer must not display Recycle Bin"
  );

  // 2. Breadcrumb is wrapped in a selectable view and copy-path button copies human path.
  app.wait_for("id:ed-breadcrumb-selectable").unwrap();
  app.wait_for("id:ed-copy-path").unwrap();
  app.press("id:ed-copy-path").unwrap();
  app.wait_for("已复制页面路径").unwrap();

  // 3. Settings Appearance: Language & UI Size take effect immediately across Settings & Sidebar.
  app.press("id:open-settings-btn").unwrap();
  app.wait_for("id:lang-en").unwrap();
  app.press("id:lang-en").unwrap();
  app.press("id:ui-font-large").unwrap();
  app.wait_for("Appearance").unwrap();
  app.wait_for("UI Size").unwrap();
  app.wait_for("Pages").unwrap();
  app.press("id:lang-zh").unwrap();
  app.press("id:ui-font-standard").unwrap();
  app.wait_for("外观").unwrap();
  app.press("id:settings-close-btn").unwrap();

  // 4. Embedded whiteboard opens directly by clicking the board preview card.
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("id:slash-whiteboard").unwrap();
  app.press("id:slash-whiteboard").unwrap();
  app.wait_for("id:board-preview-open-btn").unwrap();
  app.press("id:board-preview-open-btn").unwrap();
  app.wait_for("id:embedded-board-modal").unwrap();
  app.press("id:board-done-btn").unwrap();
}


#[test]
fn regression_narrow_viewport_drawers_and_touch_bar() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(420.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Narrow viewport (<768px) renders compact tab switcher and hides desktop inline sidebar (CAP-16-05).
  app.wait_for("id:narrow-tab-switcher").unwrap();
  app.press("id:narrow-tab-switcher").unwrap();
  app.wait_for("id:narrow-tab-popover").unwrap();
  app.press("id:narrow-tab-switcher").unwrap();
  app
    .wait_for_gone("id:narrow-tab-popover", std::time::Duration::from_millis(600))
    .unwrap();

  // 2. Opening sidebar on narrow viewport mounts overlay drawer + dismissible backdrop (CAP-16-05).
  app.press("id:toggle-sidebar-btn").unwrap();
  app.wait_for("id:mobile-left-drawer").unwrap();
  app.press("id:mobile-left-drawer-backdrop").unwrap();
  app
    .wait_for_gone("id:mobile-left-drawer", std::time::Duration::from_millis(600))
    .unwrap();

  // 3. Selecting text on narrow viewport mounts compact touch selection action bar
  //    AND draggable start/end teardrop selection handles (CAP-16-06).
  app.press("id:editor-bottom-append-zone").unwrap();
  app.press_key("mod+a").unwrap();
  app.wait_for("id:touch-selection-bar").unwrap();
  app.wait_for("id:touch-sel-copy").unwrap();
  app.wait_for("id:touch-handle-start").unwrap();
  app.wait_for("id:touch-handle-end").unwrap();

  // 4. Pressing `touch-sel-copy` copies selection, clears it, and dismisses bar + handles.
  app.press("id:touch-sel-copy").unwrap();
  app
    .wait_for_gone("id:touch-selection-bar", std::time::Duration::from_millis(600))
    .unwrap();
  app
    .wait_for_gone("id:touch-handle-start", std::time::Duration::from_millis(600))
    .unwrap();
}


#[test]
fn regression_sidebar_context_menu_new_tab_unclipped_modals_transfer_and_exports() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // Folders default to collapsed on workspace open; expand root folder to interact with its child page.
  app.press("id:sidebar-row-0").unwrap();
  app.wait_for("id:sidebar-row-1").unwrap();

  // 1. Right-click on the currently active page -> "在新标签页打开"
  //    MUST open a fresh 2nd tab (`doc-tab-1`) instead of silently no-oping!
  app.right_click("id:sidebar-row-1").unwrap();
  app.wait_for("id:sidebar-context-menu").unwrap();
  app.wait_for("id:ctx-open-new-tab").unwrap();
  app.wait_for("id:ctx-export-page-md").unwrap();
  app.wait_for("id:ctx-export-page-html").unwrap();
  app.press("id:ctx-open-new-tab").unwrap();
  app.wait_for("id:doc-tab-1").unwrap();

  // 2. "设置图标" MUST open `emoji-picker-modal` centered across the full 1280px window
  //    (NOT clipped inside the 280px sidebar!).
  app.right_click("id:sidebar-row-1").unwrap();
  app.wait_for("id:ctx-set-icon").unwrap();
  app.press("id:ctx-set-icon").unwrap();
  app.wait_for("id:emoji-picker-modal").unwrap();
  let nodes = app.screen().layout_nodes();
  let emoji_box = nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("emoji-picker-modal"))
    .expect("emoji picker modal laid out")
    .bounds;
  assert!(
    emoji_box.origin.x.0 > 350.0 && emoji_box.size.width.0 >= 350.0,
    "emoji picker must be centered over full window, not clipped in 280px sidebar: {:?}",
    emoji_box
  );
  app.press("id:emoji-picker-dismiss").unwrap();
  app
    .wait_for_gone("id:emoji-picker-modal", std::time::Duration::from_millis(800))
    .unwrap();

  // 3. "移动到工作区" dialog MUST NOT be blank/collapsed (body renders visible copy with real height).
  app.right_click("id:sidebar-row-1").unwrap();
  app.wait_for("id:ctx-transfer-move").unwrap();
  app.press("id:ctx-transfer-move").unwrap();
  app.wait_for("id:transfer-dialog-panel").unwrap();
  app.wait_for("当前没有其他目标工作区，请先在左上角工作区切换器中新建一个工作区。").unwrap();
  let t_nodes = app.screen().layout_nodes();
  let t_panel = t_nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("transfer-dialog-panel"))
    .expect("transfer panel laid out")
    .bounds;
  assert!(
    t_panel.size.height.0 >= 150.0,
    "transfer dialog body must not collapse to 0px height: {:?}",
    t_panel
  );
  app.press("id:transfer-close-btn").unwrap();
  app
    .wait_for_gone("id:transfer-dialog-panel", std::time::Duration::from_millis(800))
    .unwrap();

  // 4. Folder backup (`ctx-export-backup`) & page Markdown export (`ctx-export-page-md`)
  //    MUST survive menu dismissal (`app_owner`) and complete instead of silently cancelling!
  app.right_click("id:sidebar-row-0").unwrap();
  app.wait_for("id:ctx-export-backup").unwrap();
  app.press("id:ctx-export-backup").unwrap();
  app.wait_for("已导出 ZIP").unwrap();

  app.right_click("id:sidebar-row-1").unwrap();
  app.wait_for("id:ctx-export-page-md").unwrap();
  app.press("id:ctx-export-page-md").unwrap();
  app.wait_for("已导出 Markdown").unwrap();
}


#[test]
fn regression_sidebar_folders_collapse_on_workspace_open_and_table_spacing_is_compact() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. On initial workspace open, sidebar folders start collapsed:
  //    top-level folder `sidebar-row-0` is visible while its child page `sidebar-row-1` is hidden.
  app.wait_for("id:sidebar-row-0").unwrap();
  assert!(
    !app.shows("id:sidebar-row-1"),
    "sidebar folders must default to collapsed when opening a workspace"
  );

  // Expanding the folder reveals its child page.
  app.press("id:sidebar-row-0").unwrap();
  app.wait_for("id:sidebar-row-1").unwrap();

  // Re-opening / switching into a workspace resets all folders back to collapsed.
  app.press("id:workspace-switcher-btn").unwrap();
  app.wait_for("id:workspace-item-0").unwrap();
  app.press("id:workspace-item-0").unwrap();
  app.wait_for("id:sidebar-row-0").unwrap();
  assert!(
    !app.shows("id:sidebar-row-1"),
    "re-opening a workspace must reset sidebar folders to collapsed"
  );

  // 2. Table vertical spacing matches standard prose spacing and right/bottom `+` bars are slim.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();

  let mut paste = nsh_core::PasteEvent::default();
  paste.text = "上文标准段落\n\n| 列A | 列B |\n| --- | --- |\n| 值1 | 值2 |\n\n下文标准段落".into();
  app.paste("id:mica-editor-ime-sink", &paste).unwrap();
  app.wait_for("id:node-view-table").unwrap();
  app.wait_for("上文标准段落").unwrap();
  app.wait_for("下文标准段落").unwrap();
  app.wait_for("id:table-cell-1-0-0").unwrap();
  app.wait_for("id:table-cell-1-1-0").unwrap();
  app.wait_for("id:table-add-col-bar").unwrap();
  app.wait_for("id:table-add-row-bar").unwrap();

  let nodes = app.screen().layout_nodes();
  let top_para = nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("block-row-0"))
    .expect("top prose block")
    .bounds;
  let first_cell = nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("table-cell-1-0-0"))
    .expect("table header cell")
    .bounds;
  let last_cell = nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("table-cell-1-1-0"))
    .expect("table last row cell")
    .bounds;
  let bottom_para = nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("block-row-2"))
    .expect("bottom prose block")
    .bounds;
  let right_bar = nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("table-add-col-bar"))
    .expect("right add-column bar")
    .bounds;
  let bottom_bar = nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("table-add-row-bar"))
    .expect("bottom add-row bar")
    .bounds;

  let top_gap = first_cell.origin.y.0 - (top_para.origin.y.0 + top_para.size.height.0);
  let bottom_gap = bottom_para.origin.y.0 - (last_cell.origin.y.0 + last_cell.size.height.0);

  assert!(
    (3.0..=10.5).contains(&top_gap),
    "table top gap ({top_gap}px) must match standard prose spacing (~7px)"
  );
  assert!(
    (4.0..=12.5).contains(&bottom_gap),
    "table bottom gap ({bottom_gap}px) must stay compact near standard prose spacing (~9px)"
  );
  assert!(
    right_bar.size.width.0 <= 11.0,
    "right add-col bar width ({}px) should be slim (~10px)",
    right_bar.size.width.0
  );
  assert!(
    bottom_bar.size.height.0 <= 11.0,
    "bottom add-row bar height ({}px) should be slim (~10px)",
    bottom_bar.size.height.0
  );
}


#[test]
fn regression_sidebar_multi_select_preserves_blue_bg_on_hover() {
  use nsh_core::{
    EventKind, EventPayload, PressEvent,
    geometry::point,
    keymap::Modifiers,
  };

  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("id:sidebar-row-0").unwrap();
  // Create 3 sibling pages under the workspace root so rows 1, 2, 3 share the same parent.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("id:sidebar-row-1").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("id:sidebar-row-2").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("id:sidebar-row-3").unwrap();

  let shift_click_row = |app: &mut Headless, test_id: &str| {
    let node = app
      .screen()
      .layout_nodes()
      .into_iter()
      .find(|n| n.test_id.as_deref() == Some(test_id))
      .unwrap_or_else(|| panic!("{test_id} must be laid out"));
    let center_local = point(px(node.bounds.size.width.0 * 0.5), px(node.bounds.size.height.0 * 0.5));
    let center_page = point(
      node.bounds.origin.x + center_local.x,
      node.bounds.origin.y + center_local.y,
    );
    app.engine().control().dispatch_event(
      node.id,
      EventKind::Press,
      EventPayload::Press(PressEvent {
        position: center_local,
        page_position: center_page,
        modifiers: Modifiers { shift: true, ..Modifiers::default() },
        clicks: 1,
      }),
    );
    app.settle();
  };

  // Shift+Click first sibling page, then Shift+Click last sibling page.
  shift_click_row(&mut app, "sidebar-row-1");
  shift_click_row(&mut app, "sidebar-row-3");

  let p = mica_rust::mica_theme::light_palette();
  let rgba_to_u32 = |c: nsh_core::Rgba| -> u32 {
    let r = (c.r.clamp(0.0, 1.0) * 255.0).round() as u32;
    let g = (c.g.clamp(0.0, 1.0) * 255.0).round() as u32;
    let b = (c.b.clamp(0.0, 1.0) * 255.0).round() as u32;
    let a = (c.a.clamp(0.0, 1.0) * 255.0).round() as u32;
    (r << 24) | (g << 16) | (b << 8) | a
  };
  let selection_u32 = rgba_to_u32(p.selection_bg);
  let surface_hover_u32 = rgba_to_u32(p.surface_hover);

  // All 3 rows in the range start with the blue selection background.
  for idx in 1..=3 {
    let bg = app
      .screen()
      .view_style(&format!("id:sidebar-row-body-{idx}"))
      .unwrap()
      .unwrap()
      .background_color
      .unwrap();
    assert_eq!(
      bg, selection_u32,
      "multi-selected sidebar row {idx} should render selection_bg before hover (got #{bg:08x})"
    );
  }

  // Hovering a selected row MUST keep the blue selection tint instead of reverting to grey surface_hover.
  app.hover("id:sidebar-row-3").unwrap();
  app.settle();
  let hovered_selected_bg = app
    .screen()
    .view_style("id:sidebar-row-body-3")
    .unwrap()
    .unwrap()
    .background_color
    .unwrap();
  assert_ne!(
    hovered_selected_bg, surface_hover_u32,
    "hovering a multi-selected sidebar row must not overwrite its blue selection background with grey surface_hover"
  );
  assert_eq!(
    hovered_selected_bg & 0xFFFF_FF00,
    selection_u32 & 0xFFFF_FF00,
    "hovered multi-selected sidebar row must preserve selection_bg blue RGB (got #{hovered_selected_bg:08x}, expected RGB #{:06x})",
    selection_u32 >> 8
  );
  assert!(
    (hovered_selected_bg & 0xFF) >= (selection_u32 & 0xFF),
    "hovered multi-selected sidebar row alpha should be at least selection_bg alpha"
  );

  // Hovering an unselected row still uses grey surface_hover.
  app.hover("id:sidebar-row-0").unwrap();
  app.settle();
  let hovered_unselected_bg = app
    .screen()
    .view_style("id:sidebar-row-body-0")
    .unwrap()
    .unwrap()
    .background_color
    .unwrap();
  assert_eq!(
    hovered_unselected_bg, surface_hover_u32,
    "hovering an unselected sidebar row must use surface_hover"
  );
}


#[test]
fn regression_sidebar_located_creation_placement() {
  use nsh_core::{EventKind, EventPayload, PressEvent, geometry::point, keymap::Modifiers};

  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("id:sidebar-row-0").unwrap();

  // `virtual_list` keys rows by `view_id`, so inserting in the middle reuses existing DOM
  // elements (whose static `test_id` was captured at mount). Sort all laid-out row buttons
  // (`sidebar-row-<digits>`) by visual `bounds.origin.y` to inspect true top-to-bottom order.
  let visual_rows = |app: &mut Headless| {
    let mut rows: Vec<_> = app
      .screen()
      .layout_nodes()
      .into_iter()
      .filter(|n| {
        n.test_id.as_deref().is_some_and(|tid| {
          tid
            .strip_prefix("sidebar-row-")
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
        })
      })
      .map(|n| {
        (
          n.id,
          n.bounds.origin.x.0,
          n.bounds.origin.y.0,
          n.bounds.size.width.0,
          n.bounds.size.height.0,
        )
      })
      .collect();
    rows.sort_by(|a, b| a.2.total_cmp(&b.2));
    rows
  };
  let wait_row_count = |app: &mut Headless, expected: usize| {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
      app.settle();
      if visual_rows(app).len() == expected {
        return;
      }
      std::thread::sleep(std::time::Duration::from_millis(15));
    }
    panic!(
      "expected {expected} visible sidebar rows, got {}",
      visual_rows(app).len()
    );
  };
  let row_x = |app: &mut Headless, idx: usize| -> f32 { visual_rows(app)[idx].1 };
  let click_visual_row = |app: &mut Headless, idx: usize| {
    let (id, x, y, w, h) = visual_rows(app)[idx];
    let local = point(px(w * 0.5), px(h * 0.5));
    let page = point(px(x + w * 0.5), px(y + h * 0.5));
    app.engine().control().dispatch_event(
      id,
      EventKind::Press,
      EventPayload::Press(PressEvent {
        position: local,
        page_position: page,
        modifiers: Modifiers::default(),
        clicks: 1,
      }),
    );
    app.settle();
  };
  let paste_body = |app: &mut Headless, marker: &str| {
    let mut paste = nsh_core::PasteEvent::default();
    paste.text = marker.into();
    app.paste("id:mica-editor-ime-sink", &paste).unwrap();
    app.wait_for(marker).unwrap();
  };

  let root_x = row_x(&mut app, 0);

  // 1. Initially `欢迎使用 Mica` (inside `知识库`) is open.
  //    Create Page A and Page B via top `+`: both land inside `知识库` (depth 1).
  app.press("id:new-page-btn").unwrap();
  wait_row_count(&mut app, 3);
  paste_body(&mut app, "MARKER-PAGE-A");

  app.press("id:new-page-btn").unwrap();
  wait_row_count(&mut app, 4);
  paste_body(&mut app, "MARKER-PAGE-B");

  // Layout inside `知识库` is now:
  //   row 0: 知识库 (depth 0)
  //   row 1: 欢迎使用 Mica (depth 1)
  //   row 2: Page A (depth 1)
  //   row 3: Page B (depth 1)
  assert!((row_x(&mut app, 1) - (root_x + 16.0)).abs() <= 1.5);
  assert!((row_x(&mut app, 2) - (root_x + 16.0)).abs() <= 1.5);
  assert!((row_x(&mut app, 3) - (root_x + 16.0)).abs() <= 1.5);

  // 2. Click visual row 1 (`欢迎使用 Mica`, the FIRST sibling) and create Page C:
  //    MUST land directly below `欢迎使用 Mica` at visual row 2 (NOT at the bottom row 4!).
  click_visual_row(&mut app, 1);
  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  wait_row_count(&mut app, 5);
  paste_body(&mut app, "MARKER-PAGE-C");

  // Verify exact visual order: row 2 == Page C, row 3 == Page A, row 4 == Page B.
  click_visual_row(&mut app, 3);
  app.wait_for("MARKER-PAGE-A").unwrap();
  click_visual_row(&mut app, 4);
  app.wait_for("MARKER-PAGE-B").unwrap();
  click_visual_row(&mut app, 2);
  app.wait_for("MARKER-PAGE-C").unwrap();

  // 3. While Page C (visual row 2, inside `知识库`) is located, click top `folder+`:
  //    New folder MUST be created inside `知识库` right below Page C (visual row 3, depth 1)
  //    and become the focused located node.
  app.press("id:new-folder-btn").unwrap();
  wait_row_count(&mut app, 6);
  assert!(
    (row_x(&mut app, 3) - (root_x + 16.0)).abs() <= 1.5,
    "new folder created while located on a depth-1 page must land at depth 1 right below it"
  );
  // Immediately click top `+` while the brand-new folder (visual row 3) is focused:
  //    New page MUST land INSIDE that new folder at visual row 4 (depth 2, +32px)!
  app.press("id:new-page-btn").unwrap();
  wait_row_count(&mut app, 7);
  paste_body(&mut app, "MARKER-NESTED-IN-SUBFOLDER");
  assert!(
    (row_x(&mut app, 4) - (root_x + 32.0)).abs() <= 1.5,
    "new page created while a depth-1 folder is focused must land inside it at depth 2"
  );
  // Verify visual row 5 is Page A (confirming subfolder + nested page were inserted between Page C and Page A).
  click_visual_row(&mut app, 5);
  app.wait_for("MARKER-PAGE-A").unwrap();

  // 4. Click blank area below tree (`sidebar-root-drop-zone`) -> focuses workspace root.
  //    Creating a folder and then clicking blank area again and creating a page must both
  //    land at depth 0 (workspace root) at the bottom of the tree.
  app.press("id:sidebar-root-drop-zone").unwrap();
  app.press("id:new-folder-btn").unwrap();
  wait_row_count(&mut app, 8);
  assert!(
    (row_x(&mut app, 7) - root_x).abs() <= 1.5,
    "new folder created after clicking empty sidebar area must land at workspace root (depth 0)"
  );

  app.press("id:sidebar-root-drop-zone").unwrap();
  app.press("id:new-page-btn").unwrap();
  wait_row_count(&mut app, 9);
  paste_body(&mut app, "MARKER-ROOT-PAGE");
  assert!(
    (row_x(&mut app, 8) - root_x).abs() <= 1.5,
    "new page created after clicking empty sidebar area must land at workspace root (depth 0)"
  );

  // 5. Folder row's own `+` button while `欢迎使用 Mica` (visual row 1, direct child of `知识库`)
  //    is located -> creates right below `欢迎使用 Mica` at visual row 2!
  click_visual_row(&mut app, 1);
  app.hover("id:sidebar-row-0").unwrap();
  app.wait_for("id:sidebar-row-new-page-0").unwrap();
  app.press("id:sidebar-row-new-page-0").unwrap();
  wait_row_count(&mut app, 10);
  paste_body(&mut app, "MARKER-FOLDER-BTN-AFTER-WELCOME");
  // Verify visual row 2 is the new page and visual row 3 is Page C.
  click_visual_row(&mut app, 3);
  app.wait_for("MARKER-PAGE-C").unwrap();
  click_visual_row(&mut app, 2);
  app.wait_for("MARKER-FOLDER-BTN-AFTER-WELCOME").unwrap();
}

/// Regression (v0.15.5):
/// 1. Multi-line selection over a bullet list pressing `Tab` indents every
///    selected item (including the first line, previously pinned at indent 0).
/// 2. Pasting inside a code block after Enter cleanly writes pasted code
///    without crashing or losing trailing content, and repeated Backspace
///    inside multi-line code blocks executes cleanly.

#[test]
fn regression_untitled_placeholder_folder_rename_and_batch_delete() {
  use nsh_core::{
    EventKind, EventPayload, PressEvent,
    geometry::point,
    keymap::Modifiers,
  };

  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  let click_row_with_mods = |app: &mut Headless, test_id: &str, mods: Modifiers| {
    let node = app
      .screen()
      .layout_nodes()
      .into_iter()
      .find(|n| n.test_id.as_deref() == Some(test_id))
      .unwrap_or_else(|| panic!("{test_id} must be laid out"));
    let center_local = point(px(node.bounds.size.width.0 * 0.5), px(node.bounds.size.height.0 * 0.5));
    let center_page = point(
      node.bounds.origin.x + center_local.x,
      node.bounds.origin.y + center_local.y,
    );
    app.engine().control().dispatch_event(
      node.id,
      EventKind::Press,
      EventPayload::Press(PressEvent {
        position: center_local,
        page_position: center_page,
        modifiers: mods,
        clicks: 1,
      }),
    );
    app.settle();
  };

  // Focus workspace root (`sidebar-root-drop-zone`), then create a root folder (`row 1`)
  // and a root sibling page (`row 2`).
  app.press("id:sidebar-root-drop-zone").unwrap();
  app.press("id:new-folder-btn").unwrap();
  app.wait_for("新文件夹").unwrap();
  app.wait_for("id:sidebar-row-1").unwrap();

  app.press("id:sidebar-root-drop-zone").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();
  app.wait_for("id:sidebar-row-2").unwrap();
  assert!(
    !app.screen().texts().iter().any(|t| t.starts_with("新页面 ")),
    "new page must not carry a numeric suffix: {:?}",
    app.screen().texts()
  );

  // Page title input starts empty (gray placeholder `"新页面"`); typing directly yields exact title.
  app.wait_for("新页面").unwrap();
  app.type_text("id:page-title-input", "直接输入标题").unwrap();
  app.wait_for("直接输入标题").unwrap();
  assert!(
    !app.shows("新页面直接输入标题"),
    "typing into a new page title input must not concatenate with 新页面"
  );

  // Rename `新文件夹` (`sidebar-row-1`) via hover menu -> `重命名` modal -> stays `架构归档`.
  app.hover("id:sidebar-row-1").unwrap();
  app.wait_for("id:sidebar-row-menu-1").unwrap();
  app.press("id:sidebar-row-menu-1").unwrap();
  app.wait_for("id:ctx-rename").unwrap();
  app.press("id:ctx-rename").unwrap();
  app.wait_for("id:rename-dialog-input").unwrap();
  app.type_text("id:rename-dialog-input", "").unwrap();
  for _ in 0..4 {
    app.press_key("backspace").unwrap();
  }
  app.type_text("id:rename-dialog-input", "架构归档").unwrap();
  app.press("id:rename-dialog-save").unwrap();
  app.wait_for("架构归档").unwrap();
  assert!(
    !app.shows("新文件夹"),
    "renamed folder must not show old default name 新文件夹"
  );

  // Shift-select `架构归档` (row 1) through `直接输入标题` (row 2), batch delete both,
  // then click `new-folder-btn`: neither deleted item may resurrect.
  click_row_with_mods(&mut app, "sidebar-row-1", Modifiers::default());
  click_row_with_mods(
    &mut app,
    "sidebar-row-2",
    Modifiers { shift: true, ..Modifiers::default() },
  );

  app.hover("id:sidebar-row-1").unwrap();
  app.wait_for("id:sidebar-row-menu-1").unwrap();
  app.press("id:sidebar-row-menu-1").unwrap();
  app.wait_for("id:ctx-batch-delete").unwrap();
  app.press("id:ctx-batch-delete").unwrap();
  app
    .wait_for_gone("架构归档", std::time::Duration::from_millis(400))
    .unwrap();
  app
    .wait_for_gone("id:sidebar-row-1", std::time::Duration::from_millis(400))
    .unwrap();

  app.press("id:new-folder-btn").unwrap();
  app.wait_for("新文件夹").unwrap();
  app.wait_for("id:sidebar-row-1").unwrap();
  assert!(
    !app.shows("架构归档"),
    "deleted folder must remain deleted after creating a new folder"
  );
  assert!(
    app
      .screen()
      .layout_nodes()
      .iter()
      .all(|n| n.test_id.as_deref() != Some("sidebar-row-2")),
    "deleted page and folder must remain deleted so only 知识库 (row 0) and 新文件夹 (row 1) exist"
  );
}


#[test]
fn regression_page_more_menu_dismisses_on_outside_click_anchor_and_escape() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Clicking outside inside the editor body closes the open `⋯` page menu.
  app.press("id:ed-more").unwrap();
  app.wait_for("id:page-more-menu").unwrap();
  app.press("id:editor-bottom-append-zone").unwrap();
  app
    .wait_for_gone("id:page-more-menu", std::time::Duration::from_millis(800))
    .unwrap();

  // 2. Clicking outside in the sidebar also closes the open `⋯` page menu.
  app.press("id:ed-more").unwrap();
  app.wait_for("id:page-more-menu").unwrap();
  app.press("id:sidebar-row-0").unwrap();
  app
    .wait_for_gone("id:page-more-menu", std::time::Duration::from_millis(800))
    .unwrap();

  // 3. Re-clicking the `ed-more` anchor button toggles the menu closed without
  //    immediately reopening from an `on_press_outside` + `on_press` race.
  app.press("id:ed-more").unwrap();
  app.wait_for("id:page-more-menu").unwrap();
  app.press("id:ed-more").unwrap();
  app
    .wait_for_gone("id:page-more-menu", std::time::Duration::from_millis(800))
    .unwrap();

  // 4. Escape dismisses the menu cleanly via overlay `on_cancel`.
  app.press("id:ed-more").unwrap();
  app.wait_for("id:page-more-menu").unwrap();
  app.press_key("escape").unwrap();
  app
    .wait_for_gone("id:page-more-menu", std::time::Duration::from_millis(800))
    .unwrap();
}


#[test]
fn regression_mobile_editor_toolbar_scrolls_format_bar_and_moves_overflow_to_more_menu() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(390.0), px(844.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.wait_for("id:ed-format-bar-scroll").unwrap();
  app.wait_for("id:ed-more").unwrap();

  let screen = app.screen();
  let nodes = screen.layout_nodes();
  for hidden_on_row1 in ["ed-open-board", "ed-copy-link", "ed-copy-path"] {
    assert!(
      nodes
        .iter()
        .all(|n| n.test_id.as_deref() != Some(hidden_on_row1)),
      "{hidden_on_row1} should move into `⋯` menu on < 640px mobile header"
    );
  }
  let more_btn = nodes
    .iter()
    .find(|n| n.test_id.as_deref() == Some("ed-more"))
    .expect("ed-more missing");
  assert!(
    more_btn.bounds.origin.x.0 + more_btn.bounds.size.width.0 <= 390.0,
    "ed-more overflowed 390px: {:?}",
    more_btn.bounds
  );

  app.press("id:ed-more").unwrap();
  app.wait_for("id:page-more-menu").unwrap();
  app.wait_for("id:page-menu-board").unwrap();
  app.wait_for("id:page-menu-copy-link").unwrap();
  app.wait_for("id:page-menu-copy-path").unwrap();
}


#[test]
fn regression_workspace_row_menu_floats_in_overlay_without_pushing_rows() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("Mica 本地工作区").unwrap();

  // Create a second workspace so row 1 is visible below row 0.
  app.press("id:workspace-switcher-btn").unwrap();
  app.wait_for("id:ws-create-trigger").unwrap();
  app.press("id:ws-create-trigger").unwrap();
  app.type_text("id:ws-create-input", "第二工作区").unwrap();
  app.press("id:ws-create-confirm").unwrap();
  app
    .wait_for_gone("id:workspace-switcher-popover", std::time::Duration::from_millis(600))
    .unwrap();

  app.press("id:workspace-switcher-btn").unwrap();
  app.wait_for("id:workspace-item-0").unwrap();
  app.wait_for("id:workspace-item-1").unwrap();

  let y_before = app
    .screen()
    .layout_nodes()
    .iter()
    .find(|n| n.test_id.as_deref() == Some("workspace-item-1"))
    .expect("workspace-item-1 missing")
    .bounds
    .origin
    .y
    .0;

  app.press("id:ws-row-menu-0").unwrap();
  app.wait_for("id:ws-row-action-menu").unwrap();

  let y_after = app
    .screen()
    .layout_nodes()
    .iter()
    .find(|n| n.test_id.as_deref() == Some("workspace-item-1"))
    .expect("workspace-item-1 missing after opening menu")
    .bounds
    .origin
    .y
    .0;

  assert!(
    (y_after - y_before).abs() < 1.0,
    "ws-row-action-menu shifted sibling workspace row instead of floating: before={y_before}, after={y_after}"
  );
}


#[test]
fn regression_ctrl_wheel_and_shortcuts_zoom_within_bounds() {
  use nsh_core::geometry::point;

  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // Ctrl + `=` zooms to 105%
  app.press_key("mod+=").unwrap();
  app.wait_for("id:zoom-level-badge").unwrap();
  app.wait_for("105%").unwrap();

  // Ctrl + Wheel up over editor zooms further to 110%
  app.hold_keys("ctrl", true).unwrap();
  app
    .wheel("id:mica-rich-editor", point(px(0.0), px(-24.0)))
    .unwrap();
  app.hold_keys("ctrl", false).unwrap();
  app.wait_for("110%").unwrap();

  // Ctrl + `-` steps back down to 105%, and Ctrl + `0` resets to 100%
  app.press_key("mod+-").unwrap();
  app.wait_for("105%").unwrap();
  app.press_key("mod+0").unwrap();
  app.wait_for("100%").unwrap();
}


#[test]
fn regression_escape_closes_settings_and_layered_about_modal() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Open Settings from bottom-left gear and verify Esc closes it even though editor_view is mounted.
  app.press("id:open-settings-btn").unwrap();
  app.wait_for("id:settings-panel").unwrap();

  app.press_key("escape").unwrap();
  app
    .wait_for_gone("id:settings-panel", std::time::Duration::from_millis(800))
    .unwrap();

  // 2. Re-open Settings, focus a text input inside Data tab, and verify Esc still closes Settings.
  app.press("id:open-settings-btn").unwrap();
  app.wait_for("id:settings-panel").unwrap();
  app.press("id:settings-tab-data").unwrap();
  app.wait_for("id:settings-import-path-input").unwrap();
  app.type_text("id:settings-import-path-input", "test").unwrap();

  app.press_key("escape").unwrap();
  app
    .wait_for_gone("id:settings-panel", std::time::Duration::from_millis(800))
    .unwrap();

  // 3. Open About from Settings footer and verify Esc closes About modal too.
  app.press("id:open-settings-btn").unwrap();
  app.wait_for("id:settings-about-footer").unwrap();
  app.press("id:settings-about-footer").unwrap();
  app.wait_for("id:about-modal").unwrap();

  app.press_key("escape").unwrap();
  app
    .wait_for_gone("id:about-modal", std::time::Duration::from_millis(800))
    .unwrap();
}


mod sidebar_update_tests {
  use std::{cell::RefCell, rc::Rc};

  use mica_client::PageTreeModel;
  use nsh_base::button;
  use nsh_core::prelude::*;

  #[derive(Default)]
  pub(super) struct SidebarUpdateHarness;

  impl Component for SidebarUpdateHarness {
    type Props = ();
    type Event = NoEvent;

    fn view(_props: Self::Props, _scope: Scope<Self::Event>) -> impl IntoElement {
      let available_update: Signal<Option<mica_rust::updater::UpdateInfo>> = signal(None);
      let update_busy = signal(false);
      let update_status = signal(String::new());
      let click_count = signal(0usize);

      mica_rust::tooltip::install_tooltip_host(signal(1280.0));
      provide_context(mica_rust::sidebar::SidebarUpdateState {
        available_update,
        update_busy,
        update_status,
        on_click_update: Rc::new(move || {
          click_count.set(click_count.get_untracked() + 1);
        }),
      });

      let tree = Rc::new(RefCell::new(PageTreeModel::default()));
      let cb = Rc::new(mica_rust::sidebar::SidebarCallbacks::default());

      view()
        .w_full()
        .h_full()
        .flex_row()
        .child(mica_rust::sidebar::sidebar_view(
          tree,
          signal(0),
          signal(None),
          signal(None),
          signal(false),
          signal(Vec::new()),
          signal(280.0),
          signal("本地工作区".to_string()),
          signal("local".to_string()),
          signal(false),
          cb,
        ))
        .child(
          view()
            .p_4()
            .gap_2()
            .child(
              button()
                .test_id("test-publish-update")
                .on_press(move || {
                  update_status.set("发现新版本 v0.99.0".to_string());
                  available_update.set(Some(mica_rust::updater::UpdateInfo {
                    version: "0.99.0".to_string(),
                    download_url: "https://example.com/Mica-Setup-0.99.0.exe".to_string(),
                    asset_name: "Mica-Setup-0.99.0.exe".to_string(),
                    notes: None,
                    size: Some(1024),
                    sha256: Some("a".repeat(64)),
                    sha256_file_url: None,
                  }));
                })
                .child(text("publish")),
            )
            .child(
              button()
                .test_id("test-clear-update")
                .on_press(move || {
                  available_update.set(None);
                  update_busy.set(false);
                })
                .child(text("clear")),
            )
            .child(text(move || format!("update-clicks-{}", click_count.get()))),
        )
    }
  }
}

#[test]
fn regression_sidebar_update_button_shows_when_update_available_and_triggers_action() {
  let mut config = EngineConfig::new(Root::of::<sidebar_update_tests::SidebarUpdateHarness>());
  config.viewport = size(px(900.0), px(720.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("update-clicks-0").unwrap();
  assert!(
    !app.shows("id:sidebar-update-btn"),
    "bottom-left update button must stay hidden when no update is available"
  );

  app.press("id:test-publish-update").unwrap();
  app.wait_for("id:sidebar-update-btn").unwrap();
  app.press("id:sidebar-update-btn").unwrap();
  app.wait_for("update-clicks-1").unwrap();

  app.press("id:test-clear-update").unwrap();
  app
    .wait_for_gone("id:sidebar-update-btn", std::time::Duration::from_millis(800))
    .unwrap();
}


#[test]
fn regression_new_page_focuses_title_and_new_folder_starts_inline_rename() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Creating a new page focuses `page-title-input` directly so typing immediately sets the title.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("id:page-title-input").unwrap();
  app.press_keys(&["n", "e", "w"]).unwrap();
  app.wait_for("new").unwrap();
  assert!(
    app
      .screen()
      .layout_nodes()
      .iter()
      .any(|n| {
        n.text.as_ref().is_some_and(|t| t.text.text == "new")
          && n.bounds.origin.x < px(280.0)
      }),
    "typing after creating a new page must update the page title (reflected in sidebar), not body"
  );

  // 2. Creating a new folder enters inline rename state with `sidebar-inline-rename-input` focused.
  app.press("id:new-folder-btn").unwrap();
  app.wait_for("id:sidebar-inline-rename-input").unwrap();
  app.press_keys(&["d", "o", "c", "s", "enter"]).unwrap();
  app
    .wait_for_gone(
      "id:sidebar-inline-rename-input",
      std::time::Duration::from_millis(800),
    )
    .unwrap();
  app.wait_for("docs").unwrap();

  // 3. Creating another new folder, typing text, and pressing Escape cancels inline rename cleanly
  //    without panicking or committing typed draft on blur.
  app.press("id:new-folder-btn").unwrap();
  app.wait_for("id:sidebar-inline-rename-input").unwrap();
  app.press_keys(&["t", "m", "p", "escape"]).unwrap();
  app
    .wait_for_gone(
      "id:sidebar-inline-rename-input",
      std::time::Duration::from_millis(800),
    )
    .unwrap();
  app.wait_for("新文件夹").unwrap();
  assert!(
    !app.shows("tmp"),
    "cancelling inline folder rename with Escape must not commit typed draft on unmount blur"
  );

  // 4. Creating a folder and clicking outside (both editor append zone and blank editor margin
  //    outside the sidebar) releases inline rename mode and commits the typed name cleanly.
  app.press("id:new-folder-btn").unwrap();
  app.wait_for("id:sidebar-inline-rename-input").unwrap();
  app.press_keys(&["b", "l", "u", "r", "-", "o", "k"]).unwrap();
  app.press("id:editor-bottom-append-zone").unwrap();
  app
    .wait_for_gone(
      "id:sidebar-inline-rename-input",
      std::time::Duration::from_millis(800),
    )
    .unwrap();
  app.wait_for("blur-ok").unwrap();

  // 5. Clicking the white margin of the editor (outside the sidebar and not focusing any input)
  //    must also release the folder inline rename state cleanly.
  app.press("id:new-folder-btn").unwrap();
  app.wait_for("id:sidebar-inline-rename-input").unwrap();
  app.press_keys(&["m", "a", "r", "g", "i", "n"]).unwrap();
  let editor_node = app
    .screen()
    .layout_nodes()
    .iter()
    .find(|n| n.test_id.as_deref() == Some("mica-rich-editor"))
    .cloned()
    .expect("mica-rich-editor missing");
  let local_pt = nsh_core::geometry::point(px(24.0), px(200.0));
  let page_pt = nsh_core::geometry::point(
    editor_node.bounds.origin.x + local_pt.x,
    editor_node.bounds.origin.y + local_pt.y,
  );
  app.engine().control().dispatch_event(
    editor_node.id,
    nsh_core::EventKind::PressDown,
    nsh_core::EventPayload::Press(nsh_core::PressEvent {
      position: local_pt,
      page_position: page_pt,
      modifiers: Default::default(),
      clicks: 1,
    }),
  );
  app.settle();
  app
    .wait_for_gone(
      "id:sidebar-inline-rename-input",
      std::time::Duration::from_millis(800),
    )
    .unwrap();
  app.wait_for("margin").unwrap();
}


