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

  // 6. Rename active page via borderless large title
  app.type_text("id:page-title-input", "纯Rust架构设计").unwrap();
  app.wait_for("未命名页面纯Rust架构设计").unwrap();

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
  app.wait_for("未命名页面纯Rust架构设计").unwrap();

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
  app.wait_for("id:world-server-card").unwrap();
  app.wait_for("mica.example.com").unwrap();
  app.wait_for("云端协作\n知识库").unwrap();
  assert!(!app.shows("id:world-tab-local"), "Web must not show local mode tab");
  assert!(!app.shows("id:world-tab-cloud"), "Web must not show world tabs switcher");
  assert!(!app.shows("id:world-server-chevron"), "Web same-origin must not show server dropdown");
  assert!(!app.shows("id:world-switcher-close"), "Unauthenticated Web sign-in must not be dismissible");
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
        app_version: "0.14.11",
        update_status: signal(String::new()),
        on_open_settings: Rc::new(|| {}),
        on_open_usage_stats: Some(Rc::new(|| {})),
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
    !menu_app.shows("id:account-switch-world-btn"),
    "Web account menu must hide '切换世界…'"
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
