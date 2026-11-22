//! Route 2-B pure-Rust block editor view styled 1:1 to Mica's warm-paper Notion aesthetic:
//! - Clean 40px page header row: breadcrumb path (left) · borderless minimal icon formatting
//!   toolbar (center) · last-edited timestamp & page actions (right).
//! - Centered 720px reading column with generous whitespace, large borderless editable title,
//!   true typographic scale (H1 30px / H2 24px / H3 20px / Body 16px), and seamless in-place
//!   OS-native IME editing right on the focused block (zero debug bar, zero `[H1]` tags).
//! - Quiet bottom-right word & character count pill (`N 词 · M 字`).

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_editor::{
  DocPosition, DocSelection, EditorBlock, EditorCommand, EditorMark, EditorSession,
  NormalizedSelection, utf16_len,
};
use nsh_base::{button, input};
use nsh_core::{element::Position, geometry::px, prelude::*};

use crate::{
  atomic_blocks::{AtomicBlockInput, is_atomic_or_rich_block, render_atomic_block},
  editor_slash::{command_for_slash, slash_menu_view},
  editor_table::table_block_view,
  mica_icons,
  mica_theme::palette,
};

#[derive(Clone, PartialEq)]
struct BlockDisplay {
  id: String,
  kind: String,
  text: String,
  marks: Vec<EditorMark>,
  data: serde_json::Value,
  indent: u32,
  checked: bool,
  level: u64,
}

impl From<&EditorBlock> for BlockDisplay {
  fn from(b: &EditorBlock) -> Self {
    Self {
      id: b.id.clone(),
      kind: b.kind.clone(),
      text: b.text.clone(),
      marks: b.marks(),
      data: b.data.clone(),
      indent: b.indent(),
      checked: b.data.get("checked").and_then(|v| v.as_bool()).unwrap_or(false),
      level: b.data.get("level").and_then(|v| v.as_u64()).unwrap_or(1),
    }
  }
}

type PersistCallback = Rc<dyn Fn(String, Option<Vec<u8>>)>;

/// Reactive mirror over `EditorSession` implementing per-block signal granularity.
pub struct EditorViewState {
  session: Rc<RefCell<EditorSession>>,
  block_ids: Rc<RefCell<Vec<String>>>,
  block_signals: Rc<RefCell<HashMap<String, Signal<BlockDisplay>>>>,
  on_persist: Rc<RefCell<Option<PersistCallback>>>,
  structure_rev: Signal<u64>,
  focused_idx: Signal<usize>,
  focused_text: Signal<String>,
  composing_text: Signal<String>,
  word_count_text: Signal<String>,
  last_edited_text: Signal<String>,
  slash_open: Signal<bool>,
  slash_query: Signal<String>,
  can_undo: Signal<bool>,
  can_redo: Signal<bool>,
}

impl EditorViewState {
  pub fn new(session: EditorSession) -> Rc<Self> {
    let ids: Vec<String> = session.blocks().iter().map(|b| b.id.clone()).collect();
    let mut map = HashMap::with_capacity(ids.len());
    for b in session.blocks() {
      map.insert(b.id.clone(), signal(BlockDisplay::from(b)));
    }
    let first_text = session
      .blocks()
      .first()
      .map(|b| b.text.clone())
      .unwrap_or_default();
    let counts = compute_word_count_label(session.blocks());
    let can_u = session.can_undo();
    let can_r = session.can_redo();
    Rc::new(Self {
      session: Rc::new(RefCell::new(session)),
      block_ids: Rc::new(RefCell::new(ids)),
      block_signals: Rc::new(RefCell::new(map)),
      on_persist: Rc::new(RefCell::new(None)),
      structure_rev: signal(0),
      focused_idx: signal(0),
      focused_text: signal(first_text),
      composing_text: signal(String::new()),
      word_count_text: signal(counts),
      last_edited_text: signal(String::from("上次编辑 刚刚")),
      slash_open: signal(false),
      slash_query: signal(String::new()),
      can_undo: signal(can_u),
      can_redo: signal(can_r),
    })
  }

  pub fn set_on_persist(&self, cb: Rc<dyn Fn(String, Option<Vec<u8>>)>) {
    *self.on_persist.borrow_mut() = Some(cb);
  }

