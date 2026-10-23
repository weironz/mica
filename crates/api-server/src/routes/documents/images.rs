use axum::{
  Json,
  body::Bytes,
  extract::{Path, Query, State},
  http::HeaderMap,
};
use mica_app_core::{
  AppState,
  documents::DocumentOperation,
  store::{self},
};
use mica_infra::ApiResult;
use serde::Deserialize;
use uuid::Uuid;

use super::*;
use crate::routes::auth::user_id_from_headers;
use crate::routes::ws;

#[derive(Debug, Deserialize)]
pub struct RehostImageParams {
  /// The image block whose external `url` is replaced by the stored file.
  block_id: String,
  /// Original filename (with extension) — sets the stored file's name + mime.
  file_name: String,
}

#[derive(Debug, Deserialize)]
pub struct DropImageParams {
  /// The image block to remove.
  block_id: String,
  /// The external `url` that block must still carry. The caller decided this
  /// image is gone by fetching that exact URL; if the block now points
  /// somewhere else, someone edited it since and the decision no longer
  /// applies. Deleting on block id alone would make a stale sweep destructive.
  url: String,
}

/// `POST /api/workspaces/{ws}/documents/{doc}/drop-image?block_id=…&url=…`
///
/// Removes an image block whose source is gone for good — the counterpart to
/// `rehost-image`, and deliberately the same shape: ONE targeted op, never a
/// whole-doc rewrite. Refuses unless the block is still an image, still carries
/// the URL the caller checked, and is NOT already stored in Mica — a block with
/// a `file_id` has bytes we own, and nothing about a dead external link makes
/// those disposable.
pub async fn drop_image(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Query(params): Query<DropImageParams>,
) -> ApiResult<Json<DocumentUpdateResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  let block_id = params.block_id;
  let expected_url = params.url;
  let applied = store::apply_derived_operations(
    &state.db,
    workspace_id,
    document_id,
    user_id,
    None,
    |payload| {
      let block = payload
        .blocks
        .iter()
        .find(|b| b.id == block_id)
        .ok_or_else(|| format!("block {block_id} not found"))?;
      if block.kind != "image" {
        return Err(format!("block {block_id} is not an image"));
      }
      let data = &block.data;
      if data
        .get("file_id")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
      {
        return Err(format!(
          "block {block_id} is stored in Mica, not an external link"
        ));
      }
      let current = data.get("url").and_then(|v| v.as_str()).unwrap_or_default();
      if current != expected_url {
        return Err(format!(
          "block {block_id} no longer points at the checked url — it was edited since"
        ));
      }
      Ok(vec![DocumentOperation::DeleteBlock {
        block_id: block_id.clone(),
      }])
    },
  )
  .await?;
  ws::broadcast_applied_update(&state.hub, &applied, Uuid::nil(), None);

  Ok(Json(DocumentUpdateResponse {
    document: applied.document,
    snapshot: applied.snapshot,
    update: applied.update,
  }))
}

/// `POST /api/workspaces/{ws}/documents/{doc}/rehost-image?block_id=…&file_name=…`
/// — body is the raw image bytes.
///
/// The single write-back endpoint for re-hosting an external image into Mica.
/// The CALLER fetches the bytes (only it can — a CN-hosted server 403s hosts
/// like AppFlowy that block its datacenter IP) and POSTs them here; we store
/// them (sha256-dedup, like any upload) and point the image block at the new
/// `file_id` with ONE targeted `UpdateBlock` op — never a whole-doc rewrite, so
/// unsupported/foreign blocks elsewhere in the doc are left untouched. Shared by
/// the `mica-cli rehost-images` sweep and any future client "re-host all" action.
pub async fn rehost_image(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
  Query(params): Query<RehostImageParams>,
  body: Bytes,
) -> ApiResult<Json<DocumentUpdateResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;

  let client = reqwest::Client::new();
  let file = crate::routes::files::store_bytes(
    &state,
    &client,
    workspace_id,
    user_id,
    &params.file_name,
    &body,
  )
  .await?;
  let file_id = file.id.to_string();
  let name = file.original_name.clone();
  let block_id = params.block_id;

  let applied = store::apply_derived_operations(
    &state.db,
    workspace_id,
    document_id,
    user_id,
    None,
    |payload| {
      // Only touch the named block, and only if it is still an image — never
      // clobber a different block that a concurrent edit may have put here.
      let block = payload
        .blocks
        .iter()
        .find(|b| b.id == block_id)
        .ok_or_else(|| format!("block {block_id} not found"))?;
      if block.kind != "image" {
        return Err(format!("block {block_id} is not an image"));
      }
      Ok(vec![DocumentOperation::UpdateBlock {
        block_id: block_id.clone(),
        kind: None,
        text: None,
        data: Some(serde_json::json!({ "file_id": file_id.clone(), "name": name.clone() })),
      }])
    },
  )
  .await?;
  ws::broadcast_applied_update(&state.hub, &applied, Uuid::nil(), None);

  Ok(Json(DocumentUpdateResponse {
    document: applied.document,
    snapshot: applied.snapshot,
    update: applied.update,
  }))
}
