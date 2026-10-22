
use axum::{
  Json,
  extract::{Path, State},
  http::HeaderMap,
};
use mica_app_core::{
  AppState,
  documents::{
    DocumentOperation,
    export_markdown_with_assets, import_markdown, import_markdown_fragment,
  },
  store::{self, DocumentRecord},
};
use mica_infra::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::ws;
use super::*;
use super::export::{blob_asset_map, rewire_blob_hrefs};

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

  // A new page owns its yrs base from birth, written in the same transaction as
  // its `documents` row. Before S4 this wrote an op-model snapshot instead and
  // the base was built afterwards, best-effort — so a document could exist for a
  // moment with no content to read, and stayed unsearchable until first open.
  mica_app_core::sync::seed_base_tx(
    &mut tx,
    document.id,
    mica_app_core::sync::empty_payload(&document.root_block_id),
  )
  .await?;

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
  // Seed the yrs base with the imported content, in the import's own
  // transaction. This also makes the body searchable immediately: `content_text`
  // (the FTS index) is co-written with the base, so an imported-but-never-opened
  // page is findable by its body, not just its title.
  mica_app_core::sync::seed_base_tx(&mut tx, document.id, imported.clone()).await?;
  let snapshot = store::AppliedContent {
    version_seq: 0,
    schema_version: imported.schema_version,
    payload: serde_json::to_value(&imported)
      .map_err(|error| ApiError::Internal(error.to_string()))?,
  };

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
  // An emoji is one grapheme; anything longer is either a mistake or someone
  // trying to stuff a label in here. Bound it rather than letting the tree render
  // arbitrary text where an icon goes.
  if let Some(icon) = payload.icon.as_deref() {
    if icon.chars().count() > 8 {
      return Err(ApiError::BadRequest("icon is too long".into()));
    }
  }
  let view = sqlx::query_as::<_, View>(
    r#"
      UPDATE views
      SET name = $1,
          -- NULL = leave alone, '' = clear, otherwise set (see UpdateViewRequest)
          icon = CASE
                   WHEN $4::text IS NULL THEN icon
                   WHEN $4 = '' THEN NULL
                   ELSE $4
                 END,
          updated_at = now()
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
  .bind(payload.icon.as_deref())
  .fetch_optional(&state.db)
  .await?
  .ok_or(ApiError::NotFound)?;

  // A page's name now lives IN its document (root block `title`), with
  // `views.name` as the projection — so a rename has to reach the document, or
  // the next push would re-derive the OLD title over the column and silently
  // undo the rename. See `docs/features/page-title-plan.md`.
  //
  // The column is still written above and first: this endpoint must keep working
  // for folders (no document at all) and for the many pages that have no title
  // yet (there is no backfill). The document write is what makes the rename
  // stick for pages that DO carry one, and what makes open editors converge.
  //
  // Failure here is logged, not returned: the user asked to rename a page and
  // the rename happened. Turning a title-sync hiccup into a 500 would report a
  // rename that actually succeeded as a failure.
  if view.object_type == "document" {
    match mica_app_core::sync::set_document_title(
      &state.db,
      workspace_id,
      view.object_id,
      user_id,
      &view.name,
      &state.config.sync_tuning,
    )
    .await
    {
      // Fan it out to open editors on the same yrs channel a live edit uses —
      // nil origin so every socket receives it. Same shape as a version restore
      // (`history.rs`), because it is the same kind of event: a server-side
      // change to a document somebody may have open.
      Ok((rid, update)) if !update.is_empty() => {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&update);
        let event = serde_json::json!({
          "type": "sync.update",
          "document_id": view.object_id,
          "rid": rid,
          "actor_id": user_id,
          "update": encoded,
        });
        state.hub.broadcast_if_active(
          view.object_id,
          Uuid::nil(),
          std::sync::Arc::from(event.to_string()),
        );
      }
      Ok(_) => {}
      Err(error) => tracing::warn!(
        view_id = %view.id, document_id = %view.object_id, %error,
        "rename: could not write the title into the document"
      ),
    }
  }

  Ok(Json(ViewResponse { view }))
}

