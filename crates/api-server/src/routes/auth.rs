use argon2::{
  Argon2,
  password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
  Json,
  extract::{Request, State},
  http::{HeaderMap, StatusCode, header::AUTHORIZATION},
  middleware::Next,
  response::Response,
};
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult};
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
  refresh_token: String,
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
}

#[derive(Debug, FromRow)]
struct UserRow {
  id: Uuid,
  email: String,
  display_name: String,
  password_hash: String,
  created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
  sub: String,
  exp: usize,
}

pub async fn register(
  State(state): State<AppState>,
  Json(payload): Json<RegisterRequest>,
) -> ApiResult<Json<AuthResponse>> {
  // A self-hosted operator can lock the node to its current accounts by setting
  // MICA_REGISTRATION_ENABLED=false — the public register endpoint then refuses
  // rather than minting accounts. Login/refresh for existing users is unaffected.
  if !state.config.registration_enabled {
    return Err(ApiError::Forbidden);
  }

  let email = normalize_email(&payload.email)?;
  let display_name = normalize_display_name(&payload.display_name)?;
  validate_password(&payload.password)?;

  let password_hash = hash_password(&payload.password)?;

  let user = sqlx::query_as::<_, UserRow>(
    r#"
      INSERT INTO users (email, display_name, password_hash)
      VALUES ($1, $2, $3)
      RETURNING id, email, display_name, password_hash, created_at
    "#,
  )
  .bind(email)
  .bind(display_name)
  .bind(password_hash)
  .fetch_one(&state.db)
  .await
  .map_err(map_insert_user_error)?;

  Ok(Json(auth_response(&state, user).await?))
}

pub async fn login(
  State(state): State<AppState>,
  Json(payload): Json<LoginRequest>,
) -> ApiResult<Json<AuthResponse>> {
  let email = normalize_email(&payload.email)?;

  let user = sqlx::query_as::<_, UserRow>(
    r#"
      SELECT id, email, display_name, password_hash, created_at
      FROM users
      WHERE email = $1
    "#,
  )
  .bind(email)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::Unauthorized)?;

  verify_password(&payload.password, &user.password_hash)?;

  Ok(Json(auth_response(&state, user).await?))
}

