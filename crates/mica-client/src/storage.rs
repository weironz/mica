//! Cross-platform local vault & persistence facade (`VaultStore`).
//!
//! - Native desktop (`#[cfg(not(target_arch = "wasm32"))]`) delegates to
//!   [`mica_core::LocalStore`] (SQLite) and exposes a [`mica_sync::SqliteSyncStorage`]
//!   factory sharing the same underlying connection. Client-local UI preferences
//!   (`AppPrefs`) persist in `<vault_dir>/mica_prefs.json` without touching
//!   SQLite schema migrations.
//! - Web (`#[cfg(target_arch = "wasm32")]`) uses an in-memory CRDT vault plus
//!   [`mica_sync::MemoryMirrorStorage`] while enforcing the same
//!   [`mica_core::tree_rules`] page-tree invariants and storing preferences in
//!   memory.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::DateTime;
use mica_api_types::View;
use mica_core::tree_rules::dedup_sibling_name;
use mica_core::{Block, MicaDoc};
use mica_md::{ParseMode, SerializeOptions, parse_body, serialize_body};
use mica_sync::SyncStorage;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use mica_core::{LocalStore, LocalView, LocalWorkspace};
#[cfg(not(target_arch = "wasm32"))]
use mica_sync::SqliteSyncStorage;

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

#[cfg(not(target_arch = "wasm32"))]
const PREFS_FILENAME: &str = "mica_prefs.json";
#[cfg(target_arch = "wasm32")]
const WEB_PREFS_STORAGE_KEY: &str = "mica_prefs_v1";

const PREF_PAGE_WIDTH: &str = "pageWidth";
const PREF_SIDEBAR_WIDTH: &str = "sidebarWidth";
const PREF_KNOWN_SERVERS: &str = "knownServers";
const PREF_SERVER_CREDENTIALS: &str = "serverCredentials";
const PREF_ACTIVE_ORIGIN: &str = "activeOrigin";

const DEFAULT_PAGE_WIDTH: f32 = 800.0;
const DEFAULT_SIDEBAR_WIDTH: f32 = 280.0;

/// Lightweight client-local key-value preferences snapshot persisted to
/// `<vault_dir>/mica_prefs.json` on desktop and `window.localStorage` on Web.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPrefs {
  #[serde(flatten)]
  pub entries: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
struct PrefsState {
  entries: BTreeMap<String, String>,
  #[cfg(not(target_arch = "wasm32"))]
  path: Option<PathBuf>,
  #[cfg(target_arch = "wasm32")]
  persist_web: bool,
}

/// Remove any sensitive auth credential keys from a prefs map before writing to
/// browser `window.localStorage` (Web uses server `HttpOnly` session cookies
/// exclusively; tokens must never persist in `localStorage`).
#[cfg(any(target_arch = "wasm32", test))]
fn strip_web_secret_prefs(entries: &mut BTreeMap<String, String>) -> bool {
  entries.remove(PREF_SERVER_CREDENTIALS).is_some()
}

impl PrefsState {
  fn in_memory() -> Self {
    Self::default()
  }

  #[cfg(target_arch = "wasm32")]
  fn load_web() -> Self {
    let mut entries = crate::web_runtime::local_storage_get(WEB_PREFS_STORAGE_KEY)
      .and_then(|raw| serde_json::from_str::<BTreeMap<String, String>>(&raw).ok())
      .unwrap_or_default();
    let had_secrets = strip_web_secret_prefs(&mut entries);
    let state = Self {
      entries,
      persist_web: true,
    };
    if had_secrets {
      let _ = state.flush();
    }
    state
  }

  #[cfg(not(target_arch = "wasm32"))]
  fn load_from(path: PathBuf) -> Self {
    let entries = if path.exists() {
      std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<BTreeMap<String, String>>(&raw).ok())
        .unwrap_or_default()
    } else if let Some(legacy_prefs) = legacy_flutter_prefs_path() {
      seed_non_secret_prefs_from_legacy(&legacy_prefs)
    } else {
      BTreeMap::new()
    };
    Self {
      entries,
      path: Some(path),
    }
  }

  fn flush(&self) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = &self.path {
      if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
      {
        std::fs::create_dir_all(parent)
          .map_err(|e| format!("failed to create prefs directory {}: {e}", parent.display()))?;
      }
      let body = serde_json::to_string_pretty(&self.entries)
        .map_err(|e| format!("failed to serialize prefs: {e}"))?;
      let tmp = path.with_extension("json.tmp");
      if std::fs::write(&tmp, &body).is_ok() {
        if std::fs::rename(&tmp, path).is_ok() {
          return Ok(());
        }
        let _ = std::fs::remove_file(&tmp);
      }
      std::fs::write(path, body)
        .map_err(|e| format!("failed to write prefs {}: {e}", path.display()))?;
    }
    #[cfg(target_arch = "wasm32")]
    if self.persist_web {
      let mut safe = self.entries.clone();
      strip_web_secret_prefs(&mut safe);
      let body =
        serde_json::to_string(&safe).map_err(|e| format!("failed to serialize web prefs: {e}"))?;
      crate::web_runtime::local_storage_set(WEB_PREFS_STORAGE_KEY, &body)?;
    }
    Ok(())
  }
}

