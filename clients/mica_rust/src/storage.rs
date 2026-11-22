//! Cross-platform local vault & persistence facade (`VaultStore`).
//!
//! - Native desktop (`#[cfg(not(target_arch = "wasm32"))]`) delegates to
//!   [`mica_core::LocalStore`] (SQLite) and exposes a [`mica_sync::SqliteSyncStorage`]
//!   factory sharing the same underlying connection.
//! - Web (`#[cfg(target_arch = "wasm32")]`) uses an in-memory CRDT vault plus
//!   [`mica_sync::MemoryMirrorStorage`] while enforcing the same
//!   [`mica_core::tree_rules`] page-tree invariants.

use std::sync::{Arc, Mutex};

use mica_api_types::View;
use mica_core::{Block, MicaDoc};
use mica_md::{ParseMode, SerializeOptions, parse_body, serialize_body};
use mica_sync::SyncStorage;
use serde_json::json;
use uuid::Uuid;

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use chrono::DateTime;
#[cfg(not(target_arch = "wasm32"))]
use mica_core::{LocalStore, LocalView};
#[cfg(not(target_arch = "wasm32"))]
use mica_sync::SqliteSyncStorage;

#[cfg(target_arch = "wasm32")]
use std::collections::{HashMap, HashSet};
#[cfg(target_arch = "wasm32")]
use mica_core::tree_rules::{MissingParent, ParentLookup, judge_parent};
#[cfg(target_arch = "wasm32")]
use mica_sync::MemoryMirrorStorage;

/// Default provenance namespace for on-device documents and page trees.
pub const DEFAULT_ORIGIN: &str = "local";

const ROOT_BLOCK_ID: &str = "root";
const BOARD_BLOCK_ID: &str = "board";
const BOARD_DOC_PREFIX: &str = "board:";
const DEFAULT_VERSION_LABEL: &str = "Manual Snapshot";

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct MemoryVault {
  views: HashMap<(String, Uuid), View>,
  docs: HashMap<(String, String), Vec<u8>>,
  versions: HashMap<(String, String), Vec<(Option<String>, Vec<u8>)>>,
}

/// Unified cross-platform local vault for views, Markdown CRDT documents,
/// embedded whiteboards, version checkpoints, and `mica-sync` storage adapters.
#[derive(Clone)]
pub struct VaultStore {
  origin: String,
  client_id: u64,
  #[cfg(not(target_arch = "wasm32"))]
  store: Arc<Mutex<LocalStore>>,
  #[cfg(target_arch = "wasm32")]
  memory: Arc<Mutex<MemoryVault>>,
}

impl std::fmt::Debug for VaultStore {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("VaultStore")
      .field("origin", &self.origin)
      .field("client_id", &self.client_id)
      .finish_non_exhaustive()
  }
}

impl VaultStore {
  /// Open the platform-default vault (`<data_local_dir>/mica/vault.sqlite` on
  /// desktop), falling back to an in-memory vault on failure. Always in-memory
  /// on `wasm32`.
  pub fn open_default() -> Self {
    #[cfg(not(target_arch = "wasm32"))]
    {
      if let Some(base) = dirs_next::data_local_dir() {
        let db_path = base.join("mica").join("vault.sqlite");
        if let Some(path_str) = db_path.to_str()
          && let Ok(vault) = Self::open_at(path_str)
        {
          return vault;
        }
      }
      Self::in_memory()
    }
    #[cfg(target_arch = "wasm32")]
    {
      Self::in_memory()
    }
  }

