//! Full-window split-screen World Switcher / Cloud Sign-In overlay.
//!
//! 1:1 visual and semantic parity with:
//! - `main:clients/mica_flutter/lib/ui/sign_in_screen.dart`
//! - `main:clients/mica_flutter/lib/ui/sign_in_hero.dart`
//! - `main:clients/mica_flutter/lib/ui/sign_in_pane.dart`
//! - `main:clients/mica_flutter/lib/ui/auth_form.dart`
//! - `main:clients/mica_flutter/lib/server_list.dart`
//! - `main:clients/mica_flutter/lib/local/cache_stats.dart`

use std::rc::Rc;

use nsh_base::{button, input};
use nsh_core::{
  SubmitBehavior,
  color::{rgb, rgba, white},
  element::{ImageSrc, Svg, svg},
  geometry::px,
  prelude::*,
};

use crate::{mica_icons, mica_theme::palette};

const MICA_LOGO_SVG: &str = include_str!("../../../assets/mica-logo.svg");

/// Origin string representing the on-device local workspace.
pub const LOCAL_ORIGIN: &str = "local";

/// Minimum viewport width to display the split-screen brand hero panel beside the form.
pub const SIGN_IN_SPLIT_MIN_WIDTH: f32 = 900.0;

/// Fixed width of the right-hand form column on wide desktop viewports.
pub const SIGN_IN_FORM_WIDTH: f32 = 420.0;

const AUTH_MODE_LOGIN: u8 = 0;
const AUTH_MODE_REGISTER: u8 = 1;

