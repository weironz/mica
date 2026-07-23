use std::collections::HashMap;
use std::sync::Arc;

use mica_infra::{AiConfig, AppConfig, Mailer, S3Config};
use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::RwLock;
use uuid::Uuid;

pub mod documents;
pub mod rooms;
pub mod store;
pub mod sync;

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
  /// Server-side workspace import jobs (the client uploads a ZIP once and
  /// polls progress here).
  pub import_jobs: Arc<RwLock<HashMap<Uuid, ImportJob>>>,
  /// Outbound email (currently just the password-reset link). Defaults to a
  /// no-op logger; the api-server binary swaps in Aliyun DirectMail from env.
  pub mailer: Arc<dyn Mailer>,
}

/// Progress of one server-side workspace import.
#[derive(Debug, Clone, Serialize)]
pub struct ImportJob {
  pub status: ImportJobStatus,
  pub total: usize,
  pub done: usize,
  pub workspace_id: Option<Uuid>,
  pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportJobStatus {
  Running,
  Done,
  Error,
}

impl AppState {
  pub fn new(config: AppConfig, db: PgPool, mailer: Arc<dyn Mailer>) -> Self {
    Self {
      config: Arc::new(config),
      db,
      hub: DocumentHub::new(),
      storage: S3Config::from_env().map(Arc::new),
      ai: Arc::new(RwLock::new(AiConfig::from_env())),
      import_jobs: Arc::new(RwLock::new(HashMap::new())),
      mailer,
    }
  }
}