  /// Open (creating parent directories if needed) a SQLite-backed vault at
  /// `path`. Returns an error on `wasm32`.
  pub fn open_at(path: &str) -> Result<Self, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let trimmed = path.trim();
      if trimmed.is_empty() {
        return Err("vault path must not be empty".to_string());
      }
      if trimmed != ":memory:"
        && let Some(parent) = Path::new(trimmed).parent()
        && !parent.as_os_str().is_empty()
      {
        std::fs::create_dir_all(parent)
          .map_err(|e| format!("failed to create vault directory {}: {e}", parent.display()))?;
      }
      let store = LocalStore::open(trimmed).map_err(|e| e.to_string())?;
      let client_id = store.identity().map(|id| id.client_id).unwrap_or(1);
      Ok(Self {
        origin: DEFAULT_ORIGIN.to_string(),
        client_id,
        store: Arc::new(Mutex::new(store)),
      })
    }
    #[cfg(target_arch = "wasm32")]
    {
      let _ = path;
      Err("file-backed SQLite vault is unavailable on wasm32".to_string())
    }
  }

  /// Create an ephemeral in-memory vault (for unit tests, headless runs, and
  /// `wasm32`).
  pub fn in_memory() -> Self {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = LocalStore::open_in_memory().expect("in-memory SQLite LocalStore");
      let client_id = store.identity().map(|id| id.client_id).unwrap_or(1);
      Self {
        origin: DEFAULT_ORIGIN.to_string(),
        client_id,
        store: Arc::new(Mutex::new(store)),
      }
    }
    #[cfg(target_arch = "wasm32")]
    {
      Self {
        origin: DEFAULT_ORIGIN.to_string(),
        client_id: 1,
        memory: Arc::new(Mutex::new(MemoryVault::default())),
      }
    }
  }

  /// Default provenance origin for this vault (`"local"`).
  pub fn origin(&self) -> &str {
    &self.origin
  }

  /// Stable 53-bit CRDT actor id persisted in this vault's identity table.
  pub fn client_id(&self) -> u64 {
    self.client_id
  }

  /// Build a desktop SQLite [`SqliteSyncStorage`] sharing this vault's
  /// underlying [`LocalStore`] connection.
  #[cfg(not(target_arch = "wasm32"))]
  pub fn sqlite_sync_storage(&self, origin: &str, doc_id: &str) -> SqliteSyncStorage {
    SqliteSyncStorage::new(Arc::clone(&self.store), origin, doc_id, self.client_id)
  }

  /// Cross-platform factory returning a boxed [`SyncStorage`] suitable for
  /// [`mica_sync::SyncEngine::new`].
  pub fn create_sync_storage(&self, origin: &str, doc_id: &str) -> Box<dyn SyncStorage> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      Box::new(self.sqlite_sync_storage(origin, doc_id))
    }
    #[cfg(target_arch = "wasm32")]
    {
      let _ = (origin, doc_id);
      Box::new(MemoryMirrorStorage::new())
    }
  }

  /// List all views belonging to `origin`, ordered by `position`.
  pub fn list_views(&self, origin: &str) -> Vec<View> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let Ok(store) = self.store.lock() else {
        return Vec::new();
      };
      store
        .list_views(origin)
        .unwrap_or_default()
        .into_iter()
        .filter_map(local_view_to_api_view)
        .collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
      let Ok(mem) = self.memory.lock() else {
        return Vec::new();
      };
      let mut views: Vec<View> = mem
        .views
        .iter()
        .filter(|((orig, _), _)| orig == origin)
        .map(|(_, v)| v.clone())
        .collect();
      views.sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.name.cmp(&b.name)));
      views
    }
  }

  /// Upsert a view under `origin`. Enforces folder-only parenting and cycle
  /// prevention via [`mica_core::tree_rules`].
  pub fn save_view(&self, origin: &str, view: &View) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = self
        .store
        .lock()
        .map_err(|_| "local vault mutex poisoned".to_string())?;
      let lv = api_view_to_local_view(origin, view);
      store.save_view(&lv).map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self
        .memory
        .lock()
        .map_err(|_| "memory vault mutex poisoned".to_string())?;
      validate_memory_parent(&mem.views, origin, view.id, view.parent_view_id)?;
      mem.views.insert((origin.to_string(), view.id), view.clone());
      Ok(())
    }
  }

  /// Permanently remove a view row under `origin`.
  pub fn purge_view(&self, origin: &str, id: Uuid) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = self
        .store
        .lock()
        .map_err(|_| "local vault mutex poisoned".to_string())?;
      store
        .purge_view(origin, &id.to_string())
        .map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self
        .memory
        .lock()
        .map_err(|_| "memory vault mutex poisoned".to_string())?;
      mem.views.remove(&(origin.to_string(), id));
      Ok(())
    }
  }

  /// Parse `markdown` through single-source `mica_md::parse_body`, encode into
  /// a [`MicaDoc`], and persist under `(origin, doc_id)`.
  pub fn save_markdown(&self, origin: &str, doc_id: Uuid, markdown: &str) -> Result<(), String> {
    let doc = doc_from_markdown(markdown, Some(self.client_id));
    self.save_raw_doc(origin, &doc_id.to_string(), &doc)
  }

  /// Load `(origin, doc_id)` as a [`MicaDoc`] and serialize its body through
  /// single-source `mica_md::serialize_body`.
  pub fn load_markdown(&self, origin: &str, doc_id: Uuid) -> Option<String> {
    let doc = self.load_raw_doc(origin, &doc_id.to_string())?;
    Some(markdown_from_doc(&doc))
  }

  /// Persist whiteboard JSON inside a dedicated [`MicaDoc`] (single `"board"`
  /// block) under a namespaced key so it survives restarts cleanly.
  pub fn save_board_json(&self, origin: &str, board_id: &str, json: &str) -> Result<(), String> {
    let key = board_doc_key(board_id)?;
    let doc = doc_from_board_json(board_id, json, Some(self.client_id));
    self.save_raw_doc(origin, &key, &doc)
  }

  /// Load previously saved whiteboard JSON for `(origin, board_id)`.
  pub fn load_board_json(&self, origin: &str, board_id: &str) -> Option<String> {
    let key = board_doc_key(board_id).ok()?;
    let doc = self.load_raw_doc(origin, &key)?;
    board_json_from_doc(&doc)
  }

  /// Pin the document's current persisted snapshot as a named version
  /// checkpoint.
  pub fn create_version_snapshot(
    &self,
    origin: &str,
    doc_id: Uuid,
    label: Option<&str>,
  ) -> Result<(), String> {
    let effective_label = label
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .unwrap_or(DEFAULT_VERSION_LABEL);
    let doc_key = doc_id.to_string();

    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = self
        .store
        .lock()
        .map_err(|_| "local vault mutex poisoned".to_string())?;
      let created = store
        .create_local_version(origin, &doc_key, effective_label)
        .map_err(|e| e.to_string())?;
      if created.is_none() {
        return Err(format!("document {doc_id} has no saved snapshot yet"));
      }
      Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self
        .memory
        .lock()
        .map_err(|_| "memory vault mutex poisoned".to_string())?;
      let map_key = (origin.to_string(), doc_key);
      let Some(state) = mem.docs.get(&map_key).cloned() else {
        return Err(format!("document {doc_id} has no saved snapshot yet"));
      };
      mem
        .versions
        .entry(map_key)
        .or_default()
        .push((Some(effective_label.to_string()), state));
      Ok(())
    }
  }

  /// Return the number of stored versions (auto + named) for `(origin, doc_id)`.
  pub fn list_version_count(&self, origin: &str, doc_id: Uuid) -> usize {
    self.list_versions(origin, doc_id).len()
  }

  /// Return version summaries (`version_id`, `label`, `created_at_ms`) newest first.
  pub fn list_versions(&self, origin: &str, doc_id: Uuid) -> Vec<(String, String, i64)> {
    let doc_key = doc_id.to_string();
    #[cfg(not(target_arch = "wasm32"))]
    {
      let Ok(store) = self.store.lock() else {
        return Vec::new();
      };
      store
        .list_local_versions(origin, &doc_key)
        .unwrap_or_default()
        .into_iter()
        .map(|v| {
          let label = v.label.unwrap_or_else(|| "自动快照".to_string());
          (v.id, label, v.created_at)
        })
        .collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
      let Ok(mem) = self.memory.lock() else {
        return Vec::new();
      };
      mem
        .versions
        .get(&(origin.to_string(), doc_key))
        .map(|list| {
          list
            .iter()
            .enumerate()
            .rev()
            .map(|(idx, (lbl, _))| {
              (
                idx.to_string(),
                lbl.clone().unwrap_or_else(|| "自动快照".to_string()),
                0i64,
              )
            })
            .collect()
        })
        .unwrap_or_default()
    }
  }

  /// Restore a stored version by id and return its serialized Markdown.
  pub fn restore_version_markdown(
    &self,
    origin: &str,
    doc_id: Uuid,
    version_id: &str,
  ) -> Result<String, String> {
    let doc_key = doc_id.to_string();
    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = self
        .store
        .lock()
        .map_err(|_| "local vault mutex poisoned".to_string())?;
      let restored = store
        .restore_local_version(origin, &doc_key, version_id, self.client_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("version {version_id} not found"))?;
      Ok(markdown_from_doc(&restored))
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self
        .memory
        .lock()
        .map_err(|_| "memory vault mutex poisoned".to_string())?;
      let idx: usize = version_id
        .parse()
        .map_err(|_| format!("invalid version id {version_id}"))?;
      let map_key = (origin.to_string(), doc_key);
      let state = mem
        .versions
        .get(&map_key)
        .and_then(|list| list.get(idx))
        .map(|(_, bytes)| bytes.clone())
        .ok_or_else(|| format!("version {version_id} not found"))?;
      let doc = MicaDoc::from_update(&state).map_err(|e| e.to_string())?;
      mem.docs.insert(map_key, state);
      Ok(markdown_from_doc(&doc))
    }
  }

  fn save_raw_doc(&self, origin: &str, doc_key: &str, doc: &MicaDoc) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = self
        .store
        .lock()
        .map_err(|_| "local vault mutex poisoned".to_string())?;
      store
        .save_doc(origin, doc_key, doc)
        .map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self
        .memory
        .lock()
        .map_err(|_| "memory vault mutex poisoned".to_string())?;
      let state = doc.encode_state();
      let map_key = (origin.to_string(), doc_key.to_string());
      mem.docs.insert(map_key.clone(), state.clone());
      let entry = mem.versions.entry(map_key).or_default();
      if entry.is_empty() {
        entry.push((None, state));
      }
      Ok(())
    }
  }

  fn load_raw_doc(&self, origin: &str, doc_key: &str) -> Option<MicaDoc> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = self.store.lock().ok()?;
      store.load_doc(origin, doc_key, self.client_id).ok()?
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mem = self.memory.lock().ok()?;
      let bytes = mem.docs.get(&(origin.to_string(), doc_key.to_string()))?;
      MicaDoc::from_update_with_client_id(bytes, Some(self.client_id)).ok()
    }
  }
}