/// Reactive state and callbacks driving the full-window World Switcher overlay.
#[derive(Clone)]
pub struct WorldSwitcherState {
  pub open: Signal<bool>,
  pub can_close: Signal<bool>,
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
/// Mirrors `serverLabel` in `server_list.dart`: `https://mica.example.com:8443/mica`
/// renders as `mica.example.com`; unparseable inputs fall back to the raw string.
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
/// Mirrors `normalizeServerUrl` in `cache_stats.dart`:
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

/// Full-window World Switcher / Cloud Sign-In overlay matching `sign_in_screen.dart`.
pub fn world_switcher_overlay(state: WorldSwitcherState) -> impl IntoElement {
  when(move || state.open.get(), {
    let state = state.clone();
    move || {
      let st = state.clone();
      let dark_mode = st.dark_mode;
      let can_close = st.can_close;
      let app_version = st.app_version;

      // Responsive width tracked via `on_layout`; starts wide to avoid a narrow flash.
      let viewport_width = signal(1200.0_f32);

      // Pane-local state preserved across viewport width crossings.
      let cloud_tab = signal(st.active_origin.get_untracked() != LOCAL_ORIGIN);
      let servers_open = signal(false);
      let adding_server = signal(false);
      let new_server_url = signal(String::new());
      let add_server_err = signal(String::new());
      let confirm_remove_origin: Signal<Option<String>> = signal(None);

      // Auth form drafts preserved across resize / re-renders.
      let auth_mode = signal(AUTH_MODE_LOGIN);
      let email_draft = signal(String::new());
      let display_name_draft = signal(String::new());
      let password_draft = signal(String::new());

      let close_btn = st.on_close.clone();

      view()
        .test_id("world-switcher-overlay")
        .absolute()
        .inset_0()
        .z_index(70)
        .flex_row()
        .bg(move || palette(dark_mode.get()).surface_base)
        .on_layout(move |b: &Bounds<Points>| {
          let w = f32::from(b.size.width);
          if w > 0.0 && (viewport_width.get_untracked() - w).abs() > 1.0 {
            viewport_width.set(w);
          }
        })
        .child(when(
          move || viewport_width.get() >= SIGN_IN_SPLIT_MIN_WIDTH,
          move || sign_in_hero(app_version),
        ))
        .child(
          view()
            .test_id("world-switcher-form-pane")
            .relative()
            .h_full()
            .flex_shrink_0()
            .w(move || {
              let w = viewport_width.get();
              if w >= SIGN_IN_SPLIT_MIN_WIDTH {
                px(SIGN_IN_FORM_WIDTH)
              } else {
                px(w.max(320.0))
              }
            })
            .bg(move || palette(dark_mode.get()).surface_base)
            .child(
              scroll().w_full().flex_1().min_h(px(0.0)).child(
                view()
                  .w_full()
                  .px(px(28.0))
                  .pt(px(80.0))
                  .pb(px(48.0))
                  .items_center()
                  .child(
                    view()
                      .w_full()
                      .max_w(px(364.0))
                      .gap(px(24.0))
                      .child(world_tabs(st.clone(), cloud_tab))
                      .child(when(move || cloud_tab.get(), {
                        let st = st.clone();
                        move || {
                          cloud_pane(
                            st.clone(),
                            servers_open,
                            adding_server,
                            new_server_url,
                            add_server_err,
                            confirm_remove_origin,
                            auth_mode,
                            email_draft,
                            display_name_draft,
                            password_draft,
                          )
                        }
                      }))
                      .child(when(move || !cloud_tab.get(), {
                        let st = st.clone();
                        move || local_pane(st.clone())
                      })),
                  ),
              ),
            )
            .child(when(move || can_close.get(), {
              let close_btn = close_btn.clone();
              move || {
                let close_btn = close_btn.clone();
                button()
                  .test_id("world-switcher-close")
                  .accessibility_label("返回")
                  .absolute()
                  .top(px(40.0))
                  .right(px(16.0))
                  .z_index(72)
                  .h(px(32.0))
                  .px(px(10.0))
                  .rounded(px(6.0))
                  .flex_row()
                  .items_center()
                  .gap(px(5.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || close_btn())
                  .child(dynamic(move || {
                    mica_icons::close_sm(palette(dark_mode.get()).text_muted_hex)
                  }))
                  .child(
                    text("返回")
                      .text_size(px(13.0))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  )
              }
            })),
        )
    }
  })
}

/// Left dark brand hero panel matching `sign_in_hero.dart`.
fn sign_in_hero(app_version: &'static str) -> impl IntoElement {
  let features = [
    "离线也能写，联网后自动同步",
    "实时协作：多人同时编辑，能看见彼此的光标",
    "块编辑器 · 代码高亮 · 数学公式 · 表格",
  ];

  let mut feature_col = view().w_full().gap(px(12.0));
  for line in features {
    feature_col = feature_col.child(
      view()
        .w_full()
        .flex_row()
        .items_start()
        .gap(px(10.0))
        .child(
          view()
            .w(px(22.0))
            .h(px(22.0))
            .rounded_full()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .bg(rgba(0x2563_EB38))
            .child(hero_check_icon("#93C5FD")),
        )
        .child(
          text(line)
            .flex_1()
            .text_size(px(13.0))
            .text_color(rgb(0xE2E8F0)),
        ),
    );
  }

  view()
    .test_id("world-switcher-hero")
    .flex_1()
    .h_full()
    .items_center()
    .justify_center()
    .bg(rgb(0x0B1220))
    .bg_radial()
    .gradient_from(rgb(0x1E293B))
    .gradient_to(rgb(0x0B1220))
    .child(
      scroll().w_full().child(
        view()
          .w_full()
          .px(px(48.0))
          .py(px(40.0))
          .items_center()
          .child(
            view()
              .w_full()
              .max_w(px(460.0))
              .child(
                // Brand row: 34px MicaLogo + "Mica" 26px w600
                view()
                  .flex_row()
                  .items_center()
                  .gap(px(11.0))
                  .child(mica_logo_sized(34.0))
                  .child(
                    text("Mica")
                      .text_size(px(26.0))
                      .font_weight(FontWeight::SEMIBOLD)
                      .text_color(white()),
                  ),
              )
              .child(view().h(px(26.0)))
              .child(
                text("本地优先的\n协作知识库")
                  .text_size(px(34.0))
                  .font_weight(FontWeight::SEMIBOLD)
                  .text_color(white()),
              )
              .child(view().h(px(14.0)))
              .child(
                text("块编辑器、实时协作。改动经 CRDT 合并，多台设备之间不会互相覆盖。")
                  .text_size(px(14.0))
                  .text_color(rgb(0x94A3B8)),
              )
              .child(view().h(px(26.0)))
              .child(feature_col)
              .child(view().h(px(26.0)))
              .child(
                text(format!("CRDT SYNC · OFFLINE-FIRST · v{app_version}"))
                  .text_size(px(11.0))
                  .text_color(rgb(0x64748B)),
              ),
          ),
      ),
    )
}

/// Top segmented tab control (`云端账户` / `本地模式`).
fn world_tabs(state: WorldSwitcherState, cloud_tab: Signal<bool>) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let transparent = rgba(0x0000_0000);
  let st_cloud = state.clone();

  view()
    .w_full()
    .p(px(3.0))
    .rounded(px(8.0))
    .bg(move || palette(dark_mode.get()).surface_sunken)
    .flex_row()
    .gap(px(2.0))
    .child(
      button()
        .test_id("world-tab-cloud")
        .accessibility_label("云端账户")
        .flex_1()
        .h(px(32.0))
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(7.0))
        .border_1()
        .border_color(move || {
          if cloud_tab.get() {
            palette(dark_mode.get()).border_normal
          } else {
            transparent
          }
        })
        .bg(move || {
          if cloud_tab.get() {
            palette(dark_mode.get()).surface_base
          } else {
            transparent
          }
        })
        .on_press(move || {
          let was_local = !cloud_tab.get_untracked();
          cloud_tab.set(true);
          if was_local {
            let origin = effective_cloud_origin_untracked(&st_cloud);
            if !origin.is_empty() {
              (st_cloud.on_probe_server)(origin);
            }
          }
        })
        .child(dynamic(move || {
          let p = palette(dark_mode.get());
          let hex = if cloud_tab.get() {
            p.text_primary_hex
          } else {
            p.text_muted_hex
          };
          mica_icons::cloud(hex)
        }))
        .child(
          text("云端账户")
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
              let p = palette(dark_mode.get());
              if cloud_tab.get() {
                p.text_primary
              } else {
                p.text_muted
              }
            }),
        ),
    )
    .child(
      button()
        .test_id("world-tab-local")
        .accessibility_label("本地模式")
        .flex_1()
        .h(px(32.0))
        .rounded(px(6.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(7.0))
        .border_1()
        .border_color(move || {
          if !cloud_tab.get() {
            palette(dark_mode.get()).border_normal
          } else {
            transparent
          }
        })
        .bg(move || {
          if !cloud_tab.get() {
            palette(dark_mode.get()).surface_base
          } else {
            transparent
          }
        })
        .on_press(move || cloud_tab.set(false))
        .child(dynamic(move || {
          let p = palette(dark_mode.get());
          let hex = if !cloud_tab.get() {
            p.text_primary_hex
          } else {
            p.text_muted_hex
          };
          computer_icon(hex)
        }))
        .child(
          text("本地模式")
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
              let p = palette(dark_mode.get());
              if !cloud_tab.get() {
                p.text_primary
              } else {
                p.text_muted
              }
            }),
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn cloud_pane(
  state: WorldSwitcherState,
  servers_open: Signal<bool>,
  adding_server: Signal<bool>,
  new_server_url: Signal<String>,
  add_server_err: Signal<String>,
  confirm_remove_origin: Signal<Option<String>>,
  auth_mode: Signal<u8>,
  email_draft: Signal<String>,
  display_name_draft: Signal<String>,
  password_draft: Signal<String>,
) -> impl IntoElement {
  let servers = state.servers;
  let signed_in_origins = state.signed_in_origins;

  let st_card = state.clone();
  let st_empty = state.clone();
  let st_signed = state.clone();
  let st_auth = state.clone();
  let st_check = state.clone();

  view()
    .w_full()
    .gap(px(24.0))
    .child(when(move || servers.get().is_empty(), {
      let st = st_empty.clone();
      move || {
        add_server_row(
          st.clone(),
          adding_server,
          new_server_url,
          add_server_err,
          true,
        )
      }
    }))
    .child(when(move || !servers.get().is_empty(), {
      let st = st_card.clone();
      move || {
        server_selector_section(
          st.clone(),
          servers_open,
          adding_server,
          new_server_url,
          add_server_err,
          confirm_remove_origin,
        )
      }
    }))
    .child(when(
      move || {
        if servers.get().is_empty() {
          return false;
        }
        let origin = effective_cloud_origin(&st_check);
        signed_in_origins
          .get()
          .iter()
          .any(|(o, _)| o == &origin)
      },
      {
        let st = st_signed.clone();
        move || signed_in_card(st.clone())
      },
    ))
    .child(when(
      move || {
        if servers.get().is_empty() {
          return false;
        }
        let origin = effective_cloud_origin(&state);
        !signed_in_origins
          .get()
          .iter()
          .any(|(o, _)| o == &origin)
      },
      {
        let st = st_auth.clone();
        move || {
          auth_form_card(
            st.clone(),
            auth_mode,
            email_draft,
            display_name_draft,
            password_draft,
          )
        }
      },
    ))
}

fn server_selector_section(
  state: WorldSwitcherState,
  servers_open: Signal<bool>,
  adding_server: Signal<bool>,
  new_server_url: Signal<String>,
  add_server_err: Signal<String>,
  confirm_remove_origin: Signal<Option<String>>,
) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let health_status = state.health_status;
  let st_label = state.clone();
  let st_retry = state.clone();
  let st_list = state.clone();
  let st_add = state.clone();

  view()
    .w_full()
    .gap(px(8.0))
    .child(
      // Collapsed server health card
      view()
        .test_id("world-server-card")
        .w_full()
        .px(px(14.0))
        .py(px(11.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_raised)
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
          button()
            .test_id("world-server-toggle")
            .accessibility_label("展开或收起服务器列表")
            .flex_1()
            .min_w(px(0.0))
            .on_press(move || {
              let next = !servers_open.get_untracked();
              servers_open.set(next);
              if !next {
                confirm_remove_origin.set(None);
                adding_server.set(false);
                add_server_err.set(String::new());
              }
            })
            .child(
              view()
                .w_full()
                .gap(px(3.0))
                .child(
                  view()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                      // 6x6 status dot
                      view()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .flex_shrink_0()
                        .bg(move || {
                          let p = palette(dark_mode.get());
                          match health_status.get().as_str() {
                            "reachable" => p.success,
                            "unreachable" => p.danger,
                            _ => p.border_strong,
                          }
                        }),
                    )
                    .child(
                      text(move || match health_status.get().as_str() {
                        "reachable" => "已连接到服务器",
                        "unreachable" => "连不上服务器",
                        _ => "正在检测…",
                      })
                      .text_size(px(12.0))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(move || {
                        let p = palette(dark_mode.get());
                        if health_status.get() == "reachable" {
                          p.success
                        } else {
                          p.text_muted
                        }
                      }),
                    ),
                )
                .child(
                  text(move || server_label(&effective_cloud_origin(&st_label)))
                    .truncate()
                    .font_family("monospace")
                    .text_size(px(12.0))
                    .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            ),
        )
        .child(when(
          move || health_status.get() == "unreachable",
          {
            let st_retry = st_retry.clone();
            move || {
              let st_retry = st_retry.clone();
              button()
                .test_id("world-server-retry")
                .accessibility_label("重试连接服务器")
                .w(px(28.0))
                .h(px(28.0))
                .rounded(px(6.0))
                .items_center()
                .justify_center()
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  let origin = effective_cloud_origin_untracked(&st_retry);
                  if !origin.is_empty() {
                    (st_retry.on_probe_server)(origin);
                  }
                })
                .child(dynamic(move || {
                  refresh_icon(palette(dark_mode.get()).text_muted_hex)
                }))
            }
          },
        ))
        .child(
          button()
            .test_id("world-server-chevron")
            .accessibility_label("切换服务器列表")
            .w(px(24.0))
            .h(px(24.0))
            .rounded(px(6.0))
            .items_center()
            .justify_center()
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || servers_open.set(!servers_open.get_untracked()))
            .child(dynamic(move || {
              let hex = palette(dark_mode.get()).text_muted_hex;
              if servers_open.get() {
                chevron_up_icon(hex)
              } else {
                mica_icons::chevron_down(hex)
              }
            })),
        ),
    )
    .child(when(move || servers_open.get(), {
      let st_list = st_list.clone();
      let st_add = st_add.clone();
      move || {
        let st_list = st_list.clone();
        let st_add = st_add.clone();
        view()
          .test_id("world-server-list")
          .w_full()
          .rounded(px(8.0))
          .border_1()
          .border_color(move || palette(dark_mode.get()).border_normal)
          .bg(move || palette(dark_mode.get()).surface_base)
          .overflow_hidden()
          .child(
            view()
              .px(px(14.0))
              .pt(px(10.0))
              .pb(px(6.0))
              .child(
                text("服务器")
                  .text_size(px(11.0))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).text_faint),
              ),
          )
          .child(dynamic({
            let st_list = st_list.clone();
            move || {
              let origins = st_list.servers.get();
              let signed_in = st_list.signed_in_origins.get();
              let current = effective_cloud_origin(&st_list);
              let confirming = confirm_remove_origin.get();
              let p = palette(dark_mode.get());

              let mut rows = view().w_full();
              for (idx, origin) in origins.into_iter().enumerate() {
                let is_selected = origin == current;
                let is_signed_in = signed_in.iter().any(|(o, _)| o == &origin);
                let is_confirming = confirming.as_deref() == Some(origin.as_str());
                let host = server_label(&origin);

                let origin_sel = origin.clone();
                let st_sel = st_list.clone();
                let origin_del = origin.clone();
                let origin_confirm = origin.clone();
                let st_del = st_list.clone();

                let mut row_group = view().w_full().child(
                  view()
                    .w_full()
                    .px(px(14.0))
                    .py(px(8.0))
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                      button()
                        .test_id(&format!("world-server-row-{idx}"))
                        .accessibility_label(format!("选择服务器 {host}"))
                        .flex_1()
                        .min_w(px(0.0))
                        .h(px(28.0))
                        .px(px(4.0))
                        .rounded(px(6.0))
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                        .on_press(move || {
                          st_sel.selected_cloud_origin.set(origin_sel.clone());
                          servers_open.set(false);
                          confirm_remove_origin.set(None);
                          (st_sel.on_select_server)(origin_sel.clone());
                          (st_sel.on_probe_server)(origin_sel.clone());
                        })
                        .child(
                          view()
                            .w(px(18.0))
                            .h(px(18.0))
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .child(when(move || is_selected, move || {
                              mica_icons::check_icon(p.text_primary_hex)
                            })),
                        )
                        .child(
                          text(host.clone())
                            .flex_1()
                            .truncate()
                            .font_family("monospace")
                            .text_size(px(12.0))
                            .text_color(p.text_primary),
                        )
                        .child(when(move || is_signed_in, move || {
                          text("已登录")
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.success)
                        })),
                    )
                    .child(
                      button()
                        .test_id(&format!("world-server-remove-{idx}"))
                        .accessibility_label(format!("删除服务器 {host}"))
                        .w(px(28.0))
                        .h(px(28.0))
                        .rounded(px(6.0))
                        .items_center()
                        .justify_center()
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).danger_wash))
                        .on_press(move || {
                          if is_confirming {
                            confirm_remove_origin.set(None);
                          } else {
                            confirm_remove_origin.set(Some(origin_del.clone()));
                          }
                        })
                        .child(mica_icons::trash_icon(p.danger_hex)),
                    ),
                );

                if is_confirming {
                  row_group = row_group.child(
                    view()
                      .test_id(&format!("world-server-confirm-remove-{idx}"))
                      .mx(px(12.0))
                      .mb(px(8.0))
                      .p(px(10.0))
                      .rounded(px(7.0))
                      .border_1()
                      .border_color(p.danger)
                      .bg(p.danger_wash)
                      .gap(px(8.0))
                      .child(
                        text(format!("删除 {host}？将清除本机登录状态与离线镜像。"))
                          .text_size(px(11.5))
                          .text_color(p.danger),
                      )
                      .child(
                        view()
                          .flex_row()
                          .justify_end()
                          .gap(px(8.0))
                          .child(
                            button()
                              .test_id(&format!("world-server-cancel-remove-{idx}"))
                              .accessibility_label("取消删除服务器")
                              .h(px(26.0))
                              .px(px(10.0))
                              .rounded(px(5.0))
                              .border_1()
                              .border_color(p.border_normal)
                              .bg(p.surface_base)
                              .on_press(move || confirm_remove_origin.set(None))
                              .child(
                                text("取消")
                                  .text_size(px(11.5))
                                  .text_color(p.text_primary),
                              ),
                          )
                          .child(
                            button()
                              .test_id(&format!("world-server-do-remove-{idx}"))
                              .accessibility_label("确认删除服务器")
                              .h(px(26.0))
                              .px(px(10.0))
                              .rounded(px(5.0))
                              .bg(p.danger)
                              .on_press(move || {
                                confirm_remove_origin.set(None);
                                (st_del.on_remove_server)(origin_confirm.clone());
                              })
                              .child(
                                text("确认删除")
                                  .text_size(px(11.5))
                                  .font_weight(FontWeight::MEDIUM)
                                  .text_color(white()),
                              ),
                          ),
                      ),
                  );
                }

                rows = rows.child(row_group);
              }
              rows
            }
          }))
          .child(
            view()
              .w_full()
              .border_t_1()
              .border_color(move || palette(dark_mode.get()).border_subtle)
              .child(add_server_row(
                st_add.clone(),
                adding_server,
                new_server_url,
                add_server_err,
                false,
              )),
          )
      }
    }))
}

