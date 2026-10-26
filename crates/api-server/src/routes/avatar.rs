//! Profile pictures.
//!
//! Deliberately outside the workspace file pipeline (see `0015_user_avatar.sql`
//! for why): the bytes go straight to `avatars/{user_id}/{sha256}.{ext}` and the
//! only record is `users.avatar_key`.
//!
//! Reading is a public redirect, mirroring `files::blob` — not because an avatar
//! is a shared capability link, but because it has to be loadable by an `<img>`
//! tag, and the web client keeps its token in localStorage where an `<img>`
//! cannot reach it. Requiring auth here would mean no avatar ever renders in a
//! browser.

use axum::{
  Json,
  body::Bytes,
  extract::{Path, State},
  http::{HeaderMap, StatusCode},
  response::{IntoResponse, Redirect, Response},
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult};

use crate::routes::auth::user_id_from_headers;
use crate::routes::files::{ext_to_mime, mime_to_ext, sniff_image_ext, storage};

/// Avatars are displayed at 32 px. Four megabytes is already absurdly generous
/// for that, and the point of a cap well under the workspace upload limit is
/// that a profile picture is never the thing worth spending a 25 MB round trip
/// on — the user gets told, rather than waiting out an upload that only makes a
/// tiny circle.
const MAX_AVATAR_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct AvatarResponse {
  avatar_version: Option<String>,
}

/// A short token that changes exactly when the picture does.
///
/// The avatar URL is stable by design (it is derived from the user id), so
/// without this a replaced picture would keep showing the old bytes out of every
/// image cache between the object store and the widget. Derived FROM the key
/// rather than stored alongside it — one source of truth, so the two can never
/// disagree about which picture is current.
pub(crate) fn avatar_version(avatar_key: &str) -> String {
  let name = avatar_key.rsplit('/').next().unwrap_or(avatar_key);
  let stem = name.split('.').next().unwrap_or(name);
  stem.chars().take(16).collect()
}

/// Serde bridge for row structs that carry the raw key: emits the version, so
/// the key itself never leaves the server and the derivation stays in one place.
pub(crate) fn serialize_version<S: serde::Serializer>(
  key: &Option<String>,
  serializer: S,
) -> Result<S::Ok, S::Error> {
  match key {
    Some(key) => serializer.serialize_some(&avatar_version(key)),
    None => serializer.serialize_none(),
  }
}

/// `PUT /api/auth/me/avatar` — replace the signed-in user's picture.
///
/// The body is the raw image bytes; the `Content-Type` header names the format.
/// Not multipart: there is exactly one part, and multipart would add a parser
/// (and a dependency) to carry a single blob.
pub async fn put_avatar(
  State(state): State<AppState>,
  headers: HeaderMap,
  body: Bytes,
) -> ApiResult<Json<AvatarResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let storage = storage(&state)?;

  if body.is_empty() {
    return Err(ApiError::BadRequest("image body is empty".to_string()));
  }
  if body.len() > MAX_AVATAR_BYTES {
    return Err(ApiError::BadRequest(format!(
      "avatar must be at most {} MB",
      MAX_AVATAR_BYTES / (1024 * 1024)
    )));
  }

  let mime = headers
    .get(axum::http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .map(|s| s.split(';').next().unwrap_or(s).trim().to_ascii_lowercase())
    .unwrap_or_default();
  let (ext, content_type) = avatar_format(&mime, &body)?;

  let hash: String = {
    let mut hasher = Sha256::new();
    hasher.update(&body);
    hasher
      .finalize()
      .iter()
      .map(|b| format!("{b:02x}"))
      .collect()
  };
  let object_key = format!("avatars/{user_id}/{hash}.{ext}");

  // Same self-issued presigned PUT the importer uses (files::store_bytes).
  let client = crate::http_client::outbound_client();

  // One account, one avatar mutation at a time (#152). Every step of it is
  // serialized against the same account's other avatar writes: this
  // transaction holds the lock across upload and row swap, and
  // `discard_object` takes it again for the cleanup — with the current
  // reference re-read under it. The two lock windows are consecutive per
  // request; the gap between them is harmless because the cleanup trusts the
  // re-read rather than its own stale snapshot.
  let mut tx = state.db.begin().await?;
  lock_avatar(&mut tx, user_id).await?;
  // #153: a deleted account's JWT still verifies until it expires, so the row
  // itself is the liveness gate — checked BEFORE the object is written.
  let previous = live_avatar_key(&mut tx, user_id).await?;

  // Server-side upload: this process sends the PUT, so it must be signed for
  // the address THIS process can reach, not the browser's.
  let upload = storage.presign_put_server(&object_key);
  let put = client
    .put(&upload.url)
    .header(reqwest::header::CONTENT_TYPE, &content_type)
    .body(body.to_vec())
    .send()
    .await
    .map_err(|e| ApiError::Internal(format!("storage upload failed: {}", e.without_url())))?;
  if !put.status().is_success() {
    return Err(ApiError::Internal(format!(
      "storage upload returned {}",
      put.status()
    )));
  }

  set_avatar_key(&mut tx, user_id, Some(&object_key)).await?;
  tx.commit().await?;

  discard_object(&state, user_id, previous.as_deref()).await;

  Ok(Json(AvatarResponse {
    avatar_version: Some(avatar_version(&object_key)),
  }))
}

