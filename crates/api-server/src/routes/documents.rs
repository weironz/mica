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
    DocumentOperation, DocumentSnapshotPayload, export_html, export_markdown,
    export_markdown_with_assets, import_markdown, payload_from_value,
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
pub struct HtmlExportResponse {
  html: String,
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
  let mut results = Vec::new();
  for view in views {
    if view.object_type != "document" {
      continue;
    }
    let title_match = view.name.to_lowercase().contains(&needle);

    let mut snippet = String::new();
    if let Some(snapshot) = store::latest_snapshot(&state.db, view.object_id).await? {
      if let Ok(payload) = payload_from_value(snapshot.payload) {
        for block in &payload.blocks {
          if let Some(found) = snippet_for(&block.text, &needle) {
            snippet = found;
            break;
          }
        }
      }
    }

    if title_match || !snippet.is_empty() {
      results.push(SearchResult {
        view_id: view.id,
        object_id: view.object_id,
        name: view.name,
        snippet,
        title_match,
      });
    }
    if results.len() >= 50 {
      break;
    }
  }

  Ok(Json(SearchResponse { results }))
}

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
    ensure_view_in_workspace(&state.db, workspace_id, parent_view_id).await?;
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
    ensure_view_in_workspace(&state.db, workspace_id, parent_view_id).await?;
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
    ensure_view_in_workspace(&state.db, workspace_id, parent_view_id).await?;
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

  let imported = import_markdown(&payload.markdown, &root_block_id);
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

  let snapshot = store::latest_snapshot(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;

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
  let snapshot = store::latest_snapshot(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let payload = payload_from_value(snapshot.payload)
    .map_err(|error| ApiError::BadRequest(format!("invalid document snapshot: {error}")))?;
  let markdown =
    export_markdown(&payload).map_err(|error| ApiError::BadRequest(error.to_string()))?;

  Ok(Json(MarkdownExportResponse { markdown }))
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

  let snapshot = store::latest_snapshot(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let payload = payload_from_value(snapshot.payload)
    .map_err(|error| ApiError::BadRequest(format!("invalid document snapshot: {error}")))?;

  let mut entries = Vec::new();
  let assets = collect_assets(&state, workspace_id, &payload.blocks, &mut entries).await?;
  let markdown = export_markdown_with_assets(&payload, &assets)
    .map_err(|error| ApiError::BadRequest(error.to_string()))?;
  entries.insert(
    0,
    ZipEntry {
      name: "document.md".to_string(),
      data: markdown.into_bytes(),
    },
  );

  Ok(zip_response(build_zip(&entries), "document.zip"))
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
    let bytes = match client.get(storage.download_url(&record.object_key)).send().await {
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

  let views = fetch_workspace_views(&state.db, workspace_id).await?;
  let mut by_parent: std::collections::HashMap<Option<Uuid>, Vec<&View>> =
    std::collections::HashMap::new();
  for v in &views {
    by_parent.entry(v.parent_view_id).or_default().push(v);
  }
  for list in by_parent.values_mut() {
    list.sort_by(|a, b| a.position.cmp(&b.position));
  }
  let mut pages: Vec<(&View, Vec<String>, String)> = Vec::new();
  collect_page_paths(&by_parent, None, &Vec::new(), &mut pages);

  let storage = state.storage.clone();
  let client = reqwest::Client::new();
  let mut entries: Vec<ZipEntry> = Vec::new();
  let mut asset_by_object: std::collections::HashMap<String, String> =
    std::collections::HashMap::new();
  let mut used_assets: std::collections::HashSet<String> = std::collections::HashSet::new();
  let mut used_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
  // Page order for the import side: paths in pre-order tree order.
  let mut manifest_pages: Vec<serde_json::Value> = Vec::new();

  // Final zip path per view, decided up front so page links can target pages
  // that come later in the tree.
  let mut path_by_view: std::collections::HashMap<String, String> =
    std::collections::HashMap::new();
  for (view, folder, base) in &pages {
    if view.object_type != "document" {
      continue;
    }
    let mut path = String::new();
    for seg in folder {
      path.push_str(seg);
      path.push('/');
    }
    path.push_str(base);
    path.push_str(".md");
    path = unique_zip_path(path, &mut used_paths);
    path_by_view.insert(view.id.to_string(), path);
  }

  for (view, folder, base) in pages {
    if view.object_type == "folder" {
      // A folder is a pure container: emit NO `.md` (that was the wart), just a
      // manifest entry so the directory — even an empty one — round-trips. Its
      // `path` is the directory its children nest under (folder segments + own
      // deduped name), matching the segments collect_page_paths gave the kids.
      let mut dir = String::new();
      for seg in &folder {
        dir.push_str(seg);
        dir.push('/');
      }
      dir.push_str(&base);
      manifest_pages.push(serde_json::json!({
        "path": dir,
        "title": view.name,
        "type": "folder",
      }));
      continue;
    }
    if view.object_type != "document" {
      continue;
    }
    let Some(snapshot) = store::latest_snapshot(&state.db, view.object_id).await? else {
      continue;
    };
    let Ok(mut payload) = payload_from_value(snapshot.payload) else {
      continue;
    };
    // Internal page links (`mica://page/<viewId>`) become standard relative
    // markdown links to the target's .md inside the archive.
    rewrite_page_links(&mut payload, folder.len(), &path_by_view);

    // Image assets used by this page (de-duplicated globally by object key).
    let rel = "../".repeat(folder.len());
    let mut images: BTreeMap<String, String> = BTreeMap::new();
    if let Some(storage) = &storage {
      let mut wanted: Vec<(String, String)> = Vec::new();
      for b in &payload.blocks {
        if b.kind != "image" {
          continue;
        }
        if let Some(id) = b.data.get("file_id").and_then(|v| v.as_str()) {
          if !wanted.iter().any(|(w, _)| w == id) {
            let name = b
              .data
              .get("name")
              .and_then(|v| v.as_str())
              .unwrap_or("image")
              .to_string();
            wanted.push((id.to_string(), name));
          }
        }
      }
      if !wanted.is_empty() {
        let ids: Vec<Uuid> = wanted
          .iter()
          .filter_map(|(id, _)| Uuid::parse_str(id).ok())
          .collect();
        let records = store::fetch_files(&state.db, workspace_id, &ids).await?;
        let by_id: std::collections::HashMap<String, &store::FileRecord> =
          records.iter().map(|r| (r.id.to_string(), r)).collect();
        for (file_id, name) in &wanted {
          let Some(record) = by_id.get(file_id) else {
            continue;
          };
          let asset = if let Some(existing) = asset_by_object.get(&record.object_key) {
            existing.clone()
          } else {
            let bytes = match client.get(storage.download_url(&record.object_key)).send().await {
              Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(b) => b.to_vec(),
                Err(_) => continue,
              },
              _ => continue,
            };
            let a = unique_asset_name(name, &mut used_assets);
            entries.push(ZipEntry {
              name: format!("assets/{a}"),
              data: bytes,
            });
            asset_by_object.insert(record.object_key.clone(), a.clone());
            a
          };
          images.insert(file_id.clone(), format!("{rel}assets/{asset}"));
        }
      }
    }

    let body = export_markdown_with_assets(&payload, &images)
      .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let Some(path) = path_by_view.get(&view.id.to_string()).cloned() else {
      continue;
    };
    manifest_pages.push(serde_json::json!({
      "path": path,
      "title": view.name,
      "type": "document",
    }));
    let content = format!("# {}\n\n{}", view.name, body);
    entries.push(ZipEntry {
      name: path,
      data: content.into_bytes(),
    });
  }

  if !manifest_pages.is_empty() {
    let manifest = serde_json::json!({
      "version": 1,
      "generator": "mica",
      "pages": manifest_pages,
    });
    entries.insert(
      0,
      ZipEntry {
        name: "manifest.json".to_string(),
        data: serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
      },
    );
  }

  if entries.is_empty() {
    entries.push(ZipEntry {
      name: "README.md".to_string(),
      data: b"(empty workspace)".to_vec(),
    });
  }
  Ok(zip_response(build_zip(&entries), "workspace.zip"))
}

/// Rewrite internal page links (`mica://page/<viewId>`) in link marks to
/// relative paths of the target page's `.md` inside the archive, so the
/// exported markdown is fully standard. Links to pages outside the archive
/// keep their `mica://` href.
fn rewrite_page_links(
  payload: &mut DocumentSnapshotPayload,
  folder_depth: usize,
  path_by_view: &std::collections::HashMap<String, String>,
) {
  const SCHEME: &str = "mica://page/";
  for block in &mut payload.blocks {
    let Some(marks) = block.data.get_mut("marks").and_then(serde_json::Value::as_array_mut)
    else {
      continue;
    };
    for mark in marks {
      let Some(obj) = mark.as_object_mut() else {
        continue;
      };
      let Some(target) = obj
        .get("href")
        .and_then(serde_json::Value::as_str)
        .and_then(|href| href.strip_prefix(SCHEME))
        .and_then(|id| path_by_view.get(id))
      else {
        continue;
      };
      let rel = format!("{}{target}", "../".repeat(folder_depth));
      obj.insert("href".into(), serde_json::json!(rel));
    }
  }
}

/// Flatten the page tree into `(view, ancestor-folder segments, unique base)`,
/// in tree order, giving each page a name unique among its siblings.
fn collect_page_paths<'a>(
  by_parent: &std::collections::HashMap<Option<Uuid>, Vec<&'a View>>,
  parent: Option<Uuid>,
  folder: &[String],
  out: &mut Vec<(&'a View, Vec<String>, String)>,
) {
  let Some(children) = by_parent.get(&parent) else {
    return;
  };
  let mut used = std::collections::HashSet::new();
  for child in children {
    let base = unique_zip_path(safe_segment(&child.name), &mut used);
    out.push((child, folder.to_vec(), base.clone()));
    let mut sub = folder.to_vec();
    sub.push(base);
    collect_page_paths(by_parent, Some(child.id), &sub, out);
  }
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
  if tidy.is_empty() { "untitled".to_string() } else { tidy }
}

/// Make [candidate] unique within [used], appending `-2`, `-3`… before any
/// `.md` extension on collision. Inserts the result into [used].
fn unique_zip_path(candidate: String, used: &mut std::collections::HashSet<String>) -> String {
  if used.insert(candidate.clone()) {
    return candidate;
  }
  let (stem, ext) = match candidate.strip_suffix(".md") {
    Some(s) => (s.to_string(), ".md"),
    None => (candidate.clone(), ""),
  };
  let mut n = 2;
  loop {
    let next = format!("{stem}-{n}{ext}");
    if used.insert(next.clone()) {
      return next;
    }
    n += 1;
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

    if let Some(snapshot) = store::latest_snapshot(db, view.object_id).await? {
      if let Ok(payload) = payload_from_value(snapshot.payload) {
        if let Ok(markdown) = export_markdown(&payload) {
          let body = markdown.trim();
          if !body.is_empty() {
            out.push_str(body);
            out.push_str("\n\n");
          }
        }
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

pub async fn export_document_html(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<HtmlExportResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let snapshot = store::latest_snapshot(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let payload = payload_from_value(snapshot.payload)
    .map_err(|error| ApiError::BadRequest(format!("invalid document snapshot: {error}")))?;
  let html = export_html(&payload).map_err(|error| ApiError::BadRequest(error.to_string()))?;

  Ok(Json(HtmlExportResponse { html }))
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

  ensure_view_in_workspace(db, workspace_id, parent_view_id).await?;

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

#[cfg(test)]
mod tests {
  use super::*;

  fn view(id: u128, parent: Option<u128>, name: &str, otype: &str) -> View {
    let now = Utc::now();
    View {
      id: Uuid::from_u128(id),
      workspace_id: Uuid::from_u128(1),
      parent_view_id: parent.map(Uuid::from_u128),
      object_id: Uuid::from_u128(1000 + id),
      object_type: otype.to_string(),
      name: name.to_string(),
      icon: None,
      position: format!("{id:020}"),
      is_deleted: false,
      created_by: Uuid::from_u128(2),
      created_at: now,
      updated_at: now,
    }
  }

  /// The export path builder treats folders like any other tree node: a folder
  /// contributes its (deduped) name as a directory segment to its children, and
  /// every node — folder or document — appears in pre-order. This is what lets
  /// the export loop render a folder as a directory path (no `.md`) whose
  /// children nest under it (F1).
  #[test]
  fn collect_page_paths_nests_children_under_a_folder() {
    let chapter = view(10, None, "Chapter", "folder");
    let intro = view(11, Some(10), "Intro", "document");
    let sub = view(12, Some(10), "Sub", "folder"); // empty child folder
    let deep = view(13, Some(12), "Deep", "document");
    let all = [&chapter, &intro, &sub, &deep];

    let mut by_parent: std::collections::HashMap<Option<Uuid>, Vec<&View>> =
      std::collections::HashMap::new();
    for v in all {
      by_parent.entry(v.parent_view_id).or_default().push(v);
    }
    for list in by_parent.values_mut() {
      list.sort_by(|a, b| a.position.cmp(&b.position));
    }

    let mut pages: Vec<(&View, Vec<String>, String)> = Vec::new();
    collect_page_paths(&by_parent, None, &Vec::new(), &mut pages);

    // Pre-order, every node present (folders included).
    let names: Vec<&str> = pages.iter().map(|(v, _, _)| v.name.as_str()).collect();
    assert_eq!(names, ["Chapter", "Intro", "Sub", "Deep"]);

    // The folder "Chapter" contributes its segment to its children; the empty
    // folder "Sub" nests under "Chapter" and passes both segments to "Deep".
    let seg = |n: &str| {
      let (_, folder, base) = pages.iter().find(|(v, _, _)| v.name == n).unwrap();
      (folder.clone(), base.clone())
    };
    assert_eq!(seg("Chapter"), (vec![], "Chapter".to_string()));
    assert_eq!(seg("Intro"), (vec!["Chapter".to_string()], "Intro".to_string()));
    assert_eq!(seg("Sub"), (vec!["Chapter".to_string()], "Sub".to_string()));
    assert_eq!(
      seg("Deep"),
      (vec!["Chapter".to_string(), "Sub".to_string()], "Deep".to_string())
    );
    // => the export loop writes the empty folder "Sub" as directory path
    //    "Chapter/Sub" (manifest type:'folder', no `.md`), and "Deep" as
    //    "Chapter/Sub/Deep.md" — no stray container `.md` anywhere.
  }
}
