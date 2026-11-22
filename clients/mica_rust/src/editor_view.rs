//! Pure-Rust document editor view built on `nsh_base::rich_editor::view::RichEditor`,
//! `MicaDocumentBridge`, and Mica's custom `NodeView`s (`node_views.rs`):
//! - Strict single-row 40px page header: breadcrumb + copy-link (left) · toggleable
//!   single-row formatting toolbar with live active states (center) · last-edited,
//!   right-sidebar toggle, whiteboard, share (cloud), copy-link (green check flash),
//!   favorite star, trash, and full `⋯` page menu (right).
//! - Centered resizable page column (`page_width`, default 740px) with borderless
//!   30px SemiBold page title, `PropertyPanel` slot, single-mount native `RichEditor`,
//!   `BacklinksPanel` slot below the editor, caret-anchored 22-item `/` slash menu,
//!   expanded floating selection bubble menu (marks + inline link editor + block
//!   transforms + AI rewrite), floating `Ctrl+F` in-page find bar, and floating
//!   bottom-right word count badge.

use std::{collections::BTreeMap, rc::Rc};

use nsh_base::{
  button, input,
  rich_editor::{
    state::KeyEvent as PmKeyEvent,
    view::{
      PartStyles, RichEditorChange, RichEditorSelection, RichEditorStyles, rich_editor,
    },
  },
  text_view::PartStyle,
};
use nsh_core::{
  Color, LineHeight, PropertyValue, StyleDecl, StyleKey, StyleValue, SubmitBehavior,
  element::{ImageSrc, KeyOptions, Style, Svg, on_key_with, set_clipboard, svg},
  geometry::{Bounds, Points, px},
  keymap::When,
  prelude::*,
};

use crate::{
  editor_slash::{SlashItem, extract_slash_query, filter_slash_items},
  mica_icons,
  mica_rich_bridge::MicaDocumentBridge,
  mica_theme::palette,
  node_views::{OpenBoardCallback, register_mica_node_views},
};

pub use crate::mica_rich_bridge::OutlineItem;

const SLASH_MENU_WIDTH: f32 = 248.0;
const SLASH_MENU_MAX_H: f32 = 288.0;

pub struct EditorHeaderProps {
  pub workspace_name: Signal<String>,
  pub folder_name: Signal<String>,
  pub page_title: Signal<String>,
  pub page_id_str: Signal<String>,
  pub is_favorite: Signal<bool>,
  pub right_panel_open: Signal<bool>,
  pub on_rename_title: Rc<dyn Fn(String)>,
  pub on_open_board: Rc<dyn Fn()>,
  pub on_toggle_favorite: Rc<dyn Fn()>,
  pub on_toggle_right_panel: Rc<dyn Fn()>,
  pub on_trash_page: Rc<dyn Fn()>,
  pub on_export_markdown: Rc<dyn Fn()>,
  pub on_export_html: Rc<dyn Fn()>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  // ── 1:1 Parity Props ─────────────────────────────────────────────────────
  pub is_cloud_signed_in: Signal<bool>,
  pub show_format_bar: Signal<bool>,
  pub show_page_title: Signal<bool>,
  pub show_properties: Signal<bool>,
  pub comments_visible: Signal<bool>,
  pub page_width: Signal<f32>,
  pub ai_enabled: Signal<bool>,
  pub on_commit_page_width: Rc<dyn Fn(f32)>,
  pub on_share: Rc<dyn Fn()>,
  pub on_toggle_comments: Rc<dyn Fn()>,
  pub on_toggle_properties: Rc<dyn Fn()>,
  pub on_export_zip: Rc<dyn Fn()>,
  pub on_export_pdf: Rc<dyn Fn()>,
  pub on_import_markdown_picker: Rc<dyn Fn()>,
  pub on_open_version_history: Rc<dyn Fn()>,
  pub on_inline_ai: Rc<dyn Fn(String)>,
  pub on_insert_image: Rc<dyn Fn()>,
  pub property_panel_slot: Option<Rc<dyn Fn() -> AnyElement>>,
  pub backlinks_panel_slot: Option<Rc<dyn Fn() -> AnyElement>>,
}

#[derive(Clone, Copy)]
struct SlashMenuSignals {
  open: Signal<bool>,
  query: Signal<String>,
  index: Signal<usize>,
  anchor: Signal<(f32, f32)>,
  opened_by_btn: Signal<bool>,
  box_size: Signal<(f32, f32)>,
}

