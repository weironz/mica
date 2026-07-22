use std::collections::BTreeMap;

use axum::{
  Json,
  extract::{Path, Query, State},
  http::{HeaderMap, header},
  response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use mica_app_core::{
  AppState,
  documents::{
    DocumentOperation, DocumentSnapshotPayload, export_html, export_html_document,
    export_markdown_with_assets, import_markdown, set_image_srcs,
  },
  store::{self, DocumentRecord, SnapshotRecord, UpdateRecord},
};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::ws;
use mica_interchange::{ZipEntry, build_zip};

#[derive(Debug, Deserialize)]
pub struct CreateDocumentRequest {
  name: String,
  parent_view_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFolderRequest {
  name: String,
  parent_view_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateViewRequest {
  name: String,
}

#[derive(Debug, Deserialize)]
pub struct MoveViewRequest {
  parent_view_id: Option<Uuid>,
  position: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReorderRequest {
  /// Parent that every listed view becomes a child of (null = top level).
  #[serde(default)]
  parent_view_id: Option<Uuid>,
  /// The COMPLETE desired order of that parent's children. Positions are
  /// reassigned evenly-spaced in this order; pass the full set so nothing keeps
  /// a stale position that interleaves with the reordered ones.
  ordered_view_ids: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct ReorderResponse {
  reordered: usize,
}

#[derive(Debug, Deserialize)]
pub struct ApplyDocumentUpdateRequest {
  operations: Vec<DocumentOperation>,
}

#[derive(Debug, Deserialize)]
pub struct ImportMarkdownRequest {
  name: String,
  parent_view_id: Option<Uuid>,
  markdown: String,
}

#[derive(Debug, Serialize)]
pub struct ViewListResponse {
  views: Vec<View>,
}

#[derive(Debug, Serialize)]
pub struct DocumentCreateResponse {
  document: DocumentRecord,
  view: View,
}

#[derive(Debug, Serialize)]
pub struct DocumentBootstrapResponse {
  document: DocumentRecord,
  view: View,
  snapshot: SnapshotRecord,
}

#[derive(Debug, Serialize)]
pub struct DocumentUpdateResponse {
  document: DocumentRecord,
  snapshot: SnapshotRecord,
  update: UpdateRecord,
}

#[derive(Debug, Serialize)]
pub struct MarkdownExportResponse {
  markdown: String,
}

#[derive(Debug, Serialize)]
pub struct ViewResponse {
  view: View,
}

#[derive(Debug, Serialize, FromRow)]
pub struct View {
  id: Uuid,
  workspace_id: Uuid,
  parent_view_id: Option<Uuid>,
  object_id: Uuid,
  object_type: String,
  name: String,
  icon: Option<String>,
  position: String,
  is_deleted: bool,
  created_by: Uuid,
  created_at: DateTime<Utc>,
  updated_at: DateTime<Utc>,
}

pub async fn list_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let views = fetch_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
  q: String,
}

#[derive(Debug, Serialize)]
struct SearchResult {
  view_id: Uuid,
  object_id: Uuid,
  name: String,
  snippet: String,
  title_match: bool,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
  results: Vec<SearchResult>,
}

/// `GET /api/workspaces/{workspace_id}/search?q=...` — find pages whose title or
/// body text contains the query (case-insensitive), with a short snippet.
pub async fn search_workspace(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Query(query): Query<SearchQuery>,
) -> ApiResult<Json<SearchResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let needle = query.q.trim().to_lowercase();
  if needle.is_empty() {
    return Ok(Json(SearchResponse { results: vec![] }));
  }

  let views = fetch_workspace_views(&state.db, workspace_id).await?;
  // Only documents carry body text; folders have none.
  let docs: Vec<View> = views
    .into_iter()
    .filter(|v| v.object_type == "document")
    .collect();

  // Each document's current text is reconstructed from its yrs base (a DB round
  // trip + a CRDT decode) — the body is NOT a queryable column, so a workspace
  // with N documents is N reconstructions no matter what. Two things keep that
  // from being O(N) latency: run them concurrently (`buffered` preserves tree
  // order), and STOP once 50 hits are in hand — buffered stops polling new
  // futures the moment we stop consuming, so a 5000-doc workspace reconstructs
  // ~50-plus-a-window, not all 5000. Before this it was a sequential await loop.
  let db = &state.db;
  let needle = needle.as_str();
  use futures_util::StreamExt as _;
  let mut hits = futures_util::stream::iter(docs.into_iter().map(|view| async move {
    let title_match = view.name.to_lowercase().contains(needle);
    let mut snippet = String::new();
    // A single corrupt/unreadable document drops out of the results rather than
    // 500-ing the whole search — discovery is best-effort by nature. But it must
    // not do so SILENTLY: a corrupt payload vanishing from search was the only
    // early signal of the kind of corruption the 2026-07-19 incident produced
    // (P1-3; blob_gc.rs logs the same class of error).
    match store::current_payload(db, view.object_id).await {
      Ok(Some(payload)) => {
        for block in &payload.blocks {
          if let Some(found) = snippet_for(&block.text, needle) {
            snippet = found;
            break;
          }
        }
      }
      Ok(None) => {}
      Err(error) => tracing::warn!(
        view_id = %view.id,
        object_id = %view.object_id,
        %error,
        "search: skipping unreadable document"
      ),
    }
    (title_match || !snippet.is_empty()).then_some(SearchResult {
      view_id: view.id,
      object_id: view.object_id,
      name: view.name,
      snippet,
      title_match,
    })
  }))
  .buffered(SEARCH_CONCURRENCY);

  let mut results = Vec::new();
  while let Some(hit) = hits.next().await {
    if let Some(result) = hit {
      results.push(result);
      if results.len() >= 50 {
        break;
      }
    }
  }

  Ok(Json(SearchResponse { results }))
}

/// How many document reconstructions run at once during a search. Bounded so a
/// large workspace cannot drain the connection pool; small enough to leave room
/// for everything else the server is doing.
const SEARCH_CONCURRENCY: usize = 8;

/// First ~160 chars of a block that contains the query, or `None`.
fn snippet_for(text: &str, needle_lower: &str) -> Option<String> {
  if !text.to_lowercase().contains(needle_lower) {
    return None;
  }
  let trimmed = text.trim();
  let snippet: String = trimmed.chars().take(160).collect();
  if trimmed.chars().count() > 160 {
    Some(format!("{snippet}…"))
  } else {
    Some(snippet)
  }
}

#[derive(Debug, Serialize)]
struct Backlink {
  view_id: Uuid,
  document_id: Uuid,
  title: String,
}

#[derive(Debug, Serialize)]
pub struct BacklinksResponse {
  backlinks: Vec<Backlink>,
}

/// `GET /api/workspaces/{workspace_id}/views/{view_id}/backlinks` — the pages in
/// this workspace that link TO `view_id`, i.e. any live document whose blocks
/// carry a `mica://page/<view_id>` link mark.
///
/// This is the inverse of the forward page-link scan the transfer flow runs
/// ([`page_link_targets`] over each document's [`store::current_payload`]).
/// Computed on demand with NO maintained index — the same on-the-fly O(document)
/// walk as full-text search (see `search_workspace`); a real reverse-index table
/// only earns its keep once the scan itself is the bottleneck. Cloud-only: the
/// local (offline) world has its own store and never hits this endpoint.
pub async fn backlinks(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<BacklinksResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  // A backlink query for a page that doesn't live here is a 404, not an empty
  // list — matches transfer/move's `ensure_view_in_workspace` contract.
  ensure_view_in_workspace(&state.db, workspace_id, view_id).await?;

  let target = view_id.to_string();
  // Live views only (fetch_workspace_views filters is_deleted); folders carry no
  // blocks so they can never be a source.
  let views = fetch_workspace_views(&state.db, workspace_id).await?;

  let mut backlinks = Vec::new();
  for view in &views {
    if view.object_type != "document" {
      continue;
    }
    // A page linking to itself is not a backlink.
    if view.id == view_id {
      continue;
    }
    // Each document's payload is a DB round-trip + CRDT decode (body text is not
    // a queryable column). A single unreadable document drops out rather than
    // 500-ing the whole panel — but it must not vanish SILENTLY (same corruption
    // signal search_workspace logs).
    let payload = match store::current_payload(&state.db, view.object_id).await {
      Ok(Some(payload)) => payload,
      Ok(None) => continue,
      Err(error) => {
        tracing::warn!(
          view_id = %view.id,
          object_id = %view.object_id,
          %error,
          "backlinks: skipping unreadable document"
        );
        continue;
      }
    };
    let links_here = payload
      .blocks
      .iter()
      .any(|block| page_link_targets(&block.data).iter().any(|t| *t == target));
    if links_here {
      backlinks.push(Backlink {
        view_id: view.id,
        document_id: view.object_id,
        title: view.name.clone(),
      });
    }
  }

  // Stable order: title first (what the panel shows), view_id to break ties.
  backlinks.sort_by(|a, b| a.title.cmp(&b.title).then(a.view_id.cmp(&b.view_id)));

  Ok(Json(BacklinksResponse { backlinks }))
}

pub async fn create_document(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<CreateDocumentRequest>,
) -> ApiResult<Json<DocumentCreateResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let name = normalize_view_name(&payload.name)?;

  if let Some(parent_view_id) = payload.parent_view_id {
    ensure_parent_accepts_children(&state.db, workspace_id, parent_view_id).await?;
  }

  let mut tx = state.db.begin().await?;
  let root_block_id = format!("block_{}", Uuid::new_v4().simple());

  let document = sqlx::query_as::<_, DocumentRecord>(
    r#"
      INSERT INTO documents (workspace_id, root_block_id, created_by)
      VALUES ($1, $2, $3)
      RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(&root_block_id)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;

  store::insert_initial_snapshot(&mut tx, &document).await?;

  let position = Uuid::now_v7().to_string();
  let view = sqlx::query_as::<_, View>(
    r#"
      INSERT INTO views (
        workspace_id,
        parent_view_id,
        object_id,
        object_type,
        name,
        position,
        created_by
      )
      VALUES ($1, $2, $3, 'document', $4, $5, $6)
      RETURNING
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(payload.parent_view_id)
  .bind(document.id)
  .bind(name)
  .bind(position)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;

  tx.commit().await?;

  Ok(Json(DocumentCreateResponse { document, view }))
}

/// `POST /api/workspaces/{workspace_id}/folders`
///
/// Create a folder view — a pure container in the page tree (AFFiNE-style
/// "entity used solely for organizing content"). Unlike [`create_document`] it
/// inserts ONLY a `views` row with `object_type='folder'`: no `documents` row,
/// no snapshot, no CRDT sync. `object_id` gets a fresh (unreferenced) uuid to
/// satisfy the NOT NULL column. Export renders it as a directory, never a `.md`.
pub async fn create_folder(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<CreateFolderRequest>,
) -> ApiResult<Json<ViewResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let name = normalize_view_name(&payload.name)?;

  if let Some(parent_view_id) = payload.parent_view_id {
    ensure_parent_accepts_children(&state.db, workspace_id, parent_view_id).await?;
  }

  // No document / snapshot — a folder has no content. `object_id` is a fresh
  // uuid purely to satisfy the NOT NULL column; nothing references it.
  let object_id = Uuid::new_v4();
  let position = Uuid::now_v7().to_string();
  let view = sqlx::query_as::<_, View>(
    r#"
      INSERT INTO views (
        workspace_id,
        parent_view_id,
        object_id,
        object_type,
        name,
        position,
        created_by
      )
      VALUES ($1, $2, $3, 'folder', $4, $5, $6)
      RETURNING
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(payload.parent_view_id)
  .bind(object_id)
  .bind(name)
  .bind(position)
  .bind(user_id)
  .fetch_one(&state.db)
  .await?;

  Ok(Json(ViewResponse { view }))
}

/// `POST /api/workspaces/{workspace_id}/documents/import/markdown`
///
/// Create a new document whose initial snapshot is parsed from Markdown, and a
/// matching view in the page tree.
pub async fn import_document_markdown(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<ImportMarkdownRequest>,
) -> ApiResult<Json<DocumentBootstrapResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let name = normalize_view_name(&payload.name)?;

  if let Some(parent_view_id) = payload.parent_view_id {
    ensure_parent_accepts_children(&state.db, workspace_id, parent_view_id).await?;
  }

  let mut tx = state.db.begin().await?;
  let root_block_id = format!("block_{}", Uuid::new_v4().simple());

  let document = sqlx::query_as::<_, DocumentRecord>(
    r#"
      INSERT INTO documents (workspace_id, root_block_id, created_by)
      VALUES ($1, $2, $3)
      RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(&root_block_id)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;

  let mut imported = import_markdown(&payload.markdown, &root_block_id);
  rewire_blob_hrefs(&mut imported.blocks, workspace_id);
  let snapshot = store::insert_root_snapshot(&mut tx, document.id, &imported).await?;

  let position = Uuid::now_v7().to_string();
  let view = sqlx::query_as::<_, View>(
    r#"
      INSERT INTO views (
        workspace_id,
        parent_view_id,
        object_id,
        object_type,
        name,
        position,
        created_by
      )
      VALUES ($1, $2, $3, 'document', $4, $5, $6)
      RETURNING
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
    "#,
  )
  .bind(workspace_id)
  .bind(payload.parent_view_id)
  .bind(document.id)
  .bind(name)
  .bind(position)
  .bind(user_id)
  .fetch_one(&mut *tx)
  .await?;

  tx.commit().await?;

  Ok(Json(DocumentBootstrapResponse {
    document,
    view,
    snapshot,
  }))
}

pub async fn update_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<UpdateViewRequest>,
) -> ApiResult<Json<ViewResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let name = normalize_view_name(&payload.name)?;
  let view = sqlx::query_as::<_, View>(
    r#"
      UPDATE views
      SET name = $1, updated_at = now()
      WHERE id = $2 AND workspace_id = $3 AND is_deleted = false
      RETURNING
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
    "#,
  )
  .bind(name)
  .bind(view_id)
  .bind(workspace_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::NotFound)?;

  Ok(Json(ViewResponse { view }))
}

pub async fn delete_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  // Soft-delete (move to the recycle bin) the page and its whole subtree.
  let result = sqlx::query(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id FROM views WHERE id = $1 AND workspace_id = $2
        UNION ALL
        SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
      )
      UPDATE views
      SET is_deleted = true, updated_at = now()
      WHERE id IN (SELECT id FROM subtree)
        AND workspace_id = $2
        AND is_deleted = false
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .execute(&state.db)
  .await?;

  if result.rows_affected() == 0 {
    return Err(ApiError::NotFound);
  }

  let views = fetch_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

pub async fn list_trash(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let views = fetch_deleted_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

pub async fn restore_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  // Restore the page and the subtree that was deleted with it.
  let result = sqlx::query(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id FROM views WHERE id = $1 AND workspace_id = $2
        UNION ALL
        SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
      )
      UPDATE views
      SET is_deleted = false, updated_at = now()
      WHERE id IN (SELECT id FROM subtree)
        AND workspace_id = $2
        AND is_deleted = true
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .execute(&state.db)
  .await?;

  if result.rows_affected() == 0 {
    return Err(ApiError::NotFound);
  }

  // If the restored page's parent is no longer an active view, lift it to the
  // top level so it does not become an orphan.
  sqlx::query(
    r#"
      UPDATE views
      SET parent_view_id = NULL, updated_at = now()
      WHERE id = $1 AND workspace_id = $2 AND parent_view_id IS NOT NULL
        AND parent_view_id NOT IN (
          SELECT id FROM views WHERE workspace_id = $2 AND is_deleted = false
        )
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .execute(&state.db)
  .await?;

  let views = fetch_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

pub async fn purge_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  // Permanently remove the page and its subtree from the recycle bin.
  let result = sqlx::query(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id FROM views WHERE id = $1 AND workspace_id = $2
        UNION ALL
        SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
      )
      DELETE FROM views
      WHERE id IN (SELECT id FROM subtree) AND workspace_id = $2
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .execute(&state.db)
  .await?;

  if result.rows_affected() == 0 {
    return Err(ApiError::NotFound);
  }

  let views = fetch_deleted_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

pub async fn move_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<MoveViewRequest>,
) -> ApiResult<Json<ViewResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_view_in_workspace(&state.db, workspace_id, view_id).await?;

  if let Some(parent_view_id) = payload.parent_view_id {
    ensure_valid_parent_view(&state.db, workspace_id, view_id, parent_view_id).await?;
  }

  let position = normalize_position(payload.position)?;
  let view = sqlx::query_as::<_, View>(
    r#"
      UPDATE views
      SET parent_view_id = $1, position = $2, updated_at = now()
      WHERE id = $3 AND workspace_id = $4 AND is_deleted = false
      RETURNING
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
    "#,
  )
  .bind(payload.parent_view_id)
  .bind(position)
  .bind(view_id)
  .bind(workspace_id)
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::NotFound)?;

  Ok(Json(ViewResponse { view }))
}

/// `POST /api/workspaces/{workspace_id}/views/reorder`
///
/// Reorder a parent's children in ONE atomic call: every id in
/// `ordered_view_ids` is set as a child of `parent_view_id` (null = top level)
/// and given an evenly-spaced position in the given order. This is what a "sort
/// this folder" operation needs — the per-view `move` endpoint would take one
/// request per sibling and could interleave a failure. Positions are 10-spaced,
/// zero-padded to a fixed width so they sort lexicographically like the rest.
pub async fn reorder_views(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<ReorderRequest>,
) -> ApiResult<Json<ReorderResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  if payload.ordered_view_ids.is_empty() {
    return Err(ApiError::BadRequest(
      "ordered_view_ids cannot be empty".to_string(),
    ));
  }
  // A duplicate id would leave one of the two at a stale position — reject
  // rather than silently keep only the last.
  let mut seen = std::collections::HashSet::new();
  for id in &payload.ordered_view_ids {
    if !seen.insert(*id) {
      return Err(ApiError::BadRequest(format!("duplicate view id {id}")));
    }
  }
  // Validate everything BEFORE writing anything: each id belongs to the
  // workspace, and re-parenting under `parent_view_id` neither escapes the
  // workspace nor forms a cycle (a view under its own descendant).
  for id in &payload.ordered_view_ids {
    ensure_view_in_workspace(&state.db, workspace_id, *id).await?;
    if let Some(parent) = payload.parent_view_id {
      ensure_valid_parent_view(&state.db, workspace_id, *id, parent).await?;
    }
  }

  let mut tx = state.db.begin().await?;
  for (i, id) in payload.ordered_view_ids.iter().enumerate() {
    let position = format!("{:010}", (i + 1) * 10);
    let affected = sqlx::query(
      r#"
        UPDATE views
        SET parent_view_id = $1, position = $2, updated_at = now()
        WHERE id = $3 AND workspace_id = $4 AND is_deleted = false
      "#,
    )
    .bind(payload.parent_view_id)
    .bind(&position)
    .bind(id)
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    // Validation ran before the tx, so a 0-row UPDATE means this view was
    // deleted concurrently in the window. Reporting `reordered: len()` regardless
    // (the old behavior) is the `ssh | tee` shape — a partial reorder claimed as
    // full success, leaving one node stuck at its old parent/position (P1-3).
    // Return the anomaly instead; the `?`/return drops `tx`, rolling everything
    // back so the client can refetch and retry against fresh state.
    if affected == 0 {
      return Err(ApiError::Conflict(format!(
        "view {id} was modified concurrently during reorder; refetch and retry"
      )));
    }
  }
  tx.commit().await?;

  Ok(Json(ReorderResponse {
    reordered: payload.ordered_view_ids.len(),
  }))
}

