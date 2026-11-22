//! Pure Rust sidebar tree selection, drag-and-drop planning, and rename logic.
//!
//! 1:1 port of Mica Flutter's `tree_drop_planner.dart`, `tree_selection.dart`,
//! `main_tree_helpers.dart` (`reparentLevelFor`, `rootDropOrder`), and `rename.dart`.
//!
//! Enforces Mica's core tree invariants via `mica_core::tree_rules`:
//! - `folder` is the ONLY container; `page` is always a leaf.
//! - Self-parenting and dropping a folder into its own subtree are strictly rejected.
//! - Shift+Click range selection only spans siblings under the same parent (skipping
//!   interleaved descendants of expanded folders); cross-parent degrades to plain toggle.

use std::collections::{HashMap, HashSet};

use mica_client::TreeRow;
use mica_core::tree_rules::{MissingParent, ParentLookup, judge_parent};
use uuid::Uuid;

/// Standard sidebar tree row height in points (matches `kSidebarRowHeight = 30`).
pub const TREE_ROW_HEIGHT: f32 = 30.0;

/// Horizontal indent per nesting depth (matches `kTreeIndentUnit = 16.0`).
pub const TREE_INDENT_UNIT: f32 = 16.0;

/// Fixed expand/collapse chevron slot width (matches `kTreeExpandColumnWidth = 18.0`).
pub const TREE_EXPAND_COLUMN_WIDTH: f32 = 18.0;

/// Horizontal inset where level-0 icons and drop indicators begin (`2 + 18 = 20.0`).
pub const TREE_BASE_INSET: f32 = 2.0 + TREE_EXPAND_COLUMN_WIDTH;

/// Horizontal inset for a drop indicator at nesting `depth`.
pub fn drop_indicator_inset(depth: usize) -> f32 {
  TREE_BASE_INSET + (depth as f32) * TREE_INDENT_UNIT
}

/// Where dragged tree rows land relative to a target row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropMode {
  Before,
  Into,
  After,
}

/// Visual state of a tree drop indicator (idle, allowed in accent, blocked in warning).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropIndicatorState {
  Idle,
  Allowed,
  Blocked,
}

/// Click intent decoded from keyboard modifiers (`Shift > Ctrl/Meta > Open`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClickIntent {
  Open,
  Toggle,
  Range,
}

/// Decode modifier keys held during a sidebar tree click.
///
/// Shift takes precedence over Ctrl/Meta (Explorer & Finder semantics).
pub fn tree_click_intent(shift: bool, ctrl_or_meta: bool) -> ClickIntent {
  if shift {
    ClickIntent::Range
  } else if ctrl_or_meta {
    ClickIntent::Toggle
  } else {
    ClickIntent::Open
  }
}

/// Multi-selection state for the sidebar page tree.
///
/// Stores view IDs and the anchor ID from which `Shift+Click` ranges measure.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeSelection {
  pub ids: Vec<Uuid>,
  pub anchor: Option<Uuid>,
}

impl TreeSelection {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn is_empty(&self) -> bool {
    self.ids.is_empty()
  }

  pub fn len(&self) -> usize {
    self.ids.len()
  }

  pub fn contains(&self, id: Uuid) -> bool {
    self.ids.contains(&id)
  }

  /// Unmodified click on `id`: clears multi-select and records `id` as the anchor
  /// for a subsequent `Shift+Click`. Returns `true` if selection or anchor changed.
  pub fn anchor_on(&mut self, id: Uuid) -> bool {
    if self.ids.is_empty() && self.anchor == Some(id) {
      return false;
    }
    self.ids.clear();
    self.anchor = Some(id);
    true
  }

  /// Clears both selection and anchor (e.g. clicking empty area below tree or after a drop).
  /// Returns `true` if anything changed.
  pub fn clear(&mut self) -> bool {
    if self.ids.is_empty() && self.anchor.is_none() {
      return false;
    }
    self.ids.clear();
    self.anchor = None;
    true
  }

