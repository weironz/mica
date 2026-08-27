//! Server-side workspace import: the client uploads an archive once and the
//! whole pipeline (unzip → plan → pages/assets/links) runs here in Rust —
//! no per-page HTTP round trips, no browser memory limits, survives the tab.
//!
//! The pure planning lives in the `mica-interchange` crate; this module only
//! executes the plan: create the workspace, insert pages (with pre-generated
//! view ids so links can target pages created later), upload referenced
//! assets, and rewire image refs and page links.

use axum::{
  Json,
  body::Bytes,
  extract::{Path, Query, State},
  http::HeaderMap,
};
use mica_app_core::{
  AppState, ImportJob, ImportJobStatus, documents::import_markdown, store::DocumentRecord,
};
use mica_infra::{ApiError, ApiResult};
use mica_interchange::{ImportMode, ImportPlan, plan_import, read_zip, resolve_ref};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::documents::ensure_workspace_editor;
use crate::routes::files::{fetch_and_store_image_url, store_bytes};

#[derive(Debug, Deserialize)]
pub struct ImportParams {
  /// Workspace name when creating a new one (default "Imported").
  pub name: Option<String>,
  /// Force Notion adaptation (otherwise auto-detected from the contents).
  #[serde(default)]
  pub notion: bool,
  /// Import into this existing workspace instead of creating a new one.
  /// Tolerates a bare/empty query value (treated as absent).
  #[serde(default, deserialize_with = "empty_as_none")]
  pub workspace_id: Option<Uuid>,
  /// Import UNDER this folder (a view in [workspace_id]) instead of at the
  /// workspace root — top-level nodes become its children. Requires
  /// [workspace_id]; must reference a folder view. Empty value = absent.
  #[serde(default, deserialize_with = "empty_as_none")]
  pub parent_view_id: Option<Uuid>,
  /// How the archive's top level maps onto an existing destination:
  /// `auto` (default) — peel a single wrapper folder and spill its contents in,
  /// but wrap MULTIPLE loose roots under one container so they don't scatter;
  /// `spill` — force top-level entries directly into the destination (never
  /// wrap); `wrap` — force everything under one container named after the
  /// source. Only meaningful with [workspace_id]; a new workspace IS the
  /// container, so this is ignored there.
  #[serde(default)]
  pub container: ContainerMode,
  /// Re-host external `http(s)` image links into Mica storage during import
  /// (best-effort: a host this server can't reach keeps its link). Mirrors the
  /// client's "转存网络图片" toggle — the client passes its setting through.
  /// Defaults on: import is a one-time "bring this content in" operation, so
  /// making external images permanent is the expected default.
  #[serde(default = "default_true")]
  pub rehost_external: bool,
}

/// Stops an import from re-learning that a host is unreachable, once per image.
///
/// A CN-hosted server routinely cannot reach the CDNs a wiki links to. Each of
/// those fetches costs the full connect timeout, in series, and a documentation
/// archive tends to reference the SAME host hundreds of times — so the import
/// spends hours establishing one fact over and over.
///
/// After [Self::LIMIT] timeouts against one host, the rest of that host's images
/// are refused immediately. Only TIMEOUTS trip it: a 404 is about one image and
/// says nothing about the next, and a host that is merely slow still gets
/// [Self::LIMIT] honest attempts before being written off. A refused image keeps
/// its original link, exactly as a failed fetch would — the breaker changes how
/// long the failure takes to establish, never the outcome.
/// External image fetches in flight at once during an import.
///
/// Modest on purpose, twice over. These are someone else's servers, and an
/// import that opens fifty connections to one CDN is a bad citizen. And each
/// fetch is not just a download: it also writes to object storage and to the
/// database, which on a single-box deployment are the SAME machine — so this
/// number multiplies local IO, not merely outbound requests.
///
/// Was 8, chosen against the wall-of-timeouts case with an unstated assumption
/// that the server had headroom. On a 3.5 GB box also hosting its own object
/// store, Postgres and unrelated services, an import drove load average past 35
/// and took the machine off the network — twice. The measurement that settled
/// what it was: that archive peaked around 135 MB, so this was contention, not
/// memory. Halved rather than made configurable, per the note in `api.md` — a
/// default shown to be wrong gets a better default, not a knob.
const REHOST_CONCURRENCY: usize = 4;

/// Ceiling on an uploaded archive.
///
/// `read_zip` decompresses every entry into memory and the raw upload is still
/// held alongside it, so peak is roughly upload + inflated. Nothing bounded
/// that: an archive bigger than the server's free memory was not refused, it
/// took the process — and on a small box, the whole machine — down with it. A
/// request that cannot be served must fail as a request, not as an outage.
///
/// 256 MiB of UPLOAD sits far above real exports (a 1192-entry wiki is ~65 MiB
/// compressed) and far below what the smallest sane deployment can hold.
///
/// Two honest limits on this check:
///
/// - It measures the upload, not the inflation, so a deliberately crafted
///   archive that is small compressed and enormous expanded still gets through.
///   Catching that needs a walk of the central directory's uncompressed sizes
///   before inflating anything — worth doing, not done here.
/// - It would NOT have prevented the outage that prompted it: that archive was
///   ~65 MiB, nowhere near this. It closes a separate hole found while looking.
const MAX_ARCHIVE_UPLOAD_BYTES: usize = 256 * 1024 * 1024;

/// How many entries a job's `skipped` / `image_failures` lists carry.
///
/// These ride a polling endpoint, and a pathological archive (a node_modules
/// dump; a wiki whose every image points at one dead host) would otherwise ship
/// thousands of entries on every poll. The paired `_total` stays exact, so a
/// truncated list never understates the damage — showing 50 when 4000 were lost
/// would be worse than showing none.
const MAX_LISTED: usize = 50;

/// The head of [`MAX_LISTED`], cloned. Failures are already in page order, which
/// is the order the user will work through them in.
fn capped(failures: &[mica_app_core::ImageFailure]) -> Vec<mica_app_core::ImageFailure> {
  failures.iter().take(MAX_LISTED).cloned().collect()
}

#[derive(Default)]
struct HostBreaker {
  timeouts: std::collections::HashMap<String, u32>,
}