/// The formats an avatar may be: raster only.
///
/// Not `mime_to_ext`'s whole list — that one also names SVG, which is right for
/// workspace files (they are stored as `application/octet-stream` by
/// `stored_content_type`) and wrong here: an avatar is a PUBLIC 302 to the
/// storage origin, and an SVG served there is a script-capable document.
const AVATAR_EXTS: [&str; 6] = ["png", "jpg", "gif", "webp", "avif", "bmp"];

/// The extension and stored `Content-Type` for an avatar upload.
///
/// The header has to claim a raster image, and the BYTES have to be one — by
/// magic number, the same sniff the importer trusts over a url. The stored type
/// is derived from the sniff, never copied from the request: what the storage
/// origin answers with is ours to decide, not the uploader's.
fn avatar_format(declared: &str, bytes: &[u8]) -> ApiResult<(&'static str, String)> {
  let refuse =
    || ApiError::BadRequest("avatar must be a PNG, JPEG, GIF, WebP, AVIF or BMP image".to_string());
  mime_to_ext(declared)
    .filter(|ext| AVATAR_EXTS.contains(ext))
    .ok_or_else(refuse)?;
  let ext = sniff_image_ext(bytes)
    .filter(|ext| AVATAR_EXTS.contains(ext))
    .ok_or_else(refuse)?;
  let mime = ext_to_mime(ext).ok_or_else(refuse)?;
  Ok((ext, mica_infra::stored_content_type(mime)))
}

/// Whether a stored key names a format [AVATAR_EXTS] allows. Keys written
/// before S1 may end in `.svg` (stored with `image/svg+xml`); those are not
/// redirected to, so an existing upload cannot keep serving script.
fn is_servable_avatar_key(key: &str) -> bool {
  key
    .rsplit_once('.')
    .is_some_and(|(_, ext)| AVATAR_EXTS.contains(&ext))
}

/// `DELETE /api/auth/me/avatar` — go back to the initial-letter circle.
pub async fn delete_avatar(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<AvatarResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let mut tx = state.db.begin().await?;
  lock_avatar(&mut tx, user_id).await?;
  let previous = live_avatar_key(&mut tx, user_id).await?;
  set_avatar_key(&mut tx, user_id, None).await?;
  tx.commit().await?;
  discard_object(&state, user_id, previous.as_deref()).await;
  Ok(Json(AvatarResponse {
    avatar_version: None,
  }))
}

/// Take the per-account avatar mutex for the rest of [tx]. Transaction-scoped
/// (`pg_advisory_xact_lock`): a cancelled or failed request releases it with
/// the rollback, so a hung client cannot wedge one account's avatar forever.
///
/// The salted prefix keeps this namespace apart from the workspace-tree lock in
/// `documents` (which hashes a bare UUID with the same transform): account ids
/// and workspace ids are both UUIDs, so without the salt unrelated writes would
/// serialize against each other.
async fn lock_avatar(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  user_id: Uuid,
) -> ApiResult<()> {
  sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
    .bind(format!("avatar:{user_id}"))
    .execute(&mut **tx)
    .await?;
  Ok(())
}

/// The live account's current avatar key, or `None` when it has no picture.
///
/// #153: `deleted_at IS NULL` makes a tombstone read as "no such account" (401)
/// rather than as a user whose picture can be replaced by an unexpired JWT.
async fn live_avatar_key(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  user_id: Uuid,
) -> ApiResult<Option<String>> {
  let previous: Option<(Option<String>,)> =
    sqlx::query_as("SELECT avatar_key FROM users WHERE id = $1 AND deleted_at IS NULL")
      .bind(user_id)
      .fetch_optional(&mut **tx)
      .await?;
  Ok(previous.ok_or(ApiError::Unauthorized)?.0)
}

