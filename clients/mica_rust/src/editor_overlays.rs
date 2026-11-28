//! Floating editor overlays and toolbars:
//! - Caret-anchored `/` slash menu
//! - Page `⋯` menu & link editor popover
//! - `Ctrl+F` find bar
//! - Selection formatting bubble menu & icon helpers

use std::rc::Rc;

use mica_editor::{EditorCommand, FindMatch};
use nsh_base::{button, input};
use nsh_core::{
  SubmitBehavior,
  element::{Align, ElementRef, ImageSrc, Placement, Svg, clipboard_string, overlay, svg},
  geometry::{Point, Points, px},
  prelude::*,
};

use crate::{
  editor_bridge::MicaDocumentBridge,
  editor_keys::{close_slash, execute_slash_item, toggle_block_type, write_rich_clipboard},
  editor_slash::filter_slash_items,
  editor_view::{EditorHeaderProps, SLASH_MENU_MAX_H, SLASH_MENU_WIDTH, SlashMenuSignals},
  mica_icons,
  mica_theme::palette,
};

/// Caret-anchored floating 22-item vertical slash menu (exact Flutter `editor.dart` layout).
pub(crate) fn caret_slash_menu_view(
  bridge: Rc<MicaDocumentBridge>,
  header: Rc<EditorHeaderProps>,
  slash: SlashMenuSignals,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let ai_enabled = header.ai_enabled;
  let on_ai = header.on_inline_ai.clone();
  let on_img = header.on_insert_image.clone();

  view()
    .test_id("slash-menu")
    .absolute()
    .left(move || px(slash.anchor.get().0))
    .top(move || px(slash.anchor.get().1))
    .w(px(SLASH_MENU_WIDTH))
    .max_h(px(SLASH_MENU_MAX_H))
    .py(px(6.0))
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .z_index(36)
    .child(
      scroll()
        .w_full()
        .max_h(px(SLASH_MENU_MAX_H - 12.0))
        .child(dynamic(move || {
          let q = slash.query.get();
          let active_idx = slash.index.get();
          let items = filter_slash_items(&q, ai_enabled.get());
          let p = palette(dark_mode.get());
          let mut list = view().w_full();
          for (i, item) in items.into_iter().enumerate() {
            let is_active = i == active_idx;
            let b = bridge.clone();
            let ai_cb = on_ai.clone();
            let img_cb = on_img.clone();
            let item_copy = *item;
            let icon_color = slash_icon_hex(item_copy.icon_key, &p);
            list = list.child(
              button()
                .test_id(item_copy.id)
                .accessibility_label(format!("插入 {}", item_copy.label))
                .w_full()
                .px(px(12.0))
                .py(px(7.0))
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .bg(if is_active {
                  p.surface_hover
                } else {
                  p.surface_overlay
                })
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  close_slash(slash);
                  execute_slash_item(&b, item_copy, &ai_cb, &img_cb);
                })
                .child(slash_item_icon(item_copy.icon_key, icon_color))
                .child(
                  text(item_copy.label)
                    .text_size(px(13.5))
                    .text_color(p.text_primary)
                    .truncate(),
                ),
            );
          }
          list
        })),
    )
}

fn slash_icon_hex(key: &str, p: &crate::mica_theme::MicaPalette) -> &'static str {
  match key {
    "sparkles" => p.accent_hex,
    "alert_note" => p.accent_hex,
    "alert_tip" => p.success_hex,
    "alert_important" => p.accent_hex,
    "alert_warning" => p.warning_hex,
    "alert_caution" => p.danger_hex,
    _ => p.text_muted_hex,
  }
}

fn slash_item_icon(key: &str, color: &'static str) -> Svg {
  match key {
    "sparkles" => mica_icons::sparkles_icon(color),
    "text" => mica_icons::align_text(color),
    "h1" => mica_icons::heading_box("1", color),
    "h2" => mica_icons::heading_box("2", color),
    "h3" => mica_icons::heading_box("3", color),
    "bullet" => mica_icons::list_bullet(color),
    "ordered" => mica_icons::list_ordered(color),
    "todo" => mica_icons::todo_box(color),
    "quote" => mica_icons::quote_icon(color),
    "alert_note" | "alert_tip" | "alert_important" | "alert_warning" | "alert_caution" => {
      mica_icons::info_icon(color)
    }
    "code" => mica_icons::code_icon(color),
    "math" => math_icon(color),
    "whiteboard" => mica_icons::whiteboard(color),
    "mindmap" | "flowchart" => mica_icons::graph_nodes(color),
    "table" => mica_icons::table_icon(color),
    "divider" => mica_icons::divider_icon(color),
    "image" => image_icon(color),
    _ => mica_icons::file_doc(color),
  }
}

