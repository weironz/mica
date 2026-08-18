use anyhow::Context;
use axum::Router;
use mica_app_core::AppState;
use mica_infra::{
  ApiError, AppConfig, Environment, connect_pg_pool, run_migrations, telemetry::init_tracing,
};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

mod blob_gc;
mod bucket;
mod mail;
mod metrics;
mod password_strength;
mod rate_limit;
mod routes;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  init_tracing();

  let config = AppConfig::from_env().context("failed to load configuration")?;
  let db = connect_pg_pool(&config)
    .await
    .context("failed to connect to PostgreSQL")?;
  run_migrations(&db)
    .await
    .context("failed to run database migrations")?;

  // The signing key, resolved AFTER migrations because an unconfigured instance
  // mints its own into a table they create. `JWT_SECRET` still wins when set —
  // and is still held to the strict bar in `AppConfig::from_env`.
  let mut config = config;
  config.jwt_secret = mica_infra::ensure_jwt_secret(&db, &config.jwt_secret)
    .await
    .context("failed to resolve the JWT signing key")?;

  // One-time backfill of the derived columns on `document_yrs_base` —
  // `content_text` (search, migration 0012) and `link_targets` (backlinks, 0019)
  // — for rows that predate them, since deriving either needs the yrs decode,
  // which is Rust, not SQL. Idempotent: only touches rows still carrying a
  // sentinel, so it is cheap on every subsequent boot. Best effort: a decode
  // failure warn-logs and skips that document (it misses search / backlinks
  // until it's next edited) but never blocks startup.
  match mica_app_core::sync::backfill_derived_columns(&db).await {
    Ok(filled) if filled > 0 => info!("derived-column backfill: indexed {filled} document(s)"),
    Ok(_) => {}
    Err(error) => tracing::warn!(%error, "derived-column backfill failed; search/backlinks may miss some documents until next edit"),
  }

  // Test-environment convenience: keep the seeded test account's credentials
  // valid across restarts and DB resets. AppConfig never populates this in
  // production.
  if let Some(seed) = &config.seed_test_user {
    routes::auth::seed_test_user(&db, &seed.email, &seed.password)
      .await
      .context("failed to seed the test user")?;
    tracing::warn!(email = %seed.email, "seeded test user (MICA_SEED_TEST_USER) — test environments only");
  }

  // Who administers this instance. Instance-wide settings (the AI provider and
  // its key) are admin-only, so an instance with no admin cannot be configured
  // at all — and migration 0021 could only GUESS at the answer when it added
  // the flag to an instance that already had accounts.
  //
  // That guess was "the oldest account stood the instance up", and it was wrong
  // the first time it ran for real: the oldest account was `demo@mica.dev`, a
  // dev seed, while the operator had signed up three days later. Every other
  // heuristic fails somewhere too (both accounts owned workspaces, so "owns
  // workspaces" would not have separated them either), and being wrong locks
  // the operator out or hands the flag to someone else on a shared instance.
  //
  // So: no more guessing. New instances are unambiguous — the first signup gets
  // it inside the same INSERT — and an existing instance gets an explicit lever
  // here. Idempotent, additive (it never revokes), and safe to leave set.
  if let Ok(email) = std::env::var("MICA_ADMIN_EMAIL") {
    let email = email.trim().to_lowercase();
    if !email.is_empty() {
      match sqlx::query("UPDATE users SET is_admin = true WHERE lower(email) = $1")
        .bind(&email)
        .execute(&db)
        .await
      {
        Ok(result) if result.rows_affected() > 0 => {
          info!(%email, "granted admin (MICA_ADMIN_EMAIL)")
        }
        Ok(_) => tracing::warn!(%email, "MICA_ADMIN_EMAIL names no account on this instance"),
        Err(error) => tracing::warn!(%error, "could not apply MICA_ADMIN_EMAIL"),
      }
    }
  }

  // An instance with accounts but no admin has no way to reach its own AI
  // settings. Say so at boot rather than letting it surface as a greyed-out
  // dialog with no explanation — which is exactly how it surfaced the first
  // time, and the screen gives the reader no way to know the fix is a database
  // row.
  if let Ok(Some((users, admins))) =
    sqlx::query_as::<_, (i64, i64)>("SELECT count(*), count(*) FILTER (WHERE is_admin) FROM users")
      .fetch_optional(&db)
      .await
  {
    if users > 0 && admins == 0 {
      tracing::warn!(
        users,
        "no account on this instance is an admin, so nobody can change the AI          provider settings. Set MICA_ADMIN_EMAIL to the operator's address and          restart."
      );
    }
  }

  let addr = config.http_addr;
  // Log by default; Aliyun DirectMail when MICA_MAIL_BACKEND=directmail is set.
  let mailer = mail::build_mailer();
  let state = AppState::new(config, db, mailer);
  // The environment seeded `state.ai` above; a row saved by an admin replaces
  // it. Settings used to live ONLY in that lock, so every restart — i.e. every
  // deploy — quietly un-configured AI and the settings dialog could not say
  // whether a key was set. See `AiConfig::load` for why the stored row wins.
  if let Some(stored) = mica_infra::AiConfig::load(&state.db).await {
    info!(provider = %stored.provider.as_str(), model = %stored.model, "loaded saved AI settings");
    *state.ai.write().await = Some(stored);
  }
  // Reclaim blobs no page points at any more. Backgrounded and best-effort: a
  // GC that stops is a disk-space problem, never a reason to fail a request.
  // No-op when object storage is not configured — nothing to reclaim.
  if let Some(storage) = state.storage.clone() {
    // Before anything can be uploaded there has to be a bucket, and nothing
    // else creates one. Awaited rather than backgrounded: it is one HEAD on a
    // store the api is about to depend on, and a bucket that appears after the
    // first upload attempt is not much better than no bucket. It never fails
    // startup — see `bucket::ensure_bucket`.
    bucket::ensure_bucket(&storage).await;

    // A blob is served by a 302 to the STORAGE origin, which is what keeps an
    // uploaded SVG's scripts away from the app's tokens: they run wherever the
    // bytes live, and that is a different origin. Configuring
    // `S3_PUBLIC_BASE_URL` to the app's own origin quietly removes that
    // boundary and turns "we allow SVG uploads" into stored XSS against the
    // session. Nothing else would notice — the upload succeeds, the image
    // renders, and only the origin changed.
    //
    // A warning rather than a refusal: the app cannot know that a shared origin
    // is wrong (a reverse proxy could be routing /blobs elsewhere entirely),
    // and refusing to start over a config the operator may have meant is worse
    // than saying so. Same shape as the plaintext-HTTP warning below.
    if let Some(public) = storage.public_base_url.as_deref() {
      let app = state.config.app_base_url.trim_end_matches('/');
      if !app.is_empty() && public.trim_end_matches('/').starts_with(app) {
        tracing::warn!(
          "S3_PUBLIC_BASE_URL ({public}) shares an origin with MICA_APP_BASE_URL            ({app}). Blob reads redirect there, so an uploaded SVG would execute            SAME-ORIGIN with the app and could read its session. Serve blobs from            a separate host (or leave S3_PUBLIC_BASE_URL unset to use presigned            GETs, which already are)."
        );
      }
    }
    blob_gc::spawn(state.db.clone(), storage);
  }
  // Samples how long it takes to get a pooled connection. The idle/in_use
  // gauges show saturation; this shows the WAIT, which is what 2026-08-03
  // actually looked like (acquire 3.3s) and what a 15s-sampled gauge can miss
  // entirely between two scrapes.
  metrics::spawn_pool_probe(state.db.clone());
  // The yrs-base backfill that used to run here is gone with S5: it built bases
  // out of `document_snapshots`, and migration 0016 dropped that table. It had
  // already finished its job — 1428 bases built on 0.13.3, leaving 3735 bases for
  // 3735 documents — which is exactly the precondition that made the drop safe.
  let app = app_router(state);

  let listener = TcpListener::bind(addr)
    .await
    .with_context(|| format!("failed to bind HTTP listener on {addr}"))?;

  info!("HTTP server listening on {addr}");
  // Bound to something other than loopback means this port faces a network, and
  // this server only ever speaks plaintext HTTP — TLS is the reverse proxy's
  // job. Say so once at boot: the default (127.0.0.1) is safe and silent, so
  // the warning only fires for the deployment that actually took the risk,
  // which is the one where nobody wrote the proxy down anywhere.
  if !addr.ip().is_loopback() {
    tracing::warn!(
      "listening on {addr}, which is not loopback — this server speaks \
       plaintext HTTP only. Terminate TLS in front of it; without a proxy the \
       auth tokens (including the one in the WebSocket URL) cross the network \
       in the clear."
    );
  }

  // `into_make_service_with_connect_info` so the rate-limit middleware can read
  // the socket peer (ConnectInfo) as the fallback when there's no usable XFF.
  axum::serve(
    listener,
    app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
  )
  .with_graceful_shutdown(shutdown_signal())
  .await
  .context("HTTP server failed")?;

  Ok(())
}

