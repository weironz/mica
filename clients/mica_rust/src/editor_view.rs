//! Route 2-B pure-Rust block editor view backed directly by `mica_editor::EditorSession`.
//!
//! Architecture invariants:
//! - Single document authority: edits go straight to `EditorSession` (`MicaDoc`).
//! - `virtual_list` `count` and `keys` subscribe ONLY to `structure_rev`, while
//!   per-block text/kind/marks updates flow through per-block fine-grained signals.
//! - OS-native Chinese IME runs through a native `input()` proxy with
//!   `on_composition_change` preedit tracking and caret-accurate submission.
//! - Specialized blocks dispatch to `atomic_blocks` (`AtomicBlockRenderer` registry)
//!   and `editor_table` (`TableModel` + `TableOp`), with `/` slash palette support.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_editor::{
  DocPosition, DocSelection, EditorBlock, EditorCommand, EditorMark, EditorSession,
  NormalizedSelection, utf16_len,
};
use nsh_base::{button, input, theme};
use nsh_core::{element::Position, geometry::px, prelude::*};

use crate::{
  atomic_blocks::{AtomicBlockInput, is_atomic_or_rich_block, render_atomic_block},
  editor_slash::{command_for_slash, slash_menu_view},
  editor_table::table_block_view,
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
  status_text: Signal<String>,
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
      status_text: signal(String::from("Ready · Single-source MicaDoc CRDT")),
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
    let can_u = next.can_undo();
    let can_r = next.can_redo();
    let status = format!(
      "Blocks: {} | Rev: {} | Undo: {} | Redo: {}",
      next.blocks().len(),
      next.content_revision(),
      can_u,
      can_r
    );
    *self.session.borrow_mut() = next;
    *self.block_ids.borrow_mut() = ids;
    *self.block_signals.borrow_mut() = map;
    self.slash_open.set(false);
    self.slash_query.set(String::new());
    self.can_undo.set(can_u);
    self.can_redo.set(can_r);
    self.focused_idx.set(0);
    self.focused_text.set(first_text);
    self.status_text.set(status);
    self.structure_rev.set(self.structure_rev.get_untracked() + 1);
  }

  /// Re-creates per-block signals inside the currently mounting `editor_view`
  /// reactive owner so switching away and back via `when(...)` never reads a
  /// disposed signal.
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
      let persist_cb = self.on_persist.borrow().clone();
      if let Some(cb) = persist_cb {
        cb(self.export_markdown(), crdt_update);
      }
    }
  }

  fn sync_from_session(&self, content_changed: bool) {
    let (displays, next_idx, next_text, can_u, can_r, status) = {
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
      let status = format!(
        "Blocks: {} | Rev: {} | Undo: {} | Redo: {}",
        blocks.len(),
        session.content_revision(),
        can_u,
        can_r
      );
      (displays, next_idx, next_text, can_u, can_r, status)
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
    self.status_text.set(status);
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

pub fn editor_view(state: Rc<EditorViewState>, dark_mode: Signal<bool>) -> impl IntoElement {
  state.rebuild_block_signals();

  // Global keyboard shortcuts for the active editor
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

  let st_h1 = state.clone();
  let st_h2 = state.clone();
  let st_bold = state.clone();
  let st_italic = state.clone();
  let st_code = state.clone();
  let st_list = state.clone();
  let st_todo = state.clone();
  let st_quote = state.clone();
  let st_indent = state.clone();
  let st_outdent = state.clone();
  let st_slash_btn = state.clone();
  let st_div = state.clone();
  let st_undo = state.clone();
  let st_redo = state.clone();
  let st_status = state.clone();
  let st_ime = state.clone();
  let st_comp = state.clone();
  let st_submit = state.clone();
  let st_focus_val = state.clone();
  let st_slash_vis = state.clone();
  let st_slash_pick = state.clone();
  let st_count = state.clone();
  let st_keys = state.clone();
  let st_row = state.clone();

  view()
    .flex_1()
    .bg(move || theme().background)
    .child(
      // Formatting & structure toolbar
      view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(cmd_btn("H1", "ed-h1", move || {
          st_h1.dispatch(EditorCommand::TransformBlock {
            kind: "heading".into(),
            data: Some(serde_json::json!({ "level": 1 })),
          });
        }))
        .child(cmd_btn("H2", "ed-h2", move || {
          st_h2.dispatch(EditorCommand::TransformBlock {
            kind: "heading".into(),
            data: Some(serde_json::json!({ "level": 2 })),
          });
        }))
        .child(cmd_btn("Bold", "ed-bold", move || {
          st_bold.toggle_mark_on_focused("bold");
        }))
        .child(cmd_btn("Italic", "ed-italic", move || {
          st_italic.toggle_mark_on_focused("italic");
        }))
        .child(cmd_btn("Code", "ed-code", move || {
          st_code.toggle_mark_on_focused("code");
        }))
        .child(cmd_btn("List", "ed-list", move || {
          st_list.dispatch(EditorCommand::TransformBlock {
            kind: "bulleted_list".into(),
            data: None,
          });
        }))
        .child(cmd_btn("Todo", "ed-todo", move || {
          st_todo.dispatch(EditorCommand::TransformBlock {
            kind: "todo".into(),
            data: Some(serde_json::json!({ "checked": false })),
          });
        }))
        .child(cmd_btn("Quote", "ed-quote", move || {
          st_quote.dispatch(EditorCommand::TransformBlock {
            kind: "quote".into(),
            data: None,
          });
        }))
        .child(cmd_btn("→", "ed-indent", move || {
          st_indent.dispatch(EditorCommand::Indent { delta: 1 });
        }))
        .child(cmd_btn("←", "ed-outdent", move || {
          st_outdent.dispatch(EditorCommand::Indent { delta: -1 });
        }))
        .child(cmd_btn("+ /", "ed-slash-open", move || {
          st_slash_btn.slash_query.set(String::new());
          st_slash_btn.slash_open.set(!st_slash_btn.slash_open.get_untracked());
        }))
        .child(cmd_btn("Divider", "ed-divider", move || {
          st_div.dispatch(EditorCommand::InsertDivider);
        }))
        .child(cmd_btn("Undo", "ed-undo", move || {
          st_undo.dispatch(EditorCommand::Undo);
        }))
        .child(cmd_btn("Redo", "ed-redo", move || {
          st_redo.dispatch(EditorCommand::Redo);
        }))
        .child(
          text(move || {
            let comp = st_status.composing_text.get();
            if comp.is_empty() {
              st_status.status_text.get()
            } else {
              format!("IME 组字: [{comp}]")
            }
          })
          .text_xs()
          .text_color(move || theme().muted_foreground),
        ),
    )
    .child(
      // Focused block OS-native IME input proxy
      view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .px_4()
        .py_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child({
          let st_idx = st_focus_val.clone();
          text(move || format!("Block #{}", st_idx.focused_idx.get() + 1))
            .text_xs()
            .text_color(move || theme().muted_foreground)
        })
        .child(
          input()
            .test_id("editor-ime-input")
            .placeholder("输入正文、Markdown 快捷键（## / - / ---）或 / 打开斜杠命令...")
            .flex_1()
            .px_3()
            .py_2()
            .border_1()
            .border_color(move || theme().primary)
            .rounded(move || theme().radius)
            .text_color(move || theme().foreground)
            .value(move || st_focus_val.focused_text.get())
            .on_change(move |v: &str| {
              let composing = !st_ime.composing_text.get_untracked().is_empty();
              if !composing && v.starts_with('/') {
                st_ime.slash_open.set(true);
                st_ime.slash_query.set(v.to_owned());
              } else if st_ime.slash_open.get_untracked() && !v.starts_with('/') {
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
              st_submit.dispatch(EditorCommand::SplitBlock);
            }),
        ),
    )
    .child(when(move || st_slash_vis.slash_open.get(), move || {
      let q = st_slash_pick.slash_query.get();
      let st_p = st_slash_pick.clone();
      let st_c = st_slash_pick.clone();
      slash_menu_view(
        q,
        Rc::new(move |kind: &'static str| {
          // Clear leading `/query` text before applying block transform.
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
      )
    }))
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
          let st_todo_click = st_row.clone();
          let st_tbl = st_row.clone();

          let is_focused = computed(move || st_sel.focused_idx.get() == at.index());
          let block_disp = computed(move || {
            let idx = at.index();
            let id = st_cell.block_ids.borrow().get(idx).cloned();
            id.and_then(|bid| st_cell.block_signals.borrow().get(&bid).copied())
              .map(|sig| sig.get())
          });

          let is_table = computed(move || {
            block_disp
              .get()
              .is_some_and(|b| b.kind == "table")
          });
          let is_atomic = computed(move || {
            block_disp
              .get()
              .is_some_and(|b| b.kind != "table" && is_atomic_or_rich_block(&b.kind))
          });

          view()
            .w_full()
            .px_4()
            .py_1()
            .bg(move || {
              if is_focused.get() {
                theme().accent
              } else {
                theme().background
              }
            })
            .child(when(move || is_table.get(), move || {
              let st_d = st_tbl.clone();
              dynamic(move || {
                let disp = block_disp.get().unwrap_or(BlockDisplay {
                  id: String::new(),
                  kind: "table".into(),
                  text: String::new(),
                  marks: Vec::new(),
                  data: serde_json::json!({}),
                  indent: 0,
                  checked: false,
                  level: 1,
                });
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
                  let disp = block_disp.get().unwrap_or(BlockDisplay {
                    id: String::new(),
                    kind: "divider".into(),
                    text: String::new(),
                    marks: Vec::new(),
                    data: serde_json::json!({}),
                    indent: 0,
                    checked: false,
                    level: 1,
                  });
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
            .child(when(move || !is_table.get() && !is_atomic.get(), move || {
              let st_p = st_press.clone();
              let st_td = st_todo_click.clone();
              button()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .py_1()
                .on_press(move || {
                  let is_todo_blk = block_disp
                    .get()
                    .is_some_and(|b| b.kind == "todo");
                  if is_todo_blk && st_p.focused_idx.get_untracked() == at.index() {
                    st_td.dispatch(EditorCommand::ToggleTodo {
                      block_index: at.index(),
                    });
                  } else {
                    st_p.select_block(at.index());
                  }
                })
                .child(
                  text(move || {
                    block_disp
                      .get()
                      .map(|b| format_block_line(&b))
                      .unwrap_or_default()
                  })
                  .flex_1()
                  .text_color(move || theme().foreground),
                )
            }))
        },
      )
      .keys(move || {
        st_keys.structure_rev.get();
        st_keys.block_ids.borrow().clone()
      })
      .estimated_item_size(px(38.0))
      .flex_1(),
    )
}

fn format_block_line(b: &BlockDisplay) -> String {
  let pad = "  ".repeat(b.indent as usize);
  let body = format_marked_text(&b.text, &b.marks);
  match b.kind.as_str() {
    "heading" => format!("{pad}[H{}] {body}", b.level),
    "bulleted_list" => format!("{pad}• {body}"),
    "numbered_list" => format!("{pad}1. {body}"),
    "todo" => {
      let box_ch = if b.checked { "☑" } else { "☐" };
      format!("{pad}{box_ch} {body}")
    }
    "quote" => format!("{pad}▎ {body}"),
    _ => format!("{pad}{body}"),
  }
}

/// Renders UTF-16 inline marks clearly in block line summaries while preserving
/// exact plain text when no marks are active.
fn format_marked_text(text: &str, marks: &[EditorMark]) -> String {
  if marks.is_empty() || text.is_empty() {
    return text.to_string();
  }
  let mut badges = Vec::new();
  for m in marks {
    let tag = match m.kind.as_str() {
      "bold" => "B",
      "italic" => "I",
      "code" => "`code`",
      "strike" => "S",
      "link" => "link",
      other => other,
    };
    if !badges.contains(&tag) {
      badges.push(tag);
    }
  }
  format!("{text}  [{}]", badges.join("·"))
}

fn cmd_btn(label: &'static str, tid: &'static str, on_click: impl Fn() + 'static) -> impl IntoElement {
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
      text(label)
        .text_xs()
        .text_color(move || theme().secondary_foreground),
    )
}
