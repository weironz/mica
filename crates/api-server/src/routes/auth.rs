use argon2::{
  Argon2,
  password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::{
  Json,
  extract::State,
  http::{HeaderMap, StatusCode, header::AUTHORIZATION},
};
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
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
  let user_id = user_id_from_headers(&state, &headers)?;

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
  let user_id = user_id_from_headers(&state, &headers)?;
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
  let user_id = user_id_from_headers(&state, &headers)?;

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

pub(crate) fn user_id_from_headers(state: &AppState, headers: &HeaderMap) -> ApiResult<Uuid> {
  let token = headers
    .get(AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.strip_prefix("Bearer "))
    .ok_or(ApiError::Unauthorized)?;

  user_id_from_token(state, token)
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
