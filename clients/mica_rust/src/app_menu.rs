//! Application menu popover, About Mica modal, and grouped-card Account menu popover.
//!
//! 1:1 visual and semantic parity with:
//! - `main:clients/mica_flutter/lib/ui/application_menu.dart`
//! - `main:clients/mica_flutter/lib/ui/about_dialog.dart`
//! - `main:clients/mica_flutter/lib/ui/account_menu.dart`

use std::rc::Rc;

use nsh_base::button;
use nsh_core::{
  color::rgba,
  element::{ImageSrc, Svg, svg},
  geometry::px,
  prelude::*,
};

use crate::{mica_icons, mica_theme::palette};

const MICA_LOGO_SVG: &str = include_str!("../../../assets/mica-logo.svg");
const GITHUB_REPO_URL: &str = "https://github.com/weironz/mica";

const GROUP_FILE: u8 = 0;
const GROUP_VIEW: u8 = 1;
const GROUP_HISTORY: u8 = 2;
const GROUP_HELP: u8 = 3;

#[cfg(target_os = "macos")]
const SHORTCUT_NEW_PAGE: &str = "⌘N";
#[cfg(not(target_os = "macos"))]
const SHORTCUT_NEW_PAGE: &str = "Ctrl+N";

#[cfg(target_os = "macos")]
const SHORTCUT_NEW_TAB: &str = "⌘T";
#[cfg(not(target_os = "macos"))]
const SHORTCUT_NEW_TAB: &str = "Ctrl+T";

#[cfg(target_os = "macos")]
const SHORTCUT_CLOSE_TAB: &str = "⌘W";
#[cfg(not(target_os = "macos"))]
const SHORTCUT_CLOSE_TAB: &str = "Ctrl+W";

#[cfg(target_os = "macos")]
const SHORTCUT_SETTINGS: &str = "⌘,";
#[cfg(not(target_os = "macos"))]
const SHORTCUT_SETTINGS: &str = "Ctrl+,";

/// Callbacks and reactive state backing the top-left Application Menu popover.
#[derive(Clone)]
pub struct AppMenuCallbacks {
  pub on_new_page: Rc<dyn Fn()>,
  pub on_new_tab: Rc<dyn Fn()>,
  pub on_close_tab: Rc<dyn Fn()>,
  pub can_close_tab: Signal<bool>,
  pub on_open_settings: Rc<dyn Fn()>,
  pub sidebar_open: Signal<bool>,
  pub on_toggle_sidebar: Rc<dyn Fn()>,
  pub right_panel_open: Signal<bool>,
  pub on_toggle_right_panel: Rc<dyn Fn()>,
  pub can_go_back: Signal<bool>,
  pub can_go_forward: Signal<bool>,
  pub on_go_back: Rc<dyn Fn()>,
  pub on_go_forward: Rc<dyn Fn()>,
  pub on_open_about: Rc<dyn Fn()>,
  pub on_check_update: Rc<dyn Fn()>,
}

/// Top-left cascading Application Menu popover anchored below the 34px title bar.
pub fn app_menu_popover(
  open: Signal<bool>,
  dark_mode: Signal<bool>,
  cb: AppMenuCallbacks,
) -> impl IntoElement {
  when(move || open.get(), {
    let cb = cb.clone();
    move || {
      let active_group = signal(GROUP_FILE);
      let cb_panel = cb.clone();

      view()
        .test_id("app-menu-root")
        .absolute()
        .inset_0()
        .z_index(44)
        .child(
          button()
            .test_id("app-menu-backdrop")
            .accessibility_label("关闭应用菜单")
            .absolute()
            .inset_0()
            .on_press(move || open.set(false)),
        )
        .child(
          view()
            .test_id("app-menu-popover")
            .absolute()
            .top(px(38.0))
            .left(px(8.0))
            .z_index(45)
            .flex_row()
            .items_start()
            .gap(px(4.0))
            .child(
              // Left top-level category rail (文件 / 视图 / 历史 / 帮助)
              view()
                .test_id("app-menu-categories")
                .min_w(px(120.0))
                .p(px(4.0))
                .rounded(px(9.0))
                .border_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .bg(move || palette(dark_mode.get()).surface_overlay)
                .shadow_md()
                .child(category_row(
                  "app-menu-group-file",
                  "文件",
                  GROUP_FILE,
                  active_group,
                  dark_mode,
                ))
                .child(category_row(
                  "app-menu-group-view",
                  "视图",
                  GROUP_VIEW,
                  active_group,
                  dark_mode,
                ))
                .child(category_row(
                  "app-menu-group-history",
                  "历史",
                  GROUP_HISTORY,
                  active_group,
                  dark_mode,
                ))
                .child(category_row(
                  "app-menu-group-help",
                  "帮助",
                  GROUP_HELP,
                  active_group,
                  dark_mode,
                )),
            )
            .child(
              // Right cascading submenu panel
              view()
                .test_id("app-menu-submenu")
                .min_w(px(210.0))
                .p(px(4.0))
                .rounded(px(9.0))
                .border_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .bg(move || palette(dark_mode.get()).surface_overlay)
                .shadow_md()
                .child(when(move || active_group.get() == GROUP_FILE, {
                  let cb = cb_panel.clone();
                  move || file_submenu(open, dark_mode, cb.clone())
                }))
                .child(when(move || active_group.get() == GROUP_VIEW, {
                  let cb = cb_panel.clone();
                  move || view_submenu(open, dark_mode, cb.clone())
                }))
                .child(when(move || active_group.get() == GROUP_HISTORY, {
                  let cb = cb_panel.clone();
                  move || history_submenu(open, dark_mode, cb.clone())
                }))
                .child(when(move || active_group.get() == GROUP_HELP, {
                  let cb = cb_panel.clone();
                  move || help_submenu(open, dark_mode, cb.clone())
                })),
            ),
        )
    }
  })
}