/// Full `⋯` page menu matching `main:clients/mica_flutter/lib/ui/page_header_actions.dart`.
///
/// Rendered in a top-level [`overlay()`] anchored to the `ed-more` trigger button
/// so outside clicks and `Escape` dismiss it reliably across the whole window.
pub(crate) fn page_more_menu_view(
  header: Rc<EditorHeaderProps>,
  bridge: Rc<MicaDocumentBridge>,
  page_menu_open: Signal<bool>,
  compact: Memo<bool>,
  anchor_ref: &ElementRef,
  dark_mode: Signal<bool>,
) -> impl IntoElement + use<> {
  let comments_vis = header.comments_visible;
  let props_vis = header.show_properties;
  let on_board = header.on_open_board.clone();
  let on_comments = header.on_toggle_comments.clone();
  let on_props = header.on_toggle_properties.clone();
  let on_zip = header.on_export_zip.clone();
  let on_html = header.on_export_html.clone();
  let on_pdf = header.on_export_pdf.clone();
  let on_import = header.on_import_markdown_picker.clone();
  let on_ver = header.on_open_version_history.clone();
  let on_snap = header.on_save_snapshot.clone();
  let on_trash = header.on_trash_page.clone();
  let page_id_sig = header.page_id_str;
  let ws_name_sig = header.workspace_name;
  let folder_name_sig = header.folder_name;
  let is_compact = compact.get_untracked();

  let mut menu = view()
    .test_id("page-more-menu")
    .w(px(220.0))
    .p_1()
    .gap(px(2.0))
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .shadow_md();

  if is_compact {
    let br_link = bridge.clone();
    let br_path = bridge.clone();
    menu = menu
      .child(menu_row("page-menu-board", "打开无限白板", dark_mode, move || {
        on_board();
        page_menu_open.set(false);
      }))
      .child(menu_row("page-menu-copy-link", "复制页面链接", dark_mode, move || {
        let id = page_id_sig.get_untracked();
        set_clipboard(format!("mica://page/{id}"));
        br_link.set_status_flash("已复制链接");
        page_menu_open.set(false);
      }))
      .child(menu_row("page-menu-copy-path", "复制页面路径", dark_mode, move || {
        let mut segs: Vec<String> = Vec::new();
        let ws = ws_name_sig.get_untracked();
        if !ws.trim().is_empty() {
          segs.push(ws.trim().to_string());
        }
        for part in folder_name_sig.get_untracked().split(" > ") {
          let p = part.trim();
          if !p.is_empty() {
            segs.push(p.to_string());
          }
        }
        if !segs.is_empty() {
          set_clipboard(segs.join("/"));
          br_path.set_status_flash("已复制页面路径");
        }
        page_menu_open.set(false);
      }))
      .child(menu_sep(dark_mode));
  }

  overlay()
    .anchor(anchor_ref)
    .placement(Placement::Bottom)
    .align(Align::End)
    .offset(px(4.0))
    .margin(px(8.0))
    .on_press_outside(move |_: &Point<Points>| page_menu_open.set(false))
    .on_cancel(move || page_menu_open.set(false))
    .child(
      menu
        .child(menu_check_row(
          "page-menu-comments",
          "评论",
          comments_vis,
          dark_mode,
          move || {
            on_comments();
            page_menu_open.set(false);
          },
        ))
        .child(menu_check_row(
          "page-menu-properties",
          "属性",
          props_vis,
          dark_mode,
          move || {
            on_props();
            page_menu_open.set(false);
          },
        ))
        .child(menu_sep(dark_mode))
        .child(menu_row("page-menu-copy-md", "复制页面内容", dark_mode, move || {
          set_clipboard(bridge.export_markdown());
          bridge.set_status_flash("已复制页面内容");
          page_menu_open.set(false);
        }))
        .child(menu_sep(dark_mode))
        .child(menu_row("page-menu-export-zip", "导出(ZIP,含图片)", dark_mode, move || {
          on_zip();
          page_menu_open.set(false);
        }))
        .child(menu_row("page-menu-export-html", "导出为 HTML", dark_mode, move || {
          on_html();
          page_menu_open.set(false);
        }))
        .child(menu_row("page-menu-export-pdf", "导出为 PDF", dark_mode, move || {
          on_pdf();
          page_menu_open.set(false);
        }))
        .child(menu_sep(dark_mode))
        .child(menu_row("page-menu-import-md", "导入 Markdown…", dark_mode, move || {
          on_import();
          page_menu_open.set(false);
        }))
        .child(menu_sep(dark_mode))
        .child(menu_row("page-menu-version-history", "版本历史", dark_mode, move || {
          on_ver();
          page_menu_open.set(false);
        }))
        .child(menu_row("page-menu-snapshot", "保存检查点", dark_mode, move || {
          on_snap();
          page_menu_open.set(false);
        }))
        .child(menu_sep(dark_mode))
        .child(menu_row("page-menu-trash", "移入回收站", dark_mode, move || {
          on_trash();
          page_menu_open.set(false);
        })),
    )
}