pub async fn me(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Json<MeResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;

  let user = sqlx::query_as::<_, UserRow>(
    r#"
      SELECT id, email, display_name, password_hash, created_at
      FROM users
      WHERE id = $1
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
      WHERE id = $2
      RETURNING id, email, display_name, password_hash, created_at
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

  if payload.new_password.len() < 8 {
    return Err(ApiError::BadRequest(
      "new password must be at least 8 characters".to_string(),
    ));
  }

  let user = sqlx::query_as::<_, UserRow>(
    r#"
      SELECT id, email, display_name, password_hash, created_at
      FROM users
      WHERE id = $1
    "#,
  )
  .bind(user_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::Unauthorized)?;

  verify_password(&payload.current_password, &user.password_hash)?;
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
/// current password.
pub async fn delete_account(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<DeleteAccountRequest>,
) -> ApiResult<StatusCode> {
  let user_id = user_id_from_headers(&state, &headers).await?;

  let user = sqlx::query_as::<_, UserRow>(
    "SELECT id, email, display_name, password_hash, created_at FROM users WHERE id = $1",
  )
  .bind(user_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::Unauthorized)?;
  verify_password(&payload.password, &user.password_hash)?;

  delete_user_and_owned(&state.db, user_id).await?;
  Ok(StatusCode::NO_CONTENT)
}

/// The transactional cascade behind [`delete_account`], separated so a DB test
/// can exercise it without minting a token. Deletes the user's owned workspaces
/// (all content cascades on `workspace_id`) and the shares they authored, then
/// the user row (api_tokens / refresh_tokens / workspace_members cascade on
/// `user_id`). If the account still authored rows in workspaces owned by OTHERS
/// (`created_by` / `actor_id` / `uploaded_by` are `ON DELETE RESTRICT`), the
/// final delete is a FK violation → the whole transaction rolls back and we
/// return a readable 409 rather than silently stripping a co-owner's document
/// history. Atomic: either the account and everything it owns is gone, or
/// nothing is.
pub async fn delete_user_and_owned(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<()> {
  let mut tx = db.begin().await?;
  sqlx::query("DELETE FROM workspaces WHERE owner_id = $1")
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
  sqlx::query("DELETE FROM document_shares WHERE created_by = $1")
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
  match sqlx::query("DELETE FROM users WHERE id = $1")
    .bind(user_id)
    .execute(&mut *tx)
    .await
  {
    Ok(_) => {
      tx.commit().await?;
      Ok(())
    }
    Err(error) => {
      // tx is dropped here (rolled back) before we return.
      let is_fk = error
        .as_database_error()
        .and_then(|d| d.code())
        .as_deref()
        == Some("23503");
      if is_fk {
        Err(ApiError::Conflict(
          "your account still has content in workspaces owned by others; \
           leave those workspaces first, then delete."
            .to_string(),
        ))
      } else {
        Err(error.into())
      }
    }
  }
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
  // 244 bits of randomness from two v4 UUIDs — same recipe as a PAT
  // (tokens.rs), no extra crate.
  let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
  let token = format!("{REFRESH_PREFIX}{secret}");
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
  Json(payload): Json<RefreshRequest>,
) -> ApiResult<Json<AuthResponse>> {
  let (user_id, family_id) = rotate_refresh_token(&state.db, &payload.refresh_token).await?;

  let user = sqlx::query_as::<_, UserRow>(
    r#"
      SELECT id, email, display_name, password_hash, created_at
      FROM users
      WHERE id = $1
    "#,
  )
  .bind(user_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::Unauthorized)?;

  Ok(Json(
    auth_response_in_family(&state, user, family_id).await?,
  ))
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
  Json(payload): Json<RefreshRequest>,
) -> ApiResult<StatusCode> {
  let family: Option<Uuid> =
    sqlx::query_scalar("SELECT family_id FROM refresh_tokens WHERE token_hash = $1")
      .bind(sha256_hex(&payload.refresh_token))
      .fetch_optional(&state.db)
      .await?;

  // The whole family, not just this token: rotation means the sign-in is a
  // chain, and leaving its other links alive would sign nothing out.
  if let Some(family) = family {
    revoke_family(&state.db, family).await?;
  }

  Ok(StatusCode::NO_CONTENT)
}

/// Burn every sign-in a user has, everywhere. The access tokens already issued
/// are JWTs and cannot be recalled — they die on their own within
/// `access_token_ttl_seconds` (1h by default), which is the price of them being
/// stateless. Nothing can mint a new one after this.
pub async fn revoke_user_sessions(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<()> {
  sqlx::query(
    r#"
      UPDATE refresh_tokens
      SET revoked_at = now()
      WHERE user_id = $1 AND revoked_at IS NULL
    "#,
  )
  .bind(user_id)
  .execute(db)
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
  let _ = sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1")
    .bind(&hash)
    .execute(&state.db)
    .await;

  let scopes = row.scopes.iter().filter_map(|s| Scope::parse(s)).collect();
  Ok(Auth {
    user_id: row.user_id,
    scopes,
  })
}

pub(crate) async fn user_id_from_headers(state: &AppState, headers: &HeaderMap) -> ApiResult<Uuid> {
  let token = headers
    .get(AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.strip_prefix("Bearer "))
    .ok_or(ApiError::Unauthorized)?;
  Ok(resolve_token(state, token).await?.user_id)
}

/// Decode a bare JWT access token into a user id. Used by the WebSocket handler,
/// which receives the token via query string rather than an Authorization header.
pub(crate) fn user_id_from_token(state: &AppState, token: &str) -> ApiResult<Uuid> {
  let token = decode::<Claims>(
    token,
    &DecodingKey::from_secret(state.config.jwt_secret.as_bytes()),
    &Validation::default(),
  )
  .map_err(|_| ApiError::Unauthorized)?;

  Uuid::parse_str(&token.claims.sub).map_err(|_| ApiError::Unauthorized)
}

/// Axum middleware: authenticate every non-public `/api` request and enforce the
/// scope its HTTP method needs (safe methods → `read`, mutating → `write`). JWT
/// (password) sessions carry full scope; PATs are checked against their grant.
pub async fn scope_guard(
  State(state): State<AppState>,
  request: Request,
  next: Next,
) -> Result<Response, ApiError> {
  if is_public(request.uri().path()) {
    return Ok(next.run(request).await);
  }
  let token = request
    .headers()
    .get(AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.strip_prefix("Bearer "))
    .ok_or(ApiError::Unauthorized)?;
  let auth = resolve_token(&state, token).await?;
  let need = if request.method().is_safe() {
    Scope::Read
  } else {
    Scope::Write
  };
  if !auth.grants(need) {
    return Err(ApiError::Forbidden);
  }
  Ok(next.run(request).await)
}

fn is_public(path: &str) -> bool {
  path.ends_with("/health")
    || path.ends_with("/ready")
    || path.ends_with("/auth/login")
    || path.ends_with("/auth/register")
    // You refresh precisely BECAUSE the access token is dead — demanding a live
    // one here would be a deadlock, and the endpoint's own credential is the
    // refresh token in its body. The same router-wide `scope_guard` already
    // silently 401'd the blob endpoint for a month (see below), so this is
    // pinned by a test rather than left to be rediscovered.
    || path.ends_with("/auth/refresh")
    // Signing out must work with a dead access token — that is the normal case
    // for someone who left the app closed. The refresh token in the body is the
    // credential.
    || path.ends_with("/auth/logout")
    || is_blob_path(path)
}

/// `…/files/{file_id}/blob` (+ an optional cosmetic filename segment) — the
/// image capability link. Public BY DESIGN: the unguessable file_id is the
/// capability, which is the only reason a copied Markdown image still renders
/// in Typora / a browser (files::blob). This router-wide `scope_guard` shipped
/// with PATs (a46db0a) and silently 401'd it for a month, quietly breaking the
/// copy-portability the endpoint exists for — hence the explicit test in
/// tests/blob_public.rs.
fn is_blob_path(path: &str) -> bool {
  let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
  let Some(i) = segs.iter().position(|s| *s == "files") else {
    return false;
  };
  // files / {file_id} / blob   [ / {filename} ]
  segs.get(i + 2) == Some(&"blob") && (segs.len() == i + 3 || segs.len() == i + 4)
}

fn normalize_email(email: &str) -> ApiResult<String> {
  let email = email.trim().to_ascii_lowercase();
  if email.is_empty() || !email.contains('@') {
    return Err(ApiError::BadRequest("valid email is required".to_string()));
  }

  Ok(email)
}

fn normalize_display_name(display_name: &str) -> ApiResult<String> {
  let display_name = display_name.trim().to_string();
  if display_name.is_empty() {
    return Err(ApiError::BadRequest("display_name is required".to_string()));
  }

  Ok(display_name)
}

fn validate_password(password: &str) -> ApiResult<()> {
  if password.len() < 8 {
    return Err(ApiError::BadRequest(
      "password must be at least 8 characters".to_string(),
    ));
  }

  Ok(())
}

fn hash_password(password: &str) -> ApiResult<String> {
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
  sqlx::query(
    r#"
      INSERT INTO users (email, display_name, password_hash)
      VALUES ($1, $2, $3)
      ON CONFLICT (email) DO UPDATE SET password_hash = EXCLUDED.password_hash
    "#,
  )
  .bind(email)
  .bind("Test User")
  .bind(password_hash)
  .execute(db)
  .await?;
  Ok(())
}

fn verify_password(password: &str, password_hash: &str) -> ApiResult<()> {
  let parsed_hash =
    PasswordHash::new(password_hash).map_err(|error| ApiError::Internal(error.to_string()))?;

  Argon2::default()
    .verify_password(password.as_bytes(), &parsed_hash)
    .map_err(|_| ApiError::Unauthorized)
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
    }
  }
}

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

  // The image blob link is a capability url (the file_id IS the credential) —
  // public on purpose so a copied Markdown image still renders in Typora / a
  // browser. This router-wide guard once swallowed it for a month without a
  // peep; nothing else notices, because in-app rendering uses a different
  // (authenticated) path. Hence pinning it here.
  #[test]
  fn image_blob_link_is_public() {
    const WS: &str = "/api/workspaces/5a398481-bcf5-4a5e-83f0-a891bf0bad7e";
    const FID: &str = "55b3e5ff-4117-4d65-9434-0b17922d8e87";
    assert!(is_public(&format!("{WS}/files/{FID}/blob")));
    // …and with the cosmetic filename segment.
    assert!(is_public(&format!("{WS}/files/{FID}/blob/diagram.png")));
    assert!(is_public(&format!("{WS}/files/{FID}/blob/%E5%9B%BE.png")));
  }

  #[test]
  fn only_the_blob_link_is_public_under_files() {
    const WS: &str = "/api/workspaces/5a398481-bcf5-4a5e-83f0-a891bf0bad7e";
    const FID: &str = "55b3e5ff-4117-4d65-9434-0b17922d8e87";
    // Metadata, upload plumbing and deletion all stay authenticated.
    assert!(!is_public(&format!("{WS}/files/{FID}")));
    assert!(!is_public(&format!("{WS}/files/presign")));
    assert!(!is_public(&format!("{WS}/files/complete")));
    assert!(!is_public(&format!("{WS}/files/resolve")));
    assert!(!is_public(&format!("{WS}/files/import-url")));
    // Nothing deeper than one filename segment is a blob link.
    assert!(!is_public(&format!("{WS}/files/{FID}/blob/a/b")));
    // And the word "blob" elsewhere doesn't open a door.
    assert!(!is_public(&format!("{WS}/documents/{FID}/blob")));
  }

  #[test]
  fn ordinary_endpoints_stay_guarded() {
    assert!(!is_public("/api/workspaces"));
    assert!(!is_public("/api/documents/abc"));
    assert!(is_public("/api/health"));
  }

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

  #[test]
  fn public_paths_bypass_auth() {
    assert!(is_public("/api/health"));
    assert!(is_public("/api/auth/login"));
    assert!(is_public("/api/auth/register"));
    assert!(!is_public("/api/workspaces"));
    assert!(!is_public("/api/auth/tokens"));
  }

  #[test]
  fn refresh_is_public_or_it_could_never_be_called() {
    // Refreshing needs no access token by definition — the whole point is that
    // yours has expired. If the router-wide guard ever claims this path, every
    // client is locked out at the 24h mark with no way back except a re-login,
    // which is the exact bug refresh exists to kill.
    assert!(is_public("/api/auth/refresh"));
  }
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

    delete_user_and_owned(&db, owner)
      .await
      .expect("deleting an account that owns only its own data must succeed");

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
    // The owner, their workspace, and its document (cascade) are all gone.
    assert_eq!(count("SELECT count(*) FROM users WHERE id=$1", owner).await, 0);
    assert_eq!(
      count("SELECT count(*) FROM workspaces WHERE id=$1", ws).await,
      0
    );
    assert_eq!(
      count("SELECT count(*) FROM documents WHERE id=$1", doc).await,
      0,
      "content in an owned workspace must cascade"
    );
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
}
