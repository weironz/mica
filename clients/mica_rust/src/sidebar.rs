//! Minimal Notion-inspired warm-gray workspace sidebar matching Mica's original
//! visual & interaction spec (`docs/plans/ui-visual-refresh.md` + `account_menu.dart`):
//! - Reactive width (`sidebar_width: Signal<f32>`) with right-edge drag-to-resize handle (200..380px).
//! - Clickable workspace switcher (`Mica 本地工作区 ↕`) -> opens workspace dropdown popover.
//! - Recessed search pill (`搜索...  Ctrl+Shift+F`) -> opens modal search dialog.
//! - Collapsible `收藏` (Favorites) section above `页面` (auto-hidden when empty).
//! - Collapsible `页面 ⌄` tree with folder expand/collapse chevrons, per-row `⋯` quick actions
//!   (inline auto-focused rename, move up/down, trash), and lightweight `+` / `folder+` buttons.
//! - Quiet collapsible Recycle Bin (`回收站 (N)`).
//! - Clickable bottom account row (`L local`) -> opens account menu popover + `⚙` Settings modal.

use std::{cell::RefCell, rc::Rc};

use mica_client::PageTreeModel;
use nsh_base::{HandleEdge, ResizeHandleState, button, input, resize_handle};
use nsh_core::{geometry::Axis, keymap::When, prelude::*};
use uuid::Uuid;

use crate::{mica_icons, mica_theme::palette};

pub const SIDEBAR_MIN_WIDTH: f32 = 200.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 380.0;

pub struct SidebarCallbacks {
  pub on_select_doc: Rc<dyn Fn(Uuid, String)>,
  pub on_new_page: Rc<dyn Fn()>,
  pub on_new_folder: Rc<dyn Fn()>,
  pub on_rename_view: Rc<dyn Fn(Uuid, String)>,
  pub on_trash_view: Rc<dyn Fn(Uuid)>,
  pub on_move_view: Rc<dyn Fn(Uuid, i32)>,
  pub on_open_board: Rc<dyn Fn()>,
  pub on_restore_view: Rc<dyn Fn(Uuid)>,
  pub on_open_search: Rc<dyn Fn()>,
  pub on_open_settings: Rc<dyn Fn()>,
  pub on_toggle_workspace_menu: Rc<dyn Fn()>,
  pub on_toggle_account_menu: Rc<dyn Fn()>,
}

struct TreeRowContext {
  tree: Rc<RefCell<PageTreeModel>>,
  tree_rev: Signal<u64>,
  active_doc: Signal<Option<Uuid>>,
  dark_mode: Signal<bool>,
  open_menu_view: Signal<Option<Uuid>>,
  renaming_view: Signal<Option<Uuid>>,
  rename_draft: Signal<String>,
  cb: Rc<SidebarCallbacks>,
}

#[allow(clippy::too_many_arguments)]
pub fn sidebar_view(
  tree: Rc<RefCell<PageTreeModel>>,
  tree_rev: Signal<u64>,
  active_doc: Signal<Option<Uuid>>,
  favorite_pages: Signal<Vec<(Uuid, String)>>,
  sidebar_width: Signal<f32>,
  workspace_name: Signal<String>,
  account_name: Signal<String>,
  dark_mode: Signal<bool>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  let show_favorites = signal(true);
  let show_pages = signal(true);
  let show_trash = signal(false);
  let open_menu_view: Signal<Option<Uuid>> = signal(None);
  let renaming_view: Signal<Option<Uuid>> = signal(None);
  let rename_draft = signal(String::new());

  // Esc cancels active inline rename or closes an open per-row quick-action menu.
  effect(move || {
    if renaming_view.get().is_none() && open_menu_view.get().is_none() {
      return;
    }
    on_key_with(
      "escape",
      KeyOptions {
        when: When::Always,
        repeat: false,
        ..KeyOptions::default()
      },
      move |_| {
        renaming_view.set(None);
        open_menu_view.set(None);
      },
    );
  });

  let tree_ctx = Rc::new(TreeRowContext {
    tree: tree.clone(),
    tree_rev,
    active_doc,
    dark_mode,
    open_menu_view,
    renaming_view,
    rename_draft,
    cb: cb.clone(),
  });

  view()
    .relative()
    .w(move || px(sidebar_width.get().clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH)))
    .h_full()
    .flex_shrink_0()
    .border_r_1()
    .border_color(move || palette(dark_mode.get()).border_subtle)
    .bg(move || palette(dark_mode.get()).surface_raised)
    .child(workspace_header(workspace_name, dark_mode, cb.clone()))
    .child(search_pill(dark_mode, cb.clone()))
    .child(favorites_section(
      favorite_pages,
      active_doc,
      show_favorites,
      dark_mode,
      cb.clone(),
    ))
    .child(pages_section_header(show_pages, dark_mode, cb.clone()))
    .child(
      view()
        .flex_1()
        .min_h(px(0.0))
        .px_2()
        .py_1()
        .child(when(move || show_pages.get(), {
          let ctx = tree_ctx.clone();
          move || page_tree_list(ctx.clone())
        })),
    )
    .child(recycle_bin_section(
      tree,
      tree_rev,
      show_trash,
      dark_mode,
      cb.clone(),
    ))
    .child(account_footer(account_name, dark_mode, cb))
    .child(sidebar_resize_handle(sidebar_width, dark_mode))
}

