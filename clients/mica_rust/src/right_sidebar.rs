//! Collapsible 248px right sidebar restoring Mica's original right inspector panel:
//! - Live Page Outline (title + H1/H2/H3 headings, click to jump to rich-editor position).
//! - Document Info & Properties (word/char stats, block count).
//! - Real Version History under "版本快照" (save snapshot + list with label, timestamp & restore).
//! - Backlinks (pages referencing the current page by title or `mica://page/<id>`).

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_client::PageTreeModel;
use nsh_base::button;
use nsh_core::prelude::*;
use uuid::Uuid;

use crate::{mica_icons, mica_rich_bridge::OutlineItem, mica_theme::palette};

#[derive(Clone, PartialEq, Eq)]
pub struct SnapshotEntry {
  pub id: i64,
  pub label: String,
  pub created_at: String,
}

pub struct RightSidebarProps {
  pub dark_mode: Signal<bool>,
  pub page_title: Signal<String>,
  pub active_doc: Signal<Option<Uuid>>,
  pub outline_items: Signal<Vec<OutlineItem>>,
  pub word_count_text: Signal<String>,
  pub block_count: Signal<usize>,
  pub snapshot_count: Signal<usize>,
  pub snapshots: Signal<Vec<SnapshotEntry>>,
  pub tree: Rc<RefCell<PageTreeModel>>,
  pub docs: Rc<RefCell<HashMap<Uuid, String>>>,
  pub tree_rev: Signal<u64>,
  pub on_jump_pos: Rc<dyn Fn(usize)>,
  pub on_open_doc: Rc<dyn Fn(Uuid, String)>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  pub on_restore_snapshot: Rc<dyn Fn(i64)>,
  pub on_close: Rc<dyn Fn()>,
}

pub fn right_sidebar_view(props: Rc<RightSidebarProps>) -> impl IntoElement {
  let dark_mode = props.dark_mode;
  let p_close = props.clone();

  view()
    .test_id("right-sidebar")
    .w(px(248.0))
    .h_full()
    .flex_shrink_0()
    .border_l_1()
    .border_color(move || palette(dark_mode.get()).border_subtle)
    .bg(move || palette(dark_mode.get()).surface_raised)
    .child(
      // Header
      view()
        .h(px(40.0))
        .px_3()
        .flex_row()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .child(
          view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(dynamic(move || {
              mica_icons::outline_icon(palette(dark_mode.get()).text_muted_hex)
            }))
            .child(
              text("页面大纲")
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || palette(dark_mode.get()).text_primary),
            ),
        )
        .child(
          button()
            .test_id("right-sidebar-close")
            .accessibility_label("关闭右侧边栏")
            .w(px(24.0))
            .h(px(24.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || (p_close.on_close)())
            .child(dynamic(move || {
              mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
            })),
        ),
    )
    .child(
      // Scrollable inspector body
      scroll().flex_1().min_h(px(0.0)).child(
        view()
          .w_full()
          .px_3()
          .py_3()
          .gap_4()
          .child(outline_section(props.clone()))
          .child(section_divider(dark_mode))
          .child(page_info_section(props.clone()))
          .child(section_divider(dark_mode))
          .child(version_history_section(props.clone()))
          .child(section_divider(dark_mode))
          .child(backlinks_section(props)),
      ),
    )
}

fn section_divider(dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w_full()
    .h(px(1.0))
    .bg(move || palette(dark_mode.get()).border_normal)
}

fn outline_section(props: Rc<RightSidebarProps>) -> impl IntoElement {
  let dark_mode = props.dark_mode;

  view().gap_1().child(dynamic(move || {
    let p = palette(dark_mode.get());
    let title = props.page_title.get();
    let items = props.outline_items.get();
    let jump_title = props.on_jump_pos.clone();

    let mut list = view().w_full().gap_1();
    let display_title = if title.trim().is_empty() {
      "无标题".to_string()
    } else {
      title
    };
    list = list.child(
      button()
        .test_id("outline-title-item")
        .accessibility_label("跳转到页面标题")
        .w_full()
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || jump_title(0))
        .child(
          text(display_title)
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.text_primary)
            .truncate(),
        ),
    );

    if items.is_empty() {
      list = list.child(
        view().px_2().py_1().child(
          text("添加标题（# / ## / ###）后自动生成目录")
            .text_size(px(12.0))
            .text_color(p.text_ghost),
        ),
      );
    } else {
      for (i, item) in items.into_iter().enumerate() {
        let jump = props.on_jump_pos.clone();
        let indent = (item.level.saturating_sub(1) as f32) * 12.0;
        let target_pos = item.pos;
        let lbl = item.text;
        list = list.child(
          button()
            .test_id(&format!("outline-item-{i}"))
            .accessibility_label(format!("跳转到标题 {lbl}"))
            .w_full()
            .pl(px(8.0 + indent))
            .pr_2()
            .py_1()
            .rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || jump(target_pos))
            .child(
              text(lbl)
                .text_size(px(12.5))
                .text_color(if item.level <= 1 {
                  p.text_primary
                } else {
                  p.text_muted
                })
                .truncate(),
            ),
        );
      }
    }
    list
  }))
}