/// Floating inline link URL editor popover.
pub(crate) fn link_editor_popover(
  bridge: Rc<MicaDocumentBridge>,
  link_draft: Signal<String>,
  link_prompt_open: Signal<bool>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let b_apply = bridge.clone();
  let b_submit = bridge.clone();
  let b_remove = bridge;

  view()
    .test_id("link-editor-popover")
    .absolute()
    .top(px(46.0))
    .right(px(24.0))
    .z_index(38)
    .px_2()
    .py_1()
    .flex_row()
    .items_center()
    .gap_1()
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .child(dynamic(move || {
      mica_icons::link_icon(palette(dark_mode.get()).text_muted_hex)
    }))
    .child(
      input()
        .test_id("link-url-input")
        .auto_focus(true)
        .submit_behavior(SubmitBehavior::Submit)
        .placeholder("https://...")
        .w(px(220.0))
        .px_2()
        .py_1()
        .rounded(px(5.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .text_size(px(12.5))
        .text_color(move || palette(dark_mode.get()).text_primary)
        .bg(move || palette(dark_mode.get()).surface_base)
        .value(move || link_draft.get())
        .on_change(move |v: &str| link_draft.set(v.to_owned()))
        .on_submit(move |v: &str| {
          b_submit.set_link_mark(v);
          link_prompt_open.set(false);
          b_submit.request_focus();
        }),
    )
    .child(
      button()
        .test_id("link-apply-btn")
        .accessibility_label("应用链接")
        .px_2()
        .py_1()
        .rounded(px(5.0))
        .bg(move || palette(dark_mode.get()).accent)
        .on_press(move || {
          b_apply.set_link_mark(&link_draft.get_untracked());
          link_prompt_open.set(false);
          b_apply.request_focus();
        })
        .child(
          text("应用")
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).on_primary),
        ),
    )
    .child(
      button()
        .test_id("link-remove-btn")
        .accessibility_label("移除链接")
        .px_2()
        .py_1()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || {
          b_remove.unset_link_mark();
          link_prompt_open.set(false);
          b_remove.request_focus();
        })
        .child(
          text("移除")
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).text_muted),
        ),
    )
    .child(
      button()
        .test_id("link-close-btn")
        .accessibility_label("关闭链接编辑")
        .w(px(22.0))
        .h(px(22.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || link_prompt_open.set(false))
        .child(dynamic(move || {
          mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
        })),
    )
}

pub(crate) fn step_find_match(
  bridge: &MicaDocumentBridge,
  find_matches: Signal<Vec<FindMatch>>,
  find_index: Signal<usize>,
  delta: isize,
) {
  let hits = find_matches.get_untracked();
  if hits.is_empty() {
    return;
  }
  let len = hits.len() as isize;
  let cur = (find_index.get_untracked() as isize).clamp(0, len - 1);
  let next = ((cur + delta).rem_euclid(len)) as usize;
  find_index.set(next);
  bridge.select_find_match(&hits[next], next);
}

