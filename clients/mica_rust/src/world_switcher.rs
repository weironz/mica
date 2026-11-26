//! Centered World / Server Switcher modal (Desktop) and single-card Cloud
//! Sign-In page (Web).
//!
//! Layout strategy:
//! - **Desktop (`allow_local == true`)**: Never takes over the viewport with a
//!   full-screen marketing poster. Opens as a centered 440px modal over a
//!   dimmed workspace backdrop with two clean views:
//!   - **View A (`world-env-list`)**: Flat unified list containing the on-device
//!     local vault (`本地离线库`) and every configured self-hosted server, plus
//!     inline `+ 添加自托管服务器` creation and per-server removal.
//!   - **View B (`world-auth-form`)**: Single-server Sign-In / Register form
//!     with a `←` back button to View A, a compact server health chip, and
//!     bottom text links instead of stacked segmented tabs.
//! - **Web (`allow_local == false`)**: Clean centered 396px single-card sign-in
//!   page on the warm paper backdrop — no dark hero poster, no local mode, and
//!   no multi-server dropdown.

use std::rc::Rc;

use nsh_base::{button, input};
use nsh_core::{
  SubmitBehavior,
  color::{rgb, white},
  element::{ImageSrc, Svg, svg},
  geometry::{px, relative},
  prelude::*,
};

use crate::{mica_icons, mica_theme::palette};

const MICA_LOGO_SVG: &str = include_str!("../../../assets/mica-logo.svg");

/// Origin string representing the on-device local workspace.
pub const LOCAL_ORIGIN: &str = "local";

/// Width of the centered environment switcher modal on Desktop.
pub const WORLD_MODAL_WIDTH: f32 = 440.0;

/// Width of the centered sign-in card on Web cloud-only builds.
pub const WEB_SIGN_IN_CARD_WIDTH: f32 = 396.0;

const AUTH_MODE_LOGIN: u8 = 0;
const AUTH_MODE_REGISTER: u8 = 1;

/// Reactive state and callbacks driving the World Switcher modal / Web Sign-In.
#[derive(Clone)]
pub struct WorldSwitcherState {
  pub open: Signal<bool>,
  pub can_close: Signal<bool>,
  /// Whether the on-device local workspace mode and multi-server switcher are
  /// available (`true` on Desktop, `false` on Web cloud-only same-origin builds).
  pub allow_local: bool,
  /// When `true` on Desktop open, jump directly into View B (single-server auth
  /// form) for `selected_cloud_origin` instead of View A (environment list).
  pub initial_auth_view: Signal<bool>,
  pub dark_mode: Signal<bool>,
  pub app_version: &'static str,
  pub active_origin: Signal<String>,
  pub servers: Signal<Vec<String>>,
  pub signed_in_origins: Signal<Vec<(String, String)>>,
  pub selected_cloud_origin: Signal<String>,
  pub health_status: Signal<String>,
  pub busy: Signal<bool>,
  pub error_msg: Signal<String>,
  pub info_msg: Signal<String>,
  pub on_close: Rc<dyn Fn()>,
  pub on_enter_local: Rc<dyn Fn()>,
  pub on_select_server: Rc<dyn Fn(String)>,
  pub on_probe_server: Rc<dyn Fn(String)>,
  pub on_add_server: Rc<dyn Fn(String) -> Result<(), String>>,
  pub on_remove_server: Rc<dyn Fn(String)>,
  pub on_enter_signed_in: Rc<dyn Fn(String)>,
  pub on_login: Rc<dyn Fn(String, String, String)>,
  pub on_register: Rc<dyn Fn(String, String, String, String)>,
  pub on_forgot_password: Rc<dyn Fn(String, String)>,
}

/// Extracts a human-readable host label from a server origin URL.
///
/// `https://mica.example.com:8443/mica` renders as `mica.example.com`;
/// unparseable inputs fall back to the raw string.
pub fn server_label(origin: &str) -> String {
  let trimmed = origin.trim();
  if trimmed.is_empty() {
    return String::new();
  }
  let Some(scheme_end) = trimmed.find("://") else {
    return origin.to_string();
  };
  let scheme = &trimmed[..scheme_end];
  if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
    return origin.to_string();
  }
  let rest = &trimmed[scheme_end + 3..];
  let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
  let authority = &rest[..authority_end];
  let host_port = authority
    .rsplit_once('@')
    .map(|(_, hp)| hp)
    .unwrap_or(authority);

  let host = if let Some(inner) = host_port.strip_prefix('[') {
    let Some(close) = inner.find(']') else {
      return origin.to_string();
    };
    &inner[..close]
  } else {
    host_port
      .split_once(':')
      .map(|(h, _)| h)
      .unwrap_or(host_port)
  };

  if host.is_empty() || host.chars().any(char::is_whitespace) {
    origin.to_string()
  } else {
    host.to_ascii_lowercase()
  }
}

