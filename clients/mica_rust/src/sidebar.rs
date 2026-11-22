//! Minimal Notion-inspired warm-gray workspace sidebar matching Mica's original
//! visual specification (`docs/plans/ui-visual-refresh.md`).

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
  let show_trash = signal(false);

  let cb_new_page = cb.clone();
  let cb_new_folder = cb.clone();
  let cb_board = cb.clone();
  let cb_search = cb.clone();
  let cb_settings = cb.clone();
  let cb_restore = cb.clone();
  let cb_select = cb.clone();

  view()
    .w(px(244.0))
    .h_full()
    .flex_shrink_0()
    .border_r_1()
    .border_color(move || palette(dark_mode.get()).border_subtle)
    .bg(move || palette(dark_mode.get()).surface_raised)
    .child(
      // 1. Workspace switcher row (cloud icon + workspace name + switcher + board shortcut)
      view()
        .h(px(40.0))
        .px_3()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
          view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(dynamic(move || {
              mica_icons::cloud(palette(dark_mode.get()).text_muted_hex)
            }))
            .child(
              text(move || workspace_name.get())
                .text_size(px(14.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || palette(dark_mode.get()).text_primary),
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
            .child(
              button()
                .test_id("mode-board")
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
      // 2. Quiet recessed search pill ("搜索...  Ctrl+Shift+F")
      view().px_3().pb_2().child(
        button()
          .test_id("open-search-btn")
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
      // 3. Section header ("页面 ⌄" + lightweight `+` & `folder+` icon buttons)
      view()
        .h(px(28.0))
        .px_3()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
          view()
            .flex_row()
            .items_center()
            .gap_1()
            .child(
              text("页面")
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || palette(dark_mode.get()).text_faint),
            )
            .child(dynamic(move || {
              mica_icons::chevron_down(palette(dark_mode.get()).text_faint_hex)
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
                .w(px(24.0))
                .h(px(24.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || (cb_new_page.on_new_page)())
                .child(dynamic(move || {
                  mica_icons::plus(palette(dark_mode.get()).text_muted_hex)
                })),
            )
            .child(
              button()
                .test_id("new-folder-btn")
                .w(px(24.0))
                .h(px(24.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || (cb_new_folder.on_new_folder)())
                .child(dynamic(move || {
                  mica_icons::folder_plus(palette(dark_mode.get()).text_muted_hex)
                })),
            ),
        ),
    )
    .child(
      // 4. Compact 30px virtualized page & folder tree
      view().flex_1().px_2().py_1().child(
        virtual_list(
          move || {
            tree_rev.get();
            tree_count.borrow().visible_rows().len()
          },
          move |at: Position| {
            let t = tree_row.clone();
            let sel = cb_select.clone();
            let row_sig = computed(move || {
              tree_rev.get();
              t.borrow().visible_rows().get(at.index()).cloned()
            });
            let t_press = tree_row.clone();

            button()
              .w_full()
              .h(px(30.0))
              .px_2()
              .flex_row()
              .items_center()
              .gap_2()
              .rounded(px(6.0))
              .bg(move || {
                let p = palette(dark_mode.get());
                let is_active = row_sig
                  .get()
                  .is_some_and(|r| !r.is_folder && active_doc.get() == Some(r.object_id));
                if is_active {
                  p.surface_hover
                } else {
                  p.surface_raised
                }
              })
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                let row = { t_press.borrow().visible_rows().get(at.index()).cloned() };
                if let Some(row) = row {
                  if row.is_folder {
                    t_press.borrow_mut().toggle_expanded(row.view_id);
                    tree_rev.set(tree_rev.get_untracked() + 1);
                  } else {
                    (sel.on_select_doc)(row.object_id, row.name);
                  }
                }
              })
              .child(dynamic(move || {
                let p = palette(dark_mode.get());
                match row_sig.get() {
                  Some(r) => {
                    let is_active = !r.is_folder && active_doc.get() == Some(r.object_id);
                    let pad = (r.depth as f32) * 14.0;
                    let icon_hex = if is_active {
                      p.text_primary_hex
                    } else {
                      p.text_muted_hex
                    };
                    let mut line = view().flex_row().items_center().gap_2().pl(px(pad));
                    if r.is_folder {
                      line = line.child(mica_icons::folder(icon_hex));
                    } else {
                      line = line.child(mica_icons::file_doc(icon_hex));
                    }
                    line.child(
                      text(r.name)
                        .text_size(px(14.0))
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
                    )
                  }
                  None => view(),
                }
              }))
          },
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
        .flex_1(),
      ),
    )
    .child(
      // 5. Collapsible Recycle Bin strip (stays quiet when empty)
      view()
        .px_2()
        .py_1()
        .child(
          button()
            .test_id("toggle-recycle-bin")
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
                let n = tree_trash.borrow().trashed_views().len();
                format!("回收站 ({n})")
              })
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_faint),
            ),
        )
        .child(when(move || show_trash.get(), {
          let t_list = tree.clone();
          let cb_res = cb_restore.clone();
          move || {
            let t_dyn = t_list.clone();
            let cb_dyn = cb_res.clone();
            dynamic(move || {
              tree_rev.get();
              let items: Vec<(Uuid, String)> = t_dyn
                .borrow()
                .trashed_views()
                .into_iter()
                .map(|v| (v.id, v.name.clone()))
                .collect();
              let mut box_el = view().w_full().gap_1().py_1();
              for (idx, (vid, name)) in items.into_iter().enumerate() {
                let restore = cb_dyn.clone();
                box_el = box_el.child(
                  view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .child(
                      text(name)
                        .text_size(px(12.0))
                        .text_color(move || palette(dark_mode.get()).text_muted)
                        .truncate(),
                    )
                    .child(
                      button()
                        .test_id(&format!("restore-trash-{idx}"))
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
      // 6. Bottom pinned account bar + settings gear (exact match to original screenshot)
      view()
        .h(px(44.0))
        .px_3()
        .flex_row()
        .items_center()
        .justify_between()
        .border_t_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .child(
          view()
            .flex_row()
            .items_center()
            .gap_2()
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
                .text_color(move || palette(dark_mode.get()).text_primary),
            ),
        )
        .child(
          button()
            .test_id("open-settings-btn")
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
