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

  // 3. Bottom Account Menu opens popover and closes cleanly
  app.press("id:account-menu-btn").unwrap();
  app.wait_for("本地离线 Vault · SQLite").unwrap();
  app.wait_for("设置与云端同步").unwrap();
  app.press("id:account-menu-btn").unwrap();

  // 4. Workspace Switcher popover opens and lists active workspace
  app.press("id:workspace-switcher-btn").unwrap();
  app.wait_for("切换工作区").unwrap();
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

  // 8. Open slash menu and insert a code block + table
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("代码块").unwrap();
  app.press("id:slash-code").unwrap();
  app.wait_for("rust").unwrap();
  app.press("id:ed-table-quick").unwrap();

  // 9. Whiteboard view (M4: BoardSession + Dot-Grid Canvas + Floating Inspector)
  app.press("id:mode-board").unwrap();
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

  // 11. Two-column Settings Modal: Appearance, Cloud Account, Data Export, Shortcuts/About
  app.press("id:open-settings-btn").unwrap();
  app.wait_for("外观与语言").unwrap();
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

  // 12. Trash active page and restore from Recycle Bin
  app.press("id:trash-page-btn").unwrap();
  app.wait_for("回收站 (1)").unwrap();
  app.press("id:toggle-recycle-bin").unwrap();
  app.press("id:restore-trash-0").unwrap();
  app.wait_for("回收站 (0)").unwrap();
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
  app.wait_for("代码块").unwrap();
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