impl HostBreaker {
  /// Timeouts against one host before the rest of it is refused unattempted.
  const LIMIT: u32 = 2;

  fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
      .ok()
      .and_then(|u| u.host_str().map(str::to_string))
  }

  /// True when this host has already proved unreachable.
  fn is_open(&self, url: &str) -> bool {
    Self::host_of(url)
      .and_then(|h| self.timeouts.get(&h).copied())
      .is_some_and(|n| n >= Self::LIMIT)
  }

  /// Record an outcome. Anything that is not a timeout resets the host: one
  /// slow image among many good ones must not condemn the rest.
  fn record(&mut self, url: &str, error: &str) {
    let Some(host) = Self::host_of(url) else {
      return;
    };
    if error.contains("timed out") {
      *self.timeouts.entry(host).or_insert(0) += 1;
    } else {
      self.timeouts.remove(&host);
    }
  }
}

fn default_true() -> bool {
  true
}

#[cfg(test)]
mod tests {
  use super::*;

  fn running_job() -> ImportJob {
    ImportJob {
      status: ImportJobStatus::Running,
      total: 10,
      done: 3,
      workspace_id: None,
      error: None,
      skipped: Vec::new(),
      skipped_total: 0,
      image_failures: Vec::new(),
      image_failures_total: 0,
      cancel_requested: false,
    }
  }

  /// A cancelled run returns `Ok` — it stopped cleanly — so the FLAG has to
  /// decide the final status. If the result alone decided, a stopped import
  /// would report "done" and claim the archive is fully in.
  #[test]
  fn a_cancelled_run_is_not_reported_as_done() {
    let mut job = running_job();
    job.cancel_requested = true;

    let result: ApiResult<()> = Ok(());
    match result {
      Ok(()) if job.cancel_requested => job.status = ImportJobStatus::Cancelled,
      Ok(()) => job.status = ImportJobStatus::Done,
      Err(_) => job.status = ImportJobStatus::Error,
    }

    assert_eq!(job.status, ImportJobStatus::Cancelled);
    assert_eq!(job.done, 3, "the pages that landed are still reported");
  }

  #[test]
  fn an_untouched_run_still_completes_normally() {
    let mut job = running_job();

    let result: ApiResult<()> = Ok(());
    match result {
      Ok(()) if job.cancel_requested => job.status = ImportJobStatus::Cancelled,
      Ok(()) => job.status = ImportJobStatus::Done,
      Err(_) => job.status = ImportJobStatus::Error,
    }

    assert_eq!(job.status, ImportJobStatus::Done);
  }

  /// Cancelling only means something while the job is running: a finished import
  /// cannot be un-done by pressing cancel afterwards, and saying otherwise would
  /// promise a rollback that does not exist.
  #[test]
  fn cancelling_a_finished_job_changes_nothing() {
    for finished in [ImportJobStatus::Done, ImportJobStatus::Error] {
      let mut job = running_job();
      job.status = finished;

      if matches!(job.status, ImportJobStatus::Running) {
        job.cancel_requested = true;
      }

      assert!(!job.cancel_requested, "{finished:?} must not accept a cancel");
      assert_eq!(job.status, finished, "status unchanged");
    }
  }

  /// Cancel is idempotent — the client may well send it twice while polling.
  #[test]
  fn cancelling_twice_is_the_same_as_once() {
    let mut job = running_job();
    for _ in 0..2 {
      if matches!(job.status, ImportJobStatus::Running) {
        job.cancel_requested = true;
      }
    }
    assert!(job.cancel_requested);
    assert_eq!(job.status, ImportJobStatus::Running, "cancel does not itself end the job");
  }

