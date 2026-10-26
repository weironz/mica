//! Self-service password reset — the "forgot my password" path a public
//! deployment needs.
//!
//! Two halves:
//!   * `POST /api/auth/password/forgot {email}` — a client (desktop or web) asks
//!     for a reset. We ALWAYS answer 204, whether or not the address is
//!     registered, so the endpoint is not an account-enumeration oracle. If it
//!     is registered we mint a single-use token, store only its hash (0013), and
//!     email a link.
//!   * `GET/POST /reset-password` — a **server-rendered, no-JavaScript** page
//!     (same house style as the public share page: mounted outside `/api`,
//!     nginx-proxied, strict CSP). GET shows the new-password form; POST spends
//!     the token and sets the password. The clients never implement the reset
//!     itself — the emailed link opens in whatever browser the user has.
//!
//! The token is opaque and single-use with a short TTL, mirroring refresh tokens
//! (0006): the plaintext lives only in the emailed link, spending it is a
//! conditional UPDATE (so a used/expired link can't be replayed), and setting a
//! new password revokes every existing session — a reset is what you do when you
//! fear someone else is in the account.

use axum::{
  extract::{Form, Query, State},
  http::{StatusCode, header},
  response::{Html, IntoResponse, Response},
};
use chrono::{Duration, Utc};
use mica_app_core::AppState;
use mica_infra::{ApiResult, Mail};
use serde::Deserialize;
use uuid::Uuid;

use super::auth::{
  hash_password, mint_secret, normalize_email, revoke_user_sessions_in_tx, sha256_hex,
};

/// Reset links carry this prefix so a stray token is recognizable in a log the
/// way `mica_rt_` / `mica_pat_` are. 244 bits of randomness follow.
const RESET_PREFIX: &str = "mica_pr_";
/// A reset link is a standing key to the account, so it dies fast.
const RESET_TTL_HOURS: i64 = 1;

/// CSP for the reset pages: no scripts at all (`default-src 'none'`), inline CSS
/// only, and the form may submit to same-origin. Neutralizes any markup that
/// might reflect through the token query param, belt-and-braces with the HTML
/// escaping below.
const RESET_CSP: &str =
  "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'";

#[derive(Debug, Deserialize)]
pub struct ForgotRequest {
  email: String,
}

/// `POST /api/auth/password/forgot` — request a reset email. Always 204.
pub async fn forgot(
  State(state): State<AppState>,
  axum::Json(payload): axum::Json<ForgotRequest>,
) -> ApiResult<StatusCode> {
  let email = normalize_email(&payload.email)?;

  // Only registered addresses get an email — but the response is identical
  // either way (see the module doc: no enumeration oracle).
  if let Some(user_id) = user_id_for_email(&state.db, &email).await? {
    match mint_reset_token(&state.db, user_id).await {
      Ok(token) => {
        let link = format!(
          "{}/reset-password?token={}",
          state.config.app_base_url, token
        );
        let mail = Mail {
          to: email,
          subject: "Reset your Mica password".to_string(),
          html_body: reset_email_html(&link),
        };
        // SPAWN, don't await (#39): awaiting the send answered a registered
        // address measurably later than an unknown one — SMTP latency is tens
        // to hundreds of milliseconds, a timing oracle that re-opens the
        // enumeration the uniform 204 exists to close. Best-effort unchanged:
        // a mail failure only logs, never changes the response.
        let mailer = state.mailer.clone();
        tokio::spawn(async move {
          if let Err(error) = mailer.send(&mail).await {
            tracing::warn!(%error, "failed to send password-reset email");
          }
        });
      }
      Err(error) => {
        // Same reasoning: swallow so the timing/response can't be read as a
        // signal. The user simply sees "check your email" and can retry.
        tracing::warn!(%error, "failed to mint password-reset token");
      }
    }
  }

  Ok(StatusCode::NO_CONTENT)
}

async fn user_id_for_email(db: &sqlx::PgPool, email: &str) -> ApiResult<Option<Uuid>> {
  Ok(
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE email = $1 AND deleted_at IS NULL")
      .bind(email)
      .fetch_optional(db)
      .await?,
  )
}

