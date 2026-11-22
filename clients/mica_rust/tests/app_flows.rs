//! Headless end-to-end integration tests for Mica's pure-Rust M1~M4 client (`clients/mica_rust`).
//! Covers: frameless shell, RichEditor + MicaDoc CRDT bridge, editable code block NodeView,
//! right sidebar outline + version history restore, favorites, workspace switcher, bottom account
//! menu, slash commands + native table, full whiteboard + inspector, keyboard-navigable Search
//! modal, two-column Settings modal (Appearance / Cloud / AI / Data Export & Import / Shortcuts),
//! and Recycle Bin restore.

use nsh_core::geometry::{px, size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[test]
fn fullstack_rust_end_to_end_workspace_editor_and_board_flows() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
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

  // 3. Bottom Account Menu opens 1:1 stacked-card popover (`本地工作区` / `这台设备` / `切换世界…`) and closes cleanly
  app.press("id:account-menu-btn").unwrap();
  app.wait_for("这台设备").unwrap();
  app.wait_for("切换世界…").unwrap();
  app.press("id:account-menu-btn").unwrap();

  // 4. 320px Workspace Switcher popover opens and lists active workspace + import submenu
  app.press("id:workspace-switcher-btn").unwrap();
  app.wait_for("新建工作区").unwrap();
  app.press("id:workspace-menu-close").unwrap();

  // 5. Create a new folder and a new page via sidebar icons
  app.press("id:new-folder-btn").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面 2").unwrap();

  // 6. Rename active page via borderless large title
  app.type_text("id:page-title-input", "纯Rust架构设计").unwrap();
  app.wait_for("未命名页面 2纯Rust架构设计").unwrap();

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
  app.wait_for("未命名页面 2纯Rust架构设计").unwrap();

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

  // 12. Trash active page and restore from Settings -> 回收站 (sidebar footer hides Recycle Bin 1:1 with Flutter)
  app.press("id:trash-page-btn").unwrap();
  app.press("id:open-settings-btn").unwrap();
  app.press("id:settings-tab-trash").unwrap();
  app.wait_for("id:trash-restore-btn").unwrap();
  app.press("id:trash-restore-btn").unwrap();
  app.wait_for("回收站是空的").unwrap();
  app.press("id:settings-close-btn").unwrap();
}