  /// The skipped set is "what the archive carried that no page pointed at" —
  /// `plan.files` minus whatever got uploaded. Assets upload lazily, so that
  /// difference is exactly the silent drop the user could not otherwise see.
  #[test]
  fn skipped_is_planned_files_minus_uploaded_sorted() {
    let file_paths: std::collections::HashSet<String> =
      ["a/img.png", "b/leftover.csv", "c/used.png", "z/last.png"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut uploaded: std::collections::HashMap<String, (String, String)> =
      std::collections::HashMap::new();
    uploaded.insert("c/used.png".into(), ("id".into(), "url".into()));

    let mut skipped: Vec<String> = file_paths
      .iter()
      .filter(|p| !uploaded.contains_key(*p))
      .cloned()
      .collect();
    skipped.sort();

    // Sorted, because this rides a polling endpoint: HashSet order would
    // reshuffle the list between two polls of the same finished job.
    assert_eq!(skipped, ["a/img.png", "b/leftover.csv", "z/last.png"]);
    assert!(
      !skipped.iter().any(|p| p == "c/used.png"),
      "a referenced asset was imported, not skipped"
    );
  }

  /// A pathological archive must not ship thousands of paths on every poll — but
  /// the COUNT has to stay honest, or "50 skipped" hides that 4000 were dropped.
  #[test]
  fn skipped_list_is_capped_while_the_total_stays_true() {
    const MAX_LISTED: usize = 50;
    let mut skipped: Vec<String> = (0..4000).map(|i| format!("junk/{i:04}.bin")).collect();
    skipped.sort();
    let total = skipped.len();
    skipped.truncate(MAX_LISTED);

    assert_eq!(skipped.len(), MAX_LISTED);
    assert_eq!(total, 4000, "the reported total is the real one");
    assert_eq!(skipped[0], "junk/0000.bin", "truncation keeps the head, sorted");
  }

  fn failure(url: &str) -> mica_app_core::ImageFailure {
    mica_app_core::ImageFailure {
      url: url.to_string(),
      page: "Some page".to_string(),
      document_id: Uuid::nil(),
      block_id: "block_1".to_string(),
      reason: "connection timed out".to_string(),
      attempted: true,
    }
  }

  /// Same bargain as `skipped`: cap what rides the polling endpoint, keep the
  /// count exact. A wiki whose every image points at one dead host produces one
  /// failure per occurrence, so this list is the one most likely to be enormous.
  #[test]
  fn the_failure_list_is_capped_while_the_count_stays_true() {
    let all: Vec<_> = (0..4000)
      .map(|i| failure(&format!("https://dead.example/{i}.png")))
      .collect();
    let listed = capped(&all);

    assert_eq!(listed.len(), MAX_LISTED);
    assert_eq!(all.len(), 4000, "the total the job reports is the real one");
    assert_eq!(
      listed[0].url, "https://dead.example/0.png",
      "the head is kept, so the list matches the order pages were imported in"
    );
  }

  /// Under the cap, nothing is dropped — the common case is a handful.
  #[test]
  fn a_short_failure_list_is_passed_through_whole() {
    let all = vec![failure("https://a.example/1.png"), failure("https://b/2.png")];
    assert_eq!(capped(&all), all);
  }

  /// The list is stored as jsonb and read back on the next poll, so a field that
  /// does not survive the round trip is a field the UI silently never sees.
  /// `attempted` in particular decides which of two different sentences the user
  /// is shown.
  #[test]
  fn a_failure_survives_the_json_round_trip() {
    let mut refused = failure("https://slow.example/x.png");
    refused.attempted = false;
    refused.reason = "host had already timed out repeatedly".to_string();

    let text = serde_json::to_string(&refused).unwrap();
    let back: mica_app_core::ImageFailure = serde_json::from_str(&text).unwrap();

    assert_eq!(back, refused);
    assert!(
      !back.attempted,
      "an unattempted refusal must not come back looking like a failed fetch"
    );
  }

  /// An archive whose assets were all used reports nothing — not an empty-ish
  /// "0 skipped" line the UI would then have to special-case.
  #[test]
  fn nothing_skipped_when_every_asset_was_referenced() {
    let file_paths: std::collections::HashSet<String> =
      ["only.png"].iter().map(|s| s.to_string()).collect();
    let mut uploaded: std::collections::HashMap<String, (String, String)> =
      std::collections::HashMap::new();
    uploaded.insert("only.png".into(), ("id".into(), "url".into()));

    let skipped: Vec<String> = file_paths
      .iter()
      .filter(|p| !uploaded.contains_key(*p))
      .cloned()
      .collect();

    assert!(skipped.is_empty());
  }

  const TIMED_OUT: &str = "bad request: could not fetch the image url: timed out — this \
                           server may have no route to that host";
  const NOT_FOUND: &str = "bad request: image url returned 404 Not Found";

  /// The case that cost hours: one unreachable CDN referenced by hundreds of
  /// images, each paying the full timeout in series.
  #[test]
  fn a_host_that_keeps_timing_out_is_written_off() {
    let mut b = HostBreaker::default();
    let url = "https://static.example.cn/a.png";
    assert!(!b.is_open(url), "nothing has failed yet");

    b.record(url, TIMED_OUT);
    assert!(!b.is_open(url), "one timeout could be a blip");
    b.record("https://static.example.cn/b.png", TIMED_OUT);

    assert!(
      b.is_open("https://static.example.cn/c.png"),
      "the rest of that host must be refused without being tried"
    );
  }

  /// A 404 is about ONE image. Letting it trip the breaker would abandon every
  /// other image on a host over a single dead link — and these arrive mixed
  /// together in real archives.
  #[test]
  fn a_404_says_nothing_about_the_next_image() {
    let mut b = HostBreaker::default();
    for path in ["a", "b", "c", "d"] {
      b.record(&format!("https://aijishu.example/{path}.png"), NOT_FOUND);
    }
    assert!(!b.is_open("https://aijishu.example/e.png"));
  }

  /// A slow host that eventually answers must not be condemned by the timeouts
  /// it accumulated on the way — otherwise one bad patch loses the rest.
  #[test]
  fn a_success_clears_the_earlier_timeouts() {
    let mut b = HostBreaker::default();
    b.record("https://slow.example/1.png", TIMED_OUT);
    b.record("https://slow.example/2.png", NOT_FOUND); // any non-timeout resets
    b.record("https://slow.example/3.png", TIMED_OUT);
    assert!(
      !b.is_open("https://slow.example/4.png"),
      "the counter restarted, so this host still has attempts left"
    );
  }

  /// Hosts are independent: one dead CDN must not silence a healthy one.
  #[test]
  fn one_dead_host_does_not_affect_another() {
    let mut b = HostBreaker::default();
    b.record("https://dead.example/1.png", TIMED_OUT);
    b.record("https://dead.example/2.png", TIMED_OUT);
    assert!(b.is_open("https://dead.example/3.png"));
    assert!(!b.is_open("https://alive.example/1.png"));
  }

  /// Anything unparseable is left to the fetch to reject — the breaker must not
  /// invent a host, and must never refuse something it cannot even name.
  #[test]
  fn a_url_with_no_host_is_never_refused() {
    let mut b = HostBreaker::default();
    b.record("not a url", TIMED_OUT);
    b.record("not a url", TIMED_OUT);
    assert!(!b.is_open("not a url"));
  }

  #[test]
  fn rehost_external_defaults_on_and_can_be_turned_off() {
    // Absent → on: import brings external images in unless the caller opts out.
    let d: ImportParams = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(d.rehost_external, "import re-hosts external images by default");
    // The client mirrors its "转存网络图片" toggle through this flag.
    let off: ImportParams =
      serde_json::from_value(serde_json::json!({ "rehost_external": false })).unwrap();
    assert!(!off.rehost_external, "the client's toggle can turn it off");
  }
}

/// The wrap-vs-spill choice surfaced to the user in the import dialog. Default
/// `auto` mirrors AppFlowy/Anytype; `spill`/`wrap` are explicit overrides.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerMode {
  #[default]
  Auto,
  Spill,
  Wrap,
}

fn empty_as_none<'de, D>(deserializer: D) -> Result<Option<Uuid>, D::Error>
where
  D: serde::Deserializer<'de>,
{
  let raw: Option<String> = serde::Deserialize::deserialize(deserializer)?;
  match raw.as_deref() {
    None | Some("") => Ok(None),
    Some(v) => Uuid::parse_str(v).map(Some).map_err(serde::de::Error::custom),
  }
}

#[derive(Debug, Serialize)]
pub struct ImportStartResponse {
  pub job_id: Uuid,
}

