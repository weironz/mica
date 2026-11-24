//! Cloud workspace, page-tree, and document synchronization helpers.
//!
//! Keeps blocking HTTP bootstrap/refresh orchestration and token-rotation
//! retry logic out of `lib.rs` so the UI shell stays concise and every
//! cross-await cloud round-trip returns a single atomic payload.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use mica_api_types::{View, Workspace};
use mica_client::PageTreeModel;
use nsh_core::prelude::*;
use uuid::Uuid;

use crate::doc_sync_session::{DocSyncController, ViewsWsWatcher};
use crate::editor_bridge::MicaDocumentBridge;
use crate::net::{CloudConfig, CloudConnector};
use crate::storage::{DEFAULT_ORIGIN, VaultStore};
use crate::workspace_views::{
  PendingInvitationEntry, WorkspaceMemberEntry, WorkspaceStorageUsage,
};
use crate::world_switcher::{self, LOCAL_ORIGIN};
use crate::{MODE_DOC, OpenTab, WorkspaceEntry};

/// Return `true` for any view that represents a document/page rather than a folder.
///
/// The Mica cloud server emits `object_type = "document"` while local vaults may
/// store `"page"` or `"document"`; checking `!= "folder"` matches both cleanly
/// (mirroring `mica_client::PageTreeModel`).
#[inline]
pub fn is_page_view(v: &View) -> bool {
  v.object_type != "folder"
}

/// Return `true` when `origin` points to a remote Mica cloud server rather than
/// the local SQLite vault (`"local"` or legacy `"local://..."`).
#[inline]
pub fn is_cloud_origin(origin: &str) -> bool {
  let trimmed = origin.trim();
  !trimmed.is_empty() && trimmed != DEFAULT_ORIGIN && !trimmed.starts_with("local://")
}

/// Normalize a user-supplied server address into an `http(s)://...` origin
/// without a trailing slash.
pub fn canonicalize_cloud_url(raw: &str) -> String {
  let trimmed = raw.trim().trim_end_matches('/');
  if trimmed.is_empty() {
    return String::new();
  }
  if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
    return trimmed.to_string();
  }
  let lower = trimmed.to_ascii_lowercase();
  let is_loopback = lower.starts_with("localhost")
    || lower.starts_with("127.0.0.1")
    || lower.starts_with("[::1]");
  if is_loopback {
    format!("http://{trimmed}")
  } else {
    format!("https://{trimmed}")
  }
}

/// Atomic bootstrap payload fetched right after cloud sign-in, world switch,
/// or startup session restore.
#[derive(Debug, Clone)]
pub struct CloudBootstrapPayload {
  pub account_id: String,
  pub workspaces: Vec<Workspace>,
  pub active_workspace_id: Option<Uuid>,
  pub views: Vec<View>,
  pub initial_doc: Option<(Uuid, String)>,
  pub refreshed_tokens: Option<(String, String)>,
}

/// Atomic single-workspace snapshot fetched when switching cloud workspaces or
/// triggering a manual sync.
#[derive(Debug, Clone)]
pub struct CloudWorkspaceSnapshot {
  pub workspace_id: Uuid,
  pub views: Vec<View>,
  pub initial_doc: Option<(Uuid, String)>,
  pub refreshed_tokens: Option<(String, String)>,
}

/// Execute async `op` against `conn`; if the server responds with HTTP 401 and
/// a refresh token (or Web `HttpOnly` cookie) is available, rotate once and retry.
pub async fn with_auto_refresh<T, F, Fut>(
  conn: &mut CloudConnector,
  op: F,
) -> Result<(T, Option<(String, String)>), String>
where
  F: Fn(CloudConnector) -> Fut,
  Fut: std::future::Future<Output = Result<T, String>>,
{
  match op(conn.clone()).await {
    Ok(val) => Ok((val, None)),
    Err(err)
      if CloudConnector::is_unauthorized_error(&err)
        && (conn.refresh_token().is_some() || cfg!(target_arch = "wasm32")) =>
    {
      conn.refresh_session().await?;
      let rotated = (
        conn.access_token().unwrap_or_default().to_string(),
        conn.refresh_token().unwrap_or_default().to_string(),
      );
      let val = op(conn.clone()).await?;
      Ok((val, Some(rotated)))
    }
    Err(err) => Err(err),
  }
}

fn merge_live_and_trash(mut live: Vec<View>, trash: Vec<View>, ws_id: Uuid) -> Vec<View> {
  let mut seen: HashSet<Uuid> = HashSet::with_capacity(live.len() + trash.len());
  for v in &mut live {
    if v.workspace_id.is_nil() {
      v.workspace_id = ws_id;
    }
    seen.insert(v.id);
  }
  for mut t in trash {
    if t.workspace_id.is_nil() {
      t.workspace_id = ws_id;
    }
    t.is_deleted = true;
    if seen.insert(t.id) {
      live.push(t);
    }
  }
  live
}

type ViewsAndOpeningDoc = (Vec<View>, Option<(Uuid, String)>);

async fn fetch_views_and_first_doc(
  conn: &CloudConnector,
  ws_id: Uuid,
  preferred_doc: Option<Uuid>,
) -> Result<ViewsAndOpeningDoc, String> {
  let live = conn.list_views(ws_id).await?;
  let trash = conn.list_trash(ws_id).await.unwrap_or_default();
  let merged = merge_live_and_trash(live, trash, ws_id);

  let target_doc_id = preferred_doc
    .filter(|&did| merged.iter().any(|v| is_page_view(v) && !v.is_deleted && v.object_id == did))
    .or_else(|| {
      merged
        .iter()
        .find(|v| is_page_view(v) && !v.is_deleted)
        .map(|v| v.object_id)
    });

  let initial_doc = if let Some(doc_id) = target_doc_id {
    let md = conn.fetch_document_markdown(ws_id, doc_id).await.unwrap_or_default();
    Some((doc_id, md))
  } else {
    None
  };

  Ok((merged, initial_doc))
}

