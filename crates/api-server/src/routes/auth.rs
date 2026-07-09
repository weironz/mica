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
  user: UserResponse,
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

  Ok(Json(auth_response(&state, user)?))
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

  Ok(Json(auth_response(&state, user)?))
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

  Ok(StatusCode::NO_CONTENT)
}

fn auth_response(state: &AppState, user: UserRow) -> ApiResult<AuthResponse> {
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

  Ok(AuthResponse {
    access_token,
    token_type: "Bearer",
    expires_at,
    user: UserResponse::from(user),
  })
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
pub async fn seed_test_user(
  db: &sqlx::PgPool,
  email: &str,
  password: &str,
) -> anyhow::Result<()> {
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
    assert!(h.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
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
}