fn category_row(
  tid: &'static str,
  label: &'static str,
  group: u8,
  active_group: Signal<u8>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let transparent = rgba(0x0000_0000);
  view()
    .w_full()
    .on_mouse_enter(move |_: &MouseEvent| active_group.set(group))
    .child(
      button()
        .test_id(tid)
        .accessibility_label(label)
        .w_full()
        .min_w(px(110.0))
        .h(px(34.0))
        .px(px(10.0))
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .bg(move || {
          if active_group.get() == group {
            palette(dark_mode.get()).surface_hover
          } else {
            transparent
          }
        })
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || active_group.set(group))
        .child(
          text(label)
            .text_size(px(13.5))
            .text_color(move || palette(dark_mode.get()).text_primary),
        )
        .child(dynamic(move || {
          mica_icons::chevron_right(palette(dark_mode.get()).text_faint_hex)
        })),
    )
}

fn file_submenu(
  open: Signal<bool>,
  dark_mode: Signal<bool>,
  cb: AppMenuCallbacks,
) -> impl IntoElement {
  view()
    .w_full()
    .child(menu_action_row(
      "app-menu-new-page",
      "新建页面",
      Some(SHORTCUT_NEW_PAGE),
      None,
      open,
      dark_mode,
      cb.on_new_page,
    ))
    .child(menu_action_row(
      "app-menu-new-tab",
      "新建标签页",
      Some(SHORTCUT_NEW_TAB),
      None,
      open,
      dark_mode,
      cb.on_new_tab,
    ))
    .child(menu_action_row(
      "app-menu-close-tab",
      "关闭当前标签页",
      Some(SHORTCUT_CLOSE_TAB),
      Some(cb.can_close_tab),
      open,
      dark_mode,
      cb.on_close_tab,
    ))
    .child(
      view()
        .w_full()
        .h(px(1.0))
        .my(px(4.0))
        .bg(move || palette(dark_mode.get()).border_subtle),
    )
    .child(menu_action_row(
      "app-menu-settings",
      "设置",
      Some(SHORTCUT_SETTINGS),
      None,
      open,
      dark_mode,
      cb.on_open_settings,
    ))
}

fn view_submenu(
  open: Signal<bool>,
  dark_mode: Signal<bool>,
  cb: AppMenuCallbacks,
) -> impl IntoElement {
  let sidebar_open = cb.sidebar_open;
  let right_panel_open = cb.right_panel_open;

  view()
    .w_full()
    .child(menu_check_row(
      "app-menu-toggle-sidebar",
      "切换左侧栏显示",
      move || {
        if sidebar_open.get() {
          "收起左侧栏"
        } else {
          "展开左侧栏"
        }
      },
      sidebar_open,
      open,
      dark_mode,
      cb.on_toggle_sidebar,
    ))
    .child(menu_check_row(
      "app-menu-toggle-right-panel",
      "切换右侧栏显示",
      move || {
        if right_panel_open.get() {
          "收起右侧栏"
        } else {
          "展开右侧栏"
        }
      },
      right_panel_open,
      open,
      dark_mode,
      cb.on_toggle_right_panel,
    ))
}

fn history_submenu(
  open: Signal<bool>,
  dark_mode: Signal<bool>,
  cb: AppMenuCallbacks,
) -> impl IntoElement {
  view()
    .w_full()
    .child(menu_action_row(
      "app-menu-go-back",
      "后退",
      None,
      Some(cb.can_go_back),
      open,
      dark_mode,
      cb.on_go_back,
    ))
    .child(menu_action_row(
      "app-menu-go-forward",
      "前进",
      None,
      Some(cb.can_go_forward),
      open,
      dark_mode,
      cb.on_go_forward,
    ))
}

