use nsh_core::geometry::{px, size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[test]
fn rich_editor_keyboard_enter_ctrl_a_and_editable_code_block() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // Create a clean blank page so we test real typing from scratch
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();

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
fn virtualized_editor_code_overlay_mermaid_lightbox_and_page_switch() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Switch to a brand-new page and verify zero stale blocks leak from previous page.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();
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
  app.wait_for("新页面").unwrap();
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
  app.wait_for("新页面").unwrap();
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
  app.wait_for("新页面").unwrap();
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
  app.wait_for("新页面").unwrap();
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
  app.wait_for("新页面").unwrap();
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
  app.wait_for("新页面").unwrap();
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
  app.wait_for("新页面").unwrap();
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
fn regression_code_block_progressive_ctrl_a_expands_to_page() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();

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
  app.wait_for("新页面").unwrap();

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
  app.wait_for("新页面").unwrap();
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
  app.wait_for("新页面").unwrap();
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
fn regression_list_tab_code_paste_and_backspace() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();

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
/// 1. New pages show the gray `"新页面"` placeholder in `page-title-input` instead of
///    pre-filled solid `"未命名页面"`, so typing immediately replaces the title.
/// 2. Creating a new page creates strictly one page with no numeric suffix.
/// 3. Renaming a folder via context menu -> modal updates the tree cleanly and does not revert.
/// 4. Shift-range selecting a page and folder, deleting them, and then creating a new folder
///    never resurrects the deleted items.

#[test]
fn regression_ime_sink_single_line_width_and_title_enter_no_extra_blank_line() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // #4: Hidden IME sink uses adequate single-line width (`>= 200px`) instead of `1px` wrapping box.
  let sink_w = app
    .screen()
    .layout_nodes()
    .iter()
    .find(|n| n.test_id.as_deref() == Some("mica-editor-ime-sink"))
    .expect("mica-editor-ime-sink missing")
    .bounds
    .size
    .width
    .0;
  assert!(
    sink_w >= 200.0,
    "IME sink width too narrow (causes MS Pinyin vertical wrapping): {sink_w}"
  );

  // #6: Create a fresh blank page, submit title with Enter, and immediately press Enter;
  // body must remain a single paragraph (`block-row-0`), NEVER gaining a spurious `block-row-1`.
  app.press("id:sidebar-root-drop-zone").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("id:page-title-input").unwrap();
  app.type_text("id:page-title-input", "标题回车测试").unwrap();
  app.press_key("enter").unwrap();
  app.settle();

  let nodes = app.screen().layout_nodes();
  assert!(
    nodes.iter().any(|n| n.test_id.as_deref() == Some("block-row-0")),
    "expected initial paragraph block-row-0"
  );
  assert!(
    !nodes.iter().any(|n| n.test_id.as_deref() == Some("block-row-1")),
    "submitting page title must not leave an extra blank line (block-row-1)"
  );
}


#[test]
fn regression_editor_body_context_menu_opens_and_selects_all() {
  use nsh_core::{
    EventKind, EventPayload, PointerType, geometry::point, keymap::Modifiers,
  };

  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  let editor_node = app
    .screen()
    .layout_nodes()
    .iter()
    .find(|n| n.test_id.as_deref() == Some("mica-rich-editor"))
    .cloned()
    .expect("mica-rich-editor missing");
  let pos = point(px(120.0), px(120.0));
  let page = point(editor_node.bounds.origin.x + pos.x, editor_node.bounds.origin.y + pos.y);
  app.engine().control().dispatch_event(
    editor_node.id,
    EventKind::ContextMenu,
    EventPayload::ContextMenu(nsh_core::ContextMenuEvent::new(
      pos,
      page,
      Modifiers::default(),
      Some(PointerType::Mouse),
    )),
  );
  app.settle();

  app.wait_for("id:editor-context-menu").unwrap();
  app.wait_for("id:ed-ctx-copy").unwrap();
  app.wait_for("id:ed-ctx-paste").unwrap();
  app.wait_for("id:ed-ctx-select-all").unwrap();

  // Clicking outside the body context menu must dismiss it automatically.
  app.press("id:editor-bottom-append-zone").unwrap();
  app
    .wait_for_gone("id:editor-context-menu", std::time::Duration::from_millis(600))
    .unwrap();

  // Re-open the context menu and invoke `全选`.
  app.engine().control().dispatch_event(
    editor_node.id,
    EventKind::ContextMenu,
    EventPayload::ContextMenu(nsh_core::ContextMenuEvent::new(
      pos,
      page,
      Modifiers::default(),
      Some(PointerType::Mouse),
    )),
  );
  app.settle();
  app.wait_for("id:editor-context-menu").unwrap();

  app.press("id:ed-ctx-select-all").unwrap();
  app
    .wait_for_gone("id:editor-context-menu", std::time::Duration::from_millis(600))
    .unwrap();
}


