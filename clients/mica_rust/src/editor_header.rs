//! Responsive editor page header:
//! - Selectable breadcrumb + copy-path button (left)
//! - Toggleable formatting toolbar (inline on wide viewports, clean 2nd row when narrower)
//! - Fixed-width `28×28px` sync/save status icon with dynamic hover tooltip,
//!   right-sidebar toggle, whiteboard, share, copy link, favorite star, and more menu.

use std::rc::Rc;

use mica_editor::EditorCommand;
use nsh_base::button;
use nsh_core::{
  element::{ElementRef, set_clipboard},
  geometry::px,
  prelude::*,
};

use crate::{
  editor_bridge::MicaDocumentBridge,
  editor_keys::{close_slash, toggle_block_type, update_slash_anchor},
  editor_overlays::{
    image_icon, lock_icon, terminal_icon, toolbar_btn, toolbar_btn_active, toolbar_btn_dim,
    toolbar_btn_with_ref, toolbar_sep,
  },
  editor_view::SlashMenuSignals,
  mica_icons,
  mica_theme::palette,
};

pub struct EditorHeaderProps {
  pub workspace_name: Signal<String>,
  pub folder_name: Signal<String>,
  pub page_title: Signal<String>,
  pub page_id_str: Signal<String>,
  pub is_favorite: Signal<bool>,
  pub right_panel_open: Signal<bool>,
  pub on_rename_title: Rc<dyn Fn(String)>,
  pub on_title_focus_change: Option<Rc<dyn Fn(bool)>>,
  pub on_open_board: Rc<dyn Fn()>,
  pub on_toggle_favorite: Rc<dyn Fn()>,
  pub on_toggle_right_panel: Rc<dyn Fn()>,
  pub on_trash_page: Rc<dyn Fn()>,
  pub on_export_markdown: Rc<dyn Fn()>,
  pub on_export_html: Rc<dyn Fn()>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  // ── 1:1 Parity Props ─────────────────────────────────────────────────────
  pub is_cloud_signed_in: Signal<bool>,
  pub sync_phase: Signal<mica_sync::SyncPhase>,
  pub sync_badge: Signal<String>,
  pub show_format_bar: Signal<bool>,
  pub show_page_title: Signal<bool>,
  pub show_properties: Signal<bool>,
  pub comments_visible: Signal<bool>,
  pub page_width: Signal<f32>,
  pub ai_enabled: Signal<bool>,
  pub on_share: Rc<dyn Fn()>,
  pub on_toggle_comments: Rc<dyn Fn()>,
  pub on_toggle_properties: Rc<dyn Fn()>,
  pub on_export_zip: Rc<dyn Fn()>,
  pub on_export_pdf: Rc<dyn Fn()>,
  pub on_import_markdown_picker: Rc<dyn Fn()>,
  pub on_open_version_history: Rc<dyn Fn()>,
  pub on_inline_ai: Rc<dyn Fn(String)>,
  pub on_insert_image: Rc<dyn Fn()>,
  pub image_ctx: crate::image_blocks::ImageContext,
  pub property_panel_slot: Option<Rc<dyn Fn() -> AnyElement>>,
  pub backlinks_panel_slot: Option<Rc<dyn Fn() -> AnyElement>>,
}

pub(crate) struct EditorHeaderRuntime {
  pub dark_mode: Signal<bool>,
  pub header_width: Signal<f32>,
  pub copied_link: Signal<bool>,
  pub page_menu_open: Signal<bool>,
  pub page_more_btn_ref: ElementRef,
  pub link_prompt_open: Signal<bool>,
  pub link_draft: Signal<String>,
  pub slash: SlashMenuSignals,
}

/// Format the dynamic hover tooltip label for the fixed-width top-right sync/save status icon.
pub(crate) fn format_sync_status_tooltip(sync_badge: &str, last_edited: &str) -> String {
  let badge = sync_badge.trim();
  let edited = last_edited.trim();
  if !badge.is_empty() {
    if edited.is_empty() {
      badge.to_string()
    } else {
      format!("{badge} · {edited}")
    }
  } else if edited.is_empty() {
    "本地已保存".to_string()
  } else {
    format!("本地已保存 · {edited}")
  }
}