fn add_server_row(
  state: WorldSwitcherState,
  adding_server: Signal<bool>,
  new_server_url: Signal<String>,
  add_server_err: Signal<String>,
  boxed: bool,
) -> impl IntoElement {
  let dark_mode = state.dark_mode;

  let submit_add: Rc<dyn Fn()> = {
    let st = state.clone();
    Rc::new(move || {
      let raw = new_server_url.get_untracked();
      match normalize_server_url(&raw) {
        Ok(normalized) => match (st.on_add_server)(normalized.clone()) {
          Ok(()) => {
            new_server_url.set(String::new());
            add_server_err.set(String::new());
            adding_server.set(false);
            st.selected_cloud_origin.set(normalized.clone());
            (st.on_probe_server)(normalized);
          }
          Err(err) => add_server_err.set(err),
        },
        Err(err) => add_server_err.set(err),
      }
    })
  };

  let inner = view()
    .w_full()
    .child(when(move || !adding_server.get(), move || {
      button()
        .test_id("world-add-server-toggle")
        .accessibility_label("添加服务器")
        .w_full()
        .px(px(14.0))
        .py(px(11.0))
        .flex_row()
        .items_center()
        .gap(px(9.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || {
          add_server_err.set(String::new());
          adding_server.set(true);
        })
        .child(dynamic(move || {
          mica_icons::plus(palette(dark_mode.get()).text_primary_hex)
        }))
        .child(
          text("添加服务器…")
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || palette(dark_mode.get()).accent),
        )
    }))
    .child(when(move || adding_server.get(), {
      let submit_enter = submit_add.clone();
      let submit_btn = submit_add.clone();
      move || {
        let submit_enter = submit_enter.clone();
        let submit_btn = submit_btn.clone();
        view()
          .test_id("world-add-server-form")
          .w_full()
          .p(px(12.0))
          .gap(px(8.0))
          .child(
            input()
              .test_id("world-add-server-input")
              .auto_focus(true)
              .submit_behavior(SubmitBehavior::Submit)
              .placeholder("https://mica.example.com")
              .w_full()
              .px(px(10.0))
              .py(px(8.0))
              .rounded(px(6.0))
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .bg(move || palette(dark_mode.get()).surface_base)
              .text_size(px(12.5))
              .text_color(move || palette(dark_mode.get()).text_primary)
              .value(move || new_server_url.get())
              .on_change(move |v: &str| {
                new_server_url.set(v.to_owned());
                if !add_server_err.get_untracked().is_empty() {
                  add_server_err.set(String::new());
                }
              })
              .on_submit(move |_: &str| submit_enter()),
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
                  .accessibility_label("取消添加服务器")
                  .h(px(28.0))
                  .px(px(10.0))
                  .rounded(px(6.0))
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).border_normal)
                  .bg(move || palette(dark_mode.get()).surface_base)
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || {
                    adding_server.set(false);
                    add_server_err.set(String::new());
                  })
                  .child(
                    text("取消")
                      .text_size(px(12.0))
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  ),
              )
              .child(
                button()
                  .test_id("world-add-server-submit")
                  .accessibility_label("添加服务器")
                  .h(px(28.0))
                  .px(px(12.0))
                  .rounded(px(6.0))
                  .bg(move || palette(dark_mode.get()).accent)
                  .on_press(move || submit_btn())
                  .child(
                    text("添加")
                      .text_size(px(12.0))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(white()),
                  ),
              ),
          )
      }
    }));

  if boxed {
    view()
      .w_full()
      .rounded(px(8.0))
      .border_1()
      .border_color(move || palette(dark_mode.get()).border_normal)
      .bg(move || palette(dark_mode.get()).surface_raised)
      .overflow_hidden()
      .child(inner)
  } else {
    inner
  }
}