  pub fn load_session(&self, next: EditorSession) {
    let displays: Vec<(String, BlockDisplay)> = next
      .blocks()
      .iter()
      .map(|b| (b.id.clone(), BlockDisplay::from(b)))
      .collect();
    let ids: Vec<String> = displays.iter().map(|(id, _)| id.clone()).collect();
    let mut map = HashMap::with_capacity(ids.len());
    for (id, disp) in displays {
      map.insert(id, signal(disp));
    }
    let first_text = next
      .blocks()
      .first()
      .map(|b| b.text.clone())
      .unwrap_or_default();
    let counts = compute_word_count_label(next.blocks());
    let can_u = next.can_undo();
    let can_r = next.can_redo();
    *self.session.borrow_mut() = next;
    *self.block_ids.borrow_mut() = ids;
    *self.block_signals.borrow_mut() = map;
    self.slash_open.set(false);
    self.slash_query.set(String::new());
    self.can_undo.set(can_u);
    self.can_redo.set(can_r);
    self.focused_idx.set(0);
    self.focused_text.set(first_text);
    self.word_count_text.set(counts);
    self.structure_rev.set(self.structure_rev.get_untracked() + 1);
  }

  fn rebuild_block_signals(&self) {
    let displays: Vec<(String, BlockDisplay)> = {
      let session = self.session.borrow();
      session
        .blocks()
        .iter()
        .map(|b| (b.id.clone(), BlockDisplay::from(b)))
        .collect()
    };
    let ids: Vec<String> = displays.iter().map(|(id, _)| id.clone()).collect();
    let mut map = HashMap::with_capacity(ids.len());
    for (id, disp) in displays {
      map.insert(id, signal(disp));
    }
    *self.block_ids.borrow_mut() = ids;
    *self.block_signals.borrow_mut() = map;
  }

  pub fn dispatch(&self, cmd: EditorCommand) {
    let (content_changed, crdt_update) = {
      let patch = self.session.borrow_mut().dispatch(cmd);
      (patch.content_changed, patch.crdt_update)
    };
    self.sync_from_session(content_changed);
    if content_changed {
      self.last_edited_text.set(String::from("上次编辑 刚刚"));
      let persist_cb = self.on_persist.borrow().clone();
      if let Some(cb) = persist_cb {
        cb(self.export_markdown(), crdt_update);
      }
    }
  }

  fn sync_from_session(&self, content_changed: bool) {
    let (displays, next_idx, next_text, can_u, can_r, counts) = {
      let session = self.session.borrow();
      let blocks = session.blocks();
      let displays: Vec<(String, BlockDisplay)> = blocks
        .iter()
        .map(|b| (b.id.clone(), BlockDisplay::from(b)))
        .collect();
      let next_idx = session
        .selection()
        .map(|sel| match sel.normalized() {
          NormalizedSelection::Text { end, .. } => end.node,
          NormalizedSelection::BlockAtom { block_index }
          | NormalizedSelection::TableArea { block_index, .. } => block_index,
        })
        .unwrap_or(0)
        .min(blocks.len().saturating_sub(1));
      let next_text = blocks
        .get(next_idx)
        .map(|b| b.text.clone())
        .unwrap_or_default();
      let can_u = session.can_undo();
      let can_r = session.can_redo();
      let counts = compute_word_count_label(blocks);
      (displays, next_idx, next_text, can_u, can_r, counts)
    };

    let new_ids: Vec<String> = displays.iter().map(|(id, _)| id.clone()).collect();
    let structure_changed = *self.block_ids.borrow() != new_ids;

    if structure_changed {
      let mut updates: Vec<(Signal<BlockDisplay>, BlockDisplay)> = Vec::new();
      {
        let mut map = self.block_signals.borrow_mut();
        for (id, disp) in displays {
          if let Some(&sig) = map.get(&id) {
            if sig.get_untracked() != disp {
              updates.push((sig, disp));
            }
          } else {
            map.insert(id, signal(disp));
          }
        }
        map.retain(|id, _| new_ids.iter().any(|nid| nid == id));
      }
      *self.block_ids.borrow_mut() = new_ids;
      for (sig, disp) in updates {
        sig.set(disp);
      }
      self.structure_rev.set(self.structure_rev.get_untracked() + 1);
    } else if content_changed {
      let updates: Vec<(Signal<BlockDisplay>, BlockDisplay)> = {
        let map = self.block_signals.borrow();
        displays
          .into_iter()
          .filter_map(|(id, disp)| {
            let &sig = map.get(&id)?;
            (sig.get_untracked() != disp).then_some((sig, disp))
          })
          .collect()
      };
      for (sig, disp) in updates {
        sig.set(disp);
      }
    }

    self.focused_idx.set(next_idx);
    if self.focused_text.get_untracked() != next_text {
      self.focused_text.set(next_text);
    }
    self.can_undo.set(can_u);
    self.can_redo.set(can_r);
    self.word_count_text.set(counts);
  }