/// Normalizes a cloud server base URL into canonical `scheme://host[:port][path]` form.
///
/// - Requires absolute `http://` or `https://` with a non-empty host.
/// - Lowercases scheme and host.
/// - Drops default ports (`:80` for http, `:443` for https) and trailing slashes.
/// - Rejects malformed URLs with localized Chinese validation copy.
pub fn normalize_server_url(raw: &str) -> Result<String, String> {
  const INVALID_ERR: &str = "请输入完整的服务器地址（含 https://）";

  let trimmed = raw.trim();
  if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
    return Err(INVALID_ERR.to_string());
  }

  let Some(scheme_end) = trimmed.find("://") else {
    return Err(INVALID_ERR.to_string());
  };
  let scheme_raw = &trimmed[..scheme_end];
  let scheme = if scheme_raw.eq_ignore_ascii_case("http") {
    "http"
  } else if scheme_raw.eq_ignore_ascii_case("https") {
    "https"
  } else {
    return Err(INVALID_ERR.to_string());
  };

  let rest = &trimmed[scheme_end + 3..];
  if rest.is_empty() {
    return Err(INVALID_ERR.to_string());
  }
  let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
  let authority = &rest[..authority_end];
  let suffix = &rest[authority_end..];

  // Reject userinfo or query/fragment in base server origins.
  if authority.is_empty()
    || authority.contains('@')
    || suffix.contains('?')
    || suffix.contains('#')
  {
    return Err(INVALID_ERR.to_string());
  }

  let (host_lower, port): (String, Option<u16>) = if let Some(inner) = authority.strip_prefix('[') {
    let Some(close) = inner.find(']') else {
      return Err(INVALID_ERR.to_string());
    };
    let ipv6 = &inner[..close];
    if ipv6.is_empty() {
      return Err(INVALID_ERR.to_string());
    }
    let after = &inner[close + 1..];
    let parsed_port = if after.is_empty() {
      None
    } else if let Some(p_str) = after.strip_prefix(':') {
      let p = p_str
        .parse::<u16>()
        .ok()
        .filter(|&n| n > 0)
        .ok_or_else(|| INVALID_ERR.to_string())?;
      Some(p)
    } else {
      return Err(INVALID_ERR.to_string());
    };
    (format!("[{}]", ipv6.to_ascii_lowercase()), parsed_port)
  } else if let Some((h, p_str)) = authority.split_once(':') {
    if h.is_empty() || p_str.contains(':') {
      return Err(INVALID_ERR.to_string());
    }
    let p = p_str
      .parse::<u16>()
      .ok()
      .filter(|&n| n > 0)
      .ok_or_else(|| INVALID_ERR.to_string())?;
    (h.to_ascii_lowercase(), Some(p))
  } else {
    (authority.to_ascii_lowercase(), None)
  };

  if host_lower.is_empty()
    || host_lower
      .chars()
      .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':')))
  {
    return Err(INVALID_ERR.to_string());
  }

  let is_default_port = matches!((scheme, port), ("http", Some(80)) | ("https", Some(443)));
  let port_part = match port {
    Some(p) if !is_default_port => format!(":{p}"),
    _ => String::new(),
  };

  let path = suffix.trim_end_matches('/');
  Ok(format!("{scheme}://{host_lower}{port_part}{path}"))
}

fn effective_cloud_origin(state: &WorldSwitcherState) -> String {
  let selected = state.selected_cloud_origin.get();
  let servers = state.servers.get();
  if !selected.is_empty() && (servers.is_empty() || servers.contains(&selected)) {
    return selected;
  }
  let active = state.active_origin.get();
  if active != LOCAL_ORIGIN && !active.is_empty() && servers.contains(&active) {
    return active;
  }
  servers.first().cloned().unwrap_or_default()
}

fn effective_cloud_origin_untracked(state: &WorldSwitcherState) -> String {
  let selected = state.selected_cloud_origin.get_untracked();
  let servers = state.servers.get_untracked();
  if !selected.is_empty() && (servers.is_empty() || servers.contains(&selected)) {
    return selected;
  }
  let active = state.active_origin.get_untracked();
  if active != LOCAL_ORIGIN && !active.is_empty() && servers.contains(&active) {
    return active;
  }
  servers.first().cloned().unwrap_or_default()
}

/// Centered World Switcher modal (Desktop) or centered single-card Sign-In page (Web).
pub fn world_switcher_overlay(state: WorldSwitcherState) -> impl IntoElement {
  when(move || state.open.get(), {
    let state = state.clone();
    move || {
      let st = state.clone();
      let allow_local = st.allow_local;

      // Form drafts live per modal session so typing survives health-probe refreshes.
      let auth_mode = signal(AUTH_MODE_LOGIN);
      let email_draft = signal(String::new());
      let display_name_draft = signal(String::new());
      let password_draft = signal(String::new());
      let local_err = signal(String::new());

      if !allow_local {
        web_sign_in_page(
          st,
          auth_mode,
          email_draft,
          display_name_draft,
          password_draft,
          local_err,
        )
        .into_element()
      } else {
        let start_in_auth =
          st.initial_auth_view.get_untracked() || !st.error_msg.get_untracked().is_empty();
        let in_auth_view = signal(start_in_auth);
        let adding_server = signal(false);
        let new_server_url = signal(String::new());
        let add_server_err = signal(String::new());
        let confirm_remove_origin: Signal<Option<String>> = signal(None);

        desktop_switcher_modal(
          st,
          in_auth_view,
          adding_server,
          new_server_url,
          add_server_err,
          confirm_remove_origin,
          auth_mode,
          email_draft,
          display_name_draft,
          password_draft,
          local_err,
        )
        .into_element()
      }
    }
  })
}

