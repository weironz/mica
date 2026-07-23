use anyhow::Context;
use axum::Router;
use mica_app_core::AppState;
use mica_infra::{AppConfig, Environment, connect_pg_pool, run_migrations, telemetry::init_tracing};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

mod blob_gc;
mod mail;
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

  // FTS M1: one-time backfill of `document_yrs_base.content_text` for rows that
  // predate migration 0012 (the yrs decode is Rust, not SQL). Idempotent — only
  // touches still-empty rows — so it is cheap on every subsequent boot. Best
  // effort: a decode failure warn-logs and skips that document (search misses
  // its body until it's next edited) but never blocks startup.
  match mica_app_core::sync::backfill_content_text(&db).await {
    Ok(filled) if filled > 0 => info!("content_text backfill: indexed {filled} document(s)"),
    Ok(_) => {}
    Err(error) => tracing::warn!(%error, "content_text backfill failed; search may miss some bodies until next edit"),
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
    blob_gc::spawn(state.db.clone(), storage);
  }
  let app = app_router(state);

  let listener = TcpListener::bind(addr)
    .await
    .with_context(|| format!("failed to bind HTTP listener on {addr}"))?;

  info!("HTTP server listening on {addr}");

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
    .merge(routes::reset_router())
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