/// One aggregated row in the recycle bin: a deleted subtree root along with its
/// original live ancestor path and counts of deleted descendants.
#[derive(Debug, Clone)]
pub struct TrashEntry {
  pub view: View,
  pub original_path: String,
  pub child_folders: usize,
  pub child_pages: usize,
  pub deleted_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredServerCredential {
  origin: String,
  email: String,
  #[serde(default)]
  account_id: String,
  token: String,
  #[serde(default)]
  refresh_token: String,
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct MemoryVault {
  views: HashMap<(String, Uuid), View>,
  trash_batches: HashMap<(String, Uuid), u64>,
  next_trash_batch: u64,
  docs: HashMap<(String, String), Vec<u8>>,
  versions: HashMap<(String, String), Vec<(Option<String>, Vec<u8>)>>,
  workspaces: HashMap<(String, Uuid), (String, String)>,
  activity: Vec<(String, String, mica_core::HourlyActivityRow)>,
}

/// Unified cross-platform local vault for views, Markdown CRDT documents,
/// embedded whiteboards, version checkpoints, client preferences, and
/// `mica-sync` storage adapters.
#[derive(Clone)]
pub struct VaultStore {
  origin: String,
  client_id: u64,
  prefs: Arc<Mutex<PrefsState>>,
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
  /// Open the platform-default desktop SQLite vault, seamlessly adopting the
  /// canonical legacy Flutter desktop database (`<app_dir>/mica/local/store.db`)
  /// when present, preserving early pure-Rust `vault.sqlite` installs, and
  /// honoring `MICA_DATA_DIR`. Always in-memory on `wasm32`.
  pub fn open_default() -> Self {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let env_override = std::env::var("MICA_DATA_DIR").ok();
      if let Some(db_path) = resolve_default_vault_path(
        env_override.as_deref(),
        legacy_flutter_db_path(),
        early_rust_db_path(),
      ) && let Some(path_str) = db_path.to_str()
        && let Ok(vault) = Self::open_at(path_str)
      {
        return vault;
      }
      Self::in_memory()
    }
    #[cfg(target_arch = "wasm32")]
    {
      Self {
        origin: DEFAULT_ORIGIN.to_string(),
        client_id: 1,
        prefs: Arc::new(Mutex::new(PrefsState::load_web())),
        memory: Arc::new(Mutex::new(MemoryVault::default())),
      }
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
      let prefs = if trimmed == ":memory:" {
        PrefsState::in_memory()
      } else {
        let vault_dir = Path::new(trimmed)
          .parent()
          .filter(|p| !p.as_os_str().is_empty())
          .map(Path::to_path_buf)
          .unwrap_or_else(|| PathBuf::from("."));
        std::fs::create_dir_all(&vault_dir).map_err(|e| {
          format!(
            "failed to create vault directory {}: {e}",
            vault_dir.display()
          )
        })?;
        PrefsState::load_from(vault_dir.join(PREFS_FILENAME))
      };
      let store = LocalStore::open(trimmed).map_err(|e| e.to_string())?;
      let client_id = store.identity().map(|id| id.client_id).unwrap_or(1);
      Ok(Self {
        origin: DEFAULT_ORIGIN.to_string(),
        client_id,
        prefs: Arc::new(Mutex::new(prefs)),
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
        prefs: Arc::new(Mutex::new(PrefsState::in_memory())),
        store: Arc::new(Mutex::new(store)),
      }
    }
    #[cfg(target_arch = "wasm32")]
    {
      Self {
        origin: DEFAULT_ORIGIN.to_string(),
        client_id: 1,
        prefs: Arc::new(Mutex::new(PrefsState::in_memory())),
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

  fn lock_prefs(&self) -> Result<MutexGuard<'_, PrefsState>, String> {
    self
      .prefs
      .lock()
      .map_err(|_| "prefs mutex poisoned".to_string())
  }

  #[cfg(not(target_arch = "wasm32"))]
  fn lock_store(&self) -> Result<MutexGuard<'_, LocalStore>, String> {
    self
      .store
      .lock()
      .map_err(|_| "local vault mutex poisoned".to_string())
  }

  #[cfg(target_arch = "wasm32")]
  fn lock_memory(&self) -> Result<MutexGuard<'_, MemoryVault>, String> {
    self
      .memory
      .lock()
      .map_err(|_| "memory vault mutex poisoned".to_string())
  }

  // ── Persistent Client Preferences (`AppPrefs`) ──────────────────────────

  /// Read a raw string preference by key.
  pub fn get_pref(&self, key: &str) -> Option<String> {
    self.prefs.lock().ok()?.entries.get(key).cloned()
  }

  /// Persist a string preference by key.
  pub fn set_pref(&self, key: &str, value: &str) -> Result<(), String> {
    let mut prefs = self.lock_prefs()?;
    prefs.entries.insert(key.to_string(), value.to_string());
    prefs.flush()
  }

  /// Remove a stored preference key.
  pub fn remove_pref(&self, key: &str) -> Result<(), String> {
    let mut prefs = self.lock_prefs()?;
    prefs.entries.remove(key);
    prefs.flush()
  }

  /// Export a snapshot of all stored preferences.
  pub fn snapshot_prefs(&self) -> AppPrefs {
    self
      .prefs
      .lock()
      .map(|p| AppPrefs {
        entries: p.entries.clone(),
      })
      .unwrap_or_default()
  }

  /// Persisted editor page width in logical pixels (default `800.0`).
  pub fn page_width(&self) -> f32 {
    self
      .get_pref(PREF_PAGE_WIDTH)
      .and_then(|s| s.trim().parse::<f32>().ok())
      .filter(|w| w.is_finite() && *w > 0.0)
      .unwrap_or(DEFAULT_PAGE_WIDTH)
  }

  /// Persist editor page width in logical pixels.
  pub fn set_page_width(&self, w: f32) -> Result<(), String> {
    if !w.is_finite() || w <= 0.0 {
      return Err("page width must be a positive finite number".to_string());
    }
    self.set_pref(PREF_PAGE_WIDTH, &format!("{w:.1}"))
  }

  /// Persisted sidebar width in logical pixels (default `280.0`).
  pub fn sidebar_width(&self) -> f32 {
    self
      .get_pref(PREF_SIDEBAR_WIDTH)
      .and_then(|s| s.trim().parse::<f32>().ok())
      .filter(|w| w.is_finite() && *w > 0.0)
      .unwrap_or(DEFAULT_SIDEBAR_WIDTH)
  }

  /// Persist sidebar width in logical pixels.
  pub fn set_sidebar_width(&self, w: f32) -> Result<(), String> {
    if !w.is_finite() || w <= 0.0 {
      return Err("sidebar width must be a positive finite number".to_string());
    }
    self.set_pref(PREF_SIDEBAR_WIDTH, &format!("{w:.1}"))
  }

  /// Starred page IDs for `ws_id` in starring order (`favoritePages:{ws_id}`).
  pub fn favorites_for(&self, ws_id: Uuid) -> Vec<Uuid> {
    let Some(raw) = self.get_pref(&format!("favoritePages:{ws_id}")) else {
      return Vec::new();
    };
    let Ok(items) = serde_json::from_str::<Vec<String>>(&raw) else {
      return Vec::new();
    };
    let mut out = Vec::with_capacity(items.len());
    let mut seen = HashSet::new();
    for item in items {
      if let Ok(id) = Uuid::parse_str(item.trim())
        && seen.insert(id)
      {
        out.push(id);
      }
    }
    out
  }

  /// Persist starred page IDs for `ws_id` (`favoritePages:{ws_id}`).
  pub fn set_favorites_for(&self, ws_id: Uuid, ids: &[Uuid]) -> Result<(), String> {
    let mut deduped = Vec::with_capacity(ids.len());
    let mut seen = HashSet::new();
    for &id in ids {
      if seen.insert(id) {
        deduped.push(id.to_string());
      }
    }
    let encoded =
      serde_json::to_string(&deduped).map_err(|e| format!("failed to encode favorites: {e}"))?;
    self.set_pref(&format!("favoritePages:{ws_id}"), &encoded)
  }

  /// Custom emoji / icon assigned to `view_id` (`viewIcon:{view_id}`).
  pub fn view_icon(&self, view_id: Uuid) -> Option<String> {
    self
      .get_pref(&format!("viewIcon:{view_id}"))
      .map(|s| s.trim().to_string())
      .filter(|s| !s.is_empty())
  }

  /// Set or clear the custom emoji / icon for `view_id` (`viewIcon:{view_id}`).
  pub fn set_view_icon(&self, view_id: Uuid, icon: Option<&str>) -> Result<(), String> {
    let key = format!("viewIcon:{view_id}");
    match icon.map(str::trim).filter(|s| !s.is_empty()) {
      Some(value) => self.set_pref(&key, value),
      None => self.remove_pref(&key),
    }
  }

  /// List of previously used cloud server base URLs.
  pub fn known_servers(&self) -> Vec<String> {
    let Some(raw) = self.get_pref(PREF_KNOWN_SERVERS) else {
      return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&raw)
      .unwrap_or_default()
      .into_iter()
      .map(|s| normalize_origin(&s))
      .filter(|s| !s.is_empty())
      .collect()
  }

  /// Persist list of known cloud server base URLs (deduplicated, normalized).
  pub fn set_known_servers(&self, servers: &[String]) -> Result<(), String> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for s in servers {
      let norm = normalize_origin(s);
      if !norm.is_empty() && seen.insert(norm.clone()) {
        deduped.push(norm);
      }
    }
    let encoded = serde_json::to_string(&deduped)
      .map_err(|e| format!("failed to encode known servers: {e}"))?;
    self.set_pref(PREF_KNOWN_SERVERS, &encoded)
  }

  /// Saved server sessions as `(origin, email, token)` tuples.
  pub fn saved_server_credentials(&self) -> Vec<(String, String, String)> {
    self
      .get_pref(PREF_SERVER_CREDENTIALS)
      .and_then(|raw| serde_json::from_str::<Vec<StoredServerCredential>>(&raw).ok())
      .unwrap_or_default()
      .into_iter()
      .map(|c| (c.origin, c.email, c.token))
      .collect()
  }

  /// Look up the full saved credential `(email, access_token, refresh_token)` for `origin`.
  pub fn saved_server_credential_full(&self, origin: &str) -> Option<(String, String, String)> {
    self
      .saved_server_credential_record(origin)
      .map(|(email, _account_id, token, refresh)| (email, token, refresh))
  }

  /// Look up `(email, account_id, access_token, refresh_token)` for `origin`.
  pub fn saved_server_credential_record(
    &self,
    origin: &str,
  ) -> Option<(String, String, String, String)> {
    let norm_origin = canonical_cred_origin(origin);
    self
      .get_pref(PREF_SERVER_CREDENTIALS)
      .and_then(|raw| serde_json::from_str::<Vec<StoredServerCredential>>(&raw).ok())
      .unwrap_or_default()
      .into_iter()
      .find(|c| canonical_cred_origin(&c.origin) == norm_origin)
      .map(|c| (c.email, c.account_id, c.token, c.refresh_token))
  }

  /// Upsert saved credentials `(origin, email, token)` for a cloud server.
  pub fn save_server_credential(
    &self,
    origin: &str,
    email: &str,
    token: &str,
  ) -> Result<(), String> {
    self.save_server_credential_with_refresh(origin, email, token, "")
  }

  /// Upsert saved credentials `(origin, email, access_token, refresh_token)`,
  /// preserving any previously stored `account_id` for that server.
  pub fn save_server_credential_with_refresh(
    &self,
    origin: &str,
    email: &str,
    token: &str,
    refresh_token: &str,
  ) -> Result<(), String> {
    let prev_account_id = self
      .saved_server_credential_record(origin)
      .map(|(_, aid, _, _)| aid)
      .unwrap_or_default();
    self.save_server_credential_full(origin, email, &prev_account_id, token, refresh_token)
  }

  /// Upsert full saved credentials including authoritative server `account_id`.
  pub fn save_server_credential_full(
    &self,
    origin: &str,
    email: &str,
    account_id: &str,
    token: &str,
    refresh_token: &str,
  ) -> Result<(), String> {
    let norm_origin = canonical_cred_origin(origin);
    if norm_origin.is_empty() {
      return Err("server origin must not be empty".to_string());
    }
    let mut entries: Vec<StoredServerCredential> = self
      .get_pref(PREF_SERVER_CREDENTIALS)
      .and_then(|raw| serde_json::from_str(&raw).ok())
      .unwrap_or_default();
    let updated = StoredServerCredential {
      origin: norm_origin.clone(),
      email: email.trim().to_string(),
      account_id: account_id.trim().to_string(),
      token: token.to_string(),
      refresh_token: refresh_token.to_string(),
    };
    if let Some(existing) = entries
      .iter_mut()
      .find(|c| canonical_cred_origin(&c.origin) == norm_origin)
    {
      *existing = updated;
    } else {
      entries.push(updated);
    }
    let encoded = serde_json::to_string(&entries)
      .map_err(|e| format!("failed to encode server credentials: {e}"))?;
    self.set_pref(PREF_SERVER_CREDENTIALS, &encoded)
  }

  /// Clear saved credentials for `origin`.
  pub fn clear_server_credential(&self, origin: &str) -> Result<(), String> {
    let norm_origin = canonical_cred_origin(origin);
    let mut entries: Vec<StoredServerCredential> = self
      .get_pref(PREF_SERVER_CREDENTIALS)
      .and_then(|raw| serde_json::from_str(&raw).ok())
      .unwrap_or_default();
    entries.retain(|c| canonical_cred_origin(&c.origin) != norm_origin);
    let encoded = serde_json::to_string(&entries)
      .map_err(|e| format!("failed to encode server credentials: {e}"))?;
    self.set_pref(PREF_SERVER_CREDENTIALS, &encoded)
  }

  /// Last active origin (`"local"` or a cloud server URL).
  pub fn active_origin(&self) -> String {
    self
      .get_pref(PREF_ACTIVE_ORIGIN)
      .map(|s| normalize_origin(&s))
      .filter(|s| !s.is_empty())
      .unwrap_or_else(|| DEFAULT_ORIGIN.to_string())
  }

  /// Persist active origin (`"local"` or a cloud server URL).
  pub fn set_active_origin(&self, origin: &str) -> Result<(), String> {
    let norm = normalize_origin(origin);
    self.set_pref(
      PREF_ACTIVE_ORIGIN,
      if norm.is_empty() {
        DEFAULT_ORIGIN
      } else {
        &norm
      },
    )
  }

  /// Read a boolean preference with fallback `default`.
  pub fn bool_pref(&self, key: &str, default: bool) -> bool {
    match self.get_pref(key).as_deref().map(str::trim) {
      Some("true" | "1" | "yes") => true,
      Some("false" | "0" | "no") => false,
      _ => default,
    }
  }

  /// Persist a boolean preference.
  pub fn set_bool_pref(&self, key: &str, val: bool) -> Result<(), String> {
    self.set_pref(key, if val { "true" } else { "false" })
  }

  // ── Sync Storage Factories ──────────────────────────────────────────────

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

  // ── Views, Tree Reordering, Trash, Restore, Purge, Clone ────────────────

  /// List all views belonging to `origin`, ordered by `position`.
  pub fn list_views(&self, origin: &str) -> Vec<View> {
    #[cfg(not(target_arch = "wasm32"))]
    let mut views: Vec<View> = {
      let Ok(store) = self.store.lock() else {
        return Vec::new();
      };
      store
        .list_views(origin)
        .unwrap_or_default()
        .into_iter()
        .filter_map(local_view_to_api_view)
        .collect()
    };
    #[cfg(target_arch = "wasm32")]
    let mut views: Vec<View> = {
      let Ok(mem) = self.memory.lock() else {
        return Vec::new();
      };
      let mut list: Vec<View> = mem
        .views
        .iter()
        .filter(|((orig, _), _)| orig == origin)
        .map(|(_, v)| v.clone())
        .collect();
      list.sort_by(|a, b| {
        a.position
          .cmp(&b.position)
          .then_with(|| a.name.cmp(&b.name))
      });
      list
    };
    for v in &mut views {
      if v.icon.is_none() {
        v.icon = self.view_icon(v.id);
      }
    }
    views
  }

  /// List all views belonging to a single `(origin, ws_id)`, ordered by `position`.
  pub fn list_workspace_views(&self, origin: &str, ws_id: Uuid) -> Vec<View> {
    self
      .list_views(origin)
      .into_iter()
      .filter(|v| v.workspace_id == ws_id)
      .collect()
  }

  /// Mirror an authoritative cloud view list for `(origin, ws_id)` into the local
  /// vault: removes stale rows belonging to `(origin, ws_id)` that are absent from
  /// `views`, then upserts folders before pages so parent references stay valid.
  pub fn replace_workspace_views(
    &self,
    origin: &str,
    ws_id: Uuid,
    views: &[View],
  ) -> Result<(), String> {
    let keep_ids: HashSet<Uuid> = views.iter().map(|v| v.id).collect();
    let existing = self.list_workspace_views(origin, ws_id);
    for old in existing {
      if !keep_ids.contains(&old.id) {
        let _ = self.purge_view(origin, old.id);
      }
    }

    // Save folders before pages so `validate_memory_parent` / `ensure_valid_parent`
    // always observe folder parents first.
    let mut folders: Vec<View> = Vec::new();
    let mut pages: Vec<View> = Vec::new();
    for raw in views {
      let mut row = raw.clone();
      if row.workspace_id.is_nil() {
        row.workspace_id = ws_id;
      }
      if row.object_type == "folder" {
        folders.push(row);
      } else {
        pages.push(row);
      }
    }

    // Topologically insert folders (parents before children).
    let mut remaining = folders;
    let mut saved_ids: HashSet<Uuid> = HashSet::new();
    while !remaining.is_empty() {
      let before = remaining.len();
      let mut next_round = Vec::new();
      for folder in remaining {
        let parent_ready = folder
          .parent_view_id
          .is_none_or(|pid| saved_ids.contains(&pid) || !keep_ids.contains(&pid));
        if parent_ready {
          let _ = self.save_view(origin, &folder);
          saved_ids.insert(folder.id);
        } else {
          next_round.push(folder);
        }
      }
      if next_round.len() == before {
        // Fallback if there's an unexpected cycle in remote metadata: save as-is.
        for folder in next_round {
          let _ = self.save_view(origin, &folder);
        }
        break;
      }
      remaining = next_round;
    }

    for page in &pages {
      let _ = self.save_view(origin, page);
    }
    Ok(())
  }

  /// Upsert a view under `origin`. Enforces folder-only parenting and cycle
  /// prevention via [`mica_core::tree_rules`].
  pub fn save_view(&self, origin: &str, view: &View) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = self.lock_store()?;
      store
        .save_view(&api_view_to_local_view(origin, view))
        .map_err(|e| e.to_string())?;
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
      validate_memory_parent(&mem.views, origin, view.id, view.parent_view_id)?;
      mem
        .views
        .insert((origin.to_string(), view.id), view.clone());
    }
    if let Some(icon) = view
      .icon
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
    {
      let _ = self.set_view_icon(view.id, Some(icon));
    }
    Ok(())
  }

  /// Permanently remove a view row under `origin`.
  pub fn purge_view(&self, origin: &str, id: Uuid) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      self
        .lock_store()?
        .purge_view(origin, &id.to_string())
        .map_err(|e| e.to_string())?;
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
      mem.views.remove(&(origin.to_string(), id));
      mem.trash_batches.remove(&(origin.to_string(), id));
    }
    let _ = self.set_view_icon(id, None);
    Ok(())
  }

  /// Reorder and/or reparent `ordered_ids` under `parent_view_id`, assigning
  /// 10-digit zero-padded positions (`0000000010`, `0000000020`, ...). Enforces
  /// folder-only parenting and cycle prevention via [`mica_core::tree_rules`].
  pub fn reorder_views(
    &self,
    origin: &str,
    parent_view_id: Option<Uuid>,
    ordered_ids: &[Uuid],
  ) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let parent_str = parent_view_id.map(|id| id.to_string());
      let id_strs: Vec<String> = ordered_ids.iter().map(Uuid::to_string).collect();
      self
        .lock_store()?
        .reorder_views(origin, parent_str.as_deref(), &id_strs)
        .map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
      let mut staged = mem.views.clone();
      for (i, &id) in ordered_ids.iter().enumerate() {
        let key = (origin.to_string(), id);
        let Some(existing) = staged.get(&key).cloned() else {
          continue;
        };
        let position = format!("{:010}", (i as i64 + 1) * 10);
        if existing.position == position && existing.parent_view_id == parent_view_id {
          continue;
        }
        if existing.parent_view_id != parent_view_id {
          validate_memory_parent(&staged, origin, id, parent_view_id)?;
        }
        if let Some(entry) = staged.get_mut(&key) {
          entry.parent_view_id = parent_view_id;
          entry.position = position;
          entry.is_deleted = false;
        }
      }
      mem.views = staged;
      Ok(())
    }
  }

  /// Soft-delete `root_id` and all its descendants (`is_deleted = true`).
  /// Returns the affected view IDs.
  pub fn trash_view_subtree(&self, origin: &str, root_id: Uuid) -> Result<Vec<Uuid>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let ids = self
        .lock_store()?
        .trash_view_subtree(origin, &root_id.to_string())
        .map_err(|e| e.to_string())?;
      Ok(
        ids
          .into_iter()
          .filter_map(|s| Uuid::parse_str(&s).ok())
          .collect(),
      )
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
      let ids = collect_memory_subtree(&mem.views, origin, root_id, false);
      if ids.is_empty() {
        return Ok(Vec::new());
      }
      mem.next_trash_batch += 1;
      let batch = mem.next_trash_batch;
      for &id in &ids {
        let key = (origin.to_string(), id);
        if let Some(view) = mem.views.get_mut(&key)
          && !view.is_deleted
        {
          view.is_deleted = true;
          mem.trash_batches.insert(key, batch);
        }
      }
      Ok(ids)
    }
  }

  /// Restore `root_id` and the descendants trashed with it. If `root_id`'s
  /// parent is missing or still deleted, promotes `root_id` to top level
  /// (`parent_view_id = None`). Returns the restored view IDs.
  pub fn restore_view_subtree(&self, origin: &str, root_id: Uuid) -> Result<Vec<Uuid>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let ids = self
        .lock_store()?
        .restore_view_subtree(origin, &root_id.to_string())
        .map_err(|e| e.to_string())?;
      Ok(
        ids
          .into_iter()
          .filter_map(|s| Uuid::parse_str(&s).ok())
          .collect(),
      )
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
      let root_key = (origin.to_string(), root_id);
      let Some(root_view) = mem.views.get(&root_key).cloned() else {
        return Ok(Vec::new());
      };
      if !root_view.is_deleted {
        return Ok(Vec::new());
      }
      let root_batch = mem.trash_batches.get(&root_key).copied();
      let subtree = collect_memory_subtree(&mem.views, origin, root_id, false);
      let mut restored = Vec::new();
      for &id in &subtree {
        let key = (origin.to_string(), id);
        let matches = root_batch.is_none_or(|b| mem.trash_batches.get(&key).copied() == Some(b));
        if matches
          && let Some(v) = mem.views.get_mut(&key)
          && v.is_deleted
        {
          v.is_deleted = false;
          mem.trash_batches.remove(&key);
          restored.push(id);
        }
      }
      if let Some(pid) = root_view.parent_view_id {
        let parent_live = mem
          .views
          .get(&(origin.to_string(), pid))
          .is_some_and(|p| !p.is_deleted);
        if !parent_live && let Some(root_mut) = mem.views.get_mut(&root_key) {
          root_mut.parent_view_id = None;
        }
      }
      Ok(restored)
    }
  }

  /// Permanently delete `root_id`, all of its descendants, and their documents.
  /// Returns the number of views purged.
  pub fn purge_view_subtree(&self, origin: &str, root_id: Uuid) -> Result<usize, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let ids = self
        .lock_store()?
        .purge_view_subtree(origin, &root_id.to_string())
        .map_err(|e| e.to_string())?;
      for id_str in &ids {
        if let Ok(id) = Uuid::parse_str(id_str) {
          let _ = self.set_view_icon(id, None);
        }
      }
      Ok(ids.len())
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
      let ids = collect_memory_subtree(&mem.views, origin, root_id, false);
      let mut removed = 0usize;
      for &id in &ids {
        let key = (origin.to_string(), id);
        if let Some(v) = mem.views.remove(&key) {
          let doc_key = (origin.to_string(), v.object_id.to_string());
          mem.docs.remove(&doc_key);
          mem.versions.remove(&doc_key);
          mem.trash_batches.remove(&key);
          removed += 1;
        }
      }
      drop(mem);
      for id in ids {
        let _ = self.set_view_icon(id, None);
      }
      Ok(removed)
    }
  }

  /// Permanently delete every trashed view (and its documents) inside
  /// `workspace_id`. Returns the number of purged views.
  pub fn purge_all_trash(&self, origin: &str, workspace_id: Uuid) -> Result<usize, String> {
    let deleted: Vec<View> = self
      .list_views(origin)
      .into_iter()
      .filter(|v| v.workspace_id == workspace_id && v.is_deleted)
      .collect();
    if deleted.is_empty() {
      return Ok(0);
    }

    let deleted_ids: HashSet<Uuid> = deleted.iter().map(|v| v.id).collect();
    let roots: Vec<Uuid> = deleted
      .iter()
      .filter(|v| {
        v.parent_view_id
          .is_none_or(|pid| !deleted_ids.contains(&pid))
      })
      .map(|v| v.id)
      .collect();

    let mut total = 0usize;
    for root_id in roots {
      total += self.purge_view_subtree(origin, root_id)?;
    }

    // Catch any cyclic/orphan deleted views still remaining in this workspace.
    for view in self
      .list_views(origin)
      .into_iter()
      .filter(|v| v.workspace_id == workspace_id && v.is_deleted)
    {
      #[cfg(not(target_arch = "wasm32"))]
      {
        let store = self.lock_store()?;
        let _ = store.delete_doc(origin, &view.object_id.to_string());
        store
          .purge_view(origin, &view.id.to_string())
          .map_err(|e| e.to_string())?;
      }
      #[cfg(target_arch = "wasm32")]
      {
        let mut mem = self.lock_memory()?;
        mem.views.remove(&(origin.to_string(), view.id));
        mem.trash_batches.remove(&(origin.to_string(), view.id));
        let doc_key = (origin.to_string(), view.object_id.to_string());
        mem.docs.remove(&doc_key);
        mem.versions.remove(&doc_key);
      }
      let _ = self.set_view_icon(view.id, None);
      total += 1;
    }
    Ok(total)
  }

  /// Clone a live page (with its Markdown/CRDT content) or recursively clone a
  /// live folder subtree under the same parent, deduplicating sibling names via
  /// [`mica_core::tree_rules::dedup_sibling_name`].
  pub fn clone_view(&self, origin: &str, view_id: Uuid, new_name: &str) -> Result<View, String> {
    let all_views = self.list_views(origin);
    let root = all_views
      .iter()
      .find(|v| v.id == view_id)
      .cloned()
      .ok_or_else(|| format!("view {view_id} not found"))?;
    if root.is_deleted {
      return Err(format!("cannot clone deleted view {view_id}"));
    }

    // Collect live subtree in parent-before-child BFS order so folders are
    // saved before their children (preserving folder-only parent checks).
    let mut children_of: HashMap<Uuid, Vec<View>> = HashMap::new();
    for v in &all_views {
      if !v.is_deleted
        && let Some(pid) = v.parent_view_id
      {
        children_of.entry(pid).or_default().push(v.clone());
      }
    }
    for kids in children_of.values_mut() {
      kids.sort_by(|a, b| {
        a.position
          .cmp(&b.position)
          .then_with(|| a.name.cmp(&b.name))
      });
    }

    let mut subtree = Vec::new();
    let mut queue = VecDeque::from([root.clone()]);
    let mut seen = HashSet::from([root.id]);
    while let Some(node) = queue.pop_front() {
      let node_id = node.id;
      subtree.push(node);
      if let Some(kids) = children_of.get(&node_id) {
        for kid in kids {
          if seen.insert(kid.id) {
            queue.push_back(kid.clone());
          }
        }
      }
    }

    let sibling_names: Vec<String> = all_views
      .iter()
      .filter(|v| {
        !v.is_deleted
          && v.workspace_id == root.workspace_id
          && v.parent_view_id == root.parent_view_id
      })
      .map(|v| v.name.clone())
      .collect();
    let desired = if new_name.trim().is_empty() {
      format!("{} (副本)", root.name)
    } else {
      new_name.trim().to_string()
    };
    let final_root_name = dedup_sibling_name(&desired, &sibling_names);
    let root_position = next_sibling_position(&all_views, root.workspace_id, root.parent_view_id);
    let id_map: HashMap<Uuid, Uuid> = subtree.iter().map(|v| (v.id, Uuid::new_v4())).collect();
    let now = chrono::Utc::now();
    let mut cloned_root: Option<View> = None;

    for orig in &subtree {
      let new_id = id_map[&orig.id];
      let new_doc_id = Uuid::new_v4();
      let is_root = orig.id == root.id;

      if let Some(mut doc) = self.load_raw_doc(origin, &orig.object_id.to_string()) {
        if is_root {
          let root_block = doc.root_block_id();
          if !root_block.is_empty() {
            doc.set_block_prop(&root_block, "title", &json!(final_root_name));
          }
        }
        self.save_raw_doc(origin, &new_doc_id.to_string(), &doc)?;
      } else if orig.object_type != "folder" {
        self.save_markdown(origin, new_doc_id, "")?;
      }

      let cloned_view = View {
        id: new_id,
        workspace_id: orig.workspace_id,
        parent_view_id: if is_root {
          orig.parent_view_id
        } else {
          orig
            .parent_view_id
            .and_then(|pid| id_map.get(&pid).copied())
        },
        object_id: new_doc_id,
        object_type: orig.object_type.clone(),
        name: if is_root {
          final_root_name.clone()
        } else {
          orig.name.clone()
        },
        icon: orig.icon.clone().or_else(|| self.view_icon(orig.id)),
        position: if is_root {
          root_position.clone()
        } else {
          orig.position.clone()
        },
        is_deleted: false,
        created_by: orig.created_by,
        created_at: now,
        updated_at: now,
        state_bytes: None,
      };
      self.save_view(origin, &cloned_view)?;
      if is_root {
        cloned_root = Some(cloned_view);
      }
    }

    cloned_root.ok_or_else(|| "failed to clone view".to_string())
  }

  /// Aggregate deleted views into root-only [`TrashEntry`] rows showing each
  /// deleted root's live ancestor path, descendant counts, and deletion label.
  /// Mirrors Flutter `trash_data.dart::buildTrashEntries` 1:1.
  pub fn build_trash_entries(&self, views: &[View]) -> Vec<TrashEntry> {
    let deleted: Vec<&View> = views.iter().filter(|v| v.is_deleted).collect();
    if deleted.is_empty() {
      return Vec::new();
    }

    let deleted_ids: HashSet<Uuid> = deleted.iter().map(|v| v.id).collect();
    let mut children_of: HashMap<Uuid, Vec<&View>> = HashMap::new();
    for &v in &deleted {
      if let Some(pid) = v.parent_view_id
        && deleted_ids.contains(&pid)
      {
        children_of.entry(pid).or_default().push(v);
      }
    }
    let live_by_id: HashMap<Uuid, &View> = views
      .iter()
      .filter(|v| !v.is_deleted)
      .map(|v| (v.id, v))
      .collect();

    let mut entries = Vec::new();
    for &root in &deleted {
      if root
        .parent_view_id
        .is_some_and(|pid| deleted_ids.contains(&pid))
      {
        continue;
      }
      let (mut child_folders, mut child_pages) = (0usize, 0usize);
      let mut seen = HashSet::from([root.id]);
      let mut stack: Vec<&View> = children_of.get(&root.id).cloned().unwrap_or_default();
      while let Some(node) = stack.pop() {
        if !seen.insert(node.id) {
          continue;
        }
        if node.object_type == "folder" {
          child_folders += 1;
        } else {
          child_pages += 1;
        }
        if let Some(kids) = children_of.get(&node.id) {
          stack.extend(kids.iter().copied());
        }
      }
      entries.push(TrashEntry {
        view: root.clone(),
        original_path: live_ancestor_path(root, &live_by_id),
        child_folders,
        child_pages,
        deleted_label: format_deleted_label(root),
      });
    }
    entries
  }

  // ── Workspaces ──────────────────────────────────────────────────────────

  /// Persist a workspace name under `origin` (creating or updating the
  /// workspace record in SQLite / memory and prefs).
  pub fn save_workspace_name(&self, origin: &str, ws_id: Uuid, name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
      return Err("workspace name must not be empty".to_string());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = self.lock_store()?;
      let existing = store.list_workspaces(origin).unwrap_or_default();
      let id_str = ws_id.to_string();
      let (position, role) = if let Some(cur) = existing.iter().find(|w| w.id == id_str) {
        (cur.position.clone(), cur.role.clone())
      } else {
        let max_pos = existing
          .iter()
          .filter_map(|w| w.position.parse::<i64>().ok())
          .max()
          .unwrap_or(0);
        (format!("{:010}", max_pos + 10), "owner".to_string())
      };
      store
        .save_workspace(&LocalWorkspace {
          id: id_str,
          name: trimmed.to_string(),
          position,
          origin: origin.to_string(),
          role,
        })
        .map_err(|e| e.to_string())?;
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
      let key = (origin.to_string(), ws_id);
      let position = mem
        .workspaces
        .get(&key)
        .map(|(_, p)| p.clone())
        .unwrap_or_else(|| {
          let max_pos = mem
            .workspaces
            .iter()
            .filter(|((orig, _), _)| orig == origin)
            .filter_map(|(_, (_, pos))| pos.parse::<i64>().ok())
            .max()
            .unwrap_or(0);
          format!("{:010}", max_pos + 10)
        });
      mem.workspaces.insert(key, (trimmed.to_string(), position));
    }
    let _ = self.set_pref(&format!("workspaceName:{origin}:{ws_id}"), trimmed);
    Ok(())
  }

  /// Mirror an authoritative list of `(workspace_id, name)` entries for `origin`,
  /// preserving their remote order and pruning workspaces removed on the server.
  pub fn replace_origin_workspaces(
    &self,
    origin: &str,
    entries: &[(Uuid, String)],
  ) -> Result<(), String> {
    let keep: HashSet<Uuid> = entries.iter().map(|(id, _)| *id).collect();
    for (existing_id, _) in self.list_workspaces(origin) {
      if !keep.contains(&existing_id) {
        let _ = self.delete_workspace(origin, existing_id);
      }
    }
    for (idx, (ws_id, name)) in entries.iter().enumerate() {
      let trimmed = name.trim();
      if trimmed.is_empty() {
        continue;
      }
      let pos = format!("{:010}", (idx as i64 + 1) * 10);
      #[cfg(not(target_arch = "wasm32"))]
      {
        let store = self.lock_store()?;
        store
          .save_workspace(&LocalWorkspace {
            id: ws_id.to_string(),
            name: trimmed.to_string(),
            position: pos,
            origin: origin.to_string(),
            role: "member".to_string(),
          })
          .map_err(|e| e.to_string())?;
      }
      #[cfg(target_arch = "wasm32")]
      {
        let mut mem = self.lock_memory()?;
        mem
          .workspaces
          .insert((origin.to_string(), *ws_id), (trimmed.to_string(), pos));
      }
      let _ = self.set_pref(&format!("workspaceName:{origin}:{ws_id}"), trimmed);
    }
    Ok(())
  }

  /// List persisted `(workspace_id, name)` entries for `origin`, ordered by
  /// position.
  pub fn list_workspaces(&self, origin: &str) -> Vec<(Uuid, String)> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let Ok(store) = self.store.lock() else {
        return Vec::new();
      };
      store
        .list_workspaces(origin)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|w| Uuid::parse_str(&w.id).ok().map(|id| (id, w.name)))
        .collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
      let Ok(mem) = self.memory.lock() else {
        return Vec::new();
      };
      let mut rows: Vec<(Uuid, String, String)> = mem
        .workspaces
        .iter()
        .filter(|((orig, _), _)| orig == origin)
        .map(|((_, id), (name, pos))| (*id, name.clone(), pos.clone()))
        .collect();
      rows.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.cmp(&b.1)));
      rows.into_iter().map(|(id, name, _)| (id, name)).collect()
    }
  }

  /// Delete a workspace, its views, its documents, and its local preferences.
  pub fn delete_workspace(&self, origin: &str, ws_id: Uuid) -> Result<(), String> {
    let ws_views: Vec<View> = self
      .list_views(origin)
      .into_iter()
      .filter(|v| v.workspace_id == ws_id)
      .collect();

    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = self.lock_store()?;
      for v in &ws_views {
        let _ = store.delete_doc(origin, &v.object_id.to_string());
      }
      store
        .delete_workspace(origin, &ws_id.to_string())
        .map_err(|e| e.to_string())?;
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
      for v in &ws_views {
        mem.views.remove(&(origin.to_string(), v.id));
        mem.trash_batches.remove(&(origin.to_string(), v.id));
        let doc_key = (origin.to_string(), v.object_id.to_string());
        mem.docs.remove(&doc_key);
        mem.versions.remove(&doc_key);
      }
      mem.workspaces.remove(&(origin.to_string(), ws_id));
    }

    for v in &ws_views {
      let _ = self.set_view_icon(v.id, None);
    }
    let _ = self.remove_pref(&format!("favoritePages:{ws_id}"));
    let _ = self.remove_pref(&format!("workspaceName:{origin}:{ws_id}"));
    Ok(())
  }

  // ── Documents, Boards, Version History ──────────────────────────────────

  /// Parse `markdown` through single-source `mica_md::parse_body`, encode into
  /// a [`MicaDoc`], and persist under `(origin, doc_id)`.
  pub fn save_markdown(&self, origin: &str, doc_id: Uuid, markdown: &str) -> Result<(), String> {
    let doc = doc_from_markdown(markdown, Some(self.client_id));
    self.save_raw_doc(origin, &doc_id.to_string(), &doc)
  }

  /// Record a document creation event into the local usage activity ledger.
  pub fn record_doc_created(
    &self,
    origin: &str,
    workspace_id: Uuid,
    doc_id: Uuid,
    source: mica_core::ActivitySource,
    markdown: &str,
  ) {
    let doc = doc_from_markdown(markdown, Some(self.client_id));
    let initial_words = mica_core::word_count::count_doc(&doc).words;
    self.record_activity(
      origin,
      workspace_id,
      doc_id,
      source,
      true,
      0,
      initial_words,
      0,
    );
  }

  /// Persist `markdown` AND record the edit's word delta into the local usage
  /// activity ledger (`local_writing_activity_hourly` on desktop, `MemoryVault`
  /// on `wasm32`).
  pub fn save_markdown_tracked(
    &self,
    origin: &str,
    workspace_id: Uuid,
    doc_id: Uuid,
    markdown: &str,
    source: mica_core::ActivitySource,
  ) -> Result<(), String> {
    let doc_key = doc_id.to_string();
    let prev_words: u64 = {
      #[cfg(not(target_arch = "wasm32"))]
      {
        self
          .lock_store()
          .ok()
          .and_then(|s| s.doc_word_count(origin, &doc_key).ok().flatten())
          .map(|w| w.max(0) as u64)
          .unwrap_or(0)
      }
      #[cfg(target_arch = "wasm32")]
      {
        self
          .load_raw_doc(origin, &doc_key)
          .map(|d| mica_core::word_count::count_doc(&d).words)
          .unwrap_or(0)
      }
    };
    let doc = doc_from_markdown(markdown, Some(self.client_id));
    let next_words = mica_core::word_count::count_doc(&doc).words;
    self.save_raw_doc(origin, &doc_key, &doc)?;

    let added = next_words.saturating_sub(prev_words);
    let deleted = prev_words.saturating_sub(next_words);
    self.record_activity(
      origin,
      workspace_id,
      doc_id,
      source,
      false,
      1,
      added,
      deleted,
    );
    Ok(())
  }

  #[allow(clippy::too_many_arguments)]
  fn record_activity(
    &self,
    origin: &str,
    workspace_id: Uuid,
    doc_id: Uuid,
    source: mica_core::ActivitySource,
    is_created: bool,
    edit_events: u64,
    words_added: u64,
    words_deleted: u64,
  ) {
    let ws_str = workspace_id.to_string();
    let doc_str = doc_id.to_string();
    #[cfg(not(target_arch = "wasm32"))]
    {
      if let Ok(store) = self.lock_store() {
        let _ = store.record_writing_activity(
          origin,
          &ws_str,
          &doc_str,
          source,
          is_created,
          edit_events,
          words_added,
          words_deleted,
        );
      }
    }
    #[cfg(target_arch = "wasm32")]
    {
      if let Ok(mut mem) = self.lock_memory() {
        let now_secs = chrono::Utc::now().timestamp();
        let bucket_hour = now_secs - now_secs.rem_euclid(3_600);
        if let Some((_, _, existing)) = mem.activity.iter_mut().find(|(o, w, r)| {
          o == origin
            && w == &ws_str
            && r.bucket_hour_secs == bucket_hour
            && r.document_id == doc_str
            && r.source == source
        }) {
          existing.is_created |= is_created;
          existing.edit_events = existing.edit_events.saturating_add(edit_events);
          existing.words_added = existing.words_added.saturating_add(words_added);
          existing.words_deleted = existing.words_deleted.saturating_add(words_deleted);
        } else {
          mem.activity.push((
            origin.to_string(),
            ws_str,
            mica_core::HourlyActivityRow {
              bucket_hour_secs: bucket_hour,
              document_id: doc_str,
              source,
              is_created,
              edit_events,
              words_added,
              words_deleted,
            },
          ));
        }
      }
    }
  }

  /// Compute the full [`mica_core::UsageStatsOverview`] for `origin` (local mode).
  pub fn usage_overview(&self, origin: &str) -> Result<mica_core::UsageStatsOverview, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let mut overview = {
        let store = self.lock_store()?;
        store
          .usage_overview(origin, &HashMap::new())
          .map_err(|e| e.to_string())?
      };
      // `LocalStore::open` seeds an unused `"local"` placeholder workspace; drop it
      // whenever real UUID workspaces exist (matching `VaultStore::list_workspaces`)
      // so the UI never displays a duplicate empty workspace row.
      let has_uuid_ws = overview
        .workspaces
        .iter()
        .any(|w| Uuid::parse_str(&w.workspace_id).is_ok());
      if has_uuid_ws {
        overview.workspaces.retain(|w| {
          Uuid::parse_str(&w.workspace_id).is_ok()
            || w.live_pages > 0
            || w.folders > 0
            || w.trashed_pages > 0
        });
        overview.summary.workspace_count = overview.workspaces.len() as u64;
      }
      // Prefer user-renamed workspace display names persisted in prefs.
      for ws in &mut overview.workspaces {
        if let Ok(ws_uuid) = Uuid::parse_str(&ws.workspace_id)
          && let Some(custom) = self.get_pref(&format!("workspaceName:{origin}:{ws_uuid}"))
          && !custom.trim().is_empty()
        {
          ws.name = custom;
        }
      }
      Ok(overview)
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mem = self.lock_memory()?;
      let mut summary = mica_core::UsageSummary::default();
      let mut health = mica_core::UsageHealth::default();
      let mut workspaces = Vec::new();
      let mut ws_ids: Vec<(Uuid, String)> = mem
        .workspaces
        .iter()
        .filter(|((o, _), _)| o == origin)
        .map(|((_, id), (name, _))| (*id, name.clone()))
        .collect();
      let mut seen_ws: HashSet<Uuid> = ws_ids.iter().map(|(id, _)| *id).collect();
      for ((o, _), v) in &mem.views {
        if o == origin && seen_ws.insert(v.workspace_id) {
          let label = self
            .get_pref(&format!("workspaceName:{origin}:{}", v.workspace_id))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Mica 本地工作区".to_string());
          ws_ids.push((v.workspace_id, label));
        }
      }
      ws_ids.sort_by(|a, b| a.1.cmp(&b.1));
      for (ws_id, ws_name) in ws_ids {
        let mut live_pages = 0u64;
        let mut folders = 0u64;
        let mut trashed_pages = 0u64;
        let mut words = 0u64;
        let mut chars = 0u64;
        let mut empty_pages = 0u64;
        let mut storage_bytes = 0u64;
        for ((o, _), v) in &mem.views {
          if o != origin || v.workspace_id != ws_id {
            continue;
          }
          if v.is_deleted {
            if v.object_type != "folder" {
              trashed_pages += 1;
            }
          } else if v.object_type == "folder" {
            folders += 1;
          } else {
            live_pages += 1;
            let key = (origin.to_string(), v.object_id.to_string());
            let wc = mem
              .docs
              .get(&key)
              .and_then(|bytes| {
                storage_bytes = storage_bytes.saturating_add(bytes.len() as u64);
                MicaDoc::from_update(bytes).ok()
              })
              .map(|d| mica_core::word_count::count_doc(&d))
              .unwrap_or_default();
            words = words.saturating_add(wc.words);
            chars = chars.saturating_add(wc.chars);
            if wc.words == 0 {
              empty_pages += 1;
            }
          }
        }
        summary.workspace_count += 1;
        summary.folder_count += folders;
        summary.live_page_count += live_pages;
        summary.trashed_page_count += trashed_pages;
        summary.total_words += words;
        summary.total_chars += chars;
        summary.storage_bytes = summary.storage_bytes.saturating_add(storage_bytes);
        health.empty_pages += empty_pages;
        health.orphan_pages += live_pages;
        workspaces.push(mica_core::WorkspaceUsageItem {
          workspace_id: ws_id.to_string(),
          name: ws_name,
          role: "owner".to_string(),
          live_pages,
          folders,
          trashed_pages,
          words,
          chars,
          image_count: 0,
          attachment_count: 0,
          storage_bytes,
          quota_bytes: 0,
          link_edges: 0,
          orphan_pages: live_pages,
          empty_pages,
          public_shares: 0,
          unresolved_comments: 0,
          updated_at: None,
        });
      }
      Ok(mica_core::UsageStatsOverview {
        generated_at: mica_core::usage_stats::format_unix_rfc3339(chrono::Utc::now().timestamp()),
        summary,
        health,
        workspaces,
      })
    }
  }

  /// Compute [`mica_core::UsageStatsActivity`] for `origin` folded into the
  /// user's local timezone offset (`tz_offset_minutes`).
  pub fn usage_activity(
    &self,
    origin: &str,
    range_label: &str,
    tz_offset_minutes: i32,
    workspace_id: Option<&str>,
    include_imports: bool,
  ) -> Result<mica_core::UsageStatsActivity, String> {
    let (canonical, range_days) =
      mica_core::usage_stats::parse_activity_range(Some(range_label)).unwrap_or(("30d", 30));
    #[cfg(not(target_arch = "wasm32"))]
    {
      let store = self.lock_store()?;
      store
        .usage_activity(
          origin,
          canonical,
          range_days,
          tz_offset_minutes,
          workspace_id,
          include_imports,
        )
        .map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mem = self.lock_memory()?;
      let rows: Vec<mica_core::HourlyActivityRow> = mem
        .activity
        .iter()
        .filter(|(o, w, _)| o == origin && workspace_id.is_none_or(|ws| ws == w))
        .map(|(_, _, r)| r.clone())
        .collect();
      let tracking_since = rows.iter().map(|r| r.bucket_hour_secs).min();
      Ok(mica_core::usage_stats::fold_hourly_to_daily(
        &rows,
        &[],
        tracking_since,
        canonical,
        range_days,
        180,
        tz_offset_minutes,
        workspace_id.map(str::to_string),
        include_imports,
        chrono::Utc::now().timestamp(),
      ))
    }
  }

  /// Load `(origin, doc_id)` as a [`MicaDoc`] and serialize its body through
  /// single-source `mica_md::serialize_body`.
  pub fn load_markdown(&self, origin: &str, doc_id: Uuid) -> Option<String> {
    Some(markdown_from_doc(
      &self.load_raw_doc(origin, &doc_id.to_string())?,
    ))
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
    board_json_from_doc(&self.load_raw_doc(origin, &board_doc_key(board_id).ok()?)?)
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
      let created = self
        .lock_store()?
        .create_local_version(origin, &doc_key, effective_label)
        .map_err(|e| e.to_string())?;
      if created.is_none() {
        return Err(format!("document {doc_id} has no saved snapshot yet"));
      }
      Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
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
          (
            v.id,
            v.label.unwrap_or_else(|| "自动快照".to_string()),
            v.created_at,
          )
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

  /// Load a stored version snapshot as Markdown WITHOUT mutating the live
  /// document (used for read-only Version History diff previews).
  pub fn load_version_markdown(
    &self,
    origin: &str,
    doc_id: Uuid,
    version_id: &str,
  ) -> Result<String, String> {
    let doc_key = doc_id.to_string();
    #[cfg(not(target_arch = "wasm32"))]
    {
      let state = self
        .lock_store()?
        .local_version_state(origin, &doc_key, version_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("version {version_id} not found"))?;
      let doc = std::panic::catch_unwind(|| MicaDoc::from_update(&state))
        .map_err(|_| format!("corrupt version snapshot {version_id}"))?
        .map_err(|e| e.to_string())?;
      Ok(markdown_from_doc(&doc))
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mem = self.lock_memory()?;
      let idx: usize = version_id
        .parse()
        .map_err(|_| format!("invalid version id {version_id}"))?;
      let state = mem
        .versions
        .get(&(origin.to_string(), doc_key))
        .and_then(|list| list.get(idx))
        .map(|(_, bytes)| bytes.clone())
        .ok_or_else(|| format!("version {version_id} not found"))?;
      let doc = MicaDoc::from_update(&state).map_err(|e| e.to_string())?;
      Ok(markdown_from_doc(&doc))
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
      let restored = self
        .lock_store()?
        .restore_local_version(origin, &doc_key, version_id, self.client_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("version {version_id} not found"))?;
      Ok(markdown_from_doc(&restored))
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
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
      self
        .lock_store()?
        .save_doc(origin, doc_key, doc)
        .map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mut mem = self.lock_memory()?;
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
      self
        .store
        .lock()
        .ok()?
        .load_doc(origin, doc_key, self.client_id)
        .ok()?
    }
    #[cfg(target_arch = "wasm32")]
    {
      let mem = self.memory.lock().ok()?;
      let bytes = mem.docs.get(&(origin.to_string(), doc_key.to_string()))?;
      MicaDoc::from_update_with_client_id(bytes, Some(self.client_id)).ok()
    }
  }
}