fn board_doc_key(board_id: &str) -> Result<String, String> {
  let trimmed = board_id.trim();
  if trimmed.is_empty() {
    return Err("board_id must not be empty".to_string());
  }
  Ok(format!("{BOARD_DOC_PREFIX}{trimmed}"))
}

fn doc_from_markdown(markdown: &str, client_id: Option<u64>) -> MicaDoc {
  let body_blocks = parse_body(markdown, ParseMode::Document);
  let child_ids: Vec<String> = body_blocks.iter().map(|b| b.id.clone()).collect();
  let mut all_blocks = Vec::with_capacity(body_blocks.len() + 1);
  all_blocks.push(Block::new(ROOT_BLOCK_ID, "page").with_children(child_ids));
  all_blocks.extend(body_blocks);
  MicaDoc::from_blocks_with_client_id(ROOT_BLOCK_ID, &all_blocks, client_id)
}

fn markdown_from_doc(doc: &MicaDoc) -> String {
  let root_id = doc.root_block_id();
  let all = doc.to_blocks();
  let body: Vec<Block> = all
    .into_iter()
    .filter(|b| !(b.id == root_id && b.kind == "page"))
    .collect();
  if body.is_empty() {
    return String::new();
  }
  serialize_body(&body, &SerializeOptions::default())
}

