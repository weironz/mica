use axum::{
  Json,
  extract::{Path, State},
  http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use mica_app_core::{AppState, store};
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

  /// Live pages in this workspace — folders excluded, recycle bin excluded.
  ///
  /// Same definition the client's `countPages` uses, deliberately: the sidebar
  /// switcher already shows a count for the OPEN workspace derived from its
  /// loaded view tree, and a list endpoint answering with a different number for
  /// the same workspace would be worse than answering with none. Folders are not
  /// pages (a tree of 12 rows where 4 are folders must not claim 12 pages — the
  /// user can count them), and trashed rows are not either.
  ///
  /// Only populated by [`list`]; the single-workspace handlers leave it 0 because
  /// nothing reads it there.
  ///
  /// `sqlx(default)` is what actually makes that true. `serde(default)` alone
  /// governs DESERIALIZING json, which nothing here does — the derived `FromRow`
  /// still demanded the column, so every query that omits it (create / get /
  /// update, via `fetch_workspace_for_user{,_in_tx}`) failed to decode and the
  /// handler answered 500 "no column found for name: page_count". Creating a
  /// workspace has been broken since the initial commit.
  #[serde(default)]
  #[sqlx(default)]
  page_count: i64,
}

#[derive(Debug, Serialize, FromRow)]
pub struct WorkspaceMember {
  user_id: Uuid,
  email: String,
  display_name: String,
  role: String,
  joined_at: DateTime<Utc>,
  /// Serialized as `avatar_version` through the ONE implementation of that
  /// derivation (avatar.rs) — deriving it a second time in SQL would be two
  /// definitions of the same token, free to drift the day the key layout moves.
  /// Non-null when this member has a picture; the client builds the URL from
  /// `user_id`. Sent here so the member list does not fire a request per member
  /// that 404s for everyone who has not set one.
  #[serde(
    rename = "avatar_version",
    serialize_with = "crate::routes::avatar::serialize_version"
  )]
  avatar_key: Option<String>,
}

/// Zero-padded numeric position: lexical order == numeric order.
pub(crate) fn pad_position(n: i64) -> String {
  format!("{n:010}")
}

/// The next member `position` for `user_id` — one step past their current max,
/// so a newly-added workspace lands at the END of that user's switcher. Reads
/// committed memberships (the caller's own new row isn't inserted yet), so the
/// pool is fine even mid-transaction.
pub(crate) async fn next_member_position(db: &PgPool, user_id: Uuid) -> ApiResult<String> {
  let max: Option<i64> = sqlx::query_scalar(
    "SELECT max(nullif(position, '')::bigint) FROM workspace_members WHERE user_id = $1",
  )
  .bind(user_id)
  .fetch_one(db)
  .await?;
  Ok(pad_position(max.unwrap_or(0) + 10))
}

#[derive(Debug, Deserialize)]
pub struct ReorderWorkspacesRequest {
  workspace_ids: Vec<Uuid>,
}

/// Persist the user's drag-reordered workspace list: renumber their membership
/// positions to match `workspace_ids` order. Per-user — only rows where the
/// caller is a member are touched; unknown ids no-op.
pub async fn reorder(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<ReorderWorkspacesRequest>,
) -> ApiResult<Json<serde_json::Value>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let mut tx = state.db.begin().await?;
  for (i, wid) in payload.workspace_ids.iter().enumerate() {
    sqlx::query(
      "UPDATE workspace_members SET position = $1 WHERE workspace_id = $2 AND user_id = $3",
    )
    .bind(pad_position((i as i64 + 1) * 10))
    .bind(wid)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
  }
  tx.commit().await?;
  Ok(Json(serde_json::json!({ "ok": true })))
}

/// The workspace list query, shared with its test so what gets asserted is the
/// statement production runs — not a second copy of it that can drift.
pub(crate) const LIST_WORKSPACES_SQL: &str = r#"
      SELECT
        w.id,
        w.name,
        w.owner_id,
        wm.role::text AS role,
        w.created_at,
        w.updated_at,
        -- Correlated rather than a GROUP BY: a workspace with no pages still has
        -- to come back (as 0), and a join would have to be a LEFT JOIN with a
        -- COALESCE to say the same thing.
        (
          SELECT count(*)
          FROM views v
          WHERE v.workspace_id = w.id
            AND v.is_deleted = false
            AND v.object_type::text = 'document'
        )::bigint AS page_count
      FROM workspaces w
      INNER JOIN workspace_members wm ON wm.workspace_id = w.id
      WHERE wm.user_id = $1
      ORDER BY wm.position ASC, w.created_at ASC