  /// `Ctrl/Cmd+Click` toggle: adds `id` if absent, removes if present, and updates anchor.
  pub fn toggle(&mut self, id: Uuid) {
    if let Some(pos) = self.ids.iter().position(|&x| x == id) {
      self.ids.remove(pos);
    } else {
      self.ids.push(id);
    }
    self.anchor = Some(id);
  }

  /// `Shift+Click` range selection within a single parent.
  ///
  /// - If `anchor` and `target` are both visible and share the same `parent_view_id`,
  ///   all visible siblings between them (skipping deeper children of expanded folders)
  ///   are added to `ids`, and `anchor` stays put.
  /// - Otherwise (missing anchor, row not visible, or cross-parent), degrades cleanly
  ///   to [`Self::toggle`].
  pub fn select_range(&mut self, target: Uuid, visible: &[TreeRow]) {
    if let Some(anchor_id) = self.anchor {
      let anchor_pos = visible.iter().position(|r| r.view_id == anchor_id);
      let target_pos = visible.iter().position(|r| r.view_id == target);
      if let (Some(a), Some(t)) = (anchor_pos, target_pos) {
        let parent = visible[a].parent_view_id;
        if visible[t].parent_view_id == parent {
          let (from, to) = if a <= t { (a, t) } else { (t, a) };
          for row in &visible[from..=to] {
            if row.parent_view_id == parent && !self.ids.contains(&row.view_id) {
              self.ids.push(row.view_id);
            }
          }
          return;
        }
      }
    }
    self.toggle(target);
  }

  /// Drops selected rows and anchor that are no longer visible (e.g. after collapsing
  /// a folder or trashing a view). Returns `true` if anything changed.
  pub fn retain_visible(&mut self, visible: &[TreeRow]) -> bool {
    let visible_set: HashSet<Uuid> = visible.iter().map(|r| r.view_id).collect();
    let prev_len = self.ids.len();
    let prev_anchor = self.anchor;
    self.ids.retain(|id| visible_set.contains(id));
    if let Some(a) = self.anchor
      && !visible_set.contains(&a)
    {
      self.anchor = None;
    }
    self.ids.len() != prev_len || self.anchor != prev_anchor
  }

  /// True when a context menu or drag gesture on `id` acts on the entire selection.
  ///
  /// Requires at least 2 selected rows AND `id` being part of the selection.
  pub fn acts_on_selection(&self, id: Uuid) -> bool {
    self.ids.len() > 1 && self.ids.contains(&id)
  }
}

/// Returns the selected IDs in visible tree order with any descendant dropped whenever
/// one of its ancestors is also selected.
///
/// Prevents a batch move/drag from extracting a selected child out of its simultaneously
/// selected parent folder. Cycle-safe against stale parent pointers.
pub fn independent_selection(selected: &[Uuid], rows: &[TreeRow]) -> Vec<Uuid> {
  let wanted: HashSet<Uuid> = selected.iter().copied().collect();
  let parent_of: HashMap<Uuid, Option<Uuid>> = rows
    .iter()
    .map(|r| (r.view_id, r.parent_view_id))
    .collect();

  // Order candidates in visible tree order first, then any remaining selected IDs.
  let mut ordered: Vec<Uuid> = Vec::with_capacity(wanted.len());
  let mut seen: HashSet<Uuid> = HashSet::with_capacity(wanted.len());
  for row in rows {
    if wanted.contains(&row.view_id) && seen.insert(row.view_id) {
      ordered.push(row.view_id);
    }
  }
  for &id in selected {
    if seen.insert(id) {
      ordered.push(id);
    }
  }

  let mut kept = Vec::with_capacity(ordered.len());
  for id in ordered {
    let mut cursor = parent_of.get(&id).copied().flatten();
    // Seed with `id` itself so a stale cycle never drops the row by looping back to itself.
    let mut walked: HashSet<Uuid> = HashSet::new();
    walked.insert(id);
    let mut nested = false;
    while let Some(parent_id) = cursor {
      if !walked.insert(parent_id) {
        break;
      }
      if wanted.contains(&parent_id) {
        nested = true;
        break;
      }
      cursor = parent_of.get(&parent_id).copied().flatten();
    }
    if !nested {
      kept.push(id);
    }
  }
  kept
}