/// Fetch all cloud workspaces, the selected workspace's view tree, and its
/// opening page's Markdown asynchronously (non-blocking on both Desktop & Web).
pub async fn fetch_cloud_bootstrap(
  conn: &mut CloudConnector,
  preferred_ws: Option<Uuid>,
) -> Result<CloudBootstrapPayload, String> {
  let mut refreshed_tokens: Option<(String, String)> = None;
  let account_id = if let Some(uid) = conn.user_id() {
    uid.to_string()
  } else {
    let (me, rot) = with_auto_refresh(conn, |c| async move { c.fetch_me().await }).await?;
    if rot.is_some() {
      refreshed_tokens = rot;
    }
    let uid = me
      .id
      .filter(|s| !s.trim().is_empty())
      .ok_or_else(|| "server /api/auth/me returned empty user id".to_string())?;
    *conn = conn.clone().with_user_id(Some(uid.clone()));
    uid
  };

  let (workspaces, rot_ws) =
    with_auto_refresh(conn, |c| async move { c.list_workspaces().await }).await?;
  if rot_ws.is_some() {
    refreshed_tokens = rot_ws;
  }
  let target_ws_id = preferred_ws
    .filter(|&id| workspaces.iter().any(|w| w.id == id))
    .or_else(|| workspaces.first().map(|w| w.id));

  let (views, initial_doc) = if let Some(ws_id) = target_ws_id {
    let ((v, d), rotated) =
      with_auto_refresh(conn, move |c| async move { fetch_views_and_first_doc(&c, ws_id, None).await }).await?;
    if rotated.is_some() {
      refreshed_tokens = rotated;
    }
    (v, d)
  } else {
    (Vec::new(), None)
  };

  Ok(CloudBootstrapPayload {
    account_id,
    workspaces,
    active_workspace_id: target_ws_id,
    views,
    initial_doc,
    refreshed_tokens,
  })
}

/// Refresh a single cloud workspace's view tree and active/opening page Markdown.
pub async fn fetch_cloud_workspace_snapshot(
  conn: &mut CloudConnector,
  ws_id: Uuid,
  preferred_doc: Option<Uuid>,
) -> Result<CloudWorkspaceSnapshot, String> {
  let ((views, initial_doc), refreshed_tokens) = with_auto_refresh(conn, move |c| async move {
    fetch_views_and_first_doc(&c, ws_id, preferred_doc).await
  })
  .await?;
  Ok(CloudWorkspaceSnapshot {
    workspace_id: ws_id,
    views,
    initial_doc,
    refreshed_tokens,
  })
}

/// Fetch the latest cloud Markdown for a single document (with transparent 401 refresh).
pub async fn fetch_cloud_document(
  conn: &mut CloudConnector,
  ws_id: Uuid,
  doc_id: Uuid,
) -> Result<(String, Option<(String, String)>), String> {
  with_auto_refresh(conn, move |c| async move {
    c.fetch_document_markdown(ws_id, doc_id).await
  })
  .await
}

/// Push edited Markdown back to the cloud server (`PATCH .../markdown`, `replace_all`).
///
/// The Mica API server rejects empty `replace_all` payloads to guard against
/// accidental document wipes, so empty buffers are skipped cleanly.
pub async fn push_cloud_document(
  conn: &mut CloudConnector,
  ws_id: Uuid,
  doc_id: Uuid,
  markdown: &str,
) -> Result<Option<(String, String)>, String> {
  if markdown.trim().is_empty() {
    return Ok(None);
  }
  let md_owned = markdown.to_string();
  let ((), rotated) = with_auto_refresh(conn, move |c| {
    let md = md_owned.clone();
    async move { c.save_document_markdown(ws_id, doc_id, &md).await }
  })
  .await?;
  Ok(rotated)
}

/// Persist rotated `(access_token, refresh_token)` back into the local vault.
pub fn persist_refreshed_tokens(
  vault: &VaultStore,
  origin: &str,
  email: &str,
  tokens: Option<(String, String)>,
) {
  if let Some((access, refresh)) = tokens
    && !access.trim().is_empty()
  {
    let _ = vault.save_server_credential_with_refresh(origin, email, &access, &refresh);
  }
}

/// Shared UI & storage handles driving cloud auth, workspace bootstrap,
/// tree refresh, and cloud CRUD mutations from `lib.rs`.
#[derive(Clone)]
pub(crate) struct CloudSessionHandles {
  pub vault: Rc<VaultStore>,
  pub tree: Rc<RefCell<PageTreeModel>>,
  pub docs_store: Rc<RefCell<HashMap<Uuid, String>>>,
  pub favorites: Rc<RefCell<Vec<Uuid>>>,
  pub bridge: Rc<MicaDocumentBridge>,
  pub cloud_connector: Rc<RefCell<CloudConnector>>,
  pub active_origin: Signal<String>,
  pub active_workspace_id: Signal<Uuid>,
  pub active_folder_id: Signal<Uuid>,
  pub active_folder_name: Signal<String>,
  pub workspace_name: Signal<String>,
  pub workspaces: Signal<Vec<WorkspaceEntry>>,
  pub active_doc: Signal<Option<Uuid>>,
  pub active_doc_id_str: Signal<String>,
  pub active_title: Signal<String>,
  pub active_is_favorite: Signal<bool>,
  pub open_tabs: Signal<Vec<OpenTab>>,
  pub active_tab_idx: Signal<usize>,
  pub active_mode: Signal<u8>,
  pub server_url: Signal<String>,
  pub known_servers_sig: Signal<Vec<String>>,
  pub signed_in_origins_sig: Signal<Vec<(String, String)>>,
  pub selected_cloud_origin: Signal<String>,
  pub world_active_origin: Signal<String>,
  pub world_switcher_open: Signal<bool>,
  pub world_can_close: Signal<bool>,
  pub allow_local: bool,
  pub world_busy: Signal<bool>,
  pub world_error: Signal<String>,
  pub world_info: Signal<String>,
  pub account_id: Signal<String>,
  pub account_name: Signal<String>,
  pub account_email: Signal<String>,
  pub is_cloud_signed_in: Signal<bool>,
  pub cloud_status: Signal<String>,
  pub auth_gen: Signal<u64>,
  pub cloud_load_gen: Signal<u64>,
  pub doc_fetch_gen: Signal<u64>,
  pub doc_save_gen: Signal<u64>,
  pub doc_sync: DocSyncController,
  pub views_watcher: ViewsWsWatcher,
  pub refresh_snapshots: Rc<dyn Fn(Uuid)>,
  pub refresh_favorite_pages: Rc<dyn Fn()>,
  pub sync_tree_snapshots: Rc<dyn Fn()>,
  pub open_doc_internal: Rc<dyn Fn(Uuid, String, bool, bool)>,
}

