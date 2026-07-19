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
  AppState, ImportJob, ImportJobStatus,
  documents::import_markdown,
  store::{self, DocumentRecord},
};
use mica_infra::{ApiError, ApiResult};
use mica_interchange::{ImportMode, ImportPlan, plan_import, read_zip, resolve_ref};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::documents::ensure_workspace_editor;
use crate::routes::files::store_bytes;

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
    },
  );

  tokio::spawn(async move {
    let result = run_import(&state, user_id, job_id, params, body).await;
    let mut jobs = state.import_jobs.write().await;
    if let Some(job) = jobs.get_mut(&job_id) {
      match result {
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
  // research): into an existing workspace/folder → wrap everything under ONE
  // container named after the source; as a NEW workspace → the workspace IS the
  // container, so collapse a redundant single wrapper. The container fallback
  // name is the client-supplied source name (zip/folder), else "Imported".
  let mode = if params.workspace_id.is_some() {
    let fallback = params
      .name
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .unwrap_or("Imported")
      .to_string();
    ImportMode::IntoContainer(fallback)
  } else {
    ImportMode::NewWorkspace
  };
  let plan: ImportPlan =
    tokio::task::spawn_blocking(move || plan_import(read_zip(&body), notion, mode))
      .await
      .map_err(|e| ApiError::Internal(e.to_string()))?;
  if plan.pages.is_empty() {
    return Err(ApiError::BadRequest(
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
  let client = reqwest::Client::new();

  for (idx, page) in plan.pages.iter().enumerate() {
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
      // Mica's {file_id, name} form. External URLs stay links.
      if block.kind == "image"
        && let Some(url) = block.data.get("url").and_then(|v| v.as_str()).map(str::to_string)
        && let Some(path) = resolve_ref(from, &url, &file_paths)
      {
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
      }
      // Relative .md links become internal page links.
      if let Some(marks) = block.data.get_mut("marks").and_then(|m| m.as_array_mut()) {
        for mark in marks {
          let Some(obj) = mark.as_object_mut() else { continue };
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

  store::insert_root_snapshot(&mut tx, document.id, payload).await?;

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