pub fn editor_view(
  bridge: Rc<MicaDocumentBridge>,
  header: Rc<EditorHeaderProps>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  // Allow embedded `board` NodeViews inside `RichEditor` to open the full whiteboard.
  provide_context(OpenBoardCallback(header.on_open_board.clone()));

  let page_menu_open = signal(false);
  let has_text_selection = signal(false);
  let copied_path = signal(false);
  let copied_path_gen = signal(0u64);
  let copied_link = signal(false);

  // Caret-anchored slash menu state
  let slash = SlashMenuSignals {
    open: signal(false),
    query: signal(String::new()),
    index: signal(0usize),
    anchor: signal((12.0f32, 8.0f32)),
    opened_by_btn: signal(false),
    box_size: signal((740.0f32, 620.0f32)),
  };

  // Inline link prompt state (used by both format bar & bubble menu)
  let link_prompt_open = signal(false);
  let link_draft = signal(String::new());

  // In-page find bar (`Ctrl+F`) state
  let find_open = signal(false);
  let find_query = signal(String::new());
  let find_matches: Signal<Vec<(usize, usize)>> = signal(Vec::new());
  let find_index = signal(0usize);

  let ai_enabled = header.ai_enabled;
  let on_inline_ai_key = header.on_inline_ai.clone();
  let on_insert_image_key = header.on_insert_image.clone();

  // Wire editor-local key interception so ArrowUp / ArrowDown / Enter / Escape
  // drive the caret-anchored slash menu without stealing editor focus or
  // interfering with normal typing when the slash menu is closed.
  {
    let b_key = bridge.clone();
    bridge.set_key_interceptor(Some(Rc::new(move |ev: &PmKeyEvent| {
      if !slash.open.get_untracked() {
        return false;
      }
      let items = filter_slash_items(&slash.query.get_untracked(), ai_enabled.get_untracked());
      if items.is_empty() {
        close_slash(slash);
        return false;
      }
      match ev.key.as_str() {
        "ArrowDown" => {
          let next = (slash.index.get_untracked() + 1) % items.len();
          slash.index.set(next);
          true
        }
        "ArrowUp" => {
          let cur = slash.index.get_untracked().min(items.len() - 1);
          let next = (cur + items.len() - 1) % items.len();
          slash.index.set(next);
          true
        }
        "Enter" if !ev.ctrl && !ev.meta && !ev.alt => {
          let idx = slash.index.get_untracked().min(items.len() - 1);
          let item = *items[idx];
          close_slash(slash);
          execute_slash_item(&b_key, item, &on_inline_ai_key, &on_insert_image_key);
          true
        }
        "Escape" => {
          close_slash(slash);
          true
        }
        _ => false,
      }
    })));
  }

  // `Ctrl+F` opens in-page find bar.
  on_key_with(
    "mod+f",
    KeyOptions {
      when: When::Always,
      ..KeyOptions::default()
    },
    move |_| {
      page_menu_open.set(false);
      close_slash(slash);
      find_open.set(true);
    },
  );

  // `Shift+Enter` steps backwards when the in-page find bar is open.
  {
    let b_find_prev_key = bridge.clone();
    on_key_with(
      "shift+enter",
      KeyOptions {
        when: When::Always,
        pass_through: true,
        ..KeyOptions::default()
      },
      move |_| {
        if find_open.get_untracked() {
          step_find_match(&b_find_prev_key, find_matches, find_index, -1);
        }
      },
    );
  }

  let marquee = crate::block_marquee::BlockMarqueeState::new();
  let marquee_origin: Signal<Option<(f32, f32)>> = signal(None);
  let paper_bounds: Signal<(f32, f32)> = signal((960.0, 720.0));
  let rich_origin: Signal<(f32, f32)> = signal((0.0, 0.0));

  let m_esc = marquee.clone();
  on_key_with(
    "escape",
    KeyOptions {
      when: When::Always,
      pass_through: true,
      ..KeyOptions::default()
    },
    move |_| {
      close_slash(slash);
      page_menu_open.set(false);
      link_prompt_open.set(false);
      find_open.set(false);
      has_text_selection.set(false);
      m_esc.clear();
      marquee_origin.set(None);
    },
  );

  // Marquee multi-block keyboard actions: Delete / Backspace / Mod+C / Mod+X
  for key in ["delete", "backspace"] {
    let b_del = bridge.clone();
    let m_del = marquee.clone();
    on_key_with(
      key,
      KeyOptions {
        when: When::Always,
        pass_through: true,
        ..KeyOptions::default()
      },
      move |_| {
        if let Some((lo, hi)) = m_del.selected_blocks() {
          b_del.delete_block_range(lo, hi);
          m_del.clear();
        }
      },
    );
  }
  {
    let b_copy = bridge.clone();
    let m_copy = marquee.clone();
    on_key_with(
      "mod+c",
      KeyOptions {
        when: When::Always,
        pass_through: true,
        ..KeyOptions::default()
      },
      move |_| {
        if let Some((lo, hi)) = m_copy.selected_blocks() {
          let md = b_copy.copy_block_range_markdown(lo, hi);
          if !md.is_empty() {
            set_clipboard(md);
          }
        }
      },
    );
  }
  {
    let b_cut = bridge.clone();
    let m_cut = marquee.clone();
    on_key_with(
      "mod+x",
      KeyOptions {
        when: When::Always,
        pass_through: true,
        ..KeyOptions::default()
      },
      move |_| {
        if let Some((lo, hi)) = m_cut.selected_blocks() {
          let md = b_cut.copy_block_range_markdown(lo, hi);
          if !md.is_empty() {
            set_clipboard(md);
          }
          b_cut.delete_block_range(lo, hi);
          m_cut.clear();
        }
      },
    );
  }

  let active_kind = bridge.active_kind_signal();
  let active_level = bridge.active_level_signal();
  let active_marks = bridge.active_marks_signal();
  let can_undo = bridge.can_undo_signal();
  let can_redo = bridge.can_redo_signal();
  let last_edited = bridge.last_edited_signal();
  let word_count = bridge.word_count_signal();

  let rich_session = bridge.session();
  if rich_session.node_views().is_empty() {
    register_mica_node_views(&rich_session, dark_mode);
  }

  let is_fav = header.is_favorite;
  let right_open = header.right_panel_open;
  let show_format_bar = header.show_format_bar;
  let show_page_title = header.show_page_title;
  let show_properties = header.show_properties;
  let is_cloud = header.is_cloud_signed_in;
  let page_width = header.page_width;

  let b_copy_path = bridge.clone();
  let b_copy_link = bridge.clone();
  let b_undo = bridge.clone();
  let b_redo = bridge.clone();
  let b_para = bridge.clone();
  let b_h1 = bridge.clone();
  let b_h2 = bridge.clone();
  let b_h3 = bridge.clone();
  let b_bold = bridge.clone();
  let b_italic = bridge.clone();
  let b_strike = bridge.clone();
  let b_code = bridge.clone();
  let b_link_btn = bridge.clone();
  let b_list = bridge.clone();
  let b_ordered = bridge.clone();
  let b_todo = bridge.clone();
  let b_quote = bridge.clone();
  let b_code_blk = bridge.clone();
  let b_div = bridge.clone();
  let b_tbl = bridge.clone();
  let b_slash_btn = bridge.clone();
  let b_slash_menu = bridge.clone();
  let b_copy_md = bridge.clone();
  let b_title_enter = bridge.clone();
  let b_rich = bridge.clone();
  let b_link_pop = bridge.clone();
  let b_find = bridge.clone();

  let h_title_val = header.clone();
  let h_title_chg = header.clone();
  let h_board = header.clone();
  let h_share = header.clone();
  let h_trash = header.clone();
  let h_fav = header.clone();
  let h_right = header.clone();
  let h_copy_id = header.clone();
  let h_copy_link_id = header.clone();
  let h_img_bar = header.clone();
  let h_menu = header.clone();
  let h_slash = header.clone();
  let h_bubble = header.clone();
  let prop_slot = header.property_panel_slot.clone();
  let backlinks_slot = header.backlinks_panel_slot.clone();
  let header_width = signal(1000.0_f32);
  let header_single_row = computed(move || header_width.get() >= 1180.0);

  let render_format_bar: Rc<dyn Fn() -> AnyElement> = {
    let b_undo = b_undo.clone();
    let b_redo = b_redo.clone();
    let b_para = b_para.clone();
    let b_h1 = b_h1.clone();
    let b_h2 = b_h2.clone();
    let b_h3 = b_h3.clone();
    let b_bold = b_bold.clone();
    let b_italic = b_italic.clone();
    let b_strike = b_strike.clone();
    let b_code = b_code.clone();
    let b_link_btn = b_link_btn.clone();
    let b_list = b_list.clone();
    let b_ordered = b_ordered.clone();
    let b_todo = b_todo.clone();
    let b_quote = b_quote.clone();
    let b_code_blk = b_code_blk.clone();
    let b_div = b_div.clone();
    let b_tbl = b_tbl.clone();
    let b_slash_btn = b_slash_btn.clone();
    let h_img = h_img_bar.clone();
    Rc::new(move || {
      let b_undo = b_undo.clone();
      let b_redo = b_redo.clone();
      let b_para = b_para.clone();
      let b_h1 = b_h1.clone();
      let b_h2 = b_h2.clone();
      let b_h3 = b_h3.clone();
      let b_bold = b_bold.clone();
      let b_italic = b_italic.clone();
      let b_strike = b_strike.clone();
      let b_code = b_code.clone();
      let b_link_btn = b_link_btn.clone();
      let b_list = b_list.clone();
      let b_ordered = b_ordered.clone();
      let b_todo = b_todo.clone();
      let b_quote = b_quote.clone();
      let b_code_blk = b_code_blk.clone();
      let b_div = b_div.clone();
      let b_tbl = b_tbl.clone();
      let b_slash_btn = b_slash_btn.clone();
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
          "删除线",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "strike")),
          move || b_strike.toggle_mark("strike"),
          mica_icons::strikethrough_icon,
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
          move || b_code_blk.insert_custom_block("code_block", "", Some("rust")),
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

  view()
    .flex_1()
    .min_w(px(0.0))
    .min_h(px(0.0))
    .h_full()
    .overflow_hidden()
    .relative()
    .bg(move || palette(dark_mode.get()).surface_base)
    .on_layout(move |b: &Bounds<Points>| {
      let w = b.size.width.0;
      if w > 0.0 && (header_width.get_untracked() - w).abs() > 2.0 {
        spawn(async move {
          if (header_width.get_untracked() - w).abs() > 2.0 {
            header_width.set(w);
          }
        });
      }
    })
    .child(
      // 1. Responsive page header (single 44px row on wide viewports; drops format bar to a clean 36px 2nd row when narrower so breadcrumb + copy-path never gets squeezed)
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
                .child(when(move || !right_open.get(), {
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
                      crate::mica_theme::tr("无标题", "Untitled").to_string()
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
            .child(
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
            ),
        )
        .child(when(move || show_format_bar.get() && header_single_row.get(), {
          let fb = fmt_bar_inline.clone();
          move || fb()
        }))
        .child(
          // Right cluster: last-edited + right-sidebar toggle + whiteboard + share (cloud) + copy link + favorite + trash + more
          view()
            .flex_shrink_0()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .child(when(move || !right_open.get(), move || {
              text(move || last_edited.get())
                .text_size(move || px((13.0 * crate::mica_theme::ui_scale_factor()).round()))
                .text_color(move || palette(dark_mode.get()).text_faint)
            }))
            .child(toolbar_btn(
              "toggle-right-panel-btn",
              "切换右侧大纲与页面信息边栏",
              dark_mode,
              right_open,
              move || (h_right.on_toggle_right_panel)(),
              mica_icons::sidebar_toggle_right,
            ))
            .child(toolbar_btn(
              "ed-open-board",
              "打开无限白板",
              dark_mode,
              signal(false),
              move || (h_board.on_open_board)(),
              mica_icons::whiteboard,
            ))
            .child(when(move || is_cloud.get(), {
              let hs = h_share.clone();
              move || {
                let hs = hs.clone();
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
                  .on_press(move || (hs.on_share)())
                  .child(dynamic(move || {
                    lock_icon(palette(dark_mode.get()).text_muted_hex)
                  }))
                  .child(
                    text("共享")
                      .text_size(px(12.0))
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  )
              }
            }))
            .child(
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
                  let id = h_copy_link_id.page_id_str.get_untracked();
                  set_clipboard(format!("mica://page/{id}"));
                  copied_link.set(true);
                  b_copy_link.set_status_flash("已复制链接");
                })
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  if copied_link.get() {
                    mica_icons::check_icon(p.success_hex)
                  } else {
                    mica_icons::link_icon(p.text_muted_hex)
                  }
                })),
            )
            .child(
              button()
                .test_id("ed-star")
                .accessibility_label("收藏或取消收藏当前页面")
                .w(px(28.0))
                .h(px(28.0))
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || (h_fav.on_toggle_favorite)())
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  if is_fav.get() {
                    mica_icons::star_filled(p.favorite_hex)
                  } else {
                    mica_icons::star_icon(p.text_muted_hex)
                  }
                })),
            )
            .child(toolbar_btn(
              "trash-page-btn",
              "移入回收站",
              dark_mode,
              signal(false),
              move || (h_trash.on_trash_page)(),
              mica_icons::trash_icon,
            ))
            .child(toolbar_btn(
              "ed-more",
              "页面菜单",
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
            view()
              .w_full()
              .h(px(36.0))
              .px_3()
              .flex_row()
              .items_center()
              .justify_center()
              .border_t_1()
              .border_color(move || palette(dark_mode.get()).border_subtle)
              .child(fb())
          }
        })),
    )
    .child(when(move || page_menu_open.get(), move || {
      page_more_menu_view(h_menu.clone(), b_copy_md.clone(), page_menu_open, dark_mode)
    }))
    .child(when(move || link_prompt_open.get(), move || {
      link_editor_popover(b_link_pop.clone(), link_draft, link_prompt_open, dark_mode)
    }))
    .child(when(move || find_open.get(), move || {
      find_bar_view(b_find.clone(), find_open, find_query, find_matches, find_index, dark_mode)
    }))
    .child({
      // 2. Centered resizable paper canvas (`PageWidthFrame`) + BlockMarquee + floating word count
      let m_down = marquee.clone();
      let m_move = marquee.clone();
      let m_up = marquee.clone();
      let m_overlay = marquee.clone();
      let b_marq = bridge.clone();
      let on_commit_width = header.on_commit_page_width.clone();
      view()
        .flex_1()
        .w_full()
        .min_h(px(0.0))
        .overflow_hidden()
        .relative()
        .bg(move || palette(dark_mode.get()).surface_base)
        .on_layout(move |b: &Bounds<Points>| {
          paper_bounds.set((b.size.width.0, b.size.height.0));
        })
        .on(EventKind::PressDown, move |p: &EventPayload| {
          if let EventPayload::Press(e) = p {
            let (cw, _) = paper_bounds.get_untracked();
            let cap = crate::page_width::rendered_column_width(page_width.get_untracked(), cw);
            let content_left = ((cw - cap) * 0.5).max(0.0);
            let content_right = (content_left + cap).min(cw);
            let (_, ry) = rich_origin.get_untracked();
            let content_bottom = ry + slash.box_size.get_untracked().1;
            if crate::block_marquee::starts_marquee_in_margin(
              e.position.x.0,
              e.position.y.0,
              cw,
              content_left,
              content_right,
              content_bottom,
            ) {
              marquee_origin.set(Some((e.position.x.0, e.position.y.0)));
              m_down.clear();
            } else {
              marquee_origin.set(None);
              if m_down.is_active_untracked() {
                m_down.clear();
              }
            }
          }
        })
        .on_mouse_move(move |e: &MouseEvent| {
          if e.pressed_button != Some(MouseButton::Left) {
            return;
          }
          let Some((ox, oy)) = marquee_origin.get_untracked() else {
            return;
          };
          let (cx, cy) = (e.position.x.0, e.position.y.0);
          if (cx - ox).hypot(cy - oy) < 4.0 {
            return;
          }
          let (rx, ry) = rich_origin.get_untracked();
          let block_rects: Vec<(usize, crate::block_marquee::RectF)> = b_marq
            .top_level_block_rects()
            .into_iter()
            .map(|(idx, b)| {
              (
                idx,
                crate::block_marquee::RectF::new(
                  rx + b.origin.x.0,
                  ry + b.origin.y.0,
                  rx + b.origin.x.0 + b.size.width.0,
                  ry + b.origin.y.0 + b.size.height.0,
                ),
              )
            })
            .collect();
          m_move.update_drag(
            crate::block_marquee::RectF::from_points(ox, oy, cx, cy),
            &block_rects,
          );
        })
        .on_mouse_up(move |_: &MouseEvent| {
          if marquee_origin.get_untracked().is_some() {
            marquee_origin.set(None);
            m_up.end_drag();
          }
        })
        .child(crate::page_width::page_width_frame(
          page_width,
          dark_mode,
          on_commit_width,
          view()
            .flex_1()
            .w_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .px_6()
            .pt_8()
            .pb_6()
            .child(when(move || show_page_title.get(), move || {
              let h_val = h_title_val.clone();
              let h_chg = h_title_chg.clone();
              let b_enter = b_title_enter.clone();
              // Borderless 30px SemiBold editable document title
              input()
                .test_id("page-title-input")
                .placeholder("无标题")
                .submit_behavior(SubmitBehavior::Submit)
                .w_full()
                .pb_4()
                .text_size(px(30.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || palette(dark_mode.get()).text_primary)
                .bg(move || palette(dark_mode.get()).surface_base)
                .value(move || h_val.page_title.get())
                .on_change(move |v: &str| (h_chg.on_rename_title)(v.to_owned()))
                .on_submit(move |_: &str| {
                  b_enter.focus_pos(1);
                  b_enter.session().focus();
                })
            }))
            .child(when(move || show_properties.get(), {
              let slot = prop_slot.clone();
              move || match &slot {
                Some(render_fn) => render_fn(),
                None => view().into_element(),
              }
            }))
            .child(
              // Relative container hosting single-mount RichEditor + caret-anchored slash menu
              view()
                .flex_1()
                .w_full()
                .min_h(px(0.0))
                .overflow_hidden()
                .relative()
                .on_layout(move |bounds: &Bounds<Points>| {
                  rich_origin.set((bounds.origin.x.0, bounds.origin.y.0));
                  slash.box_size.set((bounds.size.width.0, bounds.size.height.0));
                })
                .child({
                  // Mount native `RichEditor` once against the stable `EditorSession`
                  // so keystrokes, selections, and embedded NodeViews never lose focus.
                  let b_chg = b_rich.clone();
                  let b_bubble = b_rich.clone();
                  let h_bub = h_bubble.clone();
                  rich_editor()
                    .test_id("mica-rich-editor")
                    .accessibility_label("文档正文编辑器")
                    .flex_1()
                    .w_full()
                    .min_h(px(0.0))
                    .h_full()
                    .editor(Some(rich_session))
                    .placeholder("输入正文，或输入 / 唤出命令...")
                    .styles(move || {
                      let (font_px, font_family) = use_context::<crate::mica_theme::UiAppearance>()
                        .map(|a| (a.editor_font_px.get() as f32, a.editor_font_family.get()))
                        .unwrap_or((16.0, ""));
                      mica_rich_editor_styles(dark_mode.get(), font_px, font_family)
                    })
                    .bubble_menu_placement(Placement::Top)
                    .bubble_menu_offset(px(8.0))
                    .bubble_menu(bubble_menu_view(
                      b_bubble,
                      h_bub,
                      has_text_selection,
                      link_prompt_open,
                      link_draft,
                      dark_mode,
                    ))
                    .on_selection_change(move |sel: &RichEditorSelection| {
                      has_text_selection.set(!sel.empty && sel.kind == "text");
                      if copied_link.get_untracked() {
                        copied_link.set(false);
                      }
                      if !sel.empty && slash.open.get_untracked() {
                        close_slash(slash);
                      }
                    })
                    .on_change(move |_: &RichEditorChange| {
                      if copied_link.get_untracked() {
                        copied_link.set(false);
                      }
                      sync_slash_palette_from_caret(&b_chg, slash, ai_enabled);
                    })
                })
                .child(when(move || slash.open.get(), move || {
                  caret_slash_menu_view(
                    b_slash_menu.clone(),
                    h_slash.clone(),
                    slash,
                    dark_mode,
                  )
                })),
            )
            .child({
              // BacklinksPanel slot rendered below RichEditor (slot hides itself when empty)
              match backlinks_slot {
                Some(render_fn) => render_fn(),
                None => view().into_element(),
              }
            }),
        ))
        .child(crate::block_marquee::marquee_overlay_layer(
          m_overlay,
          dark_mode,
        ))
        .child(when(
          move || !word_count.get().starts_with("0 词 · 0 字"),
          move || {
            // 3. Floating bottom-right word count badge (replaces dedicated bottom bar)
            view()
              .test_id("editor-word-count")
              .absolute()
              .right(px(12.0))
              .bottom(px(8.0))
              .z_index(12)
              .px_2()
              .py_1()
              .rounded(px(6.0))
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_subtle)
              .bg(move || palette(dark_mode.get()).surface_raised)
              .child(
                text(move || word_count.get())
                  .text_size(px(12.0))
                  .text_color(move || palette(dark_mode.get()).text_faint),
              )
          },
        ))
    })
}

