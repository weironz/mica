//! Route 2-B pure-Rust block editor view styled 1:1 to Mica's warm-paper Notion aesthetic:
//! - Strict single-row 40px page header: breadcrumb + copy-link (left) · borderless minimal
//!   icon formatting toolbar with live active states (center) · last-edited, right-sidebar
//!   toggle, whiteboard, favorite star, trash, and page menu (right).
//! - Centered 740px white paper canvas with native cross-block `.selectable(true)` mouse drag
//!   selection, inline rich text spans (bold/italic/strike/code/link), large borderless 30px
//!   editable title, true typographic scale, seamless in-place OS-native IME editing with
//!   `SubmitBehavior::Submit` + `auto_focus(true)` (Enter splits & stays focused, Backspace
//!   merges/unformats, ↑/↓ navigate blocks, Ctrl+A two-stage block/document select-all,
//!   Ctrl+Z/Y/B/I/E shortcuts), and click-below-content paper tail.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_editor::{
  DocPosition, DocSelection, EditorBlock, EditorCommand, EditorMark, EditorSession,
  NormalizedSelection, utf16_len,
};
use nsh_base::{button, input};
use nsh_core::{
  SubmitBehavior,
  element::{KeyOptions, Position, Selection, TextSelectionEvent, on_key_with, set_clipboard},
  geometry::px,
  keymap::When,
  prelude::*,
};

use crate::{
  atomic_blocks::{AtomicBlockInput, is_atomic_or_rich_block, render_atomic_block},
  editor_slash::{command_for_slash, slash_menu_view},
  editor_table::table_block_view,
  mica_icons,
  mica_theme::{MicaPalette, palette},
};

