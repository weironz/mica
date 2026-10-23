use argon2::{
  Argon2,
  password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
  Json,
  extract::{Request, State},
  http::{
    HeaderMap, HeaderValue, StatusCode,
    header::{AUTHORIZATION, SET_COOKIE},
  },
  middleware::Next,
  response::Response,
};
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use mica_app_core::{AppState, store};
use mica_infra::{ApiError, ApiResult, Environment};

use super::email_verify;
use crate::password_strength;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
  email: String,
  display_name: String,
  password: String,
}

/// Advisory-lock key serializing `register`.
///
/// A fixed number, not derived from anything: the race it closes is between two
/// DIFFERENT addresses on an empty instance, so a per-email or per-request key
/// would let both racers proceed. Change it only if `register` stops being the
/// only holder — a second holder of a DIFFERENT constant would not serialize
/// against this one.
const REGISTRATION_LOCK_KEY: i64 = 0x6d69_6361_7265_6731;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
  email: String,
  password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
  access_token: String,
  token_type: &'static str,
  expires_at: DateTime<Utc>,
  /// Spend this on `/auth/refresh` for a fresh access token. Single-use: the
  /// refresh hands back a new one. Returned in plaintext exactly once — only
  /// its hash is kept.
  refresh_token: String,
  user: UserResponse,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
  /// Optional because web no longer has it: the refresh token lives in an
  /// `HttpOnly` cookie there, so the browser sends it and script never sees it.
  /// Desktop still posts it in the body — it has no cookie jar we control.
  #[serde(default)]
  refresh_token: Option<String>,
}

/// No `used_at` here on purpose: whether the token is already spent is decided
/// by the conditional UPDATE in [refresh], not by reading the value first —
/// reading it would be a check-then-act that two concurrent refreshes could
/// both pass.
#[derive(Debug, FromRow)]
struct RefreshTokenRow {
  user_id: Uuid,
  family_id: Uuid,
  expires_at: DateTime<Utc>,
  revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
  user: UserResponse,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
  id: Uuid,
  email: String,
  display_name: String,
  created_at: DateTime<Utc>,
  /// Changes when the picture does; None means no picture. The client builds the
  /// URL from the user id — sending a URL from here as well would be two
  /// representations of one address, and only one of them could be right.
  avatar_version: Option<String>,
}

#[derive(Debug, FromRow)]
struct UserRow {
  id: Uuid,
  email: String,
  display_name: String,
  password_hash: String,
  created_at: DateTime<Utc>,
  avatar_key: Option<String>,
  /// NULL = the address was never confirmed, so this account cannot sign in.
  /// Every account that predates migration 0018 was grandfathered as verified.
  email_verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
  sub: String,
  exp: usize,
}

/// `POST /api/auth/register`
///
/// Whether the account that was just created can sign in RIGHT NOW.
///
/// The server is the only side that knows: the first account on an empty
/// instance is verified on the spot (see [`register`]), every later one has to
/// confirm its address. This used to be invisible — both cases answered
/// `204 No Content`, so the client had to guess, guessed "check your email",
/// and told the very first operator of a fresh install to wait for a message
/// that was never sent and was never needed. A fact the server acts on has to
/// be a fact the server reports.
#[derive(Debug, Serialize)]
pub struct RegisterResponse {
  pub verified: bool,
}

/// Answers `204 No Content`, not a session: the account cannot be signed in to
/// until its address is confirmed, so handing back tokens would contradict the
/// gate in [`login`]. The client's next step is "check your email".
pub async fn register(
  State(state): State<AppState>,
  Json(payload): Json<RegisterRequest>,
) -> ApiResult<Json<RegisterResponse>> {
  // Registration is CLOSED unless the operator opened it
  // (`MICA_REGISTRATION_ENABLED=true`). Login and refresh for existing users are
  // unaffected — this gate is only about minting NEW accounts.
  //
  // The one exception is a brand-new instance with no users at all. Without it,
  // defaulting to closed would make a fresh self-hosted install unusable: nothing
  // to log in as, and no way to create the first account short of knowing to set
  // an env var first. So the very first account is always allowed, and the door
  // shuts behind it.
  let email = normalize_email(&payload.email)?;
  let display_name = normalize_display_name(&payload.display_name)?;
  // The email and display name go in: the cheapest guess against a specific
  // account is that the password is made out of its own name.
  validate_password(&payload.password, &[&email, &display_name])?;

  let password_hash = hash_password(&payload.password)?;

  // One statement, so "is this the first account?" cannot be answered stale —
  // and `is_admin` is computed from the SAME snapshot as the guard, so the
  // account that stands the instance up is the one that can configure it
  // (migration 0021).
  //
  // The `WHERE NOT EXISTS` guard is NOT sufficient on its own, which is what
  // this transaction and the advisory lock below exist for. A subquery in a
  // data-modifying statement is evaluated against the STATEMENT's snapshot, and
  // under READ COMMITTED two concurrent registrations on an empty table each
  // take their snapshot before either insert is visible — so BOTH see "no users
  // yet", both pass the guard, and the instance ends up with two accounts, the
  // second of which is also `is_admin = true` because it computed the flag from
  // the same empty snapshot. There is no constraint that could have caught it:
  // `users_email_key` only prevents the same ADDRESS twice, and the two racers
  // are registering different addresses by construction.
  //
  // The lock serializes every registration against every other one, so the
  // second transaction's statement snapshot is taken after the first commits and
  // its `NOT EXISTS` genuinely sees the row. It is TRANSACTION-scoped
  // (`pg_advisory_xact_lock`, released at commit/rollback, no unlock to forget)
  // and a constant key: registration is a low-frequency, operator-facing path,
  // so serializing all of it costs nothing measurable, and a per-email key would
  // not help — the race is two DIFFERENT addresses.
  //
  // Extracted so the concurrency test drives THIS function rather than a copy of
  // the SQL: a test that retypes the statement passes whether or not the lock is
  // there, which is the "test can pass in a vacuum" trap docs/dev/lessons.md records.
  let mut tx = state.db.begin().await?;
  let user = insert_registered_user(
    &mut tx,
    &email,
    &display_name,
    &password_hash,
    state.config.registration_enabled,
  )
  .await?;

  // No row = the `WHERE` refused it: registration is closed and this instance
  // already has an account. Same 403 as before, and deliberately no hint about
  // whether the address was taken — a closed door should not answer questions.
  let Some(user) = user else {
    tx.rollback().await?;
    return Err(ApiError::Forbidden);
  };

  // The first-run account is verified on the spot, and this is not a shortcut.
  // It is the operator, on a fresh instance they just started, with nobody to
  // prove anything to — and quite possibly with no mail backend configured yet
  // (`build_mailer` falls back to logging). Requiring them to receive an email
  // from a server that may be unable to send one would turn the bootstrap
  // exception into a locked door, which is the opposite of its purpose.
  if !state.config.registration_enabled {
    sqlx::query("UPDATE users SET email_verified_at = now() WHERE id = $1")
      .bind(user.id)
      .execute(&mut *tx)
      .await?;
    tx.commit().await?;
    return Ok(Json(RegisterResponse { verified: true }));
  }

  tx.commit().await?;

  // Otherwise: the account exists but cannot be signed in to until the address is
  // confirmed. Best-effort mail — the row is already committed, so a mail outage
  // must not fail the request and leave someone holding an account they were told
  // was never created. `resend-verification` is the way back.
  if !email_verify::mint_and_send(&state, user.id, &user.email).await {
    tracing::warn!(user_id = %user.id, "registered but the verification email did not go out");
  }

  // 204, not a session: handing back tokens here would contradict the login gate
  // two lines below. Same shape as `forgot` — "we sent you mail" carries no body.
  Ok(Json(RegisterResponse { verified: false }))
}

pub async fn login(
  State(state): State<AppState>,
  Json(payload): Json<LoginRequest>,
) -> ApiResult<(HeaderMap, Json<AuthResponse>)> {
  let email = normalize_email(&payload.email)?;

  let user = sqlx::query_as::<_, UserRow>(
    r#"
      SELECT id, email, display_name, password_hash, created_at, avatar_key, email_verified_at
      FROM users
      WHERE email = $1 AND deleted_at IS NULL
    "#,
  )
  .bind(email)
  .fetch_optional(&state.db)
  .await?;
  let Some(user) = user else {
    // Pay the same Argon2 verify an existing account pays. Answering an
    // unknown address immediately made "is this email registered?" a timing
    // question any caller could ask — the oracle `forgot` and `register` are
    // careful not to be.
    let _ = verify_password(&payload.password, dummy_password_hash());
    return Err(ApiError::Unauthorized);
  };

  verify_password(&payload.password, &user.password_hash)?;
  // AFTER the password check, on purpose: answering "confirm your email" to a
  // wrong password would tell a stranger the address is registered. Reaching this
  // line means the caller already knows the credentials.
  email_verify::ensure_verified(user.email_verified_at)?;

  let response = auth_response(&state, user).await?;
  Ok((auth_cookie_headers(&state, &response), Json(response)))
}

pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<MeResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;

  let user = sqlx::query_as::<_, UserRow>(
    r#"
      SELECT id, email, display_name, password_hash, created_at, avatar_key, email_verified_at
      FROM users
      WHERE id = $1 AND deleted_at IS NULL
    "#,
  )
  .bind(user_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::Unauthorized)?;

  Ok(Json(MeResponse {
    user: UserResponse::from(user),
  }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateMeRequest {
  display_name: String,
}

/// `PATCH /api/auth/me` — update the signed-in user's display name.
pub async fn update_me(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<UpdateMeRequest>,
) -> ApiResult<Json<MeResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let display_name = normalize_display_name(&payload.display_name)?;

  let user = sqlx::query_as::<_, UserRow>(
    r#"
      UPDATE users
      SET display_name = $1, updated_at = now()
      WHERE id = $2 AND deleted_at IS NULL
      RETURNING id, email, display_name, password_hash, created_at, avatar_key, email_verified_at
    "#,
  )
  .bind(display_name)
  .bind(user_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::Unauthorized)?;

  Ok(Json(MeResponse {
    user: UserResponse::from(user),
  }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserSettingsBody {
  settings: serde_json::Value,
}

/// `GET /api/auth/me/settings` — the signed-in user's synced client settings.
///
/// A user with no row yet gets `{}`, not a 404: "never saved any" and "saved
/// the empty set" are the same thing to a client deciding which toggles to
/// apply, and a 404 would make every fresh account log an error on first boot.
pub async fn get_settings(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<UserSettingsBody>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let settings: Option<(serde_json::Value,)> =
    sqlx::query_as("SELECT settings FROM user_settings WHERE user_id = $1")
      .bind(user_id)
      .fetch_optional(&state.db)
      .await?;
  Ok(Json(UserSettingsBody {
    settings: settings.map_or_else(|| serde_json::json!({}), |(s,)| s),
  }))
}

/// `PUT /api/auth/me/settings` — replace the synced settings blob whole.
///
/// PUT, not PATCH: the client owns the vocabulary and always sends its full
/// synced set, so replace-whole is the true semantics — a merge would resurrect
/// keys a newer client deliberately dropped. Last write wins across devices;
/// settings are low-frequency and single-user, there is no conflict to referee.
pub async fn put_settings(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<UserSettingsBody>,
) -> ApiResult<StatusCode> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  if !payload.settings.is_object() {
    return Err(ApiError::BadRequest(
      "settings must be a JSON object".into(),
    ));
  }
  // A settings blob is a handful of toggles. The cap is against the table
  // quietly becoming somebody's free key-value store, not a real limit anyone
  // legitimate will meet.
  const MAX_BYTES: usize = 16 * 1024;
  if payload.settings.to_string().len() > MAX_BYTES {
    return Err(ApiError::BadRequest("settings too large".into()));
  }
  sqlx::query(
    "INSERT INTO user_settings(user_id, settings, updated_at) VALUES ($1, $2, now())
     ON CONFLICT (user_id) DO UPDATE SET settings = excluded.settings, updated_at = now()",
  )
  .bind(user_id)
  .bind(&payload.settings)
  .execute(&state.db)
  .await?;
  Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
  current_password: String,
  new_password: String,
}

/// `POST /api/auth/password` — change the signed-in user's password.
pub async fn change_password(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<ChangePasswordRequest>,
) -> ApiResult<StatusCode> {
  let user_id = user_id_from_headers(&state, &headers).await?;

  let user = sqlx::query_as::<_, UserRow>(
    r#"
      SELECT id, email, display_name, password_hash, created_at, avatar_key, email_verified_at
      FROM users
      WHERE id = $1 AND deleted_at IS NULL
    "#,
  )
  .bind(user_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::Unauthorized)?;

  verify_password(&payload.current_password, &user.password_hash)?;
  // AFTER the current password is proven, and with this user's own identifiers:
  // the row had to be fetched anyway, and checking first would let an
  // unauthenticated caller probe which passwords the policy accepts.
  validate_password(&payload.new_password, &[&user.email, &user.display_name])?;
  let password_hash = hash_password(&payload.new_password)?;

  sqlx::query(
    r#"
      UPDATE users
      SET password_hash = $1, updated_at = now()
      WHERE id = $2
    "#,
  )
  .bind(password_hash)
  .bind(user_id)
  .execute(&state.db)
  .await?;

  // Changing a password is what people do when they think someone else is in
  // their account, so every other sign-in has to die with it — otherwise the
  // intruder keeps a 30-day refresh token and the password change achieves
  // nothing. The caller's own session dies too and it simply signs in again;
  // that is cheaper than reasoning about which family is "this" device.
  revoke_user_sessions(&state.db, user_id).await?;

  Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct DeleteAccountRequest {
  /// Current password — deleting an account is irreversible, so it is gated the
  /// same way `change_password` is.
  pub password: String,
}

/// `DELETE /api/auth/me` — permanently delete the caller's account and ALL data
/// they OWN: every workspace they own (its documents, versions, files, shares,
/// CRDT base + history all cascade on `workspace_id`), plus their API/refresh
/// tokens and memberships. The user's right-to-be-forgotten path. Gated on the
/// current password. File object keys are durably staged for GC before the DB
/// cascade; S3 deletion is attempted immediately after commit.
pub async fn delete_account(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<DeleteAccountRequest>,
) -> ApiResult<StatusCode> {
  let user_id = user_id_from_headers(&state, &headers).await?;

  let user = sqlx::query_as::<_, UserRow>(
    "SELECT id, email, display_name, password_hash, created_at, avatar_key, email_verified_at FROM users WHERE id = $1 AND deleted_at IS NULL",
  )
  .bind(user_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::Unauthorized)?;
  verify_password(&payload.password, &user.password_hash)?;

  let keys = delete_user_and_owned(
    &state.db,
    user_id,
    state
      .storage
      .as_ref()
      .map_or(7 * 24 * 60 * 60, |s| s.presign_ttl_seconds),
  )
  .await?;
  if let Some(storage) = &state.storage {
    super::workspaces::delete_objects(
      storage,
      keys
        .iter()
        .map(|(workspace_id, key)| (*workspace_id, key.as_str())),
    )
    .await;
    // The face goes too. Avatars live under `avatars/{user_id}/`, outside
    // every workspace prefix, so neither the cascade above nor the blob GC
    // ever reaches them — a deleted account's picture used to stay in storage
    // forever. Best effort, after commit: the account is already gone.
    if let Some(avatar) = user.avatar_key.as_deref().filter(|key| !key.is_empty())
      && let Err(error) = reqwest::Client::new()
        .delete(storage.presign_delete(avatar))
        .send()
        .await
    {
      tracing::warn!(%user_id, error = %error.without_url(), "delete account: avatar object left behind");
    }
  }
  Ok(StatusCode::NO_CONTENT)
}

/// The transactional cascade behind [`delete_account`], separated so a DB test
/// can exercise it without minting a token. Deletes the user's owned workspaces
/// (all content cascades on `workspace_id`) and the shares they authored, then
/// turns the user row into a tombstone (see the body, and migration 0031).
/// Atomic: either the account and everything it owns is gone, or nothing is.
/// Pending-upload ledger rows deliberately have no workspace FK, so storage
/// failures after commit remain recoverable by blob GC.
pub async fn delete_user_and_owned(
  db: &sqlx::PgPool,
  user_id: Uuid,
  presign_ttl_seconds: u64,
) -> ApiResult<Vec<(Uuid, String)>> {
  let mut tx = db.begin().await?;
  // Block a concurrent workspace create's owner FK check before taking the
  // workspace snapshot. Each workspace row lock then blocks concurrent file
  // INSERTs until the pending ledger and cascades have committed together.
  let user_exists: Option<Uuid> =
    sqlx::query_scalar("SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR UPDATE")
      .bind(user_id)
      .fetch_optional(&mut *tx)
      .await?;
  if user_exists.is_none() {
    return Err(ApiError::NotFound);
  }
  let workspace_ids: Vec<Uuid> =
    sqlx::query_scalar("SELECT id FROM workspaces WHERE owner_id = $1 ORDER BY id FOR UPDATE")
      .bind(user_id)
      .fetch_all(&mut *tx)
      .await?;
  let keys: Vec<(Uuid, String)> = sqlx::query_as(
    "SELECT workspace_id, object_key FROM files \
     WHERE workspace_id = ANY($1) ORDER BY workspace_id, object_key",
  )
  .bind(&workspace_ids)
  .fetch_all(&mut *tx)
  .await?;
  for (workspace_id, key) in &keys {
    // Do not take the key advisory lock while holding a workspace row lock:
    // `complete` takes key before its file INSERT's FK workspace lock. GC's
    // registered-row cleanup uses a version-conditional DELETE so this UPSERT
    // cannot be removed by a stale GC observation.
    store::record_pending_file_upload_tx(&mut tx, *workspace_id, key, presign_ttl_seconds).await?;
  }
  sqlx::query("DELETE FROM workspaces WHERE owner_id = $1")
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
  sqlx::query("DELETE FROM document_shares WHERE created_by = $1")
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
  // C4: a tombstone, not a DELETE. What this account authored in workspaces
  // owned by OTHERS (pages, uploads, stream updates — `ON DELETE RESTRICT`)
  // made a hard delete fail with a 409 nobody could act on: anyone who had
  // ever collaborated could not leave. That work belongs to the workspace it
  // was done in, so the row stays as an anonymous author; everything that
  // identifies or authenticates the person goes. `email` gets a unique
  // `.invalid` placeholder so the real address can register again.
  //
  // Comments are the exception, by 0014's explicit contract ("a deleted
  // account takes its comments with it"): they used to go by cascade, and a
  // row that is no longer deleted cascades nothing, so they go here.
  for statement in [
    "DELETE FROM comments WHERE author_id = $1",
    "DELETE FROM comment_threads WHERE created_by = $1",
    "DELETE FROM workspace_members WHERE user_id = $1",
    "DELETE FROM api_tokens WHERE user_id = $1",
    "DELETE FROM refresh_tokens WHERE user_id = $1",
    "DELETE FROM password_reset_tokens WHERE user_id = $1",
    "DELETE FROM email_verification_tokens WHERE user_id = $1",
    "DELETE FROM import_jobs WHERE user_id = $1",
    "DELETE FROM user_settings WHERE user_id = $1",
  ] {
    sqlx::query(statement)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
  }
  sqlx::query(
    "UPDATE users SET
       email = 'deleted+' || id::text || '@deleted.invalid',
       display_name = '',
       password_hash = '',
       avatar_key = NULL,
       email_verified_at = NULL,
       is_admin = false,
       deleted_at = now(),
       updated_at = now()
     WHERE id = $1",
  )
  .bind(user_id)
  .execute(&mut *tx)
  .await?;
  tx.commit().await?;
  Ok(keys)
}

/// A fresh sign-in: its own token family.
async fn auth_response(state: &AppState, user: UserRow) -> ApiResult<AuthResponse> {
  auth_response_in_family(state, user, Uuid::new_v4()).await
}

/// [family_id] threads a rotation back to the sign-in it came from, so reuse
/// detection can burn the whole chain at once.
async fn auth_response_in_family(
  state: &AppState,
  user: UserRow,
  family_id: Uuid,
) -> ApiResult<AuthResponse> {
  let expires_at = Utc::now() + Duration::seconds(state.config.access_token_ttl_seconds);
  let claims = Claims {
    sub: user.id.to_string(),
    exp: expires_at.timestamp() as usize,
  };
  let access_token = encode(
    &Header::default(),
    &claims,
    &EncodingKey::from_secret(state.config.jwt_secret.as_bytes()),
  )
  .map_err(|error| ApiError::Internal(error.to_string()))?;

  let refresh_token = issue_refresh_token(state, user.id, family_id).await?;

  Ok(AuthResponse {
    access_token,
    token_type: "Bearer",
    expires_at,
    refresh_token,
    user: UserResponse::from(user),
  })
}

/// Mint one prefixed secret: the prefix makes a leaked string recognisable in
/// a log (`mica_rt_` refresh token, `mica_pat_` PAT, `mica_ev_` verification,
/// `mica_pr_` reset), followed by 244 bits of randomness from two v4 UUIDs —
/// no extra crate, plenty for a token. All four sites minted by the same two
/// `format!` lines; this is them, recited once.
pub(crate) fn mint_secret(prefix: &str) -> String {
  let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
  format!("{prefix}{secret}")
}

pub const REFRESH_PREFIX: &str = "mica_rt_";

async fn issue_refresh_token(
  state: &AppState,
  user_id: Uuid,
  family_id: Uuid,
) -> ApiResult<String> {
  mint_refresh_token(
    &state.db,
    state.config.refresh_token_ttl_seconds,
    user_id,
    family_id,
  )
  .await
}

/// Mint + store one refresh token, returning the plaintext (the only time it
/// exists outside the client). Takes the pool rather than [AppState] so the
/// rotation can be tested against a real database without an HTTP stack — the
/// interesting logic here IS the SQL, so a test that mocks it tests nothing.
pub async fn mint_refresh_token(
  db: &sqlx::PgPool,
  ttl_seconds: i64,
  user_id: Uuid,
  family_id: Uuid,
) -> ApiResult<String> {
  let token = mint_secret(REFRESH_PREFIX);
  let expires_at = Utc::now() + Duration::seconds(ttl_seconds);

  // Never mint into a family that has been revoked. `refresh` spends the old
  // token and then mints the new one, so a replay burning the family in between
  // would otherwise resurrect it with a live token. INSERT…SELECT…WHERE NOT
  // EXISTS is one statement, so the check and the insert cannot be split.
  //
  // A fresh sign-in passes trivially: its family has no rows yet.
  let inserted = sqlx::query(
    r#"
      INSERT INTO refresh_tokens (user_id, token_hash, family_id, expires_at)
      SELECT $1, $2, $3, $4
      WHERE NOT EXISTS (
        SELECT 1 FROM refresh_tokens
        WHERE family_id = $3 AND revoked_at IS NOT NULL
      )
    "#,
  )
  .bind(user_id)
  .bind(sha256_hex(&token))
  .bind(family_id)
  .bind(expires_at)
  .execute(db)
  .await?;

  if inserted.rows_affected() == 0 {
    return Err(ApiError::Unauthorized);
  }

  Ok(token)
}

/// Spend a refresh token, returning the `(user_id, family_id)` the next pair
/// should be minted under. Every rejection is [ApiError::Unauthorized] — the
/// caller has no business learning *why* a token it presented is unacceptable.
pub async fn rotate_refresh_token(
  db: &sqlx::PgPool,
  refresh_token: &str,
) -> ApiResult<(Uuid, Uuid)> {
  let token_hash = sha256_hex(refresh_token);

  let row = sqlx::query_as::<_, RefreshTokenRow>(
    r#"
      SELECT user_id, family_id, expires_at, revoked_at
      FROM refresh_tokens
      WHERE token_hash = $1
    "#,
  )
  .bind(&token_hash)
  .fetch_optional(db)
  .await?
  .ok_or(ApiError::Unauthorized)?;

  // A fast, friendly rejection only. The UPDATE below is the authority for BOTH
  // conditions — this snapshot is already stale by the time we act on it.
  if row.revoked_at.is_some() || row.expires_at <= Utc::now() {
    return Err(ApiError::Unauthorized);
  }

  // Spend it. Every condition that decides "may this token be spent" lives in
  // the WHERE, so the decision is the UPDATE itself: of two simultaneous
  // refreshes exactly one updates a row, and a family revoked concurrently (a
  // replay landing between the SELECT and here) takes effect immediately rather
  // than one rotation too late. Postgres decides — not a check-then-act we
  // could lose. `revoked_at` was such a check-then-act until it moved here.
  let spent = sqlx::query(
    r#"
      UPDATE refresh_tokens
      SET used_at = now()
      WHERE token_hash = $1 AND used_at IS NULL AND revoked_at IS NULL
    "#,
  )
  .bind(&token_hash)
  .execute(db)
  .await?;

  if spent.rows_affected() == 0 {
    // Already spent. Either a client raced itself or the token leaked and
    // someone else is spending it — indistinguishable from here, so assume the
    // worse one and burn the family. The user signs in again; an attacker
    // holding a stolen token gets nothing.
    revoke_family(db, row.family_id).await?;
    return Err(ApiError::Unauthorized);
  }

  Ok((row.user_id, row.family_id))
}

/// `POST /api/auth/refresh` — trade a refresh token for a fresh access token,
/// rotating the refresh token itself.
pub async fn refresh(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<RefreshRequest>,
) -> ApiResult<(HeaderMap, Json<AuthResponse>)> {
  // Body first, then cookie. The body is the explicit ask; falling back the
  // other way would let a stale cookie quietly outrank a token the caller
  // deliberately handed us.
  let presented = payload
    .refresh_token
    .filter(|token| !token.is_empty())
    .or_else(|| cookie_value(&headers, REFRESH_COOKIE))
    .ok_or(ApiError::Unauthorized)?;
  let (user_id, family_id) = rotate_refresh_token(&state.db, &presented).await?;

  let user = sqlx::query_as::<_, UserRow>(
    r#"
      SELECT id, email, display_name, password_hash, created_at, avatar_key, email_verified_at
      FROM users
      WHERE id = $1 AND deleted_at IS NULL
    "#,
  )
  .bind(user_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::Unauthorized)?;

  let response = auth_response_in_family(&state, user, family_id).await?;
  Ok((auth_cookie_headers(&state, &response), Json(response)))
}

/// `POST /api/auth/logout` — end this sign-in server-side.
///
/// Public, and takes only the refresh token: signing out must work even when
/// the access token has already expired, and the token itself is the proof that
/// the caller owns the session it is ending. Presenting someone else's would
/// require already having stolen it — at which point they can end their own
/// session, which is the worst they can do here.
///
/// Best-effort by design: an unknown/expired token is a 204, not a 404. The
/// client is signing out either way and a failure would only strand it in a
/// state it cannot leave.
pub async fn logout(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<RefreshRequest>,
) -> ApiResult<(HeaderMap, StatusCode)> {
  // Signing out has to clear the cookies even when the token is already gone or
  // unknown — otherwise a stale `mica_session` keeps authenticating the next
  // page load, and "sign out" would be a lie in exactly the case that matters.
  let presented = payload
    .refresh_token
    .filter(|token| !token.is_empty())
    .or_else(|| cookie_value(&headers, REFRESH_COOKIE))
    .unwrap_or_default();
  let family: Option<Uuid> =
    sqlx::query_scalar("SELECT family_id FROM refresh_tokens WHERE token_hash = $1")
      .bind(sha256_hex(&presented))
      .fetch_optional(&state.db)
      .await?;

  // The whole family, not just this token: rotation means the sign-in is a
  // chain, and leaving its other links alive would sign nothing out.
  if let Some(family) = family {
    revoke_family(&state.db, family).await?;
  }

  Ok((cleared_cookie_headers(), StatusCode::NO_CONTENT))
}

/// Burn every credential a user has, everywhere: refresh tokens AND personal
/// access tokens. The access tokens already issued are JWTs and cannot be
/// recalled — they die on their own within `access_token_ttl_seconds` (1h by
/// default), which is the price of them being stateless. Nothing can mint a
/// new one after this.
///
/// PATs are included because both callers are password changes, made when
/// someone thinks an intruder is in the account — and minting a PAT
/// (`POST /auth/tokens`, never expiring by default) is exactly how an intruder
/// would stay. Revoking only refresh tokens left that door open while the
/// reset page told the user every device had been signed out. Same choice as
/// Google's app passwords, which a password change revokes.
pub async fn revoke_user_sessions(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<()> {
  let mut tx = db.begin().await?;
  revoke_user_sessions_in_tx(&mut tx, user_id).await?;
  tx.commit().await?;
  Ok(())
}

pub(super) async fn revoke_user_sessions_in_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  user_id: Uuid,
) -> ApiResult<()> {
  sqlx::query(
    r#"
      UPDATE refresh_tokens
      SET revoked_at = now()
      WHERE user_id = $1 AND revoked_at IS NULL
    "#,
  )
  .bind(user_id)
  .execute(&mut **tx)
  .await?;
  sqlx::query("DELETE FROM api_tokens WHERE user_id = $1")
    .bind(user_id)
    .execute(&mut **tx)
    .await?;
  Ok(())
}

/// Burn every token of one sign-in. Used on reuse detection: the chain is
/// compromised, so nothing in it is trusted again.
pub async fn revoke_family(db: &sqlx::PgPool, family_id: Uuid) -> ApiResult<()> {
  sqlx::query(
    r#"
      UPDATE refresh_tokens
      SET revoked_at = now()
      WHERE family_id = $1 AND revoked_at IS NULL
    "#,
  )
  .bind(family_id)
  .execute(db)
  .await?;
  Ok(())
}

pub(crate) const PAT_PREFIX: &str = "mica_pat_";

/// API token permission scopes. `write` implies `read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
  Read,
  Write,
}

impl Scope {
  pub(crate) fn as_str(self) -> &'static str {
    match self {
      Scope::Read => "read",
      Scope::Write => "write",
    }
  }

  pub(crate) fn parse(s: &str) -> Option<Scope> {
    match s {
      "read" => Some(Scope::Read),
      "write" => Some(Scope::Write),
      _ => None,
    }
  }
}

/// A resolved caller: their user id and the scopes their credential grants.
#[derive(Debug, Clone)]
pub(crate) struct Auth {
  pub user_id: Uuid,
  pub scopes: Vec<Scope>,
}

impl Auth {
  fn grants(&self, need: Scope) -> bool {
    match need {
      Scope::Read => self.scopes.contains(&Scope::Read) || self.scopes.contains(&Scope::Write),
      Scope::Write => self.scopes.contains(&Scope::Write),
    }
  }
}

#[derive(FromRow)]
struct PatRow {
  user_id: Uuid,
  scopes: Vec<String>,
  expires_at: Option<DateTime<Utc>>,
}

pub(crate) fn sha256_hex(input: &str) -> String {
  let digest = Sha256::digest(input.as_bytes());
  let mut out = String::with_capacity(digest.len() * 2);
  for byte in digest {
    out.push_str(&format!("{byte:02x}"));
  }
  out
}

/// Resolve a bearer token — a PAT (`mica_pat_…`, looked up in the DB) or a JWT
/// access token (full scope) — into the caller's id and scopes.
pub(crate) async fn resolve_token(state: &AppState, token: &str) -> ApiResult<Auth> {
  if token.starts_with(PAT_PREFIX) {
    return resolve_pat(state, token).await;
  }
  let user_id = user_id_from_token(state, token)?;
  Ok(Auth {
    user_id,
    scopes: vec![Scope::Read, Scope::Write],
  })
}

async fn resolve_pat(state: &AppState, token: &str) -> ApiResult<Auth> {
  let hash = sha256_hex(token);
  let row = sqlx::query_as::<_, PatRow>(
    "SELECT user_id, scopes, expires_at FROM api_tokens WHERE token_hash = $1",
  )
  .bind(&hash)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::Unauthorized)?;

  if let Some(expires_at) = row.expires_at {
    if expires_at <= Utc::now() {
      return Err(ApiError::Unauthorized);
    }
  }

  // Best-effort last-used bookkeeping; a failure here must not block the request.
  // At most once a minute: every PAT request resolves the token twice (scope
  // guard + handler), and an unconditional UPDATE was two row writes per MCP
  // call for a "last used" that is shown to the minute anyway.
  let _ = sqlx::query(
    "UPDATE api_tokens SET last_used_at = now() \
     WHERE token_hash = $1 \
       AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')",
  )
  .bind(&hash)
  .execute(&state.db)
  .await;

  let scopes = row.scopes.iter().filter_map(|s| Scope::parse(s)).collect();
  Ok(Auth {
    user_id: row.user_id,
    scopes,
  })
}

// The credential `scope_guard` already resolved for THIS request: the token
// it saw and the user it maps to.
//
// Every authenticated handler resolves its caller again through
// `user_id_from_headers` — for a JWT that is CPU only, but for a PAT (the MCP /
// CLI path) it was a second SELECT on every call. A task-local, not a header:
// nothing a client sends can populate it, and it lives exactly as long as the
// request's handler future.
tokio::task_local! {
  static RESOLVED: (String, Uuid);
}

/// Same two sources as [`scope_guard`], and it has to stay that way.
///
/// A handful of handlers read the token themselves instead of leaning on the
/// middleware. When the cookie was added to the middleware alone, THOSE went on
/// rejecting it — half the API accepting a credential and half refusing it is
/// worse than none of it accepting: it fails per-endpoint, which reads as a
/// flaky server rather than a missing case. (Caught by the live smoke; no unit
/// test was ever going to notice a handler it did not know existed.)
pub(crate) async fn user_id_from_headers(state: &AppState, headers: &HeaderMap) -> ApiResult<Uuid> {
  let token = bearer_token(headers)
    .or_else(|| cookie_value(headers, SESSION_COOKIE))
    .ok_or(ApiError::Unauthorized)?;
  // Same token the guard resolved moments ago in this request → same answer.
  if let Ok(Some(user_id)) =
    RESOLVED.try_with(|(seen, user_id)| (*seen == token).then_some(*user_id))
  {
    return Ok(user_id);
  }
  Ok(resolve_token(state, &token).await?.user_id)
}

/// The caller's id, but only if that account still EXISTS.
///
/// The access token is a stateless JWT: once minted, nothing can revoke it. So a
/// deleted account's token keeps verifying until it expires on its own, and
/// `delete_account` cannot reach it — the cascade takes the `users` row, the
/// `refresh_tokens` and the `api_tokens`, and the JWT in the caller's hand is
/// none of those. The window is the token's lifetime, ~1 hour by default.
///
/// Deleting the account is the one action whose WHOLE POINT is that the account
/// stops being able to do things, so a path that spends real money on the
/// operator's behalf must not honour a token for a user who is gone. The paths
/// that spend are the AI endpoints, and that is where this is used.
///
/// Deliberately a separate function rather than a change to
/// [`user_id_from_headers`]: that one runs on nearly every request, and paying a
/// `users` lookup on all of them to close a window that only matters where money
/// is spent is not a trade worth making silently. The alternative — revocable
/// tokens — is a real design (a token-version column checked per request) and out
/// of proportion to this: the cost here is one indexed primary-key lookup on the
/// handful of AI calls.
pub(crate) async fn existing_user_id_from_headers(
  state: &AppState,
  headers: &HeaderMap,
) -> ApiResult<Uuid> {
  let user_id = user_id_from_headers(state, headers).await?;
  ensure_user_exists(&state.db, user_id).await?;
  Ok(user_id)
}

/// `Err(Unauthorized)` when the account behind [user_id] is gone.
///
/// Takes the pool rather than the `AppState` so the rule is testable against a
/// real database without building a whole app (config, hub, object store,
/// mailer) — the same reason `delete_user_and_owned` is split out of
/// `delete_account`.
///
/// 401, not 404/403: a token for a user who no longer exists is not "a valid
/// caller doing something forbidden", it is a credential that has ceased to
/// identify anyone. Clients already treat 401 as "session over, re-authenticate",
/// which is exactly the right instruction here.
pub(crate) async fn ensure_user_exists(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<()> {
  let exists = sqlx::query_scalar::<_, bool>(
    "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL)",
  )
  .bind(user_id)
  .fetch_one(db)
  .await?;
  if exists {
    Ok(())
  } else {
    Err(ApiError::Unauthorized)
  }
}

/// The signed-in user's id, but only when that user administers this instance.
///
/// Instance-wide settings — today the AI provider config — carry the OPERATOR's
/// credentials and are shared by everyone on the deployment, so "is signed in"
/// is the wrong bar for changing them: on an open-registration instance any
/// account could repoint the model or spend the operator's credit. `is_admin`
/// is set on the account that stood the instance up (migration 0021).
///
/// 403 rather than 404: the endpoint exists and the caller is authenticated —
/// hiding that would just send an admin hunting for a routing problem.
pub(crate) async fn admin_id_from_headers(
  state: &AppState,
  headers: &HeaderMap,
) -> ApiResult<Uuid> {
  let user_id = user_id_from_headers(state, headers).await?;
  let is_admin = sqlx::query_scalar::<_, bool>("SELECT is_admin FROM users WHERE id = $1")
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .unwrap_or(false);
  if is_admin {
    Ok(user_id)
  } else {
    Err(ApiError::Forbidden)
  }
}

/// Decode a bare JWT access token into a user id. Used by the WebSocket handler,
/// which receives the token via query string rather than an Authorization header.
/// The access token, as a cookie the browser keeps out of JavaScript's reach.
///
/// Web had nowhere safe to put a token: `localStorage` is readable by any
/// same-origin script, so one stored XSS on a shared page was an account
/// takeover, and the WebSocket had to carry the token in its URL because the
/// browser `WebSocket` API cannot set headers.
///
/// A cookie answers both at once, which is why it beats the ticket-plus-memory
/// design it replaced: `HttpOnly` puts it beyond script, and the browser attaches
/// it to the WS handshake by itself — the handshake is an ordinary HTTP request,
/// a fact easy to miss behind "browsers can't authenticate WebSockets". (Same
/// shape AFFiNE uses, under the same constraint.)
pub(crate) const SESSION_COOKIE: &str = "mica_session";

/// The refresh token, so a page reload can recover a session without ever
/// handing the durable credential to script.
pub(crate) const REFRESH_COOKIE: &str = "mica_refresh";

/// What the cookies buy, and what they cost.
///
/// **Cost:** cookies are attached by the browser, so cookie auth is CSRF-able
/// where header auth is not. `SameSite=Strict` is the answer here — a cross-site
/// request simply does not carry it — and it is enough on its own for this
/// shape: the tokens are only ever *read* from the cookie, never combined with
/// ambient state, and a double-submit token on top would mean touching every
/// write endpoint for a second layer over a defence browsers already enforce.
/// If mica ever serves its API from a different origin than its app, this
/// reasoning has to be redone, not extended.
///
/// **Not `Secure` in development**, or the cookie never arrives over plain
/// `http://127.0.0.1` and web dev silently cannot sign in.
fn session_cookie(name: &str, value: &str, max_age: i64, secure: bool) -> String {
  let secure = if secure { "; Secure" } else { "" };
  format!("{name}={value}; Max-Age={max_age}; Path=/; HttpOnly; SameSite=Strict{secure}")
}

/// Expire a cookie now. `Max-Age=0` with an empty value is the only reliable
/// delete — a browser matches on name+path, so the path must match the one it
/// was set with or the old cookie simply survives the sign-out.
fn cleared_cookie(name: &str) -> String {
  format!("{name}=; Max-Age=0; Path=/; HttpOnly; SameSite=Strict")
}

/// Read one cookie out of the `Cookie` header.
///
/// Hand-parsed rather than pulling a cookie crate: the header is
/// `a=1; b=2`, values here are our own base64/JWT alphabets, and the parse is
/// three lines. A crate would also tempt the rest of the codebase into cookie
/// state we do not want.
pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
  headers
    .get(axum::http::header::COOKIE)?
    .to_str()
    .ok()?
    .split(';')
    .filter_map(|pair| pair.trim().split_once('='))
    .find(|(key, _)| *key == name)
    .map(|(_, value)| value.trim().to_string())
    .filter(|value| !value.is_empty())
}

/// `Authorization: Bearer …`, the credential desktop uses and the one that wins
/// wherever both are present — an explicit header is a deliberate act, a cookie
/// is ambient.
pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<String> {
  headers
    .get(AUTHORIZATION)?
    .to_str()
    .ok()?
    .strip_prefix("Bearer ")
    .map(str::to_string)
    .filter(|token| !token.is_empty())
}

/// The token for a WebSocket handshake, from three sources in descending order
/// of how deliberate they are — the one function both WS routes (`ws.rs`, the
/// document and tree sockets; `ai_ws.rs`, the AI stream) call, after each used
/// to carry its own copy of this sequence.
///
/// 1. `Authorization: Bearer` — desktop. An explicit header is an explicit act.
/// 2. The session **cookie** — web. The WS handshake is an ordinary HTTP
///    request, so the browser attaches it by itself. This is what lets the web
///    client stop putting a JWT in the URL: "browsers cannot authenticate a
///    WebSocket" is only true of custom headers, and taking it as true of
///    cookies too is what kept the token in the query string for so long.
/// 3. `?token=` — the compatibility tail (`query_token`). Still read so an
///    older web build (and any non-browser client) keeps working; it is no
///    longer written by ours.
pub(crate) fn token_from_request(headers: &HeaderMap, query_token: Option<&str>) -> Option<String> {
  if let Some(token) = bearer_token(headers) {
    return Some(token);
  }

  if let Some(token) = cookie_value(headers, SESSION_COOKIE) {
    return Some(token);
  }

  query_token
    .map(str::trim)
    .filter(|token| !token.is_empty())
    .map(str::to_string)
}

/// The `Set-Cookie` pair for a freshly minted session.
fn auth_cookie_headers(state: &AppState, response: &AuthResponse) -> HeaderMap {
  let secure = state.config.environment == Environment::Production;
  let mut headers = HeaderMap::new();
  for raw in [
    session_cookie(
      SESSION_COOKIE,
      &response.access_token,
      state.config.access_token_ttl_seconds,
      secure,
    ),
    session_cookie(
      REFRESH_COOKIE,
      &response.refresh_token,
      state.config.refresh_token_ttl_seconds,
      secure,
    ),
  ] {
    if let Ok(value) = HeaderValue::from_str(&raw) {
      headers.append(SET_COOKIE, value);
    }
  }
  headers
}

fn cleared_cookie_headers() -> HeaderMap {
  let mut headers = HeaderMap::new();
  for name in [SESSION_COOKIE, REFRESH_COOKIE] {
    if let Ok(value) = HeaderValue::from_str(&cleared_cookie(name)) {
      headers.append(SET_COOKIE, value);
    }
  }
  headers
}

pub(crate) fn user_id_from_token(state: &AppState, token: &str) -> ApiResult<Uuid> {
  session_from_token(state, token).map(|(user_id, _)| user_id)
}

/// The user AND the moment this token stops being valid (`exp`, unix seconds).
///
/// Every HTTP request re-checks `exp` because every request re-reads the token.
/// A WebSocket does not: it authenticates once at the upgrade and then lives for
/// as long as it stays connected, so a socket opened one second before expiry
/// stayed authorised for hours. Handing the deadline out here lets the socket
/// enforce it — see `ws.rs`, which closes with 4401 when it passes.
pub(crate) fn session_from_token(state: &AppState, token: &str) -> ApiResult<(Uuid, u64)> {
  let token = decode::<Claims>(
    token,
    &DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
    &Validation::default(),
  )
  .map_err(|_| ApiError::Unauthorized)?;

  let user_id = Uuid::parse_str(&token.claims.sub).map_err(|_| ApiError::Unauthorized)?;
  Ok((user_id, token.claims.exp as u64))
}

/// Axum middleware: authenticate every request on the routes it is layered
/// over and enforce the scope its HTTP method needs (safe methods → `read`,
/// mutating → `write`). The public half of `/api` is mounted WITHOUT this
/// guard (`routes::layered_api`) — there is no path table here to keep in
/// sync with the router. JWT (password) sessions carry full scope; PATs are
/// checked against their grant.
pub async fn scope_guard(
  State(state): State<AppState>,
  request: Request,
  next: Next,
) -> Result<Response, ApiError> {
  let bearer = bearer_token(request.headers());
  let from_cookie = bearer.is_none();
  let token = bearer
    .or_else(|| cookie_value(request.headers(), SESSION_COOKIE))
    .ok_or(ApiError::Unauthorized)?;
  // A cookie is attached by the BROWSER, so a state-changing request riding on
  // one must come from the app's own origin. SameSite=Strict alone let a
  // same-site page (an uploaded HTML file on the storage origin) send them.
  // Bearer tokens are exempt: a hostile page cannot read one to send it.
  if from_cookie
    && !request.method().is_safe()
    && !crate::origin::origin_allowed(&state.config, request.headers())
  {
    return Err(ApiError::Forbidden);
  }
  let auth = resolve_token(&state, &token).await?;
  let need = if request.method().is_safe() {
    Scope::Read
  } else {
    Scope::Write
  };
  if !auth.grants(need) {
    return Err(ApiError::Forbidden);
  }
  Ok(
    RESOLVED
      .scope((token, auth.user_id), next.run(request))
      .await,
  )
}

// The unguarded routes (health probes and the mail-link endpoints whose
// caller can by definition not sign in yet) are mounted WITHOUT this guard in
// `routes::public_api_router`. That used to be an `is_public` path-suffix
// table here, kept in sync with the router by hand: the blob link and
// resend-verification were each silently misclassified that way, and a
// `{param}` value spelling one of the suffixes (e.g.
// `DELETE /ai/providers/health`) bypassed the guard. The coverage those
// entries' tests carried now runs as real requests through the assembled
// routers — see `route_layer_tests` in routes/mod.rs.
pub(crate) fn normalize_email(email: &str) -> ApiResult<String> {
  let email = email.trim().to_ascii_lowercase();
  if email.is_empty() || !email.contains('@') {
    return Err(ApiError::BadRequest("valid email is required".to_string()));
  }

  Ok(email)
}

fn normalize_display_name(display_name: &str) -> ApiResult<String> {
  super::names::normalize_name(display_name, "display_name")
}

fn validate_password(password: &str, identifiers: &[&str]) -> ApiResult<()> {
  password_strength::check(password, identifiers)
    .map_err(|weak| ApiError::BadRequest(weak.message().to_string()))
}

pub(crate) fn hash_password(password: &str) -> ApiResult<String> {
  let salt = SaltString::generate(&mut OsRng);
  Argon2::default()
    .hash_password(password.as_bytes(), &salt)
    .map(|hash| hash.to_string())
    .map_err(|error| ApiError::Internal(error.to_string()))
}

/// Startup hook for `MICA_SEED_TEST_USER`: upsert the test account so E2E
/// runs always have known credentials — created if missing, password reset if
/// it already exists. Production never reaches here (AppConfig strips the
/// variable there).
pub async fn seed_test_user(db: &sqlx::PgPool, email: &str, password: &str) -> anyhow::Result<()> {
  let password_hash =
    hash_password(password).map_err(|error| anyhow::anyhow!("hash failed: {error:?}"))?;
  // Seeded already-verified, and this is not optional: a seeded account has no
  // inbox anyone can check, so leaving `email_verified_at` NULL would make the
  // whole mechanism useless — the account would exist and be unable to sign in.
  // `coalesce` on the conflict path so re-seeding an existing account keeps its
  // original confirmation time rather than rewriting it on every boot.
  sqlx::query(
    r#"
      INSERT INTO users (email, display_name, password_hash, email_verified_at)
      VALUES ($1, $2, $3, now())
      ON CONFLICT (email) DO UPDATE SET
        password_hash = EXCLUDED.password_hash,
        email_verified_at = coalesce(users.email_verified_at, now())
    "#,
  )
  .bind(email)
  .bind("Test User")
  .bind(password_hash)
  .execute(db)
  .await?;
  Ok(())
}

/// A real Argon2 hash (same default parameters as every stored one) of a
/// random password nobody knows, computed once — for [login]'s unknown-email
/// path to verify against.
fn dummy_password_hash() -> &'static str {
  static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
  HASH
    .get_or_init(|| hash_password(&Uuid::new_v4().to_string()).expect("hashing a random password"))
}

