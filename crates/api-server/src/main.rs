use anyhow::Context;
use axum::Router;
use mica_app_core::AppState;
use mica_infra::{AppConfig, Environment, connect_pg_pool, run_migrations, telemetry::init_tracing};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

mod blob_gc;
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

  let addr = config.http_addr;
  // Log by default; Aliyun DirectMail when MICA_MAIL_BACKEND=directmail is set.
  let mailer = mail::build_mailer();
  let state = AppState::new(config, db, mailer);
  // Reclaim blobs no page points at any more. Backgrounded and best-effort: a
  // GC that stops is a disk-space problem, never a reason to fail a request.
  // No-op when object storage is not configured — nothing to reclaim.
  if let Some(storage) = state.storage.clone() {
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
  let api = routes::api_router().layer(axum::middleware::from_fn_with_state(
    state.clone(),
    routes::auth::scope_guard,
  ));
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