fn app_router(state: AppState) -> Router {
  // Authenticate + enforce token scopes on every /api route (public ones opt out
  // inside the guard). WebSocket routes keep their own query-token auth.
  let api = routes::api_router()
    .layer(axum::middleware::from_fn_with_state(
      state.clone(),
      routes::auth::scope_guard,
    ))
    // An unmatched /api path answered with axum's default 404: status only,
    // EMPTY body. Every other error here carries `{code, message}`, so the one
    // response you get while guessing at the API was the one that told you
    // nothing — and with no published spec, guessing is how people find it.
    // Registered AFTER the scope guard on purpose: a path that does not exist
    // is a 404 for everyone, and answering 401 first sends people hunting for a
    // credential problem they do not have.
    .fallback(|| async { ApiError::NotFound });
  Router::new()
    .nest("/api", api)
    .merge(routes::ws_router())
    .merge(routes::share_router())
    .merge(routes::legal_router())
    .merge(routes::reset_router())
    // NOT under `/api`, and that placement IS the access control: nginx only
    // proxies /api, /ws, /s and the two mail links, so this is reachable from
    // the compose network and not from the internet. See metrics.rs.
    .route("/metrics", axum::routing::get(metrics::metrics_handler))
    // Outermost of our own layers, so a request rejected by the rate limiter or
    // by CORS is still counted — those are exactly the requests you want to see
    // during an incident, and a middleware that only observes the happy path
    // reports calm while the front door is on fire.
    .layer(axum::middleware::from_fn(metrics::track))
    .layer(TraceLayer::new_for_http())
    // Throttle the auth endpoints per client IP + cap Argon2 concurrency. Inner
    // to CORS (so a preflight is answered before the limiter sees it); the
    // Extension carries the shared guard the middleware extracts.
    .layer(axum::middleware::from_fn(rate_limit::auth_rate_limit))
    .layer(axum::Extension(rate_limit::AuthGuard::from_env()))
    .layer(cors_layer(&state.config))
    .with_state(state)
}



