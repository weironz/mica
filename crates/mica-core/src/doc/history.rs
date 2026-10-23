//! Local editor history. Remote updates and non-editor writers have no origin
//! and never enter this manager, even while an editor batch is open.

use yrs::{undo, Doc, Transact, TransactionMut, UndoManager};

use super::{MicaDoc, BLOCKS};

#[path = "history_placements.rs"]
mod placements;
use placements::ChildPlacements;

const EDITOR_ORIGIN: &str = "mica/editor-intent";

pub(super) struct EditorHistory {
  manager: UndoManager,
  batch_depth: usize,
}

impl EditorHistory {
  fn new(doc: &Doc) -> Self {
    // The editor already submits typing bursts at 400ms. Only explicit batch
    // boundaries group history here: a coarse op can span many transactions
    // and a slow batch must still undo atomically.
    let mut manager = UndoManager::with_options(undo::Options {
      capture_timeout_millis: u64::MAX,
      tracked_origins: [EDITOR_ORIGIN.into()].into(),
      ..Default::default()
    });
    manager.expand_scope(doc, &doc.get_or_insert_map(BLOCKS));
    Self {
      manager,
      batch_depth: 0,
    }
  }
}

impl MicaDoc {
  /// Start one editor undo group. Nested batches share the outer group's
  /// boundary. History is allocated lazily; ordinary document use is unchanged.
  pub fn begin_history_batch(&mut self) {
    let history = self
      .history
      .get_or_insert_with(|| EditorHistory::new(&self.doc));
    if history.batch_depth == 0 {
      history.manager.reset();
    }
    history.batch_depth += 1;
  }

  /// Close an editor undo group. Calling this without a matching begin is a
  /// no-op. A subsequent batch always creates a separate undo step.
  pub fn end_history_batch(&mut self) {
    if let Some(history) = &mut self.history {
      if history.batch_depth > 0 {
        history.batch_depth -= 1;
        if history.batch_depth == 0 {
          history.manager.reset();
        }
      }
    }
  }

  /// Clear personal history when an editor opens this document. Also closes
  /// any previous batch; initialization and repair writes remain untracked.
  pub fn reset_history(&mut self) {
    if let Some(history) = &mut self.history {
      history.manager.clear_all();
      history.manager.reset();
      history.batch_depth = 0;
    }
  }

  /// Undo the latest local editor batch, preserving remote changes. Close the
  /// current batch before calling; returns false if no change can be undone.
  pub fn undo(&mut self) -> bool {
    let Some(history) = &mut self.history else {
      return false;
    };
    if history.batch_depth != 0 {
      return false;
    }
    history.manager.reset();
    let placements = ChildPlacements::capture(&self.doc);
    let changed = history.manager.undo_blocking();
    if changed {
      placements.remove_conflicts(&self.doc);
    }
    changed
  }

  /// Redo the latest undone editor batch. Remote updates do not clear redo.
  pub fn redo(&mut self) -> bool {
    let Some(history) = &mut self.history else {
      return false;
    };
    if history.batch_depth != 0 {
      return false;
    }
    history.manager.reset();
    let placements = ChildPlacements::capture(&self.doc);
    let changed = history.manager.redo_blocking();
    if changed {
      placements.remove_conflicts(&self.doc);
    }
    changed
  }

  /// Number of undo/redo items, for aligning editor selection metadata when
  /// one call skips items already overwritten or deleted by a remote peer.
  pub fn history_depths(&self) -> (u32, u32) {
    self.history.as_ref().map_or((0, 0), |history| {
      (
        u32::try_from(history.manager.undo_stack().len()).unwrap_or(u32::MAX),
        u32::try_from(history.manager.redo_stack().len()).unwrap_or(u32::MAX),
      )
    })
  }

  pub fn can_undo(&self) -> bool {
    self
      .history
      .as_ref()
      .is_some_and(|history| history.manager.can_undo())
  }

  pub fn can_redo(&self) -> bool {
    self
      .history
      .as_ref()
      .is_some_and(|history| history.manager.can_redo())
  }

  /// Only editor-intent methods use this transaction. Loading, set_blocks,
  /// single-field maintenance writes and apply_update keep their own ordinary
  /// transactions, so a remote update cannot acquire the active batch's origin.
  pub(super) fn editor_transaction(&self) -> TransactionMut<'_> {
    if self
      .history
      .as_ref()
      .is_some_and(|history| history.batch_depth > 0)
    {
      self.doc.transact_mut_with(EDITOR_ORIGIN)
    } else {
      self.doc.transact_mut()
    }
  }
}
