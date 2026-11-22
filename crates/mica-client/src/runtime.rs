//! Unified L2 headless client runtime (`MicaClientRuntime`) orchestrating
//! workspaces, page trees, active `EditorSession`, embedded `BoardSession`,
//! Sans-I/O `SyncEngine`, and search/backlinks under `ClaimTracker` guards.

use std::collections::{BTreeMap, HashMap};

use mica_api_types::View;
use mica_board::{BoardCommand, BoardPatch, BoardSession};
use mica_editor::{EditorCommand, EditorPatch, EditorSession};
use mica_sync::{MemoryMirrorStorage, SyncConfig, SyncEffect, SyncEngine, SyncEvent};
use uuid::Uuid;

use crate::claim::{ClaimTracker, SessionClaim, WorldOrigin};
use crate::search::{SearchHit, search_cached_documents};
use crate::tree::PageTreeModel;

/// Pure-Rust headless runtime for any Mica L3 shell (NSH, GPUI, TUI, or Flutter).
pub struct MicaClientRuntime {
  claims: ClaimTracker,
  tree: PageTreeModel,
  editors: HashMap<Uuid, EditorSession>,
  sync_engines: HashMap<Uuid, SyncEngine>,
  active_doc_id: Option<Uuid>,
  active_board: Option<(String, BoardSession)>,
}

impl Default for MicaClientRuntime {
  fn default() -> Self {
    Self::new(WorldOrigin::Local)
  }
}

impl MicaClientRuntime {
  pub fn new(origin: WorldOrigin) -> Self {
    Self {
      claims: ClaimTracker::new(origin),
      tree: PageTreeModel::default(),
      editors: HashMap::new(),
      sync_engines: HashMap::new(),
      active_doc_id: None,
      active_board: None,
    }
  }

  pub fn claims(&self) -> &ClaimTracker {
    &self.claims
  }

  pub fn tree(&self) -> &PageTreeModel {
    &self.tree
  }

  pub fn tree_mut(&mut self) -> &mut PageTreeModel {
    &mut self.tree
  }

  pub fn active_doc_id(&self) -> Option<Uuid> {
    self.active_doc_id
  }

  pub fn load_workspace_views(&mut self, workspace_id: Uuid, views: Vec<View>) -> SessionClaim {
    let claim = self.claims.switch_workspace(Some(workspace_id));
    self.tree.replace_views(views);
    self.active_doc_id = None;
    self.active_board = None;
    claim
  }

  /// Open (or create in memory) a document backed by `EditorSession` + `SyncEngine`.
  pub fn open_document_markdown(
    &mut self,
    doc_id: Uuid,
    initial_markdown: &str,
  ) -> SessionClaim {
    let claim = self.claims.open_document(Some(doc_id));
    self.active_doc_id = Some(doc_id);
    self.active_board = None;
    self
      .editors
      .entry(doc_id)
      .or_insert_with(|| EditorSession::from_markdown(initial_markdown));
    self.sync_engines.entry(doc_id).or_insert_with(|| {
      SyncEngine::new(
        SyncConfig {
          document_id: Some(doc_id.to_string()),
          ..Default::default()
        },
        Some(Box::new(MemoryMirrorStorage::new())),
      )
    });
    claim
  }

  pub fn active_editor(&self) -> Option<&EditorSession> {
    self.active_doc_id.and_then(|id| self.editors.get(&id))
  }

  pub fn active_editor_mut(&mut self) -> Option<&mut EditorSession> {
    self.active_doc_id.and_then(|id| self.editors.get_mut(&id))
  }

  /// Dispatch an editor command on the active document and forward any emitted
  /// CRDT update directly into its `SyncEngine` (single L2 pipeline).
  pub fn dispatch_editor(&mut self, cmd: EditorCommand) -> Option<(EditorPatch, Vec<SyncEffect>)> {
    let doc_id = self.active_doc_id?;
    let patch = self.editors.get_mut(&doc_id)?.dispatch(cmd);
    let effects = if let Some(diff) = patch.crdt_update.clone() {
      self
        .sync_engines
        .get_mut(&doc_id)
        .map(|eng| eng.handle_event(SyncEvent::ApplyLocalDiff(diff)))
        .unwrap_or_default()
    } else {
      Vec::new()
    };
    Some((patch, effects))
  }

