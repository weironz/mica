use std::collections::HashMap;
use std::sync::Arc;

use mica_infra::{AiConfig, AppConfig, Environment, Mailer, S3Config};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::RwLock;
use uuid::Uuid;

pub mod comments;
pub mod documents;
pub mod rooms;
pub mod search;
pub mod store;
pub mod sync;

pub use rooms::{DocumentHub, PresenceEntry, Room, RoomMessage};

#[derive(Clone)]
pub struct AppState {
  pub config: Arc<AppConfig>,
  pub db: PgPool,
  pub hub: DocumentHub,
  /// Rooms for `/ws/workspaces/{id}/views` — one per WORKSPACE, keyed by its
  /// id, carrying "the view tree changed" pings from the Postgres NOTIFY
  /// listener (migration 0025) to every client showing that sidebar. The same
  /// `DocumentHub` machinery as [`Self::hub`], deliberately a separate
  /// instance: one map keyed by two kinds of id would work only until it
  /// didn't, and nothing would say which kind a given entry was.
  pub views_hub: DocumentHub,
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
  /// In-process body-text index behind search (see `search::BodyIndex`) —
  /// lazily refreshed at query time, warmed once at startup.
  pub body_index: Arc<search::BodyIndex>,
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

  /// External images `rehost_external` promised to bring in and did not — they
  /// are still links pointing at somebody else's server.
  ///
  /// This exists because the failure was SILENT: a re-host that failed logged a
  /// warning server-side, left the block on its original url, and the import
  /// reported plain success. The user then did the reasonable thing — deleted
  /// the AppFlowy workspace they had just exported — and the links died with
  /// it. Nothing had told them the images were still borrowed.
  ///
  /// That is not a rare network hiccup. A server-side fetch fails SYSTEMATICALLY
  /// for the hosts people import from: AppFlowy blocks datacenter IPs, and a
  /// CN-hosted server has no route to medium/imgur. So for a whole class of
  /// archives, "re-host external images" quietly did nothing at all.
  ///
  /// Recorded per OCCURRENCE (page + block), not per url: the same link on
  /// twenty pages is twenty blocks still pointing outward, and a retry has to
  /// fix each one. `document_id` + `block_id` are exactly what the
  /// `rehost-image` endpoint takes, so the list is directly actionable.
  #[serde(default)]
  pub image_failures: Vec<ImageFailure>,

  /// How many failed in total. [`image_failures`] is capped like [`skipped`],
  /// so a pathological archive still reports an honest count.
  #[serde(default)]
  pub image_failures_total: usize,

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

/// One image the import was asked to bring in and left as a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageFailure {
  /// The link the block still points at. Also the thing to fetch on a retry.
  pub url: String,
  /// The page it sits on — a list of urls with no page names is a list nobody
  /// can act on.
  pub page: String,
  /// Where to patch it. `POST /workspaces/{ws}/documents/{document_id}
  /// /rehost-image?block_id=…` takes this pair, which is why they are stored
  /// rather than re-derived by walking every document later.
  pub document_id: Uuid,
  pub block_id: String,
  /// Why it did not land, in the words the fetch itself used.
  pub reason: String,
  /// False when the circuit breaker refused it WITHOUT a request, because its
  /// host had already timed out repeatedly.
  ///
  /// Called out separately because it is the most invisible kind: nothing
  /// failed, it simply never happened, and a per-request error log has no line
  /// to write. Before this, those were a single aggregate `refused` count in a
  /// server log the user never sees.
  pub attempted: bool,
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
  /// The api restarted while this import was running, so the task that owned it
  /// is gone.
  ///
  /// Distinct from both neighbours on purpose. `Running` would show a progress
  /// bar for something with nothing behind it — the poll would never advance
  /// again. `Error` would be wrong in the way that matters: nothing failed, and
  /// the pages already written are real and staying. What the user needs to
  /// know is exactly this: it stopped partway, and the workspace holds part of
  /// the archive.
  Interrupted,
}

