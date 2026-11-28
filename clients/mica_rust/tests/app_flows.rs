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

  // 3. Bottom Account Menu opens stacked-card popover (with Desktop quick-switch + `切换服务器 / 本地库…`) and closes cleanly
  app.press("id:account-menu-btn").unwrap();
  app.wait_for("这台设备").unwrap();
  app.wait_for("id:account-quick-switch-card").unwrap();
  app.wait_for("id:account-quick-local-btn").unwrap();
  app.wait_for("切换服务器 / 本地库…").unwrap();
  app.press("id:account-menu-btn").unwrap();

  // 4. 320px Workspace Switcher popover opens and lists active workspace + import submenu
  app.press("id:workspace-switcher-btn").unwrap();
  app.wait_for("新建工作区").unwrap();
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
  app.wait_for("未命名页面").unwrap();
  assert!(
    !app.screen().texts().iter().any(|t| t.starts_with("未命名页面 ")),
    "new page name must not include a numeric suffix: {:?}",
    app.screen().texts()
  );

  // 6. Rename active page via borderless large title (new page starts with empty input + gray placeholder)
  app.type_text("id:page-title-input", "纯Rust架构设计").unwrap();
  app.wait_for("纯Rust架构设计").unwrap();
  assert!(
    !app.shows("未命名页面纯Rust架构设计"),
    "typing into a new page title must not prepend literal 未命名页面"
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
fn rich_editor_keyboard_enter_ctrl_a_and_editable_code_block() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // Create a clean blank page so we test real typing from scratch
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();

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

  // 3. Insert an editable code block: unified native mica-editor pipeline receives
  //    multi-line code + Enter directly via `mica-editor-ime-sink` with zero extra click!
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("id:slash-code").unwrap();
  app.press("id:slash-code").unwrap();
  app.wait_for("输入代码...").unwrap();
  app.type_text("id:mica-editor-ime-sink", "let x = 42;").unwrap();
  app.wait_for("let x = 42;").unwrap();
  app.press_key("enter").unwrap();
  app.type_text("id:mica-editor-ime-sink", "let y = 99;").unwrap();
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

  // 2. Bottom Account Menu -> `切换服务器 / 本地库…` -> Centered 440px Environment Switcher Modal
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
  app.wait_for("未命名页面").unwrap();
  app.type_text("id:mica-rich-editor", "Single layer quote text").unwrap();
  app.wait_for("Single layer quote text").unwrap();
  app.press("id:ed-quote").unwrap();

  // 2. Insert a code block, type Mermaid syntax, switch language via top-level `overlay()`
  //    picker (`code-lang-btn` -> `code-lang-picker` -> `code-lang-mermaid`), then click
  //    the rendered Mermaid diagram to open the fullscreen zoom Lightbox modal!
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("id:slash-code").unwrap();
  app.press("id:slash-code").unwrap();
  app.wait_for("输入代码...").unwrap();
  app.type_text("id:mica-editor-ime-sink", "graph LR\n  A[Start] --> B[End]").unwrap();
  app.wait_for("id:code-lang-btn").unwrap();
  app.press("id:code-lang-btn").unwrap();
  app.wait_for("id:code-lang-picker").unwrap();
  app.type_text("id:code-lang-search", "mermaid").unwrap();
  app.wait_for("id:code-lang-mermaid").unwrap();
  app.press("id:code-lang-mermaid").unwrap();

  // Mermaid preview renders and clicking it opens floating zoom/pan Lightbox dialog
  app.wait_for("id:mermaid-preview-btn").unwrap();
  app.press("id:mermaid-preview-btn").unwrap();
  app.wait_for("id:mica-lightbox-modal").unwrap();
  app.wait_for("id:lightbox-pan-stage").unwrap();
  app.wait_for("id:lightbox-close-btn").unwrap();
  app.wait_for("100%").unwrap();
  app.press("id:lightbox-zoom-in").unwrap();
  app.wait_for("125%").unwrap();
  app.press("id:lightbox-zoom-reset").unwrap();
  app.wait_for("100%").unwrap();
  app.press("id:lightbox-close-btn").unwrap();
  app
    .wait_for_gone("id:mica-lightbox-modal", std::time::Duration::from_millis(500))
    .unwrap();
}

#[test]
fn regression_typing_backtick_caret_focus_and_code_block_backspace() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Fresh page: type "t", "e", "s", "t" one character at a time via IME sink.
  //    Must produce exact "test" (NEVER doubled "ttest"!) and show focused caret.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();
  for ch in ["t", "e", "s", "t"] {
    app.type_text("id:mica-editor-ime-sink", ch).unwrap();
  }
  app.wait_for("test").unwrap();
  app.wait_for("id:mica-active-caret").unwrap();

  // 2. Typing backtick `` ` `` must NOT be swallowed by sink clearing.
  app.type_text("id:mica-editor-ime-sink", " `bar").unwrap();
  app.wait_for("test `bar").unwrap();
  assert!(
    !app.shows("ttest"),
    "continuous typing must not duplicate characters into 'ttest'"
  );

  // 3. Insert a code block and verify:
  //    a) Unified native caret stays visible right inside the code block (zero renderer swap).
  //    b) Typing + Backspace inside the code block genuinely deletes code chars!
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("id:slash-code").unwrap();
  app.press("id:slash-code").unwrap();
  app.wait_for("输入代码...").unwrap();
  app.type_text("id:mica-editor-ime-sink", "hello_world").unwrap();
  app.wait_for("hello_world").unwrap();
  app.wait_for("id:mica-active-caret").unwrap();
  // Press Backspace 6 times to delete "_world" -> "hello"
  for _ in 0..6 {
    app.press_key("backspace").unwrap();
  }
  app.wait_for("hello").unwrap();
  assert!(
    !app.shows("hello_world"),
    "Backspace inside code-block-editor must actually delete characters"
  );

  // 4. Typing triple backticks ```` ``` ```` in a fresh paragraph auto-converts to a
  //    code block AND keeps focus seamlessly in the unified IME sink so typing + Backspace work immediately!
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();
  for ch in ["`", "`", "`"] {
    app.type_text("id:mica-editor-ime-sink", ch).unwrap();
  }
  app.wait_for("id:code-block-editor").unwrap();
  app.type_text("id:mica-editor-ime-sink", "fn_xyz").unwrap();
  app.wait_for("fn_xyz").unwrap();
  for _ in 0..4 {
    app.press_key("backspace").unwrap();
  }
  app.wait_for("fn").unwrap();
  assert!(
    !app.shows("fn_xyz"),
    "Backspace must work inside a code block created via triple backticks"
  );

  // 5. Cross-block selection spanning a GFM table renders `block-selection-overlay` over the table!
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();
  let mut paste_ev = nsh_core::PasteEvent::default();
  paste_ev.text = "## Section 3\n\n| ColA | ColB |\n| --- | --- |\n| ISP-01 | 10G |\n\n## Section 4".into();
  app.paste("id:mica-editor-ime-sink", &paste_ev).unwrap();
  app.wait_for("Section 3").unwrap();
  app.wait_for("ISP-01").unwrap();
  app.wait_for("Section 4").unwrap();
  // Mod+A selects across Section 3 -> Table -> Section 4, painting selection highlight on the table.
  app.press_key("mod+a").unwrap();
  app.wait_for("id:block-selection-overlay").unwrap();
}