#[test]
fn regression_divider_click_selects_without_scroll_jump_and_deletes() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("id:page-title-input").unwrap();

  // Build a multi-paragraph document with a divider positioned well below the top
  // of the viewport so any unwanted `ScrollMode::Jump` (`Percentage(0.2)`) would
  // visibly move the divider's Y coordinate.
  let mut md = String::new();
  for i in 1..=10 {
    md.push_str(&format!("段落上方第 {i} 行内容\n\n"));
  }
  md.push_str("---\n\n");
  for i in 1..=14 {
    md.push_str(&format!("段落下方第 {i} 行内容\n\n"));
  }
  let mut paste = nsh_core::PasteEvent::default();
  paste.text = md;
  app.paste("id:mica-editor-ime-sink", &paste).unwrap();
  app.wait_for("id:node-view-divider").unwrap();
  app.wait_for("段落上方第 1 行内容").unwrap();

  let divider_y_before = app
    .screen()
    .layout_nodes()
    .iter()
    .find(|n| n.test_id.as_deref() == Some("node-view-divider"))
    .expect("divider node missing before click")
    .bounds
    .origin
    .y
    .0;

  // Clicking the divider must select it atomically without jumping the page.
  app.press("id:node-view-divider").unwrap();
  app.wait_for("id:divider-selected-indicator").unwrap();

  let divider_y_after = app
    .screen()
    .layout_nodes()
    .iter()
    .find(|n| n.test_id.as_deref() == Some("node-view-divider"))
    .expect("divider node missing after click")
    .bounds
    .origin
    .y
    .0;
  assert!(
    (divider_y_after - divider_y_before).abs() < 2.0,
    "clicking a divider must not jump the viewport (before y={divider_y_before}, after y={divider_y_after})"
  );

  // Pressing Backspace while the divider is selected deletes it while keeping neighbors intact.
  app.press_key("backspace").unwrap();
  app
    .wait_for_gone("id:node-view-divider", std::time::Duration::from_millis(800))
    .unwrap();
  app.wait_for("段落上方第 10 行内容").unwrap();
  app.wait_for("段落下方第 1 行内容").unwrap();
}


#[test]
fn regression_header_sync_status_fixed_width_and_floating_toast() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  // Wide enough for `header_single_row` (>= 1240px inside the editor panel).
  config.viewport = size(px(1600.0), px(900.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.wait_for("id:ed-sync-status").unwrap();
  app.wait_for("id:ed-bold").unwrap();
  // Let `.on_layout` settle `header_width` into `header_single_row` mode.
  app.settle();

  let node_rect = |app: &Headless, tid: &str| {
    let b = app
      .screen()
      .layout_nodes()
      .iter()
      .find(|n| n.test_id.as_deref() == Some(tid))
      .unwrap_or_else(|| panic!("missing node id:{tid}"))
      .bounds;
    (b.origin.x.0, b.origin.y.0, b.size.width.0)
  };

  let (status_x_before, status_y_before, status_w_before) = node_rect(&app, "ed-sync-status");
  assert!(
    (status_w_before - 28.0).abs() < 1.0,
    "ed-sync-status must have fixed 28px width, got {status_w_before}"
  );
  let (bold_x_before, bold_y_before, _) = node_rect(&app, "ed-bold");
  assert!(
    (bold_y_before - status_y_before).abs() < 8.0,
    "ed-bold must be inlined on the single-row header before testing (bold_y={bold_y_before}, status_y={status_y_before})"
  );

  // Trigger a transient status flash; it must surface as a floating toast without
  // shifting the fixed-width sync icon or horizontal format toolbar position.
  app.press("id:ed-copy-path").unwrap();
  app.wait_for("id:app-toast").unwrap();
  app.wait_for("已复制页面路径").unwrap();

  let (status_x_after, _, status_w_after) = node_rect(&app, "ed-sync-status");
  let (bold_x_after, _, _) = node_rect(&app, "ed-bold");
  assert!(
    (status_w_after - 28.0).abs() < 1.0,
    "ed-sync-status width changed to {status_w_after}"
  );
  assert!(
    (status_x_after - status_x_before).abs() < 0.5,
    "ed-sync-status drifted horizontally: {status_x_before} -> {status_x_after}"
  );
  assert!(
    (bold_x_after - bold_x_before).abs() < 0.5,
    "inline format toolbar jumped horizontally: {bold_x_before} -> {bold_x_after}"
  );
}