#[cfg(target_os = "windows")]
fn env_dir(name: &str) -> Option<PathBuf> {
  std::env::var_os(name)
    .filter(|v| !v.is_empty())
    .map(PathBuf::from)
}

#[cfg(not(target_arch = "wasm32"))]
fn desktop_roaming_config_dir() -> Option<PathBuf> {
  // On Windows, `dirs_next::config_dir()` queries `SHGetKnownFolderPath` and
  // ignores `%APPDATA%` overrides used by isolated smoke tests (#179). Honor an
  // explicit `APPDATA` override first so `VaultStore` matches `single_instance`.
  #[cfg(target_os = "windows")]
  if let Some(dir) = env_dir("APPDATA") {
    return Some(dir);
  }
  dirs_next::config_dir()
}

#[cfg(not(target_arch = "wasm32"))]
fn desktop_local_data_dir() -> Option<PathBuf> {
  #[cfg(target_os = "windows")]
  if let Some(dir) = env_dir("LOCALAPPDATA") {
    return Some(dir);
  }
  dirs_next::data_local_dir()
}

#[cfg(not(target_arch = "wasm32"))]
fn legacy_flutter_db_path() -> Option<PathBuf> {
  #[cfg(any(target_os = "windows", target_os = "macos"))]
  let base = desktop_roaming_config_dir()?;
  #[cfg(not(any(target_os = "windows", target_os = "macos")))]
  let base = dirs_next::data_dir()?;
  Some(base.join("mica").join("local").join("store.db"))
}

