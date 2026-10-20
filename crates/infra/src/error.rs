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

  /// A 400 whose *reason* the client has to act on, not merely display.
  /// [`Self::BadRequest`] serializes as the generic code `bad_request`, so the
  /// only thing telling "the archive has no Markdown in it" apart from any other
  /// rejection was its English message — and the client then showed that message
  /// to the user verbatim, untranslated. The first field is the stable machine
  /// code; the second is still the human-readable message for logs.
  #[error("bad request: {1}")]
  BadRequestCode(&'static str, String),

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

impl ApiError {
  /// The message a CLIENT may see. Database and migration errors carry the
  /// raw sqlx/Postgres text (constraint and column names, sometimes values);
  /// that is logged here and replaced with a generic line. `Internal` keeps
  /// its message: those are written by hand for the user ("AI returned an
  /// empty response"), and the sites that used to embed foreign error text
  /// with secrets in it no longer do.
  pub fn public_message(&self) -> String {
    match self {
      Self::Database(_) | Self::Migration(_) => {
        tracing::error!(error = %self, "database error");
        "internal database error".to_string()
      }
      _ => self.to_string(),
    }
  }
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
      Self::BadRequestCode(code, _) => (StatusCode::BAD_REQUEST, *code),
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
      message: self.public_message(),
    };

    (status, Json(body)).into_response()
  }
}

#[cfg(test)]
mod public_message_tests {
  use super::ApiError;

  /// THE BUG (C3): a database error's text — constraint, column, sometimes the
  /// offending value — went straight into the response body.
  #[test]
  fn database_errors_do_not_reach_the_client_verbatim() {
    let error = ApiError::Database(sqlx::Error::Protocol("relation \"secret_table\" does not exist".into()));
    assert_eq!(error.public_message(), "internal database error");
    assert!(error.to_string().contains("secret_table"), "the log keeps the detail");

    let authored = ApiError::Internal("AI returned an empty response".into());
    assert_eq!(authored.public_message(), "internal error: AI returned an empty response");
  }
}