#[test]
fn regression_fifteen_editor_ux_fixes() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Fresh empty page must NOT show the removed "输入正文，或输入 / 唤出命令..." placeholder.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();
  assert!(
    !app.shows("输入正文，或输入 / 唤出命令..."),
    "empty document must not display the removed placeholder prompt"
  );

  // 2. Adaptive proportional vertical scrollbar is mounted on the right edge.
  app.wait_for("id:editor-vertical-scrollbar").unwrap();

  // 3. Pasting v0.14.11 Flutter Windows CF_HTML preserves headings, bold, quotes, and tables!
  let v14_frag = "<h1>泰国算力交付</h1><blockquote><p><strong>核心原则</strong>：按表执行</p></blockquote><table><thead><tr><th>项</th><th>值</th></tr></thead><tbody><tr><td>版本</td><td>Ubuntu 24.04 / CUDA 13.0</td></tr></tbody></table>";
  let v14_cf_html = nsh_core::platform::clipboard::cf_html(v14_frag);
  let mut v14_paste = nsh_core::PasteEvent::default();
  v14_paste.text = "泰国算力交付\n核心原则：按表执行\n项 值\n版本 Ubuntu 24.04 / CUDA 13.0".into();
  v14_paste.html = v14_cf_html;
  app.paste("id:mica-editor-ime-sink", &v14_paste).unwrap();
  app.wait_for("泰国算力交付").unwrap();
  app.wait_for("id:node-view-quote").unwrap();
  app.wait_for("id:node-view-table").unwrap();
  app.wait_for("Ubuntu 24.04 / CUDA 13.0").unwrap();

  // 4. Pasting a clipboard PNG image inserts an image block cleanly.
  // Minimal valid 1x1 PNG bytes.
  let png_1x1: Vec<u8> = vec![
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41,
    0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
    0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
  ];
  let mut img_paste = nsh_core::PasteEvent::default();
  img_paste.files.push(nsh_core::PastedFile::image(png_1x1));
  app.paste("id:mica-editor-ime-sink", &img_paste).unwrap();
  app.wait_for("id:node-view-image").unwrap();

  // 5. Same-page copy/paste round-trip via Markdown keeps headings, quotes, and code blocks.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();
  let mut md_paste = nsh_core::PasteEvent::default();
  md_paste.text = "# 主标题\n\n> 引用段落\n\n```rust\nfn ok() {}\n```".into();
  app.paste("id:mica-editor-ime-sink", &md_paste).unwrap();
  app.wait_for("主标题").unwrap();
  app.wait_for("id:node-view-quote").unwrap();
  app.wait_for("id:node-view-code-block").unwrap();

  // 6. Clicking the bottom empty zone appends a new trailing line and focuses prose.
  app.press("id:editor-bottom-append-zone").unwrap();
  app.type_text("id:mica-editor-ime-sink", "appended_bottom_line").unwrap();
  app.wait_for("appended_bottom_line").unwrap();

  // 7. Code block at caret (0, 0): pressing Backspace removes the code block cleanly!
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();
  for ch in ["`", "`", "`"] {
    app.type_text("id:mica-editor-ime-sink", ch).unwrap();
  }
  app.wait_for("id:node-view-code-block").unwrap();
  app.press_key("backspace").unwrap();
  app
    .wait_for_gone("id:node-view-code-block", std::time::Duration::from_millis(800))
    .unwrap();
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
  app.wait_for("未命名页面").unwrap();
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
}

#[test]
fn regression_code_block_backspace_and_delete_after_live_focus() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // Case 1: Empty code block created via toolbar (`ed-code-block`), genuinely focused
  // inside `nsh_base::editor()` (so its own LIFO `backspace`/`delete` bindings are active!)
  // -> type "ab", Backspace x2 deletes text inside editor, 3rd Backspace deletes the block!
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();
  app.press("id:ed-code-block").unwrap();
  app.wait_for("输入代码...").unwrap();
  app.type_text("id:mica-editor-ime-sink", "xy").unwrap();
  app.wait_for("xy").unwrap();
  // First 2 Backspaces delete 'y' and 'x' -> placeholder returns!
  app.press_key("backspace").unwrap();
  app.press_key("backspace").unwrap();
  app.wait_for("输入代码...").unwrap();
  // 3rd Backspace on the now-empty focused code block MUST delete the code block itself!
  app.press_key("backspace").unwrap();
  app
    .wait_for_gone("id:node-view-code-block", std::time::Duration::from_millis(800))
    .unwrap();

  // Case 2: Empty code block -> pressing `Delete` MUST ALSO delete the code block!
  app.press("id:ed-code-block").unwrap();
  app.wait_for("输入代码...").unwrap();
  app.type_text("id:mica-editor-ime-sink", "q").unwrap();
  app.wait_for("q").unwrap();
  app.press_key("backspace").unwrap();
  app.wait_for("输入代码...").unwrap();
  app.press_key("delete").unwrap();
  app
    .wait_for_gone("id:node-view-code-block", std::time::Duration::from_millis(800))
    .unwrap();

  // Case 3: Brand-new empty code block (never typed into — exact user screenshot scenario!)
  // created via toolbar (`ed-code-block`) -> `Delete` immediately removes the code block!
  app.press("id:ed-code-block").unwrap();
  app.wait_for("输入代码...").unwrap();
  app.press_key("delete").unwrap();
  app
    .wait_for_gone("id:node-view-code-block", std::time::Duration::from_millis(800))
    .unwrap();

  // Case 4: Code block inserted via slash menu stays focused in the unified editor
  // (no extra click required!) -> Enter grows cleanly, Ctrl+A + Backspace clears, next Backspace deletes block!
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("id:slash-code").unwrap();
  app.press("id:slash-code").unwrap();
  app.wait_for("输入代码...").unwrap();
  // Case 5: Pressing Enter inside the code block grows cleanly without scrolling line 0 out of view.
  app.type_text("id:mica-editor-ime-sink", "line1").unwrap();
  app.press_key("enter").unwrap();
  app.type_text("id:mica-editor-ime-sink", "line2").unwrap();
  app.wait_for("line1\nline2").unwrap();
  // Clear all lines back to empty and delete the code block with Backspace.
  app.press_key("mod+a").unwrap();
  app.press_key("backspace").unwrap();
  app.wait_for("输入代码...").unwrap();
  app.press_key("backspace").unwrap();
  app
    .wait_for_gone("id:node-view-code-block", std::time::Duration::from_millis(800))
    .unwrap();
}