impl CloudSessionHandles {
  /// Fire a background cloud mutation with transparent 401 token refresh and
  /// ownership verification before reporting status or persisting rotated tokens.
  pub(crate) fn spawn_cloud_op<F, Fut>(&self, label: &'static str, op: F)
  where
    F: Fn(CloudConnector) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + 'static,
  {
    let orig = self.active_origin.get_untracked();
    if !is_cloud_origin(&orig) || !self.is_cloud_signed_in.get_untracked() {
      return;
    }
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = with_auto_refresh(&mut conn, op).await.map(|(_, r)| (conn, r));
      if h.active_origin.get_untracked() != orig {
        return;
      }
      match res {
        Ok((conn, rot)) if rot.is_some() => {
          *h.cloud_connector.borrow_mut() = conn;
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &srv_url, &email, rot);
          }
        }
        Ok(_) => {}
        Err(err) => h.cloud_status.set(format!("{label}失败：{err}")),
      }
    });
  }

  /// Apply a workspace's view tree and optional opening document to the UI shell
  /// (and optionally mirror them into the local SQLite vault).
  pub(crate) fn apply_workspace_views(
    &self,
    origin: String,
    ws_id: Uuid,
    ws_name: String,
    views: Vec<View>,
    initial_doc: Option<(Uuid, String)>,
    mirror_to_vault: bool,
  ) {
    self.bridge.flush_pending_persist();
    let is_cloud = is_cloud_origin(&origin);
    if mirror_to_vault {
      let _ = self.vault.replace_workspace_views(&origin, ws_id, &views);
      // For cloud origins, `SqliteSyncStorage` owns the authoritative Yrs CRDT
      // `doc_snapshot`; never overwrite it with a synthetic Markdown parse.
      if !is_cloud
        && let Some((did, ref md)) = initial_doc
      {
        let _ = self.vault.save_markdown(&origin, did, md);
      }
    }

    let mut next_docs = HashMap::new();
    for row in &views {
      if is_page_view(row) {
        let md = self
          .vault
          .load_markdown(&origin, row.object_id)
          .unwrap_or_default();
        next_docs.insert(row.object_id, md);
      }
    }
    if let Some((did, md)) = initial_doc.clone() {
      next_docs.insert(did, md);
    }

    let first_folder = views.iter().find(|r| r.object_type == "folder" && !r.is_deleted);
    let fid = first_folder.map(|r| r.id).unwrap_or(Uuid::nil());
    let fname = first_folder
      .map(|r| r.name.clone())
      .unwrap_or_else(|| ws_name.clone());

    let opening_page = initial_doc
      .as_ref()
      .and_then(|(did, _)| {
        views
          .iter()
          .find(|r| is_page_view(r) && !r.is_deleted && r.object_id == *did)
      })
      .or_else(|| views.iter().find(|r| is_page_view(r) && !r.is_deleted));

    let mut next_tree = PageTreeModel::new(views.clone());
    for folder in views
      .iter()
      .filter(|r| r.object_type == "folder" && !r.is_deleted && r.parent_view_id.is_none())
    {
      next_tree.set_expanded(folder.id, true);
    }
    if !fid.is_nil() {
      next_tree.set_expanded(fid, true);
    }
    *self.tree.borrow_mut() = next_tree;
    *self.docs_store.borrow_mut() = next_docs.clone();
    *self.favorites.borrow_mut() = self.vault.favorites_for(ws_id);

    self.active_workspace_id.set(ws_id);
    self.active_origin.set(origin.clone());
    self.workspace_name.set(ws_name);
    self.active_folder_id.set(fid);
    self.active_folder_name.set(fname);

    if let Some(page) = opening_page {
      let doc_id = page.object_id;
      let title = page.name.clone();
      let active_md = next_docs.get(&doc_id).cloned().unwrap_or_default();
      self.active_doc.set(Some(doc_id));
      self.active_doc_id_str.set(doc_id.to_string());
      self.active_title.set(title.clone());
      self
        .active_is_favorite
        .set(self.favorites.borrow().contains(&doc_id));
      (self.refresh_snapshots)(doc_id);
      self.bridge.load_markdown(&active_md);
      self.open_tabs.set(vec![OpenTab::single(doc_id, title)]);
      self.active_tab_idx.set(0);
      if is_cloud && self.is_cloud_signed_in.get_untracked() && !ws_id.is_nil() {
        let srv_url = self.server_url.get_untracked();
        self.doc_sync.open_cloud_doc(
          srv_url,
          origin.clone(),
          ws_id,
          doc_id,
          self.account_name.get_untracked(),
        );
      } else {
        self.doc_sync.close();
      }
    } else {
      self.doc_sync.close();
      self.active_doc.set(None);
      self.active_doc_id_str.set(String::new());
      self.active_title.set("暂无页面".to_string());
      self.active_is_favorite.set(false);
      self.bridge.load_markdown("");
      self.open_tabs.set(Vec::new());
      self.active_tab_idx.set(0);
    }

    if is_cloud && self.is_cloud_signed_in.get_untracked() && !ws_id.is_nil() {
      let srv_url = self.server_url.get_untracked();
      self.views_watcher.watch(srv_url, origin, ws_id);
    } else {
      self.views_watcher.stop();
    }

    (self.refresh_favorite_pages)();
    self.active_mode.set(MODE_DOC);
    (self.sync_tree_snapshots)();
  }

  /// Lightweight sidebar-only tree refresh triggered by `/ws/workspaces/{ws}/views`
  /// (`{"type":"views_changed"}`) — updates folders, pages, titles, and trash
  /// without disturbing the active editor CRDT document.
  pub(crate) fn refresh_views_only(&self, server_url: String, cache_origin: String, ws_id: Uuid) {
    if !is_cloud_origin(&cache_origin) || !self.is_cloud_signed_in.get_untracked() || ws_id.is_nil() {
      return;
    }
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = async {
        let ((live, trash), rot) = with_auto_refresh(&mut conn, move |c| async move {
          let live = c.list_views(ws_id).await?;
          let trash = c.list_trash(ws_id).await.unwrap_or_default();
          Ok((live, trash))
        })
        .await?;
        Ok::<_, String>((conn, merge_live_and_trash(live, trash, ws_id), rot))
      }
      .await;
      if h.active_origin.get_untracked() != cache_origin
        || h.active_workspace_id.get_untracked() != ws_id
        || !h.is_cloud_signed_in.get_untracked()
      {
        return;
      }
      if let Ok((conn, merged, rot)) = res {
        if rot.is_some() {
          *h.cloud_connector.borrow_mut() = conn;
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &server_url, &email, rot);
          }
        }
        let _ = h.vault.replace_workspace_views(&cache_origin, ws_id, &merged);
        let page_cnt = merged
          .iter()
          .filter(|v| is_page_view(v) && !v.is_deleted)
          .count();
        let mut all = h.workspaces.get_untracked();
        for w in &mut all {
          if w.id == ws_id {
            w.page_count = Some(page_cnt);
          }
        }
        h.workspaces.set(all);

        h.tree.borrow_mut().replace_views(merged.clone());

        if let Some(cur_doc) = h.active_doc.get_untracked()
          && let Some(row) = merged
            .iter()
            .find(|v| is_page_view(v) && !v.is_deleted && v.object_id == cur_doc)
          && h.active_title.get_untracked() != row.name
        {
          h.active_title.set(row.name.clone());
          let mut tabs = h.open_tabs.get_untracked();
          for tab in &mut tabs {
            if tab.doc_id == cur_doc {
              tab.title = row.name.clone();
            }
          }
          h.open_tabs.set(tabs);
        }
        (h.refresh_favorite_pages)();
        (h.sync_tree_snapshots)();
      }
    });
  }

  /// Refresh a single cloud workspace's tree and opening document asynchronously.
  pub(crate) fn refresh_workspace(
    &self,
    origin: String,
    ws_id: Uuid,
    ws_name: String,
    preferred_doc: Option<Uuid>,
  ) {
    if !is_cloud_origin(&origin) || !self.is_cloud_signed_in.get_untracked() {
      return;
    }
    let epoch = self.cloud_load_gen.get_untracked().wrapping_add(1);
    self.cloud_load_gen.set(epoch);
    self.cloud_status.set(format!("正在同步云端工作区：{ws_name}…"));
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = fetch_cloud_workspace_snapshot(&mut conn, ws_id, preferred_doc)
        .await
        .map(|s| (conn, s));
      if h.cloud_load_gen.get_untracked() != epoch
        || h.active_origin.get_untracked() != origin
        || h.active_workspace_id.get_untracked() != ws_id
      {
        return;
      }
      match res {
        Ok((conn, snap)) => {
          *h.cloud_connector.borrow_mut() = conn;
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &srv_url, &email, snap.refreshed_tokens);
          }
          let page_cnt = snap
            .views
            .iter()
            .filter(|v| is_page_view(v) && !v.is_deleted)
            .count();
          let mut all = h.workspaces.get_untracked();
          for w in &mut all {
            if w.id == ws_id {
              w.page_count = Some(page_cnt);
            }
          }
          h.workspaces.set(all);
          h.apply_workspace_views(origin, ws_id, ws_name.clone(), snap.views, snap.initial_doc, true);
          h.cloud_status.set(format!("已同步云端工作区：{ws_name}"));
        }
        Err(err) => {
          h.cloud_status.set(format!("同步云端工作区失败：{err}"));
        }
      }
    });
  }

  /// Apply a full cloud bootstrap payload (workspaces + first workspace tree +
  /// opening document) after login, world switch, or startup restore.
  pub(crate) fn apply_bootstrap(
    &self,
    raw_server_url: String,
    email: String,
    conn: CloudConnector,
    payload: CloudBootstrapPayload,
  ) {
    let server_url = mica_client::normalize_server_url(&raw_server_url)
      .unwrap_or_else(|_| canonicalize_cloud_url(&raw_server_url));
    let Ok(cache_origin) = mica_client::cloud_cache_origin(&server_url, &payload.account_id) else {
      self.cloud_status.set("云端账户标识无效，无法建立隔离缓存".to_string());
      self.world_error.set("云端账户标识无效，请重新登录".to_string());
      return;
    };

    let access_tok = conn.access_token().unwrap_or_default().to_string();
    let refresh_tok = conn.refresh_token().unwrap_or_default().to_string();
    *self.cloud_connector.borrow_mut() = conn;

    // Web (`!self.allow_local`) relies strictly on server `HttpOnly` cookies
    // and never stores tokens in `window.localStorage`.
    if self.allow_local {
      let _ = self.vault.save_server_credential_full(
        &server_url,
        &email,
        &payload.account_id,
        &access_tok,
        &refresh_tok,
      );
    }
    let _ = self.vault.set_active_origin(&server_url);

    let mut servers = self.known_servers_sig.get_untracked();
    if !servers.iter().any(|s| s == &server_url) {
      servers.push(server_url.clone());
      let _ = self.vault.set_known_servers(&servers);
      self.known_servers_sig.set(servers);
    }

    let mut creds = self.signed_in_origins_sig.get_untracked();
    creds.retain(|(o, _)| o != &server_url);
    creds.push((server_url.clone(), email.clone()));
    self.signed_in_origins_sig.set(creds);

    self.server_url.set(server_url.clone());
    self.selected_cloud_origin.set(server_url.clone());
    self.world_active_origin.set(server_url.clone());
    self.account_id.set(payload.account_id);
    self.account_email.set(email.clone());
    self.account_name.set(email);
    self.is_cloud_signed_in.set(true);
    self.world_can_close.set(true);
    self.world_switcher_open.set(false);

    let ws_pairs: Vec<(Uuid, String)> = payload
      .workspaces
      .iter()
      .map(|w| (w.id, w.name.clone()))
      .collect();
    let _ = self.vault.replace_origin_workspaces(&cache_origin, &ws_pairs);

    let ws_entries: Vec<WorkspaceEntry> = payload
      .workspaces
      .iter()
      .map(|w| WorkspaceEntry {
        id: w.id,
        name: w.name.clone(),
        origin: cache_origin.clone(),
        page_count: Some(w.page_count.max(0) as usize),
      })
      .collect();
    let ws_count = ws_entries.len();
    self.workspaces.set(ws_entries.clone());

    if let Some(ws_id) = payload.active_workspace_id {
      let ws_name = ws_entries
        .iter()
        .find(|w| w.id == ws_id)
        .map(|w| w.name.clone())
        .unwrap_or_else(|| "云端工作区".to_string());
      self.apply_workspace_views(
        cache_origin,
        ws_id,
        ws_name,
        payload.views,
        payload.initial_doc,
        true,
      );
      self.cloud_status.set(format!(
        "已连接云端：{}（{} 个工作区）",
        world_switcher::server_label(&server_url),
        ws_count
      ));
    } else {
      self.apply_workspace_views(
        cache_origin,
        Uuid::nil(),
        "暂无云端工作区".to_string(),
        Vec::new(),
        None,
        false,
      );
      self.cloud_status.set(format!(
        "已登录云端：{}（暂无工作区，可在左上角新建）",
        world_switcher::server_label(&server_url)
      ));
    }
  }

  /// Authenticate against `raw_url` and immediately load cloud workspaces, views,
  /// and the opening document.
  pub(crate) fn login(&self, raw_url: String, email: String, password: String) {
    let url = canonicalize_cloud_url(&raw_url);
    let auth_epoch = self.auth_gen.get_untracked().wrapping_add(1);
    let load_epoch = self.cloud_load_gen.get_untracked().wrapping_add(1);
    self.auth_gen.set(auth_epoch);
    self.cloud_load_gen.set(load_epoch);
    self.world_busy.set(true);
    self.world_error.set(String::new());
    self.world_info.set(String::new());
    self.cloud_status.set("正在登录并加载云端工作区…".to_string());

    let h = self.clone();
    let url_req = url.clone();
    let email_req = email.clone();
    spawn(async move {
      let outcome = async {
        let mut conn = CloudConnector::new(CloudConfig {
          server_url: url_req,
          access_token: None,
          email: Some(email_req.clone()),
        });
        let logged_email = conn.login(&email_req, &password).await?;
        let boot = fetch_cloud_bootstrap(&mut conn, None).await?;
        Ok::<_, String>((conn, logged_email, boot))
      }
      .await;
      if h.auth_gen.get_untracked() != auth_epoch {
        return;
      }
      match outcome {
        Ok((conn, logged_email, boot)) => h.apply_bootstrap(url, logged_email, conn, boot),
        Err(err) => {
          h.cloud_status.set(format!("登录失败：{err}"));
          h.world_error.set(err);
        }
      }
      h.world_busy.set(false);
    });
  }

  /// Enter a previously signed-in cloud origin (used by World Switcher click AND
  /// startup auto-restore). Clears stale/expired credentials cleanly on 401.
  pub(crate) fn enter_origin(&self, raw_origin: String, interactive: bool) {
    let origin = mica_client::normalize_server_url(&raw_origin)
      .unwrap_or_else(|_| canonicalize_cloud_url(&raw_origin));
    let Some((email, saved_account_id, token, refresh)) =
      self.vault.saved_server_credential_record(&origin)
    else {
      if interactive || !self.allow_local {
        if interactive {
          self.world_error.set("未找到该服务器的登录凭证，请重新登录".to_string());
        }
        if !self.allow_local {
          self.world_can_close.set(false);
        }
        self.world_switcher_open.set(true);
      }
      return;
    };
    if token.trim().is_empty() && refresh.trim().is_empty() {
      let _ = self.vault.clear_server_credential(&origin);
      let mut creds = self.signed_in_origins_sig.get_untracked();
      creds.retain(|(o, _)| o != &origin);
      self.signed_in_origins_sig.set(creds);
      if interactive || !self.allow_local {
        self.world_error.set("登录凭证已失效，请重新输入密码登录".to_string());
        if !self.allow_local {
          self.world_can_close.set(false);
        }
        self.world_switcher_open.set(true);
      }
      return;
    }

    let auth_epoch = self.auth_gen.get_untracked().wrapping_add(1);
    let load_epoch = self.cloud_load_gen.get_untracked().wrapping_add(1);
    self.auth_gen.set(auth_epoch);
    self.cloud_load_gen.set(load_epoch);
    if interactive {
      self.world_busy.set(true);
      self.world_error.set(String::new());
    }
    self.cloud_status.set(format!(
      "正在连接云端：{}…",
      world_switcher::server_label(&origin)
    ));

    // Only pre-hydrate cached workspaces when an authoritative `#111`
    // account-scoped cache key (`[normalized_server_url, account_id]`) is
    // available; pre-#111 bare-URL rows are never claimed or read.
    let scoped_cache = (!saved_account_id.trim().is_empty())
      .then(|| mica_client::cloud_cache_origin(&origin, &saved_account_id).ok())
      .flatten();
    let cached_ws = scoped_cache
      .as_deref()
      .map(|key| self.vault.list_workspaces(key))
      .unwrap_or_default();
    let preferred_ws = cached_ws.first().map(|(id, _)| *id);
    if let Some(ref cache_key) = scoped_cache
      && !cached_ws.is_empty()
      && !interactive
    {
      self.server_url.set(origin.clone());
      self.selected_cloud_origin.set(origin.clone());
      self.world_active_origin.set(origin.clone());
      self.account_id.set(saved_account_id.clone());
      self.account_email.set(email.clone());
      self.account_name.set(email.clone());
      self.is_cloud_signed_in.set(true);
      let entries: Vec<WorkspaceEntry> = cached_ws
        .iter()
        .map(|(id, name)| WorkspaceEntry {
          id: *id,
          name: name.clone(),
          origin: cache_key.clone(),
          page_count: None,
        })
        .collect();
      self.workspaces.set(entries);
      if let Some((first_id, first_name)) = cached_ws.first() {
        let cached_views = self.vault.list_workspace_views(cache_key, *first_id);
        self.apply_workspace_views(
          cache_key.clone(),
          *first_id,
          first_name.clone(),
          cached_views,
          None,
          false,
        );
      }
    }

    let h = self.clone();
    let origin_req = origin.clone();
    let email_req = email.clone();
    let uid_req = (!saved_account_id.trim().is_empty()).then_some(saved_account_id);
    spawn(async move {
      let outcome = async {
        let mut conn = CloudConnector::new(CloudConfig {
          server_url: origin_req,
          access_token: (!token.trim().is_empty()).then_some(token),
          email: Some(email_req),
        })
        .with_user_id(uid_req)
        .with_refresh_token((!refresh.trim().is_empty()).then_some(refresh));
        if conn.access_token().is_none() {
          conn.refresh_session().await?;
        }
        let boot = fetch_cloud_bootstrap(&mut conn, preferred_ws).await?;
        Ok::<_, String>((conn, boot))
      }
      .await;
      if h.auth_gen.get_untracked() != auth_epoch {
        return;
      }
      match outcome {
        Ok((conn, boot)) => h.apply_bootstrap(origin, email, conn, boot),
        Err(err) => {
          if CloudConnector::is_unauthorized_error(&err) {
            let _ = h.vault.clear_server_credential(&origin);
            let mut creds = h.signed_in_origins_sig.get_untracked();
            creds.retain(|(o, _)| o != &origin);
            h.signed_in_origins_sig.set(creds);
            h.is_cloud_signed_in.set(false);
            h.world_error.set("登录已过期，请重新登录".to_string());
            h.cloud_status.set("云端登录已过期，请重新登录".to_string());
            if !h.allow_local {
              h.world_can_close.set(false);
              h.world_switcher_open.set(true);
            } else if interactive {
              h.world_switcher_open.set(true);
            }
          } else {
            h.cloud_status.set(format!("连接云端失败：{err}"));
            if interactive || (!h.allow_local && !h.is_cloud_signed_in.get_untracked()) {
              h.world_error.set(err);
              if !h.allow_local {
                h.world_can_close.set(false);
                h.world_switcher_open.set(true);
              }
            }
          }
        }
      }
      h.world_busy.set(false);
    });
  }

  /// Register a new cloud account; auto-login if immediately verified, otherwise
  /// prompt the user to check their verification email.
  pub(crate) fn register(
    &self,
    raw_url: String,
    email: String,
    display_name: String,
    password: String,
  ) {
    let url = canonicalize_cloud_url(&raw_url);
    let auth_epoch = self.auth_gen.get_untracked().wrapping_add(1);
    self.auth_gen.set(auth_epoch);
    self.world_busy.set(true);
    self.world_error.set(String::new());
    self.world_info.set(String::new());

    let h = self.clone();
    let url_req = url.clone();
    let email_req = email.clone();
    let pass_req = password.clone();
    spawn(async move {
      let outcome = async {
        let mut conn = CloudConnector::new(CloudConfig {
          server_url: url_req,
          access_token: None,
          email: Some(email_req.clone()),
        });
        let user = conn.register(&email_req, &display_name, &pass_req).await?;
        if user.verified {
          let logged_email = conn.login(&email_req, &pass_req).await?;
          let boot = fetch_cloud_bootstrap(&mut conn, None).await?;
          Ok::<_, String>(Some((conn, logged_email, boot)))
        } else {
          Ok(None)
        }
      }
      .await;
      if h.auth_gen.get_untracked() != auth_epoch {
        return;
      }
      match outcome {
        Ok(Some((conn, logged_email, boot))) => h.apply_bootstrap(url, logged_email, conn, boot),
        Ok(None) => {
          h.world_info
            .set("注册成功，请查收验证邮件完成激活后再登录".to_string());
          h.cloud_status.set("已发送注册验证邮件".to_string());
        }
        Err(err) => h.world_error.set(err),
      }
      h.world_busy.set(false);
    });
  }

  /// Leave the cloud world: restore local SQLite workspaces on Desktop, or
  /// clear cloud session & return to the full-screen Cloud Sign-In gate on Web.
  pub(crate) fn logout_to_local(&self) {
    self.doc_sync.close();
    self.views_watcher.stop();
    self.auth_gen.set(self.auth_gen.get_untracked().wrapping_add(1));
    self
      .cloud_load_gen
      .set(self.cloud_load_gen.get_untracked().wrapping_add(1));
    self
      .doc_fetch_gen
      .set(self.doc_fetch_gen.get_untracked().wrapping_add(1));
    self
      .doc_save_gen
      .set(self.doc_save_gen.get_untracked().wrapping_add(1));
    self.world_busy.set(false);

    if !self.allow_local {
      let logout_conn = self.cloud_connector.borrow().clone();
      let auth_epoch = self.auth_gen.get_untracked();
      let h = self.clone();
      self.world_busy.set(true);
      spawn(async move {
        // Await non-blocking `POST /api/auth/logout` so the server revokes the
        // refresh family and clears HttpOnly cookies BEFORE the sign-in gate
        // becomes interactive (immune to immediate reload).
        let _ = logout_conn.logout_server().await;
        if h.auth_gen.get_untracked() != auth_epoch {
          return;
        }
        let web_origin = {
          let cur = h.selected_cloud_origin.get_untracked();
          if cur.is_empty() { h.server_url.get_untracked() } else { cur }
        };
        let prev_server = h.server_url.get_untracked();
        if is_cloud_origin(&prev_server) {
          let _ = h.vault.clear_server_credential(&prev_server);
        }
        if is_cloud_origin(&web_origin) {
          let _ = h.vault.clear_server_credential(&web_origin);
        }
        let mut creds = h.signed_in_origins_sig.get_untracked();
        creds.retain(|(o, _)| o != &prev_server && o != &web_origin);
        h.signed_in_origins_sig.set(creds);
        *h.cloud_connector.borrow_mut() = CloudConnector::new(CloudConfig {
          server_url: web_origin.clone(),
          access_token: None,
          email: None,
        });
        h.is_cloud_signed_in.set(false);
        h.world_active_origin.set(web_origin.clone());
        h.account_id.set(String::new());
        h.account_email.set(String::new());
        h.account_name.set("未登录".to_string());
        h.cloud_status.set("请登录云端账户".to_string());
        h.world_error.set(String::new());
        h.world_info.set(String::new());
        h.workspaces.set(Vec::new());
        h.apply_workspace_views(web_origin, Uuid::nil(), String::new(), Vec::new(), None, false);
        h.world_busy.set(false);
        h.world_can_close.set(false);
        h.world_switcher_open.set(true);
      });
      return;
    }

    let _ = self.vault.set_active_origin(DEFAULT_ORIGIN);
    *self.cloud_connector.borrow_mut() = CloudConnector::new(CloudConfig {
      server_url: self.server_url.get_untracked(),
      access_token: None,
      email: None,
    });
    self.is_cloud_signed_in.set(false);
    self.world_can_close.set(true);
    self.world_active_origin.set(LOCAL_ORIGIN.to_string());
    self.account_id.set(String::new());
    self.account_email.set("这台设备".to_string());
    self.account_name.set("本地工作区".to_string());
    self.cloud_status.set("已切换回本地离线 SQLite Vault".to_string());

    let local_all = self.vault.list_views(DEFAULT_ORIGIN);
    let mut saved_ws = self.vault.list_workspaces(DEFAULT_ORIGIN);
    if saved_ws.is_empty() {
      let fallback_id = local_all
        .first()
        .map(|v| v.workspace_id)
        .unwrap_or_else(Uuid::new_v4);
      saved_ws.push((fallback_id, "Mica 本地工作区".to_string()));
    }
    let entries: Vec<WorkspaceEntry> = saved_ws
      .into_iter()
      .map(|(id, name)| WorkspaceEntry {
        id,
        name,
        origin: DEFAULT_ORIGIN.to_string(),
        page_count: None,
      })
      .collect();
    let first = entries[0].clone();
    self.workspaces.set(entries);
    let ws_views = self.vault.list_workspace_views(DEFAULT_ORIGIN, first.id);
    self.apply_workspace_views(
      DEFAULT_ORIGIN.to_string(),
      first.id,
      first.name,
      ws_views,
      None,
      false,
    );
  }

  /// Create a cloud page via `POST /api/workspaces/{ws}/documents` so its `id`
  /// and `object_id` match the server authoritatively.
  pub(crate) fn create_cloud_page(
    &self,
    origin: String,
    ws_id: Uuid,
    parent: Option<Uuid>,
    name: String,
    markdown: String,
    in_new_tab: bool,
  ) {
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    let name_req = name.clone();
    let md_req = markdown.clone();
    spawn(async move {
      let res = async {
        let name_c = name_req.clone();
        let (mut view, mut rot) = with_auto_refresh(&mut conn, move |c| {
          let n = name_c.clone();
          async move { c.create_document(ws_id, &n, parent).await }
        })
        .await?;
        if view.workspace_id.is_nil() {
          view.workspace_id = ws_id;
        }
        if !md_req.trim().is_empty() {
          let obj_id = view.object_id;
          let md_c = md_req.clone();
          let (_, rot2) = with_auto_refresh(&mut conn, move |c| {
            let m = md_c.clone();
            async move { c.save_document_markdown(ws_id, obj_id, &m).await }
          })
          .await?;
          if rot2.is_some() {
            rot = rot2;
          }
        }
        Ok::<_, String>((conn, view, rot))
      }
      .await;
      if h.active_origin.get_untracked() != origin
        || h.active_workspace_id.get_untracked() != ws_id
      {
        return;
      }
      match res {
        Ok((conn, view, rot)) => {
          *h.cloud_connector.borrow_mut() = conn;
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &srv_url, &email, rot);
          }
          let doc_id = view.object_id;
          let title = view.name.clone();
          let _ = h.vault.save_view(&origin, &view);
          {
            let mut t = h.tree.borrow_mut();
            t.upsert_view(view);
            if let Some(pid) = parent {
              t.set_expanded(pid, true);
            }
          }
          h.docs_store.borrow_mut().insert(doc_id, markdown);
          (h.sync_tree_snapshots)();
          (h.open_doc_internal)(doc_id, title, true, in_new_tab);
        }
        Err(err) => h.cloud_status.set(format!("云端新建页面失败：{err}")),
      }
    });
  }

  /// Create a cloud folder via `POST /api/workspaces/{ws}/folders`.
  pub(crate) fn create_cloud_folder(
    &self,
    origin: String,
    ws_id: Uuid,
    parent: Option<Uuid>,
    name: String,
  ) {
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = async {
        let name_c = name.clone();
        let (mut view, rot) = with_auto_refresh(&mut conn, move |c| {
          let n = name_c.clone();
          async move { c.create_folder(ws_id, &n, parent).await }
        })
        .await?;
        if view.workspace_id.is_nil() {
          view.workspace_id = ws_id;
        }
        Ok::<_, String>((conn, view, rot))
      }
      .await;
      if h.active_origin.get_untracked() != origin
        || h.active_workspace_id.get_untracked() != ws_id
      {
        return;
      }
      match res {
        Ok((conn, view, rot)) => {
          *h.cloud_connector.borrow_mut() = conn;
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &srv_url, &email, rot);
          }
          let _ = h.vault.save_view(&origin, &view);
          {
            let mut t = h.tree.borrow_mut();
            let fid = view.id;
            t.upsert_view(view);
            t.set_expanded(fid, true);
            if let Some(pid) = parent {
              t.set_expanded(pid, true);
            }
          }
          (h.sync_tree_snapshots)();
        }
        Err(err) => h.cloud_status.set(format!("云端新建文件夹失败：{err}")),
      }
    });
  }

  /// Create a cloud workspace via `POST /api/workspaces` and switch into it.
  pub(crate) fn create_cloud_workspace(&self, origin: String, name: String) {
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = with_auto_refresh(&mut conn, move |c| {
        let n = name.clone();
        async move { c.create_workspace(&n).await }
      })
      .await
      .map(|(w, r)| (conn, w, r));
      if h.active_origin.get_untracked() != origin {
        return;
      }
      match res {
        Ok((conn, ws, rot)) => {
          *h.cloud_connector.borrow_mut() = conn;
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &srv_url, &email, rot);
          }
          let _ = h.vault.save_workspace_name(&origin, ws.id, &ws.name);
          let entry = WorkspaceEntry {
            id: ws.id,
            name: ws.name.clone(),
            origin: origin.clone(),
            page_count: Some(0),
          };
          let mut all = h.workspaces.get_untracked();
          all.push(entry);
          h.workspaces.set(all);
          h.apply_workspace_views(origin.clone(), ws.id, ws.name.clone(), Vec::new(), None, false);
          h.refresh_workspace(origin, ws.id, ws.name, None);
        }
        Err(err) => h.cloud_status.set(format!("云端新建工作区失败：{err}")),
      }
    });
  }

  /// Import a batch of parsed pages (and any nested folder paths) directly into
  /// the active cloud workspace via the server API, then refresh the tree.
  pub(crate) fn import_cloud_pages(
    &self,
    origin: String,
    ws_id: Uuid,
    base_parent: Option<Uuid>,
    specs: Vec<crate::transfer::ImportedPageSpec>,
  ) {
    if specs.is_empty() || !is_cloud_origin(&origin) || !self.is_cloud_signed_in.get_untracked() || ws_id.is_nil() {
      return;
    }
    let total = specs.len();
    self.cloud_status.set(format!("正在导入 {total} 篇页面到云端…"));
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = async {
        let mut folder_cache: HashMap<(Option<Uuid>, String), Uuid> = HashMap::new();
        let mut created_views: Vec<View> = Vec::new();
        let mut created_docs: Vec<(Uuid, String, String)> = Vec::new();
        let mut rot: Option<(String, String)> = None;

        for spec in specs {
          let mut parent = base_parent;
          for seg in spec.folder_path {
            let seg_trim = seg.trim().to_string();
            if seg_trim.is_empty() {
              continue;
            }
            let key = (parent, seg_trim.clone());
            let fid = if let Some(&existing) = folder_cache.get(&key) {
              existing
            } else {
              let seg_c = seg_trim.clone();
              let (mut fview, r) = with_auto_refresh(&mut conn, move |c| {
                let s = seg_c.clone();
                async move { c.create_folder(ws_id, &s, parent).await }
              })
              .await?;
              if r.is_some() {
                rot = r;
              }
              if fview.workspace_id.is_nil() {
                fview.workspace_id = ws_id;
              }
              let fid = fview.id;
              created_views.push(fview);
              folder_cache.insert(key, fid);
              fid
            };
            parent = Some(fid);
          }

          let title_c = spec.title.clone();
          let (mut pview, r1) = with_auto_refresh(&mut conn, move |c| {
            let t = title_c.clone();
            async move { c.create_document(ws_id, &t, parent).await }
          })
          .await?;
          if r1.is_some() {
            rot = r1;
          }
          if pview.workspace_id.is_nil() {
            pview.workspace_id = ws_id;
          }
          if !spec.markdown.trim().is_empty() {
            let obj_id = pview.object_id;
            let md_c = spec.markdown.clone();
            let (_, r2) = with_auto_refresh(&mut conn, move |c| {
              let m = md_c.clone();
              async move { c.save_document_markdown(ws_id, obj_id, &m).await }
            })
            .await?;
            if r2.is_some() {
              rot = r2;
            }
          }
          created_docs.push((pview.object_id, pview.name.clone(), spec.markdown));
          created_views.push(pview);
        }
        Ok::<_, String>((conn, created_views, created_docs, rot))
      }
      .await;
      if h.active_origin.get_untracked() != origin
        || h.active_workspace_id.get_untracked() != ws_id
      {
        return;
      }
      match res {
        Ok((conn, created_views, created_docs, rot)) => {
          *h.cloud_connector.borrow_mut() = conn;
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &srv_url, &email, rot);
          }
          let count = created_docs.len();
          {
            let mut t = h.tree.borrow_mut();
            for v in created_views {
              let is_folder = v.object_type == "folder";
              let vid = v.id;
              let pid = v.parent_view_id;
              let _ = h.vault.save_view(&origin, &v);
              t.upsert_view(v);
              if is_folder {
                t.set_expanded(vid, true);
              }
              if let Some(p) = pid {
                t.set_expanded(p, true);
              }
            }
          }
          let mut last_doc = None;
          {
            let mut ds = h.docs_store.borrow_mut();
            for (did, title, md) in created_docs {
              ds.insert(did, md);
              last_doc = Some((did, title));
            }
          }
          (h.sync_tree_snapshots)();
          if let Some((did, title)) = last_doc {
            (h.open_doc_internal)(did, title, true, false);
          }
          h.cloud_status.set(format!("已导入 {count} 篇页面到云端"));
          h.bridge.set_status_flash(format!("已导入 {count} 篇页面"));
        }
        Err(err) => {
          h.cloud_status.set(format!("云端导入失败：{err}"));
          h.bridge.set_status_flash(format!("导入失败：{err}"));
        }
      }
    });
  }

  /// Restore a Web (`wasm32`) cloud session silently via the server's `HttpOnly`
  /// `mica_refresh` cookie (`POST /api/auth/refresh` with `{}`).
  fn restore_web_cookie_session(&self, raw_origin: String) {
    let origin = canonicalize_cloud_url(&raw_origin);
    if !is_cloud_origin(&origin) {
      self.world_can_close.set(false);
      self.world_switcher_open.set(true);
      return;
    }
    let auth_epoch = self.auth_gen.get_untracked().wrapping_add(1);
    let load_epoch = self.cloud_load_gen.get_untracked().wrapping_add(1);
    self.auth_gen.set(auth_epoch);
    self.cloud_load_gen.set(load_epoch);
    self.world_busy.set(true);
    self.world_error.set(String::new());
    self.cloud_status.set("正在恢复云端会话…".to_string());

    let h = self.clone();
    let origin_req = origin.clone();
    spawn(async move {
      let outcome = async {
        let mut conn = CloudConnector::new(CloudConfig {
          server_url: origin_req,
          access_token: None,
          email: None,
        });
        let email = conn.refresh_session_cookie().await?;
        let boot = fetch_cloud_bootstrap(&mut conn, None).await?;
        Ok::<_, String>((conn, email, boot))
      }
      .await;
      if h.auth_gen.get_untracked() != auth_epoch {
        return;
      }
      match outcome {
        Ok((conn, email, boot)) => h.apply_bootstrap(origin, email, conn, boot),
        Err(_) => {
          h.is_cloud_signed_in.set(false);
          h.cloud_status.set("请登录云端账户".to_string());
          h.world_error.set(String::new());
          h.world_can_close.set(false);
          h.world_switcher_open.set(true);
        }
      }
      h.world_busy.set(false);
    });
  }

  /// Automatically restore a valid cloud session on startup (and keep the
  /// full-screen Cloud Sign-In gate open on Web when unauthenticated).
  pub(crate) fn restore_startup_session(&self) {
    if !self.allow_local {
      let web_origin = self.selected_cloud_origin.get_untracked();
      self.restore_web_cookie_session(web_origin);
      return;
    }

    let saved_origin = self.vault.active_origin();
    if is_cloud_origin(&saved_origin)
      && self
        .vault
        .saved_server_credential_full(&saved_origin)
        .is_some_and(|(_, tok, rt)| !tok.trim().is_empty() || !rt.trim().is_empty())
    {
      self.enter_origin(saved_origin, false);
    }
  }

  /// Fetch cloud workspace storage usage, members, and pending invitations into
  /// the Workspace Settings modal signals (guarded by `gen` + origin/ws ownership).
  pub(crate) fn load_workspace_settings(
    &self,
    gen_sig: Signal<u64>,
    storage_usage: Signal<Option<WorkspaceStorageUsage>>,
    members_sig: Signal<Vec<WorkspaceMemberEntry>>,
    invitations_sig: Signal<Vec<PendingInvitationEntry>>,
  ) {
    let orig = self.active_origin.get_untracked();
    let ws_id = self.active_workspace_id.get_untracked();
    if !is_cloud_origin(&orig) || !self.is_cloud_signed_in.get_untracked() || ws_id.is_nil() {
      storage_usage.set(None);
      members_sig.set(Vec::new());
      invitations_sig.set(Vec::new());
      return;
    }
    let epoch = gen_sig.get_untracked().wrapping_add(1);
    gen_sig.set(epoch);
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = async {
        let (usage, mut rot) =
          match with_auto_refresh(&mut conn, move |c| async move { c.workspace_usage(ws_id).await })
            .await
          {
            Ok((u, r)) => (Some(u), r),
            Err(_) => (None, None),
          };
        let members =
          match with_auto_refresh(&mut conn, move |c| async move { c.list_members(ws_id).await })
            .await
          {
            Ok((m, r)) => {
              if r.is_some() {
                rot = r;
              }
              m
            }
            Err(_) => Vec::new(),
          };
        let invs = match with_auto_refresh(&mut conn, move |c| async move {
          c.list_invitations(ws_id).await
        })
        .await
        {
          Ok((i, r)) => {
            if r.is_some() {
              rot = r;
            }
            i
          }
          Err(_) => Vec::new(),
        };
        (conn, usage, members, invs, rot)
      }
      .await;
      if gen_sig.get_untracked() != epoch
        || h.active_origin.get_untracked() != orig
        || h.active_workspace_id.get_untracked() != ws_id
      {
        return;
      }
      let (conn, usage, members, invs, rot) = res;
      if rot.is_some() {
        *h.cloud_connector.borrow_mut() = conn;
        if h.allow_local {
          persist_refreshed_tokens(&h.vault, &srv_url, &email, rot);
        }
      }
      storage_usage.set(usage.map(|(used_bytes, quota_bytes)| WorkspaceStorageUsage {
        used_bytes,
        quota_bytes,
      }));
      members_sig.set(
        members
          .into_iter()
          .map(|m| WorkspaceMemberEntry {
            user_id: m.user_id,
            display_name: m.display_name,
            email: m.email,
            role: m.role,
          })
          .collect(),
      );
      invitations_sig.set(
        invs
          .into_iter()
          .map(|i| PendingInvitationEntry {
            id: i.id,
            email: i.email.unwrap_or_default(),
            role: i.role,
          })
          .collect(),
      );
    });
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn canonicalize_cloud_url_adds_scheme_and_strips_slash() {
    assert_eq!(
      canonicalize_cloud_url("mica.cloudcele.com/"),
      "https://mica.cloudcele.com"
    );
    assert_eq!(
      canonicalize_cloud_url("https://mica.cloudcele.com///"),
      "https://mica.cloudcele.com"
    );
    assert_eq!(
      canonicalize_cloud_url("127.0.0.1:8080/"),
      "http://127.0.0.1:8080"
    );
    assert_eq!(
      canonicalize_cloud_url("localhost:3000"),
      "http://localhost:3000"
    );
  }

  #[test]
  fn origin_classification_separates_local_and_cloud() {
    assert!(!is_cloud_origin("local"));
    assert!(!is_cloud_origin("local://1234"));
    assert!(!is_cloud_origin(""));
    assert!(is_cloud_origin("https://mica.cloudcele.com"));
    assert!(is_cloud_origin("http://127.0.0.1:8080"));
  }
}
