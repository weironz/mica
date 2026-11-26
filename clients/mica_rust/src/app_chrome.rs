//! Application shell chrome widgets:
//! - Unified 34px frameless top bar (`TitleBarStyle::Hidden` + `window_drag`) & DocTabStrip
//! - Document loading skeleton overlay
//! - New-Tab `+` 2-item popup menu
//! - Embedded document board fullscreen modal with 3-way semantic merge

use std::{cell::RefCell, rc::Rc};

use mica_board::BoardSession;
use nsh_base::button;
use nsh_core::{element::desktop::title_bar, geometry::px, prelude::*};
use uuid::Uuid;

use crate::{
  MODE_DOC, OpenTab, atomic_blocks, board_view, editor_bridge::MicaDocumentBridge, mica_icons,
  mica_theme::palette, responsive_shell, tooltip,
};

#[derive(Clone)]
pub(crate) struct EmbeddedBoardEdit {
  pub(crate) block_pos: usize,
  pub(crate) base_json: String,
  pub(crate) session: Rc<RefCell<BoardSession>>,
  pub(crate) rev: Signal<u64>,
}

impl PartialEq for EmbeddedBoardEdit {
  fn eq(&self, other: &Self) -> bool {
    self.block_pos == other.block_pos
      && self.base_json == other.base_json
      && Rc::ptr_eq(&self.session, &other.session)
  }
}

pub(crate) struct TopBarProps {
  pub(crate) dark_mode: Signal<bool>,
  pub(crate) is_narrow: Memo<bool>,
  pub(crate) sidebar_open: Signal<bool>,
  pub(crate) sidebar_width: Signal<f32>,
  pub(crate) app_menu_open: Signal<bool>,
  pub(crate) workspace_menu_open: Signal<bool>,
  pub(crate) account_menu_open: Signal<bool>,
  pub(crate) new_tab_menu_open: Signal<bool>,
  pub(crate) active_title: Signal<String>,
  pub(crate) narrow_tab_count: Signal<usize>,
  pub(crate) narrow_tabs_open: Signal<bool>,
  pub(crate) open_tabs: Signal<Vec<OpenTab>>,
  pub(crate) active_tab_idx: Signal<usize>,
  pub(crate) active_mode: Signal<u8>,
  pub(crate) can_go_back: Memo<bool>,
  pub(crate) can_go_forward: Memo<bool>,
  pub(crate) on_nav_back: Rc<dyn Fn()>,
  pub(crate) on_nav_forward: Rc<dyn Fn()>,
  pub(crate) on_select_tab: Rc<dyn Fn(Uuid, String)>,
  pub(crate) on_close_tab: Rc<dyn Fn(usize)>,
}

