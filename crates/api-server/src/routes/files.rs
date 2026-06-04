use std::sync::Arc;
use std::time::Duration;

use axum::{
  Json,
  extract::{Path, State},
  http::{HeaderMap, StatusCode},
  response::{IntoResponse, Redirect, Response},
};
use mica_app_core::{AppState, store};
use mica_infra::{ApiError, ApiResult, S3Config};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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

#[derive(Debug, Deserialize)]
pub struct ImportUrlRequest {
  url: String,
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
    &safe_file_name(&payload.file_name),
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

/// `POST /api/workspaces/{workspace_id}/files/import-url`
///
/// Server-side fetch a remote image and re-host it (so pasted image URLs don't
/// rot). The bytes are downloaded here, content-addressed, uploaded to storage
/// via a self-issued presigned PUT, and recorded — returning a file like a
/// normal upload.
pub async fn import_url(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<ImportUrlRequest>,
) -> ApiResult<Json<FileResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  let storage = storage(&state)?;

  let url = payload.url.trim();
  if !(url.starts_with("http://") || url.starts_with("https://")) {
    return Err(ApiError::BadRequest("url must be http(s)".to_string()));
  }

  let client = reqwest::Client::builder()
    .timeout(Duration::from_secs(20))
    .build()
    .map_err(|e| ApiError::Internal(e.to_string()))?;

  let response = client
    .get(url)
    .send()
    .await
    .map_err(|_| ApiError::BadRequest("could not fetch the image url".to_string()))?;
  if !response.status().is_success() {
    return Err(ApiError::BadRequest(format!(
      "image url returned {}",
      response.status()
    )));
  }

  let header_mime = response
    .headers()
    .get(reqwest::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
    .unwrap_or_default();

  let bytes = response
    .bytes()
    .await
    .map_err(|_| ApiError::BadRequest("could not read the image url".to_string()))?;
  let byte_size = bytes.len() as i64;
  validate_byte_size(byte_size, storage.max_upload_bytes)?;

  // Determine MIME + extension (header first, else the URL's extension).
  let ext = mime_to_ext(&header_mime)
    .map(str::to_string)
    .or_else(|| file_extension(url));
  let mime = if header_mime.starts_with("image/") {
    header_mime
  } else {
    match ext.as_deref().and_then(ext_to_mime) {
      Some(m) => m.to_string(),
      None => return Err(ApiError::BadRequest("url is not an image".to_string())),
    }
  };

  let hash = {
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
  };
  let object_key = match &ext {
    Some(ext) => format!("workspaces/{workspace_id}/{hash}.{ext}"),
    None => format!("workspaces/{workspace_id}/{hash}"),
  };

  // Upload via a self-issued presigned PUT (storage signs; we do the PUT).
  let upload = storage.presign_put(&object_key);
  let put = client
    .put(&upload.url)
    .header(reqwest::header::CONTENT_TYPE, &mime)
    .body(bytes.to_vec())
    .send()
    .await
    .map_err(|e| ApiError::Internal(format!("storage upload failed: {e}")))?;
  if !put.status().is_success() {
    return Err(ApiError::Internal(format!(
      "storage upload returned {}",
      put.status()
    )));
  }

  let original_name = safe_file_name(&url_file_name(url, ext.as_deref()));
  let file = store::insert_file(
    &state.db,
    workspace_id,
    user_id,
    &object_key,
    &original_name,
    &mime,
    byte_size,
  )
  .await?;
  let download_url = storage.download_url(&file.object_key);

  Ok(Json(FileResponse { file, download_url }))
}

/// `GET /api/workspaces/{workspace_id}/files/{file_id}/blob`
///
/// A stable, never-expiring public link to an image's bytes — it 302-redirects
/// to a freshly-signed storage URL on every request, so the link itself never
/// goes stale. Unauthenticated (the `file_id` UUID is the capability), so copied
/// Markdown images keep displaying in other apps. Used for copy/export.
pub async fn blob(
  State(state): State<AppState>,
  Path((workspace_id, file_id)): Path<(Uuid, Uuid)>,
) -> Response {
  let Ok(storage) = storage(&state) else {
    return StatusCode::NOT_FOUND.into_response();
  };
  match store::fetch_file(&state.db, workspace_id, file_id).await {
    Ok(Some(file)) => {
      Redirect::temporary(&storage.download_url(&file.object_key)).into_response()
    }
    _ => StatusCode::NOT_FOUND.into_response(),
  }
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

/// Server-side upload of in-memory bytes (the workspace importer's path):
/// content-hash the bytes, PUT via a self-issued presigned URL, and record
/// the file (deduplicated by object key). Returns the stored record.
pub(crate) async fn store_bytes(
  state: &AppState,
  client: &reqwest::Client,
  workspace_id: Uuid,
  user_id: Uuid,
  file_name: &str,
  bytes: &[u8],
) -> ApiResult<store::FileRecord> {
  let storage = storage(state)?;
  let byte_size = bytes.len() as i64;
  validate_byte_size(byte_size, storage.max_upload_bytes)?;

  let ext = file_extension(file_name);
  let mime = ext
    .as_deref()
    .and_then(ext_to_mime)
    .unwrap_or("application/octet-stream")
    .to_string();
  let hash = {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
  };
  let object_key = match &ext {
    Some(ext) => format!("workspaces/{workspace_id}/{hash}.{ext}"),
    None => format!("workspaces/{workspace_id}/{hash}"),
  };

  let upload = storage.presign_put(&object_key);
  let put = client
    .put(&upload.url)
    .header(reqwest::header::CONTENT_TYPE, &mime)
    .body(bytes.to_vec())
    .send()
    .await
    .map_err(|e| ApiError::Internal(format!("storage upload failed: {e}")))?;
  if !put.status().is_success() {
    return Err(ApiError::Internal(format!(
      "storage upload returned {}",
      put.status()
    )));
  }

  store::insert_file(
    &state.db,
    workspace_id,
    user_id,
    &object_key,
    &safe_file_name(file_name),
    &mime,
    byte_size,
  )
  .await
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

/// Lowercase alphanumeric extension of [file_name], or None. Ignores any query
/// string / fragment so URLs like `a.png?x=1` still resolve to `png`.
fn file_extension(file_name: &str) -> Option<String> {
  let path = file_name.split(['?', '#']).next().unwrap_or(file_name);
  let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
  let ext = base.rsplit_once('.')?.1;
  if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
    return None;
  }
  Some(ext.to_ascii_lowercase())
}

fn mime_to_ext(mime: &str) -> Option<&'static str> {
  match mime {
    "image/png" => Some("png"),
    "image/jpeg" => Some("jpg"),
    "image/gif" => Some("gif"),
    "image/webp" => Some("webp"),
    "image/bmp" => Some("bmp"),
    "image/svg+xml" => Some("svg"),
    "image/avif" => Some("avif"),
    _ => None,
  }
}

fn ext_to_mime(ext: &str) -> Option<&'static str> {
  match ext {
    "png" => Some("image/png"),
    "jpg" | "jpeg" => Some("image/jpeg"),
    "gif" => Some("image/gif"),
    "webp" => Some("image/webp"),
    "bmp" => Some("image/bmp"),
    "svg" => Some("image/svg+xml"),
    "avif" => Some("image/avif"),
    _ => None,
  }
}

