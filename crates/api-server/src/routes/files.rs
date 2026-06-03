use std::sync::Arc;

use axum::{
  Json,
  extract::{Path, State},
  http::HeaderMap,
};
use mica_app_core::{AppState, store};
use mica_infra::{ApiError, ApiResult, S3Config};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::documents::{ensure_workspace_editor, ensure_workspace_member};

#[derive(Debug, Deserialize)]
pub struct PresignRequest {
  file_name: String,
  mime_type: String,
  byte_size: i64,
  /// Lowercase hex sha256 of the file bytes (client-computed). Used as the
  /// content-addressed object key so identical uploads dedup.
  content_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct CompleteRequest {
  object_key: String,
  /// Original upload filename, preserved for export (object keys are hashes).
  file_name: String,
  mime_type: String,
  byte_size: i64,
}

#[derive(Debug, Serialize)]
pub struct PresignResponse {
  object_key: String,
  upload_url: String,
  method: &'static str,
  expires_in: u64,
  max_byte_size: i64,
}

#[derive(Debug, Serialize)]
pub struct FileResponse {
  file: store::FileRecord,
  download_url: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveRequest {
  ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct ResolveResponse {
  files: Vec<FileResponse>,
}

/// `POST /api/workspaces/{workspace_id}/files/presign`
///
/// Issues a presigned upload URL the client uses to PUT the object directly to
/// object storage. No metadata row is created until `complete` is called.
pub async fn presign(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<PresignRequest>,
) -> ApiResult<Json<PresignResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  let storage = storage(&state)?;

  validate_mime(&payload.mime_type)?;
  validate_byte_size(payload.byte_size, storage.max_upload_bytes)?;

  let object_key = build_object_key(workspace_id, &payload.content_hash, &payload.file_name)?;
  let upload = storage.presign_put(&object_key);

  Ok(Json(PresignResponse {
    object_key,
    upload_url: upload.url,
    method: upload.method,
    expires_in: upload.expires_in,
    max_byte_size: storage.max_upload_bytes,
  }))
}

/// `POST /api/workspaces/{workspace_id}/files/complete`
///
/// Records metadata after a successful upload and returns a URL for reading the
/// object (used as an image block's `url`).
pub async fn complete(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<CompleteRequest>,
) -> ApiResult<Json<FileResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  let storage = storage(&state)?;

  validate_mime(&payload.mime_type)?;
  validate_byte_size(payload.byte_size, storage.max_upload_bytes)?;
  ensure_key_in_workspace(workspace_id, &payload.object_key)?;

  let file = store::insert_file(
    &state.db,
    workspace_id,
    user_id,
    &payload.object_key,
    &sanitize_file_name(&payload.file_name),
    &payload.mime_type,
    payload.byte_size,
  )
  .await?;
  let download_url = storage.download_url(&file.object_key);

  Ok(Json(FileResponse { file, download_url }))
}

/// `POST /api/workspaces/{workspace_id}/files/resolve`
///
/// Resolve many file ids to fresh download URLs at once. Image blocks store only
/// a `file_id`, so the client calls this on document load to obtain displayable
/// (and never-stale) URLs. Unknown ids are silently dropped.
pub async fn resolve(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<ResolveRequest>,
) -> ApiResult<Json<ResolveResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  let storage = storage(&state)?;

  let records = store::fetch_files(&state.db, workspace_id, &payload.ids).await?;
  let files = records
    .into_iter()
    .map(|file| {
      let download_url = storage.download_url(&file.object_key);
      FileResponse { file, download_url }
    })
    .collect();

  Ok(Json(ResolveResponse { files }))
}

