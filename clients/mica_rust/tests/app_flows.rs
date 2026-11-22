//! Headless end-to-end integration tests for Mica's pure-Rust client (`clients/mica_rust`).

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

  // 2. Create a new folder and a new page via sidebar icons
  app.press("id:new-folder-btn").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面 2").unwrap();

  // 3. Rename the active page directly via the borderless large document title
  app.type_text("id:page-title-input", "纯Rust架构设计").unwrap();
  app.wait_for("未命名页面 2纯Rust架构设计").unwrap();

  // 4. Type Markdown shortcut + CJK text in-place through native IME proxy
  app.type_text("id:editor-ime-input", "## ").unwrap();
  app.type_text("id:editor-ime-input", "全栈Rust架构").unwrap();

  // 5. Apply Bold inline mark, insert divider, undo divider
  app.press("id:ed-bold").unwrap();
  app.wait_for("5 词 · 8 字").unwrap();
  app.press("id:ed-divider").unwrap();
  app.wait_for("────────────────────────").unwrap();
  app.press("id:ed-undo").unwrap();

  // 6. Open slash menu and insert an interactive table, add a row
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("Table (2×2)").unwrap();
  app.press("id:slash-table").unwrap();
  app.wait_for("Table (2×2)").unwrap();
  app.press("id:table-add-row").unwrap();
  app.wait_for("Table (3×2)").unwrap();

  // 7. Switch to Whiteboard view (BoardSession + Canvas): sticky, node, mindmap, inline label, SVG export
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

  // 8. Full-text Search across workspace via mica_client::search
  app.press("id:open-search-btn").unwrap();
  app.wait_for("Search Workspace").unwrap();
  app.type_text("id:search-query-input", "纯Rust").unwrap();
  app.wait_for("Found 1 result(s) for \"纯Rust\"").unwrap();
  app.press("id:search-hit-0").unwrap();
  app.wait_for("Table (3×2)").unwrap();

  // 9. Settings panel: theme toggle, locale toggle, Markdown/HTML export, version snapshot
  app.press("id:open-settings-btn").unwrap();
  app.wait_for("Settings & Sync").unwrap();
  app.press("id:settings-toggle-theme").unwrap();
  app.wait_for("Theme: Dark").unwrap();
  app.press("id:settings-toggle-locale").unwrap();
  app.wait_for("Language: English").unwrap();
  app.press("id:settings-export-md").unwrap();
  app.press("id:settings-export-html").unwrap();
  app.press("id:settings-save-snapshot").unwrap();
  app.wait_for("Version snapshot saved (total: 2)").unwrap();

  // 10. Trash active page and restore from Recycle Bin
  app.press("id:settings-close-btn").unwrap();
  app.press("id:trash-page-btn").unwrap();
  app.wait_for("回收站 (1)").unwrap();
  app.press("id:toggle-recycle-bin").unwrap();
  app.press("id:restore-trash-0").unwrap();
  app.wait_for("回收站 (0)").unwrap();
}