fn doc_from_board_json(board_id: &str, board_json: &str, client_id: Option<u64>) -> MicaDoc {
  let blocks = vec![
    Block::new(ROOT_BLOCK_ID, "page").with_children(vec![BOARD_BLOCK_ID.to_string()]),
    Block::new(BOARD_BLOCK_ID, "board")
      .with_text(board_json)
      .with_data(json!({ "board_id": board_id })),
  ];
  MicaDoc::from_blocks_with_client_id(ROOT_BLOCK_ID, &blocks, client_id)
}

fn board_json_from_doc(doc: &MicaDoc) -> Option<String> {
  doc
    .to_blocks()
    .into_iter()
    .find(|b| b.kind == "board" || b.id == BOARD_BLOCK_ID)
    .map(|b| b.text)
}

#[cfg(not(target_arch = "wasm32"))]
fn api_view_to_local_view(origin: &str, view: &View) -> LocalView {
  LocalView {
    id: view.id.to_string(),
    workspace_id: view.workspace_id.to_string(),
    parent_id: view.parent_view_id.map(|pid| pid.to_string()),
    object_id: view.object_id.to_string(),
    name: view.name.clone(),
    position: view.position.clone(),
    trashed: view.is_deleted,
    origin: origin.to_string(),
    object_type: view.object_type.clone(),
  }
}

#[cfg(not(target_arch = "wasm32"))]
fn local_view_to_api_view(lv: LocalView) -> Option<View> {
  let id = Uuid::parse_str(&lv.id).ok()?;
  let object_id = Uuid::parse_str(&lv.object_id).ok()?;
  let workspace_id = Uuid::parse_str(&lv.workspace_id).unwrap_or_else(|_| Uuid::nil());
  let parent_view_id = lv.parent_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
  Some(View {
    id,
    workspace_id,
    parent_view_id,
    object_id,
    object_type: lv.object_type,
    name: lv.name,
    icon: None,
    position: lv.position,
    is_deleted: lv.trashed,
    created_by: Uuid::nil(),
    created_at: DateTime::UNIX_EPOCH,
    updated_at: DateTime::UNIX_EPOCH,
    state_bytes: None,
  })
}