/// One heading item exported to the right-hand Page Outline sidebar.
#[derive(Clone, PartialEq, Eq)]
pub struct OutlineItem {
  pub block_index: usize,
  pub level: u64,
  pub text: String,
}

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
  /// Live `(start, end)` UTF-16 caret/selection inside the focused native `input()`.
  input_sel: Signal<(u32, u32)>,
  /// Controlled selection range pushed to the focused `input()` when commands/Ctrl+A move it.
  controlled_sel: Signal<(u32, u32)>,
  /// True when the user has selected the entire document via two-stage `Ctrl+A`.
  all_selected: Signal<bool>,
  /// True while the user is drag-selecting text across the `.selectable(true)` reading canvas.
  canvas_selected_text: Signal<String>,
  composing_text: Signal<String>,
  word_count_text: Signal<String>,
  block_count: Signal<usize>,
  outline_items: Signal<Vec<OutlineItem>>,
  last_edited_text: Signal<String>,
  active_kind: Signal<String>,
  active_level: Signal<u64>,
  active_marks: Signal<Vec<String>>,
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
    let first = session.blocks().first();
    let first_text = first.map(|b| b.text.clone()).unwrap_or_default();
    let first_end = utf16_len(&first_text);
    let first_kind = first
      .map(|b| b.kind.clone())
      .unwrap_or_else(|| "paragraph".into());
    let first_level = first
      .and_then(|b| b.data.get("level").and_then(|v| v.as_u64()))
      .unwrap_or(1);
    let first_marks: Vec<String> = first
      .map(|b| b.marks().into_iter().map(|m| m.kind).collect())
      .unwrap_or_default();
    let counts = compute_word_count_label(session.blocks());
    let outline = compute_outline(session.blocks());
    let blk_count = session.blocks().len();
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
      input_sel: signal((first_end, first_end)),
      controlled_sel: signal((first_end, first_end)),
      all_selected: signal(false),
      canvas_selected_text: signal(String::new()),
      composing_text: signal(String::new()),
      word_count_text: signal(counts),
      block_count: signal(blk_count),
      outline_items: signal(outline),
      last_edited_text: signal(String::from("上次编辑 刚刚")),
      active_kind: signal(first_kind),
      active_level: signal(first_level),
      active_marks: signal(first_marks),
      slash_open: signal(false),
      slash_query: signal(String::new()),
      can_undo: signal(can_u),
      can_redo: signal(can_r),
    })
  }

  pub fn word_count_signal(&self) -> Signal<String> {
    self.word_count_text
  }

  pub fn block_count_signal(&self) -> Signal<usize> {
    self.block_count
  }

  pub fn outline_signal(&self) -> Signal<Vec<OutlineItem>> {
    self.outline_items
  }

  pub fn set_on_persist(&self, cb: Rc<dyn Fn(String, Option<Vec<u8>>)>) {
    *self.on_persist.borrow_mut() = Some(cb);
  }

  pub fn set_status_flash(&self, msg: impl Into<String>) {
    self.last_edited_text.set(msg.into());
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
    let first = next.blocks().first();
    let first_text = first.map(|b| b.text.clone()).unwrap_or_default();
    let first_end = utf16_len(&first_text);
    let first_kind = first
      .map(|b| b.kind.clone())
      .unwrap_or_else(|| "paragraph".into());
    let first_level = first
      .and_then(|b| b.data.get("level").and_then(|v| v.as_u64()))
      .unwrap_or(1);
    let first_marks: Vec<String> = first
      .map(|b| b.marks().into_iter().map(|m| m.kind).collect())
      .unwrap_or_default();
    let counts = compute_word_count_label(next.blocks());
    let outline = compute_outline(next.blocks());
    let blk_count = next.blocks().len();
    let can_u = next.can_undo();
    let can_r = next.can_redo();
    *self.session.borrow_mut() = next;
    *self.block_ids.borrow_mut() = ids;
    *self.block_signals.borrow_mut() = map;
    self.slash_open.set(false);
    self.slash_query.set(String::new());
    self.all_selected.set(false);
    self.canvas_selected_text.set(String::new());
    self.can_undo.set(can_u);
    self.can_redo.set(can_r);
    self.focused_idx.set(0);
    self.focused_text.set(first_text);
    self.input_sel.set((first_end, first_end));
    self.controlled_sel.set((first_end, first_end));
    self.active_kind.set(first_kind);
    self.active_level.set(first_level);
    self.active_marks.set(first_marks);
    self.word_count_text.set(counts);
    self.block_count.set(blk_count);
    self.outline_items.set(outline);
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
    let is_selection_only = matches!(cmd, EditorCommand::SetSelection { .. });
    if !is_selection_only {
      self.all_selected.set(false);
    }
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
    let (
      displays,
      next_idx,
      next_text,
      next_sel_u16,
      next_kind,
      next_level,
      next_marks,
      can_u,
      can_r,
      counts,
      outline,
      blk_count,
    ) = {
      let session = self.session.borrow();
      let blocks = session.blocks();
      let displays: Vec<(String, BlockDisplay)> = blocks
        .iter()
        .map(|b| (b.id.clone(), BlockDisplay::from(b)))
        .collect();
      let (next_idx, next_sel_u16) = session
        .selection()
        .map(|sel| match sel.normalized() {
          NormalizedSelection::Text { start, end } => {
            if start.node == end.node {
              (end.node, (start.offset, end.offset))
            } else {
              (end.node, (end.offset, end.offset))
            }
          }
          NormalizedSelection::BlockAtom { block_index }
          | NormalizedSelection::TableArea { block_index, .. } => (block_index, (0, 0)),
        })
        .unwrap_or((0, (0, 0)));
      let clamped_idx = next_idx.min(blocks.len().saturating_sub(1));
      let blk = blocks.get(clamped_idx);
      let next_text = blk.map(|b| b.text.clone()).unwrap_or_default();
      let max_u16 = utf16_len(&next_text);
      let sel_clamped = (next_sel_u16.0.min(max_u16), next_sel_u16.1.min(max_u16));
      let next_kind = blk
        .map(|b| b.kind.clone())
        .unwrap_or_else(|| "paragraph".into());
      let next_level = blk
        .and_then(|b| b.data.get("level").and_then(|v| v.as_u64()))
        .unwrap_or(1);
      let next_marks: Vec<String> = blk
        .map(|b| b.marks().into_iter().map(|m| m.kind).collect())
        .unwrap_or_default();
      let can_u = session.can_undo();
      let can_r = session.can_redo();
      let counts = compute_word_count_label(blocks);
      let outline = compute_outline(blocks);
      let blk_count = blocks.len();
      (
        displays,
        clamped_idx,
        next_text,
        sel_clamped,
        next_kind,
        next_level,
        next_marks,
        can_u,
        can_r,
        counts,
        outline,
        blk_count,
      )
    };

    let new_ids: Vec<String> = displays.iter().map(|(id, _)| id.clone()).collect();
    let structure_changed = *self.block_ids.borrow() != new_ids;
    let prev_focused = self.focused_idx.get_untracked();

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
    if structure_changed || prev_focused != next_idx {
      self.input_sel.set(next_sel_u16);
      self.controlled_sel.set(next_sel_u16);
    }
    if self.active_kind.get_untracked() != next_kind {
      self.active_kind.set(next_kind);
    }
    if self.active_level.get_untracked() != next_level {
      self.active_level.set(next_level);
    }
    if self.active_marks.get_untracked() != next_marks {
      self.active_marks.set(next_marks);
    }
    self.can_undo.set(can_u);
    self.can_redo.set(can_r);
    self.word_count_text.set(counts);
    self.block_count.set(blk_count);
    if self.outline_items.get_untracked() != outline {
      self.outline_items.set(outline);
    }
  }

  pub fn select_block(&self, idx: usize) {
    let clamped = {
      let len = self.session.borrow().blocks().len();
      if len == 0 {
        return;
      }
      idx.min(len - 1)
    };
    let end_u16 = self
      .session
      .borrow()
      .blocks()
      .get(clamped)
      .map(|b| utf16_len(&b.text))
      .unwrap_or(0);
    self.all_selected.set(false);
    self.dispatch(EditorCommand::SetSelection {
      selection: Some(DocSelection::caret(clamped, end_u16)),
    });
    self.input_sel.set((end_u16, end_u16));
    self.controlled_sel.set((end_u16, end_u16));
  }

  /// Clicking the blank paper space below the last block focuses the last empty paragraph,
  /// or appends a fresh empty paragraph after non-empty / table / atomic blocks (Notion behavior).
  pub fn focus_or_append_tail_block(&self) {
    let (last_idx, needs_split) = {
      let session = self.session.borrow();
      let blocks = session.blocks();
      if blocks.is_empty() {
        (0, false)
      } else {
        let last_idx = blocks.len() - 1;
        let last = &blocks[last_idx];
        let needs = last.kind != "paragraph" || !last.text.is_empty();
        (last_idx, needs)
      }
    };
    self.select_block(last_idx);
    if needs_split {
      self.dispatch(EditorCommand::SplitBlock);
    }
  }

  /// Sync live caret/selection from native `input()` into `EditorSession` before splitting.
  pub fn split_focused_at_caret(&self) {
    let idx = self.focused_idx.get_untracked();
    let (s, e) = self.input_sel.get_untracked();
    self.dispatch(EditorCommand::SetSelection {
      selection: Some(DocSelection::from_positions(
        DocPosition::new(idx, s),
        DocPosition::new(idx, e),
      )),
    });
    self.dispatch(EditorCommand::SplitBlock);
  }

  /// Handle Backspace at column 0: first unformat heading/list/todo/quote to paragraph,
  /// otherwise merge into previous block via `EditorCommand::DeleteBackward`.
  pub fn backspace_at_start_if_caret_zero(&self) {
    let (s, e) = self.input_sel.get_untracked();
    if s != 0 || e != 0 {
      return;
    }
    let idx = self.focused_idx.get_untracked();
    let kind = self.active_kind.get_untracked();
    if matches!(
      kind.as_str(),
      "heading" | "bulleted_list" | "numbered_list" | "todo" | "quote"
    ) {
      self.dispatch(EditorCommand::TransformBlock {
        kind: "paragraph".into(),
        data: None,
      });
    } else if idx > 0 {
      self.dispatch(EditorCommand::SetSelection {
        selection: Some(DocSelection::caret(idx, 0)),
      });
      self.dispatch(EditorCommand::DeleteBackward);
    }
  }

  /// Two-stage Notion `Ctrl+A`:
  /// 1) First press selects the entire focused block (when non-empty & not yet fully selected).
  /// 2) Second press (or empty block / outside input) selects the ENTIRE document!
  pub fn handle_select_all(&self) {
    let idx = self.focused_idx.get_untracked();
    let (block_count, cur_len, last_len) = {
      let s = self.session.borrow();
      let blocks = s.blocks();
      let cur_len = blocks.get(idx).map(|b| utf16_len(&b.text)).unwrap_or(0);
      let last_len = blocks.last().map(|b| utf16_len(&b.text)).unwrap_or(0);
      (blocks.len(), cur_len, last_len)
    };
    if block_count == 0 {
      return;
    }
    let (sel_s, sel_e) = self.input_sel.get_untracked();
    let block_already_all = cur_len == 0 || (sel_s == 0 && sel_e >= cur_len);
    if !self.all_selected.get_untracked() && !block_already_all && cur_len > 0 {
      self.input_sel.set((0, cur_len));
      self.controlled_sel.set((0, cur_len));
      self.dispatch(EditorCommand::SetSelection {
        selection: Some(DocSelection::from_positions(
          DocPosition::new(idx, 0),
          DocPosition::new(idx, cur_len),
        )),
      });
      return;
    }
    // Stage 2: Select the entire document across all blocks.
    self.dispatch(EditorCommand::SetSelection {
      selection: Some(DocSelection::from_positions(
        DocPosition::new(0, 0),
        DocPosition::new(block_count - 1, last_len),
      )),
    });
    self.all_selected.set(true);
    self.set_status_flash(format!("已全选整篇文档（{block_count} 块）"));
  }

  pub fn delete_all_if_all_selected(&self) -> bool {
    if !self.all_selected.get_untracked() {
      return false;
    }
    self.dispatch(EditorCommand::DeleteSelectedBlocks);
    self.all_selected.set(false);
    true
  }

  pub fn copy_all_if_all_selected(&self) -> bool {
    if !self.all_selected.get_untracked() {
      return false;
    }
    let md = self.export_markdown();
    set_clipboard(md);
    self.set_status_flash("已复制整篇文档");
    true
  }

  pub fn move_focus_relative(&self, delta: isize) {
    let len = self.session.borrow().blocks().len();
    if len == 0 {
      return;
    }
    let cur = self.focused_idx.get_untracked() as isize;
    let target = (cur + delta).clamp(0, (len as isize) - 1) as usize;
    if target != cur as usize {
      self.select_block(target);
    }
  }

  /// Toggles heading/list/todo/quote vs paragraph cleanly (clicking active H1 resets to paragraph).
  pub fn toggle_block_kind(&self, target_kind: &str, target_level: Option<u64>, data: Option<serde_json::Value>) {
    let cur_kind = self.active_kind.get_untracked();
    let cur_level = self.active_level.get_untracked();
    let already_same = cur_kind == target_kind
      && target_level.is_none_or(|lvl| cur_level == lvl);
    if already_same && target_kind != "paragraph" {
      self.dispatch(EditorCommand::TransformBlock {
        kind: "paragraph".into(),
        data: None,
      });
    } else {
      self.dispatch(EditorCommand::TransformBlock {
        kind: target_kind.into(),
        data,
      });
    }
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

  /// Toggle an inline mark on the user's dragged selection range `(s, e)` if non-empty,
  /// otherwise on the entire focused block so toolbar clicks always produce immediate feedback.
  pub fn toggle_mark_on_focused(&self, mark_kind: &str) {
    let idx = self.focused_idx.get_untracked();
    let len = self
      .session
      .borrow()
      .blocks()
      .get(idx)
      .map(|b| utf16_len(&b.text))
      .unwrap_or(0);
    if len == 0 {
      self.set_status_flash("请先输入文字后再应用行内格式");
      return;
    }
    let (s, e) = self.input_sel.get_untracked();
    let (start, end) = if s < e && e <= len {
      (s, e)
    } else {
      (0, len)
    };
    self.dispatch(EditorCommand::SetSelection {
      selection: Some(DocSelection::from_positions(
        DocPosition::new(idx, start),
        DocPosition::new(idx, end),
      )),
    });
    self.dispatch(EditorCommand::ToggleMark {
      kind: mark_kind.into(),
      href: None,
    });
  }
}

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
  state: Rc<EditorViewState>,
  header: Rc<EditorHeaderProps>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  state.rebuild_block_signals();
  let page_menu_open = signal(false);

  // Register real editor keyboard shortcuts (Ctrl+A / Ctrl+Z / Ctrl+Y / Ctrl+B / Ctrl+I / Ctrl+E / Ctrl+C / Backspace / Esc)
  {
    let st_a = state.clone();
    on_key_with(
      "mod+a",
      KeyOptions {
        when: When::Always,
        ..KeyOptions::default()
      },
      move |_| st_a.handle_select_all(),
    );
    let st_z = state.clone();
    on_key_with(
      "mod+z",
      KeyOptions {
        when: When::Always,
        ..KeyOptions::default()
      },
      move |_| st_z.dispatch(EditorCommand::Undo),
    );
    let st_y = state.clone();
    on_key_with(
      "mod+y",
      KeyOptions {
        when: When::Always,
        ..KeyOptions::default()
      },
      move |_| st_y.dispatch(EditorCommand::Redo),
    );
    let st_sz = state.clone();
    on_key_with(
      "shift+mod+z",
      KeyOptions {
        when: When::Always,
        ..KeyOptions::default()
      },
      move |_| st_sz.dispatch(EditorCommand::Redo),
    );
    let st_b = state.clone();
    on_key_with(
      "mod+b",
      KeyOptions {
        when: When::Always,
        ..KeyOptions::default()
      },
      move |_| st_b.toggle_mark_on_focused("bold"),
    );
    let st_i = state.clone();
    on_key_with(
      "mod+i",
      KeyOptions {
        when: When::Always,
        ..KeyOptions::default()
      },
      move |_| st_i.toggle_mark_on_focused("italic"),
    );
    let st_e = state.clone();
    on_key_with(
      "mod+e",
      KeyOptions {
        when: When::Always,
        ..KeyOptions::default()
      },
      move |_| st_e.toggle_mark_on_focused("code"),
    );
    let st_c = state.clone();
    on_key_with(
      "mod+c",
      KeyOptions {
        when: When::Always,
        pass_through: true,
        ..KeyOptions::default()
      },
      move |_| {
        let _ = st_c.copy_all_if_all_selected();
      },
    );
    let st_del = state.clone();
    on_key_with(
      "backspace",
      KeyOptions {
        when: When::Always,
        pass_through: true,
        ..KeyOptions::default()
      },
      move |_| {
        let _ = st_del.delete_all_if_all_selected();
      },
    );
    let st_esc = state.clone();
    on_key_with(
      "escape",
      KeyOptions {
        when: When::Always,
        pass_through: true,
        ..KeyOptions::default()
      },
      move |_| {
        st_esc.all_selected.set(false);
        st_esc.slash_open.set(false);
        page_menu_open.set(false);
      },
    );
  }

  let st_count = state.clone();
  let st_keys = state.clone();
  let st_row = state.clone();
  let st_undo = state.clone();
  let st_redo = state.clone();
  let st_para = state.clone();
  let st_h1 = state.clone();
  let st_h2 = state.clone();
  let st_h3 = state.clone();
  let st_bold = state.clone();
  let st_italic = state.clone();
  let st_strike = state.clone();
  let st_code = state.clone();
  let st_list = state.clone();
  let st_ordered = state.clone();
  let st_todo = state.clone();
  let st_quote = state.clone();
  let st_div = state.clone();
  let st_tbl_btn = state.clone();
  let st_slash_btn = state.clone();
  let st_slash_vis = state.clone();
  let st_slash_pick = state.clone();
  let st_words = state.clone();
  let st_edited = state.clone();
  let st_copy_path = state.clone();
  let st_copy_md = state.clone();
  let st_tail = state.clone();
  let st_canvas_sel = state.clone();
  let st_all_sel = state.clone();

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

  let active_kind = state.active_kind;
  let active_level = state.active_level;
  let active_marks = state.active_marks;
  let can_undo = state.can_undo;
  let can_redo = state.can_redo;
  let is_fav = header.is_favorite;
  let right_open = header.right_panel_open;

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
        .gap_2()
        .border_b_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .bg(move || palette(dark_mode.get()).surface_base)
        .child(
          // Left breadcrumb path + copy-link icon (compacts automatically when right sidebar is open)
          view()
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
                st_copy_path.set_status_flash("已复制页面链接");
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
              can_undo,
              move || st_undo.dispatch(EditorCommand::Undo),
              mica_icons::undo,
            ))
            .child(toolbar_btn_dim(
              "ed-redo",
              "重做 (Ctrl+Y)",
              dark_mode,
              can_redo,
              move || st_redo.dispatch(EditorCommand::Redo),
              mica_icons::redo,
            ))
            .child(toolbar_sep(dark_mode))
            .child(toolbar_btn_active(
              "ed-para",
              "正文段落",
              dark_mode,
              computed(move || active_kind.get() == "paragraph"),
              move || st_para.toggle_block_kind("paragraph", None, None),
              mica_icons::align_text,
            ))
            .child(toolbar_btn_active(
              "ed-h1",
              "一级标题",
              dark_mode,
              computed(move || active_kind.get() == "heading" && active_level.get() == 1),
              move || {
                st_h1.toggle_block_kind(
                  "heading",
                  Some(1),
                  Some(serde_json::json!({ "level": 1 })),
                );
              },
              |c| mica_icons::heading_box("1", c),
            ))
            .child(toolbar_btn_active(
              "ed-h2",
              "二级标题",
              dark_mode,
              computed(move || active_kind.get() == "heading" && active_level.get() == 2),
              move || {
                st_h2.toggle_block_kind(
                  "heading",
                  Some(2),
                  Some(serde_json::json!({ "level": 2 })),
                );
              },
              |c| mica_icons::heading_box("2", c),
            ))
            .child(toolbar_btn_active(
              "ed-h3",
              "三级标题",
              dark_mode,
              computed(move || active_kind.get() == "heading" && active_level.get() == 3),
              move || {
                st_h3.toggle_block_kind(
                  "heading",
                  Some(3),
                  Some(serde_json::json!({ "level": 3 })),
                );
              },
              |c| mica_icons::heading_box("3", c),
            ))
            .child(toolbar_sep(dark_mode))
            .child(toolbar_btn_active(
              "ed-bold",
              "粗体 (Ctrl+B)",
              dark_mode,
              computed(move || active_marks.get().iter().any(|m| m == "bold")),
              move || st_bold.toggle_mark_on_focused("bold"),
              mica_icons::bold_icon,
            ))
            .child(toolbar_btn_active(
              "ed-italic",
              "斜体 (Ctrl+I)",
              dark_mode,
              computed(move || active_marks.get().iter().any(|m| m == "italic")),
              move || st_italic.toggle_mark_on_focused("italic"),
              mica_icons::italic_icon,
            ))
            .child(toolbar_btn_active(
              "ed-strike",
              "删除线",
              dark_mode,
              computed(move || active_marks.get().iter().any(|m| m == "strike")),
              move || st_strike.toggle_mark_on_focused("strike"),
              mica_icons::strikethrough_icon,
            ))
            .child(toolbar_btn_active(
              "ed-code",
              "行内代码 (Ctrl+E)",
              dark_mode,
              computed(move || active_marks.get().iter().any(|m| m == "code")),
              move || st_code.toggle_mark_on_focused("code"),
              mica_icons::code_icon,
            ))
            .child(toolbar_sep(dark_mode))
            .child(toolbar_btn_active(
              "ed-list",
              "无序列表",
              dark_mode,
              computed(move || active_kind.get() == "bulleted_list"),
              move || st_list.toggle_block_kind("bulleted_list", None, None),
              mica_icons::list_bullet,
            ))
            .child(toolbar_btn_active(
              "ed-ordered",
              "有序列表",
              dark_mode,
              computed(move || active_kind.get() == "numbered_list"),
              move || st_ordered.toggle_block_kind("numbered_list", None, None),
              mica_icons::list_ordered,
            ))
            .child(toolbar_btn_active(
              "ed-todo",
              "待办清单",
              dark_mode,
              computed(move || active_kind.get() == "todo"),
              move || {
                st_todo.toggle_block_kind("todo", None, Some(serde_json::json!({ "checked": false })));
              },
              mica_icons::todo_box,
            ))
            .child(toolbar_btn_active(
              "ed-quote",
              "引用块",
              dark_mode,
              computed(move || active_kind.get() == "quote"),
              move || st_quote.toggle_block_kind("quote", None, None),
              mica_icons::quote_icon,
            ))
            .child(toolbar_sep(dark_mode))
            .child(toolbar_btn(
              "ed-divider",
              "插入分割线",
              dark_mode,
              signal(false),
              move || st_div.dispatch(EditorCommand::InsertDivider),
              mica_icons::divider_icon,
            ))
            .child(toolbar_btn(
              "ed-table-quick",
              "插入表格",
              dark_mode,
              signal(false),
              move || st_tbl_btn.dispatch(command_for_slash("table")),
              mica_icons::table_icon,
            ))
            .child(toolbar_btn(
              "ed-slash-open",
              "插入块菜单 (/)",
              dark_mode,
              state.slash_open,
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
          // Right cluster: last-edited + right-sidebar toggle + whiteboard + favorite + trash + more
          view()
            .flex_shrink_0()
            .flex_row()
            .items_center()
            .gap_1()
            .child(when(move || !right_open.get(), {
              let last_edited_sig = st_edited.last_edited_text;
              move || {
                text(move || last_edited_sig.get())
                  .text_size(px(11.5))
                  .text_color(move || palette(dark_mode.get()).text_faint)
              }
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
                .w(px(26.0))
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
      let st_cm = st_copy_md.clone();
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
          set_clipboard(st_cm.export_markdown());
          st_cm.set_status_flash("已复制 Markdown");
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
      // 2. Centered 740px paper canvas: native `.selectable(true)` cross-block drag selection
      view()
        .flex_1()
        .w_full()
        .items_center()
        .selectable(true)
        .on_text_selection_change(move |ev: &TextSelectionEvent| {
          st_canvas_sel.canvas_selected_text.set(ev.text.clone());
        })
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
              // Large borderless editable document title (30px SemiBold)
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
                .on_submit({
                  let st_title_enter = state.clone();
                  move |_: &str| st_title_enter.select_block(0)
                }),
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
                  let st_all = st_all_sel.clone();

                  let is_focused = computed(move || st_sel.focused_idx.get() == at.index());
                  let block_disp = computed(move || {
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
                    .rounded(px(4.0))
                    .bg(move || {
                      let p = palette(dark_mode.get());
                      if st_all.all_selected.get() {
                        p.selection_bg
                      } else {
                        p.surface_base
                      }
                    })
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
                            let st_sel_chg = st_e_f.clone();
                            let st_sel_ctrl = st_e_f.clone();
                            let st_key = st_e_f.clone();
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
                            let is_heading_blk = block_style_sig.get_untracked().0 == "heading";
                            let placeholder_str = computed(move || {
                              let (kind, level, _, _) = block_style_sig.get();
                              match kind.as_str() {
                                "heading" => match level {
                                  1 => "一级标题",
                                  2 => "二级标题",
                                  3 => "三级标题",
                                  _ => "标题",
                                },
                                "bulleted_list" | "numbered_list" => "列表项",
                                "todo" => "待办事项",
                                "quote" => "输入引用内容...",
                                _ => "输入正文，或输入 / 唤出命令...",
                              }
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
                                    text(format!("{}.", at.index() + 1))
                                      .text_size(px(15.0))
                                      .text_color(p.text_muted),
                                  ),
                                  "todo" => view().child(
                                    button()
                                      .accessibility_label("切换待办完成状态")
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
                                  .auto_focus(true)
                                  .submit_behavior(SubmitBehavior::Submit)
                                  .placeholder(move || placeholder_str.get())
                                  .flex_1()
                                  .text_size(move || fsize.get())
                                  .font_weight(if is_heading_blk {
                                    FontWeight::SEMIBOLD
                                  } else {
                                    FontWeight::NORMAL
                                  })
                                  .text_color(move || palette(dark_mode.get()).text_primary)
                                  .selection_color(move || palette(dark_mode.get()).selection_hsla)
                                  .bg({
                                    let st_bg = st_val.clone();
                                    move || {
                                      let p = palette(dark_mode.get());
                                      if st_bg.all_selected.get() {
                                        p.selection_bg
                                      } else {
                                        p.surface_base
                                      }
                                    }
                                  })
                                  .value(move || st_val.focused_text.get())
                                  .selected_range(move || st_sel_ctrl.controlled_sel.get())
                                  .on_selection_change(move |sel: &Selection| {
                                    st_sel_chg.input_sel.set((sel.start, sel.end));
                                  })
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
                                    st_ime.input_sel.set((end, end));
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
                                  .on_key_press(move |k: &str| match k {
                                    "Backspace" => st_key.backspace_at_start_if_caret_zero(),
                                    "ArrowUp" => st_key.move_focus_relative(-1),
                                    "ArrowDown" => st_key.move_focus_relative(1),
                                    _ => {}
                                  })
                                  .on_submit(move |_: &str| {
                                    st_sub.split_focused_at_caret();
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
                                .pl(px(left_indent))
                                // Plain mouse-up focus: preserves native drag selection when user drags!
                                .on_mouse_up(move |_: &nsh_core::MouseEvent| {
                                  if st_click.canvas_selected_text.get_untracked().is_empty() {
                                    st_click.select_block(at.index());
                                  }
                                });

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
                                  text(format!("{}.", at.index() + 1))
                                    .text_size(px(15.0))
                                    .text_color(p.text_muted),
                                );
                              } else if disp.kind == "todo" {
                                let glyph = if disp.checked { "☑" } else { "☐" };
                                row = row.child(
                                  button()
                                    .accessibility_label("切换待办完成状态")
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

                              row.child(render_rich_block_text(&disp, fsize, fweight, p))
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
            )
            .child(
              // Notion-style clickable white paper tail below last block
              view()
                .w_full()
                .h(px(140.0))
                .on_mouse_up(move |_: &nsh_core::MouseEvent| {
                  if st_tail.canvas_selected_text.get_untracked().is_empty() {
                    st_tail.focus_or_append_tail_block();
                  }
                }),
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

/// Renders a block's text + UTF-16 inline marks (`bold`, `italic`, `strike`, `code`, `link`)
/// using nested `nsh_core::text` spans so native drag selection & rich formatting both work!
fn render_rich_block_text(
  disp: &BlockDisplay,
  fsize: f32,
  base_weight: FontWeight,
  p: MicaPalette,
) -> impl IntoElement {
  if disp.text.is_empty() {
    return text(" ")
      .flex_1()
      .text_size(px(fsize))
      .text_color(p.text_ghost);
  }
  let base_color = if disp.kind == "quote" {
    p.text_muted
  } else {
    p.text_primary
  };
  if disp.marks.is_empty() {
    return text(disp.text.clone())
      .flex_1()
      .text_size(px(fsize))
      .font_weight(base_weight)
      .text_color(base_color);
  }

  let total_u16 = utf16_len(&disp.text);
  let mut cuts: Vec<u32> = vec![0, total_u16];
  for m in &disp.marks {
    let s = m.start.min(total_u16);
    let e = m.end.min(total_u16);
    if e > s {
      cuts.push(s);
      cuts.push(e);
    }
  }
  cuts.sort_unstable();
  cuts.dedup();

  let mut container = text("")
    .flex_1()
    .text_size(px(fsize))
    .font_weight(base_weight)
    .text_color(base_color);

  for win in cuts.windows(2) {
    let (s, e) = (win[0], win[1]);
    if e <= s {
      continue;
    }
    let slice = slice_utf16(&disp.text, s, e);
    if slice.is_empty() {
      continue;
    }
    let mut is_bold = false;
    let mut is_italic = false;
    let mut is_strike = false;
    let mut is_code = false;
    let mut is_link = false;
    for m in &disp.marks {
      if m.start <= s && m.end >= e {
        match m.kind.as_str() {
          "bold" => is_bold = true,
          "italic" => is_italic = true,
          "strike" => is_strike = true,
          "code" => is_code = true,
          "link" => is_link = true,
          _ => {}
        }
      }
    }
    let mut span = text(slice).text_size(px(fsize));
    if is_bold {
      span = span.font_weight(FontWeight::BOLD);
    }
    if is_italic {
      span = span.italic();
    }
    if is_strike {
      span = span.line_through();
    }
    if is_code {
      span = span.bg(p.code_bg).text_color(p.danger);
    } else if is_link {
      span = span.underline().text_color(p.accent);
    } else {
      span = span.text_color(base_color);
    }
    container = container.child(span);
  }
  container
}

fn slice_utf16(text: &str, start_u16: u32, end_u16: u32) -> String {
  let mut cur = 0u32;
  let mut out = String::new();
  for ch in text.chars() {
    let w = ch.len_utf16() as u32;
    let next = cur + w;
    if cur >= start_u16 && next <= end_u16 {
      out.push(ch);
    }
    cur = next;
    if cur >= end_u16 {
      break;
    }
  }
  out
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
  match b.kind.as_str() {
    "heading" => match b.level {
      1 => (30.0, FontWeight::SEMIBOLD, 14.0),
      2 => (24.0, FontWeight::SEMIBOLD, 10.0),
      3 => (20.0, FontWeight::SEMIBOLD, 8.0),
      4 => (18.0, FontWeight::SEMIBOLD, 6.0),
      _ => (16.0, FontWeight::SEMIBOLD, 4.0),
    },
    _ => (16.0, FontWeight::NORMAL, 2.0),
  }
}

fn compute_outline(blocks: &[EditorBlock]) -> Vec<OutlineItem> {
  blocks
    .iter()
    .enumerate()
    .filter_map(|(idx, b)| {
      if b.kind == "heading" {
        let label = b.text.trim();
        if !label.is_empty() {
          let level = b.data.get("level").and_then(|v| v.as_u64()).unwrap_or(1);
          return Some(OutlineItem {
            block_index: idx,
            level,
            text: label.to_string(),
          });
        }
      }
      None
    })
    .collect()
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
    .w(px(26.0))
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
    .w(px(25.0))
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
  enabled: Signal<bool>,
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
    .w(px(25.0))
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
    .mx_1()
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