impl ImportJobStatus {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Running => "running",
      Self::Done => "done",
      Self::Error => "error",
      Self::Cancelled => "cancelled",
      Self::Interrupted => "interrupted",
    }
  }

  pub fn parse(value: &str) -> Option<Self> {
    match value {
      "running" => Some(Self::Running),
      "done" => Some(Self::Done),
      "error" => Some(Self::Error),
      "cancelled" => Some(Self::Cancelled),
      "interrupted" => Some(Self::Interrupted),
      _ => None,
    }
  }
}

impl AppState {
  /// Panics on an EMPTY `jwt_secret`.
  ///
  /// Empty is the "not configured yet" marker `AppConfig::from_env` leaves when
  /// `JWT_SECRET` is absent; `mica_infra::ensure_jwt_secret` is what turns it
  /// into a real key, and it runs in `main` after migrations. Getting here with
  /// an empty one means that step was skipped — and an empty signing key signs
  /// anything, so failing loudly at construction beats serving forgeable tokens.
  pub fn new(config: AppConfig, db: PgPool, mailer: Arc<dyn Mailer>) -> Self {
    assert!(
      !config.jwt_secret.trim().is_empty(),
      "AppState built with an empty jwt_secret — ensure_jwt_secret() was not called"
    );
    // The object store's default credentials are published (see
    // `DEFAULT_S3_SECRET_KEY`) and :9000 is internet-facing, so an operator who
    // never touched them is running a bucket anyone can write. That trade is
    // deliberate — it buys a quickstart with nothing to fill in — but it must
    // not be silent on a production node, where documentation would otherwise be
    // the only place it was ever mentioned.
    if config.environment == Environment::Production && mica_infra::default_s3_secret_in_use() {
      tracing::warn!(
        "S3_SECRET_KEY is the published default — anyone can read and write this \
         instance's files over :9000. Set S3_ACCESS_KEY and S3_SECRET_KEY in the \
         env file this stack was started with (openssl rand -hex 32) and restart."
      );
    }

    Self {
      config: Arc::new(config),
      db,
      hub: DocumentHub::new(),
      views_hub: DocumentHub::new(),
      storage: S3Config::from_env().map(Arc::new),
      ai: Arc::new(RwLock::new(AiConfig::from_env())),
      import_jobs: Arc::new(RwLock::new(HashMap::new())),
      mailer,
      body_index: Arc::new(search::BodyIndex::default()),
    }
  }
}

/// Durable side of the import jobs. The in-memory map stays the hot path — the
/// import loop touches it once per page — and this mirrors it into `import_jobs`
/// so the record outlives the process (migration 0023).
pub mod import_store {
  use super::{ImportJob, ImportJobStatus};
  use sqlx::PgPool;
  use uuid::Uuid;

  /// Rows an import history shows at once. Generous, but bounded: a history
  /// nobody trimmed is a query that gets slower forever.
  pub const HISTORY_LIMIT: i64 = 50;

