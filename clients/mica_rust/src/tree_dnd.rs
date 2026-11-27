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

use mica_client::{PageTreeModel, TreeRow};
use mica_core::tree_rules::{MissingParent, ParentLookup, judge_parent};
use uuid::Uuid;

/// Standard sidebar tree row height in points (matches `kSidebarRowHeight = 30`).
pub const TREE_ROW_HEIGHT: f32 = 30.0;

/// Full vertical pitch of each tree row including its `1px` top & bottom margins (`30 + 2`).
pub const TREE_ROW_PITCH: f32 = TREE_ROW_HEIGHT + 2.0;

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

/// Computes continuous 60fps vertical edge auto-scroll velocity (in px per 16ms
/// tick; negative scrolls up, positive scrolls down, `0.0` in the middle band)
/// when dragging tree rows near or past the top/bottom edge of the sidebar tree.
pub fn tree_drag_autoscroll_speed(local_y: f32, viewport_h: f32) -> f32 {
  if viewport_h <= 1.0 {
    return 0.0;
  }
  let zone = 44.0_f32.min(viewport_h * 0.25);
  const MIN_SPEED: f32 = 3.5;
  const MAX_SPEED: f32 = 14.0;
  const RAMP: f32 = 72.0;
  let ramp = |past: f32| {
    let t = (past / RAMP).clamp(0.0, 1.0);
    MIN_SPEED + t * (MAX_SPEED - MIN_SPEED)
  };
  if local_y < zone {
    -ramp(zone - local_y)
  } else if local_y > viewport_h - zone {
    ramp(local_y - (viewport_h - zone))
  } else {
    0.0
  }
}

/// Computes the next clamped scroll offset for a sidebar auto-scroll tick.
///
/// When `content_h` has not yet been reported by an `on_scroll` event (`<= 0.0`),
/// falls back to `row_count * TREE_ROW_PITCH` so dragging works on the very first
/// drag before any manual wheel scroll has occurred.
pub fn compute_next_scroll_y(
  cur_y: f32,
  speed: f32,
  content_h: f32,
  viewport_h: f32,
  row_count: usize,
) -> f32 {
  let effective_ch = if content_h > 0.0 {
    content_h
  } else {
    (row_count as f32) * TREE_ROW_PITCH
  };
  let max_y = (effective_ch - viewport_h.max(0.0)).max(0.0);
  (cur_y + speed).clamp(0.0, max_y)
}

/// Maps a scroll-adjusted vertical coordinate (`scroll_y + local_y`) to the
/// corresponding visible tree row index and row-local Y offset.
pub fn tree_drag_row_at_y(
  content_y: f32,
  row_pitch: f32,
  row_count: usize,
) -> Option<(usize, f32)> {
  if row_count == 0 || row_pitch <= 0.0 || content_y < 0.0 {
    return None;
  }
  let idx = (content_y / row_pitch).floor() as usize;
  if idx >= row_count {
    return None;
  }
  let local_y = (content_y - (idx as f32) * row_pitch).clamp(0.0, row_pitch);
  Some((idx, local_y))
}

/// Placement target resolved for creating a new page or folder in the sidebar.
///
/// Mirrors Mica Flutter `v0.14.11` (`_createLocated`, `_createInFolder`,
/// `createParentForLocated`, `orderedSiblingsPlacingAfter`):
/// - Focused folder -> inside that folder, appended at the end (`after_view_id: None`).
/// - Focused/active page -> sibling under that page's folder (or root), directly below
///   that page (`after_view_id: Some(page.view_id)`).
/// - Root focused (clicked blank area below tree) or nothing located -> workspace root end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreatePlacement {
  pub parent_id: Option<Uuid>,
  pub after_view_id: Option<Uuid>,
}

fn effective_parent_id(row: &TreeRow, all_live_rows: &[TreeRow]) -> Option<Uuid> {
  row.parent_view_id.filter(|&pid| {
    all_live_rows
      .iter()
      .any(|r| r.view_id == pid && r.is_folder)
  })
}

/// Resolve the currently located live tree row (page or folder) for sidebar creation.
///
/// Returns `None` when `root_focused` is `true` (user explicitly clicked empty space
/// below the tree to target the workspace root, even while a document remains open).
pub fn resolve_located_row(
  all_live_rows: &[TreeRow],
  focused_nav_id: Option<Uuid>,
  root_focused: bool,
  active_doc_object_id: Option<Uuid>,
) -> Option<&TreeRow> {
  if root_focused {
    return None;
  }
  if let Some(fid) = focused_nav_id
    && let Some(row) = all_live_rows.iter().find(|r| r.view_id == fid)
  {
    return Some(row);
  }
  if let Some(doc_id) = active_doc_object_id {
    return all_live_rows
      .iter()
      .find(|r| !r.is_folder && r.object_id == doc_id);
  }
  None
}

