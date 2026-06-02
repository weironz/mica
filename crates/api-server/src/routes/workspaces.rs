use axum::{
  Json,
  extract::{Path, State},
  http::HeaderMap,
};
use chrono::{DateTime, Utc};
use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceRequest {
  name: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWorkspaceRequest {
  name: String,
}

#[derive(Debug, Deserialize)]
pub struct AddWorkspaceMemberRequest {
  email: String,
  role: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWorkspaceMemberRequest {
  role: String,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceResponse {
  workspace: Workspace,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceListResponse {
  workspaces: Vec<Workspace>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceMemberListResponse {
  members: Vec<WorkspaceMember>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceMemberResponse {
  member: WorkspaceMember,
}

#[derive(Debug, Serialize, FromRow)]
pub struct Workspace {
  id: Uuid,
  name: String,
  owner_id: Uuid,
  role: String,
  created_at: DateTime<Utc>,
  updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct WorkspaceMember {
  user_id: Uuid,
  email: String,
  display_name: String,
  role: String,
  joined_at: DateTime<Utc>,
}

pub async fn list(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<WorkspaceListResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;

  let workspaces = sqlx::query_as::<_, Workspace>(
    r#"
      SELECT
        w.id,
        w.name,
        w.owner_id,
        wm.role::text AS role,
        w.created_at,
        w.updated_at
      FROM workspaces w
      INNER JOIN workspace_members wm ON wm.workspace_id = w.id
      WHERE wm.user_id = $1
      ORDER BY w.created_at ASC
    "#,
  )
  .bind(user_id)
  .fetch_all(&state.db)
  .await?;

  Ok(Json(WorkspaceListResponse { workspaces }))
}

pub async fn create(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<CreateWorkspaceRequest>,
) -> ApiResult<Json<WorkspaceResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  let name = normalize_workspace_name(&payload.name)?;

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

  sqlx::query(
    r#"
      INSERT INTO workspace_members (workspace_id, user_id, role)
      VALUES ($1, $2, 'owner')
    "#,
  )
  .bind(workspace_id)
  .bind(user_id)
  .execute(&mut *tx)
  .await?;

  let workspace = fetch_workspace_for_user_in_tx(&mut tx, workspace_id, user_id)
    .await?
    .ok_or(ApiError::Internal(
      "created workspace was not found".to_string(),
    ))?;

  tx.commit().await?;

  Ok(Json(WorkspaceResponse { workspace }))
}

pub async fn get(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<WorkspaceResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  let workspace = fetch_workspace_for_user(&state.db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  Ok(Json(WorkspaceResponse { workspace }))
}

pub async fn update(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<UpdateWorkspaceRequest>,
) -> ApiResult<Json<WorkspaceResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  let name = normalize_workspace_name(&payload.name)?;

  let role = workspace_role(&state.db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  if !can_update_workspace(&role) {
    return Err(ApiError::Forbidden);
  }

  sqlx::query(
    r#"
      UPDATE workspaces
      SET name = $1, updated_at = now()
      WHERE id = $2
    "#,
  )
  .bind(name)
  .bind(workspace_id)
  .execute(&state.db)
  .await?;

  let workspace = fetch_workspace_for_user(&state.db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  Ok(Json(WorkspaceResponse { workspace }))
}

pub async fn list_members(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<WorkspaceMemberListResponse>> {
  let user_id = user_id_from_headers(&state, &headers)?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  let members = fetch_workspace_members(&state.db, workspace_id).await?;

  Ok(Json(WorkspaceMemberListResponse { members }))
}

pub async fn add_member(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<AddWorkspaceMemberRequest>,
) -> ApiResult<Json<WorkspaceMemberResponse>> {
  let actor_id = user_id_from_headers(&state, &headers)?;
  ensure_can_manage_members(&state.db, workspace_id, actor_id).await?;

  let email = normalize_email(&payload.email)?;
  let role = normalize_member_role(&payload.role)?;
  let member_user_id = user_id_by_email(&state.db, &email)
    .await?
    .ok_or(ApiError::NotFound)?;

  sqlx::query(
    r#"
      INSERT INTO workspace_members (workspace_id, user_id, role)
      VALUES ($1, $2, $3::workspace_role)
      ON CONFLICT (workspace_id, user_id) DO UPDATE
      SET role = EXCLUDED.role
    "#,
  )
  .bind(workspace_id)
  .bind(member_user_id)
  .bind(role)
  .execute(&state.db)
  .await?;

  let member = fetch_workspace_member(&state.db, workspace_id, member_user_id)
    .await?
    .ok_or(ApiError::Internal(
      "workspace member was not found".to_string(),
    ))?;

  Ok(Json(WorkspaceMemberResponse { member }))
}

pub async fn update_member(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, member_user_id)): Path<(Uuid, Uuid)>,
  Json(payload): Json<UpdateWorkspaceMemberRequest>,
) -> ApiResult<Json<WorkspaceMemberResponse>> {
  let actor_id = user_id_from_headers(&state, &headers)?;
  ensure_can_manage_members(&state.db, workspace_id, actor_id).await?;
  ensure_not_workspace_owner(&state.db, workspace_id, member_user_id).await?;

  let role = normalize_member_role(&payload.role)?;

  let result = sqlx::query(
    r#"
      UPDATE workspace_members
      SET role = $1::workspace_role
      WHERE workspace_id = $2 AND user_id = $3
    "#,
  )
  .bind(role)
  .bind(workspace_id)
  .bind(member_user_id)
  .execute(&state.db)
  .await?;

  if result.rows_affected() == 0 {
    return Err(ApiError::NotFound);
  }

  let member = fetch_workspace_member(&state.db, workspace_id, member_user_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  Ok(Json(WorkspaceMemberResponse { member }))
}

pub async fn remove_member(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, member_user_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<WorkspaceMemberListResponse>> {
  let actor_id = user_id_from_headers(&state, &headers)?;
  ensure_can_manage_members(&state.db, workspace_id, actor_id).await?;
  ensure_not_workspace_owner(&state.db, workspace_id, member_user_id).await?;

  let result = sqlx::query(
    r#"
      DELETE FROM workspace_members
      WHERE workspace_id = $1 AND user_id = $2
    "#,
  )
  .bind(workspace_id)
  .bind(member_user_id)
  .execute(&state.db)
  .await?;

  if result.rows_affected() == 0 {
    return Err(ApiError::NotFound);
  }

  let members = fetch_workspace_members(&state.db, workspace_id).await?;

  Ok(Json(WorkspaceMemberListResponse { members }))
}

async fn fetch_workspace_for_user(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<Option<Workspace>> {
  sqlx::query_as::<_, Workspace>(
    r#"
      SELECT
        w.id,
        w.name,
        w.owner_id,
        wm.role::text AS role,
        w.created_at,
        w.updated_at
      FROM workspaces w
      INNER JOIN workspace_members wm ON wm.workspace_id = w.id
      WHERE w.id = $1 AND wm.user_id = $2
    "#,
  )
  .bind(workspace_id)
  .bind(user_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

async fn fetch_workspace_for_user_in_tx(
  tx: &mut Transaction<'_, Postgres>,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<Option<Workspace>> {
  sqlx::query_as::<_, Workspace>(
    r#"
      SELECT
        w.id,
        w.name,
        w.owner_id,
        wm.role::text AS role,
        w.created_at,
        w.updated_at
      FROM workspaces w
      INNER JOIN workspace_members wm ON wm.workspace_id = w.id
      WHERE w.id = $1 AND wm.user_id = $2
    "#,
  )
  .bind(workspace_id)
  .bind(user_id)
  .fetch_optional(&mut **tx)
  .await
  .map_err(ApiError::from)
}

async fn fetch_workspace_members(
  db: &PgPool,
  workspace_id: Uuid,
) -> ApiResult<Vec<WorkspaceMember>> {
  sqlx::query_as::<_, WorkspaceMember>(
    r#"
      SELECT
        u.id AS user_id,
        u.email,
        u.display_name,
        wm.role::text AS role,
        wm.joined_at
      FROM workspace_members wm
      INNER JOIN users u ON u.id = wm.user_id
      WHERE wm.workspace_id = $1
      ORDER BY wm.joined_at ASC
    "#,
  )
  .bind(workspace_id)
  .fetch_all(db)
  .await
  .map_err(ApiError::from)
}

async fn fetch_workspace_member(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<Option<WorkspaceMember>> {
  sqlx::query_as::<_, WorkspaceMember>(
    r#"
      SELECT
        u.id AS user_id,
        u.email,
        u.display_name,
        wm.role::text AS role,
        wm.joined_at
      FROM workspace_members wm
      INNER JOIN users u ON u.id = wm.user_id
      WHERE wm.workspace_id = $1 AND wm.user_id = $2
    "#,
  )
  .bind(workspace_id)
  .bind(user_id)
  .fetch_optional(db)
  .await
  .map_err(ApiError::from)
}

async fn workspace_role(
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

async fn user_id_by_email(db: &PgPool, email: &str) -> ApiResult<Option<Uuid>> {
  sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE email = $1")
    .bind(email)
    .fetch_optional(db)
    .await
    .map_err(ApiError::from)
}

async fn ensure_workspace_member(db: &PgPool, workspace_id: Uuid, user_id: Uuid) -> ApiResult<()> {
  workspace_role(db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  Ok(())
}

async fn ensure_can_manage_members(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<()> {
  let role = workspace_role(db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;

  if !matches!(role.as_str(), "owner" | "admin") {
    return Err(ApiError::Forbidden);
  }

  Ok(())
}

async fn ensure_not_workspace_owner(
  db: &PgPool,
  workspace_id: Uuid,
  user_id: Uuid,
) -> ApiResult<()> {
  let owner_id = sqlx::query_scalar::<_, Uuid>("SELECT owner_id FROM workspaces WHERE id = $1")
    .bind(workspace_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

  if owner_id == user_id {
    return Err(ApiError::Forbidden);
  }

  Ok(())
}

fn normalize_workspace_name(name: &str) -> ApiResult<String> {
  let name = name.trim().to_string();
  if name.is_empty() {
    return Err(ApiError::BadRequest(
      "workspace name is required".to_string(),
    ));
  }

  Ok(name)
}

fn normalize_email(email: &str) -> ApiResult<String> {
  let email = email.trim().to_ascii_lowercase();
  if email.is_empty() || !email.contains('@') {
    return Err(ApiError::BadRequest("valid email is required".to_string()));
  }

  Ok(email)
}

fn normalize_member_role(role: &str) -> ApiResult<String> {
  let role = role.trim().to_ascii_lowercase();
  if matches!(role.as_str(), "admin" | "editor" | "commenter" | "viewer") {
    return Ok(role);
  }

  Err(ApiError::BadRequest(
    "role must be admin, editor, commenter, or viewer".to_string(),
  ))
}

fn can_update_workspace(role: &str) -> bool {
  matches!(role, "owner" | "admin")
}