/// Mint one reset token, returning the plaintext (it exists nowhere else). Any
/// EARLIER unused token for this user is deleted first, so requesting a new link
/// invalidates the old one — a user who requests twice, or an attacker who
/// triggered a spurious request, leaves exactly one live link.
async fn mint_reset_token(db: &sqlx::PgPool, user_id: Uuid) -> ApiResult<String> {
  sqlx::query("DELETE FROM password_reset_tokens WHERE user_id = $1")
    .bind(user_id)
    .execute(db)
    .await?;

  let token = mint_secret(RESET_PREFIX);
  let expires_at = Utc::now() + Duration::hours(RESET_TTL_HOURS);

  sqlx::query(
    "INSERT INTO password_reset_tokens (token_hash, user_id, expires_at) VALUES ($1, $2, $3)",
  )
  .bind(sha256_hex(&token))
  .bind(user_id)
  .bind(expires_at)
  .execute(db)
  .await?;

  Ok(token)
}

#[derive(Debug, Deserialize)]
pub struct ResetQuery {
  token: Option<String>,
}

/// `GET /reset-password?token=…` — show the new-password form. The token is not
/// validated here (validating on GET would be a probe oracle and a link is often
/// pre-fetched by mail clients); the POST is the single authority.
pub async fn reset_page(Query(query): Query<ResetQuery>) -> Response {
  let token = query.token.unwrap_or_default();
  reset_form_page(&token, None)
}

#[derive(Debug, Deserialize)]
pub struct ResetForm {
  token: String,
  password: String,
}

/// `POST /reset-password` — spend the token and set the new password.
pub async fn reset_submit(State(state): State<AppState>, Form(form): Form<ResetForm>) -> Response {
  // The SAME policy as register / change-password. This used to be its own
  // `len() < 8` — bytes, not characters, and none of the rest of the policy —
  // so `password`, `12345678` or three CJK characters were accepted here and
  // only here. Checked BEFORE the token is spent, so a rejected password does
  // not burn the link.
  match reset_identifiers(&state.db, &form.token).await {
    Ok(Some((email, name))) => {
      if let Err(weak) = crate::password_strength::check(&form.password, &[&email, &name]) {
        return reset_form_page(&form.token, Some(weak.message()));
      }
    }
    Ok(None) => return reset_invalid_page(),
    Err(ResetError::Invalid) => return reset_invalid_page(),
    Err(ResetError::Internal) => return reset_error_page(),
  }

  match consume_and_reset(&state.db, &form.token, &form.password).await {
    Ok(()) => reset_done_page(&state.config.app_base_url),
    Err(ResetError::Invalid) => reset_invalid_page(),
    Err(ResetError::Internal) => reset_error_page(),
  }
}

#[derive(Debug)]
enum ResetError {
  /// Unknown, expired, or already-used token — all the same to the user.
  Invalid,
  Internal,
}

/// The email and display name of a still-spendable token's user, WITHOUT
/// spending it — the strength check needs them as identifiers.
async fn reset_identifiers(
  db: &sqlx::PgPool,
  token: &str,
) -> Result<Option<(String, String)>, ResetError> {
  sqlx::query_as(
    r#"
      SELECT u.email, u.display_name
      FROM password_reset_tokens t JOIN users u ON u.id = t.user_id
      WHERE t.token_hash = $1 AND t.used_at IS NULL AND t.expires_at > now()
        AND u.deleted_at IS NULL
    "#,
  )
  .bind(sha256_hex(token))
  .fetch_optional(db)
  .await
  .map_err(|error| {
    tracing::error!(%error, "password reset: token lookup failed");
    ResetError::Internal
  })
}