/// 1. Unified 34px frameless top bar (`TitleBarStyle::Hidden` + `window_drag`).
pub(crate) fn render_top_bar(props: TopBarProps) -> impl IntoElement {
  let TopBarProps {
    dark_mode,
    is_narrow,
    sidebar_open,
    sidebar_width,
    app_menu_open,
    workspace_menu_open,
    account_menu_open,
    new_tab_menu_open,
    active_title,
    narrow_tab_count,
    narrow_tabs_open,
    open_tabs,
    active_tab_idx,
    active_mode,
    can_go_back,
    can_go_forward,
    on_nav_back,
    on_nav_forward,
    on_select_tab,
    on_close_tab,
  } = props;

  view()
    .h(px(34.0))
    .w_full()
    .flex_row()
    .items_end()
    .window_drag(true)
    .bg(move || palette(dark_mode.get()).surface_sunken)
    .child(
      view()
        .w(move || {
          let raw = if !is_narrow.get() && sidebar_open.get() {
            sidebar_width.get() + 7.0
          } else {
            0.0
          };
          px(raw.max(84.0))
        })
        .h_full()
        .pl(move || {
          let inset = title_bar().left.0;
          px(if inset > 0.0 { inset + 8.0 } else { 8.0 })
        })
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .flex_shrink_0()
        .window_drag(true)
        .child(
          // Clickable top-left M logo opening 1:1 Application Menu
          view().window_drag(false).child(tooltip::with_tooltip(
            button()
              .test_id("app-menu-btn")
              .accessibility_label("应用菜单")
              .w(px(30.0))
              .h(px(30.0))
              .items_center()
              .justify_center()
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                tooltip::hide_tooltip();
                workspace_menu_open.set(false);
                account_menu_open.set(false);
                app_menu_open.set(!app_menu_open.get_untracked());
              })
              .child(mica_icons::mica_logo()),
            "应用菜单",
          )),
        )
        .child(
          view().window_drag(false).child(tooltip::with_dynamic_tooltip(
            button()
              .test_id("toggle-sidebar-btn")
              .accessibility_label("折叠或展开左侧边栏")
              .w(px(30.0))
              .h(px(30.0))
              .items_center()
              .justify_center()
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                tooltip::hide_tooltip();
                sidebar_open.set(!sidebar_open.get_untracked());
              })
              .child(dynamic(move || {
                mica_icons::sidebar_toggle(palette(dark_mode.get()).text_muted_hex)
              })),
            move || {
              if sidebar_open.get_untracked() {
                "折叠侧边栏"
              } else {
                "展开侧边栏"
              }
            },
          )),
        ),
    )
    .child(
      // DocTabStrip: per-tab `<` `>` history + scrollable tabs (`max_w 200px`) + `+` menu
      view()
        .flex_1()
        .min_w(px(0.0))
        .h_full()
        .pl_1()
        .pr(move || {
          let inset = title_bar().right.0;
          px(if inset > 0.0 { inset + 8.0 } else { 8.0 })
        })
        .flex_row()
        .items_end()
        .gap_1()
        .window_drag(true)
        .child(
          view()
            .h_full()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .pr_1()
            .window_drag(false)
            .child(tooltip::with_tooltip(
              button()
                .test_id("nav-back-btn")
                .accessibility_label("后退")
                .w(px(26.0))
                .h(px(26.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  tooltip::hide_tooltip();
                  on_nav_back();
                })
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  mica_icons::chevron_left(if can_go_back.get() {
                    p.text_muted_hex
                  } else {
                    p.text_ghost_hex
                  })
                })),
              "后退",
            ))
            .child(tooltip::with_tooltip(
              button()
                .test_id("nav-forward-btn")
                .accessibility_label("前进")
                .w(px(26.0))
                .h(px(26.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  tooltip::hide_tooltip();
                  on_nav_forward();
                })
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  mica_icons::chevron_right(if can_go_forward.get() {
                    p.text_muted_hex
                  } else {
                    p.text_ghost_hex
                  })
                })),
              "前进",
            )),
        )
        .child(dynamic(move || {
          if is_narrow.get() {
            return view()
              .h_full()
              .items_center()
              .justify_center()
              .window_drag(false)
              .child(responsive_shell::narrow_tab_switcher_button(
                active_title,
                narrow_tab_count,
                narrow_tabs_open,
                dark_mode,
              ))
              .into_element();
          }
          let tabs = open_tabs.get();
          let active_idx = active_tab_idx.get();
          let mode = active_mode.get();
          let p = palette(dark_mode.get());
          let mut strip = view().flex_row().items_end().gap(px(2.0)).window_drag(false);
          for (idx, tab) in tabs.iter().enumerate() {
            let is_active = mode == MODE_DOC && idx == active_idx;
            let sel_cb = on_select_tab.clone();
            let close_cb = on_close_tab.clone();
            let doc_id = tab.doc_id;
            let title_for_sel = tab.title.clone();
            let title_display = if tab.title.trim().is_empty() {
              "未命名页面".to_string()
            } else {
              tab.title.clone()
            };
            let can_close = tabs.len() > 1;

            let mut tab_el = view()
              .h(px(28.0))
              .max_w(px(200.0))
              .px_3()
              .flex_row()
              .items_center()
              .gap_2()
              .rounded_t(px(8.0))
              .bg(if is_active {
                p.surface_base
              } else {
                p.surface_sunken
              });

            tab_el = tab_el.child(
              button()
                .test_id(&format!("doc-tab-{idx}"))
                .accessibility_label(format!("切换到标签 {}", title_display))
                .on_press(move || {
                  active_tab_idx.set(idx);
                  sel_cb(doc_id, title_for_sel.clone());
                })
                .child(
                  text(title_display)
                    .text_size(px(12.0))
                    .font_weight(if is_active {
                      FontWeight::MEDIUM
                    } else {
                      FontWeight::NORMAL
                    })
                    .text_color(if is_active {
                      p.text_primary
                    } else {
                      p.text_muted
                    })
                    .truncate(),
                ),
            );
            if can_close && is_active {
              tab_el = tab_el.child(
                button()
                  .test_id(&format!("doc-tab-close-{idx}"))
                  .accessibility_label("关闭标签页")
                  .w(px(18.0))
                  .h(px(18.0))
                  .items_center()
                  .justify_center()
                  .rounded(px(4.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || close_cb(idx))
                  .child(mica_icons::close_sm(p.text_faint_hex)),
              );
            }
            strip = strip.child(tab_el);
          }
          strip.into_element()
        }))
        .child(
          view()
            .h_full()
            .items_center()
            .justify_center()
            .pl_1()
            .window_drag(false)
            .child(tooltip::with_tooltip(
              button()
                .test_id("tab-new-page-btn")
                .accessibility_label("新建标签页")
                .w(px(28.0))
                .h(px(26.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  tooltip::hide_tooltip();
                  new_tab_menu_open.set(!new_tab_menu_open.get_untracked());
                })
                .child(dynamic(move || {
                  mica_icons::plus(palette(dark_mode.get()).text_muted_hex)
                })),
              "新建标签页",
            )),
        ),
    )
}

