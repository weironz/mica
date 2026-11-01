//! Which blocks changed since the reader last looked — so a replica that
//! re-reads the document after every remote update can re-read only the blocks
//! that update touched instead of the whole document (2,000 blocks ≈ 10 ms per
//! update on the web, measured 2026-11-01).
//!
//! Opt-in per document ([`MicaDoc::track_changes`]): the server's documents
//! never pay for the observer. Every transaction counts — remote merges, local
//! edits, undo/redo — because the reader's cache must match the document
//! whichever way it changed.
//!
//! The answer is deliberately coarse: either "only the content (text, marks,
//! type, props) of these blocks changed", or `None` = read everything. Any
//! change to the tree's shape (a block added, removed or moved, a `children`
//! array or the root touched) is `None`, so the reader never has to re-derive
//! tree order — that stays in [`MicaDoc::to_blocks`].

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use yrs::types::{Event, PathSegment};
use yrs::{DeepObservable, Map};

use super::{MicaDoc, BLOCKS, META};

/// Past this many distinct blocks a full read is as cheap; stop listing them.
const MAX_LISTED: usize = 1024;
const OBSERVER: &str = "mica-core/changes";

#[derive(Default)]
pub(super) struct ChangeLog {
  ids: HashSet<String>,
  /// The reader must read everything: tracking just started, the shape
  /// changed, or too many blocks changed to be worth listing.
  all: bool,
}

impl ChangeLog {
  fn content(&mut self, id: &str) {
    if self.all {
      return;
    }
    if self.ids.len() >= MAX_LISTED {
      self.structural();
    } else {
      self.ids.insert(id.to_string());
    }
  }

  fn structural(&mut self) {
    self.all = true;
    self.ids.clear();
  }
}

/// Classify one deep event under the `blocks` map.
fn record(log: &mut ChangeLog, txn: &yrs::TransactionMut, event: &Event) {
  let path = event.path();
  let mut segments = path.iter();
  let id = match segments.next() {
    Some(PathSegment::Key(id)) => id,
    // The `blocks` map itself: blocks added or removed. Or a shape we do not
    // write — either way, not something to patch.
    _ => return log.structural(),
  };
  match segments.next() {
    // Under the block: its `children` array is the tree's shape.
    Some(PathSegment::Key(key)) if key.as_ref() == "children" => log.structural(),
    Some(_) => log.content(id),
    // The block's own map: a key replaced. `children` replaced wholesale is
    // still the shape.
    None => match event {
      Event::Map(e) if e.keys(txn).contains_key("children") => log.structural(),
      _ => log.content(id),
    },
  }
}

impl MicaDoc {
  /// Start recording which blocks change (see the module docs). Idempotent;
  /// the first [`Self::take_changed_block_ids`] after it says "read everything".
  pub fn track_changes(&mut self) {
    if self.changes.is_some() {
      return;
    }
    let log = Arc::new(Mutex::new(ChangeLog {
      all: true,
      ..Default::default()
    }));
    let blocks = self.doc.get_or_insert_map(BLOCKS);
    let meta = self.doc.get_or_insert_map(META);
    let on_blocks = log.clone();
    blocks.observe_deep(OBSERVER, move |txn, events| {
      let mut log = on_blocks.lock().unwrap_or_else(|e| e.into_inner());
      for event in events.iter() {
        record(&mut log, txn, event);
      }
    });
    let on_meta = log.clone();
    meta.observe_deep(OBSERVER, move |_, _| {
      on_meta
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .structural();
    });
    self.changes = Some(log);
  }

  /// The blocks whose content changed since the last call, and reset. `None`:
  /// read the whole document — tracking is off or just started, the tree's
  /// shape changed, or too many blocks changed to list. `Some(empty)`: nothing
  /// changed.
  pub fn take_changed_block_ids(&mut self) -> Option<Vec<String>> {
    let mut log = self
      .changes
      .as_ref()?
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    let all = std::mem::take(&mut log.all);
    let ids = std::mem::take(&mut log.ids);
    (!all).then(|| {
      let mut ids: Vec<String> = ids.into_iter().collect();
      ids.sort();
      ids
    })
  }