/// Spend the token (conditional UPDATE: the decision IS the UPDATE, so an
/// expired/used link cannot slip through a check-then-act) and set the password.
/// Every existing session is revoked — a reset is done precisely when the old
/// password may be compromised.
async fn consume_and_reset(
  db: &sqlx::PgPool,
  token: &str,
  new_password: &str,
) -> Result<(), ResetError> {
  let token_hash = sha256_hex(token);
  let password_hash = hash_password(new_password).await.map_err(|error| {
    tracing::error!(%error, "password reset: hashing failed");
    ResetError::Internal
  })?;
  let mut tx = db.begin().await.map_err(|error| {
    tracing::error!(%error, "password reset: beginning transaction failed");
    ResetError::Internal
  })?;

  // Resolve the token's owner FIRST and lock that user row before spending the
  // token. The same row lock is the serialization boundary shared with PAT mint
  // and login refresh issuance (#123): a rotation committed while an old
  // password login was blocked on refresh INSERT waits here, then the revoke
  // below catches what that login inserted, or the login re-reads the bumped
  // epoch/hash and is refused. No user row back → the link is not spendable.
  let user_id: Option<Uuid> = sqlx::query_scalar(
    "SELECT user_id FROM password_reset_tokens WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()",
  )
  .bind(&token_hash)
  .fetch_optional(&mut *tx)
  .await
  .map_err(|error| {
    tracing::error!(%error, "password reset: token lookup failed");
    ResetError::Internal
  })?;
  let Some(user_id) = user_id else {
    return Err(ResetError::Invalid);
  };
  let live_user: Option<Uuid> =
    sqlx::query_scalar("SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR UPDATE")
      .bind(user_id)
      .fetch_optional(&mut *tx)
      .await
      .map_err(|error| {
        tracing::error!(%error, "password reset: locking the user row failed");
        ResetError::Internal
      })?;
  if live_user.is_none() {
    return Err(ResetError::Invalid);
  }

  // Atomically mark it used only if it is still unused AND unexpired — the
  // decision IS the UPDATE, so a used/expired link cannot slip through a
  // check-then-act.
  let spent: Option<Uuid> = sqlx::query_scalar(
    r#"
      UPDATE password_reset_tokens
      SET used_at = now()
      WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
      RETURNING user_id
    "#,
  )
  .bind(&token_hash)
  .fetch_optional(&mut *tx)
  .await
  .map_err(|error| {
    tracing::error!(%error, "password reset: token spend query failed");
    ResetError::Internal
  })?;

  if spent.is_none() {
    return Err(ResetError::Invalid);
  }

  let updated = sqlx::query(
    "UPDATE users SET password_hash = $1, updated_at = now(), credential_epoch = extract(epoch from now())::bigint WHERE id = $2 AND deleted_at IS NULL",
  )
  .bind(password_hash)
  .bind(user_id)
  .execute(&mut *tx)
  .await
  .map_err(|error| {
    tracing::error!(%error, "password reset: setting the new password failed");
    ResetError::Internal
  })?;

  if updated.rows_affected() == 0 {
    return Err(ResetError::Invalid);
  }

  // Kill every sign-in in the same transaction as spending the reset link.
  revoke_user_sessions_in_tx(&mut tx, user_id)
    .await
    .map_err(|error| {
      tracing::error!(%error, "password reset: revoking sessions failed");
      ResetError::Internal
    })?;
  tx.commit().await.map_err(|error| {
    tracing::error!(%error, "password reset: committing failed");
    ResetError::Internal
  })?;

  Ok(())
}

/// The two page routes, mounted OUTSIDE `/api` (like the share page) so the auth
/// scope-guard never sees them — the token in the form is the only credential.
/// The POST is a token-guessing surface AND runs an Argon2 hash before spending
/// the token, so it carries BOTH the per-IP throttle and the global Argon2
/// concurrency gate (#124) — the same gate login/register/change-password/delete
/// use, held across the whole request so a flood cannot pin every core. The GET
/// riding along is harmless — it renders a static page — so it keeps only the
/// per-IP throttle.
pub fn router() -> axum::Router<AppState> {
  use axum::routing::{get, post};
  axum::Router::new()
    .route(
      "/reset-password",
      crate::rate_limit::ip_throttled(get(reset_page)),
    )
    .route(
      "/reset-password",
      crate::rate_limit::argon2_throttled(post(reset_submit)),
    )
}