/// Web (`allow_local == false`): minimal centered 396px single-card sign-in page.
fn web_sign_in_page(
  state: WorldSwitcherState,
  auth_mode: Signal<u8>,
  email_draft: Signal<String>,
  display_name_draft: Signal<String>,
  password_draft: Signal<String>,
  local_err: Signal<String>,
) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let can_close = state.can_close;
  let on_close = state.on_close.clone();
  let app_version = state.app_version;

  view()
    .test_id("world-switcher-overlay")
    .absolute()
    .inset_0()
    .z_index(80)
    .items_center()
    .justify_center()
    .p(px(20.0))
    .bg(move || palette(dark_mode.get()).surface_raised)
    .child(
      view()
        .test_id("world-switcher-form-pane")
        .relative()
        .w(px(WEB_SIGN_IN_CARD_WIDTH))
        .max_w(relative(0.94))
        .p(px(28.0))
        .gap(px(18.0))
        .rounded(px(16.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .shadow_lg()
        .child(
          // Brand header + optional close button (hidden on unauthenticated Web gate)
          view()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(
              view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .min_w(px(0.0))
                .child(mica_logo_sized(34.0))
                .child(
                  view()
                    .gap(px(2.0))
                    .min_w(px(0.0))
                    .child(
                      text(move || {
                        if auth_mode.get() == AUTH_MODE_REGISTER {
                          "创建 Mica 账号"
                        } else {
                          "登录到 Mica"
                        }
                      })
                      .text_size(px(18.0))
                      .font_weight(FontWeight::BOLD)
                      .text_color(move || palette(dark_mode.get()).text_primary),
                    )
                    .child(
                      text("云端协作知识库")
                        .text_size(px(12.5))
                        .text_color(move || palette(dark_mode.get()).text_muted),
                    ),
                ),
            )
            .child(when(
              move || can_close.get(),
              {
                let on_close = on_close.clone();
                move || close_icon_button(dark_mode, on_close.clone())
              },
            )),
        )
        .child(server_status_chip(state.clone()))
        .child(auth_form(
          state,
          auth_mode,
          email_draft,
          display_name_draft,
          password_draft,
          local_err,
        ))
        .child(
          view()
            .pt(px(4.0))
            .border_t_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .items_center()
            .child(
              text(format!(
                "Mica Web v{app_version} · 安全会话（365 天滑动续期）"
              ))
              .text_size(px(11.0))
              .text_color(move || palette(dark_mode.get()).text_faint),
            ),
        ),
    )
}

/// Desktop (`allow_local == true`): centered 440px modal over dimmed backdrop.
#[allow(clippy::too_many_arguments)]
fn desktop_switcher_modal(
  state: WorldSwitcherState,
  in_auth_view: Signal<bool>,
  adding_server: Signal<bool>,
  new_server_url: Signal<String>,
  add_server_err: Signal<String>,
  confirm_remove_origin: Signal<Option<String>>,
  auth_mode: Signal<u8>,
  email_draft: Signal<String>,
  display_name_draft: Signal<String>,
  password_draft: Signal<String>,
  local_err: Signal<String>,
) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let can_close = state.can_close;
  let on_close = state.on_close.clone();

  view()
    .test_id("world-switcher-overlay")
    .absolute()
    .inset_0()
    .z_index(80)
    .items_center()
    .justify_center()
    .p(px(20.0))
    .bg(move || palette(dark_mode.get()).backdrop)
    .child(
      button()
        .test_id("world-switcher-backdrop")
        .accessibility_label("关闭切换弹窗")
        .absolute()
        .inset_0()
        .on_press({
          let on_close = on_close.clone();
          move || {
            if can_close.get_untracked() {
              on_close();
            }
          }
        }),
    )
    .child(
      view()
        .test_id("world-switcher-modal")
        .relative()
        .z_index(81)
        .w(px(WORLD_MODAL_WIDTH))
        .max_w(relative(0.94))
        .max_h(relative(0.88))
        .rounded(px(14.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .shadow_lg()
        .overflow_hidden()
        .child(dynamic({
          let st = state.clone();
          move || {
            if in_auth_view.get() {
              desktop_auth_view(
                st.clone(),
                in_auth_view,
                auth_mode,
                email_draft,
                display_name_draft,
                password_draft,
                local_err,
              )
              .into_element()
            } else {
              desktop_env_list_view(
                st.clone(),
                in_auth_view,
                adding_server,
                new_server_url,
                add_server_err,
                confirm_remove_origin,
                auth_mode,
                local_err,
              )
              .into_element()
            }
          }
        })),
    )
}

/// View A on Desktop: flat unified list of Local Vault + all configured servers.
#[allow(clippy::too_many_arguments)]
fn desktop_env_list_view(
  state: WorldSwitcherState,
  in_auth_view: Signal<bool>,
  adding_server: Signal<bool>,
  new_server_url: Signal<String>,
  add_server_err: Signal<String>,
  confirm_remove_origin: Signal<Option<String>>,
  auth_mode: Signal<u8>,
  local_err: Signal<String>,
) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let can_close = state.can_close;
  let on_close = state.on_close.clone();
  let app_version = state.app_version;
  let active_origin = state.active_origin;
  let servers = state.servers;
  let signed_in_origins = state.signed_in_origins;
  let error_msg = state.error_msg;
  let info_msg = state.info_msg;
  let busy = state.busy;

  let on_enter_local = state.on_enter_local.clone();
  let on_select_server = state.on_select_server.clone();
  let on_probe_server = state.on_probe_server.clone();
  let on_add_server = state.on_add_server.clone();
  let on_remove_server = state.on_remove_server.clone();
  let on_enter_signed_in = state.on_enter_signed_in.clone();

  view()
    .test_id("world-env-list")
    .w_full()
    .child(
      // Modal header
      view()
        .px(px(20.0))
        .py(px(15.0))
        .border_b_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .child(
          view()
            .flex_row()
            .items_center()
            .gap(px(9.0))
            .child(mica_logo_sized(20.0))
            .child(
              text("切换服务器与本地库")
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || palette(dark_mode.get()).text_primary),
            ),
        )
        .child(when(
          move || can_close.get(),
          {
            let on_close = on_close.clone();
            move || close_icon_button(dark_mode, on_close.clone())
          },
        )),
    )
    .child(
      // Body list
      view()
        .p(px(12.0))
        .gap(px(6.0))
        .child(
          // 1. On-device Local Vault row
          button()
            .test_id("world-enter-local-btn")
            .accessibility_label("本地离线库")
            .w_full()
            .px(px(12.0))
            .py(px(10.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(move || {
              let p = palette(dark_mode.get());
              if active_origin.get() == LOCAL_ORIGIN {
                p.accent
              } else {
                p.border_subtle
              }
            })
            .bg(move || {
              let p = palette(dark_mode.get());
              if active_origin.get() == LOCAL_ORIGIN {
                p.accent_soft
              } else {
                p.surface_overlay
              }
            })
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .on_press({
              let on_enter_local = on_enter_local.clone();
              let on_close = on_close.clone();
              move || {
                if busy.get_untracked() {
                  return;
                }
                if active_origin.get_untracked() == LOCAL_ORIGIN {
                  on_close();
                } else {
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
                .gap(px(11.0))
                .child(
                  view()
                    .w(px(34.0))
                    .h(px(34.0))
                    .rounded(px(8.0))
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .bg(move || {
                      let p = palette(dark_mode.get());
                      if active_origin.get() == LOCAL_ORIGIN {
                        p.accent_soft
                      } else {
                        p.surface_sunken
                      }
                    })
                    .child(dynamic(move || {
                      let p = palette(dark_mode.get());
                      let color = if active_origin.get() == LOCAL_ORIGIN {
                        p.accent_hex
                      } else {
                        p.text_muted_hex
                      };
                      computer_icon(color)
                    })),
                )
                .child(
                  view()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(2.0))
                    .child(
                      text("本地离线库")
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || palette(dark_mode.get()).text_primary),
                    )
                    .child(
                      text("数据保存在这台设备")
                        .truncate()
                        .text_size(px(11.5))
                        .text_color(move || palette(dark_mode.get()).text_muted),
                    ),
                ),
            )
            .child(dynamic(move || {
              let p = palette(dark_mode.get());
              if active_origin.get() == LOCAL_ORIGIN {
                status_badge("当前", p.accent_soft, p.accent)
              } else {
                status_badge("离线可用", p.surface_sunken, p.text_muted)
              }
            })),
        )
        .child(
          // 2. Configured self-hosted servers
          dynamic({
            let on_select_server = on_select_server.clone();
            let on_probe_server = on_probe_server.clone();
            let on_remove_server = on_remove_server.clone();
            let on_enter_signed_in = on_enter_signed_in.clone();
            let on_close = on_close.clone();
            move || {
              let list = servers.get();
              let active = active_origin.get();
              let signed_in = signed_in_origins.get();
              let confirming = confirm_remove_origin.get();

              let mut col = view().w_full().gap(px(6.0));
              for (idx, origin) in list.iter().enumerate() {
                let origin_owned = origin.clone();
                let host = server_label(origin);
                let is_active = active == *origin;
                let signed_email = signed_in
                  .iter()
                  .find(|(o, _)| o == origin)
                  .map(|(_, e)| e.clone());
                let is_signed_in = signed_email.is_some();
                let is_confirming = confirming.as_deref() == Some(origin.as_str());

                if is_confirming {
                  col = col.child(
                    view()
                      .test_id(&format!("world-server-confirm-{idx}"))
                      .w_full()
                      .px(px(12.0))
                      .py(px(10.0))
                      .rounded(px(10.0))
                      .border_1()
                      .border_color(move || palette(dark_mode.get()).danger)
                      .bg(move || palette(dark_mode.get()).danger_wash)
                      .flex_row()
                      .items_center()
                      .justify_between()
                      .gap(px(8.0))
                      .child(
                        text(format!("移除 {host} 及其凭据？"))
                          .truncate()
                          .text_size(px(12.5))
                          .font_weight(FontWeight::MEDIUM)
                          .text_color(move || palette(dark_mode.get()).danger),
                      )
                      .child(
                        view()
                          .flex_row()
                          .items_center()
                          .gap(px(6.0))
                          .flex_shrink_0()
                          .child(
                            button()
                              .test_id(&format!("world-server-remove-cancel-{idx}"))
                              .h(px(26.0))
                              .px(px(8.0))
                              .rounded(px(6.0))
                              .border_1()
                              .border_color(move || palette(dark_mode.get()).border_normal)
                              .bg(move || palette(dark_mode.get()).surface_overlay)
                              .on_press(move || confirm_remove_origin.set(None))
                              .child(
                                text("取消")
                                  .text_size(px(11.5))
                                  .text_color(move || palette(dark_mode.get()).text_primary),
                              ),
                          )
                          .child(
                            button()
                              .test_id(&format!("world-server-remove-ok-{idx}"))
                              .h(px(26.0))
                              .px(px(10.0))
                              .rounded(px(6.0))
                              .bg(move || palette(dark_mode.get()).danger)
                              .on_press({
                                let on_remove_server = on_remove_server.clone();
                                let origin_del = origin_owned.clone();
                                move || {
                                  confirm_remove_origin.set(None);
                                  on_remove_server(origin_del.clone());
                                }
                              })
                              .child(
                                text("移除")
                                  .text_size(px(11.5))
                                  .font_weight(FontWeight::MEDIUM)
                                  .text_color(white()),
                              ),
                          ),
                      ),
                  );
                } else {
                  let subtitle = match &signed_email {
                    Some(email) if !email.trim().is_empty() => email.clone(),
                    Some(_) => "已登录".to_string(),
                    None if is_active => "登录已过期 · 点击重新登录".to_string(),
                    None => "未登录 · 点击登录".to_string(),
                  };

                  let on_select = on_select_server.clone();
                  let on_probe = on_probe_server.clone();
                  let on_enter = on_enter_signed_in.clone();
                  let on_close_row = on_close.clone();
                  let origin_click = origin_owned.clone();
                  let origin_rm = origin_owned.clone();

                  let mut row = view()
                    .test_id(&format!("world-server-row-{idx}"))
                    .w_full()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(move || {
                      let p = palette(dark_mode.get());
                      if is_active { p.accent } else { p.border_subtle }
                    })
                    .bg(move || {
                      let p = palette(dark_mode.get());
                      if is_active {
                        p.accent_soft
                      } else {
                        p.surface_overlay
                      }
                    })
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .pr(px(8.0))
                    .child(
                      button()
                        .test_id(&format!("world-server-enter-{idx}"))
                        .accessibility_label(format!("切换到服务器 {host}"))
                        .flex_1()
                        .min_w(px(0.0))
                        .px(px(12.0))
                        .py(px(10.0))
                        .rounded(px(9.0))
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(10.0))
                        .on_press(move || {
                          if busy.get_untracked() {
                            return;
                          }
                          on_select(origin_click.clone());
                          if is_signed_in {
                            if is_active {
                              on_close_row();
                            } else {
                              on_enter(origin_click.clone());
                            }
                          } else {
                            error_msg.set(String::new());
                            info_msg.set(String::new());
                            local_err.set(String::new());
                            auth_mode.set(AUTH_MODE_LOGIN);
                            on_probe(origin_click.clone());
                            in_auth_view.set(true);
                          }
                        })
                        .child(
                          view()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex_row()
                            .items_center()
                            .gap(px(11.0))
                            .child(
                              view()
                                .w(px(34.0))
                                .h(px(34.0))
                                .rounded(px(8.0))
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .bg(move || {
                                  let p = palette(dark_mode.get());
                                  if is_active {
                                    p.accent_soft
                                  } else {
                                    p.surface_sunken
                                  }
                                })
                                .child(dynamic(move || {
                                  let p = palette(dark_mode.get());
                                  let color = if is_active || is_signed_in {
                                    p.accent_hex
                                  } else {
                                    p.text_muted_hex
                                  };
                                  mica_icons::cloud(color)
                                })),
                            )
                            .child(
                              view()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap(px(2.0))
                                .child(
                                  text(host.clone())
                                    .truncate()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || palette(dark_mode.get()).text_primary),
                                )
                                .child(
                                  view()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(5.0))
                                    .min_w(px(0.0))
                                    .child(
                                      view()
                                        .w(px(6.0))
                                        .h(px(6.0))
                                        .rounded_full()
                                        .flex_shrink_0()
                                        .bg(move || {
                                          let p = palette(dark_mode.get());
                                          if is_signed_in {
                                            rgb(0x22C55E)
                                          } else if is_active {
                                            p.warning
                                          } else {
                                            p.text_faint
                                          }
                                        }),
                                    )
                                    .child(
                                      text(subtitle)
                                        .truncate()
                                        .text_size(px(11.5))
                                        .text_color(move || {
                                          let p = palette(dark_mode.get());
                                          if is_active && !is_signed_in {
                                            p.warning
                                          } else {
                                            p.text_muted
                                          }
                                        }),
                                    ),
                                ),
                            ),
                        )
                        .child(dynamic(move || {
                          let p = palette(dark_mode.get());
                          if is_active && is_signed_in {
                            status_badge("当前", p.accent_soft, p.accent)
                          } else if is_active {
                            status_badge("需重登", p.warning_wash, p.warning)
                          } else if is_signed_in {
                            status_badge("已登录", p.success_wash, p.success)
                          } else {
                            status_badge("登录 →", p.surface_sunken, p.text_muted)
                          }
                        })),
                    );

                  if !is_active {
                    row = row.child(
                      button()
                        .test_id(&format!("world-server-remove-{idx}"))
                        .accessibility_label(format!("移除服务器 {host}"))
                        .w(px(26.0))
                        .h(px(26.0))
                        .rounded(px(6.0))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).danger_wash))
                        .on_press(move || confirm_remove_origin.set(Some(origin_rm.clone())))
                        .child(dynamic(move || {
                          mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                        })),
                    );
                  }

                  col = col.child(row);
                }
              }
              col
            }
          }),
        )
        .child(
          // Error banner on View A (e.g., if entering a signed-in server failed)
          when(
            move || !error_msg.get().trim().is_empty(),
            move || {
              view()
                .test_id("world-list-error")
                .w_full()
                .px(px(11.0))
                .py(px(8.0))
                .rounded(px(8.0))
                .bg(move || palette(dark_mode.get()).danger_wash)
                .flex_row()
                .items_center()
                .gap(px(7.0))
                .child(dynamic(move || {
                  error_circle_icon(palette(dark_mode.get()).danger_hex)
                }))
                .child(
                  text(move || error_msg.get())
                    .flex_1()
                    .text_size(px(12.0))
                    .text_color(move || palette(dark_mode.get()).danger),
                )
            },
          ),
        )
        .child(
          // 3. Inline "+ 添加自托管服务器" section
          dynamic({
            let on_add_server = on_add_server.clone();
            let on_select_server = on_select_server.clone();
            let on_probe_server = on_probe_server.clone();
            move || {
              if !adding_server.get() {
                button()
                  .test_id("world-add-server-btn")
                  .accessibility_label("添加自托管服务器")
                  .w_full()
                  .h(px(38.0))
                  .px(px(12.0))
                  .rounded(px(10.0))
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).border_normal)
                  .bg(move || palette(dark_mode.get()).surface_raised)
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .flex_row()
                  .items_center()
                  .justify_center()
                  .gap(px(6.0))
                  .on_press(move || {
                    adding_server.set(true);
                    new_server_url.set(String::new());
                    add_server_err.set(String::new());
                  })
                  .child(
                    text("+ 添加自托管服务器")
                      .text_size(px(13.0))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(move || palette(dark_mode.get()).text_muted),
                  )
                  .into_element()
              } else {
                let submit_add = {
                  let on_add_server = on_add_server.clone();
                  let on_select_server = on_select_server.clone();
                  let on_probe_server = on_probe_server.clone();
                  Rc::new(move || {
                    let raw = new_server_url.get_untracked();
                    match normalize_server_url(&raw) {
                      Ok(url) => match on_add_server(url.clone()) {
                        Ok(()) => {
                          adding_server.set(false);
                          new_server_url.set(String::new());
                          add_server_err.set(String::new());
                          error_msg.set(String::new());
                          info_msg.set(String::new());
                          local_err.set(String::new());
                          auth_mode.set(AUTH_MODE_LOGIN);
                          on_select_server(url.clone());
                          on_probe_server(url);
                          in_auth_view.set(true);
                        }
                        Err(e) => add_server_err.set(e),
                      },
                      Err(e) => add_server_err.set(e),
                    }
                  })
                };

                view()
                  .test_id("world-add-server-panel")
                  .w_full()
                  .p(px(12.0))
                  .gap(px(8.0))
                  .rounded(px(10.0))
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).border_normal)
                  .bg(move || palette(dark_mode.get()).surface_raised)
                  .child(
                    text("输入自托管服务器地址")
                      .text_size(px(12.0))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(move || palette(dark_mode.get()).text_muted),
                  )
                  .child(
                    input()
                      .test_id("world-add-server-input")
                      .value(move || new_server_url.get())
                      .placeholder("https://mica.example.com")
                      .submit_behavior(SubmitBehavior::Submit)
                      .on_change(move |v: &str| {
                        new_server_url.set(v.to_owned());
                        if !add_server_err.get_untracked().is_empty() {
                          add_server_err.set(String::new());
                        }
                      })
                      .on_submit({
                        let submit_add = submit_add.clone();
                        move |_: &str| submit_add()
                      })
                      .w_full()
                      .h(px(36.0))
                      .px(px(10.0))
                      .rounded(px(7.0))
                      .border_1()
                      .border_color(move || palette(dark_mode.get()).border_normal)
                      .bg(move || palette(dark_mode.get()).surface_overlay)
                      .text_size(px(13.0))
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  )
                  .child(when(
                    move || !add_server_err.get().is_empty(),
                    move || {
                      text(move || add_server_err.get())
                        .text_size(px(11.5))
                        .text_color(move || palette(dark_mode.get()).danger)
                    },
                  ))
                  .child(
                    view()
                      .flex_row()
                      .justify_end()
                      .gap(px(8.0))
                      .child(
                        button()
                          .test_id("world-add-server-cancel")
                          .h(px(30.0))
                          .px(px(12.0))
                          .rounded(px(7.0))
                          .border_1()
                          .border_color(move || palette(dark_mode.get()).border_normal)
                          .bg(move || palette(dark_mode.get()).surface_overlay)
                          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                          .on_press(move || {
                            adding_server.set(false);
                            add_server_err.set(String::new());
                          })
                          .child(
                            text("取消")
                              .text_size(px(12.5))
                              .text_color(move || palette(dark_mode.get()).text_muted),
                          ),
                      )
                      .child(
                        button()
                          .test_id("world-add-server-confirm")
                          .h(px(30.0))
                          .px(px(14.0))
                          .rounded(px(7.0))
                          .bg(move || palette(dark_mode.get()).accent)
                          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
                          .on_press(move || submit_add())
                          .child(
                            text("连接并登录")
                              .text_size(px(12.5))
                              .font_weight(FontWeight::MEDIUM)
                              .text_color(white()),
                          ),
                      ),
                  )
                  .into_element()
              }
            }
          }),
        ),
    )
    .child(
      // Isolation footer
      view()
        .px(px(20.0))
        .py(px(10.0))
        .border_t_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .bg(move || palette(dark_mode.get()).surface_raised)
        .child(
          text(format!(
            "各服务器与本地库的数据和凭据严格隔离 · Mica v{app_version}"
          ))
          .text_size(px(11.0))
          .text_color(move || palette(dark_mode.get()).text_faint),
        ),
    )
}

