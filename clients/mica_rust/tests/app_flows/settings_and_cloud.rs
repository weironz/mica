use nsh_core::geometry::{px, size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

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
fn regression_usage_statistics_dashboard_and_live_updates() {
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).unwrap();

  app.wait_for("欢迎使用 Mica").unwrap();

  // 1. Create a fresh page so local activity records at least 1 creation today.
  app.press("id:new-page-btn").unwrap();
  app.wait_for("新页面").unwrap();

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
        avatar_url: signal(None),
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
    "Web account menu must hide '管理服务器…'"
  );
  assert!(
    !menu_app.shows("id:account-menu-update-btn"),
    "Web account menu must hide '检查更新'"
  );
  assert!(
    !menu_app.shows("检查更新"),
    "Web account menu must not show update text"
  );
  assert!(!menu_app.shows("本地工作区"));

  struct WebOnlyAppMenuAndAboutRoot;
  impl Component for WebOnlyAppMenuAndAboutRoot {
    type Props = ();
    type Event = NoEvent;

    fn view(_props: Self::Props, _scope: Scope<Self::Event>) -> impl IntoElement {
      let app_cb = mica_rust::app_menu::AppMenuCallbacks {
        on_new_page: Rc::new(|| {}),
        on_new_tab: Rc::new(|| {}),
        on_close_tab: Rc::new(|| {}),
        can_close_tab: signal(false),
        on_open_settings: Rc::new(|| {}),
        sidebar_open: signal(true),
        on_toggle_sidebar: Rc::new(|| {}),
        right_panel_open: signal(false),
        on_toggle_right_panel: Rc::new(|| {}),
        can_go_back: signal(false),
        can_go_forward: signal(false),
        on_go_back: Rc::new(|| {}),
        on_go_forward: Rc::new(|| {}),
        on_open_about: Rc::new(|| {}),
        show_check_update: false,
        on_check_update: Rc::new(|| {}),
      };
      let about_props = mica_rust::app_menu::AboutModalProps {
        open: signal(true),
        dark_mode: signal(false),
        app_version: "0.14.11",
        show_check_update: false,
        update_status: signal(String::new()),
        available_update: signal(None),
        update_busy: signal(false),
        on_check_update: Rc::new(|| {}),
        on_install_update: Rc::new(|| {}),
      };
      view()
        .w_full()
        .h_full()
        .relative()
        .child(mica_rust::app_menu::app_menu_popover(
          signal(true),
          signal(false),
          app_cb,
        ))
        .child(mica_rust::app_menu::about_modal_view(about_props))
    }
  }

  let mut web_menu_cfg = EngineConfig::new(Root::of::<WebOnlyAppMenuAndAboutRoot>());
  web_menu_cfg.viewport = size(px(1280.0), px(820.0));
  let mut web_menu_app = Headless::start(web_menu_cfg).unwrap();
  web_menu_app.wait_for("id:app-menu-popover").unwrap();
  web_menu_app.press("id:app-menu-group-help").unwrap();
  web_menu_app.wait_for("id:app-menu-about").unwrap();
  assert!(
    !web_menu_app.shows("id:app-menu-check-update"),
    "Web help menu must hide '检查更新'"
  );
  web_menu_app.wait_for("当前版本 v0.14.11").unwrap();
  web_menu_app.wait_for("id:about-close-btn").unwrap();
  assert!(
    !web_menu_app.shows("id:about-check-update-btn"),
    "Web About modal must hide '检查更新'"
  );
  assert!(
    !web_menu_app.shows("id:about-install-update-btn"),
    "Web About modal must hide install-update button"
  );
  assert!(
    !web_menu_app.shows("检查更新"),
    "Web app menu and About modal must not show '检查更新' anywhere"
  );
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


mod wave2_settings_tests {
  use std::rc::Rc;

  use nsh_core::geometry::{px, size};
  use nsh_core::prelude::*;
  use nsh_engine::{EngineConfig, Root};
  use nsh_testing::Headless;

  fn build_settings_view(signed_in: bool) -> impl IntoElement {
    let dark = signal(false);
    let legacy = Rc::new(mica_rust::settings_view::SettingsCallbacks {
      dark_mode: dark,
      locale_zh: signal(true),
      server_url: signal("https://mica.example.com".to_string()),
      cloud_status: signal(String::new()),
      export_preview: signal(String::new()),
      account_email: signal("user@mica.dev".to_string()),
      is_cloud_signed_in: signal(signed_in),
      on_toggle_theme: Rc::new(|| {}),
      on_toggle_locale: Rc::new(|| {}),
      on_check_server: Rc::new(|_| {}),
      on_cloud_login: Rc::new(|_, _, _| {}),
      on_cloud_logout: Rc::new(|| {}),
      on_sync_now: Rc::new(|| {}),
      on_export_markdown: Rc::new(|| {}),
      on_export_html: Rc::new(|| {}),
      on_export_bundle: Rc::new(|| {}),
      on_import_markdown: Rc::new(|_| {}),
      on_save_snapshot: Rc::new(|| {}),
      on_close: Rc::new(|| {}),
    });
    let mut props = mica_rust::settings_view::SettingsViewProps::from_legacy(legacy);
    props.allow_local = false;
    if signed_in {
      props.avatar_url.set(Some(
        "https://mica.example.com/api/users/u1/avatar?v=1".to_string(),
      ));
    }
    view()
      .w_full()
      .h_full()
      .relative()
      .child(mica_rust::settings_view::settings_modal_view_with_props(
        Rc::new(props),
      ))
  }