/// `POST /api/workspaces/import` — body is the raw archive bytes.
pub async fn start_import(
  State(state): State<AppState>,
  headers: HeaderMap,
  Query(params): Query<ImportParams>,
  body: Bytes,
) -> ApiResult<Json<ImportStartResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  if body.is_empty() {
    return Err(ApiError::BadRequest("empty archive".to_string()));
  }
  // Refuse before decompressing, not while running out of memory doing it.
  if body.len() > MAX_ARCHIVE_UPLOAD_BYTES {
    return Err(ApiError::BadRequest(format!(
      "archive is {} MiB; this server accepts up to {} MiB — split it and import the parts \
       (they can go into the same workspace with workspace_id)",
      body.len() / (1024 * 1024),
      MAX_ARCHIVE_UPLOAD_BYTES / (1024 * 1024)
    )));
  }
  if let Some(ws) = params.workspace_id {
    ensure_workspace_editor(&state.db, ws, user_id).await?;
  }
  // Importing under a folder: it must be a folder view in the (required)
  // target workspace — pages can't be parented under a document.
  if let Some(parent) = params.parent_view_id {
    let Some(ws) = params.workspace_id else {
      return Err(ApiError::BadRequest(
        "parent_view_id requires workspace_id".to_string(),
      ));
    };
    super::documents::ensure_parent_accepts_children(&state.db, ws, parent).await?;
  }

  let job_id = Uuid::new_v4();
  state.import_jobs.write().await.insert(
    job_id,
    ImportJob {
      status: ImportJobStatus::Running,
      total: 0,
      done: 0,
      workspace_id: params.workspace_id,
      error: None,
      skipped: Vec::new(),
      skipped_total: 0,
      image_failures: Vec::new(),
      image_failures_total: 0,
      cancel_requested: false,
    },
  );

  {
    let jobs = state.import_jobs.read().await;
    if let Some(job) = jobs.get(&job_id) {
      mica_app_core::import_store::create(&state.db, job_id, user_id, job).await;
    }
  }

  tokio::spawn(async move {
    let result = run_import(&state, user_id, job_id, params, body).await;
    let finished = {
      let mut jobs = state.import_jobs.write().await;
      let job = jobs.get_mut(&job_id);
      match job {
        Some(job) => {
          match result {
            // A cancelled run returns Ok — it stopped cleanly — so the flag, not
            // the result, decides the final status. Reporting "done" for an
            // import the user stopped halfway would claim their archive is fully
            // in.
            Ok(()) if job.cancel_requested => job.status = ImportJobStatus::Cancelled,
            Ok(()) => job.status = ImportJobStatus::Done,
            Err(e) => {
              job.status = ImportJobStatus::Error;
              job.error = Some(e.to_string());
            }
          }
          Some(job.clone())
        }
        None => None,
      }
    };
    // The row is written OUTSIDE the lock and after the terminal status is
    // decided: this is the write that matters, because it is the one a restart
    // cannot take back. Progress updates during the run are throttled and may
    // lag, but the final state is recorded exactly once, here.
    if let Some(job) = finished {
      mica_app_core::import_store::update(&state.db, job_id, &job).await;
    }
  });

  Ok(Json(ImportStartResponse { job_id }))
}


/// Mirror progress into `import_jobs`, but only every [`PERSIST_EVERY`] pages.
///
/// The in-memory map is the hot path — the loop writes it once per page, and
/// that is what the polling endpoint reads. The row exists so the record
/// survives a restart, and for THAT a stale `done` is fine: whatever the last
/// checkpoint said, plus a status of `interrupted`, already tells the user the
/// true thing ("it stopped partway, part of the archive is in"). One UPDATE per
/// page would turn a 600-page archive into 600 write round-trips for a number
/// nobody reads in between.
const PERSIST_EVERY: usize = 25;

async fn persist_progress(state: &AppState, job_id: Uuid, done: usize) {
  if done % PERSIST_EVERY != 0 {
    return;
  }
  let snapshot = state.import_jobs.read().await.get(&job_id).cloned();
  if let Some(job) = snapshot {
    mica_app_core::import_store::update(&state.db, job_id, &job).await;
  }
}

/// `POST /api/import/jobs/{job_id}/cancel` — ask a running import to stop.
///
/// Sets a flag the import loop reads at its next page boundary; it does not abort
/// the task. Returns the job so the caller sees the state it is in, including how
/// many pages already landed.
///
/// Unzipping and planning happen before the loop starts and are not interruptible
/// — a cancel during those seconds takes effect when the first page is reached.
/// Idempotent: cancelling a finished job changes nothing.
pub async fn cancel_import_job(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(job_id): Path<Uuid>,
) -> ApiResult<Json<ImportJob>> {
  user_id_from_headers(&state, &headers).await?;
  let mut jobs = state.import_jobs.write().await;
  let job = jobs.get_mut(&job_id).ok_or(ApiError::NotFound)?;
  if matches!(job.status, ImportJobStatus::Running) {
    job.cancel_requested = true;
  }
  Ok(Json(job.clone()))
}

/// `GET /api/import/jobs/{job_id}`
pub async fn import_job(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(job_id): Path<Uuid>,
) -> ApiResult<Json<ImportJob>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  if let Some(job) = state.import_jobs.read().await.get(&job_id).cloned() {
    return Ok(Json(job));
  }
  // Not in memory: either this api restarted since the import started, or the
  // job finished long enough ago to have been dropped. Both used to be a 404,
  // which is how a client that reopened after a deploy lost the ability to say
  // ANYTHING about an import that had written half a workspace. Scoped to the
  // caller so a job id is not a lookup into someone else's history.
  mica_app_core::import_store::get(&state.db, job_id, user_id)
    .await
    .map(Json)
    .ok_or(ApiError::NotFound)
}

/// `GET /api/import/jobs` — this user's recent imports, newest first.
///
/// The history the memory map could never hold. Notion, Outline and Slack all
/// keep long imports under Settings, and the reason is the same one that made
/// this table necessary: progress has to live somewhere the user can find
/// AGAIN, after closing the tab, after a restart, after a week.
pub async fn import_history(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<ImportHistoryResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  Ok(Json(ImportHistoryResponse {
    jobs: mica_app_core::import_store::history(&state.db, user_id)
      .await
      .into_iter()
      .map(|(id, job, created_at)| ImportHistoryEntry {
        job_id: id,
        created_at,
        job,
      })
      .collect(),
  }))
}

