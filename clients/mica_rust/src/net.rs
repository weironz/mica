//! Cloud HTTP API client & Sans-I/O [`SyncEngine`] coordinator (`CloudConnector`).
//!
//! Uses [`mica_client`] for single-source endpoint URLs and account/sync
//! generation guards ([`ClaimTracker`]), and [`mica_sync`] for the Sans-I/O
//! document synchronization state machine.
//!
//! Transport is unified through [`send_http`]: desktop uses `ureq` on a
//! dedicated worker thread ([`run_blocking`]), and Web (`wasm32`) uses
//! async `window.fetch` with `AbortController` timeouts via [`crate::web_runtime::fetch_request`].

use mica_api_types::{View, Workspace};
use mica_client::protocol::{decode_auth, decode_views, decode_workspaces};
use mica_client::{ApiEndpoints, ClaimTracker, WorldOrigin};
use mica_sync::{
  MemoryMirrorStorage, SyncConfig, SyncEffect, SyncEngine, SyncEvent, SyncPhase, SyncStorage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

const HEALTH_TIMEOUT_SECS: u64 = 5;
const LOGIN_TIMEOUT_SECS: u64 = 10;
const API_TIMEOUT_SECS: u64 = 15;
const AI_TIMEOUT_SECS: u64 = 60;

/// Serializable cloud connection & session configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudConfig {
  pub server_url: String,
  pub access_token: Option<String>,
  pub email: Option<String>,
}

/// Result of probing a Mica server's `/api/ready` (or `/api/health`) endpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerProbeMeta {
  pub reachable: bool,
  pub version: Option<String>,
  pub registration_open: Option<bool>,
}

/// Authenticated or newly registered cloud user identity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudUser {
  pub id: Option<String>,
  pub email: String,
  pub display_name: String,
  pub verified: bool,
}

/// Public share link status for a cloud document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareState {
  pub shared: bool,
  pub token: Option<String>,
  pub url: Option<String>,
}

/// Individual comment message inside a [`CommentThread`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentItem {
  pub id: String,
  #[serde(default)]
  pub author_id: String,
  #[serde(default)]
  pub author_name: String,
  pub body: String,
  #[serde(default)]
  pub created_at: String,
  #[serde(default)]
  pub edited_at: Option<String>,
}

/// Document comment thread with optional resolved anchor range.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentThread {
  pub id: String,
  #[serde(default)]
  pub status: String,
  #[serde(default)]
  pub quote: String,
  #[serde(default)]
  pub created_by: String,
  #[serde(default)]
  pub created_at: String,
  #[serde(default)]
  pub resolved: bool,
  #[serde(default)]
  pub resolved_by: Option<String>,
  #[serde(default)]
  pub resolved_at: Option<String>,
  #[serde(default)]
  pub start_block: Option<String>,
  #[serde(default)]
  pub start_offset: Option<u32>,
  #[serde(default)]
  pub end_block: Option<String>,
  #[serde(default)]
  pub end_offset: Option<u32>,
  #[serde(default)]
  pub comments: Vec<CommentItem>,
}

/// Personal Access Token metadata returned by `/api/auth/tokens`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiTokenInfo {
  pub id: String,
  pub name: String,
  pub can_write: bool,
  pub created_at: String,
  pub expires_at: Option<String>,
  pub last_used_at: Option<String>,
}

/// Newly minted Personal Access Token including its one-time plaintext secret.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedApiToken {
  pub info: ApiTokenInfo,
  pub token: String,
}

/// Configured cloud AI provider entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProviderConfig {
  pub id: String,
  pub name: String,
  pub base_url: String,
  pub protocol: String,
  pub has_key: bool,
  pub key_tail: String,
  pub model: String,
  pub active: bool,
}

/// Workspace member metadata returned by `GET /api/workspaces/{ws}/members`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMemberInfo {
  pub user_id: String,
  pub email: String,
  pub display_name: String,
  pub role: String,
  pub joined_at: String,
  pub avatar_version: Option<String>,
}

/// Workspace invitation metadata (sent or received).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInvitationInfo {
  pub id: String,
  pub workspace_id: String,
  pub workspace_name: Option<String>,
  pub email: Option<String>,
  pub role: String,
  pub invited_by: Option<String>,
  pub created_at: String,
}

/// Node in a cloud workspace's page-link graph.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudGraphNode {
  pub view_id: String,
  pub name: String,
  pub degree: i64,
}

/// Directed edge (`source -> target`) in a cloud workspace's page-link graph.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudGraphEdge {
  pub source: String,
  pub target: String,
}

/// Payload returned by `GET /api/workspaces/{ws}/graph`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudGraphPayload {
  pub nodes: Vec<CloudGraphNode>,
  pub edges: Vec<CloudGraphEdge>,
  pub unlinked: i64,
}

/// Coordinates cloud HTTP auth/health probes, [`ClaimTracker`] ownership
/// generations, cloud resource APIs, and per-document Sans-I/O [`SyncEngine`]
/// instances.
#[derive(Debug, Clone)]
pub struct CloudConnector {
  config: CloudConfig,
  user_id: Option<String>,
  refresh_token: Option<String>,
  endpoints: ApiEndpoints,
  claims: ClaimTracker,
  phase: SyncPhase,
}

impl CloudConnector {
  /// Build a connector from `config`. Starts in [`WorldOrigin::Cloud`] only
  /// when both a non-empty `server_url` and `access_token` are present.
  pub fn new(mut config: CloudConfig) -> Self {
    let endpoints = ApiEndpoints::new(config.server_url.trim());
    config.server_url = endpoints.base_url().to_string();

    let has_token = config.access_token.as_deref().is_some_and(|t| !t.trim().is_empty());
    let origin = if has_token && !endpoints.base_url().is_empty() {
      WorldOrigin::Cloud { server_url: endpoints.base_url().to_string() }
    } else {
      WorldOrigin::Local
    };

    Self {
      config,
      user_id: None,
      refresh_token: None,
      endpoints,
      claims: ClaimTracker::new(origin),
      phase: SyncPhase::Offline,
    }
  }

  /// Attach the authenticated server user UUID (used for `#111` account-scoped
  /// cloud cache keys).
  pub fn with_user_id(mut self, user_id: Option<String>) -> Self {
    self.user_id = user_id
      .map(|u| u.trim().to_string())
      .filter(|u| !u.is_empty());
    self
  }

  /// Authenticated user UUID, if known from login/refresh/`GET /api/auth/me`.
  pub fn user_id(&self) -> Option<&str> {
    self
      .user_id
      .as_deref()
      .map(str::trim)
      .filter(|u| !u.is_empty())
  }

  /// Attach a persisted refresh token so expired access tokens can be rotated
  /// via `POST /api/auth/refresh`.
  pub fn with_refresh_token(mut self, refresh_token: Option<String>) -> Self {
    self.refresh_token = refresh_token
      .map(|t| t.trim().to_string())
      .filter(|t| !t.is_empty());
    self
  }

  /// Active JWT access token, if authenticated.
  pub fn access_token(&self) -> Option<&str> {
    self
      .config
      .access_token
      .as_deref()
      .map(str::trim)
      .filter(|t| !t.is_empty())
  }

  /// Active refresh token, if one was issued at login/refresh.
  pub fn refresh_token(&self) -> Option<&str> {
    self
      .refresh_token
      .as_deref()
      .map(str::trim)
      .filter(|t| !t.is_empty())
  }

  /// Return `true` when an error string indicates an HTTP 401 / missing auth
  /// condition where session refresh or re-authentication is required.
  pub fn is_unauthorized_error(err: &str) -> bool {
    err.contains("401")
      || err.contains("Unauthorized")
      || err.contains("authentication required")
  }

  /// Probe `GET /api/health` on the configured server.
  pub async fn check_health(&self) -> Result<String, String> {
    self.require_server()?;
    let url = self.endpoints.health();
    let body = send_http("GET", &url, None, None, HEALTH_TIMEOUT_SECS).await
      .map_err(|e| format!("health check failed for {url}: {e}"))?;
    Ok(format_health_summary(&body))
  }

  /// Probe `origin` via `GET /api/ready` (falling back to `GET /api/health`) to
  /// discover reachability, server version, and open-registration status.
  pub async fn probe_server_meta(&self, origin: &str) -> Result<ServerProbeMeta, String> {
    let target = {
      let t = origin.trim().trim_end_matches('/');
      if t.is_empty() { self.endpoints.base_url().to_string() } else { t.to_string() }
    };
    if target.is_empty() {
      return Err("server URL is empty".to_string());
    }

    for path in ["/api/ready", "/api/health"] {
      if let Ok(body) = send_http("GET", &format!("{target}{path}"), None, None, HEALTH_TIMEOUT_SECS).await {
        return Ok(parse_probe_meta(&body));
      }
    }
    Ok(ServerProbeMeta::default())
  }

  /// Authenticate via `POST /api/auth/login`, persist token & email in memory,
  /// and advance [`ClaimTracker`] to the cloud origin.
  pub async fn login(&mut self, email: &str, password: &str) -> Result<String, String> {
    self.require_server()?;
    let email_trimmed = email.trim();
    if email_trimmed.is_empty() || password.is_empty() {
      return Err("email and password are required".to_string());
    }

    let claim = self.claims.capture();
    let payload = json!({ "email": email_trimmed, "password": password });
    let body = send_http("POST", &self.endpoints.login(), None, Some(&payload), LOGIN_TIMEOUT_SECS).await
      .map_err(|e| format!("login request failed: {e}"))?;
    let auth = decode_auth(&body).map_err(|e| format!("invalid login response: {e}"))?;

    if !self.claims.matches_account(&claim) {
      return Err("account session changed during login".to_string());
    }

    self.config.access_token = Some(auth.access_token);
    self.user_id = Some(auth.user.id.to_string());
    self.refresh_token = (!auth.refresh_token.trim().is_empty()).then_some(auth.refresh_token);
    self.config.email = Some(auth.user.email.clone());
    self.claims.switch_account(WorldOrigin::Cloud {
      server_url: self.endpoints.base_url().to_string(),
    });
    Ok(auth.user.email)
  }

