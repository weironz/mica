//! Workspace page-tree ViewModel backed directly by `mica_core::tree_rules`.
//!
//! Enforces the single-authority invariant: folders are the only containers and
//! pages are always leaves; cycle checks mirror `mica_core::tree_rules`.

use std::collections::{HashMap, HashSet};

use mica_api_types::View;
use mica_core::tree_rules::{MissingParent, ParentLookup, ParentRefusal, judge_parent};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Flattened visible row for virtualized sidebar rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeRow {
  pub view_id: Uuid,
  pub object_id: Uuid,
  pub name: String,
  pub icon: Option<String>,
  pub is_folder: bool,
  pub depth: usize,
  pub has_children: bool,
  pub expanded: bool,
  pub parent_view_id: Option<Uuid>,
}

/// Pure-Rust page-tree ViewModel for a single workspace.
#[derive(Debug, Clone, Default)]
pub struct PageTreeModel {
  views: HashMap<Uuid, View>,
  expanded_folders: HashSet<Uuid>,
}

impl PageTreeModel {
  pub fn new(views: Vec<View>) -> Self {
    let mut model = Self::default();
    model.replace_views(views);
    model
  }

  pub fn replace_views(&mut self, views: Vec<View>) {
    self.views = views.into_iter().map(|v| (v.id, v)).collect();
    self.expanded_folders.retain(|id| {
      self
        .views
        .get(id)
        .is_some_and(|v| !v.is_deleted && v.object_type == "folder")
    });
  }

  pub fn upsert_view(&mut self, view: View) {
    self.views.insert(view.id, view);
  }

  pub fn get(&self, view_id: Uuid) -> Option<&View> {
    self.views.get(&view_id)
  }

  pub fn set_expanded(&mut self, folder_id: Uuid, expanded: bool) {
    if expanded {
      self.expanded_folders.insert(folder_id);
    } else {
      self.expanded_folders.remove(&folder_id);
    }
  }

  pub fn toggle_expanded(&mut self, folder_id: Uuid) -> bool {
    if self.expanded_folders.remove(&folder_id) {
      false
    } else {
      self.expanded_folders.insert(folder_id);
      true
    }
  }

  /// Exact same page-count definition as `mica_api_types::Workspace::page_count`:
  /// live pages only — folders excluded, recycle bin excluded.
  pub fn count_live_pages(&self) -> usize {
    self
      .views
      .values()
      .filter(|v| !v.is_deleted && v.object_type != "folder")
      .count()
  }

  /// Recycle-bin entries sorted by `updated_at` descending.
  pub fn trashed_views(&self) -> Vec<&View> {
    let mut out: Vec<&View> = self.views.values().filter(|v| v.is_deleted).collect();
    out.sort_by(|a, b| {
      b.updated_at
        .cmp(&a.updated_at)
        .then_with(|| a.name.cmp(&b.name))
    });
    out
  }

  /// Validate whether `view_id` can be moved under `target_parent` using
  /// `mica_core::tree_rules::judge_parent` plus cycle detection.
  pub fn validate_move(
    &self,
    view_id: Uuid,
    target_parent: Option<Uuid>,
  ) -> Result<(), ParentRefusal> {
    let Some(parent_id) = target_parent else {
      return Ok(());
    };
    let lookup = match self.views.get(&parent_id) {
      Some(p) if !p.is_deleted && p.object_type == "folder" => ParentLookup::Folder,
      Some(p) if !p.is_deleted => ParentLookup::NotFolder,
      _ => ParentLookup::Missing,
    };
    judge_parent(view_id == parent_id, lookup, MissingParent::Refuse)?;

    // Walk up from target_parent to detect cycle into own subtree.
    let mut cursor = Some(parent_id);
    let mut visited = HashSet::new();
    while let Some(curr) = cursor {
      if curr == view_id {
        return Err(ParentRefusal::IntoOwnSubtree);
      }
      if !visited.insert(curr) {
        break;
      }
      cursor = self.views.get(&curr).and_then(|v| v.parent_view_id);
    }
    Ok(())
  }

  /// Breadcrumb path from root to `view_id` (inclusive), cycle-safe.
  pub fn breadcrumbs(&self, view_id: Uuid) -> Vec<&View> {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = Some(view_id);
    while let Some(id) = cursor {
      if !seen.insert(id) {
        break;
      }
      let Some(v) = self.views.get(&id) else {
        break;
      };
      if v.is_deleted {
        break;
      }
      chain.push(v);
      cursor = v.parent_view_id;
    }
    chain.reverse();
    chain
  }

  /// Expand all ancestor folders of `view_id` so it becomes visible in the sidebar.
  pub fn reveal(&mut self, view_id: Uuid) {
    let ancestors: Vec<Uuid> = self
      .breadcrumbs(view_id)
      .into_iter()
      .filter(|v| v.id != view_id && v.object_type == "folder")
      .map(|v| v.id)
      .collect();
    for id in ancestors {
      self.expanded_folders.insert(id);
    }
  }

  /// Flatten visible live tree rows in sidebar order (folders/pages sorted by
  /// `position`, then `created_at`, then `name`).
  pub fn visible_rows(&self) -> Vec<TreeRow> {
    self.flatten_rows(false)
  }