pub async fn bootstrap_document(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DocumentBootstrapResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let document = store::fetch_document(&state.db, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  let view = fetch_document_view(&state.db, workspace_id, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  let mut snapshot = store::latest_snapshot(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  // Serve LIVE content: for a doc edited via yrs sync the op-model snapshot is
  // frozen at the pre-yrs seed, so materialize the current blocks from the yrs
  // base — otherwise re-opening the page renders a near-blank stub.
  if let Some(payload) = store::current_payload(&state.db, document_id).await? {
    snapshot.payload =
      serde_json::to_value(&payload).map_err(|error| ApiError::Internal(error.to_string()))?;
  }

  Ok(Json(DocumentBootstrapResponse {
    document,
    view,
    snapshot,
  }))
}

pub async fn apply_document_update(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<ApplyDocumentUpdateRequest>,
) -> ApiResult<Json<DocumentUpdateResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  if payload.operations.is_empty() {
    return Err(ApiError::BadRequest(
      "at least one document operation is required".to_string(),
    ));
  }

  let applied = store::apply_document_operations(
    &state.db,
    workspace_id,
    document_id,
    user_id,
    &payload.operations,
  )
  .await?;

  // Reach any clients editing this document over WebSocket. A REST write has no
  // originating connection, so it is attributed to the nil connection id.
  ws::broadcast_applied_update(&state.hub, &applied, Uuid::nil(), None);

  Ok(Json(DocumentUpdateResponse {
    document: applied.document,
    snapshot: applied.snapshot,
    update: applied.update,
  }))
}

pub async fn export_document_markdown(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<MarkdownExportResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let assets = blob_asset_map(&payload.blocks, workspace_id);
  let markdown = export_markdown_with_assets(&payload, &assets)
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;

  Ok(Json(MarkdownExportResponse { markdown }))
}

#[derive(Debug, Serialize)]
pub struct OutlineHeading {
  block_id: String,
  level: i64,
  text: String,
}

#[derive(Debug, Serialize)]
pub struct DocumentOutlineResponse {
  /// Headings in document order — the anchors an AI names to write in place
  /// (`insert_at`/`find_replace`) instead of rewriting the whole doc.
  headings: Vec<OutlineHeading>,
  /// Every top-level block id in document order (finer anchors than headings).
  block_ids: Vec<String>,
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/outline`
///
/// The document's structure map (headings + block ids) so an AI can anchor a
/// local write rather than replace the whole page — the "get outline first,
/// then patch" loop the note-app MCP servers (Obsidian, Notion) converge on.
pub async fn document_outline(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<DocumentOutlineResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  let payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  Ok(Json(outline_from_payload(&payload)))
}

/// Pure: walk the block tree from the root in document order, collecting every
/// top-level block id and (for `heading` blocks) a heading entry. Testable
/// without a DB.
fn outline_from_payload(
  payload: &mica_app_core::documents::DocumentSnapshotPayload,
) -> DocumentOutlineResponse {
  let by_id: std::collections::HashMap<&str, &mica_app_core::documents::Block> =
    payload.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
  let mut headings = Vec::new();
  let mut block_ids = Vec::new();
  outline_walk(
    &payload.root_block_id,
    &by_id,
    &mut headings,
    &mut block_ids,
  );
  DocumentOutlineResponse {
    headings,
    block_ids,
  }
}

fn outline_walk(
  id: &str,
  by_id: &std::collections::HashMap<&str, &mica_app_core::documents::Block>,
  headings: &mut Vec<OutlineHeading>,
  block_ids: &mut Vec<String>,
) {
  let Some(block) = by_id.get(id) else {
    return;
  };
  // Only the root's DIRECT children — these are exactly the anchors `insert_at`
  // accepts (it resolves against root.children). Mica stores a flat block list
  // under the root, so this is also the whole body; don't recurse and advertise
  // deeper ids that insert_at would reject.
  for child_id in &block.children {
    if let Some(child) = by_id.get(child_id.as_str()) {
      block_ids.push(child.id.clone());
      if child.kind == "heading" {
        headings.push(OutlineHeading {
          block_id: child.id.clone(),
          level: child
            .data
            .get("level")
            .and_then(|v| v.as_i64())
            .unwrap_or(1),
          text: child.text.clone(),
        });
      }
    }
  }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownUpdateMode {
  /// Wipe the document body and write the markdown fresh.
  ReplaceAll,
  /// Add the markdown after the existing content (least conflict-prone).
  Append,
  /// Insert the markdown right after `anchor` (a top-level block id from the
  /// outline) — a local write that leaves the rest of the page untouched.
  InsertAt,
  /// Replace every occurrence of `find` with `replace` across the doc's block
  /// text (no `markdown`). Errors if nothing matches.
  FindReplace,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMarkdownRequest {
  pub mode: MarkdownUpdateMode,
  #[serde(default)]
  pub markdown: String,
  /// `insert_at`: the top-level block id to insert after.
  #[serde(default)]
  pub anchor: Option<String>,
  /// `find_replace`: the text to find / its replacement.
  #[serde(default)]
  pub find: Option<String>,
  #[serde(default)]
  pub replace: Option<String>,
}

/// `PATCH /api/workspaces/{workspace_id}/documents/{document_id}/markdown`
///
/// Write markdown into an EXISTING document (the AI-facing write path). Content
/// is markdown-in — the block/CRDT ops are derived server-side (reusing
/// `import_markdown` + the authoritative `apply_document_operations`), so callers
/// never construct raw ops. `append` is the safe default; `replace_all` wipes
/// first. (Anchored `insert_at`/`find_replace` land in M2.)
pub async fn update_document_markdown(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Json(request): Json<UpdateMarkdownRequest>,
) -> ApiResult<Json<DocumentUpdateResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  // Derive the ops from the snapshot INSIDE the write lock (no read-then-apply
  // TOCTOU: an anchor index / delete target can't drift under a concurrent edit).
  let applied =
    store::apply_derived_operations(&state.db, workspace_id, document_id, user_id, |payload| {
      markdown_update_ops(payload, &request, workspace_id)
    })
    .await?;
  ws::broadcast_applied_update(&state.hub, &applied, Uuid::nil(), None);

  Ok(Json(DocumentUpdateResponse {
    document: applied.document,
    snapshot: applied.snapshot,
    update: applied.update,
  }))
}

/// Pure: derive the block ops that write [markdown] into a doc whose current
/// state is [current]. `replace_all` first deletes the root's top-level children
/// (delete cascades their subtrees); then the parsed markdown tree is grafted
/// under the root — each block inserted with its `children` stripped and re-linked
/// via `parent_id`, in pre-order so parents exist first. Testable without a DB.
/// [workspace_id] scopes the blob hrefs this may rewire back into file
/// references — see [rewire_blob_hrefs].
fn markdown_update_ops(
  current: &mica_app_core::documents::DocumentSnapshotPayload,
  request: &UpdateMarkdownRequest,
  workspace_id: Uuid,
) -> Result<Vec<DocumentOperation>, String> {
  let root_id = current.root_block_id.as_str();

  // find_replace edits existing block text in place — no markdown parse/graft.
  if matches!(request.mode, MarkdownUpdateMode::FindReplace) {
    let find = request
      .find
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or("find_replace requires a non-empty `find`")?;
    let replace = request.replace.as_deref().unwrap_or("");
    let mut ops = Vec::new();
    let mut skipped_formatted = false;
    for block in &current.blocks {
      if block.id == root_id || !block.text.contains(find) {
        continue;
      }
      // A block's inline marks are UTF-16 offset ranges into its text. A blind
      // text replace would leave those offsets pointing at the wrong characters
      // — silently mangling bold/italic/link/math. Never touch a marked block;
      // steer the caller to replace_all/insert_at for formatted content.
      let has_marks = block
        .data
        .get("marks")
        .and_then(|m| m.as_array())
        .is_some_and(|a| !a.is_empty());
      if has_marks {
        skipped_formatted = true;
        continue;
      }
      ops.push(DocumentOperation::UpdateBlock {
        block_id: block.id.clone(),
        kind: None,
        text: Some(block.text.replace(find, replace)),
        data: None,
      });
    }
    if ops.is_empty() {
      return Err(if skipped_formatted {
        format!("{find:?} appears only in formatted text; use replace_all or insert_at instead")
      } else {
        format!("no block text contains {find:?}")
      });
    }
    return Ok(ops);
  }

  // Parse the incoming markdown up front so an empty body is rejected BEFORE any
  // destructive delete (a replace_all with empty markdown must not wipe the doc).
  let tmp_root = format!("block_{}", Uuid::new_v4().simple());
  let mut parsed = import_markdown(&request.markdown, &tmp_root);
  rewire_blob_hrefs(&mut parsed.blocks, workspace_id);
  let has_content = parsed
    .blocks
    .iter()
    .find(|b| b.id == tmp_root)
    .is_some_and(|r| !r.children.is_empty());

  let mut ops = Vec::new();
  // Where the new content grafts under the root: append (None), replace_all
  // (None, after wiping), or insert_at (right after the anchor).
  let start_index = match request.mode {
    MarkdownUpdateMode::ReplaceAll => {
      if !has_content {
        return Err(
          "replace_all needs markdown content — refusing to wipe the document".to_string(),
        );
      }
      if let Some(root) = current.blocks.iter().find(|b| b.id == root_id) {
        for child in &root.children {
          ops.push(DocumentOperation::DeleteBlock {
            block_id: child.clone(),
          });
        }
      }
      None
    }
    MarkdownUpdateMode::Append => None,
    MarkdownUpdateMode::InsertAt => {
      let anchor = request
        .anchor
        .as_deref()
        .ok_or("insert_at requires an `anchor` block id")?;
      let root = current
        .blocks
        .iter()
        .find(|b| b.id == root_id)
        .ok_or("document has no root block")?;
      let pos = root
        .children
        .iter()
        .position(|c| c == anchor)
        .ok_or_else(|| format!("anchor {anchor:?} is not a top-level block"))?;
      Some(pos + 1)
    }
    MarkdownUpdateMode::FindReplace => unreachable!("handled above"),
  };

  let by_id: std::collections::HashMap<&str, &mica_app_core::documents::Block> =
    parsed.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
  graft_ops(&tmp_root, root_id, start_index, &by_id, &mut ops);
  Ok(ops)
}

/// Emit InsertBlock ops for every child of [parsed_parent], re-parenting them
/// under [op_parent] (children stripped; re-linked by insertion order). Top-level
/// blocks land at [start_index] (incrementing) so `insert_at` positions after an
/// anchor; `None` appends. Descendants recurse appended under the real block id.
fn graft_ops(
  parsed_parent: &str,
  op_parent: &str,
  start_index: Option<usize>,
  by_id: &std::collections::HashMap<&str, &mica_app_core::documents::Block>,
  ops: &mut Vec<DocumentOperation>,
) {
  let Some(parent) = by_id.get(parsed_parent) else {
    return;
  };
  let mut index = start_index;
  for child_id in &parent.children {
    let Some(child) = by_id.get(child_id.as_str()) else {
      continue;
    };
    let mut block = (*child).clone();
    block.children = Vec::new();
    ops.push(DocumentOperation::InsertBlock {
      block,
      parent_id: op_parent.to_string(),
      index,
    });
    graft_ops(&child.id, &child.id, None, by_id, ops);
    index = index.map(|i| i + 1);
  }
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/export.zip`
///
/// A portable ZIP: `document.md` with Mica images rewritten to `assets/<name>`
/// plus the image bytes under `assets/` (external image links are kept as-is).
pub async fn export_document_zip(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  let payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  // The page's name rides on the FILE NAME — it is nowhere in the text (the body
  // is exported verbatim), so hardcoding `document.md` silently threw the name
  // away: you got a zip of "document.md" whichever page you exported, and an
  // import could only ever call it "document".
  let base = fetch_document_view(&state.db, workspace_id, document_id)
    .await?
    .map(|view| safe_segment(&view.name))
    .unwrap_or_else(|| "document".to_string());

  let mut entries = Vec::new();
  let assets = collect_assets(&state, workspace_id, &payload.blocks, &mut entries).await?;
  let markdown = export_markdown_with_assets(&payload, &assets)
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;
  entries.insert(
    0,
    ZipEntry {
      name: format!("{base}.md"),
      data: markdown.into_bytes(),
    },
  );

  Ok(zip_response(build_zip(&entries), &format!("{base}.zip")))
}

/// Gather image assets referenced by [blocks]: fetch each Mica image's bytes,
/// push them into [entries] under `assets/`, and return a `file_id → assets/path`
/// map for the Markdown rewrite. External (url) images are left untouched.
async fn collect_assets(
  state: &AppState,
  workspace_id: Uuid,
  blocks: &[mica_app_core::documents::Block],
  entries: &mut Vec<ZipEntry>,
) -> ApiResult<BTreeMap<String, String>> {
  // file_id -> original name, in document order.
  let mut wanted: Vec<(String, String)> = Vec::new();
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    let file_id = block.data.get("file_id").and_then(|v| v.as_str());
    if let Some(id) = file_id {
      if !wanted.iter().any(|(w, _)| w == id) {
        let name = block
          .data
          .get("name")
          .and_then(|v| v.as_str())
          .unwrap_or("image")
          .to_string();
        wanted.push((id.to_string(), name));
      }
    }
  }
  if wanted.is_empty() {
    return Ok(BTreeMap::new());
  }

  let storage = state
    .storage
    .clone()
    .ok_or_else(|| ApiError::Unavailable("file storage is not configured".to_string()))?;
  let ids: Vec<Uuid> = wanted
    .iter()
    .filter_map(|(id, _)| Uuid::parse_str(id).ok())
    .collect();
  let records = store::fetch_files(&state.db, workspace_id, &ids).await?;
  let by_id: std::collections::HashMap<String, &store::FileRecord> =
    records.iter().map(|r| (r.id.to_string(), r)).collect();

  let client = reqwest::Client::new();
  let mut map = BTreeMap::new();
  let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
  for (file_id, name) in wanted {
    let Some(record) = by_id.get(&file_id) else {
      continue;
    };
    let bytes = match client
      .get(storage.download_url(&record.object_key))
      .send()
      .await
    {
      Ok(resp) if resp.status().is_success() => match resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(_) => continue,
      },
      _ => continue,
    };
    let asset = unique_asset_name(&name, &mut used);
    entries.push(ZipEntry {
      name: format!("assets/{asset}"),
      data: bytes,
    });
    map.insert(file_id, format!("assets/{asset}"));
  }
  Ok(map)
}

/// Make a unique `assets/` filename, appending `-1`, `-2`… on collision.
fn unique_asset_name(name: &str, used: &mut std::collections::HashSet<String>) -> String {
  if used.insert(name.to_string()) {
    return name.to_string();
  }
  let (stem, ext) = match name.rsplit_once('.') {
    Some((s, e)) => (s.to_string(), format!(".{e}")),
    None => (name.to_string(), String::new()),
  };
  let mut n = 1;
  loop {
    let candidate = format!("{stem}-{n}{ext}");
    if used.insert(candidate.clone()) {
      return candidate;
    }
    n += 1;
  }
}

/// `GET /api/workspaces/{workspace_id}/export.zip`
///
/// The whole workspace as a Markdown ZIP, organised by the page tree: each page
/// is `<ancestors…>/<page>.md`, a page with children also names a folder, and
/// images are de-duplicated under a root `assets/` folder (referenced with the
/// right relative `../` depth).
pub async fn export_workspace_zip(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  let entries = build_tree_zip(&state, workspace_id, None).await?;
  Ok(zip_response(build_zip(&entries), "workspace.zip"))
}

/// `GET /api/workspaces/export.zip` — EVERY workspace the user belongs to, in
/// switcher (position) order, each under its own `<name>/` subdir, plus a
/// top-level `workspaces.json` manifest. Each subdir is byte-identical to that
/// workspace's own `export.zip`, so re-importing a subdir round-trips.
pub async fn export_all_workspaces_zip(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let workspaces: Vec<(Uuid, String)> = sqlx::query_as(
    r#"
      SELECT w.id, w.name
      FROM workspaces w
      INNER JOIN workspace_members wm ON wm.workspace_id = w.id
      WHERE wm.user_id = $1
      ORDER BY wm.position ASC, w.created_at ASC
    "#,
  )
  .bind(user_id)
  .fetch_all(&state.db)
  .await?;

  let mut entries: Vec<ZipEntry> = Vec::new();
  let mut manifest_ws: Vec<serde_json::Value> = Vec::new();
  let mut used_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();
  for (ws_id, ws_name) in &workspaces {
    // Unique subdir per workspace (two "test" workspaces must not collide).
    let base = zip_safe_name(ws_name, "workspace");
    let mut dir = base.clone();
    let mut n = 2;
    // Dedup case-insensitively so "Test" and "test" don't both unzip into the
    // same folder on a case-insensitive filesystem (Windows / default macOS).
    while !used_dirs.insert(dir.to_lowercase()) {
      dir = format!("{base} ({n})");
      n += 1;
    }
    for e in build_tree_zip(&state, *ws_id, None).await? {
      entries.push(ZipEntry {
        name: format!("{dir}/{}", e.name),
        data: e.data,
      });
    }
    manifest_ws.push(serde_json::json!({ "name": ws_name, "dir": dir }));
  }
  let manifest = serde_json::json!({
    "version": 1,
    "generator": "mica",
    "kind": "workspaces",
    "workspaces": manifest_ws,
  });
  entries.push(ZipEntry {
    name: "workspaces.json".to_string(),
    data: serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
  });
  Ok(zip_response(build_zip(&entries), "mica-workspaces.zip"))
}

/// `GET /api/workspaces/{workspace_id}/views/{view_id}/export.zip`
///
/// One folder's subtree as an archive — same shape as the workspace export
/// (paths relative to the folder, shared `assets/`, `manifest.json`), so it
/// imports back the same way. Every export level is a zip on purpose: a bare
/// `.md` cannot carry the images, and used to emit `![](photo.png)` pointing at
/// a file that was nowhere in the download.
pub async fn export_folder_zip(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let views = fetch_workspace_views(&state.db, workspace_id).await?;
  let folder = views
    .iter()
    .find(|v| v.id == view_id)
    .ok_or(ApiError::NotFound)?;
  if folder.object_type != "folder" {
    return Err(ApiError::BadRequest(
      "only a folder can be exported this way".to_string(),
    ));
  }
  let filename = format!("{}.zip", zip_safe_name(&folder.name, "folder"));
  let entries = build_tree_zip(&state, workspace_id, Some(view_id)).await?;
  Ok(zip_response(build_zip(&entries), &filename))
}

/// A filename-safe rendition of a user-authored name, for the download's
/// `Content-Disposition` (path separators / control chars must not escape it).
fn zip_safe_name(name: &str, fallback: &str) -> String {
  let cleaned: String = name
    .chars()
    .map(|c| {
      if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
        '-'
      } else {
        c
      }
    })
    .collect();
  let cleaned = cleaned.trim().trim_matches('.').trim();
  if cleaned.is_empty() {
    fallback.to_string()
  } else {
    cleaned.chars().take(80).collect()
  }
}

