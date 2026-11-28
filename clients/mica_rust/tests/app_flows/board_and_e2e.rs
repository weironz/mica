use nsh_core::geometry::{px, size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[test]
fn fullstack_rust_end_to_end_workspace_editor_and_board_flows() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  // 1. Initial warm-gray workspace shell & starter document (including Rust code block!) render cleanly
  app.wait_for("Mica 本地工作区").unwrap();
  app.wait_for("欢迎使用 Mica").unwrap();

  // 2. Right Outline Sidebar toggles open and shows headings + version history list + backlinks
  app.press("id:toggle-right-panel-btn").unwrap();
  app.wait_for("页面大纲").unwrap();
  app.wait_for("核心能力").unwrap();
  app.wait_for("初始版本").unwrap();
  app.wait_for("反向链接").unwrap();
  app.press("id:right-sidebar-close").unwrap();

  // 3. Bottom Account Menu opens stacked-card popover (with Desktop bounded quick-switch + `管理服务器…`) and closes cleanly
  app.press("id:account-menu-btn").unwrap();
  app.wait_for("这台设备").unwrap();
  app.wait_for("id:account-quick-switch-card").unwrap();
  app.wait_for("id:account-quick-switch-scroll").unwrap();
  app.wait_for("id:account-quick-local-btn").unwrap();
  app.wait_for("管理服务器…").unwrap();
  app.press("id:account-menu-btn").unwrap();

  // 4. 320px Workspace Switcher popover opens with collapsed import toggle; expanding reveals compact format tags
  app.press("id:workspace-switcher-btn").unwrap();
  app.wait_for("新建工作区").unwrap();
  app.wait_for("导入工作区").unwrap();
  assert!(
    !app.shows("id:ws-import-zip"),
    "workspace import options should be collapsed by default"
  );
  app.press("id:ws-import-toggle").unwrap();
  app.wait_for("从 ZIP 导入").unwrap();
  app.wait_for("Mica / Markdown").unwrap();
  app.wait_for("从 Notion 导入").unwrap();
  app.wait_for("Markdown & CSV").unwrap();
  assert!(
    !app.screen().texts().iter().any(|t| t.contains("从 ZIP 导入(") || t.contains("从 Notion 导入(")),
    "workspace import items must not use cramped parenthesized labels: {:?}",
    app.screen().texts()
  );
  app.press("id:workspace-menu-close").unwrap();

  // 5. Create a new folder and a new page via sidebar icons (names must NOT carry numeric suffixes)
  app.press("id:new-folder-btn").unwrap();
  app.wait_for("新文件夹").unwrap();
  assert!(
    !app.screen().texts().iter().any(|t| t.starts_with("文件夹 ") || t.starts_with("新文件夹 ")),
    "new folder name must not include a numeric suffix: {:?}",
    app.screen().texts()
  );
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();
  assert!(
    !app.screen().texts().iter().any(|t| t.starts_with("新页面 ")),
    "new page name must not include a numeric suffix: {:?}",
    app.screen().texts()
  );

  // 6. Rename active page via borderless large title (new page starts with empty input + gray placeholder)
  app.type_text("id:page-title-input", "纯Rust架构设计").unwrap();
  app.wait_for("纯Rust架构设计").unwrap();
  assert!(
    !app.shows("新页面纯Rust架构设计"),
    "typing into a new page title must not prepend literal 新页面"
  );

  // 7. Toolbar formatting, Favorites pinned section, Slash menu (Code Block & Table & Divider) + Undo
  app.press("id:ed-star").unwrap();
  app.wait_for("收藏").unwrap();
  app.press("id:ed-h1").unwrap();
  app.press("id:ed-divider").unwrap();
  app.press("id:ed-undo").unwrap();

  // 8. Open caret-anchored 22-item slash menu and insert an original borderless code block + table
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("id:slash-code").unwrap();
  app.press("id:slash-code").unwrap();
  app.wait_for("id:code-block-editor").unwrap();
  app.press("id:ed-table-quick").unwrap();

  // 9. Whiteboard view (Feishu 1:1 50px topbar, 10-tool vertical dock, contextual inspector)
  app.press("id:ed-open-board").unwrap();
  app.wait_for("Auto Layout").unwrap();
  app.press("id:board-add-sticky").unwrap();
  app.press("id:board-add-node").unwrap();
  app.press("id:board-add-child").unwrap();
  app.press("id:board-add-sibling").unwrap();
  app.press("id:board-layout").unwrap();
  app.press("id:board-edit-label").unwrap();
  app.type_text("id:board-node-text-input", "核心节点").unwrap();
  app.press("id:board-edit-done").unwrap();
  app.press("id:board-export-svg").unwrap();
  app.press("id:board-undo").unwrap();

  // 10. Floating Search Modal opens over workspace, finds hit, and opens document
  app.press("id:open-search-btn").unwrap();
  app.wait_for("输入关键词实时检索当前工作区全部文档").unwrap();
  app.type_text("id:search-query-input", "纯Rust").unwrap();
  app.wait_for("找到 1 条与 \"纯Rust\" 相关的结果").unwrap();
  app.press("id:search-hit-0").unwrap();
  app.wait_for("纯Rust架构设计").unwrap();

  // 11. 1120×840 Two-column Settings Modal: Appearance, Cloud Account, Data Export, Shortcuts
  app.press("id:open-settings-btn").unwrap();
  app.wait_for("外观").unwrap();
  app.press("id:settings-toggle-theme").unwrap();
  app.wait_for("当前：深色模式").unwrap();
  app.press("id:settings-toggle-locale").unwrap();
  app.wait_for("Language: English").unwrap();
  app.press("id:settings-tab-account").unwrap();
  app.wait_for("登录 Mica 云端账号").unwrap();
  app.press("id:settings-tab-data").unwrap();
  app.press("id:settings-export-md").unwrap();
  app.press("id:settings-export-html").unwrap();
  app.press("id:settings-save-snapshot").unwrap();
  app.wait_for("版本快照已保存（共 2 个）").unwrap();
  app.press("id:settings-tab-about").unwrap();
  app.wait_for("常用快捷键").unwrap();
  app.press("id:settings-close-btn").unwrap();

  // 12. Trash active page via `⋯` page menu (header trash icon intentionally removed) and restore from Settings -> 回收站
  app
    .wait_for_gone("id:trash-page-btn", std::time::Duration::from_millis(200))
    .unwrap();
  app.press("id:ed-more").unwrap();
  app.wait_for("id:page-menu-trash").unwrap();
  app.press("id:page-menu-trash").unwrap();
  app.press("id:open-settings-btn").unwrap();
  app.press("id:settings-tab-trash").unwrap();
  app.wait_for("id:trash-restore-btn").unwrap();
  app.press("id:trash-restore-btn").unwrap();
  app.wait_for("回收站是空的").unwrap();
  app.press("id:settings-close-btn").unwrap();
}


