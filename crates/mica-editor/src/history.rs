//! Editor undo/redo selection metadata synchronized with `mica_core::MicaDoc`'s
//! native CRDT `UndoManager` (yrs forward-operation compatible, issue #79).
//!
//! Mirrors `operation_history.dart` (`HistorySelection`) and
//! `controller_history.dart` (stack depth alignment when remote peers overwrite
//! or delete local items).

use mica_core::MicaDoc;

use crate::model::EditorBlock;
use crate::selection::{DocPosition, DocSelection};

/// Stable reference to one selection endpoint across undo/redo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEndpoint {
  pub id: String,
  pub index: usize,
  pub offset: u32,
}

/// Captured selection metadata proportional to selection endpoints, not the whole document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HistorySelection {
  #[default]
  None,
  Text {
    anchor: HistoryEndpoint,
    focus: HistoryEndpoint,
  },
  BlockAtom {
    id: String,
    index: usize,
  },
  TableCell {
    id: String,
    index: usize,
    row: usize,
    col: usize,
    end_row: Option<usize>,
    end_col: Option<usize>,
  },
}

impl HistorySelection {
  pub fn capture(blocks: &[EditorBlock], selection: Option<&DocSelection>) -> Self {
    let Some(sel) = selection else {
      return Self::None;
    };
    if blocks.is_empty() {
      return Self::None;
    }
    let endpoint = |pos: DocPosition| -> HistoryEndpoint {
      let idx = pos.node.min(blocks.len() - 1);
      HistoryEndpoint {
        id: blocks[idx].id.clone(),
        index: idx,
        offset: pos.offset,
      }
    };
    match *sel {
      DocSelection::BlockAtom { block_index } => {
        let idx = block_index.min(blocks.len() - 1);
        Self::BlockAtom {
          id: blocks[idx].id.clone(),
          index: idx,
        }
      }
      DocSelection::TableCell {
        block_index,
        row,
        col,
        end_row,
        end_col,
        ..
      } => {
        let idx = block_index.min(blocks.len() - 1);
        Self::TableCell {
          id: blocks[idx].id.clone(),
          index: idx,
          row,
          col,
          end_row,
          end_col,
        }
      }
      _ => Self::Text {
        anchor: endpoint(sel.anchor_pos()),
        focus: endpoint(sel.focus_pos()),
      },
    }
  }