/// Pure sidebar drag-and-drop planner (validation, reparent depth, sibling ordering).
pub struct TreeDropPlanner;

impl TreeDropPlanner {
  /// True if `candidate_id` equals `root_id` or lies anywhere inside `root_id`'s subtree.
  pub fn is_self_or_descendant(candidate_id: Uuid, root_id: Uuid, all_rows: &[TreeRow]) -> bool {
    if candidate_id == root_id {
      return true;
    }
    let parents: HashMap<Uuid, Option<Uuid>> = all_rows
      .iter()
      .map(|r| (r.view_id, r.parent_view_id))
      .collect();
    let mut cursor = Some(candidate_id);
    let mut seen: HashSet<Uuid> = HashSet::new();
    while let Some(curr) = cursor {
      if !seen.insert(curr) {
        break;
      }
      let parent = parents.get(&curr).copied().flatten();
      if parent == Some(root_id) {
        return true;
      }
      cursor = parent;
    }
    false
  }

  /// Effective destination parent for dropping onto `target` in `mode`.
  pub fn drop_parent_id(target: &TreeRow, mode: DropMode) -> Option<Uuid> {
    match mode {
      DropMode::Into => Some(target.view_id),
      DropMode::Before | DropMode::After => target.parent_view_id,
    }
  }

  /// Validates whether `dragged` rows can drop onto `target` in `mode`.
  ///
  /// Enforces:
  /// 1. Non-empty drag set.
  /// 2. Target is not any dragged row or inside any dragged row's subtree (no cycles).
  /// 3. Effective destination parent is either root (`None`) or a `folder` row via
  ///    `mica_core::tree_rules::judge_parent` (pages can NEVER accept `DropMode::Into`).
  pub fn can_drop(
    dragged: &[Uuid],
    target: &TreeRow,
    mode: DropMode,
    all_rows: &[TreeRow],
  ) -> bool {
    if dragged.is_empty() {
      return false;
    }
    if dragged
      .iter()
      .any(|&d| Self::is_self_or_descendant(target.view_id, d, all_rows))
    {
      return false;
    }
    let Some(parent_id) = Self::drop_parent_id(target, mode) else {
      return true;
    };
    let lookup = match all_rows.iter().find(|r| r.view_id == parent_id) {
      Some(r) if r.is_folder => ParentLookup::Folder,
      Some(_) => ParentLookup::NotFolder,
      None => ParentLookup::Missing,
    };
    judge_parent(dragged.contains(&parent_id), lookup, MissingParent::Refuse).is_ok()
  }

  /// Computes the target nesting level when hovering the bottom (`After`) zone of a row
  /// at `row_depth`, allowing the user to drag left to pop out to an ancestor level.
  ///
  /// - `base_inset = 20.0`, `indent = 16.0`.
  /// - Bounded below by `next_row_depth.unwrap_or(0).min(row_depth)` so the indicator
  ///   never points to an ancestor gap that does not border this row.
  pub fn reparent_level_for(
    pointer_x: f32,
    row_depth: usize,
    next_row_depth: Option<usize>,
  ) -> usize {
    let min_level = match next_row_depth {
      None => 0,
      Some(next) => next.min(row_depth),
    };
    if min_level >= row_depth {
      return row_depth;
    }
    let raw = ((pointer_x - TREE_BASE_INSET) / TREE_INDENT_UNIT).round() as isize;
    raw.clamp(min_level as isize, row_depth as isize) as usize
  }

  /// Resolves an `After` drop at `level` back to the ancestor row at that depth.
  ///
  /// Dropping after `visible[target_idx]` at `level < target.depth` means becoming
  /// the next sibling of the ancestor at depth `level`.
  pub fn resolve_after_target(target_idx: usize, level: usize, visible: &[TreeRow]) -> Uuid {
    let target = &visible[target_idx];
    if level >= target.depth {
      return target.view_id;
    }
    let by_id: HashMap<Uuid, &TreeRow> = visible.iter().map(|r| (r.view_id, r)).collect();
    let mut cursor = target;
    for _ in level..target.depth {
      let Some(pid) = cursor.parent_view_id else {
        return cursor.view_id;
      };
      let Some(&parent) = by_id.get(&pid) else {
        return cursor.view_id;
      };
      cursor = parent;
    }
    cursor.view_id
  }