#[test]
fn flutter_parity_app_menu_world_switcher_graph_version_diff_and_width_grips() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Top-left M-logo Application Menu (`文件` / `视图` / `历史` / `帮助`)
  app.press("id:app-menu-btn").unwrap();
  app.wait_for("文件").unwrap();
  app.wait_for("视图").unwrap();
  app.wait_for("帮助").unwrap();
  app.press_key("escape").unwrap();

  // 2. Bottom Account Menu -> `管理服务器…` -> Centered 440px Environment Switcher Modal
  app.press("id:account-menu-btn").unwrap();
  app.wait_for("id:account-switch-world-btn").unwrap();
  app.press("id:account-switch-world-btn").unwrap();
  app.wait_for("id:world-switcher-modal").unwrap();
  app.wait_for("id:world-env-list").unwrap();
  app.wait_for("切换服务器与本地库").unwrap();
  app.wait_for("id:world-enter-local-btn").unwrap();
  app.wait_for("id:world-server-enter-0").unwrap();
  let t0 = std::time::Instant::now();
  app.press("id:world-server-enter-0").unwrap();
  app.wait_for("id:world-auth-form").unwrap();
  app.wait_for("id:world-server-card").unwrap();
  app.wait_for("id:world-auth-mode-register").unwrap();
  assert!(
    t0.elapsed() < std::time::Duration::from_millis(1500),
    "entering server auth view must not block the UI thread on HTTP health probe (took {:?})",
    t0.elapsed()
  );
  // Switch between Login and Register via bottom text links (no stacked segmented tabs)
  app.press("id:world-auth-mode-register").unwrap();
  app.wait_for("id:world-auth-display-name").unwrap();
  app.press("id:world-auth-mode-login").unwrap();
  // Return to flat environment list and re-enter local vault
  app.press("id:world-auth-back-btn").unwrap();
  app.wait_for("id:world-env-list").unwrap();
  app.press("id:world-enter-local-btn").unwrap();

  // 3. Sidebar top row -> Knowledge Graph (900×640) & Workspace Settings (440px)
  app.press("id:open-graph-btn").unwrap();
  app.wait_for("关系图").unwrap();
  app.press_key("escape").unwrap();

  app.press("id:open-ws-settings-btn").unwrap();
  app.wait_for("工作区设置").unwrap();
  app.press_key("escape").unwrap();

  // 4. Right sidebar -> 980×640 Version History Diff modal
  app.press("id:toggle-right-panel-btn").unwrap();
  app.wait_for("id:open-version-history-btn").unwrap();
  app.press("id:open-version-history-btn").unwrap();
  app.wait_for("版本历史").unwrap();
  app.press_key("escape").unwrap();
  app.press("id:right-sidebar-close").unwrap();

  // 5. Editor body drag-to-resize width grips have been removed (page width is configured in Settings).
  assert!(
    app.screen().layout_nodes().iter().all(|n| {
      n.test_id.as_deref() != Some("page-width-grip-left")
        && n.test_id.as_deref() != Some("page-width-grip-right")
    }),
    "page-width drag grips must not be mounted on the editor canvas"
  );

  // 6. 1120×840 Settings Dialog with all 8 tabs including Recycle Bin & Live Appearance Preview
  app.press("id:open-settings-btn").unwrap();
  app.wait_for("id:settings-appearance-preview").unwrap();
  app.press("id:settings-tab-trash").unwrap();
  app.wait_for("回收站").unwrap();
  app.press("id:settings-close-btn").unwrap();
}


