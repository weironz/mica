//! Page version history — yrs-native (docs/version-history-plan.md). Auto
//! snapshots are captured on a cadence in `sync::push_update`; these endpoints
//! list them, preview one read-only, pin a named version, and restore. Restore
//! is a NEW forward update (never a rewrite of the log), broadcast as a yrs
//! `sync.update` so open editors converge — the same channel a normal edit uses.
//!
//! The old op-model history (document_versions / document_snapshots) froze with
//! P4①b and is intentionally no longer touched here (that was the split-brain
//! trap that made restore silently blank yrs docs).

use axum::{
  Json,
  extract::{Path, State},
  http::HeaderMap,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use mica_app_core::{
  AppState,
  documents::DocumentSnapshotPayload,
  store::{self, YrsVersionMeta},
  sync,
};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::documents::{ensure_workspace_editor, ensure_workspace_member};

#[derive(Debug, Deserialize)]
pub struct CreateVersionRequest {
  name: String,
}

#[derive(Debug, Deserialize)]
pub struct RestoreRequest {
  version_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct HistoryResponse {
  versions: Vec<YrsVersionMeta>,
}

#[derive(Debug, Serialize)]
pub struct VersionResponse {
  version: YrsVersionMeta,
}

#[derive(Debug, Serialize)]
pub struct VersionPreviewResponse {
  /// The document's blocks AS OF this version, for a read-only preview.
  payload: DocumentSnapshotPayload,
}

#[derive(Debug, Serialize)]
pub struct RestoreResponse {
  /// The stream rid the restore landed at (0/unchanged when the target already
  /// matched the live state).
  rid: i64,
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/history`
///
/// The version timeline, newest first — auto snapshots and named checkpoints
/// interleaved (metadata only; blobs load per-version on preview/restore).
pub async fn get_history(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<HistoryResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state, workspace_id, document_id).await?;

  let versions = store::list_yrs_versions(&state.db, document_id).await?;
  Ok(Json(HistoryResponse { versions }))
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/versions/{version_id}`
///
/// One version's content, decoded to blocks, so the client can render a
/// read-only preview without touching the live document.
pub async fn get_version(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id, version_id)): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<Json<VersionPreviewResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state, workspace_id, document_id).await?;

  let state_blob = store::fetch_yrs_version_state(&state.db, document_id, version_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let payload = store::yrs_state_to_payload(&state_blob)?;
  Ok(Json(VersionPreviewResponse { payload }))
}

/// `POST /api/workspaces/{workspace_id}/documents/{document_id}/versions`
///
/// Pins the document's current state as a NAMED version (never auto-pruned).
pub async fn create_version(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<CreateVersionRequest>,
) -> ApiResult<Json<VersionResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state, workspace_id, document_id).await?;

  let name = normalize_version_name(&payload.name)?;
  let version = store::create_named_yrs_version(&state.db, document_id, &name, user_id).await?;
  Ok(Json(VersionResponse { version }))
}

/// `POST /api/workspaces/{workspace_id}/documents/{document_id}/restore`
///
/// Restore the document to a version. CRDT-safe: the restore is a NEW forward
/// update (see `sync::restore_yrs_version`), pushed and broadcast as a yrs
/// `sync.update` so every open editor converges — never a hard reset.
pub async fn restore(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<RestoreRequest>,
) -> ApiResult<Json<RestoreResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state, workspace_id, document_id).await?;

  let state_blob = store::fetch_yrs_version_state(&state.db, document_id, payload.version_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  let (rid, update) =
    sync::restore_yrs_version(&state.db, workspace_id, document_id, user_id, &state_blob).await?;

  // Fan the restore out to open editors on the same yrs channel a live edit
  // uses — nil origin so every socket (including none, if idle) receives it.
  if !update.is_empty() {
    let event = json!({
      "type": "sync.update",
      "document_id": document_id,
      "rid": rid,
      "actor_id": user_id,
      "update": STANDARD.encode(&update),
    });
    state
      .hub
      .broadcast_if_active(document_id, Uuid::nil(), std::sync::Arc::from(event.to_string()));
  }

  Ok(Json(RestoreResponse { rid }))
}

async fn ensure_document_in_workspace(
  state: &AppState,
  workspace_id: Uuid,
  document_id: Uuid,
) -> ApiResult<()> {
  store::fetch_document(&state.db, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  Ok(())
}

fn normalize_version_name(name: &str) -> ApiResult<String> {
  let name = name.trim().to_string();
  if name.is_empty() {
    return Err(ApiError::BadRequest("version name is required".to_string()));
  }
  Ok(name)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn version_name_is_trimmed() {
    assert_eq!(normalize_version_name("  v1  ").unwrap(), "v1");
  }

  #[test]
  fn blank_version_name_is_rejected() {
    assert!(matches!(
      normalize_version_name("   "),
      Err(ApiError::BadRequest(_))
    ));
  }
}