  /// Computes the new sibling order when `dragged` rows are dropped before or after `target_id`.
  ///
  /// Removes all `dragged` items before locating `target_id` so dropping below a
  /// dragged item's previous slot never lands off-by-one. Returns `siblings` unchanged
  /// if `target_id` is in `dragged` or missing from `siblings`.
  pub fn sibling_order_after_drop(
    siblings: &[Uuid],
    dragged: &[Uuid],
    target_id: Uuid,
    before: bool,
  ) -> Vec<Uuid> {
    let moving: HashSet<Uuid> = dragged.iter().copied().collect();
    if moving.contains(&target_id) {
      return siblings.to_vec();
    }
    let rest: Vec<Uuid> = siblings
      .iter()
      .copied()
      .filter(|id| !moving.contains(id))
      .collect();
    let Some(at) = rest.iter().position(|&id| id == target_id) else {
      return siblings.to_vec();
    };
    let insert_at = if before { at } else { at + 1 };
    let mut out = Vec::with_capacity(rest.len() + dragged.len());
    out.extend_from_slice(&rest[..insert_at]);
    out.extend_from_slice(dragged);
    out.extend_from_slice(&rest[insert_at..]);
    out
  }

  /// Computes root children order after dropping `dragged` onto the bottom root zone.
  ///
  /// Filters out any dragged IDs already at the root before appending them at the end.
  pub fn root_drop_order(root_siblings: &[Uuid], dragged: &[Uuid]) -> Vec<Uuid> {
    let moving: HashSet<Uuid> = dragged.iter().copied().collect();
    let mut out: Vec<Uuid> = root_siblings
      .iter()
      .copied()
      .filter(|id| !moving.contains(id))
      .collect();
    out.extend_from_slice(dragged);
    out
  }

  /// Maps vertical pointer offset within a tree row into `Before` (0..30%),
  /// `Into` (30..70%), or `After` (70..100%).
  pub fn drop_zone_for_local_y(local_y: f32, row_height: f32) -> DropMode {
    if row_height <= 0.0 {
      return DropMode::Into;
    }
    let ratio = (local_y / row_height).clamp(0.0, 1.0);
    if ratio < 0.30 {
      DropMode::Before
    } else if ratio < 0.70 {
      DropMode::Into
    } else {
      DropMode::After
    }
  }
}