pub(crate) fn editor_header_view(
  bridge: Rc<MicaDocumentBridge>,
  header: Rc<EditorHeaderProps>,
  rt: EditorHeaderRuntime,
) -> impl IntoElement {
  let dark_mode = rt.dark_mode;
  let header_width = rt.header_width;
  let header_single_row = computed(move || header_width.get() >= 1240.0);
  let header_compact = computed(move || header_width.get() < 640.0);
  let format_bar_scroll = computed(move || header_width.get() < 720.0);
  let copied_link = rt.copied_link;
  let page_menu_open = rt.page_menu_open;
  let page_more_btn_ref = rt.page_more_btn_ref;
  let link_prompt_open = rt.link_prompt_open;
  let link_draft = rt.link_draft;
  let slash = rt.slash;

  let copied_path = signal(false);
  let copied_path_gen = signal(0u64);

  let active_kind = bridge.active_kind_signal();
  let active_level = bridge.active_level_signal();
  let active_marks = bridge.active_marks_signal();
  let can_undo = bridge.can_undo_signal();
  let can_redo = bridge.can_redo_signal();
  let last_edited = bridge.last_edited_signal();

  let is_fav = header.is_favorite;
  let right_open = header.right_panel_open;
  let show_format_bar = header.show_format_bar;
  let is_cloud = header.is_cloud_signed_in;
  let sync_phase = header.sync_phase;
  let sync_badge = header.sync_badge;

  let render_format_bar: Rc<dyn Fn() -> AnyElement> = {
    let br = bridge.clone();
    let h_img = header.clone();
    Rc::new(move || {
      let b_undo = br.clone();
      let b_redo = br.clone();
      let b_para = br.clone();
      let b_h1 = br.clone();
      let b_h2 = br.clone();
      let b_h3 = br.clone();
      let b_bold = br.clone();
      let b_italic = br.clone();
      let b_strike = br.clone();
      let b_highlight = br.clone();
      let b_code = br.clone();
      let b_link_btn = br.clone();
      let b_list = br.clone();
      let b_ordered = br.clone();
      let b_todo = br.clone();
      let b_outdent = br.clone();
      let b_indent = br.clone();
      let b_quote = br.clone();
      let b_code_blk = br.clone();
      let b_div = br.clone();
      let b_tbl = br.clone();
      let b_slash_btn = br.clone();
      let h_img = h_img.clone();
      view()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap(px(1.0))
        .child(toolbar_btn_dim(
          "ed-undo",
          "撤销 (Ctrl+Z)",
          dark_mode,
          computed(move || can_undo.get()),
          move || b_undo.undo(),
          mica_icons::undo,
        ))
        .child(toolbar_btn_dim(
          "ed-redo",
          "重做 (Ctrl+Y)",
          dark_mode,
          computed(move || can_redo.get()),
          move || b_redo.redo(),
          mica_icons::redo,
        ))
        .child(toolbar_sep(dark_mode))
        .child(toolbar_btn_active(
          "ed-para",
          "正文段落",
          dark_mode,
          computed(move || active_kind.get() == "paragraph"),
          move || b_para.set_block_type("paragraph", None),
          mica_icons::align_text,
        ))
        .child(toolbar_btn_active(
          "ed-h1",
          "一级标题",
          dark_mode,
          computed(move || active_kind.get() == "heading" && active_level.get() == 1),
          move || toggle_block_type(&b_h1, "heading", Some(1)),
          |c| mica_icons::heading_box("1", c),
        ))
        .child(toolbar_btn_active(
          "ed-h2",
          "二级标题",
          dark_mode,
          computed(move || active_kind.get() == "heading" && active_level.get() == 2),
          move || toggle_block_type(&b_h2, "heading", Some(2)),
          |c| mica_icons::heading_box("2", c),
        ))
        .child(toolbar_btn_active(
          "ed-h3",
          "三级标题",
          dark_mode,
          computed(move || active_kind.get() == "heading" && active_level.get() == 3),
          move || toggle_block_type(&b_h3, "heading", Some(3)),
          |c| mica_icons::heading_box("3", c),
        ))
        .child(toolbar_sep(dark_mode))
        .child(toolbar_btn_active(
          "ed-bold",
          "加粗 (Ctrl+B)",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "bold" || m == "strong")),
          move || b_bold.toggle_mark("bold"),
          mica_icons::bold_icon,
        ))
        .child(toolbar_btn_active(
          "ed-italic",
          "斜体 (Ctrl+I)",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "italic" || m == "em")),
          move || b_italic.toggle_mark("italic"),
          mica_icons::italic_icon,
        ))
        .child(toolbar_btn_active(
          "ed-strike",
          "删除线 (Ctrl+Shift+S)",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "strike")),
          move || b_strike.toggle_mark("strike"),
          mica_icons::strikethrough_icon,
        ))
        .child(toolbar_btn_active(
          "ed-highlight",
          "高亮 (Ctrl+Shift+H)",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "highlight")),
          move || b_highlight.toggle_mark("highlight"),
          mica_icons::highlight_icon,
        ))
        .child(toolbar_btn_active(
          "ed-code",
          "行内代码",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "code")),
          move || b_code.toggle_mark("code"),
          mica_icons::code_icon,
        ))
        .child(toolbar_btn_active(
          "ed-link",
          "链接 (Ctrl+K)",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "link")),
          move || {
            link_draft.set(b_link_btn.current_link_href().unwrap_or_default());
            link_prompt_open.set(!link_prompt_open.get_untracked());
          },
          mica_icons::link_icon,
        ))
        .child(toolbar_sep(dark_mode))
        .child(toolbar_btn_active(
          "ed-list",
          "无序列表",
          dark_mode,
          computed(move || matches!(active_kind.get().as_str(), "bulleted_list" | "bulletList")),
          move || toggle_block_type(&b_list, "bulleted_list", None),
          mica_icons::list_bullet,
        ))
        .child(toolbar_btn_active(
          "ed-ordered",
          "有序列表",
          dark_mode,
          computed(move || matches!(active_kind.get().as_str(), "numbered_list" | "orderedList")),
          move || toggle_block_type(&b_ordered, "numbered_list", None),
          mica_icons::list_ordered,
        ))
        .child(toolbar_btn_active(
          "ed-todo",
          "待办清单",
          dark_mode,
          computed(move || matches!(active_kind.get().as_str(), "todo" | "taskItem")),
          move || toggle_block_type(&b_todo, "todo", None),
          mica_icons::todo_box,
        ))
        .child(toolbar_btn(
          "ed-outdent",
          "减少缩进 (Shift+Tab)",
          dark_mode,
          signal(false),
          move || {
            b_outdent.dispatch(EditorCommand::Indent { delta: -1 });
            b_outdent.request_focus();
          },
          mica_icons::indent_decrease,
        ))
        .child(toolbar_btn(
          "ed-indent",
          "增加缩进 (Tab)",
          dark_mode,
          signal(false),
          move || {
            b_indent.dispatch(EditorCommand::Indent { delta: 1 });
            b_indent.request_focus();
          },
          mica_icons::indent_increase,
        ))
        .child(toolbar_btn_active(
          "ed-quote",
          "引用块",
          dark_mode,
          computed(move || matches!(active_kind.get().as_str(), "quote" | "blockquote")),
          move || toggle_block_type(&b_quote, "quote", None),
          mica_icons::quote_icon,
        ))
        .child(toolbar_btn_active(
          "ed-code-block",
          "代码块",
          dark_mode,
          computed(move || active_kind.get() == "code_block"),
          move || b_code_blk.insert_custom_block("code_block", "", None),
          terminal_icon,
        ))
        .child(toolbar_sep(dark_mode))
        .child(toolbar_btn(
          "ed-divider",
          "插入分割线",
          dark_mode,
          signal(false),
          move || b_div.insert_divider(),
          mica_icons::divider_icon,
        ))
        .child(toolbar_btn(
          "ed-table-quick",
          "插入表格",
          dark_mode,
          signal(false),
          move || b_tbl.insert_table(2, 2),
          mica_icons::table_icon,
        ))
        .child(toolbar_btn(
          "ed-image",
          "插入图片",
          dark_mode,
          signal(false),
          move || (h_img.on_insert_image)(),
          image_icon,
        ))
        .child(toolbar_btn(
          "ed-slash-open",
          "插入块菜单 (/)",
          dark_mode,
          slash.open,
          move || {
            if slash.open.get_untracked() {
              close_slash(slash);
            } else {
              slash.query.set(String::new());
              slash.index.set(0);
              slash.opened_by_btn.set(true);
              update_slash_anchor(&b_slash_btn, slash);
              slash.open.set(true);
            }
          },
          mica_icons::plus,
        ))
        .into_element()
    })
  };
  let fmt_bar_inline = render_format_bar.clone();
  let fmt_bar_second = render_format_bar;

  let h_copy_id = header.clone();
  let h_right = header.clone();
  let h_board = header.clone();
  let h_share = header.clone();
  let h_copy_link_id = header.clone();
  let h_fav = header.clone();
  let b_copy_path = bridge.clone();
  let b_copy_link = bridge;

  // Responsive page header (single 44px row on wide viewports; drops format bar to a clean 36px 2nd row when narrower so breadcrumb + copy-path never gets squeezed)
  view()
    .w_full()
    .border_b_1()
    .border_color(move || palette(dark_mode.get()).border_subtle)
    .bg(move || palette(dark_mode.get()).surface_base)
    .child(
      view()
        .w_full()
        .h(px(44.0))
        .px_3()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
          // Left breadcrumb path (left-click draggable text selection!) + copy-path button
          view()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .flex_row()
            .items_center()
            .gap_1()
            .child(
              view()
                .test_id("ed-breadcrumb-selectable")
                .selectable(true)
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_1()
                .child(when(move || !header_compact.get() && !right_open.get(), {
                  let hb = header.clone();
                  move || {
                    let hb1 = hb.clone();
                    let hb2 = hb.clone();
                    view()
                      .flex_row()
                      .items_center()
                      .gap_1()
                      .child(
                        text(move || hb1.workspace_name.get())
                          .text_size(move || px((13.5 * crate::mica_theme::ui_scale_factor()).round()))
                          .text_color(move || palette(dark_mode.get()).text_muted)
                          .truncate(),
                      )
                      .child(when(
                        {
                          let hb_chk = hb2.clone();
                          move || !hb_chk.folder_name.get().trim().is_empty()
                        },
                        {
                          let hb_f = hb2.clone();
                          move || {
                            let hb_f2 = hb_f.clone();
                            view()
                              .flex_row()
                              .items_center()
                              .gap_1()
                              .child(
                                text(">")
                                  .text_size(move || px((12.5 * crate::mica_theme::ui_scale_factor()).round()))
                                  .text_color(move || palette(dark_mode.get()).text_ghost),
                              )
                              .child(
                                text(move || hb_f2.folder_name.get())
                                  .text_size(move || px((13.5 * crate::mica_theme::ui_scale_factor()).round()))
                                  .text_color(move || palette(dark_mode.get()).text_muted)
                                  .truncate(),
                              )
                          }
                        },
                      ))
                      .child(
                        text(">")
                          .text_size(move || px((12.5 * crate::mica_theme::ui_scale_factor()).round()))
                          .text_color(move || palette(dark_mode.get()).text_ghost),
                      )
                  }
                }))
                .child({
                  let hb = header.clone();
                  text(move || {
                    let t = hb.page_title.get();
                    if t.trim().is_empty() {
                      crate::mica_theme::tr("新页面", "New page").to_string()
                    } else {
                      t
                    }
                  })
                  .text_size(move || px((13.5 * crate::mica_theme::ui_scale_factor()).round()))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).text_primary)
                  .truncate()
                }),
            )
            .child(when(move || !header_compact.get(), {
              let h_copy_id = h_copy_id.clone();
              let b_copy_path = b_copy_path.clone();
              move || {
                let h_copy_id = h_copy_id.clone();
                let b_copy_path = b_copy_path.clone();
                crate::tooltip::with_tooltip(
                  button()
                    .test_id("ed-copy-path")
                    .accessibility_label("复制页面路径")
                    .w(px(28.0))
                    .h(px(28.0))
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      crate::tooltip::hide_tooltip();
                      // Match Flutter `_pagePath()`: copy human-readable containing path
                      // (`Workspace/Folder/...`, or full path segments joined with `/`),
                      // NOT `mica://page/<uuid>` (which belongs to the right-side copy-link button).
                      let ws = h_copy_id.workspace_name.get_untracked();
                      let folder = h_copy_id.folder_name.get_untracked();
                      let mut segs: Vec<String> = Vec::new();
                      if !ws.trim().is_empty() {
                        segs.push(ws.trim().to_string());
                      }
                      for part in folder.split(" > ") {
                        let p = part.trim();
                        if !p.is_empty() {
                          segs.push(p.to_string());
                        }
                      }
                      let path = segs.join("/");
                      if !path.is_empty() {
                        set_clipboard(path);
                        copied_path.set(true);
                        let next_gen = copied_path_gen.get_untracked().wrapping_add(1);
                        copied_path_gen.set(next_gen);
                        spawn(async move {
                          timer(std::time::Duration::from_millis(1600)).await;
                          if copied_path_gen.get_untracked() == next_gen {
                            copied_path.set(false);
                          }
                        });
                        b_copy_path.set_status_flash("已复制页面路径");
                      }
                    })
                    .child(dynamic(move || {
                      let p = palette(dark_mode.get());
                      if copied_path.get() {
                        mica_icons::check_icon("#16A34A")
                      } else {
                        mica_icons::copy_icon(p.text_muted_hex)
                      }
                    })),
                  "复制路径",
                )
              }
            })),
        )
        .child(when(move || show_format_bar.get() && header_single_row.get(), {
          let fb = fmt_bar_inline.clone();
          move || fb()
        }))
        .child(
          // Right cluster: fixed status icon + right-sidebar toggle + whiteboard + share (cloud) + copy link + favorite + more
          view()
            .flex_shrink_0()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .child(when(move || !header_compact.get(), move || {
              crate::tooltip::with_dynamic_tooltip(
                view()
                  .test_id("ed-sync-status")
                  .w(px(28.0))
                  .h(px(28.0))
                  .flex_shrink_0()
                  .items_center()
                  .justify_center()
                  .rounded(px(5.0))
                  .child(dynamic(move || {
                    let p = palette(dark_mode.get());
                    let badge = sync_badge.get();
                    if !badge.trim().is_empty() {
                      match sync_phase.get() {
                        mica_sync::SyncPhase::Syncing => mica_icons::refresh_icon(p.accent_hex),
                        mica_sync::SyncPhase::Offline => mica_icons::cloud_off(p.warning_hex),
                        mica_sync::SyncPhase::Synced => mica_icons::cloud_check(p.text_muted_hex),
                      }
                    } else {
                      mica_icons::check_icon(p.text_faint_hex)
                    }
                  })),
                move || format_sync_status_tooltip(&sync_badge.get(), &last_edited.get()),
              )
            }))
            .child(toolbar_btn(
              "toggle-right-panel-btn",
              "页面大纲与信息",
              dark_mode,
              right_open,
              move || (h_right.on_toggle_right_panel)(),
              mica_icons::sidebar_toggle_right,
            ))
            .child(when(move || !header_compact.get(), {
              let hb = h_board.clone();
              move || {
                let hb = hb.clone();
                toolbar_btn(
                  "ed-open-board",
                  "无限白板",
                  dark_mode,
                  signal(false),
                  move || (hb.on_open_board)(),
                  mica_icons::whiteboard,
                )
              }
            }))
            .child(when(move || is_cloud.get(), {
              let hs = h_share.clone();
              move || {
                let hs = hs.clone();
                crate::tooltip::with_tooltip(
                  button()
                    .test_id("ed-share-btn")
                    .accessibility_label("共享页面")
                    .h(px(26.0))
                    .px_2()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .rounded(px(5.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      crate::tooltip::hide_tooltip();
                      (hs.on_share)();
                    })
                    .child(dynamic(move || {
                      lock_icon(palette(dark_mode.get()).text_muted_hex)
                    }))
                    .child(when(move || !header_compact.get(), move || {
                      text("共享")
                        .text_size(px(12.0))
                        .text_color(move || palette(dark_mode.get()).text_primary)
                    })),
                  "共享页面",
                )
              }
            }))
            .child(when(move || !header_compact.get(), {
              let hl = h_copy_link_id.clone();
              let bl = b_copy_link.clone();
              move || {
                let hl = hl.clone();
                let bl = bl.clone();
                crate::tooltip::with_tooltip(
                  button()
                    .test_id("ed-copy-link")
                    .accessibility_label("复制链接")
                    .w(px(28.0))
                    .h(px(28.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      crate::tooltip::hide_tooltip();
                      let id = hl.page_id_str.get_untracked();
                      set_clipboard(format!("mica://page/{id}"));
                      copied_link.set(true);
                      bl.set_status_flash("已复制链接");
                    })
                    .child(dynamic(move || {
                      let p = palette(dark_mode.get());
                      if copied_link.get() {
                        mica_icons::check_icon(p.success_hex)
                      } else {
                        mica_icons::link_icon(p.text_muted_hex)
                      }
                    })),
                  "复制页面链接",
                )
              }
            }))
            .child(crate::tooltip::with_dynamic_tooltip(
              button()
                .test_id("ed-star")
                .accessibility_label("收藏或取消收藏当前页面")
                .w(px(28.0))
                .h(px(28.0))
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  crate::tooltip::hide_tooltip();
                  (h_fav.on_toggle_favorite)();
                })
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  if is_fav.get() {
                    mica_icons::star_filled(p.favorite_hex)
                  } else {
                    mica_icons::star_icon(p.text_muted_hex)
                  }
                })),
              move || if is_fav.get_untracked() { "取消收藏" } else { "收藏" },
            ))
            .child(toolbar_btn_with_ref(
              "ed-more",
              "更多操作",
              &page_more_btn_ref,
              dark_mode,
              page_menu_open,
              move || page_menu_open.set(!page_menu_open.get_untracked()),
              mica_icons::more_dots,
            )),
        ),
    )
    .child(when(move || show_format_bar.get() && !header_single_row.get(), {
      let fb = fmt_bar_second.clone();
      move || {
        let fb = fb.clone();
        dynamic(move || {
          let scrollable = format_bar_scroll.get();
          let row = view()
            .w_full()
            .h(px(36.0))
            .px_3()
            .flex_row()
            .items_center()
            .border_t_1()
            .border_color(move || palette(dark_mode.get()).border_subtle);
          if scrollable {
            row.child(
              scroll()
                .test_id("ed-format-bar-scroll")
                .horizontal()
                .scrollbar(false)
                .flex_1()
                .min_w(px(0.0))
                .h_full()
                .child(view().h_full().flex_row().items_center().child(fb())),
            )
          } else {
            row.justify_center().child(fb())
          }
        })
      }
    }))
}

#[cfg(test)]
mod tests {
  use super::format_sync_status_tooltip;

  #[test]
  fn sync_status_tooltip_formats_cloud_and_local_states() {
    assert_eq!(
      format_sync_status_tooltip("已实时同步", "上次编辑 刚刚"),
      "已实时同步 · 上次编辑 刚刚"
    );
    assert_eq!(
      format_sync_status_tooltip("正在同步… · 2 人在线", "上次编辑 刚刚"),
      "正在同步… · 2 人在线 · 上次编辑 刚刚"
    );
    assert_eq!(format_sync_status_tooltip("离线缓存", ""), "离线缓存");
    assert_eq!(
      format_sync_status_tooltip("", "上次编辑 刚刚"),
      "本地已保存 · 上次编辑 刚刚"
    );
    assert_eq!(format_sync_status_tooltip("", "  "), "本地已保存");
  }
}
