use axum::{
  Json,
  extract::{Path, State},
  http::HeaderMap,
};
use chrono::{DateTime, Utc};
use mica_app_core::{
  AppState,
  documents::{
    DocumentOperation, export_html, export_markdown, import_markdown, payload_from_value,
  },
  store::{self, DocumentRecord, SnapshotRecord, UpdateRecord},
};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::ws;

#[derive(Debug, Deserialize)]
pub struct CreateDocumentRequest {
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
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let views = fetch_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
}

pub async fn create_document(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<CreateDocumentRequest>,
) -> ApiResult<Json<DocumentCreateResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
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
  let user_id = user_id_from_headers(&state, &headers)?;
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
  let user_id = user_id_from_headers(&state, &headers)?;
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
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  let result = sqlx::query(
    r#"
      UPDATE views
      SET is_deleted = true, updated_at = now()
      WHERE id = $1 AND workspace_id = $2 AND is_deleted = false
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

pub async fn move_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<MoveViewRequest>,
) -> ApiResult<Json<ViewResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
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
  let user_id = user_id_from_headers(&state, &headers)?;
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
  let user_id = user_id_from_headers(&state, &headers)?;
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
  let user_id = user_id_from_headers(&state, &headers)?;
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

pub async fn export_document_html(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<HtmlExportResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
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
