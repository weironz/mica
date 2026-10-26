//! Email verification — an account proves it controls its address before it can
//! be signed in to.
//!
//! Why it exists: registration with no verification means "unlimited accounts"
//! costs nothing but a well-formed string, and an unbounded account count is the
//! multiplier that makes every per-workspace limit meaningless. The per-workspace
//! byte quota bounds what one workspace can do; this bounds how many there can be.
//!
//! Deliberately shaped like [`super::password_reset`], down to reusing its page
//! shell: both are "prove you can read this inbox" flows, both are server-rendered
//! with no JavaScript, and both mount their page OUTSIDE `/api` so the auth
//! scope-guard never sees them — the token in the URL is the only credential. A
//! second house style for the same kind of page would be one more thing to keep in
//! sync.
//!
//! Two things it deliberately does NOT do:
//!   * block anything other than sign-in. Gating every write on a verified flag
//!     would put this check on dozens of paths; refusing the sign-in refuses all
//!     of them at once, in one place.
//!   * leave the link usable after it works. Single-use and expiring, like a reset
//!     link: one that lingers in an inbox is a standing key to an unclaimed
//!     account.

use axum::{
  extract::{Query, State},
  http::StatusCode,
  response::Response,
};
use chrono::{DateTime, Duration, Utc};
use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult, Mail};
use serde::Deserialize;
use uuid::Uuid;

use super::auth::{mint_secret, normalize_email, sha256_hex};
use super::password_reset::{escape_attr, html_response, page_shell};

#[cfg(test)]
#[path = "email_verify_atomicity_tests.rs"]
mod atomicity_tests;

/// Prefix so a leaked string is recognisable in a log or a bug report as a
/// verification link rather than an unlabelled secret.
const VERIFY_PREFIX: &str = "mica_ev_";

/// 24h, not the reset flow's 1h. A reset answers something the user is doing right
/// now; a verification mail can sit in an inbox until the evening. Still bounded —
/// an address that never confirms should not leave a live link behind forever.
const VERIFY_TTL_HOURS: i64 = 24;

/// Mint a verification token for `user_id` and email the link.
///
/// Best-effort, and the caller must treat it that way: the account already exists
/// when this runs, so a mail outage cannot be allowed to fail the registration and
/// leave someone holding an account they were told was never created. They can ask
/// for another link.
///
/// Returns whether the mail went out, purely so the caller can log it. Do NOT
/// surface that to the client — on the resend path it would be an
/// address-enumeration oracle.
pub async fn mint_and_send(state: &AppState, user_id: Uuid, email: &str) -> bool {
  match mint_token(&state.db, user_id).await {
    Ok(token) => {
      let mail = verification_mail(state, email, &token);
      match state.mailer.send(&mail).await {
        Ok(()) => true,
        Err(error) => {
          tracing::warn!(%error, "failed to send verification email");
          false
        }
      }
    }
    Err(error) => {
      tracing::warn!(%error, "failed to mint verification token");
      false
    }
  }
}

/// [`mint_and_send`]'s twin for a no-oracle endpoint (#39): the response must
/// not wait on SMTP, because "registered address answers slower" was the
/// exact timing channel awaiting closed over — a uniform 204 means nothing if
/// it arrives at different times. Minting stays in the request (a DB query
/// has the same shape either way); only the send moves off-task. Used by
/// [`resend`]; registration keeps [`mint_and_send`], whose caller wants to
/// know whether the mail went out.
pub async fn mint_and_spawn(state: &AppState, user_id: Uuid, email: &str) {
  match mint_token(&state.db, user_id).await {
    Ok(token) => {
      let mail = verification_mail(state, email, &token);
      let mailer = state.mailer.clone();
      tokio::spawn(async move {
        if let Err(error) = mailer.send(&mail).await {
          tracing::warn!(%error, "failed to send verification email");
        }
      });
    }
    Err(error) => {
      tracing::warn!(%error, "failed to mint verification token");
    }
  }
}

fn verification_mail(state: &AppState, email: &str, token: &str) -> Mail {
  let link = format!("{}/verify-email?token={}", state.config.app_base_url, token);
  Mail {
    to: email.to_string(),
    subject: "Confirm your Mica email address".to_string(),
    html_body: verify_email_html(&link),
  }
}