fn verify_password(password: &str, password_hash: &str) -> ApiResult<()> {
  let parsed_hash =
    PasswordHash::new(password_hash).map_err(|error| ApiError::Internal(error.to_string()))?;

  Argon2::default()
    .verify_password(password.as_bytes(), &parsed_hash)
    .map_err(|_| ApiError::Unauthorized)
}

/// Take the registration lock and attempt the insert, in one transaction.
///
/// `Ok(None)` means the guard refused — registration is closed and the instance
/// already has an account. `Ok(Some(_))` means the row landed (and, on an empty
/// instance, carries `is_admin`).
///
/// A free function rather than inline code in [`register`] so the concurrency
/// test exercises THIS statement and THIS lock. A test that retypes the SQL
/// passes identically with the lock removed, which would make it a test of the
/// copy rather than of the behaviour.
async fn insert_registered_user(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  email: &str,
  display_name: &str,
  password_hash: &str,
  registration_enabled: bool,
) -> ApiResult<Option<UserRow>> {
  // Transaction-scoped: released at commit/rollback, so there is no unlock to
  // forget on an error path. The key is constant because the race is between two
  // DIFFERENT addresses on an empty instance — a per-email key would not
  // serialize them.
  sqlx::query("SELECT pg_advisory_xact_lock($1)")
    .bind(REGISTRATION_LOCK_KEY)
    .execute(&mut **tx)
    .await?;

  sqlx::query_as::<_, UserRow>(
    r#"
      INSERT INTO users (email, display_name, password_hash, is_admin)
      SELECT $1, $2, $3, NOT EXISTS (SELECT 1 FROM users WHERE deleted_at IS NULL)
      WHERE $4 OR NOT EXISTS (SELECT 1 FROM users WHERE deleted_at IS NULL)
      RETURNING id, email, display_name, password_hash, created_at, avatar_key, email_verified_at
    "#,
  )
  .bind(email)
  .bind(display_name)
  .bind(password_hash)
  .bind(registration_enabled)
  .fetch_optional(&mut **tx)
  .await
  .map_err(map_insert_user_error)
}

