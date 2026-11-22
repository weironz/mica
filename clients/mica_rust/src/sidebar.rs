//! Minimal Notion-inspired warm-gray workspace sidebar matching Mica's original
//! visual & interaction spec (`docs/plans/ui-visual-refresh.md` + `account_menu.dart`):
//! - Clickable workspace switcher (`Mica 本地工作区 ↕`) -> opens workspace dropdown popover.
//! - Recessed search pill (`搜索...  Ctrl+Shift+F`) -> opens modal search dialog.
//! - Collapsible `页面 ⌄` tree + lightweight `+` / `folder+` icon buttons.
//! - Quiet collapsible Recycle Bin (`回收站 (N)`).
//! - Clickable bottom account row (`L local`) -> opens account menu popover + `⚙` Settings modal.

use std::{cell::RefCell, rc::Rc};

use mica_client::PageTreeModel;
use nsh_base::button;
use nsh_core::{element::Position, geometry::px, prelude::*};
use uuid::Uuid;

use crate::{mica_icons, mica_theme::palette};

pub struct SidebarCallbacks {
  pub on_select_doc: Rc<dyn Fn(Uuid, String)>,
  pub on_new_page: Rc<dyn Fn()>,
  pub on_new_folder: Rc<dyn Fn()>,
  pub on_open_board: Rc<dyn Fn()>,
  pub on_restore_view: Rc<dyn Fn(Uuid)>,
  pub on_open_search: Rc<dyn Fn()>,
  pub on_open_settings: Rc<dyn Fn()>,
  pub on_toggle_workspace_menu: Rc<dyn Fn()>,
  pub on_toggle_account_menu: Rc<dyn Fn()>,
}