/// `GET /api/workspaces/{workspace_id}/files/{file_id}`
///
/// Returns file metadata and a fresh download URL.
pub async fn get_file(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, file_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<FileResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  let storage = storage(&state)?;

  let file = store::fetch_file(&state.db, workspace_id, file_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let download_url = storage.download_url(&file.object_key);

  Ok(Json(FileResponse { file, download_url }))
}

/// `DELETE /api/workspaces/{workspace_id}/files/{file_id}`
pub async fn delete_file(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, file_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<Value>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let deleted = store::delete_file(&state.db, workspace_id, file_id).await?;
  if !deleted {
    return Err(ApiError::NotFound);
  }

  Ok(Json(json!({ "deleted": true })))
}

fn storage(state: &AppState) -> ApiResult<Arc<S3Config>> {
  state.storage.clone().ok_or_else(|| {
    ApiError::Unavailable("file storage is not configured on this server".to_string())
  })
}

fn validate_mime(mime_type: &str) -> ApiResult<()> {
  if mime_type.trim().is_empty() {
    return Err(ApiError::BadRequest("mime_type is required".to_string()));
  }

  Ok(())
}

fn validate_byte_size(byte_size: i64, max_upload_bytes: i64) -> ApiResult<()> {
  if byte_size <= 0 {
    return Err(ApiError::BadRequest(
      "byte_size must be positive".to_string(),
    ));
  }
  if byte_size > max_upload_bytes {
    return Err(ApiError::BadRequest(format!(
      "file exceeds the maximum upload size of {max_upload_bytes} bytes"
    )));
  }

  Ok(())
}

/// Content-addressed object key: `workspaces/{ws}/{sha256}.{ext}`. Identical
/// bytes (same hash) map to the same key, giving free per-workspace dedup. The
/// original filename is preserved separately (in the file row) for export.
fn build_object_key(workspace_id: Uuid, content_hash: &str, file_name: &str) -> ApiResult<String> {
  let hash = content_hash.trim().to_ascii_lowercase();
  if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
    return Err(ApiError::BadRequest(
      "content_hash must be a hex sha256".to_string(),
    ));
  }
  let ext = file_extension(file_name);
  let name = match ext {
    Some(ext) => format!("{hash}.{ext}"),
    None => hash,
  };
  Ok(format!("workspaces/{workspace_id}/{name}"))
}

/// Lowercase alphanumeric extension of [file_name], or None.
fn file_extension(file_name: &str) -> Option<String> {
  let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
  let ext = base.rsplit_once('.')?.1;
  if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
    return None;
  }
  Some(ext.to_ascii_lowercase())
}

/// Confirm a client-provided object key belongs to this workspace, blocking
/// completion against another workspace's prefix.
fn ensure_key_in_workspace(workspace_id: Uuid, object_key: &str) -> ApiResult<()> {
  let prefix = format!("workspaces/{workspace_id}/");
  if !object_key.starts_with(&prefix) {
    return Err(ApiError::BadRequest(
      "object_key does not belong to this workspace".to_string(),
    ));
  }

  Ok(())
}

/// Reduce a client file name to a safe basename: strip any path and keep only
/// alphanumerics plus `-`, `_`, and `.`.
fn sanitize_file_name(name: &str) -> String {
  let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
  let cleaned: String = base
    .chars()
    .map(|ch| {
      if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
        ch
      } else {
        '_'
      }
    })
    .collect();
  let trimmed = cleaned.trim_matches('.').to_string();

  if trimmed.is_empty() {
    "file".to_string()
  } else {
    trimmed
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn object_key_is_content_addressed_and_scoped() {
    let workspace_id = Uuid::from_u128(42);
    let hash = "a".repeat(64);
    let key = build_object_key(workspace_id, &hash, "photo.PNG").unwrap();
    assert_eq!(key, format!("workspaces/{workspace_id}/{hash}.png"));

    // Same bytes (hash) → same key regardless of original filename → dedup.
    let key2 = build_object_key(workspace_id, &hash, "other-name.png").unwrap();
    assert_eq!(key, key2);

    // A bad hash is rejected.
    assert!(build_object_key(workspace_id, "not-a-hash", "x.png").is_err());

    // No extension is fine.
    let key3 = build_object_key(workspace_id, &hash, "noext").unwrap();
    assert_eq!(key3, format!("workspaces/{workspace_id}/{hash}"));
  }

  #[test]
  fn sanitize_strips_path_and_unsafe_chars() {
    assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
    assert_eq!(sanitize_file_name("my file (1).PNG"), "my_file__1_.PNG");
    assert_eq!(sanitize_file_name("..."), "file");
  }

  #[test]
  fn key_in_workspace_is_enforced() {
    let ws = Uuid::from_u128(1);
    assert!(ensure_key_in_workspace(ws, &format!("workspaces/{ws}/abc/x.png")).is_ok());
    assert!(matches!(
      ensure_key_in_workspace(ws, "workspaces/00000000-0000-0000-0000-000000000002/x.png"),
      Err(ApiError::BadRequest(_))
    ));
  }

  #[test]
  fn byte_size_bounds_are_validated() {
    assert!(validate_byte_size(1, 100).is_ok());
    assert!(matches!(
      validate_byte_size(0, 100),
      Err(ApiError::BadRequest(_))
    ));
    assert!(matches!(
      validate_byte_size(101, 100),
      Err(ApiError::BadRequest(_))
    ));
  }
}