#[cfg(not(target_arch = "wasm32"))]
fn early_rust_db_path() -> Option<PathBuf> {
  Some(
    desktop_local_data_dir()?
      .join("mica")
      .join("vault.sqlite"),
  )
}

#[cfg(not(target_arch = "wasm32"))]
fn legacy_flutter_prefs_path() -> Option<PathBuf> {
  Some(desktop_roaming_config_dir()?.join("mica").join("prefs.json"))
}

/// Pure, testable desktop SQLite vault path resolution:
/// 1. `MICA_DATA_DIR` override -> `<MICA_DATA_DIR>/store.db`
/// 2. Existing canonical legacy Flutter DB (`<app_dir>/mica/local/store.db`)
/// 3. Existing early pure-Rust DB (`<data_local_dir>/mica/vault.sqlite`)
/// 4. Fresh install default -> canonical `<app_dir>/mica/local/store.db`
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn resolve_default_vault_path(
  env_override: Option<&str>,
  legacy_db: Option<PathBuf>,
  early_rust_db: Option<PathBuf>,
) -> Option<PathBuf> {
  if let Some(dir) = env_override.map(str::trim).filter(|s| !s.is_empty()) {
    return Some(PathBuf::from(dir).join("store.db"));
  }
  if let Some(ref legacy) = legacy_db
    && legacy.exists()
  {
    return Some(legacy.clone());
  }
  if let Some(ref early) = early_rust_db
    && early.exists()
  {
    return Some(early.clone());
  }
  legacy_db.or(early_rust_db)
}