/// Card shown when the selected cloud origin already has a stored session (`auth_form.dart: SignedInCard`).
fn signed_in_card(state: WorldSwitcherState) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let active_origin = state.active_origin;
  let signed_in_origins = state.signed_in_origins;

  let st_body = state.clone();
  let st_btn_label = state.clone();
  let st_btn_click = state.clone();
  let st_hint = state.clone();

  view()
    .test_id("world-signed-in-card")
    .w_full()
    .child(
      text("你已经登录了")
        .text_size(px(20.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || palette(dark_mode.get()).text_primary),
    )
    .child(view().h(px(10.0)))
    .child(
      view()
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(dynamic(move || {
          check_circle_icon(palette(dark_mode.get()).success_hex)
        }))
        .child(
          text(move || {
            let origin = effective_cloud_origin(&st_body);
            let name = signed_in_origins
              .get()
              .into_iter()
              .find(|(o, _)| o == &origin)
              .map(|(_, n)| n)
              .unwrap_or_default();
            let host = server_label(&origin);
            if name.trim().is_empty() {
              host
            } else {
              format!("{name} · {host}")
            }
          })
          .flex_1()
          .truncate()
          .text_size(px(13.5))
          .text_color(move || palette(dark_mode.get()).text_primary),
        ),
    )
    .child(view().h(px(18.0)))
    .child(
      button()
        .test_id("world-enter-signed-in-btn")
        .accessibility_label("进入已登录服务器")
        .w_full()
        .h(px(40.0))
        .rounded(px(8.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .bg(move || palette(dark_mode.get()).accent)
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
        .on_press(move || {
          let origin = effective_cloud_origin_untracked(&st_btn_click);
          if !origin.is_empty() {
            (st_btn_click.on_enter_signed_in)(origin);
          }
        })
        .child(arrow_back_icon("#FFFFFF"))
        .child(
          text(move || {
            let origin = effective_cloud_origin(&st_btn_label);
            if origin == active_origin.get() {
              "返回"
            } else {
              "进入"
            }
          })
          .text_size(px(13.5))
          .font_weight(FontWeight::MEDIUM)
          .text_color(white()),
        ),
    )
    .child(view().h(px(14.0)))
    .child(
      text(move || {
        let origin = effective_cloud_origin(&st_hint);
        if origin == active_origin.get() {
          "要去别处的话，在上面选另一个服务器或本地模式；否则点「返回」回到刚才的页面。"
        } else {
          "凭据还在，点「进入」直接用这台服务器，不用再输密码。"
        }
      })
      .text_size(px(12.0))
      .text_color(move || palette(dark_mode.get()).text_muted),
    )
}

/// Cloud login/register credentials card (`auth_form.dart: AuthFormCard`).
fn auth_form_card(
  state: WorldSwitcherState,
  auth_mode: Signal<u8>,
  email_draft: Signal<String>,
  display_name_draft: Signal<String>,
  password_draft: Signal<String>,
) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let busy = state.busy;
  let error_msg = state.error_msg;
  let info_msg = state.info_msg;
  let transparent = rgba(0x0000_0000);

  let submit: Rc<dyn Fn()> = {
    let st = state.clone();
    Rc::new(move || {
      if st.busy.get_untracked() {
        return;
      }
      let origin = effective_cloud_origin_untracked(&st);
      if origin.is_empty() {
        return;
      }
      let email = email_draft.get_untracked().trim().to_string();
      let password = password_draft.get_untracked();
      if auth_mode.get_untracked() == AUTH_MODE_REGISTER {
        let display_name = display_name_draft.get_untracked().trim().to_string();
        (st.on_register)(origin, email, display_name, password);
      } else {
        (st.on_login)(origin, email, password);
      }
    })
  };

  let submit_email = submit.clone();
  let submit_name = submit.clone();
  let submit_pwd = submit.clone();
  let submit_btn = submit.clone();
  let st_forgot = state.clone();

  view()
    .test_id("world-auth-form")
    .w_full()
    .child(
      text("登录到 Mica")
        .text_size(px(20.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || palette(dark_mode.get()).text_primary),
    )
    .child(view().h(px(16.0)))
    .child(
      // 登录 / 注册 segmented toggle
      view()
        .w_full()
        .p(px(3.0))
        .rounded(px(8.0))
        .bg(move || palette(dark_mode.get()).surface_sunken)
        .flex_row()
        .gap(px(2.0))
        .child(
          button()
            .test_id("world-auth-mode-login")
            .accessibility_label("登录模式")
            .flex_1()
            .h(px(32.0))
            .rounded(px(6.0))
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .disabled(move || busy.get())
            .border_1()
            .border_color(move || {
              if auth_mode.get() == AUTH_MODE_LOGIN {
                palette(dark_mode.get()).border_normal
              } else {
                transparent
              }
            })
            .bg(move || {
              if auth_mode.get() == AUTH_MODE_LOGIN {
                palette(dark_mode.get()).surface_base
              } else {
                transparent
              }
            })
            .on_press(move || auth_mode.set(AUTH_MODE_LOGIN))
            .child(
              text("登录")
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                  let p = palette(dark_mode.get());
                  if auth_mode.get() == AUTH_MODE_LOGIN {
                    p.text_primary
                  } else {
                    p.text_muted
                  }
                }),
            ),
        )
        .child(
          button()
            .test_id("world-auth-mode-register")
            .accessibility_label("注册模式")
            .flex_1()
            .h(px(32.0))
            .rounded(px(6.0))
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .disabled(move || busy.get())
            .border_1()
            .border_color(move || {
              if auth_mode.get() == AUTH_MODE_REGISTER {
                palette(dark_mode.get()).border_normal
              } else {
                transparent
              }
            })
            .bg(move || {
              if auth_mode.get() == AUTH_MODE_REGISTER {
                palette(dark_mode.get()).surface_base
              } else {
                transparent
              }
            })
            .on_press(move || auth_mode.set(AUTH_MODE_REGISTER))
            .child(
              text("注册")
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                  let p = palette(dark_mode.get());
                  if auth_mode.get() == AUTH_MODE_REGISTER {
                    p.text_primary
                  } else {
                    p.text_muted
                  }
                }),
            ),
        ),
    )
    .child(view().h(px(16.0)))
    .child(
      // Email input
      view()
        .w_full()
        .gap(px(5.0))
        .child(
          text("邮箱")
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).text_muted),
        )
        .child(
          input()
            .test_id("world-auth-email")
            .placeholder("you@example.com")
            .readonly(move || busy.get())
            .submit_behavior(SubmitBehavior::Submit)
            .w_full()
            .px(px(12.0))
            .py(px(9.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_base)
            .text_size(px(13.5))
            .text_color(move || palette(dark_mode.get()).text_primary)
            .value(move || email_draft.get())
            .on_change(move |v: &str| email_draft.set(v.to_owned()))
            .on_submit(move |_: &str| submit_email()),
        ),
    )
    .child(when(move || auth_mode.get() == AUTH_MODE_REGISTER, {
      let submit_name = submit_name.clone();
      move || {
        let submit_name = submit_name.clone();
        view()
          .w_full()
          .pt(px(12.0))
          .gap(px(5.0))
          .child(
            text("昵称")
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_muted),
          )
          .child(
            input()
              .test_id("world-auth-display-name")
              .placeholder("显示名称")
              .readonly(move || busy.get())
              .submit_behavior(SubmitBehavior::Submit)
              .w_full()
              .px(px(12.0))
              .py(px(9.0))
              .rounded(px(7.0))
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .bg(move || palette(dark_mode.get()).surface_base)
              .text_size(px(13.5))
              .text_color(move || palette(dark_mode.get()).text_primary)
              .value(move || display_name_draft.get())
              .on_change(move |v: &str| display_name_draft.set(v.to_owned()))
              .on_submit(move |_: &str| submit_name()),
          )
      }
    }))
    .child(view().h(px(12.0)))
    .child(
      // Masked password input
      view()
        .w_full()
        .gap(px(5.0))
        .child(
          text("密码")
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).text_muted),
        )
        .child(
          input()
            .test_id("world-auth-password")
            .placeholder("输入密码")
            .masked(true)
            .readonly(move || busy.get())
            .submit_behavior(SubmitBehavior::Submit)
            .w_full()
            .px(px(12.0))
            .py(px(9.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_base)
            .text_size(px(13.5))
            .text_color(move || palette(dark_mode.get()).text_primary)
            .value(move || password_draft.get())
            .on_change(move |v: &str| password_draft.set(v.to_owned()))
            .on_submit(move |_: &str| submit_pwd()),
        ),
    )
    .child(view().h(px(16.0)))
    .child(
      // Primary submit button
      button()
        .test_id("world-auth-submit")
        .accessibility_label("提交登录或注册")
        .w_full()
        .h(px(40.0))
        .rounded(px(8.0))
        .items_center()
        .justify_center()
        .disabled(move || busy.get())
        .bg(move || palette(dark_mode.get()).accent)
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
        .on_press(move || submit_btn())
        .child(
          text(move || {
            if busy.get() {
              "处理中…"
            } else if auth_mode.get() == AUTH_MODE_REGISTER {
              "注册"
            } else {
              "登录"
            }
          })
          .text_size(px(13.5))
          .font_weight(FontWeight::MEDIUM)
          .text_color(white()),
        ),
    )
    .child(when(
      move || !error_msg.get().is_empty(),
      move || {
        view()
          .test_id("world-auth-error")
          .w_full()
          .mt(px(12.0))
          .p(px(10.0))
          .rounded(px(8.0))
          .border_1()
          .border_color(move || palette(dark_mode.get()).danger)
          .bg(move || palette(dark_mode.get()).danger_wash)
          .flex_row()
          .items_start()
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
    ))
    .child(when(
      move || !info_msg.get().is_empty(),
      move || {
        view()
          .test_id("world-auth-info")
          .w_full()
          .mt(px(12.0))
          .p(px(10.0))
          .rounded(px(8.0))
          .border_1()
          .border_color(move || palette(dark_mode.get()).accent)
          .bg(move || palette(dark_mode.get()).accent_soft)
          .flex_row()
          .items_start()
          .gap(px(7.0))
          .child(dynamic(move || {
            mica_icons::info_icon(palette(dark_mode.get()).accent_hex)
          }))
          .child(
            text(move || info_msg.get())
              .flex_1()
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_primary),
          )
      },
    ))
    .child(when(move || auth_mode.get() == AUTH_MODE_LOGIN, {
      let st_forgot = st_forgot.clone();
      move || {
        let st_forgot = st_forgot.clone();
        view().w_full().pt(px(8.0)).flex_row().child(
          button()
            .test_id("world-auth-forgot")
            .accessibility_label("忘记密码")
            .h(px(28.0))
            .px(px(6.0))
            .rounded(px(5.0))
            .disabled(move || busy.get())
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || {
              if st_forgot.busy.get_untracked() {
                return;
              }
              let origin = effective_cloud_origin_untracked(&st_forgot);
              let email = email_draft.get_untracked().trim().to_string();
              (st_forgot.on_forgot_password)(origin, email);
            })
            .child(
              text("忘记密码？")
                .text_size(px(12.5))
                .text_color(move || palette(dark_mode.get()).accent),
            ),
        )
      }
    }))
}