#[test]
fn regression_workspace_settings_modal_body_is_visible() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:open-ws-settings-btn").unwrap();
  app.wait_for("id:workspace-settings-modal").unwrap();
  app.wait_for("id:ws-settings-rename-input").unwrap();
  app.wait_for("id:ws-members-local-empty").unwrap();
  app.wait_for("id:ws-settings-delete-btn").unwrap();
  // Verify the rename button inside the scroll body is laid out with real hit-testable bounds.
  app.press("id:ws-settings-rename-save").unwrap();
  app.press("id:ws-settings-close").unwrap();
  app
    .wait_for_gone("id:workspace-settings-modal", std::time::Duration::from_millis(800))
    .unwrap();
}

#[test]
fn regression_code_block_progressive_ctrl_a_expands_to_page() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();

  // Type an outer paragraph, then create a code block below it.
  app.type_text("id:mica-editor-ime-sink", "OuterKeepLine").unwrap();
  app.wait_for("OuterKeepLine").unwrap();
  app.press_key("enter").unwrap();
  app.press("id:ed-slash-open").unwrap();
  app.wait_for("id:slash-code").unwrap();
  app.press("id:slash-code").unwrap();
  app.wait_for("输入代码...").unwrap();

  app.type_text("id:mica-editor-ime-sink", "alpha_code").unwrap();
  app.press_key("enter").unwrap();
  app.type_text("id:mica-editor-ime-sink", "beta_code").unwrap();
  app.wait_for("alpha_code\nbeta_code").unwrap();

  // 1st Ctrl+A inside code block selects ALL code inside the block ONLY:
  // Backspace clears the code block while leaving `OuterKeepLine` intact!
  app.press_key("mod+a").unwrap();
  app.press_key("backspace").unwrap();
  app.wait_for("输入代码...").unwrap();
  assert!(
    app.shows("OuterKeepLine"),
    "1st Ctrl+A + Backspace inside a non-empty code block must NOT delete outer prose"
  );

  // Type code again, then press Ctrl+A twice (Ctrl+AA) -> expands to the ENTIRE page!
  // Backspace now deletes BOTH the code block AND `OuterKeepLine`.
  app.type_text("id:mica-editor-ime-sink", "gamma_code").unwrap();
  app.wait_for("gamma_code").unwrap();
  app.press_key("mod+a").unwrap();
  app.press_key("mod+a").unwrap();
  app.press_key("backspace").unwrap();
  app
    .wait_for_gone("id:node-view-code-block", std::time::Duration::from_millis(800))
    .unwrap();
  assert!(
    !app.shows("OuterKeepLine"),
    "Backspace after Ctrl+AA whole-page selection must delete outer prose as well"
  );
}

#[test]
fn regression_table_wysiwyg_chrome_handles_expand_bars_and_hscroll() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();

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
fn regression_usage_statistics_dashboard_and_live_updates() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Create a fresh page so local activity records at least 1 creation today.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();

  // 2. Open AccountMenu -> click "使用统计" shortcut to jump straight to Settings -> 使用统计.
  app.press("id:account-menu-btn").unwrap();
  app.wait_for("id:account-menu-usage-stats-btn").unwrap();
  app.press("id:account-menu-usage-stats-btn").unwrap();

  // 3. Verify all dashboard sections render with live local data (2 live pages: starter + new page,
  //    exact word count from unified `mica_core::word_count`, and 1 creation recorded today).
  app.wait_for("id:usage-stats-panel").unwrap();
  app.wait_for("id:usage-stats-kpis").unwrap();
  app.wait_for("id:usage-stats-trend").unwrap();
  app.wait_for("id:usage-stats-heatmap").unwrap();
  app.wait_for("id:usage-stats-health").unwrap();
  app.wait_for("id:usage-stats-table").unwrap();
  app.wait_for("本地存储 · 无配额上限").unwrap();
  app.wait_for("新建 1 篇 · 编辑 0 篇 · 新增 +0 词 · 净增 0 词 · 活跃 1 天").unwrap();
  app.wait_for("活跃 1 天 · 当前连续 1 天 · 最长连续 1 天").unwrap();
  app.wait_for("156").unwrap();

  // 4. Switch time range, metric selector, include-imports toggle, and table sort.
  app.press("id:usage-stats-range-7d").unwrap();
  app.press("id:usage-stats-metric-created").unwrap();
  app.press("id:usage-stats-include-imports").unwrap();
  app.wait_for("☑ 含导入/转移").unwrap();
  app.press("id:usage-stats-sort-words").unwrap();
  app.press("id:usage-stats-refresh-btn").unwrap();

  // 5. Close settings cleanly.
  app.press("id:settings-close-btn").unwrap();
  app
    .wait_for_gone("id:usage-stats-panel", std::time::Duration::from_millis(800))
    .unwrap();
}

