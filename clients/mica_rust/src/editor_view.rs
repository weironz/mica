//! Pure-Rust document editor view built on `nsh_base::rich_editor::view::RichEditor`,
//! `MicaDocumentBridge`, and Mica's custom `NodeView`s (`node_views.rs`):
//! - Strict single-row 40px page header: breadcrumb + copy-link (left) · borderless minimal
//!   icon formatting toolbar with live active states (center) · last-edited, right-sidebar
//!   toggle, whiteboard, favorite star, trash, and page menu (right).
//! - Centered 740px white paper canvas with borderless 30px SemiBold editable page title,
//!   native ProseMirror/Tiptap `RichEditor` (multi-block selection, OS-native IME, inline
//!   marks, tables, real `nsh_base::editor()` code blocks, RaTeX math, Mermaid diagrams,
//!   embedded whiteboards, interactive todo checklists), floating warm-paper bubble menu
//!   when text is selected, slash command palette (`/`), and quiet word count pill.

use std::{collections::BTreeMap, rc::Rc};

use nsh_base::{
  button, input,
  rich_editor::view::{
    PartStyles, RichEditorChange, RichEditorSelection, RichEditorStyles, rich_editor,
  },
  text_view::PartStyle,
};
use nsh_core::{
  Color, LineHeight, PropertyValue, StyleDecl, StyleKey, StyleValue, SubmitBehavior,
  element::{KeyOptions, Style, on_key_with, set_clipboard},
  geometry::px,
  keymap::When,
  prelude::*,
};

use crate::{
  editor_slash::SLASH_ITEMS,
  mica_icons,
  mica_rich_bridge::MicaDocumentBridge,
  mica_theme::palette,
  node_views::{OpenBoardCallback, register_mica_node_views},
};

pub use crate::mica_rich_bridge::OutlineItem;

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
}