fn map_insert_user_error(error: sqlx::Error) -> ApiError {
  if let sqlx::Error::Database(db_error) = &error
    && db_error.constraint() == Some("users_email_key")
  {
    return ApiError::Conflict("email is already registered".to_string());
  }

  ApiError::Database(error)
}

impl From<UserRow> for UserResponse {
  fn from(user: UserRow) -> Self {
    Self {
      id: user.id,
      email: user.email,
      display_name: user.display_name,
      created_at: user.created_at,
      avatar_version: user
        .avatar_key
        .as_deref()
        .map(crate::routes::avatar::avatar_version),
    }
  }
}

#[cfg(test)]
#[path = "auth_contract_gold_tests.rs"]
mod contract_gold_tests;

#[cfg(test)]
mod tests {
  use super::*;

  fn auth(scopes: Vec<Scope>) -> Auth {
    Auth {
      user_id: Uuid::nil(),
      scopes,
    }
  }

  #[test]
  fn write_implies_read() {
    let a = auth(vec![Scope::Write]);
    assert!(a.grants(Scope::Read), "write must grant read");
    assert!(a.grants(Scope::Write));
  }

  // The is_public table these used to probe is gone — the blob/avatar
  // capability links and the "cannot sign in yet" mail endpoints are mounted
  // in `routes::public_api_router`, and their probe coverage now runs as real
  // requests through the assembled routers (`route_layer_tests` in
  // routes/mod.rs).