  /// Fetch the authenticated user's identity (`GET /api/auth/me`) to resolve
  /// `user_id` when restoring legacy credentials saved without `account_id`.
  pub async fn fetch_me(&self) -> Result<CloudUser, String> {
    let body = self.authed_get("/api/auth/me", LOGIN_TIMEOUT_SECS).await?;
    let val: Value =
      serde_json::from_str(&body).map_err(|e| format!("invalid /api/auth/me response: {e}"))?;
    let user_obj = val.get("user").unwrap_or(&val);
    let id = user_obj
      .get("id")
      .and_then(Value::as_str)
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .map(str::to_string)
      .ok_or_else(|| "missing user id in /api/auth/me response".to_string())?;
    let email = user_obj
      .get("email")
      .and_then(Value::as_str)
      .unwrap_or_default()
      .to_string();
    let display_name = user_obj
      .get("display_name")
      .and_then(Value::as_str)
      .unwrap_or(&email)
      .to_string();
    Ok(CloudUser {
      id: Some(id),
      email,
      display_name,
      verified: true,
    })
  }

  /// Rotate an expired JWT access token via `POST /api/auth/refresh`.
  pub async fn refresh_session(&mut self) -> Result<(), String> {
    self.require_server()?;
    let payload = match self.refresh_token() {
      Some(refresh_tok) => json!({ "refresh_token": refresh_tok }),
      None if cfg!(target_arch = "wasm32") => json!({}),
      None => return Err("refresh token is unavailable".to_string()),
    };

    let body = send_http("POST", &self.endpoints.refresh(), None, Some(&payload), LOGIN_TIMEOUT_SECS).await
      .map_err(|e| format!("token refresh failed: {e}"))?;
    let auth = decode_auth(&body).map_err(|e| format!("invalid refresh response: {e}"))?;
    self.config.access_token = Some(auth.access_token);
    self.user_id = Some(auth.user.id.to_string());
    if !auth.refresh_token.trim().is_empty() {
      self.refresh_token = Some(auth.refresh_token);
    }
    self.config.email = Some(auth.user.email);
    self.claims.switch_account(WorldOrigin::Cloud {
      server_url: self.endpoints.base_url().to_string(),
    });
    Ok(())
  }

  /// Restore or rotate a browser cloud session using the server's `HttpOnly`
  /// `mica_refresh` cookie (`POST /api/auth/refresh` with `{}` body), returning
  /// the authenticated user's email without touching `localStorage`.
  pub async fn refresh_session_cookie(&mut self) -> Result<String, String> {
    self.require_server()?;
    let payload = json!({});
    let body = send_http("POST", &self.endpoints.refresh(), None, Some(&payload), LOGIN_TIMEOUT_SECS).await
      .map_err(|e| format!("cookie session refresh failed: {e}"))?;
    let auth = decode_auth(&body).map_err(|e| format!("invalid refresh response: {e}"))?;
    let email = auth.user.email.clone();
    self.config.access_token = Some(auth.access_token);
    self.user_id = Some(auth.user.id.to_string());
    self.refresh_token = None;
    self.config.email = Some(email.clone());
    self.claims.switch_account(WorldOrigin::Cloud {
      server_url: self.endpoints.base_url().to_string(),
    });
    Ok(email)
  }

  /// End the current cloud session server-side (`POST /api/auth/logout`),
  /// revoking the refresh token family and clearing `HttpOnly` auth cookies.
  pub async fn logout_server(&self) -> Result<(), String> {
    self.require_server()?;
    let url = format!("{}/api/auth/logout", self.endpoints.base_url());
    let payload = match self.refresh_token() {
      Some(rt) => json!({ "refresh_token": rt }),
      None => json!({}),
    };
    let _ = send_http("POST", &url, None, Some(&payload), LOGIN_TIMEOUT_SECS).await
      .map_err(|e| format!("logout failed: {e}"))?;
    Ok(())
  }

  // ── Cloud Workspaces, Views & Documents ─────────────────────────────────

  /// Fetch all cloud workspaces the signed-in account belongs to (`GET /api/workspaces`).
  pub async fn list_workspaces(&self) -> Result<Vec<Workspace>, String> {
    let body = self.authed_get("/api/workspaces", API_TIMEOUT_SECS).await?;
    decode_workspaces(&body).map_err(|e| format!("invalid workspaces response: {e}"))
  }