pub fn editor_view(
  bridge: Rc<MicaDocumentBridge>,
  header: Rc<EditorHeaderProps>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  // Allow embedded `board` NodeViews inside `RichEditor` to open the full whiteboard.
  provide_context(OpenBoardCallback(header.on_open_board.clone()));

  let page_menu_open = signal(false);
  let slash_open = signal(false);
  let slash_query = signal(String::new());
  let has_text_selection = signal(false);

  on_key_with(
    "escape",
    KeyOptions {
      when: When::Always,
      pass_through: true,
      ..KeyOptions::default()
    },
    move |_| {
      slash_open.set(false);
      slash_query.set(String::new());
      page_menu_open.set(false);
      has_text_selection.set(false);
    },
  );

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

  let b_copy_path = bridge.clone();
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
  let b_list = bridge.clone();
  let b_ordered = bridge.clone();
  let b_todo = bridge.clone();
  let b_quote = bridge.clone();
  let b_div = bridge.clone();
  let b_tbl = bridge.clone();
  let b_copy_md = bridge.clone();
  let b_slash = bridge.clone();
  let b_title_enter = bridge.clone();
  let b_rich = bridge.clone();

  let h_title_val = header.clone();
  let h_title_chg = header.clone();
  let h_board = header.clone();
  let h_trash = header.clone();
  let h_fav = header.clone();
  let h_right = header.clone();
  let h_copy_id = header.clone();
  let h_menu_exp_md = header.clone();
  let h_menu_exp_html = header.clone();
  let h_menu_snap = header.clone();
  let h_menu_right = header.clone();
  let h_menu_trash = header.clone();

  view()
    .flex_1()
    .min_w(px(0.0))
    .relative()
    .bg(move || palette(dark_mode.get()).surface_base)
    .child(
      // 1. Strict single-row 40px page header (NO wrapping!)
      view()
        .w_full()
        .h(px(40.0))
        .px_3()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_1()
        .border_b_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .bg(move || palette(dark_mode.get()).surface_base)
        .child(
          // Left breadcrumb path + copy-link icon (truncates smoothly on narrow screens)
          view()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
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
                      .text_size(px(12.5))
                      .text_color(move || palette(dark_mode.get()).text_muted)
                      .truncate(),
                  )
                  .child(
                    text(">")
                      .text_size(px(11.5))
                      .text_color(move || palette(dark_mode.get()).text_ghost),
                  )
                  .child(
                    text(move || hb2.folder_name.get())
                      .text_size(px(12.5))
                      .text_color(move || palette(dark_mode.get()).text_muted)
                      .truncate(),
                  )
                  .child(
                    text(">")
                      .text_size(px(11.5))
                      .text_color(move || palette(dark_mode.get()).text_ghost),
                  )
              }
            }))
            .child({
              let hb = header.clone();
              text(move || {
                let t = hb.page_title.get();
                if t.trim().is_empty() {
                  "无标题".to_string()
                } else {
                  t
                }
              })
              .text_size(px(12.5))
              .font_weight(FontWeight::MEDIUM)
              .text_color(move || palette(dark_mode.get()).text_primary)
              .truncate()
            })
            .child(toolbar_btn(
              "ed-copy-path",
              "复制页面链接",
              dark_mode,
              signal(false),
              move || {
                let id = h_copy_id.page_id_str.get_untracked();
                set_clipboard(format!("mica://page/{id}"));
                b_copy_path.set_status_flash("已复制页面链接");
              },
              mica_icons::copy_icon,
            )),
        )
        .child(
          // Center compact single-row icon formatting toolbar (exact Notion/Mica order)
          view()
            .flex_shrink_0()
            .flex_row()
            .items_center()
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
              move || {
                toggle_block_type(&b_h1, "heading", Some(1));
              },
              |c| mica_icons::heading_box("1", c),
            ))
            .child(toolbar_btn_active(
              "ed-h2",
              "二级标题",
              dark_mode,
              computed(move || active_kind.get() == "heading" && active_level.get() == 2),
              move || {
                toggle_block_type(&b_h2, "heading", Some(2));
              },
              |c| mica_icons::heading_box("2", c),
            ))
            .child(toolbar_btn_active(
              "ed-h3",
              "三级标题",
              dark_mode,
              computed(move || active_kind.get() == "heading" && active_level.get() == 3),
              move || {
                toggle_block_type(&b_h3, "heading", Some(3));
              },
              |c| mica_icons::heading_box("3", c),
            ))
            .child(toolbar_sep(dark_mode))
            .child(toolbar_btn_active(
              "ed-bold",
              "粗体 (Ctrl+B)",
              dark_mode,
              computed(move || {
                active_marks
                  .get()
                  .iter()
                  .any(|m| m == "bold" || m == "strong")
              }),
              move || b_bold.toggle_mark("bold"),
              mica_icons::bold_icon,
            ))
            .child(toolbar_btn_active(
              "ed-italic",
              "斜体 (Ctrl+I)",
              dark_mode,
              computed(move || {
                active_marks
                  .get()
                  .iter()
                  .any(|m| m == "italic" || m == "em")
              }),
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
              "行内代码 (Ctrl+E)",
              dark_mode,
              computed(move || active_marks.get().iter().any(|m| m == "code")),
              move || b_code.toggle_mark("code"),
              mica_icons::code_icon,
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
              "ed-slash-open",
              "插入块菜单 (/)",
              dark_mode,
              slash_open,
              move || {
                slash_query.set(String::new());
                slash_open.set(!slash_open.get_untracked());
              },
              mica_icons::plus,
            )),
        )
        .child(
          // Right cluster: last-edited + right-sidebar toggle + whiteboard + favorite + trash + more
          view()
            .flex_shrink_0()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .child(when(move || !right_open.get(), move || {
              text(move || last_edited.get())
                .text_size(px(11.5))
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
            .child(
              button()
                .test_id("ed-star")
                .accessibility_label("收藏或取消收藏当前页面")
                .w(px(23.0))
                .h(px(26.0))
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
              "页面更多操作",
              dark_mode,
              page_menu_open,
              move || page_menu_open.set(!page_menu_open.get_untracked()),
              mica_icons::more_dots,
            )),
        ),
    )
    .child(when(move || page_menu_open.get(), move || {
      let exp_md = h_menu_exp_md.on_export_markdown.clone();
      let exp_html = h_menu_exp_html.on_export_html.clone();
      let snap = h_menu_snap.on_save_snapshot.clone();
      let right_cb = h_menu_right.on_toggle_right_panel.clone();
      let trash_cb = h_menu_trash.on_trash_page.clone();
      let b_cm = b_copy_md.clone();
      view()
        .test_id("page-more-menu")
        .absolute()
        .top(px(42.0))
        .right(px(12.0))
        .w(px(208.0))
        .p_1()
        .gap_1()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .z_index(35)
        .child(menu_row("page-menu-copy-md", "复制 Markdown", dark_mode, move || {
          set_clipboard(b_cm.export_markdown());
          b_cm.set_status_flash("已复制 Markdown");
          page_menu_open.set(false);
        }))
        .child(menu_row("page-menu-export-md", "导出 Markdown", dark_mode, move || {
          exp_md();
          page_menu_open.set(false);
        }))
        .child(menu_row("page-menu-export-html", "导出 HTML", dark_mode, move || {
          exp_html();
          page_menu_open.set(false);
        }))
        .child(menu_row("page-menu-snapshot", "保存版本快照", dark_mode, move || {
          snap();
          page_menu_open.set(false);
        }))
        .child(menu_row("page-menu-outline", "展开/隐藏右侧大纲", dark_mode, move || {
          right_cb();
          page_menu_open.set(false);
        }))
        .child(
          view()
            .w_full()
            .h(px(1.0))
            .bg(move || palette(dark_mode.get()).border_subtle),
        )
        .child(menu_row("page-menu-trash", "移入回收站", dark_mode, move || {
          trash_cb();
          page_menu_open.set(false);
        }))
    }))
    .child(when(move || slash_open.get(), move || {
      let b_pick = b_slash.clone();
      dynamic(move || {
        let q = slash_query.get();
        slash_palette_view(q, b_pick.clone(), slash_open, slash_query, dark_mode)
      })
    }))
    .child(
      // 2. Centered 740px white paper canvas
      view()
        .flex_1()
        .w_full()
        .items_center()
        .bg(move || palette(dark_mode.get()).surface_base)
        .child(
          view()
            .flex_1()
            .w_full()
            .max_w(px(740.0))
            .px_8()
            .pt_8()
            .pb_4()
            .child(
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
                .value(move || h_title_val.page_title.get())
                .on_change(move |v: &str| (h_title_chg.on_rename_title)(v.to_owned()))
                .on_submit(move |_: &str| {
                  b_title_enter.focus_pos(1);
                  b_title_enter.session().focus();
                }),
            )
            .child({
              // Mount native `RichEditor` once against the stable `EditorSession`
              // so keystrokes, selections, and embedded NodeViews never lose focus.
              let b_chg = b_rich.clone();
              let b_bubble = b_rich.clone();
              rich_editor()
                .test_id("mica-rich-editor")
                .accessibility_label("文档正文编辑器")
                .flex_1()
                .w_full()
                .min_h(px(360.0))
                .editor(Some(rich_session))
                .placeholder("输入正文，或输入 / 唤出命令...")
                .styles(move || mica_rich_editor_styles(dark_mode.get()))
                .bubble_menu_placement(Placement::Top)
                .bubble_menu_offset(px(8.0))
                .bubble_menu(bubble_menu_view(b_bubble, has_text_selection, dark_mode))
                .on_selection_change(move |sel: &RichEditorSelection| {
                  has_text_selection.set(!sel.empty && sel.kind == "text");
                })
                .on_change(move |_: &RichEditorChange| {
                  sync_slash_palette_from_caret(&b_chg, slash_open, slash_query);
                })
            }),
        ),
    )
    .child(
      // 3. Quiet bottom status bar with right-aligned word/char count pill
      view()
        .h(px(32.0))
        .px_4()
        .flex_row()
        .items_center()
        .justify_end()
        .bg(move || palette(dark_mode.get()).surface_base)
        .child(
          view()
            .test_id("editor-word-count")
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .bg(move || palette(dark_mode.get()).surface_sunken)
            .child(
              text(move || word_count.get())
                .text_size(px(12.0))
                .text_color(move || palette(dark_mode.get()).text_faint),
            ),
        ),
    )
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
  slash_open: Signal<bool>,
  slash_query: Signal<String>,
) {
  let state = bridge.session().state();
  let sel = state.selection();
  if !sel.empty() {
    return;
  }
  let parent = sel.resolved_from().parent();
  if parent.is_textblock() && parent.ty().name() == "paragraph" {
    let txt = parent.text_content();
    if txt.starts_with('/') && txt.len() <= 24 && !txt.contains(' ') {
      slash_open.set(true);
      slash_query.set(txt);
      return;
    }
  }
  if slash_open.get_untracked() && !slash_query.get_untracked().is_empty() {
    slash_open.set(false);
    slash_query.set(String::new());
  }
}

