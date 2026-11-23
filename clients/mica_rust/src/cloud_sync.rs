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

use crate::editor_bridge::MicaDocumentBridge;
use crate::net::{self, CloudConfig, CloudConnector};
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

/// Execute `op` against `conn`; if the server responds with HTTP 401 and a
/// refresh token is available, rotate the session once and retry.
pub fn with_auto_refresh<T>(
  conn: &mut CloudConnector,
  op: impl Fn(&CloudConnector) -> Result<T, String>,
) -> Result<(T, Option<(String, String)>), String> {
  match op(conn) {
    Ok(val) => Ok((val, None)),
    Err(err) if CloudConnector::is_unauthorized_error(&err) && conn.refresh_token().is_some() => {
      conn.refresh_session()?;
      let rotated = (
        conn.access_token().unwrap_or_default().to_string(),
        conn.refresh_token().unwrap_or_default().to_string(),
      );
      let val = op(conn)?;
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

fn fetch_views_and_first_doc(
  conn: &CloudConnector,
  ws_id: Uuid,
  preferred_doc: Option<Uuid>,
) -> Result<ViewsAndOpeningDoc, String> {
  let live = conn.list_views(ws_id)?;
  let trash = conn.list_trash(ws_id).unwrap_or_default();
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
    let md = conn.fetch_document_markdown(ws_id, doc_id).unwrap_or_default();
    Some((doc_id, md))
  } else {
    None
  };

  Ok((merged, initial_doc))
}

/// Fetch all cloud workspaces, the selected workspace's view tree, and its
/// opening page's Markdown in a single blocking worker invocation.
pub fn fetch_cloud_bootstrap(
  conn: &mut CloudConnector,
  preferred_ws: Option<Uuid>,
) -> Result<CloudBootstrapPayload, String> {
  let (workspaces, mut refreshed_tokens) = with_auto_refresh(conn, |c| c.list_workspaces())?;
  let target_ws_id = preferred_ws
    .filter(|&id| workspaces.iter().any(|w| w.id == id))
    .or_else(|| workspaces.first().map(|w| w.id));

  let (views, initial_doc) = if let Some(ws_id) = target_ws_id {
    let ((v, d), rotated) = with_auto_refresh(conn, |c| fetch_views_and_first_doc(c, ws_id, None))?;
    if rotated.is_some() {
      refreshed_tokens = rotated;
    }
    (v, d)
  } else {
    (Vec::new(), None)
  };

  Ok(CloudBootstrapPayload {
    workspaces,
    active_workspace_id: target_ws_id,
    views,
    initial_doc,
    refreshed_tokens,
  })
}

/// Refresh a single cloud workspace's view tree and active/opening page Markdown.
pub fn fetch_cloud_workspace_snapshot(
  conn: &mut CloudConnector,
  ws_id: Uuid,
  preferred_doc: Option<Uuid>,
) -> Result<CloudWorkspaceSnapshot, String> {
  let ((views, initial_doc), refreshed_tokens) =
    with_auto_refresh(conn, |c| fetch_views_and_first_doc(c, ws_id, preferred_doc))?;
  Ok(CloudWorkspaceSnapshot {
    workspace_id: ws_id,
    views,
    initial_doc,
    refreshed_tokens,
  })
}

/// Fetch the latest cloud Markdown for a single document (with transparent 401 refresh).
pub fn fetch_cloud_document(
  conn: &mut CloudConnector,
  ws_id: Uuid,
  doc_id: Uuid,
) -> Result<(String, Option<(String, String)>), String> {
  with_auto_refresh(conn, |c| c.fetch_document_markdown(ws_id, doc_id))
}

/// Push edited Markdown back to the cloud server (`PATCH .../markdown`, `replace_all`).
///
/// The Mica API server rejects empty `replace_all` payloads to guard against
/// accidental document wipes, so empty buffers are skipped cleanly.
pub fn push_cloud_document(
  conn: &mut CloudConnector,
  ws_id: Uuid,
  doc_id: Uuid,
  markdown: &str,
) -> Result<Option<(String, String)>, String> {
  if markdown.trim().is_empty() {
    return Ok(None);
  }
  let ((), rotated) = with_auto_refresh(conn, |c| c.save_document_markdown(ws_id, doc_id, markdown))?;
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
  pub world_busy: Signal<bool>,
  pub world_error: Signal<String>,
  pub world_info: Signal<String>,
  pub account_name: Signal<String>,
  pub account_email: Signal<String>,
  pub is_cloud_signed_in: Signal<bool>,
  pub cloud_status: Signal<String>,
  pub auth_gen: Signal<u64>,
  pub cloud_load_gen: Signal<u64>,
  pub doc_fetch_gen: Signal<u64>,
  pub doc_save_gen: Signal<u64>,
  pub refresh_snapshots: Rc<dyn Fn(Uuid)>,
  pub refresh_favorite_pages: Rc<dyn Fn()>,
  pub sync_tree_snapshots: Rc<dyn Fn()>,
  pub open_doc_internal: Rc<dyn Fn(Uuid, String, bool, bool)>,
}

impl CloudSessionHandles {
  /// Fire a background cloud mutation with transparent 401 token refresh and
  /// ownership verification before reporting status or persisting rotated tokens.
  pub(crate) fn spawn_cloud_op<F>(&self, label: &'static str, op: F)
  where
    F: Fn(&CloudConnector) -> Result<(), String> + Send + 'static,
  {
    let orig = self.active_origin.get_untracked();
    if !is_cloud_origin(&orig) || !self.is_cloud_signed_in.get_untracked() {
      return;
    }
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = net::run_blocking(move || with_auto_refresh(&mut conn, op).map(|(_, r)| (conn, r)))
        .await;
      if h.active_origin.get_untracked() != orig {
        return;
      }
      match res {
        Ok((conn, rot)) if rot.is_some() => {
          *h.cloud_connector.borrow_mut() = conn;
          persist_refreshed_tokens(&h.vault, &orig, &email, rot);
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
    if mirror_to_vault {
      let _ = self.vault.replace_workspace_views(&origin, ws_id, &views);
      if let Some((did, ref md)) = initial_doc {
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
    self.active_origin.set(origin);
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
    } else {
      self.active_doc.set(None);
      self.active_doc_id_str.set(String::new());
      self.active_title.set("暂无页面".to_string());
      self.active_is_favorite.set(false);
      self.bridge.load_markdown("");
      self.open_tabs.set(Vec::new());
      self.active_tab_idx.set(0);
    }

    (self.refresh_favorite_pages)();
    self.active_mode.set(MODE_DOC);
    (self.sync_tree_snapshots)();
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
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = net::run_blocking(move || {
        fetch_cloud_workspace_snapshot(&mut conn, ws_id, preferred_doc).map(|s| (conn, s))
      })
      .await;
      if h.cloud_load_gen.get_untracked() != epoch
        || h.active_origin.get_untracked() != origin
        || h.active_workspace_id.get_untracked() != ws_id
      {
        return;
      }
      match res {
        Ok((conn, snap)) => {
          *h.cloud_connector.borrow_mut() = conn;
          persist_refreshed_tokens(&h.vault, &origin, &email, snap.refreshed_tokens);
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
    origin: String,
    email: String,
    conn: CloudConnector,
    payload: CloudBootstrapPayload,
  ) {
    let access_tok = conn.access_token().unwrap_or_default().to_string();
    let refresh_tok = conn.refresh_token().unwrap_or_default().to_string();
    *self.cloud_connector.borrow_mut() = conn;

    let _ = self
      .vault
      .save_server_credential_with_refresh(&origin, &email, &access_tok, &refresh_tok);
    let _ = self.vault.set_active_origin(&origin);

    let mut servers = self.known_servers_sig.get_untracked();
    if !servers.iter().any(|s| s == &origin) {
      servers.push(origin.clone());
      let _ = self.vault.set_known_servers(&servers);
      self.known_servers_sig.set(servers);
    }

    let mut creds = self.signed_in_origins_sig.get_untracked();
    creds.retain(|(o, _)| o != &origin);
    creds.push((origin.clone(), email.clone()));
    self.signed_in_origins_sig.set(creds);

    self.server_url.set(origin.clone());
    self.selected_cloud_origin.set(origin.clone());
    self.world_active_origin.set(origin.clone());
    self.account_email.set(email.clone());
    self.account_name.set(email);
    self.is_cloud_signed_in.set(true);
    self.world_switcher_open.set(false);

    let ws_pairs: Vec<(Uuid, String)> = payload
      .workspaces
      .iter()
      .map(|w| (w.id, w.name.clone()))
      .collect();
    let _ = self.vault.replace_origin_workspaces(&origin, &ws_pairs);

    let ws_entries: Vec<WorkspaceEntry> = payload
      .workspaces
      .iter()
      .map(|w| WorkspaceEntry {
        id: w.id,
        name: w.name.clone(),
        origin: origin.clone(),
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
        origin.clone(),
        ws_id,
        ws_name,
        payload.views,
        payload.initial_doc,
        true,
      );
      self.cloud_status.set(format!(
        "已连接云端：{}（{} 个工作区）",
        world_switcher::server_label(&origin),
        ws_count
      ));
    } else {
      self.apply_workspace_views(
        origin.clone(),
        Uuid::nil(),
        "暂无云端工作区".to_string(),
        Vec::new(),
        None,
        false,
      );
      self.cloud_status.set(format!(
        "已登录云端：{}（暂无工作区，可在左上角新建）",
        world_switcher::server_label(&origin)
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
      let outcome = net::run_blocking(move || {
        let mut conn = CloudConnector::new(CloudConfig {
          server_url: url_req,
          access_token: None,
          email: Some(email_req.clone()),
        });
        let logged_email = conn.login(&email_req, &password)?;
        let boot = fetch_cloud_bootstrap(&mut conn, None)?;
        Ok::<_, String>((conn, logged_email, boot))
      })
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
    let origin = canonicalize_cloud_url(&raw_origin);
    let Some((email, token, refresh)) = self.vault.saved_server_credential_full(&origin) else {
      if interactive {
        self.world_error.set("未找到该服务器的登录凭证，请重新登录".to_string());
        self.world_switcher_open.set(true);
      }
      return;
    };
    if token.trim().is_empty() && refresh.trim().is_empty() {
      let _ = self.vault.clear_server_credential(&origin);
      let mut creds = self.signed_in_origins_sig.get_untracked();
      creds.retain(|(o, _)| o != &origin);
      self.signed_in_origins_sig.set(creds);
      if interactive {
        self.world_error.set("登录凭证已失效，请重新输入密码登录".to_string());
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

    // Show cached cloud workspaces right away while the network refresh runs.
    let cached_ws = self.vault.list_workspaces(&origin);
    let preferred_ws = cached_ws.first().map(|(id, _)| *id);
    if !cached_ws.is_empty() && !interactive {
      let entries: Vec<WorkspaceEntry> = cached_ws
        .iter()
        .map(|(id, name)| WorkspaceEntry {
          id: *id,
          name: name.clone(),
          origin: origin.clone(),
          page_count: None,
        })
        .collect();
      self.workspaces.set(entries);
      if let Some((first_id, first_name)) = cached_ws.first() {
        let cached_views = self.vault.list_workspace_views(&origin, *first_id);
        self.apply_workspace_views(
          origin.clone(),
          *first_id,
          first_name.clone(),
          cached_views,
          None,
          false,
        );
      }
      self.server_url.set(origin.clone());
      self.selected_cloud_origin.set(origin.clone());
      self.world_active_origin.set(origin.clone());
      self.account_email.set(email.clone());
      self.account_name.set(email.clone());
      self.is_cloud_signed_in.set(true);
    }

    let h = self.clone();
    let origin_req = origin.clone();
    let email_req = email.clone();
    spawn(async move {
      let outcome = net::run_blocking(move || {
        let mut conn = CloudConnector::new(CloudConfig {
          server_url: origin_req,
          access_token: (!token.trim().is_empty()).then_some(token),
          email: Some(email_req),
        })
        .with_refresh_token((!refresh.trim().is_empty()).then_some(refresh));
        if conn.access_token().is_none() {
          conn.refresh_session()?;
        }
        let boot = fetch_cloud_bootstrap(&mut conn, preferred_ws)?;
        Ok::<_, String>((conn, boot))
      })
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
            if interactive {
              h.world_switcher_open.set(true);
            }
          } else {
            h.cloud_status.set(format!("连接云端失败：{err}"));
            if interactive {
              h.world_error.set(err);
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
      let outcome = net::run_blocking(move || {
        let mut conn = CloudConnector::new(CloudConfig {
          server_url: url_req,
          access_token: None,
          email: Some(email_req.clone()),
        });
        let user = conn.register(&email_req, &display_name, &pass_req)?;
        if user.verified {
          let logged_email = conn.login(&email_req, &pass_req)?;
          let boot = fetch_cloud_bootstrap(&mut conn, None)?;
          Ok::<_, String>(Some((conn, logged_email, boot)))
        } else {
          Ok(None)
        }
      })
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

  /// Leave the cloud world and restore the local SQLite workspaces cleanly.
  pub(crate) fn logout_to_local(&self) {
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
    let _ = self.vault.set_active_origin(DEFAULT_ORIGIN);
    *self.cloud_connector.borrow_mut() = CloudConnector::new(CloudConfig {
      server_url: self.server_url.get_untracked(),
      access_token: None,
      email: None,
    });
    self.is_cloud_signed_in.set(false);
    self.world_active_origin.set(LOCAL_ORIGIN.to_string());
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
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    let name_req = name.clone();
    let md_req = markdown.clone();
    spawn(async move {
      let res = net::run_blocking(move || {
        let (mut view, mut rot) =
          with_auto_refresh(&mut conn, |c| c.create_document(ws_id, &name_req, parent))?;
        if view.workspace_id.is_nil() {
          view.workspace_id = ws_id;
        }
        if !md_req.trim().is_empty() {
          let (_, rot2) =
            with_auto_refresh(&mut conn, |c| c.save_document_markdown(ws_id, view.object_id, &md_req))?;
          if rot2.is_some() {
            rot = rot2;
          }
        }
        Ok::<_, String>((conn, view, rot))
      })
      .await;
      if h.active_origin.get_untracked() != origin
        || h.active_workspace_id.get_untracked() != ws_id
      {
        return;
      }
      match res {
        Ok((conn, view, rot)) => {
          *h.cloud_connector.borrow_mut() = conn;
          persist_refreshed_tokens(&h.vault, &origin, &email, rot);
          let doc_id = view.object_id;
          let title = view.name.clone();
          let _ = h.vault.save_view(&origin, &view);
          let _ = h.vault.save_markdown(&origin, doc_id, &markdown);
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
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = net::run_blocking(move || {
        let (mut view, rot) =
          with_auto_refresh(&mut conn, |c| c.create_folder(ws_id, &name, parent))?;
        if view.workspace_id.is_nil() {
          view.workspace_id = ws_id;
        }
        Ok::<_, String>((conn, view, rot))
      })
      .await;
      if h.active_origin.get_untracked() != origin
        || h.active_workspace_id.get_untracked() != ws_id
      {
        return;
      }
      match res {
        Ok((conn, view, rot)) => {
          *h.cloud_connector.borrow_mut() = conn;
          persist_refreshed_tokens(&h.vault, &origin, &email, rot);
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
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = net::run_blocking(move || {
        with_auto_refresh(&mut conn, |c| c.create_workspace(&name)).map(|(w, r)| (conn, w, r))
      })
      .await;
      if h.active_origin.get_untracked() != origin {
        return;
      }
      match res {
        Ok((conn, ws, rot)) => {
          *h.cloud_connector.borrow_mut() = conn;
          persist_refreshed_tokens(&h.vault, &origin, &email, rot);
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

  /// If the last active origin persisted in the vault was a cloud server with
  /// valid credentials, automatically restore and refresh it on startup.
  pub(crate) fn restore_startup_session(&self) {
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
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    spawn(async move {
      let res = net::run_blocking(move || {
        let (usage, mut rot) = match with_auto_refresh(&mut conn, |c| c.workspace_usage(ws_id)) {
          Ok((u, r)) => (Some(u), r),
          Err(_) => (None, None),
        };
        let members = match with_auto_refresh(&mut conn, |c| c.list_members(ws_id)) {
          Ok((m, r)) => {
            if r.is_some() {
              rot = r;
            }
            m
          }
          Err(_) => Vec::new(),
        };
        let invs = match with_auto_refresh(&mut conn, |c| c.list_invitations(ws_id)) {
          Ok((i, r)) => {
            if r.is_some() {
              rot = r;
            }
            i
          }
          Err(_) => Vec::new(),
        };
        (conn, usage, members, invs, rot)
      })
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
        persist_refreshed_tokens(&h.vault, &orig, &email, rot);
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
