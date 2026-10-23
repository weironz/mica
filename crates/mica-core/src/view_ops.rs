//! Page-tree (view) subtree operations for the local store — ONE transaction
//! per operation (issue #67).
//!
//! These used to live in the Flutter FFI layer (`clients/mica_flutter/rust/src/
//! api/store.rs`): every operation re-listed the whole `local_view` table, then
//! wrote rows one at a time with no surrounding transaction — O(subtree ×
//! whole library) reads, and a crash mid-loop left a half-trashed, half- moved
//! or half-cloned tree on disk. They now run here, against the connection
//! directly:
//!
//! - the subtree is collected by ONE recursive CTE (the same shape the
//!   server's cascade already uses), not by a BFS re-scanning the full table
//!   per node;
//! - every write of the operation commits in a single
//!   `unchecked_transaction`, so the disk never shows half an operation.
//!
//! Parent/position/lookup reads stay single POINT queries — a write passing
//! `parent_accepts_children` used to cost a full `list_views` scan each time.

use rusqlite::{params, OptionalExtension, Row};

use crate::doc::MicaDoc;
use crate::store::{LocalStore, LocalView, StoreError};

/// What [`LocalStore::clone_view`] produced: where the copy landed, the name it
/// settled on after sibling dedup, and how many documents were actually copied
/// (folders and doc-less views do not count). The FFI maps this one-to-one
/// onto its wire `CloneViewResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneViewOutcome {
  pub root_view_id: String,
  pub new_name: String,
  pub docs: u32,
}

/// `base` if free among `siblings`, else `base 2`, `base 3`, … — the number is
/// locale-neutral, the caller supplies the localized base.
///
/// Same rule as the server's `dedup_sibling_name` (api-server documents.rs) so
/// a page cloned offline and one cloned in the cloud settle on the same name.
/// Kept a free function: pure, and that is what makes it testable without a
/// store.
pub fn dedup_sibling_name(base: &str, siblings: &[String]) -> String {
  if !siblings.iter().any(|s| s == base) {
    return base.to_string();
  }
  (2..)
    .map(|n| format!("{base} {n}"))
    .find(|c| !siblings.iter().any(|s| s == c))
    .expect("an unused suffix always exists")
}

/// The full column list in the same order [`LocalStore::list_views`] reads
/// them, shared by the row queries below.
fn view_from_row(r: &Row<'_>) -> rusqlite::Result<LocalView> {
  Ok(LocalView {
    id: r.get(0)?,
    workspace_id: r.get(1)?,
    parent_id: r.get(2)?,
    object_id: r.get(3)?,
    name: r.get(4)?,
    position: r.get(5)?,
    trashed: r.get::<_, i64>(6)? != 0,
    origin: r.get(7)?,
    object_type: r.get(8)?,
  })
}

const VIEW_COLUMNS: &str =
  "id, workspace_id, parent_id, object_id, name, position, trashed, origin, object_type";

impl LocalStore {
  /// True when a view may live under `parent_id`: at the top level, or under
  /// a FOLDER. A parent the store cannot see (not yet written) is accepted —
  /// this guards the shape that is visible here, and the server checks again
  /// on the way up. ONE indexed lookup; this used to list the origin's whole
  /// table to answer the same question (issue #67).
  pub fn parent_accepts_children(
    &self,
    origin: &str,
    parent_id: Option<&str>,
  ) -> Result<bool, StoreError> {
    let Some(parent_id) = parent_id else {
      return Ok(true);
    };
    let object_type: Option<String> = self
      .conn
      .query_row(
        "SELECT object_type FROM local_view WHERE origin=?1 AND id=?2",
        params![origin, parent_id],
        |r| r.get(0),
      )
      .optional()?;
    Ok(object_type.as_deref().is_none_or(|t| t == "folder"))
  }

