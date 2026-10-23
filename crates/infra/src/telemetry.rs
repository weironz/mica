use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_tracing() {
  // Default info (#37). The old default put tower_http at DEBUG, whose request
  // spans record the full URI — and the WS/auth compatibility tail still
  // carries the bearer token in `?token=`, so an unconfigured node wrote live
  // tokens to its logs. RUST_LOG still overrides for anyone who wants more,
  // and the trace layer redacts the token before recording the URI even then.
  let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
    EnvFilter::new("mica_api_server=info,mica_app_core=info,mica_infra=info,tower_http=info")
  });

  tracing_subscriber::registry()
    .with(filter)
    .with(fmt::layer())
    .init();
}