/// Floating top-right in-page find bar (`Ctrl+F`), 1:1 with Flutter `_buildFindBar`.
pub(crate) fn find_bar_view(
  bridge: Rc<MicaDocumentBridge>,
  find_open: Signal<bool>,
  find_query: Signal<String>,
  find_matches: Signal<Vec<FindMatch>>,
  find_index: Signal<usize>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let b_chg = bridge.clone();
  let b_submit = bridge.clone();
  let b_prev = bridge.clone();
  let b_next = bridge.clone();
  let b_close_key = bridge.clone();
  let b_close_btn = bridge;

  view()
    .test_id("editor-find-bar")
    .absolute()
    .top(px(46.0))
    .right(px(14.0))
    .z_index(37)
    .px_2()
    .py_1()
    .flex_row()
    .items_center()
    .gap(px(4.0))
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .child(dynamic(move || {
      mica_icons::search(palette(dark_mode.get()).text_muted_hex)
    }))
    .child(
      input()
        .test_id("editor-find-input")
        .auto_focus(true)
        .submit_behavior(SubmitBehavior::Submit)
        .placeholder("页内查找")
        .w(px(172.0))
        .px_1()
        .py_1()
        .text_size(px(13.0))
        .text_color(move || palette(dark_mode.get()).text_primary)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .value(move || find_query.get())
        .on_change(move |q: &str| {
          find_query.set(q.to_owned());
          let hits = b_chg.find_matches(q);
          find_index.set(0);
          if let Some(first) = hits.first() {
            b_chg.select_find_match(first, 0);
          }
          find_matches.set(hits);
        })
        .on_submit(move |_: &str| {
          step_find_match(&b_submit, find_matches, find_index, 1);
        })
        .on_key_press(move |k: &str| {
          if k == "Escape" {
            find_open.set(false);
            b_close_key.clear_find_highlights();
            b_close_key.request_focus();
          }
        }),
    )
    .child(
      view()
        .test_id("editor-find-count")
        .w(px(48.0))
        .items_end()
        .child(
          text(move || {
            let q = find_query.get();
            if q.is_empty() {
              String::new()
            } else {
              let total = find_matches.get().len();
              if total == 0 {
                "无结果".to_string()
              } else {
                format!("{}/{}", find_index.get().min(total - 1) + 1, total)
              }
            }
          })
          .text_size(px(11.5))
          .text_color(move || palette(dark_mode.get()).text_faint),
        ),
    )
    .child(
      button()
        .test_id("editor-find-prev")
        .accessibility_label("上一个 (Shift+Enter)")
        .w(px(24.0))
        .h(px(24.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || step_find_match(&b_prev, find_matches, find_index, -1))
        .child(dynamic(move || {
          chevron_up_icon(palette(dark_mode.get()).text_muted_hex)
        })),
    )
    .child(
      button()
        .test_id("editor-find-next")
        .accessibility_label("下一个 (Enter)")
        .w(px(24.0))
        .h(px(24.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || step_find_match(&b_next, find_matches, find_index, 1))
        .child(dynamic(move || {
          mica_icons::chevron_down(palette(dark_mode.get()).text_muted_hex)
        })),
    )
    .child(
      button()
        .test_id("editor-find-close")
        .accessibility_label("关闭 (Esc)")
        .w(px(24.0))
        .h(px(24.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || {
          find_open.set(false);
          b_close_btn.clear_find_highlights();
          b_close_btn.request_focus();
        })
        .child(dynamic(move || {
          mica_icons::close_sm(palette(dark_mode.get()).text_muted_hex)
        })),
    )
}

/// Expanded floating selection bubble menu (marks + link + block transforms + AI rewrite).
pub(crate) fn bubble_menu_view(
  bridge: Rc<MicaDocumentBridge>,
  header: Rc<EditorHeaderProps>,
  has_text_selection: Signal<bool>,
  link_prompt_open: Signal<bool>,
  link_draft: Signal<String>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  when(move || has_text_selection.get(), move || {
    let active_marks = bridge.active_marks_signal();
    let active_kind = bridge.active_kind_signal();
    let active_level = bridge.active_level_signal();
    let ai_enabled = header.ai_enabled;
    let on_ai = header.on_inline_ai.clone();

    let b_bold = bridge.clone();
    let b_italic = bridge.clone();
    let b_strike = bridge.clone();
    let b_highlight = bridge.clone();
    let b_code = bridge.clone();
    let b_link = bridge.clone();
    let b_h1 = bridge.clone();
    let b_h2 = bridge.clone();
    let b_h3 = bridge.clone();
    let b_para = bridge.clone();
    let b_bullet = bridge.clone();
    let b_ordered = bridge.clone();
    let b_todo = bridge.clone();
    let b_outdent = bridge.clone();
    let b_indent = bridge.clone();
    let b_quote = bridge.clone();
    let b_code_blk = bridge.clone();
    let b_ai = bridge.clone();

    view()
      .test_id("editor-bubble-menu")
      .flex_row()
      .items_center()
      .gap(px(2.0))
      .px_1()
      .py_1()
      .rounded(px(8.0))
      .border_1()
      .border_color(move || palette(dark_mode.get()).border_normal)
      .bg(move || palette(dark_mode.get()).surface_overlay)
      .child(bubble_btn(
        "bubble-bold",
        "加粗",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "bold" || m == "strong")),
        move || b_bold.toggle_mark("bold"),
        mica_icons::bold_icon,
      ))
      .child(bubble_btn(
        "bubble-italic",
        "斜体",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "italic" || m == "em")),
        move || b_italic.toggle_mark("italic"),
        mica_icons::italic_icon,
      ))
      .child(bubble_btn(
        "bubble-strike",
        "删除线",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "strike")),
        move || b_strike.toggle_mark("strike"),
        mica_icons::strikethrough_icon,
      ))
      .child(bubble_btn(
        "bubble-highlight",
        "高亮",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "highlight")),
        move || b_highlight.toggle_mark("highlight"),
        mica_icons::highlight_icon,
      ))
      .child(bubble_btn(
        "bubble-code",
        "行内代码",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "code")),
        move || b_code.toggle_mark("code"),
        mica_icons::code_icon,
      ))
      .child(bubble_btn(
        "bubble-link",
        "链接",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "link")),
        move || {
          link_draft.set(b_link.current_link_href().unwrap_or_default());
          link_prompt_open.set(true);
        },
        mica_icons::link_icon,
      ))
      .child(toolbar_sep(dark_mode))
      .child(bubble_btn(
        "bubble-h1",
        "一级标题",
        dark_mode,
        computed(move || active_kind.get() == "heading" && active_level.get() == 1),
        move || toggle_block_type(&b_h1, "heading", Some(1)),
        |c| mica_icons::heading_box("1", c),
      ))
      .child(bubble_btn(
        "bubble-h2",
        "二级标题",
        dark_mode,
        computed(move || active_kind.get() == "heading" && active_level.get() == 2),
        move || toggle_block_type(&b_h2, "heading", Some(2)),
        |c| mica_icons::heading_box("2", c),
      ))
      .child(bubble_btn(
        "bubble-h3",
        "三级标题",
        dark_mode,
        computed(move || active_kind.get() == "heading" && active_level.get() == 3),
        move || toggle_block_type(&b_h3, "heading", Some(3)),
        |c| mica_icons::heading_box("3", c),
      ))
      .child(bubble_btn(
        "bubble-para",
        "正文",
        dark_mode,
        computed(move || active_kind.get() == "paragraph"),
        move || b_para.set_block_type("paragraph", None),
        mica_icons::align_text,
      ))
      .child(bubble_btn(
        "bubble-bullet",
        "无序列表",
        dark_mode,
        computed(move || matches!(active_kind.get().as_str(), "bulleted_list" | "bulletList")),
        move || toggle_block_type(&b_bullet, "bulleted_list", None),
        mica_icons::list_bullet,
      ))
      .child(bubble_btn(
        "bubble-ordered",
        "有序列表",
        dark_mode,
        computed(move || matches!(active_kind.get().as_str(), "numbered_list" | "orderedList")),
        move || toggle_block_type(&b_ordered, "numbered_list", None),
        mica_icons::list_ordered,
      ))
      .child(bubble_btn(
        "bubble-todo",
        "待办",
        dark_mode,
        computed(move || matches!(active_kind.get().as_str(), "todo" | "taskItem")),
        move || toggle_block_type(&b_todo, "todo", None),
        mica_icons::todo_box,
      ))
      .child(bubble_btn(
        "bubble-outdent",
        "减少缩进 (Shift+Tab)",
        dark_mode,
        computed(|| false),
        move || {
          b_outdent.dispatch(EditorCommand::Indent { delta: -1 });
          b_outdent.request_focus();
        },
        mica_icons::indent_decrease,
      ))
      .child(bubble_btn(
        "bubble-indent",
        "增加缩进 (Tab)",
        dark_mode,
        computed(|| false),
        move || {
          b_indent.dispatch(EditorCommand::Indent { delta: 1 });
          b_indent.request_focus();
        },
        mica_icons::indent_increase,
      ))
      .child(bubble_btn(
        "bubble-quote",
        "引用",
        dark_mode,
        computed(move || matches!(active_kind.get().as_str(), "quote" | "blockquote")),
        move || toggle_block_type(&b_quote, "quote", None),
        mica_icons::quote_icon,
      ))
      .child(bubble_btn(
        "bubble-code-block",
        "代码块",
        dark_mode,
        computed(move || active_kind.get() == "code_block"),
        move || b_code_blk.insert_custom_block("code_block", "", None),
        terminal_icon,
      ))
      .child(when(move || ai_enabled.get(), {
        let on_ai = on_ai.clone();
        let b_ai = b_ai.clone();
        move || {
          let on_ai = on_ai.clone();
          let b_ai = b_ai.clone();
          view()
            .flex_row()
            .items_center()
            .child(toolbar_sep(dark_mode))
            .child(
              button()
                .test_id("bubble-ai")
                .accessibility_label("用 AI 改写所选")
                .h(px(26.0))
                .px_2()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .rounded(px(5.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || on_ai(b_ai.selected_text()))
                .child(dynamic(move || {
                  mica_icons::sparkles_icon(palette(dark_mode.get()).accent_hex)
                }))
                .child(
                  text("用 AI 改写所选")
                    .text_size(px(11.5))
                    .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            )
        }
      }))
  })
}

