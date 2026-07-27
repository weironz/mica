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
use crate::routes::files::{mime_to_ext, storage};

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
  // The extension lookup doubles as the format check: a MIME we cannot name an
  // extension for is one we are not willing to serve back.
  let ext = mime_to_ext(&mime).ok_or_else(|| {
    ApiError::BadRequest("avatar must be a PNG, JPEG, GIF, WebP or AVIF image".to_string())
  })?;

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
  let client = reqwest::Client::new();
  let upload = storage.presign_put(&object_key);
  let put = client
    .put(&upload.url)
    .header(reqwest::header::CONTENT_TYPE, &mime)
    .body(body.to_vec())
    .send()
    .await
    .map_err(|e| ApiError::Internal(format!("storage upload failed: {e}")))?;
  if !put.status().is_success() {
    return Err(ApiError::Internal(format!(
      "storage upload returned {}",
      put.status()
    )));
  }

  let previous = swap_avatar_key(&state, user_id, Some(&object_key)).await?;
  discard_object(&state, previous.as_deref(), &object_key).await;

  Ok(Json(AvatarResponse {
    avatar_version: Some(avatar_version(&object_key)),
  }))
}

/// `DELETE /api/auth/me/avatar` — go back to the initial-letter circle.
pub async fn delete_avatar(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<AvatarResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let previous = swap_avatar_key(&state, user_id, None).await?;
  discard_object(&state, previous.as_deref(), "").await;
  Ok(Json(AvatarResponse {
    avatar_version: None,
  }))
}

/// Point the user at `key` (or at nothing) and return what they pointed at
/// before, so the caller can clean up the object it just replaced.
async fn swap_avatar_key(
  state: &AppState,
  user_id: Uuid,
  key: Option<&str>,
) -> ApiResult<Option<String>> {
  // Read-then-write rather than one clever RETURNING: the only writer of a
  // user's own avatar is that user, and a self-join whose "old" alias silently
  // depends on snapshot semantics is not worth the round trip it saves.
  let previous: Option<(Option<String>,)> =
    sqlx::query_as("SELECT avatar_key FROM users WHERE id = $1")
      .bind(user_id)
      .fetch_optional(&state.db)
      .await?;
  let previous = previous.ok_or(ApiError::Unauthorized)?.0;

  sqlx::query("UPDATE users SET avatar_key = $1, updated_at = now() WHERE id = $2")
    .bind(key)
    .bind(user_id)
    .execute(&state.db)
    .await?;

  Ok(previous)
}

/// Delete the object a user no longer points at.
///
/// Nothing else can reference it — the key is content-addressed under that one
/// user's prefix — so there is no GC sweep to defer to, and an orphan here would
/// simply never be collected by anything. Best-effort: the row is already
/// updated, and failing the request because the cleanup failed would tell the
/// user their avatar did not change when it did.
async fn discard_object(state: &AppState, previous: Option<&str>, keeping: &str) {
  let Some(previous) = previous else { return };
  if previous.is_empty() || previous == keeping {
    return;
  }
  let Ok(storage) = storage(state) else { return };
  let url = storage.presign_delete(previous);
  if let Err(error) = reqwest::Client::new().delete(&url).send().await {
    tracing::warn!(%previous, %error, "avatar: could not delete the replaced object");
  }
}

/// `GET /api/users/{user_id}/avatar`
///
/// A stable link to a user's picture: 302 to a freshly-signed storage URL on
/// every request, so the link itself never expires. Unauthenticated — see the
/// module docs; kept public by `auth::is_avatar_path`, pinned by that module's
/// `avatar_read_route_is_public` test.
pub async fn get_avatar(State(state): State<AppState>, Path(user_id): Path<Uuid>) -> Response {
  let Ok(storage) = storage(&state) else {
    return StatusCode::NOT_FOUND.into_response();
  };
  let key: Option<(Option<String>,)> = sqlx::query_as("SELECT avatar_key FROM users WHERE id = $1")
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

  #[test]
  fn a_key_of_an_unexpected_shape_still_yields_something() {
    // Never panics and never returns the empty string for a non-empty key: an
    // empty version would serialize as a present-but-blank field, which reads
    // as "has a picture" to the client.
    assert!(!avatar_version("legacy-key-with-no-slashes").is_empty());
    assert!(!avatar_version("a").is_empty());
  }
}