  #[derive(Default)]
  struct Wave2SettingsSignedOut;

  impl Component for Wave2SettingsSignedOut {
    type Props = ();
    type Event = NoEvent;

    fn view(_props: Self::Props, _scope: Scope<Self::Event>) -> impl IntoElement {
      build_settings_view(false)
    }
  }

  #[derive(Default)]
  struct Wave2SettingsSignedIn;

  impl Component for Wave2SettingsSignedIn {
    type Props = ();
    type Event = NoEvent;

    fn view(_props: Self::Props, _scope: Scope<Self::Event>) -> impl IntoElement {
      build_settings_view(true)
    }
  }

  #[test]
  fn regression_settings_mobile_responsive_layout_shows_full_width_detail_pane() {
    let mut config = EngineConfig::new(Root::of::<Wave2SettingsSignedOut>());
    config.viewport = size(px(390.0), px(844.0));
    config.environment.os = nsh_core::platform::Os::Windows;
    let mut app = Headless::start(config).unwrap();

    app.wait_for("id:settings-mobile-nav").unwrap();
    app.wait_for("id:settings-detail-scroll").unwrap();

    let screen = app.screen();
    let nodes = screen.layout_nodes();
    let panel = nodes
      .iter()
      .find(|n| n.test_id.as_deref() == Some("settings-panel"))
      .expect("settings-panel missing");
    let detail = nodes
      .iter()
      .find(|n| n.test_id.as_deref() == Some("settings-detail-scroll"))
      .expect("settings-detail-scroll missing");
    assert!(
      panel.bounds.size.width.0 <= 390.0,
      "settings-panel overflowed 390px: {}",
      panel.bounds.size.width.0
    );
    assert!(
      detail.bounds.size.width.0 >= panel.bounds.size.width.0 - 12.0,
      "detail pane cut off on mobile: panel_w={}, detail_w={}",
      panel.bounds.size.width.0,
      detail.bounds.size.width.0
    );
    assert!(
      (detail.bounds.origin.x.0 - panel.bounds.origin.x.0).abs() < 12.0,
      "detail pane offset horizontally on mobile: panel_x={}, detail_x={}",
      panel.bounds.origin.x.0,
      detail.bounds.origin.x.0
    );

    app.press("id:settings-tab-account").unwrap();
    app.wait_for("id:settings-server-url-readonly").unwrap();
  }

  #[test]
  fn regression_settings_web_mode_hides_desktop_only_controls_and_exposes_avatar_actions() {
    let mut config = EngineConfig::new(Root::of::<Wave2SettingsSignedIn>());
    config.viewport = size(px(1200.0), px(840.0));
    config.environment.os = nsh_core::platform::Os::Windows;
    let mut app = Headless::start(config).unwrap();

    app.wait_for("id:settings-panel").unwrap();
    assert!(
      !app
        .screen()
        .layout_nodes()
        .iter()
        .any(|n| n.test_id.as_deref() == Some("settings-tab-diagnostics")),
      "Diagnostics tab must stay hidden when allow_local == false"
    );
    assert!(
      !app
        .screen()
        .layout_nodes()
        .iter()
        .any(|n| n.test_id.as_deref() == Some("close-behavior-quit")),
      "Desktop window close behavior must stay hidden on Web"
    );

    app.press("id:settings-tab-account").unwrap();
    app.wait_for("id:settings-change-avatar-btn").unwrap();
    app.wait_for("id:settings-remove-avatar-btn").unwrap();

    app.press("id:settings-tab-data").unwrap();
    app.wait_for("id:settings-upload-md-web-btn").unwrap();
    let nodes = app.screen().layout_nodes().to_vec();
    for hidden_id in [
      "settings-import-folder-btn",
      "settings-import-path-input",
      "settings-clear-cache-btn",
    ] {
      assert!(
        nodes.iter().all(|n| n.test_id.as_deref() != Some(hidden_id)),
        "Desktop-only Data control {hidden_id} should be hidden on Web"
      );
    }
  }
}