  /// Where the next child of `parent_id` goes: after the last LIVE sibling,
  /// in steps of ten, zero-padded to ten digits so string ordering is
  /// numeric ordering. The gap of ten is what leaves room to drop something
  /// between two siblings without renumbering them. Same rule the FFI layer
  /// used to spell out over a full `list_views` scan — now one query.
  pub fn next_child_position(
    &self,
    origin: &str,
    parent_id: Option<&str>,
  ) -> Result<String, StoreError> {
    // NULL-safe `IS`: a NULL parent_id matches top-level rows, a set one
    // behaves like `=`.
    let mut stmt = self.conn.prepare(
      "SELECT position FROM local_view WHERE origin=?1 AND trashed=0 AND parent_id IS ?2",
    )?;
    let positions: Vec<String> = stmt
      .query_map(params![origin, parent_id], |r| r.get(0))?
      .collect::<Result<Vec<_>, _>>()?;
    let max = positions
      .iter()
      .filter_map(|p| p.parse::<i64>().ok())
      .max()
      .unwrap_or(0);
    Ok(format!("{:010}", max + 10))
  }

  /// `root_id` plus every descendant, in `list_views` (position) order.
  ///
  /// The recursive walk every subtree operation shares (trash, restore,
  /// purge, clone), as ONE recursive CTE — the local mirror of the server's
  /// recursive-CTE cascade. `live_only` stops at trashed nodes like the
  /// server's `is_deleted = false` CTE: a trashed node is left out, and so
  /// is everything under it (the walk never descends from a node that was
  /// itself excluded). The UNION's built-in dedup replaces the visited set,
  /// so a parent cycle terminates instead of walking forever.
  pub fn subtree_ids(
    &self,
    origin: &str,
    root_id: &str,
    live_only: bool,
  ) -> Result<Vec<String>, StoreError> {
    Ok(
      self
        .subtree_rows(origin, root_id, live_only)?
        .into_iter()
        .map(|v| v.id)
        .collect(),
    )
  }