/// One-time import of allowlisted, strictly non-secret UI preferences from
/// legacy Flutter `<config_dir>/mica/prefs.json`. Never reads or migrates
/// tokens, credentials, or auth secrets.
#[cfg(not(target_arch = "wasm32"))]
fn seed_non_secret_prefs_from_legacy(legacy_path: &Path) -> BTreeMap<String, String> {
  let mut out = BTreeMap::new();
  let Ok(raw) = std::fs::read_to_string(legacy_path) else {
    return out;
  };
  let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&raw) else {
    return out;
  };
  if let Some(b) = map.get("darkMode").and_then(serde_json::Value::as_bool) {
    out.insert(
      "darkMode".to_string(),
      if b { "true" } else { "false" }.to_string(),
    );
  }
  if let Some(w) = map.get(PREF_PAGE_WIDTH).and_then(serde_json::Value::as_f64)
    && w.is_finite()
    && (400.0..=2400.0).contains(&w)
  {
    out.insert(PREF_PAGE_WIDTH.to_string(), w.to_string());
  }
  if let Some(w) = map
    .get(PREF_SIDEBAR_WIDTH)
    .and_then(serde_json::Value::as_f64)
    && w.is_finite()
    && (180.0..=640.0).contains(&w)
  {
    out.insert(PREF_SIDEBAR_WIDTH.to_string(), w.to_string());
  }
  if let Some(arr) = map
    .get(PREF_KNOWN_SERVERS)
    .and_then(serde_json::Value::as_array)
  {
    let mut servers = Vec::new();
    for item in arr {
      if let Some(s) = item.as_str() {
        let norm = normalize_origin(s);
        if !norm.is_empty() && !servers.iter().any(|x| x == &norm) {
          servers.push(norm);
        }
      }
    }
    if !servers.is_empty()
      && let Ok(encoded) = serde_json::to_string(&servers)
    {
      out.insert(PREF_KNOWN_SERVERS.to_string(), encoded);
    }
  }
  out
}