fn close_slash(slash: SlashMenuSignals) {
  slash.open.set(false);
  slash.query.set(String::new());
  slash.index.set(0);
  slash.opened_by_btn.set(false);
}

fn update_slash_anchor(bridge: &MicaDocumentBridge, slash: SlashMenuSignals) {
  let (box_w, box_h) = slash.box_size.get_untracked();
  let max_w = box_w.max(SLASH_MENU_WIDTH + 16.0);
  let max_h = box_h.max(SLASH_MENU_MAX_H + 16.0);
  if let Some(rect) = bridge.caret_coords() {
    let left = rect.origin.x.0.clamp(8.0, (max_w - SLASH_MENU_WIDTH - 8.0).max(8.0));
    let below = rect.origin.y.0 + rect.size.height.0 + 6.0;
    let top = if below + SLASH_MENU_MAX_H > max_h && rect.origin.y.0 > SLASH_MENU_MAX_H + 6.0 {
      (rect.origin.y.0 - SLASH_MENU_MAX_H - 6.0).max(8.0)
    } else {
      below.clamp(8.0, (max_h - SLASH_MENU_MAX_H - 8.0).max(8.0))
    };
    slash.anchor.set((left, top));
  } else {
    slash.anchor.set((12.0, 8.0));
  }
}

