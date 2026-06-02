use axum::{
  Json,
  http::StatusCode,
  response::{IntoResponse, Response},
};
use serde::Serialize;

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
  #[error("bad request: {0}")]
  BadRequest(String),

  #[error("unauthorized")]
  Unauthorized,

  #[error("forbidden")]
  Forbidden,

  #[error("not found")]
  NotFound,

  #[error("conflict: {0}")]
  Conflict(String),

  #[error("service unavailable: {0}")]
  Unavailable(String),

  #[error("database error: {0}")]
  Database(#[from] sqlx::Error),

  #[error("database migration error: {0}")]
  Migration(#[from] sqlx::migrate::MigrateError),

  #[error("internal error: {0}")]
  Internal(String),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
  code: &'static str,
  message: String,
}

impl IntoResponse for ApiError {
  fn into_response(self) -> Response {
    let (status, code) = match &self {
      Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
      Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
      Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
      Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
      Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
      Self::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable"),
      Self::Database(_) | Self::Migration(_) | Self::Internal(_) => {
        (StatusCode::INTERNAL_SERVER_ERROR, "internal")
      }
    };

    let body = ErrorBody {
      code,
      message: self.to_string(),
    };

    (status, Json(body)).into_response()
  }
}