fn help_submenu(
  open: Signal<bool>,
  dark_mode: Signal<bool>,
  cb: AppMenuCallbacks,
) -> impl IntoElement {
  view()
    .w_full()
    .child(menu_action_row(
      "app-menu-about",
      "关于 Mica",
      None,
      None,
      open,
      dark_mode,
      cb.on_open_about,
    ))
    .child(menu_action_row(
      "app-menu-check-update",
      "检查更新",
      None,
      None,
      open,
      dark_mode,
      cb.on_check_update,
    ))
}

fn menu_action_row(
  tid: &'static str,
  label: &'static str,
  shortcut: Option<&'static str>,
  enabled: Option<Signal<bool>>,
  open: Signal<bool>,
  dark_mode: Signal<bool>,
  action: Rc<dyn Fn()>,
) -> impl IntoElement {
  let is_disabled = move || enabled.is_some_and(|sig| !sig.get());
  let transparent = rgba(0x0000_0000);

  let mut btn = button()
    .test_id(tid)
    .accessibility_label(label)
    .w_full()
    .min_w(px(202.0))
    .h(px(34.0))
    .px(px(10.0))
    .rounded(px(6.0))
    .flex_row()
    .items_center()
    .justify_between()
    .gap_4()
    .disabled(is_disabled)
    .bg(transparent)
    .hover(move |s| {
      if enabled.is_some_and(|sig| !sig.get_untracked()) {
        s.bg(transparent)
      } else {
        s.bg(palette(dark_mode.get_untracked()).surface_hover)
      }
    })
    .on_press(move || {
      if enabled.is_some_and(|sig| !sig.get_untracked()) {
        return;
      }
      open.set(false);
      action();
    })
    .child(
      text(label)
        .text_size(px(13.5))
        .text_color(move || {
          let p = palette(dark_mode.get());
          if is_disabled() {
            p.text_ghost
          } else {
            p.text_primary
          }
        }),
    );

  if let Some(sc) = shortcut {
    btn = btn.child(
      text(sc)
        .text_size(px(11.5))
        .text_color(move || {
          let p = palette(dark_mode.get());
          if is_disabled() {
            p.text_ghost
          } else {
            p.text_faint
          }
        }),
    );
  }
  btn
}