fn strip_leading_slash_in_current_block(bridge: &MicaDocumentBridge) {
  let session = bridge.session();
  let state = session.state();
  let rfrom = state.selection().resolved_from();
  let parent = rfrom.parent();
  if parent.is_textblock() && parent.ty().name() == "paragraph" && parent.text_content().starts_with('/') {
    let from = rfrom.start(rfrom.depth);
    let to = rfrom.end(rfrom.depth);
    if from < to {
      let mut tr = state.tr();
      if tr.delete(from, to).is_ok() {
        let _ = session.dispatch(tr);
      }
    }
  }
}

fn apply_slash_item(bridge: &MicaDocumentBridge, kind: &str) {
  strip_leading_slash_in_current_block(bridge);
  match kind {
    "heading1" => bridge.set_block_type("heading", Some(1)),
    "heading2" => bridge.set_block_type("heading", Some(2)),
    "heading3" => bridge.set_block_type("heading", Some(3)),
    "bulleted_list" => bridge.set_block_type("bulleted_list", None),
    "numbered_list" => bridge.set_block_type("numbered_list", None),
    "todo" => bridge.set_block_type("todo", None),
    "quote" => bridge.set_block_type("quote", None),
    "code_block" => bridge.insert_custom_block("code_block", "", Some("rust")),
    "table" => bridge.insert_table(2, 2),
    "math" => bridge.insert_custom_block("math", r"E = mc^2", None),
    "mermaid" => bridge.insert_custom_block("mermaid", "graph LR\n  A[Mica] --> B[Pure Rust]", None),
    "board" => {
      let starter = mica_board::starter_board_json("mindmap");
      bridge.insert_custom_block("board", &starter, None);
    }
    "divider" => bridge.insert_divider(),
    _ => bridge.set_block_type("paragraph", None),
  }
  bridge.session().focus();
}

