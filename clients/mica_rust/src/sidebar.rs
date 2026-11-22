//! Virtualized workspace page-tree sidebar backed directly by `mica_client::PageTreeModel`.
//!
//! Supports folders, pages, inline rename, recycle bin restore/purge, search,
//! and settings navigation while upholding the folder-is-only-container invariant.

use std::{cell::RefCell, rc::Rc};

use mica_client::PageTreeModel;
use nsh_base::{button, input, theme};
use nsh_core::{element::Position, geometry::px, prelude::*};
use uuid::Uuid;

pub struct SidebarCallbacks {
  pub on_select_doc: Rc<dyn Fn(Uuid, String)>,
  pub on_new_page: Rc<dyn Fn()>,
  pub on_new_folder: Rc<dyn Fn()>,
  pub on_rename_active: Rc<dyn Fn(String)>,
  pub on_trash_active: Rc<dyn Fn()>,
  pub on_restore_view: Rc<dyn Fn(Uuid)>,
  pub on_open_search: Rc<dyn Fn()>,
  pub on_open_settings: Rc<dyn Fn()>,
}

pub fn sidebar_view(
  tree: Rc<RefCell<PageTreeModel>>,
  tree_rev: Signal<u64>,
  active_doc: Signal<Option<Uuid>>,
  active_title: Signal<String>,
  cb: Rc<SidebarCallbacks>,
) -> impl IntoElement {
  let tree_count = tree.clone();
  let tree_keys = tree.clone();
  let tree_row = tree.clone();
  let tree_trash = tree.clone();
  let show_trash = signal(false);
  let renaming = signal(false);
  let rename_draft = signal(String::new());

  let cb_new_page = cb.clone();
  let cb_new_folder = cb.clone();
  let cb_search = cb.clone();
  let cb_settings = cb.clone();
  let cb_rename = cb.clone();
  let cb_trash = cb.clone();
  let cb_restore = cb.clone();
  let cb_select = cb.clone();

  view()
    .w(px(268.0))
    .h_full()
    .border_r_1()
    .border_color(move || theme().border)
    .bg(move || theme().sidebar)
    .child(
      // Header + actions
      view()
        .p_3()
        .gap_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
          view()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
              text("Mica Workspace")
                .text_sm()
                .text_color(move || theme().sidebar_foreground),
            )
            .child(
              view()
                .flex_row()
                .gap_1()
                .child(sidebar_btn("Search", "open-search-btn", move || {
                  (cb_search.on_open_search)();
                }))
                .child(sidebar_btn("⚙", "open-settings-btn", move || {
                  (cb_settings.on_open_settings)();
                })),
            ),
        )
        .child(
          view()
            .flex_row()
            .flex_wrap()
            .gap_1()
            .child(sidebar_btn("+ New", "new-page-btn", move || {
              (cb_new_page.on_new_page)();
            }))
            .child(sidebar_btn("+ Folder", "new-folder-btn", move || {
              (cb_new_folder.on_new_folder)();
            }))
            .child(sidebar_btn("Rename", "rename-page-btn", move || {
              rename_draft.set(active_title.get_untracked());
              renaming.set(!renaming.get_untracked());
            }))
            .child(sidebar_btn("Trash", "trash-page-btn", move || {
              (cb_trash.on_trash_active)();
            })),
        ),
    )
    .child(when(move || renaming.get(), move || {
      let cb_r = cb_rename.clone();
      view()
        .flex_row()
        .items_center()
        .gap_1()
        .p_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
          input()
            .test_id("rename-page-input")
            .flex_1()
            .px_2()
            .py_1()
            .border_1()
            .border_color(move || theme().primary)
            .rounded(move || theme().radius)
            .text_color(move || theme().foreground)
            .value(move || rename_draft.get())
            .on_change(move |v: &str| rename_draft.set(v.to_owned())),
        )
        .child(sidebar_btn("OK", "rename-confirm-btn", move || {
          let next = rename_draft.get_untracked();
          if !next.trim().is_empty() {
            (cb_r.on_rename_active)(next);
          }
          renaming.set(false);
        }))
    }))
    .child(
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
            .px_3()
            .py_2()
            .rounded(move || theme().radius)
            .bg(move || {
              let is_active = row_sig
                .get()
                .is_some_and(|r| !r.is_folder && active_doc.get() == Some(r.object_id));
              if is_active {
                theme().sidebar_accent
              } else {
                theme().sidebar
              }
            })
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
            .child(
              text(move || match row_sig.get() {
                Some(r) => {
                  let pad = "  ".repeat(r.depth);
                  let glyph = if r.is_folder {
                    if r.expanded { "▾ " } else { "▸ " }
                  } else {
                    "• "
                  };
                  format!("{pad}{glyph}{}", r.name)
                }
                None => String::new(),
              })
              .text_sm()
              .text_color(move || theme().sidebar_foreground),
            )
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
      .estimated_item_size(px(34.0))
      .flex_1(),
    )
    .child(
      // Recycle bin drawer toggle + list
      view()
        .p_2()
        .gap_1()
        .border_t_1()
        .border_color(move || theme().border)
        .child(
          button()
            .test_id("toggle-recycle-bin")
            .w_full()
            .px_2()
            .py_1()
            .rounded(move || theme().radius)
            .on_press(move || show_trash.set(!show_trash.get_untracked()))
            .child(
              text(move || {
                tree_rev.get();
                let n = tree_trash.borrow().trashed_views().len();
                format!("Recycle Bin ({n})")
              })
              .text_xs()
              .text_color(move || theme().muted_foreground),
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
              let mut box_el = view().w_full().gap_1();
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
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                    )
                    .child(sidebar_btn("Restore", &format!("restore-trash-{idx}"), move || {
                      (restore.on_restore_view)(vid);
                    })),
                );
              }
              box_el
            })
          }
        })),
    )
}

fn sidebar_btn(label: &str, tid: &str, on_click: impl Fn() + 'static) -> impl IntoElement {
  button()
    .test_id(tid)
    .px_2()
    .py_1()
    .border_1()
    .border_color(move || theme().border)
    .rounded(move || theme().radius)
    .bg(move || theme().secondary)
    .on_press(on_click)
    .child(
      text(label.to_string())
        .text_xs()
        .text_color(move || theme().secondary_foreground),
    )
}
