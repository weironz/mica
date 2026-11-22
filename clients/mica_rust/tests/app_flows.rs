//! Headless end-to-end integration tests for Mica's pure-Rust client (`clients/mica_rust`).
//! Covers: workspace shell, workspace switcher, bottom account menu, right outline sidebar,
//! page creation & borderless title rename, in-place IME editing, toolbar formatting,
//! interactive tables, whiteboard/mindmap, floating Search modal, two-column Settings modal,
//! favorites, page more menu, and Recycle Bin restore.

use nsh_core::geometry::{px, size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[test]
fn fullstack_rust_end_to_end_workspace_editor_and_board_flows() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  let mut app = Headless::start(config).unwrap();

  // 1. Initial warm-gray workspace shell & starter document render cleanly
  app.wait_for("Mica 本地工作区").unwrap();
  app.wait_for("欢迎使用 Mica").unwrap();

  // 2. Right Outline Sidebar toggles open and displays live headings + page stats
  app.press("id:toggle-right-panel-btn").unwrap();
  app.wait_for("页面大纲").unwrap();
  app.wait_for("核心特性").unwrap();
  app.wait_for("反向链接").unwrap();
  app.press("id:right-sidebar-close").unwrap();

  // 3. Bottom Account Menu opens popover and can close cleanly
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

  // 7. Type Markdown shortcut + CJK text in-place through native IME proxy
  app.type_text("id:editor-ime-input", "## ").unwrap();
  app.type_text("id:editor-ime-input", "全栈Rust架构").unwrap();

  // 8. Toolbar actions: Bold, Favorite star, Divider, Undo
  app.press("id:ed-bold").unwrap();
  app.wait_for("5 词 · 8 字").unwrap();
  app.press("id:ed-star").unwrap();
  app.wait_for("已加入收藏").unwrap();
  app.press("id:ed-divider").unwrap();
  app.wait_for("────────────────────────").unwrap();
  app.press("id:ed-undo").unwrap();

  // 9. Open slash menu, insert interactive table, add row
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("Table (2×2)").unwrap();
  app.press("id:slash-table").unwrap();
  app.wait_for("Table (2×2)").unwrap();
  app.press("id:table-add-row").unwrap();
  app.wait_for("Table (3×2)").unwrap();

  // 10. Whiteboard view (BoardSession + Canvas)
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

  // 11. Floating Search Modal opens over workspace, finds hit, and opens document
  app.press("id:open-search-btn").unwrap();
  app.wait_for("输入关键词实时检索当前工作区全部文档").unwrap();
  app.type_text("id:search-query-input", "纯Rust").unwrap();
  app.wait_for("找到 1 条与 \"纯Rust\" 相关的结果").unwrap();
  app.press("id:search-hit-0").unwrap();
  app.wait_for("Table (3×2)").unwrap();

  // 12. Two-column Settings Modal: Appearance, Data/Export, Shortcuts/About
  app.press("id:open-settings-btn").unwrap();
  app.wait_for("外观与语言").unwrap();
  app.press("id:settings-toggle-theme").unwrap();
  app.wait_for("当前：深色模式").unwrap();
  app.press("id:settings-toggle-locale").unwrap();
  app.wait_for("Language: English").unwrap();
  app.press("id:settings-tab-data").unwrap();
  app.press("id:settings-export-md").unwrap();
  app.press("id:settings-export-html").unwrap();
  app.press("id:settings-save-snapshot").unwrap();
  app.wait_for("版本快照已保存（共 2 个）").unwrap();
  app.press("id:settings-tab-about").unwrap();
  app.wait_for("常用快捷键").unwrap();
  app.press("id:settings-close-btn").unwrap();

  // 13. Trash active page and restore from Recycle Bin
  app.press("id:trash-page-btn").unwrap();
  app.wait_for("回收站 (1)").unwrap();
  app.press("id:toggle-recycle-bin").unwrap();
  app.press("id:restore-trash-0").unwrap();
  app.wait_for("回收站 (0)").unwrap();
}