/// Build the archive entries for a subtree: [root] `None` = the whole
/// workspace, `Some(view_id)` = that folder's children. Paths are relative to
/// the root, image assets are de-duplicated into a shared `assets/`, and a
/// `manifest.json` records the tree for round-tripping back through import.
async fn build_tree_zip(
  state: &AppState,
  workspace_id: Uuid,
  root: Option<Uuid>,
) -> ApiResult<Vec<ZipEntry>> {
  let views = fetch_workspace_views(&state.db, workspace_id).await?;
  // Store-neutral tree for the shared builder (mica_interchange::export_tree) —
  // the SAME walk the local (SQLite) export feeds, so cloud + local produce
  // identically-structured archives and export→import stays one round-trip
  // invariant (see export_tree.rs on the cosmetic asset-name tiebreak).
  let nodes: Vec<mica_interchange::TreeNode> = views
    .iter()
    .map(|v| mica_interchange::TreeNode {
      id: v.id.to_string(),
      parent_id: v.parent_view_id.map(|p| p.to_string()),
      position: v.position.clone(),
      name: v.name.clone(),
      object_type: v.object_type.clone(),
      object_id: v.object_id.to_string(),
    })
    .collect();

  // Each document's current payload, keyed by object_id.
  let mut payloads: std::collections::HashMap<String, DocumentSnapshotPayload> =
    std::collections::HashMap::new();
  for v in &views {
    if v.object_type != "document" {
      continue;
    }
    if let Some(p) = store::current_payload(&state.db, v.object_id).await? {
      payloads.insert(v.object_id.to_string(), p);
    }
  }

  // Referenced image blobs: file_id -> (first-seen block name), then fetch the
  // records and download the bytes. Dedup key = the storage object key, so two
  // file_ids of the same blob share one `assets/` entry (as the old walk did).
  let mut file_name: BTreeMap<String, String> = BTreeMap::new();
  for v in &views {
    if v.object_type != "document" {
      continue;
    }
    let Some(payload) = payloads.get(&v.object_id.to_string()) else {
      continue;
    };
    for b in &payload.blocks {
      if b.kind != "image" {
        continue;
      }
      if let Some(id) = b.data.get("file_id").and_then(|x| x.as_str()) {
        file_name.entry(id.to_string()).or_insert_with(|| {
          b.data
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("image")
            .to_string()
        });
      }
    }
  }
  let mut images: std::collections::HashMap<String, mica_interchange::ImageAsset> =
    std::collections::HashMap::new();
  if let Some(storage) = state.storage.clone() {
    if !file_name.is_empty() {
      let ids: Vec<Uuid> = file_name
        .keys()
        .filter_map(|id| Uuid::parse_str(id).ok())
        .collect();
      let records = store::fetch_files(&state.db, workspace_id, &ids).await?;
      let by_id: std::collections::HashMap<String, &store::FileRecord> =
        records.iter().map(|r| (r.id.to_string(), r)).collect();
      let client = reqwest::Client::new();
      for (file_id, name) in &file_name {
        let Some(record) = by_id.get(file_id) else {
          continue;
        };
        let bytes = match client
          .get(storage.download_url(&record.object_key))
          .send()
          .await
        {
          Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(_) => continue,
          },
          _ => continue,
        };
        images.insert(
          file_id.clone(),
          mica_interchange::ImageAsset {
            name: name.clone(),
            bytes,
            dedup_key: record.object_key.clone(),
          },
        );
      }
    }
  }

  let root_str = root.map(|r| r.to_string());
  Ok(mica_interchange::build_markdown_tree_zip(
    &nodes,
    root_str.as_deref(),
    &payloads,
    &images,
  ))
}

/// The canonical, workspace-scoped path that serves one blob's bytes.
///
/// The trailing name is cosmetic — `files::blob_named` ignores it — but it
/// keeps the exported Markdown readable and survives a re-import (see
/// [parse_blob_href]).
fn blob_href(workspace_id: Uuid, file_id: &str, name: &str) -> String {
  format!(
    "/api/workspaces/{workspace_id}/files/{file_id}/blob/{}",
    safe_segment(name)
  )
}

