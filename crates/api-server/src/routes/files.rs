use std::net::IpAddr;
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
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  let storage = storage(&state)?;

  validate_mime(&payload.mime_type)?;
  ensure_storable(&state, workspace_id, payload.byte_size, storage.max_upload_bytes).await?;

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
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  let storage = storage(&state)?;

  validate_mime(&payload.mime_type)?;
  // Checked again here, not just at presign: presign is advisory (a client can
  // skip it, or reuse one URL) and this is the call that creates the row.
  ensure_storable(&state, workspace_id, payload.byte_size, storage.max_upload_bytes).await?;
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
  let user_id = user_id_from_headers(&state, &headers).await?;
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

/// True for addresses an import fetch must never reach: loopback, private,
/// link-local (incl. the 169.254.169.254 cloud-metadata IP), CGNAT, and their
/// IPv6 equivalents. Keeps `import-url` from being turned into an SSRF probe of
/// the server's own network.
fn is_blocked_addr(ip: IpAddr) -> bool {
  match ip {
    IpAddr::V4(v4) => {
      let o = v4.octets();
      v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
        || o[0] == 0
        // CGNAT 100.64.0.0/10
        || (o[0] == 100 && (64..=127).contains(&o[1]))
    }
    IpAddr::V6(v6) => {
      if let Some(mapped) = v6.to_ipv4_mapped() {
        return is_blocked_addr(IpAddr::V4(mapped));
      }
      let seg0 = v6.segments()[0];
      v6.is_loopback()
        || v6.is_unspecified()
        // ULA fc00::/7
        || (seg0 & 0xfe00) == 0xfc00
        // link-local fe80::/10
        || (seg0 & 0xffc0) == 0xfe80
    }
  }
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
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  let file = fetch_and_store_image_url(&state, workspace_id, user_id, payload.url.trim()).await?;
  let download_url = storage(&state)?.download_url(&file.object_key);
  Ok(Json(FileResponse { file, download_url }))
}

/// Fetch an external image URL server-side (SSRF-guarded, redirects off, 20 s
/// timeout) and store it, returning the stored file. Shared by the `import-url`
/// endpoint and the workspace-import re-host of external image links. Returns an
/// error (never panics) on an unreachable host — a CN-hosted server routinely
/// cannot reach medium/imgur/… — so the caller decides whether to keep the link.
pub(crate) async fn fetch_and_store_image_url(
  state: &AppState,
  workspace_id: Uuid,
  user_id: Uuid,
  url: &str,
) -> ApiResult<store::FileRecord> {
  let storage = storage(state)?;

  let url = url.trim();
  let parsed =
    reqwest::Url::parse(url).map_err(|_| ApiError::BadRequest("url must be http(s)".to_string()))?;
  if !matches!(parsed.scheme(), "http" | "https") {
    return Err(ApiError::BadRequest("url must be http(s)".to_string()));
  }
  // SSRF guard: resolve the host and refuse any loopback/private/link-local
  // target (127/8, 10/8, 172.16-31/12, 192.168/16, 169.254/16 incl. the cloud
  // metadata IP, CGNAT 100.64/10, ::1, fc00::/7, fe80::/10, IPv4-mapped v6).
  // The `reqwest` client below re-resolves, so this is a best-effort screen (a
  // rebinding host could still slip a later lookup), paired with redirects off
  // so a public URL cannot 30x-bounce onto an internal address.
  let resolved = parsed
    .socket_addrs(|| match parsed.scheme() {
      "https" => Some(443),
      _ => Some(80),
    })
    .map_err(|_| {
      ApiError::BadRequest(
        "could not fetch the image url: DNS or network unreachable from this server".to_string(),
      )
    })?;
  if resolved.is_empty() || resolved.iter().any(|addr| is_blocked_addr(addr.ip())) {
    return Err(ApiError::BadRequest(
      "refusing to fetch that url: it resolves to a private or loopback address".to_string(),
    ));
  }

  let client = reqwest::Client::builder()
    .timeout(Duration::from_secs(20))
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .map_err(|e| ApiError::Internal(e.to_string()))?;

  // Say WHY. `map_err(|_| ..)` threw the cause away, so a server that simply
  // cannot reach the host (blocked/DNS-poisoned CDN — routine for a CN-hosted
  // server pulling from medium/imgur/…) was indistinguishable from a bad URL,
  // and the UI could only shrug. The client falls back to loading the url
  // itself when this fails, so this is a diagnostic, not a dead end.
  let response = client.get(url).send().await.map_err(|e| {
    let why = if e.is_timeout() {
      "timed out — this server may have no route to that host"
    } else if e.is_connect() {
      "connection failed — DNS or network unreachable from this server"
    } else if e.is_redirect() {
      "too many redirects"
    } else {
      "request failed"
    };
    ApiError::BadRequest(format!("could not fetch the image url: {why}"))
  })?;
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
  ensure_storable(state, workspace_id, byte_size, storage.max_upload_bytes).await?;

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

  let original_name = name_with_ext(&url_file_name(url, ext.as_deref()), ext.as_deref());
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

  Ok(file)
}

