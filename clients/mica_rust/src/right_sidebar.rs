//! Resizable right tools rail (`220..480px`, default `300px`, `surface_base`) restoring
//! Mica's 1:1 Flutter right sidebar & below-editor backlinks panel:
//! - Leading-edge `nsh_base::resize_handle` (`220..=480px`, default `300px`).
//! - Tabbed header rail: `大纲` / `评论` (with unresolved badge) + optional `历史` shortcut
//!   to open the 980×640 Version History Diff modal + close button (`id:right-sidebar-close`).
//! - `大纲` tab: live Page Outline (`页面大纲` + H1/H2/H3 jump), Document Info, quick Version
//!   History snapshots (`初始版本` + `id:restore-snapshot-{i}` + `"完整历史与对比..."`),
//!   and Backlinks (`反向链接`).
//! - `评论` tab: 1:1 `CommentPanel` threads (open first, orphaned quote strike-through,
//!   inline reply, resolve/reopen, delete).
//! - `backlinks_below_editor_view`: quiet hairline + `N 个反向链接` list mounted below
//!   `RichEditor` (hidden when empty).

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_client::PageTreeModel;
use nsh_base::{HandleEdge, ResizeHandleState, button, input, resize_handle};
use nsh_core::{
  color::Rgba,
  element::{ImageSrc, Svg, svg},
  geometry::Axis,
  prelude::*,
};
use uuid::Uuid;

use crate::{
  mica_icons,
  mica_rich_bridge::OutlineItem,
  mica_theme::{MicaPalette, palette},
};