/// View B on Desktop: single-server sign-in / register form inside the centered modal.
fn desktop_auth_view(
  state: WorldSwitcherState,
  in_auth_view: Signal<bool>,
  auth_mode: Signal<u8>,
  email_draft: Signal<String>,
  display_name_draft: Signal<String>,
  password_draft: Signal<String>,
  local_err: Signal<String>,
) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let can_close = state.can_close;
  let on_close = state.on_close.clone();
  let error_msg = state.error_msg;
  let info_msg = state.info_msg;

  view()
    .w_full()
    .child(
      // Header: Back arrow + "登录到 {host}" + Close button
      view()
        .px(px(14.0))
        .py(px(13.0))
        .border_b_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
          view()
            .flex_1()
            .min_w(px(0.0))
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(
              button()
                .test_id("world-auth-back-btn")
                .accessibility_label("返回环境列表")
                .w(px(28.0))
                .h(px(28.0))
                .rounded(px(7.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  error_msg.set(String::new());
                  info_msg.set(String::new());
                  local_err.set(String::new());
                  in_auth_view.set(false);
                })
                .child(dynamic(move || {
                  arrow_back_icon(palette(dark_mode.get()).text_muted_hex)
                })),
            )
            .child(
              text({
                let st = state.clone();
                move || {
                  let host = server_label(&effective_cloud_origin(&st));
                  let host_disp = if host.is_empty() {
                    "Mica 云端".to_string()
                  } else {
                    host
                  };
                  if auth_mode.get() == AUTH_MODE_REGISTER {
                    format!("注册账号 · {host_disp}")
                  } else {
                    format!("登录到 {host_disp}")
                  }
                }
              })
              .truncate()
              .text_size(px(15.0))
              .font_weight(FontWeight::SEMIBOLD)
              .text_color(move || palette(dark_mode.get()).text_primary),
            ),
        )
        .child(when(
          move || can_close.get(),
          {
            let on_close = on_close.clone();
            move || close_icon_button(dark_mode, on_close.clone())
          },
        )),
    )
    .child(
      view()
        .p(px(20.0))
        .gap(px(16.0))
        .child(server_status_chip(state.clone()))
        .child(auth_form(
          state,
          auth_mode,
          email_draft,
          display_name_draft,
          password_draft,
          local_err,
        )),
    )
}