/// Full-pane document loading skeleton displayed while initial cloud content fetches.
pub(crate) fn render_doc_loading_skeleton(
  active_title: Signal<String>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  view()
    .test_id("doc-loading-skeleton")
    .absolute()
    .top(px(0.0))
    .left(px(0.0))
    .right(px(0.0))
    .bottom(px(0.0))
    .bg(move || palette(dark_mode.get()).surface_base)
    .items_center()
    .pt(px(72.0))
    .px(px(32.0))
    .child(
      view()
        .flex_col()
        .w_full()
        .max_w(px(640.0))
        .gap(px(14.0))
        .child(
          text(move || {
            let t = active_title.get();
            if t.trim().is_empty() {
              "正在加载页面内容…".to_string()
            } else {
              format!("正在加载「{t}」…")
            }
          })
          .text_size(px(13.0))
          .text_color(move || palette(dark_mode.get()).text_muted),
        )
        .child(
          view()
            .w(px(260.0))
            .h(px(22.0))
            .rounded(px(6.0))
            .bg(move || palette(dark_mode.get()).surface_sunken),
        )
        .child(
          view()
            .w_full()
            .h(px(14.0))
            .rounded(px(4.0))
            .bg(move || palette(dark_mode.get()).surface_sunken),
        )
        .child(
          view()
            .w_full()
            .max_w(px(480.0))
            .h(px(14.0))
            .rounded(px(4.0))
            .bg(move || palette(dark_mode.get()).surface_sunken),
        )
        .child(
          view()
            .w_full()
            .max_w(px(380.0))
            .h(px(14.0))
            .rounded(px(4.0))
            .bg(move || palette(dark_mode.get()).surface_sunken),
        ),
    )
}

/// Slim top warning banner displayed above cloud workspaces when the saved
/// session has expired, preventing silent offline staleness.
pub(crate) fn render_cloud_relogin_banner(
  dark_mode: Signal<bool>,
  on_relogin: Rc<dyn Fn()>,
) -> impl IntoElement {
  view()
    .test_id("cloud-relogin-banner")
    .w_full()
    .h(px(36.0))
    .px_4()
    .flex_row()
    .items_center()
    .justify_between()
    .gap_3()
    .bg(move || palette(dark_mode.get()).warning_wash)
    .border_b_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .child(
      text("云端登录已过期 — 当前仅显示本地缓存，无法同步或加载未缓存的云端页面（云端数据安全无损）")
        .flex_1()
        .text_size(px(12.5))
        .text_color(move || palette(dark_mode.get()).warning)
        .truncate(),
    )
    .child(
      button()
        .test_id("cloud-relogin-btn")
        .h(px(24.0))
        .px_3()
        .rounded(px(5.0))
        .bg(move || palette(dark_mode.get()).accent)
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
        .on_press(move || on_relogin())
        .child(
          text("重新登录")
            .text_size(px(12.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || palette(dark_mode.get()).on_primary),
        ),
    )
}

/// Full-pane error/relogin card shown when an uncached cloud page cannot be
/// fetched, so an expired session or network failure never masquerades as a
/// blank empty document.
pub(crate) fn render_doc_load_error_overlay(
  active_title: Signal<String>,
  doc_load_error: Signal<Option<String>>,
  is_cloud_signed_in: Signal<bool>,
  dark_mode: Signal<bool>,
  on_relogin: Rc<dyn Fn()>,
  on_retry: Rc<dyn Fn()>,
) -> impl IntoElement {
  view()
    .test_id("doc-load-error-overlay")
    .absolute()
    .top(px(0.0))
    .left(px(0.0))
    .right(px(0.0))
    .bottom(px(0.0))
    .bg(move || palette(dark_mode.get()).surface_base)
    .items_center()
    .justify_center()
    .px(px(32.0))
    .child(
      view()
        .flex_col()
        .w_full()
        .max_w(px(460.0))
        .p_6()
        .gap_3()
        .rounded(px(10.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_raised)
        .child(
          text(move || {
            let t = active_title.get();
            if t.trim().is_empty() {
              "暂无法加载云端页面".to_string()
            } else {
              format!("暂无法加载「{t}」")
            }
          })
          .text_size(px(16.0))
          .font_weight(FontWeight::SEMIBOLD)
          .text_color(move || palette(dark_mode.get()).text_primary),
        )
        .child(
          text(move || {
            doc_load_error
              .get()
              .unwrap_or_else(|| "云端登录已过期，请重新登录后加载该页面内容。".to_string())
          })
          .text_size(px(13.0))
          .text_color(move || palette(dark_mode.get()).text_muted),
        )
        .child(
          view()
            .flex_row()
            .items_center()
            .gap_2()
            .pt_2()
            .child(when(
              move || !is_cloud_signed_in.get(),
              {
                let relogin = on_relogin.clone();
                move || {
                  let relogin = relogin.clone();
                  button()
                    .test_id("doc-error-relogin-btn")
                    .h(px(32.0))
                    .px_4()
                    .rounded(px(6.0))
                    .bg(move || palette(dark_mode.get()).accent)
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
                    .on_press(move || relogin())
                    .child(
                      text("重新登录云端")
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || palette(dark_mode.get()).on_primary),
                    )
                }
              },
            ))
            .child(when(
              move || is_cloud_signed_in.get(),
              {
                let retry = on_retry.clone();
                move || {
                  let retry = retry.clone();
                  button()
                    .test_id("doc-error-retry-btn")
                    .h(px(32.0))
                    .px_4()
                    .rounded(px(6.0))
                    .bg(move || palette(dark_mode.get()).accent)
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
                    .on_press(move || retry())
                    .child(
                      text("重试加载")
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || palette(dark_mode.get()).on_primary),
                    )
                }
              },
            )),
        ),
    )
}

