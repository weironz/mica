use anyhow::Context;
use axum::Router;
use mica_app_core::AppState;
use mica_infra::{AppConfig, connect_pg_pool, run_migrations, telemetry::init_tracing};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

mod routes;
mod zip;

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

  let addr = config.http_addr;
  let state = AppState::new(config, db);
  let app = app_router(state);

  let listener = TcpListener::bind(addr)
    .await
    .with_context(|| format!("failed to bind HTTP listener on {addr}"))?;

  info!("HTTP server listening on {addr}");

  axum::serve(listener, app)
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("HTTP server failed")?;

  Ok(())
}

fn app_router(state: AppState) -> Router {
  Router::new()
    .nest("/api", routes::api_router())
    .merge(routes::ws_router())
    .layer(TraceLayer::new_for_http())
    .layer(CorsLayer::permissive())
    .with_state(state)
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