fn bubble_btn<F, I>(
  tid: &'static str,
  a11y: &'static str,
  dark_mode: Signal<bool>,
  active: Memo<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  crate::tooltip::with_tooltip(
    button()
      .test_id(tid)
      .accessibility_label(a11y)
      .w(px(28.0))
      .h(px(28.0))
      .items_center()
      .justify_center()
      .rounded(px(5.0))
      .bg(move || {
        let p = palette(dark_mode.get());
        if active.get() {
          p.surface_hover
        } else {
          p.surface_overlay
        }
      })
      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
      .on_press(move || {
        crate::tooltip::hide_tooltip();
        on_click();
      })
      .child(dynamic(move || {
        let p = palette(dark_mode.get());
        icon_fn(if active.get() {
          p.text_primary_hex
        } else {
          p.text_muted_hex
        })
      })),
    a11y,
  )
}

fn local_stroked(size_px: f32, color: &str, inner: &str) -> Svg {
  let doc = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{inner}</svg>"##
  );
  svg(ImageSrc::svg_markup(&doc)).w(px(size_px)).h(px(size_px))
}

pub(crate) fn terminal_icon(color: &str) -> Svg {
  local_stroked(
    17.0,
    color,
    r##"<rect x="2.8" y="4" width="14.4" height="12" rx="2"/><path d="M6.2 8.2L8.6 10.2L6.2 12.2M10.5 12.2H13.8"/>"##,
  )
}