  /// Insert the job as it starts. Failing to record it must NOT stop the
  /// import: losing the history entry is a smaller harm than refusing to
  /// import, which is the whole reason this returns nothing.
  pub async fn create(db: &PgPool, id: Uuid, user_id: Uuid, job: &ImportJob) {
    let result = sqlx::query(
      "INSERT INTO import_jobs (id, user_id, workspace_id, status, total, done) \
       VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(user_id)
    .bind(job.workspace_id)
    .bind(job.status.as_str())
    .bind(job.total as i32)
    .bind(job.done as i32)
    .execute(db)
    .await;
    if let Err(error) = result {
      tracing::warn!(%error, %id, "could not record the import job; it will run unrecorded");
    }
  }

  /// Mirror the current progress. Called at a coarser cadence than the loop
  /// updates memory — see the caller — because one UPDATE per page turns a
  /// 600-page archive into 600 write round-trips for a number nobody is reading
  /// between polls.
  pub async fn update(db: &PgPool, id: Uuid, job: &ImportJob) {
    let skipped = serde_json::to_value(&job.skipped).unwrap_or_else(|_| serde_json::json!([]));
    // Same fallback as `skipped`: a serialisation failure must degrade to an
    // empty list, never stop the progress write. Losing the list is bad; losing
    // the row that says an import is running is worse.
    let image_failures =
      serde_json::to_value(&job.image_failures).unwrap_or_else(|_| serde_json::json!([]));
    let result = sqlx::query(
      "UPDATE import_jobs SET status = $2, total = $3, done = $4, error = $5, \
         skipped = $6, skipped_total = $7, workspace_id = COALESCE($8, workspace_id), \
         image_failures = $9, image_failures_total = $10, \
         updated_at = now() \
       WHERE id = $1",
    )
    .bind(id)
    .bind(job.status.as_str())
    .bind(job.total as i32)
    .bind(job.done as i32)
    .bind(job.error.as_deref())
    .bind(skipped)
    .bind(job.skipped_total as i32)
    .bind(job.workspace_id)
    .bind(image_failures)
    .bind(job.image_failures_total as i32)
    .execute(db)
    .await;
    if let Err(error) = result {
      tracing::warn!(%error, %id, "could not update the stored import job");
    }
  }

  /// One stored job, for a poll that outlived the process that started it.
  pub async fn get(db: &PgPool, id: Uuid, user_id: Uuid) -> Option<ImportJob> {
    let row = sqlx::query_as::<
      _,
      (
        String,
        i32,
        i32,
        Option<Uuid>,
        Option<String>,
        serde_json::Value,
        i32,
        serde_json::Value,
        i32,
      ),
    >(
      "SELECT status, total, done, workspace_id, error, skipped, skipped_total, \
              image_failures, image_failures_total \
       FROM import_jobs WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()?;
    Some(ImportJob {
      status: ImportJobStatus::parse(&row.0)?,
      total: row.1.max(0) as usize,
      done: row.2.max(0) as usize,
      workspace_id: row.3,
      error: row.4,
      skipped: serde_json::from_value(row.5).unwrap_or_default(),
      skipped_total: row.6.max(0) as usize,
      image_failures: serde_json::from_value(row.7).unwrap_or_default(),
      image_failures_total: row.8.max(0) as usize,
      cancel_requested: false,
    })
  }

  /// This user's imports, newest first, for the history list.
  pub async fn history(db: &PgPool, user_id: Uuid) -> Vec<(Uuid, ImportJob, String)> {
    let rows = sqlx::query_as::<
      _,
      (
        Uuid,
        String,
        i32,
        i32,
        Option<Uuid>,
        Option<String>,
        serde_json::Value,
        i32,
        serde_json::Value,
        i32,
        chrono::DateTime<chrono::Utc>,
      ),
    >(
      "SELECT id, status, total, done, workspace_id, error, skipped, skipped_total, \
              image_failures, image_failures_total, created_at \
       FROM import_jobs WHERE user_id = $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(user_id)
    .bind(HISTORY_LIMIT)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    rows
      .into_iter()
      .filter_map(|r| {
        Some((
          r.0,
          ImportJob {
            status: ImportJobStatus::parse(&r.1)?,
            total: r.2.max(0) as usize,
            done: r.3.max(0) as usize,
            workspace_id: r.4,
            error: r.5,
            skipped: serde_json::from_value(r.6).unwrap_or_default(),
            skipped_total: r.7.max(0) as usize,
            image_failures: serde_json::from_value(r.8).unwrap_or_default(),
            image_failures_total: r.9.max(0) as usize,
            cancel_requested: false,
          },
          r.10.to_rfc3339(),
        ))
      })
      .collect()
  }

  /// At boot: every job still marked `running` belongs to a process that no
  /// longer exists.
  ///
  /// Without this they stay `running` forever and the UI polls a progress bar
  /// that can never advance — the exact "nothing says what happened" this table
  /// was added to end. Returns how many were corrected, so the log can say it
  /// out loud: a deploy that interrupts an import is worth one line.
  pub async fn mark_interrupted(db: &PgPool) -> u64 {
    sqlx::query("UPDATE import_jobs SET status = 'interrupted', updated_at = now() WHERE status = 'running'")
      .execute(db)
      .await
      .map(|r| r.rows_affected())
      .unwrap_or(0)
  }
}