/// One token, returning the plaintext (it exists nowhere else). Earlier unused
/// tokens for this user are deleted first, so asking for a new link invalidates
/// the old one — there is never more than one live link per account.
async fn mint_token(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<String> {
  sqlx::query("DELETE FROM email_verification_tokens WHERE user_id = $1")
    .bind(user_id)
    .execute(db)
    .await?;

  let token = mint_secret(VERIFY_PREFIX);
  let expires_at = Utc::now() + Duration::hours(VERIFY_TTL_HOURS);

  sqlx::query(
    "INSERT INTO email_verification_tokens (token_hash, user_id, expires_at)
     VALUES ($1, $2, $3)",
  )
  .bind(sha256_hex(&token))
  .bind(user_id)
  .bind(expires_at)
  .execute(db)
  .await?;

  Ok(token)
}

#[derive(Debug, Deserialize)]
pub struct VerifyQuery {
  token: Option<String>,
}

/// `GET /verify-email?token=…` — spend the link and mark the address confirmed.
///
/// Acts on GET, unlike the reset flow, because there is nothing to ask: clicking
/// the link IS the confirmation. The cost is that a mail client which pre-fetches
/// links spends the token — acceptable, because spending it achieves exactly what
/// the user was about to do. A reset link cannot work this way: pre-fetching one
/// would hand a stranger the password form.
pub async fn verify_page(
  State(state): State<AppState>,
  Query(query): Query<VerifyQuery>,
) -> Response {
  let Some(token) = query.token.as_deref().filter(|t| !t.is_empty()) else {
    return invalid_page();
  };

  match consume_and_verify(&state.db, token).await {
    Ok(true) => done_page(&state.config.app_base_url),
    Ok(false) => invalid_page(),
    Err(error) => {
      tracing::error!(%error, "email verification failed");
      error_page()
    }
  }
}

async fn consume_and_verify(db: &sqlx::PgPool, token: &str) -> ApiResult<bool> {
  let mut tx = db.begin().await?;
  // A failed user update rolls token consumption back, so the link can be retried.
  let user_id: Option<Uuid> = sqlx::query_scalar(
    r#"
      UPDATE email_verification_tokens
      SET used_at = now()
      WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
      RETURNING user_id
    "#,
  )
  .bind(sha256_hex(token))
  .fetch_optional(&mut *tx)
  .await?;
  let Some(user_id) = user_id else {
    return Ok(false);
  };
  // Preserve the first confirmation time and refuse tombstoned accounts.
  let updated = sqlx::query(
    "UPDATE users SET email_verified_at = coalesce(email_verified_at, now()) WHERE id = $1 AND deleted_at IS NULL",
  )
  .bind(user_id)
  .execute(&mut *tx)
  .await?;
  if updated.rows_affected() == 0 {
    return Ok(false);
  }
  tx.commit().await?;
  Ok(true)
}

#[derive(Debug, Deserialize)]
pub struct ResendRequest {
  email: String,
}

/// `POST /api/auth/resend-verification` — send another link.
///
/// Always 204, whatever happened: unknown address, already verified, mail
/// failure. Anything else is an enumeration oracle, and this endpoint has to be
/// unauthenticated (the whole point is that the caller cannot sign in yet). Rate
/// limiting is the per-IP bucket's job: `rate_limit::ip_throttled` is attached
/// where the route is mounted (`routes::public_api_router`).
pub async fn resend(
  State(state): State<AppState>,
  axum::Json(payload): axum::Json<ResendRequest>,
) -> ApiResult<StatusCode> {
  let email = normalize_email(&payload.email)?;

  // Only an existing, still-unverified account gets a link. Re-sending to an
  // already-verified address would be a free mail cannon aimed at a real inbox.
  let target: Option<Uuid> = sqlx::query_scalar(
    "SELECT id FROM users WHERE email = $1 AND email_verified_at IS NULL AND deleted_at IS NULL",
  )
  .bind(&email)
  .fetch_optional(&state.db)
  .await?;

  if let Some(user_id) = target {
    // Off-task send (#39): awaiting the mail made a registered, unverified
    // address answer measurably later than an unknown one — the timing oracle
    // the uniform 204 exists to deny.
    mint_and_spawn(&state, user_id, &email).await;
  }

  Ok(StatusCode::NO_CONTENT)
}

/// Refuse a sign-in whose address was never confirmed.
///
/// A distinct machine-readable code rather than a bare 403: the client must be
/// able to tell "confirm your email" (offer the resend) apart from "wrong
/// password" (offer nothing). Not an enumeration leak — reaching this requires the
/// correct password, so the caller already knows the account exists.
pub fn ensure_verified(email_verified_at: Option<DateTime<Utc>>) -> ApiResult<()> {
  if email_verified_at.is_none() {
    return Err(ApiError::BadRequestCode(
      "email_not_verified",
      "confirm your email address before signing in".to_string(),
    ));
  }
  Ok(())
}

// ---- HTML (server-rendered, no JavaScript; shell shared with the reset page) --

fn verify_email_html(link: &str) -> String {
  format!(
    r#"<p>Confirm this address to finish creating your Mica account:</p>
<p><a href="{link}">Confirm my email address</a></p>
<p>The link works once and expires in {VERIFY_TTL_HOURS} hours.</p>
<p>If you did not create a Mica account, ignore this email — nothing happens
until the link is used.</p>"#,
    link = escape_attr(link),
  )
}

fn done_page(app_base_url: &str) -> Response {
  html_response(
    StatusCode::OK,
    page_shell(
      "Email confirmed",
      &format!(
        r#"<h1>Email confirmed</h1>
<p>You can sign in now.</p>
<p><a href="{}">Open Mica</a></p>"#,
        escape_attr(app_base_url)
      ),
    ),
  )
}

/// One page for "expired", "already used" and "never existed". Telling them apart
/// would answer questions a stranger holding a guessed token should not get
/// answered, and the next step is identical in all three cases.
fn invalid_page() -> Response {
  html_response(
    StatusCode::BAD_REQUEST,
    page_shell(
      "Link no longer valid",
      r#"<h1>This link no longer works</h1>
<p>Confirmation links can be used once and expire after a day. Try signing in —
the app will offer to send a new one.</p>"#,
    ),
  )
}

fn error_page() -> Response {
  html_response(
    StatusCode::INTERNAL_SERVER_ERROR,
    page_shell(
      "Something went wrong",
      r#"<h1>Something went wrong</h1>
<p>Please try the link again in a moment.</p>"#,
    ),
  )
}

/// The page route, mounted OUTSIDE `/api` (like the reset page and the public
/// share page) so the auth scope-guard never sees it.
pub fn router() -> axum::Router<AppState> {
  use axum::routing::get;
  axum::Router::new().route("/verify-email", get(verify_page))
}

/// #39: `resend` answers in the same time however the address resolves —
/// awaiting the send made a registered, unverified address measurably slower.
/// The mailer's send parks behind a gate, so the test needs no wall-clock
/// guessing. Gated on `DATABASE_URL` like the other pg tests (skipped green
/// locally, a hard error in CI).
#[cfg(test)]
mod resend_timing_pg {
  use super::*;
  use mica_infra::Mailer;
  use std::sync::{Arc, Mutex};

  async fn pool() -> Option<sqlx::PgPool> {
    let Some(url) = mica_infra::test_database_url() else {
      return None;
    };
    Some(
      sqlx::PgPool::connect(&url)
        .await
        .expect("DATABASE_URL is set but the connection failed"),
    )
  }

  /// A mailer whose send blocks until released, recording what it was asked
  /// to send — the deterministic stand-in for "SMTP is slow".
  #[derive(Default)]
  struct GateMailer {
    release: tokio::sync::Notify,
    sent: Mutex<Vec<Mail>>,
  }

  #[async_trait::async_trait]
  impl Mailer for GateMailer {
    async fn send(&self, mail: &Mail) -> anyhow::Result<()> {
      self.release.notified().await;
      self.sent.lock().unwrap().push(mail.clone());
      Ok(())
    }
  }

  fn timing_config() -> mica_infra::AppConfig {
    mica_infra::AppConfig {
      environment: mica_infra::Environment::Test,
      http_addr: "127.0.0.1:0".parse().unwrap(),
      database_url: String::new(),
      database_max_connections: 1,
      jwt_secret: "resend-timing-test-secret".to_string(),
      ws_min_protocol: 1,
      access_token_ttl_seconds: 60,
      refresh_token_ttl_seconds: 60,
      cors_allowed_origins: Vec::new(),
      seed_test_user: None,
      registration_enabled: false,
      workspace_quota_bytes: 0,
      owner_quota_bytes: 0,
      search_index_mlock: false,
      sync_tuning: Default::default(),
      app_base_url: "http://localhost:8080".to_string(),
    }
  }

  #[tokio::test]
  async fn resend_answers_204_before_the_mail_goes_out() {
    let Some(db) = pool().await else { return };
    let user = Uuid::new_v4();
    let email = format!("{user}@verify.test");
    sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','h')")
      .bind(user)
      .bind(&email)
      .execute(&db)
      .await
      .unwrap();
    let mailer = Arc::new(GateMailer::default());
    let state = AppState::new(timing_config(), db.clone(), mailer.clone());

    let response = tokio::time::timeout(
      std::time::Duration::from_secs(5),
      resend(
        axum::extract::State(state),
        axum::Json(ResendRequest {
          email: email.clone(),
        }),
      ),
    )
    .await
    .expect("resend must not wait on the mail — that wait was the #39 oracle")
    .expect("resend answers Ok");
    assert_eq!(response, StatusCode::NO_CONTENT);

    // The mail still goes out — off-task — once the gate opens.
    mailer.release.notify_one();
    for _ in 0..100 {
      if !mailer.sent.lock().unwrap().is_empty() {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    {
      let sent = mailer.sent.lock().unwrap();
      assert_eq!(sent.len(), 1, "the verification mail must still be sent");
      assert_eq!(sent[0].to, email);
    }

    sqlx::query("DELETE FROM users WHERE id=$1")
      .bind(user)
      .execute(&db)
      .await
      .ok();
  }
}