fn toggle_block_type(bridge: &MicaDocumentBridge, kind: &str, level: Option<u64>) {
  let cur_kind = bridge.active_kind_signal().get_untracked();
  let cur_level = bridge.active_level_signal().get_untracked();
  let same_kind = match kind {
    "bulleted_list" => matches!(cur_kind.as_str(), "bulleted_list" | "bulletList"),
    "numbered_list" => matches!(cur_kind.as_str(), "numbered_list" | "orderedList"),
    "todo" => matches!(cur_kind.as_str(), "todo" | "taskItem"),
    "quote" => matches!(cur_kind.as_str(), "quote" | "blockquote"),
    other => cur_kind == other,
  };
  let already_same = same_kind && level.is_none_or(|lvl| cur_level == lvl);
  if already_same && kind != "paragraph" {
    bridge.set_block_type("paragraph", None);
  } else {
    bridge.set_block_type(kind, level);
  }
}

fn sync_slash_palette_from_caret(
  bridge: &MicaDocumentBridge,
  slash: SlashMenuSignals,
  ai_enabled: Signal<bool>,
) {
  let state = bridge.session().state();
  let sel = state.selection();
  if !sel.empty() {
    if slash.open.get_untracked() && !slash.opened_by_btn.get_untracked() {
      close_slash(slash);
    }
    return;
  }
  let rfrom = sel.resolved_from();
  let parent = rfrom.parent();
  if parent.is_textblock() && parent.ty().name() != "code_block" {
    let before = parent
      .content()
      .text_between(0, rfrom.parent_offset, None, Some("\u{fffc}"));
    if let Some((_, query)) = extract_slash_query(&before) {
      let items = filter_slash_items(query, ai_enabled.get_untracked());
      if items.is_empty() {
        close_slash(slash);
        return;
      }
      slash.opened_by_btn.set(false);
      slash.query.set(query.to_owned());
      let cur = slash.index.get_untracked();
      if cur >= items.len() {
        slash.index.set(0);
      }
      update_slash_anchor(bridge, slash);
      slash.open.set(true);
      return;
    }
  }
  if slash.open.get_untracked() && !slash.opened_by_btn.get_untracked() {
    close_slash(slash);
  }
}