/// Point a live account at `key` (or at nothing). #153: the write itself is
/// gated on `deleted_at IS NULL`, and a zero-row UPDATE (the account was deleted
/// after the read) is rejected rather than silently rewriting a tombstone.
async fn set_avatar_key(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  user_id: Uuid,
  key: Option<&str>,
) -> ApiResult<()> {
  let updated = sqlx::query(
    "UPDATE users SET avatar_key = $1, updated_at = now() WHERE id = $2 AND deleted_at IS NULL",
  )
  .bind(key)
  .bind(user_id)
  .execute(&mut **tx)
  .await?;
  if updated.rows_affected() == 0 {
    return Err(ApiError::Unauthorized);
  }
  Ok(())
}

/// Delete the object an account no longer points at.
///
/// Nothing else can reference it — the key is content-addressed under that one
/// user's prefix — so there is no GC sweep to defer to, and an orphan here would
/// simply never be collected by anything. Best-effort: the row is already
/// updated, and failing the request because the cleanup failed would tell the
/// user their avatar did not change when it did.
///
/// #152: the decision is re-checked under the same per-account lock every swap
/// takes, with the store DELETE issued inside that lock. Two overlapping PUTs
/// cannot then leave the row pointing at an object a slower cleanup deletes.
async fn discard_object(state: &AppState, user_id: Uuid, previous: Option<&str>) {
  let Some(previous) = previous.filter(|key| !key.is_empty()) else {
    return;
  };
  let Ok(storage) = storage(state) else { return };
  let mut tx = match state.db.begin().await {
    Ok(tx) => tx,
    Err(_) => return,
  };
  if lock_avatar(&mut tx, user_id).await.is_err() {
    return;
  }
  // Fail CLOSED on a read error: not being able to see the current reference
  // is not evidence that `previous` is unused. Only a successful read that
  // positively points elsewhere may authorize the delete; a missing row
  // (account gone) still cleans up, since nothing can reference the object.
  let recheck =
    sqlx::query_as::<_, (Option<String>,)>("SELECT avatar_key FROM users WHERE id = $1")
      .bind(user_id)
      .fetch_optional(&mut *tx)
      .await;
  let current = match recheck {
    Ok(row) => row.map(|(key,)| key),
    Err(error) => {
      tracing::warn!(%previous, %error, "avatar: could not re-read the avatar key; keeping the object");
      return;
    }
  };
  if current.flatten().as_deref() == Some(previous) {
    return; // a later request re-selected this very object; leave it
  }
  let url = storage.presign_delete(previous);
  if let Err(error) = crate::http_client::outbound_client()
    .delete(&url)
    .send()
    .await
  {
    tracing::warn!(%previous, %error, "avatar: could not delete the replaced object");
  }
  let _ = tx.commit().await;
}

