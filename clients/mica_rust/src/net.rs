//! Cloud HTTP API client & Sans-I/O [`SyncEngine`] coordinator (`CloudConnector`).
//!
//! Uses [`mica_client`] for single-source endpoint URLs and account/sync
//! generation guards ([`ClaimTracker`]), and [`mica_sync`] for the Sans-I/O
//! document synchronization state machine.
//!
//! Blocking HTTP calls (`ureq`) are isolated behind `#[cfg(not(target_arch = "wasm32"))]`
//! so the module compiles cleanly for both desktop and Web (`wasm32`) targets.

use mica_client::{ApiEndpoints, ClaimTracker, WorldOrigin};
use mica_sync::{
  MemoryMirrorStorage, SyncConfig, SyncEffect, SyncEngine, SyncEvent, SyncPhase, SyncStorage,
};
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
use mica_client::protocol::{LoginRequest, decode_auth};

#[cfg(not(target_arch = "wasm32"))]
const HEALTH_TIMEOUT_SECS: u64 = 5;
#[cfg(not(target_arch = "wasm32"))]
const LOGIN_TIMEOUT_SECS: u64 = 10;

/// Serializable cloud connection & session configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudConfig {
  pub server_url: String,
  pub access_token: Option<String>,
  pub email: Option<String>,
}

/// Coordinates cloud HTTP auth/health probes, [`ClaimTracker`] ownership
/// generations, and per-document Sans-I/O [`SyncEngine`] instances.
#[derive(Debug, Clone)]
pub struct CloudConnector {
  config: CloudConfig,
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

    let has_token = config
      .access_token
      .as_deref()
      .is_some_and(|t| !t.trim().is_empty());
    let origin = if has_token && !endpoints.base_url().is_empty() {
      WorldOrigin::Cloud {
        server_url: endpoints.base_url().to_string(),
      }
    } else {
      WorldOrigin::Local
    };

