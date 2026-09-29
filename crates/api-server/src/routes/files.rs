use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
  Json,
  extract::{Path, State},
  http::{HeaderMap, StatusCode},
  response::{IntoResponse, Redirect, Response},
};
use futures_util::StreamExt;
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

/// The upload a client should perform, or the row that means none is needed.
///
/// `upload` and `existing` are alternatives, and exactly one is set. They are
/// separate fields rather than a nullable `upload_url` because the client needs
/// to tell "there is nothing to PUT" apart from "the server forgot the URL" —
/// and because the dedup answer, when it applies, is the whole response: the
/// caller wants the file id, not an upload it should skip.
#[derive(Debug, Serialize)]
pub struct PresignResponse {
  object_key: String,
  /// Present when the client must PUT. Absent when the bytes are already stored.
  #[serde(skip_serializing_if = "Option::is_none")]
  upload: Option<PresignUpload>,
  /// Present when the bytes are already stored: the existing record, so the
  /// caller can use the file immediately without an upload round-trip.
  #[serde(skip_serializing_if = "Option::is_none")]
  existing: Option<FileResponse>,
}

/// The fields of a presigned upload the client needs.
#[derive(Debug, Serialize)]
pub struct PresignUpload {
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
///
/// When the object is ALREADY recorded, no NEW URL is issued — `existing` names
/// the row instead. Previously issued URLs remain usable until expiry; this
/// check alone does not make the object immutable (see P1-02 in the plan).
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

  // Same key already recorded → hand back the row and issue no new URL. The
  // normal client skips the PUT; an older signed URL can still be replayed.
  if let Some(existing) = store::fetch_file_by_key(&state.db, workspace_id, &object_key).await? {
    let download_url = storage.download_url(&existing.object_key);
    return Ok(Json(existing_object_response(object_key, existing, download_url)));
  }

  let upload = storage.presign_put(&object_key);

  Ok(Json(PresignResponse {
    object_key,
    upload: Some(PresignUpload {
      upload_url: upload.url,
      method: upload.method,
      expires_in: upload.expires_in,
      max_byte_size: storage.max_upload_bytes,
    }),
    existing: None,
  }))
}

/// The response for bytes the store already holds: the recorded row, and NO
/// upload.
///
/// Split out to assert that dedup issues no new URL without needing a database
/// and object store. A URL issued before this record existed remains valid.
fn existing_object_response(
  object_key: String,
  existing: store::FileRecord,
  download_url: String,
) -> PresignResponse {
  PresignResponse {
    object_key,
    upload: None,
    existing: Some(FileResponse {
      file: existing,
      download_url,
    }),
  }
}

/// `POST /api/workspaces/{workspace_id}/files/complete`
///
/// Records metadata after a successful upload and returns a URL for reading the
/// object (used as an image block's `url`).
///
/// The object is CHECKED against the store before a row is written. A presigned
/// upload URL is usable by whoever holds it, for whatever bytes they like, so
/// everything the client says here — that it uploaded anything, and that it was
/// this many bytes — is a claim. The `files` row is what the workspace quota is
/// summed from, so an unverified claim is a number the client gets to choose.
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

  // Ask the store what actually landed, and require it to agree with the claim.
  // Without this, `complete` could be called for a key nothing was ever written
  // to (a row pointing at nothing) or with a `byte_size` unrelated to the bytes
  // (a quota that counts a number the client invented).
  let head = reqwest::Client::new()
    .head(storage.presign_head_object(&payload.object_key))
    .send()
    .await
    .map_err(|e| ApiError::Internal(format!("storage check failed: {e}")))?;
  let stored_len = head
    .headers()
    .get(reqwest::header::CONTENT_LENGTH)
    .and_then(|v| v.to_str().ok())
    .and_then(|s| s.parse::<i64>().ok());
  let byte_size =
    uploaded_object_verdict(head.status().as_u16(), stored_len, payload.byte_size)?;

  let file = store::insert_file(
    &state.db,
    workspace_id,
    user_id,
    &payload.object_key,
    &safe_file_name(&payload.file_name),
    &payload.mime_type,
    byte_size,
  )
  .await?;
  let download_url = storage.download_url(&file.object_key);

  Ok(Json(FileResponse { file, download_url }))
}