  pub fn select_block(&self, idx: usize) {
    let end_u16 = self
      .session
      .borrow()
      .blocks()
      .get(idx)
      .map(|b| utf16_len(&b.text))
      .unwrap_or(0);
    self.dispatch(EditorCommand::SetSelection {
      selection: Some(DocSelection::caret(idx, end_u16)),
    });
  }

  pub fn export_markdown(&self) -> String {
    self
      .session
      .borrow()
      .export_markdown(&std::collections::BTreeMap::new())
  }

  pub fn export_html(&self) -> String {
    let md = self.export_markdown();
    let doc = mica_md::parse(&md, &mica_md::ParseOptions::default());
    mica_md::render_html(&doc, &mica_md::HtmlOptions::default()).unwrap_or_default()
  }

  fn toggle_mark_on_focused(&self, mark_kind: &str) {
    let idx = self.focused_idx.get_untracked();
    let len = self
      .session
      .borrow()
      .blocks()
      .get(idx)
      .map(|b| utf16_len(&b.text))
      .unwrap_or(0);
    if len > 0 {
      self.dispatch(EditorCommand::SetSelection {
        selection: Some(DocSelection::from_positions(
          DocPosition::new(idx, 0),
          DocPosition::new(idx, len),
        )),
      });
      self.dispatch(EditorCommand::ToggleMark {
        kind: mark_kind.into(),
        href: None,
      });
    }
  }
}

pub struct EditorHeaderProps {
  pub workspace_name: Signal<String>,
  pub folder_name: Signal<String>,
  pub page_title: Signal<String>,
  pub on_rename_title: Rc<dyn Fn(String)>,
  pub on_open_board: Rc<dyn Fn()>,
  pub on_trash_page: Rc<dyn Fn()>,
}