pub(crate) fn image_icon(color: &str) -> Svg {
  local_stroked(
    17.0,
    color,
    r##"<rect x="3" y="4" width="14" height="12" rx="2"/><circle cx="7.5" cy="8" r="1.2"/><path d="M3.8 14L7.8 10.2L10.8 13L13.2 10.8L16.4 14"/>"##,
  )
}

pub(crate) fn lock_icon(color: &str) -> Svg {
  local_stroked(
    14.0,
    color,
    r##"<rect x="4.5" y="9" width="11" height="7.5" rx="1.8"/><path d="M7 9V6.8A3 3 0 0 1 13 6.8V9"/>"##,
  )
}

pub(crate) fn chevron_up_icon(color: &str) -> Svg {
  local_stroked(12.0, color, r##"<path d="M5 12.5L10 7.5L15 12.5"/>"##)
}

pub(crate) fn math_icon(color: &str) -> Svg {
  local_stroked(
    15.0,
    color,
    r##"<path d="M14.5 5H6.5L11 10L6.5 15H14.5"/>"##,
  )
}

pub(crate) fn toolbar_btn<F, I>(
  tid: &'static str,
  a11y: &'static str,
  dark_mode: Signal<bool>,
  active: Signal<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  toolbar_btn_inner(tid, a11y, None, dark_mode, active, on_click, icon_fn)
}

/// Variant of [`toolbar_btn`] that binds an [`ElementRef`] to the button so a
/// top-level [`overlay()`] popover can anchor to it.
pub(crate) fn toolbar_btn_with_ref<F, I>(
  tid: &'static str,
  a11y: &'static str,
  el_ref: &ElementRef,
  dark_mode: Signal<bool>,
  active: Signal<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement + 'static
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  toolbar_btn_inner(tid, a11y, Some(el_ref), dark_mode, active, on_click, icon_fn)
}

fn toolbar_btn_inner<F, I>(
  tid: &'static str,
  a11y: &'static str,
  el_ref: Option<&ElementRef>,
  dark_mode: Signal<bool>,
  active: Signal<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement + 'static
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  let mut btn = button()
    .test_id(tid)
    .accessibility_label(a11y)
    .w(px(28.0))
    .h(px(28.0))
    .items_center()
    .justify_center()
    .rounded(px(5.0))
    .bg(move || {
      let p = palette(dark_mode.get());
      if active.get() {
        p.surface_hover
      } else {
        p.surface_base
      }
    })
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(move || {
      crate::tooltip::hide_tooltip();
      on_click();
    })
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      icon_fn(if active.get() {
        p.text_primary_hex
      } else {
        p.text_muted_hex
      })
    }));
  if let Some(r) = el_ref {
    btn = btn.element_ref(r);
  }
  crate::tooltip::with_tooltip(btn, a11y)
}

