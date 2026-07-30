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

use super::auth::{normalize_email, sha256_hex};
use super::password_reset::{escape_attr, html_response, page_shell};

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
      let link = format!("{}/verify-email?token={}", state.config.app_base_url, token);
      let mail = Mail {
        to: email.to_string(),
        subject: "Confirm your Mica email address".to_string(),
        html_body: verify_email_html(&link),
      };
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

/// One token, returning the plaintext (it exists nowhere else). Earlier unused
/// tokens for this user are deleted first, so asking for a new link invalidates
/// the old one — there is never more than one live link per account.
async fn mint_token(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<String> {
  sqlx::query("DELETE FROM email_verification_tokens WHERE user_id = $1")
    .bind(user_id)
    .execute(db)
    .await?;

  // 244 bits from two v4 UUIDs — the same recipe as a refresh token.
  let secret = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
  let token = format!("{VERIFY_PREFIX}{secret}");
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

  // Spend the token atomically, only if still unused AND unexpired, returning
  // whose it is. No row back → not spendable, and we say so without saying why.
  let spent: Result<Option<Uuid>, sqlx::Error> = sqlx::query_scalar(
    r#"
      UPDATE email_verification_tokens
      SET used_at = now()
      WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
      RETURNING user_id
    "#,
  )
  .bind(sha256_hex(token))
  .fetch_optional(&state.db)
  .await;

  let user_id = match spent {
    Ok(Some(id)) => id,
    Ok(None) => return invalid_page(),
    Err(error) => {
      tracing::error!(%error, "email verification: token spend query failed");
      return error_page();
    }
  };

  // `coalesce` keeps the FIRST confirmation time, so a second link — or a
  // pre-fetching mail client followed by a real click — cannot rewrite history.
  if let Err(error) = sqlx::query(
    "UPDATE users SET email_verified_at = coalesce(email_verified_at, now()) WHERE id = $1",
  )
  .bind(user_id)
  .execute(&state.db)
  .await
  {
    tracing::error!(%error, "email verification: marking the user verified failed");
    return error_page();
  }

  done_page(&state.config.app_base_url)
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
/// limiting is the existing per-IP bucket's job — the path is under `/auth/`.
pub async fn resend(
  State(state): State<AppState>,
  axum::Json(payload): axum::Json<ResendRequest>,
) -> ApiResult<StatusCode> {
  let email = normalize_email(&payload.email)?;

  // Only an existing, still-unverified account gets a link. Re-sending to an
  // already-verified address would be a free mail cannon aimed at a real inbox.
  let target: Option<Uuid> =
    sqlx::query_scalar("SELECT id FROM users WHERE email = $1 AND email_verified_at IS NULL")
      .bind(&email)
      .fetch_optional(&state.db)
      .await?;

  if let Some(user_id) = target {
    mint_and_send(&state, user_id, &email).await;
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