pub fn editor_view(
  state: Rc<EditorViewState>,
  dark_mode: Signal<bool>,
  header: Rc<EditorHeaderProps>,
) -> impl IntoElement {
  state.rebuild_block_signals();

  // Global undo/redo keyboard shortcuts
  {
    let st_u = state.clone();
    on_key("ctrl+z", move |_| st_u.dispatch(EditorCommand::Undo));
    let st_u_mac = state.clone();
    on_key("cmd+z", move |_| st_u_mac.dispatch(EditorCommand::Undo));
    let st_r = state.clone();
    on_key("ctrl+shift+z", move |_| st_r.dispatch(EditorCommand::Redo));
    let st_r_mac = state.clone();
    on_key("cmd+shift+z", move |_| st_r_mac.dispatch(EditorCommand::Redo));
  }

  let st_undo = state.clone();
  let st_redo = state.clone();
  let st_para = state.clone();
  let st_h1 = state.clone();
  let st_h2 = state.clone();
  let st_h3 = state.clone();
  let st_bold = state.clone();
  let st_italic = state.clone();
  let st_code = state.clone();
  let st_list = state.clone();
  let st_todo = state.clone();
  let st_quote = state.clone();
  let st_div = state.clone();
  let st_tbl_btn = state.clone();
  let st_slash_btn = state.clone();
  let st_slash_vis = state.clone();
  let st_slash_pick = state.clone();
  let st_count = state.clone();
  let st_keys = state.clone();
  let st_row = state.clone();
  let st_edited = state.clone();
  let st_words = state.clone();

  let h_bread = header.clone();
  let h_title_val = header.clone();
  let h_title_chg = header.clone();
  let h_board = header.clone();
  let h_trash = header.clone();

  view()
    .flex_1()
    .bg(move || palette(dark_mode.get()).surface_base)
    .child(
      // 1. Original 40px page header row: breadcrumb (left) · borderless icon toolbar (center) · meta/actions (right)
      view()
        .w_full()
        .min_h(px(40.0))
        .px_4()
        .py_1()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_2()
        .border_b_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .bg(move || palette(dark_mode.get()).surface_base)
        .child(
          // Breadcrumb: workspace > folder > page
          view()
            .flex_row()
            .items_center()
            .gap_1()
            .child(
              text(move || h_bread.workspace_name.get())
                .text_size(px(13.0))
                .text_color(move || palette(dark_mode.get()).text_muted),
            )
            .child(
              text(">")
                .text_size(px(12.0))
                .text_color(move || palette(dark_mode.get()).text_ghost),
            )
            .child({
              let hb = header.clone();
              text(move || hb.folder_name.get())
                .text_size(px(13.0))
                .text_color(move || palette(dark_mode.get()).text_muted)
            })
            .child(
              text(">")
                .text_size(px(12.0))
                .text_color(move || palette(dark_mode.get()).text_ghost),
            )
            .child({
              let hb = header.clone();
              text(move || hb.page_title.get())
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || palette(dark_mode.get()).text_primary)
            })
            .child(icon_btn("ed-copy-path", dark_mode, || {}, mica_icons::copy_icon)),
        )
        .child(
          // Minimal borderless icon formatting toolbar (exact Notion/Mica order)
          view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(icon_btn(
              "ed-undo",
              dark_mode,
              move || st_undo.dispatch(EditorCommand::Undo),
              mica_icons::undo,
            ))
            .child(icon_btn(
              "ed-redo",
              dark_mode,
              move || st_redo.dispatch(EditorCommand::Redo),
              mica_icons::redo,
            ))
            .child(toolbar_sep(dark_mode))
            .child(icon_btn(
              "ed-para",
              dark_mode,
              move || {
                st_para.dispatch(EditorCommand::TransformBlock {
                  kind: "paragraph".into(),
                  data: None,
                });
              },
              mica_icons::align_text,
            ))
            .child(icon_btn(
              "ed-h1",
              dark_mode,
              move || {
                st_h1.dispatch(EditorCommand::TransformBlock {
                  kind: "heading".into(),
                  data: Some(serde_json::json!({ "level": 1 })),
                });
              },
              |c| mica_icons::heading_box("1", c),
            ))
            .child(icon_btn(
              "ed-h2",
              dark_mode,
              move || {
                st_h2.dispatch(EditorCommand::TransformBlock {
                  kind: "heading".into(),
                  data: Some(serde_json::json!({ "level": 2 })),
                });
              },
              |c| mica_icons::heading_box("2", c),
            ))
            .child(icon_btn(
              "ed-h3",
              dark_mode,
              move || {
                st_h3.dispatch(EditorCommand::TransformBlock {
                  kind: "heading".into(),
                  data: Some(serde_json::json!({ "level": 3 })),
                });
              },
              |c| mica_icons::heading_box("3", c),
            ))
            .child(toolbar_sep(dark_mode))
            .child(icon_btn(
              "ed-bold",
              dark_mode,
              move || st_bold.toggle_mark_on_focused("bold"),
              mica_icons::bold_icon,
            ))
            .child(icon_btn(
              "ed-italic",
              dark_mode,
              move || st_italic.toggle_mark_on_focused("italic"),
              mica_icons::italic_icon,
            ))
            .child(icon_btn(
              "ed-code",
              dark_mode,
              move || st_code.toggle_mark_on_focused("code"),
              mica_icons::code_icon,
            ))
            .child(toolbar_sep(dark_mode))
            .child(icon_btn(
              "ed-list",
              dark_mode,
              move || {
                st_list.dispatch(EditorCommand::TransformBlock {
                  kind: "bulleted_list".into(),
                  data: None,
                });
              },
              mica_icons::list_bullet,
            ))
            .child(icon_btn(
              "ed-todo",
              dark_mode,
              move || {
                st_todo.dispatch(EditorCommand::TransformBlock {
                  kind: "todo".into(),
                  data: Some(serde_json::json!({ "checked": false })),
                });
              },
              mica_icons::todo_box,
            ))
            .child(icon_btn(
              "ed-quote",
              dark_mode,
              move || {
                st_quote.dispatch(EditorCommand::TransformBlock {
                  kind: "quote".into(),
                  data: None,
                });
              },
              mica_icons::quote_icon,
            ))
            .child(toolbar_sep(dark_mode))
            .child(icon_btn(
              "ed-divider",
              dark_mode,
              move || st_div.dispatch(EditorCommand::InsertDivider),
              mica_icons::divider_icon,
            ))
            .child(icon_btn(
              "ed-table-quick",
              dark_mode,
              move || st_tbl_btn.dispatch(command_for_slash("table")),
              mica_icons::table_icon,
            ))
            .child(icon_btn(
              "ed-slash-open",
              dark_mode,
              move || {
                st_slash_btn.slash_query.set(String::new());
                st_slash_btn
                  .slash_open
                  .set(!st_slash_btn.slash_open.get_untracked());
              },
              mica_icons::plus,
            )),
        )
        .child(
          // Right cluster: last-edited + whiteboard + favorite + trash + more
          view()
            .flex_row()
            .items_center()
            .gap_1()
            .child(
              text(move || st_edited.last_edited_text.get())
                .text_size(px(12.0))
                .text_color(move || palette(dark_mode.get()).text_faint),
            )
            .child(icon_btn(
              "ed-open-board",
              dark_mode,
              move || (h_board.on_open_board)(),
              mica_icons::whiteboard,
            ))
            .child(icon_btn("ed-star", dark_mode, || {}, mica_icons::star_icon))
            .child(icon_btn(
              "trash-page-btn",
              dark_mode,
              move || (h_trash.on_trash_page)(),
              mica_icons::trash_icon,
            ))
            .child(icon_btn("ed-more", dark_mode, || {}, mica_icons::more_dots)),
        ),
    )
    .child(when(move || st_slash_vis.slash_open.get(), move || {
      let q = st_slash_pick.slash_query.get();
      let st_p = st_slash_pick.clone();
      let st_c = st_slash_pick.clone();
      view()
        .w_full()
        .items_center()
        .py_1()
        .child(
          view()
            .w_full()
            .max_w(px(740.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .child(slash_menu_view(
              q,
              Rc::new(move |kind: &'static str| {
                if st_p.focused_text.get_untracked().starts_with('/') {
                  st_p.dispatch(EditorCommand::ImeUpdate {
                    text: String::new(),
                    base: 0,
                    extent: 0,
                    composing_valid: false,
                    composing_collapsed: true,
                    shift_pressed: false,
                  });
                }
                st_p.dispatch(command_for_slash(kind));
                st_p.slash_open.set(false);
                st_p.slash_query.set(String::new());
              }),
              Rc::new(move || {
                st_c.slash_open.set(false);
                st_c.slash_query.set(String::new());
              }),
            )),
        )
    }))
    .child(
      // 2. Centered 740px paper canvas: large borderless title + seamless in-place block list
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
              // Large borderless editable document title (30px SemiBold, exact match to screenshot 2)
              input()
                .test_id("page-title-input")
                .placeholder("无标题")
                .w_full()
                .pb_4()
                .text_size(px(30.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || palette(dark_mode.get()).text_primary)
                .bg(move || palette(dark_mode.get()).surface_base)
                .value(move || h_title_val.page_title.get())
                .on_change(move |v: &str| (h_title_chg.on_rename_title)(v.to_owned())),
            )
            .child(
              // Route 2-B virtualized block list: `count` & `keys` track ONLY `structure_rev`
              virtual_list(
                move || {
                  st_count.structure_rev.get();
                  st_count.block_ids.borrow().len()
                },
                move |at: Position| {
                  let st_cell = st_row.clone();
                  let st_press = st_row.clone();
                  let st_sel = st_row.clone();
                  let st_tbl = st_row.clone();
                  let st_edit = st_row.clone();

                  let is_focused = computed(move || st_sel.focused_idx.get() == at.index());
                  let block_disp = computed(move || {
                    // Track `structure_rev` so recycled `virtual_list` slots re-bind to the new
                    // block ID's signal on page switch / insert / delete / undo without waking
                    // siblings during ordinary in-block typing.
                    st_cell.structure_rev.get();
                    let idx = at.index();
                    let id = st_cell.block_ids.borrow().get(idx).cloned();
                    id.and_then(|bid| st_cell.block_signals.borrow().get(&bid).copied())
                      .map(|sig| sig.get())
                  });

                  let is_table = computed(move || {
                    block_disp.get().is_some_and(|b| b.kind == "table")
                  });
                  let is_atomic = computed(move || {
                    block_disp
                      .get()
                      .is_some_and(|b| b.kind != "table" && is_atomic_or_rich_block(&b.kind))
                  });

                  view()
                    .w_full()
                    .py_1()
                    .child(when(move || is_table.get(), move || {
                      let st_d = st_tbl.clone();
                      dynamic(move || {
                        let disp = block_disp.get().unwrap_or_else(empty_table_disp);
                        let st_inner = st_d.clone();
                        table_block_view(
                          at.index(),
                          &disp.data,
                          Rc::new(move |cmd| st_inner.dispatch(cmd)),
                        )
                      })
                    }))
                    .child(when(move || is_atomic.get(), {
                      let st_ap_outer = st_press.clone();
                      move || {
                        let st_ap_dyn = st_ap_outer.clone();
                        dynamic(move || {
                          let disp = block_disp.get().unwrap_or_else(empty_divider_disp);
                          let dm = dark_mode.get();
                          let st_ap = st_ap_dyn.clone();
                          button()
                            .w_full()
                            .on_press(move || st_ap.select_block(at.index()))
                            .child(render_atomic_block(
                              &AtomicBlockInput {
                                id: disp.id,
                                kind: disp.kind,
                                text: disp.text,
                                data: disp.data,
                              },
                              dm,
                            ))
                        })
                      }
                    }))
                    .child(when(move || !is_table.get() && !is_atomic.get(), {
                      let st_p_outer = st_press.clone();
                      let st_e_outer = st_edit.clone();
                      move || {
                        let st_p_f = st_p_outer.clone();
                        let st_e_f = st_e_outer.clone();
                        let st_p_nf = st_p_outer.clone();
                        // Separate structural typography signature from text so typing / CJK IME
                        // never tears down or recreates the focused native `input()`.
                        let block_style_sig = computed(move || {
                          block_disp
                            .get()
                            .map(|b| (b.kind, b.level, b.indent, b.checked))
                            .unwrap_or_else(|| ("paragraph".into(), 1, 0, false))
                        });

                        view()
                          .w_full()
                          .child(when(move || is_focused.get(), move || {
                            let st_ime = st_e_f.clone();
                            let st_comp = st_e_f.clone();
                            let st_sub = st_e_f.clone();
                            let st_val = st_e_f.clone();
                            let st_td = st_p_f.clone();
                            let is_quote = computed(move || block_style_sig.get().0 == "quote");
                            let left_pad = computed(move || {
                              let (kind, _, indent, _) = block_style_sig.get();
                              let base = (indent as f32) * 20.0;
                              px(if kind == "quote" { base + 12.0 } else { base })
                            });
                            let fsize = computed(move || {
                              let (kind, level, _, _) = block_style_sig.get();
                              px(heading_font_size(&kind, level))
                            });
                            let is_heading_or_bold = computed(move || {
                              block_disp.get().is_some_and(|b| {
                                b.kind == "heading" || b.marks.iter().any(|m| m.kind == "bold")
                              })
                            });

                            view()
                              .w_full()
                              .flex_row()
                              .items_center()
                              .gap_2()
                              .py_1()
                              .pl(move || left_pad.get())
                              .border_l(move || {
                                if is_quote.get() { px(3.0) } else { px(0.0) }
                              })
                              .border_l_color(move || palette(dark_mode.get()).quote_bar)
                              .child(dynamic(move || {
                                let (kind, _, _, checked) = block_style_sig.get();
                                let p = palette(dark_mode.get());
                                let st_td_c = st_td.clone();
                                match kind.as_str() {
                                  "bulleted_list" => view().child(
                                    text("•").text_size(px(16.0)).text_color(p.text_muted),
                                  ),
                                  "numbered_list" => view().child(
                                    text("1.").text_size(px(15.0)).text_color(p.text_muted),
                                  ),
                                  "todo" => view().child(
                                    button()
                                      .on_press(move || {
                                        st_td_c.dispatch(EditorCommand::ToggleTodo {
                                          block_index: at.index(),
                                        });
                                      })
                                      .child(
                                        text(if checked { "☑" } else { "☐" })
                                          .text_size(px(16.0))
                                          .text_color(p.text_muted),
                                      ),
                                  ),
                                  _ => view(),
                                }
                              }))
                              .child(
                                input()
                                  .test_id("editor-ime-input")
                                  .placeholder("输入正文，或输入 / 唤出命令...")
                                  .flex_1()
                                  .text_size(move || fsize.get())
                                  .font_weight(if is_heading_or_bold.get_untracked() {
                                    FontWeight::SEMIBOLD
                                  } else {
                                    FontWeight::NORMAL
                                  })
                                  .text_color(move || palette(dark_mode.get()).text_primary)
                                  .bg(move || palette(dark_mode.get()).surface_base)
                                  .value(move || st_val.focused_text.get())
                                  .on_change(move |v: &str| {
                                    let composing =
                                      !st_ime.composing_text.get_untracked().is_empty();
                                    if !composing && v.starts_with('/') {
                                      st_ime.slash_open.set(true);
                                      st_ime.slash_query.set(v.to_owned());
                                    } else if st_ime.slash_open.get_untracked()
                                      && !v.starts_with('/')
                                    {
                                      st_ime.slash_open.set(false);
                                    }
                                    let end = utf16_len(v);
                                    st_ime.dispatch(EditorCommand::ImeUpdate {
                                      text: v.to_owned(),
                                      base: end,
                                      extent: end,
                                      composing_valid: composing,
                                      composing_collapsed: !composing,
                                      shift_pressed: false,
                                    });
                                  })
                                  .on_composition_change(move |c: &Composition| {
                                    if c.composing {
                                      st_comp.composing_text.set(c.text.clone());
                                    } else {
                                      st_comp.composing_text.set(String::new());
                                      st_comp.dispatch(EditorCommand::CommitComposition);
                                    }
                                  })
                                  .on_submit(move |_: &str| {
                                    st_sub.dispatch(EditorCommand::SplitBlock);
                                  }),
                              )
                          }))
                          .child(when(move || !is_focused.get(), move || {
                            let st_click_outer = st_p_nf.clone();
                            dynamic(move || {
                              let disp = block_disp.get().unwrap_or_else(empty_para_disp);
                              let p = palette(dark_mode.get());
                              let (fsize, fweight, top_pad) = block_typography(&disp);
                              let left_indent = (disp.indent as f32) * 20.0;
                              let st_click = st_click_outer.clone();
                              let st_td = st_click_outer.clone();

                              let mut row = view()
                                .w_full()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .pt(px(top_pad))
                                .pl(px(left_indent));

                              if disp.kind == "quote" {
                                row = row
                                  .pl(px(left_indent + 12.0))
                                  .border_l(px(3.0))
                                  .border_l_color(p.quote_bar);
                              }
                              if disp.kind == "bulleted_list" {
                                row = row.child(
                                  text("•").text_size(px(16.0)).text_color(p.text_muted),
                                );
                              } else if disp.kind == "numbered_list" {
                                row = row.child(
                                  text("1.").text_size(px(15.0)).text_color(p.text_muted),
                                );
                              } else if disp.kind == "todo" {
                                let glyph = if disp.checked { "☑" } else { "☐" };
                                row = row.child(
                                  button()
                                    .on_press(move || {
                                      st_td.dispatch(EditorCommand::ToggleTodo {
                                        block_index: at.index(),
                                      });
                                    })
                                    .child(
                                      text(glyph).text_size(px(16.0)).text_color(p.text_muted),
                                    ),
                                );
                              }

                              let display_str = if disp.text.is_empty() {
                                "/".to_string()
                              } else {
                                disp.text.clone()
                              };
                              let text_col = if disp.text.is_empty() {
                                p.text_ghost
                              } else if disp.kind == "quote" {
                                p.text_muted
                              } else {
                                p.text_primary
                              };

                              row.child(
                                button()
                                  .flex_1()
                                  .flex_row()
                                  .items_center()
                                  .on_press(move || st_click.select_block(at.index()))
                                  .child(
                                    text(display_str)
                                      .flex_1()
                                      .text_size(px(fsize))
                                      .font_weight(fweight)
                                      .text_color(text_col),
                                  ),
                              )
                            })
                          }))
                      }
                    }))
                },
              )
              .keys(move || {
                st_keys.structure_rev.get();
                st_keys.block_ids.borrow().clone()
              })
              .estimated_item_size(px(34.0))
              .flex_1(),
            ),
        ),
    )
    .child(
      // 3. Quiet bottom status bar with right-aligned word/char count pill ("9 词 · 14 字")
      view()
        .h(px(32.0))
        .px_4()
        .flex_row()
        .items_center()
        .justify_end()
        .bg(move || palette(dark_mode.get()).surface_base)
        .child(
          view()
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .bg(move || palette(dark_mode.get()).surface_sunken)
            .child(
              text(move || st_words.word_count_text.get())
                .text_size(px(12.0))
                .text_color(move || palette(dark_mode.get()).text_faint),
            ),
        ),
    )
}

