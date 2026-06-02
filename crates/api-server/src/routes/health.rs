use axum::{Json, extract::State};
use mica_app_core::AppState;
use mica_infra::{ApiResult, ping_pg_pool};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
  status: &'static str,
  service: &'static str,
  version: &'static str,
}

pub async fn health() -> Json<HealthResponse> {
  Json(HealthResponse {
    status: "ok",
    service: "mica-api-server",
    version: env!("CARGO_PKG_VERSION"),
  })
}

pub async fn ready(State(state): State<AppState>) -> ApiResult<Json<HealthResponse>> {
  ping_pg_pool(&state.db).await?;

  Ok(Json(HealthResponse {
    status: "ready",
    service: "mica-api-server",
    version: env!("CARGO_PKG_VERSION"),
  }))
}