/// Compact server connectivity chip (`test_id("world-server-card")`) with status
/// dot, host label, connectivity text, and manual retry button.
fn server_status_chip(state: WorldSwitcherState) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let health_status = state.health_status;
  let on_probe_server = state.on_probe_server.clone();

  view()
    .test_id("world-server-card")
    .w_full()
    .px(px(12.0))
    .py(px(9.0))
    .rounded(px(9.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_raised)
    .flex_row()
    .items_center()
    .justify_between()
    .gap(px(10.0))
    .child(
      view()
        .flex_1()
        .min_w(px(0.0))
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
          view()
            .w(px(8.0))
            .h(px(8.0))
            .rounded_full()
            .flex_shrink_0()
            .bg(move || {
              let p = palette(dark_mode.get());
              match health_status.get().as_str() {
                "reachable" => rgb(0x22C55E),
                "unreachable" => p.danger,
                _ => p.text_faint,
              }
            }),
        )
        .child(
          text({
            let st = state.clone();
            move || {
              let host = server_label(&effective_cloud_origin(&st));
              if host.is_empty() {
                "未选择服务器".to_string()
              } else {
                host
              }
            }
          })
          .truncate()
          .text_size(px(12.5))
          .font_weight(FontWeight::MEDIUM)
          .text_color(move || palette(dark_mode.get()).text_primary),
        )
        .child(
          text(move || match health_status.get().as_str() {
            "reachable" => "· 已连接",
            "unreachable" => "· 连不上服务器",
            _ => "· 正在检测…",
          })
          .text_size(px(11.5))
          .flex_shrink_0()
          .text_color(move || {
            let p = palette(dark_mode.get());
            match health_status.get().as_str() {
              "reachable" => p.success,
              "unreachable" => p.danger,
              _ => p.text_muted,
            }
          }),
        ),
    )
    .child(
      button()
        .test_id("world-server-refresh-btn")
        .accessibility_label("重新检测服务器连接")
        .w(px(26.0))
        .h(px(26.0))
        .rounded(px(6.0))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press({
          let st = state.clone();
          move || {
            let origin = effective_cloud_origin_untracked(&st);
            if !origin.is_empty() {
              on_probe_server(origin);
            }
          }
        })
        .child(dynamic(move || {
          refresh_icon(palette(dark_mode.get()).text_muted_hex)
        })),
    )
}