/// New-Tab `+` 2-item popup menu (`新建页面` / `打开已有页面…`).
pub(crate) fn render_new_tab_menu_popover(
  new_tab_menu_open: Signal<bool>,
  dark_mode: Signal<bool>,
  on_new_page: Rc<dyn Fn()>,
  on_open_search: Rc<dyn Fn(Option<String>, bool)>,
) -> impl IntoElement {
  view()
    .test_id("new-tab-menu-popover")
    .absolute()
    .top(px(36.0))
    .left(px(340.0))
    .w(px(172.0))
    .p_1()
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .z_index(45)
    .child(
      button()
        .test_id("new-tab-create-page")
        .w_full()
        .h(px(30.0))
        .px_2()
        .flex_row()
        .items_center()
        .gap_2()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || {
          new_tab_menu_open.set(false);
          on_new_page();
        })
        .child(
          text("新建页面")
            .text_size(px(13.0))
            .text_color(move || palette(dark_mode.get()).text_primary),
        ),
    )
    .child(
      button()
        .test_id("new-tab-open-existing")
        .w_full()
        .h(px(30.0))
        .px_2()
        .flex_row()
        .items_center()
        .gap_2()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || {
          new_tab_menu_open.set(false);
          on_open_search(None, true);
        })
        .child(
          text("打开已有页面…")
            .text_size(px(13.0))
            .text_color(move || palette(dark_mode.get()).text_primary),
        ),
    )
}

/// Embedded document board fullscreen modal mounted below the 34px titlebar.
pub(crate) fn render_embedded_board_modal(
  embedded_board_edit: Signal<Option<EmbeddedBoardEdit>>,
  bridge: Rc<MicaDocumentBridge>,
  dark_mode: Signal<bool>,
) -> AnyElement {
  let Some(ed) = embedded_board_edit.get() else {
    return view().into_element();
  };
  let bs = ed.session.clone();
  let bs_close = ed.session.clone();
  let base_json_persist = ed.base_json.clone();
  let base_json_close = ed.base_json.clone();
  let pos = ed.block_pos;
  let br_persist = bridge.clone();
  let br_close = bridge;
  view()
    .test_id("embedded-board-modal")
    .absolute()
    .top(px(34.0))
    .left(px(0.0))
    .right(px(0.0))
    .bottom(px(0.0))
    .z_index(60)
    .bg(move || palette(dark_mode.get()).surface_base)
    .child(board_view::board_view_with_rev_and_close(
      bs,
      ed.rev,
      dark_mode,
      Rc::new(move |local_json: String| {
        let merged = atomic_blocks::merge_embedded_board_json(
          &base_json_persist,
          &local_json,
          &base_json_persist,
        );
        br_persist.update_block_source_at(pos, &merged);
      }),
      Some(Rc::new(move || {
        let local_json = bs_close.borrow().export_compact_json();
        let merged = atomic_blocks::merge_embedded_board_json(
          &base_json_close,
          &local_json,
          &base_json_close,
        );
        br_close.update_block_source_at(pos, &merged);
        embedded_board_edit.set(None);
      })),
    ))
    .into_element()
}