fn execute_slash_item(
  bridge: &MicaDocumentBridge,
  item: SlashItem,
  on_inline_ai: &Rc<dyn Fn(String)>,
  on_insert_image: &Rc<dyn Fn()>,
) {
  if item.kind == "__ai__" {
    bridge.replace_slash_token_and_apply("__ai__", None, None, None);
    on_inline_ai(String::new());
    return;
  }
  if item.kind == "image" {
    bridge.replace_slash_token_and_apply("__ai__", None, None, None);
    on_insert_image();
    return;
  }
  bridge.replace_slash_token_and_apply(item.kind, item.level, item.alert, item.template);
}

/// Caret-anchored floating 22-item vertical slash menu (exact Flutter `editor.dart` layout).
fn caret_slash_menu_view(
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
fn page_more_menu_view(
  header: Rc<EditorHeaderProps>,
  bridge: Rc<MicaDocumentBridge>,
  page_menu_open: Signal<bool>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let comments_vis = header.comments_visible;
  let props_vis = header.show_properties;
  let on_comments = header.on_toggle_comments.clone();
  let on_props = header.on_toggle_properties.clone();
  let on_zip = header.on_export_zip.clone();
  let on_html = header.on_export_html.clone();
  let on_pdf = header.on_export_pdf.clone();
  let on_import = header.on_import_markdown_picker.clone();
  let on_ver = header.on_open_version_history.clone();
  let on_snap = header.on_save_snapshot.clone();
  let on_trash = header.on_trash_page.clone();

  view()
    .test_id("page-more-menu")
    .absolute()
    .top(px(42.0))
    .right(px(12.0))
    .w(px(220.0))
    .p_1()
    .gap(px(2.0))
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .z_index(35)
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
    }))
}