/// `GET /api/workspaces/{workspace_id}/files/{file_id}/blob`
/// `GET /api/workspaces/{workspace_id}/files/{file_id}/blob/{filename}`
///
/// A stable, never-expiring public link to an image's bytes — it 302-redirects
/// to a freshly-signed storage URL on every request, so the link itself never
/// goes stale. Unauthenticated (the `file_id` UUID is the capability), so copied
/// Markdown images keep displaying in other apps. Used for copy/export.
/// Kept public by `auth::is_blob_path`; `auth`'s own `image_blob_link_is_public`
/// test guards it.
///
/// The optional trailing filename is COSMETIC — it is ignored entirely (the
/// file_id alone resolves the bytes). It exists because a url ending in `/blob`
/// tells a human, a browser's "save as", or a renderer keying off the extension
/// nothing about being a PNG; `…/blob/diagram.png` does. Same shape as a GitHub
/// raw url. The bare `/blob` form stays valid so links already copied out keep
/// working.
pub async fn blob(
  State(state): State<AppState>,
  Path((workspace_id, file_id)): Path<(Uuid, Uuid)>,
) -> Response {
  blob_inner(state, workspace_id, file_id).await
}

pub async fn blob_named(
  State(state): State<AppState>,
  Path((workspace_id, file_id, _filename)): Path<(Uuid, Uuid, String)>,
) -> Response {
  blob_inner(state, workspace_id, file_id).await
}

async fn blob_inner(state: AppState, workspace_id: Uuid, file_id: Uuid) -> Response {
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
  let user_id = user_id_from_headers(&state, &headers).await?;
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
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let deleted = store::delete_file(&state.db, workspace_id, file_id).await?;
  if !deleted {
    return Err(ApiError::NotFound);
  }

  Ok(Json(json!({ "deleted": true })))
}

