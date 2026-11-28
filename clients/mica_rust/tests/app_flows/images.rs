use nsh_core::geometry::{px, size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

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
  app.wait_for("新页面").unwrap();

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
fn regression_image_compact_hover_toolbar_and_frameless_lightbox() {
  use std::{io::Read, io::Write, net::TcpListener};

  let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test server");
  let addr = listener.local_addr().unwrap();
  let _srv = std::thread::spawn(move || {
    for stream in listener.incoming().take(4).flatten() {
      let mut s = stream;
      let mut buf = [0u8; 512];
      let _ = s.read(&mut buf);
      let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
  });
  let test_url = format!("http://{addr}/ocp-demo.png");

  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();

  let mut img_paste = nsh_core::PasteEvent::default();
  img_paste.text = test_url;
  app.paste("id:mica-editor-ime-sink", &img_paste).unwrap();
  app.wait_for("id:image-card").unwrap();

  // Hovering the image reveals a compact icon-only toolbar instead of text buttons.
  app.hover("id:image-card").unwrap();
  app.wait_for("id:image-hover-toolbar").unwrap();
  app.wait_for("id:image-zoom-btn").unwrap();
  app.wait_for("id:image-copy-btn").unwrap();
  app.wait_for("id:image-align-btn").unwrap();
  app.wait_for("id:image-caption-btn").unwrap();
  app.wait_for("id:image-more-btn").unwrap();

  let tb = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("image-hover-toolbar"))
    .expect("image-hover-toolbar must be laid out");
  assert!(
    tb.bounds.size.width.0 <= 180.0,
    "compact icon hover toolbar width must be <= 180px, got {}",
    tb.bounds.size.width.0
  );
  let visible_texts = app.screen().texts();
  assert!(
    !visible_texts.iter().any(|&t| t == "放大" || t == "删除"),
    "hover toolbar must not render bulky inline Chinese text labels: {visible_texts:?}"
  );

  let card_x = |app: &mut Headless| {
    app
      .screen()
      .layout_nodes()
      .into_iter()
      .find(|n| n.test_id.as_deref() == Some("image-card"))
      .expect("image-card must exist")
      .bounds
      .origin
      .x
      .0
  };
  let initial_x = card_x(&mut app);

  // Align button cycles left (default) -> center -> right -> left cleanly.
  app.press("id:image-align-btn").unwrap();
  app.settle();
  let center_x = card_x(&mut app);
  assert!(
    center_x > initial_x + 40.0,
    "expected center alignment to move card right ({initial_x} -> {center_x})"
  );

  app.press("id:image-align-btn").unwrap();
  app.settle();
  let right_x = card_x(&mut app);
  assert!(
    right_x > center_x + 40.0,
    "expected right alignment to move card further right ({center_x} -> {right_x})"
  );

  app.press("id:image-align-btn").unwrap();
  app.settle();
  let left_x = card_x(&mut app);
  assert!(
    (left_x - initial_x).abs() < 4.0,
    "expected cycling back to left alignment ({initial_x} vs {left_x})"
  );

  // `⋯` button opens the full image menu with 3-way alignment controls.
  app.press("id:image-more-btn").unwrap();
  app.wait_for("id:image-context-menu").unwrap();
  app.wait_for("id:img-menu-align-left").unwrap();
  app.wait_for("id:img-menu-align-center").unwrap();
  app.wait_for("id:img-menu-align-right").unwrap();
  app.press("id:img-menu-align-center").unwrap();
  app
    .wait_for_gone("id:image-context-menu", std::time::Duration::from_millis(600))
    .unwrap();
  let recentered_x = card_x(&mut app);
  assert!(
    (recentered_x - center_x).abs() < 4.0,
    "img-menu-align-center must set center alignment ({center_x} vs {recentered_x})"
  );

  // Zoom button opens frameless full-viewport image lightbox (no boxed header or margin card).
  app.hover("id:image-card").unwrap();
  app.wait_for("id:image-zoom-btn").unwrap();
  app.press("id:image-zoom-btn").unwrap();
  app.wait_for("id:mica-lightbox-modal").unwrap();
  app.wait_for("id:lightbox-pan-stage").unwrap();
  app.wait_for("id:lightbox-zoom-in").unwrap();
  app.wait_for("id:lightbox-zoom-out").unwrap();
  app.wait_for("id:lightbox-zoom-reset").unwrap();
  app.wait_for("id:lightbox-close-btn").unwrap();

  let backdrop = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("mica-lightbox-backdrop"))
    .expect("mica-lightbox-backdrop must exist");
  let modal = app
    .screen()
    .layout_nodes()
    .into_iter()
    .find(|n| n.test_id.as_deref() == Some("mica-lightbox-modal"))
    .expect("mica-lightbox-modal must exist");
  assert_eq!(
    modal.bounds.size,
    backdrop.bounds.size,
    "image lightbox must be frameless and fill backdrop cleanly without an enclosing card"
  );
  let lb_texts = app.screen().texts();
  assert!(
    !lb_texts.iter().any(|&t| t == "图片预览" || t.contains("双指捏合")),
    "frameless image lightbox must not render boxed modal header or hint banner: {lb_texts:?}"
  );

  app.press("id:lightbox-zoom-in").unwrap();
  app.wait_for("125%").unwrap();
  app.press("id:lightbox-zoom-reset").unwrap();
  app.wait_for("100%").unwrap();

  app.press("id:lightbox-close-btn").unwrap();
  app
    .wait_for_gone("id:mica-lightbox-modal", std::time::Duration::from_millis(600))
    .unwrap();
}