fn heading_font_size(kind: &str, level: u64) -> f32 {
  if kind == "heading" {
    match level {
      1 => 30.0,
      2 => 24.0,
      3 => 20.0,
      4 => 18.0,
      _ => 16.0,
    }
  } else {
    16.0
  }
}

fn block_typography(b: &BlockDisplay) -> (f32, FontWeight, f32) {
  let has_bold = b.marks.iter().any(|m| m.kind == "bold");
  match b.kind.as_str() {
    "heading" => match b.level {
      1 => (30.0, FontWeight::SEMIBOLD, 14.0),
      2 => (24.0, FontWeight::SEMIBOLD, 10.0),
      3 => (20.0, FontWeight::SEMIBOLD, 8.0),
      4 => (18.0, FontWeight::SEMIBOLD, 6.0),
      _ => (16.0, FontWeight::SEMIBOLD, 4.0),
    },
    _ => (
      16.0,
      if has_bold {
        FontWeight::BOLD
      } else {
        FontWeight::NORMAL
      },
      2.0,
    ),
  }
}

fn compute_word_count_label(blocks: &[EditorBlock]) -> String {
  let mut chars = 0usize;
  let mut words = 0usize;
  for b in blocks {
    let t = b.text.trim();
    if t.is_empty() {
      continue;
    }
    chars += t.chars().filter(|c| !c.is_whitespace()).count();
    let mut in_ascii_word = false;
    for ch in t.chars() {
      if ch.is_ascii_alphanumeric() {
        if !in_ascii_word {
          words += 1;
          in_ascii_word = true;
        }
      } else {
        in_ascii_word = false;
        if !ch.is_whitespace() && !ch.is_ascii_punctuation() {
          words += 1;
        }
      }
    }
  }
  format!("{words} 词 · {chars} 字")
}