/// Decide whether the object a client claims to have uploaded is really there and
/// really that size.
///
/// Pure, so the three answers that matter — absent, wrong size, agreed — are
/// testable without a store or a network.
///
/// Fails CLOSED when the store reports no length. That is the deliberate half:
/// accepting the client's number whenever the store withholds its own would make
/// the whole check decorative, and "we could not verify" must not read the same
/// as "verified".
fn uploaded_object_verdict(
  status: u16,
  stored_len: Option<i64>,
  declared_len: i64,
) -> ApiResult<i64> {
  if status == 404 {
    return Err(ApiError::BadRequest(
      "no object was uploaded for this key".to_string(),
    ));
  }
  if !(200..300).contains(&status) {
    return Err(ApiError::Internal(format!(
      "storage returned {status} when checking the uploaded object"
    )));
  }
  let Some(stored) = stored_len else {
    return Err(ApiError::BadRequest(
      "storage did not report the uploaded object's size, so the upload cannot be verified"
        .to_string(),
    ));
  };
  if stored != declared_len {
    return Err(ApiError::BadRequest(format!(
      "uploaded object is {stored} bytes but {declared_len} was declared"
    )));
  }
  Ok(stored)
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

/// Append one streamed chunk to `body`, refusing as soon as the running total
/// would pass `max_bytes`.
///
/// Split out so the rule is testable without a network or an `AppState`: this is
/// the whole of the memory bound, and the interesting cases (a chunk that lands
/// exactly on the cap, one that crosses it, an empty chunk) need no HTTP.
///
/// `saturating_add` because both lengths are usize-derived; an overflow here
/// would wrap small and PASS the check.
fn append_within_cap(body: &mut Vec<u8>, chunk: &[u8], max_bytes: i64) -> ApiResult<()> {
  let total = (body.len() as i64).saturating_add(chunk.len() as i64);
  if total > max_bytes {
    return Err(ApiError::BadRequest(format!(
      "image is too large: it exceeds the {max_bytes} byte limit"
    )));
  }
  body.extend_from_slice(chunk);
  Ok(())
}

/// Is this resolved address list safe to connect to?
///
/// The RULE, separated from the resolver so it can be tested with a hand-written
/// list — which is the only way to cover the case it exists for. A hostname that
/// answers with BOTH a public and a private address is exactly how a DNS
/// rebinding attempt looks, and it cannot be reached from a test that resolves a
/// real name (let alone an IP literal, which yields a single address).
///
/// Refusing when ANY address is blocked — rather than dropping the blocked ones
/// and keeping the rest — is the point: connecting to the public answer would
/// leave the private one live for the next lookup, which is the attack. The whole
/// name is refused.
///
/// An EMPTY list is also refused. That is not paranoia: it means resolution
/// produced nothing usable, and treating "no addresses" as "no blocked
/// addresses" would let an empty answer through to a client that then resolves
/// on its own.
fn addresses_are_safe(resolved: &[std::net::SocketAddr]) -> bool {
  !resolved.is_empty() && !resolved.iter().any(|addr| is_blocked_addr(addr.ip()))
}

/// Resolve `url`'s host and return the addresses an import fetch is allowed to
/// connect to, refusing the request if the resolution is unusable.
///
/// Split out from the handler so the handler's job is only to hand these
/// addresses to `reqwest::ClientBuilder::resolve_to_addrs`, which is what pins
/// the connection to what was checked.
fn vetted_pinned_addrs(
  url: &reqwest::Url,
  port: u16,
) -> ApiResult<Vec<std::net::SocketAddr>> {
  let resolved = url.socket_addrs(|| url.port_or_known_default()).map_err(|_| {
    ApiError::BadRequest(
      "could not fetch the image url: DNS or network unreachable from this server".to_string(),
    )
  })?;
  if !addresses_are_safe(&resolved) {
    return Err(ApiError::BadRequest(
      "refusing to fetch that url: it resolves to a private or loopback address".to_string(),
    ));
  }
  // Re-state each address with the port the request will actually use:
  // `socket_addrs` resolves against the URL's default, which is not necessarily
  // the port an explicit `:8443` in the url names.
  Ok(
    resolved
      .iter()
      .map(|addr| std::net::SocketAddr::new(addr.ip(), port))
      .collect(),
  )
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
  let host = parsed
    .host_str()
    .ok_or_else(|| ApiError::BadRequest("url must have a host".to_string()))?
    .to_string();

  // SSRF guard: resolve the host ONCE and refuse any loopback/private/link-local
  // target (127/8, 10/8, 172.16-31/12, 192.168/16, 169.254/16 incl. the cloud
  // metadata IP, CGNAT 100.64/10, ::1, fc00::/7, fe80::/10, IPv4-mapped v6).
  //
  // Resolving and then letting reqwest resolve AGAIN is the bug this used to
  // have: a host whose DNS answers are attacker-controlled (or simply short-TTL)
  // can return a public address to the check and a private one to the connection
  // — classic DNS rebinding, and it makes the screen above decorative. So the
  // vetted addresses are PINNED onto the client below via `resolve_to_addrs`, and
  // the request connects to one of THESE or fails. Redirects stay off, so a
  // public URL cannot 30x-bounce onto an internal address either.
  //
  // The SNI/Host header still carries the original name (reqwest sets it from the
  // URL, not from the resolved address), so TLS and virtual hosting are
  // unaffected — which is what makes pinning viable for CDNs rather than a
  // rejection of them.
  let port = parsed.port_or_known_default().unwrap_or(80);
  let pinned = vetted_pinned_addrs(&parsed, port)?;

  // 8s, not 20. This timeout is only ever spent on a fetch that is going to
  // fail — a reachable host answers in well under a second, and an unreachable
  // one stays unreachable. The old 20s was paid PER IMAGE, in series, during an
  // import: one unreachable image host turned a 235-page import into hours of
  // waiting for answers that were never coming.
  let client = reqwest::Client::builder()
    .timeout(Duration::from_secs(8))
    .redirect(reqwest::redirect::Policy::none())
    .resolve_to_addrs(&host, &pinned)
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

  // Cheap up-front refusal when the server volunteers its size. This is NOT the
  // enforcement — a lying or chunked response can omit or understate it — but it
  // avoids reading a body we already know is too big.
  if let Some(declared) = response
    .headers()
    .get(reqwest::header::CONTENT_LENGTH)
    .and_then(|v| v.to_str().ok())
    .and_then(|s| s.parse::<i64>().ok())
    && declared > storage.max_upload_bytes
  {
    return Err(ApiError::BadRequest(format!(
      "image is too large: {declared} bytes exceeds the {} byte limit",
      storage.max_upload_bytes
    )));
  }

  // Stream, counting as we go, and STOP the moment the cap is passed. The old
  // `response.bytes().await` buffered the entire body first and only then asked
  // whether it was allowed — so a hostile or merely enormous image source made
  // the API hold all of it in memory before refusing, once per request. The
  // declared `Content-Length` above cannot replace this: it is remote input.
  let mut body: Vec<u8> = Vec::new();
  let mut stream = response.bytes_stream();
  while let Some(chunk) = stream.next().await {
    let chunk =
      chunk.map_err(|_| ApiError::BadRequest("could not read the image url".to_string()))?;
    append_within_cap(&mut body, &chunk, storage.max_upload_bytes)?;
  }
  let bytes = body;
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
  let upload = storage.presign_put_server(&object_key);
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

  let upload = storage.presign_put_server(&object_key);
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

/// The two rejections here are not the same kind of thing.
///
/// A non-positive size is a client bug — nobody can act on it, so it stays a
/// generic `bad_request`; inventing friendly copy for it would be lying about a
/// failure we have not characterised.
///
/// Exceeding the per-file cap IS actionable ("compress it, or pick a smaller
/// one"), and it is the refusal most easily mistaken for the quota: a workspace
/// with 4 GiB free still rejects one oversized file, so the storage bar honestly
/// reads "plenty of room" while the upload fails. Without a code of its own the
/// client could only relay the English sentence or match on its text — the
/// second representation `docs/design-adoption.md` draws a line against. So it
/// gets `file_too_large`, a sibling to `workspace_quota_exceeded`, and the two
/// full-sounding failures become distinguishable without reading prose.
fn validate_byte_size(byte_size: i64, max_upload_bytes: i64) -> ApiResult<()> {
  if byte_size <= 0 {
    return Err(ApiError::BadRequest(
      "byte_size must be positive".to_string(),
    ));
  }
  if byte_size > max_upload_bytes {
    return Err(ApiError::BadRequestCode(
      "file_too_large",
      format!("file exceeds the maximum upload size of {max_upload_bytes} bytes"),
    ));
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

  /// P1-04: the streaming read must refuse the moment the cap is passed, so the
  /// body never grows past `max_bytes` in memory.
  ///
  /// The bug was `response.bytes().await` followed by `ensure_storable` — the
  /// whole remote body landed in RAM before anyone asked whether it was allowed,
  /// once per request. A remote source could therefore make the API hold
  /// arbitrarily much before being told no.
  #[test]
  fn streamed_chunks_are_capped_before_they_are_buffered() {
    let cap = 10i64;
    let mut body: Vec<u8> = Vec::new();

    // Under the cap: kept.
    append_within_cap(&mut body, b"12345", cap).unwrap();
    assert_eq!(body.len(), 5);

    // EXACTLY the cap: allowed — the bound is "> cap refuses", not ">= cap".
    append_within_cap(&mut body, b"67890", cap).unwrap();
    assert_eq!(body.len(), 10, "a body that lands exactly on the cap is fine");

    // One byte past: refused, and the buffer is left as it was — the extra byte
    // is NOT appended before the check runs.
    let err = append_within_cap(&mut body, b"x", cap).unwrap_err();
    assert!(matches!(err, ApiError::BadRequest(_)));
    assert_eq!(body.len(), 10, "the oversized chunk is not buffered");

    // The refusal is about the TOTAL, not one chunk: many small chunks crossing
    // the cap are caught too (a chunked response has no useful Content-Length).
    let mut body2: Vec<u8> = vec![0; 9];
    assert!(append_within_cap(&mut body2, b"12", cap).is_err());
    assert_eq!(body2.len(), 9);

    // Empty chunks are harmless (they happen on keep-alives).
    let mut body3: Vec<u8> = Vec::new();
    append_within_cap(&mut body3, b"", cap).unwrap();
    assert!(body3.is_empty());
  }

  /// P1-02: `complete` must believe the STORE, not the client.
  ///
  /// A presigned upload URL is usable by whoever holds it, for whatever bytes
  /// they like, so "I uploaded N bytes" is a claim — and `files.byte_size` is
  /// what the workspace quota is summed from. Accepting the claim let a client
  /// write a row pointing at an object that was never uploaded, and pick the
  /// number the quota counts.
  ///
  /// The `None` case is the one worth arguing about: a store that reports no
  /// length is a store we could not check, and "could not verify" must not read
  /// the same as "verified". Failing closed there is what keeps the check from
  /// being decorative on a non-conforming endpoint.
  #[test]
  fn complete_requires_the_store_to_confirm_the_upload() {
    // There is nothing there → refused, not recorded.
    assert!(matches!(
      uploaded_object_verdict(404, None, 10),
      Err(ApiError::BadRequest(_))
    ));

    // A store error is not "verified" either — and it is an Internal (our side /
    // the store's), not a BadRequest blaming the caller.
    assert!(matches!(
      uploaded_object_verdict(500, Some(10), 10),
      Err(ApiError::Internal(_))
    ));
    assert!(matches!(
      uploaded_object_verdict(403, Some(10), 10),
      Err(ApiError::Internal(_))
    ));

    // Present and the right size → the STORED length is what gets recorded.
    assert_eq!(uploaded_object_verdict(200, Some(10), 10).unwrap(), 10);

    // Present but a different size → refused. Both directions: a client that
    // understates (to slip past the quota) and one that overstates.
    assert!(matches!(
      uploaded_object_verdict(200, Some(9999), 10),
      Err(ApiError::BadRequest(_))
    ));
    assert!(matches!(
      uploaded_object_verdict(200, Some(10), 9999),
      Err(ApiError::BadRequest(_))
    ));

    // The store withheld a length → fail CLOSED rather than fall back to the
    // client's number.
    assert!(
      matches!(
        uploaded_object_verdict(200, None, 10),
        Err(ApiError::BadRequest(_))
      ),
      "an unverifiable upload must not be recorded as verified"
    );

    // Zero is a real size, not a missing one: an empty object that the client
    // also declared empty is accepted.
    assert_eq!(uploaded_object_verdict(200, Some(0), 0).unwrap(), 0);
    // ...but zero stored against a non-zero claim is still a mismatch.
    assert!(matches!(
      uploaded_object_verdict(200, Some(0), 5),
      Err(ApiError::BadRequest(_))
    ));
  }

  /// P1-02, the dedup half: a recorded key must yield NO NEW upload URL.
  ///
  /// This does not revoke a URL issued earlier for the same key. Its replay
  /// remains an open part of P1-02.
  ///
  /// This test is a bit narrow and worth saying so: it pins "the dedup response
  /// carries no upload", which is the guard, but it cannot exercise the database
  /// lookup that decides WHEN to answer that way. A regression that skipped the
  /// lookup entirely (always issuing a URL) would still pass this. The lookup
  /// itself is covered end-to-end by the dedup test in `quota_pg`.
  #[test]
  fn an_existing_object_yields_no_upload_url() {
    let key = format!("workspaces/{}/{}", Uuid::new_v4(), "a".repeat(64));
    let record = store::FileRecord {
      id: Uuid::new_v4(),
      workspace_id: Uuid::new_v4(),
      uploaded_by: Uuid::new_v4(),
      object_key: key.clone(),
      original_name: "photo.png".to_string(),
      mime_type: "image/png".to_string(),
      byte_size: 1234,
      created_at: chrono::Utc::now(),
    };

    let response = existing_object_response(key.clone(), record.clone(), "https://cdn/x".into());

    assert!(
      response.upload.is_none(),
      "no URL may be issued for an object that is already stored — that URL is \
       the whole mechanism for overwriting it"
    );
    let existing = response.existing.expect("the existing row is returned");
    assert_eq!(existing.file.id, record.id, "so the caller can use it as-is");
    assert_eq!(existing.file.object_key, key);

    // And the serialized shape: the client keys off the ABSENCE of `upload`, so
    // emitting `"upload": null` would be indistinguishable from a server bug
    // that lost the URL. `skip_serializing_if` keeps the two apart.
    let json = serde_json::to_value(PresignResponse {
      object_key: key,
      upload: None,
      existing: Some(FileResponse {
        file: record,
        download_url: "https://cdn/x".to_string(),
      }),
    })
    .unwrap();
    assert!(
      json.get("upload").is_none(),
      "an omitted upload must be ABSENT, not null: got {json}"
    );
    assert!(json.get("existing").is_some());
  }

  /// P1-02: an address is vetted ONCE and pinned, so a later lookup cannot swap
  /// in a private target. This pins the vetting rule itself; that the handler
  /// hands the result to `resolve_to_addrs` is what makes it binding, and is
  /// asserted by the literal `resolve_to_addrs(&host, &pinned)` in `import_url`.
  ///
  /// The metadata IP is called out because it is the highest-value target: a
  /// fetch to 169.254.169.254 on a cloud instance returns credentials.
  #[test]
  fn ssrf_vetting_refuses_private_and_metadata_targets() {
    let port = 443u16;
    // Public → allowed, and the returned address carries the REQUEST's port.
    let public = reqwest::Url::parse("https://1.1.1.1/img.png").unwrap();
    let pinned = vetted_pinned_addrs(&public, port).unwrap();
    assert_eq!(pinned, vec!["1.1.1.1:443".parse().unwrap()]);

    // Every blocked family this guard claims to cover, through the same entry
    // point the handler uses (so a future refactor cannot quietly bypass one).
    for blocked in [
      "http://127.0.0.1/img.png",       // loopback
      "http://10.1.2.3/img.png",        // private 10/8
      "http://192.168.1.1/img.png",     // private 192.168/16
      "http://172.16.5.4/img.png",      // private 172.16/12
      "http://169.254.169.254/latest/meta-data/", // cloud metadata
      "http://100.64.0.1/img.png",      // CGNAT
      "http://0.0.0.0/img.png",         // unspecified
      "http://[::1]/img.png",           // IPv6 loopback
      "http://[fd00::1]/img.png",       // IPv6 ULA
      "http://[fe80::1]/img.png",       // IPv6 link-local
      "http://[::ffff:127.0.0.1]/img.png", // IPv4-mapped loopback
    ] {
      let url = reqwest::Url::parse(blocked).unwrap();
      let result = vetted_pinned_addrs(&url, port);
      assert!(
        matches!(result, Err(ApiError::BadRequest(_))),
        "{blocked} must be refused, got {result:?}"
      );
    }
  }

  /// P1-03, the half the URL-level test above CANNOT reach: a hostname that
  /// resolves to a public AND a private address.
  ///
  /// Every case in the test above is an IP literal, which resolves to exactly one
  /// address — and on a one-element list "refuse if ANY is blocked" and "refuse if
  /// ALL are blocked" are the same predicate. An earlier version of that test
  /// therefore passed unchanged when the rule was weakened from `any` to `all`,
  /// i.e. it did not cover the rebinding shape at all despite being named for it.
  /// This test is what actually discriminates, so it feeds the rule a list
  /// directly instead of going through DNS.
  #[test]
  fn ssrf_vetting_refuses_a_host_with_any_private_answer() {
    let public: std::net::SocketAddr = "93.184.216.34:443".parse().unwrap();
    let private: std::net::SocketAddr = "10.0.0.7:443".parse().unwrap();
    let metadata: std::net::SocketAddr = "169.254.169.254:443".parse().unwrap();

    // A public answer alone is fine.
    assert!(addresses_are_safe(&[public]));
    // A private one alone is refused.
    assert!(!addresses_are_safe(&[private]));
    // THE CASE: both, in either order. This is a rebinding host, and connecting
    // to its public answer would leave the private one available to the next
    // lookup — so the whole name must be refused, not merely filtered.
    assert!(
      !addresses_are_safe(&[public, private]),
      "a host with a private answer must be refused even when it also answers publicly"
    );
    assert!(
      !addresses_are_safe(&[private, public]),
      "order must not matter"
    );
    assert!(
      !addresses_are_safe(&[public, public, metadata]),
      "the metadata IP hiding among public answers must still be caught"
    );
    // An empty resolution is refused, not treated as vacuously safe.
    assert!(
      !addresses_are_safe(&[]),
      "an empty answer must not be read as 'no blocked addresses'"
    );
  }

  /// The refusal a person reads is chosen off the `code` FIELD OF THE JSON, not
  /// off the enum variant — `clients/mica_flutter/lib/api/client.dart` `_decode`
  /// lifts `body['code']` and `main.dart` `_apiMessage` switches on the literal
  /// strings `file_too_large` / `workspace_quota_exceeded`.
  ///
  /// That is a contract across two languages with nothing but this test holding
  /// the ends together. Rename a code here and nothing in Rust complains; the
  /// Dart switch just stops matching and silently falls back to relaying the
  /// English sentence — the exact failure the codes were introduced to end, and
  /// one no compiler on either side can see. So the serialized bytes are
  /// asserted, not the variant.
  #[tokio::test]
  async fn the_size_refusals_serialize_the_codes_the_client_switches_on() {
    use axum::response::IntoResponse;

    async fn code_of(err: ApiError) -> String {
      let body = err.into_response().into_body();
      let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
      let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
      json["code"].as_str().unwrap().to_string()
    }

    assert_eq!(
      code_of(validate_byte_size(101, 100).unwrap_err()).await,
      "file_too_large",
      "main.dart `_apiMessage` matches this exact string"
    );

    // Its sibling, asserted here too so the pair cannot drift apart: these are
    // the two refusals a user cannot tell apart from the storage bar alone.
    assert_eq!(
      code_of(ApiError::BadRequestCode(
        "workspace_quota_exceeded",
        "x".into()
      ))
      .await,
      "workspace_quota_exceeded"
    );

    // A generic rejection must NOT gain a specific code: `_apiMessage` falls
    // through to the raw message for these on purpose, because friendly copy for
    // an uncharacterised failure is a lie.
    assert_eq!(
      code_of(validate_byte_size(0, 100).unwrap_err()).await,
      "bad_request"
    );
  }

  #[test]
  fn byte_size_bounds_are_validated() {
    assert!(validate_byte_size(1, 100).is_ok());
    assert!(validate_byte_size(100, 100).is_ok(), "the cap itself fits");
    // A client bug nobody can act on stays generic.
    assert!(matches!(
      validate_byte_size(0, 100),
      Err(ApiError::BadRequest(_))
    ));
    // Too big is actionable, and must be tellable APART from the quota refusal
    // without reading the English message — the client picks its copy off this
    // code alone.
    assert!(
      matches!(
        validate_byte_size(101, 100),
        Err(ApiError::BadRequestCode("file_too_large", _))
      ),
      "the per-file cap must carry its own code, not a generic bad_request"
    );
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

  /// The `mica_storage_bytes` gauges must agree with the expression the quota
  /// actually enforces.
  ///
  /// They are two SQL statements over the same table — a textbook second
  /// representation. If they drift, the gauge answers "who is near the quota
  /// wall" with a number that is not the one doing the refusing, which is the
  /// only question it exists to answer. So they are pinned to each other here
  /// rather than trusted to stay in step.
  #[tokio::test]
  async fn the_capacity_gauge_matches_what_the_quota_enforces() {
    let Some(db) = pool().await else { return };
    let (ws, user) = seed_workspace(&db).await;
    let (small, small_user) = seed_workspace(&db).await;
    put(&db, ws, user, "gauge-big-a", 700).await;
    put(&db, ws, user, "gauge-big-b", 300).await;
    put(&db, small, small_user, "gauge-small-a", 40).await;

    let enforced = store::workspace_bytes_used(&db, ws).await.unwrap();
    assert_eq!(enforced, 1000);

    // The aggregate the exposition renders (metrics.rs `load_db_snapshot`).
    let (total, max): (i64, i64) = sqlx::query_as(
      r#"SELECT
           coalesce((SELECT sum(total) FROM
             (SELECT sum(byte_size) AS total FROM files GROUP BY workspace_id) t), 0)::bigint,
           coalesce((SELECT max(total) FROM
             (SELECT sum(byte_size) AS total FROM files GROUP BY workspace_id) t), 0)::bigint"#,
    )
    .fetch_one(&db)
    .await
    .unwrap();

    // Written as inequalities on purpose: the test database is shared, so other
    // workspaces may exist. What must hold regardless is that the aggregate
    // COUNTS this workspace the same way the quota does — a `WHERE` clause
    // drifting apart from the quota's would break both of these.
    assert!(
      max >= enforced,
      "the largest workspace cannot be smaller than one we just measured ({max} < {enforced})"
    );
    assert!(
      total >= enforced + 40,
      "the total must include every workspace, including the small one ({total})"
    );

    cleanup(&db, &[ws, small], &[user, small_user]).await;
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

  /// The two byte-counting expressions in this codebase must stay DIFFERENT, and
  /// must differ by exactly the unreferenced bytes.
  ///
  /// They read like the same query with a stray `WHERE`, which is why someone
  /// periodically proposes "unifying" them. They answer different questions:
  ///
  /// - `store::workspace_bytes_used` — what the disk holds. The quota refuses on
  ///   it and `GET /workspaces/{id}/usage` DISPLAYS it. Both halves of the
  ///   "used / quota" ratio come from this one, or the ratio itself is wrong.
  /// - `export_all_stats` (routes/documents.rs) — what the export zip would
  ///   CONTAIN. An unreferenced blob is awaiting the GC sweep and no live page
  ///   points at it, so it is not in the archive.
  ///
  /// Collapsing them breaks whichever side loses: counting unreferenced bytes in
  /// the export overstates a download the user is about to wait for, and dropping
  /// them from the quota reopens the trash-and-reupload loop that
  /// `bytes_awaiting_the_blob_gc_still_count` exists to close.
  ///
  /// So what is pinned here is the RELATIONSHIP, not equality.
  #[tokio::test]
  async fn the_quota_counts_bytes_the_export_estimate_leaves_out() {
    let Some(db) = pool().await else { return };
    let (ws, user) = seed_workspace(&db).await;

    let live = format!("ws/{ws}/live-{}", Uuid::new_v4());
    let dead = format!("ws/{ws}/dead-{}", Uuid::new_v4());
    put(&db, ws, user, &live, 1000).await;
    put(&db, ws, user, &dead, 250).await;
    sqlx::query("UPDATE files SET unreferenced_since = now() WHERE object_key = $1")
      .bind(&dead)
      .execute(&db)
      .await
      .unwrap();

    // What the quota enforces AND what /usage reports — the same call the handler
    // makes (routes/workspaces.rs `workspace_usage`), not a copy of its SQL.
    let enforced = store::workspace_bytes_used(&db, ws).await.unwrap();

    // The byte predicate `export_all_stats` runs, scoped to this workspace rather
    // than the caller's memberships so the two numbers are comparable.
    let in_archive = sqlx::query_scalar::<_, i64>(
      r#"SELECT coalesce(sum(f.byte_size), 0)::bigint
         FROM files f
         WHERE f.workspace_id = $1
           AND f.unreferenced_since IS NULL"#,
    )
    .bind(ws)
    .fetch_one(&db)
    .await
    .unwrap();

    assert_eq!(
      enforced, 1250,
      "the quota counts every byte on the disk, swept or not"
    );
    assert_eq!(
      in_archive, 1000,
      "the export estimate counts only blobs a live page still references"
    );
    assert_eq!(
      enforced - in_archive,
      250,
      "the gap is exactly the bytes awaiting the GC sweep; if it is ever zero by \
       construction, one of the two counts was collapsed into the other"
    );

    cleanup(&db, &[ws], &[user]).await;
  }
}
