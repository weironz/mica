//! Headless end-to-end integration tests for the pure-Rust Mica client (`clients/mica_rust`).

use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[test]
fn fullstack_rust_end_to_end_workspace_editor_and_board_flows() {
  let mut app = Headless::start(EngineConfig::new(Root::of::<mica_rust::MicaApp>())).unwrap();

  // 1. Initial workspace tree and Markdown document render cleanly
  app.wait_for("Mica Workspace").unwrap();
  app.wait_for("[H1] Mica Fullstack Rust").unwrap();

  // 2. Create a new page via sidebar -> switches editor to clean page
  app.press("id:new-page-btn").unwrap();
  app.wait_for("Untitled Page 2").unwrap();

  // 3. Type Markdown shortcut + CJK text through native IME proxy
  app.type_text("id:editor-ime-input", "## ").unwrap();
  app.type_text("id:editor-ime-input", "全栈Rust架构").unwrap();
  app.wait_for("[H2] 全栈Rust架构").unwrap();

  // 4. Insert divider and undo/redo via L2 EditorSession
  app.press("id:ed-divider").unwrap();
  app.wait_for("────────────────────────").unwrap();
  app.press("id:ed-undo").unwrap();

  // 5. Switch to Whiteboard tab (BoardSession + Canvas) and add nodes/mind-child
  app.press("id:mode-board").unwrap();
  app.wait_for("Auto Layout").unwrap();
  app.press("id:board-add-node").unwrap();
  app.press("id:board-add-child").unwrap();
  app.press("id:board-layout").unwrap();
  app.press("id:board-undo").unwrap();

  // 6. Switch back to Document tab -> content remains intact
  app.press("id:mode-doc").unwrap();
  app.wait_for("[H2] 全栈Rust架构").unwrap();
}