/// Floating inline link URL editor popover.
fn link_editor_popover(
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
          b_submit.session().focus();
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
          b_apply.session().focus();
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
          b_remove.session().focus();
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

fn step_find_match(
  bridge: &MicaDocumentBridge,
  find_matches: Signal<Vec<(usize, usize)>>,
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
  let (from, to) = hits[next];
  bridge.select_range(from, to);
}

/// Floating top-right in-page find bar (`Ctrl+F`), 1:1 with Flutter `_buildFindBar`.
fn find_bar_view(
  bridge: Rc<MicaDocumentBridge>,
  find_open: Signal<bool>,
  find_query: Signal<String>,
  find_matches: Signal<Vec<(usize, usize)>>,
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
          if let Some(&(from, to)) = hits.first() {
            b_chg.select_range(from, to);
          }
          find_matches.set(hits);
        })
        .on_submit(move |_: &str| {
          step_find_match(&b_submit, find_matches, find_index, 1);
        })
        .on_key_press(move |k: &str| {
          if k == "Escape" {
            find_open.set(false);
            b_close_key.session().focus();
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
          b_close_btn.session().focus();
        })
        .child(dynamic(move || {
          mica_icons::close_sm(palette(dark_mode.get()).text_muted_hex)
        })),
    )
}

/// Expanded floating selection bubble menu (marks + link + block transforms + AI rewrite).
fn bubble_menu_view(
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
    let b_code = bridge.clone();
    let b_link = bridge.clone();
    let b_h1 = bridge.clone();
    let b_h2 = bridge.clone();
    let b_h3 = bridge.clone();
    let b_para = bridge.clone();
    let b_bullet = bridge.clone();
    let b_ordered = bridge.clone();
    let b_todo = bridge.clone();
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
        move || b_code_blk.insert_custom_block("code_block", "", Some("rust")),
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
    .on_press(on_click)
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      icon_fn(if active.get() {
        p.text_primary_hex
      } else {
        p.text_muted_hex
      })
    }))
}

fn decl_style(decls: impl IntoIterator<Item = (StyleKey, PropertyValue)>) -> Style {
  Style::from_decls(decls.into_iter().map(|(k, v)| StyleDecl::Set(k, v)))
}

fn num_prop(key: StyleKey, n: f32) -> (StyleKey, PropertyValue) {
  (key, key.number_value(n).unwrap_or(PropertyValue::Default))
}

fn str_prop(key: StyleKey, s: &str) -> (StyleKey, PropertyValue) {
  (
    key,
    key
      .parse_value(StyleValue::Str(s))
      .unwrap_or(PropertyValue::Default),
  )
}

fn color_prop(key: StyleKey, c: impl Into<Color>) -> (StyleKey, PropertyValue) {
  (key, PropertyValue::Color(Some(c.into())))
}

