//! Comment endpoints (docs/comments-plan.md, Phase 1).
//!
//! Anchors are yrs sticky indexes held in Postgres, so the document's Markdown is
//! never touched. Two consequences shape everything here:
//!
//! - **Creating** a thread needs the live document, to take the sticky index for
//!   the selected range. A range that can't be anchored is rejected rather than
//!   stored as a guess.
//! - **Listing** needs it too, to map each stored anchor back onto current text.
//!   An anchor that no longer resolves (or resolves to nothing — yrs keeps
//!   tombstones, so a deleted range collapses instead of failing) gets ONE more
//!   chance: its saved quote is searched for in the current text and, on a
//!   confident match, the thread is re-anchored in place (`store::rematch` /
//!   `store::reanchor`). Only if that fails is it an ORPHAN: the thread comes
//!   back with `anchor: null` and is flagged in the database, so the client shows
//!   the discussion against its quote and draws no highlight over unrelated
//!   words.
//!
//! Writing is gated on the `commenter` role via `permissions_for_role` — a
//! commenter can comment without being able to edit the document, which is the
//! whole point of that role existing.

use axum::{
  extract::{Path, State},
  http::HeaderMap,
  Json,
};
use mica_app_core::{comments as store, AppState};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::documents::{
  ensure_document_in_workspace, permissions_for_role, workspace_role,
};

