//! Transport-agnostic HTTP/WS endpoint builders and wire DTOs over `mica-api-types`.
//!
//! Keeps URL construction and payload shapes single-source across Rust clients
//! (desktop, Web WASM, and future pure-Rust shell) without pulling an HTTP runtime.

use mica_api_types::{
  AuthResponse, BatchOutcome, View, ViewListResponse, Workspace, WorkspaceListResponse,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiEndpoints {
  base_url: String,
}

impl ApiEndpoints {
  pub fn new(base_url: impl Into<String>) -> Self {
    let raw = base_url.into();
    let trimmed = raw.trim_end_matches('/').to_owned();
    Self { base_url: trimmed }
  }

  pub fn base_url(&self) -> &str {
    &self.base_url
  }

  pub fn health(&self) -> String {
    format!("{}/api/health", self.base_url)
  }

  pub fn login(&self) -> String {
    format!("{}/api/auth/login", self.base_url)
  }

  pub fn refresh(&self) -> String {
    format!("{}/api/auth/refresh", self.base_url)
  }

  pub fn me(&self) -> String {
    format!("{}/api/me", self.base_url)
  }

  pub fn workspaces(&self) -> String {
    format!("{}/api/workspaces", self.base_url)
  }

  pub fn workspace_views(&self, workspace_id: Uuid) -> String {
    format!("{}/api/workspaces/{workspace_id}/views", self.base_url)
  }

  pub fn document_ws(&self, document_id: Uuid, access_token: &str) -> String {
    let ws_base = if let Some(rest) = self.base_url.strip_prefix("https://") {
      format!("wss://{rest}")
    } else if let Some(rest) = self.base_url.strip_prefix("http://") {
      format!("ws://{rest}")
    } else {
      self.base_url.clone()
    };
    format!("{ws_base}/api/ws/documents/{document_id}?token={access_token}")
  }

  pub fn views_events_ws(&self, workspace_id: Uuid, access_token: &str) -> String {
    let ws_base = if let Some(rest) = self.base_url.strip_prefix("https://") {
      format!("wss://{rest}")
    } else if let Some(rest) = self.base_url.strip_prefix("http://") {
      format!("ws://{rest}")
    } else {
      self.base_url.clone()
    };
    format!("{ws_base}/api/ws/workspaces/{workspace_id}/views?token={access_token}")
  }
}

/// Request payload for `POST /api/auth/login`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
  pub email: String,
  pub password: String,
}

/// Request payload for `POST /api/auth/refresh`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshRequest {
  pub refresh_token: String,
}

/// Decode helpers sharing `mica-api-types` definitions.
pub fn decode_auth(json: &str) -> Result<AuthResponse, serde_json::Error> {
  serde_json::from_str(json)
}

pub fn decode_workspaces(json: &str) -> Result<Vec<Workspace>, serde_json::Error> {
  serde_json::from_str::<WorkspaceListResponse>(json).map(|r| r.workspaces)
}

pub fn decode_views(json: &str) -> Result<Vec<View>, serde_json::Error> {
  serde_json::from_str::<ViewListResponse>(json).map(|r| r.views)
}

pub fn decode_batch_outcome(json: &str) -> Result<BatchOutcome, serde_json::Error> {
  serde_json::from_str(json)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn builds_http_and_ws_endpoints() {
    let api = ApiEndpoints::new("https://notes.example.com/");
    let doc = Uuid::nil();
    assert_eq!(api.health(), "https://notes.example.com/api/health");
    assert_eq!(
      api.document_ws(doc, "tok"),
      "wss://notes.example.com/api/ws/documents/00000000-0000-0000-0000-000000000000?token=tok"
    );
    let http = ApiEndpoints::new("http://localhost:8080");
    assert!(
      http
        .views_events_ws(doc, "t")
        .starts_with("ws://localhost:8080/")
    );
  }
}