fn slash_palette_view(
  query: String,
  bridge: Rc<MicaDocumentBridge>,
  slash_open: Signal<bool>,
  slash_query: Signal<String>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let q = query.trim().trim_start_matches('/').to_lowercase();
  let mut panel = view()
    .test_id("slash-menu")
    .w_full()
    .max_w(px(740.0))
    .flex_row()
    .flex_wrap()
    .items_center()
    .gap_1()
    .px_3()
    .py_2()
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .child(
      text("插入块 (/):")
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_faint),
    );

  for item in SLASH_ITEMS {
    if !q.is_empty()
      && !item.label.to_lowercase().contains(&q)
      && !item.keywords.to_lowercase().contains(&q)
    {
      continue;
    }
    let b = bridge.clone();
    let kind = item.kind;
    panel = panel.child(
      button()
        .test_id(item.id)
        .accessibility_label(format!("插入 {}", item.label))
        .px_2()
        .py_1()
        .rounded(px(5.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .bg(move || palette(dark_mode.get()).surface_sunken)
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || {
          apply_slash_item(&b, kind);
          slash_open.set(false);
          slash_query.set(String::new());
        })
        .child(
          text(item.label)
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).text_primary),
        ),
    );
  }

  panel = panel.child(
    button()
      .test_id("slash-close")
      .accessibility_label("关闭插入块菜单")
      .px_2()
      .py_1()
      .rounded(px(5.0))
      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
      .on_press(move || {
        slash_open.set(false);
        slash_query.set(String::new());
      })
      .child(
        text("✕")
          .text_size(px(12.0))
          .text_color(move || palette(dark_mode.get()).text_muted),
      ),
  );

  view().w_full().items_center().py_1().px_4().child(panel)
}