/// Clean Email + Password auth form without stacked segmented tabs; switches
/// between Sign-In and Register via bottom text links.
fn auth_form(
  state: WorldSwitcherState,
  auth_mode: Signal<u8>,
  email_draft: Signal<String>,
  display_name_draft: Signal<String>,
  password_draft: Signal<String>,
  local_err: Signal<String>,
) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let busy = state.busy;
  let error_msg = state.error_msg;
  let info_msg = state.info_msg;

  let submit_form = {
    let st = state.clone();
    let on_login = state.on_login.clone();
    let on_register = state.on_register.clone();
    Rc::new(move || {
      if busy.get_untracked() {
        return;
      }
      let origin = effective_cloud_origin_untracked(&st);
      if origin.is_empty() {
        local_err.set("请先选择或添加一台服务器".to_string());
        return;
      }
      let email = email_draft.get_untracked().trim().to_string();
      let password = password_draft.get_untracked();
      if email.is_empty() || !email.contains('@') {
        local_err.set("请输入有效的邮箱地址".to_string());
        return;
      }
      if auth_mode.get_untracked() == AUTH_MODE_REGISTER {
        if password.len() < 8 {
          local_err.set("密码至少需要 8 位字符".to_string());
          return;
        }
        local_err.set(String::new());
        let name = display_name_draft.get_untracked().trim().to_string();
        on_register(origin, email, name, password);
      } else {
        if password.is_empty() {
          local_err.set("请输入密码".to_string());
          return;
        }
        local_err.set(String::new());
        on_login(origin, email, password);
      }
    })
  };

  let trigger_forgot = {
    let st = state.clone();
    let on_forgot = state.on_forgot_password.clone();
    Rc::new(move || {
      if busy.get_untracked() {
        return;
      }
      let origin = effective_cloud_origin_untracked(&st);
      let email = email_draft.get_untracked().trim().to_string();
      if origin.is_empty() {
        local_err.set("请先选择一台服务器".to_string());
        return;
      }
      if email.is_empty() || !email.contains('@') {
        local_err.set("请先在上方填写您的邮箱地址，再点击忘记密码".to_string());
        return;
      }
      local_err.set(String::new());
      on_forgot(origin, email);
    })
  };

  let combined_error = move || {
    let loc = local_err.get();
    if !loc.trim().is_empty() {
      loc
    } else {
      error_msg.get()
    }
  };

  view()
    .test_id("world-auth-form")
    .w_full()
    .gap(px(12.0))
    .child(
      // Email field
      form_field(
        "邮箱",
        dark_mode,
        input()
          .test_id("world-auth-email")
          .value(move || email_draft.get())
          .placeholder("you@example.com")
          .readonly(move || busy.get())
          .submit_behavior(SubmitBehavior::Submit)
          .on_change(move |v: &str| {
            email_draft.set(v.to_owned());
            if !local_err.get_untracked().is_empty() {
              local_err.set(String::new());
            }
          })
          .on_submit({
            let submit = submit_form.clone();
            move |_: &str| submit()
          })
          .w_full()
          .h(px(38.0))
          .px(px(11.0))
          .rounded(px(8.0))
          .border_1()
          .border_color(move || palette(dark_mode.get()).border_normal)
          .bg(move || palette(dark_mode.get()).surface_base)
          .text_size(px(13.5))
          .text_color(move || palette(dark_mode.get()).text_primary),
      ),
    )
    .child(when(
      move || auth_mode.get() == AUTH_MODE_REGISTER,
      {
        let submit = submit_form.clone();
        move || {
          form_field(
            "显示名称（可选）",
            dark_mode,
            input()
              .test_id("world-auth-display-name")
              .value(move || display_name_draft.get())
              .placeholder("怎么称呼你")
              .readonly(move || busy.get())
              .submit_behavior(SubmitBehavior::Submit)
              .on_change(move |v: &str| display_name_draft.set(v.to_owned()))
              .on_submit({
                let submit = submit.clone();
                move |_: &str| submit()
              })
              .w_full()
              .h(px(38.0))
              .px(px(11.0))
              .rounded(px(8.0))
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .bg(move || palette(dark_mode.get()).surface_base)
              .text_size(px(13.5))
              .text_color(move || palette(dark_mode.get()).text_primary),
          )
        }
      },
    ))
    .child(
      // Password field
      form_field(
        "密码",
        dark_mode,
        input()
          .test_id("world-auth-password")
          .value(move || password_draft.get())
          .placeholder(move || {
            if auth_mode.get() == AUTH_MODE_REGISTER {
              "设置密码（至少 8 位）"
            } else {
              "输入密码"
            }
          })
          .masked(true)
          .readonly(move || busy.get())
          .submit_behavior(SubmitBehavior::Submit)
          .on_change(move |v: &str| {
            password_draft.set(v.to_owned());
            if !local_err.get_untracked().is_empty() {
              local_err.set(String::new());
            }
          })
          .on_submit({
            let submit = submit_form.clone();
            move |_: &str| submit()
          })
          .w_full()
          .h(px(38.0))
          .px(px(11.0))
          .rounded(px(8.0))
          .border_1()
          .border_color(move || palette(dark_mode.get()).border_normal)
          .bg(move || palette(dark_mode.get()).surface_base)
          .text_size(px(13.5))
          .text_color(move || palette(dark_mode.get()).text_primary),
      ),
    )
    .child(when(
      move || !combined_error().trim().is_empty(),
      move || {
        view()
          .test_id("world-auth-error")
          .w_full()
          .px(px(11.0))
          .py(px(8.0))
          .rounded(px(8.0))
          .bg(move || palette(dark_mode.get()).danger_wash)
          .flex_row()
          .items_center()
          .gap(px(7.0))
          .child(dynamic(move || {
            error_circle_icon(palette(dark_mode.get()).danger_hex)
          }))
          .child(
            text(combined_error)
              .flex_1()
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).danger),
          )
      },
    ))
    .child(when(
      move || !info_msg.get().trim().is_empty(),
      move || {
        view()
          .test_id("world-auth-info")
          .w_full()
          .px(px(11.0))
          .py(px(8.0))
          .rounded(px(8.0))
          .bg(move || palette(dark_mode.get()).success_wash)
          .flex_row()
          .items_center()
          .gap(px(7.0))
          .child(dynamic(move || {
            check_circle_icon(palette(dark_mode.get()).success_hex)
          }))
          .child(
            text(move || info_msg.get())
              .flex_1()
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).success),
          )
      },
    ))
    .child(
      // Primary submit CTA
      button()
        .test_id("world-auth-submit")
        .accessibility_label("提交登录或注册")
        .w_full()
        .h(px(40.0))
        .mt(px(2.0))
        .rounded(px(8.0))
        .bg(move || {
          let p = palette(dark_mode.get());
          if busy.get() { p.accent_hover } else { p.accent }
        })
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
        .items_center()
        .justify_center()
        .on_press(move || submit_form())
        .child(
          text(move || {
            if busy.get() {
              "请稍候…"
            } else if auth_mode.get() == AUTH_MODE_REGISTER {
              "创建账号"
            } else {
              "登录"
            }
          })
          .text_size(px(14.0))
          .font_weight(FontWeight::MEDIUM)
          .text_color(white()),
        ),
    )
    .child(
      // Bottom helper links: "忘记密码？" on left, "没有账号？注册" / "已有账号？去登录" on right
      view()
        .w_full()
        .pt(px(2.0))
        .flex_row()
        .items_center()
        .justify_between()
        .child(dynamic({
          let trigger_forgot = trigger_forgot.clone();
          move || {
            if auth_mode.get() == AUTH_MODE_LOGIN {
              button()
                .test_id("world-auth-forgot")
                .accessibility_label("忘记密码")
                .py(px(4.0))
                .px(px(2.0))
                .on_press({
                  let tf = trigger_forgot.clone();
                  move || tf()
                })
                .child(
                  text("忘记密码？")
                    .text_size(px(12.5))
                    .text_color(move || palette(dark_mode.get()).text_muted),
                )
                .into_element()
            } else {
              view().into_element()
            }
          }
        }))
        .child(dynamic(move || {
          if auth_mode.get() == AUTH_MODE_LOGIN {
            button()
              .test_id("world-auth-mode-register")
              .accessibility_label("切换到注册账号")
              .py(px(4.0))
              .px(px(2.0))
              .on_press(move || {
                error_msg.set(String::new());
                info_msg.set(String::new());
                local_err.set(String::new());
                auth_mode.set(AUTH_MODE_REGISTER);
              })
              .child(
                text("没有账号？注册")
                  .text_size(px(12.5))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).accent),
              )
              .into_element()
          } else {
            button()
              .test_id("world-auth-mode-login")
              .accessibility_label("返回登录")
              .py(px(4.0))
              .px(px(2.0))
              .on_press(move || {
                error_msg.set(String::new());
                info_msg.set(String::new());
                local_err.set(String::new());
                auth_mode.set(AUTH_MODE_LOGIN);
              })
              .child(
                text("已有账号？去登录")
                  .text_size(px(12.5))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).accent),
              )
              .into_element()
          }
        })),
    )
}