  /// Flatten ALL live (non-deleted) tree rows in sidebar order, regardless of
  /// whether ancestor folders are currently collapsed.
  pub fn all_live_rows(&self) -> Vec<TreeRow> {
    self.flatten_rows(true)
  }

  fn flatten_rows(&self, include_collapsed: bool) -> Vec<TreeRow> {
    let mut children_by_parent: HashMap<Option<Uuid>, Vec<&View>> = HashMap::new();
    for v in self.views.values() {
      if v.is_deleted {
        continue;
      }
      let effective_parent = v.parent_view_id.filter(|pid| {
        self
          .views
          .get(pid)
          .is_some_and(|p| !p.is_deleted && p.object_type == "folder")
      });
      children_by_parent
        .entry(effective_parent)
        .or_default()
        .push(v);
    }
    for list in children_by_parent.values_mut() {
      list.sort_by(|a, b| {
        a.position
          .cmp(&b.position)
          .then_with(|| a.created_at.cmp(&b.created_at))
          .then_with(|| a.name.cmp(&b.name))
      });
    }

    let mut out = Vec::new();
    let mut visited = HashSet::new();
    self.walk(
      None,
      0,
      include_collapsed,
      &children_by_parent,
      &mut visited,
      &mut out,
    );
    out
  }

  fn walk(
    &self,
    parent: Option<Uuid>,
    depth: usize,
    include_collapsed: bool,
    children_by_parent: &HashMap<Option<Uuid>, Vec<&View>>,
    visited: &mut HashSet<Uuid>,
    out: &mut Vec<TreeRow>,
  ) {
    let Some(children) = children_by_parent.get(&parent) else {
      return;
    };
    for v in children {
      if !visited.insert(v.id) {
        continue;
      }
      let is_folder = v.object_type == "folder";
      let has_children = is_folder
        && children_by_parent
          .get(&Some(v.id))
          .is_some_and(|c| !c.is_empty());
      let expanded = is_folder && self.expanded_folders.contains(&v.id);
      out.push(TreeRow {
        view_id: v.id,
        object_id: v.object_id,
        name: v.name.clone(),
        icon: v.icon.clone(),
        is_folder,
        depth,
        has_children,
        expanded,
        parent_view_id: v.parent_view_id,
      });
      if expanded || (include_collapsed && is_folder) {
        self.walk(
          Some(v.id),
          depth + 1,
          include_collapsed,
          children_by_parent,
          visited,
          out,
        );
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::Utc;

  fn mk_view(name: &str, object_type: &str, parent: Option<Uuid>, pos: &str) -> View {
    let now = Utc::now();
    View {
      id: Uuid::new_v4(),
      workspace_id: Uuid::new_v4(),
      parent_view_id: parent,
      object_id: Uuid::new_v4(),
      object_type: object_type.into(),
      name: name.into(),
      icon: None,
      position: pos.into(),
      is_deleted: false,
      created_by: Uuid::new_v4(),
      created_at: now,
      updated_at: now,
      state_bytes: None,
    }
  }

  #[test]
  fn folder_is_only_container_and_cycle_is_rejected() {
    let folder_a = mk_view("A", "folder", None, "a");
    let folder_b = mk_view("B", "folder", Some(folder_a.id), "a");
    let page = mk_view("P", "page", Some(folder_b.id), "a");
    let mut trashed = mk_view("Trash", "folder", None, "z");
    trashed.is_deleted = true;

    let tree = PageTreeModel::new(vec![
      folder_a.clone(),
      folder_b.clone(),
      page.clone(),
      trashed.clone(),
    ]);

    assert_eq!(tree.count_live_pages(), 1);
    assert_eq!(tree.validate_move(page.id, Some(folder_a.id)), Ok(()));
    assert_eq!(
      tree.validate_move(folder_a.id, Some(page.id)),
      Err(ParentRefusal::NotAFolder)
    );
    assert_eq!(
      tree.validate_move(folder_a.id, Some(folder_a.id)),
      Err(ParentRefusal::SelfParent)
    );
    assert_eq!(
      tree.validate_move(folder_a.id, Some(folder_b.id)),
      Err(ParentRefusal::IntoOwnSubtree)
    );
    assert_eq!(
      tree.validate_move(page.id, Some(trashed.id)),
      Err(ParentRefusal::NotFound)
    );
  }

  #[test]
  fn reveal_expands_ancestors_and_flattens_visible_rows() {
    let f1 = mk_view("Folder 1", "folder", None, "a");
    let f2 = mk_view("Folder 2", "folder", Some(f1.id), "a");
    let p = mk_view("Leaf Page", "page", Some(f2.id), "a");
    let mut tree = PageTreeModel::new(vec![f1.clone(), f2.clone(), p.clone()]);

    assert_eq!(tree.visible_rows().len(), 1);
    assert!(!tree.visible_rows()[0].expanded);
    let all = tree.all_live_rows();
    assert_eq!(all.len(), 3);
    assert_eq!(all[2].view_id, p.id);
    tree.reveal(p.id);
    let rows = tree.visible_rows();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].depth, 0);
    assert_eq!(rows[1].depth, 1);
    assert_eq!(rows[2].depth, 2);
    assert_eq!(rows[2].view_id, p.id);
    assert_eq!(tree.breadcrumbs(p.id).len(), 3);
  }
}
