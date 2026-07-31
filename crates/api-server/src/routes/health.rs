use axum::{Json, extract::State};
use mica_app_core::AppState;
use mica_infra::{ApiResult, ping_pg_pool};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
  status: &'static str,
  service: &'static str,
  version: &'static str,
  /// Whether `POST /auth/register` would accept a new account right now.
  ///
  /// Only `ready` fills this in, because answering it honestly needs the
  /// database — see [`registration_open`]. `health` is the liveness probe and
  /// deliberately touches nothing, so it omits the field rather than guessing;
  /// a client seeing it absent should read that as "don't know" and leave its
  /// registration entry exactly as it was.
  #[serde(skip_serializing_if = "Option::is_none")]
  registration_open: Option<bool>,
}

pub async fn health() -> Json<HealthResponse> {
  Json(HealthResponse {
    status: "ok",
    service: "mica-api-server",
    version: env!("CARGO_PKG_VERSION"),
    registration_open: None,
  })
}

/// Is public registration actually usable on this instance?
///
/// NOT the same question as `config.registration_enabled`. A brand-new instance
/// with no users at all always accepts its very first account, whatever the flag
/// says — `auth::register` folds that exception into its INSERT so a fresh
/// self-hosted install is not locked out of itself. A client that hid its
/// registration entry on the flag alone would recreate precisely the lockout the
/// server goes out of its way to avoid, so the same two-part condition is
/// mirrored here for the client to read.
async fn registration_open(state: &AppState) -> bool {
  if state.config.registration_enabled {
    return true;
  }
  // No users yet → the first-account exception applies. A failed query means we
  // cannot prove the instance is empty, and the safe answer to "may strangers
  // sign up here?" is no.
  sqlx::query_scalar::<_, bool>("SELECT NOT EXISTS (SELECT 1 FROM users)")
    .fetch_one(&state.db)
    .await
    .unwrap_or(false)
}

pub async fn ready(State(state): State<AppState>) -> ApiResult<Json<HealthResponse>> {
  ping_pg_pool(&state.db).await?;

  Ok(Json(HealthResponse {
    status: "ready",
    service: "mica-api-server",
    version: env!("CARGO_PKG_VERSION"),
    registration_open: Some(registration_open(&state).await),
  }))
}