pub fn sidebar_view(
  tree: Rc<RefCell<PageTreeModel>>,
  tree_rev: Signal<u64>,
  active_doc: Signal<Option<Uuid>>,
  workspace_name: Signal<String>,
  account_name: Signal<String>,
  dark_mode: Signal<bool>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  let tree_count = tree.clone();
  let tree_keys = tree.clone();
  let tree_row = tree.clone();
  let tree_trash = tree.clone();
  let show_pages = signal(true);
  let show_trash = signal(false);

  let cb_new_page = cb.clone();
  let cb_new_folder = cb.clone();
  let cb_board = cb.clone();
  let cb_search = cb.clone();
  let cb_settings = cb.clone();
  let cb_restore = cb.clone();
  let cb_select = cb.clone();
  let cb_ws_menu = cb.clone();
  let cb_acct_menu = cb.clone();

  view()
    .w(px(244.0))
    .h_full()
    .flex_shrink_0()
    .border_r_1()
    .border_color(move || palette(dark_mode.get()).border_subtle)
    .bg(move || palette(dark_mode.get()).surface_raised)
    .child(
      // 1. Clickable Workspace Switcher row + Graph/Whiteboard shortcuts
      view()
        .h(px(40.0))
        .px_2()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_1()
        .child(
          button()
            .test_id("workspace-switcher-btn")
            .accessibility_label("切换或新建工作区")
            .flex_1()
            .min_w(px(0.0))
            .h(px(30.0))
            .px_2()
            .flex_row()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || (cb_ws_menu.on_toggle_workspace_menu)())
            .child(dynamic(move || {
              mica_icons::cloud(palette(dark_mode.get()).text_muted_hex)
            }))
            .child(
              text(move || workspace_name.get())
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || palette(dark_mode.get()).text_primary)
                .truncate(),
            )
            .child(dynamic(move || {
              mica_icons::chevrons_up_down(palette(dark_mode.get()).text_faint_hex)
            })),
        )
        .child(
          view()
            .flex_row()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .child(
              button()
                .test_id("mode-board")
                .accessibility_label("打开无限白板与思维导图")
                .w(px(26.0))
                .h(px(26.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || (cb_board.on_open_board)())
                .child(dynamic(move || {
                  mica_icons::graph_nodes(palette(dark_mode.get()).text_muted_hex)
                })),
            ),
        ),
    )
    .child(
      // 2. Quiet recessed search pill ("搜索...  Ctrl+Shift+F") -> modal search
      view().px_3().pb_2().child(
        button()
          .test_id("open-search-btn")
          .accessibility_label("搜索工作区")
          .w_full()
          .h(px(30.0))
          .px_2()
          .flex_row()
          .items_center()
          .justify_between()
          .rounded(px(6.0))
          .bg(move || palette(dark_mode.get()).surface_sunken)
          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
          .on_press(move || (cb_search.on_open_search)())
          .child(
            view()
              .flex_row()
              .items_center()
              .gap_2()
              .child(dynamic(move || {
                mica_icons::search(palette(dark_mode.get()).text_ghost_hex)
              }))
              .child(
                text("搜索...")
                  .text_size(px(13.0))
                  .text_color(move || palette(dark_mode.get()).text_ghost),
              ),
          )
          .child(
            text("Ctrl+Shift+F")
              .text_size(px(11.0))
              .text_color(move || palette(dark_mode.get()).text_faint),
          ),
      ),
    )
    .child(
      // 3. Collapsible Section header ("页面 ⌄" + lightweight `+` & `folder+` icons)
      view()
        .h(px(28.0))
        .px_3()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
          button()
            .test_id("toggle-pages-section")
            .accessibility_label("展开或收起页面列表")
            .flex_row()
            .items_center()
            .gap_1()
            .px_1()
            .py_1()
            .rounded(px(4.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || show_pages.set(!show_pages.get_untracked()))
            .child(
              text("页面")
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || palette(dark_mode.get()).text_faint),
            )
            .child(dynamic(move || {
              let hex = palette(dark_mode.get()).text_faint_hex;
              if show_pages.get() {
                mica_icons::chevron_down(hex)
              } else {
                mica_icons::chevron_right(hex)
              }
            })),
        )
        .child(
          view()
            .flex_row()
            .items_center()
            .gap_1()
            .child(
              button()
                .test_id("new-page-btn")
                .accessibility_label("新建页面")
                .w(px(24.0))
                .h(px(24.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || (cb_new_page.on_new_page)())
                .child(dynamic(move || {
                  mica_icons::plus(palette(dark_mode.get()).text_faint_hex)
                })),
            )
            .child(
              button()
                .test_id("new-folder-btn")
                .accessibility_label("新建文件夹")
                .w(px(24.0))
                .h(px(24.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || (cb_new_folder.on_new_folder)())
                .child(dynamic(move || {
                  mica_icons::folder_plus(palette(dark_mode.get()).text_faint_hex)
                })),
            ),
        ),
    )
    .child(
      // 4. Virtualized compact page & folder tree
      view().flex_1().px_2().py_1().child(when(
        move || show_pages.get(),
        move || {
          let tc = tree_count.clone();
          let tk = tree_keys.clone();
          let tr = tree_row.clone();
          let cbs = cb_select.clone();
          virtual_list(
            move || {
              tree_rev.get();
              tc.borrow().visible_rows().len()
            },
            move |at: Position| {
              let t_read = tr.clone();
              let t_press = tr.clone();
              let cb_sel = cbs.clone();

              let row_info = computed(move || {
                tree_rev.get();
                t_read.borrow().visible_rows().get(at.index()).cloned()
              });

              let is_active = computed(move || {
                let curr = active_doc.get();
                row_info
                  .get()
                  .is_some_and(|r| !r.is_folder && Some(r.object_id) == curr)
              });

              let row_label = computed(move || {
                row_info
                  .get()
                  .map(|r| {
                    if r.name.trim().is_empty() {
                      "无标题".to_string()
                    } else {
                      r.name
                    }
                  })
                  .unwrap_or_default()
              });

              let indent_px = computed(move || {
                row_info
                  .get()
                  .map(|r| (r.depth as f32) * 14.0)
                  .unwrap_or(0.0)
              });

              button()
                .w_full()
                .h(px(30.0))
                .pl(move || px(8.0 + indent_px.get()))
                .pr_2()
                .flex_row()
                .items_center()
                .gap_2()
                .rounded(px(6.0))
                .bg(move || {
                  let p = palette(dark_mode.get());
                  if is_active.get() {
                    p.surface_hover
                  } else {
                    p.surface_raised
                  }
                })
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  if let Some(row) = row_info.get_untracked() {
                    if row.is_folder {
                      t_press.borrow_mut().toggle_expanded(row.view_id);
                      tree_rev.set(tree_rev.get_untracked() + 1);
                    } else {
                      (cb_sel.on_select_doc)(row.object_id, row.name);
                    }
                  }
                })
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  let is_folder = row_info.get().is_some_and(|r| r.is_folder);
                  let icon_hex = if is_active.get() {
                    p.text_primary_hex
                  } else {
                    p.text_muted_hex
                  };
                  if is_folder {
                    mica_icons::folder(icon_hex)
                  } else {
                    mica_icons::file_doc(icon_hex)
                  }
                }))
                .child(
                  text(move || row_label.get())
                    .flex_1()
                    .text_size(px(13.5))
                    .text_color(move || {
                      let p = palette(dark_mode.get());
                      if is_active.get() {
                        p.text_primary
                      } else {
                        p.text_muted
                      }
                    })
                    .truncate(),
                )
            },
          )
          .keys(move || {
            tree_rev.get();
            tk.borrow()
              .visible_rows()
              .into_iter()
              .map(|r| r.view_id.to_string())
              .collect::<Vec<_>>()
          })
          .estimated_item_size(px(30.0))
          .flex_1()
        },
      )),
    )
    .child(
      // 5. Quiet collapsible Recycle Bin
      view()
        .px_2()
        .py_1()
        .child(
          button()
            .test_id("toggle-recycle-bin")
            .accessibility_label("展开或收起回收站")
            .w_full()
            .h(px(28.0))
            .px_2()
            .flex_row()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || show_trash.set(!show_trash.get_untracked()))
            .child(dynamic(move || {
              mica_icons::trash_icon(palette(dark_mode.get()).text_faint_hex)
            }))
            .child(
              text({
                let t_tr = tree_trash.clone();
                move || {
                  tree_rev.get();
                  let count = t_tr.borrow().trashed_views().len();
                  format!("回收站 ({count})")
                }
              })
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_faint),
            ),
        )
        .child(when(move || show_trash.get(), {
          let t_tr = tree_trash.clone();
          let restore_cb = cb_restore.clone();
          move || {
            let t_inner = t_tr.clone();
            let r_inner = restore_cb.clone();
            dynamic(move || {
              tree_rev.get();
              let items: Vec<(Uuid, String)> = t_inner
                .borrow()
                .trashed_views()
                .into_iter()
                .map(|v| (v.id, v.name.clone()))
                .collect();
              let mut box_el = view().w_full().pl_2().py_1().gap_1();
              if items.is_empty() {
                return box_el.child(
                  text("回收站为空")
                    .text_size(px(12.0))
                    .text_color(move || palette(dark_mode.get()).text_ghost),
                );
              }
              for (idx, (vid, raw_name)) in items.into_iter().enumerate() {
                let restore = r_inner.clone();
                let name = if raw_name.trim().is_empty() {
                  "无标题".to_string()
                } else {
                  raw_name
                };
                box_el = box_el.child(
                  view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .child(
                      text(name.clone())
                        .text_size(px(12.0))
                        .text_color(move || palette(dark_mode.get()).text_muted)
                        .truncate(),
                    )
                    .child(
                      button()
                        .test_id(&format!("restore-trash-{idx}"))
                        .accessibility_label(format!("恢复页面 {name}"))
                        .px_2()
                        .py_1()
                        .rounded(px(6.0))
                        .bg(move || palette(dark_mode.get()).surface_sunken)
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                        .on_press(move || (restore.on_restore_view)(vid))
                        .child(
                          text("恢复")
                            .text_size(px(11.0))
                            .text_color(move || palette(dark_mode.get()).text_primary),
                        ),
                    ),
                );
              }
              box_el
            })
          }
        })),
    )
    .child(
      // 6. Bottom pinned clickable Account menu trigger + Settings gear modal trigger
      view()
        .h(px(44.0))
        .px_2()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_1()
        .border_t_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .child(
          button()
            .test_id("account-menu-btn")
            .accessibility_label("打开账号菜单")
            .flex_1()
            .h(px(32.0))
            .px_2()
            .flex_row()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || (cb_acct_menu.on_toggle_account_menu)())
            .child(
              view()
                .w(px(24.0))
                .h(px(24.0))
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(move || palette(dark_mode.get()).surface_hover)
                .child(
                  text(move || {
                    account_name
                      .get()
                      .chars()
                      .next()
                      .map(|c| c.to_uppercase().to_string())
                      .unwrap_or_else(|| "M".to_string())
                  })
                  .text_size(px(11.0))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            )
            .child(
              text(move || account_name.get())
                .text_size(px(13.0))
                .text_color(move || palette(dark_mode.get()).text_primary)
                .truncate(),
            ),
        )
        .child(
          button()
            .test_id("open-settings-btn")
            .accessibility_label("打开设置")
            .w(px(28.0))
            .h(px(28.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || (cb_settings.on_open_settings)())
            .child(dynamic(move || {
              mica_icons::settings_gear(palette(dark_mode.get()).text_muted_hex)
            })),
        ),
    )
}