/// Validates a rename input against `current_name` (matches Flutter's `renamedTo`).
///
/// Returns `None` when the trimmed input is empty or unchanged.
pub fn renamed_to(input: &str, current_name: &str) -> Option<String> {
  let trimmed = input.trim();
  if trimmed.is_empty() || trimmed == current_name {
    return None;
  }
  Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn mk_row(
    id: Uuid,
    name: &str,
    is_folder: bool,
    depth: usize,
    parent: Option<Uuid>,
  ) -> TreeRow {
    TreeRow {
      view_id: id,
      object_id: Uuid::new_v4(),
      name: name.to_string(),
      icon: None,
      is_folder,
      depth,
      has_children: is_folder,
      expanded: is_folder,
      parent_view_id: parent,
    }
  }

  #[test]
  fn click_intent_precedence_shift_over_ctrl_over_open() {
    assert_eq!(tree_click_intent(false, false), ClickIntent::Open);
    assert_eq!(tree_click_intent(false, true), ClickIntent::Toggle);
    assert_eq!(tree_click_intent(true, false), ClickIntent::Range);
    assert_eq!(tree_click_intent(true, true), ClickIntent::Range);
  }

  #[test]
  fn folder_is_only_container_and_anti_cycle_is_enforced() {
    let f1 = Uuid::new_v4();
    let f2 = Uuid::new_v4();
    let p1 = Uuid::new_v4();
    let p2 = Uuid::new_v4();
    let rows = vec![
      mk_row(f1, "Folder 1", true, 0, None),
      mk_row(f2, "Folder 2", true, 1, Some(f1)),
      mk_row(p1, "Page 1", false, 2, Some(f2)),
      mk_row(p2, "Page 2", false, 0, None),
    ];

    // Page cannot accept Into
    assert!(!TreeDropPlanner::can_drop(&[p2], &rows[2], DropMode::Into, &rows));
    // Page CAN accept Before / After (its parent is Folder 2)
    assert!(TreeDropPlanner::can_drop(&[p2], &rows[2], DropMode::Before, &rows));
    assert!(TreeDropPlanner::can_drop(&[p2], &rows[2], DropMode::After, &rows));

    // Folder CAN accept Into for non-ancestor
    assert!(TreeDropPlanner::can_drop(&[p2], &rows[0], DropMode::Into, &rows));
    assert!(TreeDropPlanner::can_drop(&[p2], &rows[1], DropMode::Into, &rows));

    // Anti-cycle: folder cannot drop onto itself or its descendants (any mode)
    assert!(!TreeDropPlanner::can_drop(&[f1], &rows[0], DropMode::Into, &rows));
    assert!(!TreeDropPlanner::can_drop(&[f1], &rows[1], DropMode::Into, &rows));
    assert!(!TreeDropPlanner::can_drop(&[f1], &rows[1], DropMode::Before, &rows));
    assert!(!TreeDropPlanner::can_drop(&[f1], &rows[2], DropMode::After, &rows));

    // Batch containing ancestor f1 and unrelated p2 must also be rejected on f2
    assert!(!TreeDropPlanner::can_drop(&[p2, f1], &rows[1], DropMode::Into, &rows));
  }

  #[test]
  fn shift_range_selects_same_parent_siblings_and_skips_nested_children() {
    let f1 = Uuid::new_v4();
    let child_a = Uuid::new_v4();
    let f2 = Uuid::new_v4();
    let p_root = Uuid::new_v4();
    let rows = vec![
      mk_row(f1, "F1", true, 0, None),
      mk_row(child_a, "Child A", false, 1, Some(f1)),
      mk_row(f2, "F2", true, 0, None),
      mk_row(p_root, "P Root", false, 0, None),
    ];

    let mut sel = TreeSelection::new();
    sel.anchor_on(f1);
    sel.select_range(p_root, &rows);
    // Must include f1, f2, p_root — and NOT child_a!
    assert_eq!(sel.ids, vec![f1, f2, p_root]);
    assert!(sel.acts_on_selection(f2));
    assert!(!sel.acts_on_selection(child_a));

    // Cross-parent Shift+Click degrades to toggle on child_a
    sel.select_range(child_a, &rows);
    assert!(sel.contains(child_a));
    assert_eq!(sel.anchor, Some(child_a));
  }

  #[test]
  fn retain_visible_drops_collapsed_children_and_stale_anchor() {
    let f1 = Uuid::new_v4();
    let c1 = Uuid::new_v4();
    let p2 = Uuid::new_v4();
    let full = vec![
      mk_row(f1, "F1", true, 0, None),
      mk_row(c1, "C1", false, 1, Some(f1)),
      mk_row(p2, "P2", false, 0, None),
    ];
    let collapsed = vec![
      mk_row(f1, "F1", true, 0, None),
      mk_row(p2, "P2", false, 0, None),
    ];

    let mut sel = TreeSelection::new();
    sel.toggle(p2);
    sel.toggle(c1);
    assert_eq!(sel.anchor, Some(c1));
    assert!(!sel.retain_visible(&full));
    assert!(sel.retain_visible(&collapsed));
    assert_eq!(sel.ids, vec![p2]);
    assert_eq!(sel.anchor, None);
  }

  #[test]
  fn independent_selection_preserves_visible_order_and_drops_nested_descendants() {
    let f1 = Uuid::new_v4();
    let f2 = Uuid::new_v4();
    let p_deep = Uuid::new_v4();
    let p_other = Uuid::new_v4();
    let rows = vec![
      mk_row(f1, "F1", true, 0, None),
      mk_row(f2, "F2", true, 1, Some(f1)),
      mk_row(p_deep, "Deep", false, 2, Some(f2)),
      mk_row(p_other, "Other", false, 0, None),
    ];

    // Selected in reverse click order: p_other, p_deep, f1
    let res = independent_selection(&[p_other, p_deep, f1], &rows);
    // Visible tree order: f1 comes before p_other; p_deep is dropped because f1 is selected
    assert_eq!(res, vec![f1, p_other]);
  }

  #[test]
  fn sibling_order_after_drop_handles_before_after_and_batch_move() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();
    let d = Uuid::new_v4();
    let siblings = vec![a, b, c, d];

    // Move [a, b] after c -> [c, a, b, d]
    assert_eq!(
      TreeDropPlanner::sibling_order_after_drop(&siblings, &[a, b], c, false),
      vec![c, a, b, d]
    );
    // Move [c, d] before b -> [a, c, d, b]
    assert_eq!(
      TreeDropPlanner::sibling_order_after_drop(&siblings, &[c, d], b, true),
      vec![a, c, d, b]
    );
    // Dropping onto one of the dragged rows is a no-op
    assert_eq!(
      TreeDropPlanner::sibling_order_after_drop(&siblings, &[a, b], a, true),
      siblings
    );
  }

  #[test]
  fn root_drop_order_moves_existing_root_items_to_end_without_duplication() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();
    let nested = Uuid::new_v4();
    assert_eq!(
      TreeDropPlanner::root_drop_order(&[a, b, c], &[a, nested]),
      vec![b, c, a, nested]
    );
  }

  #[test]
  fn reparent_level_and_resolve_after_target_pop_out_to_ancestors() {
    let f1 = Uuid::new_v4();
    let f2 = Uuid::new_v4();
    let leaf = Uuid::new_v4();
    let rows = vec![
      mk_row(f1, "F1", true, 0, None),
      mk_row(f2, "F2", true, 1, Some(f1)),
      mk_row(leaf, "Leaf", false, 2, Some(f2)),
    ];

    // Last row (next_row_depth = None): can pop out all the way to level 0
    assert_eq!(TreeDropPlanner::reparent_level_for(20.0, 2, None), 0);
    assert_eq!(TreeDropPlanner::reparent_level_for(36.0, 2, None), 1);
    assert_eq!(TreeDropPlanner::reparent_level_for(52.0, 2, None), 2);
    assert_eq!(TreeDropPlanner::reparent_level_for(200.0, 2, None), 2);

    // Bounded by next sibling at depth 1: cannot pop out to level 0
    assert_eq!(TreeDropPlanner::reparent_level_for(0.0, 2, Some(1)), 1);

    // Expanded folder whose next row is deeper: locked to row_depth
    assert_eq!(TreeDropPlanner::reparent_level_for(0.0, 0, Some(1)), 0);

    // Resolve ancestor target from leaf (index 2, depth 2)
    assert_eq!(TreeDropPlanner::resolve_after_target(2, 2, &rows), leaf);
    assert_eq!(TreeDropPlanner::resolve_after_target(2, 1, &rows), f2);
    assert_eq!(TreeDropPlanner::resolve_after_target(2, 0, &rows), f1);
  }

  #[test]
  fn drop_zone_splits_30_40_30_and_renamed_to_validates() {
    assert_eq!(TreeDropPlanner::drop_zone_for_local_y(0.0, 30.0), DropMode::Before);
    assert_eq!(TreeDropPlanner::drop_zone_for_local_y(8.9, 30.0), DropMode::Before);
    assert_eq!(TreeDropPlanner::drop_zone_for_local_y(9.0, 30.0), DropMode::Into);
    assert_eq!(TreeDropPlanner::drop_zone_for_local_y(20.9, 30.0), DropMode::Into);
    assert_eq!(TreeDropPlanner::drop_zone_for_local_y(21.0, 30.0), DropMode::After);
    assert_eq!(TreeDropPlanner::drop_zone_for_local_y(30.0, 30.0), DropMode::After);

    assert_eq!(renamed_to("   ", "Old"), None);
    assert_eq!(renamed_to("Old", "Old"), None);
    assert_eq!(renamed_to("  Old  ", "Old"), None);
    assert_eq!(renamed_to("  New Name  ", "Old"), Some("New Name".to_string()));
  }
}