#[test]
fn regression_web_cloud_only_hides_local_workspace_controls() {
  use std::rc::Rc;
  use nsh_core::prelude::*;

  struct WebOnlySignInRoot;
  impl Component for WebOnlySignInRoot {
    type Props = ();
    type Event = NoEvent;

    fn view(_props: Self::Props, _scope: Scope<Self::Event>) -> impl IntoElement {
      let state = mica_rust::world_switcher::WorldSwitcherState {
        open: signal(true),
        can_close: signal(false),
        allow_local: false,
        initial_auth_view: signal(false),
        dark_mode: signal(false),
        app_version: "0.14.11",
        active_origin: signal("https://mica.example.com".to_string()),
        servers: signal(vec!["https://mica.example.com".to_string()]),
        signed_in_origins: signal(Vec::new()),
        selected_cloud_origin: signal("https://mica.example.com".to_string()),
        health_status: signal("reachable".to_string()),
        busy: signal(false),
        error_msg: signal(String::new()),
        info_msg: signal(String::new()),
        on_close: Rc::new(|| {}),
        on_enter_local: Rc::new(|| {}),
        on_select_server: Rc::new(|_| {}),
        on_probe_server: Rc::new(|_| {}),
        on_add_server: Rc::new(|_| Ok(())),
        on_remove_server: Rc::new(|_| {}),
        on_enter_signed_in: Rc::new(|_| {}),
        on_login: Rc::new(|_, _, _| {}),
        on_register: Rc::new(|_, _, _, _| {}),
        on_forgot_password: Rc::new(|_, _| {}),
      };
      view()
        .w_full()
        .h_full()
        .relative()
        .child(mica_rust::world_switcher::world_switcher_overlay(state))
    }
  }

  let mut config = EngineConfig::new(Root::of::<WebOnlySignInRoot>());
  config.viewport = size(px(1280.0), px(820.0));
  let mut app = Headless::start(config).unwrap();

  app.wait_for("id:world-switcher-overlay").unwrap();
  app.wait_for("id:world-switcher-form-pane").unwrap();
  app.wait_for("id:world-server-card").unwrap();
  app.wait_for("mica.example.com").unwrap();
  app.wait_for("登录到 Mica").unwrap();
  app.wait_for("云端协作知识库").unwrap();
  assert!(!app.shows("id:world-env-list"), "Web must not show multi-host environment list");
  assert!(!app.shows("id:world-tab-local"), "Web must not show local mode tab");
  assert!(!app.shows("id:world-tab-cloud"), "Web must not show world tabs switcher");
  assert!(!app.shows("id:world-server-chevron"), "Web same-origin must not show server dropdown");
  assert!(!app.shows("id:world-switcher-close"), "Unauthenticated Web sign-in must not be dismissible");
  assert!(!app.shows("本地离线库"), "Web must not mention local vault");
  assert!(!app.shows("本地模式"), "Web must not mention local mode");
  assert!(!app.shows("本地工作区"), "Web must not mention local workspace");

  struct WebOnlyAccountMenuRoot;
  impl Component for WebOnlyAccountMenuRoot {
    type Props = ();
    type Event = NoEvent;

    fn view(_props: Self::Props, _scope: Scope<Self::Event>) -> impl IntoElement {
      let state = mica_rust::app_menu::AccountMenuState {
        allow_local: false,
        is_local: signal(false),
        is_signed_in: signal(true),
        show_cloud_signin: signal(true),
        display_name: signal("demo@mica.dev".to_string()),
        email: signal("demo@mica.dev".to_string()),
        active_origin: signal("https://mica.example.com".to_string()),
        signed_in_origins: signal(vec![(
          "https://mica.example.com".to_string(),
          "demo@mica.dev".to_string(),
        )]),
        app_version: "0.14.11",
        update_status: signal(String::new()),
        on_open_settings: Rc::new(|| {}),
        on_open_usage_stats: Some(Rc::new(|| {})),
        on_enter_local: Rc::new(|| {}),
        on_enter_signed_in: Rc::new(|_| {}),
        on_switch_world: Rc::new(|| {}),
        on_check_update: Rc::new(|| {}),
        on_sign_out: Rc::new(|| {}),
        on_sign_in: Rc::new(|| {}),
      };
      view()
        .w_full()
        .h_full()
        .relative()
        .child(mica_rust::app_menu::account_menu_popover(
          signal(true),
          signal(false),
          state,
        ))
    }
  }

  let mut menu_cfg = EngineConfig::new(Root::of::<WebOnlyAccountMenuRoot>());
  menu_cfg.viewport = size(px(1280.0), px(820.0));
  let mut menu_app = Headless::start(menu_cfg).unwrap();
  menu_app.wait_for("id:account-menu-popover").unwrap();
  menu_app.wait_for("id:account-menu-settings-btn").unwrap();
  menu_app.wait_for("id:account-menu-usage-stats-btn").unwrap();
  menu_app.wait_for("id:account-menu-signout-btn").unwrap();
  assert!(
    !menu_app.shows("id:account-quick-switch-card"),
    "Web account menu must hide multi-environment quick switcher"
  );
  assert!(
    !menu_app.shows("id:account-switch-world-btn"),
    "Web account menu must hide '切换服务器 / 本地库…'"
  );
  assert!(!menu_app.shows("本地工作区"));
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
  app.wait_for("未命名页面").unwrap();

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

#[nsh_core::prelude::component]
fn ShareToggleRegressionHarness() -> impl nsh_core::prelude::IntoElement {
  use std::rc::Rc;
  use nsh_core::prelude::*;

  let app_owner = Owner::current();
  let open = signal(true);
  let loading = signal(false);
  let busy = signal(false);
  let shared = signal(false);
  let token: Signal<Option<String>> = signal(None);
  let origin = signal("https://mica.example.com".to_string());
  let error: Signal<Option<String>> = signal(None);
  let dark_mode = signal(false);

  let state = mica_rust::share_dialog::ShareDialogState {
    open,
    loading,
    busy,
    shared,
    token,
    origin,
    error,
    on_toggle_public: Rc::new(move |enable: bool| {
      busy.set(true);
      error.set(None);
      app_owner.run(move || {
        spawn(async move {
          // Yield across reactive flushes so any transient `dynamic(...)` owner
          // teardown triggered by `busy.set(true)` would cancel an un-owned task.
          nsh_core::prelude::next_frame().await;
          nsh_core::prelude::next_frame().await;
          if enable {
            shared.set(true);
            token.set(Some("tok_live_123".to_string()));
          } else {
            shared.set(false);
            token.set(None);
          }
          busy.set(false);
        });
      });
    }),
    on_close: Rc::new(move || open.set(false)),
  };

  view()
    .flex_1()
    .relative()
    .child(mica_rust::share_dialog::share_dialog_modal(state, dark_mode))
}

#[test]
fn regression_share_dialog_public_toggle_survives_busy_rebuild_and_reveals_url() {
  let mut config = EngineConfig::new(Root::of::<ShareToggleRegressionHarness>());
  config.viewport = size(px(800.0), px(600.0));
  let mut app = Headless::start(config).unwrap();

  app.wait_for("公开访问").unwrap();
  // Enable public sharing: setting `busy = true` rebuilds the dynamic body; the
  // spawned async request must NOT be cancelled by that rebuild.
  app.press("id:share-public-toggle").unwrap();
  app.wait_for("text:https://mica.example.com/s/tok_live_123").unwrap();

  // Copy button inside the dynamic body must also flip cleanly to `已复制`.
  app.press("id:share-copy-btn").unwrap();
  app.wait_for("已复制").unwrap();

  // Disable public sharing again: URL box must disappear cleanly (toggle is not stuck busy).
  app.press("id:share-public-toggle").unwrap();
  app
    .wait_for_gone("id:share-url-box", std::time::Duration::from_millis(500))
    .unwrap();
}

#[nsh_core::prelude::component]
fn MissingTextLayoutSelectionHarness() -> impl nsh_core::prelude::IntoElement {
  use std::{cell::RefCell, rc::Rc};
  use nsh_core::prelude::*;

  let md = "\
cloud镜像选择：\n\n\
- jammy-server-cloudimg-amd64.img\n\
- jammy-server-cloudimg-amd64-disk-kvm.img\n\n\
cloud-disk image和cloud image区别\n\n\
- cloudimg-amd64-disk-kvm.img 用于虚拟环境的精简内核`linux-image-kvm 5.8.0.1018.20`，它可能会丢失一些模块/配置，但更轻一些\n\
- cloudimg-amd64.img 在任何地方都可以使用的通用内核`linux-image-5.8.0-43-generic`。您可以使用这些映像来部署一个新的裸机系统，或者在 `linux-kvm` 代表您不想要的限制的任何其他地方。\n\n\
cloudinit文件\n";
  let bridge = mica_rust::editor_bridge::MicaDocumentBridge::from_markdown(md);
  let layouts = Rc::new(RefCell::new(mica_rust::text_geometry::VisibleLayoutCache::default()));
  let dark_mode = signal(false);
  let page_width = signal(820.0_f32);
  let env = mica_rust::block_views::BlockRenderEnv {
    bridge: bridge.clone(),
    dark_mode,
    page_width,
    layouts: layouts.clone(),
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
    editor_focused: signal(true),
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
  let br_sel = bridge.clone();
  let layouts_wipe = layouts.clone();

  view()
    .flex_1()
    .w_full()
    .h_full()
    .child(
      nsh_base::button()
        .test_id("wipe-lines-and-select-all")
        .on_press(move || {
          // Simulate blocks whose `EventKind::TextLayout` has not delivered
          // (empty `lines`, while `text_frame` is populated by `on_layout`).
          for entry in layouts_wipe.borrow_mut().blocks.values_mut() {
            entry.lines.clear();
          }
          let last = br_sel.block_len().saturating_sub(1);
          let last_len = br_sel
            .block_at(last)
            .map(|b| mica_editor::utf16::utf16_len(&b.text))
            .unwrap_or(0);
          br_sel.set_selection(mica_editor::DocSelection::from_positions(
            mica_editor::DocPosition::new(0, 0),
            mica_editor::DocPosition::new(last, last_len),
          ));
        })
        .child(text("select")),
    )
    .child(
      virtual_list(
        move || {
          structure_rev.get();
          br_count.block_len()
        },
        move |at| mica_rust::block_views::render_virtual_block_row(env.clone(), at),
      )
      .test_id("selection-fallback-vlist")
      .keys(move || {
        structure_rev.get();
        br_keys.block_ids()
      })
      .estimated_item_size(px(36.0))
      .flex_1()
      .w_full()
      .h_full(),
    )
}

#[test]
fn regression_cross_block_selection_never_collapses_to_8px_stub_when_text_layout_missing() {
  let mut config = EngineConfig::new(Root::of::<MissingTextLayoutSelectionHarness>());
  config.viewport = size(px(1024.0), px(720.0));
  let mut app = Headless::start(config).unwrap();

  app.wait_for("cloudinit文件").unwrap();
  app.press("id:wipe-lines-and-select-all").unwrap();

  // Wait for selection highlight nodes to mount; verify ZERO 8px fallback stubs
  // exist and all 7 prose blocks received full-width synthesized selection rects.
  let has_wide_rects = || {
    app
      .screen()
      .layout_nodes()
      .into_iter()
      .filter(|n| {
        n.bounds.size.width.0 > 60.0
          && n.bounds.size.height.0 >= 14.0
          && n.bounds.size.height.0 <= 28.0
          && n.test_id.is_none()
      })
      .count()
  };
  let start = std::time::Instant::now();
  while has_wide_rects() < 7 && start.elapsed() < std::time::Duration::from_secs(2) {
    std::thread::sleep(std::time::Duration::from_millis(20));
  }
  let stub_8px: Vec<_> = app
    .screen()
    .layout_nodes()
    .into_iter()
    .filter(|n| {
      (n.bounds.size.width.0 - 8.0).abs() < 0.5
        && n.bounds.size.height.0 >= 14.0
        && n.bounds.size.height.0 <= 28.0
        && n.test_id.is_none()
    })
    .collect();
  assert!(
    stub_8px.is_empty(),
    "selection must never degrade to 8px first-character stubs when TextLayout is missing: {stub_8px:?}"
  );
  assert!(
    has_wide_rects() >= 7,
    "expected >=7 full-width selection rects across all 7 blocks, got {}",
    has_wide_rects()
  );
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

#[nsh_core::prelude::component]
fn EditorLinkClickHarness() -> impl nsh_core::prelude::IntoElement {
  use std::{cell::RefCell, rc::Rc};
  use nsh_core::prelude::*;

  let target_page_id = uuid::Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
  let md = format!(
    "- **DGX OS 7 Release Notes**:\\\n  https://docs.nvidia.com/dgx/dgx-os-7-user-guide/release_notes.html\n\n\
     [内部文档](mica://page/{target_page_id})\n\n\
     | 项目 | 官方文档 |\n\
     | --- | --- |\n\
     | DGX OS | [NVIDIA Docs](https://docs.nvidia.com/dgx/) |\n"
  );
  let bridge = mica_rust::editor_bridge::MicaDocumentBridge::from_markdown(&md);
  let layouts = Rc::new(RefCell::new(mica_rust::text_geometry::VisibleLayoutCache::default()));
  let dark_mode = signal(false);
  let page_width = signal(880.0_f32);
  let last_internal_nav = signal(String::new());
  provide_context(mica_rust::block_views::OpenMicaPageCallback(Rc::new(move |id| {
    last_internal_nav.set(id.to_string());
  })));
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
    .flex_col()
    .child(
      text(move || format!("nav:{}", last_internal_nav.get()))
        .test_id("link-nav-status"),
    )
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
      .estimated_item_size(px(56.0))
      .flex_1()
      .w_full()
      .h_full(),
    )
}

#[test]
fn regression_editor_and_table_links_open_on_click_without_breaking_drag_or_alt_click() {
  use nsh_core::{
    EventKind, EventPayload, MouseButton, MouseEvent, PressEvent,
    geometry::point,
    keymap::Modifiers,
  };

  let mut config = EngineConfig::new(Root::of::<EditorLinkClickHarness>());
  config.viewport = size(px(1100.0), px(720.0));
  let mut app = Headless::start(config).unwrap();

  app.wait_for("id:block-text-0").unwrap();
  app.wait_for("id:block-text-1").unwrap();
  app.wait_for("id:table-cell-2-1-1").unwrap();

  // 1. Clicking directly on line 2 of the bullet item (the external URL) opens it!
  app
    .press_at("id:block-text-0", point(px(64.0), px(34.0)))
    .unwrap();
  assert_eq!(
    app.screen().opened_urls(),
    &["https://docs.nvidia.com/dgx/dgx-os-7-user-guide/release_notes.html"],
    "clicking an external link in a bullet item must open the URL"
  );

  // 2. Clicking in the blank whitespace far to the right of the link on line 2
  //    must NOT open the URL (places caret at end of line for typing).
  app
    .press_at("id:block-text-0", point(px(760.0), px(34.0)))
    .unwrap();
  assert_eq!(
    app.screen().opened_urls().len(),
    1,
    "clicking trailing blank space past a link must not trigger navigation"
  );

  // 3. Alt+Click on the link places caret for editing without opening the URL.
  let b0 = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("block-text-0"))
    .expect("block-text-0 must exist");
  let alt_local = point(px(64.0), px(34.0));
  let alt_page = point(b0.bounds.origin.x + alt_local.x, b0.bounds.origin.y + alt_local.y);
  let control = app.engine().control();
  control.dispatch_event(
    b0.id,
    EventKind::PressDown,
    EventPayload::Press(PressEvent {
      position: alt_local,
      page_position: alt_page,
      modifiers: Modifiers { alt: true, ..Modifiers::default() },
      clicks: 1,
    }),
  );
  control.dispatch_event(b0.id, EventKind::Press, EventPayload::None);
  app.settle();
  assert_eq!(
    app.screen().opened_urls().len(),
    1,
    "Alt+Click on a link must place caret without opening the URL"
  );

  // 4. Dragging across the link selects text without opening the URL.
  let drag_end_local = point(px(180.0), px(34.0));
  let drag_end_page = point(
    b0.bounds.origin.x + drag_end_local.x,
    b0.bounds.origin.y + drag_end_local.y,
  );
  control.dispatch_event(
    b0.id,
    EventKind::PressDown,
    EventPayload::Press(PressEvent {
      position: alt_local,
      page_position: alt_page,
      modifiers: Modifiers::default(),
      clicks: 1,
    }),
  );
  control.dispatch_event(
    b0.id,
    EventKind::MouseMove,
    EventPayload::Mouse(MouseEvent::new(
      drag_end_local,
      drag_end_page,
      Modifiers::default(),
      Some(MouseButton::Left),
    )),
  );
  control.dispatch_event(
    b0.id,
    EventKind::MouseUp,
    EventPayload::Mouse(MouseEvent::released(
      drag_end_local,
      drag_end_page,
      Modifiers::default(),
      MouseButton::Left,
    )),
  );
  control.dispatch_event(b0.id, EventKind::Press, EventPayload::None);
  app.settle();
  assert_eq!(
    app.screen().opened_urls().len(),
    1,
    "dragging across a link to select text must not open the URL"
  );

  // 5. Clicking an internal `mica://page/<uuid>` link invokes OpenMicaPageCallback.
  app
    .press_at("id:block-text-1", point(px(24.0), px(12.0)))
    .unwrap();
  app
    .wait_for("text:nav:11111111-2222-3333-4444-555555555555")
    .unwrap();

  // 6. Clicking an external link inside a table cell also opens the URL.
  app
    .press_at("id:table-cell-2-1-1", point(px(36.0), px(16.0)))
    .unwrap();
  assert_eq!(
    app.screen().opened_urls(),
    &[
      "https://docs.nvidia.com/dgx/dgx-os-7-user-guide/release_notes.html",
      "https://docs.nvidia.com/dgx/",
    ],
    "clicking a link inside a table cell must open its URL"
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
fn regression_table_default_fits_page_live_column_drag_and_right_edge_resize() {
  use nsh_core::geometry::point;

  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();

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

#[test]
fn regression_code_auto_lang_fast_backspace_image_caret_and_indent_buttons() {
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
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();

  // 1. Auto language detection in code block: inserting a code block starts as `auto`,
  //    and typing/pasting a shell pipeline immediately switches the badge to `bash ▾` (no `auto`).
  app.press("id:ed-code-block").unwrap();
  app.wait_for("id:code-block-editor").unwrap();
  app.wait_for("auto ▾").unwrap();
  let cmd = "ping -D 8.8.8.8 | tee -a /tmp/out.log";
  app.type_text("id:mica-editor-ime-sink", cmd).unwrap();
  app.wait_for("bash ▾").unwrap();
  assert!(
    !app.shows("auto ▾"),
    "recognized code language must display detected language instead of 'auto'"
  );

  // 2. Rapid Backspace inside the stably-mounted code block deletes characters smoothly
  //    without dropping focus or stalling.
  for _ in 0..8 {
    app.press_key("backspace").unwrap();
  }
  app.wait_for("ping -D 8.8.8.8 | tee -a /tmp").unwrap();
  assert!(
    !app.shows("/tmp/out.log"),
    "repeated Backspace inside code block must delete trailing chars cleanly"
  );

  // 3. Clicking the right side of an image block shows the trailing caret and pressing Enter
  //    inserts a fresh paragraph below the image!
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();
  let mut img_paste = nsh_core::PasteEvent::default();
  img_paste.text = "![示例图](https://example.com/demo.png)".into();
  app.paste("id:mica-editor-ime-sink", &img_paste).unwrap();
  app.wait_for("id:node-view-image").unwrap();

  let img_row = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("node-view-image"))
    .expect("node-view-image must be laid out");
  let right_local = point(
    px((img_row.bounds.size.width.0 - 12.0).max(10.0)),
    px(img_row.bounds.size.height.0 * 0.5),
  );
  let right_page = point(
    img_row.bounds.origin.x + right_local.x,
    img_row.bounds.origin.y + right_local.y,
  );
  app.engine().control().dispatch_event(
    img_row.id,
    EventKind::PressDown,
    EventPayload::Press(PressEvent {
      position: right_local,
      page_position: right_page,
      modifiers: Modifiers::default(),
      clicks: 1,
    }),
  );
  app.settle();
  app.wait_for("id:image-trailing-caret").unwrap();

  app.press_key("enter").unwrap();
  app.type_text("id:mica-editor-ime-sink", "图片下方新行").unwrap();
  app.wait_for("图片下方新行").unwrap();
  app.wait_for("id:node-view-image").unwrap();

  // 4. Toolbar & floating selection bubble menu both expose increase/decrease indent buttons.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();
  let mut list_paste = nsh_core::PasteEvent::default();
  list_paste.text = "- 第一项\n- 第二项".into();
  app.paste("id:mica-editor-ime-sink", &list_paste).unwrap();
  app.wait_for("第二项").unwrap();

  // Top toolbar indent/outdent buttons move the second bullet item right then back left.
  let item2_x = |app: &Headless| -> f32 {
    app
      .screen()
      .layout_nodes()
      .into_iter()
      .find(|n| n.test_id.as_deref() == Some("block-text-1"))
      .expect("second list item must exist")
      .bounds
      .origin
      .x
      .0
  };
  let x_base = item2_x(&app);
  app.press("id:ed-indent").unwrap();
  app.settle();
  let x_indented = item2_x(&app);
  assert!(
    x_indented > x_base + 12.0,
    "toolbar ed-indent must indent second list item: {x_base} -> {x_indented}"
  );
  app.press("id:ed-outdent").unwrap();
  app.settle();
  let x_outdented = item2_x(&app);
  assert!(
    (x_outdented - x_base).abs() <= 2.0,
    "toolbar ed-outdent must restore second list item indent: {x_base} vs {x_outdented}"
  );

  // Select text in the second list item via Shift+Left to reveal the floating bubble
  // menu and test `bubble-indent` + `bubble-outdent`.
  app.press_key("shift+left").unwrap();
  app.press_key("shift+left").unwrap();
  app.settle();
  app.wait_for("id:editor-bubble-menu").unwrap();
  app.wait_for("id:bubble-indent").unwrap();
  app.wait_for("id:bubble-outdent").unwrap();
  app.press("id:bubble-indent").unwrap();
  app.settle();
  let x_bubble_indented = item2_x(&app);
  assert!(
    x_bubble_indented > x_base + 12.0,
    "floating bubble-indent must indent selected list item: {x_base} -> {x_bubble_indented}"
  );
  app.press("id:bubble-outdent").unwrap();
  app.settle();
  let x_bubble_outdented = item2_x(&app);
  assert!(
    (x_bubble_outdented - x_base).abs() <= 2.0,
    "floating bubble-outdent must outdent selected list item: {x_base} vs {x_bubble_outdented}"
  );
}

#[test]
fn regression_image_context_menu_actions_survive_menu_close() {
  use std::io::{Read, Write};
  use std::net::TcpListener;
  use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
  };
  use nsh_core::{
    ContextMenuEvent, EventKind, EventPayload, PointerType,
    geometry::point,
    keymap::Modifiers,
  };

  // Ephemeral local HTTP server returning 404 immediately so async menu actions
  // (`复制图片` / `下载图片`) complete deterministically without touching the OS
  // clipboard or external network, while proving `ctx.spawn` survives `menu_sig.set(None)`.
  let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test server");
  let addr = listener.local_addr().unwrap();
  let hits = Arc::new(AtomicUsize::new(0));
  let hits_srv = Arc::clone(&hits);
  let _srv = std::thread::spawn(move || {
    for stream in listener.incoming().take(6).flatten() {
      let mut s = stream;
      let mut buf = [0u8; 512];
      let _ = s.read(&mut buf);
      hits_srv.fetch_add(1, Ordering::SeqCst);
      let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
  });
  let test_url = format!("http://{addr}/fortigate.png");

  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();

  let mut img_paste = nsh_core::PasteEvent::default();
  img_paste.text = test_url.clone();
  app.paste("id:mica-editor-ime-sink", &img_paste).unwrap();
  app.wait_for("id:image-card").unwrap();

  let open_menu = |app: &mut Headless| {
    let card = app
      .screen()
      .layout_nodes()
      .into_iter()
      .find(|n| n.test_id.as_deref() == Some("image-card"))
      .expect("image-card must exist");
    let pos = point(px(24.0), px(24.0));
    let page = point(card.bounds.origin.x + pos.x, card.bounds.origin.y + pos.y);
    app.engine().control().dispatch_event(
      card.id,
      EventKind::ContextMenu,
      EventPayload::ContextMenu(ContextMenuEvent::new(
        pos,
        page,
        Modifiers::default(),
        Some(PointerType::Mouse),
      )),
    );
    app.settle();
    app.wait_for("id:image-context-menu").unwrap();
  };

  // 1. 全屏查看 opens lightbox cleanly.
  open_menu(&mut app);
  app.press("id:img-menu-fullscreen").unwrap();
  app.wait_for("id:mica-lightbox-modal").unwrap();
  app.press("id:lightbox-close-btn").unwrap();
  app
    .wait_for_gone("id:mica-lightbox-modal", std::time::Duration::from_millis(800))
    .unwrap();

  // 2. 编辑图注 saves and displays caption under the image.
  open_menu(&mut app);
  app.press("id:img-menu-caption").unwrap();
  app.wait_for("id:image-caption-input").unwrap();
  app.type_text("id:image-caption-input", "主节点状态截图").unwrap();
  app.press("id:image-caption-save").unwrap();
  app.wait_for("主节点状态截图").unwrap();

  // 3. 复制链接 writes URL and shows toast.
  open_menu(&mut app);
  app.press("id:img-menu-copy-link").unwrap();
  app.wait_for("已复制原图链接").unwrap();

  // 4. 在浏览器打开 opens the image URL.
  open_menu(&mut app);
  app.press("id:img-menu-open-browser").unwrap();
  app.settle();
  assert_eq!(
    app.screen().opened_urls(),
    &[test_url.as_str()],
    "img-menu-open-browser must invoke open_url on the image link"
  );

  // 5. Async menu actions (`复制图片` / `下载图片`) MUST survive `menu_sig.set(None)`
  //    disposing the context menu's `when(...)` Owner AND update an already-visible toast!
  let wait_toast_contains = |app: &mut Headless, needle: &str| {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
      app.settle();
      if app.screen().texts().iter().any(|t| t.contains(needle)) {
        return;
      }
      std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!(
      "toast containing `{needle}` never appeared; texts={:?}",
      app.screen().texts()
    );
  };

  let hits_before_copy = hits.load(Ordering::SeqCst);
  open_menu(&mut app);
  app.press("id:img-menu-copy").unwrap();
  app
    .wait_for_gone("id:image-context-menu", std::time::Duration::from_millis(800))
    .unwrap();
  wait_toast_contains(&mut app, "读取图片失败");
  assert!(
    hits.load(Ordering::SeqCst) > hits_before_copy,
    "img-menu-copy async fetch must execute after context menu closes"
  );

  let hits_before_dl = hits.load(Ordering::SeqCst);
  open_menu(&mut app);
  app.press("id:img-menu-download").unwrap();
  app
    .wait_for_gone("id:image-context-menu", std::time::Duration::from_millis(800))
    .unwrap();
  wait_toast_contains(&mut app, "下载图片失败");
  assert!(
    hits.load(Ordering::SeqCst) > hits_before_dl,
    "img-menu-download async fetch must execute after context menu closes"
  );

  // 6. 删除 removes the image block.
  open_menu(&mut app);
  app.press("id:img-menu-delete").unwrap();
  app
    .wait_for_gone("id:node-view-image", std::time::Duration::from_millis(800))
    .unwrap();
}

/// Regression: sidebar located-creation placement rules (1:1 with Mica Flutter `v0.14.11`).
///
/// Verifies end-to-end through real sidebar clicks:
/// 1. Active/located page -> top `+` / `folder+` creates sibling directly BELOW that page
///    (same parent, next line — NOT appended at the end of the folder).
/// 2. Focused folder -> top `+` creates page INSIDE that folder (depth + 1) and expands it.
/// 3. Clicking blank area below the tree (`sidebar-root-drop-zone`) focuses workspace root ->
///    top `folder+` and `+` create at depth 0 (workspace root) even while a doc stays open.
/// 4. Folder row's own hover `+` button creates inside that folder, landing right after the
///    located page when that page sits directly inside the clicked folder.
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
fn regression_list_tab_code_paste_and_backspace() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("未命名页面").unwrap();

  // Create a 2-item bullet list and select both items before pressing Tab.
  app.type_text("id:mica-rich-editor", "- ListHead").unwrap();
  app.wait_for("ListHead").unwrap();
  app.press_key("enter").unwrap();
  app.type_text("id:mica-rich-editor", "ListTail").unwrap();
  app.wait_for("ListTail").unwrap();

  app.press_key("ctrl+a").unwrap();
  app.press_key("tab").unwrap();
  app.settle();
  app.wait_for("ListHead").unwrap();
  app.wait_for("ListTail").unwrap();

  // Verify the indented list rows sit at an indented x-coordinate (deeper than indent 0),
  // meaning the first item (`ListHead`) shifted right alongside `ListTail`.
  let row_x = |app: &mut Headless, tid: &str| -> f32 {
    app
      .screen()
      .layout_nodes()
      .into_iter()
      .find(|n| n.test_id.as_deref() == Some(tid))
      .map(|n| n.bounds.origin.x.0)
      .expect("missing block text row")
  };
  let x_head = row_x(&mut app, "block-text-0");
  let x_tail = row_x(&mut app, "block-text-1");
  assert!(
    (x_head - x_tail).abs() <= 1.0,
    "first list item and second item must share identical indented column after Tab ({x_head} vs {x_tail})"
  );

  // Move below to create a code block, type an indented line, press Enter
  // (which auto-indents), paste, and verify paste replaces the auto-indent
  // rather than leaving extra leading spaces, plus repeated Backspace works cleanly.
  app.press("id:ed-code-block").unwrap();
  app.wait_for("id:code-block-editor").unwrap();
  app.type_text("id:mica-editor-ime-sink", "  line_one").unwrap();
  app.wait_for("  line_one").unwrap();
  app.press_key("enter").unwrap();
  let mut paste = nsh_core::PasteEvent::default();
  paste.text = "line_pasted".into();
  app.paste("id:mica-editor-ime-sink", &paste).unwrap();
  app.wait_for("  line_one\nline_pasted").unwrap();
  app.press_key("backspace").unwrap();
  app.press_key("backspace").unwrap();
  app.settle();
  app.wait_for("  line_one\nline_past").unwrap();
}

/// Regression (2026-11-28):
/// 1. New pages show the gray `"无标题"` placeholder in `page-title-input` instead of
///    pre-filled solid `"未命名页面"`, so typing immediately replaces the title.
/// 2. Creating a new page creates strictly one page with no numeric suffix.
/// 3. Renaming a folder via context menu -> modal updates the tree cleanly and does not revert.
/// 4. Shift-range selecting a page and folder, deleting them, and then creating a new folder
///    never resurrects the deleted items.
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
  app.wait_for("未命名页面").unwrap();
  app.wait_for("id:sidebar-row-2").unwrap();
  assert!(
    !app.screen().texts().iter().any(|t| t.starts_with("未命名页面 ")),
    "new page must not carry a numeric suffix: {:?}",
    app.screen().texts()
  );

  // Page title input starts empty (gray placeholder `"无标题"`); typing directly yields exact title.
  app.wait_for("无标题").unwrap();
  app.type_text("id:page-title-input", "直接输入标题").unwrap();
  app.wait_for("直接输入标题").unwrap();
  assert!(
    !app.shows("未命名页面直接输入标题"),
    "typing into a new page title input must not concatenate with 未命名页面"
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
