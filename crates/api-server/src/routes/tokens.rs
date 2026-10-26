//! Personal Access Token management: create / list / revoke.
//!
//! The plaintext token (`mica_pat_<hex>`) is returned ONCE on creation; only its
//! SHA-256 hash is stored. Auth + scope enforcement for these routes is handled
//! by the `scope_guard` middleware (create/revoke need `write`, list needs
//! `read`), so a read-only token can list its own tokens but not mint new ones.
//! Minting additionally requires a session JWT no older than the account's
//! credential epoch (#29) — see `create_token`.

use axum::{
  Json,
  extract::{Path, State},
  http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Duration, Utc};
use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::routes::auth::{
  PAT_PREFIX, SESSION_COOKIE, Scope, bearer_token, cookie_value, ensure_pat_mint_allowed_in_tx,
  mint_secret, sha256_hex, user_and_iat_from_token, user_id_from_headers,
};

/// Longest expiry a token may ask for (ten years); omit it for no expiry.
const MAX_TOKEN_DAYS: i64 = 3650;

#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
  name: String,
  /// `read` and/or `write` (write implies read). Defaults to `["read"]`.
  #[serde(default)]
  scopes: Vec<String>,
  /// Days until expiry; omit for a token that never expires.
  expires_in_days: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreatedToken {
  id: Uuid,
  name: String,
  scopes: Vec<String>,
  /// The secret — shown ONCE, never retrievable again.
  token: String,
  created_at: DateTime<Utc>,
  expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct TokenInfo {
  id: Uuid,
  name: String,
  scopes: Vec<String>,
  created_at: DateTime<Utc>,
  last_used_at: Option<DateTime<Utc>>,
  expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct TokenListResponse {
  tokens: Vec<TokenInfo>,
}

/// `POST /api/auth/tokens` — mint a new token (returns the secret once).
///
/// Minting is privileged beyond "has write scope" (#29):
///
/// * the credential must be a **session JWT**, never another PAT — otherwise a
///   short-lived write-scoped PAT converts itself into a permanent one, and a
///   read-only token's "I can list my tokens" stops being a boundary;
/// * the JWT must be no older than the account's credential epoch — a token
///   stolen before a password change (or left over from a deleted account)
///   would otherwise mint a PAT that outlives the rotation by years.
pub async fn create_token(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(req): Json<CreateTokenRequest>,
) -> ApiResult<Json<CreatedToken>> {
  let token = bearer_token(&headers)
    .or_else(|| cookie_value(&headers, SESSION_COOKIE))
    .ok_or(ApiError::Unauthorized)?;
  if token.starts_with(PAT_PREFIX) {
    // 403, not 401: the credential resolves fine — minting is simply not
    // something a PAT may do, and re-authenticating with the same token
    // changes nothing.
    return Err(ApiError::Forbidden);
  }
  let (user_id, iat) = user_and_iat_from_token(&state, &token)?;
  let name = super::names::normalize_name(&req.name, "name")?;

  let scopes: Vec<String> = if req.scopes.is_empty() {
    vec![Scope::Read.as_str().to_string()]
  } else {
    for s in &req.scopes {
      if Scope::parse(s).is_none() {
        return Err(ApiError::BadRequest(format!(
          "unknown scope '{s}' (use 'read' and/or 'write')"
        )));
      }
    }
    req.scopes.clone()
  };

  // Bounded: `Duration::days` and `DateTime + Duration` both PANIC out of
  // range, so a huge `expires_in_days` took the request down instead of being
  // refused.
  let expires_at = match req.expires_in_days {
    None => None,
    Some(d @ 1..=MAX_TOKEN_DAYS) => Some(Utc::now() + Duration::days(d)),
    Some(_) => {
      return Err(ApiError::BadRequest(format!(
        "expires_in_days must be between 1 and {MAX_TOKEN_DAYS}"
      )));
    }
  };

  let token = mint_secret(PAT_PREFIX);
  let token_hash = sha256_hex(&token);

  // The epoch check and the INSERT happen under ONE user-row lock (#123): a
  // password change/reset that commits in between cannot leave a durable PAT
  // that outlived the rotation. If the rotation won the race, the locked row
  // already carries the new epoch and this stale JWT is refused; if this mint
  // won, the rotation waits and its revoke catches the row we insert now.
  let mut tx = state.db.begin().await?;
  let locked = ensure_pat_mint_allowed_in_tx(&mut tx, user_id, iat).await?;
  let (id, created_at) = sqlx::query_as::<_, (Uuid, DateTime<Utc>)>(
    r#"
      INSERT INTO api_tokens (user_id, name, token_hash, scopes, expires_at)
      VALUES ($1, $2, $3, $4, $5)
      RETURNING id, created_at
    "#,
  )
  .bind(locked)
  .bind(&name)
  .bind(&token_hash)
  .bind(&scopes)
  .bind(expires_at)
  .fetch_one(&mut *tx)
  .await?;
  tx.commit().await?;

  Ok(Json(CreatedToken {
    id,
    name,
    scopes,
    token,
    created_at,
    expires_at,
  }))
}

/// `GET /api/auth/tokens` — list the caller's tokens (never the secret).
pub async fn list_tokens(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<TokenListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let tokens = sqlx::query_as::<_, TokenInfo>(
    r#"
      SELECT id, name, scopes, created_at, last_used_at, expires_at
      FROM api_tokens
      WHERE user_id = $1
      ORDER BY created_at DESC
    "#,
  )
  .bind(user_id)
  .fetch_all(&state.db)
  .await?;
  Ok(Json(TokenListResponse { tokens }))
}

/// `DELETE /api/auth/tokens/{id}` — revoke one of the caller's tokens.
pub async fn revoke_token(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let result = sqlx::query("DELETE FROM api_tokens WHERE id = $1 AND user_id = $2")
    .bind(id)
    .bind(user_id)
    .execute(&state.db)
    .await?;
  if result.rows_affected() == 0 {
    return Err(ApiError::NotFound);
  }
  Ok(StatusCode::NO_CONTENT)
}
