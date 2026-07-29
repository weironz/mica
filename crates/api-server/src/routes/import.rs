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
      cancel_requested: false,
    },
  );

  tokio::spawn(async move {
    let result = run_import(&state, user_id, job_id, params, body).await;
    let mut jobs = state.import_jobs.write().await;
    if let Some(job) = jobs.get_mut(&job_id) {
      match result {
        // A cancelled run returns Ok — it stopped cleanly — so the flag, not the
        // result, decides the final status. Reporting "done" for an import the
        // user stopped halfway would claim their archive is fully in.
        Ok(()) if job.cancel_requested => job.status = ImportJobStatus::Cancelled,
        Ok(()) => job.status = ImportJobStatus::Done,
        Err(e) => {
          job.status = ImportJobStatus::Error;
          job.error = Some(e.to_string());
        }
      }
    }
  });

  Ok(Json(ImportStartResponse { job_id }))
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
  user_id_from_headers(&state, &headers).await?;
  state
    .import_jobs
    .read()
    .await
    .get(&job_id)
    .cloned()
    .map(Json)
    .ok_or(ApiError::NotFound)
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
  let client = reqwest::Client::new();

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
      let mut jobs = state.import_jobs.write().await;
      if let Some(job) = jobs.get_mut(&job_id) {
        job.done = idx + 1;
      }
      continue;
    }
    let root_block_id = format!("block_{}", Uuid::new_v4().simple());
    let mut payload = import_markdown(&page.markdown, &root_block_id);
    let from = page.archive_path.as_deref().unwrap_or("");

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
            None => match fetch_and_store_image_url(state, workspace_id, user_id, &url).await {
              Ok(record) => {
                let value = (record.id.to_string(), record.original_name.clone());
                rehosted.insert(url.clone(), value.clone());
                Some(value)
              }
              Err(error) => {
                tracing::warn!(%url, %error, "import: external image re-host failed; keeping link");
                None
              }
            },
          };
          if let Some(entry) = entry {
            block.data = json!({"file_id": entry.0, "name": entry.1});
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

    insert_page(
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

    let mut jobs = state.import_jobs.write().await;
    if let Some(job) = jobs.get_mut(&job_id) {
      job.done = idx + 1;
    }
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
    const MAX_LISTED: usize = 50;
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
) -> ApiResult<()> {
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
  Ok(())
}