pub async fn delete_view(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, view_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ViewListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;

  // Soft-delete (move to the recycle bin) the page and its whole subtree —
  // the batch trash of one id, so both paths stamp `deleted_at` alike.
  if super::trash::trash_views_batch(&state.db, workspace_id, &[view_id]).await?.is_empty() {
    return Err(ApiError::NotFound);
  }

  let views = fetch_workspace_views(&state.db, workspace_id).await?;

  Ok(Json(ViewListResponse { views }))
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

  // Live content, materialized from the yrs base. `version_seq` reports the
  // document's own seq rather than a `document_snapshots` row id: since S4 that
  // table has no current row to report, and the client only ever used this
  // number to tell one read apart from a later one — which `current_seq` does
  // exactly, and honestly.
  let payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let snapshot = store::AppliedContent {
    version_seq: document.current_seq,
    schema_version: payload.schema_version,
    payload: serde_json::to_value(&payload)
      .map_err(|error| ApiError::Internal(error.to_string()))?,
  };

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

#[derive(Debug, Deserialize)]
pub struct BatchReadRequest {
  document_ids: Vec<Uuid>,
}

/// One document's result. Either `markdown` or `error` is present, never both.
#[derive(Debug, Serialize)]
pub struct BatchReadItem {
  document_id: Uuid,
  #[serde(skip_serializing_if = "Option::is_none")]
  markdown: Option<String>,
  /// Why this ONE document could not be read. Inlined rather than failing the
  /// request: a survey of a few hundred pages should not be lost because one of
  /// them was deleted between listing and reading.
  #[serde(skip_serializing_if = "Option::is_none")]
  error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BatchReadResponse {
  documents: Vec<BatchReadItem>,
}

/// `POST /api/workspaces/{workspace_id}/documents/batch-read`
///
/// Read many pages' Markdown in ONE request. Scanning a workspace — "which of
/// these are empty?" — meant a request per page, thousands of them; this makes
/// it one, and checks workspace membership once instead of per document.
///
/// It is one ROUND TRIP, not one query: the documents are still assembled one
/// at a time, because each is a CRDT payload rendered to Markdown separately.
/// The win is the network, which is where the cost actually was.
pub async fn batch_read_documents(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
  Json(payload): Json<BatchReadRequest>,
) -> ApiResult<Json<BatchReadResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  check_batch_ids(&payload.document_ids)?;

  let mut documents = Vec::with_capacity(payload.document_ids.len());
  for document_id in payload.document_ids {
    let rendered = read_one_markdown(&state, workspace_id, document_id).await;
    documents.push(match rendered {
      Ok(markdown) => BatchReadItem {
        document_id,
        markdown: Some(markdown),
        error: None,
      },
      Err(error) => BatchReadItem {
        document_id,
        markdown: None,
        error: Some(error.to_string()),
      },
    });
  }

  Ok(Json(BatchReadResponse { documents }))
}

/// The body of [export_document_markdown] without the HTTP wrapper, so the
/// batch route renders pages exactly the way the single route does.
async fn read_one_markdown(
  state: &AppState,
  workspace_id: Uuid,
  document_id: Uuid,
) -> ApiResult<String> {
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let payload = store::current_payload(&state.db, document_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  let assets = blob_asset_map(&payload.blocks, workspace_id);
  export_markdown_with_assets(&payload, &assets)
    .map_err(|error| ApiError::BadRequest(error.to_string()))
}

#[derive(Debug, Serialize)]
pub struct OutlineHeading {
  pub(crate) block_id: String,
  pub(crate) level: i64,
  pub(crate) text: String,
}

#[derive(Debug, Serialize)]
pub struct DocumentOutlineResponse {
  /// Headings in document order — the anchors an AI names to write in place
  /// (`insert_at`/`find_replace`) instead of rewriting the whole doc.
  pub(crate) headings: Vec<OutlineHeading>,
  /// Every top-level block id in document order (finer anchors than headings).
  pub(crate) block_ids: Vec<String>,
  /// The document's current sequence number. Pass it back as `expected_seq` on a
  /// write to make the write conflict-checked (409 if the doc changed meanwhile).
  pub(crate) seq: i64,
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
  // The current seq lets a caller pin the version it read here and pass it back
  // as `expected_seq` for a conflict-checked write (optimistic concurrency).
  let seq: i64 = sqlx::query_scalar("SELECT current_seq FROM documents WHERE id = $1")
    .bind(document_id)
    .fetch_one(&state.db)
    .await?;
  Ok(Json(outline_from_payload(&payload, seq)))
}

/// Pure: walk the block tree from the root in document order, collecting every
/// top-level block id and (for `heading` blocks) a heading entry. Testable
/// without a DB.
pub(super) fn outline_from_payload(
  payload: &mica_app_core::documents::DocumentSnapshotPayload,
  seq: i64,
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
    seq,
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
  /// Remove the block named by `anchor`, and its subtree (no `markdown`).
  ///
  /// The only way to take anything OUT of a page short of rewriting the whole
  /// body. Without it the four other modes could add and overwrite but never
  /// remove, so "drop this paragraph" meant `replace_all` — re-sending the
  /// entire document to delete one block, which is how a caller that cannot
  /// reproduce the original bytes exactly (an agent retyping a page with
  /// non-breaking spaces or control characters in it) silently damages the
  /// rest of the page.
  Delete,
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
  /// Optimistic concurrency: the `seq` the caller last saw (from the outline or a
  /// prior write ack). When set and the document has moved on, the write is
  /// refused with 409 instead of clobbering the intervening edit. Absent = the
  /// existing last-writer-wins behaviour.
  #[serde(default)]
  pub expected_seq: Option<i64>,
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
  let applied = store::apply_derived_operations(
    &state.db,
    workspace_id,
    document_id,
    user_id,
    request.expected_seq,
    |payload| markdown_update_ops(payload, &request, workspace_id),
  )
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
pub(super) fn markdown_update_ops(
  current: &mica_app_core::documents::DocumentSnapshotPayload,
  request: &UpdateMarkdownRequest,
  workspace_id: Uuid,
) -> Result<Vec<DocumentOperation>, String> {
  let root_id = current.root_block_id.as_str();

  // delete removes an existing block — no markdown parse/graft either.
  if matches!(request.mode, MarkdownUpdateMode::Delete) {
    let anchor = request
      .anchor
      .as_deref()
      .filter(|s| !s.is_empty())
      .ok_or("delete requires an `anchor` block id")?;
    if anchor == root_id {
      return Err("delete cannot remove the document root".to_string());
    }
    if !current.blocks.iter().any(|b| b.id == anchor) {
      return Err(format!("no block {anchor:?} in this document"));
    }
    return Ok(vec![DocumentOperation::DeleteBlock {
      block_id: anchor.to_string(),
    }]);
  }

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
  // FRAGMENT parsing for the modes that graft into an existing document, and
  // whole-document parsing only for the one that IS the whole document.
  //
  // Every mode used to call `import_markdown`, which strips YAML front matter
  // off the front of what it is handed. For a fragment that is a position
  // error: appending `---\nbody\n---\ntail` had `body` lifted out as YAML and
  // dropped on `tmp_root` (thrown away below), so the caller silently got only
  // `tail`. A leading `---` in an appended chunk is a thematic break — that is
  // what the format says and what the caller meant.
  let mut parsed = match request.mode {
    MarkdownUpdateMode::ReplaceAll => import_markdown(&request.markdown, &tmp_root),
    _ => import_markdown_fragment(&request.markdown, &tmp_root),
  };
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
      // Front matter belongs to the DOCUMENT, not to any block, so grafting the
      // parsed children leaves it behind on `tmp_root` — where it used to be
      // thrown away. Exporting a page with properties and replace_all-ing it
      // straight back therefore lost them, silently: a round-trip the dialect
      // is supposed to hold (CLAUDE.md §4). Carry it onto the real root.
      //
      // Only when the incoming markdown HAS front matter: a body-only
      // replace_all must not wipe properties the caller never mentioned.
      // And built from the root's CURRENT data, because UpdateBlock replaces
      // `data` wholesale rather than merging into it.
      if let Some(fm) = parsed
        .blocks
        .iter()
        .find(|b| b.id == tmp_root)
        .and_then(|b| b.data.get("front_matter"))
        .cloned()
      {
        let mut data = current
          .blocks
          .iter()
          .find(|b| b.id == root_id)
          .map(|b| b.data.clone())
          .unwrap_or(serde_json::Value::Null);
        if !data.is_object() {
          data = serde_json::Value::Object(serde_json::Map::new());
        }
        if let Some(map) = data.as_object_mut() {
          map.insert("front_matter".to_string(), fm);
        }
        ops.push(DocumentOperation::UpdateBlock {
          block_id: root_id.to_string(),
          kind: None,
          text: None,
          data: Some(data),
        });
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
    MarkdownUpdateMode::FindReplace | MarkdownUpdateMode::Delete => {
      unreachable!("handled above")
    }
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


#[cfg(test)]
mod markdown_update_tests {
  use super::*;

  /// A one-paragraph document to write into.
  fn current_doc() -> mica_app_core::documents::DocumentSnapshotPayload {
    mica_app_core::documents::DocumentSnapshotPayload {
      schema_version: 1,
      root_block_id: "root".to_string(),
      blocks: vec![
        mica_app_core::documents::Block {
          id: "root".to_string(),
          kind: "paragraph".to_string(),
          text: String::new(),
          data: serde_json::json!({"front_matter": "title: Kept"}),
          children: vec!["p1".to_string()],
        },
        mica_app_core::documents::Block {
          id: "p1".to_string(),
          kind: "paragraph".to_string(),
          text: "existing".to_string(),
          data: serde_json::Value::Null,
          children: Vec::new(),
        },
      ],
    }
  }

  fn request(mode: MarkdownUpdateMode, markdown: &str) -> UpdateMarkdownRequest {
    UpdateMarkdownRequest {
      mode,
      markdown: markdown.to_string(),
      anchor: None,
      find: None,
      replace: None,
      expected_seq: None,
    }
  }

  /// Every text an InsertBlock op carries, in order.
  fn inserted_texts(ops: &[DocumentOperation]) -> Vec<String> {
    ops
      .iter()
      .filter_map(|op| match op {
        DocumentOperation::InsertBlock { block, .. } if !block.text.is_empty() => {
          Some(block.text.clone())
        }
        _ => None,
      })
      .collect()
  }

  /// Appending content that OPENS with `---` used to lose everything up to the
  /// closing fence: the fragment was parsed as a whole document, so the parser
  /// read that as YAML front matter and stashed it on a throwaway root. The
  /// caller got the tail and no error. Reported 2026-08-07 from an MCP write.
  #[test]
  fn append_does_not_swallow_content_between_dashes() {
    let ops = markdown_update_ops(
      &current_doc(),
      &request(MarkdownUpdateMode::Append, "---\nbody\n---\ntail"),
      Uuid::new_v4(),
    )
    .expect("append should build ops");

    assert_eq!(
      inserted_texts(&ops),
      vec!["body", "tail"],
      "the body between the fences must reach the document"
    );
  }

  /// Same position error, same fix, other grafting mode.
  #[test]
  fn insert_at_does_not_swallow_content_between_dashes() {
    let mut req = request(MarkdownUpdateMode::InsertAt, "---\nbody\n---\ntail");
    req.anchor = Some("p1".to_string());
    let ops =
      markdown_update_ops(&current_doc(), &req, Uuid::new_v4()).expect("insert_at should build ops");
    assert_eq!(inserted_texts(&ops), vec!["body", "tail"]);
  }

  /// `replace_all` IS the whole document, so it keeps reading front matter —
  /// and now carries it onto the real root instead of dropping it with the
  /// temporary one. Without this, export → replace_all → export lost the page's
  /// properties silently.
  #[test]
  fn replace_all_carries_front_matter_onto_the_root() {
    let ops = markdown_update_ops(
      &current_doc(),
      &request(MarkdownUpdateMode::ReplaceAll, "---\ntitle: New\n---\nbody"),
      Uuid::new_v4(),
    )
    .expect("replace_all should build ops");

    let fm = ops
      .iter()
      .find_map(|op| match op {
        DocumentOperation::UpdateBlock {
          block_id,
          data: Some(data),
          ..
        } if block_id == "root" => data.get("front_matter").and_then(|v| v.as_str()),
        _ => None,
      })
      .expect("replace_all must write the front matter to the root");
    assert_eq!(fm, "title: New");
    assert_eq!(inserted_texts(&ops), vec!["body"]);
  }

  /// A body-only `replace_all` must not wipe properties the caller never
  /// mentioned — "replace the text" is not "delete my metadata".
  #[test]
  fn replace_all_without_front_matter_leaves_existing_properties_alone() {
    let ops = markdown_update_ops(
      &current_doc(),
      &request(MarkdownUpdateMode::ReplaceAll, "just body"),
      Uuid::new_v4(),
    )
    .expect("replace_all should build ops");

    assert!(
      !ops.iter().any(|op| matches!(
        op,
        DocumentOperation::UpdateBlock { block_id, .. } if block_id == "root"
      )),
      "no root update means the existing front matter survives untouched"
    );
  }
}


#[cfg(test)]
mod tests {
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
    let out = outline_from_payload(&payload, 7);
    assert_eq!(out.seq, 7);
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
      expected_seq: None,
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
  fn markdown_update_delete_removes_exactly_the_anchored_block() {
    use mica_app_core::documents::DocumentOperation;
    let current = doc_with_children(&["a", "b"]);
    let mut request = upd(MarkdownUpdateMode::Delete, "");
    request.anchor = Some("b".into());
    let ops = markdown_update_ops(&current, &request, Uuid::new_v4()).unwrap();
    assert_eq!(
      ops,
      [DocumentOperation::DeleteBlock { block_id: "b".into() }],
      "one targeted op — deleting a block must not rewrite the page around it"
    );
  }

  #[test]
  fn markdown_update_delete_refuses_the_root_and_unknown_anchors() {
    let current = doc_with_children(&["a"]);

    // Deleting the root would empty the document through a call that reads as
    // "remove one block". `delete_block` also refuses, but by the time the op
    // reaches it the caller has already been told the request was accepted.
    let mut root = upd(MarkdownUpdateMode::Delete, "");
    root.anchor = Some("root".into());
    assert!(markdown_update_ops(&current, &root, Uuid::new_v4()).is_err());

    // A stale block id is the normal failure here — an outline read minutes ago,
    // the block since removed. It must say so, not delete something else.
    let mut gone = upd(MarkdownUpdateMode::Delete, "");
    gone.anchor = Some("vanished".into());
    let err = markdown_update_ops(&current, &gone, Uuid::new_v4()).unwrap_err();
    assert!(err.contains("vanished"), "the error names the anchor: {err}");

    // No anchor at all must not be read as "delete nothing, succeed".
    let bare = upd(MarkdownUpdateMode::Delete, "");
    assert!(markdown_update_ops(&current, &bare, Uuid::new_v4()).is_err());
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
      expected_seq: None,
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
      expected_seq: None,
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

}