/// Floating warm-paper inline formatting bar anchored above selected text.
fn bubble_menu_view(
  bridge: Rc<MicaDocumentBridge>,
  has_text_selection: Signal<bool>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  when(move || has_text_selection.get(), move || {
    let active_marks = bridge.active_marks_signal();
    let b_bold = bridge.clone();
    let b_italic = bridge.clone();
    let b_strike = bridge.clone();
    let b_code = bridge.clone();

    view()
      .test_id("editor-bubble-menu")
      .flex_row()
      .items_center()
      .gap_1()
      .px_1()
      .py_1()
      .rounded(px(8.0))
      .border_1()
      .border_color(move || palette(dark_mode.get()).border_normal)
      .bg(move || palette(dark_mode.get()).surface_overlay)
      .child(bubble_btn(
        "bubble-bold",
        "加粗选中文字",
        dark_mode,
        computed(move || {
          active_marks
            .get()
            .iter()
            .any(|m| m == "bold" || m == "strong")
        }),
        move || b_bold.toggle_mark("bold"),
        mica_icons::bold_icon,
      ))
      .child(bubble_btn(
        "bubble-italic",
        "斜体选中文字",
        dark_mode,
        computed(move || {
          active_marks
            .get()
            .iter()
            .any(|m| m == "italic" || m == "em")
        }),
        move || b_italic.toggle_mark("italic"),
        mica_icons::italic_icon,
      ))
      .child(bubble_btn(
        "bubble-strike",
        "为选中文字添加删除线",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "strike")),
        move || b_strike.toggle_mark("strike"),
        mica_icons::strikethrough_icon,
      ))
      .child(bubble_btn(
        "bubble-code",
        "设为行内代码",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "code")),
        move || b_code.toggle_mark("code"),
        mica_icons::code_icon,
      ))
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
    .w(px(23.0))
    .h(px(26.0))
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
fn mica_rich_editor_styles(dark_mode: bool) -> RichEditorStyles {
  let p = palette(dark_mode);
  let mut styles = RichEditorStyles::default();

  styles.root = Some(PartStyle(decl_style([
    color_prop(StyleKey::BackgroundColor, p.surface_base),
    color_prop(StyleKey::Color, p.text_primary),
  ])));
  styles.content = Some(PartStyle(decl_style([
    num_prop(StyleKey::PaddingBottom, 160.0),
  ])));
  styles.caret = Some(PartStyle(decl_style([color_prop(
    StyleKey::BackgroundColor,
    p.text_primary,
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
      num_prop(StyleKey::FontSize, 16.0),
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
      num_prop(StyleKey::FontSize, 30.0),
      num_prop(StyleKey::MarginTop, 18.0),
      num_prop(StyleKey::MarginBottom, 6.0),
    ]),
  );
  nodes.insert(
    "heading[level=2]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, 24.0),
      num_prop(StyleKey::MarginTop, 14.0),
      num_prop(StyleKey::MarginBottom, 4.0),
    ]),
  );
  nodes.insert(
    "heading[level=3]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, 20.0),
      num_prop(StyleKey::MarginTop, 10.0),
      num_prop(StyleKey::MarginBottom, 4.0),
    ]),
  );
  nodes.insert(
    "heading[level=4]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, 18.0),
      num_prop(StyleKey::MarginTop, 8.0),
      num_prop(StyleKey::MarginBottom, 2.0),
    ]),
  );
  nodes.insert(
    "heading[level=5]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, 16.0),
      num_prop(StyleKey::MarginTop, 6.0),
      num_prop(StyleKey::MarginBottom, 2.0),
    ]),
  );
  nodes.insert(
    "heading[level=6]".to_string(),
    decl_style([
      num_prop(StyleKey::FontSize, 15.0),
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

  let list_style = decl_style([
    num_prop(StyleKey::MarginTop, 2.0),
    num_prop(StyleKey::MarginBottom, 2.0),
    num_prop(StyleKey::FontSize, 16.0),
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
      color_prop(StyleKey::BackgroundColor, p.code_bg),
      color_prop(StyleKey::Color, p.danger),
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
    .w(px(23.0))
    .h(px(26.0))
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
    .w(px(22.0))
    .h(px(26.0))
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
    .w(px(22.0))
    .h(px(26.0))
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