  /// Resolve captured selection against the restored `blocks`.
  pub fn restore(&self, blocks: &[EditorBlock]) -> Option<DocSelection> {
    if blocks.is_empty() {
      return None;
    }
    let find_index = |id: &str, fallback: usize| -> usize {
      blocks
        .iter()
        .position(|b| b.id == id)
        .unwrap_or_else(|| fallback.min(blocks.len() - 1))
    };
    let raw = match self {
      Self::None => return None,
      Self::BlockAtom { id, index } => {
        let idx = find_index(id, *index);
        if blocks[idx].is_atomic() {
          DocSelection::BlockAtom { block_index: idx }
        } else {
          DocSelection::Caret {
            block_index: idx,
            offset: 0,
          }
        }
      }
      Self::TableCell {
        id,
        index,
        row,
        col,
        end_row,
        end_col,
      } => {
        let idx = find_index(id, *index);
        if blocks[idx].kind == "table" {
          DocSelection::TableCell {
            block_index: idx,
            row: *row,
            col: *col,
            end_row: *end_row,
            end_col: *end_col,
            anchor: None,
            focus: None,
          }
        } else {
          DocSelection::Caret {
            block_index: idx,
            offset: 0,
          }
        }
      }
      Self::Text { anchor, focus } => {
        let resolve = |ep: &HistoryEndpoint| -> DocPosition {
          let idx = find_index(&ep.id, ep.index);
          DocPosition::new(idx, ep.offset.min(blocks[idx].utf16_len()))
        };
        DocSelection::from_positions(resolve(anchor), resolve(focus))
      }
    };
    raw.clamp_and_snap(blocks)
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HistoryMetaEntry {
  before: HistorySelection,
  after: HistorySelection,
}

/// Manages typing-batch coalescing and selection metadata alongside `MicaDoc`'s
/// CRDT `UndoManager`.
#[derive(Debug, Default)]
pub struct EditorHistoryTracker {
  undo_meta: Vec<HistoryMetaEntry>,
  redo_meta: Vec<HistoryMetaEntry>,
  /// When `Some(block_id)`, a `MicaDoc` history batch is currently open for
  /// coalescing consecutive plain-text edits inside that block.
  open_typing_block: Option<String>,
  /// Undo stack depth recorded before the current batch began.
  batch_start_undo_depth: u32,
  /// Selection captured before the current batch began.
  batch_before_sel: HistorySelection,
}

impl EditorHistoryTracker {
  pub fn new() -> Self {
    Self::default()
  }

  /// Clear all tracked history metadata and reset `MicaDoc`'s history.
  pub fn reset(&mut self, doc: &mut MicaDoc) {
    self.open_typing_block = None;
    self.batch_start_undo_depth = 0;
    self.batch_before_sel = HistorySelection::None;
    self.undo_meta.clear();
    self.redo_meta.clear();
    doc.reset_history();
  }

  /// Close any open typing coalescing batch so subsequent edits form a distinct
  /// undo step (and so `MicaDoc::undo`/`redo` can execute).
  pub fn flush_typing_batch(&mut self, doc: &mut MicaDoc) {
    if self.open_typing_block.take().is_some() {
      doc.end_history_batch();
    }
  }

  /// Execute a coalescible typing edit inside `block_id`.
  ///
  /// Consecutive typing edits to the same block share one `MicaDoc` history
  /// batch until flushed or interrupted by a structural command / block switch.
  pub fn record_typing_edit<F>(
    &mut self,
    doc: &mut MicaDoc,
    block_id: &str,
    before_sel: HistorySelection,
    apply: F,
    after_sel: impl FnOnce() -> HistorySelection,
  ) where
    F: FnOnce(&mut MicaDoc),
  {
    let continuing = self.open_typing_block.as_deref() == Some(block_id);
    if !continuing {
      self.flush_typing_batch(doc);
      self.batch_start_undo_depth = doc.history_depths().0;
      self.batch_before_sel = before_sel;
      doc.begin_history_batch();
      self.open_typing_block = Some(block_id.to_string());
    }

    apply(doc);

    let (undo_depth, redo_depth) = doc.history_depths();
    let after = after_sel();
    if undo_depth > self.batch_start_undo_depth {
      if self.undo_meta.len() < undo_depth as usize {
        self.undo_meta.push(HistoryMetaEntry {
          before: self.batch_before_sel.clone(),
          after,
        });
      } else if let Some(top) = self.undo_meta.last_mut() {
        top.after = after;
      }
      if redo_depth == 0 {
        self.redo_meta.clear();
      }
    }
  }

  /// Execute an atomic structural edit (split, merge, format, paste, table op,
  /// move, AI edit, etc.) as ONE isolated `MicaDoc` undo step.
  pub fn record_structural_edit<F>(
    &mut self,
    doc: &mut MicaDoc,
    before_sel: HistorySelection,
    apply: F,
    after_sel: impl FnOnce() -> HistorySelection,
  ) where
    F: FnOnce(&mut MicaDoc),
  {
    self.flush_typing_batch(doc);
    let depth_before = doc.history_depths().0;
    doc.begin_history_batch();
    apply(doc);
    doc.end_history_batch();

    let (depth_after, redo_depth) = doc.history_depths();
    if depth_after > depth_before {
      while self.undo_meta.len() + 1 < depth_after as usize {
        self.undo_meta.push(HistoryMetaEntry {
          before: before_sel.clone(),
          after: before_sel.clone(),
        });
      }
      self.undo_meta.push(HistoryMetaEntry {
        before: before_sel,
        after: after_sel(),
      });
      if redo_depth == 0 {
        self.redo_meta.clear();
      }
    }
  }

  /// Step undo (`redo == false`) or redo (`redo == true`) on `MicaDoc`.
  ///
  /// Returns `Some(restored_selection_metadata)` if the document changed,
  /// aligning stacks with `MicaDoc::history_depths()` when remote peers caused
  /// yrs to skip overwritten items.
  pub fn step(&mut self, doc: &mut MicaDoc, redo: bool) -> Option<HistorySelection> {
    self.flush_typing_batch(doc);
    let changed = if redo { doc.redo() } else { doc.undo() };
    let (undo_depth, redo_depth) = doc.history_depths();
    let (source, dest, src_depth) = if redo {
      (&mut self.redo_meta, &mut self.undo_meta, redo_depth as usize)
    } else {
      (&mut self.undo_meta, &mut self.redo_meta, undo_depth as usize)
    };

    let restored = if changed {
      if source.is_empty() {
        HistorySelection::None
      } else {
        let remaining = src_depth.min(source.len());
        let pick_idx = remaining.min(source.len() - 1);
        let entry = source[pick_idx].clone();
        source.truncate(remaining);
        dest.push(HistoryMetaEntry {
          before: entry.after.clone(),
          after: entry.before.clone(),
        });
        entry.before
      }
    } else {
      if !doc.can_undo() {
        self.undo_meta.clear();
      }
      if !doc.can_redo() {
        self.redo_meta.clear();
      }
      return None;
    };

    if !doc.can_undo() {
      self.undo_meta.clear();
    }
    if !doc.can_redo() {
      self.redo_meta.clear();
    }
    Some(restored)
  }

  pub fn can_undo(&self, doc: &MicaDoc) -> bool {
    doc.can_undo()
  }

  pub fn can_redo(&self, doc: &MicaDoc) -> bool {
    doc.can_redo()
  }
}
