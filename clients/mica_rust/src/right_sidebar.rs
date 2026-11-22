//! Collapsible 248px right sidebar restoring Mica's original right inspector panel:
//! - Live Page Outline (title + H1/H2/H3 headings, click to jump/focus block).
//! - Document Info & Properties (word/char stats, block count, version snapshots).
//! - Backlinks (pages referencing the current page by title or `mica://page/<id>`).

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_client::PageTreeModel;
use nsh_base::button;
use nsh_core::{geometry::px, prelude::*};
use uuid::Uuid;

use crate::{editor_view::OutlineItem, mica_icons, mica_theme::palette};

pub struct RightSidebarProps {
  pub dark_mode: Signal<bool>,
  pub page_title: Signal<String>,
  pub active_doc: Signal<Option<Uuid>>,
  pub outline_items: Signal<Vec<OutlineItem>>,
  pub word_count_text: Signal<String>,
  pub block_count: Signal<usize>,
  pub snapshot_count: Signal<usize>,
  pub tree: Rc<RefCell<PageTreeModel>>,
  pub docs: Rc<RefCell<HashMap<Uuid, String>>>,
  pub tree_rev: Signal<u64>,
  pub on_jump_block: Rc<dyn Fn(usize)>,
  pub on_open_doc: Rc<dyn Fn(Uuid, String)>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  pub on_close: Rc<dyn Fn()>,
}

pub fn right_sidebar_view(props: Rc<RightSidebarProps>) -> impl IntoElement {
  let dark_mode = props.dark_mode;
  let p_close = props.clone();
  let p_outline = props.clone();
  let p_stats = props.clone();
  let p_snap = props.clone();
  let p_backlinks = props.clone();

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
      // Scrollable body
      view()
        .flex_1()
        .px_3()
        .py_3()
        .gap_4()
        .child(
          // 1. Outline section
          view().gap_1().child(dynamic(move || {
            let p = palette(dark_mode.get());
            let title = p_outline.page_title.get();
            let items = p_outline.outline_items.get();
            let jump_title = p_outline.on_jump_block.clone();

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
                let jump = p_outline.on_jump_block.clone();
                let indent = ((item.level.saturating_sub(1)) as f32) * 12.0;
                let block_idx = item.block_index;
                let lbl = item.text.clone();
                list = list.child(
                  button()
                    .test_id(&format!("outline-item-{i}"))
                    .accessibility_label(format!("跳转到标题 {}", lbl))
                    .w_full()
                    .pl(px(8.0 + indent))
                    .pr_2()
                    .py_1()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || jump(block_idx))
                    .child(
                      text(lbl)
                        .text_size(px(12.5))
                        .text_color(p.text_muted)
                        .truncate(),
                    ),
                );
              }
            }
            list
          })),
        )
        .child(
          // Divider
          view()
            .w_full()
            .h(px(1.0))
            .bg(move || palette(dark_mode.get()).border_normal),
        )
        .child(
          // 2. Document Properties & Version Snapshots
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
                  text(move || p_stats.word_count_text.get())
                    .text_size(px(12.5))
                    .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            )
            .child({
              let ps = props.clone();
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
                  text(move || format!("{} 块", ps.block_count.get()))
                    .text_size(px(12.5))
                    .text_color(move || palette(dark_mode.get()).text_primary),
                )
            })
            .child({
              let ps = props.clone();
              view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                  text("版本快照")
                    .text_size(px(12.5))
                    .text_color(move || palette(dark_mode.get()).text_muted),
                )
                .child(
                  text(move || format!("{} 个", ps.snapshot_count.get()))
                    .text_size(px(12.5))
                    .text_color(move || palette(dark_mode.get()).text_primary),
                )
            })
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
                .on_press(move || (p_snap.on_save_snapshot)())
                .child(dynamic(move || {
                  mica_icons::history_icon(palette(dark_mode.get()).text_muted_hex)
                }))
                .child(
                  text("保存版本快照")
                    .text_size(px(12.0))
                    .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            ),
        )
        .child(
          view()
            .w_full()
            .h(px(1.0))
            .bg(move || palette(dark_mode.get()).border_normal),
        )
        .child(
          // 3. Backlinks
          view()
            .gap_1()
            .child(
              text("反向链接")
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || palette(dark_mode.get()).text_faint),
            )
            .child(dynamic(move || {
              p_backlinks.tree_rev.get();
              let cur_id = p_backlinks.active_doc.get();
              let cur_title = p_backlinks.page_title.get();
              let p = palette(dark_mode.get());
              let mut hits: Vec<(Uuid, String)> = Vec::new();
              if let Some(cid) = cur_id {
                let needle_link = format!("mica://page/{cid}");
                let needle_wiki = format!("[[{}]]", cur_title.trim());
                let tree = p_backlinks.tree.borrow();
                let docs = p_backlinks.docs.borrow();
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
                  let open = p_backlinks.on_open_doc.clone();
                  let open_name = name.clone();
                  box_el = box_el.child(
                    button()
                      .test_id(&format!("backlink-{i}"))
                      .accessibility_label(format!("打开反向链接 {}", name))
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
            })),
        ),
    )
}