  /// [`Self::subtree_ids`] returning the full rows (position order).
  fn subtree_rows(
    &self,
    origin: &str,
    root_id: &str,
    live_only: bool,
  ) -> Result<Vec<LocalView>, StoreError> {
    // The walk must start from the root id even when no such row exists —
    // descendants pointing at a missing parent are still part of the
    // subtree (they are exactly what a purge must reach). In live_only
    // mode the root itself must exist AND be live.
    let anchor = if live_only {
      "SELECT ?2 WHERE EXISTS(
                 SELECT 1 FROM local_view WHERE origin=?1 AND id=?2 AND trashed=0)"
    } else {
      "SELECT ?2"
    };
    let live_filter = if live_only { "AND lv.trashed=0" } else { "" };
    let sql = format!(
      "WITH RECURSIVE subtree(id) AS (
                 {anchor}
                 UNION
                 SELECT lv.id FROM local_view lv JOIN subtree s ON lv.parent_id = s.id
                 WHERE lv.origin=?1 {live_filter}
             )
             SELECT lv.id, lv.workspace_id, lv.parent_id, lv.object_id, lv.name,
                    lv.position, lv.trashed, lv.origin, lv.object_type
             FROM local_view lv JOIN subtree s ON lv.id = s.id
             WHERE lv.origin=?1
             ORDER BY lv.position"
    );
    let mut stmt = self.conn.prepare(&sql)?;
    let rows = stmt
      .query_map(params![origin, root_id], view_from_row)?
      .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
  }

  /// Flip `trashed` on exactly `ids`, leaving every other field alone — one
  /// transaction (one commit) instead of one full-row upsert per node, so a
  /// crash mid-subtree cannot leave a half-trashed tree (issue #67). Ids
  /// with no row are skipped, like the filter over `list_views` did.
  pub fn set_views_trashed(
    &self,
    origin: &str,
    ids: &[String],
    trashed: bool,
  ) -> Result<(), StoreError> {
    if ids.is_empty() {
      return Ok(());
    }
    let tx = self.conn.unchecked_transaction()?;
    {
      let mut stmt = tx.prepare("UPDATE local_view SET trashed=?3 WHERE origin=?1 AND id=?2")?;
      for id in ids {
        stmt.execute(params![origin, id, trashed as i64])?;
      }
    }
    tx.commit()?;
    Ok(())
  }

  /// Move `root_id` and its whole subtree to the recycle bin, in one
  /// transaction. Returns the affected ids so the caller can close an editor
  /// that was inside it.
  ///
  /// A folder carries its children: trashing one must trash the subtree, or
  /// the children survive as orphans pointing at a trashed parent.
  pub fn trash_view_subtree(&self, origin: &str, root_id: &str) -> Result<Vec<String>, StoreError> {
    let ids = self.subtree_ids(origin, root_id, false)?;
    self.set_views_trashed(origin, &ids, true)?;
    Ok(ids)
  }

  /// Bring `root_id` and the subtree trashed with it back out of the bin, in
  /// one transaction. If the restored ROOT's parent is no longer a live
  /// view, the root is lifted to the top level — restoring into a trashed
  /// parent would leave it invisible with no way back. Mirrors the server's
  /// `restore_view`.
  pub fn restore_view_subtree(
    &self,
    origin: &str,
    root_id: &str,
  ) -> Result<Vec<String>, StoreError> {
    let ids = self.subtree_ids(origin, root_id, false)?;
    let tx = self.conn.unchecked_transaction()?;
    {
      let mut stmt = tx.prepare("UPDATE local_view SET trashed=0 WHERE origin=?1 AND id=?2")?;
      for id in &ids {
        stmt.execute(params![origin, id])?;
      }
    }
    // Lift the root if its parent is gone or still in the bin. The
    // un-trash above already landed in this transaction, so a parent
    // restored by it reads as live here. Whole-origin scope, no workspace
    // filter — same as the old FFI snapshot over `list_views(origin)`.
    let root_parent: Option<Option<String>> = tx
      .query_row(
        "SELECT parent_id FROM local_view WHERE origin=?1 AND id=?2",
        params![origin, root_id],
        |r| r.get(0),
      )
      .optional()?;
    if let Some(Some(parent_id)) = root_parent {
      let parent_live: bool = tx.query_row(
        "SELECT EXISTS(
                     SELECT 1 FROM local_view WHERE origin=?1 AND id=?2 AND trashed=0)",
        params![origin, parent_id],
        |r| r.get(0),
      )?;
      if !parent_live {
        tx.execute(
          "UPDATE local_view SET parent_id=NULL, trashed=0 WHERE origin=?1 AND id=?2",
          params![origin, root_id],
        )?;
      }
    }
    tx.commit()?;
    Ok(ids)
  }

  /// Permanently remove `root_id` and its subtree, documents included, in
  /// one transaction. Returns the affected ids. There is no undo past this
  /// point.
  pub fn purge_view_subtree(&self, origin: &str, root_id: &str) -> Result<Vec<String>, StoreError> {
    let ids = self.subtree_ids(origin, root_id, false)?;
    let tx = self.conn.unchecked_transaction()?;
    {
      let mut read = tx.prepare("SELECT object_id FROM local_view WHERE origin=?1 AND id=?2")?;
      let mut purge = tx.prepare("DELETE FROM local_view WHERE origin=?1 AND id=?2")?;
      for id in &ids {
        let object_id: Option<String> = read
          .query_row(params![origin, id], |r| r.get(0))
          .optional()?;
        if let Some(doc_id) = object_id {
          // Statements on this connection join the open transaction;
          // the commit below folds every document into the same
          // all-or-nothing delete as the view rows.
          self.delete_doc_rows(&doc_id)?;
        }
        purge.execute(params![origin, id])?;
      }
    }
    tx.commit()?;
    Ok(ids)
  }

  /// Renumber `ordered_ids` as consecutive children of `parent_id` — one
  /// transaction covering the whole move (issue #67). Positions restart at
  /// 10 and step by ten, matching [`Self::next_child_position`] so a later
  /// insert still lands after them. Rows already in the right place are left
  /// alone — an untouched row is one less write and one less sync update.
  /// Reparenting is part of the same move: dragging into another folder
  /// changes both. Ids with no row (a stale list dragged before a refresh)
  /// are skipped rather than inventing phantom rows.
  pub fn reorder_views(
    &self,
    origin: &str,
    parent_id: Option<&str>,
    ordered_ids: &[String],
  ) -> Result<(), StoreError> {
    let tx = self.conn.unchecked_transaction()?;
    {
      let mut read =
        tx.prepare("SELECT position, parent_id FROM local_view WHERE origin=?1 AND id=?2")?;
      let mut write = tx.prepare(
        "UPDATE local_view SET parent_id=?3, position=?4, trashed=0
                 WHERE origin=?1 AND id=?2",
      )?;
      for (i, id) in ordered_ids.iter().enumerate() {
        let position = format!("{:010}", (i as i64 + 1) * 10);
        let row: Option<(String, Option<String>)> = read
          .query_row(params![origin, id], |r| Ok((r.get(0)?, r.get(1)?)))
          .optional()?;
        let Some((cur_position, cur_parent)) = row else {
          continue;
        };
        if cur_position == position && cur_parent.as_deref() == parent_id {
          continue;
        }
        write.execute(params![origin, id, parent_id, position])?;
      }
    }
    tx.commit()?;
    Ok(())
  }

  /// Duplicate `view_id` and everything under it, beside the original — the
  /// WHOLE operation in ONE transaction (issue #67): subtree walk, fresh
  /// ids, doc copies, sibling-name dedup and positioning all commit
  /// together. The FFI layer used to do this as a list-scan plus one upsert
  /// per node with no transaction; a crash between two nodes left a
  /// half-copied tree behind.
  ///
  /// Returns `None` when the view is gone or in the recycle bin: the subtree
  /// walk enumerates LIVE rows only, like the server's copy — a trashed node
  /// stays behind, together with everything under it.
  pub fn clone_view(
    &self,
    origin: &str,
    view_id: &str,
    root_name: &str,
    client_id: u64,
  ) -> Result<Option<CloneViewOutcome>, StoreError> {
    // The root first: gone or trashed means nothing to clone (same as the
    // server). Reads happen BEFORE the transaction starts; a failure mid-
    // clone rolls back to this exact state.
    let root: Option<LocalView> = self
      .conn
      .query_row(
        &format!("SELECT {VIEW_COLUMNS} FROM local_view WHERE origin=?1 AND id=?2"),
        params![origin, view_id],
        view_from_row,
      )
      .optional()?;
    let Some(root) = root else {
      return Ok(None);
    };
    if root.trashed {
      return Ok(None);
    }

    let subtree = self.subtree_rows(origin, view_id, true)?;

    // Dedup against LIVE siblings under the same parent; position comes
    // from the shared rule so a clone lands exactly where a newly created
    // page would (`parent_id IS ?2` is the NULL-safe top-level match).
    let mut stmt = self
      .conn
      .prepare("SELECT name FROM local_view WHERE origin=?1 AND trashed=0 AND parent_id IS ?2")?;
    let sibling_names: Vec<String> = stmt
      .query_map(params![origin, root.parent_id], |r| r.get(0))?
      .collect::<Result<Vec<_>, _>>()?;
    let new_name = dedup_sibling_name(root_name, &sibling_names);
    let root_position = self.next_child_position(origin, root.parent_id.as_deref())?;

    // Fresh id per node. The ROOT keeps its parent (it lands beside the
    // original); inner nodes point at their COPIED parent.
    let id_map: std::collections::HashMap<String, String> = subtree
      .iter()
      .map(|v| (v.id.clone(), format!("view_{}", uuid::Uuid::new_v4())))
      .collect();

    let tx = self.conn.unchecked_transaction()?;
    let mut docs: u32 = 0;
    // This never fires — `id_map` covers every row of the subtree by
    // construction — but a silent fallback id would corrupt the tree.
    let fallback = String::new();
    for v in &subtree {
      let new_id = id_map.get(&v.id).unwrap_or(&fallback);
      let is_root = v.id == view_id;
      let new_doc_id = format!("doc_{}", uuid::Uuid::new_v4());
      match self.load_doc(&v.object_id, client_id) {
        Ok(Some(doc)) => {
          self.save_doc(&new_doc_id, &doc)?;
          docs += 1;
        }
        // A view with no document — or one whose snapshot won't decode —
        // still gets an empty one, or the copy opens to nothing. Same
        // construction the FFI used (`MicaDocument::from_markdown`).
        _ => {
          let block_root = format!("block_{}", uuid::Uuid::new_v4());
          let payload = mica_markdown::import_markdown("", &block_root);
          let doc = MicaDoc::from_blocks(&payload.root_block_id, &payload.blocks);
          self.save_doc(&new_doc_id, &doc)?;
        }
      }
      self.save_view(&LocalView {
        id: new_id.clone(),
        workspace_id: v.workspace_id.clone(),
        parent_id: if is_root {
          v.parent_id.clone()
        } else {
          v.parent_id
            .as_ref()
            .map(|p| id_map.get(p).cloned().unwrap_or_default())
        },
        object_id: new_doc_id,
        name: if is_root {
          new_name.clone()
        } else {
          v.name.clone()
        },
        position: if is_root {
          root_position.clone()
        } else {
          v.position.clone()
        },
        trashed: false,
        origin: v.origin.clone(),
        object_type: v.object_type.clone(),
      })?;
    }
    tx.commit()?;
    Ok(Some(CloneViewOutcome {
      root_view_id: id_map.get(view_id).cloned().unwrap_or_default(),
      new_name,
      docs,
    }))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::store::LocalWorkspace;

  fn tmp_store() -> (LocalStore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mica_view_ops_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = LocalStore::open(dir.join("s.db").to_str().unwrap()).unwrap();
    (store, dir)
  }

  fn view(id: &str, parent: Option<&str>, name: &str, pos: &str) -> LocalView {
    LocalView {
      id: id.into(),
      workspace_id: "ws".into(),
      parent_id: parent.map(str::to_string),
      object_id: format!("doc_{id}"),
      name: name.into(),
      position: pos.into(),
      trashed: false,
      origin: "local".into(),
      object_type: "document".into(),
    }
  }

  fn folder(id: &str, parent: Option<&str>, name: &str, pos: &str) -> LocalView {
    LocalView {
      object_type: "folder".into(),
      ..view(id, parent, name, pos)
    }
  }

  fn names(store: &LocalStore) -> Vec<String> {
    store
      .list_views("local")
      .unwrap()
      .into_iter()
      .map(|v| v.name)
      .collect()
  }

  #[test]
  fn subtree_walk_matches_bfs_semantics() {
    let (store, _dir) = tmp_store();
    store
      .save_view(&folder("f", None, "F", "0000000010"))
      .unwrap();
    store
      .save_view(&folder("k", Some("f"), "K", "0000000020"))
      .unwrap();
    store
      .save_view(&view("g", Some("k"), "G", "0000000030"))
      .unwrap();
    store
      .save_view(&view("o", None, "O", "0000000040"))
      .unwrap();

    assert_eq!(
      store.subtree_ids("local", "f", false).unwrap(),
      ["f", "k", "g"]
    );
    assert_eq!(store.subtree_ids("local", "k", false).unwrap(), ["k", "g"]);
    assert_eq!(
      store.subtree_ids("local", "ghost", false).unwrap(),
      Vec::<String>::new()
    );

    // live_only stops at a trashed node AND everything under it.
    store
      .set_views_trashed("local", &["k".to_string()], true)
      .unwrap();
    assert_eq!(store.subtree_ids("local", "f", true).unwrap(), ["f"]);
  }

  #[test]
  fn a_parent_cycle_terminates_via_cte_dedup() {
    let (store, _dir) = tmp_store();
    store
      .save_view(&view("a", Some("b"), "A", "0000000010"))
      .unwrap();
    store
      .save_view(&folder("b", Some("a"), "B", "0000000020"))
      .unwrap();
    let ids = store.subtree_ids("local", "a", false).unwrap();
    assert_eq!(ids, ["a", "b"], "both, once each");
  }

  #[test]
  fn trash_and_restore_roundtrip_in_one_transaction() {
    let (store, _dir) = tmp_store();
    store
      .save_view(&folder("f", None, "F", "0000000010"))
      .unwrap();
    store
      .save_view(&view("c", Some("f"), "C", "0000000020"))
      .unwrap();

    let hit = store.trash_view_subtree("local", "f").unwrap();
    assert_eq!(hit.len(), 2);
    assert!(store.list_views("local").unwrap().iter().all(|v| v.trashed));

    let back = store.restore_view_subtree("local", "f").unwrap();
    assert_eq!(back.len(), 2);
    let all = store.list_views("local").unwrap();
    assert!(all.iter().all(|v| !v.trashed));
    assert_eq!(all.iter().find(|v| v.id == "f").unwrap().parent_id, None);
  }

  #[test]
  fn restore_lifts_a_root_whose_parent_is_still_trashed() {
    let (store, _dir) = tmp_store();
    store
      .save_view(&folder("f", None, "F", "0000000010"))
      .unwrap();
    store
      .save_view(&view("c", Some("f"), "C", "0000000020"))
      .unwrap();
    store.trash_view_subtree("local", "f").unwrap();
    store.restore_view_subtree("local", "c").unwrap();

    let all = store.list_views("local").unwrap();
    let child = all.iter().find(|v| v.id == "c").unwrap();
    assert!(!child.trashed && child.parent_id.is_none());
    assert!(all.iter().find(|v| v.id == "f").unwrap().trashed);
  }

  #[test]
  fn purge_takes_documents_with_it() {
    let (store, _dir) = tmp_store();
    store
      .save_view(&folder("f", None, "F", "0000000010"))
      .unwrap();
    store
      .save_view(&view("c", Some("f"), "C", "0000000020"))
      .unwrap();
    store
      .save_view(&view("o", None, "O", "0000000030"))
      .unwrap();
    for id in ["f", "c", "o"] {
      let root = format!("block_{id}");
      let payload = mica_markdown::import_markdown("x", &root);
      let doc = MicaDoc::from_blocks(&payload.root_block_id, &payload.blocks);
      store.save_doc(&format!("doc_{id}"), &doc).unwrap();
    }

    let hit = store.purge_view_subtree("local", "f").unwrap();
    assert_eq!(hit, ["f", "c"]);
    assert_eq!(names(&store), ["O"]);
    assert!(store.list_docs().unwrap() == ["doc_o"]);
  }

  #[test]
  fn reorder_renumbers_and_reparents() {
    let (store, _dir) = tmp_store();
    store
      .save_view(&view("x", None, "X", "0000000010"))
      .unwrap();
    store
      .save_view(&view("y", None, "Y", "0000000020"))
      .unwrap();
    store
      .save_view(&folder("f", None, "F", "0000000030"))
      .unwrap();

    store
      .reorder_views("local", Some("f"), &["y".to_string(), "x".to_string()])
      .unwrap();
    let all = store.list_views("local").unwrap();
    let y = all.iter().find(|v| v.id == "y").unwrap();
    let x = all.iter().find(|v| v.id == "x").unwrap();
    assert_eq!(
      (y.position.as_str(), y.parent_id.as_deref()),
      ("0000000010", Some("f"))
    );
    assert_eq!(
      (x.position.as_str(), x.parent_id.as_deref()),
      ("0000000020", Some("f"))
    );
    // The folder itself is untouched.
    assert_eq!(
      all.iter().find(|v| v.id == "f").unwrap().position,
      "0000000030"
    );
  }

  #[test]
  fn reorder_skips_unknown_ids_and_unmoved_rows() {
    let (store, _dir) = tmp_store();
    store
      .save_view(&view("x", None, "X", "0000000010"))
      .unwrap();
    store
      .reorder_views("local", None, &["ghost".to_string(), "x".to_string()])
      .unwrap();
    let all = store.list_views("local").unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].position, "0000000020");

    // Repeating the SAME reorder is a no-op: every row is already at its
    // slot with the same parent, so nothing is touched.
    let before = store
      .list_views("local")
      .unwrap()
      .into_iter()
      .map(|v| (v.id, v.position, v.parent_id))
      .collect::<Vec<_>>();
    store
      .reorder_views("local", None, &["ghost".to_string(), "x".to_string()])
      .unwrap();
    let after = store
      .list_views("local")
      .unwrap()
      .into_iter()
      .map(|v| (v.id, v.position, v.parent_id))
      .collect::<Vec<_>>();
    assert_eq!(before, after);
  }

  #[test]
  fn parent_check_is_one_indexed_lookup() {
    let (store, _dir) = tmp_store();
    assert!(store.parent_accepts_children("local", None).unwrap());
    assert!(store
      .parent_accepts_children("local", Some("ghost"))
      .unwrap());
    store
      .save_view(&folder("f", None, "F", "0000000010"))
      .unwrap();
    store
      .save_view(&view("d", None, "D", "0000000020"))
      .unwrap();
    assert!(store.parent_accepts_children("local", Some("f")).unwrap());
    assert!(!store.parent_accepts_children("local", Some("d")).unwrap());
    // Origin-scoped: the remote namespace sees its own folder.
    let mut remote = folder("d", None, "RD", "0000000010");
    remote.origin = "https://example.test".into();
    store.save_view(&remote).unwrap();
    assert!(store
      .parent_accepts_children("https://example.test", Some("d"))
      .unwrap());
  }

  #[test]
  fn next_position_skips_trashed_and_scopes_to_parent() {
    let (store, _dir) = tmp_store();
    store
      .save_view(&view("a", None, "A", "0000000010"))
      .unwrap();
    store
      .save_view(&view("gone", None, "G", "0000000090"))
      .unwrap();
    store
      .save_view(&folder("f", None, "F", "0000000020"))
      .unwrap();
    store
      .save_view(&view("deep", Some("f"), "D", "0000000500"))
      .unwrap();
    store
      .set_views_trashed("local", &["gone".to_string()], true)
      .unwrap();

    assert_eq!(
      store.next_child_position("local", None).unwrap(),
      "0000000030",
      "trashed rows do not reserve slots; the deep child stays scoped",
    );
    assert_eq!(
      store.next_child_position("local", Some("f")).unwrap(),
      "0000000510",
    );
  }

  #[test]
  fn clone_copies_subtree_content_and_lands_beside_the_original() {
    let (store, _dir) = tmp_store();
    store
      .save_view(&folder("root", None, "Parent", "0000000010"))
      .unwrap();
    store
      .save_view(&folder("kid", Some("root"), "Child", "0000000020"))
      .unwrap();
    store
      .save_view(&view("grandkid", Some("kid"), "Grandchild", "0000000030"))
      .unwrap();
    for id in ["root", "kid", "grandkid"] {
      let root_block = format!("block_{id}");
      let payload = mica_markdown::import_markdown(&format!("# {id}"), &root_block);
      let doc = MicaDoc::from_blocks(&payload.root_block_id, &payload.blocks);
      store.save_doc(&format!("doc_{id}"), &doc).unwrap();
    }

    let out = store
      .clone_view("local", "root", "Parent", 1)
      .unwrap()
      .expect("a live root clones");
    assert_eq!(out.docs, 3);
    assert_eq!(out.new_name, "Parent 2");

    let all = store.list_views("local").unwrap();
    assert_eq!(all.len(), 6, "three originals + three copies");
    let new_root = all.iter().find(|v| v.id == out.root_view_id).unwrap();
    assert_eq!(new_root.parent_id, None);
    assert_eq!(new_root.position, "0000000020");

    let copied_kid = all
      .iter()
      .find(|v| v.parent_id.as_deref() == Some(out.root_view_id.as_str()))
      .unwrap();
    assert_eq!(copied_kid.name, "Child");
    let copied = store.load_doc(&copied_kid.object_id, 1).unwrap().unwrap();
    assert!(
      copied.to_blocks().iter().any(|b| b.text.contains("kid")),
      "content copied, not just the row",
    );
  }

  #[test]
  fn clone_refuses_missing_or_trashed_roots() {
    let (store, _dir) = tmp_store();
    assert!(store.clone_view("local", "nope", "X", 1).unwrap().is_none());

    store
      .save_view(&folder("f", None, "F", "0000000010"))
      .unwrap();
    store.trash_view_subtree("local", "f").unwrap();
    assert!(store.clone_view("local", "f", "F", 1).unwrap().is_none());

    // ...and a trashed node inside a LIVE subtree stays behind, with
    // everything under it.
    store
      .save_view(&folder("b", None, "B", "0000000030"))
      .unwrap();
    store
      .save_view(&view("bu", Some("b"), "BU", "0000000040"))
      .unwrap();
    store.trash_view_subtree("local", "bu").unwrap();

    let out = store.clone_view("local", "b", "B", 1).unwrap().unwrap();
    // The clone lands beside the original, so "B" is taken by its own
    // source and dedup moves the copy to "B 2" — same as the server.
    assert_eq!(out.new_name, "B 2");
    let all = store.list_views("local").unwrap();
    let copies: Vec<_> = all
      .iter()
      .filter(|v| !["f", "b", "bu"].contains(&v.id.as_str()))
      .collect();
    assert_eq!(copies.len(), 1, "only b copied; bu stayed in the bin");
    assert!(all.iter().find(|v| v.id == "bu").unwrap().trashed);
  }

  #[test]
  fn clone_of_a_view_with_no_document_still_opens() {
    let (store, _dir) = tmp_store();
    store
      .save_view(&view("solo", None, "Solo", "0000000010"))
      .unwrap();
    let out = store
      .clone_view("local", "solo", "Solo", 1)
      .unwrap()
      .unwrap();
    assert_eq!(out.docs, 0);
    let all = store.list_views("local").unwrap();
    let copy = all.iter().find(|v| v.id == out.root_view_id).unwrap();
    assert!(
      store.load_doc(&copy.object_id, 1).unwrap().is_some(),
      "the copy opens to an empty document rather than nothing",
    );
  }

  #[test]
  fn clone_lands_the_whole_subtree_atomically() {
    // The invariant the single transaction is FOR: after clone_view
    // returns, every row of the copy is present and live — no half-tree a
    // crash would have left behind on the old per-row write path.
    let (store, _dir) = tmp_store();
    store
      .save_view(&folder("root", None, "R", "0000000010"))
      .unwrap();
    store
      .save_view(&view("k", Some("root"), "K", "0000000020"))
      .unwrap();
    let out = store.clone_view("local", "root", "R", 1).unwrap().unwrap();
    let all = store.list_views("local").unwrap();
    let copies: Vec<_> = all
      .iter()
      .filter(|v| !["root", "k"].contains(&v.id.as_str()))
      .collect();
    assert_eq!(
      copies.len(),
      2,
      "both copies visible after the single commit"
    );
    assert!(copies.iter().all(|v| !v.trashed));
    let _ = out;
  }

  #[test]
  fn dedup_fills_gaps_like_the_server() {
    assert_eq!(dedup_sibling_name("日志方案 副本", &[]), "日志方案 副本");
    let sibs = vec!["Notes".to_string()];
    assert_eq!(dedup_sibling_name("Notes", &sibs), "Notes 2");
    let sibs = vec!["Notes".into(), "Notes 2".into(), "Notes 3".into()];
    assert_eq!(dedup_sibling_name("Notes", &sibs), "Notes 4");
    let sibs = vec!["Notes".into(), "Notes 3".into()];
    assert_eq!(dedup_sibling_name("Notes", &sibs), "Notes 2");
  }

  #[test]
  fn workspaces_are_not_touched_by_view_ops() {
    let (store, _dir) = tmp_store();
    let ws = LocalWorkspace {
      id: "ws".into(),
      name: "W".into(),
      position: "0000000010".into(),
      origin: "local".into(),
      role: "owner".into(),
    };
    store.save_workspace(&ws).unwrap();
    store
      .save_view(&folder("f", None, "F", "0000000010"))
      .unwrap();
    store.purge_view_subtree("local", "f").unwrap();
    assert!(store.list_workspaces("local").unwrap().contains(&ws));
  }
}
