//! Import completion mail and its account-owned preference.
//!
//! Claim once after the terminal job has been persisted. The user row lock
//! serializes the claim with opt-out/deletion; network IO happens afterwards.
//! There is deliberately no delivery/retry queue: 'sending' means uncertain
//! after a crash, and 'failed' is recorded on a provider error, never 'sent'.
use axum::{Json, extract::State, http::HeaderMap};
use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult, Mail};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::auth::user_id_from_headers;
use super::password_reset::escape_attr;

#[derive(Serialize, Deserialize)]
pub struct ImportNotifications {
  enabled: bool,
}

pub async fn get_preferences(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<ImportNotifications>> {
  let user = user_id_from_headers(&state, &headers).await?;
  let enabled = sqlx::query_scalar::<_, bool>(
    "SELECT import_completion_email_enabled FROM users WHERE id = $1 AND deleted_at IS NULL",
  )
  .bind(user)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::NotFound)?;
  Ok(Json(ImportNotifications { enabled }))
}

pub async fn put_preferences(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(body): Json<ImportNotifications>,
) -> ApiResult<Json<ImportNotifications>> {
  let user = user_id_from_headers(&state, &headers).await?;
  let enabled = sqlx::query_scalar::<_, bool>(
    "UPDATE users SET import_completion_email_enabled = $2 WHERE id = $1 AND deleted_at IS NULL \
     RETURNING import_completion_email_enabled",
  )
  .bind(user)
  .bind(body.enabled)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::NotFound)?;
  Ok(Json(ImportNotifications { enabled }))
}

#[derive(FromRow)]
struct Completion {
  email: String,
  status: String,
  total: i32,
  done: i32,
  skipped_total: i32,
  image_failures_total: i32,
  error: Option<String>,
}

/// The persisted owner, preference and terminal counts are the authority.
async fn claim(state: &AppState, user: Uuid, id: Uuid) -> Result<Option<Completion>, sqlx::Error> {
  sqlx::query_as::<_, Completion>(
    "WITH recipient AS MATERIALIZED (\
       SELECT id, email FROM users WHERE id = $1 AND deleted_at IS NULL \
       AND import_completion_email_enabled FOR UPDATE\
     ), claimed AS (\
       UPDATE import_jobs j SET completion_email_state = 'sending' FROM recipient r \
       WHERE j.id = $2 AND j.user_id = r.id AND j.status IN ('done','error','cancelled') \
       AND j.completion_email_state IS NULL \
       RETURNING j.status, j.total, j.done, j.skipped_total, \
                 j.image_failures_total, j.error\
     ) SELECT r.email, c.* FROM recipient r CROSS JOIN claimed c",
  )
  .bind(user)
  .bind(id)
  .fetch_optional(&state.db)
  .await
}

pub(super) async fn notify(state: &AppState, user: Uuid, id: Uuid) {
  if !state.mailer.delivers_email() {
    tracing::info!(%id, "import completion email not attempted: no delivery backend configured");
    return;
  }
  let completion = match claim(state, user, id).await {
    Ok(Some(value)) => value,
    Ok(None) => return,
    Err(error) => {
      tracing::warn!(%id, %error, "could not claim import completion email");
      return;
    }
  };
  let mail = completion_mail(&state.config.app_base_url, &completion);
  let outcome = match state.mailer.send(&mail).await {
    Ok(()) => "sent",
    Err(error) => {
      tracing::warn!(%id, %error, "import completion email failed (not retried)");
      "failed"
    }
  };
  if let Err(error) = sqlx::query(
    "UPDATE import_jobs SET completion_email_state = $2 WHERE id = $1 AND user_id = $3 \
     AND completion_email_state = 'sending'",
  )
  .bind(id)
  .bind(outcome)
  .bind(user)
  .execute(&state.db)
  .await
  {
    tracing::warn!(%id, %error, "could not record import email delivery outcome");
  }
}

fn completion_mail(base: &str, job: &Completion) -> Mail {
  let status = match job.status.as_str() {
    "done" => "完成",
    "cancelled" => "已取消",
    _ => "失败",
  };
  let root = base.trim_end_matches('/');
  let link = root;
  let failure = if job.status == "error" && job.total == 0 {
    "未知（页面规划前失败）".to_owned()
  } else {
    job.total.saturating_sub(job.done).max(0).to_string()
  };
  let detail = job.error.as_deref().map_or(String::new(), |error| {
    format!("<p>原因：{}</p>", escape_attr(error))
  });
  Mail {
    to: job.email.clone(),
    subject: format!("Mica 导入{status}"),
    html_body: format!(
      "<p>你的后台导入任务{status}。</p><p>成功页面：{}；失败或未完成页面：{}；跳过文件：{}；图片失败：{}。</p>{detail}<p><a href=\"{}\">打开 Mica</a>，在设置 → 数据中查看导入记录</p><p>可在设置 → 数据中关闭导入完成邮件。</p>",
      job.done,
      failure,
      job.skipped_total,
      job.image_failures_total,
      escape_attr(link),
    ),
  }
}

#[cfg(test)]
mod tests;