// ---- HTML (server-rendered, no JavaScript) -------------------------------

pub(super) fn html_response(status: StatusCode, body: String) -> Response {
  (
    status,
    [
      (header::CONTENT_TYPE, "text/html; charset=utf-8"),
      (header::CONTENT_SECURITY_POLICY, RESET_CSP),
    ],
    Html(body),
  )
    .into_response()
}

pub(super) fn page_shell(title: &str, inner: &str) -> String {
  format!(
    "<!doctype html><html lang=en><head><meta charset=utf-8>\
     <meta name=viewport content=\"width=device-width, initial-scale=1\">\
     <title>{title}</title><style>{css}</style></head>\
     <body><main class=card>{inner}</main></body></html>",
    title = escape_html(title),
    css = PAGE_CSS,
  )
}

const PAGE_CSS: &str = "\
:root{color-scheme:light dark}\
body{font-family:system-ui,-apple-system,Segoe UI,Roboto,sans-serif;margin:0;\
min-height:100vh;display:flex;align-items:center;justify-content:center;\
background:#f1f5f9;color:#0f172a}\
@media(prefers-color-scheme:dark){body{background:#0f172a;color:#e2e8f0}\
.card{background:#1e293b!important}input{background:#0f172a;color:#e2e8f0;\
border-color:#334155!important}}\
.card{background:#fff;max-width:26rem;width:calc(100% - 2rem);padding:2rem;\
border-radius:14px;box-shadow:0 10px 30px rgba(0,0,0,.08)}\
h1{font-size:1.25rem;margin:0 0 1rem}p{color:#64748b;line-height:1.5}\
label{display:block;font-size:.85rem;margin:1rem 0 .35rem;font-weight:600}\
input{width:100%;box-sizing:border-box;padding:.6rem .7rem;font-size:1rem;\
border:1px solid #cbd5e1;border-radius:8px}\
button{margin-top:1.25rem;width:100%;padding:.65rem;font-size:1rem;\
font-weight:600;color:#fff;background:#2563eb;border:0;border-radius:8px;\
cursor:pointer}button:hover{background:#1d4ed8}\
.err{color:#dc2626;font-size:.9rem;margin-top:.75rem}\
.ok{color:#16a34a}";

fn reset_form_page(token: &str, error: Option<&str>) -> Response {
  let err_html = error
    .map(|e| format!("<p class=err>{}</p>", escape_html(e)))
    .unwrap_or_default();
  let inner = format!(
    "<h1>Choose a new password</h1>\
     <p>Enter a new password for your Mica account.</p>\
     <form method=post action=\"/reset-password\">\
       <input type=hidden name=token value=\"{token}\">\
       <label for=pw>New password</label>\
       <input id=pw type=password name=password minlength=8 required autofocus \
         autocomplete=new-password placeholder=\"at least 8 characters\">\
       <button type=submit>Reset password</button>\
       {err_html}\
     </form>",
    token = escape_attr(token),
  );
  html_response(StatusCode::OK, page_shell("Reset your password", &inner))
}

fn reset_done_page(app_base_url: &str) -> Response {
  let inner = format!(
    "<h1 class=ok>Password reset</h1>\
     <p>Your password has been changed and every device has been signed out. \
     Open Mica and sign in with your new password.</p>\
     <p><a href=\"{}\">Go to Mica</a></p>",
    escape_attr(app_base_url),
  );
  html_response(StatusCode::OK, page_shell("Password reset", &inner))
}

fn reset_invalid_page() -> Response {
  let inner = "<h1>This link has expired</h1>\
     <p>Password reset links can only be used once and expire after an hour. \
     Request a new one from the app's sign-in screen.</p>";
  html_response(StatusCode::BAD_REQUEST, page_shell("Link expired", inner))
}