  /// The named blocks, in the order asked; ids not in the document are left
  /// out. Read in one transaction, like [`Self::to_blocks`].
  pub fn blocks_by_id(&self, ids: &[String]) -> Vec<crate::block::Block> {
    let blocks_map = self.doc.get_or_insert_map(BLOCKS);
    let txn = yrs::Transact::transact(&self.doc);
    ids
      .iter()
      .filter(|id| blocks_map.contains_key(&txn, id))
      .filter_map(|id| self.read_block(&txn, &blocks_map, id))
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::block::Block;

  fn block(id: &str, kind: &str, text: &str, children: &[&str]) -> Block {
    Block {
      id: id.to_string(),
      kind: kind.to_string(),
      text: text.to_string(),
      data: serde_json::json!({}),
      children: children.iter().map(|c| c.to_string()).collect(),
    }
  }

  fn doc() -> MicaDoc {
    MicaDoc::from_blocks(
      "r",
      &[
        block("r", "page", "", &["a", "b"]),
        block("a", "paragraph", "alpha", &[]),
        block("b", "paragraph", "beta", &[]),
      ],
    )
  }

  /// A peer's edit, delivered as an update — the path the replicas care about.
  fn remote(from: &MicaDoc, edit: impl FnOnce(&mut MicaDoc)) -> Vec<u8> {
    let mut peer = MicaDoc::from_update(&from.encode_state()).unwrap();
    let sv = peer.state_vector();
    edit(&mut peer);
    peer.encode_diff(&sv).unwrap()
  }

  #[test]
  fn untracked_and_freshly_tracked_documents_say_read_everything() {
    let mut d = doc();
    assert_eq!(d.take_changed_block_ids(), None);
    d.track_changes();
    assert_eq!(d.take_changed_block_ids(), None, "nothing read yet");
    assert_eq!(d.take_changed_block_ids(), Some(vec![]));
  }

  #[test]
  fn content_edits_name_their_blocks_from_any_source() {
    let mut d = doc();
    d.track_changes();
    d.take_changed_block_ids();

    let update = remote(&d, |p| p.text_insert("a", 5, "!"));
    d.apply_update(&update).unwrap();
    assert_eq!(d.take_changed_block_ids(), Some(vec!["a".to_string()]));

    let update = remote(&d, |p| {
      p.set_block_prop("b", "indent", &serde_json::json!(1));
    });
    d.apply_update(&update).unwrap();
    assert_eq!(d.take_changed_block_ids(), Some(vec!["b".to_string()]));

    d.begin_history_batch();
    d.text_insert("b", 0, "x");
    d.update_block_kind("a", "heading");
    d.end_history_batch();
    assert_eq!(
      d.take_changed_block_ids(),
      Some(vec!["a".to_string(), "b".to_string()])
    );
    assert!(d.undo());
    assert_eq!(
      d.take_changed_block_ids(),
      Some(vec!["a".to_string(), "b".to_string()]),
      "undo changes the same blocks back"
    );
    let read = d.blocks_by_id(&["b".to_string(), "gone".to_string()]);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].text, "beta");
  }

  #[test]
  fn any_change_to_the_shape_says_read_everything() {
    type Edit = Box<dyn Fn(&mut MicaDoc)>;
    let shape_edits: Vec<Edit> = vec![
      Box::new(|p| p.insert_block("r", 1, &block("c", "paragraph", "new", &[]))),
      Box::new(|p| p.delete_block("b", false)),
      Box::new(|p| p.move_block("b", "r", 0)),
      Box::new(|p| p.split_block("a", 2, "a2", "paragraph")),
    ];
    for edit in shape_edits {
      let mut d = doc();
      d.track_changes();
      d.take_changed_block_ids();
      let update = remote(&d, |p| edit(p));
      d.apply_update(&update).unwrap();
      assert_eq!(d.take_changed_block_ids(), None);
      assert_eq!(d.take_changed_block_ids(), Some(vec![]), "and resets");
    }
  }

  #[test]
  fn too_many_blocks_is_a_full_read() {
    let ids: Vec<String> = (0..=MAX_LISTED).map(|i| format!("p{i}")).collect();
    let mut blocks = vec![Block {
      children: ids.clone(),
      ..block("r", "page", "", &[])
    }];
    blocks.extend(ids.iter().map(|id| block(id, "paragraph", "", &[])));
    let mut d = MicaDoc::from_blocks("r", &blocks);
    d.track_changes();
    d.take_changed_block_ids();
    for id in &ids {
      d.text_insert(id, 0, "x");
    }
    assert_eq!(d.take_changed_block_ids(), None);
  }
}