/// Resolve `(parent_id, after_view_id)` for top-bar `+` (New Page) and `folder+` (New Folder).
pub fn placement_for_located(
  all_live_rows: &[TreeRow],
  focused_nav_id: Option<Uuid>,
  root_focused: bool,
  active_doc_object_id: Option<Uuid>,
) -> CreatePlacement {
  let Some(located) =
    resolve_located_row(all_live_rows, focused_nav_id, root_focused, active_doc_object_id)
  else {
    return CreatePlacement {
      parent_id: None,
      after_view_id: None,
    };
  };
  if located.is_folder {
    CreatePlacement {
      parent_id: Some(located.view_id),
      after_view_id: None,
    }
  } else {
    CreatePlacement {
      parent_id: effective_parent_id(located, all_live_rows),
      after_view_id: Some(located.view_id),
    }
  }
}

/// Resolve `(parent_id, after_view_id)` when clicking a specific folder row's `+` / `folder+`
/// button or its "新建子页面 / 新建子文件夹" context menu item.
///
/// Always places inside `folder_id`; if the currently located node is a page sitting
/// directly inside `folder_id`, lands right below that page rather than at folder end.
pub fn placement_in_folder(
  all_live_rows: &[TreeRow],
  folder_id: Uuid,
  focused_nav_id: Option<Uuid>,
  root_focused: bool,
  active_doc_object_id: Option<Uuid>,
) -> CreatePlacement {
  if !all_live_rows
    .iter()
    .any(|r| r.view_id == folder_id && r.is_folder)
  {
    return CreatePlacement {
      parent_id: None,
      after_view_id: None,
    };
  }
  let after_view_id =
    resolve_located_row(all_live_rows, focused_nav_id, root_focused, active_doc_object_id)
      .filter(|loc| !loc.is_folder && effective_parent_id(loc, all_live_rows) == Some(folder_id))
      .map(|loc| loc.view_id);
  CreatePlacement {
    parent_id: Some(folder_id),
    after_view_id,
  }
}

/// Compute the desired sibling order under `parent_id` placing `created_view_id`
/// immediately after `anchor_view_id`.
///
/// Returns `None` when `anchor_view_id` is missing from `parent_id`'s live siblings or
/// when `created_view_id` already sits immediately after `anchor_view_id` (avoiding a
/// redundant `reorder_views` round-trip).
pub fn ordered_siblings_placing_after(
  all_live_rows: &[TreeRow],
  parent_id: Option<Uuid>,
  anchor_view_id: Uuid,
  created_view_id: Uuid,
) -> Option<Vec<Uuid>> {
  let current_siblings: Vec<Uuid> = all_live_rows
    .iter()
    .filter(|r| effective_parent_id(r, all_live_rows) == parent_id)
    .map(|r| r.view_id)
    .collect();
  let without_created: Vec<Uuid> = current_siblings
    .iter()
    .copied()
    .filter(|&id| id != created_view_id)
    .collect();
  let anchor_idx = without_created.iter().position(|&id| id == anchor_view_id)?;
  let mut desired = Vec::with_capacity(without_created.len() + 1);
  desired.extend_from_slice(&without_created[..=anchor_idx]);
  desired.push(created_view_id);
  desired.extend_from_slice(&without_created[anchor_idx + 1..]);
  if current_siblings == desired {
    None
  } else {
    Some(desired)
  }
}

/// Compute the desired sibling order under `parent_id` ensuring `created_view_id` lands
/// at the end, returning `None` if it is already last.
pub fn ordered_siblings_appending(
  all_live_rows: &[TreeRow],
  parent_id: Option<Uuid>,
  created_view_id: Uuid,
) -> Option<Vec<Uuid>> {
  let current_siblings: Vec<Uuid> = all_live_rows
    .iter()
    .filter(|r| effective_parent_id(r, all_live_rows) == parent_id)
    .map(|r| r.view_id)
    .collect();
  if current_siblings.last() == Some(&created_view_id) {
    return None;
  }
  let mut desired: Vec<Uuid> = current_siblings
    .into_iter()
    .filter(|&id| id != created_view_id)
    .collect();
  desired.push(created_view_id);
  Some(desired)
}