  /// Open an embedded `mica-board` block inside the active document.
  pub fn open_board(&mut self, block_id: impl Into<String>, board_json: &str) {
    self.active_board = Some((block_id.into(), BoardSession::from_json(board_json)));
  }

  pub fn active_board(&self) -> Option<(&str, &BoardSession)> {
    self.active_board.as_ref().map(|(id, s)| (id.as_str(), s))
  }

  pub fn dispatch_board(&mut self, cmd: BoardCommand) -> Option<BoardPatch> {
    self.active_board.as_mut().map(|(_, s)| s.dispatch(cmd))
  }

  pub fn close_board(&mut self) -> Option<(String, String)> {
    self
      .active_board
      .take()
      .map(|(id, session)| (id, session.export_compact_json()))
  }

  /// Export active document Markdown via single-source `mica-md`.
  pub fn export_active_markdown(&self) -> Option<String> {
    self
      .active_editor()
      .map(|ed| ed.export_markdown(&BTreeMap::new()))
  }

  /// Search across all loaded documents in this runtime using `mica_core::search_text`.
  pub fn search_loaded(&self, query: &str, limit: usize) -> Vec<SearchHit> {
    let docs: Vec<(Uuid, String, Vec<mica_core::Block>)> = self
      .editors
      .iter()
      .map(|(id, ed)| {
        let title = self
          .tree
          .visible_rows()
          .into_iter()
          .find(|r| r.object_id == *id)
          .map(|r| r.name)
          .unwrap_or_else(|| "Untitled".into());
        let blocks = ed.blocks().iter().map(|b| b.to_core_block()).collect();
        (*id, title, blocks)
      })
      .collect();
    search_cached_documents(&docs, query, limit)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::Utc;
  use mica_editor::DocSelection;

  fn sample_page(ws: Uuid, doc: Uuid, name: &str) -> View {
    let now = Utc::now();
    View {
      id: Uuid::new_v4(),
      workspace_id: ws,
      parent_view_id: None,
      object_id: doc,
      object_type: "page".into(),
      name: name.into(),
      icon: None,
      position: "a".into(),
      is_deleted: false,
      created_by: Uuid::new_v4(),
      created_at: now,
      updated_at: now,
      state_bytes: None,
    }
  }

  #[test]
  fn unified_runtime_edits_document_syncs_and_operates_board() {
    let ws = Uuid::new_v4();
    let doc = Uuid::new_v4();
    let mut rt = MicaClientRuntime::new(WorldOrigin::Local);
    rt.load_workspace_views(ws, vec![sample_page(ws, doc, "Architecture")]);

    let claim = rt.open_document_markdown(doc, "# Mica\n\nFullstack Rust");
    assert!(rt.claims().is_current(&claim));

    let (patch, _effects) = rt
      .dispatch_editor(EditorCommand::SetSelection {
        selection: Some(DocSelection::caret(1, 14)),
      })
      .unwrap();
    assert!(!patch.content_changed);

    let (patch2, _effects2) = rt
      .dispatch_editor(EditorCommand::InsertText {
        text: " Core".into(),
      })
      .unwrap();
    assert!(patch2.content_changed);
    assert!(
      rt.export_active_markdown()
        .unwrap()
        .contains("Fullstack Rust Core")
    );

    let hits = rt.search_loaded("Fullstack", 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].document_id, doc);

    // Embedded board round-trip
    rt.open_board("blk-board", &mica_board::starter_board_json("mindmap"));
    let bp = rt
      .dispatch_board(BoardCommand::AddNode {
        world_x: 100.0,
        world_y: 120.0,
        kind: "rounded".into(),
        icon: None,
        text: Some("L2 Runtime".into()),
        width: None,
        height: None,
        fill: None,
        stroke: None,
        start_editing: false,
      })
      .unwrap();
    assert!(bp.snapshot_changed);
    let (bid, compact) = rt.close_board().unwrap();
    assert_eq!(bid, "blk-board");
    assert!(compact.contains("L2 Runtime"));
  }
}
