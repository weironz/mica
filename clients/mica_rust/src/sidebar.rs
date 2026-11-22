//! Pure-Rust L3 sidebar backed directly by `mica_client::PageTreeModel`.

use std::{cell::RefCell, rc::Rc};

use mica_client::{PageTreeModel, TreeRow};
use nsh_base::{button, theme};
use nsh_core::{element::Position, geometry::px, prelude::*};
use uuid::Uuid;

pub fn sidebar_view(
  tree: Rc<RefCell<PageTreeModel>>,
  tree_rev: Signal<u64>,
  active_doc: Signal<Option<Uuid>>,
  on_select_doc: Rc<dyn Fn(Uuid, String)>,
  on_new_page: Rc<dyn Fn()>,
) -> impl IntoElement {
  let rows_cache: Rc<RefCell<Vec<TreeRow>>> = Rc::new(RefCell::new(tree.borrow().visible_rows()));
  let rows_for_count = rows_cache.clone();
  let rows_for_keys = rows_cache.clone();
  let rows_for_item = rows_cache.clone();
  let tree_for_sync = tree.clone();
  let tree_for_click = tree.clone();

  view()
    .w(px(248.0))
    .border_r_1()
    .border_color(move || theme().border)
    .bg(move || theme().muted)
    .child(
      view()
        .flex_row()
        .items_center()
        .justify_between()
        .px_3()
        .py_3()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
          text("Mica Workspace")
            .text_sm()
            .text_color(move || theme().foreground),
        )
        .child(
          button()
            .test_id("new-page-btn")
            .px_2()
            .py_1()
            .rounded(move || theme().radius)
            .bg(move || theme().primary)
            .on_press(move || on_new_page())
            .child(
              text("+ New")
                .text_xs()
                .text_color(move || theme().primary_foreground),
            ),
        ),
    )
    .child(
      virtual_list(
        move || {
          tree_rev.get();
          let next = tree_for_sync.borrow().visible_rows();
          let len = next.len();
          *rows_for_count.borrow_mut() = next;
          len
        },
        move |at: Position| {
          let rows_ref = rows_for_item.clone();
          let tree_click = tree_for_click.clone();
          let on_select = on_select_doc.clone();
          let row = computed(move || {
            tree_rev.get();
            rows_ref.borrow().get(at.index()).cloned()
          });
          let is_active = computed(move || {
            row
              .get()
              .is_some_and(|r| !r.is_folder && active_doc.get() == Some(r.object_id))
          });

          button()
            .flex_row()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .bg(move || {
              if is_active.get() {
                theme().accent
              } else {
                theme().muted
              }
            })
            .on_press(move || {
              if let Some(r) = row.get_untracked() {
                if r.is_folder {
                  tree_click.borrow_mut().toggle_expanded(r.view_id);
                  tree_rev.set(tree_rev.get_untracked() + 1);
                } else {
                  on_select(r.object_id, r.name.clone());
                }
              }
            })
            .child(
              text(move || {
                row
                  .get()
                  .map(|r| {
                    let indent = "  ".repeat(r.depth);
                    let glyph = if r.is_folder {
                      if r.expanded { "▾ " } else { "▸ " }
                    } else {
                      "• "
                    };
                    format!("{indent}{glyph}{}", r.name)
                  })
                  .unwrap_or_default()
              })
              .text_sm()
              .text_color(move || theme().foreground),
            )
        },
      )
      .keys(move || {
        tree_rev.get();
        rows_for_keys
          .borrow()
          .iter()
          .map(|r| r.view_id.to_string())
          .collect::<Vec<_>>()
      })
      .estimated_item_size(px(34.0))
      .flex_1(),
    )
}
