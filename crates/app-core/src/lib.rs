use std::collections::HashMap;
use std::sync::Arc;

use mica_infra::{AiConfig, AppConfig, Mailer, S3Config};
use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::RwLock;
use uuid::Uuid;

pub mod comments;
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

  /// Archive entries that came in but never made it into the workspace: files
  /// no imported page references.
  ///
  /// Non-Markdown entries are uploaded lazily — only when a page actually links
  /// to them — so an archive routinely carries assets nothing points at (an
  /// export's leftovers, a Notion `.csv` beside a database page, a stray
  /// screenshot). Those are dropped silently today; the count is what lets the
  /// user tell "imported everything" apart from "imported most of it".
  ///
  /// Paths, not counts, so the client can show *which* — and capped when the
  /// list is absurd (see the collection site) rather than shipping a
  /// ten-thousand-entry array through a polling endpoint.
  #[serde(default)]
  pub skipped: Vec<String>,

  /// How many were skipped in total. [`skipped`] is capped, so a pathological
  /// archive still reports an honest count even when the list is truncated —
  /// showing 50 when 4000 were dropped would be worse than showing none.
  #[serde(default)]
  pub skipped_total: usize,

  /// Someone asked this import to stop.
  ///
  /// The loop checks it at each PAGE boundary rather than being aborted outright:
  /// killing the task mid-page could leave a document created but empty, or a
  /// view pointing at nothing. One page later is a state the importer already
  /// knows how to be in.
  ///
  /// Cancelling does **not** roll back. Pages already written stay, and the job
  /// reports how many — undoing an import into an EXISTING workspace would mean
  /// deleting rows the user may already have opened and edited, and a partial
  /// import you can see and delete yourself is safer than one that deletes
  /// things for you. A cancelled import into a NEW workspace leaves that
  /// workspace, which the user can drop in one action.
  #[serde(default)]
  pub cancel_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportJobStatus {
  Running,
  Done,
  Error,
  /// Stopped on request, at a page boundary. Whatever had already been imported
  /// stays — see [`ImportJob::cancel_requested`].
  Cancelled,
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