fn form_field(
  label: &'static str,
  dark_mode: Signal<bool>,
  field: impl IntoElement + 'static,
) -> impl IntoElement {
  view()
    .w_full()
    .gap(px(5.0))
    .child(
      text(label)
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_muted),
    )
    .child(field)
}

fn close_icon_button(dark_mode: Signal<bool>, on_close: Rc<dyn Fn()>) -> impl IntoElement {
  button()
    .test_id("world-switcher-close")
    .accessibility_label("关闭")
    .w(px(28.0))
    .h(px(28.0))
    .rounded(px(7.0))
    .flex_shrink_0()
    .items_center()
    .justify_center()
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(move || on_close())
    .child(dynamic(move || {
      mica_icons::close_sm(palette(dark_mode.get()).text_muted_hex)
    }))
}

fn status_badge(label: &'static str, bg: nsh_core::color::Rgba, fg: nsh_core::color::Rgba) -> View {
  view()
    .px(px(8.0))
    .py(px(3.0))
    .rounded(px(999.0))
    .flex_shrink_0()
    .bg(bg)
    .child(
      text(label)
        .text_size(px(11.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg),
    )
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

fn computer_icon(color: &str) -> Svg {
  stroked_icon(
    16.0,
    color,
    r##"<rect x="3" y="4" width="14" height="9.5" rx="1.6"/><path d="M7.5 16.5H12.5M10 13.5V16.5"/>"##,
  )
}

fn refresh_icon(color: &str) -> Svg {
  stroked_icon(
    15.0,
    color,
    r##"<path d="M15.8 10A5.8 5.8 0 1 1 14 5.8L16.2 7.8M16.2 4.5V7.8H12.9"/>"##,
  )
}

fn check_circle_icon(color: &str) -> Svg {
  stroked_icon(
    15.0,
    color,
    r##"<circle cx="10" cy="10" r="7"/><path d="M6.8 10.2L9 12.4L13.5 7.8"/>"##,
  )
}

fn error_circle_icon(color: &str) -> Svg {
  stroked_icon(
    15.0,
    color,
    r##"<circle cx="10" cy="10" r="7"/><path d="M10 6.5V10.5M10 13.2H10.05"/>"##,
  )
}

fn arrow_back_icon(color: &str) -> Svg {
  stroked_icon(16.0, color, r##"<path d="M15 10H5M9.5 5.5L5 10L9.5 14.5"/>"##)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn server_label_extracts_lowercase_host_or_falls_back() {
    assert_eq!(server_label("https://mica.example.com"), "mica.example.com");
    assert_eq!(
      server_label("https://Mica.Example.com:8443/sub/path"),
      "mica.example.com"
    );
    assert_eq!(server_label("http://127.0.0.1:8080"), "127.0.0.1");
    assert_eq!(server_label("http://[::1]:8080/api"), "::1");
    assert_eq!(server_label("local"), "local");
    assert_eq!(server_label("not-a-url"), "not-a-url");
    assert_eq!(server_label("https://"), "https://");
    assert_eq!(server_label(""), "");
  }

  #[test]
  fn normalize_server_url_canonicalizes_valid_http_and_https_urls() {
    assert_eq!(
      normalize_server_url("  https://Mica.Example.com:443/mica/  ").unwrap(),
      "https://mica.example.com/mica"
    );
    assert_eq!(
      normalize_server_url("http://LOCALHOST:80/").unwrap(),
      "http://localhost"
    );
    assert_eq!(
      normalize_server_url("http://127.0.0.1:8080///").unwrap(),
      "http://127.0.0.1:8080"
    );
    assert_eq!(
      normalize_server_url("HTTPS://mica.cloudcele.com:8443/v1/").unwrap(),
      "https://mica.cloudcele.com:8443/v1"
    );
    assert_eq!(
      normalize_server_url("http://[::1]:3000/").unwrap(),
      "http://[::1]:3000"
    );
  }

  #[test]
  fn normalize_server_url_rejects_malformed_or_unsupported_urls() {
    for bad in [
      "",
      "   ",
      "mica.example.com",
      "ftp://mica.example.com",
      "https://",
      "https://:443",
      "https://mica.example.com:0",
      "https://mica.example.com:99999",
      "https://mica example.com",
      "https://user:pass@mica.example.com",
      "https://mica.example.com?foo=bar",
      "https://mica.example.com#hash",
    ] {
      assert!(
        normalize_server_url(bad).is_err(),
        "expected error for invalid server url: {bad:?}"
      );
    }
  }
}