fn canonical_cred_origin(origin: &str) -> String {
  crate::normalize_server_url(origin).unwrap_or_else(|_| normalize_origin(origin))
}

fn normalize_origin(origin: &str) -> String {
  origin.trim().trim_end_matches('/').to_string()
}

fn next_sibling_position(
  all_views: &[View],
  workspace_id: Uuid,
  parent_view_id: Option<Uuid>,
) -> String {
  let max = all_views
    .iter()
    .filter(|v| {
      !v.is_deleted && v.workspace_id == workspace_id && v.parent_view_id == parent_view_id
    })
    .filter_map(|v| v.position.parse::<i64>().ok())
    .max()
    .unwrap_or(0);
  format!("{:010}", max + 10)
}

fn live_ancestor_path(view: &View, live_by_id: &HashMap<Uuid, &View>) -> String {
  let mut names = Vec::new();
  let mut seen = HashSet::from([view.id]);
  let mut cursor = view.parent_view_id;
  while let Some(pid) = cursor {
    if !seen.insert(pid) {
      break;
    }
    let Some(parent) = live_by_id.get(&pid) else {
      break;
    };
    names.push(parent.name.as_str());
    cursor = parent.parent_view_id;
  }
  names.reverse();
  names.join(" / ")
}

fn format_deleted_label(view: &View) -> String {
  if view.updated_at != DateTime::UNIX_EPOCH {
    view.updated_at.format("%Y-%m-%d %H:%M").to_string()
  } else {
    "已删除".to_string()
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
  let body: Vec<Block> = doc
    .to_blocks()
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
  let parent_view_id = lv
    .parent_id
    .as_deref()
    .and_then(|s| Uuid::parse_str(s).ok());
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
fn collect_memory_subtree(
  views: &HashMap<(String, Uuid), View>,
  origin: &str,
  root_id: Uuid,
  live_only: bool,
) -> Vec<Uuid> {
  let mut children_of: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
  for ((orig, id), v) in views {
    if orig == origin
      && (!live_only || !v.is_deleted)
      && let Some(pid) = v.parent_view_id
    {
      children_of.entry(pid).or_default().push(*id);
    }
  }
  if live_only
    && views
      .get(&(origin.to_string(), root_id))
      .is_none_or(|r| r.is_deleted)
  {
    return Vec::new();
  }
  let mut out = Vec::new();
  let mut seen = HashSet::from([root_id]);
  let mut queue = VecDeque::from([root_id]);
  while let Some(curr) = queue.pop_front() {
    if views.contains_key(&(origin.to_string(), curr)) {
      out.push(curr);
    }
    if let Some(kids) = children_of.get(&curr) {
      for &kid in kids {
        if seen.insert(kid) {
          queue.push_back(kid);
        }
      }
    }
  }
  out
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
    kind: &str,
    parent: Option<Uuid>,
    pos: &str,
  ) -> View {
    let now = Utc::now();
    View {
      id: Uuid::new_v4(),
      workspace_id: ws,
      parent_view_id: parent,
      object_id: doc,
      object_type: kind.to_string(),
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
    assert!(
      loaded.contains("# Title") && loaded.contains("**bold**") && loaded.contains("- Item 2")
    );

    let board_json = r#"{"nodes":[{"id":"n1","text":"Mica Board"}],"edges":[]}"#;
    vault
      .save_board_json(vault.origin(), "main-board", board_json)
      .unwrap();
    assert_eq!(
      vault
        .load_board_json(vault.origin(), "main-board")
        .as_deref(),
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
    assert_eq!((listed[0].id, listed[1].id), (folder.id, page.id));

    vault.purge_view("local", page.id).unwrap();
    assert_eq!(vault.list_views("local").len(), 1);
  }

  #[test]
  fn version_snapshots_increment_count_and_load_readonly_markdown() {
    let vault = VaultStore::in_memory();
    let doc_id = Uuid::new_v4();
    assert_eq!(vault.list_version_count("local", doc_id), 0);
    assert!(
      vault
        .create_version_snapshot("local", doc_id, Some("v1"))
        .is_err()
    );

    vault.save_markdown("local", doc_id, "# Draft 1").unwrap();
    let after_save = vault.list_version_count("local", doc_id);
    assert!(after_save >= 1);

    vault
      .create_version_snapshot("local", doc_id, Some("Checkpoint 1"))
      .unwrap();
    assert_eq!(vault.list_version_count("local", doc_id), after_save + 1);

    let cp_id = vault
      .list_versions("local", doc_id)
      .into_iter()
      .find(|(_, lbl, _)| lbl == "Checkpoint 1")
      .map(|(id, _, _)| id)
      .unwrap();

    vault.save_markdown("local", doc_id, "# Draft 2").unwrap();
    // Non-mutating version preview MUST NOT alter the live doc.
    assert!(
      vault
        .load_version_markdown("local", doc_id, &cp_id)
        .unwrap()
        .contains("# Draft 1")
    );
    assert!(
      vault
        .load_markdown("local", doc_id)
        .unwrap()
        .contains("# Draft 2")
    );

    assert!(
      vault
        .restore_version_markdown("local", doc_id, &cp_id)
        .unwrap()
        .contains("# Draft 1")
    );
    vault
      .create_version_snapshot("local", doc_id, None)
      .unwrap();
    assert!(vault.list_version_count("local", doc_id) >= after_save + 2);
  }

  #[test]
  fn prefs_round_trip_and_persist_to_json_file() {
    let dir = std::env::temp_dir().join(format!("mica_prefs_test_{}", Uuid::new_v4()));
    let db_str = dir.join("vault.sqlite").to_str().unwrap().to_string();
    let (ws_id, fav_a, fav_b, view_id) = (
      Uuid::new_v4(),
      Uuid::new_v4(),
      Uuid::new_v4(),
      Uuid::new_v4(),
    );

    {
      let vault = VaultStore::open_at(&db_str).unwrap();
      assert_eq!((vault.page_width(), vault.sidebar_width()), (800.0, 280.0));
      assert_eq!(vault.active_origin(), DEFAULT_ORIGIN);
      assert!(vault.bool_pref("showLineNumbers", true));
      assert!(!vault.bool_pref("showLineNumbers", false));

      vault.set_page_width(920.0).unwrap();
      vault.set_sidebar_width(310.0).unwrap();
      vault
        .set_favorites_for(ws_id, &[fav_a, fav_b, fav_a])
        .unwrap();
      vault.set_view_icon(view_id, Some("📘")).unwrap();
      vault
        .set_known_servers(&[
          "https://notes.mica.dev/".into(),
          "https://notes.mica.dev".into(),
          "http://localhost:8080/".into(),
        ])
        .unwrap();
      vault
        .save_server_credential("https://notes.mica.dev/", "alice@mica.dev", "tok-abc")
        .unwrap();
      vault.set_active_origin("https://notes.mica.dev/").unwrap();
      vault.set_bool_pref("focusMode", true).unwrap();
    }

    assert!(dir.join(PREFS_FILENAME).exists());
    {
      let reopened = VaultStore::open_at(&db_str).unwrap();
      assert_eq!(
        (reopened.page_width(), reopened.sidebar_width()),
        (920.0, 310.0)
      );
      assert_eq!(reopened.favorites_for(ws_id), vec![fav_a, fav_b]);
      assert_eq!(reopened.view_icon(view_id).as_deref(), Some("📘"));
      assert_eq!(
        reopened.known_servers(),
        vec![
          "https://notes.mica.dev".to_string(),
          "http://localhost:8080".to_string()
        ]
      );
      assert_eq!(
        reopened.saved_server_credentials(),
        vec![(
          "https://notes.mica.dev".into(),
          "alice@mica.dev".into(),
          "tok-abc".into()
        )]
      );
      assert_eq!(reopened.active_origin(), "https://notes.mica.dev");
      assert!(reopened.bool_pref("focusMode", false));
      assert_eq!(
        reopened
          .snapshot_prefs()
          .entries
          .get("focusMode")
          .map(String::as_str),
        Some("true")
      );

      reopened
        .clear_server_credential("https://notes.mica.dev")
        .unwrap();
      assert!(reopened.saved_server_credentials().is_empty());
      reopened.set_view_icon(view_id, None).unwrap();
      assert!(reopened.view_icon(view_id).is_none());
      reopened.remove_pref("focusMode").unwrap();
      assert!(!reopened.bool_pref("focusMode", false));
    }
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn reorder_views_renumbers_reparents_and_enforces_tree_rules() {
    let vault = VaultStore::in_memory();
    let ws = Uuid::new_v4();
    let folder_a = make_view(ws, Uuid::new_v4(), "Folder A", "folder", None, "0000000010");
    let folder_b = make_view(
      ws,
      Uuid::new_v4(),
      "Folder B",
      "folder",
      Some(folder_a.id),
      "0000000010",
    );
    let page_1 = make_view(ws, Uuid::new_v4(), "Page 1", "page", None, "0000000020");
    let page_2 = make_view(ws, Uuid::new_v4(), "Page 2", "page", None, "0000000030");
    for v in [&folder_a, &folder_b, &page_1, &page_2] {
      vault.save_view(DEFAULT_ORIGIN, v).unwrap();
    }

    vault
      .reorder_views(DEFAULT_ORIGIN, Some(folder_a.id), &[page_2.id, page_1.id])
      .unwrap();
    let views = vault.list_views(DEFAULT_ORIGIN);
    let p2 = views.iter().find(|v| v.id == page_2.id).unwrap();
    let p1 = views.iter().find(|v| v.id == page_1.id).unwrap();
    assert_eq!(
      (p2.parent_view_id, p2.position.as_str()),
      (Some(folder_a.id), "0000000010")
    );
    assert_eq!(
      (p1.parent_view_id, p1.position.as_str()),
      (Some(folder_a.id), "0000000020")
    );

    assert!(
      vault
        .reorder_views(DEFAULT_ORIGIN, Some(page_1.id), &[page_2.id])
        .is_err()
    );
    assert!(
      vault
        .reorder_views(DEFAULT_ORIGIN, Some(folder_b.id), &[folder_a.id])
        .is_err()
    );
  }

  #[test]
  fn trash_restore_promote_purge_and_build_trash_entries() {
    let vault = VaultStore::in_memory();
    let ws = Uuid::new_v4();
    let live_parent = make_view(ws, Uuid::new_v4(), "Projects", "folder", None, "0000000010");
    let folder = make_view(
      ws,
      Uuid::new_v4(),
      "Archive",
      "folder",
      Some(live_parent.id),
      "0000000010",
    );
    let sub_folder = make_view(
      ws,
      Uuid::new_v4(),
      "Old",
      "folder",
      Some(folder.id),
      "0000000010",
    );
    let (doc_id, standalone_doc) = (Uuid::new_v4(), Uuid::new_v4());
    let leaf_page = make_view(
      ws,
      doc_id,
      "Spec",
      "page",
      Some(sub_folder.id),
      "0000000010",
    );
    let standalone = make_view(ws, standalone_doc, "Scratch", "page", None, "0000000020");

    for v in [&live_parent, &folder, &sub_folder, &leaf_page, &standalone] {
      vault.save_view(DEFAULT_ORIGIN, v).unwrap();
    }
    vault
      .save_markdown(DEFAULT_ORIGIN, doc_id, "Spec body")
      .unwrap();
    vault
      .save_markdown(DEFAULT_ORIGIN, standalone_doc, "Scratch body")
      .unwrap();

    let trashed = vault.trash_view_subtree(DEFAULT_ORIGIN, folder.id).unwrap();
    assert_eq!(trashed.len(), 3);

    let entries = vault.build_trash_entries(&vault.list_views(DEFAULT_ORIGIN));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].view.id, folder.id);
    assert_eq!(entries[0].original_path, "Projects");
    assert_eq!((entries[0].child_folders, entries[0].child_pages), (1, 1));
    assert!(!entries[0].deleted_label.is_empty());

    // Restoring a child whose parent is still deleted promotes child to root.
    assert_eq!(
      vault
        .restore_view_subtree(DEFAULT_ORIGIN, leaf_page.id)
        .unwrap(),
      vec![leaf_page.id]
    );
    let leaf_after = vault
      .list_views(DEFAULT_ORIGIN)
      .into_iter()
      .find(|v| v.id == leaf_page.id)
      .unwrap();
    assert!(!leaf_after.is_deleted && leaf_after.parent_view_id.is_none());

    assert_eq!(
      vault
        .restore_view_subtree(DEFAULT_ORIGIN, folder.id)
        .unwrap()
        .len(),
      2
    );
    assert!(
      vault
        .list_views(DEFAULT_ORIGIN)
        .iter()
        .all(|v| !v.is_deleted)
    );

    vault.trash_view_subtree(DEFAULT_ORIGIN, folder.id).unwrap();
    vault
      .trash_view_subtree(DEFAULT_ORIGIN, standalone.id)
      .unwrap();
    assert_eq!(vault.purge_all_trash(DEFAULT_ORIGIN, ws).unwrap(), 3);
    assert!(
      vault
        .load_markdown(DEFAULT_ORIGIN, standalone_doc)
        .is_none()
    );
  }

  #[test]
  fn clone_view_copies_page_and_folder_subtrees_and_workspace_crud() {
    let vault = VaultStore::in_memory();
    let ws = Uuid::new_v4();
    vault
      .save_workspace_name(DEFAULT_ORIGIN, ws, "Engineering")
      .unwrap();
    assert!(
      vault
        .list_workspaces(DEFAULT_ORIGIN)
        .iter()
        .any(|(id, n)| *id == ws && n == "Engineering")
    );
    vault
      .save_workspace_name(DEFAULT_ORIGIN, ws, "Core Eng")
      .unwrap();
    assert!(
      vault
        .list_workspaces(DEFAULT_ORIGIN)
        .iter()
        .any(|(id, n)| *id == ws && n == "Core Eng")
    );

    let folder = make_view(ws, Uuid::new_v4(), "Guides", "folder", None, "0000000010");
    let doc_id = Uuid::new_v4();
    let page = make_view(
      ws,
      doc_id,
      "Onboarding",
      "page",
      Some(folder.id),
      "0000000010",
    );
    vault.save_view(DEFAULT_ORIGIN, &folder).unwrap();
    vault.save_view(DEFAULT_ORIGIN, &page).unwrap();
    vault
      .save_markdown(DEFAULT_ORIGIN, doc_id, "# Welcome\n\nStep one.")
      .unwrap();
    vault.set_view_icon(page.id, Some("🧭")).unwrap();

    let cloned_folder = vault
      .clone_view(DEFAULT_ORIGIN, folder.id, "Guides")
      .unwrap();
    assert_ne!(cloned_folder.id, folder.id);
    assert_ne!(cloned_folder.name, "Guides");
    let cloned_child = vault
      .list_views(DEFAULT_ORIGIN)
      .into_iter()
      .find(|v| v.parent_view_id == Some(cloned_folder.id))
      .expect("cloned child page");
    assert_ne!(cloned_child.id, page.id);
    assert_eq!(cloned_child.icon.as_deref(), Some("🧭"));
    assert!(
      vault
        .load_markdown(DEFAULT_ORIGIN, cloned_child.object_id)
        .unwrap()
        .contains("Step one.")
    );

    vault.delete_workspace(DEFAULT_ORIGIN, ws).unwrap();
    assert!(
      vault
        .list_views(DEFAULT_ORIGIN)
        .into_iter()
        .all(|v| v.workspace_id != ws)
    );
    assert!(vault.load_markdown(DEFAULT_ORIGIN, doc_id).is_none());
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn default_vault_path_adopts_legacy_store_db_and_preserves_early_rust_fallback() {
    let root = std::env::temp_dir().join(format!("mica_vault_path_test_{}", Uuid::new_v4()));
    let legacy_db = root
      .join("roaming")
      .join("mica")
      .join("local")
      .join("store.db");
    let early_db = root.join("local").join("mica").join("vault.sqlite");
    let custom_dir = root.join("custom");

    // 1. MICA_DATA_DIR always takes precedence.
    assert_eq!(
      resolve_default_vault_path(
        Some(custom_dir.to_str().unwrap()),
        Some(legacy_db.clone()),
        Some(early_db.clone()),
      ),
      Some(custom_dir.join("store.db"))
    );

    // 2. Fresh install (neither exists) defaults to canonical legacy path `store.db`.
    assert_eq!(
      resolve_default_vault_path(None, Some(legacy_db.clone()), Some(early_db.clone())),
      Some(legacy_db.clone())
    );

    // 3. Early pure-Rust install (`vault.sqlite` exists, `store.db` does not) is preserved.
    std::fs::create_dir_all(early_db.parent().unwrap()).unwrap();
    std::fs::write(&early_db, b"").unwrap();
    assert_eq!(
      resolve_default_vault_path(None, Some(legacy_db.clone()), Some(early_db.clone())),
      Some(early_db.clone())
    );

    // 4. Legacy Flutter `store.db` takes precedence once present.
    std::fs::create_dir_all(legacy_db.parent().unwrap()).unwrap();
    std::fs::write(&legacy_db, b"").unwrap();
    assert_eq!(
      resolve_default_vault_path(None, Some(legacy_db.clone()), Some(early_db.clone())),
      Some(legacy_db.clone())
    );

    let _ = std::fs::remove_dir_all(&root);
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn opening_legacy_local_store_db_preserves_notes_folders_and_snapshots() {
    let root = std::env::temp_dir().join(format!("mica_legacy_db_test_{}", Uuid::new_v4()));
    let legacy_db = root.join("mica").join("local").join("store.db");
    std::fs::create_dir_all(legacy_db.parent().unwrap()).unwrap();
    let db_str = legacy_db.to_str().unwrap().to_string();

    let ws_id = Uuid::new_v4();
    let folder_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let page_id = Uuid::new_v4();

    // Seed directly with `mica_core::LocalStore` (exact engine used by legacy Flutter FFI).
    {
      let store = LocalStore::open(&db_str).unwrap();
      store
        .save_workspace(&LocalWorkspace {
          id: ws_id.to_string(),
          name: "旧版 Flutter 本地工作区".to_string(),
          position: "0000000010".to_string(),
          origin: DEFAULT_ORIGIN.to_string(),
          role: "owner".to_string(),
        })
        .unwrap();
      store
        .save_view(&LocalView {
          id: folder_id.to_string(),
          workspace_id: ws_id.to_string(),
          parent_id: None,
          object_id: folder_id.to_string(),
          object_type: "folder".to_string(),
          name: "历史笔记".to_string(),
          position: "0000000010".to_string(),
          trashed: false,
          origin: DEFAULT_ORIGIN.to_string(),
        })
        .unwrap();
      store
        .save_view(&LocalView {
          id: page_id.to_string(),
          workspace_id: ws_id.to_string(),
          parent_id: Some(folder_id.to_string()),
          object_id: doc_id.to_string(),
          object_type: "page".to_string(),
          name: "重要记录".to_string(),
          position: "0000000010".to_string(),
          trashed: false,
          origin: DEFAULT_ORIGIN.to_string(),
        })
        .unwrap();
      let doc = doc_from_markdown(
        "# 旧版标题\n\n这段内容来自 v0.14.11 Flutter 本地库。",
        Some(1),
      );
      store
        .save_doc(DEFAULT_ORIGIN, &doc_id.to_string(), &doc)
        .unwrap();
      store
        .create_local_version(DEFAULT_ORIGIN, &doc_id.to_string(), "v0.14.11 快照")
        .unwrap();
    }

    // Reopen through pure-Rust `VaultStore` and confirm zero-migration readability.
    {
      let vault = VaultStore::open_at(&db_str).unwrap();
      let workspaces = vault.list_workspaces(DEFAULT_ORIGIN);
      assert!(
        workspaces
          .iter()
          .any(|(id, name)| *id == ws_id && name == "旧版 Flutter 本地工作区")
      );
      let views = vault.list_views(DEFAULT_ORIGIN);
      assert_eq!(views.len(), 2);
      assert!(
        views
          .iter()
          .any(|v| v.id == page_id && v.parent_view_id == Some(folder_id))
      );
      let md = vault.load_markdown(DEFAULT_ORIGIN, doc_id).unwrap();
      assert!(md.contains("这段内容来自 v0.14.11 Flutter 本地库。"));
      let versions = vault.list_versions(DEFAULT_ORIGIN, doc_id);
      assert!(
        versions
          .iter()
          .any(|(_, label, _)| label == "v0.14.11 快照")
      );
    }

    let _ = std::fs::remove_dir_all(&root);
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn legacy_prefs_seed_imports_only_non_secret_ui_fields() {
    let root = std::env::temp_dir().join(format!("mica_legacy_prefs_test_{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let legacy_json = root.join("prefs.json");
    std::fs::write(
      &legacy_json,
      r#"{
        "darkMode": true,
        "pageWidth": 960,
        "sidebarWidth": 320,
        "knownServers": ["https://notes.mica.dev/", "https://notes.mica.dev"],
        "serverCredentials": "[{\"token\":\"secret-should-not-import\"}]",
        "accessToken": "secret-jwt",
        "refreshToken": "secret-refresh"
      }"#,
    )
    .unwrap();

    let seeded = seed_non_secret_prefs_from_legacy(&legacy_json);
    assert_eq!(seeded.get("darkMode").map(String::as_str), Some("true"));
    assert_eq!(seeded.get(PREF_PAGE_WIDTH).map(String::as_str), Some("960"));
    assert_eq!(
      seeded.get(PREF_SIDEBAR_WIDTH).map(String::as_str),
      Some("320")
    );
    assert_eq!(
      seeded.get(PREF_KNOWN_SERVERS).map(String::as_str),
      Some(r#"["https://notes.mica.dev"]"#)
    );
    assert!(!seeded.contains_key(PREF_SERVER_CREDENTIALS));
    assert!(!seeded.contains_key("accessToken"));
    assert!(!seeded.contains_key("refreshToken"));

    let _ = std::fs::remove_dir_all(&root);
  }

  #[test]
  fn strip_web_secret_prefs_removes_credentials_while_keeping_ui_prefs() {
    let mut entries = BTreeMap::new();
    entries.insert("darkMode".to_string(), "true".to_string());
    entries.insert(PREF_PAGE_WIDTH.to_string(), "880".to_string());
    entries.insert(
      PREF_SERVER_CREDENTIALS.to_string(),
      r#"[{"origin":"https://mica.example.com","email":"a@b.c","token":"jwt","refresh_token":"rt"}]"#.to_string(),
    );

    assert!(strip_web_secret_prefs(&mut entries));
    assert!(!entries.contains_key(PREF_SERVER_CREDENTIALS));
    assert_eq!(entries.get("darkMode").map(String::as_str), Some("true"));
    assert_eq!(
      entries.get(PREF_PAGE_WIDTH).map(String::as_str),
      Some("880")
    );
    assert!(!strip_web_secret_prefs(&mut entries));
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn cloud_cache_origin_isolates_accounts_quarantines_legacy_rows_and_replays_flutter_outbox() {
    use mica_sync::{SyncConfig, SyncEffect, SyncEngine, SyncEvent};

    let root = std::env::temp_dir().join(format!("mica_p0_1_isolation_{}", Uuid::new_v4()));
    let db_path = root.join("local").join("store.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let db_str = db_path.to_str().unwrap().to_string();

    let server_raw = "https://Mica.Example.com:443/app/";
    let server_norm = crate::normalize_server_url(server_raw).unwrap();
    let key_a = crate::cloud_cache_origin(server_raw, "user-a").unwrap();
    let key_b = crate::cloud_cache_origin("https://mica.example.com/app", "user-b").unwrap();
    let legacy_bare = crate::legacy_cloud_cache_origin(server_raw).unwrap();
    assert_eq!(server_norm, "https://mica.example.com/app");
    assert_eq!(key_a, r#"["https://mica.example.com/app","user-a"]"#);

    let ws_a = Uuid::new_v4();
    let ws_b = Uuid::new_v4();
    let doc_a = Uuid::new_v4();
    let doc_b = Uuid::new_v4();
    let doc_legacy = Uuid::new_v4();

    // Seed as Flutter 0.14.11 (`LocalStore` directly):
    // - Account A has a page + an unpushed offline CRDT edit in `doc_update` outbox.
    // - Account B has its own separate page.
    // - A pre-#111 bare-URL row sits quarantined in the same DB.
    {
      let store = LocalStore::open(&db_str).unwrap();
      let mut base_a = doc_from_markdown("# Account A", Some(10));
      let sv0 = base_a.state_vector();
      let b_id = base_a
        .to_blocks()
        .into_iter()
        .find(|b| b.kind != "page")
        .unwrap()
        .id;
      base_a.text_insert(&b_id, 9, " offline edit");
      let offline_diff = base_a.encode_diff(&sv0).unwrap();

      store.save_doc(&key_a, &doc_a.to_string(), &base_a).unwrap();
      store
        .append_update(&key_a, &doc_a.to_string(), &offline_diff)
        .unwrap();

      let doc_b_state = doc_from_markdown("# Account B Secret", Some(20));
      store
        .save_doc(&key_b, &doc_b.to_string(), &doc_b_state)
        .unwrap();

      let legacy_doc = doc_from_markdown("# Pre-111 Quarantined", Some(30));
      store
        .save_doc(&legacy_bare, &doc_legacy.to_string(), &legacy_doc)
        .unwrap();
    }

    // Reopen through pure-Rust `VaultStore`:
    let vault = VaultStore::open_at(&db_str).unwrap();
    vault
      .save_view(
        &key_a,
        &make_view(ws_a, doc_a, "A Private", "page", None, "0000000010"),
      )
      .unwrap();
    vault
      .save_view(
        &key_b,
        &make_view(ws_b, doc_b, "B Private", "page", None, "0000000010"),
      )
      .unwrap();
    vault
      .save_view(
        &legacy_bare,
        &make_view(ws_a, doc_legacy, "Legacy Row", "page", None, "0000000010"),
      )
      .unwrap();

    // 1. Account A sees ONLY A's views/docs, never B's or quarantined legacy rows.
    let views_a = vault.list_views(&key_a);
    assert_eq!(views_a.len(), 1);
    assert_eq!(views_a[0].name, "A Private");
    assert!(vault.load_markdown(&key_a, doc_b).is_none());
    assert!(vault.load_markdown(&key_a, doc_legacy).is_none());

    // 2. Account B sees ONLY B's views/docs, never A's or quarantined legacy rows.
    let views_b = vault.list_views(&key_b);
    assert_eq!(views_b.len(), 1);
    assert_eq!(views_b[0].name, "B Private");
    assert!(vault.load_markdown(&key_b, doc_a).is_none());
    assert!(vault.load_markdown(&key_b, doc_legacy).is_none());

    // 3. `SyncEngine` created via `vault.create_sync_storage(&key_a, &doc_a)`
    //    seeds Account A's Flutter doc AND finds the pending offline outbox edit!
    let storage = vault.create_sync_storage(&key_a, &doc_a.to_string());
    let mut engine = SyncEngine::new(
      SyncConfig {
        client_id: Some(vault.client_id()),
        self_name: Some("Alice".into()),
        document_id: Some(doc_a.to_string()),
        restore_unacked: Vec::new(),
      },
      Some(storage),
    );
    let effects = engine.handle_event(SyncEvent::Connect);
    assert!(
      effects
        .iter()
        .any(|e| matches!(e, SyncEffect::Ready { .. }))
    );
    assert!(
      !engine.outbox_empty(),
      "Flutter 0.14.11 unpushed outbox edit must be queued for replay"
    );
    assert!(engine.all_blocks_json().contains("offline edit"));

    let _ = std::fs::remove_dir_all(&root);
  }
}