  #[test]
  fn read_only_cannot_write() {
    let a = auth(vec![Scope::Read]);
    assert!(a.grants(Scope::Read));
    assert!(!a.grants(Scope::Write), "read-only must not grant write");
  }

  #[test]
  fn empty_scope_grants_nothing() {
    let a = auth(vec![]);
    assert!(!a.grants(Scope::Read));
    assert!(!a.grants(Scope::Write));
  }

  #[test]
  fn scope_parse_roundtrip() {
    assert_eq!(Scope::parse("read"), Some(Scope::Read));
    assert_eq!(Scope::parse("write"), Some(Scope::Write));
    assert_eq!(Scope::parse("admin"), None);
    assert_eq!(Scope::Read.as_str(), "read");
    assert_eq!(Scope::Write.as_str(), "write");
  }

  #[test]
  fn sha256_hex_is_64_lowercase_hex_and_deterministic() {
    let h = sha256_hex("mica_pat_example");
    assert_eq!(h.len(), 64);
    assert!(
      h.chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
    );
    assert_eq!(h, sha256_hex("mica_pat_example"));
    assert_ne!(h, sha256_hex("mica_pat_other"));
  }

  /// The first account on an empty instance is verified on the spot, but both
  /// cases used to answer `204 No Content` — so the client could not tell them
  /// apart, guessed "check your email", and sent the very first operator of a
  /// fresh install to wait for a message that was never sent and was never
  /// needed. The response has to carry the fact the server acted on.
  #[test]
  fn the_register_response_states_whether_the_account_can_sign_in_now() {
    let first = serde_json::to_value(RegisterResponse { verified: true }).unwrap();
    assert_eq!(first, serde_json::json!({"verified": true}));
    let later = serde_json::to_value(RegisterResponse { verified: false }).unwrap();
    assert_eq!(later, serde_json::json!({"verified": false}));
  }

  // Refresh and resend-verification MUST stay reachable with no access token
  // (the caller's is dead / does not exist yet — that is the whole point);
  // both shipped broken under a router-wide guard before. They are covered
  // now by `route_layer_tests::the_credential_and_mail_endpoints_are_throttled`
  // in routes/mod.rs, which drives them through the real router.
}

/// The rotation's correctness IS its SQL — the conditional UPDATE is what makes
/// spending a token atomic, and a mocked database would test nothing at all. So
/// these run against a real Postgres, gated on `DATABASE_URL`: skipped (green)
/// without one locally, but any failure to reach the database is a hard error,
/// matching app-core/tests/sync_pg.rs. Skipping WITH a database set — or in CI,
/// where one is always provisioned — would be a silent lie, not a convenience.
///
///   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
///   cargo test -p mica-api-server refresh_pg
#[cfg(test)]
mod refresh_pg {
  use super::*;
  use sqlx::PgPool;

  /// Skipping without a database is a local convenience; skipping WITH one is a
  /// lie. A set-but-unusable `DATABASE_URL` panics, and in CI a MISSING one
  /// panics too, because there the database is always provisioned: its absence
  /// means the workflow regressed, not that these assertions may quietly stop
  /// running. (See app-core/tests/sync_pg.rs, which learned this the hard way.)
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

  async fn seed_user(db: &PgPool) -> Uuid {
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
      .bind(user)
      .bind(format!("{user}@refresh.test"))
      .execute(db)
      .await
      .unwrap();
    user
  }

  /// Settings storage semantics the client sync leans on: an account with no
  /// row reads as `{}` (not an error), and a PUT REPLACES the blob — a merge
  /// would resurrect keys a newer client deliberately dropped.
  #[tokio::test]
  async fn user_settings_default_empty_and_replace_whole() {
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;

    let read = |db: sqlx::PgPool| async move {
      let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT settings FROM user_settings WHERE user_id = $1")
          .bind(user)
          .fetch_optional(&db)
          .await
          .unwrap();
      row.map_or_else(|| serde_json::json!({}), |(s,)| s)
    };
    assert_eq!(
      read(db.clone()).await,
      serde_json::json!({}),
      "no row reads as empty"
    );

    let upsert = |db: sqlx::PgPool, v: serde_json::Value| async move {
      sqlx::query(
        "INSERT INTO user_settings(user_id, settings, updated_at) VALUES ($1, $2, now())
         ON CONFLICT (user_id) DO UPDATE SET settings = excluded.settings, updated_at = now()",
      )
      .bind(user)
      .bind(v)
      .execute(&db)
      .await
      .unwrap();
    };
    upsert(
      db.clone(),
      serde_json::json!({"showPageTitle": "true", "themeMode": "dark"}),
    )
    .await;
    assert_eq!(
      read(db.clone()).await,
      serde_json::json!({"showPageTitle": "true", "themeMode": "dark"})
    );

    // Replace-whole: themeMode dropped by the client must be GONE after PUT.
    upsert(db.clone(), serde_json::json!({"showPageTitle": "false"})).await;
    assert_eq!(
      read(db.clone()).await,
      serde_json::json!({"showPageTitle": "false"}),
      "PUT replaces the blob; a merge would have kept themeMode"
    );
  }