/// Compute the next 10-digit zero-padded sibling position under `parent_id` strictly
/// greater than every existing numeric sibling position in `tree`.
pub fn next_sibling_position(tree: &PageTreeModel, parent_id: Option<Uuid>) -> String {
  let all_rows = tree.all_live_rows();
  let mut max_num: i64 = 0;
  let mut count: i64 = 0;
  for row in &all_rows {
    if effective_parent_id(row, &all_rows) == parent_id {
      count += 1;
      if let Some(view) = tree.get(row.view_id)
        && let Ok(n) = view.position.trim().parse::<i64>()
      {
        max_num = max_num.max(n);
      }
    }
  }
  let next = (max_num + 10).max((count + 1) * 10);
  format!("{next:010}")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tree_drag_autoscroll_speed_ramps_near_and_past_edges() {
    assert_eq!(tree_drag_autoscroll_speed(200.0, 400.0), 0.0);
    assert_eq!(tree_drag_autoscroll_speed(50.0, 0.0), 0.0);

    let top_inner = tree_drag_autoscroll_speed(20.0, 400.0);
    let top_past = tree_drag_autoscroll_speed(-30.0, 400.0);
    assert!(top_inner < 0.0, "near top edge must scroll up: {top_inner}");
    assert!(
      top_past < top_inner,
      "past top edge ({top_past}) must scroll faster than inside zone ({top_inner})"
    );

    let bot_inner = tree_drag_autoscroll_speed(380.0, 400.0);
    let bot_past = tree_drag_autoscroll_speed(430.0, 400.0);
    assert!(bot_inner > 0.0, "near bottom edge must scroll down: {bot_inner}");
    assert!(
      bot_past > bot_inner,
      "past bottom edge ({bot_past}) must scroll faster than inside zone ({bot_inner})"
    );
  }

  #[test]
  fn compute_next_scroll_y_estimates_content_before_first_scroll_and_clamps() {
    // 30 rows * 32px = 960px content in a 320px viewport -> max scroll 640px.
    let y1 = compute_next_scroll_y(0.0, 10.0, 0.0, 320.0, 30);
    assert!((y1 - 10.0).abs() < 0.01);

    // Clamps at bottom when content_h is measured.
    let y_bot = compute_next_scroll_y(635.0, 12.0, 960.0, 320.0, 30);
    assert!((y_bot - 640.0).abs() < 0.01);

    // Clamps at top.
    let y_top = compute_next_scroll_y(4.0, -12.0, 960.0, 320.0, 30);
    assert_eq!(y_top, 0.0);

    // Non-scrollable short list never scrolls past 0.
    assert_eq!(compute_next_scroll_y(0.0, 10.0, 0.0, 400.0, 5), 0.0);
  }

  #[test]
  fn tree_drag_row_at_y_maps_scroll_offset_to_row_and_local_y() {
    assert_eq!(tree_drag_row_at_y(-1.0, TREE_ROW_PITCH, 10), None);
    assert_eq!(tree_drag_row_at_y(0.0, TREE_ROW_PITCH, 0), None);
    assert_eq!(tree_drag_row_at_y(320.0, TREE_ROW_PITCH, 10), None);

    let (idx, ly) = tree_drag_row_at_y(32.0 * 4.0 + 15.0, TREE_ROW_PITCH, 10).unwrap();
    assert_eq!(idx, 4);
    assert!((ly - 15.0).abs() < 0.01);
  }

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

  #[test]
  fn placement_for_located_handles_folder_page_and_root_focus() {
    let f1 = Uuid::new_v4();
    let p1 = Uuid::new_v4();
    let p2 = Uuid::new_v4();
    let p_root = Uuid::new_v4();
    let rows = vec![
      mk_row(f1, "Folder 1", true, 0, None),
      mk_row(p1, "Page 1", false, 1, Some(f1)),
      mk_row(p2, "Page 2", false, 1, Some(f1)),
      mk_row(p_root, "Root Page", false, 0, None),
    ];

    // 1. Focused folder -> inside that folder, appended at end.
    assert_eq!(
      placement_for_located(&rows, Some(f1), false, Some(p1_obj(&rows, p1))),
      CreatePlacement {
        parent_id: Some(f1),
        after_view_id: None,
      }
    );

    // 2. Focused page inside folder -> sibling inside same folder, directly after page.
    assert_eq!(
      placement_for_located(&rows, Some(p1), false, None),
      CreatePlacement {
        parent_id: Some(f1),
        after_view_id: Some(p1),
      }
    );

    // 3. Fallback to active_doc when focused_nav_id is None.
    assert_eq!(
      placement_for_located(&rows, None, false, Some(p1_obj(&rows, p_root))),
      CreatePlacement {
        parent_id: None,
        after_view_id: Some(p_root),
      }
    );

    // 4. Root focused (clicked blank area below tree) -> workspace root even with open doc.
    assert_eq!(
      placement_for_located(&rows, Some(f1), true, Some(p1_obj(&rows, p1))),
      CreatePlacement {
        parent_id: None,
        after_view_id: None,
      }
    );

    // 5. Nothing focused and no active doc -> workspace root.
    assert_eq!(
      placement_for_located(&rows, None, false, None),
      CreatePlacement {
        parent_id: None,
        after_view_id: None,
      }
    );
  }

  fn p1_obj(rows: &[TreeRow], vid: Uuid) -> Uuid {
    rows.iter().find(|r| r.view_id == vid).unwrap().object_id
  }

  #[test]
  fn placement_in_folder_anchors_after_direct_child_page_only() {
    let f1 = Uuid::new_v4();
    let f2 = Uuid::new_v4();
    let p1 = Uuid::new_v4();
    let p2 = Uuid::new_v4();
    let rows = vec![
      mk_row(f1, "F1", true, 0, None),
      mk_row(p1, "P1", false, 1, Some(f1)),
      mk_row(f2, "F2", true, 1, Some(f1)),
      mk_row(p2, "P2", false, 2, Some(f2)),
    ];

    // Located page is direct child of f1 -> lands right after p1 inside f1.
    assert_eq!(
      placement_in_folder(&rows, f1, Some(p1), false, None),
      CreatePlacement {
        parent_id: Some(f1),
        after_view_id: Some(p1),
      }
    );

    // Located page is in nested child folder f2, NOT direct child of f1 -> appends to f1 end.
    assert_eq!(
      placement_in_folder(&rows, f1, Some(p2), false, None),
      CreatePlacement {
        parent_id: Some(f1),
        after_view_id: None,
      }
    );

    // Located node is a folder itself (f2) -> appends to f1 end.
    assert_eq!(
      placement_in_folder(&rows, f1, Some(f2), false, None),
      CreatePlacement {
        parent_id: Some(f1),
        after_view_id: None,
      }
    );

    // Root focused -> appends to f1 end (explicit folder button still creates inside f1).
    assert_eq!(
      placement_in_folder(&rows, f1, Some(p1), true, None),
      CreatePlacement {
        parent_id: Some(f1),
        after_view_id: None,
      }
    );
  }

  #[test]
  fn ordered_siblings_placing_after_inserts_next_line_and_skips_when_already_adjacent() {
    let f1 = Uuid::new_v4();
    let p1 = Uuid::new_v4();
    let p2 = Uuid::new_v4();
    let p3 = Uuid::new_v4();
    let created = Uuid::new_v4();

    // `created` currently appended at end of f1: [p1, p2, p3, created]
    let rows = vec![
      mk_row(f1, "F1", true, 0, None),
      mk_row(p1, "P1", false, 1, Some(f1)),
      mk_row(p2, "P2", false, 1, Some(f1)),
      mk_row(p3, "P3", false, 1, Some(f1)),
      mk_row(created, "New", false, 1, Some(f1)),
    ];

    // Anchor p1 (first) -> [p1, created, p2, p3]
    assert_eq!(
      ordered_siblings_placing_after(&rows, Some(f1), p1, created),
      Some(vec![p1, created, p2, p3])
    );

    // Anchor p2 (middle) -> [p1, p2, created, p3]
    assert_eq!(
      ordered_siblings_placing_after(&rows, Some(f1), p2, created),
      Some(vec![p1, p2, created, p3])
    );

    // Anchor p3 (already immediately before created) -> None (no redundant reorder)
    assert_eq!(
      ordered_siblings_placing_after(&rows, Some(f1), p3, created),
      None
    );

    // If created somehow sorted BEFORE last sibling p3: [p1, p2, created, p3],
    // anchoring after p3 still emits [p1, p2, p3, created].
    let misordered = vec![
      mk_row(f1, "F1", true, 0, None),
      mk_row(p1, "P1", false, 1, Some(f1)),
      mk_row(p2, "P2", false, 1, Some(f1)),
      mk_row(created, "New", false, 1, Some(f1)),
      mk_row(p3, "P3", false, 1, Some(f1)),
    ];
    assert_eq!(
      ordered_siblings_placing_after(&misordered, Some(f1), p3, created),
      Some(vec![p1, p2, p3, created])
    );
    assert_eq!(
      ordered_siblings_appending(&misordered, Some(f1), created),
      Some(vec![p1, p2, p3, created])
    );
    assert_eq!(ordered_siblings_appending(&rows, Some(f1), created), None);
  }
}