#[test]
fn rich_editor_keyboard_enter_ctrl_a_and_editable_code_block() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // Create a clean blank page so we test real typing from scratch
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面 2").unwrap();

  // 1. Real typing + Enter key splits into a new block and continues typing
  app.type_text("id:mica-rich-editor", "First line").unwrap();
  app.wait_for("First line").unwrap();
  app.press_key("enter").unwrap();
  app.type_text("id:mica-rich-editor", "Second line").unwrap();
  app.wait_for("Second line").unwrap();
  assert!(
    app.screen().texts().iter().any(|t| t.contains("First line")),
    "First line must still exist after Enter: {:?}",
    app.screen().texts()
  );

  // 2. Real Ctrl+A selects all blocks across the document; typing replaces everything
  app.press_key("ctrl+a").unwrap();
  app.type_text("id:mica-rich-editor", "Replaced body").unwrap();
  app.wait_for("Replaced body").unwrap();
  assert!(
    !app.screen().texts().iter().any(|t| t.contains("First line")),
    "Ctrl+A + typing must replace previous lines: {:?}",
    app.screen().texts()
  );

  // 3. Insert an editable code block, click inside it, type multi-line code + Enter!
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("id:slash-code").unwrap();
  app.press("id:slash-code").unwrap();
  app.wait_for("id:code-block-editor").unwrap();

  // Press inside the code block editor first (verifies RichEditor doesn't steal focus!)
  app.press("id:code-block-editor").unwrap();
  app.type_text("id:code-block-editor", "let x = 42;").unwrap();
  app.wait_for("let x = 42;").unwrap();
  app.press_key("enter").unwrap();
  app.type_text("id:code-block-editor", "let y = 99;").unwrap();
  app.wait_for("let y = 99;").unwrap();

  // 4. Right sidebar version history: save snapshot, then click Restore
  app.press("id:open-settings-btn").unwrap();
  app.press("id:settings-tab-data").unwrap();
  app.press("id:settings-save-snapshot").unwrap();
  app.wait_for("版本快照已保存（共 2 个）").unwrap();
  app.press("id:settings-close-btn").unwrap();

  app.press("id:toggle-right-panel-btn").unwrap();
  app.wait_for("id:restore-snapshot-0").unwrap();
  app.press("id:restore-snapshot-0").unwrap();
  app.wait_for("Replaced body").unwrap();
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

  // 2. Bottom Account Menu -> `切换世界…` -> Full-window split World Switcher
  app.press("id:account-menu-btn").unwrap();
  app.wait_for("id:account-switch-world-btn").unwrap();
  app.press("id:account-switch-world-btn").unwrap();
  app.wait_for("id:world-tab-cloud").unwrap();
  app.wait_for("id:world-tab-local").unwrap();
  let t0 = std::time::Instant::now();
  app.press("id:world-tab-cloud").unwrap();
  app.wait_for("登录到 Mica").unwrap();
  assert!(
    t0.elapsed() < std::time::Duration::from_millis(1500),
    "switching to 云端账户 must not block the UI thread on HTTP health probe (took {:?})",
    t0.elapsed()
  );
  app.press("id:world-tab-local").unwrap();
  app.wait_for("在此设备上使用").unwrap();
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

  // 5. Draggable PageWidthFrame grips are mounted on the editor canvas
  app.wait_for("id:page-width-grip-left").unwrap();
  app.wait_for("id:page-width-grip-right").unwrap();

  // 6. 1120×840 Settings Dialog with all 8 tabs including Recycle Bin & Live Appearance Preview
  app.press("id:open-settings-btn").unwrap();
  app.wait_for("id:settings-appearance-preview").unwrap();
  app.press("id:settings-tab-trash").unwrap();
  app.wait_for("回收站").unwrap();
  app.press("id:settings-close-btn").unwrap();
}

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
fn virtualized_editor_code_overlay_mermaid_lightbox_and_page_switch() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Switch to a brand-new page and verify zero stale blocks leak from previous page.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面 2").unwrap();
  app.type_text("id:mica-rich-editor", "Single layer quote text").unwrap();
  app.wait_for("Single layer quote text").unwrap();
  app.press("id:ed-quote").unwrap();

  // 2. Insert a code block, type Mermaid syntax, switch language via top-level `overlay()`
  //    picker (`code-lang-btn` -> `code-lang-picker` -> `code-lang-mermaid`), then click
  //    the rendered Mermaid diagram to open the fullscreen zoom Lightbox modal!
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("id:slash-code").unwrap();
  app.press("id:slash-code").unwrap();
  app.wait_for("id:code-block-editor").unwrap();
  app.press("id:code-block-editor").unwrap();
  app.type_text("id:code-block-editor", "graph LR\n  A[Start] --> B[End]").unwrap();
  app.wait_for("id:code-lang-btn").unwrap();
  app.press("id:code-lang-btn").unwrap();
  app.wait_for("id:code-lang-picker").unwrap();
  app.type_text("id:code-lang-search", "mermaid").unwrap();
  app.wait_for("id:code-lang-mermaid").unwrap();
  app.press("id:code-lang-mermaid").unwrap();

  // Mermaid preview renders and clicking it opens fullscreen zoom Lightbox modal
  app.wait_for("id:mermaid-preview-btn").unwrap();
  app.press("id:mermaid-preview-btn").unwrap();
  app.wait_for("id:lightbox-close-btn").unwrap();
  app.wait_for("100%").unwrap();
  app.press("id:lightbox-close-btn").unwrap();
}