pub(crate) fn toolbar_btn_active<F, I>(
  tid: &'static str,
  a11y: &'static str,
  dark_mode: Signal<bool>,
  active: Memo<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  crate::tooltip::with_tooltip(
    button()
      .test_id(tid)
      .accessibility_label(a11y)
      .w(px(28.0))
      .h(px(28.0))
      .items_center()
      .justify_center()
      .rounded(px(5.0))
      .bg(move || {
        let p = palette(dark_mode.get());
        if active.get() {
          p.surface_hover
        } else {
          p.surface_base
        }
      })
      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
      .on_press(move || {
        crate::tooltip::hide_tooltip();
        on_click();
      })
      .child(dynamic(move || {
        let p = palette(dark_mode.get());
        icon_fn(if active.get() {
          p.text_primary_hex
        } else {
          p.text_muted_hex
        })
      })),
    a11y,
  )
}

pub(crate) fn toolbar_btn_dim<F, I>(
  tid: &'static str,
  a11y: &'static str,
  dark_mode: Signal<bool>,
  enabled: Memo<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  crate::tooltip::with_tooltip(
    button()
      .test_id(tid)
      .accessibility_label(a11y)
      .w(px(28.0))
      .h(px(28.0))
      .items_center()
      .justify_center()
      .rounded(px(5.0))
      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
      .on_press(move || {
        crate::tooltip::hide_tooltip();
        on_click();
      })
      .child(dynamic(move || {
        let p = palette(dark_mode.get());
        icon_fn(if enabled.get() {
          p.text_muted_hex
        } else {
          p.text_ghost_hex
        })
      })),
    a11y,
  )
}

pub(crate) fn toolbar_sep(dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w(px(1.0))
    .h(px(14.0))
    .mx(px(2.0))
    .bg(move || palette(dark_mode.get()).border_normal)
}

fn menu_sep(dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w_full()
    .h(px(1.0))
    .my(px(2.0))
    .bg(move || palette(dark_mode.get()).border_subtle)
}

