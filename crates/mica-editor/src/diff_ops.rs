//! Minimal block-list diffing using Patience LIS (longest increasing subsequence).
//!
//! Mirrors `controller.dart::_diffOps` and `_inOrderIds`: blocks already in the
//! longest common relative order stay put, and only the out-of-order blocks emit
//! `move_block` operations.

use std::collections::{HashMap, HashSet};

use crate::model::{BlockPatch, DocOp, EditorBlock, block_data_eq};

/// Ids in `order` forming a longest subsequence whose relative order already
/// matches `to` — the blocks a reorder diff can leave in place (Patience LIS,
/// O(n log n)).
pub fn in_order_ids(order: &[String], to: &[EditorBlock]) -> HashSet<String> {
  let rank: HashMap<&str, usize> = to
    .iter()
    .enumerate()
    .map(|(i, b)| (b.id.as_str(), i))
    .collect();
  let seq: Vec<usize> = order
    .iter()
    .filter_map(|id| rank.get(id.as_str()).copied())
    .collect();
  let mut tails: Vec<usize> = Vec::new();
  let mut prev: Vec<Option<usize>> = vec![None; seq.len()];
  for i in 0..seq.len() {
    let mut lo = 0usize;
    let mut hi = tails.len();
    while lo < hi {
      let mid = (lo + hi) >> 1;
      if seq[tails[mid]] < seq[i] {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    if lo > 0 {
      prev[i] = Some(tails[lo - 1]);
    }
    if lo == tails.len() {
      tails.push(i);
    } else {
      tails[lo] = i;
    }
  }
  let mut keep = HashSet::new();
  let mut cur = tails.last().copied();
  while let Some(i) = cur {
    keep.insert(order[i].clone());
    cur = prev[i];
  }
  keep
}

/// Minimal `DocOp` sequence transforming `from` into `to`:
/// deletes → inserts & LIS-minimal `move_block`s in `to` order → content updates.
pub fn diff_block_ops(root_id: &str, from: &[EditorBlock], to: &[EditorBlock]) -> Vec<DocOp> {
  let from_by_id: HashMap<&str, &EditorBlock> = from.iter().map(|b| (b.id.as_str(), b)).collect();
  let to_ids: HashSet<&str> = to.iter().map(|b| b.id.as_str()).collect();
  let mut ops = Vec::new();
  let mut order: Vec<String> = Vec::with_capacity(from.len());

  for b in from {
    if to_ids.contains(b.id.as_str()) {
      order.push(b.id.clone());
    } else {
      ops.push(DocOp::DeleteBlock {
        block_id: b.id.clone(),
      });
    }
  }

  let stay = in_order_ids(&order, to);
  let after = |order: &[String], index: usize| -> usize {
    if index == 0 {
      0
    } else {
      order
        .iter()
        .position(|id| id == &to[index - 1].id)
        .map_or(order.len(), |p| p + 1)
    }
  };

  for (index, b) in to.iter().enumerate() {
    let Some(&old) = from_by_id.get(b.id.as_str()) else {
      let at = after(&order, index);
      order.insert(at, b.id.clone());
      ops.push(DocOp::InsertBlock {
        parent_id: root_id.to_string(),
        index: at,
        block: b.to_core_block(),
      });
      continue;
    };
    if !stay.contains(&b.id) {
      if let Some(pos) = order.iter().position(|id| id == &b.id) {
        order.remove(pos);
      }
      let at = after(&order, index);
      order.insert(at, b.id.clone());
      ops.push(DocOp::MoveBlock {
        block_id: b.id.clone(),
        parent_id: root_id.to_string(),
        index: at,
      });
    }
    if old.kind != b.kind || old.text != b.text || !block_data_eq(&old.data, &b.data) {
      ops.push(DocOp::UpdateBlock {
        block_id: b.id.clone(),
        kind: (old.kind != b.kind).then(|| b.kind.clone()),
        text: (old.text != b.text).then(|| b.text.clone()),
        data: Some(b.data.clone()),
      });
    }
  }
  ops
}

/// Incremental `BlockPatch` list turning `from` into `to` for renderer caches.
pub fn diff_block_patches(from: &[EditorBlock], to: &[EditorBlock]) -> Vec<BlockPatch> {
  let from_by_id: HashMap<&str, (usize, &EditorBlock)> = from
    .iter()
    .enumerate()
    .map(|(i, b)| (b.id.as_str(), (i, b)))
    .collect();
  let to_ids: HashSet<&str> = to.iter().map(|b| b.id.as_str()).collect();
  let mut patches = Vec::new();

  for b in from {
    if !to_ids.contains(b.id.as_str()) {
      patches.push(BlockPatch::Remove {
        block_id: b.id.clone(),
      });
    }
  }

  let mut order_changed = from.len() != to.len();
  for (i, b) in to.iter().enumerate() {
    match from_by_id.get(b.id.as_str()) {
      None => {
        order_changed = true;
        patches.push(BlockPatch::Upsert {
          index: i,
          block: b.clone(),
        });
      }
      Some(&(old_idx, old)) => {
        if old_idx != i {
          order_changed = true;
        }
        if old.kind != b.kind || old.text != b.text || !block_data_eq(&old.data, &b.data) {
          patches.push(BlockPatch::Upsert {
            index: i,
            block: b.clone(),
          });
        }
      }
    }
  }

  if order_changed {
    patches.push(BlockPatch::Reorder {
      order: to.iter().map(|b| b.id.clone()).collect(),
    });
  }
  patches
}

/// Compute the reordered block list when moving `block_ids` (preserving their
/// existing relative order) to insertion slot `target_index` in `0..=blocks.len()`.
/// Returns `None` if no valid blocks were specified or order did not change.
pub fn compute_move_blocks_target(
  blocks: &[EditorBlock],
  block_ids: &[String],
  target_index: usize,
) -> Option<Vec<EditorBlock>> {
  if blocks.is_empty() || block_ids.is_empty() {
    return None;
  }
  let moving_set: HashSet<&str> = block_ids.iter().map(String::as_str).collect();
  let target = target_index.min(blocks.len());

  // Count how many moving blocks sit strictly before `target` so the insertion
  // slot adjusts cleanly once moving blocks are lifted out.
  let before_target = blocks[..target]
    .iter()
    .filter(|b| moving_set.contains(b.id.as_str()))
    .count();

  let mut remaining = Vec::with_capacity(blocks.len());
  let mut moved = Vec::new();
  for b in blocks {
    if moving_set.contains(b.id.as_str()) {
      moved.push(b.clone());
    } else {
      remaining.push(b.clone());
    }
  }
  if moved.is_empty() {
    return None;
  }
  let insert_at = (target - before_target).min(remaining.len());
  let mut result = Vec::with_capacity(blocks.len());
  result.extend(remaining.drain(..insert_at));
  result.extend(moved);
  result.extend(remaining);

  if result.iter().map(|b| &b.id).eq(blocks.iter().map(|b| &b.id)) {
    return None;
  }
  Some(result)
}