pub(crate) fn storage(state: &AppState) -> ApiResult<Arc<S3Config>> {
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
  ensure_storable(state, workspace_id, byte_size, storage.max_upload_bytes).await?;

  // The name is the first guess at the type; the BYTES are the authority when it
  // gives nothing usable (an extension-less name used to land as
  // `application/octet-stream`, which no browser renders as an image).
  let ext = file_extension(file_name).or_else(|| sniff_image_ext(bytes).map(str::to_string));
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
    &name_with_ext(file_name, ext.as_deref()),
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

/// May this workspace store `byte_size` more bytes?
///
/// Two limits answering different questions: `max_upload_bytes` caps ONE file (a
/// 4 GB upload is a mistake whoever sends it), the quota caps the workspace total
/// (a thousand legal 50 MB files is not a mistake — it is a full disk). Neither
/// implies the other, which is why the per-file check alone left the node with no
/// bound at all.
///
/// Every path that stores bytes goes through here — presign, complete, import-url
/// and the workspace-import re-host. One function rather than a check pasted at
/// four sites: the fourth is precisely the one that gets forgotten.
///
/// Refuses with a machine-readable `workspace_quota_exceeded` so the client can
/// say something specific ("this workspace is full") rather than relay a sentence.
/// `quota <= 0` means unlimited and skips the query — an operator who turned the
/// limit off should not pay for it on every upload.
async fn ensure_storable(
  state: &AppState,
  workspace_id: Uuid,
  byte_size: i64,
  max_upload_bytes: i64,
) -> ApiResult<()> {
  validate_byte_size(byte_size, max_upload_bytes)?;

  let quota = state.config.workspace_quota_bytes;
  if quota <= 0 {
    return Ok(());
  }
  let used = store::workspace_bytes_used(&state.db, workspace_id).await?;
  // `saturating_add`: both sides are i64 from the wire or the database, and an
  // overflow here would wrap negative and PASS the check.
  if used.saturating_add(byte_size) > quota {
    return Err(ApiError::BadRequestCode(
      "workspace_quota_exceeded",
      format!(
        "workspace storage is full: {used} of {quota} bytes used, \
         this upload needs {byte_size} more"
      ),
    ));
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

pub(crate) fn mime_to_ext(mime: &str) -> Option<&'static str> {
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
///
/// Tests "does it have a USABLE extension", not "does it contain a dot": an
/// AppFlowy blob url ends in `<base64>=.` — it contains a dot but the dot is
/// bare, so taking it verbatim produced names like `0QYX…X2M.` with no
/// extension at all (and an extension-less object key). Keep the stem and
/// append the real extension instead.
fn url_file_name(url: &str, ext: Option<&str>) -> String {
  let path = url.split(['?', '#']).next().unwrap_or(url);
  let base = path.rsplit('/').next().unwrap_or("");
  if file_extension(base).is_some() {
    return base.to_string();
  }
  let stem = base.trim_end_matches('.');
  match (stem.is_empty(), ext) {
    (false, Some(ext)) => format!("{stem}.{ext}"),
    (false, None) => stem.to_string(),
    (true, Some(ext)) => format!("image.{ext}"),
    (true, None) => "image".to_string(),
  }
}

/// The image type [bytes] actually is, by magic number — the authority when a
/// name/url can't be trusted for it. A url whose last segment carries no
/// extension otherwise lands as `application/octet-stream`, which browsers
/// refuse to render as an image.
fn sniff_image_ext(bytes: &[u8]) -> Option<&'static str> {
  match bytes {
    [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some("png"),
    [0xff, 0xd8, 0xff, ..] => Some("jpg"),
    [b'G', b'I', b'F', b'8', ..] => Some("gif"),
    [b'B', b'M', ..] => Some("bmp"),
    // RIFF....WEBP
    [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
    // ....ftypavif
    [_, _, _, _, b'f', b't', b'y', b'p', b'a', b'v', b'i', b'f', ..] => Some("avif"),
    _ if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") => Some("svg"),
    _ => None,
  }
}

/// A display name that always carries a usable extension: sanitize, then append
/// [ext] when the name itself has none. Without this an image shows up in
/// exports/downloads as an extension-less blob even though we know its type.
fn name_with_ext(name: &str, ext: Option<&str>) -> String {
  let safe = safe_file_name(name);
  if file_extension(&safe).is_some() {
    return safe;
  }
  match ext {
    Some(ext) => format!("{}.{ext}", safe.trim_end_matches('.')),
    None => safe,
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
  // Drop underscores hugging the extension dot, then trim the ends. Trailing
  // dots go too: a name ending in `.` (AppFlowy's `<base64>=.`) is an
  // extension-less name wearing an extension's clothes.
  let tidy = out.replace("_.", ".").replace("._", ".");
  let trimmed = tidy.trim_matches('_').trim_end_matches('.').to_string();
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
    // A bare trailing dot is not an extension — it must not survive (it shipped
    // names like `0QYX…X2M.` from AppFlowy's `<base64>=.` urls).
    assert_eq!(safe_file_name("0QYXMbZ8=."), "0QYXMbZ8");
    assert_eq!(safe_file_name("...."), "file");
  }

  #[test]
  fn url_file_name_requires_a_usable_extension_not_just_a_dot() {
    // Normal url → last segment verbatim.
    assert_eq!(url_file_name("https://e.com/a/photo.png", Some("png")), "photo.png");
    // THE BUG: AppFlowy's blob url ends in `<base64>=.` — contains a dot, but
    // the dot is bare. The stem is kept and the real extension appended.
    assert_eq!(
      url_file_name("https://beta.appflowy.cloud/v1/blob/x/0QYXMbZ8=.", Some("png")),
      "0QYXMbZ8=.png"
    );
    // No extension at all in the segment → same treatment.
    assert_eq!(url_file_name("https://e.com/a/0QYXMbZ8", Some("png")), "0QYXMbZ8.png");
    // Nothing usable → the generic name.
    assert_eq!(url_file_name("https://e.com/", Some("png")), "image.png");
    assert_eq!(url_file_name("https://e.com/", None), "image");
  }

  #[test]
  fn name_with_ext_guarantees_an_extension() {
    // Already has one → untouched (beyond sanitizing).
    assert_eq!(name_with_ext("我的照片.png", Some("png")), "我的照片.png");
    // Missing/bare-dot → the known extension is appended, no double dot.
    assert_eq!(name_with_ext("0QYXMbZ8=.", Some("png")), "0QYXMbZ8.png");
    assert_eq!(name_with_ext("noext", Some("jpg")), "noext.jpg");
    // Unknown type → left as-is rather than inventing an extension.
    assert_eq!(name_with_ext("noext", None), "noext");
  }

  #[test]
  fn sniff_image_ext_reads_magic_numbers() {
    assert_eq!(sniff_image_ext(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
    assert_eq!(sniff_image_ext(b"\xff\xd8\xffrest"), Some("jpg"));
    assert_eq!(sniff_image_ext(b"GIF89a"), Some("gif"));
    assert_eq!(sniff_image_ext(b"RIFF____WEBPrest"), Some("webp"));
    assert_eq!(sniff_image_ext(b"____ftypavif"), Some("avif"));
    assert_eq!(sniff_image_ext(b"<svg xmlns="), Some("svg"));
    // Not an image → no guess (the caller then keeps octet-stream).
    assert_eq!(sniff_image_ext(b"just text"), None);
    assert_eq!(sniff_image_ext(b""), None);
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

  #[test]
  fn ssrf_guard_blocks_private_and_metadata_addresses() {
    let blocked = [
      "127.0.0.1",         // loopback
      "0.0.0.0",           // unspecified
      "10.1.2.3",          // private A
      "172.16.0.1",        // private B
      "172.31.255.255",    // private B (upper)
      "192.168.1.1",       // private C
      "169.254.169.254",   // link-local / cloud metadata
      "100.64.0.1",        // CGNAT
      "::1",               // v6 loopback
      "::",                // v6 unspecified
      "fc00::1",           // v6 ULA
      "fd12:3456::1",      // v6 ULA
      "fe80::1",           // v6 link-local
      "::ffff:127.0.0.1",  // v4-mapped loopback
      "::ffff:169.254.169.254", // v4-mapped metadata
    ];
    for s in blocked {
      let ip: IpAddr = s.parse().unwrap();
      assert!(is_blocked_addr(ip), "{s} should be blocked");
    }

    let allowed = ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"];
    for s in allowed {
      let ip: IpAddr = s.parse().unwrap();
      assert!(!is_blocked_addr(ip), "{s} should be allowed");
    }
  }
}

/// The quota's accounting, against a real database.
///
/// `ensure_storable` needs an `AppState` (config + pool + storage), so what gets
/// pinned here is the part that can be wrong in a way nobody notices: what
/// `workspace_bytes_used` COUNTS. The refusal itself is a `>` on two integers; the
/// interesting question is whether those integers describe the disk.
#[cfg(test)]
mod quota_pg {
  use mica_app_core::store;
  use sqlx::PgPool;
  use uuid::Uuid;

  /// Skipping without a database is a local convenience; skipping WITH one is a
  /// lie, and in CI a missing one means the workflow regressed.
  async fn pool() -> Option<PgPool> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
      assert!(
        std::env::var("CI").is_err(),
        "DATABASE_URL is unset in CI — the postgres service block regressed; \
         these tests must not silently pass"
      );
      return None;
    };
    Some(
      PgPool::connect(&url)
        .await
        .expect("DATABASE_URL is set but the connection failed"),
    )
  }

  async fn seed_workspace(db: &PgPool) -> (Uuid, Uuid) {
    let user = Uuid::new_v4();
    let ws = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
      .bind(user)
      .bind(format!("{user}@quota.test"))
      .execute(db)
      .await
      .unwrap();
    sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'Q',$2)")
      .bind(ws)
      .bind(user)
      .execute(db)
      .await
      .unwrap();
    (ws, user)
  }

  async fn put(db: &PgPool, ws: Uuid, user: Uuid, key: &str, bytes: i64) {
    store::insert_file(db, ws, user, key, "f.png", "image/png", bytes)
      .await
      .unwrap();
  }

  /// Remove only what this test created (workspaces cascade to their files).
  async fn cleanup(db: &PgPool, workspaces: &[Uuid], users: &[Uuid]) {
    for w in workspaces {
      sqlx::query("DELETE FROM workspaces WHERE id=$1")
        .bind(w)
        .execute(db)
        .await
        .ok();
    }
    for u in users {
      sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(u)
        .execute(db)
        .await
        .ok();
    }
  }

  #[tokio::test]
  async fn usage_sums_this_workspace_only_and_dedups() {
    let Some(db) = pool().await else { return };
    let (ws, user) = seed_workspace(&db).await;
    let (other, other_user) = seed_workspace(&db).await;

    assert_eq!(
      store::workspace_bytes_used(&db, ws).await.unwrap(),
      0,
      "a fresh workspace occupies nothing"
    );

    let k1 = format!("ws/{ws}/a-{}", Uuid::new_v4());
    let k2 = format!("ws/{ws}/b-{}", Uuid::new_v4());
    put(&db, ws, user, &k1, 1000).await;
    put(&db, ws, user, &k2, 2500).await;
    assert_eq!(store::workspace_bytes_used(&db, ws).await.unwrap(), 3500);

    // Content-addressed keys dedup: the same bytes uploaded twice are ONE row, so
    // re-uploading an identical image must not consume quota twice. Otherwise the
    // quota would punish the case dedup exists to make free.
    put(&db, ws, user, &k1, 1000).await;
    assert_eq!(
      store::workspace_bytes_used(&db, ws).await.unwrap(),
      3500,
      "a duplicate upload adds a reference, not size"
    );

    // Another workspace's files are not this workspace's problem — a per-instance
    // sum here would let one workspace exhaust everyone else.
    put(
      &db,
      other,
      other_user,
      &format!("ws/{other}/c-{}", Uuid::new_v4()),
      9_000_000,
    )
    .await;
    assert_eq!(
      store::workspace_bytes_used(&db, ws).await.unwrap(),
      3500,
      "the quota is per workspace, not per instance"
    );

    // The arithmetic `ensure_storable` performs, stated once so the intent is
    // pinned even though building its `AppState` is out of reach here.
    let used = store::workspace_bytes_used(&db, ws).await.unwrap();
    assert!(used + 1000 > 4000, "a 1000-byte upload must not fit in 4000");
    assert!(used + 500 <= 4000, "but 500 bytes must");

    cleanup(&db, &[ws, other], &[user, other_user]).await;
  }

  /// Unreferenced-but-not-yet-swept files still occupy the disk, so they still
  /// count. Otherwise trash-and-reupload is an unbounded loop that never trips the
  /// quota while the bytes pile up until the GC's grace period lapses.
  #[tokio::test]
  async fn bytes_awaiting_the_blob_gc_still_count() {
    let Some(db) = pool().await else { return };
    let (ws, user) = seed_workspace(&db).await;
    let key = format!("ws/{ws}/dead-{}", Uuid::new_v4());
    put(&db, ws, user, &key, 4242).await;
    sqlx::query("UPDATE files SET unreferenced_since = now() WHERE object_key = $1")
      .bind(&key)
      .execute(&db)
      .await
      .unwrap();

    assert_eq!(
      store::workspace_bytes_used(&db, ws).await.unwrap(),
      4242,
      "the bytes are still on the disk until the sweep removes them"
    );

    cleanup(&db, &[ws], &[user]).await;
  }
}