pub const RIGHT_SIDEBAR_MIN_WIDTH: f32 = 220.0;
pub const RIGHT_SIDEBAR_MAX_WIDTH: f32 = 480.0;
pub const RIGHT_SIDEBAR_DEFAULT_WIDTH: f32 = 300.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolsTab {
  Outline,
  Comments,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SnapshotEntry {
  pub id: i64,
  pub label: String,
  pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BacklinkEntry {
  pub doc_id: Uuid,
  pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentItem {
  pub id: String,
  pub author: String,
  pub body: String,
  pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentThreadEntry {
  pub id: String,
  pub quote: String,
  pub is_resolved: bool,
  pub is_orphaned: bool,
  pub can_delete: bool,
  pub comments: Vec<CommentItem>,
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

/// Optional host hooks for persistent width, active rail tab, comments, and opening the
/// full 980×640 Version History Diff modal.
#[derive(Clone)]
pub struct RightSidebarExtras {
  pub width: Signal<f32>,
  pub active_tab: Signal<ToolsTab>,
  pub comments: Signal<Vec<CommentThreadEntry>>,
  pub on_open_version_history: Option<Rc<dyn Fn()>>,
  pub on_focus_thread: Option<Rc<dyn Fn(String)>>,
  pub on_reply_comment: Option<Rc<dyn Fn(String, String)>>,
  pub on_set_comment_resolved: Option<Rc<dyn Fn(String, bool)>>,
  pub on_delete_comment: Option<Rc<dyn Fn(String)>>,
}

impl Default for RightSidebarExtras {
  fn default() -> Self {
    Self {
      width: signal(RIGHT_SIDEBAR_DEFAULT_WIDTH),
      active_tab: signal(ToolsTab::Outline),
      comments: signal(Vec::new()),
      on_open_version_history: None,
      on_focus_thread: None,
      on_reply_comment: None,
      on_set_comment_resolved: None,
      on_delete_comment: None,
    }
  }
}

/// Backwards-compatible entry point with built-in resizable `300px` (`220..480px`) width
/// and `大纲` / `评论` tabs.
pub fn right_sidebar_view(props: Rc<RightSidebarProps>) -> impl IntoElement {
  right_sidebar_view_with_extras(props, Rc::new(RightSidebarExtras::default()))
}

/// Upgraded resizable right tools rail (`220..480px`, default `300px`, `surface_base`)
/// with full comments + version history modal hooks.
pub fn right_sidebar_view_with_extras(
  props: Rc<RightSidebarProps>,
  extras: Rc<RightSidebarExtras>,
) -> impl IntoElement {
  let dark_mode = props.dark_mode;
  let width_sig = extras.width;
  let active_tab = extras.active_tab;
  let p_close = props.clone();
  let ex_header = extras.clone();
  let ex_body = extras.clone();
  let props_body = props.clone();

  let replying_to: Signal<Option<String>> = signal(None);
  let reply_draft = signal(String::new());

  view()
    .test_id("right-sidebar")
    .relative()
    .w(move || {
      px(width_sig
        .get()
        .clamp(RIGHT_SIDEBAR_MIN_WIDTH, RIGHT_SIDEBAR_MAX_WIDTH))
    })
    .h_full()
    .flex_shrink_0()
    .border_l_1()
    .border_color(move || palette(dark_mode.get()).border_subtle)
    .bg(move || palette(dark_mode.get()).surface_base)
    .child(
      // Header tab bar: 大纲 / 评论 (+ unresolved badge) | 历史 shortcut + ×
      view()
        .h(px(40.0))
        .px_2()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_1()
        .border_b_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .child(dynamic(move || {
          let p = palette(dark_mode.get());
          let cur = active_tab.get();
          let unresolved = ex_header
            .comments
            .get()
            .iter()
            .filter(|t| !t.is_resolved)
            .count();

          view()
            .flex_row()
            .items_center()
            .gap_1()
            .child(tools_tab_btn(
              "right-tab-outline",
              "大纲",
              cur == ToolsTab::Outline,
              0,
              dark_mode,
              p,
              mica_icons::outline_icon(if cur == ToolsTab::Outline {
                p.text_primary_hex
              } else {
                p.text_muted_hex
              }),
              move || active_tab.set(ToolsTab::Outline),
            ))
            .child(tools_tab_btn(
              "right-tab-comments",
              "评论",
              cur == ToolsTab::Comments,
              unresolved,
              dark_mode,
              p,
              comment_bubble_icon(if cur == ToolsTab::Comments {
                p.text_primary_hex
              } else {
                p.text_muted_hex
              }),
              move || active_tab.set(ToolsTab::Comments),
            ))
        }))
        .child(
          view()
            .flex_row()
            .items_center()
            .gap_1()
            .child(when(
              {
                let has_vh = extras.on_open_version_history.is_some();
                move || has_vh
              },
              {
                let open_vh = extras.on_open_version_history.clone();
                move || {
                  let cb = open_vh.clone();
                  button()
                    .test_id("open-version-history-btn")
                    .accessibility_label("打开版本历史与对比")
                    .w(px(24.0))
                    .h(px(24.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      if let Some(f) = &cb {
                        f();
                      }
                    })
                    .child(dynamic(move || {
                      mica_icons::history_icon(palette(dark_mode.get()).text_muted_hex)
                    }))
                }
              },
            ))
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
        ),
    )
    .child(
      // Scrollable tab body
      scroll().flex_1().min_h(px(0.0)).child(dynamic(move || {
        match active_tab.get() {
          ToolsTab::Outline => view()
            .w_full()
            .px_3()
            .py_3()
            .gap_4()
            .child(outline_section(props_body.clone()))
            .child(section_divider(dark_mode))
            .child(page_info_section(props_body.clone()))
            .child(section_divider(dark_mode))
            .child(version_history_section(
              props_body.clone(),
              ex_body.on_open_version_history.clone(),
            ))
            .child(section_divider(dark_mode))
            .child(backlinks_section(props_body.clone()))
            .into_element(),
          ToolsTab::Comments => comments_tab_view(
            ex_body.clone(),
            dark_mode,
            replying_to,
            reply_draft,
          )
          .into_element(),
        }
      })),
    )
    .child(right_sidebar_resize_handle(width_sig, dark_mode))
}

/// Quiet below-editor backlinks panel mounted beneath `RichEditor`.
/// Renders nothing when `backlinks` is empty (1:1 with Flutter `BacklinksPanel`).
pub fn backlinks_below_editor_view(
  backlinks: Signal<Vec<BacklinkEntry>>,
  dark_mode: Signal<bool>,
  on_open_doc: Rc<dyn Fn(Uuid)>,
) -> impl IntoElement {
  when(move || !backlinks.get().is_empty(), {
    let on_open_doc = on_open_doc.clone();
    move || {
      let open_outer = on_open_doc.clone();
      view()
        .test_id("editor-backlinks-panel")
        .w_full()
        .pt(px(16.0))
        .child(dynamic(move || {
          let items = backlinks.get();
          if items.is_empty() {
            return view().into_element();
          }
          let p = palette(dark_mode.get());
          let count = items.len();
          let mut col = view()
            .w_full()
            .gap(px(6.0))
            .child(view().w_full().h(px(1.0)).bg(p.border_subtle))
            .child(
              view()
                .pt(px(6.0))
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(mica_icons::link_icon(p.text_muted_hex))
                .child(
                  text(format!("{count} 个反向链接"))
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text_muted),
                ),
            );

          for (i, link) in items.into_iter().enumerate() {
            let open = open_outer.clone();
            let doc_id = link.doc_id;
            let title = if link.title.trim().is_empty() {
              "未命名".to_string()
            } else {
              link.title
            };
            col = col.child(
              button()
                .test_id(&format!("editor-backlink-{i}"))
                .accessibility_label(format!("打开反向链接 {title}"))
                .w_full()
                .px(px(8.0))
                .py(px(7.0))
                .flex_row()
                .items_center()
                .gap_2()
                .rounded(px(6.0))
                .bg(p.surface_sunken)
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || open(doc_id))
                .child(mica_icons::file_doc(p.text_muted_hex))
                .child(
                  text(title)
                    .flex_1()
                    .text_size(px(13.5))
                    .text_color(p.text_primary)
                    .truncate(),
                ),
            );
          }
          col.into_element()
        }))
    }
  })
}

#[allow(clippy::too_many_arguments)]
fn tools_tab_btn(
  tid: &'static str,
  label: &'static str,
  active: bool,
  badge: usize,
  dark_mode: Signal<bool>,
  p: MicaPalette,
  icon: Svg,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  let mut row = view()
    .flex_row()
    .items_center()
    .gap(px(5.0))
    .child(icon)
    .child(
      text(label)
        .text_size(px(12.5))
        .font_weight(if active { FontWeight::MEDIUM } else { FontWeight::NORMAL })
        .text_color(if active { p.text_primary } else { p.text_muted }),
    );
  if badge > 0 {
    row = row.child(
      text(format!("{badge}"))
        .text_size(px(11.0))
        .text_color(p.text_faint),
    );
  }

  button()
    .test_id(tid)
    .accessibility_label(label)
    .px_2()
    .py(px(5.0))
    .rounded(px(6.0))
    .bg(if active { p.surface_hover } else { p.surface_base })
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_press)
    .child(row)
}

fn right_sidebar_resize_handle(
  width: Signal<f32>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let drag_start_w = signal(RIGHT_SIDEBAR_DEFAULT_WIDTH);
  let handle_state = signal(ResizeHandleState::Idle);

  resize_handle()
    .axis(Axis::Horizontal)
    .edge(HandleEdge::Leading)
    .accessibility_label("拖拽调整右侧边栏宽度")
    .on_drag_start(move || {
      drag_start_w.set(width.get_untracked());
    })
    .on_drag_move(move |offset: &Points| {
      // Leading edge on a right-docked rail: dragging left (negative offset) widens it.
      let next = (drag_start_w.get_untracked() - offset.0)
        .clamp(RIGHT_SIDEBAR_MIN_WIDTH, RIGHT_SIDEBAR_MAX_WIDTH);
      width.set(next);
    })
    .on_drag_end(move |offset: &Points| {
      let next = (drag_start_w.get_untracked() - offset.0)
        .clamp(RIGHT_SIDEBAR_MIN_WIDTH, RIGHT_SIDEBAR_MAX_WIDTH);
      width.set(next);
    })
    .on_state_change(move |st: &ResizeHandleState| {
      handle_state.set(*st);
    })
    .child(
      view()
        .test_id("right-sidebar-resize-handle")
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

fn section_divider(dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w_full()
    .h(px(1.0))
    .bg(move || palette(dark_mode.get()).border_normal)
}

fn outline_section(props: Rc<RightSidebarProps>) -> impl IntoElement {
  let dark_mode = props.dark_mode;

  view()
    .gap_1()
    .child(
      text("页面大纲")
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_faint),
    )
    .child(dynamic(move || {
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

fn version_history_section(
  props: Rc<RightSidebarProps>,
  on_open_version_history: Option<Rc<dyn Fn()>>,
) -> impl IntoElement {
  let dark_mode = props.dark_mode;
  let p_count = props.clone();
  let p_save = props.clone();
  let p_list = props;
  let has_full_history = on_open_version_history.is_some();

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
        .bg(move || palette(dark_mode.get()).surface_raised)
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
    .child(when(move || has_full_history, {
      let open_vh = on_open_version_history.clone();
      move || {
        let cb = open_vh.clone();
        button()
          .test_id("right-open-full-history-btn")
          .accessibility_label("打开完整版本历史与对比")
          .w_full()
          .py_1()
          .px_2()
          .items_center()
          .justify_center()
          .rounded(px(6.0))
          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
          .on_press(move || {
            if let Some(f) = &cb {
              f();
            }
          })
          .child(
            text("完整历史与对比...")
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_muted),
          )
      }
    }))
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
            .bg(p.surface_raised)
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

fn comments_tab_view(
  extras: Rc<RightSidebarExtras>,
  dark_mode: Signal<bool>,
  replying_to: Signal<Option<String>>,
  reply_draft: Signal<String>,
) -> impl IntoElement {
  view()
    .test_id("right-comments-panel")
    .w_full()
    .px_3()
    .py_3()
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      let mut threads = extras.comments.get();
      if threads.is_empty() {
        return view()
          .py(px(24.0))
          .child(
            text("还没有评论。选中文字后选择「添加评论」。")
              .text_size(px(12.5))
              .text_color(p.text_muted),
          )
          .into_element();
      }

      // Open threads first, resolved second (1:1 with Flutter CommentPanel).
      threads.sort_by_key(|t| t.is_resolved);
      let active_reply_id = replying_to.get();

      let mut col = view().w_full().gap(px(18.0));
      for (i, thread) in threads.into_iter().enumerate() {
        let tid = thread.id.clone();
        let is_replying = active_reply_id.as_deref() == Some(tid.as_str());
        let quote_text = if thread.quote.is_empty() {
          "—".to_string()
        } else {
          thread.quote.clone()
        };

        let quote_el = if thread.is_orphaned {
          text(quote_text)
            .text_size(px(12.0))
            .text_color(p.text_muted)
            .line_through()
            .into_element()
        } else {
          text(quote_text)
            .text_size(px(12.0))
            .text_color(p.text_muted)
            .into_element()
        };

        let focus_cb = extras.on_focus_thread.clone();
        let tid_focus = tid.clone();
        let can_focus = !thread.is_orphaned && focus_cb.is_some();

        let mut card = view().test_id(&format!("comment-thread-{i}")).w_full().gap(px(6.0)).child(
          button()
            .test_id(&format!("comment-quote-{i}"))
            .accessibility_label("定位到被评论的文字")
            .w_full()
            .px_2()
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(if thread.is_orphaned {
              p.surface_sunken
            } else {
              p.comment_highlight
            })
            .on_press(move || {
              if can_focus
                && let Some(f) = &focus_cb
              {
                f(tid_focus.clone());
              }
            })
            .child(quote_el),
        );

        if thread.is_orphaned {
          card = card.child(
            text("被评论的文字已被删除")
              .text_size(px(11.0))
              .text_color(p.text_faint),
          );
        }

        if thread.is_resolved {
          card = card.child(
            view()
              .flex_row()
              .items_center()
              .gap_1()
              .child(mica_icons::check_icon(p.text_faint_hex))
              .child(text("已解决").text_size(px(11.0)).text_color(p.text_faint)),
          );
        }

        for c in &thread.comments {
          let mut item_col = view().w_full().gap(px(2.0));
          if !c.author.is_empty() {
            item_col = item_col.child(
              text(format!("{} · {}", c.author, c.created_at))
                .text_size(px(11.0))
                .text_color(p.text_faint),
            );
          }
          item_col = item_col.child(
            text(c.body.clone())
              .text_size(px(12.5))
              .text_color(p.text_primary),
          );
          card = card.child(item_col);
        }

        if is_replying {
          let reply_cb_sub = extras.on_reply_comment.clone();
          let tid_sub = tid.clone();
          card = card.child(
            input()
              .test_id(&format!("comment-reply-input-{i}"))
              .w_full()
              .h(px(32.0))
              .px_2()
              .rounded(px(6.0))
              .border_1()
              .border_color(p.border_normal)
              .bg(p.surface_raised)
              .text_size(px(12.5))
              .text_color(p.text_primary)
              .auto_focus(true)
              .placeholder("回复…")
              .value(move || reply_draft.get())
              .on_change(move |v: &str| reply_draft.set(v.to_owned()))
              .on_submit(move |v: &str| {
                let body = v.trim();
                if !body.is_empty()
                  && let Some(f) = &reply_cb_sub
                {
                  f(tid_sub.clone(), body.to_string());
                  reply_draft.set(String::new());
                  replying_to.set(None);
                }
              }),
          );
        }

        let mut actions = view().w_full().flex_row().items_center().gap_2();
        if is_replying {
          let reply_cb_btn = extras.on_reply_comment.clone();
          let tid_btn = tid.clone();
          actions = actions.child(
            button()
              .test_id(&format!("comment-post-{i}"))
              .px_2()
              .py_1()
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                let body = reply_draft.get_untracked().trim().to_string();
                if !body.is_empty()
                  && let Some(f) = &reply_cb_btn
                {
                  f(tid_btn.clone(), body);
                  reply_draft.set(String::new());
                  replying_to.set(None);
                }
              })
              .child(text("评论").text_size(px(12.0)).text_color(p.accent)),
          );
        } else {
          let tid_open = tid.clone();
          actions = actions.child(
            button()
              .test_id(&format!("comment-reply-{i}"))
              .px_2()
              .py_1()
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                reply_draft.set(String::new());
                replying_to.set(Some(tid_open.clone()));
              })
              .child(text("回复").text_size(px(12.0)).text_color(p.text_muted)),
          );
        }

        let resolve_cb = extras.on_set_comment_resolved.clone();
        let tid_res = tid.clone();
        let next_resolved = !thread.is_resolved;
        actions = actions.child(
          button()
            .test_id(&format!("comment-resolve-{i}"))
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || {
              if let Some(f) = &resolve_cb {
                f(tid_res.clone(), next_resolved);
              }
            })
            .child(
              text(if thread.is_resolved {
                "重新打开"
              } else {
                "标记已解决"
              })
              .text_size(px(12.0))
              .text_color(p.text_muted),
            ),
        );

        actions = actions.child(view().flex_1());

        if thread.can_delete {
          let del_cb = extras.on_delete_comment.clone();
          let tid_del = tid.clone();
          actions = actions.child(
            button()
              .test_id(&format!("comment-delete-{i}"))
              .accessibility_label("删除该评论")
              .w(px(24.0))
              .h(px(24.0))
              .items_center()
              .justify_center()
              .rounded(px(6.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).danger_wash))
              .on_press(move || {
                if let Some(f) = &del_cb {
                  f(tid_del.clone());
                }
              })
              .child(mica_icons::trash_icon(p.text_faint_hex)),
          );
        }

        card = card.child(actions);
        col = col.child(card);
      }
      col.into_element()
    }))
}

fn comment_bubble_icon(color: &str) -> Svg {
  let doc = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 5.2H15.5A1.5 1.5 0 0 1 17 6.7V12.5A1.5 1.5 0 0 1 15.5 14H8.5L4.8 16.8V14H4.5A1.5 1.5 0 0 1 3 12.5V6.7A1.5 1.5 0 0 1 4.5 5.2Z"/></svg>"##
  );
  svg(ImageSrc::svg_markup(&doc)).w(px(15.0)).h(px(15.0))
}