fn workspace_header(
  workspace_name: Signal<String>,
  dark_mode: Signal<bool>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  let cb_ws_menu = cb.clone();
  let cb_board = cb;

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
    )
}

fn search_pill(dark_mode: Signal<bool>, cb: Rc<SidebarCallbacks>) -> impl IntoElement {
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
      .on_press(move || (cb.on_open_search)())
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
  )
}

fn favorites_section(
  favorite_pages: Signal<Vec<(Uuid, String)>>,
  active_doc: Signal<Option<Uuid>>,
  show_favorites: Signal<bool>,
  dark_mode: Signal<bool>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  when(move || !favorite_pages.get().is_empty(), {
    let cb_outer = cb.clone();
    move || {
      let cb_list = cb_outer.clone();
      view()
        .w_full()
        .pb_1()
        .child(
          view()
            .h(px(26.0))
            .px_3()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
              button()
                .test_id("toggle-favorites-section")
                .accessibility_label("展开或收起收藏列表")
                .flex_row()
                .items_center()
                .gap_1()
                .px_1()
                .py_1()
                .rounded(px(4.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || show_favorites.set(!show_favorites.get_untracked()))
                .child(
                  text("收藏")
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || palette(dark_mode.get()).text_faint),
                )
                .child(dynamic(move || {
                  let hex = palette(dark_mode.get()).text_faint_hex;
                  if show_favorites.get() {
                    mica_icons::chevron_down(hex)
                  } else {
                    mica_icons::chevron_right(hex)
                  }
                })),
            ),
        )
        .child(when(move || show_favorites.get(), {
          let cb_rows = cb_list.clone();
          move || {
            let cb_dyn = cb_rows.clone();
            dynamic(move || {
              let items = favorite_pages.get();
              let curr = active_doc.get();
              let p = palette(dark_mode.get());
              let mut list = view().w_full().px_2().gap_1();
              for (idx, (doc_id, raw_name)) in items.into_iter().enumerate() {
                let select_cb = cb_dyn.clone();
                let is_active = curr == Some(doc_id);
                let display_name = if raw_name.trim().is_empty() {
                  "无标题".to_string()
                } else {
                  raw_name.clone()
                };
                let open_name = raw_name;
                list = list.child(
                  button()
                    .test_id(&format!("favorite-page-{idx}"))
                    .accessibility_label(format!("打开收藏页面 {display_name}"))
                    .w_full()
                    .h(px(28.0))
                    .px_2()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .rounded(px(6.0))
                    .bg(if is_active {
                      p.surface_hover
                    } else {
                      p.surface_raised
                    })
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || (select_cb.on_select_doc)(doc_id, open_name.clone()))
                    .child(mica_icons::star_filled(p.favorite_hex))
                    .child(
                      text(display_name)
                        .flex_1()
                        .text_size(px(13.0))
                        .text_color(if is_active {
                          p.text_primary
                        } else {
                          p.text_muted
                        })
                        .truncate(),
                    ),
                );
              }
              list
            })
          }
        }))
    }
  })
}

fn pages_section_header(
  show_pages: Signal<bool>,
  dark_mode: Signal<bool>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  let cb_new_page = cb.clone();
  let cb_new_folder = cb;

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
    )
}

