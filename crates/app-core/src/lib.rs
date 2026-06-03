use std::sync::Arc;

use mica_infra::{AiConfig, AppConfig, S3Config};
use sqlx::PgPool;
use tokio::sync::RwLock;

pub mod documents;
pub mod rooms;
pub mod store;

pub use rooms::{DocumentHub, PresenceEntry, Room, RoomMessage};

#[derive(Clone)]
pub struct AppState {
  pub config: Arc<AppConfig>,
  pub db: PgPool,
  pub hub: DocumentHub,
  /// Object-store config for file uploads; `None` disables the file endpoints.
  pub storage: Option<Arc<S3Config>>,
  /// AI provider config, mutable at runtime via the settings endpoint. `None`
  /// (until configured) makes the AI endpoints return 503.
  pub ai: Arc<RwLock<Option<AiConfig>>>,
}

impl AppState {
  pub fn new(config: AppConfig, db: PgPool) -> Self {
    Self {
      config: Arc::new(config),
      db,
      hub: DocumentHub::new(),
      storage: S3Config::from_env().map(Arc::new),
      ai: Arc::new(RwLock::new(AiConfig::from_env())),
    }
  }
}
