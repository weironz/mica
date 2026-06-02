use axum::{
  Json,
  extract::{Path, Query, State},
  http::HeaderMap,
};
use mica_app_core::{
  AppState,
  store::{self, SnapshotRecord, UpdateLogEntry, VersionRecord},
};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::documents::{ensure_workspace_editor, ensure_workspace_member};
use crate::routes::ws;

const DEFAULT_HISTORY_LIMIT: i64 = 50;

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
  limit: Option<i64>,
  before_seq: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVersionRequest {
  name: String,
}

#[derive(Debug, Deserialize)]
pub struct RestoreRequest {
  version_id: Option<Uuid>,
  version_seq: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct HistoryResponse {
  current_seq: i64,
  updates: Vec<UpdateLogEntry>,
  versions: Vec<VersionRecord>,
}

#[derive(Debug, Serialize)]
pub struct VersionResponse {
  version: VersionRecord,
}

#[derive(Debug, Serialize)]
pub struct VersionDetailResponse {
  version: VersionRecord,
  snapshot: SnapshotRecord,
}

#[derive(Debug, Serialize)]
pub struct RestoreResponse {
  document: store::DocumentRecord,
  snapshot: SnapshotRecord,
  update: store::UpdateRecord,
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/history`
///
/// Returns the append-only change log (newest first, paginated) alongside the
/// document's named versions.
pub async fn get_history(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Query(query): Query<HistoryQuery>,
) -> ApiResult<Json<HistoryResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let document = store::fetch_document(&state.db, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  let limit = query.limit.unwrap_or(DEFAULT_HISTORY_LIMIT);
  let updates = store::list_updates(&state.db, document_id, limit, query.before_seq).await?;
  let versions = store::list_versions(&state.db, document_id).await?;

  Ok(Json(HistoryResponse {
    current_seq: document.current_seq,
    updates,
    versions,
  }))
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/versions/{version_id}`
///
/// Returns a named version together with the snapshot payload it pins, so the
/// client can preview the stored state.
pub async fn get_version(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id, version_id)): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<Json<VersionDetailResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state, workspace_id, document_id).await?;

  let version = store::fetch_version(&state.db, document_id, version_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let snapshot = store::fetch_snapshot(&state.db, document_id, version.snapshot_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  Ok(Json(VersionDetailResponse { version, snapshot }))
}

/// `POST /api/workspaces/{workspace_id}/documents/{document_id}/versions`
///
/// Pins the document's current state as a named, restorable version.
pub async fn create_version(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<CreateVersionRequest>,
) -> ApiResult<Json<VersionResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state, workspace_id, document_id).await?;

  let name = normalize_version_name(&payload.name)?;
  let version = store::create_named_version(&state.db, document_id, &name, user_id).await?;

  Ok(Json(VersionResponse { version }))
}

/// `POST /api/workspaces/{workspace_id}/documents/{document_id}/restore`
///
/// Restores the document to a prior version. History stays append-only: the
/// restore is recorded as a new update and broadcast to connected clients.
pub async fn restore(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<RestoreRequest>,
) -> ApiResult<Json<RestoreResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state, workspace_id, document_id).await?;

  let source_version_seq = resolve_source_version_seq(&state, document_id, &payload).await?;

  let applied = store::restore_snapshot(
    &state.db,
    workspace_id,
    document_id,
    user_id,
    source_version_seq,
  )
  .await?;

  // Surface the restore to anyone editing over WebSocket as a fresh update.
  ws::broadcast_applied_update(&state.hub, &applied, Uuid::nil(), None);

  Ok(Json(RestoreResponse {
    document: applied.document,
    snapshot: applied.snapshot,
    update: applied.update,
  }))
}

/// Which restore source the request names, after validating that exactly one of
/// `version_id` / `version_seq` was supplied.
enum RestoreTarget {
  Version(Uuid),
  VersionSeq(i64),
}

fn restore_target(payload: &RestoreRequest) -> ApiResult<RestoreTarget> {
  match (payload.version_id, payload.version_seq) {
    (Some(_), Some(_)) => Err(ApiError::BadRequest(
      "provide either version_id or version_seq, not both".to_string(),
    )),
    (Some(version_id), None) => Ok(RestoreTarget::Version(version_id)),
    (None, Some(version_seq)) => Ok(RestoreTarget::VersionSeq(version_seq)),
    (None, None) => Err(ApiError::BadRequest(
      "version_id or version_seq is required".to_string(),
    )),
  }
}

/// Resolve the request to the concrete snapshot sequence to restore from,
/// confirming the named version or snapshot exists.
async fn resolve_source_version_seq(
  state: &AppState,
  document_id: Uuid,
  payload: &RestoreRequest,
) -> ApiResult<i64> {
  match restore_target(payload)? {
    RestoreTarget::Version(version_id) => {
      let version = store::fetch_version(&state.db, document_id, version_id)
        .await?
        .ok_or(ApiError::NotFound)?;
      Ok(version.version_seq)
    }
    RestoreTarget::VersionSeq(version_seq) => {
      store::fetch_snapshot_by_version_seq(&state.db, document_id, version_seq)
        .await?
        .ok_or(ApiError::NotFound)?;
      Ok(version_seq)
    }
  }
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

  #[test]
  fn restore_target_accepts_version_id() {
    let id = Uuid::from_u128(7);
    let target = restore_target(&RestoreRequest {
      version_id: Some(id),
      version_seq: None,
    })
    .unwrap();
    assert!(matches!(target, RestoreTarget::Version(value) if value == id));
  }

  #[test]
  fn restore_target_accepts_version_seq() {
    let target = restore_target(&RestoreRequest {
      version_id: None,
      version_seq: Some(3),
    })
    .unwrap();
    assert!(matches!(target, RestoreTarget::VersionSeq(3)));
  }

  #[test]
  fn restore_target_rejects_both() {
    assert!(matches!(
      restore_target(&RestoreRequest {
        version_id: Some(Uuid::from_u128(1)),
        version_seq: Some(1),
      }),
      Err(ApiError::BadRequest(_))
    ));
  }

  #[test]
  fn restore_target_rejects_neither() {
    assert!(matches!(
      restore_target(&RestoreRequest {
        version_id: None,
        version_seq: None,
      }),
      Err(ApiError::BadRequest(_))
    ));
  }
}
