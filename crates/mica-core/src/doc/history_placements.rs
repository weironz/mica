//! Preserve a peer's surviving child occurrence when undo/redo restores an old
//! position. The item IDs live only for this history call, not on stack entries.

use std::collections::HashSet;

use yrs::{
  Any, Array, ArrayRef, Assoc, Doc, IndexedSequence, Map, MapRef, Out, ReadTxn, Transact, ID,
};

use super::super::{get_children, BLOCKS};

pub(super) struct ChildPlacements(HashSet<ID>);

impl ChildPlacements {
  pub(super) fn capture(doc: &Doc) -> Self {
    let blocks = doc.get_or_insert_map(BLOCKS);
    let txn = doc.transact();
    Self(
      placements(&txn, &blocks)
        .into_iter()
        .map(|placement| placement.item_id)
        .collect(),
    )
  }

  pub(super) fn remove_conflicts(self, doc: &Doc) {
    let blocks = doc.get_or_insert_map(BLOCKS);
    // Ordinary origin: the repair must not enter history or clear redo. The
    // caller publishes its diff only after this transaction has also committed.
    let mut txn = doc.transact_mut();
    let after = placements(&txn, &blocks);
    let surviving: HashSet<&str> = after
      .iter()
      .filter(|placement| self.0.contains(&placement.item_id))
      .map(|placement| placement.block_id.as_str())
      .collect();
    // Scan backwards within every array so removals keep earlier indexes valid.
    // The surviving occurrence can belong to a different parent.
    for placement in after.iter().rev() {
      if surviving.contains(placement.block_id.as_str()) && !self.0.contains(&placement.item_id) {
        placement.children.remove(&mut txn, placement.index);
      }
    }
  }
}

struct ChildPlacement {
  block_id: String,
  children: ArrayRef,
  index: u32,
  item_id: ID,
}

fn placements<T: ReadTxn>(txn: &T, blocks: &MapRef) -> Vec<ChildPlacement> {
  let mut result = Vec::new();
  for (_, value) in blocks.iter(txn) {
    let Out::YMap(block) = value else {
      continue;
    };
    let Some(children) = get_children(txn, &block) else {
      continue;
    };
    for (index, value) in children.iter(txn).enumerate() {
      let Out::Any(Any::String(block_id)) = value else {
        continue;
      };
      let index = index as u32;
      let Some(item_id) = children
        .sticky_index(txn, index, Assoc::After)
        .and_then(|position| position.id().copied())
      else {
        continue;
      };
      result.push(ChildPlacement {
        block_id: block_id.to_string(),
        children: children.clone(),
        index,
        item_id,
      });
    }
  }
  result
}