#[cfg(target_arch = "wasm32")]
fn validate_memory_parent(
  views: &HashMap<(String, Uuid), View>,
  origin: &str,
  view_id: Uuid,
  parent_id: Option<Uuid>,
) -> Result<(), String> {
  let Some(pid) = parent_id else {
    return Ok(());
  };
  let lookup = match views.get(&(origin.to_string(), pid)) {
    None => ParentLookup::Missing,
    Some(p) if p.object_type == "folder" => ParentLookup::Folder,
    Some(_) => ParentLookup::NotFolder,
  };
  judge_parent(view_id == pid, lookup, MissingParent::Accept)
    .map_err(|why| format!("view {view_id} cannot live under {pid}: {why:?}"))?;

  let mut cursor = Some(pid);
  let mut seen = HashSet::new();
  while let Some(curr) = cursor {
    if curr == view_id {
      return Err(format!("view {view_id} cannot live under {pid}: cycle"));
    }
    if !seen.insert(curr) {
      break;
    }
    cursor = views
      .get(&(origin.to_string(), curr))
      .and_then(|v| v.parent_view_id);
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::Utc;

  fn make_view(
    ws: Uuid,
    doc: Uuid,
    name: &str,
    object_type: &str,
    parent: Option<Uuid>,
    pos: &str,
  ) -> View {
    let now = Utc::now();
    View {
      id: Uuid::new_v4(),
      workspace_id: ws,
      parent_view_id: parent,
      object_id: doc,
      object_type: object_type.to_string(),
      name: name.to_string(),
      icon: None,
      position: pos.to_string(),
      is_deleted: false,
      created_by: Uuid::new_v4(),
      created_at: now,
      updated_at: now,
      state_bytes: None,
    }
  }

  #[test]
  fn markdown_and_board_round_trip_in_memory() {
    let vault = VaultStore::in_memory();
    assert_eq!(vault.origin(), DEFAULT_ORIGIN);

    let doc_id = Uuid::new_v4();
    let md = "# Title\n\nParagraph with **bold** and `code`.\n\n- Item 1\n- Item 2";
    vault.save_markdown(vault.origin(), doc_id, md).unwrap();
    let loaded = vault.load_markdown(vault.origin(), doc_id).unwrap();
    assert!(loaded.contains("# Title"));
    assert!(loaded.contains("**bold**"));
    assert!(loaded.contains("- Item 2"));

    let board_json = r#"{"nodes":[{"id":"n1","text":"Mica Board"}],"edges":[]}"#;
    vault
      .save_board_json(vault.origin(), "main-board", board_json)
      .unwrap();
    assert_eq!(
      vault.load_board_json(vault.origin(), "main-board").as_deref(),
      Some(board_json)
    );
  }

  #[test]
  fn views_enforce_folder_parent_and_purge() {
    let vault = VaultStore::in_memory();
    let ws = Uuid::new_v4();
    let folder = make_view(ws, Uuid::new_v4(), "Docs", "folder", None, "a");
    let page = make_view(ws, Uuid::new_v4(), "Note", "page", Some(folder.id), "b");
    let invalid_child = make_view(ws, Uuid::new_v4(), "Bad", "page", Some(page.id), "c");

    vault.save_view("local", &folder).unwrap();
    vault.save_view("local", &page).unwrap();
    assert!(vault.save_view("local", &invalid_child).is_err());

    let listed = vault.list_views("local");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, folder.id);
    assert_eq!(listed[1].id, page.id);

    vault.purge_view("local", page.id).unwrap();
    assert_eq!(vault.list_views("local").len(), 1);
  }

  #[test]
  fn version_snapshots_increment_count() {
    let vault = VaultStore::in_memory();
    let doc_id = Uuid::new_v4();
    assert_eq!(vault.list_version_count("local", doc_id), 0);
    assert!(vault.create_version_snapshot("local", doc_id, Some("v1")).is_err());

    vault.save_markdown("local", doc_id, "# Draft 1").unwrap();
    let after_save = vault.list_version_count("local", doc_id);
    assert!(after_save >= 1);

    vault
      .create_version_snapshot("local", doc_id, Some("Checkpoint 1"))
      .unwrap();
    assert_eq!(vault.list_version_count("local", doc_id), after_save + 1);

    let versions = vault.list_versions("local", doc_id);
    assert!(!versions.is_empty());
    let cp_id = versions
      .iter()
      .find(|(_, lbl, _)| lbl == "Checkpoint 1")
      .map(|(id, _, _)| id.clone())
      .unwrap();

    vault.save_markdown("local", doc_id, "# Draft 2").unwrap();
    let restored = vault
      .restore_version_markdown("local", doc_id, &cp_id)
      .unwrap();
    assert!(restored.contains("# Draft 1"));

    vault.create_version_snapshot("local", doc_id, None).unwrap();
    assert!(vault.list_version_count("local", doc_id) >= after_save + 2);
  }
}
