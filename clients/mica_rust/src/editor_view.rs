//! Route 2-B pure-Rust block editor view backed directly by `mica_editor::EditorSession`.
//!
//! Architecture invariants (verified in Phase 0):
//! - Single document authority: edits go straight to `EditorSession` (`MicaDoc`).
//! - `virtual_list` `count` and `keys` subscribe ONLY to `structure_rev`, while
//!   per-block text/kind updates flow through per-block fine-grained signals.
//! - OS-native Chinese IME runs through a native `input()` proxy with
//!   `on_composition_change` preedit tracking and caret-accurate submission.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_editor::{
  DocPosition, DocSelection, EditorBlock, EditorCommand, EditorSession, NormalizedSelection,
  utf16_len,
};
use nsh_base::{button, input, theme};
use nsh_core::{element::Position, geometry::px, prelude::*};

#[derive(Clone, PartialEq)]
struct BlockDisplay {
  kind: String,
  text: String,
  indent: u32,
  checked: bool,
  level: u64,
}

impl From<&EditorBlock> for BlockDisplay {
  fn from(b: &EditorBlock) -> Self {
    Self {
      kind: b.kind.clone(),
      text: b.text.clone(),
      indent: b.indent(),
      checked: b.data.get("checked").and_then(|v| v.as_bool()).unwrap_or(false),
      level: b.data.get("level").and_then(|v| v.as_u64()).unwrap_or(1),
    }
  }
}

/// Reactive mirror over `EditorSession` implementing per-block signal granularity.
pub struct EditorViewState {
  session: Rc<RefCell<EditorSession>>,
  block_ids: Rc<RefCell<Vec<String>>>,
  block_signals: Rc<RefCell<HashMap<String, Signal<BlockDisplay>>>>,
  structure_rev: Signal<u64>,
  focused_idx: Signal<usize>,
  focused_text: Signal<String>,
  composing_text: Signal<String>,
  status_text: Signal<String>,
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
      structure_rev: signal(0),
      focused_idx: signal(0),
      focused_text: signal(first_text),
      composing_text: signal(String::new()),
      status_text: signal(String::from("Ready · Single-source MicaDoc CRDT")),
      can_undo: signal(can_u),
      can_redo: signal(can_r),
    })
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
    let content_changed = self.session.borrow_mut().dispatch(cmd).content_changed;
    self.sync_from_session(content_changed);
  }

  fn sync_from_session(&self, content_changed: bool) {
    // Snapshot everything from `session` and release the RefCell borrow before
    // firing any reactive signals (which synchronously re-run subscribers).
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
      // Fine-grained O(1) update: only touch signals whose block actually changed.
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
}

pub fn editor_view(state: Rc<EditorViewState>) -> impl IntoElement {
  state.rebuild_block_signals();
  let st_h1 = state.clone();
  let st_h2 = state.clone();
  let st_bold = state.clone();
  let st_list = state.clone();
  let st_todo = state.clone();
  let st_div = state.clone();
  let st_undo = state.clone();
  let st_redo = state.clone();
  let st_ime = state.clone();
  let st_submit = state.clone();
  let st_comp = state.clone();
  let st_count = state.clone();
  let st_keys = state.clone();
  let st_row = state.clone();
  let st_status = state.clone();
  let st_focus_val = state.clone();

  view()
    .flex_1()
    .bg(move || theme().background)
    .child(
      // Formatting & command bar wired directly to `EditorSession`
      view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
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
          let idx = st_bold.focused_idx.get_untracked();
          let len = st_bold
            .session
            .borrow()
            .blocks()
            .get(idx)
            .map(|b| utf16_len(&b.text))
            .unwrap_or(0);
          if len > 0 {
            st_bold.dispatch(EditorCommand::SetSelection {
              selection: Some(DocSelection::from_positions(
                DocPosition::new(idx, 0),
                DocPosition::new(idx, len),
              )),
            });
            st_bold.dispatch(EditorCommand::ToggleMark {
              kind: "bold".into(),
              href: None,
            });
          }
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
            .placeholder("输入正文或 Markdown 快捷键（如 ## 标题、- 列表、--- 分隔线）...")
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
          let is_focused = computed(move || st_sel.focused_idx.get() == at.index());
          let block_disp = computed(move || {
            let idx = at.index();
            let id = st_cell.block_ids.borrow().get(idx).cloned();
            id.and_then(|bid| st_cell.block_signals.borrow().get(&bid).copied())
              .map(|sig| sig.get())
          });

          button()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .bg(move || {
              if is_focused.get() {
                theme().accent
              } else {
                theme().background
              }
            })
            .on_press(move || st_press.select_block(at.index()))
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
        },
      )
      .keys(move || {
        st_keys.structure_rev.get();
        st_keys.block_ids.borrow().clone()
      })
      .estimated_item_size(px(36.0))
      .flex_1(),
    )
}

fn format_block_line(b: &BlockDisplay) -> String {
  let pad = "  ".repeat(b.indent as usize);
  match b.kind.as_str() {
    "heading" => format!("{pad}[H{}] {}", b.level, b.text),
    "bulleted_list" => format!("{pad}• {}", b.text),
    "numbered_list" => format!("{pad}1. {}", b.text),
    "todo" => {
      let box_ch = if b.checked { "☑" } else { "☐" };
      format!("{pad}{box_ch} {}", b.text)
    }
    "quote" => format!("{pad}▎ {}", b.text),
    "code_block" => format!("{pad}``` {}", b.text),
    "divider" => "────────────────────────".to_string(),
    "table" => "[Table]".to_string(),
    "image" => "[Image]".to_string(),
    _ => format!("{pad}{}", b.text),
  }
}

fn cmd_btn(
  label: &'static str,
  test_id: &'static str,
  on_click: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(test_id)
    .px_2()
    .py_1()
    .rounded(move || theme().radius)
    .bg(move || theme().secondary)
    .on_press(on_click)
    .child(
      text(label)
        .text_xs()
        .text_color(move || theme().secondary_foreground),
    )
}