fn reset_error_page() -> Response {
  let inner = "<h1>Something went wrong</h1>\
     <p>We couldn't reset your password just now. Please try again in a moment.</p>";
  html_response(
    StatusCode::INTERNAL_SERVER_ERROR,
    page_shell("Error", inner),
  )
}

fn reset_email_html(link: &str) -> String {
  // Kept simple and inline: one line, one button-styled link, an expiry note,
  // and the "ignore this" line that a legitimate but unrequested reset needs.
  format!(
    "<div style=\"font-family:system-ui,sans-serif;font-size:15px;color:#0f172a\">\
     <p>Someone (hopefully you) asked to reset the password for your Mica account.</p>\
     <p><a href=\"{link}\" style=\"display:inline-block;padding:10px 18px;\
     background:#2563eb;color:#fff;text-decoration:none;border-radius:8px\">\
     Reset your password</a></p>\
     <p style=\"color:#64748b\">This link expires in one hour and can be used once. \
     If you didn't request it, you can safely ignore this email — your password \
     stays the same.</p>\
     <p style=\"color:#94a3b8;font-size:13px\">If the button doesn't work, copy this \
     link:<br>{link_text}</p></div>",
    link = escape_attr(link),
    link_text = escape_html(link),
  )
}

/// Escape for HTML text content.
pub(super) fn escape_html(input: &str) -> String {
  input
    .replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
    .replace('\'', "&#39;")
}

/// Escape for a double-quoted attribute value (`href`, hidden input). Same set;
/// named apart so the intent at each call site is explicit.
pub(super) fn escape_attr(input: &str) -> String {
  escape_html(input)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn token_is_reflected_escaped_into_the_form() {
    // A crafted token must not break out of the hidden input's value attribute.
    let page = reset_form_page("\"><script>alert(1)</script>", None);
    let escaped = escape_attr("\"><script>alert(1)</script>");
    assert!(!escaped.contains('<'));
    assert!(!escaped.contains('"'));
    assert!(escaped.contains("&lt;script&gt;"));
    let _ = page; // constructed without panicking
  }

  #[test]
  fn escaping_covers_the_five_dangerous_chars() {
    assert_eq!(
      escape_html("a&b<c>d\"e'f"),
      "a&amp;b&lt;c&gt;d&quot;e&#39;f"
    );
  }

  #[test]
  fn reset_email_contains_the_link() {
    let html = reset_email_html("https://mica.example.com/reset-password?token=mica_pr_abc");
    assert!(html.contains("reset-password?token=mica_pr_abc"));
    assert!(html.contains("expires in one hour"));
  }
}

#[cfg(test)]
#[path = "password_reset_atomicity_tests.rs"]
mod atomicity_tests;

/// The reset flow's correctness IS its SQL — the single-use spend is a
/// conditional UPDATE, so these run against a real Postgres, gated on
/// `DATABASE_URL` (skipped green without one locally; a hard error in CI, where
/// one is always provisioned). Same contract as auth.rs's `refresh_pg`.
#[cfg(test)]
mod reset_pg {
  use super::*;
  use sqlx::PgPool;