fn page_tree_list(ctx: Rc<TreeRowContext>) -> impl IntoElement {
  let tree_count = ctx.tree.clone();
  let tree_keys = ctx.tree.clone();
  let tree_rev = ctx.tree_rev;
  let row_ctx = ctx;

  virtual_list(
    move || {
      tree_rev.get();
      tree_count.borrow().visible_rows().len()
    },
    move |at: Position| render_tree_row(row_ctx.clone(), at),
  )
  .keys(move || {
    tree_rev.get();
    tree_keys
      .borrow()
      .visible_rows()
      .into_iter()
      .map(|r| r.view_id.to_string())
      .collect::<Vec<_>>()
  })
  .estimated_item_size(px(30.0))
  .flex_1()
}

fn render_tree_row(ctx: Rc<TreeRowContext>, at: Position) -> impl IntoElement {
  let tree_read = ctx.tree.clone();
  let tree_press = ctx.tree.clone();
  let tree_rev = ctx.tree_rev;
  let active_doc = ctx.active_doc;
  let dark_mode = ctx.dark_mode;
  let open_menu_view = ctx.open_menu_view;
  let renaming_view = ctx.renaming_view;
  let rename_draft = ctx.rename_draft;
  let row_idx = at.index();

  let row_info = computed(move || {
    tree_rev.get();
    tree_read.borrow().visible_rows().get(at.index()).cloned()
  });

  let row_view_id = computed(move || row_info.get().map(|r| r.view_id));

  let is_active = computed(move || {
    let curr = active_doc.get();
    row_info
      .get()
      .is_some_and(|r| !r.is_folder && Some(r.object_id) == curr)
  });

  let is_renaming = computed(move || {
    let vid = row_view_id.get();
    vid.is_some() && renaming_view.get() == vid
  });

  let is_menu_open = computed(move || {
    let vid = row_view_id.get();
    vid.is_some() && open_menu_view.get() == vid
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

  let cb_select = ctx.cb.clone();
  let cb_rename = ctx.cb.clone();
  let cb_menu = ctx.cb.clone();

  view()
    .w_full()
    .child(
      view()
        .w_full()
        .h(px(30.0))
        .pl(move || px(6.0 + indent_px.get()))
        .pr_1()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(6.0))
        .bg(move || {
          let p = palette(dark_mode.get());
          if is_active.get() || is_menu_open.get() || is_renaming.get() {
            p.surface_hover
          } else {
            p.surface_raised
          }
        })
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .child(when(move || is_renaming.get(), {
          let cb_r = cb_rename.clone();
          move || {
            let cb_submit = cb_r.clone();
            let cb_confirm = cb_r.clone();
            view()
              .flex_1()
              .min_w(px(0.0))
              .flex_row()
              .items_center()
              .gap_1()
              .child(
                input()
                  .test_id("sidebar-inline-rename-input")
                  .auto_focus(true)
                  .placeholder("输入名称...")
                  .flex_1()
                  .min_w(px(0.0))
                  .h(px(24.0))
                  .px_2()
                  .rounded(px(4.0))
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).accent)
                  .bg(move || palette(dark_mode.get()).surface_base)
                  .text_size(px(13.0))
                  .text_color(move || palette(dark_mode.get()).text_primary)
                  .value(move || rename_draft.get())
                  .on_change(move |v: &str| rename_draft.set(v.to_owned()))
                  .on_submit(move |submitted: &str| {
                    let trimmed = submitted.trim();
                    if !trimmed.is_empty()
                      && let Some(vid) = row_view_id.get_untracked()
                    {
                      (cb_submit.on_rename_view)(vid, trimmed.to_string());
                    }
                    renaming_view.set(None);
                  })
                  .on_key_press(move |key: &str| {
                    if key == "Escape" {
                      renaming_view.set(None);
                    }
                  }),
              )
              .child(
                button()
                  .test_id("sidebar-inline-rename-confirm")
                  .accessibility_label("确认重命名")
                  .w(px(22.0))
                  .h(px(22.0))
                  .flex_shrink_0()
                  .items_center()
                  .justify_center()
                  .rounded(px(4.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_base))
                  .on_press(move || {
                    let draft = rename_draft.get_untracked();
                    let trimmed = draft.trim();
                    if !trimmed.is_empty()
                      && let Some(vid) = row_view_id.get_untracked()
                    {
                      (cb_confirm.on_rename_view)(vid, trimmed.to_string());
                    }
                    renaming_view.set(None);
                  })
                  .child(dynamic(move || {
                    mica_icons::check_icon(palette(dark_mode.get()).accent_hex)
                  })),
              )
              .child(
                button()
                  .test_id("sidebar-inline-rename-cancel")
                  .accessibility_label("取消重命名")
                  .w(px(22.0))
                  .h(px(22.0))
                  .flex_shrink_0()
                  .items_center()
                  .justify_center()
                  .rounded(px(4.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_base))
                  .on_press(move || renaming_view.set(None))
                  .child(dynamic(move || {
                    mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                  })),
              )
          }
        }))
        .child(when(move || !is_renaming.get(), {
          let t_press = tree_press.clone();
          let cb_sel = cb_select.clone();
          move || {
            let t_p = t_press.clone();
            let cb_s = cb_sel.clone();
            view()
              .flex_1()
              .min_w(px(0.0))
              .h_full()
              .flex_row()
              .items_center()
              .gap_1()
              .child(
                button()
                  .test_id(&format!("sidebar-row-{row_idx}"))
                  .accessibility_label(move || {
                    let label = row_label.get();
                    if row_info.get().is_some_and(|r| r.is_folder) {
                      Some(SharedString::from(format!("展开或收起文件夹 {label}")))
                    } else {
                      Some(SharedString::from(format!("打开页面 {label}")))
                    }
                  })
                  .flex_1()
                  .min_w(px(0.0))
                  .h_full()
                  .px_1()
                  .flex_row()
                  .items_center()
                  .gap_1()
                  .rounded(px(5.0))
                  .on_press(move || {
                    open_menu_view.set(None);
                    if let Some(row) = row_info.get_untracked() {
                      if row.is_folder {
                        t_p.borrow_mut().toggle_expanded(row.view_id);
                        tree_rev.set(tree_rev.get_untracked() + 1);
                      } else {
                        (cb_s.on_select_doc)(row.object_id, row.name);
                      }
                    }
                  })
                  .child(dynamic(move || {
                    let p = palette(dark_mode.get());
                    let info = row_info.get();
                    let is_folder = info.as_ref().is_some_and(|r| r.is_folder);
                    let expanded = info.as_ref().is_some_and(|r| r.expanded);
                    let icon_hex = if is_active.get() {
                      p.text_primary_hex
                    } else {
                      p.text_muted_hex
                    };
                    if is_folder {
                      view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .flex_shrink_0()
                        .child(
                          view()
                            .w(px(14.0))
                            .h(px(14.0))
                            .items_center()
                            .justify_center()
                            .child(if expanded {
                              mica_icons::chevron_down(p.text_faint_hex)
                            } else {
                              mica_icons::chevron_right(p.text_faint_hex)
                            }),
                        )
                        .child(mica_icons::folder(icon_hex))
                    } else {
                      view()
                        .flex_row()
                        .items_center()
                        .flex_shrink_0()
                        .pl(px(4.0))
                        .child(mica_icons::file_doc(icon_hex))
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
                  ),
              )
              .child(
                button()
                  .test_id(&format!("sidebar-row-menu-{row_idx}"))
                  .accessibility_label(move || {
                    let label = row_label.get();
                    Some(SharedString::from(format!("打开 {label} 操作菜单")))
                  })
                  .w(px(22.0))
                  .h(px(22.0))
                  .flex_shrink_0()
                  .items_center()
                  .justify_center()
                  .rounded(px(4.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_sunken))
                  .on_press(move || {
                    if let Some(vid) = row_view_id.get_untracked() {
                      renaming_view.set(None);
                      if open_menu_view.get_untracked() == Some(vid) {
                        open_menu_view.set(None);
                      } else {
                        open_menu_view.set(Some(vid));
                      }
                    }
                  })
                  .child(dynamic(move || {
                    let p = palette(dark_mode.get());
                    let hex = if is_active.get() || is_menu_open.get() {
                      p.text_muted_hex
                    } else {
                      p.text_ghost_hex
                    };
                    mica_icons::more_dots(hex)
                  })),
              )
          }
        })),
    )
    .child(when(move || is_menu_open.get() && !is_renaming.get(), {
      let cb_m = cb_menu.clone();
      move || {
        let cb_up = cb_m.clone();
        let cb_down = cb_m.clone();
        let cb_trash = cb_m.clone();
        view()
          .w_full()
          .pl(move || px(10.0 + indent_px.get()))
          .pr_1()
          .pt_1()
          .pb_1()
          .child(
            view()
              .w_full()
              .px_1()
              .py_1()
              .flex_row()
              .flex_wrap()
              .items_center()
              .gap_1()
              .rounded(px(6.0))
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .bg(move || palette(dark_mode.get()).surface_overlay)
              .child(row_menu_btn(
                "row-action-rename",
                "重命名",
                "重命名当前项",
                dark_mode,
                false,
                move || {
                  if let Some(row) = row_info.get_untracked() {
                    rename_draft.set(row.name);
                    renaming_view.set(Some(row.view_id));
                    open_menu_view.set(None);
                  }
                },
              ))
              .child(row_menu_btn(
                "row-action-move-up",
                "上移",
                "向上移动当前项",
                dark_mode,
                false,
                move || {
                  if let Some(vid) = row_view_id.get_untracked() {
                    (cb_up.on_move_view)(vid, -1);
                  }
                  open_menu_view.set(None);
                },
              ))
              .child(row_menu_btn(
                "row-action-move-down",
                "下移",
                "向下移动当前项",
                dark_mode,
                false,
                move || {
                  if let Some(vid) = row_view_id.get_untracked() {
                    (cb_down.on_move_view)(vid, 1);
                  }
                  open_menu_view.set(None);
                },
              ))
              .child(row_menu_btn(
                "row-action-trash",
                "移入回收站",
                "将当前项移入回收站",
                dark_mode,
                true,
                move || {
                  if let Some(vid) = row_view_id.get_untracked() {
                    (cb_trash.on_trash_view)(vid);
                  }
                  open_menu_view.set(None);
                },
              )),
          )
      }
    }))
}

fn row_menu_btn(
  tid: &'static str,
  label: &'static str,
  a11y_label: &'static str,
  dark_mode: Signal<bool>,
  destructive: bool,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(a11y_label)
    .px_2()
    .py_1()
    .rounded(px(4.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_press)
    .child(
      text(label)
        .text_size(px(11.5))
        .text_color(move || {
          let p = palette(dark_mode.get());
          if destructive {
            p.danger
          } else {
            p.text_primary
          }
        }),
    )
}

fn recycle_bin_section(
  tree: Rc<RefCell<PageTreeModel>>,
  tree_rev: Signal<u64>,
  show_trash: Signal<bool>,
  dark_mode: Signal<bool>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  let tree_count = tree.clone();
  let tree_list = tree;

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
          text(move || {
            tree_rev.get();
            let count = tree_count.borrow().trashed_views().len();
            format!("回收站 ({count})")
          })
          .text_size(px(12.0))
          .text_color(move || palette(dark_mode.get()).text_faint),
        ),
    )
    .child(when(move || show_trash.get(), {
      let t_tr = tree_list.clone();
      let restore_cb = cb.clone();
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
    }))
}

fn account_footer(
  account_name: Signal<String>,
  dark_mode: Signal<bool>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  let cb_acct_menu = cb.clone();
  let cb_settings = cb;

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
    )
}