/// Derive a display filename from a URL's last path segment (falling back to
/// `image.<ext>`).
fn url_file_name(url: &str, ext: Option<&str>) -> String {
  let path = url.split(['?', '#']).next().unwrap_or(url);
  let base = path.rsplit('/').next().unwrap_or("");
  if base.contains('.') && !base.is_empty() {
    base.to_string()
  } else {
    match ext {
      Some(ext) => format!("image.{ext}"),
      None => "image".to_string(),
    }
  }
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

/// Tidy a client/URL file name for use as display/export metadata: strip any
/// directory, keep letters/digits of ANY script (so Chinese & English survive)
/// plus `-`, `_`, `.`, and replace every other char (spaces, parentheses,
/// punctuation) with a single `_`. Keeps Markdown link targets clean —
/// `My Photo (1).png` → `My_Photo_1.png`, `我的照片 v2.png` → `我的照片_v2.png`.
fn safe_file_name(name: &str) -> String {
  let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
  let mut out = String::new();
  let mut prev_underscore = false;
  for ch in base.chars() {
    if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
      out.push(ch);
      prev_underscore = ch == '_';
    } else if !prev_underscore {
      out.push('_');
      prev_underscore = true;
    }
  }
  // Drop underscores hugging the extension dot, then trim the ends.
  let tidy = out.replace("_.", ".").replace("._", ".");
  let trimmed = tidy.trim_matches('_').to_string();
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
  fn safe_file_name_keeps_unicode_drops_specials() {
    assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
    // Spaces/parentheses → underscores (collapsed, trimmed off the extension).
    assert_eq!(safe_file_name("my file (1).PNG"), "my_file_1.PNG");
    // Chinese (and English) letters/digits are preserved.
    assert_eq!(safe_file_name("photos/我的照片.png"), "我的照片.png");
    assert_eq!(safe_file_name("我的照片 v2.png"), "我的照片_v2.png");
    assert_eq!(safe_file_name("  "), "file");
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
