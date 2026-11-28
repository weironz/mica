//! Cloud workspace, page-tree, and document synchronization helpers.
//!
//! Keeps blocking HTTP bootstrap/refresh orchestration and token-rotation
//! retry logic out of `lib.rs` so the UI shell stays concise and every
//! cross-await cloud round-trip returns a single atomic payload.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

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
  pub display_name: Option<String>,
  pub avatar_version: Option<String>,
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

/// TTL for reusing a just-completed single-flight token rotation when a sibling
/// async task cloned `CloudConnector` moments before the leader wrote back the
/// rotated tokens. Prevents replaying an already-spent refresh token (which
/// would trigger server-side reuse detection and revoke the entire session family).
const COMPLETED_ROTATION_TTL_MS: u64 = 60_000;

fn refresh_now_ms() -> u64 {
  #[cfg(not(target_arch = "wasm32"))]
  {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
  }
  #[cfg(target_arch = "wasm32")]
  {
    js_sys::Date::now().max(0.0) as u64
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RotatedSession {
  access_token: String,
  refresh_token: Option<String>,
  user_id: Option<String>,
  display_name: Option<String>,
  email: Option<String>,
}

impl RotatedSession {
  fn from_connector(conn: &CloudConnector) -> Self {
    Self {
      access_token: conn.access_token().unwrap_or_default().to_string(),
      refresh_token: conn.refresh_token().map(str::to_string),
      user_id: conn.user_id().map(str::to_string),
      display_name: conn.display_name().map(str::to_string),
      email: conn.config().email.clone(),
    }
  }

  fn apply_to(&self, conn: &mut CloudConnector) {
    if self.access_token.trim().is_empty() {
      conn.clear_access_token();
    }
    conn.apply_rotated_session(
      self.access_token.clone(),
      self.refresh_token.clone(),
      self.user_id.clone(),
      self.display_name.clone(),
      self.email.clone(),
    );
  }

  fn token_pair(&self) -> (String, String) {
    (
      self.access_token.clone(),
      self.refresh_token.clone().unwrap_or_default(),
    )
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RefreshKey {
  server_url: String,
  token_key: String,
}

impl RefreshKey {
  fn from_connector(conn: &CloudConnector) -> Self {
    let server_url = canonicalize_cloud_url(conn.base_url());
    let token_key = match conn.refresh_token() {
      Some(rt) => rt.to_string(),
      None => format!("__cookie__:{}", conn.access_token().unwrap_or("")),
    };
    Self {
      server_url,
      token_key,
    }
  }
}

#[derive(Default)]
struct WaiterSlot {
  result: Option<Result<RotatedSession, String>>,
  waker: Option<Waker>,
}

struct WaiterFuture {
  slot: Rc<RefCell<WaiterSlot>>,
}

impl Future for WaiterFuture {
  type Output = Result<RotatedSession, String>;

  fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    let mut slot = self.slot.borrow_mut();
    if let Some(res) = slot.result.take() {
      return Poll::Ready(res);
    }
    slot.waker = Some(cx.waker().clone());
    Poll::Pending
  }
}

#[derive(Default)]
struct RefreshGate {
  in_flight: HashMap<RefreshKey, Vec<Rc<RefCell<WaiterSlot>>>>,
  completed: HashMap<RefreshKey, (u64, RotatedSession)>,
}

enum RefreshAcquire {
  Cached(RotatedSession),
  Wait(WaiterFuture),
  Leader(InFlightGuard),
}

struct InFlightGuard {
  gate: Rc<RefCell<RefreshGate>>,
  key: RefreshKey,
  finished: bool,
}

impl InFlightGuard {
  fn complete(mut self, result: Result<RotatedSession, String>) {
    self.finished = true;
    let waiters = {
      let mut g = self.gate.borrow_mut();
      if let Ok(ref rot) = result {
        g.completed.insert(self.key.clone(), (refresh_now_ms(), rot.clone()));
      }
      g.in_flight.remove(&self.key).unwrap_or_default()
    };
    for slot in waiters {
      let mut s = slot.borrow_mut();
      s.result = Some(result.clone());
      if let Some(w) = s.waker.take() {
        w.wake();
      }
    }
  }
}

impl Drop for InFlightGuard {
  fn drop(&mut self) {
    if self.finished {
      return;
    }
    let waiters = {
      let mut g = self.gate.borrow_mut();
      g.in_flight.remove(&self.key).unwrap_or_default()
    };
    for slot in waiters {
      let mut s = slot.borrow_mut();
      s.result = Some(Err("token refresh cancelled".to_string()));
      if let Some(w) = s.waker.take() {
        w.wake();
      }
    }
  }
}

impl RefreshGate {
  fn acquire(gate: &Rc<RefCell<Self>>, key: RefreshKey, now_ms: u64) -> RefreshAcquire {
    let mut g = gate.borrow_mut();
    g.completed
      .retain(|_, (ts, _)| now_ms.saturating_sub(*ts) <= COMPLETED_ROTATION_TTL_MS);
    if let Some((_, cached)) = g.completed.get(&key) {
      return RefreshAcquire::Cached(cached.clone());
    }
    if let Some(waiters) = g.in_flight.get_mut(&key) {
      let slot = Rc::new(RefCell::new(WaiterSlot::default()));
      waiters.push(Rc::clone(&slot));
      return RefreshAcquire::Wait(WaiterFuture { slot });
    }
    g.in_flight.insert(key.clone(), Vec::new());
    RefreshAcquire::Leader(InFlightGuard {
      gate: Rc::clone(gate),
      key,
      finished: false,
    })
  }

  fn clear_server(&mut self, server_url: &str) {
    let norm = canonicalize_cloud_url(server_url);
    self.completed.retain(|k, _| k.server_url != norm);
  }
}

thread_local! {
  static REFRESH_GATE: Rc<RefCell<RefreshGate>> = Rc::new(RefCell::new(RefreshGate::default()));
}

/// Clear any cached single-flight token rotations for `server_url` (called on
/// explicit login, logout, or session expiry so tokens never leak across accounts).
pub fn clear_refresh_gate_for_server(server_url: &str) {
  REFRESH_GATE.with(|gate| gate.borrow_mut().clear_server(server_url));
}

/// Single-flight `POST /api/auth/refresh` deduplicated per `(server_url, refresh_token)`.
///
/// Guarantees that concurrent 401 bursts (e.g. multiple image downloads + document
/// fetch + usage stats + WebSocket reconnect) send AT MOST ONE refresh request for
/// a given refresh token, preventing server-side reuse detection from revoking the
/// active login family.
pub async fn refresh_session_single_flight(
  conn: &mut CloudConnector,
) -> Result<(String, String), String> {
  let key = RefreshKey::from_connector(conn);
  let gate = REFRESH_GATE.with(Rc::clone);
  match RefreshGate::acquire(&gate, key, refresh_now_ms()) {
    RefreshAcquire::Cached(rot) => {
      rot.apply_to(conn);
      Ok(rot.token_pair())
    }
    RefreshAcquire::Wait(fut) => {
      let rot = fut.await?;
      rot.apply_to(conn);
      Ok(rot.token_pair())
    }
    RefreshAcquire::Leader(guard) => {
      #[cfg(target_arch = "wasm32")]
      let refresh_res = {
        let before_ms = crate::web_runtime::web_last_refresh_ms();
        let lock_name = format!("mica_auth_refresh:{}", canonicalize_cloud_url(conn.base_url()));
        let mut conn_owned = conn.clone();
        let locked = crate::web_runtime::with_cross_tab_lock(&lock_name, move || async move {
          if crate::web_runtime::web_last_refresh_ms() > before_ms {
            // A sibling browser tab rotated the shared HttpOnly session cookies
            // while we waited for the lock. Drop our expired in-memory Bearer
            // token so retries use the fresh HttpOnly cookie instead of replaying
            // the spent refresh cookie.
            conn_owned.clear_access_token();
            return Ok(conn_owned);
          }
          conn_owned.refresh_session().await?;
          crate::web_runtime::mark_web_refreshed_now();
          Ok(conn_owned)
        })
        .await;
        locked.map(|updated| {
          *conn = updated;
        })
      };
      #[cfg(not(target_arch = "wasm32"))]
      let refresh_res = conn.refresh_session().await;

      match refresh_res {
        Ok(()) => {
          let rot = RotatedSession::from_connector(conn);
          let pair = rot.token_pair();
          guard.complete(Ok(rot));
          Ok(pair)
        }
        Err(err) => {
          guard.complete(Err(err.clone()));
          Err(err)
        }
      }
    }
  }
}

/// Refresh a shared `CloudConnector` via single-flight deduplication and persist
/// rotated tokens into `vault` on desktop.
pub async fn refresh_shared_connector(
  cloud_connector: &Rc<RefCell<CloudConnector>>,
  vault: &VaultStore,
  allow_local: bool,
  server_url: &str,
  email: &str,
) -> Result<(String, String), String> {
  let mut conn = cloud_connector.borrow().clone();
  let stale_access = conn.access_token().map(str::to_string);
  let stale_server = canonicalize_cloud_url(conn.base_url());
  let rotated = refresh_session_single_flight(&mut conn).await?;
  {
    let mut current = cloud_connector.borrow_mut();
    // Ownership check after `.await`: only commit if the active connector still
    // points to the same server and hasn't already been replaced by a newer login.
    if canonicalize_cloud_url(current.base_url()) == stale_server
      && current.access_token().map(str::to_string) == stale_access
    {
      *current = conn;
    }
  }
  if allow_local {
    persist_refreshed_tokens(vault, server_url, email, Some(rotated.clone()));
  }
  Ok(rotated)
}

/// Execute async `op` against `conn`; if the server responds with HTTP 401 and
/// a refresh token (or Web `HttpOnly` cookie) is available, rotate once via the
/// single-flight refresh coordinator and retry.
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
      let rotated = refresh_session_single_flight(conn).await?;
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
  let (account_id, display_name, avatar_version) =
    if let (Some(uid), Some(dname)) = (conn.user_id(), conn.display_name()) {
      (
        uid.to_string(),
        Some(dname.to_string()),
        conn.avatar_version().map(str::to_string),
      )
    } else {
      let (me, rot) = with_auto_refresh(conn, |c| async move { c.fetch_me().await }).await?;
      if rot.is_some() {
        refreshed_tokens = rot;
      }
      let uid = me
        .id
        .filter(|s| !s.trim().is_empty())
        .or_else(|| conn.user_id().map(str::to_string))
        .ok_or_else(|| "server /api/auth/me returned empty user id".to_string())?;
      let dname = (!me.display_name.trim().is_empty()).then(|| me.display_name.trim().to_string());
      let av = me
        .avatar_version
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
      *conn = conn
        .clone()
        .with_user_id(Some(uid.clone()))
        .with_display_name(dname.clone())
        .with_avatar_version(av.clone());
      (uid, dname, av)
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
    display_name,
    avatar_version,
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

/// Coordinates in-flight optimistic view mutations (delete, batch-trash,
/// rename, reorder, empty-trash) against incoming `/ws/.../views` refresh
/// broadcasts so an intermediate `GET /views` can never resurrect newly deleted
/// items or clobber an in-progress title edit.
#[derive(Default)]
pub(crate) struct PendingViewOps {
  in_flight: usize,
  deferred_refresh: Option<(String, String, Uuid)>,
  pending_title_renames: HashMap<Uuid, String>,
}

impl PendingViewOps {
  pub(crate) fn begin_op(&mut self) {
    self.in_flight = self.in_flight.saturating_add(1);
  }

  pub(crate) fn end_op(&mut self) -> Option<(String, String, Uuid)> {
    self.in_flight = self.in_flight.saturating_sub(1);
    if self.in_flight == 0 {
      self.deferred_refresh.take()
    } else {
      None
    }
  }

  pub(crate) fn has_in_flight(&self) -> bool {
    self.in_flight > 0
  }

  pub(crate) fn defer_if_busy(
    &mut self,
    server_url: String,
    cache_origin: String,
    ws_id: Uuid,
  ) -> bool {
    if self.in_flight > 0 {
      self.deferred_refresh = Some((server_url, cache_origin, ws_id));
      true
    } else {
      false
    }
  }

  pub(crate) fn mark_deferred_refresh(
    &mut self,
    server_url: String,
    cache_origin: String,
    ws_id: Uuid,
  ) {
    self.deferred_refresh = Some((server_url, cache_origin, ws_id));
  }

  pub(crate) fn set_pending_title(&mut self, view_id: Uuid, name: String) {
    self.pending_title_renames.insert(view_id, name);
  }

  pub(crate) fn clear_pending_title_if_matches(&mut self, view_id: Uuid, committed: &str) {
    if self
      .pending_title_renames
      .get(&view_id)
      .is_some_and(|cur| cur == committed)
    {
      self.pending_title_renames.remove(&view_id);
    }
  }

  pub(crate) fn clear_pending_title(&mut self, view_id: Uuid) {
    self.pending_title_renames.remove(&view_id);
  }

  pub(crate) fn pending_title_for(&self, view_id: Uuid) -> Option<&str> {
    self.pending_title_renames.get(&view_id).map(String::as_str)
  }

  pub(crate) fn clear_all_pending_titles(&mut self) {
    self.pending_title_renames.clear();
  }
}

/// RAII guard ensuring `PendingViewOps::in_flight` is always decremented even if
/// an async cloud view task is cancelled or panics (`async-review-checklist.md` §3).
struct ViewOpGuard {
  handles: CloudSessionHandles,
}

impl ViewOpGuard {
  fn enter(handles: CloudSessionHandles) -> Self {
    handles.pending_view_ops.borrow_mut().begin_op();
    handles
      .views_refresh_gen
      .set(handles.views_refresh_gen.get_untracked().wrapping_add(1));
    Self { handles }
  }
}

impl Drop for ViewOpGuard {
  fn drop(&mut self) {
    let follow_up = self.handles.pending_view_ops.borrow_mut().end_op();
    if let Some((srv, orig, ws_id)) = follow_up {
      self.handles.refresh_views_only(srv, orig, ws_id);
    }
  }
}

/// Shared UI & storage handles driving cloud auth, workspace bootstrap,
/// tree refresh, and cloud CRUD mutations from `lib.rs`.
#[derive(Clone)]
pub(crate) struct CloudSessionHandles {
  pub app_owner: Owner,
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
  pub focused_nav_id: Signal<Option<Uuid>>,
  pub root_focused: Signal<bool>,
  pub focus_title_rev: Signal<u64>,
  pub sidebar_inline_rename: Signal<Option<Uuid>>,
  pub active_doc_id_str: Signal<String>,
  pub active_title: Signal<String>,
  pub title_input_focused: Signal<bool>,
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
  pub account_avatar_version: Signal<Option<String>>,
  pub avatar_cache_bust_rev: Signal<u64>,
  pub is_cloud_signed_in: Signal<bool>,
  pub cloud_status: Signal<String>,
  pub auth_gen: Signal<u64>,
  pub cloud_load_gen: Signal<u64>,
  pub views_refresh_gen: Signal<u64>,
  pub pending_view_ops: Rc<RefCell<PendingViewOps>>,
  pub doc_fetch_gen: Signal<u64>,
  pub doc_save_gen: Signal<u64>,
  pub doc_loading: Signal<bool>,
  pub doc_load_error: Signal<Option<String>>,
  pub doc_sync: DocSyncController,
  pub views_watcher: ViewsWsWatcher,
  pub refresh_snapshots: Rc<dyn Fn(Uuid)>,
  pub refresh_favorite_pages: Rc<dyn Fn()>,
  pub sync_tree_snapshots: Rc<dyn Fn()>,
  pub open_doc_internal: Rc<dyn Fn(Uuid, String, bool, bool)>,
}

pub(crate) fn dedup_view_ids(ids: Vec<Uuid>) -> Vec<Uuid> {
  let mut unique = Vec::with_capacity(ids.len());
  let mut seen = HashSet::new();
  for id in ids {
    if seen.insert(id) {
      unique.push(id);
    }
  }
  unique
}

impl CloudSessionHandles {
  /// Spawn a cloud async task under the long-lived application `Owner` so closing
  /// transient UI scopes (modals, context menus, popovers, hover strips) never
  /// aborts an in-flight HTTP mutation before it reaches the server.
  pub(crate) fn spawn_on_app<F>(&self, fut: F)
  where
    F: Future<Output = ()> + 'static,
  {
    self.app_owner.run(move || {
      spawn(fut);
    });
  }

  /// Sync a renamed view (page or folder) to cloud with pending-title protection
  /// so concurrent `views_changed` broadcasts cannot revert it mid-flight.
  pub(crate) fn rename_cloud_view(
    &self,
    origin: &str,
    ws_id: Uuid,
    view_id: Uuid,
    next_name: String,
  ) {
    if !is_cloud_origin(origin) || !self.is_cloud_signed_in.get_untracked() || ws_id.is_nil() {
      self.pending_view_ops.borrow_mut().clear_pending_title(view_id);
      return;
    }
    self
      .pending_view_ops
      .borrow_mut()
      .set_pending_title(view_id, next_name.clone());
    let rename_req = next_name.clone();
    let ops_done = self.pending_view_ops.clone();
    self.spawn_cloud_op_with_done(
      "云端重命名",
      move |c| {
        let rename_req = rename_req.clone();
        async move { c.update_view(ws_id, view_id, &rename_req, None).await.map(|_| ()) }
      },
      Some(Box::new(move |_ok| {
        ops_done
          .borrow_mut()
          .clear_pending_title_if_matches(view_id, &next_name);
      })),
    );
  }

  /// Handle an unrecoverable cloud 401 (refresh token expired or revoked) with
  /// ownership verification so a stale request from a previous server/account
  /// can never terminate a newly signed-in session (`async-review-checklist.md` §1).
  pub(crate) fn mark_cloud_session_expired(&self, target_server_url: &str, open_prompt: bool) {
    let cur_srv = canonicalize_cloud_url(&self.server_url.get_untracked());
    let target_norm = canonicalize_cloud_url(target_server_url);
    if !target_norm.is_empty() && !cur_srv.is_empty() && cur_srv != target_norm {
      return;
    }
    let origin_to_clear = if !target_norm.is_empty() {
      target_norm
    } else {
      cur_srv
    };
    if !origin_to_clear.is_empty() {
      clear_refresh_gate_for_server(&origin_to_clear);
      let _ = self.vault.clear_server_credential(&origin_to_clear);
      let mut creds = self.signed_in_origins_sig.get_untracked();
      creds.retain(|(o, _)| canonicalize_cloud_url(o) != origin_to_clear);
      self.signed_in_origins_sig.set(creds);
    }
    self.doc_sync.close();
    self.views_watcher.stop();
    self.is_cloud_signed_in.set(false);
    self.doc_loading.set(false);
    self.world_error.set("登录已过期，请重新登录".to_string());
    self.cloud_status.set("云端登录已过期，请重新登录".to_string());

    // Never leave an uncached cloud page looking like a blank empty document.
    if is_cloud_origin(&self.active_origin.get_untracked())
      && let Some(did) = self.active_doc.get_untracked()
    {
      let has_local_body = self.docs_store.borrow().contains_key(&did)
        || self
          .vault
          .load_markdown(&self.active_origin.get_untracked(), did)
          .is_some();
      if !has_local_body {
        self.doc_load_error.set(Some(
          "云端登录已过期，暂无法拉取该页面最新内容（云端数据安全无损），请重新登录后继续。"
            .to_string(),
        ));
      }
    }

    if !self.allow_local {
      self.world_can_close.set(false);
      self.world_switcher_open.set(true);
    } else if open_prompt {
      self.world_switcher_open.set(true);
    }
  }

  /// Fire a background cloud mutation with transparent 401 token refresh,
  /// ownership verification, and RAII in-flight view-op tracking so concurrent
  /// `views_changed` WS events never resurrect optimistically deleted items.
  pub(crate) fn spawn_cloud_op<F, Fut>(&self, label: &'static str, op: F)
  where
    F: Fn(CloudConnector) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + 'static,
  {
    self.spawn_cloud_op_with_done(label, op, None);
  }

  /// Same as [`Self::spawn_cloud_op`], with an optional completion callback
  /// invoked on the UI owner once the request settles (e.g., to release a
  /// pending title rename lock).
  pub(crate) fn spawn_cloud_op_with_done<F, Fut>(
    &self,
    label: &'static str,
    op: F,
    on_done: Option<Box<dyn FnOnce(bool)>>,
  ) where
    F: Fn(CloudConnector) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + 'static,
  {
    let orig = self.active_origin.get_untracked();
    if !is_cloud_origin(&orig) || !self.is_cloud_signed_in.get_untracked() {
      if let Some(cb) = on_done {
        cb(true);
      }
      return;
    }
    let cur_ws = self.active_workspace_id.get_untracked();
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    let guard = ViewOpGuard::enter(self.clone());
    self.spawn_on_app(async move {
      let _guard = guard;
      let res = with_auto_refresh(&mut conn, op).await.map(|(_, r)| (conn, r));
      let ok = res.is_ok();
      if let Some(cb) = on_done {
        cb(ok);
      }
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
        Err(err) => {
          if CloudConnector::is_unauthorized_error(&err) {
            h.mark_cloud_session_expired(&srv_url, false);
          } else {
            let msg = format!("{label}失败：{err}");
            h.cloud_status.set(msg.clone());
            h.bridge.set_status_flash(msg);
            if !cur_ws.is_nil() {
              h.pending_view_ops
                .borrow_mut()
                .mark_deferred_refresh(srv_url, orig, cur_ws);
            }
          }
        }
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
    self
      .views_refresh_gen
      .set(self.views_refresh_gen.get_untracked().wrapping_add(1));
    self.pending_view_ops.borrow_mut().clear_all_pending_titles();
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

    let opening_page = initial_doc
      .as_ref()
      .and_then(|(did, _)| {
        views
          .iter()
          .find(|r| is_page_view(r) && !r.is_deleted && r.object_id == *did)
      })
      .or_else(|| views.iter().find(|r| is_page_view(r) && !r.is_deleted));

    // Avoid synchronously reading thousands of SQLite snapshots on the UI thread
    // when switching large cloud workspaces (e.g. 3,000+ pages). Cloud pages are
    // loaded lazily on open, and uncached pages are deliberately omitted from
    // `next_docs` so `open_doc_internal` can distinguish "not yet fetched" from
    // "genuinely empty" and show a loading skeleton instead of a blank page.
    let mut next_docs = HashMap::new();
    if is_cloud {
      if let Some((did, md)) = initial_doc.clone() {
        next_docs.insert(did, md);
      } else if let Some(page) = opening_page
        && let Some(md) = self.vault.load_markdown(&origin, page.object_id)
      {
        next_docs.insert(page.object_id, md);
      }
    } else {
      for row in &views {
        if is_page_view(row)
          && let Some(md) = self.vault.load_markdown(&origin, row.object_id)
        {
          next_docs.insert(row.object_id, md);
        }
      }
      if let Some((did, md)) = initial_doc.clone() {
        next_docs.insert(did, md);
      }
    }

    // Every workspace open / switch starts with all folders collapsed by default.
    let next_tree = PageTreeModel::new(views.clone());
    // Derive breadcrumb folder strictly from `opening_page`'s real parent chain;
    // never fall back to `ws_name` (which produced `compute > compute > 暂无页面`).
    let (fid, fname) = if let Some(page) = opening_page {
      (
        page.parent_view_id.unwrap_or_else(Uuid::nil),
        crate::app_modals::resolve_parent_folder_name(&next_tree, page.object_id)
          .unwrap_or_default(),
      )
    } else {
      (Uuid::nil(), String::new())
    };
    *self.tree.borrow_mut() = next_tree;
    *self.docs_store.borrow_mut() = next_docs.clone();
    *self.favorites.borrow_mut() = self.vault.favorites_for(ws_id);

    self.active_workspace_id.set(ws_id);
    self.active_origin.set(origin.clone());
    self.workspace_name.set(ws_name);
    self.active_folder_id.set(fid);
    self.active_folder_name.set(fname);
    self.root_focused.set(false);
    self.focused_nav_id.set(opening_page.map(|p| p.id));

    if let Some(page) = opening_page {
      let doc_id = page.object_id;
      let title = page.name.clone();
      let cached_md = next_docs.get(&doc_id).cloned();
      let is_cloud_signed = is_cloud && self.is_cloud_signed_in.get_untracked() && !ws_id.is_nil();
      let active_md = cached_md.clone().unwrap_or_default();
      self.active_doc.set(Some(doc_id));
      self.active_doc_id_str.set(doc_id.to_string());
      self.active_title.set(title.clone());
      self
        .active_is_favorite
        .set(self.favorites.borrow().contains(&doc_id));
      (self.refresh_snapshots)(doc_id);
      self.doc_loading.set(is_cloud_signed && cached_md.is_none());
      if is_cloud && !is_cloud_signed && cached_md.is_none() {
        self.doc_load_error.set(Some(
          "云端登录已过期，暂无法拉取该页面最新内容（云端数据安全无损），请重新登录后继续。"
            .to_string(),
        ));
      } else {
        self.doc_load_error.set(None);
      }
      self.bridge.load_markdown(&active_md);
      self.open_tabs.set(vec![OpenTab::single(doc_id, title)]);
      self.active_tab_idx.set(0);
      if is_cloud_signed {
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
      self.doc_loading.set(false);
      self.doc_load_error.set(None);
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
  /// without disturbing the active editor CRDT document, any in-flight local view
  /// mutation, or an in-progress title edit.
  pub(crate) fn refresh_views_only(&self, server_url: String, cache_origin: String, ws_id: Uuid) {
    if !is_cloud_origin(&cache_origin) || !self.is_cloud_signed_in.get_untracked() || ws_id.is_nil() {
      return;
    }
    if self
      .pending_view_ops
      .borrow_mut()
      .defer_if_busy(server_url.clone(), cache_origin.clone(), ws_id)
    {
      return;
    }
    let refresh_epoch = self.views_refresh_gen.get_untracked().wrapping_add(1);
    self.views_refresh_gen.set(refresh_epoch);
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    self.spawn_on_app(async move {
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
      if h.views_refresh_gen.get_untracked() != refresh_epoch
        || h.pending_view_ops.borrow().has_in_flight()
        || h.active_origin.get_untracked() != cache_origin
        || h.active_workspace_id.get_untracked() != ws_id
        || !h.is_cloud_signed_in.get_untracked()
      {
        return;
      }
      if let Ok((conn, mut merged, rot)) = res {
        if rot.is_some() {
          *h.cloud_connector.borrow_mut() = conn;
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &server_url, &email, rot);
          }
        }
        // Preserve any locally typed, not-yet-flushed title edits so a background
        // `views_changed` refresh never reverts the sidebar or title input mid-edit.
        {
          let ops = h.pending_view_ops.borrow();
          for row in &mut merged {
            if let Some(local_name) = ops.pending_title_for(row.id) {
              row.name = local_name.to_string();
            }
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
        {
          let editing_this_title = h.title_input_focused.get_untracked()
            || h.pending_view_ops.borrow().pending_title_for(row.id).is_some();
          if !editing_this_title && h.active_title.get_untracked() != row.name {
            h.active_title.set(row.name.clone());
            let mut tabs = h.open_tabs.get_untracked();
            for tab in &mut tabs {
              if tab.doc_id == cur_doc {
                tab.title = row.name.clone();
              }
            }
            h.open_tabs.set(tabs);
          }
        }
        (h.refresh_favorite_pages)();
        (h.sync_tree_snapshots)();
      }
    });
  }

  /// Put the UI shell into an immediate, clean loading state when switching to an
  /// uncached cloud workspace (instead of clearing to a misleading "暂无页面" empty
  /// state while `GET /views` is still in flight).
  pub(crate) fn begin_cloud_workspace_switch(&self, origin: String, ws_id: Uuid, ws_name: String) {
    self.bridge.flush_pending_persist();
    self
      .views_refresh_gen
      .set(self.views_refresh_gen.get_untracked().wrapping_add(1));
    self.pending_view_ops.borrow_mut().clear_all_pending_titles();
    self.doc_sync.close();
    self.views_watcher.stop();

    *self.tree.borrow_mut() = PageTreeModel::new(Vec::new());
    *self.docs_store.borrow_mut() = HashMap::new();
    *self.favorites.borrow_mut() = self.vault.favorites_for(ws_id);

    self.active_workspace_id.set(ws_id);
    self.active_origin.set(origin);
    self.workspace_name.set(ws_name.clone());
    self.active_folder_id.set(Uuid::nil());
    self.active_folder_name.set(String::new());
    self.root_focused.set(false);
    self.focused_nav_id.set(None);
    self.active_doc.set(None);
    self.active_doc_id_str.set(String::new());
    self.active_title.set(format!("正在加载 {ws_name}…"));
    self.active_is_favorite.set(false);
    self.doc_load_error.set(None);
    self.doc_loading.set(true);
    self.bridge.load_markdown("");
    self.open_tabs.set(Vec::new());
    self.active_tab_idx.set(0);
    self.active_mode.set(MODE_DOC);
    (self.refresh_favorite_pages)();
    (self.sync_tree_snapshots)();
  }

  /// Refresh a single cloud workspace's tree and opening document asynchronously.
  ///
  /// Fetches `list_views` + `list_trash` first and renders the sidebar tree
  /// immediately upon arrival (so 1,000+ page workspaces do not block on opening
  /// document Markdown); the opening page then streams through `doc_sync`.
  pub(crate) fn refresh_workspace(
    &self,
    origin: String,
    ws_id: Uuid,
    ws_name: String,
    preferred_doc: Option<Uuid>,
  ) {
    if !is_cloud_origin(&origin) || !self.is_cloud_signed_in.get_untracked() {
      self.doc_loading.set(false);
      return;
    }
    let epoch = self.cloud_load_gen.get_untracked().wrapping_add(1);
    self.cloud_load_gen.set(epoch);
    self.doc_load_error.set(None);
    self.cloud_status.set(format!("正在同步云端工作区：{ws_name}…"));
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    self.spawn_on_app(async move {
      let res = with_auto_refresh(&mut conn, move |c| async move {
        let live = c.list_views(ws_id).await?;
        let trash = c.list_trash(ws_id).await.unwrap_or_default();
        Ok(merge_live_and_trash(live, trash, ws_id))
      })
      .await
      .map(|(views, rot)| (conn, views, rot));
      if h.cloud_load_gen.get_untracked() != epoch
        || h.active_origin.get_untracked() != origin
        || h.active_workspace_id.get_untracked() != ws_id
      {
        return;
      }
      match res {
        Ok((conn, merged, rot)) => {
          *h.cloud_connector.borrow_mut() = conn;
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &srv_url, &email, rot);
          }
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
          let initial_hint = preferred_doc.and_then(|did| {
            h.vault
              .load_markdown(&origin, did)
              .map(|md| (did, md))
          });
          h.apply_workspace_views(origin, ws_id, ws_name.clone(), merged, initial_hint, true);
          h.cloud_status.set(format!("已同步云端工作区：{ws_name}"));
        }
        Err(err) => {
          h.doc_loading.set(false);
          if CloudConnector::is_unauthorized_error(&err) {
            h.mark_cloud_session_expired(&srv_url, false);
          } else {
            let msg = format!("同步云端工作区「{ws_name}」失败：{err}");
            h.cloud_status.set(msg.clone());
            h.doc_load_error.set(Some(msg));
            h.bridge.set_status_flash("同步云端工作区失败");
          }
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
    clear_refresh_gate_for_server(&server_url);
    self.doc_load_error.set(None);
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
    let resolved_name = payload
      .display_name
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .map(str::to_string)
      .unwrap_or_else(|| crate::sidebar::format_footer_account_label(&email));
    self.account_name.set(resolved_name);
    self.avatar_cache_bust_rev.set(0);
    self.account_avatar_version.set(payload.avatar_version);
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
    self.spawn_on_app(async move {
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
      self.account_name.set(crate::sidebar::format_footer_account_label(&email));
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
    self.spawn_on_app(async move {
      let outcome = async {
        let mut conn = CloudConnector::new(CloudConfig {
          server_url: origin_req,
          access_token: (!token.trim().is_empty()).then_some(token),
          email: Some(email_req),
        })
        .with_user_id(uid_req)
        .with_refresh_token((!refresh.trim().is_empty()).then_some(refresh));
        if conn.access_token().is_none() {
          let _ = refresh_session_single_flight(&mut conn).await?;
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
            h.mark_cloud_session_expired(&origin, interactive);
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
    self.spawn_on_app(async move {
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
      self.spawn_on_app(async move {
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
          clear_refresh_gate_for_server(&prev_server);
          let _ = h.vault.clear_server_credential(&prev_server);
        }
        if is_cloud_origin(&web_origin) {
          clear_refresh_gate_for_server(&web_origin);
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
        h.account_avatar_version.set(None);
        h.avatar_cache_bust_rev.set(0);
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

    clear_refresh_gate_for_server(&self.server_url.get_untracked());
    self.doc_load_error.set(None);
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
    self.account_avatar_version.set(None);
    self.avatar_cache_bust_rev.set(0);
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

  /// Reload `tree` from `vault` after a local `reorder_views`, preserving every
  /// currently expanded folder and ensuring `reveal_id`'s ancestors (and optionally
  /// `reveal_id` itself, for folders) are expanded.
  pub(crate) fn reload_tree_preserving_expanded(
    tree_ref: &RefCell<PageTreeModel>,
    vault: &VaultStore,
    origin: &str,
    ws_id: Uuid,
    reveal_id: Uuid,
    expand_self: bool,
  ) {
    let expanded: Vec<Uuid> = tree_ref
      .borrow()
      .all_live_rows()
      .into_iter()
      .filter(|r| r.is_folder && r.expanded)
      .map(|r| r.view_id)
      .collect();
    let views = vault.list_workspace_views(origin, ws_id);
    let mut next_tree = PageTreeModel::new(views);
    for id in expanded {
      next_tree.set_expanded(id, true);
    }
    next_tree.reveal(reveal_id);
    if expand_self {
      next_tree.set_expanded(reveal_id, true);
    }
    *tree_ref.borrow_mut() = next_tree;
  }

  /// Create a cloud page via `POST /api/workspaces/{ws}/documents` so its `id`
  /// and `object_id` match the server authoritatively. When `after_view_id` is
  /// `Some`, places the newly created page immediately after that sibling via
  /// `POST /api/workspaces/{ws}/views/reorder` inside a `ViewOpGuard` so
  /// intermediate WS broadcasts cannot jump the tree order.
  #[allow(clippy::too_many_arguments)]
  pub(crate) fn create_cloud_page(
    &self,
    origin: String,
    ws_id: Uuid,
    parent: Option<Uuid>,
    after_view_id: Option<Uuid>,
    name: String,
    markdown: String,
    in_new_tab: bool,
  ) {
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    let is_blank = markdown.is_empty();
    let name_req = name.clone();
    let md_req = markdown.clone();
    let guard = ViewOpGuard::enter(self.clone());
    self.spawn_on_app(async move {
      let _guard = guard;
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
        Ok((mut conn, view, rot)) => {
          *h.cloud_connector.borrow_mut() = conn.clone();
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &srv_url, &email, rot);
          }
          let view_id = view.id;
          let doc_id = view.object_id;
          let title = view.name.clone();
          let _ = h.vault.save_view(&origin, &view);
          {
            let mut t = h.tree.borrow_mut();
            t.upsert_view(view);
            t.reveal(view_id);
          }
          let reorder_ids = after_view_id.and_then(|anchor_id| {
            let rows = h.tree.borrow().all_live_rows();
            crate::tree_dnd::ordered_siblings_placing_after(&rows, parent, anchor_id, view_id)
          });
          if let Some(ref ordered) = reorder_ids
            && h.vault.reorder_views(&origin, parent, ordered).is_ok()
          {
            Self::reload_tree_preserving_expanded(&h.tree, &h.vault, &origin, ws_id, view_id, false);
          }
          h.root_focused.set(false);
          h.focused_nav_id.set(Some(view_id));
          h.docs_store.borrow_mut().insert(doc_id, markdown);
          (h.sync_tree_snapshots)();
          (h.open_doc_internal)(doc_id, title, true, in_new_tab);
          if is_blank {
            h.focus_title_rev.set(h.focus_title_rev.get_untracked().wrapping_add(1));
          }

          if let Some(ordered) = reorder_ids {
            let ord_req = ordered.clone();
            let reorder_res = with_auto_refresh(&mut conn, move |c| {
              let ids = ord_req.clone();
              async move { c.reorder_views(ws_id, parent, &ids).await }
            })
            .await;
            if h.active_origin.get_untracked() != origin {
              return;
            }
            match reorder_res {
              Ok((_, rot2)) if rot2.is_some() => {
                *h.cloud_connector.borrow_mut() = conn;
                if h.allow_local {
                  persist_refreshed_tokens(&h.vault, &srv_url, &email, rot2);
                }
              }
              Ok(_) => {}
              Err(err) => {
                if CloudConnector::is_unauthorized_error(&err) {
                  h.mark_cloud_session_expired(&srv_url, false);
                } else {
                  let msg = format!("云端新建页面定位排序失败：{err}");
                  h.cloud_status.set(msg.clone());
                  h.bridge.set_status_flash(msg);
                  h.pending_view_ops
                    .borrow_mut()
                    .mark_deferred_refresh(srv_url, origin, ws_id);
                }
              }
            }
          }
        }
        Err(err) => {
          if CloudConnector::is_unauthorized_error(&err) {
            h.mark_cloud_session_expired(&srv_url, false);
          } else {
            h.cloud_status.set(format!("云端新建页面失败：{err}"));
          }
        }
      }
    });
  }

  /// Create a cloud folder via `POST /api/workspaces/{ws}/folders`, optionally
  /// placing it directly after `after_view_id` via `POST /views/reorder`.
  pub(crate) fn create_cloud_folder(
    &self,
    origin: String,
    ws_id: Uuid,
    parent: Option<Uuid>,
    after_view_id: Option<Uuid>,
    name: String,
  ) {
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    let guard = ViewOpGuard::enter(self.clone());
    self.spawn_on_app(async move {
      let _guard = guard;
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
        Ok((mut conn, view, rot)) => {
          *h.cloud_connector.borrow_mut() = conn.clone();
          if h.allow_local {
            persist_refreshed_tokens(&h.vault, &srv_url, &email, rot);
          }
          let fid = view.id;
          let _ = h.vault.save_view(&origin, &view);
          {
            let mut t = h.tree.borrow_mut();
            t.upsert_view(view);
            t.set_expanded(fid, true);
            t.reveal(fid);
          }
          let reorder_ids = after_view_id.and_then(|anchor_id| {
            let rows = h.tree.borrow().all_live_rows();
            crate::tree_dnd::ordered_siblings_placing_after(&rows, parent, anchor_id, fid)
          });
          if let Some(ref ordered) = reorder_ids
            && h.vault.reorder_views(&origin, parent, ordered).is_ok()
          {
            Self::reload_tree_preserving_expanded(&h.tree, &h.vault, &origin, ws_id, fid, true);
          }
          h.root_focused.set(false);
          h.focused_nav_id.set(Some(fid));
          h.sidebar_inline_rename.set(Some(fid));
          (h.sync_tree_snapshots)();

          if let Some(ordered) = reorder_ids {
            let ord_req = ordered.clone();
            let reorder_res = with_auto_refresh(&mut conn, move |c| {
              let ids = ord_req.clone();
              async move { c.reorder_views(ws_id, parent, &ids).await }
            })
            .await;
            if h.active_origin.get_untracked() != origin {
              return;
            }
            match reorder_res {
              Ok((_, rot2)) if rot2.is_some() => {
                *h.cloud_connector.borrow_mut() = conn;
                if h.allow_local {
                  persist_refreshed_tokens(&h.vault, &srv_url, &email, rot2);
                }
              }
              Ok(_) => {}
              Err(err) => {
                if CloudConnector::is_unauthorized_error(&err) {
                  h.mark_cloud_session_expired(&srv_url, false);
                } else {
                  let msg = format!("云端新建文件夹定位排序失败：{err}");
                  h.cloud_status.set(msg.clone());
                  h.bridge.set_status_flash(msg);
                  h.pending_view_ops
                    .borrow_mut()
                    .mark_deferred_refresh(srv_url, origin, ws_id);
                }
              }
            }
          }
        }
        Err(err) => {
          if CloudConnector::is_unauthorized_error(&err) {
            h.mark_cloud_session_expired(&srv_url, false);
          } else {
            h.cloud_status.set(format!("云端新建文件夹失败：{err}"));
          }
        }
      }
    });
  }

  /// Create a cloud workspace via `POST /api/workspaces` and switch into it.
  pub(crate) fn create_cloud_workspace(&self, origin: String, name: String) {
    let srv_url = self.server_url.get_untracked();
    let email = self.account_email.get_untracked();
    let mut conn = self.cloud_connector.borrow().clone();
    let h = self.clone();
    self.spawn_on_app(async move {
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
    self.spawn_on_app(async move {
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
    let preauth_json = crate::web_runtime::take_web_preauth_json();
    self.spawn_on_app(async move {
      let outcome = async {
        let mut conn = CloudConnector::new(CloudConfig {
          server_url: origin_req,
          access_token: None,
          email: None,
        });
        let email = match preauth_json
          .as_deref()
          .and_then(|raw| conn.apply_preauth_response(raw).ok())
        {
          Some(e) => e,
          None => conn.refresh_session_cookie().await?,
        };
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
    self.spawn_on_app(async move {
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
            avatar_url: mica_client::build_avatar_url(
              &srv_url,
              &m.user_id,
              m.avatar_version.as_deref(),
            ),
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

  #[test]
  fn refresh_gate_deduplicates_in_flight_caches_rotation_and_wakes_on_cancel() {
    use std::task::{RawWaker, RawWakerVTable};
    fn noop_waker() -> Waker {
      unsafe fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
      }
      unsafe fn noop(_: *const ()) {}
      static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
      unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
    }

    let gate = Rc::new(RefCell::new(RefreshGate::default()));
    let key = RefreshKey {
      server_url: "https://mica.example.com".to_string(),
      token_key: "rt_1".to_string(),
    };

    // 1. First caller becomes leader; second becomes waiter.
    let leader = match RefreshGate::acquire(&gate, key.clone(), 1000) {
      RefreshAcquire::Leader(g) => g,
      _ => panic!("first caller must be leader"),
    };
    let mut waiter = match RefreshGate::acquire(&gate, key.clone(), 1005) {
      RefreshAcquire::Wait(f) => f,
      _ => panic!("concurrent caller must wait"),
    };
    let w = noop_waker();
    let mut cx = Context::from_waker(&w);
    assert!(Pin::new(&mut waiter).poll(&mut cx).is_pending());

    // Leader completes -> waiter receives rotated tokens & subsequent stale caller hits cache.
    let rot = RotatedSession {
      access_token: "jwt_2".to_string(),
      refresh_token: Some("rt_2".to_string()),
      user_id: Some("u1".to_string()),
      display_name: Some("Will".to_string()),
      email: Some("w@example.com".to_string()),
    };
    leader.complete(Ok(rot.clone()));
    match Pin::new(&mut waiter).poll(&mut cx) {
      Poll::Ready(Ok(got)) => assert_eq!(got, rot),
      other => panic!("expected Ready(Ok), got {other:?}"),
    }
    match RefreshGate::acquire(&gate, key.clone(), 1050) {
      RefreshAcquire::Cached(got) => assert_eq!(got, rot),
      _ => panic!("late sibling with stale rt_1 must hit completed cache"),
    }

    // 2. Explicit clear_server drops cached rotation.
    gate.borrow_mut().clear_server("https://mica.example.com/");
    let leader2 = match RefreshGate::acquire(&gate, key.clone(), 2000) {
      RefreshAcquire::Leader(g) => g,
      _ => panic!("cleared server must acquire fresh leader"),
    };
    let mut waiter2 = match RefreshGate::acquire(&gate, key.clone(), 2001) {
      RefreshAcquire::Wait(f) => f,
      _ => panic!("second waiter must wait"),
    };

    // 3. RAII Drop guard: dropping leader mid-flight wakes waiters with Err (no hang).
    drop(leader2);
    match Pin::new(&mut waiter2).poll(&mut cx) {
      Poll::Ready(Err(msg)) => assert!(msg.contains("cancelled"), "msg={msg}"),
      other => panic!("expected Ready(Err(cancelled)), got {other:?}"),
    }
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn concurrent_401_burst_issues_single_refresh_without_triggering_reuse_revocation() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
      Arc,
      atomic::{AtomicUsize, Ordering},
    };

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock auth server");
    let addr = listener.local_addr().unwrap();
    let refresh_calls = Arc::new(AtomicUsize::new(0));
    let refresh_calls_srv = Arc::clone(&refresh_calls);

    let server_thread = std::thread::spawn(move || {
      // Handle 6 initial 401s + 1 refresh + 6 retried 200s = 13 requests max.
      for _ in 0..20 {
        let Ok((mut stream, _)) = listener.accept() else {
          break;
        };
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
          continue;
        }
        let req = String::from_utf8_lossy(&buf[..n]);
        let first_line = req.lines().next().unwrap_or("");
        let (status, body) = if first_line.starts_with("POST /api/auth/refresh") {
          let prev = refresh_calls_srv.fetch_add(1, Ordering::SeqCst);
          if prev == 0 {
            // Delay slightly so all concurrent 401 callers pile onto the in-flight slot.
            std::thread::sleep(std::time::Duration::from_millis(60));
            (
              "200 OK",
              r#"{"access_token":"jwt-fresh","token_type":"Bearer","expires_at":"2099-01-01T00:00:00Z","refresh_token":"rt-fresh","user":{"id":"00000000-0000-4000-8000-000000000001","email":"u@mica.dev","display_name":"Will","created_at":"2026-01-01T00:00:00Z"}}"#,
            )
          } else {
            // Strict server-side single-use refresh token reuse detection!
            ("401 Unauthorized", r#"{"error":"refresh token reused - family revoked"}"#)
          }
        } else if req.to_ascii_lowercase().contains("authorization: bearer jwt-fresh") {
          ("200 OK", r#"{"markdown":"fresh content"}"#)
        } else {
          ("401 Unauthorized", r#"{"error":"access token expired"}"#)
        };
        let resp = format!(
          "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
          body.len()
        );
        let _ = stream.write_all(resp.as_bytes());
      }
    });

    let base_url = format!("http://{addr}");
    clear_refresh_gate_for_server(&base_url);
    let stale_conn = CloudConnector::new(CloudConfig {
      server_url: base_url.clone(),
      access_token: Some("jwt-expired".to_string()),
      email: Some("u@mica.dev".to_string()),
    })
    .with_refresh_token(Some("rt-single-use".to_string()));

    let ws_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    type FetchOutcome = Result<(String, Option<(String, String)>), String>;
    type FetchFuture = Pin<Box<dyn Future<Output = FetchOutcome>>>;

    // Drive 6 concurrent `with_auto_refresh` tasks on a minimal single-threaded
    // executor (matching NSH's UI async runtime).
    let mut futs: Vec<FetchFuture> = (0..6)
      .map(|_| {
        let mut c = stale_conn.clone();
        Box::pin(async move {
          with_auto_refresh(&mut c, move |cc| async move {
            cc.fetch_document_markdown(ws_id, doc_id).await
          })
          .await
        }) as FetchFuture
      })
      .collect();

    use std::task::{RawWaker, RawWakerVTable};
    fn noop_waker() -> Waker {
      unsafe fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
      }
      unsafe fn noop(_: *const ()) {}
      static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
      unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
    }
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut results: Vec<Option<FetchOutcome>> = (0..futs.len()).map(|_| None).collect();
    let start = std::time::Instant::now();
    while results.iter().any(Option::is_none) {
      assert!(
        start.elapsed() < std::time::Duration::from_secs(10),
        "concurrent single-flight refresh timed out"
      );
      for (i, fut) in futs.iter_mut().enumerate() {
        if results[i].is_none()
          && let Poll::Ready(res) = fut.as_mut().poll(&mut cx)
        {
          results[i] = Some(res);
        }
      }
      std::thread::sleep(std::time::Duration::from_millis(5));
    }

    assert_eq!(
      refresh_calls.load(Ordering::SeqCst),
      1,
      "6 concurrent 401s must trigger EXACTLY ONE POST /api/auth/refresh"
    );
    for (idx, res) in results.into_iter().enumerate() {
      let (md, rot) = res.unwrap().unwrap_or_else(|e| panic!("task {idx} failed: {e}"));
      assert_eq!(md, "fresh content");
      assert_eq!(rot, Some(("jwt-fresh".to_string(), "rt-fresh".to_string())));
    }
    clear_refresh_gate_for_server(&base_url);
    drop(server_thread);
  }

  #[test]
  fn pending_view_ops_defers_refresh_until_all_mutations_finish_and_protects_title() {
    let mut ops = PendingViewOps::default();
    let ws = Uuid::new_v4();
    let vid = Uuid::new_v4();

    // No in-flight ops -> refresh runs immediately.
    assert!(!ops.defer_if_busy("https://mica.example".into(), "cloud".into(), ws));

    // Two overlapping mutations (e.g. rapid delete A + delete B):
    ops.begin_op();
    ops.begin_op();
    assert!(ops.has_in_flight());
    // Incoming WS `views_changed` after delete A MUST be deferred so delete B doesn't resurrect.
    assert!(ops.defer_if_busy("https://mica.example".into(), "cloud".into(), ws));
    assert!(ops.end_op().is_none(), "still 1 op in flight");
    // Last op finishes -> returns exactly one coalesced follow-up refresh.
    let follow_up = ops.end_op();
    assert_eq!(
      follow_up,
      Some(("https://mica.example".to_string(), "cloud".to_string(), ws))
    );
    assert!(!ops.has_in_flight());

    // Pending title rename protects against stale overwrite and only clears when matching committed text.
    ops.set_pending_title(vid, "测试abc标题".to_string());
    assert_eq!(ops.pending_title_for(vid), Some("测试abc标题"));
    ops.clear_pending_title_if_matches(vid, "测试a标题");
    assert_eq!(
      ops.pending_title_for(vid),
      Some("测试abc标题"),
      "older committed rename must not unlock newer in-progress typing"
    );
    ops.clear_pending_title_if_matches(vid, "测试abc标题");
    assert_eq!(ops.pending_title_for(vid), None);

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    assert_eq!(dedup_view_ids(vec![a, b, a, b]), vec![a, b]);
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn cloud_rename_and_batch_trash_survive_disposed_when_scope_owner() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use nsh_core::geometry::{px, size};
    use nsh_engine::{EngineConfig, Root};
    use nsh_testing::Headless;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock view ops server");
    let addr = listener.local_addr().unwrap();
    let recorded_methods: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded_srv = Arc::clone(&recorded_methods);
    let view_id = Uuid::new_v4();
    let ws_id = Uuid::new_v4();

    let server_thread = std::thread::spawn(move || {
      for _ in 0..2 {
        let Ok((mut stream, _)) = listener.accept() else {
          break;
        };
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
        let mut raw = Vec::new();
        let mut chunk = [0u8; 2048];
        let mut expected_total = None;
        loop {
          match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
              raw.extend_from_slice(&chunk[..n]);
              if expected_total.is_none()
                && let Some(hdr_end) = raw.windows(4).position(|w| w == b"\r\n\r\n")
              {
                let hdr_str = String::from_utf8_lossy(&raw[..hdr_end]);
                let content_len = hdr_str
                  .lines()
                  .find_map(|line| {
                    let (k, v) = line.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                      .then(|| v.trim().parse::<usize>().ok())
                      .flatten()
                  })
                  .unwrap_or(0);
                expected_total = Some(hdr_end + 4 + content_len);
              }
              if expected_total.is_some_and(|need| raw.len() >= need) {
                break;
              }
            }
          }
        }
        if raw.is_empty() {
          continue;
        }
        let req = String::from_utf8_lossy(&raw);
        let first_line = req.lines().next().unwrap_or("").to_string();
        let (status, body) = if first_line.starts_with("PATCH ") {
          (
            "200 OK",
            format!(
              r#"{{"view":{{"id":"{view_id}","workspace_id":"{ws_id}","parent_view_id":null,"object_id":"{view_id}","object_type":"folder","name":"架构归档","icon":null,"position":"0000000010","is_deleted":false,"created_by":"00000000-0000-4000-8000-000000000001","created_at":"2026-11-28T00:00:00Z","updated_at":"2026-11-28T00:00:00Z"}}}}"#
            ),
          )
        } else {
          ("200 OK", r#"{"affected":1,"skipped":[]}"#.to_string())
        };
        recorded_srv.lock().unwrap().push(first_line);
        let resp = format!(
          "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
          body.len()
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.flush();
      }
    });

    static MOCK_TARGET: std::sync::Mutex<Option<(String, Uuid, Uuid)>> =
      std::sync::Mutex::new(None);
    *MOCK_TARGET.lock().unwrap() = Some((format!("http://{addr}"), ws_id, view_id));

    #[nsh_core::prelude::component]
    fn DisposedModalCloudOpHarness() -> impl IntoElement {
      let app_owner = Owner::current();
      let modal_open = signal(true);
      let done_count = signal(0usize);
      let (base_url, ws_id, view_id) = MOCK_TARGET.lock().unwrap().clone().expect("mock target set");

      view()
        .flex_1()
        .child(text(move || format!("done-{}", done_count.get())))
        .child(when(move || modal_open.get(), move || {
          let base = base_url.clone();
          nsh_base::button()
            .test_id("trigger-modal-cloud-ops")
            .on_press(move || {
              modal_open.set(false);
              let base_c = base.clone();
              app_owner.run(move || {
                spawn(async move {
                  nsh_core::prelude::next_frame().await;
                  nsh_core::prelude::next_frame().await;
                  let conn = CloudConnector::new(CloudConfig {
                    server_url: base_c,
                    access_token: Some("tok".to_string()),
                    email: Some("w@mica.dev".to_string()),
                  });
                  conn
                    .update_view(ws_id, view_id, "架构归档", None)
                    .await
                    .expect("mock PATCH /views must succeed");
                  conn
                    .batch_trash_views(ws_id, &[view_id])
                    .await
                    .expect("mock POST /views/batch-trash must succeed");
                  done_count.set(2);
                });
              });
            })
            .child(text("提交云端变更"))
        }))
    }

    let mut config = EngineConfig::new(Root::of::<DisposedModalCloudOpHarness>());
    config.viewport = size(px(400.0), px(300.0));
    let mut app = Headless::start(config).unwrap();
    app.wait_for("id:trigger-modal-cloud-ops").unwrap();
    app.press("id:trigger-modal-cloud-ops").unwrap();
    app.wait_for("done-2").unwrap();

    let hits = recorded_methods.lock().unwrap().clone();
    assert!(
      hits.iter().any(|h| h.starts_with(&format!("PATCH /api/workspaces/{ws_id}/views/{view_id}"))),
      "expected PATCH folder rename after modal closed, got: {hits:?}"
    );
    assert!(
      hits.iter().any(|h| h.starts_with(&format!("POST /api/workspaces/{ws_id}/views/batch-trash"))),
      "expected POST batch-trash after menu closed, got: {hits:?}"
    );
    let _ = server_thread.join();
  }

  #[test]
  fn breadcrumb_folder_resolver_never_falls_back_to_workspace_name() {
    let ws_id = Uuid::new_v4();
    let folder_id = Uuid::new_v4();
    let folder = crate::app_modals::make_view(ws_id, folder_id, "网络", "folder", None, "0000000010");
    let root_page =
      crate::app_modals::make_view(ws_id, Uuid::new_v4(), "根页面", "page", None, "0000000020");
    let nested_page = crate::app_modals::make_view(
      ws_id,
      Uuid::new_v4(),
      "VPN 配置",
      "page",
      Some(folder_id),
      "0000000030",
    );

    let empty_tree = PageTreeModel::new(Vec::new());
    assert_eq!(crate::app_modals::resolve_parent_folder_name(&empty_tree, root_page.object_id), None);

    let root_only_tree = PageTreeModel::new(vec![folder.clone(), root_page.clone()]);
    assert_eq!(
      crate::app_modals::resolve_parent_folder_name(&root_only_tree, root_page.object_id),
      None,
      "root-level page must not inherit unrelated workspace folder name"
    );

    let nested_tree = PageTreeModel::new(vec![folder, root_page, nested_page.clone()]);
    assert_eq!(
      crate::app_modals::resolve_parent_folder_name(&nested_tree, nested_page.object_id).as_deref(),
      Some("网络")
    );
  }
}