/// Wrapped in an object rather than returned as a bare array, matching every
/// other list endpoint here (`{"views": …}`, `{"tokens": …}`). Consistency is
/// the whole reason: the client has one code path for list responses, and a
/// lone endpoint answering with a top-level array would need its own.
#[derive(Debug, serde::Serialize)]
pub struct ImportHistoryResponse {
  jobs: Vec<ImportHistoryEntry>,
}

#[derive(Debug, serde::Serialize)]
pub struct ImportHistoryEntry {
  job_id: Uuid,
  created_at: String,
  #[serde(flatten)]
  job: ImportJob,
}

async fn run_import(
  state: &AppState,
  user_id: Uuid,
  job_id: Uuid,
  params: ImportParams,
  body: Bytes,
) -> ApiResult<()> {
  // Unzip + plan are CPU-bound — keep them off the async workers.
  let notion = params.notion;
  // Destination decides the "container vs flatten" shape (see import-convention
  // research): into an existing workspace/folder the user picks `container`
  // (auto/spill/wrap; default auto peels a single wrapper & spills, wraps only
  // multiple loose roots); as a NEW workspace → the workspace IS the container,
  // so `container` is moot and a redundant single wrapper is collapsed. The
  // container fallback name is the client-supplied source name, else "Imported".
  let mode = if params.workspace_id.is_some() {
    let fallback = params
      .name
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .unwrap_or("Imported")
      .to_string();
    match params.container {
      ContainerMode::Auto => ImportMode::Auto(fallback),
      ContainerMode::Spill => ImportMode::IntoLocation,
      ContainerMode::Wrap => ImportMode::IntoContainer(fallback),
    }
  } else {
    ImportMode::NewWorkspace
  };
  let plan: ImportPlan =
    tokio::task::spawn_blocking(move || plan_import(read_zip(&body), notion, mode))
      .await
      .map_err(|e| ApiError::Internal(e.to_string()))?;
  if plan.pages.is_empty() {
    // Coded, not merely messaged: this is the one import rejection the user can
    // actually act on (re-export from Notion as "Markdown & CSV" rather than
    // "HTML"), so the client has to recognise it and say so in their language
    // instead of echoing this English sentence into a red banner.
    return Err(ApiError::BadRequestCode(
      "import_no_markdown",
      "no markdown pages found in the archive".to_string(),
    ));
  }

  let workspace_id = match params.workspace_id {
    Some(id) => id,
    None => {
      // Client name (zip/folder) wins; else the collapsed wrapper's name
      // (Logseq: the stripped folder names the new workspace); else "Imported".
      let name = params
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(plan.wrapper_name.as_deref())
        .unwrap_or("Imported");
      create_workspace(state, user_id, name).await?
    }
  };
  {
    let mut jobs = state.import_jobs.write().await;
    if let Some(job) = jobs.get_mut(&job_id) {
      job.total = plan.pages.len();
      job.workspace_id = Some(workspace_id);
    }
  }
  // Written immediately rather than at the next checkpoint: the page count and
  // the workspace are what make a stored job READABLE ("137 of 592, into
  // Notion 导入"), and an import interrupted early would otherwise be a row
  // saying 0 of 0 with no destination.
  persist_progress(state, job_id, 0).await;

  // Top-level nodes (no parent in the plan) hang off the import target: the
  // given folder, or the workspace root when none.
  let root_parent = params.parent_view_id;
  // Pre-generate every page's view id so links can target pages that come
  // later in the plan (forward references included).
  let view_ids: Vec<Uuid> = plan.pages.iter().map(|_| Uuid::new_v4()).collect();
  let file_paths: std::collections::HashSet<String> = plan.files.keys().cloned().collect();
  let mut uploaded: std::collections::HashMap<String, (String, String)> =
    std::collections::HashMap::new();
  // External URLs re-hosted this import, keyed by url, so the same link shared
  // across pages is fetched once.
  let mut rehosted: std::collections::HashMap<String, (String, String)> =
    std::collections::HashMap::new();
  // The other half of that memo: urls this import has already established it
  // cannot bring in, as (reason, was it actually attempted).
  //
  // Needed for two things. It stops the page loop re-fetching a url the
  // pre-scan already failed on — a dead host linked from forty pages cost forty
  // more timeouts, in series, after the concurrent pass had already settled the
  // question. And it carries the reason to the failure record, so the list the
  // user sees says *why* rather than just *which*.
  let mut unfetched: std::collections::HashMap<String, (String, bool)> =
    std::collections::HashMap::new();
  // Every block left pointing at somebody else's server, with enough to fix it.
  let mut image_failures: Vec<mica_app_core::ImageFailure> = Vec::new();
  let client = reqwest::Client::new();

  // Fetch every external image BEFORE walking the pages, several at a time.
  //
  // The per-block path below is still the one that rewires a block, and it
  // still works alone — this pass only fills the cache it reads. Done here
  // because that path is deep inside two nested loops and strictly serial: an
  // archive with a few hundred external images spent the whole import waiting
  // on one request at a time. The scan costs a second parse of each page's
  // Markdown, which is CPU and bounded; the fetches are the network, and that
  // is where the time actually went.
  if params.rehost_external {
    let mut wanted: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for page in plan.pages.iter().filter(|p| !p.is_folder) {
      let from = page.archive_path.as_deref().unwrap_or("");
      for block in import_markdown(&page.markdown, "scan").blocks {
        if block.kind != "image" {
          continue;
        }
        let Some(url) = block.data.get("url").and_then(|v| v.as_str()) else {
          continue;
        };
        // Images that resolve inside the archive are uploaded from its bytes;
        // only real links go over the network.
        if resolve_ref(from, url, &file_paths).is_some() {
          continue;
        }
        if (url.starts_with("http://") || url.starts_with("https://"))
          && seen.insert(url.to_string())
        {
          wanted.push(url.to_string());
        }
      }
    }

    if !wanted.is_empty() {
      tracing::info!(count = wanted.len(), "import: pre-fetching external images");
    }
    let mut breaker = HostBreaker::default();
    let mut refused = 0usize;
    for chunk in wanted.chunks(REHOST_CONCURRENCY) {
      if state
        .import_jobs
        .read()
        .await
        .get(&job_id)
        .is_some_and(|j| j.cancel_requested)
      {
        return Ok(());
      }
      // The breaker is consulted between chunks, so a dead host costs at most
      // one chunk of timeouts rather than one per image.
      let live: Vec<&String> = chunk.iter().filter(|u| !breaker.is_open(u)).collect();
      for url in chunk.iter().filter(|u| breaker.is_open(u)) {
        // Refused WITHOUT a request. Recorded with the same weight as a failed
        // fetch, because to the document there is no difference: the block is
        // still a link. This was the most invisible of the three outcomes —
        // nothing errored, it simply never happened, and it left only an
        // aggregate count in a log the user never reads.
        unfetched.insert(
          url.clone(),
          (
            "host had already timed out repeatedly, so this one was skipped without a request"
              .to_string(),
            false,
          ),
        );
      }
      refused += chunk.len() - live.len();
      let results = futures_util::future::join_all(live.into_iter().map(|url| async move {
        (
          url.clone(),
          fetch_and_store_image_url(state, workspace_id, user_id, url).await,
        )
      }))
      .await;
      for (url, result) in results {
        match result {
          Ok(record) => {
            rehosted.insert(url, (record.id.to_string(), record.original_name.clone()));
          }
          Err(error) => {
            let reason = error.to_string();
            breaker.record(&url, &reason);
            tracing::warn!(%url, error = %reason, "import: external image re-host failed; keeping link");
            unfetched.insert(url, (reason, true));
          }
        }
      }
    }
    if refused > 0 {
      // Said out loud, with the count: these images kept their links without
      // ever being tried, and an operator who does not know that will read the
      // import as having re-hosted everything it could.
      tracing::warn!(
        refused,
        "import: images skipped unattempted — their host had already timed out repeatedly; \
         they keep their original links (re-run `mica-cli rehost-images` from a network that \
         can reach them)"
      );
    }
  }

  for (idx, page) in plan.pages.iter().enumerate() {
    // Stop between pages when asked. Checked HERE rather than by aborting the
    // task, so we never stop with a document created but its content half
    // written. `done` already reflects the pages that landed, and nothing is
    // rolled back — see `ImportJob::cancel_requested`.
    if state
      .import_jobs
      .read()
      .await
      .get(&job_id)
      .is_some_and(|j| j.cancel_requested)
    {
      return Ok(());
    }
    // A folder is a pure container: just a view, no document/snapshot, no
    // markdown/asset processing (F2 round-trip — folders survive export→import).
    if page.is_folder {
      insert_folder(
        state,
        workspace_id,
        user_id,
        view_ids[idx],
        page.parent.map(|p| view_ids[p]).or(root_parent),
        &page.title,
      )
      .await?;
      {
        let mut jobs = state.import_jobs.write().await;
        if let Some(job) = jobs.get_mut(&job_id) {
          job.done = idx + 1;
        }
      }
      persist_progress(state, job_id, idx + 1).await;
      continue;
    }
    let root_block_id = format!("block_{}", Uuid::new_v4().simple());
    let mut payload = page_seed(&page.markdown, &page.title, &root_block_id);
    let from = page.archive_path.as_deref().unwrap_or("");
    // Failures for THIS page, held until insert_page hands back the document id
    // they have to be addressed to. (url, block_id, reason, attempted)
    let mut page_failures: Vec<(String, String, String, bool)> = Vec::new();

    for block in &mut payload.blocks {
      // Image refs that resolve inside the archive: upload once, rewire to
      // Mica's {file_id, name} form. External http(s) links: re-host into our
      // storage too (best-effort) when `rehost_external` is on, else stay links.
      if block.kind == "image"
        && let Some(url) = block.data.get("url").and_then(|v| v.as_str()).map(str::to_string)
      {
        if let Some(path) = resolve_ref(from, &url, &file_paths) {
          let entry = match uploaded.get(&path) {
            Some(hit) => hit.clone(),
            None => {
              let base = path.rsplit('/').next().unwrap_or(&path);
              let record =
                store_bytes(state, &client, workspace_id, user_id, base, &plan.files[&path])
                  .await?;
              let value = (record.id.to_string(), record.original_name.clone());
              uploaded.insert(path.clone(), value.clone());
              value
            }
          };
          block.data = json!({"file_id": entry.0, "name": entry.1});
        } else if params.rehost_external
          && (url.starts_with("http://") || url.starts_with("https://"))
        {
          // Pull the external image into Mica so it can't rot. Server-side and
          // best-effort — a host this server can't reach (CN routing to
          // medium/imgur/…) just keeps its link, exactly as before. The client's
          // per-image re-host remains the fallback for those.
          let entry = match rehosted.get(&url) {
            Some(hit) => Some(hit.clone()),
            // Already established as unreachable by the concurrent pre-scan.
            // Do NOT try again: the pre-scan covers every external url in the
            // archive, so reaching here means the answer is known, and a dead
            // host linked from forty pages used to cost forty more timeouts in
            // series after the question was already settled.
            None if unfetched.contains_key(&url) => None,
            None => match fetch_and_store_image_url(state, workspace_id, user_id, &url).await {
              Ok(record) => {
                let value = (record.id.to_string(), record.original_name.clone());
                rehosted.insert(url.clone(), value.clone());
                Some(value)
              }
              Err(error) => {
                let reason = error.to_string();
                tracing::warn!(%url, error = %reason, "import: external image re-host failed; keeping link");
                unfetched.insert(url.clone(), (reason, true));
                None
              }
            },
          };
          match entry {
            Some(entry) => block.data = json!({"file_id": entry.0, "name": entry.1}),
            // Left as a link, and now said out loud. `document_id` is stamped
            // after insert_page below — the page does not exist yet.
            None => {
              let (reason, attempted) = unfetched
                .get(&url)
                .cloned()
                .unwrap_or_else(|| ("could not be fetched".to_string(), true));
              page_failures.push((url.clone(), block.id.clone(), reason, attempted));
            }
          }
        }
      }
      // Relative .md links become internal page links.
      if let Some(marks) = block.data.get_mut("marks").and_then(|m| m.as_array_mut()) {
        for mark in marks {
          let Some(obj) = mark.as_object_mut() else {
            continue;
          };
          let Some(target) = obj
            .get("href")
            .and_then(|v| v.as_str())
            .and_then(|href| resolve_ref(from, href, &plan.md_paths))
            .and_then(|path| plan.page_by_path.get(&path))
          else {
            continue;
          };
          obj.insert("href".into(), json!(format!("mica://page/{}", view_ids[*target])));
        }
      }
    }

    let document_id = insert_page(
      state,
      workspace_id,
      user_id,
      view_ids[idx],
      page.parent.map(|p| view_ids[p]).or(root_parent),
      &page.title,
      &root_block_id,
      &payload,
    )
    .await?;

    for (url, block_id, reason, attempted) in page_failures {
      image_failures.push(mica_app_core::ImageFailure {
        url,
        page: page.title.clone(),
        document_id,
        block_id,
        reason,
        attempted,
      });
    }

    {
      let mut jobs = state.import_jobs.write().await;
      if let Some(job) = jobs.get_mut(&job_id) {
        job.done = idx + 1;
        // Published as the import runs, not only at the end: a long archive
        // against a dead host should show the damage accumulating rather than
        // look clean until the last page lands.
        job.image_failures_total = image_failures.len();
        job.image_failures = capped(&image_failures);
      }
    }
    persist_progress(state, job_id, idx + 1).await;
  }

  // Whatever came in the archive that no page ended up pointing at. `uploaded` is
  // filled only when a page actually references an entry (assets are uploaded
  // lazily), so the difference IS the set that silently didn't make it — an
  // export's leftovers, a Notion `.csv` next to a database page, a stray
  // screenshot. Without this the user cannot tell "imported everything" apart
  // from "imported most of it".
  {
    // Sorted so the list is stable between polls of the status endpoint;
    // HashSet iteration order would reshuffle it on every request.
    let mut skipped: Vec<String> = file_paths
      .iter()
      .filter(|p| !uploaded.contains_key(*p))
      .cloned()
      .collect();
    skipped.sort();
    // Capped: this rides a polling endpoint, and a pathological archive (a
    // node_modules dump) would otherwise ship thousands of paths on every poll.
    // The client shows a count, so a truncated list still tells the truth about
    // how many — see `skipped_total`.
    let mut jobs = state.import_jobs.write().await;
    if let Some(job) = jobs.get_mut(&job_id) {
      job.skipped_total = skipped.len();
      skipped.truncate(MAX_LISTED);
      job.skipped = skipped;
    }
  }
  Ok(())
}