#[derive(Debug, Deserialize)]
pub struct CreateThreadRequest {
  pub start_block: String,
  pub start_offset: u32,
  pub end_block: String,
  pub end_offset: u32,
  /// The anchored text as the author saw it — the orphan fallback and the list
  /// preview. A snapshot only; it never drives the anchor.
  #[serde(default)]
  pub quote: String,
  pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct ReplyRequest {
  pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveRequest {
  pub resolved: bool,
}

/// A resolved anchor: where the comment sits in the document RIGHT NOW (UTF-16
/// offsets, matching Dart string indices). Absent when the thread is orphaned.
#[derive(Debug, Serialize)]
pub struct AnchorDto {
  pub start_block: String,
  pub start_offset: u32,
  pub end_block: String,
  pub end_offset: u32,
}

#[derive(Debug, Serialize)]
pub struct CommentDto {
  pub id: Uuid,
  pub author_id: Uuid,
  pub body: String,
  pub created_at: String,
  pub edited_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ThreadDto {
  pub id: Uuid,
  pub status: String,
  pub quote: String,
  pub created_by: Uuid,
  pub created_at: String,
  pub resolved_by: Option<Uuid>,
  pub resolved_at: Option<String>,
  /// null → orphaned (anchor gone): show the thread against `quote`, draw nothing.
  pub anchor: Option<AnchorDto>,
  pub comments: Vec<CommentDto>,
}

#[derive(Debug, Serialize)]
pub struct ThreadsResponse {
  pub threads: Vec<ThreadDto>,
}

fn comment_dto(row: store::CommentRow) -> CommentDto {
  CommentDto {
    id: row.id,
    author_id: row.author_id,
    body: row.body,
    created_at: row.created_at.to_rfc3339(),
    edited_at: row.edited_at.map(|t| t.to_rfc3339()),
  }
}

fn anchor_dto(range: store::CommentRange) -> AnchorDto {
  AnchorDto {
    start_block: range.start_block,
    start_offset: range.start_offset,
    end_block: range.end_block,
    end_offset: range.end_offset,
  }
}

/// Anyone who can read the workspace may READ comments; writing needs
/// `can_comment`. Also pins the document to the workspace in the path, so a
/// member of workspace A cannot reach a thread on workspace B's document.
async fn authorize(
  state: &AppState,
  headers: &HeaderMap,
  workspace_id: Uuid,
  document_id: Uuid,
  need_write: bool,
) -> ApiResult<Uuid> {
  let user_id = user_id_from_headers(state, headers).await?;
  let role = workspace_role(&state.db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let perms = permissions_for_role(&role);
  if !perms.can_read || (need_write && !perms.can_comment) {
    return Err(ApiError::Forbidden);
  }
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  Ok(user_id)
}

/// A thread must belong to the document in the path — otherwise
/// `/documents/{A}/comments/{thread-of-B}` would let a member of one document act
/// on another's.
async fn thread_in_document(
  state: &AppState,
  document_id: Uuid,
  thread_id: Uuid,
) -> ApiResult<store::ThreadRow> {
  let thread = store::fetch_thread(&state.db, thread_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  if thread.document_id != document_id {
    return Err(ApiError::NotFound);
  }
  Ok(thread)
}

/// `POST /api/workspaces/{workspace_id}/documents/{document_id}/comments`
pub async fn create(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Json(req): Json<CreateThreadRequest>,
) -> ApiResult<Json<ThreadDto>> {
  let user_id = authorize(&state, &headers, workspace_id, document_id, true).await?;
  if req.body.trim().is_empty() {
    return Err(ApiError::BadRequest("comment body is empty".into()));
  }

  // The anchor is taken against the CURRENT document. If the range can't be
  // anchored (block gone, offsets past the end — a stale selection), refuse: a
  // comment that can never resolve is worse than a failed request.
  let doc = store::load_doc(&state.db, document_id).await?;
  let anchor = doc
    .sticky_for_range(
      &req.start_block,
      req.start_offset,
      &req.end_block,
      req.end_offset,
    )
    .ok_or_else(|| ApiError::BadRequest("cannot anchor that range".into()))?;

  let (thread, first) = store::create_thread(
    &state.db,
    document_id,
    user_id,
    &anchor,
    &req.quote,
    &req.body,
  )
  .await?;
  let live = doc.resolve_range(&anchor).filter(|r| !r.is_empty());
  Ok(Json(ThreadDto {
    id: thread.id,
    status: thread.status,
    quote: thread.quote,
    created_by: thread.created_by,
    created_at: thread.created_at.to_rfc3339(),
    resolved_by: thread.resolved_by,
    resolved_at: thread.resolved_at.map(|t| t.to_rfc3339()),
    anchor: live.map(anchor_dto),
    comments: vec![comment_dto(first)],
  }))
}

/// `GET /api/workspaces/{workspace_id}/documents/{document_id}/comments`
///
/// Resolves every anchor against the live document and flags newly orphaned
/// threads as a side effect — this is where orphanhood is derived, so a stored
/// status can never drift away from what the text actually says.
pub async fn list(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ThreadsResponse>> {
  authorize(&state, &headers, workspace_id, document_id, false).await?;

  let rows = store::list_threads(&state.db, document_id).await?;
  if rows.is_empty() {
    return Ok(Json(ThreadsResponse {
      threads: Vec::new(),
    }));
  }
  let ids: Vec<Uuid> = rows.iter().map(|t| t.id).collect();
  let replies = store::list_comments(&state.db, &ids).await?;
  let doc = store::load_doc(&state.db, document_id).await?;

  let mut newly_orphaned = Vec::new();
  let mut reanchored: Vec<(Uuid, store::Anchor)> = Vec::new();
  // `&mut None`: the quote index is built on the first thread that needs
  // re-anchoring, so a listing where every anchor resolves pays nothing.
  let mut quotes: Option<store::QuoteIndex> = None;
  let mut threads = Vec::with_capacity(rows.len());
  for row in rows {
    // Dead anchor ≠ dead thread: `set_blocks` (any REST/MCP write, any version
    // restore) replaces every block's text object, killing anchors over text
    // that never changed. `anchor_state` re-anchors from the saved quote when it
    // is confident, and leaves a real deletion an orphan.
    let resolved = store::anchor_state(&doc, &mut quotes, &row);
    if resolved.status == store::STATUS_ORPHANED && row.status != store::STATUS_ORPHANED {
      newly_orphaned.push(row.id);
    }
    if let Some(anchor) = resolved.fresh_anchor {
      reanchored.push((row.id, anchor));
    }
    let comments = replies
      .iter()
      .filter(|c| c.thread_id == row.id)
      .cloned()
      .map(comment_dto)
      .collect();
    threads.push(ThreadDto {
      id: row.id,
      status: resolved.status,
      quote: row.quote,
      created_by: row.created_by,
      created_at: row.created_at.to_rfc3339(),
      resolved_by: row.resolved_by,
      resolved_at: row.resolved_at.map(|t| t.to_rfc3339()),
      anchor: resolved.range.map(anchor_dto),
      comments,
    });
  }
  // Best-effort bookkeeping: the response above is already correct even if these
  // writes fail, so a read never fails because of them (the next listing derives
  // the same answers from the document again).
  let _ = store::mark_orphaned(&state.db, &newly_orphaned).await;
  for (thread_id, anchor) in &reanchored {
    let _ = store::reanchor(&state.db, *thread_id, anchor).await;
  }

  Ok(Json(ThreadsResponse { threads }))
}

/// `POST /api/workspaces/{ws}/documents/{doc}/comments/{thread_id}/reply`
pub async fn reply(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id, thread_id)): Path<(Uuid, Uuid, Uuid)>,
  Json(req): Json<ReplyRequest>,
) -> ApiResult<Json<CommentDto>> {
  let user_id = authorize(&state, &headers, workspace_id, document_id, true).await?;
  if req.body.trim().is_empty() {
    return Err(ApiError::BadRequest("comment body is empty".into()));
  }
  thread_in_document(&state, document_id, thread_id).await?;
  let row = store::add_reply(&state.db, thread_id, user_id, &req.body).await?;
  Ok(Json(comment_dto(row)))
}

/// `POST /api/workspaces/{ws}/documents/{doc}/comments/{thread_id}/resolve`
pub async fn resolve(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id, thread_id)): Path<(Uuid, Uuid, Uuid)>,
  Json(req): Json<ResolveRequest>,
) -> ApiResult<Json<serde_json::Value>> {
  let user_id = authorize(&state, &headers, workspace_id, document_id, true).await?;
  thread_in_document(&state, document_id, thread_id).await?;
  store::set_resolved(&state.db, thread_id, user_id, req.resolved).await?;
  Ok(Json(serde_json::json!({ "resolved": req.resolved })))
}

/// `DELETE /api/workspaces/{ws}/documents/{doc}/comments/{thread_id}`
///
/// The author can always remove their own thread; otherwise it takes write access
/// (an editor moderating). A commenter cannot delete other people's discussions.
pub async fn delete(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id, thread_id)): Path<(Uuid, Uuid, Uuid)>,
) -> ApiResult<Json<serde_json::Value>> {
  let user_id = authorize(&state, &headers, workspace_id, document_id, true).await?;
  let thread = thread_in_document(&state, document_id, thread_id).await?;
  if thread.created_by != user_id {
    let role = workspace_role(&state.db, workspace_id, user_id)
      .await?
      .ok_or(ApiError::NotFound)?;
    if !permissions_for_role(&role).can_write {
      return Err(ApiError::Forbidden);
    }
  }
  store::delete_thread(&state.db, thread_id).await?;
  Ok(Json(serde_json::json!({ "deleted": true })))
}