fn icon_btn<F, I>(
  tid: &'static str,
  dark_mode: Signal<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  button()
    .test_id(tid)
    .w(px(28.0))
    .h(px(28.0))
    .items_center()
    .justify_center()
    .rounded(px(6.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(dynamic(move || icon_fn(palette(dark_mode.get()).text_muted_hex)))
}

fn toolbar_sep(dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w(px(1.0))
    .h(px(16.0))
    .mx_1()
    .bg(move || palette(dark_mode.get()).border_normal)
}

fn empty_para_disp() -> BlockDisplay {
  BlockDisplay {
    id: String::new(),
    kind: "paragraph".into(),
    text: String::new(),
    marks: Vec::new(),
    data: serde_json::json!({}),
    indent: 0,
    checked: false,
    level: 1,
  }
}

fn empty_table_disp() -> BlockDisplay {
  BlockDisplay {
    id: String::new(),
    kind: "table".into(),
    text: String::new(),
    marks: Vec::new(),
    data: serde_json::json!({}),
    indent: 0,
    checked: false,
    level: 1,
  }
}

fn empty_divider_disp() -> BlockDisplay {
  BlockDisplay {
    id: String::new(),
    kind: "divider".into(),
    text: String::new(),
    marks: Vec::new(),
    data: serde_json::json!({}),
    indent: 0,
    checked: false,
    level: 1,
  }
}