async fn create_workspace(state: &AppState, user_id: Uuid, name: &str) -> ApiResult<Uuid> {
  let mut tx = state.db.begin().await?;
  let workspace_id = sqlx::query_scalar::<_, Uuid>(
    r#"
      INSERT INTO workspaces (name, owner_id)
      VALUES ($1, $2)
      RETURNING id
    "#,
  )
  .bind(name)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;
  let next_pos = crate::routes::workspaces::next_member_position(&state.db, user_id).await?;
  sqlx::query(
    r#"
      INSERT INTO workspace_members (workspace_id, user_id, role, position)
      VALUES ($1, $2, 'owner', $3)
    "#,
  )
  .bind(workspace_id)
  .bind(user_id)
  .bind(next_pos)
  .execute(&mut *tx)
  .await?;
  tx.commit().await?;
  Ok(workspace_id)
}

/// Insert a folder view (pure container) — no document, no snapshot. `object_id`
/// is a fresh unreferenced uuid to satisfy the NOT NULL column. Mirrors
/// [`documents::create_folder`], used by the import executor for folder pages.
async fn insert_folder(
  state: &AppState,
  workspace_id: Uuid,
  user_id: Uuid,
  view_id: Uuid,
  parent_view_id: Option<Uuid>,
  title: &str,
) -> ApiResult<()> {
  let title = title.trim();
  let title = if title.is_empty() { "Untitled" } else { title };
  let object_id = Uuid::new_v4();
  let position = Uuid::now_v7().to_string();
  sqlx::query(
    r#"
      INSERT INTO views (
        id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by
      )
      VALUES ($1, $2, $3, $4, 'folder', $5, $6, $7)
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .bind(parent_view_id)
  .bind(object_id)
  .bind(title)
  .bind(position)
  .bind(user_id)
  .execute(&state.db)
  .await?;
  Ok(())
}

/// The document seed for one planned page: its Markdown parsed into blocks,
/// with the page's own name on the root block.
///
/// The name goes INTO the document, not only into the `views.name` column beside
/// it, so an imported page knows what it is called by exactly the same rule as a
/// page created here and renamed — `docs/page-title-plan.md`. Folded into the
/// seed rather than written afterwards: it rides along in the initial state and
/// costs no update at all, which is what makes it compatible with §5.3's
/// "importing 400 pages must not mean 400 document writes".
///
/// A nameless entry gets no title and keeps falling back to the column, where
/// `insert_page` puts "Untitled" — a literal "Untitled" stored as the document's
/// own title would export as `# Untitled`.
///
/// Extracted so the rule is testable: `run_import` builds a page's payload here
/// and nowhere else, and the image rewiring downstream only edits blocks.
fn page_seed(
  markdown: &str,
  title: &str,
  root_block_id: &str,
) -> mica_app_core::documents::DocumentSnapshotPayload {
  let mut payload = import_markdown(markdown, root_block_id);
  mica_markdown::set_document_title(&mut payload, title);
  payload
}

#[allow(clippy::too_many_arguments)]
async fn insert_page(
  state: &AppState,
  workspace_id: Uuid,
  user_id: Uuid,
  view_id: Uuid,
  parent_view_id: Option<Uuid>,
  title: &str,
  root_block_id: &str,
  payload: &mica_app_core::documents::DocumentSnapshotPayload,
  // Returns the DOCUMENT id, not the view id. They are different ids, and the
  // one the `rehost-image` endpoint takes — the one an image failure has to be
  // addressed to — is this one.
) -> ApiResult<Uuid> {
  let mut tx = state.db.begin().await?;
  let document = sqlx::query_as::<_, DocumentRecord>(
    r#"
      INSERT INTO documents (workspace_id, root_block_id, created_by)
      VALUES ($1, $2, $3)
      RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(root_block_id)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;

  // Seed the yrs base with the page's content, in the import's own transaction.
  // `content_text` (the FTS index) is co-written with the base, so each imported
  // page's BODY is searchable at once — batch import of a whole Notion workspace
  // is the case that motivated this (100+ pages a user searches before opening
  // any). Before S4 this wrote an op-model snapshot and built the base
  // afterwards, best-effort and per page.
  mica_app_core::sync::seed_base_tx(&mut tx, document.id, payload.clone()).await?;

  let title = title.trim();
  let title = if title.is_empty() { "Untitled" } else { title };
  let position = Uuid::now_v7().to_string();
  sqlx::query(
    r#"
      INSERT INTO views (
        id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by
      )
      VALUES ($1, $2, $3, $4, 'document', $5, $6, $7)
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .bind(parent_view_id)
  .bind(document.id)
  .bind(title)
  .bind(position)
  .bind(user_id)
  .execute(&mut *tx)
  .await?;

  tx.commit().await?;
  Ok(document.id)
}


#[cfg(test)]
mod page_seed_tests {
  use super::page_seed;

  /// An imported page carries its name inside the document, the same way a
  /// renamed one does. Before this, every imported page depended on the
  /// `views.name` fallback, and a Mica archive re-exported from a fresh import
  /// got its title back from the column rather than from the document.
  #[test]
  fn an_imported_page_carries_its_name_in_the_document() {
    let seed = page_seed("正文。", "季度回顾", "root");
    assert_eq!(mica_markdown::document_title(&seed), Some("季度回顾"));
  }

  /// A nameless archive entry must NOT store a title. `insert_page` puts the
  /// literal "Untitled" in the column for these; storing that as the document's
  /// own title would make it export as `# Untitled` on a page nobody named.
  #[test]
  fn a_nameless_entry_stores_no_title_and_keeps_falling_back() {
    assert_eq!(mica_markdown::document_title(&page_seed("正文。", "", "root")), None);
    assert_eq!(mica_markdown::document_title(&page_seed("正文。", "  ", "root")), None);
  }

  /// The title lives on the root block beside `front_matter`, and a page's front
  /// matter comes from the file it was imported from — losing it here would
  /// silently drop every page property in the archive.
  #[test]
  fn front_matter_from_the_file_survives_the_title() {
    let seed = page_seed("---\ntags: [a]\n---\n\n正文。", "名字", "root");
    assert_eq!(mica_markdown::document_title(&seed), Some("名字"));
    let root = seed.blocks.iter().find(|b| b.id == "root").expect("root");
    assert_eq!(root.data.get("front_matter").and_then(|v| v.as_str()), Some("tags: [a]"));
  }

  /// The body is untouched: the title is metadata on the root, never a block.
  /// (Import already stripped a duplicated `# name` upstream, in `plan_import`.)
  #[test]
  fn the_title_does_not_become_a_block_in_the_body() {
    let seed = page_seed("正文。", "季度回顾", "root");
    assert!(
      !seed.blocks.iter().any(|b| b.text.contains("季度回顾")),
      "the title must not appear as content: {:?}",
      seed.blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
    );
  }
}

#[cfg(test)]
mod import_job_persistence_tests {
  use super::PERSIST_EVERY;
  use mica_app_core::ImportJobStatus;

  /// The stored `status` column is a string, and every reader (the client, the
  /// CLI's `ImportJobView`, a human running psql) matches on it. A renamed
  /// variant would silently turn old rows into unparseable ones, so the wire
  /// spelling is pinned here rather than left to `#[serde(rename_all)]`.
  #[test]
  fn every_status_has_a_stable_spelling() {
    for (status, text) in [
      (ImportJobStatus::Running, "running"),
      (ImportJobStatus::Done, "done"),
      (ImportJobStatus::Error, "error"),
      (ImportJobStatus::Cancelled, "cancelled"),
      (ImportJobStatus::Interrupted, "interrupted"),
    ] {
      assert_eq!(status.as_str(), text);
      assert_eq!(ImportJobStatus::parse(text), Some(status));
    }
  }

  /// A row written by a NEWER server must not come back as a crash or as the
  /// wrong state on an older one — an unknown status reads as absent, and the
  /// caller falls back to "no record", not to "done".
  #[test]
  fn an_unknown_status_is_not_guessed_at() {
    assert_eq!(ImportJobStatus::parse("teleported"), None);
    assert_eq!(ImportJobStatus::parse(""), None);
    assert_eq!(ImportJobStatus::parse("RUNNING"), None);
  }

  /// Interrupted is its own state, not a synonym. The whole reason migration
  /// 0023 exists is that a restart needs an answer that is neither "still
  /// going" (a progress bar with nothing behind it) nor "failed" (the imported
  /// pages are real and staying).
  #[test]
  fn interrupted_is_distinct_from_error_and_cancelled() {
    assert_ne!(ImportJobStatus::Interrupted, ImportJobStatus::Error);
    assert_ne!(ImportJobStatus::Interrupted, ImportJobStatus::Cancelled);
    assert_ne!(ImportJobStatus::Interrupted, ImportJobStatus::Running);
  }

  /// The throttle must not be 1 (an UPDATE per page — 600 round-trips for a
  /// 600-page archive) nor so coarse that an interrupted import reports a
  /// wildly stale count.
  #[test]
  fn the_progress_throttle_stays_in_a_sane_band() {
    assert!(
      (5..=100).contains(&PERSIST_EVERY),
      "PERSIST_EVERY = {PERSIST_EVERY}: one write per page is a round-trip storm,        and hundreds of pages between checkpoints makes a stored job a lie"
    );
  }
}