    Self {
      config,
      endpoints,
      claims: ClaimTracker::new(origin),
      phase: SyncPhase::Offline,
    }
  }

  /// Probe `GET /api/health` on the configured server.
  ///
  /// Returns a concise human-readable status summary on success, or a clean
  /// error message on network/HTTP failure or on `wasm32`.
  pub fn check_health(&self) -> Result<String, String> {
    if self.endpoints.base_url().is_empty() {
      return Err("server URL is empty".to_string());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
      let url = self.endpoints.health();
      let agent = http_agent(HEALTH_TIMEOUT_SECS);
      let mut resp = agent
        .get(&url)
        .call()
        .map_err(|e| format!("health check failed for {url}: {e}"))?;
      let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("failed to read health response: {e}"))?;
      Ok(format_health_summary(&body))
    }
    #[cfg(target_arch = "wasm32")]
    {
      Err("blocking HTTP health check is unavailable on wasm32".to_string())
    }
  }

  /// Authenticate via `POST /api/auth/login`, persist the returned access token
  /// and user email in memory, and advance [`ClaimTracker`] to the cloud origin.
  pub fn login(&mut self, email: &str, password: &str) -> Result<String, String> {
    if self.endpoints.base_url().is_empty() {
      return Err("server URL is empty".to_string());
    }
    let email_trimmed = email.trim();
    if email_trimmed.is_empty() || password.is_empty() {
      return Err("email and password are required".to_string());
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
      let claim = self.claims.capture();
      let url = self.endpoints.login();
      let payload = LoginRequest {
        email: email_trimmed.to_string(),
        password: password.to_string(),
      };
      let agent = http_agent(LOGIN_TIMEOUT_SECS);
      let mut resp = agent
        .post(&url)
        .send_json(&payload)
        .map_err(|e| format!("login request failed: {e}"))?;
      let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("failed to read login response: {e}"))?;
      let auth = decode_auth(&body).map_err(|e| format!("invalid login response: {e}"))?;

      if !self.claims.matches_account(&claim) {
        return Err("account session changed during login".to_string());
      }

      self.config.access_token = Some(auth.access_token);
      self.config.email = Some(auth.user.email.clone());
      self.claims.switch_account(WorldOrigin::Cloud {
        server_url: self.endpoints.base_url().to_string(),
      });
      Ok(auth.user.email)
    }
    #[cfg(target_arch = "wasm32")]
    {
      let _ = (email_trimmed, password);
      Err("blocking HTTP login is unavailable on wasm32".to_string())
    }
  }

  /// Clear stored credentials and advance [`ClaimTracker`] back to
  /// [`WorldOrigin::Local`].
  pub fn logout(&mut self) {
    self.config.access_token = None;
    self.config.email = None;
    self.phase = SyncPhase::Offline;
    self.claims.switch_account(WorldOrigin::Local);
  }

  /// Update the target server URL. If the URL changes while authenticated, the
  /// previous server's token is cleared and [`ClaimTracker`] resets to local so
  /// credentials are never sent to a different host.
  pub fn set_server_url(&mut self, server_url: impl Into<String>) {
    let next_endpoints = ApiEndpoints::new(server_url.into().trim());
    let changed = next_endpoints.base_url() != self.endpoints.base_url();
    self.endpoints = next_endpoints;
    self.config.server_url = self.endpoints.base_url().to_string();
    if changed && self.is_authenticated() {
      self.logout();
    }
  }

  /// Current derived sync status label for UI chrome.
  pub fn sync_phase_label(&self) -> String {
    if !self.is_authenticated() {
      return "Local Only".to_string();
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

  /// Update the connector's tracked [`SyncPhase`] (e.g. from
  /// [`SyncEffect::SyncPhaseChanged`]).
  pub fn set_sync_phase(&mut self, phase: SyncPhase) {
    self.phase = phase;
  }

  /// Construct a Sans-I/O [`SyncEngine`] for `doc_id` backed by
  /// [`MemoryMirrorStorage`].
  pub fn create_doc_sync(&self, doc_id: &str, client_id: u64, self_name: &str) -> SyncEngine {
    self.create_doc_sync_with_storage(
      doc_id,
      client_id,
      self_name,
      Box::new(MemoryMirrorStorage::new()),
    )
  }

  /// Construct a Sans-I/O [`SyncEngine`] for `doc_id` with a caller-supplied
  /// [`SyncStorage`] backend (such as `VaultStore::create_sync_storage`).
  pub fn create_doc_sync_with_storage(
    &self,
    doc_id: &str,
    client_id: u64,
    self_name: &str,
    storage: Box<dyn SyncStorage>,
  ) -> SyncEngine {
    let name_opt = {
      let trimmed = self_name.trim();
      if trimmed.is_empty() {
        self
          .config
          .email
          .as_deref()
          .map(str::trim)
          .filter(|s| !s.is_empty())
          .map(String::from)
      } else {
        Some(trimmed.to_string())
      }
    };
    let config = SyncConfig {
      client_id: Some(client_id),
      self_name: name_opt,
      document_id: Some(doc_id.to_string()),
      restore_unacked: Vec::new(),
    };
    SyncEngine::new(config, Some(storage))
  }

  /// Feed a local CRDT update into a Sans-I/O [`SyncEngine`] and return the
  /// resulting [`SyncEffect`]s for the host to execute.
  pub fn push_local_update(engine: &mut SyncEngine, update: Vec<u8>) -> Vec<SyncEffect> {
    engine.handle_event(SyncEvent::ApplyLocalDiff(update))
  }

  /// Normalized server base URL (without trailing slash).
  pub fn server_url(&self) -> &str {
    self.endpoints.base_url()
  }

  /// Whether a non-empty access token is currently held.
  pub fn is_authenticated(&self) -> bool {
    self
      .config
      .access_token
      .as_deref()
      .is_some_and(|t| !t.trim().is_empty())
  }

  /// Human-readable account badge for the UI header/settings panel.
  pub fn account_label(&self) -> String {
    if !self.is_authenticated() {
      return "Local Vault".to_string();
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
}

#[cfg(not(target_arch = "wasm32"))]
fn http_agent(timeout_secs: u64) -> ureq::Agent {
  ureq::Agent::config_builder()
    .timeout_global(Some(Duration::from_secs(timeout_secs)))
    .build()
    .into()
}

#[cfg(not(target_arch = "wasm32"))]
fn format_health_summary(raw_body: &str) -> String {
  let trimmed = raw_body.trim();
  if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed)
    && let Some(status) = val.get("status").and_then(serde_json::Value::as_str)
  {
    if let Some(version) = val.get("version").and_then(serde_json::Value::as_str) {
      return format!("{status} (v{version})");
    }
    return status.to_string();
  }
  if trimmed.is_empty() {
    "ok".to_string()
  } else {
    trimmed.to_string()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn local_defaults_and_phase_labels() {
    let mut conn = CloudConnector::new(CloudConfig {
      server_url: "https://notes.example.com/".to_string(),
      access_token: None,
      email: None,
    });
    assert_eq!(conn.server_url(), "https://notes.example.com");
    assert!(!conn.is_authenticated());
    assert_eq!(conn.account_label(), "Local Vault");
    assert_eq!(conn.sync_phase_label(), "Local Only");
    assert_eq!(conn.claims().capture().origin, WorldOrigin::Local);

    conn.set_sync_phase(SyncPhase::Synced);
    // Unauthenticated still shows Local Only.
    assert_eq!(conn.sync_phase_label(), "Local Only");
  }

  #[test]
  fn authenticated_config_and_logout_advance_claims() {
    let mut conn = CloudConnector::new(CloudConfig {
      server_url: "https://cloud.mica.test/".to_string(),
      access_token: Some("tok-123".to_string()),
      email: Some("dev@mica.test".to_string()),
    });
    assert!(conn.is_authenticated());
    assert_eq!(conn.account_label(), "dev@mica.test");
    assert_eq!(conn.sync_phase_label(), "Offline");

    conn.set_sync_phase(SyncPhase::Syncing);
    assert_eq!(conn.sync_phase_label(), "Syncing");
    conn.set_sync_phase(SyncPhase::Synced);
    assert_eq!(conn.sync_phase_label(), "Synced");

    let cloud_claim = conn.claims().capture();
    assert_eq!(
      cloud_claim.origin,
      WorldOrigin::Cloud {
        server_url: "https://cloud.mica.test".to_string(),
      }
    );

    conn.logout();
    assert!(!conn.is_authenticated());
    assert_eq!(conn.account_label(), "Local Vault");
    assert_eq!(conn.sync_phase_label(), "Local Only");
    assert!(!conn.claims().matches_account(&cloud_claim));
    assert_eq!(conn.claims().capture().origin, WorldOrigin::Local);
  }

  #[test]
  fn create_doc_sync_and_push_local_update() {
    let conn = CloudConnector::new(CloudConfig::default());
    let mut engine = conn.create_doc_sync("doc-1", 42, "Alice");
    let doc = mica_core::MicaDoc::from_blocks(
      "root",
      &[
        mica_core::Block::new("root", "page").with_children(vec!["b1".to_string()]),
        mica_core::Block::new("b1", "paragraph").with_text("Hello sync"),
      ],
    );
    let update = doc.encode_state();
    let effects = CloudConnector::push_local_update(&mut engine, update);
    assert!(!effects.is_empty());
    assert!(!engine.outbox_empty());
  }
}
