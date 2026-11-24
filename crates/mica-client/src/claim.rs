//! Async ownership & generation guards implementing `docs/dev/async-review-checklist.md`.
//!
//! Every cross-`await` or background result must carry a [`SessionClaim`] captured
//! at dispatch time and validate it via [`ClaimTracker::is_current`] before
//! mutating shared client state (including on error paths).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifies whether a workspace/document belongs to the local SQLite world
/// or a specific cloud server origin.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorldOrigin {
  Local,
  Cloud { server_url: String },
}

/// Snapshot of active account/world/workspace/document/sync ownership captured
/// before issuing an async call or scheduling a timer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionClaim {
  pub account_generation: u64,
  pub origin: WorldOrigin,
  pub workspace_id: Option<Uuid>,
  pub document_id: Option<Uuid>,
  pub sync_generation: u64,
}

/// Monotonic generation tracker held by the client shell.
#[derive(Debug, Clone)]
pub struct ClaimTracker {
  account_generation: u64,
  origin: WorldOrigin,
  workspace_id: Option<Uuid>,
  document_id: Option<Uuid>,
  sync_generation: u64,
}

impl Default for ClaimTracker {
  fn default() -> Self {
    Self::new(WorldOrigin::Local)
  }
}

impl ClaimTracker {
  pub fn new(origin: WorldOrigin) -> Self {
    Self {
      account_generation: 1,
      origin,
      workspace_id: None,
      document_id: None,
      sync_generation: 1,
    }
  }

  /// Capture the current claim before launching async work.
  pub fn capture(&self) -> SessionClaim {
    SessionClaim {
      account_generation: self.account_generation,
      origin: self.origin.clone(),
      workspace_id: self.workspace_id,
      document_id: self.document_id,
      sync_generation: self.sync_generation,
    }
  }

  /// Switch account or server; invalidates all previous claims.
  pub fn switch_account(&mut self, origin: WorldOrigin) -> SessionClaim {
    self.account_generation = self.account_generation.saturating_add(1);
    self.origin = origin;
    self.workspace_id = None;
    self.document_id = None;
    self.sync_generation = self.sync_generation.saturating_add(1);
    self.capture()
  }

  /// Switch active workspace; invalidates previous document & sync claims.
  pub fn switch_workspace(&mut self, workspace_id: Option<Uuid>) -> SessionClaim {
    self.workspace_id = workspace_id;
    self.document_id = None;
    self.sync_generation = self.sync_generation.saturating_add(1);
    self.capture()
  }

  /// Switch active document; bumps `sync_generation` even for A -> B -> A reopen.
  pub fn open_document(&mut self, document_id: Option<Uuid>) -> SessionClaim {
    self.document_id = document_id;
    self.sync_generation = self.sync_generation.saturating_add(1);
    self.capture()
  }

  /// Bump sync generation when closing/reconciling a document sync session.
  pub fn bump_sync(&mut self) -> u64 {
    self.sync_generation = self.sync_generation.saturating_add(1);
    self.sync_generation
  }

  /// Exact match across account, origin, workspace, document, and sync epoch.
  pub fn is_current(&self, claim: &SessionClaim) -> bool {
    self.account_generation == claim.account_generation
      && self.origin == claim.origin
      && self.workspace_id == claim.workspace_id
      && self.document_id == claim.document_id
      && self.sync_generation == claim.sync_generation
  }

  /// Account + origin match (for workspace-list or profile-level responses).
  pub fn matches_account(&self, claim: &SessionClaim) -> bool {
    self.account_generation == claim.account_generation && self.origin == claim.origin
  }

  /// Account + origin + workspace match (for page-tree/search responses).
  pub fn matches_workspace(&self, claim: &SessionClaim) -> bool {
    self.matches_account(claim) && self.workspace_id == claim.workspace_id
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reopen_same_document_invalidates_stale_claim() {
    let mut tracker = ClaimTracker::new(WorldOrigin::Cloud {
      server_url: "https://mica.example".into(),
    });
    let ws = Uuid::new_v4();
    let doc_a = Uuid::new_v4();
    let doc_b = Uuid::new_v4();

    tracker.switch_workspace(Some(ws));
    let claim_a1 = tracker.open_document(Some(doc_a));
    assert!(tracker.is_current(&claim_a1));

    tracker.open_document(Some(doc_b));
    assert!(!tracker.is_current(&claim_a1));
    assert!(tracker.matches_workspace(&claim_a1));

    let claim_a2 = tracker.open_document(Some(doc_a));
    assert!(!tracker.is_current(&claim_a1));
    assert!(tracker.is_current(&claim_a2));
  }

  #[test]
  fn account_switch_invalidates_workspace_and_account_claims() {
    let mut tracker = ClaimTracker::new(WorldOrigin::Local);
    let ws = Uuid::new_v4();
    tracker.switch_workspace(Some(ws));
    let claim = tracker.capture();

    tracker.switch_account(WorldOrigin::Cloud {
      server_url: "https://cloud.example".into(),
    });
    assert!(!tracker.matches_account(&claim));
    assert!(!tracker.matches_workspace(&claim));
    assert!(!tracker.is_current(&claim));
  }
}