fn sidebar_resize_handle(
  sidebar_width: Signal<f32>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let drag_start_w = signal(244.0_f32);
  let handle_state = signal(ResizeHandleState::Idle);

  resize_handle()
    .axis(Axis::Horizontal)
    .edge(HandleEdge::Trailing)
    .accessibility_label("拖拽调整侧边栏宽度")
    .on_drag_start(move || {
      drag_start_w.set(sidebar_width.get_untracked());
    })
    .on_drag_move(move |offset: &Points| {
      let next = (drag_start_w.get_untracked() + offset.0)
        .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
      sidebar_width.set(next);
    })
    .on_drag_end(move |offset: &Points| {
      let next = (drag_start_w.get_untracked() + offset.0)
        .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
      sidebar_width.set(next);
    })
    .on_state_change(move |st: &ResizeHandleState| {
      handle_state.set(*st);
    })
    .child(
      view()
        .test_id("sidebar-resize-handle")
        .w(px(2.0))
        .h_full()
        .bg(move || {
          let p = palette(dark_mode.get());
          match handle_state.get() {
            ResizeHandleState::Pressed | ResizeHandleState::Dragging => p.accent,
            ResizeHandleState::Hovered => p.border_strong,
            ResizeHandleState::Idle => Rgba {
              r: 0.0,
              g: 0.0,
              b: 0.0,
              a: 0.0,
            },
          }
        }),
    )
}