/// On-device local mode tab panel matching `sign_in_pane.dart: _localPanel`.
fn local_pane(state: WorldSwitcherState) -> impl IntoElement {
  let dark_mode = state.dark_mode;
  let on_enter_local = state.on_enter_local;

  view()
    .test_id("world-local-panel")
    .w_full()
    .child(
      text("在此设备上使用")
        .text_size(px(22.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || palette(dark_mode.get()).text_primary),
    )
    .child(view().h(px(8.0)))
    .child(
      text("无需账户，内容只保存在这台设备上。随时可以再登录云端账户来同步和协作。")
        .text_size(px(13.5))
        .text_color(move || palette(dark_mode.get()).text_muted),
    )
    .child(view().h(px(24.0)))
    .child(
      button()
        .test_id("world-enter-local-btn")
        .accessibility_label("开始使用本地模式")
        .w_full()
        .h(px(46.0))
        .rounded(px(8.0))
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).accent)
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
        .on_press(move || on_enter_local())
        .child(
          text("开始使用")
            .text_size(px(14.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(white()),
        ),
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

fn hero_check_icon(color: &str) -> Svg {
  stroked_icon(13.0, color, r##"<path d="M4.5 10.5L8.2 14.2L15.5 6.3"/>"##)
}

fn computer_icon(color: &str) -> Svg {
  stroked_icon(
    15.0,
    color,
    r##"<rect x="3" y="4" width="14" height="9.5" rx="1.6"/><path d="M7.5 16.5H12.5M10 13.5V16.5"/>"##,
  )
}

fn refresh_icon(color: &str) -> Svg {
  stroked_icon(
    16.0,
    color,
    r##"<path d="M15.8 10A5.8 5.8 0 1 1 14 5.8L16.2 7.8M16.2 4.5V7.8H12.9"/>"##,
  )
}

fn chevron_up_icon(color: &str) -> Svg {
  stroked_icon(12.0, color, r##"<path d="M5 12.5L10 7.5L15 12.5"/>"##)
}

fn check_circle_icon(color: &str) -> Svg {
  stroked_icon(
    16.0,
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
