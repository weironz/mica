use std::sync::Arc;

use mica_infra::{AppConfig, S3Config};
use sqlx::PgPool;

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
}

impl AppState {
  pub fn new(config: AppConfig, db: PgPool) -> Self {
    Self {
      config: Arc::new(config),
      db,
      hub: DocumentHub::new(),
      storage: S3Config::from_env().map(Arc::new),
    }
  }
}