/// Builds declaration-backed `RichEditorStyles` matching Mica's warm-gray Notion
/// typography scale and surface tokens (using `Style::from_decls` so `parts::merge`
/// preserves every node & mark style).
fn mica_rich_editor_styles(dark_mode: bool, base_font_px: f32, font_family: &str) -> RichEditorStyles {
  let p = palette(dark_mode);
  let base_px = base_font_px.clamp(13.0, 22.0);
  let scale = base_px / 16.0;
  let mut styles = RichEditorStyles::default();

  let mut root_props = vec![
    color_prop(StyleKey::BackgroundColor, p.surface_base),
    color_prop(StyleKey::Color, p.text_primary),
    num_prop(StyleKey::FontSize, base_px),
  ];
  if !font_family.is_empty() {
    root_props.push(str_prop(StyleKey::FontFamily, font_family));
  }
  styles.root = Some(PartStyle(decl_style(root_props)));
  styles.content = Some(PartStyle(decl_style([
    num_prop(StyleKey::PaddingBottom, 160.0),
  ])));
  styles.caret = Some(PartStyle(decl_style([color_prop(
    StyleKey::BackgroundColor,
    p.accent,
  )])));
  styles.selection = Some(PartStyle(decl_style([color_prop(
    StyleKey::BackgroundColor,
    p.selection_bg,
  )])));
  styles.placeholder = Some(PartStyle(decl_style([color_prop(
    StyleKey::Color,
    p.text_ghost,
  )])));
  styles.list_marker = Some(PartStyle(decl_style([
    color_prop(StyleKey::Color, p.text_muted),
    num_prop(StyleKey::FontSize, 15.0),
  ])));
  styles.task_checkbox = Some(PartStyle(decl_style([
    num_prop(StyleKey::Width, 18.0),
    num_prop(StyleKey::Height, 18.0),
    num_prop(StyleKey::MarginRight, 8.0),
    num_prop(StyleKey::MarginTop, 4.0),
  ])));
  styles.selected_node = Some(PartStyle(decl_style([
    num_prop(StyleKey::BorderRadius, 6.0),
    num_prop(StyleKey::OutlineWidth, 2.0),
    str_prop(StyleKey::OutlineStyle, "solid"),
    color_prop(StyleKey::OutlineColor, p.accent),
  ])));
  styles.selected_cell = Some(PartStyle(decl_style([color_prop(
    StyleKey::BackgroundColor,
    p.selection_bg,
  )])));

  let mut nodes = BTreeMap::new();
  nodes.insert(
    "paragraph".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, base_px),
      (
        StyleKey::LineHeight,
        PropertyValue::LineHeight(LineHeight::Multiple(1.65)),
      ),
      num_prop(StyleKey::MarginTop, 3.0),
      num_prop(StyleKey::MarginBottom, 3.0),
      color_prop(StyleKey::Color, p.text_primary),
    ]),
  );
  nodes.insert(
    "heading".to_string(),
    decl_style([
      (StyleKey::FontWeight, PropertyValue::FontWeight(Some(600))),
      (
        StyleKey::LineHeight,
        PropertyValue::LineHeight(LineHeight::Multiple(1.35)),
      ),
      color_prop(StyleKey::Color, p.text_primary),
    ]),
  );
  nodes.insert(
    "heading[level=1]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, (30.0 * scale).round()),
      num_prop(StyleKey::MarginTop, 18.0),
      num_prop(StyleKey::MarginBottom, 6.0),
    ]),
  );
  nodes.insert(
    "heading[level=2]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, (24.0 * scale).round()),
      num_prop(StyleKey::MarginTop, 14.0),
      num_prop(StyleKey::MarginBottom, 4.0),
    ]),
  );
  nodes.insert(
    "heading[level=3]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, (20.0 * scale).round()),
      num_prop(StyleKey::MarginTop, 10.0),
      num_prop(StyleKey::MarginBottom, 4.0),
    ]),
  );
  nodes.insert(
    "heading[level=4]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, (18.0 * scale).round()),
      num_prop(StyleKey::MarginTop, 8.0),
      num_prop(StyleKey::MarginBottom, 2.0),
    ]),
  );
  nodes.insert(
    "heading[level=5]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, (16.0 * scale).round()),
      num_prop(StyleKey::MarginTop, 6.0),
      num_prop(StyleKey::MarginBottom, 2.0),
    ]),
  );
  nodes.insert(
    "heading[level=6]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, (14.0 * scale).round()),
      (StyleKey::FontWeight, PropertyValue::FontWeight(Some(500))),
      num_prop(StyleKey::MarginTop, 6.0),
      num_prop(StyleKey::MarginBottom, 2.0),
      color_prop(StyleKey::Color, p.text_muted),
    ]),
  );

  let quote_style = decl_style([
    num_prop(StyleKey::BorderLeftWidth, 3.0),
    color_prop(StyleKey::BorderLeftColor, p.quote_bar),
    num_prop(StyleKey::PaddingLeft, 14.0),
    num_prop(StyleKey::PaddingTop, 2.0),
    num_prop(StyleKey::PaddingBottom, 2.0),
    num_prop(StyleKey::MarginTop, 6.0),
    num_prop(StyleKey::MarginBottom, 6.0),
    color_prop(StyleKey::Color, p.text_muted),
  ]);
  nodes.insert("blockquote".to_string(), quote_style.clone());
  nodes.insert("quote".to_string(), quote_style);

  // GFM Callout variants (`quote[alert=...]`)
  for (kind, border_col) in [
    ("note", p.alert_note),
    ("tip", p.alert_tip),
    ("important", p.alert_important),
    ("warning", p.alert_warning),
    ("caution", p.alert_caution),
  ] {
    nodes.insert(
      format!("quote[alert={kind}]"),
      decl_style([
        num_prop(StyleKey::BorderLeftWidth, 3.0),
        color_prop(StyleKey::BorderLeftColor, border_col),
        color_prop(StyleKey::BackgroundColor, p.surface_raised),
        num_prop(StyleKey::BorderRadius, 6.0),
        num_prop(StyleKey::PaddingLeft, 14.0),
        num_prop(StyleKey::PaddingRight, 12.0),
        num_prop(StyleKey::PaddingTop, 8.0),
        num_prop(StyleKey::PaddingBottom, 8.0),
        color_prop(StyleKey::Color, p.text_primary),
      ]),
    );
  }

  let list_style = decl_style([
    num_prop(StyleKey::MarginTop, 2.0),
    num_prop(StyleKey::MarginBottom, 2.0),
    num_prop(StyleKey::FontSize, base_px),
    (
      StyleKey::LineHeight,
      PropertyValue::LineHeight(LineHeight::Multiple(1.65)),
    ),
  ]);
  for list_name in ["bulletList", "bulleted_list", "orderedList", "numbered_list"] {
    nodes.insert(list_name.to_string(), list_style.clone());
  }

  let item_style = decl_style([
    num_prop(StyleKey::MarginTop, 2.0),
    num_prop(StyleKey::MarginBottom, 2.0),
  ]);
  for item_name in ["listItem", "list_item", "taskItem", "todo"] {
    nodes.insert(item_name.to_string(), item_style.clone());
  }

  nodes.insert(
    "table".to_string(),
    decl_style([
      num_prop(StyleKey::MarginTop, 8.0),
      num_prop(StyleKey::MarginBottom, 8.0),
      num_prop(StyleKey::BorderWidth, 1.0),
      color_prop(StyleKey::BorderColor, p.border_normal),
      num_prop(StyleKey::BorderRadius, 6.0),
    ]),
  );
  let header_cell_style = decl_style([
    color_prop(StyleKey::BackgroundColor, p.surface_raised),
    (StyleKey::FontWeight, PropertyValue::FontWeight(Some(600))),
    num_prop(StyleKey::PaddingHorizontal, 10.0),
    num_prop(StyleKey::PaddingVertical, 6.0),
    num_prop(StyleKey::BorderWidth, 1.0),
    color_prop(StyleKey::BorderColor, p.border_normal),
  ]);
  nodes.insert("tableHeader".to_string(), header_cell_style.clone());
  nodes.insert("table_header".to_string(), header_cell_style);

  let body_cell_style = decl_style([
    num_prop(StyleKey::PaddingHorizontal, 10.0),
    num_prop(StyleKey::PaddingVertical, 6.0),
    num_prop(StyleKey::BorderWidth, 1.0),
    color_prop(StyleKey::BorderColor, p.border_normal),
  ]);
  nodes.insert("tableCell".to_string(), body_cell_style.clone());
  nodes.insert("table_cell".to_string(), body_cell_style);

  let mut marks = BTreeMap::new();
  let bold_style = decl_style([(StyleKey::FontWeight, PropertyValue::FontWeight(Some(700)))]);
  marks.insert("bold".to_string(), bold_style.clone());
  marks.insert("strong".to_string(), bold_style);

  let italic_style = decl_style([str_prop(StyleKey::FontStyle, "italic")]);
  marks.insert("italic".to_string(), italic_style.clone());
  marks.insert("em".to_string(), italic_style);

  marks.insert(
    "strike".to_string(),
    decl_style([str_prop(StyleKey::TextDecorationLine, "line-through")]),
  );
  marks.insert(
    "underline".to_string(),
    decl_style([str_prop(StyleKey::TextDecorationLine, "underline")]),
  );
  marks.insert(
    "code".to_string(),
    decl_style([
      str_prop(StyleKey::FontFamily, "monospace"),
      num_prop(StyleKey::FontSize, 14.0),
      color_prop(StyleKey::BackgroundColor, p.inline_code_bg),
      color_prop(StyleKey::Color, p.inline_code_ink),
      num_prop(StyleKey::BorderRadius, 4.0),
      num_prop(StyleKey::PaddingHorizontal, 4.0),
    ]),
  );
  marks.insert(
    "link".to_string(),
    decl_style([
      color_prop(StyleKey::Color, p.accent),
      str_prop(StyleKey::TextDecorationLine, "underline"),
    ]),
  );

  styles.nodes = Some(PartStyles(nodes));
  styles.marks = Some(PartStyles(marks));
  styles
}