/// `file_id -> a path that actually serves the bytes`, for the Markdown exports
/// that ship no bytes of their own.
///
/// With no map an uploaded image degrades to its ORIGINAL FILENAME, and every
/// client names a pasted image `pasted-image.png` — so a reader got
/// `![](pasted-image.png)` for every image in the workspace: unresolvable, and
/// not even distinguishable from one another. The file_id is already in the
/// block; spending it on a href costs no query and makes the export fetchable.
/// The ZIP exports keep their own `assets/` map — bytes travel with those.
fn blob_asset_map(
  blocks: &[mica_app_core::documents::Block],
  workspace_id: Uuid,
) -> BTreeMap<String, String> {
  let mut map = BTreeMap::new();
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    let Some(file_id) = block.data.get("file_id").and_then(|v| v.as_str()) else {
      continue;
    };
    let name = block
      .data
      .get("name")
      .and_then(|v| v.as_str())
      .unwrap_or("image");
    map.insert(file_id.to_string(), blob_href(workspace_id, file_id, name));
  }
  map
}

/// Recognise one of our own blob hrefs and recover `(file_id, name)`.
///
/// Only paths for THIS workspace resolve. A href aimed at another workspace's
/// blob is not ours to claim: blob GC recomputes each workspace's reference set
/// from its OWN views, so a cross-workspace reference is invisible to the GC
/// that owns the bytes — it would collect them and break this page. Left as a
/// plain link, it stays exactly as honest as it is: a link to someone else's
/// file.
fn parse_blob_href(href: &str, workspace_id: Uuid) -> Option<(String, String)> {
  let path = href.split(['?', '#']).next()?;
  let at = path.find("/api/workspaces/")?;
  let rest = &path[at + "/api/workspaces/".len()..];
  let (ws, rest) = rest.split_once('/')?;
  if ws != workspace_id.to_string() {
    return None;
  }
  let rest = rest.strip_prefix("files/")?;
  let (file_id, rest) = rest.split_once('/')?;
  Uuid::parse_str(file_id).ok()?;
  let name = match rest {
    "blob" => String::new(),
    other => percent_decode(other.strip_prefix("blob/")?),
  };
  let name = if name.is_empty() {
    "image".to_string()
  } else {
    name
  };
  Some((file_id.to_string(), name))
}

/// Turn `![](…/files/{id}/blob/…)` back into Mica's `{file_id, name}` form.
///
/// Symmetric with [blob_asset_map]. Without it a Markdown round-trip would
/// quietly downgrade every uploaded image into an external link pointing at
/// itself — still rendering, but no longer a reference, so blob GC would stop
/// counting it and eventually delete the bytes out from under the page.
fn rewire_blob_hrefs(blocks: &mut [mica_app_core::documents::Block], workspace_id: Uuid) {
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    let Some(url) = block.data.get("url").and_then(|v| v.as_str()) else {
      continue;
    };
    if let Some((file_id, name)) = parse_blob_href(url, workspace_id) {
      block.data = serde_json::json!({"file_id": file_id, "name": name});
    }
  }
}

/// Decode `%XX` escapes back to a UTF-8 string; leave malformed escapes alone.
/// In-house rather than a dependency: this is the only percent-decode in the
/// server, and it is a dozen lines.
fn percent_decode(input: &str) -> String {
  let bytes = input.as_bytes();
  let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'%'
      && i + 2 < bytes.len()
      && let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16)
    {
      out.push(byte);
      i += 3;
    } else {
      out.push(bytes[i]);
      i += 1;
    }
  }
  String::from_utf8_lossy(&out).into_owned()
}

/// A path segment safe for a filename: keep letters/digits of any script plus
/// `-_.`, collapse other runs to `_`; never empty.
fn safe_segment(name: &str) -> String {
  let mut out = String::new();
  let mut prev_us = false;
  for ch in name.chars() {
    if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
      out.push(ch);
      prev_us = ch == '_';
    } else if !prev_us {
      out.push('_');
      prev_us = true;
    }
  }
  let tidy = out.trim_matches('_').to_string();
  if tidy.is_empty() {
    "untitled".to_string()
  } else {
    tidy
  }
}

fn zip_response(bytes: Vec<u8>, filename: &str) -> Response {
  (
    [
      (header::CONTENT_TYPE, "application/zip".to_string()),
      (
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\""),
      ),
    ],
    bytes,
  )
    .into_response()
}

/// `GET /api/workspaces/{workspace_id}/export/markdown` — the whole workspace as
/// one clean Markdown document, pages in tree order (title heading depth follows
/// the page tree).
pub async fn export_workspace_markdown(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<MarkdownExportResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  let markdown = workspace_markdown(&state.db, workspace_id, 1).await?;
  Ok(Json(MarkdownExportResponse { markdown }))
}

/// `GET /api/export/markdown` — every workspace the user belongs to, each as a
/// top-level section, concatenated into one Markdown document.
pub async fn export_all_markdown(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<MarkdownExportResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let workspaces = sqlx::query_as::<_, (Uuid, String)>(
    r#"
      SELECT w.id, w.name
      FROM workspaces w
      JOIN workspace_members m ON m.workspace_id = w.id
      WHERE m.user_id = $1
      ORDER BY w.created_at ASC
    "#,
  )
  .bind(user_id)
  .fetch_all(&state.db)
  .await?;

  let mut out = String::new();
  for (id, name) in workspaces {
    out.push_str(&format!("# {name}\n\n"));
    let body = workspace_markdown(&state.db, id, 2).await?;
    if !body.is_empty() {
      out.push_str(&body);
      out.push_str("\n\n");
    }
    out.push_str("---\n\n");
  }

  Ok(Json(MarkdownExportResponse {
    markdown: out.trim().to_string(),
  }))
}

/// Render every document page of a workspace into one Markdown string, in
/// page-tree order. `base_level` is the heading level of top-level pages.
async fn workspace_markdown(
  db: &PgPool,
  workspace_id: Uuid,
  base_level: usize,
) -> ApiResult<String> {
  let views = fetch_workspace_views(db, workspace_id).await?;

  let mut by_parent: std::collections::HashMap<Option<Uuid>, Vec<&View>> =
    std::collections::HashMap::new();
  for view in &views {
    by_parent.entry(view.parent_view_id).or_default().push(view);
  }

  let mut ordered: Vec<(&View, usize)> = Vec::new();
  collect_view_order(&by_parent, None, 0, &mut ordered);

  let mut out = String::new();
  for (view, depth) in ordered {
    if view.object_type != "document" {
      continue;
    }
    let level = (base_level + depth).min(6);
    out.push_str(&"#".repeat(level));
    out.push(' ');
    out.push_str(&view.name);
    out.push_str("\n\n");

    if let Some(payload) = store::current_payload(db, view.object_id).await? {
      let assets = blob_asset_map(&payload.blocks, workspace_id);
      // Propagate a page's export failure instead of swallowing it (the old
      // `if let Ok(..)` with no else). This export is a backup/migration; a page
      // that silently reduces to its heading with the body gone is a backup that
      // LOOKS complete and isn't — the incident B shape. Matches the single-doc
      // export path (`export_markdown`) which already `?`s this. (P1-3.)
      let markdown = export_markdown_with_assets(&payload, &assets)
        .map_err(|error| ApiError::BadRequest(format!("export failed for page {}: {error}", view.name)))?;
      let body = markdown.trim();
      if !body.is_empty() {
        out.push_str(body);
        out.push_str("\n\n");
      }
    }
  }

  Ok(out.trim().to_string())
}