  /// Create a new cloud workspace (`POST /api/workspaces`).
  pub async fn create_workspace(&self, name: &str) -> Result<Workspace, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
      return Err("workspace name must not be empty".to_string());
    }
    let body = self.authed_post_json("/api/workspaces", &json!({ "name": trimmed }), API_TIMEOUT_SECS).await?;
    let val: Value = serde_json::from_str(body.trim())
      .map_err(|e| format!("invalid create workspace response: {e}"))?;
    let ws_val = val.get("workspace").cloned().unwrap_or(val);
    serde_json::from_value(ws_val).map_err(|e| format!("invalid workspace payload: {e}"))
  }

  /// Rename a cloud workspace (`PATCH /api/workspaces/{ws}`).
  pub async fn update_workspace(&self, workspace_id: Uuid, name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
      return Err("workspace name must not be empty".to_string());
    }
    self.authed_patch_json(
      &format!("/api/workspaces/{workspace_id}"),
      &json!({ "name": trimmed }),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  /// Delete a cloud workspace (`DELETE /api/workspaces/{ws}`).
  pub async fn delete_workspace(&self, workspace_id: Uuid) -> Result<(), String> {
    self.authed_delete(&format!("/api/workspaces/{workspace_id}"), API_TIMEOUT_SECS).await?;
    Ok(())
  }

  /// Fetch all live views in a cloud workspace (`GET /api/workspaces/{ws}/views`).
  pub async fn list_views(&self, workspace_id: Uuid) -> Result<Vec<View>, String> {
    let body = self.authed_get(&format!("/api/workspaces/{workspace_id}/views"), API_TIMEOUT_SECS).await?;
    decode_views(&body).map_err(|e| format!("invalid views response: {e}"))
  }

  /// Fetch trashed views in a cloud workspace (`GET /api/workspaces/{ws}/trash`).
  pub async fn list_trash(&self, workspace_id: Uuid) -> Result<Vec<View>, String> {
    let body = self.authed_get(&format!("/api/workspaces/{workspace_id}/trash"), API_TIMEOUT_SECS).await?;
    decode_views(&body).map_err(|e| format!("invalid trash views response: {e}"))
  }

  /// Read a cloud document's Markdown (`GET /api/workspaces/{ws}/documents/{doc}/markdown`,
  /// falling back to `/export/markdown` for older servers).
  pub async fn fetch_document_markdown(&self, workspace_id: Uuid, document_id: Uuid) -> Result<String, String> {
    let primary = format!("/api/workspaces/{workspace_id}/documents/{document_id}/markdown");
    let body = match self.authed_get(&primary, API_TIMEOUT_SECS).await {
      Ok(b) => b,
      Err(err) if err.contains("404") => self.authed_get(
        &format!("/api/workspaces/{workspace_id}/documents/{document_id}/export/markdown"),
        API_TIMEOUT_SECS,
      ).await?,
      Err(err) => return Err(err),
    };
    let val: Value = serde_json::from_str(body.trim())
      .map_err(|e| format!("invalid document markdown response: {e}"))?;
    Ok(j_str(&val, "markdown"))
  }

  /// Persist a cloud document's Markdown via `PATCH /api/workspaces/{ws}/documents/{doc}/markdown`
  /// (`mode: "replace_all"`).
  pub async fn save_document_markdown(
    &self,
    workspace_id: Uuid,
    document_id: Uuid,
    markdown: &str,
  ) -> Result<(), String> {
    self.authed_patch_json(
      &format!("/api/workspaces/{workspace_id}/documents/{document_id}/markdown"),
      &json!({ "mode": "replace_all", "markdown": markdown }),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  /// Create a new page in a cloud workspace (`POST /api/workspaces/{ws}/documents`).
  pub async fn create_document(
    &self,
    workspace_id: Uuid,
    name: &str,
    parent_view_id: Option<Uuid>,
  ) -> Result<View, String> {
    let mut payload = json!({ "name": name });
    if let Some(pid) = parent_view_id {
      payload["parent_view_id"] = json!(pid);
    }
    let body = self.authed_post_json(
      &format!("/api/workspaces/{workspace_id}/documents"),
      &payload,
      API_TIMEOUT_SECS,
    ).await?;
    let val: Value = serde_json::from_str(body.trim())
      .map_err(|e| format!("invalid create document response: {e}"))?;
    let view_val = val.get("view").cloned().unwrap_or(val);
    serde_json::from_value(view_val).map_err(|e| format!("invalid view payload: {e}"))
  }

  /// Create a new folder in a cloud workspace (`POST /api/workspaces/{ws}/folders`).
  pub async fn create_folder(
    &self,
    workspace_id: Uuid,
    name: &str,
    parent_view_id: Option<Uuid>,
  ) -> Result<View, String> {
    let mut payload = json!({ "name": name });
    if let Some(pid) = parent_view_id {
      payload["parent_view_id"] = json!(pid);
    }
    let body = self.authed_post_json(
      &format!("/api/workspaces/{workspace_id}/folders"),
      &payload,
      API_TIMEOUT_SECS,
    ).await?;
    let val: Value = serde_json::from_str(body.trim())
      .map_err(|e| format!("invalid create folder response: {e}"))?;
    let view_val = val.get("view").cloned().unwrap_or(val);
    serde_json::from_value(view_val).map_err(|e| format!("invalid view payload: {e}"))
  }

  /// Rename or update the icon of a cloud view (`PATCH /api/workspaces/{ws}/views/{view}`).
  pub async fn update_view(
    &self,
    workspace_id: Uuid,
    view_id: Uuid,
    name: &str,
    icon: Option<&str>,
  ) -> Result<View, String> {
    let mut payload = json!({ "name": name });
    if let Some(ic) = icon {
      payload["icon"] = json!(ic);
    }
    let body = self.authed_patch_json(
      &format!("/api/workspaces/{workspace_id}/views/{view_id}"),
      &payload,
      API_TIMEOUT_SECS,
    ).await?;
    let val: Value = serde_json::from_str(body.trim())
      .map_err(|e| format!("invalid update view response: {e}"))?;
    let view_val = val.get("view").cloned().unwrap_or(val);
    serde_json::from_value(view_val).map_err(|e| format!("invalid view payload: {e}"))
  }

  /// Move a cloud view (and its subtree) into the recycle bin (`DELETE /api/workspaces/{ws}/views/{view}`).
  pub async fn delete_view(&self, workspace_id: Uuid, view_id: Uuid) -> Result<(), String> {
    self.authed_delete(
      &format!("/api/workspaces/{workspace_id}/views/{view_id}"),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  /// Restore a trashed cloud view (`POST /api/workspaces/{ws}/views/{view}/restore`).
  pub async fn restore_view(&self, workspace_id: Uuid, view_id: Uuid) -> Result<(), String> {
    self.authed_post_json(
      &format!("/api/workspaces/{workspace_id}/views/{view_id}/restore"),
      &json!({}),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  /// Permanently purge a trashed cloud view (`DELETE /api/workspaces/{ws}/trash/{view}`).
  pub async fn purge_view(&self, workspace_id: Uuid, view_id: Uuid) -> Result<(), String> {
    self.authed_delete(
      &format!("/api/workspaces/{workspace_id}/trash/{view_id}"),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  /// Reorder / reparent views in a cloud workspace (`POST /api/workspaces/{ws}/views/reorder`).
  pub async fn reorder_views(
    &self,
    workspace_id: Uuid,
    parent_view_id: Option<Uuid>,
    ordered_view_ids: &[Uuid],
  ) -> Result<(), String> {
    self.authed_post_json(
      &format!("/api/workspaces/{workspace_id}/views/reorder"),
      &json!({
        "parent_view_id": parent_view_id,
        "ordered_view_ids": ordered_view_ids,
      }),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  /// Register a new account via `POST /api/auth/register`.
  pub async fn register(&self, email: &str, display_name: &str, password: &str) -> Result<CloudUser, String> {
    let base = self.require_server()?;
    let (email_t, display_t) = (email.trim(), display_name.trim());
    if email_t.is_empty() || password.is_empty() {
      return Err("email and password are required".to_string());
    }

    let payload = json!({
      "email": email_t,
      "display_name": if display_t.is_empty() { email_t } else { display_t },
      "password": password,
    });
    let body = send_http(
      "POST",
      &format!("{base}/api/auth/register"),
      None,
      Some(&payload),
      LOGIN_TIMEOUT_SECS,
    ).await
    .map_err(|e| format!("registration failed: {e}"))?;
    Ok(parse_register_user(&body, email_t, display_t))
  }

  /// Request a password-reset email (`POST /api/auth/password/forgot`, falling
  /// back to `POST /api/auth/forgot-password` on 404).
  pub async fn forgot_password(&self, email: &str) -> Result<(), String> {
    let base = self.require_server()?;
    let email_t = email.trim();
    if email_t.is_empty() {
      return Err("email is required".to_string());
    }

    let payload = json!({ "email": email_t });
    match send_http(
      "POST",
      &format!("{base}/api/auth/password/forgot"),
      None,
      Some(&payload),
      LOGIN_TIMEOUT_SECS,
    ).await {
      Ok(_) => Ok(()),
      Err(err) if err.contains("404") => {
        send_http(
          "POST",
          &format!("{base}/api/auth/forgot-password"),
          None,
          Some(&payload),
          LOGIN_TIMEOUT_SECS,
        ).await
        .map_err(|e| format!("forgot password request failed: {e}"))?;
        Ok(())
      }
      Err(e) => Err(format!("forgot password request failed: {e}")),
    }
  }

  // ── Document Sharing ────────────────────────────────────────────────────

  /// Query current public share state for a cloud document.
  pub async fn get_share(&self, workspace_id: Uuid, document_id: Uuid) -> Result<ShareState, String> {
    let body = self.authed_get(
      &format!("/api/workspaces/{workspace_id}/documents/{document_id}/share"),
      API_TIMEOUT_SECS,
    ).await?;
    parse_share_state(&body, self.endpoints.base_url())
  }

  /// Create or retrieve the active public share link for a cloud document.
  pub async fn create_share(&self, workspace_id: Uuid, document_id: Uuid) -> Result<ShareState, String> {
    let body = self.authed_post_json(
      &format!("/api/workspaces/{workspace_id}/documents/{document_id}/share"),
      &json!({}),
      API_TIMEOUT_SECS,
    ).await?;
    parse_share_state(&body, self.endpoints.base_url())
  }

  /// Revoke the active public share link for a cloud document.
  pub async fn delete_share(&self, workspace_id: Uuid, document_id: Uuid) -> Result<ShareState, String> {
    let body = self.authed_delete(
      &format!("/api/workspaces/{workspace_id}/documents/{document_id}/share"),
      API_TIMEOUT_SECS,
    ).await?;
    if body.trim().is_empty() {
      return Ok(ShareState::default());
    }
    parse_share_state(&body, self.endpoints.base_url())
  }

  // ── Document Comments ───────────────────────────────────────────────────

  /// List comment threads for a cloud document.
  pub async fn list_comments(&self, workspace_id: Uuid, document_id: Uuid) -> Result<Vec<CommentThread>, String> {
    let body = self.authed_get(
      &format!("/api/workspaces/{workspace_id}/documents/{document_id}/comments"),
      API_TIMEOUT_SECS,
    ).await?;
    parse_comment_threads(&body)
  }

  /// Create a new anchored comment thread on a cloud document.
  #[allow(clippy::too_many_arguments)]
  pub async fn create_comment(
    &self,
    workspace_id: Uuid,
    document_id: Uuid,
    start_block: &str,
    start_offset: u32,
    end_block: &str,
    end_offset: u32,
    quote: &str,
    body: &str,
  ) -> Result<CommentThread, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
      return Err("comment body cannot be empty".to_string());
    }
    let payload = json!({
      "start_block": start_block, "start_offset": start_offset,
      "end_block": end_block, "end_offset": end_offset,
      "quote": quote, "body": trimmed,
    });
    let resp = self.authed_post_json(
      &format!("/api/workspaces/{workspace_id}/documents/{document_id}/comments"),
      &payload,
      API_TIMEOUT_SECS,
    ).await?;
    let val: Value = serde_json::from_str(&resp).map_err(|e| format!("invalid create comment response: {e}"))?;
    parse_comment_thread(val.get("thread").unwrap_or(&val))
  }

  /// Reply to an existing comment thread.
  pub async fn reply_comment(
    &self,
    workspace_id: Uuid,
    document_id: Uuid,
    thread_id: Uuid,
    body: &str,
  ) -> Result<CommentItem, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
      return Err("reply body cannot be empty".to_string());
    }
    let resp = self.authed_post_json(
      &format!("/api/workspaces/{workspace_id}/documents/{document_id}/comments/{thread_id}/reply"),
      &json!({ "body": trimmed }),
      API_TIMEOUT_SECS,
    ).await?;
    let val: Value = serde_json::from_str(&resp).map_err(|e| format!("invalid comment reply response: {e}"))?;
    Ok(parse_comment_item(val.get("comment").unwrap_or(&val)))
  }

  /// Mark a comment thread resolved or reopened.
  pub async fn resolve_comment(
    &self,
    workspace_id: Uuid,
    document_id: Uuid,
    thread_id: Uuid,
    resolved: bool,
  ) -> Result<(), String> {
    self.authed_post_json(
      &format!("/api/workspaces/{workspace_id}/documents/{document_id}/comments/{thread_id}/resolve"),
      &json!({ "resolved": resolved }),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  /// Delete a comment thread.
  pub async fn delete_comment(&self, workspace_id: Uuid, document_id: Uuid, thread_id: Uuid) -> Result<(), String> {
    self.authed_delete(
      &format!("/api/workspaces/{workspace_id}/documents/{document_id}/comments/{thread_id}"),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  // ── Personal Access Tokens ──────────────────────────────────────────────

  /// List Personal Access Tokens for the signed-in user.
  pub async fn list_tokens(&self) -> Result<Vec<ApiTokenInfo>, String> {
    parse_token_list(&self.authed_get("/api/auth/tokens", API_TIMEOUT_SECS).await?)
  }

  /// Mint a new Personal Access Token (`POST /api/auth/tokens`).
  pub async fn create_token(
    &self,
    name: &str,
    can_write: bool,
    expires_in_days: Option<i64>,
  ) -> Result<CreatedApiToken, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
      return Err("token name cannot be empty".to_string());
    }
    let scopes = if can_write { vec!["read", "write"] } else { vec!["read"] };
    let mut payload = json!({ "name": trimmed, "scopes": scopes });
    if let Some(days) = expires_in_days {
      payload["expires_in_days"] = json!(days);
    }
    parse_created_token(&self.authed_post_json("/api/auth/tokens", &payload, API_TIMEOUT_SECS).await?)
  }

  /// Revoke an existing Personal Access Token (`DELETE /api/auth/tokens/{id}`).
  pub async fn revoke_token(&self, token_id: &str) -> Result<(), String> {
    let trimmed = token_id.trim();
    if trimmed.is_empty() {
      return Err("token id cannot be empty".to_string());
    }
    self.authed_delete(&format!("/api/auth/tokens/{trimmed}"), API_TIMEOUT_SECS).await?;
    Ok(())
  }

  // ── AI Settings & Completion ────────────────────────────────────────────

  /// List configured AI providers from `GET /api/ai/settings`.
  pub async fn list_ai_providers(&self) -> Result<Vec<AiProviderConfig>, String> {
    parse_ai_providers(&self.authed_get("/api/ai/settings", API_TIMEOUT_SECS).await?)
  }

  /// Save or activate an AI provider via `PATCH /api/ai/settings`.
  pub async fn save_ai_provider(
    &self,
    config: &AiProviderConfig,
    api_key: Option<&str>,
  ) -> Result<Vec<AiProviderConfig>, String> {
    let mut payload = json!({
      "provider_id": config.id,
      "provider": config.protocol,
      "base_url": config.base_url,
      "model": config.model,
    });
    if let Some(key) = api_key {
      payload["api_key"] = json!(key);
    }
    parse_ai_providers(&self.authed_patch_json("/api/ai/settings", &payload, API_TIMEOUT_SECS).await?)
  }

  /// Delete a configured AI provider via `DELETE /api/ai/providers/{id}`.
  pub async fn delete_ai_provider(&self, provider_id: &str) -> Result<Vec<AiProviderConfig>, String> {
    let trimmed = provider_id.trim();
    if trimmed.is_empty() {
      return Err("provider_id cannot be empty".to_string());
    }
    parse_ai_providers(&self.authed_delete(&format!("/api/ai/providers/{trimmed}"), API_TIMEOUT_SECS).await?)
  }

  /// Fetch live available models from an AI provider via `POST /api/ai/models`.
  pub async fn fetch_ai_models(
    &self,
    provider_id: Option<&str>,
    base_url: &str,
    protocol: &str,
    api_key: Option<&str>,
  ) -> Result<Vec<String>, String> {
    let mut payload = json!({ "base_url": base_url.trim(), "provider": protocol.trim() });
    if let Some(pid) = provider_id.map(str::trim).filter(|s| !s.is_empty()) {
      payload["provider_id"] = json!(pid);
    }
    if let Some(key) = api_key.map(str::trim).filter(|s| !s.is_empty()) {
      payload["api_key"] = json!(key);
    }
    let body = self.authed_post_json("/api/ai/models", &payload, API_TIMEOUT_SECS).await?;
    let val: Value = serde_json::from_str(&body).map_err(|e| format!("invalid AI models response: {e}"))?;
    Ok(
      val
        .get("models")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default(),
    )
  }

  /// Execute an AI writing prompt against the cloud server (`POST /api/ai/complete`),
  /// cleanly returning an error when offline, unauthenticated, or unconfigured.
  pub async fn stream_ai_prompt(&self, system: &str, prompt: &str) -> Result<String, String> {
    let prompt_t = prompt.trim();
    if prompt_t.is_empty() {
      return Err("prompt cannot be empty".to_string());
    }
    if self.endpoints.base_url().is_empty() || !self.is_authenticated() {
      return Err("AI assistant requires a signed-in cloud connection".to_string());
    }
    let mut payload = json!({ "prompt": prompt_t });
    if !system.trim().is_empty() {
      payload["system"] = json!(system.trim());
    }
    let body = self
      .authed_post_json("/api/ai/complete", &payload, AI_TIMEOUT_SECS)
      .await
      .map_err(|e| format!("AI request unavailable: {e}"))?;
    let val: Value = serde_json::from_str(&body).map_err(|e| format!("invalid AI completion response: {e}"))?;
    let text = val
      .get("text")
      .and_then(Value::as_str)
      .ok_or_else(|| "AI completion response missing text".to_string())?;
    if text.trim().is_empty() {
      return Err("AI returned an empty response".to_string());
    }
    Ok(text.to_string())
  }

  // ── Usage Statistics ─────────────────────────────────────────────────────

  /// Fetch global & per-workspace usage statistics (`GET /api/stats/overview`).
  pub async fn get_usage_overview(&self) -> Result<mica_core::UsageStatsOverview, String> {
    let body = self.authed_get("/api/stats/overview", API_TIMEOUT_SECS).await?;
    serde_json::from_str(&body)
      .map_err(|e| format!("invalid usage stats overview response: {e}"))
  }

  /// Fetch writing activity trend & 180-day heatmap (`GET /api/stats/activity`).
  pub async fn get_usage_activity(
    &self,
    range: &str,
    tz_offset_minutes: i32,
    workspace_id: Option<&str>,
    include_imports: bool,
  ) -> Result<mica_core::UsageStatsActivity, String> {
    let mut path = format!(
      "/api/stats/activity?range={range}&tz_offset={tz_offset_minutes}&include_imports={include_imports}"
    );
    if let Some(ws_id) = workspace_id.filter(|s| !s.trim().is_empty()) {
      path.push_str("&workspace_id=");
      path.push_str(ws_id.trim());
    }
    let body = self.authed_get(&path, API_TIMEOUT_SECS).await?;
    serde_json::from_str(&body)
      .map_err(|e| format!("invalid usage stats activity response: {e}"))
  }

  // ── Workspace Members & Invitations ─────────────────────────────────────

  /// List members of `workspace_id` (`GET /api/workspaces/{ws}/members`).
  pub async fn list_members(&self, workspace_id: Uuid) -> Result<Vec<WorkspaceMemberInfo>, String> {
    parse_member_list(&self.authed_get(&format!("/api/workspaces/{workspace_id}/members"), API_TIMEOUT_SECS).await?)
  }

  /// Invite a user by email into `workspace_id` (`POST /api/workspaces/{ws}/members`).
  pub async fn invite_member(&self, workspace_id: Uuid, email: &str, role: &str) -> Result<WorkspaceInvitationInfo, String> {
    let email_t = email.trim();
    if email_t.is_empty() {
      return Err("email is required".to_string());
    }
    let role_t = if role.trim().is_empty() { "editor" } else { role.trim() };
    let body = self.authed_post_json(
      &format!("/api/workspaces/{workspace_id}/members"),
      &json!({ "email": email_t, "role": role_t }),
      API_TIMEOUT_SECS,
    ).await?;
    let val: Value = serde_json::from_str(&body).map_err(|e| format!("invalid invite response: {e}"))?;
    Ok(parse_invitation(val.get("invitation").unwrap_or(&val)))
  }

  /// List pending invitations sent for `workspace_id` (`GET /api/workspaces/{ws}/invitations`).
  pub async fn list_invitations(&self, workspace_id: Uuid) -> Result<Vec<WorkspaceInvitationInfo>, String> {
    parse_invitation_list(
      &self.authed_get(&format!("/api/workspaces/{workspace_id}/invitations"), API_TIMEOUT_SECS).await?,
    )
  }

  /// Fetch workspace storage usage `(bytes_used, quota_bytes)` (`GET /api/workspaces/{ws}/usage`).
  pub async fn workspace_usage(&self, workspace_id: Uuid) -> Result<(u64, u64), String> {
    let body = self.authed_get(&format!("/api/workspaces/{workspace_id}/usage"), API_TIMEOUT_SECS).await?;
    let val: Value = serde_json::from_str(body.trim())
      .map_err(|e| format!("invalid usage response: {e}"))?;
    let used = val.get("bytes_used").and_then(Value::as_u64).unwrap_or(0);
    let quota = val.get("quota_bytes").and_then(Value::as_u64).unwrap_or(0);
    Ok((used, quota))
  }

  /// Revoke a pending workspace invitation (`DELETE /api/workspaces/{ws}/invitations/{id}`).
  pub async fn revoke_invitation(&self, workspace_id: Uuid, invitation_id: &str) -> Result<(), String> {
    self.authed_delete(
      &format!("/api/workspaces/{workspace_id}/invitations/{invitation_id}"),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  /// Change a workspace member's role (`PATCH /api/workspaces/{ws}/members/{user_id}`).
  pub async fn update_member_role(&self, workspace_id: Uuid, user_id: &str, role: &str) -> Result<(), String> {
    self.authed_patch_json(
      &format!("/api/workspaces/{workspace_id}/members/{user_id}"),
      &json!({ "role": role }),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  /// Remove a member from a workspace (`DELETE /api/workspaces/{ws}/members/{user_id}`).
  pub async fn remove_member(&self, workspace_id: Uuid, user_id: &str) -> Result<(), String> {
    self.authed_delete(
      &format!("/api/workspaces/{workspace_id}/members/{user_id}"),
      API_TIMEOUT_SECS,
    ).await?;
    Ok(())
  }

  /// List pending invitations addressed to the signed-in user (`GET /api/invitations`).
  pub async fn list_received_invitations(&self) -> Result<Vec<WorkspaceInvitationInfo>, String> {
    parse_invitation_list(&self.authed_get("/api/invitations", API_TIMEOUT_SECS).await?)
  }

  // ── Knowledge Graph ─────────────────────────────────────────────────────

  /// Fetch the page-link graph for `workspace_id` (`GET /api/workspaces/{ws}/graph`).
  pub async fn fetch_graph(&self, workspace_id: Uuid) -> Result<CloudGraphPayload, String> {
    parse_graph_payload(&self.authed_get(&format!("/api/workspaces/{workspace_id}/graph"), API_TIMEOUT_SECS).await?)
  }

  // ── Session & SyncEngine Lifecycle ──────────────────────────────────────

  /// Clear stored credentials and advance [`ClaimTracker`] back to [`WorldOrigin::Local`].
  pub fn logout(&mut self) {
    self.config.access_token = None;
    self.user_id = None;
    self.refresh_token = None;
    self.config.email = None;
    self.phase = SyncPhase::Offline;
    self.claims.switch_account(WorldOrigin::Local);
  }

  /// Update the target server URL. Clears auth if the host changed so tokens
  /// never leak across servers.
  pub fn set_server_url(&mut self, server_url: impl Into<String>) {
    let next_endpoints = ApiEndpoints::new(server_url.into().trim());
    let changed = next_endpoints.base_url() != self.endpoints.base_url();
    self.endpoints = next_endpoints;
    self.config.server_url = self.endpoints.base_url().to_string();
    if changed && (self.is_authenticated() || self.refresh_token.is_some()) {
      self.logout();
    }
  }

  /// Current derived sync status label for UI chrome.
  pub fn sync_phase_label(&self) -> String {
    if !self.is_authenticated() {
      return if cfg!(target_arch = "wasm32") {
        "Offline".to_string()
      } else {
        "Local Only".to_string()
      };
    }
    match self.phase {
      SyncPhase::Synced => "Synced".to_string(),
      SyncPhase::Syncing => "Syncing".to_string(),
      SyncPhase::Offline => "Offline".to_string(),
    }
  }

  /// Current [`SyncPhase`] tracked by this connector.
  pub fn sync_phase(&self) -> SyncPhase {
    self.phase
  }

  /// Update the connector's tracked [`SyncPhase`].
  pub fn set_sync_phase(&mut self, phase: SyncPhase) {
    self.phase = phase;
  }

  /// Construct a Sans-I/O [`SyncEngine`] for `doc_id` backed by [`MemoryMirrorStorage`].
  pub fn create_doc_sync(&self, doc_id: &str, client_id: u64, self_name: &str) -> SyncEngine {
    self.create_doc_sync_with_storage(doc_id, client_id, self_name, Box::new(MemoryMirrorStorage::new()))
  }

  /// Construct a Sans-I/O [`SyncEngine`] for `doc_id` with a caller-supplied [`SyncStorage`].
  pub fn create_doc_sync_with_storage(
    &self,
    doc_id: &str,
    client_id: u64,
    self_name: &str,
    storage: Box<dyn SyncStorage>,
  ) -> SyncEngine {
    let name_opt = if self_name.trim().is_empty() {
      self.config.email.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
    } else {
      Some(self_name.trim().to_string())
    };
    SyncEngine::new(
      SyncConfig {
        client_id: Some(client_id),
        self_name: name_opt,
        document_id: Some(doc_id.to_string()),
        restore_unacked: Vec::new(),
      },
      Some(storage),
    )
  }

  /// Feed a local CRDT update into a Sans-I/O [`SyncEngine`] and return [`SyncEffect`]s.
  pub fn push_local_update(engine: &mut SyncEngine, update: Vec<u8>) -> Vec<SyncEffect> {
    engine.handle_event(SyncEvent::ApplyLocalDiff(update))
  }

  /// Normalized server base URL (without trailing slash).
  pub fn server_url(&self) -> &str {
    self.endpoints.base_url()
  }

  /// Whether a non-empty access token is currently held.
  pub fn is_authenticated(&self) -> bool {
    self.config.access_token.as_deref().is_some_and(|t| !t.trim().is_empty())
  }

  /// Human-readable account badge for the UI header/settings panel.
  pub fn account_label(&self) -> String {
    if !self.is_authenticated() {
      return if cfg!(target_arch = "wasm32") {
        "Not signed in".to_string()
      } else {
        "Local Vault".to_string()
      };
    }
    self
      .config
      .email
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .map(String::from)
      .unwrap_or_else(|| "Signed in".to_string())
  }

  /// Borrow the active [`ClaimTracker`] generation guard.
  pub fn claims(&self) -> &ClaimTracker {
    &self.claims
  }

  /// Borrow the underlying [`CloudConfig`].
  pub fn config(&self) -> &CloudConfig {
    &self.config
  }

  /// Borrow the single-source [`ApiEndpoints`] builder.
  pub fn endpoints(&self) -> &ApiEndpoints {
    &self.endpoints
  }

  fn require_server(&self) -> Result<&str, String> {
    let base = self.endpoints.base_url();
    if base.is_empty() {
      return Err("server URL is empty".to_string());
    }
    Ok(base)
  }

  fn require_auth(&self) -> Result<(&str, &str), String> {
    let base = self.require_server()?;
    let token = self
      .config
      .access_token
      .as_deref()
      .map(str::trim)
      .filter(|t| !t.is_empty())
      .ok_or_else(|| "authentication required".to_string())?;
    Ok((base, token))
  }

  async fn authed_get(&self, path: &str, timeout_secs: u64) -> Result<String, String> {
    let (base, token) = self.require_auth()?;
    send_http("GET", &format!("{base}{path}"), Some(token), None, timeout_secs)
      .await
      .map_err(|e| format!("GET {path} failed: {e}"))
  }

  async fn authed_post_json(&self, path: &str, payload: &Value, timeout_secs: u64) -> Result<String, String> {
    let (base, token) = self.require_auth()?;
    send_http("POST", &format!("{base}{path}"), Some(token), Some(payload), timeout_secs)
      .await
      .map_err(|e| format!("POST {path} failed: {e}"))
  }

  async fn authed_patch_json(&self, path: &str, payload: &Value, timeout_secs: u64) -> Result<String, String> {
    let (base, token) = self.require_auth()?;
    send_http("PATCH", &format!("{base}{path}"), Some(token), Some(payload), timeout_secs)
      .await
      .map_err(|e| format!("PATCH {path} failed: {e}"))
  }

  async fn authed_delete(&self, path: &str, timeout_secs: u64) -> Result<String, String> {
    let (base, token) = self.require_auth()?;
    send_http("DELETE", &format!("{base}{path}"), Some(token), None, timeout_secs)
      .await
      .map_err(|e| format!("DELETE {path} failed: {e}"))
  }
}

#[cfg(not(target_arch = "wasm32"))]
fn http_agent(timeout_secs: u64) -> ureq::Agent {
  ureq::Agent::config_builder()
    .timeout_global(Some(Duration::from_secs(timeout_secs)))
    .build()
    .into()
}

#[cfg(not(target_arch = "wasm32"))]
async fn send_http(
  method: &str,
  url: &str,
  bearer_token: Option<&str>,
  json_body: Option<&Value>,
  timeout_secs: u64,
) -> Result<String, String> {
  let method_owned = method.to_string();
  let url_owned = url.to_string();
  let token_owned = bearer_token.map(str::to_string);
  let body_owned = json_body.cloned();
  run_blocking(move || {
    send_http_blocking(
      &method_owned,
      &url_owned,
      token_owned.as_deref(),
      body_owned.as_ref(),
      timeout_secs,
    )
  })
  .await
}

#[cfg(not(target_arch = "wasm32"))]
fn send_http_blocking(
  method: &str,
  url: &str,
  bearer_token: Option<&str>,
  json_body: Option<&Value>,
  timeout_secs: u64,
) -> Result<String, String> {
  let agent = http_agent(timeout_secs);
  let auth_header = bearer_token
    .map(str::trim)
    .filter(|t| !t.is_empty())
    .map(|t| format!("Bearer {t}"));
  let mut resp = match method {
    "GET" => {
      let mut req = agent.get(url);
      if let Some(ref h) = auth_header {
        req = req.header("Authorization", h);
      }
      req.call().map_err(|e| e.to_string())?
    }
    "DELETE" => {
      let mut req = agent.delete(url);
      if let Some(ref h) = auth_header {
        req = req.header("Authorization", h);
      }
      req.call().map_err(|e| e.to_string())?
    }
    "POST" => {
      let mut req = agent.post(url);
      if let Some(ref h) = auth_header {
        req = req.header("Authorization", h);
      }
      match json_body {
        Some(payload) => req.send_json(payload).map_err(|e| e.to_string())?,
        None => req.send_empty().map_err(|e| e.to_string())?,
      }
    }
    "PATCH" => {
      let mut req = agent.patch(url);
      if let Some(ref h) = auth_header {
        req = req.header("Authorization", h);
      }
      match json_body {
        Some(payload) => req.send_json(payload).map_err(|e| e.to_string())?,
        None => req.send_empty().map_err(|e| e.to_string())?,
      }
    }
    other => return Err(format!("unsupported HTTP method {other}")),
  };
  Ok(resp.body_mut().read_to_string().unwrap_or_default())
}

#[cfg(target_arch = "wasm32")]
async fn send_http(
  method: &str,
  url: &str,
  bearer_token: Option<&str>,
  json_body: Option<&Value>,
  timeout_secs: u64,
) -> Result<String, String> {
  let serialized = match json_body {
    Some(payload) => Some(
      serde_json::to_string(payload).map_err(|e| format!("failed to serialize request body: {e}"))?,
    ),
    None => None,
  };
  crate::web_runtime::fetch_request(
    method,
    url,
    bearer_token,
    serialized.as_deref(),
    timeout_secs,
  )
  .await
}

fn j_str(v: &Value, key: &str) -> String {
  v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn j_opt_str(v: &Value, key: &str) -> Option<String> {
  v.get(key).and_then(Value::as_str).map(String::from)
}

fn format_health_summary(raw_body: &str) -> String {
  let trimmed = raw_body.trim();
  if let Ok(val) = serde_json::from_str::<Value>(trimmed)
    && let Some(status) = val.get("status").and_then(Value::as_str)
  {
    return match val.get("version").and_then(Value::as_str) {
      Some(version) => format!("{status} (v{version})"),
      None => status.to_string(),
    };
  }
  if trimmed.is_empty() { "ok".to_string() } else { trimmed.to_string() }
}

fn parse_probe_meta(raw_body: &str) -> ServerProbeMeta {
  let Ok(val) = serde_json::from_str::<Value>(raw_body.trim()) else {
    return ServerProbeMeta { reachable: true, version: None, registration_open: None };
  };
  ServerProbeMeta {
    reachable: true,
    version: j_opt_str(&val, "version"),
    registration_open: val.get("registration_open").and_then(Value::as_bool),
  }
}

fn parse_register_user(raw_body: &str, email: &str, display_name: &str) -> CloudUser {
  let fallback = if display_name.is_empty() { email } else { display_name };
  let Ok(val) = serde_json::from_str::<Value>(raw_body.trim()) else {
    return CloudUser { id: None, email: email.into(), display_name: fallback.into(), verified: false };
  };
  if let Some(u) = val.get("user") {
    return CloudUser {
      id: j_opt_str(u, "id"),
      email: j_opt_str(u, "email").unwrap_or_else(|| email.into()),
      display_name: j_opt_str(u, "display_name").unwrap_or_else(|| fallback.into()),
      verified: val.get("verified").and_then(Value::as_bool).unwrap_or(true),
    };
  }
  CloudUser {
    id: j_opt_str(&val, "id"),
    email: j_opt_str(&val, "email").unwrap_or_else(|| email.into()),
    display_name: j_opt_str(&val, "display_name").unwrap_or_else(|| fallback.into()),
    verified: val.get("verified").and_then(Value::as_bool).unwrap_or(false),
  }
}

fn parse_share_state(raw_body: &str, base_url: &str) -> Result<ShareState, String> {
  let val: Value = serde_json::from_str(raw_body.trim()).map_err(|e| format!("invalid share response: {e}"))?;
  let shared = val.get("shared").and_then(Value::as_bool).unwrap_or(false);
  let token = j_opt_str(&val, "token").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
  let url = j_opt_str(&val, "url")
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .or_else(|| {
      shared
        .then(|| token.as_deref().map(|t| format!("{}/s/{t}", base_url.trim_end_matches('/'))))
        .flatten()
    });
  Ok(ShareState { shared, token, url })
}

fn parse_comment_item(val: &Value) -> CommentItem {
  CommentItem {
    id: j_str(val, "id"),
    author_id: j_str(val, "author_id"),
    author_name: j_str(val, "author_name"),
    body: j_str(val, "body"),
    created_at: j_str(val, "created_at"),
    edited_at: j_opt_str(val, "edited_at"),
  }
}

fn parse_comment_thread(val: &Value) -> Result<CommentThread, String> {
  let status = j_opt_str(val, "status").unwrap_or_else(|| "open".into());
  let resolved = val
    .get("resolved")
    .and_then(Value::as_bool)
    .unwrap_or_else(|| status.eq_ignore_ascii_case("resolved"));
  let anchor = val.get("anchor");
  let a_str = |k: &str| anchor.and_then(|a| a.get(k)).or_else(|| val.get(k)).and_then(Value::as_str).map(String::from);
  let a_u32 = |k: &str| anchor.and_then(|a| a.get(k)).or_else(|| val.get(k)).and_then(Value::as_u64).map(|n| n as u32);
  let comments = val
    .get("comments")
    .and_then(Value::as_array)
    .map(|arr| arr.iter().map(parse_comment_item).collect())
    .unwrap_or_default();

  Ok(CommentThread {
    id: j_str(val, "id"),
    status,
    quote: j_str(val, "quote"),
    created_by: j_str(val, "created_by"),
    created_at: j_str(val, "created_at"),
    resolved,
    resolved_by: j_opt_str(val, "resolved_by"),
    resolved_at: j_opt_str(val, "resolved_at"),
    start_block: a_str("start_block"),
    start_offset: a_u32("start_offset"),
    end_block: a_str("end_block"),
    end_offset: a_u32("end_offset"),
    comments,
  })
}

fn parse_comment_threads(raw_body: &str) -> Result<Vec<CommentThread>, String> {
  let val: Value = serde_json::from_str(raw_body.trim()).map_err(|e| format!("invalid comments response: {e}"))?;
  let arr = val
    .get("threads")
    .and_then(Value::as_array)
    .or_else(|| val.as_array())
    .ok_or_else(|| "comments response missing threads array".to_string())?;
  arr.iter().map(parse_comment_thread).collect()
}

fn parse_token_info(val: &Value) -> ApiTokenInfo {
  let can_write = val.get("can_write").and_then(Value::as_bool).unwrap_or_else(|| {
    val
      .get("scopes")
      .and_then(Value::as_array)
      .is_some_and(|arr| arr.iter().any(|s| s.as_str() == Some("write")))
  });
  ApiTokenInfo {
    id: j_str(val, "id"),
    name: j_str(val, "name"),
    can_write,
    created_at: j_str(val, "created_at"),
    expires_at: j_opt_str(val, "expires_at"),
    last_used_at: j_opt_str(val, "last_used_at"),
  }
}

fn parse_token_list(raw_body: &str) -> Result<Vec<ApiTokenInfo>, String> {
  let val: Value = serde_json::from_str(raw_body.trim()).map_err(|e| format!("invalid token list: {e}"))?;
  let arr = val
    .get("tokens")
    .and_then(Value::as_array)
    .or_else(|| val.as_array())
    .ok_or_else(|| "token list response missing tokens array".to_string())?;
  Ok(arr.iter().map(parse_token_info).collect())
}

fn parse_created_token(raw_body: &str) -> Result<CreatedApiToken, String> {
  let val: Value = serde_json::from_str(raw_body.trim()).map_err(|e| format!("invalid created token: {e}"))?;
  Ok(CreatedApiToken { info: parse_token_info(&val), token: j_str(&val, "token") })
}

fn parse_ai_providers(raw_body: &str) -> Result<Vec<AiProviderConfig>, String> {
  let val: Value = serde_json::from_str(raw_body.trim()).map_err(|e| format!("invalid AI settings response: {e}"))?;
  let active_id = j_str(&val, "provider_id");
  let active_hint = j_str(&val, "key_hint");

  if let Some(arr) = val.get("configured_providers").and_then(Value::as_array).filter(|a| !a.is_empty()) {
    return Ok(
      arr
        .iter()
        .map(|item| {
          let id = j_opt_str(item, "provider_id").or_else(|| j_opt_str(item, "id")).unwrap_or_default();
          let name = j_opt_str(item, "name").filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone());
          let active = item
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| !active_id.is_empty() && id == active_id);
          let key_tail = j_opt_str(item, "key_tail")
            .or_else(|| j_opt_str(item, "key_hint"))
            .unwrap_or_else(|| if active { active_hint.clone() } else { String::new() });
          AiProviderConfig {
            id,
            name,
            base_url: j_str(item, "base_url"),
            protocol: j_opt_str(item, "protocol")
              .or_else(|| j_opt_str(item, "provider"))
              .unwrap_or_else(|| "openai".into()),
            has_key: item.get("has_key").and_then(Value::as_bool).unwrap_or(false),
            key_tail,
            model: j_str(item, "model"),
            active,
          }
        })
        .collect(),
    );
  }

  if val.get("configured").and_then(Value::as_bool).unwrap_or(false) && !active_id.is_empty() {
    return Ok(vec![AiProviderConfig {
      id: active_id.clone(),
      name: active_id,
      base_url: j_str(&val, "base_url"),
      protocol: j_opt_str(&val, "provider").unwrap_or_else(|| "openai".into()),
      has_key: val.get("has_key").and_then(Value::as_bool).unwrap_or(false),
      key_tail: active_hint,
      model: j_str(&val, "model"),
      active: true,
    }]);
  }
  Ok(Vec::new())
}

fn parse_member_list(raw_body: &str) -> Result<Vec<WorkspaceMemberInfo>, String> {
  let val: Value = serde_json::from_str(raw_body.trim()).map_err(|e| format!("invalid members response: {e}"))?;
  let arr = val
    .get("members")
    .and_then(Value::as_array)
    .or_else(|| val.as_array())
    .ok_or_else(|| "members response missing members array".to_string())?;
  Ok(
    arr
      .iter()
      .map(|m| WorkspaceMemberInfo {
        user_id: j_str(m, "user_id"),
        email: j_str(m, "email"),
        display_name: j_str(m, "display_name"),
        role: j_opt_str(m, "role").unwrap_or_else(|| "viewer".into()),
        joined_at: j_str(m, "joined_at"),
        avatar_version: j_opt_str(m, "avatar_version"),
      })
      .collect(),
  )
}

fn parse_invitation(val: &Value) -> WorkspaceInvitationInfo {
  WorkspaceInvitationInfo {
    id: j_str(val, "id"),
    workspace_id: j_str(val, "workspace_id"),
    workspace_name: j_opt_str(val, "workspace_name"),
    email: j_opt_str(val, "email"),
    role: j_opt_str(val, "role").unwrap_or_else(|| "viewer".into()),
    invited_by: j_opt_str(val, "invited_by_name").or_else(|| j_opt_str(val, "invited_by")),
    created_at: j_str(val, "created_at"),
  }
}

fn parse_invitation_list(raw_body: &str) -> Result<Vec<WorkspaceInvitationInfo>, String> {
  let val: Value = serde_json::from_str(raw_body.trim()).map_err(|e| format!("invalid invitations response: {e}"))?;
  let arr = val
    .get("invitations")
    .and_then(Value::as_array)
    .or_else(|| val.as_array())
    .ok_or_else(|| "invitations response missing invitations array".to_string())?;
  Ok(arr.iter().map(parse_invitation).collect())
}

fn parse_graph_payload(raw_body: &str) -> Result<CloudGraphPayload, String> {
  let val: Value = serde_json::from_str(raw_body.trim()).map_err(|e| format!("invalid graph response: {e}"))?;
  let nodes = val
    .get("nodes")
    .and_then(Value::as_array)
    .map(|arr| {
      arr
        .iter()
        .map(|n| CloudGraphNode {
          view_id: j_str(n, "view_id"),
          name: j_str(n, "name"),
          degree: n.get("degree").and_then(Value::as_i64).unwrap_or(0),
        })
        .collect()
    })
    .unwrap_or_default();
  let edges = val
    .get("edges")
    .and_then(Value::as_array)
    .map(|arr| {
      arr
        .iter()
        .map(|e| CloudGraphEdge { source: j_str(e, "source"), target: j_str(e, "target") })
        .collect()
    })
    .unwrap_or_default();
  Ok(CloudGraphPayload {
    nodes,
    edges,
    unlinked: val.get("unlinked").and_then(Value::as_i64).unwrap_or(0),
  })
}

/// Run a blocking HTTP / I/O closure on a dedicated OS thread without stalling
/// the UI event loop, waking the awaiting `nsh` task immediately on completion.
#[cfg(not(target_arch = "wasm32"))]
pub async fn run_blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
  use std::future::Future;
  use std::pin::Pin;
  use std::sync::{Arc, Mutex};
  use std::task::{Context, Poll, Waker};

  struct Slot<T> {
    result: Option<T>,
    waker: Option<Waker>,
  }

  struct BlockingFuture<T> {
    slot: Arc<Mutex<Slot<T>>>,
  }

  impl<T> Future for BlockingFuture<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
      let mut guard = self.slot.lock().unwrap_or_else(|e| e.into_inner());
      if let Some(value) = guard.result.take() {
        Poll::Ready(value)
      } else {
        guard.waker = Some(cx.waker().clone());
        Poll::Pending
      }
    }
  }

  let slot = Arc::new(Mutex::new(Slot { result: None, waker: None }));
  let worker_slot = Arc::clone(&slot);
  let _ = std::thread::Builder::new().name("mica-net".into()).spawn(move || {
    let value = f();
    let waker = {
      let mut guard = worker_slot.lock().unwrap_or_else(|e| e.into_inner());
      guard.result = Some(value);
      guard.waker.take()
    };
    if let Some(w) = waker {
      w.wake();
    }
  });

  BlockingFuture { slot }.await
}

/// WebAssembly fallback: cloud blocking calls already return immediate errors
/// on `wasm32`, so run inline without spawning a thread.
#[cfg(target_arch = "wasm32")]
pub async fn run_blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
  f()
}

#[cfg(test)]
mod tests {
  use super::*;
  #[cfg(not(target_arch = "wasm32"))]
  use std::{io::{Read, Write}, net::TcpListener, thread};

  #[cfg(not(target_arch = "wasm32"))]
  fn mock_conn(status: &'static str, body: &str, authed: bool) -> (CloudConnector, String, thread::JoinHandle<String>) {
    let body = body.to_string();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind localhost");
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
      let (mut stream, _) = listener.accept().unwrap();
      let (mut raw, mut buf, mut hdr_end, mut clen) = (Vec::new(), [0u8; 4096], None, 0usize);
      loop {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
          break;
        }
        raw.extend_from_slice(&buf[..n]);
        if hdr_end.is_none()
          && let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n")
        {
          hdr_end = Some(pos + 4);
          for line in String::from_utf8_lossy(&raw[..pos + 4]).lines() {
            if let Some((k, v)) = line.split_once(':')
              && k.eq_ignore_ascii_case("content-length")
            {
              clen = v.trim().parse().unwrap_or(0);
            }
          }
        }
        if let Some(end) = hdr_end
          && raw.len() >= end + clen
        {
          break;
        }
      }
      let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
      );
      let _ = stream.write_all(resp.as_bytes());
      let _ = stream.flush();
      String::from_utf8_lossy(&raw).to_string()
    });
    let url = format!("http://{addr}");
    let conn = CloudConnector::new(CloudConfig {
      server_url: url.clone(),
      access_token: authed.then(|| "jwt".to_string()),
      email: authed.then(|| "a@mica.dev".to_string()),
    });
    (conn, url, handle)
  }

  #[test]
  fn local_defaults_and_phase_labels() {
    let mut conn = CloudConnector::new(CloudConfig {
      server_url: "https://notes.example.com/".into(),
      access_token: None,
      email: None,
    });
    assert_eq!(conn.server_url(), "https://notes.example.com");
    assert!(!conn.is_authenticated());
    assert_eq!((conn.account_label().as_str(), conn.sync_phase_label().as_str()), ("Local Vault", "Local Only"));
    assert_eq!(conn.claims().capture().origin, WorldOrigin::Local);
    conn.set_sync_phase(SyncPhase::Synced);
    assert_eq!(conn.sync_phase_label(), "Local Only");
  }

  #[test]
  fn authenticated_config_and_logout_advance_claims() {
    let mut conn = CloudConnector::new(CloudConfig {
      server_url: "https://cloud.mica.test/".into(),
      access_token: Some("tok-123".into()),
      email: Some("dev@mica.test".into()),
    });
    assert!(conn.is_authenticated());
    assert_eq!((conn.account_label().as_str(), conn.sync_phase_label().as_str()), ("dev@mica.test", "Offline"));
    conn.set_sync_phase(SyncPhase::Syncing);
    assert_eq!(conn.sync_phase_label(), "Syncing");
    conn.set_sync_phase(SyncPhase::Synced);
    assert_eq!(conn.sync_phase_label(), "Synced");

    let cloud_claim = conn.claims().capture();
    assert_eq!(cloud_claim.origin, WorldOrigin::Cloud { server_url: "https://cloud.mica.test".into() });
    conn.logout();
    assert!(!conn.is_authenticated() && !conn.claims().matches_account(&cloud_claim));
    assert_eq!(conn.claims().capture().origin, WorldOrigin::Local);
  }

  #[test]
  fn create_doc_sync_and_push_local_update() {
    let conn = CloudConnector::new(CloudConfig::default());
    let mut engine = conn.create_doc_sync("doc-1", 42, "Alice");
    let doc = mica_core::MicaDoc::from_blocks(
      "root",
      &[
        mica_core::Block::new("root", "page").with_children(vec!["b1".into()]),
        mica_core::Block::new("b1", "paragraph").with_text("Hello sync"),
      ],
    );
    let effects = CloudConnector::push_local_update(&mut engine, doc.encode_state());
    assert!(!effects.is_empty() && !engine.outbox_empty());
  }

  #[cfg(not(target_arch = "wasm32"))]
  fn block_on_test<F: std::future::Future>(fut: F) -> F::Output {
    use std::sync::{Arc, Condvar, Mutex};
    use std::task::{Context, Poll, Wake, Waker};
    struct CondWaker {
      ready: Mutex<bool>,
      cv: Condvar,
    }
    impl Wake for CondWaker {
      fn wake(self: Arc<Self>) {
        *self.ready.lock().unwrap() = true;
        self.cv.notify_all();
      }
    }
    let mut fut = Box::pin(fut);
    let wake = Arc::new(CondWaker { ready: Mutex::new(false), cv: Condvar::new() });
    let waker = Waker::from(Arc::clone(&wake));
    let mut cx = Context::from_waker(&waker);
    loop {
      match fut.as_mut().poll(&mut cx) {
        Poll::Ready(out) => return out,
        Poll::Pending => {
          let mut g = wake.ready.lock().unwrap();
          while !*g {
            g = wake.cv.wait(g).unwrap();
          }
          *g = false;
        }
      }
    }
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn probe_register_forgot_share_comments_tokens_ai_members_graph_and_ai_prompt() {
    block_on_test(async {
    let (ws, doc, tid) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());

    // 1. probe_server_meta, register, forgot_password
    let (_, probe_url, probe_h) = mock_conn(
      "200 OK",
      r#"{"status":"ready","version":"0.14.11","registration_open":true}"#,
      false,
    );
    let meta = CloudConnector::new(CloudConfig::default()).probe_server_meta(&probe_url).await.unwrap();
    assert!(meta.reachable && meta.version.as_deref() == Some("0.14.11") && meta.registration_open == Some(true));
    assert!(probe_h.join().unwrap().starts_with("GET /api/ready "));

    let (reg_c, _, reg_h) = mock_conn("200 OK", r#"{"verified":true}"#, false);
    let u = reg_c.register("new@mica.dev", "New User", "secret123").await.unwrap();
    assert!(u.verified && u.email == "new@mica.dev" && reg_h.join().unwrap().starts_with("POST /api/auth/register "));

    let (fp_c, _, fp_h) = mock_conn("204 No Content", "", false);
    fp_c.forgot_password("new@mica.dev").await.unwrap();
    assert!(fp_h.join().unwrap().starts_with("POST /api/auth/password/forgot "));

    // 2. share create / get / delete
    let (sc, surl, sh) = mock_conn("200 OK", r#"{"shared":true,"token":"abc123"}"#, true);
    let share = sc.create_share(ws, doc).await.unwrap();
    assert!(share.shared && share.url.as_deref() == Some(format!("{surl}/s/abc123").as_str()));
    assert!(sh.join().unwrap().to_ascii_lowercase().contains("authorization: bearer jwt"));
    let (gsc, _, _) = mock_conn("200 OK", r#"{"shared":true,"token":"abc123"}"#, true);
    assert!(gsc.get_share(ws, doc).await.unwrap().shared);
    let (dsc, _, _) = mock_conn("200 OK", r#"{"shared":false}"#, true);
    assert!(!dsc.delete_share(ws, doc).await.unwrap().shared);

    // 3. comments list / create / reply / resolve / delete
    let (lc, _, _) = mock_conn(
      "200 OK",
      r#"{"threads":[{"id":"t1","status":"open","quote":"hi","created_by":"u1","created_at":"2026-01-01T00:00:00Z","anchor":{"start_block":"b1","start_offset":0,"end_block":"b1","end_offset":2},"comments":[{"id":"c1","author_id":"u1","body":"Nice!","created_at":"2026-01-01T00:00:00Z"}]}]}"#,
      true,
    );
    let threads = lc.list_comments(ws, doc).await.unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].start_block.as_deref(), Some("b1"));
    assert_eq!(threads[0].comments[0].body, "Nice!");
    let (cc, _, _) = mock_conn("200 OK", r#"{"id":"t2","status":"open","comments":[]}"#, true);
    assert_eq!(cc.create_comment(ws, doc, "b1", 0, "b1", 5, "w", "Hello").await.unwrap().id, "t2");
    let (rc, _, _) = mock_conn("200 OK", r#"{"id":"c3","author_id":"u2","body":"Reply!"}"#, true);
    assert_eq!(rc.reply_comment(ws, doc, tid, "Reply!").await.unwrap().body, "Reply!");
    mock_conn("200 OK", r#"{"resolved":true}"#, true).0.resolve_comment(ws, doc, tid, true).await.unwrap();
    mock_conn("200 OK", r#"{"deleted":true}"#, true).0.delete_comment(ws, doc, tid).await.unwrap();

    // 4. tokens list / create / revoke
    let (tc, _, _) = mock_conn("200 OK", r#"{"tokens":[{"id":"tk1","name":"CLI","scopes":["read","write"],"created_at":"2026-01-01T00:00:00Z"}]}"#, true);
    assert!(tc.list_tokens().await.unwrap()[0].can_write);
    let (ctc, _, _) = mock_conn("200 OK", r#"{"id":"tk2","name":"CI","scopes":["read"],"token":"mica_pat_secret","created_at":"2026-01-01T00:00:00Z"}"#, true);
    assert_eq!(ctc.create_token("CI", false, Some(30)).await.unwrap().token, "mica_pat_secret");
    mock_conn("204 No Content", "", true).0.revoke_token("tk2").await.unwrap();

    // 5. AI providers list / save / delete / models / prompt
    let ai_body = r#"{"configured":true,"provider_id":"deepseek","key_hint":"9f2a","configured_providers":[{"provider_id":"deepseek","has_key":true,"model":"deepseek-chat","base_url":"https://api.deepseek.com","protocol":"openai","active":true}]}"#;
    let providers = mock_conn("200 OK", ai_body, true).0.list_ai_providers().await.unwrap();
    assert_eq!((providers[0].id.as_str(), providers[0].key_tail.as_str()), ("deepseek", "9f2a"));
    assert_eq!(mock_conn("200 OK", ai_body, true).0.save_ai_provider(&providers[0], Some("sk-9f2a")).await.unwrap().len(), 1);
    assert!(mock_conn("200 OK", r#"{"configured":false,"configured_providers":[]}"#, true).0.delete_ai_provider("deepseek").await.unwrap().is_empty());
    assert_eq!(
      mock_conn("200 OK", r#"{"models":["deepseek-chat"],"endpoint":"https://api.deepseek.com/v1/models"}"#, true)
        .0
        .fetch_ai_models(Some("deepseek"), "https://api.deepseek.com", "openai", None).await
        .unwrap(),
      vec!["deepseek-chat"]
    );
    assert!(CloudConnector::new(CloudConfig::default()).stream_ai_prompt("", "Summarize").await.is_err());
    assert_eq!(mock_conn("200 OK", r#"{"text":"Summary"}"#, true).0.stream_ai_prompt("Brief", "Sum").await.unwrap(), "Summary");

    // 6. workspace members, invitations & graph
    let members = mock_conn("200 OK", r#"{"members":[{"user_id":"u1","email":"a@mica.dev","display_name":"Alice","role":"owner","joined_at":"2026-01-01T00:00:00Z"}]}"#, true).0.list_members(ws).await.unwrap();
    assert_eq!(members[0].role, "owner");
    let inv = mock_conn("200 OK", r#"{"invitation":{"id":"inv1","workspace_id":"ws1","email":"b@mica.dev","role":"editor","created_at":"2026-01-01T00:00:00Z"}}"#, true).0.invite_member(ws, "b@mica.dev", "editor").await.unwrap();
    assert_eq!(inv.id, "inv1");
    assert_eq!(mock_conn("200 OK", r#"{"invitations":[{"id":"inv1","workspace_id":"ws1","role":"editor"}]}"#, true).0.list_invitations(ws).await.unwrap().len(), 1);
    assert_eq!(mock_conn("200 OK", r#"{"invitations":[{"id":"inv2","workspace_id":"ws2","workspace_name":"Team","role":"viewer"}]}"#, true).0.list_received_invitations().await.unwrap()[0].workspace_name.as_deref(), Some("Team"));

    let graph = mock_conn("200 OK", r#"{"nodes":[{"view_id":"v1","name":"Home","degree":2}],"edges":[{"source":"v1","target":"v2"}],"unlinked":3}"#, true).0.fetch_graph(ws).await.unwrap();
    assert_eq!((graph.nodes.len(), graph.edges.len(), graph.unlinked), (1, 1, 3));

    // 7. Cookie-based refresh (`POST /api/auth/refresh` with `{}`) & server logout (`POST /api/auth/logout`)
    let (mut rc, _, rh) = mock_conn(
      "200 OK",
      r#"{"access_token":"jwt-rotated","token_type":"Bearer","expires_at":"2026-12-31T00:00:00Z","refresh_token":"rt-rotated","user":{"id":"00000000-0000-0000-0000-000000000001","email":"cookie@mica.dev","display_name":"Cookie","created_at":"2026-01-01T00:00:00Z"}}"#,
      false,
    );
    let refreshed_email = rc.refresh_session_cookie().await.unwrap();
    assert_eq!(refreshed_email, "cookie@mica.dev");
    assert_eq!(rc.access_token(), Some("jwt-rotated"));
    assert!(rc.refresh_token().is_none(), "cookie mode must not retain refresh token in client state");
    let raw_req = rh.join().unwrap();
    assert!(raw_req.starts_with("POST /api/auth/refresh "));
    assert!(raw_req.ends_with("{}"));

    let (lc_out, _, lh_out) = mock_conn("204 No Content", "", false);
    lc_out.logout_server().await.unwrap();
    assert!(lh_out.join().unwrap().starts_with("POST /api/auth/logout "));
    });
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn run_blocking_executes_off_calling_thread_and_wakes() {
    use std::future::Future;
    use std::sync::{Arc, Condvar, Mutex};
    use std::task::{Context, Poll, Wake, Waker};

    struct CondWaker {
      ready: Mutex<bool>,
      cv: Condvar,
    }
    impl Wake for CondWaker {
      fn wake(self: Arc<Self>) {
        *self.ready.lock().unwrap() = true;
        self.cv.notify_all();
      }
    }

    let caller = thread::current().id();
    let mut fut = Box::pin(run_blocking(move || thread::current().id()));
    let wake = Arc::new(CondWaker { ready: Mutex::new(false), cv: Condvar::new() });
    let waker = Waker::from(Arc::clone(&wake));
    let mut cx = Context::from_waker(&waker);

    let worker_id = loop {
      match fut.as_mut().poll(&mut cx) {
        Poll::Ready(id) => break id,
        Poll::Pending => {
          let mut g = wake.ready.lock().unwrap();
          while !*g {
            g = wake.cv.wait(g).unwrap();
          }
          *g = false;
        }
      }
    };
    assert_ne!(worker_id, caller, "HTTP work must execute off the UI thread");
  }
}