fn local_stroked(size_px: f32, color: &str, inner: &str) -> Svg {
  let doc = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{inner}</svg>"##
  );
  svg(ImageSrc::svg_markup(&doc)).w(px(size_px)).h(px(size_px))
}

fn terminal_icon(color: &str) -> Svg {
  local_stroked(
    17.0,
    color,
    r##"<rect x="2.8" y="4" width="14.4" height="12" rx="2"/><path d="M6.2 8.2L8.6 10.2L6.2 12.2M10.5 12.2H13.8"/>"##,
  )
}

fn image_icon(color: &str) -> Svg {
  local_stroked(
    17.0,
    color,
    r##"<rect x="3" y="4" width="14" height="12" rx="2"/><circle cx="7.5" cy="8" r="1.2"/><path d="M3.8 14L7.8 10.2L10.8 13L13.2 10.8L16.4 14"/>"##,
  )
}

fn lock_icon(color: &str) -> Svg {
  local_stroked(
    14.0,
    color,
    r##"<rect x="4.5" y="9" width="11" height="7.5" rx="1.8"/><path d="M7 9V6.8A3 3 0 0 1 13 6.8V9"/>"##,
  )
}

fn chevron_up_icon(color: &str) -> Svg {
  local_stroked(12.0, color, r##"<path d="M5 12.5L10 7.5L15 12.5"/>"##)
}

fn math_icon(color: &str) -> Svg {
  local_stroked(
    15.0,
    color,
    r##"<path d="M14.5 5H6.5L11 10L6.5 15H14.5"/>"##,
  )
}

fn toolbar_btn<F, I>(
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
    .on_press(on_click)
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      icon_fn(if active.get() {
        p.text_primary_hex
      } else {
        p.text_muted_hex
      })
    }))
}

fn toolbar_btn_active<F, I>(
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
    .on_press(on_click)
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      icon_fn(if active.get() {
        p.text_primary_hex
      } else {
        p.text_muted_hex
      })
    }))
}

fn toolbar_btn_dim<F, I>(
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
  button()
    .test_id(tid)
    .accessibility_label(a11y)
    .w(px(28.0))
    .h(px(28.0))
    .items_center()
    .justify_center()
    .rounded(px(5.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      icon_fn(if enabled.get() {
        p.text_muted_hex
      } else {
        p.text_ghost_hex
      })
    }))
}

fn toolbar_sep(dark_mode: Signal<bool>) -> impl IntoElement {
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