#[test]
fn regression_capability_gaps_closed() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  // 1. Starter page's unfocused Rust code block renders syntax-highlighted preview cleanly (CAP-06-10).
  app.wait_for("欢迎使用 Mica").unwrap();
  app.wait_for("id:node-view-code-block").unwrap();
  app.wait_for("fn main() {\n    println!(\"Hello from Mica Pure Rust!\");\n}").unwrap();

  // 2. Focus code block -> `⋯` menu -> `编辑标题` -> inline bottom title input -> persists `code-block-caption` (CAP-06-06).
  app.press("id:code-block-editor").unwrap();
  app.wait_for("id:code-more-btn").unwrap();
  app.press("id:code-more-btn").unwrap();
  app.wait_for("id:code-menu-title").unwrap();
  app.press("id:code-menu-title").unwrap();
  app.wait_for("id:code-title-input").unwrap();
  app.type_text("id:code-title-input", "main.rs 示例").unwrap();
  app.press("保存").unwrap();
  app.wait_for("id:code-block-caption").unwrap();
  app.wait_for("main.rs 示例").unwrap();

  // 3. Table cell focus, Tab navigation to next cell, and Escape back to document (CAP-05-06, CAP-05-07).
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();
  let mut tbl_paste = nsh_core::PasteEvent::default();
  tbl_paste.text = "| 字段 | 数值 |\n| --- | --- |\n| GPU | 8x B300 |\n| IB | 800G |".into();
  app.paste("id:mica-editor-ime-sink", &tbl_paste).unwrap();
  app.wait_for("id:node-view-table").unwrap();
  app.wait_for("8x B300").unwrap();
  app.press("id:table-cell-0-0-0").unwrap();
  app.wait_for("id:table-cell-input-0-0-0").unwrap();
  app.press_key("tab").unwrap();
  app.wait_for("id:table-cell-input-0-0-1").unwrap();
  app.press_key("escape").unwrap();
  app
    .wait_for_gone("id:table-cell-input-0-0-1", std::time::Duration::from_millis(600))
    .unwrap();
}


#[test]
fn regression_board_inspector_never_collapses_empty_after_edit_and_drag() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:ed-open-board").unwrap();
  app.wait_for("Auto Layout").unwrap();

  // Add a mind_root node via dock button (selects it immediately)
  app.press("id:board-add-node").unwrap();
  app.wait_for("id:board-inspector-card").unwrap();
  let card = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("board-inspector-card"))
    .expect("node inspector card must be mounted")
    .bounds;
  assert!(
    card.size.width.0 >= 400.0 && card.size.height.0 >= 32.0,
    "node inspector card must never collapse empty (got {:?})",
    card.size
  );

  // Enter & exit inline edit: inspector must hide during edit and return intact afterwards
  app.press("id:board-edit-label").unwrap();
  app.wait_for("id:board-edit-done").unwrap();
  app.press("id:board-edit-done").unwrap();
  app.wait_for("id:board-node-dashed").unwrap();

  // Drag on canvas and verify inspector remains valid (or cleanly absent if deselected)
  app.drag("id:board-canvas", nsh_core::geometry::point(px(120.0), px(60.0))).unwrap();
  if let Some(after) = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("board-inspector-card"))
  {
    assert!(
      after.bounds.size.width.0 >= 300.0 && after.bounds.size.height.0 >= 32.0,
      "contextual inspector card must never render as an empty collapsed pill (got {:?})",
      after.bounds.size
    );
  }
}