  const DAY: i64 = 60 * 60 * 24;

  #[tokio::test]
  async fn delete_account_removes_the_user_and_everything_they_own() {
    let Some(db) = pool().await else {
      return;
    };
    let owner = seed_user(&db).await;
    let bystander = seed_user(&db).await;

    let ws = Uuid::new_v4();
    sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'W',$2)")
      .bind(ws)
      .bind(owner)
      .execute(&db)
      .await
      .unwrap();
    let owned_key = format!("workspaces/{ws}/{}.png", "d".repeat(64));
    sqlx::query(
      "INSERT INTO files(workspace_id,uploaded_by,object_key,original_name,mime_type,byte_size) \
       VALUES($1,$2,$3,'x.png','image/png',3)",
    )
    .bind(ws)
    .bind(owner)
    .bind(&owned_key)
    .execute(&db)
    .await
    .unwrap();
    let doc = Uuid::new_v4();
    sqlx::query(
      "INSERT INTO documents(id,workspace_id,root_block_id,created_by) VALUES($1,$2,'root',$3)",
    )
    .bind(doc)
    .bind(ws)
    .bind(owner)
    .execute(&db)
    .await
    .unwrap();
    let other_ws = Uuid::new_v4();
    sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'O',$2)")
      .bind(other_ws)
      .bind(bystander)
      .execute(&db)
      .await
      .unwrap();
    let owned_ws2 = Uuid::new_v4();
    sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'W2',$2)")
      .bind(owned_ws2)
      .bind(owner)
      .execute(&db)
      .await
      .unwrap();
    let owned_key2 = format!("workspaces/{owned_ws2}/{}.png", "e".repeat(64));
    sqlx::query(
      "INSERT INTO files(workspace_id,uploaded_by,object_key,original_name,mime_type,byte_size) \
       VALUES($1,$2,$3,'y.png','image/png',3)",
    )
    .bind(owned_ws2)
    .bind(owner)
    .bind(&owned_key2)
    .execute(&db)
    .await
    .unwrap();

    let deleted_keys = delete_user_and_owned(&db, owner, 900)
      .await
      .expect("deleting an account that owns only its own data must succeed");
    assert_eq!(deleted_keys.len(), 2);
    assert!(deleted_keys.contains(&(ws, owned_key.clone())));
    assert!(deleted_keys.contains(&(owned_ws2, owned_key2.clone())));

    let count = |sql: &'static str, id: Uuid| {
      let db = db.clone();
      async move {
        sqlx::query_scalar::<_, i64>(sql)
          .bind(id)
          .fetch_one(&db)
          .await
          .unwrap()
      }
    };
    // The owner (a tombstone now), their workspace, and its document (cascade)
    // are all gone.
    assert_eq!(
      count(
        "SELECT count(*) FROM users WHERE id=$1 AND deleted_at IS NULL",
        owner
      )
      .await,
      0
    );
    assert_eq!(
      count("SELECT count(*) FROM workspaces WHERE id=$1", ws).await,
      0
    );
    assert_eq!(
      count("SELECT count(*) FROM documents WHERE id=$1", doc).await,
      0,
      "content in an owned workspace must cascade"
    );
    let pending_count: i64 = sqlx::query_scalar(
      "SELECT count(*) FROM pending_file_uploads WHERE object_key = $1 AND workspace_id = $2",
    )
    .bind(&owned_key)
    .bind(ws)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
      pending_count, 1,
      "account deletion must leave S3 keys for GC"
    );
    let other_pending_count: i64 = sqlx::query_scalar(
      "SELECT count(*) FROM pending_file_uploads WHERE object_key = $1 AND workspace_id = $2",
    )
    .bind(&owned_key2)
    .bind(owned_ws2)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(other_pending_count, 1);
    // A different user and their workspace are untouched.
    assert_eq!(
      count("SELECT count(*) FROM users WHERE id=$1", bystander).await,
      1,
      "a bystander's account must survive"
    );
    assert_eq!(
      count("SELECT count(*) FROM workspaces WHERE id=$1", other_ws).await,
      1
    );

    // Leave the table as we found it for repeat runs.
    sqlx::query("DELETE FROM pending_file_uploads WHERE workspace_id=$1")
      .bind(ws)
      .execute(&db)
      .await
      .unwrap();
    sqlx::query("DELETE FROM pending_file_uploads WHERE workspace_id=$1")
      .bind(owned_ws2)
      .execute(&db)
      .await
      .unwrap();
    sqlx::query("DELETE FROM workspaces WHERE owner_id=$1")
      .bind(bystander)
      .execute(&db)
      .await
      .ok();
    sqlx::query("DELETE FROM users WHERE id=$1")
      .bind(bystander)
      .execute(&db)
      .await
      .ok();
  }

  /// THE BUG (C4): anyone who had ever written a page or uploaded a file in a
  /// workspace owned by someone else could not delete their account — those
  /// rows hold `ON DELETE RESTRICT` references, and the delete answered 409
  /// with advice the user could not act on. The account now becomes a
  /// tombstone: the shared work stays in the other workspace, every trace of
  /// the person goes, and the address can sign up again.
  #[tokio::test]
  async fn a_collaborator_can_delete_their_account_and_leaves_a_tombstone() {
    let Some(db) = pool().await else { return };
    let leaver = seed_user(&db).await;
    let host = seed_user(&db).await;
    let ws = Uuid::new_v4();
    sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'H',$2)")
      .bind(ws)
      .bind(host)
      .execute(&db)
      .await
      .unwrap();
    sqlx::query("INSERT INTO workspace_members(workspace_id,user_id,role) VALUES($1,$2,'editor')")
      .bind(ws)
      .bind(leaver)
      .execute(&db)
      .await
      .unwrap();
    let doc = Uuid::new_v4();
    sqlx::query(
      "INSERT INTO documents(id,workspace_id,root_block_id,created_by) VALUES($1,$2,'root',$3)",
    )
    .bind(doc)
    .bind(ws)
    .bind(leaver)
    .execute(&db)
    .await
    .unwrap();
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
      .bind(leaver)
      .fetch_one(&db)
      .await
      .unwrap();

    delete_user_and_owned(&db, leaver, 900)
      .await
      .expect("authorship in someone else's workspace must not block deletion");

    let (tomb_email, name, hash, deleted): (String, String, String, bool) = sqlx::query_as(
      "SELECT email, display_name, password_hash, deleted_at IS NOT NULL FROM users WHERE id=$1",
    )
    .bind(leaver)
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(deleted);
    assert_eq!(tomb_email, format!("deleted+{leaver}@deleted.invalid"));
    assert_eq!((name.as_str(), hash.as_str()), ("", ""));
    let doc_kept: i64 = sqlx::query_scalar("SELECT count(*) FROM documents WHERE id=$1")
      .bind(doc)
      .fetch_one(&db)
      .await
      .unwrap();
    assert_eq!(doc_kept, 1, "the shared page stays in the host's workspace");
    let memberships: i64 =
      sqlx::query_scalar("SELECT count(*) FROM workspace_members WHERE user_id=$1")
        .bind(leaver)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(memberships, 0);
    assert!(
      matches!(
        ensure_user_exists(&db, leaver).await,
        Err(ApiError::Unauthorized)
      ),
      "a still-valid access token must stop identifying anyone"
    );
    assert!(
      matches!(
        delete_user_and_owned(&db, leaver, 900).await,
        Err(ApiError::NotFound)
      ),
      "a tombstone is not deleted twice"
    );
    // The address is free again.
    let again = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
      .bind(again)
      .bind(&email)
      .execute(&db)
      .await
      .expect("the real address can register again");

    sqlx::query("DELETE FROM workspaces WHERE id=$1")
      .bind(ws)
      .execute(&db)
      .await
      .ok();
    for u in [leaver, host, again] {
      sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(u)
        .execute(&db)
        .await
        .ok();
    }
  }

  #[tokio::test]
  async fn account_deletion_rolls_back_if_a_file_key_cannot_be_tracked() {
    let Some(db) = pool().await else { return };
    let owner = seed_user(&db).await;
    let ws = Uuid::new_v4();
    sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'W',$2)")
      .bind(ws)
      .bind(owner)
      .execute(&db)
      .await
      .unwrap();
    let legacy_key = format!("legacy/{}", Uuid::new_v4());
    sqlx::query(
      "INSERT INTO files(workspace_id,uploaded_by,object_key,original_name,mime_type,byte_size) \
       VALUES($1,$2,$3,'x.png','image/png',3)",
    )
    .bind(ws)
    .bind(owner)
    .bind(&legacy_key)
    .execute(&db)
    .await
    .unwrap();

    assert!(delete_user_and_owned(&db, owner, 900).await.is_err());
    let user_count: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = $1")
      .bind(owner)
      .fetch_one(&db)
      .await
      .unwrap();
    let workspace_count: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces WHERE id = $1")
      .bind(ws)
      .fetch_one(&db)
      .await
      .unwrap();
    let file_count: i64 = sqlx::query_scalar("SELECT count(*) FROM files WHERE object_key = $1")
      .bind(&legacy_key)
      .fetch_one(&db)
      .await
      .unwrap();
    assert_eq!((user_count, workspace_count, file_count), (1, 1, 1));

    sqlx::query("DELETE FROM workspaces WHERE id = $1")
      .bind(ws)
      .execute(&db)
      .await
      .unwrap();
    sqlx::query("DELETE FROM users WHERE id = $1")
      .bind(owner)
      .execute(&db)
      .await
      .unwrap();
  }

  #[tokio::test]
  async fn rotation_hands_back_the_same_family_and_burns_the_old_token() {
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let family = Uuid::new_v4();
    let first = mint_refresh_token(&db, DAY, user, family).await.unwrap();

    let (got_user, got_family) = rotate_refresh_token(&db, &first).await.unwrap();
    assert_eq!(got_user, user);
    assert_eq!(
      got_family, family,
      "a rotation stays in its sign-in's family"
    );

    // The token just spent is dead — and spending it again is a replay.
    assert!(rotate_refresh_token(&db, &first).await.is_err());
  }

  #[tokio::test]
  async fn a_replay_burns_the_whole_family_including_the_live_token() {
    // The theft case: an attacker copies a refresh token, the real client
    // spends it first, then the attacker spends it too. We cannot tell the two
    // apart, so the entire chain dies and the human signs in again.
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let family = Uuid::new_v4();

    let stolen = mint_refresh_token(&db, DAY, user, family).await.unwrap();
    rotate_refresh_token(&db, &stolen).await.unwrap();
    let live = mint_refresh_token(&db, DAY, user, family).await.unwrap();

    // Attacker replays the old one.
    assert!(rotate_refresh_token(&db, &stolen).await.is_err());

    // The token the honest client is holding must ALSO be dead now — a reuse
    // detection that leaves the chain usable protects nobody.
    assert!(
      rotate_refresh_token(&db, &live).await.is_err(),
      "reuse detection must revoke the family, not just the replayed token"
    );
  }

  #[tokio::test]
  async fn one_family_dying_does_not_touch_another_sign_in() {
    // Signing out one device must not sign out the rest.
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;

    let laptop = Uuid::new_v4();
    let phone = Uuid::new_v4();
    let laptop_token = mint_refresh_token(&db, DAY, user, laptop).await.unwrap();
    let phone_token = mint_refresh_token(&db, DAY, user, phone).await.unwrap();

    rotate_refresh_token(&db, &laptop_token).await.unwrap();
    assert!(rotate_refresh_token(&db, &laptop_token).await.is_err()); // burns `laptop`

    assert!(
      rotate_refresh_token(&db, &phone_token).await.is_ok(),
      "the other device's session is a separate family and must survive"
    );
  }

  #[tokio::test]
  async fn an_expired_token_is_refused_without_burning_anything() {
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let family = Uuid::new_v4();
    let expired = mint_refresh_token(&db, -1, user, family).await.unwrap();
    assert!(rotate_refresh_token(&db, &expired).await.is_err());

    // Expiry is not evidence of theft: a token minted later in the same family
    // still works. (In practice the client is long gone, but conflating "old"
    // with "stolen" would sign people out for merely being idle.)
    let fresh = mint_refresh_token(&db, DAY, user, family).await.unwrap();
    assert!(rotate_refresh_token(&db, &fresh).await.is_ok());
  }

  #[tokio::test]
  async fn a_revoked_token_is_refused() {
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let family = Uuid::new_v4();
    let token = mint_refresh_token(&db, DAY, user, family).await.unwrap();

    revoke_family(&db, family).await.unwrap();
    assert!(rotate_refresh_token(&db, &token).await.is_err());
  }

  #[tokio::test]
  async fn changing_the_password_kills_every_sign_in() {
    // The reason people change a password is that they think someone else is
    // in there. If the intruder's refresh token survives it, they keep the
    // account for another 30 days and the password change achieved nothing.
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let laptop = mint_refresh_token(&db, DAY, user, Uuid::new_v4())
      .await
      .unwrap();
    let intruder = mint_refresh_token(&db, DAY, user, Uuid::new_v4())
      .await
      .unwrap();

    revoke_user_sessions(&db, user).await.unwrap();

    assert!(rotate_refresh_token(&db, &laptop).await.is_err());
    assert!(
      rotate_refresh_token(&db, &intruder).await.is_err(),
      "every family dies, not just the one that asked"
    );
  }

  /// THE BUG: a password change revoked refresh tokens only, so a PAT an
  /// intruder had minted kept full API access after the victim's reset.
  #[tokio::test]
  async fn changing_the_password_kills_personal_access_tokens_too() {
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let other = seed_user(&db).await;
    for owner in [user, other] {
      sqlx::query(
        "INSERT INTO api_tokens (user_id, name, token_hash, scopes) VALUES ($1, 'x', $2, ARRAY['write'])",
      )
      .bind(owner)
      .bind(Uuid::new_v4().to_string())
      .execute(&db)
      .await
      .unwrap();
    }

    revoke_user_sessions(&db, user).await.unwrap();

    let count = |u: Uuid| {
      let db = db.clone();
      async move {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM api_tokens WHERE user_id = $1")
          .bind(u)
          .fetch_one(&db)
          .await
          .unwrap()
      }
    };
    assert_eq!(
      count(user).await,
      0,
      "the intruder's PAT dies with the password"
    );
    assert_eq!(count(other).await, 1, "other users' tokens are untouched");
  }

  #[tokio::test]
  async fn revoking_one_user_leaves_other_users_alone() {
    let Some(db) = pool().await else { return };
    let alice = seed_user(&db).await;
    let bob = seed_user(&db).await;
    let bob_token = mint_refresh_token(&db, DAY, bob, Uuid::new_v4())
      .await
      .unwrap();

    revoke_user_sessions(&db, alice).await.unwrap();

    assert!(
      rotate_refresh_token(&db, &bob_token).await.is_ok(),
      "one user changing their password must not sign out everyone else"
    );
  }

  #[tokio::test]
  async fn a_revoked_family_can_never_be_minted_back_into() {
    // `refresh` spends the old token and THEN mints the new one. A replay
    // burning the family in between would otherwise leave a live token in a
    // family that was just declared compromised — resurrecting it.
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let family = Uuid::new_v4();
    mint_refresh_token(&db, DAY, user, family).await.unwrap();

    revoke_family(&db, family).await.unwrap();

    assert!(
      mint_refresh_token(&db, DAY, user, family).await.is_err(),
      "a dead family must stay dead"
    );
  }

  #[tokio::test]
  async fn spending_is_refused_the_moment_the_family_dies() {
    // Spending checks `revoked_at` in the UPDATE's WHERE, not in an earlier
    // SELECT: a snapshot read is a check-then-act, and a revoke landing between
    // the two would let one more rotation slip through.
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let family = Uuid::new_v4();
    let token = mint_refresh_token(&db, DAY, user, family).await.unwrap();

    revoke_family(&db, family).await.unwrap();
    assert!(rotate_refresh_token(&db, &token).await.is_err());

    // Still unspent: refused, not consumed — the row is evidence, not a
    // casualty.
    let used: Option<Option<DateTime<Utc>>> =
      sqlx::query_scalar("SELECT used_at FROM refresh_tokens WHERE token_hash = $1")
        .bind(sha256_hex(&token))
        .fetch_optional(&db)
        .await
        .unwrap();
    assert!(used.unwrap().is_none());
  }

  #[tokio::test]
  async fn a_replay_racing_a_real_rotation_leaves_nothing_spendable() {
    // The interleaving the reviewer named: an attacker replays a spent token
    // (burning the family) at the same moment the honest client rotates its
    // live one. However it interleaves, the family must end up with no
    // spendable token — that is the invariant, not who wins.
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let family = Uuid::new_v4();

    let stolen = mint_refresh_token(&db, DAY, user, family).await.unwrap();
    rotate_refresh_token(&db, &stolen).await.unwrap(); // now spent
    let live = mint_refresh_token(&db, DAY, user, family).await.unwrap();

    let _ = tokio::join!(
      rotate_refresh_token(&db, &stolen), // replay -> burns the family
      rotate_refresh_token(&db, &live),   // honest client, racing it
    );

    let spendable: i64 = sqlx::query_scalar(
      r#"
        SELECT count(*) FROM refresh_tokens
        WHERE family_id = $1 AND used_at IS NULL AND revoked_at IS NULL
      "#,
    )
    .bind(family)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(
      spendable, 0,
      "a family declared compromised must not keep a usable token, whichever \
       way the race fell"
    );
    assert!(rotate_refresh_token(&db, &live).await.is_err());
  }

  #[tokio::test]
  async fn logout_kills_the_family_the_token_belongs_to() {
    // Signing out cleared only the client's own copy. Anyone holding a stolen
    // one kept the account for 30 more days — and rotation meant they could
    // roll it forward indefinitely.
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let family = Uuid::new_v4();
    let token = mint_refresh_token(&db, DAY, user, family).await.unwrap();

    // What the handler does.
    let found: Option<Uuid> =
      sqlx::query_scalar("SELECT family_id FROM refresh_tokens WHERE token_hash = $1")
        .bind(sha256_hex(&token))
        .fetch_optional(&db)
        .await
        .unwrap();
    revoke_family(&db, found.unwrap()).await.unwrap();

    assert!(rotate_refresh_token(&db, &token).await.is_err());
  }

  #[tokio::test]
  async fn an_unknown_token_is_refused() {
    let Some(db) = pool().await else { return };
    assert!(
      rotate_refresh_token(&db, "mica_rt_not_a_real_token")
        .await
        .is_err()
    );
  }

  #[tokio::test]
  async fn only_the_hash_is_stored_never_the_token() {
    // The plaintext exists exactly once, in the response. A database dump must
    // not be a pile of live credentials.
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let token = mint_refresh_token(&db, DAY, user, Uuid::new_v4())
      .await
      .unwrap();
    assert!(token.starts_with(REFRESH_PREFIX));

    let stored: Option<String> =
      sqlx::query_scalar("SELECT token_hash FROM refresh_tokens WHERE user_id = $1")
        .bind(user)
        .fetch_optional(&db)
        .await
        .unwrap();
    let stored = stored.unwrap();
    assert_eq!(stored, sha256_hex(&token));
    assert!(!stored.contains(&token));
  }

  #[tokio::test]
  async fn concurrent_rotations_of_one_token_produce_exactly_one_winner() {
    // The reason spending is a conditional UPDATE and not read-then-write: two
    // in-flight refreshes (an app that fired twice) must not both mint a
    // session. Postgres picks the winner; the loser is treated as a replay.
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let family = Uuid::new_v4();
    let token = mint_refresh_token(&db, DAY, user, family).await.unwrap();

    let (a, b) = tokio::join!(
      rotate_refresh_token(&db, &token),
      rotate_refresh_token(&db, &token)
    );
    assert_eq!(
      [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
      1,
      "exactly one of two concurrent rotations may win"
    );
  }

  /// The registration gate lives INSIDE the insert, and this pins both arms of
  /// its `WHERE`: closed + an existing account refuses; closed + an empty
  /// instance still lets the first account through.
  ///
  /// Asserting the SQL rather than the handler is deliberate — the handler needs
  /// a whole `AppState`, and the interesting part is not the `if` (there isn't
  /// one any more) but whether the guard can be answered stale. It can't, because
  /// it is evaluated in the same statement and the same snapshot as the write.
  /// A count-then-insert would pass a test like this and still mint two "first"
  /// accounts under concurrency.
  async fn try_register(db: &PgPool, email: &str, open: bool) -> Option<Uuid> {
    sqlx::query_scalar::<_, Uuid>(
      r#"
        INSERT INTO users (email, display_name, password_hash)
        SELECT $1, 'T', 'x'
        WHERE $2 OR NOT EXISTS (SELECT 1 FROM users)
        RETURNING id
      "#,
    )
    .bind(email)
    .bind(open)
    .fetch_optional(db)
    .await
    .unwrap()
  }

  #[tokio::test]
  async fn registration_closed_refuses_once_the_instance_has_an_account() {
    let Some(db) = pool().await else { return };
    // The real database always has accounts by this point (this test suite seeds
    // plenty), which is exactly the state being asserted.
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
      .fetch_one(&db)
      .await
      .unwrap();
    assert!(existing > 0, "precondition: the instance is not empty");

    let email = format!("{}@closed.test", Uuid::new_v4());
    assert!(
      try_register(&db, &email, false).await.is_none(),
      "closed registration must not create an account"
    );
    let landed: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE email = $1")
      .bind(&email)
      .fetch_one(&db)
      .await
      .unwrap();
    assert_eq!(landed, 0, "and must not have written a row either");

    // Open, same statement, same instance: the account appears. Without this half
    // the test above would also pass if the INSERT were simply broken.
    let id = try_register(&db, &email, true)
      .await
      .expect("open registration must create an account");
    sqlx::query("DELETE FROM users WHERE id = $1")
      .bind(id)
      .execute(&db)
      .await
      .ok();
  }

  /// A fresh self-hosted install has to be able to create its first account, or
  /// closing registration by default would make it unusable — nothing to log in
  /// as, and no way in without knowing to set an env var first. Verified on an
  /// EMPTY database, since that is the only state where the exception applies.
  #[tokio::test]
  async fn a_brand_new_instance_may_create_its_first_account() {
    let Some(db) = pool().await else { return };
    // A throwaway database: the shared dev one has users, and emptying it to
    // prove a point about emptiness would be a poor trade.
    // `AssertSqlSafe` because sqlx 0.9 only accepts `&'static str` otherwise, and
    // CREATE/DROP DATABASE cannot take a bind parameter. The name is a UUID this
    // test just minted — no external input reaches it.
    let name = format!("mica_firstrun_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
      .execute(&db)
      .await
      .unwrap();
    let url = std::env::var("DATABASE_URL").unwrap();
    let scratch_url = url
      .rsplit_once('/')
      .map(|(base, _)| format!("{base}/{name}"))
      .unwrap();
    let scratch = PgPool::connect(&scratch_url).await.unwrap();
    sqlx::query(
      "CREATE TABLE users (
         id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
         email text NOT NULL UNIQUE,
         display_name text NOT NULL,
         password_hash text NOT NULL,
         created_at timestamptz NOT NULL DEFAULT now(),
         avatar_key text
       )",
    )
    .execute(&scratch)
    .await
    .unwrap();

    // Closed — and it still goes through, because there is nobody here yet.
    assert!(
      try_register(&scratch, "first@new.test", false)
        .await
        .is_some(),
      "the first account must be creatable on an empty instance"
    );
    // ...and the door shuts behind it.
    assert!(
      try_register(&scratch, "second@new.test", false)
        .await
        .is_none(),
      "the second account must be refused while registration is closed"
    );

    scratch.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}")))
      .execute(&db)
      .await
      .ok();
  }

  /// C4: a tombstone is not an account. Before tombstones, the sole user
  /// deleting themselves left an empty instance whose next sign-up became the
  /// first account (and admin); a leftover tombstone row must not turn that
  /// into an instance nobody can join with registration closed.
  #[tokio::test]
  async fn an_instance_holding_only_tombstones_is_still_fresh() {
    let Some(db) = pool().await else { return };
    let (scratch, name) = scratch_db(&db, "mica_tombfresh").await;
    sqlx::query(
      "INSERT INTO users(email,display_name,password_hash,deleted_at) \
       VALUES('deleted+x@deleted.invalid','','',now())",
    )
    .execute(&scratch)
    .await
    .unwrap();
    let mut tx = scratch.begin().await.unwrap();
    let first = insert_registered_user(&mut tx, "first@new.test", "F", "x", false)
      .await
      .unwrap();
    tx.commit().await.unwrap();
    assert!(
      first.is_some(),
      "closed registration still admits the first live account"
    );
    let is_admin: bool =
      sqlx::query_scalar("SELECT is_admin FROM users WHERE email='first@new.test'")
        .fetch_one(&scratch)
        .await
        .unwrap();
    assert!(is_admin);

    scratch.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}")))
      .execute(&db)
      .await
      .ok();
  }

  /// P1-01: two concurrent registrations on an EMPTY instance must not both
  /// become admin.
  ///
  /// The bug is in the ADMIN FLAG, not in the door. The statement computes both
  /// `is_admin` and the closed-door guard from `NOT EXISTS (SELECT 1 FROM users)`
  /// — two subqueries in one data-modifying statement, each evaluated against the
  /// STATEMENT's snapshot. Under READ COMMITTED two concurrent registrations each
  /// take that snapshot before the other's insert is visible, so both compute
  /// `is_admin = true` and the instance ends up with TWO administrators. (The
  /// recommended remedy in the plan — "the second request is refused" — describes
  /// the CLOSED case; with registration OPEN, two accounts is correct and only
  /// the flag is wrong. Both halves are asserted below.)
  ///
  /// No constraint can catch it: the two racers use different addresses by
  /// construction, so `users_email_key` never fires, and "at most one admin" has
  /// no schema-level expression precisely because the role is meant to be handed
  /// over later (migration 0021 explains why it is a column, not a name).
  ///
  /// The interleaving is CONSTRUCTED, not raced: A takes the advisory lock and
  /// inserts WITHOUT committing, B (the same production function) blocks on the
  /// lock, and only then does A commit. Without the lock B does not block at all —
  /// it reads the same empty table and computes `is_admin = true` too.
  #[tokio::test]
  async fn concurrent_first_registrations_cannot_both_become_admin() {
    let Some(db) = pool().await else { return };
    let (scratch, name) = scratch_db(&db, "mica_regrace").await;

    // A: take the lock and insert, but stay uncommitted. Registration is OPEN —
    // the race is about both requests believing they are the FIRST, whatever the
    // door is doing.
    let mut tx_a = scratch.begin().await.unwrap();
    let a = insert_registered_user(&mut tx_a, "a@race.test", "A", "x", true)
      .await
      .unwrap();
    assert!(a.is_some(), "the first racer lands");

    // B: the SAME production function on a second connection. It must block on
    // the advisory lock rather than read the empty table.
    let scratch_b = scratch.clone();
    let racer = tokio::spawn(async move {
      let mut tx_b = scratch_b.begin().await.unwrap();
      let result = insert_registered_user(&mut tx_b, "b@race.test", "B", "x", true).await;
      tx_b.commit().await.unwrap();
      result
    });

    // Observe the block rather than assuming it: a fixed sleep would let this
    // pass on a slow machine by committing A before B ever started. The result is
    // only logged — the assertion below is what decides, so the test fails on the
    // DEFECT (two admins) rather than on the missing serialization point.
    let blocked = saw_advisory_waiter(&db).await;
    assert!(
      blocked,
      "expected the second registration to block on the lock; without that the \
       assertions below would be racing rather than interleaved"
    );
    tx_a.commit().await.unwrap();

    // With registration OPEN both accounts are expected — the door is open. The
    // LOSER is the one that must not carry the flag.
    let b = racer.await.unwrap().unwrap();
    assert!(
      b.is_some(),
      "registration is open, so the second account is legitimately created"
    );

    let admins: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE is_admin")
      .fetch_one(&scratch)
      .await
      .unwrap();
    assert_eq!(
      admins, 1,
      "exactly one account may be the first-run admin; two means both racers \
       computed is_admin from the same pre-commit snapshot"
    );

    // And the closed-door half, on a fresh empty instance: the guard must let
    // exactly ONE account through.
    let (scratch2, name2) = scratch_db(&db, "mica_regrace_closed").await;
    let mut tx_c = scratch2.begin().await.unwrap();
    assert!(
      insert_registered_user(&mut tx_c, "c@race.test", "C", "x", false)
        .await
        .unwrap()
        .is_some(),
      "the first account gets through the closed door"
    );
    let scratch2_b = scratch2.clone();
    let loser = tokio::spawn(async move {
      let mut tx_d = scratch2_b.begin().await.unwrap();
      let result = insert_registered_user(&mut tx_d, "d@race.test", "D", "x", false).await;
      tx_d.commit().await.unwrap();
      result
    });
    let blocked = saw_advisory_waiter(&db).await;
    assert!(
      blocked,
      "expected the second (closed-door) registration to block on the lock"
    );
    tx_c.commit().await.unwrap();
    assert!(
      loser.await.unwrap().unwrap().is_none(),
      "the closed door must admit exactly one account"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
      .fetch_one(&scratch2)
      .await
      .unwrap();
    assert_eq!(
      count, 1,
      "exactly one account exists after the closed-door race"
    );

    drop_scratch_db(&db, scratch, scratch2, &name, &name2).await;
  }

  /// A throwaway database with the `users` columns these tests touch, plus the
  /// name needed to drop it again.
  ///
  /// The shared dev database has users, and every test here is ABOUT emptiness,
  /// so emptying it to prove a point would be a poor trade.
  async fn scratch_db(db: &PgPool, prefix: &str) -> (PgPool, String) {
    // `AssertSqlSafe` because sqlx 0.9 only accepts `&'static str` otherwise, and
    // CREATE/DROP DATABASE cannot take a bind parameter. The name is a UUID this
    // test just minted — no external input reaches it.
    let name = format!("{prefix}_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
      .execute(db)
      .await
      .unwrap();
    let url = std::env::var("DATABASE_URL").unwrap();
    let scratch_url = url
      .rsplit_once('/')
      .map(|(base, _)| format!("{base}/{name}"))
      .unwrap();
    let scratch = PgPool::connect(&scratch_url).await.unwrap();
    sqlx::query(
      "CREATE TABLE users (
         id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
         email text NOT NULL UNIQUE,
         display_name text NOT NULL,
         password_hash text NOT NULL,
         created_at timestamptz NOT NULL DEFAULT now(),
         avatar_key text,
         email_verified_at timestamptz,
         is_admin boolean NOT NULL DEFAULT false,
         deleted_at timestamptz
       )",
    )
    .execute(&scratch)
    .await
    .unwrap();
    (scratch, name)
  }

  async fn drop_scratch_db(
    db: &PgPool,
    scratch: PgPool,
    scratch2: PgPool,
    name: &str,
    name2: &str,
  ) {
    scratch.close().await;
    scratch2.close().await;
    for n in [name, name2] {
      sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP DATABASE {n}")))
        .execute(db)
        .await
        .ok();
    }
  }

  /// Wait a bounded time for some connection to be blocked on the registration
  /// advisory lock, and report whether one was seen.
  ///
  /// Polling `pg_locks` makes the barrier OBSERVED instead of assumed — a fixed
  /// sleep would let these tests pass on a slow machine by committing before the
  /// racer even started.
  ///
  /// It does NOT panic on timeout, and that is deliberate. An earlier version
  /// asserted a waiter existed, which meant that on the pre-fix code the test
  /// failed with "never reached the lock" — technically a failure, but it proves
  /// only that there is no serialization point, not the thing we actually care
  /// about (a second admin). Returning a bool lets the caller commit and let the
  /// real invariant — the admin count — be what fails. A regression test should
  /// break on the defect, not on the mechanism that prevents it.
  async fn saw_advisory_waiter(db: &PgPool) -> bool {
    for _ in 0..200 {
      let waiting: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_locks l
           JOIN pg_database d ON d.oid = l.database
          WHERE l.locktype = 'advisory' AND NOT l.granted
            AND d.datname NOT IN ('postgres', 'template0', 'template1')",
      )
      .fetch_one(db)
      .await
      .unwrap();
      if waiting > 0 {
        return true;
      }
      tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    false
  }

  /// P2-01: a deleted account's still-valid JWT must not keep spending.
  ///
  /// The access token is stateless and therefore unrevocable: `delete_account`
  /// cascades away the `users` row, the `refresh_tokens` and the `api_tokens`,
  /// and the JWT in the caller's hand is none of those. So it keeps verifying
  /// until `exp` (~1 hour by default), and the endpoints that spend the
  /// operator's AI credit are the ones where that window costs real money.
  ///
  /// This pins the rule those endpoints call: after the row is gone, the check
  /// refuses. Testing `ensure_user_exists` rather than the handlers is a
  /// deliberate trade — the handlers need an `AppState` (AI config, pool,
  /// mailer), and building one would test the scaffolding more than the rule.
  /// The call sites are pinned by the compiler only in the weak sense that they
  /// call the helper; what matters is that the rule itself is right.
  #[tokio::test]
  async fn a_deleted_account_stops_passing_the_existence_check() {
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;

    // Alive: the check passes, so the endpoints behave exactly as before.
    ensure_user_exists(&db, user)
      .await
      .expect("an existing account passes");

    // Deleted the way `delete_user_and_owned` does it — the row goes.
    sqlx::query("DELETE FROM users WHERE id = $1")
      .bind(user)
      .execute(&db)
      .await
      .unwrap();

    assert!(
      matches!(
        ensure_user_exists(&db, user).await,
        Err(ApiError::Unauthorized)
      ),
      "a token for a user who no longer exists must be refused (401, so clients \
       re-authenticate) rather than spending the operator's AI credit until it \
       expires on its own"
    );

    // And an id that never existed is refused identically — no way to tell
    // "deleted" from "never was", which is fine, since neither identifies anyone.
    assert!(matches!(
      ensure_user_exists(&db, Uuid::new_v4()).await,
      Err(ApiError::Unauthorized)
    ));
  }
}