  async fn pool() -> Option<PgPool> {
    let Some(url) = mica_infra::test_database_url() else {
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
    sqlx::query(
      "INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','old-hash')",
    )
    .bind(user)
    .bind(format!("{user}@reset.test"))
    .execute(db)
    .await
    .unwrap();
    user
  }

  async fn password_hash_of(db: &PgPool, user: Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT password_hash FROM users WHERE id=$1")
      .bind(user)
      .fetch_one(db)
      .await
      .unwrap()
  }

  /// A mailer whose send blocks until released, recording what it was asked
  /// to send — the deterministic stand-in for "SMTP is slow", so the #39
  /// timing test needs no sleep-based guessing about wall-clock durations.
  #[derive(Default)]
  struct GateMailer {
    release: tokio::sync::Notify,
    sent: std::sync::Mutex<Vec<Mail>>,
  }

  #[async_trait::async_trait]
  impl mica_infra::Mailer for GateMailer {
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
      jwt_secret: "reset-timing-test-secret".to_string(),
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

  /// #39: awaiting the send made a REGISTERED address answer measurably later
  /// than an unknown one — the timing oracle the uniform 204 exists to deny.
  /// With the send parked behind the gate, the handler must still answer; the
  /// mail then goes out once the gate opens.
  #[tokio::test]
  async fn forgot_answers_204_before_the_mail_goes_out() {
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;
    let email = format!("{user}@reset.test");
    let mailer = std::sync::Arc::new(GateMailer::default());
    let state = AppState::new(timing_config(), db.clone(), mailer.clone());

    let response = tokio::time::timeout(
      std::time::Duration::from_secs(5),
      forgot(
        axum::extract::State(state),
        axum::Json(ForgotRequest {
          email: email.clone(),
        }),
      ),
    )
    .await
    .expect("forgot must not wait on the mail — that wait was the #39 oracle")
    .expect("forgot answers Ok");
    assert_eq!(response, StatusCode::NO_CONTENT);

    // The mail still goes out — off-task — once the send is let through.
    mailer.release.notify_one();
    for _ in 0..100 {
      if !mailer.sent.lock().unwrap().is_empty() {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    {
      let sent = mailer.sent.lock().unwrap();
      assert_eq!(sent.len(), 1, "the reset mail must still be sent");
      assert_eq!(sent[0].to, email);
    }

    sqlx::query("DELETE FROM users WHERE id=$1")
      .bind(user)
      .execute(&db)
      .await
      .ok();
  }

  #[tokio::test]
  async fn a_reset_sets_the_password_and_is_single_use() {
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;

    let token = mint_reset_token(&db, user).await.unwrap();
    assert!(token.starts_with(RESET_PREFIX));

    // Spending it sets a real (argon2) hash — no longer the seeded placeholder.
    consume_and_reset(&db, &token, "a-new-password")
      .await
      .expect("a fresh, unexpired token must reset the password");
    let after = password_hash_of(&db, user).await;
    assert_ne!(after, "old-hash", "the password must actually change");
    assert!(after.starts_with("$argon2"), "and be a real argon2 hash");

    // Single-use: the same link cannot be spent twice.
    assert!(
      matches!(
        consume_and_reset(&db, &token, "another-one").await,
        Err(ResetError::Invalid)
      ),
      "a spent link must be refused"
    );

    sqlx::query("DELETE FROM users WHERE id=$1")
      .bind(user)
      .execute(&db)
      .await
      .ok();
  }

  #[tokio::test]
  async fn minting_again_invalidates_the_previous_link() {
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;

    let first = mint_reset_token(&db, user).await.unwrap();
    let second = mint_reset_token(&db, user).await.unwrap();
    assert_ne!(first, second);

    // The first link is gone (deleted on the second request), so it can't reset.
    assert!(matches!(
      consume_and_reset(&db, &first, "x-password").await,
      Err(ResetError::Invalid)
    ));
    // The latest one still works.
    consume_and_reset(&db, &second, "y-password")
      .await
      .expect("the most recent link must work");

    sqlx::query("DELETE FROM users WHERE id=$1")
      .bind(user)
      .execute(&db)
      .await
      .ok();
  }

  #[tokio::test]
  async fn an_expired_link_is_refused() {
    let Some(db) = pool().await else { return };
    let user = seed_user(&db).await;

    let token = mint_reset_token(&db, user).await.unwrap();
    // Backdate its expiry past now.
    sqlx::query(
      "UPDATE password_reset_tokens SET expires_at = now() - interval '1 minute' WHERE user_id=$1",
    )
    .bind(user)
    .execute(&db)
    .await
    .unwrap();

    assert!(
      matches!(
        consume_and_reset(&db, &token, "z-password").await,
        Err(ResetError::Invalid)
      ),
      "an expired link must be refused"
    );
    // And the password stayed as seeded.
    assert_eq!(password_hash_of(&db, user).await, "old-hash");

    sqlx::query("DELETE FROM users WHERE id=$1")
      .bind(user)
      .execute(&db)
      .await
      .ok();
  }
}