"#;

pub async fn list(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<WorkspaceListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;

  let workspaces = sqlx::query_as::<_, Workspace>(LIST_WORKSPACES_SQL)
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
  let user_id = user_id_from_headers(&state, &headers).await?;
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

  // New workspace goes to the END of this user's switcher (next position after
  // their current max). Positions are zero-padded numeric text (lexical order ==
  // numeric order).
  let next_pos = next_member_position(&state.db, user_id).await?;
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
  let user_id = user_id_from_headers(&state, &headers).await?;
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
  let user_id = user_id_from_headers(&state, &headers).await?;
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

pub async fn delete(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
  let user_id = user_id_from_headers(&state, &headers).await?;

  // Only the owner may delete a workspace; the cascade removes its members,
  // views, documents, and history.
  let role = workspace_role(&state.db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  if role != "owner" {
    return Err(ApiError::Forbidden);
  }

  // Preserve every key before the cascade removes its only file row. A failed
  // storage DELETE (or a crash before sending it) is then retried by pending
  // upload GC. The ledger and workspace deletion commit atomically.
  let keys = delete_workspace_records(
    &state.db,
    workspace_id,
    state.storage.as_ref().map_or(7 * 24 * 60 * 60, |s| s.presign_ttl_seconds),
  )
  .await?;

  if let Some(storage) = &state.storage {
    delete_objects(storage, keys.iter().map(|key| (workspace_id, key.as_str()))).await;
  }

  Ok(StatusCode::NO_CONTENT)
}

/// Keep a durable recovery path for every file before removing its workspace.
/// `FOR UPDATE` stops a concurrent file INSERT's FK check until the cascade has
/// finished; listing first without it can miss a just-completed upload. A
/// listing or ledger failure rolls the entire deletion back rather than losing
/// the last known object key.
async fn delete_workspace_records(
  db: &PgPool,
  workspace_id: Uuid,
  presign_ttl_seconds: u64,
) -> ApiResult<Vec<String>> {
  let mut tx = db.begin().await?;
  let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM workspaces WHERE id = $1 FOR UPDATE")
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?;
  if exists.is_none() {
    return Err(ApiError::NotFound);
  }

  let keys: Vec<String> = sqlx::query_scalar(
    "SELECT object_key FROM files WHERE workspace_id = $1 ORDER BY object_key",
  )
  .bind(workspace_id)
  .fetch_all(&mut *tx)
  .await?;
  for key in &keys {
    // We hold the workspace row lock, so a new `files` INSERT cannot pass its
    // FK check until this transaction commits. Do not take the key advisory
    // lock here: `complete` takes key before FK, and reversing that order
    // would deadlock. GC's registered-row cleanup conditionally deletes only
    // the pending version it observed, so this UPSERT cannot be lost to it.
    store::record_pending_file_upload_tx(&mut tx, workspace_id, key, presign_ttl_seconds).await?;
  }
  sqlx::query("DELETE FROM workspaces WHERE id = $1")
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?;
  tx.commit().await?;
  Ok(keys)
}

pub(crate) async fn delete_objects<'a>(
  storage: &mica_infra::storage::S3Config,
  keys: impl Iterator<Item = (Uuid, &'a str)>,
) {
  let http = reqwest::Client::new();
  for (workspace_id, key) in keys {
    // S3 deletion is an immediate best effort. The committed ledger remains
    // available to GC even when this request is interrupted or S3 rejects it.
    match http.delete(storage.presign_delete(key)).send().await {
      Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 404 => {}
      Ok(resp) => {
        tracing::warn!(%workspace_id, object_key = %key, status = %resp.status(), "workspace delete: object delete rejected, pending GC retry")
      }
      Err(error) => {
        tracing::warn!(%workspace_id, object_key = %key, %error, "workspace delete: object delete failed, pending GC retry")
      }
    }
  }
}

#[derive(Debug, Serialize)]
pub struct WorkspaceUsageResponse {
  /// Bytes this workspace occupies.
  pub bytes_used: i64,
  /// The limit in force, or `0` when quotas are disabled.
  pub quota_bytes: i64,
  /// The per-file upload cap — a SECOND limit, unrelated to the quota above.
  ///
  /// Sent here so a rejected upload can name the number it exceeded. The client
  /// otherwise learns this only from a SUCCESSFUL presign, which is exactly the
  /// call that fails when the file is too big; the alternative was digging the
  /// figure out of the error's English prose, which is the message-matching this
  /// codebase refuses to do.
  ///
  /// `0` means unknown — this server has no storage configured, so there is no
  /// cap because there are no uploads at all. Same convention as `quota_bytes`
  /// and the workspace list's `page_count`: 0 is "no answer", not "zero bytes".
  pub max_upload_bytes: i64,
}

/// `GET /workspaces/{workspace_id}/usage`
///
/// What a member is allowed to know about their own storage. A separate call
/// rather than a field on the workspace LIST, because the list is fetched on
/// every start and every switch, and a `sum(byte_size)` per workspace does not
/// belong on that path for a number nobody is looking at yet.
///
/// `bytes_used` goes through the SAME function the upload path checks against
/// (`store::workspace_bytes_used`) rather than a second query saying the same
/// thing — a "you have room" screen that disagrees with the refusal you get on
/// upload would be worse than no screen at all.
pub async fn workspace_usage(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<WorkspaceUsageResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;

  Ok(Json(WorkspaceUsageResponse {
    bytes_used: mica_app_core::store::workspace_bytes_used(&state.db, workspace_id).await?,
    quota_bytes: state.config.workspace_quota_bytes,
    // Read straight off the optional config rather than through
    // `files::storage`, which turns a missing one into a 503: a server without
    // object storage still has a real quota answer, and failing the whole screen
    // over the field that is merely nice to have would be backwards.
    max_upload_bytes: state.storage.as_ref().map_or(0, |s| s.max_upload_bytes),
  }))
}

pub async fn list_members(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<WorkspaceMemberListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
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
  let actor_id = user_id_from_headers(&state, &headers).await?;
  ensure_can_manage_members(&state.db, workspace_id, actor_id).await?;

  let email = normalize_email(&payload.email)?;
  let role = normalize_member_role(&payload.role)?;
  let member_user_id = user_id_by_email(&state.db, &email)
    .await?
    .ok_or(ApiError::NotFound)?;

  // New member: the workspace lands at the END of THEIR switcher. An existing
  // member (re-invite / role change) keeps their position — only role updates.
  let member_pos = next_member_position(&state.db, member_user_id).await?;
  sqlx::query(
    r#"
      INSERT INTO workspace_members (workspace_id, user_id, role, position)
      VALUES ($1, $2, $3::workspace_role, $4)
      ON CONFLICT (workspace_id, user_id) DO UPDATE
      SET role = EXCLUDED.role
    "#,
  )
  .bind(workspace_id)
  .bind(member_user_id)
  .bind(role)
  .bind(member_pos)
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
  let actor_id = user_id_from_headers(&state, &headers).await?;
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
  let actor_id = user_id_from_headers(&state, &headers).await?;
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
        wm.joined_at,
        u.avatar_key
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
        wm.joined_at,
        u.avatar_key
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

#[cfg(test)]
mod usage_wire {
  use super::*;

  /// The `/usage` body's FIELD NAMES are a contract with
  /// `clients/mica_flutter/lib/api/client.dart`, which indexes them as string
  /// literals (`response['max_upload_bytes']`). Renaming one here compiles
  /// fine and breaks nothing visibly: the Dart side reads null, falls back to
  /// 0, and the upload error quietly drops to the vaguer sentence with no
  /// number — a regression nobody would notice until a user complained twice.
  ///
  /// `max_upload_bytes` in particular is the per-file cap, NOT the quota. It
  /// rides on this response because the only other place the server states it
  /// is a successful presign, and the presign for an oversized file is the one
  /// that fails.
  #[test]
  fn the_usage_body_keeps_the_keys_the_client_indexes() {
    let json = serde_json::to_value(WorkspaceUsageResponse {
      bytes_used: 3500,
      quota_bytes: 5_368_709_120,
      max_upload_bytes: 26_214_400,
    })
    .unwrap();

    assert_eq!(json["bytes_used"], 3500);
    assert_eq!(json["quota_bytes"], 5_368_709_120i64);
    assert_eq!(json["max_upload_bytes"], 26_214_400);

    // 0 is the "no answer" sentinel on every one of these — a server without
    // storage configured, or with quotas off. The client must be able to tell
    // that apart from a real limit, so it has to survive serialization as 0
    // rather than being skipped.
    let empty = serde_json::to_value(WorkspaceUsageResponse {
      bytes_used: 0,
      quota_bytes: 0,
      max_upload_bytes: 0,
    })
    .unwrap();
    assert_eq!(empty["max_upload_bytes"], 0);
    assert!(
      empty.get("max_upload_bytes").is_some(),
      "the field must be present-and-zero, not omitted"
    );
  }
}

#[cfg(test)]
mod workspace_decode_pg {
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

  /// `POST /api/workspaces` answered 500 from the initial commit until
  /// 2026-07-30: `Workspace::page_count` carried `#[serde(default)]`, which says
  /// nothing about sqlx, so the derived `FromRow` demanded a column the
  /// single-workspace queries never select — "no column found for name:
  /// page_count". Creating a workspace, opening one, renaming one: all 500.
  ///
  /// A unit test could not have caught it (the decode only happens against a
  /// real row) and no existing DB test decoded `Workspace` from the short SELECT,
  /// which is exactly how it survived every release. So decode both shapes here:
  /// the one WITH `page_count` (list) and the one without (create/get/update).
  #[tokio::test]
  async fn a_workspace_decodes_from_queries_that_omit_page_count() {
    let Some(db) = pool().await else { return };
    let user = Uuid::new_v4();
    let ws = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
      .bind(user)
      .bind(format!("{user}@t.dev"))
      .execute(&db)
      .await
      .unwrap();
    sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'W',$2)")
      .bind(ws)
      .bind(user)
      .execute(&db)
      .await
      .unwrap();
    sqlx::query(
      "INSERT INTO workspace_members(workspace_id,user_id,role,position)
       VALUES($1,$2,'owner','0000000010')",
    )
    .bind(ws)
    .bind(user)
    .execute(&db)
    .await
    .unwrap();

    let short = fetch_workspace_for_user(&db, ws, user)
      .await
      .expect("the create/get/update SELECT must decode")
      .expect("the row exists");
    assert_eq!(short.page_count, 0, "absent column means 0, not an error");

    let mut tx = db.begin().await.unwrap();
    let in_tx = fetch_workspace_for_user_in_tx(&mut tx, ws, user)
      .await
      .expect("the create SELECT must decode")
      .expect("the row exists");
    tx.commit().await.unwrap();
    assert_eq!(in_tx.page_count, 0);

    let listed = sqlx::query_as::<_, Workspace>(LIST_WORKSPACES_SQL)
      .bind(user)
      .fetch_all(&db)
      .await
      .expect("the list SELECT must still decode");
    assert_eq!(
      listed.len(),
      1,
      "the default must not shadow the real column"
    );

    sqlx::query("DELETE FROM workspaces WHERE id=$1")
      .bind(ws)
      .execute(&db)
      .await
      .ok();
    sqlx::query("DELETE FROM users WHERE id=$1")
      .bind(user)
      .execute(&db)
      .await
      .ok();
  }
}

#[cfg(test)]
mod workspace_delete_gc_pg {
  use super::*;
  use std::time::Duration;

  async fn fixture() -> Option<(PgPool, Uuid, Uuid)> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
      assert!(std::env::var("CI").is_err(), "DATABASE_URL must be set in CI");
      return None;
    };
    let db = PgPool::connect(&url).await.expect("test Postgres must be reachable");
    mica_infra::run_migrations(&db).await.unwrap();
    let user = Uuid::new_v4();
    let workspace = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'GC','x')")
      .bind(user)
      .bind(format!("{user}@workspace-delete-gc.test"))
      .execute(&db)
      .await
      .unwrap();
    sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'GC',$2)")
      .bind(workspace)
      .bind(user)
      .execute(&db)
      .await
      .unwrap();
    Some((db, user, workspace))
  }

  async fn file(db: &PgPool, user: Uuid, workspace: Uuid, key: &str) {
    sqlx::query(
      "INSERT INTO files(workspace_id,uploaded_by,object_key,original_name,mime_type,byte_size) \
       VALUES($1,$2,$3,'x.png','image/png',3)",
    )
    .bind(workspace)
    .bind(user)
    .bind(key)
    .execute(db)
    .await
    .unwrap();
  }

  async fn cleanup(db: &PgPool, user: Uuid, workspace: Uuid) {
    sqlx::query("DELETE FROM pending_file_uploads WHERE workspace_id = $1")
      .bind(workspace)
      .execute(db)
      .await
      .unwrap();
    sqlx::query("DELETE FROM workspaces WHERE id = $1")
      .bind(workspace)
      .execute(db)
      .await
      .unwrap();
    sqlx::query("DELETE FROM users WHERE id = $1")
      .bind(user)
      .execute(db)
      .await
      .unwrap();
  }

  /// A storage-less server still has old file rows to protect. The ledger
  /// must commit in the same transaction that cascades those rows away.
  #[tokio::test]
  async fn deleting_without_storage_preserves_every_object_key_for_gc() {
    let Some((db, user, workspace)) = fixture().await else { return };
    let key = format!("workspaces/{workspace}/{}.png", "a".repeat(64));
    file(&db, user, workspace, &key).await;

    let keys = delete_workspace_records(&db, workspace, 7 * 24 * 60 * 60)
      .await
      .unwrap();
    assert_eq!(keys, vec![key.clone()]);
    let workspace_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces WHERE id = $1")
      .bind(workspace)
      .fetch_one(&db)
      .await
      .unwrap();
    let file_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM files WHERE object_key = $1")
      .bind(&key)
      .fetch_one(&db)
      .await
      .unwrap();
    let pending_rows: i64 = sqlx::query_scalar(
      "SELECT count(*) FROM pending_file_uploads WHERE object_key = $1 AND workspace_id = $2",
    )
    .bind(&key)
    .bind(workspace)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!((workspace_rows, file_rows, pending_rows), (0, 0, 1));
    cleanup(&db, user, workspace).await;
  }

  /// An unexpected legacy key cannot be entered into the constrained ledger.
  /// In that case keeping the workspace is safer than deleting its last index.
  #[tokio::test]
  async fn ledger_failure_rolls_back_workspace_deletion() {
    let Some((db, user, workspace)) = fixture().await else { return };
    let key = format!("legacy/{}.png", Uuid::new_v4());
    file(&db, user, workspace, &key).await;

    assert!(delete_workspace_records(&db, workspace, 900).await.is_err());
    let workspace_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces WHERE id = $1")
      .bind(workspace)
      .fetch_one(&db)
      .await
      .unwrap();
    let file_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM files WHERE object_key = $1")
      .bind(&key)
      .fetch_one(&db)
      .await
      .unwrap();
    assert_eq!((workspace_rows, file_rows), (1, 1));
    cleanup(&db, user, workspace).await;
  }

  /// GC may have read an expired ledger for a registered file just as workspace
  /// deletion refreshes that ledger and cascades the file row. The stale GC
  /// DELETE blocks on the UPSERT's row lock, then must not erase the refresh.
  #[tokio::test]
  async fn stale_gc_prune_cannot_erase_a_workspace_deletion_ledger() {
    let Some((db, user, workspace)) = fixture().await else { return };
    let key = format!("workspaces/{workspace}/{}.png", "b".repeat(64));
    file(&db, user, workspace, &key).await;
    store::record_pending_file_upload(&db, workspace, &key, 900)
      .await
      .unwrap();
    sqlx::query(
      "UPDATE pending_file_uploads SET created_at = now() - interval '31 days', \
       expires_at = now() - interval '2 days' WHERE object_key = $1",
    )
    .bind(&key)
    .execute(&db)
    .await
    .unwrap();

    let mut gc = db.begin().await.unwrap();
    store::lock_file_object_key(&mut gc, &key).await.unwrap();
    let observed: (DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
      "SELECT created_at, expires_at FROM pending_file_uploads WHERE object_key = $1",
    )
    .bind(&key)
    .fetch_one(&mut *gc)
    .await
    .unwrap();
    let registered: bool = sqlx::query_scalar(
      "SELECT EXISTS (SELECT 1 FROM files WHERE object_key = $1)",
    )
    .bind(&key)
    .fetch_one(&mut *gc)
    .await
    .unwrap();
    assert!(registered, "GC must take its registered-file branch");

    // Same operations and lock order as delete_workspace_records, paused
    // before commit so the GC DELETE really waits on the updated ledger row.
    let mut deleting = db.begin().await.unwrap();
    sqlx::query("SELECT id FROM workspaces WHERE id = $1 FOR UPDATE")
      .bind(workspace)
      .fetch_one(&mut *deleting)
      .await
      .unwrap();
    store::record_pending_file_upload_tx(&mut deleting, workspace, &key, 900)
      .await
      .unwrap();
    sqlx::query("DELETE FROM workspaces WHERE id = $1")
      .bind(workspace)
      .execute(&mut *deleting)
      .await
      .unwrap();

    let gc_key = key.clone();
    let observed_expiry = observed.1;
    let mut prune = tokio::spawn(async move {
      let affected = crate::blob_gc::prune_registered_pending_upload_tx(
        &mut gc,
        &gc_key,
        observed.0,
        observed.1,
      )
      .await
      .unwrap();
      gc.commit().await.unwrap();
      affected
    });
    assert!(tokio::time::timeout(Duration::from_millis(100), &mut prune).await.is_err());
    deleting.commit().await.unwrap();
    assert_eq!(prune.await.unwrap(), 0, "stale GC must not delete the refreshed ledger");
    let current_expiry: DateTime<Utc> = sqlx::query_scalar(
      "SELECT expires_at FROM pending_file_uploads WHERE object_key = $1",
    )
    .bind(&key)
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(current_expiry > observed_expiry);
    let remaining_files: i64 = sqlx::query_scalar("SELECT count(*) FROM files WHERE object_key = $1")
      .bind(&key)
      .fetch_one(&db)
      .await
      .unwrap();
    assert_eq!(remaining_files, 0);
    cleanup(&db, user, workspace).await;
  }

  /// Run explicitly with disposable DATABASE_URL and S3_* pointing at RustFS.
  /// The test deliberately signs DELETE with the wrong secret: the object must
  /// survive and remain discoverable after its `files` row has cascaded away.
  #[tokio::test]
  #[ignore = "requires disposable Postgres and S3-compatible object store"]
  async fn failed_rustfs_delete_keeps_a_durable_gc_key() {
    let Some((db, user, workspace)) = fixture().await else { return };
    let storage = mica_infra::storage::S3Config::from_env().expect("set S3_* for RustFS");
    crate::bucket::ensure_bucket(&storage).await;
    let key = format!("workspaces/{workspace}/{}.png", "c".repeat(64));
    let http = reqwest::Client::new();
    let put = http
      .put(storage.presign_put_server(&key).url)
      .body("abc")
      .send()
      .await
      .unwrap();
    assert!(put.status().is_success(), "test PUT: {}", put.status());
    file(&db, user, workspace, &key).await;

    let keys = delete_workspace_records(&db, workspace, 900).await.unwrap();
    let mut wrong_credentials = storage.clone();
    wrong_credentials.secret_key = "definitely-not-the-rustfs-secret".into();
    let rejected = http
      .delete(wrong_credentials.presign_delete(&key))
      .send()
      .await
      .unwrap();
    assert_eq!(rejected.status().as_u16(), 403, "the test must exercise rejected DELETE");
    delete_objects(&wrong_credentials, keys.iter().map(|key| (workspace, key.as_str()))).await;
    let head = http.head(storage.presign_head_object(&key)).send().await.unwrap();
    assert!(head.status().is_success(), "failed DELETE must leave object: {}", head.status());
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_file_uploads WHERE object_key = $1")
      .bind(&key)
      .fetch_one(&db)
      .await
      .unwrap();
    assert_eq!(pending, 1, "GC must be able to retry after S3 recovers");

    let remove = http.delete(storage.presign_delete(&key)).send().await.unwrap();
    assert!(remove.status().is_success(), "test cleanup: {}", remove.status());
    cleanup(&db, user, workspace).await;
  }
}