fn page_info_section(props: Rc<RightSidebarProps>) -> impl IntoElement {
  let dark_mode = props.dark_mode;
  let p_words = props.clone();
  let p_blocks = props;

  view()
    .gap_2()
    .child(
      text("页面信息")
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_faint),
    )
    .child(
      view()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
          text("字数统计")
            .text_size(px(12.5))
            .text_color(move || palette(dark_mode.get()).text_muted),
        )
        .child(
          text(move || p_words.word_count_text.get())
            .text_size(px(12.5))
            .text_color(move || palette(dark_mode.get()).text_primary),
        ),
    )
    .child(
      view()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
          text("内容块数")
            .text_size(px(12.5))
            .text_color(move || palette(dark_mode.get()).text_muted),
        )
        .child(
          text(move || format!("{} 块", p_blocks.block_count.get()))
            .text_size(px(12.5))
            .text_color(move || palette(dark_mode.get()).text_primary),
        ),
    )
}

fn version_history_section(props: Rc<RightSidebarProps>) -> impl IntoElement {
  let dark_mode = props.dark_mode;
  let p_count = props.clone();
  let p_save = props.clone();
  let p_list = props;

  view()
    .gap_2()
    .child(
      view()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
          text("版本快照")
            .text_size(px(12.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || palette(dark_mode.get()).text_faint),
        )
        .child(
          text(move || format!("{} 个", p_count.snapshot_count.get()))
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).text_muted),
        ),
    )
    .child(
      button()
        .test_id("right-save-snapshot-btn")
        .accessibility_label("保存当前页面版本快照")
        .w_full()
        .py_1()
        .px_2()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_base)
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || (p_save.on_save_snapshot)())
        .child(dynamic(move || {
          mica_icons::history_icon(palette(dark_mode.get()).text_muted_hex)
        }))
        .child(
          text("保存版本快照")
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).text_primary),
        ),
    )
    .child(dynamic(move || {
      let entries = p_list.snapshots.get();
      let p = palette(dark_mode.get());
      let mut list = view().test_id("snapshot-list").w_full().gap_1();
      if entries.is_empty() {
        return list.child(
          text("暂无版本快照，点击上方按钮保存当前版本")
            .text_size(px(12.0))
            .text_color(p.text_ghost),
        );
      }
      for (i, entry) in entries.into_iter().enumerate() {
        let restore = p_list.on_restore_snapshot.clone();
        let snap_id = entry.id;
        let display_label = if entry.label.trim().is_empty() {
          format!("快照 #{snap_id}")
        } else {
          entry.label
        };
        let created_at = entry.created_at;
        list = list.child(
          view()
            .w_full()
            .px_2()
            .py_1()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .rounded(px(6.0))
            .border_1()
            .border_color(p.border_subtle)
            .bg(p.surface_base)
            .child(
              view()
                .flex_1()
                .min_w(px(0.0))
                .child(
                  text(display_label.clone())
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text_primary)
                    .truncate(),
                )
                .child(
                  text(created_at)
                    .text_size(px(11.0))
                    .text_color(p.text_faint)
                    .truncate(),
                ),
            )
            .child(
              button()
                .test_id(&format!("restore-snapshot-{i}"))
                .accessibility_label(format!("恢复版本快照 {display_label}"))
                .px_2()
                .py_1()
                .flex_shrink_0()
                .rounded(px(5.0))
                .bg(move || palette(dark_mode.get()).surface_sunken)
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || restore(snap_id))
                .child(
                  text("恢复")
                    .text_size(px(11.0))
                    .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            ),
        );
      }
      list
    }))
}

fn backlinks_section(props: Rc<RightSidebarProps>) -> impl IntoElement {
  let dark_mode = props.dark_mode;

  view()
    .gap_1()
    .child(
      text("反向链接")
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_faint),
    )
    .child(dynamic(move || {
      props.tree_rev.get();
      let cur_id = props.active_doc.get();
      let cur_title = props.page_title.get();
      let p = palette(dark_mode.get());
      let mut hits: Vec<(Uuid, String)> = Vec::new();
      if let Some(cid) = cur_id {
        let needle_link = format!("mica://page/{cid}");
        let needle_wiki = format!("[[{}]]", cur_title.trim());
        let tree = props.tree.borrow();
        let docs = props.docs.borrow();
        for row in tree.visible_rows() {
          if row.is_folder || row.object_id == cid {
            continue;
          }
          if let Some(md) = docs.get(&row.object_id)
            && (md.contains(&needle_link)
              || (!cur_title.trim().is_empty() && md.contains(&needle_wiki)))
          {
            hits.push((row.object_id, row.name));
          }
        }
      }

      let mut box_el = view().w_full().gap_1();
      if hits.is_empty() {
        box_el = box_el.child(
          text("暂无其他页面链接到此页")
            .text_size(px(12.0))
            .text_color(p.text_ghost),
        );
      } else {
        for (i, (doc_id, name)) in hits.into_iter().enumerate() {
          let open = props.on_open_doc.clone();
          let open_name = name.clone();
          box_el = box_el.child(
            button()
              .test_id(&format!("backlink-{i}"))
              .accessibility_label(format!("打开反向链接 {name}"))
              .w_full()
              .px_2()
              .py_1()
              .flex_row()
              .items_center()
              .gap_2()
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || open(doc_id, open_name.clone()))
              .child(mica_icons::file_doc(p.text_faint_hex))
              .child(
                text(name)
                  .text_size(px(12.5))
                  .text_color(p.text_primary)
                  .truncate(),
              ),
          );
        }
      }
      box_el
    }))
}