fn collect_view_order<'a>(
  by_parent: &std::collections::HashMap<Option<Uuid>, Vec<&'a View>>,
  parent: Option<Uuid>,
  depth: usize,
  out: &mut Vec<(&'a View, usize)>,
) {
  if let Some(children) = by_parent.get(&parent) {
    for child in children {
      out.push((child, depth));
      collect_view_order(by_parent, Some(child.id), depth + 1, out);
    }
  }
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/export/html`
///
/// One page as a **self-contained** `.html` file: a full HTML5 document with an
/// embedded stylesheet and every image inlined as a `data:` URI, so it opens
/// offline and survives the source page being deleted. This is why it embeds
/// bytes rather than reusing the share page's public blob URLs — a downloaded
/// file must not depend on the server still being up.
#[derive(Debug, Deserialize)]
pub struct HtmlExportQuery {
  /// The author's editor page width in px, so the export is as wide as the doc
  /// was written (WYSIWYG). Absent → a sensible default.
  width: Option<u32>,
}

pub async fn export_document_html(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Query(q): Query<HtmlExportQuery>,
) -> ApiResult<Response> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  let mut payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let title = fetch_document_view(&state.db, workspace_id, document_id)
    .await?
    .map(|view| view.name)
    .unwrap_or_else(|| "document".to_string());

  // Bytes → data: URIs. An image whose bytes can't be fetched simply keeps its
  // existing url (set_image_srcs skips it), so a missing asset degrades to a
  // broken <img>, never a failed export.
  let data_uris = collect_asset_data_uris(&state, workspace_id, &payload.blocks).await?;
  set_image_srcs(&mut payload, &data_uris);

  let html = export_html_document(&payload, &title, q.width.unwrap_or(1160))
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;

  let filename = format!("{}.html", safe_segment(&title));
  Ok(
    (
      [
        (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
        (
          header::CONTENT_DISPOSITION,
          format!("attachment; filename=\"{filename}\""),
        ),
      ],
      html,
    )
      .into_response(),
  )
}

/// Like [collect_assets] but for a single self-contained file: fetch each Mica
/// image's bytes and return a `file_id → data:<mime>;base64,…` map instead of
/// writing files. External (url) images are left for [set_image_srcs] to skip.
async fn collect_asset_data_uris(
  state: &AppState,
  workspace_id: Uuid,
  blocks: &[mica_app_core::documents::Block],
) -> ApiResult<BTreeMap<String, String>> {
  let mut wanted: Vec<String> = Vec::new();
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    if let Some(id) = block.data.get("file_id").and_then(|v| v.as_str()) {
      if !wanted.iter().any(|w| w == id) {
        wanted.push(id.to_string());
      }
    }
  }
  if wanted.is_empty() {
    return Ok(BTreeMap::new());
  }

  let storage = state
    .storage
    .clone()
    .ok_or_else(|| ApiError::Unavailable("file storage is not configured".to_string()))?;
  let ids: Vec<Uuid> = wanted.iter().filter_map(|id| Uuid::parse_str(id).ok()).collect();
  let records = store::fetch_files(&state.db, workspace_id, &ids).await?;
  let by_id: std::collections::HashMap<String, &store::FileRecord> =
    records.iter().map(|r| (r.id.to_string(), r)).collect();

  let client = reqwest::Client::new();
  let mut map = BTreeMap::new();
  for file_id in wanted {
    let Some(record) = by_id.get(&file_id) else {
      continue;
    };
    let bytes = match client.get(storage.download_url(&record.object_key)).send().await {
      Ok(resp) if resp.status().is_success() => match resp.bytes().await {
        Ok(b) => b.to_vec(),
        Err(_) => continue,
      },
      _ => continue,
    };
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    map.insert(file_id, format!("data:{};base64,{}", record.mime_type, b64));
  }
  Ok(map)
}

/// Point each uploaded-image block's `url` at its public blob path so a renderer
/// that reads `url` (export_html) can show it. Uploaded images store
/// `{file_id, name}` and NO `url`, so without this they render `<img src="">`.
/// External images (already a `url`) are left alone. The blob path is a public
/// capability URL (`is_blob_path`), so it resolves on an unauthenticated share
/// page too.
fn inline_blob_hrefs(blocks: &mut [mica_app_core::documents::Block], workspace_id: Uuid) {
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    let Some(file_id) = block
      .data
      .get("file_id")
      .and_then(|v| v.as_str())
      .map(str::to_string)
    else {
      continue;
    };
    let name = block
      .data
      .get("name")
      .and_then(|v| v.as_str())
      .unwrap_or("image")
      .to_string();
    if let Some(obj) = block.data.as_object_mut() {
      obj.insert(
        "url".to_string(),
        serde_json::json!(blob_href(workspace_id, &file_id, &name)),
      );
    }
  }
}

// ── Public sharing (publish a page to a /s/{token} URL) ──────────────────────

#[derive(Debug, Serialize)]
pub struct ShareResponse {
  /// Whether the document currently has an active public link.
  shared: bool,
  /// The share token when shared; the client composes `{origin}/s/{token}`.
  #[serde(skip_serializing_if = "Option::is_none")]
  token: Option<String>,
}

/// `GET /api/workspaces/{ws}/documents/{id}/share` — current share status.
pub async fn get_share(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ShareResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let share = store::fetch_active_share_for_doc(&state.db, workspace_id, document_id).await?;
  Ok(Json(ShareResponse {
    shared: share.is_some(),
    token: share.map(|s| s.token),
  }))
}

/// `POST …/share` — publish (create-or-return the active share). Idempotent:
/// re-publishing returns the SAME link.
pub async fn create_share(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ShareResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let share = store::create_or_get_share(&state.db, workspace_id, document_id, user_id).await?;
  Ok(Json(ShareResponse {
    shared: true,
    token: Some(share.token),
  }))
}

/// `DELETE …/share` — unpublish (soft-revoke). The public link 404s at once.
pub async fn delete_share(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ShareResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  store::revoke_share(&state.db, workspace_id, document_id).await?;
  Ok(Json(ShareResponse {
    shared: false,
    token: None,
  }))
}

/// `GET /s/{token}` — the PUBLIC read-only page. No auth: the unguessable token
/// is the only credential, and it is re-checked (`revoked_at IS NULL`) on every
/// hit so revocation is instant. A missing/revoked/wrong token and a private
/// doc all return the SAME 404 — the page never reveals that a document exists.
/// Renders the server-side HTML (Rust-first data-plane) wrapped in a minimal
/// shell; images resolve through the public blob capability URLs.
pub async fn public_share_page(
  State(state): State<AppState>,
  Path(token): Path<String>,
) -> Response {
  let not_found = || {
    (
      axum::http::StatusCode::NOT_FOUND,
      [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
      "<!doctype html><meta charset=utf-8><title>Not found</title><p>This page is not available.</p>",
    )
      .into_response()
  };

  let Ok(Some(share)) = store::fetch_share_by_token(&state.db, &token).await else {
    return not_found();
  };
  // The share row alone is not enough: a token can outlive its document. If the
  // view was trashed (`is_deleted`) or purged, `fetch_document_view` (which
  // filters `is_deleted = false`) returns `None`, and we must 404 rather than
  // keep serving the still-present `current_payload` of deleted content.
  let Ok(Some(view)) = fetch_document_view(&state.db, share.workspace_id, share.document_id).await
  else {
    return not_found();
  };
  let Ok(Some(mut payload)) = store::current_payload(&state.db, share.document_id).await else {
    return not_found();
  };
  inline_blob_hrefs(&mut payload.blocks, share.workspace_id);
  let Ok(body) = export_html(&payload) else {
    return not_found();
  };
  let title = view.name;

  let html = render_share_shell(&title, &body, share.allow_indexing);
  (
    [
      (header::CONTENT_TYPE, "text/html; charset=utf-8"),
      // The share page is server-rendered static HTML that needs no JS. A
      // strict CSP with no `script-src` falls back to `default-src 'none'`,
      // which neutralizes inline `<script>` AND `on*` event handlers (e.g.
      // `<img onerror>`) that survive the GFM tagfilter in raw-HTML content —
      // the storage-XSS -> token-theft vector. `img-src`/`font-src`/`style-src`
      // match what `render_share_shell` actually uses (inline CSS, blob/data
      // images, system fonts).
      (header::CONTENT_SECURITY_POLICY, SHARE_CSP),
    ],
    html,
  )
    .into_response()
}

/// CSP for the public share page. No `script-src` -> inline scripts and `on*`
/// event handlers are blocked via the `default-src 'none'` fallback.
const SHARE_CSP: &str =
  "default-src 'none'; img-src 'self' data: https:; style-src 'self' 'unsafe-inline'; font-src 'self' data:";

/// Wrap the HTML export fragment in a standalone, readable page. `noindex`
/// unless the share opted into indexing, so a shared page is not silently
/// crawlable.
fn render_share_shell(title: &str, body_html: &str, allow_indexing: bool) -> String {
  let safe_title = escape_html_min(title);
  let robots = if allow_indexing {
    ""
  } else {
    "<meta name=\"robots\" content=\"noindex\">"
  };
  format!(
    "<!doctype html>\n<html lang=\"zh\">\n<head>\n<meta charset=\"utf-8\">\n\
     <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
     {robots}\n<title>{safe_title}</title>\n<style>\n\
     body{{max-width:1160px;margin:2.5rem auto;padding:0 1.5rem;\
     font-family:-apple-system,BlinkMacSystemFont,'Segoe UI','Microsoft YaHei',\
     'PingFang SC',sans-serif;line-height:1.7;color:#1f2328;}}\n\
     img{{max-width:100%;height:auto;border-radius:6px;}}\n\
     pre{{background:#f6f8fa;padding:1rem;border-radius:6px;overflow:auto;}}\n\
     code{{background:#f6f8fa;padding:.15em .35em;border-radius:4px;}}\n\
     pre code{{background:none;padding:0;}}\n\
     blockquote{{margin:0;padding-left:1rem;border-left:3px solid #d0d7de;color:#57606a;}}\n\
     table{{width:100%;border-collapse:collapse;}}\n\
     td,th{{border:1px solid #d0d7de;padding:.4em .6em;}}\n\
     hr{{border:none;border-top:1px solid #d0d7de;margin:2rem 0;}}\n\
     h1{{margin-bottom:1.5rem;}}\n\
     .mica-footer{{margin-top:3rem;padding-top:1rem;border-top:1px solid #eaeef2;\
     color:#8c959f;font-size:.85rem;}}\n</style>\n</head>\n<body>\n\
     <h1>{safe_title}</h1>\n{body_html}\n\
     <div class=\"mica-footer\">用 Mica 制作</div>\n</body>\n</html>\n"
  )
}

/// Minimal HTML-text escaping for the title (the body is escaped by export_html).
fn escape_html_min(s: &str) -> String {
  s.replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
}

async fn fetch_workspace_views(db: &PgPool, workspace_id: Uuid) -> ApiResult<Vec<View>> {
  sqlx::query_as::<_, View>(
    r#"
      SELECT
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
      FROM views
      WHERE workspace_id = $1 AND is_deleted = false
      ORDER BY parent_view_id NULLS FIRST, position ASC
    "#,
  )
  .bind(workspace_id)
  .fetch_all(db)
  .await
  .map_err(ApiError::from)
}

async fn fetch_deleted_workspace_views(db: &PgPool, workspace_id: Uuid) -> ApiResult<Vec<View>> {
  sqlx::query_as::<_, View>(
    r#"
      SELECT
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
      FROM views
      WHERE workspace_id = $1 AND is_deleted = true
      ORDER BY updated_at DESC
    "#,
  )
  .bind(workspace_id)
  .fetch_all(db)
  .await
  .map_err(ApiError::from)
}

async fn fetch_document_view(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
) -> ApiResult<Option<View>> {
  sqlx::query_as::<_, View>(
    r#"
      SELECT
        id,
        workspace_id,
        parent_view_id,
        object_id,
        object_type::text AS object_type,
        name,
        icon,
        position,
        is_deleted,
        created_by,
        created_at,
        updated_at
      FROM views
      WHERE workspace_id = $1 AND object_id = $2 AND object_type = 'document' AND is_deleted = false
      LIMIT 1
    "#,
  )
  .bind(workspace_id)
  .bind(document_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

async fn ensure_document_in_workspace(
  db: &PgPool,
  workspace_id: Uuid,
  document_id: Uuid,
) -> ApiResult<()> {
  let exists = sqlx::query_scalar::<_, bool>(
    r#"
      SELECT EXISTS (
        SELECT 1
        FROM documents
        WHERE id = $1 AND workspace_id = $2
      )
    "#,
  )
  .bind(document_id)
  .bind(workspace_id)
  .fetch_one(db)
  .await?;

  if !exists {
    return Err(ApiError::NotFound);
  }

  Ok(())
}

/// Read/write/comment capabilities derived from a workspace role. Surfaced to
/// clients on bootstrap so the editor can enable or disable mutating actions.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DocumentPermissions {
  pub can_read: bool,
  pub can_write: bool,
  pub can_comment: bool,
}

pub(crate) fn permissions_for_role(role: &str) -> DocumentPermissions {
  let can_write = matches!(role, "owner" | "admin" | "editor");
  let can_comment = can_write || role == "commenter";
  DocumentPermissions {
    can_read: true,
    can_write,
    can_comment,
  }
}

pub(crate) async fn workspace_role(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<Option<String>> {
  sqlx::query_scalar::<_, String>(
    r#"
      SELECT role::text
      FROM workspace_members
      WHERE workspace_id = $1 AND user_id = $2
    "#,
  )
  .bind(workspace_id)
  .bind(user_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

pub(crate) async fn ensure_workspace_member(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<()> {
  workspace_role(db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  Ok(())
}

pub(crate) async fn ensure_workspace_editor(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<()> {
  let role = workspace_role(db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  if !matches!(role.as_str(), "owner" | "admin" | "editor") {
    return Err(ApiError::Forbidden);
  }

  Ok(())
}

async fn ensure_view_in_workspace(db: &PgPool, workspace_id: Uuid, view_id: Uuid) -> ApiResult<()> {
  let exists = sqlx::query_scalar::<_, bool>(
    r#"
      SELECT EXISTS (
        SELECT 1
        FROM views
        WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
      )
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .fetch_one(db)
  .await?;

  if !exists {
    return Err(ApiError::NotFound);
  }

  Ok(())
}

/// A parent must be a live FOLDER in this workspace. Pages are leaves — only
/// folders contain (see `migrations/0011_pages_are_leaves.sql`, which repairs
/// the trees that predate this and enforces the same rule at the DB as a
/// backstop). Every path that sets `parent_view_id` goes through here so the
/// caller gets a 400 with a reason instead of tripping the trigger's 500.
pub(crate) async fn ensure_parent_accepts_children(
  db: &PgPool,
  workspace_id: Uuid,
  parent_view_id: Uuid,
) -> ApiResult<()> {
  let object_type = sqlx::query_scalar::<_, String>(
    r#"
      SELECT object_type::text
      FROM views
      WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
    "#,
  )
  .bind(parent_view_id)
  .bind(workspace_id)
  .fetch_optional(db)
  .await?
  .ok_or(ApiError::NotFound)?;

  if object_type != "folder" {
    return Err(ApiError::BadRequest(
      "parent_view_id must be a folder — pages cannot contain pages".to_string(),
    ));
  }

  Ok(())
}

async fn ensure_valid_parent_view(
  db: &PgPool,
  workspace_id: Uuid,
  view_id: Uuid,
  parent_view_id: Uuid,
) -> ApiResult<()> {
  if parent_view_id == view_id {
    return Err(ApiError::BadRequest(
      "parent_view_id cannot be the same view".to_string(),
    ));
  }

  ensure_parent_accepts_children(db, workspace_id, parent_view_id).await?;

  let would_cycle = sqlx::query_scalar::<_, bool>(
    r#"
      WITH RECURSIVE descendants (id) AS (
        SELECT id
        FROM views
        WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
        UNION ALL
        SELECT v.id
        FROM views v
        INNER JOIN descendants d ON v.parent_view_id = d.id
        WHERE v.workspace_id = $2 AND v.is_deleted = false
      )
      SELECT EXISTS (
        SELECT 1
        FROM descendants
        WHERE id = $3
      )
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .bind(parent_view_id)
  .fetch_one(db)
  .await?;

  if would_cycle {
    return Err(ApiError::BadRequest(
      "parent_view_id cannot be a descendant view".to_string(),
    ));
  }

  Ok(())
}

fn normalize_view_name(name: &str) -> ApiResult<String> {
  let name = name.trim().to_string();
  if name.is_empty() {
    return Err(ApiError::BadRequest("view name is required".to_string()));
  }

  Ok(name)
}

fn normalize_position(position: Option<String>) -> ApiResult<String> {
  let Some(position) = position else {
    return Ok(Uuid::now_v7().to_string());
  };

  let position = position.trim().to_string();
  if position.is_empty() {
    return Err(ApiError::BadRequest("position cannot be empty".to_string()));
  }

  Ok(position)
}

// ── Cross-workspace transfer (move / copy) ───────────────────────────────────
// Move or copy a page (its whole subtree) or a folder into ANOTHER workspace on
// this server. No note app re-parents in place across workspaces: doc identity
// and blobs are both workspace-namespaced, so an in-place move invites doc-id
// collisions and lets the source's per-workspace blob GC reclaim images the
// moved page still needs. So this is copy-into-destination (new view + new
// document + blobs physically copied into the destination workspace), then a
// soft-delete of the source for a "move". Ordered blobs-first, so a half-failure
// leaves harmless orphan bytes in the destination (its GC reclaims them), never a
// page that lost its images — the thing Notion/AFFiNE get wrong and our Postgres
// transaction lets us beat. See docs/cross-workspace-move.md.

#[derive(Debug, Deserialize)]
pub struct TransferRequest {
  dest_workspace_id: Uuid,
  /// Parent folder in the destination (null = destination root).
  #[serde(default)]
  parent_view_id: Option<Uuid>,
  /// true = move (soft-delete the source after copying); false = copy (keep source).
  #[serde(default)]
  remove_source: bool,
  /// Report what WOULD happen (counts + dangling links) without mutating anything.
  #[serde(default)]
  dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct DanglingLink {
  /// Name of the moved document whose link now dangles.
  document: String,
  /// The `mica://page/<id>` target that stays in the source workspace.
  target_view_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct TransferResponse {
  new_root_view_id: Option<Uuid>,
  documents: usize,
  folders: usize,
  images: usize,
  dangling_links: Vec<DanglingLink>,
  removed_source: bool,
  dry_run: bool,
}

#[derive(Debug, FromRow)]
struct TransferRow {
  id: Uuid,
  parent_view_id: Option<Uuid>,
  object_id: Uuid,
  object_type: String,
  name: String,
  position: String,
}

/// `POST /api/workspaces/{workspace_id}/views/{view_id}/transfer`
pub async fn transfer_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((src_workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(request): Json<TransferRequest>,
) -> ApiResult<Json<TransferResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let dest_workspace_id = request.dest_workspace_id;
  if dest_workspace_id == src_workspace_id {
    return Err(ApiError::BadRequest(
      "destination workspace must differ from the source".to_string(),
    ));
  }
  // Editor in BOTH: to remove from the source and to create in the destination.
  ensure_workspace_editor(&state.db, src_workspace_id, user_id).await?;
  ensure_workspace_editor(&state.db, dest_workspace_id, user_id).await?;
  ensure_view_in_workspace(&state.db, src_workspace_id, view_id).await?;

  // A destination parent, if given, must be a live folder in the destination.
  if let Some(parent) = request.parent_view_id {
    ensure_parent_accepts_children(&state.db, dest_workspace_id, parent).await?;
  }

  // 1. Enumerate the subtree (live rows only).
  let subtree = sqlx::query_as::<_, TransferRow>(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id, parent_view_id, object_id, object_type::text AS object_type, name, position
        FROM views
        WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
        UNION ALL
        SELECT v.id, v.parent_view_id, v.object_id, v.object_type::text, v.name, v.position
        FROM views v JOIN subtree s ON v.parent_view_id = s.id
        WHERE v.is_deleted = false
      )
      SELECT id, parent_view_id, object_id, object_type, name, position FROM subtree
    "#,
  )
  .bind(view_id)
  .bind(src_workspace_id)
  .fetch_all(&state.db)
  .await?;
  if subtree.is_empty() {
    return Err(ApiError::NotFound);
  }
  let subtree_view_ids: std::collections::HashSet<Uuid> = subtree.iter().map(|r| r.id).collect();

  // 2. Pre-scan documents: referenced file_ids + cross-workspace dangling links.
  let mut payloads: std::collections::HashMap<Uuid, DocumentSnapshotPayload> =
    std::collections::HashMap::new();
  let mut referenced_files: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
  let mut dangling_links: Vec<DanglingLink> = Vec::new();
  let mut documents = 0usize;
  let mut folders = 0usize;
  for row in &subtree {
    if row.object_type != "document" {
      folders += 1;
      continue;
    }
    documents += 1;
    let Some(payload) = store::current_payload(&state.db, row.object_id).await? else {
      continue;
    };
    for block in &payload.blocks {
      if let Some(fid) = block
        .data
        .get("file_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
      {
        referenced_files.insert(fid);
      }
      for target in page_link_targets(&block.data) {
        if let Ok(tid) = Uuid::parse_str(&target) {
          if !subtree_view_ids.contains(&tid) {
            dangling_links.push(DanglingLink {
              document: row.name.clone(),
              target_view_id: tid,
            });
          }
        }
      }
    }
    payloads.insert(row.object_id, payload);
  }
  let images = referenced_files.len();

  if request.dry_run {
    return Ok(Json(TransferResponse {
      new_root_view_id: None,
      documents,
      folders,
      images,
      dangling_links,
      removed_source: false,
      dry_run: true,
    }));
  }

  // 3. Copy blobs into the destination BEFORE the transaction: content-addressed
  //    keys make the PUT idempotent, and a half-failure leaves only orphan bytes
  //    in dest (its GC reclaims them), never a page with broken images.
  let storage = state
    .storage
    .as_ref()
    .ok_or_else(|| ApiError::Internal("file storage is not configured".to_string()))?;
  let http = reqwest::Client::new();
  let mut file_map: std::collections::HashMap<Uuid, Uuid> = std::collections::HashMap::new();
  for &src_file_id in &referenced_files {
    let Some(src_file) = store::fetch_file(&state.db, src_workspace_id, src_file_id).await? else {
      continue; // dangling reference already; nothing to copy
    };
    let suffix = src_file
      .object_key
      .strip_prefix(&format!("workspaces/{src_workspace_id}/"))
      .ok_or_else(|| ApiError::Internal("source object_key not under its workspace".to_string()))?;
    let dest_key = format!("workspaces/{dest_workspace_id}/{suffix}");

    let bytes = http
      .get(storage.download_url(&src_file.object_key))
      .send()
      .await
      .map_err(|e| ApiError::Internal(format!("blob fetch failed: {e}")))?
      .error_for_status()
      .map_err(|e| ApiError::Internal(format!("blob fetch returned {e}")))?
      .bytes()
      .await
      .map_err(|e| ApiError::Internal(format!("blob read failed: {e}")))?;
    let upload = storage.presign_put(&dest_key);
    let put = http
      .put(&upload.url)
      .header(reqwest::header::CONTENT_TYPE, &src_file.mime_type)
      .body(bytes.to_vec())
      .send()
      .await
      .map_err(|e| ApiError::Internal(format!("blob upload failed: {e}")))?;
    if !put.status().is_success() {
      return Err(ApiError::Internal(format!(
        "blob upload returned {}",
        put.status()
      )));
    }

    let dest_file = store::insert_file(
      &state.db,
      dest_workspace_id,
      user_id,
      &dest_key,
      &src_file.original_name,
      &src_file.mime_type,
      src_file.byte_size,
    )
    .await?;
    file_map.insert(src_file_id, dest_file.id);
  }

  // 4. One transaction: build the destination tree (new ids), then soft-delete
  //    the source subtree for a move.
  let view_map: std::collections::HashMap<Uuid, Uuid> =
    subtree.iter().map(|r| (r.id, Uuid::new_v4())).collect();
  let ordered = topo_order_subtree(&subtree);

  let mut tx = state.db.begin().await?;
  for row in &ordered {
    let new_view_id = view_map[&row.id];
    let dest_parent = if row.id == view_id {
      request.parent_view_id
    } else {
      row.parent_view_id.and_then(|p| view_map.get(&p).copied())
    };
    if row.object_type == "document" {
      let mut payload = payloads.remove(&row.object_id).unwrap_or_else(|| {
        // Substituting an empty payload is CORRECT for a genuinely-empty doc
        // (current_payload returns None only when no snapshot row exists). It is
        // NOT silent anymore: the one way this loses content — a doc with a yrs
        // base but no snapshot row — would land here, and a move then deletes the
        // source. Logging makes that edge diagnosable instead of invisible (P0-4).
        tracing::warn!(
          object_id = %row.object_id,
          "transfer/clone: no snapshot for document — substituting empty payload"
        );
        DocumentSnapshotPayload {
          schema_version: 1,
          root_block_id: "root".to_string(),
          blocks: Vec::new(),
        }
      });
      rewrite_transferred_payload(&mut payload, &file_map, &view_map);
      let document = sqlx::query_as::<_, DocumentRecord>(
        r#"
          INSERT INTO documents (workspace_id, root_block_id, created_by)
          VALUES ($1, $2, $3)
          RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
        "#,
      )
      .bind(dest_workspace_id)
      .bind(&payload.root_block_id)
      .bind(user_id)
      .fetch_one(&mut *tx)
      .await?;
      store::insert_root_snapshot(&mut tx, document.id, &payload).await?;
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'document', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(dest_workspace_id)
      .bind(dest_parent)
      .bind(document.id)
      .bind(&row.name)
      .bind(&row.position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    } else {
      // Folder: a view with no document. object_id is a fresh unused uuid.
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'folder', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(dest_workspace_id)
      .bind(dest_parent)
      .bind(Uuid::new_v4())
      .bind(&row.name)
      .bind(&row.position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    }
  }

  if request.remove_source {
    sqlx::query(
      r#"
        WITH RECURSIVE subtree AS (
          SELECT id FROM views WHERE id = $1 AND workspace_id = $2
          UNION ALL
          SELECT v.id FROM views v JOIN subtree s ON v.parent_view_id = s.id
        )
        UPDATE views SET is_deleted = true, updated_at = now()
        WHERE id IN (SELECT id FROM subtree)
      "#,
    )
    .bind(view_id)
    .bind(src_workspace_id)
    .execute(&mut *tx)
    .await?;
  }

  tx.commit().await?;

  Ok(Json(TransferResponse {
    new_root_view_id: Some(view_map[&view_id]),
    documents,
    folders,
    images,
    dangling_links,
    removed_source: request.remove_source,
    dry_run: false,
  }))
}

// ── Clone (duplicate a view within the same workspace) ───────────────────────
// A same-workspace cousin of transfer: enumerate the subtree, give every node a
// fresh id + doc + snapshot, rewrite in-subtree page links to the new ids. Two
// deliberate differences from transfer:
//   - Blobs are NOT copied. object_key is content-addressed and workspace-scoped
//     (workspaces/{id}/{sha256}.{ext}); within ONE workspace the copy references
//     the same file_id — sharing the bytes is exactly the sha256-dedup intent,
//     and there's nothing to re-upload. So file_map stays empty and file_ids
//     pass through unchanged.
//   - The source is never removed, and the root copy gets a fresh name (deduped
//     among its siblings) + a fresh position so it sits beside the original.

#[derive(Debug, Deserialize)]
pub struct CloneRequest {
  /// Parent for the copy's root (null = the source's own parent, i.e. beside it).
  #[serde(default)]
  parent_view_id: Option<Uuid>,
  /// The copy's root name, locale-aware, computed by the caller (e.g. "X 副本").
  /// Deduped against siblings server-side. Absent → a "{source} 副本" fallback.
  #[serde(default)]
  name: Option<String>,
  /// Report what WOULD happen (counts) without mutating anything.
  #[serde(default)]
  dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct CloneResponse {
  new_root_view_id: Option<Uuid>,
  new_name: String,
  documents: usize,
  folders: usize,
  dry_run: bool,
}

/// `POST /api/workspaces/{workspace_id}/views/{view_id}/clone`
pub async fn clone_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(request): Json<CloneRequest>,
) -> ApiResult<Json<CloneResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_view_in_workspace(&state.db, workspace_id, view_id).await?;

  // 1. Enumerate the subtree (live rows only) — same shape as transfer.
  let subtree = sqlx::query_as::<_, TransferRow>(
    r#"
      WITH RECURSIVE subtree AS (
        SELECT id, parent_view_id, object_id, object_type::text AS object_type, name, position
        FROM views
        WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
        UNION ALL
        SELECT v.id, v.parent_view_id, v.object_id, v.object_type::text, v.name, v.position
        FROM views v JOIN subtree s ON v.parent_view_id = s.id
        WHERE v.is_deleted = false
      )
      SELECT id, parent_view_id, object_id, object_type, name, position FROM subtree
    "#,
  )
  .bind(view_id)
  .bind(workspace_id)
  .fetch_all(&state.db)
  .await?;
  let Some(root) = subtree.iter().find(|r| r.id == view_id) else {
    return Err(ApiError::NotFound);
  };

  // 2. Resolve the copy's parent: an explicit folder in this workspace, or the
  //    source's own parent (beside the original).
  let target_parent = match request.parent_view_id {
    Some(parent) => {
      ensure_parent_accepts_children(&state.db, workspace_id, parent).await?;
      Some(parent)
    }
    None => root.parent_view_id,
  };

  // 3. Name the copy: caller's locale-aware base (fallback "{source} 副本"),
  //    deduped against the live siblings under the target parent.
  let base_name = request
    .name
    .clone()
    .unwrap_or_else(|| format!("{} 副本", root.name));
  let siblings = sqlx::query_scalar::<_, String>(
    "SELECT name FROM views WHERE workspace_id = $1 AND is_deleted = false \
     AND parent_view_id IS NOT DISTINCT FROM $2",
  )
  .bind(workspace_id)
  .bind(target_parent)
  .fetch_all(&state.db)
  .await?;
  let new_name = dedup_sibling_name(&base_name, &siblings);

  // 4. Pre-scan document payloads (needed to rewrite in-subtree links). No blob
  //    copy, no dangling-link scan: every link target stays in this workspace.
  let mut payloads: std::collections::HashMap<Uuid, DocumentSnapshotPayload> =
    std::collections::HashMap::new();
  let mut documents = 0usize;
  let mut folders = 0usize;
  for row in &subtree {
    if row.object_type != "document" {
      folders += 1;
      continue;
    }
    documents += 1;
    if let Some(payload) = store::current_payload(&state.db, row.object_id).await? {
      payloads.insert(row.object_id, payload);
    }
  }

  if request.dry_run {
    return Ok(Json(CloneResponse {
      new_root_view_id: None,
      new_name,
      documents,
      folders,
      dry_run: true,
    }));
  }

  // 5. One transaction: build the copied tree with fresh ids. file_map is empty
  //    (blobs shared), view_map remaps in-subtree links to the new ids.
  let view_map: std::collections::HashMap<Uuid, Uuid> =
    subtree.iter().map(|r| (r.id, Uuid::new_v4())).collect();
  let empty_files: std::collections::HashMap<Uuid, Uuid> = std::collections::HashMap::new();
  let ordered = topo_order_subtree(&subtree);

  let mut tx = state.db.begin().await?;
  for row in &ordered {
    let new_view_id = view_map[&row.id];
    let is_root = row.id == view_id;
    let dest_parent = if is_root {
      target_parent
    } else {
      row.parent_view_id.and_then(|p| view_map.get(&p).copied())
    };
    // The root sits beside the original: fresh name + fresh position so it does
    // not collide with the source under the same parent. Inner nodes keep their
    // name/position — their parent is a new view, so nothing collides there.
    let node_name = if is_root { &new_name } else { &row.name };
    let node_position = if is_root {
      Uuid::now_v7().to_string()
    } else {
      row.position.clone()
    };

    if row.object_type == "document" {
      let mut payload = payloads.remove(&row.object_id).unwrap_or_else(|| {
        // Substituting an empty payload is CORRECT for a genuinely-empty doc
        // (current_payload returns None only when no snapshot row exists). It is
        // NOT silent anymore: the one way this loses content — a doc with a yrs
        // base but no snapshot row — would land here, and a move then deletes the
        // source. Logging makes that edge diagnosable instead of invisible (P0-4).
        tracing::warn!(
          object_id = %row.object_id,
          "transfer/clone: no snapshot for document — substituting empty payload"
        );
        DocumentSnapshotPayload {
          schema_version: 1,
          root_block_id: "root".to_string(),
          blocks: Vec::new(),
        }
      });
      rewrite_transferred_payload(&mut payload, &empty_files, &view_map);
      let document = sqlx::query_as::<_, DocumentRecord>(
        r#"
          INSERT INTO documents (workspace_id, root_block_id, created_by)
          VALUES ($1, $2, $3)
          RETURNING id, workspace_id, root_block_id, current_seq, created_by, created_at, updated_at
        "#,
      )
      .bind(workspace_id)
      .bind(&payload.root_block_id)
      .bind(user_id)
      .fetch_one(&mut *tx)
      .await?;
      store::insert_root_snapshot(&mut tx, document.id, &payload).await?;
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'document', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(workspace_id)
      .bind(dest_parent)
      .bind(document.id)
      .bind(node_name)
      .bind(&node_position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    } else {
      sqlx::query(
        r#"
          INSERT INTO views (id, workspace_id, parent_view_id, object_id, object_type, name, position, created_by)
          VALUES ($1, $2, $3, $4, 'folder', $5, $6, $7)
        "#,
      )
      .bind(new_view_id)
      .bind(workspace_id)
      .bind(dest_parent)
      .bind(Uuid::new_v4())
      .bind(node_name)
      .bind(&node_position)
      .bind(user_id)
      .execute(&mut *tx)
      .await?;
    }
  }
  tx.commit().await?;

  Ok(Json(CloneResponse {
    new_root_view_id: Some(view_map[&view_id]),
    new_name,
    documents,
    folders,
    dry_run: false,
  }))
}

/// Pick a sibling-unique name: `base` if free, else `base 2`, `base 3`, … The
/// number is locale-neutral, so the caller supplies the localized base ("X 副本"
/// / "X copy") and we only break ties.
fn dedup_sibling_name(base: &str, siblings: &[String]) -> String {
  if !siblings.iter().any(|s| s == base) {
    return base.to_string();
  }
  (2..)
    .map(|n| format!("{base} {n}"))
    .find(|c| !siblings.iter().any(|s| s == c))
    .expect("an unused suffix always exists")
}

/// The `mica://page/<viewId>` targets referenced by a block's link marks.
fn page_link_targets(data: &serde_json::Value) -> Vec<String> {
  const SCHEME: &str = "mica://page/";
  let Some(marks) = data.get("marks").and_then(|m| m.as_array()) else {
    return Vec::new();
  };
  marks
    .iter()
    .filter_map(|mark| {
      mark
        .get("href")
        .and_then(|h| h.as_str())
        .and_then(|href| href.strip_prefix(SCHEME))
        .map(str::to_string)
    })
    .collect()
}

/// Rewrite a transferred document's blocks for their new home: remap uploaded-
/// image `file_id`s to the destination copies, and remap in-subtree page links to
/// the new view ids. Links to pages left in the source keep their `mica://` href
/// (they dangle — surfaced to the user as a warning before the move).
fn rewrite_transferred_payload(
  payload: &mut DocumentSnapshotPayload,
  file_map: &std::collections::HashMap<Uuid, Uuid>,
  view_map: &std::collections::HashMap<Uuid, Uuid>,
) {
  const SCHEME: &str = "mica://page/";
  for block in &mut payload.blocks {
    let Some(data) = block.data.as_object_mut() else {
      continue;
    };
    // Image file_id → destination copy. Drop any cached blob `url` so the client
    // re-resolves it against the destination workspace.
    if let Some(new_id) = data
      .get("file_id")
      .and_then(|v| v.as_str())
      .and_then(|s| Uuid::parse_str(s).ok())
      .and_then(|old| file_map.get(&old))
    {
      let new_id = *new_id;
      data.insert("file_id".into(), serde_json::json!(new_id.to_string()));
      data.remove("url");
    }
    // In-subtree page links → new view ids.
    if let Some(marks) = data.get_mut("marks").and_then(|m| m.as_array_mut()) {
      for mark in marks {
        let Some(obj) = mark.as_object_mut() else {
          continue;
        };
        let Some(new_href) = obj
          .get("href")
          .and_then(|h| h.as_str())
          .and_then(|href| href.strip_prefix(SCHEME))
          .and_then(|id| Uuid::parse_str(id).ok())
          .and_then(|old| view_map.get(&old))
          .map(|new| format!("{SCHEME}{new}"))
        else {
          continue;
        };
        obj.insert("href".into(), serde_json::json!(new_href));
      }
    }
  }
}

/// Order the subtree so a parent always precedes its children (root first). The
/// recursive CTE does not guarantee parent-first, and we insert with FK-linked
/// parents, so the order matters.
fn topo_order_subtree(subtree: &[TransferRow]) -> Vec<&TransferRow> {
  let ids: std::collections::HashSet<Uuid> = subtree.iter().map(|r| r.id).collect();
  let mut by_parent: std::collections::HashMap<Option<Uuid>, Vec<&TransferRow>> =
    std::collections::HashMap::new();
  for r in subtree {
    // The root's real parent is outside the subtree — anchor it at None.
    let key = r.parent_view_id.filter(|p| ids.contains(p));
    by_parent.entry(key).or_default().push(r);
  }
  let mut out: Vec<&TransferRow> = Vec::with_capacity(subtree.len());
  let mut stack: Vec<Option<Uuid>> = vec![None];
  while let Some(parent) = stack.pop() {
    if let Some(children) = by_parent.get(&parent) {
      for child in children {
        out.push(child);
        stack.push(Some(child.id));
      }
    }
  }
  out
}

#[cfg(test)]
mod tests {
  /// An image block carrying [data] — the only field these tests vary.
  fn image_block(data: serde_json::Value) -> mica_app_core::documents::Block {
    mica_app_core::documents::Block {
      id: "img_1".to_string(),
      kind: "image".to_string(),
      text: String::new(),
      data,
      children: Vec::new(),
    }
  }

  /// An uploaded image must survive `export → import` as the SAME file, not as
  /// a link. Before the asset map, a Markdown export fell back to the block's
  /// original filename — and every client names a pasted image
  /// `pasted-image.png`, so nine images across three real workspaces all
  /// exported as `![](pasted-image.png)`: unfetchable, and indistinguishable.
  /// Re-importing that produced an image block pointing at a bare filename,
  /// silently dropping the file reference — which would also make blob GC stop
  /// counting the file and eventually delete the bytes.
  #[test]
  fn an_uploaded_image_round_trips_through_markdown_as_the_same_file() {
    let ws = Uuid::new_v4();
    let file_id = Uuid::new_v4().to_string();
    let block = image_block(serde_json::json!({"file_id": file_id, "name": "pasted-image.png"}));

    let assets = blob_asset_map(std::slice::from_ref(&block), ws);
    let href = assets.get(&file_id).expect("the file_id gets a href");
    assert!(
      href.contains(&file_id) && href.starts_with(&format!("/api/workspaces/{ws}/files/")),
      "the href must actually serve the bytes: {href}"
    );

    // …and the import side recognises what the export side wrote.
    let mut back = vec![image_block(serde_json::json!({"url": href}))];
    rewire_blob_hrefs(&mut back, ws);
    assert_eq!(back[0].data["file_id"], serde_json::json!(file_id));
    assert_eq!(back[0].data["name"], serde_json::json!("pasted-image.png"));
    assert!(back[0].data.get("url").is_none(), "no longer a plain link");
  }

  /// A href for a DIFFERENT workspace must stay a link. Rewiring it would forge
  /// a reference to a file this workspace's readers may not be allowed to see —
  /// and would let a pasted URL smuggle another tenant's blob into a page.
  #[test]
  fn a_blob_href_from_another_workspace_is_never_claimed() {
    let mine = Uuid::new_v4();
    let theirs = Uuid::new_v4();
    let file_id = Uuid::new_v4().to_string();
    let href = blob_href(theirs, &file_id, "secret.png");

    let mut blocks = vec![image_block(serde_json::json!({"url": href}))];
    rewire_blob_hrefs(&mut blocks, mine);
    assert!(blocks[0].data.get("file_id").is_none(), "not ours to claim");
    assert!(blocks[0].data.get("url").is_some(), "stays a plain link");

    // Nor does an external look-alike get claimed.
    let mut evil = vec![image_block(
      serde_json::json!({"url": "https://evil.test/api/workspaces/nope/files/x/blob/a.png"}),
    )];
    rewire_blob_hrefs(&mut evil, mine);
    assert!(evil[0].data.get("file_id").is_none());
  }

  /// A plain external image is not ours and must pass through untouched — the
  /// whole point of the guard is that it only claims what it wrote.
  #[test]
  fn an_external_image_url_survives_import_as_a_link() {
    let mut blocks = vec![image_block(
      serde_json::json!({"url": "https://example.com/photo.png"}),
    )];
    rewire_blob_hrefs(&mut blocks, Uuid::new_v4());
    assert_eq!(
      blocks[0].data["url"],
      serde_json::json!("https://example.com/photo.png")
    );
  }

  #[test]
  fn percent_decode_recovers_utf8_and_leaves_junk_alone() {
    assert_eq!(percent_decode("pasted-image.png"), "pasted-image.png");
    assert_eq!(percent_decode("%E5%9B%BE.png"), "图.png");
    // A malformed escape is data, not a parse error.
    assert_eq!(percent_decode("100%zz"), "100%zz");
    assert_eq!(percent_decode("%"), "%");
  }

  /// `safe_segment` is the whole of the name→file-name rule, and since the body
  /// is exported verbatim the file name is now the ONLY place a page's name
  /// survives an export. A bug here loses it silently.
  #[test]
  fn safe_segment_makes_a_usable_file_name_from_any_page_name() {
    assert_eq!(safe_segment("mica-cli"), "mica-cli");
    assert_eq!(safe_segment("无引用 blob 自动回收"), "无引用_blob_自动回收");
    // Path separators and Windows-hostile characters cannot survive as-is —
    // one `/` would silently fabricate a directory inside the archive.
    assert_eq!(safe_segment("a/b"), "a_b");
    assert_eq!(safe_segment("what? really!"), "what_really");
    // Runs collapse and edges are trimmed, so names stay readable.
    assert_eq!(safe_segment("  spaced   out  "), "spaced_out");
    // A name that survives nothing still must yield a file name.
    assert_eq!(safe_segment("///"), "untitled");
    assert_eq!(safe_segment(""), "untitled");
  }

  use super::*;

  #[test]
  fn outline_lists_headings_and_block_ids_in_document_order() {
    use mica_app_core::documents::{Block, DocumentSnapshotPayload};
    let blk = |id: &str, kind: &str, text: &str, data: serde_json::Value, kids: Vec<&str>| Block {
      id: id.into(),
      kind: kind.into(),
      text: text.into(),
      data,
      children: kids.into_iter().map(String::from).collect(),
    };
    let payload = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".into(),
      blocks: vec![
        blk(
          "root",
          "page",
          "",
          serde_json::Value::Null,
          vec!["h1", "p", "h2"],
        ),
        blk(
          "h1",
          "heading",
          "Intro",
          serde_json::json!({"level": 1}),
          vec![],
        ),
        blk("p", "paragraph", "body", serde_json::Value::Null, vec![]),
        blk(
          "h2",
          "heading",
          "Details",
          serde_json::json!({"level": 2}),
          vec![],
        ),
      ],
    };
    let out = outline_from_payload(&payload);
    assert_eq!(out.block_ids, ["h1", "p", "h2"]);
    assert_eq!(
      out
        .headings
        .iter()
        .map(|h| (h.level, h.text.as_str()))
        .collect::<Vec<_>>(),
      [(1, "Intro"), (2, "Details")],
    );
  }

  fn doc_with_children(kids: &[&str]) -> mica_app_core::documents::DocumentSnapshotPayload {
    use mica_app_core::documents::{Block, DocumentSnapshotPayload};
    let mut blocks = vec![Block {
      id: "root".into(),
      kind: "page".into(),
      text: "".into(),
      data: serde_json::Value::Null,
      children: kids.iter().map(|s| s.to_string()).collect(),
    }];
    for k in kids {
      blocks.push(Block {
        id: (*k).into(),
        kind: "paragraph".into(),
        text: "existing".into(),
        data: serde_json::Value::Null,
        children: vec![],
      });
    }
    DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".into(),
      blocks,
    }
  }

  fn upd(mode: MarkdownUpdateMode, markdown: &str) -> UpdateMarkdownRequest {
    UpdateMarkdownRequest {
      mode,
      markdown: markdown.into(),
      anchor: None,
      find: None,
      replace: None,
    }
  }

  #[test]
  fn markdown_update_append_grafts_under_root_without_deletes() {
    use mica_app_core::documents::DocumentOperation;
    let current = doc_with_children(&["old"]);
    let ops = markdown_update_ops(
      &current,
      &upd(MarkdownUpdateMode::Append, "# Title\n\nhello"),
      Uuid::new_v4(),
    )
    .unwrap();
    assert!(
      ops
        .iter()
        .all(|o| !matches!(o, DocumentOperation::DeleteBlock { .. })),
      "append never deletes",
    );
    let top_inserts = ops
      .iter()
      .filter(
        |o| matches!(o, DocumentOperation::InsertBlock { parent_id, .. } if parent_id == "root"),
      )
      .count();
    assert!(
      top_inserts >= 2,
      "heading + paragraph grafted under the existing root"
    );
    assert!(
      ops.iter().all(|o| match o {
        DocumentOperation::InsertBlock { block, .. } => block.children.is_empty(),
        _ => true,
      }),
      "inserted blocks have children stripped (re-linked via parent_id)",
    );
  }

  #[test]
  fn markdown_update_replace_all_deletes_existing_top_level_first() {
    use mica_app_core::documents::DocumentOperation;
    let current = doc_with_children(&["a", "b"]);
    let ops = markdown_update_ops(
      &current,
      &upd(MarkdownUpdateMode::ReplaceAll, "fresh body"),
      Uuid::new_v4(),
    )
    .unwrap();
    let deletes: Vec<&str> = ops
      .iter()
      .filter_map(|o| match o {
        DocumentOperation::DeleteBlock { block_id } => Some(block_id.as_str()),
        _ => None,
      })
      .collect();
    assert_eq!(
      deletes,
      ["a", "b"],
      "existing top-level children deleted first"
    );
    assert!(
      ops
        .iter()
        .any(|o| matches!(o, DocumentOperation::InsertBlock { .. })),
      "then the new markdown is grafted in",
    );
  }

  #[test]
  fn markdown_update_insert_at_positions_after_the_anchor() {
    use mica_app_core::documents::DocumentOperation;
    let current = doc_with_children(&["a", "b", "c"]);
    let mut request = upd(MarkdownUpdateMode::InsertAt, "inserted");
    request.anchor = Some("a".into());
    let ops = markdown_update_ops(&current, &request, Uuid::new_v4()).unwrap();
    // The (single) new top-level paragraph lands at index 1 — right after "a".
    let top = ops
      .iter()
      .find(
        |o| matches!(o, DocumentOperation::InsertBlock { parent_id, .. } if parent_id == "root"),
      )
      .expect("a top-level insert");
    match top {
      DocumentOperation::InsertBlock { index, .. } => assert_eq!(*index, Some(1)),
      _ => unreachable!(),
    }
  }

  #[test]
  fn markdown_update_insert_at_unknown_anchor_errors() {
    let current = doc_with_children(&["a"]);
    let mut request = upd(MarkdownUpdateMode::InsertAt, "x");
    request.anchor = Some("ghost".into());
    assert!(markdown_update_ops(&current, &request, Uuid::new_v4()).is_err());
  }

  #[test]
  fn markdown_update_find_replace_updates_matching_blocks() {
    use mica_app_core::documents::DocumentOperation;
    let current = doc_with_children(&["a", "b"]); // both blocks' text == "existing"
    let request = UpdateMarkdownRequest {
      mode: MarkdownUpdateMode::FindReplace,
      markdown: String::new(),
      anchor: None,
      find: Some("existing".into()),
      replace: Some("updated".into()),
    };
    let ops = markdown_update_ops(&current, &request, Uuid::new_v4()).unwrap();
    let updated: Vec<(&str, &str)> = ops
      .iter()
      .filter_map(|o| match o {
        DocumentOperation::UpdateBlock {
          block_id,
          text: Some(t),
          ..
        } => Some((block_id.as_str(), t.as_str())),
        _ => None,
      })
      .collect();
    assert_eq!(updated, [("a", "updated"), ("b", "updated")]);
    // No matches → an error, not a silent no-op.
    let mut miss = request;
    miss.find = Some("nope".into());
    assert!(markdown_update_ops(&current, &miss, Uuid::new_v4()).is_err());
  }

  #[test]
  fn markdown_update_find_replace_skips_formatted_blocks() {
    use mica_app_core::documents::{Block, DocumentSnapshotPayload};
    // The only matching block carries an inline mark → a text-only replace would
    // desync its UTF-16 offsets, so find_replace must refuse rather than corrupt.
    let payload = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".into(),
      blocks: vec![
        Block {
          id: "root".into(),
          kind: "page".into(),
          text: "".into(),
          data: serde_json::Value::Null,
          children: vec!["p".into()],
        },
        Block {
          id: "p".into(),
          kind: "paragraph".into(),
          text: "see docs now".into(),
          data: serde_json::json!({"marks": [{"type": "link", "start": 4, "end": 8}]}),
          children: vec![],
        },
      ],
    };
    let request = UpdateMarkdownRequest {
      mode: MarkdownUpdateMode::FindReplace,
      markdown: String::new(),
      anchor: None,
      find: Some("see ".into()),
      replace: Some(String::new()),
    };
    let err = markdown_update_ops(&payload, &request, Uuid::new_v4()).unwrap_err();
    assert!(
      err.contains("formatted"),
      "should refuse formatted blocks, got: {err}"
    );
  }

  #[test]
  fn markdown_update_replace_all_empty_refuses_to_wipe() {
    let current = doc_with_children(&["a", "b"]);
    let err = markdown_update_ops(
      &current,
      &upd(MarkdownUpdateMode::ReplaceAll, "  \n "),
      Uuid::new_v4(),
    )
    .unwrap_err();
    assert!(
      err.contains("wipe") || err.contains("content"),
      "empty replace_all must not wipe the doc, got: {err}",
    );
  }

  fn text_block(id: &str, data: serde_json::Value) -> mica_app_core::documents::Block {
    mica_app_core::documents::Block {
      id: id.to_string(),
      kind: "text".to_string(),
      text: "x".to_string(),
      data,
      children: Vec::new(),
    }
  }

  /// A transferred doc must point its image at the DESTINATION file copy (else
  /// the source's per-workspace GC reclaims the bytes and the moved page breaks),
  /// remap links to pages that came along, and leave links to pages left behind
  /// untouched (they dangle — surfaced as a warning, not silently rewritten).
  #[test]
  fn transfer_rewrites_file_ids_and_in_subtree_links_only() {
    let old_file = Uuid::new_v4();
    let new_file = Uuid::new_v4();
    let in_sub_old = Uuid::new_v4();
    let in_sub_new = Uuid::new_v4();
    let outside = Uuid::new_v4();

    let file_map = std::collections::HashMap::from([(old_file, new_file)]);
    let view_map = std::collections::HashMap::from([(in_sub_old, in_sub_new)]);

    let mut payload = DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        image_block(serde_json::json!({
          "file_id": old_file.to_string(),
          "name": "x.png",
          "url": format!("/api/workspaces/{}/files/{}/blob/x.png", Uuid::new_v4(), old_file),
        })),
        text_block(
          "t1",
          serde_json::json!({"marks": [
            {"type": "link", "href": format!("mica://page/{in_sub_old}")},
            {"type": "link", "href": format!("mica://page/{outside}")},
          ]}),
        ),
      ],
    };
    rewrite_transferred_payload(&mut payload, &file_map, &view_map);

    assert_eq!(
      payload.blocks[0].data["file_id"],
      serde_json::json!(new_file.to_string()),
      "image file_id must point at the destination copy",
    );
    assert!(
      payload.blocks[0].data.get("url").is_none(),
      "stale cached blob url must be dropped so the client re-resolves",
    );
    let marks = payload.blocks[1].data["marks"].as_array().unwrap();
    assert_eq!(
      marks[0]["href"],
      serde_json::json!(format!("mica://page/{in_sub_new}")),
      "a link to a page that came along is remapped",
    );
    assert_eq!(
      marks[1]["href"],
      serde_json::json!(format!("mica://page/{outside}")),
      "a link to a page left behind is preserved (dangles, warned)",
    );
  }

  #[test]
  fn dedup_sibling_name_only_numbers_on_collision() {
    // Free name → used as-is.
    assert_eq!(dedup_sibling_name("日志方案 副本", &[]), "日志方案 副本");
    // Collision → first free numbered suffix (locale-neutral number).
    assert_eq!(
      dedup_sibling_name("日志方案 副本", &["日志方案 副本".into()]),
      "日志方案 副本 2"
    );
    // Skips taken numbers, does not reuse them.
    assert_eq!(
      dedup_sibling_name(
        "X 副本",
        &["X 副本".into(), "X 副本 2".into(), "X 副本 4".into()]
      ),
      "X 副本 3"
    );
    // Unrelated siblings never force a suffix.
    assert_eq!(dedup_sibling_name("X 副本", &["Y".into(), "Z".into()]), "X 副本");
  }

  #[test]
  fn page_link_targets_extracts_only_mica_page_ids() {
    let a = Uuid::new_v4();
    let data = serde_json::json!({"marks": [
      {"type": "link", "href": format!("mica://page/{a}")},
      {"type": "link", "href": "https://example.com"},
      {"type": "bold"},
    ]});
    assert_eq!(page_link_targets(&data), vec![a.to_string()]);
    assert!(page_link_targets(&serde_json::json!({})).is_empty());
  }

  #[test]
  fn topo_order_puts_parents_before_children() {
    let root = Uuid::new_v4();
    let child = Uuid::new_v4();
    let grandchild = Uuid::new_v4();
    let outside_parent = Uuid::new_v4(); // root's real parent, outside the subtree
    let row = |id, parent| TransferRow {
      id,
      parent_view_id: parent,
      object_id: Uuid::new_v4(),
      object_type: "folder".to_string(),
      name: "n".to_string(),
      position: "0".to_string(),
    };
    // Deliberately not parent-first, mimicking the CTE's arbitrary order.
    let subtree = vec![
      row(grandchild, Some(child)),
      row(root, Some(outside_parent)),
      row(child, Some(root)),
    ];
    let ordered: Vec<Uuid> = topo_order_subtree(&subtree).iter().map(|r| r.id).collect();
    let pos = |id: Uuid| ordered.iter().position(|&x| x == id).unwrap();
    assert_eq!(ordered.len(), 3);
    assert!(pos(root) < pos(child), "root must precede its child");
    assert!(pos(child) < pos(grandchild), "child must precede its grandchild");
  }

  /// The page-tree invariant guard runs entirely in SQL — `object_type` lives in
  /// Postgres, and the backstop is a DB trigger — so a mock would assert nothing.
  /// Gated on `DATABASE_URL`, hardened the same way as auth.rs::refresh_pg and
  /// app-core/tests/sync_pg.rs: skipped (green) without a database locally, but a
  /// set-but-unreachable URL — or a missing one in CI — is a hard failure, never
  /// a silent pass.
  ///
  ///   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
  ///   cargo test -p mica-api-server parent_guard_pg
  mod parent_guard_pg {
    use super::*;

    async fn pool() -> Option<PgPool> {
      let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(
          std::env::var("CI").is_err(),
          "DATABASE_URL is unset in CI — the postgres service block regressed; \
           these tests must not silently pass"
        );
        return None;
      };
      Some(
        PgPool::connect(&url)
          .await
          .expect("DATABASE_URL is set but the connection failed"),
      )
    }

    /// Seed the FK chain a view needs: user → workspace. Returns (workspace, user).
    async fn seed_workspace(db: &PgPool) -> (Uuid, Uuid) {
      let user = Uuid::new_v4();
      let ws = Uuid::new_v4();
      sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
        .bind(user)
        .bind(format!("{user}@parent.test"))
        .execute(db)
        .await
        .unwrap();
      sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'W',$2)")
        .bind(ws)
        .bind(user)
        .execute(db)
        .await
        .unwrap();
      (ws, user)
    }

    /// Insert a top-level view (parent_view_id NULL, so the trigger stays out of
    /// the way) of the given object_type. Returns its id.
    async fn seed_view(db: &PgPool, ws: Uuid, user: Uuid, object_type: &str) -> Uuid {
      let id = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO views(id,workspace_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$3,$4::object_type,'V','0',$5)",
      )
      .bind(id)
      .bind(ws)
      .bind(Uuid::new_v4())
      .bind(object_type)
      .bind(user)
      .execute(db)
      .await
      .unwrap();
      id
    }

    #[tokio::test]
    async fn a_page_is_refused_as_a_parent_but_a_folder_is_accepted() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      let folder = seed_view(&db, ws, user, "folder").await;
      let page = seed_view(&db, ws, user, "document").await;

      // A folder is a legal container.
      ensure_parent_accepts_children(&db, ws, folder)
        .await
        .expect("a folder must accept children");

      // A page is a leaf: the guard rejects it with a readable 400, not the
      // trigger's 500.
      let err = ensure_parent_accepts_children(&db, ws, page)
        .await
        .expect_err("a page must not accept children");
      assert!(
        matches!(err, ApiError::BadRequest(_)),
        "a page parent must be a 400, got {err:?}"
      );

      // A parent that does not exist in this workspace is a 404, not a 400.
      let missing = ensure_parent_accepts_children(&db, ws, Uuid::new_v4())
        .await
        .expect_err("an unknown parent must be rejected");
      assert!(
        matches!(missing, ApiError::NotFound),
        "an unknown parent must be a 404, got {missing:?}"
      );
    }

    /// A page that links to another page shows up in that page's backlinks, and
    /// never the reverse (link direction is not symmetric). Gated the same way as
    /// the parent-guard tests: green without a DB locally, hard-fail in CI.
    ///
    ///   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
    ///   cargo test -p mica-api-server backlinks_pg
    #[tokio::test]
    async fn backlinks_are_the_inverse_of_forward_page_links() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      // Two pages; A has a block whose link mark points at B.
      let (view_b, _doc_b) = seed_document(&db, ws, user, "B", serde_json::json!([])).await;
      let (view_a, _doc_a) = seed_document(
        &db,
        ws,
        user,
        "A",
        serde_json::json!([{
          "id": "blk_link",
          "type": "paragraph",
          "text": "see B",
          "data": {"marks": [
            {"type": "link", "href": format!("mica://page/{view_b}")}
          ]}
        }]),
      )
      .await;

      // B's backlinks contain A (and carry A's view id + document id + title).
      let b_links = collect_backlinks(&db, ws, view_b).await;
      assert_eq!(b_links.len(), 1, "B should have exactly one backlink");
      assert_eq!(b_links[0].view_id, view_a, "the backlink source is A's view");
      assert_eq!(b_links[0].title, "A");

      // A's backlinks are empty — links point one way.
      let a_links = collect_backlinks(&db, ws, view_a).await;
      assert!(a_links.is_empty(), "A should have no backlinks, got {a_links:?}");
    }

    /// A page that links to ITSELF is not its own backlink.
    #[tokio::test]
    async fn a_self_link_is_not_a_backlink() {
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;

      // Seed the page, then rewrite its snapshot to link to its own view id.
      let (view_id, doc_id) = seed_document(&db, ws, user, "S", serde_json::json!([])).await;
      let payload = serde_json::json!({
        "schema_version": 1,
        "root_block_id": "root",
        "blocks": [{
          "id": "root",
          "type": "paragraph",
          "text": "self",
          "data": {"marks": [
            {"type": "link", "href": format!("mica://page/{view_id}")}
          ]}
        }]
      });
      sqlx::query(
        "UPDATE document_snapshots SET payload = $1 WHERE document_id = $2 AND version_seq = 0",
      )
      .bind(&payload)
      .bind(doc_id)
      .execute(&db)
      .await
      .unwrap();

      let links = collect_backlinks(&db, ws, view_id).await;
      assert!(links.is_empty(), "a self-link must not be a backlink, got {links:?}");
    }

    /// Seed a document view (a leaf page) with a content snapshot. `blocks` is the
    /// snapshot payload's `blocks` array; with no yrs base row `current_payload`
    /// falls back to this snapshot verbatim, so link marks placed here are exactly
    /// what the backlink scan sees. Returns (view_id, document_id).
    async fn seed_document(
      db: &PgPool,
      ws: Uuid,
      user: Uuid,
      title: &str,
      blocks: serde_json::Value,
    ) -> (Uuid, Uuid) {
      let doc_id = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO documents(id,workspace_id,root_block_id,created_by) VALUES($1,$2,'root',$3)",
      )
      .bind(doc_id)
      .bind(ws)
      .bind(user)
      .execute(db)
      .await
      .unwrap();
      let payload = serde_json::json!({
        "schema_version": 1,
        "root_block_id": "root",
        "blocks": blocks,
      });
      sqlx::query(
        "INSERT INTO document_snapshots(document_id,version_seq,schema_version,payload) \
         VALUES($1,0,1,$2)",
      )
      .bind(doc_id)
      .bind(&payload)
      .execute(db)
      .await
      .unwrap();
      let view_id = Uuid::new_v4();
      sqlx::query(
        "INSERT INTO views(id,workspace_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$3,'document'::object_type,$4,'0',$5)",
      )
      .bind(view_id)
      .bind(ws)
      .bind(doc_id)
      .bind(title)
      .bind(user)
      .execute(db)
      .await
      .unwrap();
      (view_id, doc_id)
    }

    /// A row of the backlinks endpoint's JSON response, minimal fields the tests
    /// assert on. `Backlink` itself is private and Serialize-only.
    #[derive(Debug, serde::Deserialize)]
    struct BacklinkRow {
      view_id: Uuid,
      title: String,
    }

    /// Run the backlink scan the handler runs (member auth is orthogonal here) and
    /// return its rows, round-tripped through JSON to prove the wire shape.
    async fn collect_backlinks(db: &PgPool, ws: Uuid, view_id: Uuid) -> Vec<BacklinkRow> {
      let target = view_id.to_string();
      let views = fetch_workspace_views(db, ws).await.unwrap();
      let mut rows = Vec::new();
      for view in &views {
        if view.object_type != "document" || view.id == view_id {
          continue;
        }
        let Some(payload) = store::current_payload(db, view.object_id).await.unwrap() else {
          continue;
        };
        let hit = payload
          .blocks
          .iter()
          .any(|b| page_link_targets(&b.data).iter().any(|t| *t == target));
        if hit {
          let json = serde_json::json!({"view_id": view.id, "title": view.name});
          rows.push(serde_json::from_value(json).unwrap());
        }
      }
      rows.sort_by(|a: &BacklinkRow, b: &BacklinkRow| a.title.cmp(&b.title));
      rows
    }

    #[tokio::test]
    async fn the_db_trigger_backstops_a_write_that_skips_the_guard() {
      // The guard is the front door; `views_parent_must_be_folder` (migration
      // 0011) covers every path that forgets to call it. Writing
      // parent_view_id = <page> straight into the table must still be refused.
      let Some(db) = pool().await else { return };
      let (ws, user) = seed_workspace(&db).await;
      let page = seed_view(&db, ws, user, "document").await;

      let direct = sqlx::query(
        "INSERT INTO views(workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
         VALUES($1,$2,$3,'document'::object_type,'V','0',$4)",
      )
      .bind(ws)
      .bind(page)
      .bind(Uuid::new_v4())
      .bind(user)
      .execute(&db)
      .await;
      assert!(
        direct.is_err(),
        "the trigger must reject nesting a view under a page"
      );
    }
  }
}