/// `GET /api/users/{user_id}/avatar`
///
/// A stable link to a user's picture: 302 to a freshly-signed storage URL on
/// every request, so the link itself never expires. Unauthenticated — see the
/// module docs; mounted without the scope guard in
/// `routes::public_api_router`, pinned by the router-level probe tests there
/// (`route_layer_tests` in routes/mod.rs).
pub async fn get_avatar(State(state): State<AppState>, Path(user_id): Path<Uuid>) -> Response {
  let Ok(storage) = storage(&state) else {
    return StatusCode::NOT_FOUND.into_response();
  };
  // #153: a tombstone reads as "no picture", not as a redirect to whatever an
  // unexpired-but-dead token may have written into the row.
  let key: Option<(Option<String>,)> =
    sqlx::query_as("SELECT avatar_key FROM users WHERE id = $1 AND deleted_at IS NULL")
      .bind(user_id)
      .fetch_optional(&state.db)
      .await
      .ok()
      .flatten();
  match key.and_then(|(k,)| k) {
    // 404 rather than a placeholder image: "no picture set" is the client's
    // call to render (it draws the initial), and a served placeholder would
    // make every avatar-less account look like a broken upload.
    None => (StatusCode::NOT_FOUND, Json(json!({"error": "no avatar"}))).into_response(),
    // A pre-S1 SVG avatar reads as "no picture" (the client draws the
    // initial) rather than as a link to a script-capable document.
    Some(key) if !is_servable_avatar_key(&key) => {
      (StatusCode::NOT_FOUND, Json(json!({"error": "no avatar"}))).into_response()
    }
    Some(key) => Redirect::temporary(&storage.download_url(&key)).into_response(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // The URL is stable by design, so this token is the ONLY thing that tells a
  // cache the picture changed. If it stopped varying with the bytes, a replaced
  // avatar would keep showing the old face and nothing would report an error.
  #[test]
  fn the_version_changes_with_the_bytes() {
    let a = avatar_version("avatars/55b3e5ff-4117-4d65-9434-0b17922d8e87/abc123def4567890aa.png");
    let b = avatar_version("avatars/55b3e5ff-4117-4d65-9434-0b17922d8e87/ffff123def4567890a.png");
    assert_ne!(a, b);
    assert_eq!(a, "abc123def4567890");
  }

  #[test]
  fn the_same_bytes_keep_the_same_version() {
    // Same content re-uploaded: the key is content-addressed, so nothing about
    // the displayed picture changed and the cache should not be busted.
    let key = "avatars/55b3e5ff-4117-4d65-9434-0b17922d8e87/abc123def4567890aa.jpg";
    assert_eq!(avatar_version(key), avatar_version(key));
    // The extension is not part of the identity — the same bytes served as
    // .jpg or .jpeg are the same picture.
    assert_eq!(
      avatar_version(key),
      avatar_version("avatars/55b3e5ff-4117-4d65-9434-0b17922d8e87/abc123def4567890aa.jpeg")
    );
  }

  const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
  const JPEG: &[u8] = b"\xff\xd8\xff\xe0\0\x10JFIF";
  const SVG: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>";

  /// S1 (review 2026-10-21): an avatar is served by a PUBLIC 302 straight to the
  /// storage origin, with the Content-Type it was uploaded with. Accepting
  /// `image/svg+xml` there meant a script-carrying document on that origin for
  /// anyone holding the link — the hole `stored_content_type` closed for
  /// workspace files (B4), reopened through the avatar path.
  #[test]
  fn an_svg_avatar_is_refused() {
    assert!(matches!(
      avatar_format("image/svg+xml", SVG),
      Err(ApiError::BadRequest(_))
    ));
  }

  /// The header is the client's claim; the bytes are the file. SVG bytes
  /// declared as PNG must not get through by lying about the type.
  #[test]
  fn svg_bytes_under_a_raster_label_are_refused() {
    assert!(matches!(
      avatar_format("image/png", SVG),
      Err(ApiError::BadRequest(_))
    ));
    assert!(matches!(
      avatar_format("image/png", b"<?xml version=\"1.0\"?><svg/>"),
      Err(ApiError::BadRequest(_))
    ));
  }

  /// And the stored type follows the bytes too, not the header — what the
  /// storage origin answers with is decided here, by what the file IS.
  #[test]
  fn the_stored_type_comes_from_the_bytes() {
    let (ext, content_type) = avatar_format("image/png", PNG).unwrap();
    assert_eq!((ext, content_type.as_str()), ("png", "image/png"));
    let (ext, content_type) = avatar_format("image/png", JPEG).unwrap();
    assert_eq!((ext, content_type.as_str()), ("jpg", "image/jpeg"));
  }

  #[test]
  fn a_non_image_header_is_still_refused() {
    assert!(matches!(
      avatar_format("text/html", PNG),
      Err(ApiError::BadRequest(_))
    ));
  }

  /// The existing-data half: an SVG avatar stored before the upload check
  /// existed must stop being redirected to.
  #[test]
  fn a_legacy_svg_key_is_not_served() {
    let user = "55b3e5ff-4117-4d65-9434-0b17922d8e87";
    assert!(!is_servable_avatar_key(&format!(
      "avatars/{user}/abc123.svg"
    )));
    assert!(is_servable_avatar_key(&format!(
      "avatars/{user}/abc123.png"
    )));
    assert!(is_servable_avatar_key(&format!(
      "avatars/{user}/abc123.jpg"
    )));
  }

  #[test]
  fn a_key_of_an_unexpected_shape_still_yields_something() {
    // Never panics and never returns the empty string for a non-empty key: an
    // empty version would serialize as a present-but-blank field, which reads
    // as "has a picture" to the client.
    assert!(!avatar_version("legacy-key-with-no-slashes").is_empty());
    assert!(!avatar_version("a").is_empty());
  }
}