fn menu_check_row(
  tid: &'static str,
  a11y_label: &'static str,
  label: impl Fn() -> &'static str + Copy + 'static,
  checked: Signal<bool>,
  open: Signal<bool>,
  dark_mode: Signal<bool>,
  action: Rc<dyn Fn()>,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(a11y_label)
    .w_full()
    .min_w(px(202.0))
    .h(px(34.0))
    .px(px(10.0))
    .rounded(px(6.0))
    .flex_row()
    .items_center()
    .gap(px(8.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(move || {
      open.set(false);
      action();
    })
    .child(
      view()
        .w(px(18.0))
        .h(px(18.0))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .child(when(move || checked.get(), move || {
          dynamic(move || mica_icons::check_icon(palette(dark_mode.get()).text_primary_hex))
        })),
    )
    .child(
      text(label)
        .text_size(px(13.5))
        .text_color(move || palette(dark_mode.get()).text_primary),
    )
}

/// Props for the centered "关于 Mica" modal dialog (`about_dialog.dart` parity).
#[derive(Clone)]
pub struct AboutModalProps {
  pub open: Signal<bool>,
  pub dark_mode: Signal<bool>,
  pub app_version: &'static str,
  pub update_status: Signal<String>,
  pub available_update: Signal<Option<crate::updater::UpdateInfo>>,
  pub update_busy: Signal<bool>,
  pub on_check_update: Rc<dyn Fn()>,
  pub on_install_update: Rc<dyn Fn()>,
}

/// Centered "关于 Mica" modal dialog matching `about_dialog.dart`.
pub fn about_modal_view(props: AboutModalProps) -> impl IntoElement {
  let AboutModalProps {
    open,
    dark_mode,
    app_version,
    update_status,
    available_update,
    update_busy,
    on_check_update,
    on_install_update,
  } = props;
  when(move || open.get(), {
    let on_check_update = on_check_update.clone();
    let on_install_update = on_install_update.clone();
    move || {
      let check_cb = on_check_update.clone();
      let install_cb = on_install_update.clone();

      view()
        .test_id("about-modal-backdrop")
        .absolute()
        .inset_0()
        .z_index(60)
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("about-backdrop-dismiss")
            .accessibility_label("关闭关于弹窗")
            .absolute()
            .inset_0()
            .on_press(move || open.set(false)),
        )
        .child(
          view()
            .test_id("about-modal")
            .w(px(420.0))
            .rounded(px(12.0))
            .p(px(24.0))
            .gap(px(16.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .shadow_lg()
            .z_index(61)
            .child(
              // Brand header: 40px MicaLogo + "Mica" + "v{app_version}"
              view()
                .flex_row()
                .items_center()
                .gap(px(14.0))
                .child(mica_logo_sized(40.0))
                .child(
                  view()
                    .gap(px(2.0))
                    .child(
                      text("Mica")
                        .text_size(px(20.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || palette(dark_mode.get()).text_primary),
                    )
                    .child(
                      text(format!("v{app_version}"))
                        .text_size(px(12.5))
                        .text_color(move || palette(dark_mode.get()).text_muted),
                    ),
                ),
            )
            .child(
              text("云优先的协作式 Markdown 工作区。")
                .text_size(px(13.5))
                .text_color(move || palette(dark_mode.get()).text_muted),
            )
            .child(
              // Source code link button
              view().flex_row().items_center().child(
                button()
                  .test_id("about-source-code-btn")
                  .accessibility_label("打开 Mica 源代码仓库")
                  .h(px(32.0))
                  .px(px(10.0))
                  .rounded(px(6.0))
                  .flex_row()
                  .items_center()
                  .gap(px(8.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || {
                    drop(open_url(GITHUB_REPO_URL));
                  })
                  .child(dynamic(move || {
                    mica_icons::code_icon(palette(dark_mode.get()).accent_hex)
                  }))
                  .child(
                    text("源代码（MIT / Apache-2.0）")
                      .text_size(px(13.0))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(move || palette(dark_mode.get()).accent),
                  ),
              ),
            )
            .child(
              // Update check card
              view()
                .p(px(12.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .bg(move || palette(dark_mode.get()).surface_raised)
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                  view()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                      text(move || {
                        let status = update_status.get();
                        if status.trim().is_empty() {
                          format!("当前版本 v{app_version}")
                        } else {
                          status
                        }
                      })
                      .text_size(px(12.5))
                      .text_color(move || palette(dark_mode.get()).text_muted),
                    ),
                )
                .child(
                  view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(when(
                      move || available_update.get().is_some() && crate::updater::update_supported(),
                      {
                        let install_cb = install_cb.clone();
                        move || {
                          let install_cb = install_cb.clone();
                          button()
                            .test_id("about-install-update-btn")
                            .accessibility_label("下载并安装更新")
                            .h(px(30.0))
                            .px(px(12.0))
                            .rounded(px(6.0))
                            .bg(move || palette(dark_mode.get()).accent)
                            .disabled(move || update_busy.get())
                            .on_press(move || install_cb())
                            .child(
                              text(move || {
                                if update_busy.get() {
                                  "安装中…".to_string()
                                } else if let Some(info) = available_update.get() {
                                  format!("安装 v{}", info.version)
                                } else {
                                  "立即安装".to_string()
                                }
                              })
                              .text_size(px(12.5))
                              .font_weight(FontWeight::MEDIUM)
                              .text_color(nsh_core::color::white()),
                            )
                        }
                      },
                    ))
                    .child(
                      button()
                        .test_id("about-check-update-btn")
                        .accessibility_label("检查更新")
                        .h(px(30.0))
                        .px(px(12.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(move || palette(dark_mode.get()).border_normal)
                        .bg(move || palette(dark_mode.get()).surface_overlay)
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                        .disabled(move || update_busy.get())
                        .on_press(move || check_cb())
                        .child(
                          text(move || {
                            if update_busy.get() { "检查中…" } else { "检查更新" }
                          })
                          .text_size(px(12.5))
                          .font_weight(FontWeight::MEDIUM)
                          .text_color(move || palette(dark_mode.get()).text_primary),
                        ),
                    ),
                ),
            )
            .child(
              // Footer close button
              view().flex_row().justify_end().child(
                button()
                  .test_id("about-close-btn")
                  .accessibility_label("关闭")
                  .h(px(34.0))
                  .px(px(16.0))
                  .rounded(px(6.0))
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).border_normal)
                  .bg(move || palette(dark_mode.get()).surface_raised)
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || open.set(false))
                  .child(
                    text("关闭")
                      .text_size(px(13.0))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  ),
              ),
            ),
        )
    }
  })
}

/// Reactive state and callbacks for the bottom-left grouped-card Account Menu popover.
#[derive(Clone)]
pub struct AccountMenuState {
  pub allow_local: bool,
  pub is_local: Signal<bool>,
  pub is_signed_in: Signal<bool>,
  pub show_cloud_signin: Signal<bool>,
  pub display_name: Signal<String>,
  pub email: Signal<String>,
  pub active_origin: Signal<String>,
  pub signed_in_origins: Signal<Vec<(String, String)>>,
  pub app_version: &'static str,
  pub update_status: Signal<String>,
  pub on_open_settings: Rc<dyn Fn()>,
  pub on_open_usage_stats: Option<Rc<dyn Fn()>>,
  pub on_enter_local: Rc<dyn Fn()>,
  pub on_enter_signed_in: Rc<dyn Fn(String)>,
  pub on_switch_world: Rc<dyn Fn()>,
  pub on_check_update: Rc<dyn Fn()>,
  pub on_sign_out: Rc<dyn Fn()>,
  pub on_sign_in: Rc<dyn Fn()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionAction {
  SignOut,
  SignInCloud,
}

fn account_avatar_initial(is_local: bool, display_name: &str, email: &str) -> String {
  if is_local {
    return "本".to_string();
  }
  let source = if !display_name.trim().is_empty() {
    display_name.trim()
  } else {
    email.trim()
  };
  source
    .chars()
    .next()
    .map(|c| c.to_uppercase().to_string())
    .unwrap_or_else(|| "M".to_string())
}

fn account_identity_copy(
  is_local: bool,
  display_name: &str,
  email: &str,
) -> (String, Option<String>) {
  if is_local {
    ("本地工作区".to_string(), Some("这台设备".to_string()))
  } else {
    let raw_name = display_name.trim();
    let raw_email = email.trim();
    let clean_name = if !raw_name.is_empty() && !raw_name.eq_ignore_ascii_case(raw_email) {
      raw_name.to_string()
    } else if let Some((local_part, domain)) = raw_email.split_once('@')
      && !local_part.trim().is_empty()
      && domain.contains('.')
    {
      local_part.trim().to_string()
    } else if !raw_name.is_empty() {
      raw_name.to_string()
    } else {
      "未登录".to_string()
    };
    let sub = if raw_email.is_empty() || raw_email.eq_ignore_ascii_case(&clean_name) {
      None
    } else {
      Some(raw_email.to_string())
    };
    (clean_name, sub)
  }
}

fn account_session_action(
  is_local: bool,
  is_signed_in: bool,
  show_cloud_signin: bool,
) -> Option<SessionAction> {
  if is_signed_in {
    Some(SessionAction::SignOut)
  } else if !is_local || show_cloud_signin {
    Some(SessionAction::SignInCloud)
  } else {
    None
  }
}

/// Grouped-card Account Menu popover anchored at `bottom: 50px, left: 10px`
/// matching `main:clients/mica_flutter/lib/ui/account_menu.dart`.
pub fn account_menu_popover(
  open: Signal<bool>,
  dark_mode: Signal<bool>,
  state: AccountMenuState,
) -> impl IntoElement {
  when(move || open.get(), {
    let state = state.clone();
    move || {
      let st = state.clone();

      let is_local = st.is_local;
      let is_signed_in = st.is_signed_in;
      let show_cloud_signin = st.show_cloud_signin;
      let display_name = st.display_name;
      let email = st.email;
      let active_origin = st.active_origin;
      let signed_in_origins = st.signed_in_origins;
      let app_version = st.app_version;
      let update_status = st.update_status;
      let on_open_settings = st.on_open_settings.clone();
      let on_open_usage_stats = st.on_open_usage_stats.clone();
      let on_enter_local = st.on_enter_local.clone();
      let on_enter_signed_in = st.on_enter_signed_in.clone();
      let on_switch_world = st.on_switch_world.clone();
      let on_check_update = st.on_check_update.clone();
      let on_sign_out = st.on_sign_out.clone();
      let on_sign_in = st.on_sign_in.clone();

      view()
        .test_id("account-menu-root")
        .absolute()
        .inset_0()
        .z_index(44)
        .child(
          button()
            .test_id("account-menu-backdrop")
            .accessibility_label("关闭账号菜单")
            .absolute()
            .inset_0()
            .on_press(move || open.set(false)),
        )
        .child(
          // Outer sunken container: rounded 14px, padding 5px, subtle border, 6px card gap
          view()
            .test_id("account-menu-popover")
            .absolute()
            .bottom(px(50.0))
            .left(px(10.0))
            .z_index(45)
            .p(px(5.0))
            .gap(px(6.0))
            .rounded(px(14.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .bg(move || palette(dark_mode.get()).surface_sunken)
            .shadow_md()
            .child(
              // Card 1: Identity (40x40 avatar + display name + subtitle)
              account_card(dark_mode).child(
                view()
                  .test_id("account-identity-card")
                  .px(px(8.0))
                  .py(px(8.0))
                  .flex_row()
                  .items_center()
                  .gap(px(12.0))
                  .child(
                    view()
                      .w(px(40.0))
                      .h(px(40.0))
                      .rounded_full()
                      .flex_shrink_0()
                      .items_center()
                      .justify_center()
                      .bg(move || palette(dark_mode.get()).surface_hover)
                      .child(
                        text(move || {
                          account_avatar_initial(
                            is_local.get(),
                            &display_name.get(),
                            &email.get(),
                          )
                        })
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || palette(dark_mode.get()).text_primary),
                      ),
                  )
                  .child(
                    view()
                      .flex_1()
                      .min_w(px(0.0))
                      .gap(px(2.0))
                      .child(
                        text(move || {
                          account_identity_copy(
                            is_local.get(),
                            &display_name.get(),
                            &email.get(),
                          )
                          .0
                        })
                        .truncate()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || palette(dark_mode.get()).text_primary),
                      )
                      .child(when(
                        move || {
                          account_identity_copy(
                            is_local.get(),
                            &display_name.get(),
                            &email.get(),
                          )
                          .1
                          .is_some()
                        },
                        move || {
                          text(move || {
                            account_identity_copy(
                              is_local.get(),
                              &display_name.get(),
                              &email.get(),
                            )
                            .1
                            .unwrap_or_default()
                          })
                          .truncate()
                          .text_size(px(12.0))
                          .text_color(move || palette(dark_mode.get()).text_muted)
                        },
                      )),
                  ),
              ),
            )
            .child(when(
              {
                let allow_local = st.allow_local;
                move || allow_local
              },
              {
                let on_enter_local = on_enter_local.clone();
                let on_enter_signed_in = on_enter_signed_in.clone();
                move || {
                  let on_enter_local = on_enter_local.clone();
                  let on_enter_signed_in = on_enter_signed_in.clone();
                  account_card(dark_mode)
                    .test_id("account-quick-switch-card")
                    .child(
                      view()
                        .px(px(10.0))
                        .pt(px(5.0))
                        .pb(px(3.0))
                        .child(
                          text("切换环境")
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || palette(dark_mode.get()).text_faint),
                        ),
                    )
                    .child(
                      button()
                        .test_id("account-quick-local-btn")
                        .accessibility_label("切换到本地离线库")
                        .w_full()
                        .h(px(34.0))
                        .px(px(10.0))
                        .rounded(px(7.0))
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(8.0))
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                        .on_press({
                          let on_enter_local = on_enter_local.clone();
                          move || {
                            open.set(false);
                            if active_origin.get_untracked() != crate::world_switcher::LOCAL_ORIGIN {
                              on_enter_local();
                            }
                          }
                        })
                        .child(
                          view()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex_row()
                            .items_center()
                            .gap(px(9.0))
                            .child(dynamic(move || {
                              let p = palette(dark_mode.get());
                              let c = if active_origin.get() == crate::world_switcher::LOCAL_ORIGIN {
                                p.accent_hex
                              } else {
                                p.text_muted_hex
                              };
                              computer_small_icon(c)
                            }))
                            .child(
                              text("本地离线库")
                                .truncate()
                                .text_size(px(13.0))
                                .text_color(move || palette(dark_mode.get()).text_primary),
                            ),
                        )
                        .child(when(
                          move || active_origin.get() == crate::world_switcher::LOCAL_ORIGIN,
                          move || {
                            text("✓")
                              .text_size(px(12.5))
                              .font_weight(FontWeight::SEMIBOLD)
                              .text_color(move || palette(dark_mode.get()).accent)
                          },
                        )),
                    )
                    .child(dynamic({
                      let on_enter_signed_in = on_enter_signed_in.clone();
                      move || {
                        let creds = signed_in_origins.get();
                        let cur = active_origin.get();
                        let mut list = view().w_full().gap(px(1.0));
                        for (idx, (origin, _email)) in creds.into_iter().enumerate() {
                          let host = crate::world_switcher::server_label(&origin);
                          let is_cur = cur == origin;
                          let on_enter = on_enter_signed_in.clone();
                          let origin_click = origin.clone();
                          list = list.child(
                            button()
                              .test_id(&format!("account-quick-server-{idx}"))
                              .accessibility_label(format!("切换到服务器 {host}"))
                              .w_full()
                              .h(px(34.0))
                              .px(px(10.0))
                              .rounded(px(7.0))
                              .flex_row()
                              .items_center()
                              .justify_between()
                              .gap(px(8.0))
                              .hover(move |s| {
                                s.bg(palette(dark_mode.get_untracked()).surface_hover)
                              })
                              .on_press(move || {
                                open.set(false);
                                if active_origin.get_untracked() != origin_click {
                                  on_enter(origin_click.clone());
                                }
                              })
                              .child(
                                view()
                                  .flex_1()
                                  .min_w(px(0.0))
                                  .flex_row()
                                  .items_center()
                                  .gap(px(9.0))
                                  .child(dynamic(move || {
                                    let p = palette(dark_mode.get());
                                    let c = if is_cur {
                                      p.accent_hex
                                    } else {
                                      p.text_muted_hex
                                    };
                                    mica_icons::cloud(c)
                                  }))
                                  .child(
                                    text(host)
                                      .truncate()
                                      .text_size(px(13.0))
                                      .text_color(move || palette(dark_mode.get()).text_primary),
                                  ),
                              )
                              .child(when(
                                move || is_cur,
                                move || {
                                  text("✓")
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || palette(dark_mode.get()).accent)
                                },
                              )),
                          );
                        }
                        list
                      }
                    }))
                }
              },
            ))
            .child(
              // Card 2: Actions (38px rows)
              account_card(dark_mode)
                .child(account_action_row(
                  "account-menu-settings-btn",
                  "设置",
                  move || SHORTCUT_SETTINGS.to_string(),
                  mica_icons::settings_gear,
                  open,
                  dark_mode,
                  on_open_settings,
                ))
                .child(when(
                  {
                    let has_stats = on_open_usage_stats.is_some();
                    move || has_stats
                  },
                  {
                    let on_stats = on_open_usage_stats.clone();
                    move || {
                      let cb = on_stats.clone().unwrap_or_else(|| Rc::new(|| {}));
                      account_action_row(
                        "account-menu-usage-stats-btn",
                        "使用统计",
                        String::new,
                        mica_icons::bar_chart_icon,
                        open,
                        dark_mode,
                        cb,
                      )
                    }
                  },
                ))
                .child(when(
                  {
                    let allow_local = st.allow_local;
                    move || allow_local
                  },
                  {
                    let on_switch_world = on_switch_world.clone();
                    move || {
                      account_action_row(
                        "account-switch-world-btn",
                        "切换服务器 / 本地库…",
                        String::new,
                        swap_horiz_icon,
                        open,
                        dark_mode,
                        on_switch_world.clone(),
                      )
                    }
                  },
                ))
                .child(account_action_row(
                  "account-menu-update-btn",
                  "检查更新",
                  move || {
                    let st = update_status.get();
                    if st.trim().is_empty() {
                      format!("v{app_version}")
                    } else {
                      st
                    }
                  },
                  update_up_icon,
                  open,
                  dark_mode,
                  on_check_update,
                )),
            )
            .child(when(
              move || {
                account_session_action(
                  is_local.get(),
                  is_signed_in.get(),
                  show_cloud_signin.get(),
                )
                .is_some()
              },
              {
                let on_sign_out = on_sign_out.clone();
                let on_sign_in = on_sign_in.clone();
                move || {
                  let sign_out = on_sign_out.clone();
                  let sign_in = on_sign_in.clone();
                  account_card(dark_mode)
                    .child(when(
                      move || {
                        account_session_action(
                          is_local.get(),
                          is_signed_in.get(),
                          show_cloud_signin.get(),
                        ) == Some(SessionAction::SignOut)
                      },
                      {
                        let sign_out = sign_out.clone();
                        move || {
                          account_action_row(
                            "account-menu-signout-btn",
                            "退出登录",
                            String::new,
                            logout_icon,
                            open,
                            dark_mode,
                            sign_out.clone(),
                          )
                        }
                      },
                    ))
                    .child(when(
                      move || {
                        account_session_action(
                          is_local.get(),
                          is_signed_in.get(),
                          show_cloud_signin.get(),
                        ) == Some(SessionAction::SignInCloud)
                      },
                      {
                        let sign_in = sign_in.clone();
                        move || {
                          account_action_row(
                            "account-menu-signin-btn",
                            "登录云端…",
                            String::new,
                            login_icon,
                            open,
                            dark_mode,
                            sign_in.clone(),
                          )
                        }
                      },
                    ))
                }
              },
            )),
        )
    }
  })
}

fn account_card(dark_mode: Signal<bool>) -> View {
  view()
    .w(px(272.0))
    .p(px(4.0))
    .rounded(px(10.0))
    .bg(move || palette(dark_mode.get()).surface_overlay)
}

fn account_action_row<I, T>(
  tid: &'static str,
  label: &'static str,
  trailing: T,
  icon_fn: fn(&str) -> I,
  open: Signal<bool>,
  dark_mode: Signal<bool>,
  action: Rc<dyn Fn()>,
) -> impl IntoElement
where
  I: IntoElement + 'static,
  T: Fn() -> String + Copy + 'static,
{
  button()
    .test_id(tid)
    .accessibility_label(label)
    .w_full()
    .h(px(38.0))
    .px(px(10.0))
    .rounded(px(7.0))
    .flex_row()
    .items_center()
    .justify_between()
    .gap(px(8.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(move || {
      open.set(false);
      action();
    })
    .child(
      view()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .min_w(px(0.0))
        .child(dynamic(move || {
          icon_fn(palette(dark_mode.get()).text_muted_hex)
        }))
        .child(
          text(label)
            .text_size(px(13.5))
            .text_color(move || palette(dark_mode.get()).text_primary),
        ),
    )
    .child(when(
      move || !trailing().is_empty(),
      move || {
        text(trailing)
          .text_size(px(11.5))
          .text_color(move || palette(dark_mode.get()).text_faint)
      },
    ))
}

fn mica_logo_sized(size_px: f32) -> Svg {
  svg(ImageSrc::svg_markup(MICA_LOGO_SVG))
    .w(px(size_px))
    .h(px(size_px))
}

fn stroked_icon(size_px: f32, color: &str, inner: &str) -> Svg {
  let markup = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{inner}</svg>"##
  );
  svg(ImageSrc::svg_markup(&markup))
    .w(px(size_px))
    .h(px(size_px))
}

fn swap_horiz_icon(color: &str) -> Svg {
  stroked_icon(
    18.0,
    color,
    r##"<path d="M5.5 7.5H15.5M12.5 4.5L15.5 7.5L12.5 10.5M14.5 12.5H4.5M7.5 9.5L4.5 12.5L7.5 15.5"/>"##,
  )
}

fn computer_small_icon(color: &str) -> Svg {
  stroked_icon(
    16.0,
    color,
    r##"<rect x="3" y="4" width="14" height="9.5" rx="1.6"/><path d="M7.5 16.5H12.5M10 13.5V16.5"/>"##,
  )
}

fn update_up_icon(color: &str) -> Svg {
  stroked_icon(
    18.0,
    color,
    r##"<circle cx="10" cy="10" r="6.8"/><path d="M10 13.2V6.8M7.2 9.4L10 6.6L12.8 9.4"/>"##,
  )
}

fn logout_icon(color: &str) -> Svg {
  stroked_icon(
    18.0,
    color,
    r##"<path d="M8 4H5.2A1.5 1.5 0 0 0 3.7 5.5V14.5A1.5 1.5 0 0 0 5.2 16H8M12.5 7L15.8 10L12.5 13M8.5 10H15.5"/>"##,
  )
}

fn login_icon(color: &str) -> Svg {
  stroked_icon(
    18.0,
    color,
    r##"<path d="M12 4H14.8A1.5 1.5 0 0 1 16.3 5.5V14.5A1.5 1.5 0 0 1 14.8 16H12M8.5 7L11.8 10L8.5 13M4 10H11.5"/>"##,
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn local_identity_uses_local_copy_and_initial() {
    assert_eq!(account_avatar_initial(true, "Alice", "a@b.com"), "本");
    assert_eq!(
      account_identity_copy(true, "Alice", "a@b.com"),
      ("本地工作区".to_string(), Some("这台设备".to_string()))
    );
  }

  #[test]
  fn cloud_identity_falls_back_cleanly() {
    assert_eq!(account_avatar_initial(false, "willmica", "w@example.com"), "W");
    assert_eq!(account_avatar_initial(false, "", "alice@example.com"), "A");
    assert_eq!(account_avatar_initial(false, "", ""), "M");
    assert_eq!(
      account_identity_copy(false, "willmica", "w@example.com"),
      ("willmica".to_string(), Some("w@example.com".to_string()))
    );
    assert_eq!(
      account_identity_copy(false, "willzhmic@outlook.com", "willzhmic@outlook.com"),
      ("willzhmic".to_string(), Some("willzhmic@outlook.com".to_string()))
    );
    assert_eq!(
      account_identity_copy(false, "", ""),
      ("未登录".to_string(), None)
    );
  }

  #[test]
  fn session_card_visibility_matches_account_menu_rules() {
    assert_eq!(
      account_session_action(false, true, false),
      Some(SessionAction::SignOut)
    );
    assert_eq!(
      account_session_action(false, false, false),
      Some(SessionAction::SignInCloud)
    );
    assert_eq!(account_session_action(true, false, false), None);
    assert_eq!(
      account_session_action(true, false, true),
      Some(SessionAction::SignInCloud)
    );
  }
}