/// CORS policy. The bundled web app is served same-origin with `/api`, so it
/// never triggers CORS; this only governs third-party browser reads. In
/// production an empty allowlist denies all cross-origin (was
/// `CorsLayer::permissive()`, which let any site read the API); in development
/// the web app runs on a different localhost port than the API, so an empty
/// allowlist stays permissive there for convenience. Set `CORS_ALLOWED_ORIGINS`
/// (comma-separated) to grant specific origins in production.
fn cors_layer(config: &AppConfig) -> CorsLayer {
  use axum::http::{HeaderValue, Method, header};
  if !config.cors_allowed_origins.is_empty() {
    let origins: Vec<HeaderValue> = config
      .cors_allowed_origins
      .iter()
      .filter_map(|origin| origin.parse::<HeaderValue>().ok())
      .collect();
    return CorsLayer::new()
      .allow_origin(origins)
      .allow_methods([
        Method::GET,
        Method::POST,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
      ])
      .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
  }
  match config.environment {
    Environment::Production => CorsLayer::new(),
    _ => CorsLayer::permissive(),
  }
}

async fn shutdown_signal() {
  let ctrl_c = async {
    tokio::signal::ctrl_c()
      .await
      .expect("failed to install Ctrl+C handler");
  };

  #[cfg(unix)]
  let terminate = async {
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
      .expect("failed to install SIGTERM handler")
      .recv()
      .await;
  };

  #[cfg(not(unix))]
  let terminate = std::future::pending::<()>();

  tokio::select! {
    _ = ctrl_c => {},
    _ = terminate => {},
  }
}

#[cfg(test)]
mod not_found_body_tests {
  use super::*;
  use axum::response::IntoResponse;

  /// The payload `app_router`'s `/api` fallback answers with. Pinned because an
  /// unmatched path used to come back as a bare status with NO body, which is
  /// precisely the response someone gets while guessing at an undocumented API
  /// — the one place a machine-readable reason is worth most. Covers the shape,
  /// not the wiring: that the fallback sits AFTER the scope guard (so a missing
  /// path reads as 404, never 401) is asserted by the comment there and by the
  /// deploy smoke check, since building the real router needs an AppState.
  #[tokio::test]
  async fn unmatched_api_path_answers_with_a_json_reason() {
    let response = ApiError::NotFound.into_response();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);

    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
      .await
      .expect("error body should be readable");
    let body: serde_json::Value =
      serde_json::from_slice(&bytes).expect("error body should be JSON, not empty");

    assert_eq!(body["code"], "not_found");
    assert!(
      body["message"].as_str().is_some_and(|m| !m.is_empty()),
      "a reason with no message is the empty body again, wearing JSON: {body}"
    );
  }
}