fn menu_row(
  tid: &'static str,
  label: &'static str,
  dark_mode: Signal<bool>,
  on_click: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .w_full()
    .px_3()
    .py_1()
    .rounded(px(6.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(
      text(label)
        .text_size(px(12.5))
        .text_color(move || palette(dark_mode.get()).text_primary),
    )
}

fn menu_check_row(
  tid: &'static str,
  label: &'static str,
  checked: Signal<bool>,
  dark_mode: Signal<bool>,
  on_click: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .w_full()
    .px_3()
    .py_1()
    .flex_row()
    .items_center()
    .justify_between()
    .rounded(px(6.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(
      text(move || {
        if checked.get() {
          format!("✓ {label}")
        } else {
          label.to_string()
        }
      })
      .text_size(px(12.5))
      .text_color(move || palette(dark_mode.get()).text_primary),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn editor_context_menu_overlay(
  menu_pos: Signal<Option<(f32, f32)>>,
  bridge: Rc<MicaDocumentBridge>,
  header: Rc<EditorHeaderProps>,
  has_selection: Signal<bool>,
  is_cloud: Signal<bool>,
  viewport_bounds: Signal<(f32, f32)>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  when(move || menu_pos.get().is_some(), move || {
    let Some((raw_x, raw_y)) = menu_pos.get() else {
      return view().into_element();
    };
    let (vw, vh) = viewport_bounds.get_untracked();
    let max_x = if vw > 210.0 { vw - 208.0 } else { 8.0 };
    let max_y = if vh > 260.0 { vh - 248.0 } else { 8.0 };
    let left_x = raw_x.clamp(8.0, max_x.max(8.0));
    let top_y = raw_y.clamp(8.0, max_y.max(8.0));
    let has_sel = has_selection.get();
    let show_comment = has_sel && is_cloud.get();
    let (b_cp, b_ct, b_ps, b_pp, b_sa) = (
      bridge.clone(),
      bridge.clone(),
      bridge.clone(),
      bridge.clone(),
      bridge.clone(),
    );
    let on_comment = header.on_toggle_comments.clone();

    let ctx_btn = move |tid: &'static str, label: &'static str, enabled: bool, on_act: Rc<dyn Fn()>| {
      button()
        .test_id(tid)
        .accessibility_label(label)
        .w_full()
        .px_3()
        .py_1()
        .rounded(px(6.0))
        .hover(move |s| {
          if enabled {
            s.bg(palette(dark_mode.get_untracked()).surface_hover)
          } else {
            s
          }
        })
        .on_press(move || {
          if enabled {
            on_act();
          }
        })
        .child(
          text(label)
            .text_size(px(12.5))
            .text_color(move || {
              let p = palette(dark_mode.get());
              if enabled { p.text_primary } else { p.text_ghost }
            }),
        )
    };

    let mut card = view()
      .test_id("editor-context-menu")
      .absolute()
      .left(px(left_x))
      .top(px(top_y))
      .w(px(196.0))
      .p_1()
      .gap(px(2.0))
      .rounded(px(8.0))
      .border_1()
      .border_color(move || palette(dark_mode.get()).border_normal)
      .bg(move || palette(dark_mode.get()).surface_overlay)
      .shadow_md()
      .child(ctx_btn(
        "ed-ctx-copy",
        "复制",
        has_sel,
        Rc::new(move || {
          if let Some(payload) = b_cp.copy_selection() {
            write_rich_clipboard(payload);
            b_cp.set_status_flash("已复制");
          }
          menu_pos.set(None);
        }),
      ))
      .child(ctx_btn(
        "ed-ctx-cut",
        "剪切",
        has_sel,
        Rc::new(move || {
          if let Some(payload) = b_ct.copy_selection() {
            write_rich_clipboard(payload);
            b_ct.dispatch(EditorCommand::DeleteBackward);
            b_ct.set_status_flash("已剪切");
          }
          menu_pos.set(None);
        }),
      ));
    if show_comment {
      card = card.child(ctx_btn(
        "ed-ctx-comment",
        "添加评论",
        true,
        Rc::new(move || {
          menu_pos.set(None);
          on_comment();
        }),
      ));
    }
    card = card
      .child(menu_sep(dark_mode))
      .child(ctx_btn(
        "ed-ctx-paste",
        "粘贴",
        true,
        Rc::new(move || {
          menu_pos.set(None);
          let br = b_ps.clone();
          spawn(async move {
            let txt = clipboard_string().await;
            let html = crate::clipboard_platform::read_windows_cf_html(&txt).unwrap_or_default();
            if !txt.is_empty() || !html.is_empty() {
              br.paste_clipboard(&txt, &html, false);
            }
            br.request_focus();
          });
        }),
      ))
      .child(ctx_btn(
        "ed-ctx-paste-plain",
        "粘贴为纯文本",
        true,
        Rc::new(move || {
          menu_pos.set(None);
          let br = b_pp.clone();
          spawn(async move {
            let txt = clipboard_string().await;
            if !txt.is_empty() {
              br.paste_clipboard(&txt, "", true);
            }
            br.request_focus();
          });
        }),
      ))
      .child(menu_sep(dark_mode))
      .child(ctx_btn(
        "ed-ctx-select-all",
        "全选",
        true,
        Rc::new(move || {
          menu_pos.set(None);
          b_sa.select_all();
          b_sa.request_focus();
        }),
      ));

    overlay()
      .on_cancel(move || menu_pos.set(None))
      .child(
        view()
          .absolute()
          .inset_0()
          .z_index(78)
          .child(
            button()
              .test_id("editor-context-menu-dismiss")
              .accessibility_label("关闭编辑器右键菜单")
              .absolute()
              .inset_0()
              .on_press(move || menu_pos.set(None)),
          )
          .child(card),
      )
      .into_element()
  })
}
