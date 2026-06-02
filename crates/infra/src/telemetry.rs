use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_tracing() {
  let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
    EnvFilter::new("mica_api_server=debug,mica_app_core=debug,mica_infra=debug,tower_http=debug")
  });

  tracing_subscriber::registry()
    .with(filter)
    .with(fmt::layer())
    .init();
}
