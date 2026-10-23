//! The recycle-bin tools: trash, restore, move, purge, empty.
use rmcp::{
  ErrorData as McpError, handler::server::wrapper::Parameters, model::CallToolResult, tool,
  tool_router,
};
use serde_json::json;

use crate::args::*;
use crate::results::*;
use crate::*;

#[tool_router(router = trash_router, vis = "pub(crate)")]
impl MicaMcp {
  #[tool(
    description = "Move a page (and its subtree) to the recycle bin — a SOFT delete, \
                       recoverable in the app. Requires confirm=true. For several pages use \
                       mica_trash_views instead of calling this N times; to delete for good, \
                       mica_purge_view / mica_empty_trash.",
    annotations(title = "Trash page", read_only_hint = false, destructive_hint = true)
  )]
  pub(crate) async fn mica_trash_view(
    &self,
    Parameters(TrashArgs {
      workspace_id,
      view_id,
      confirm,
    }): Parameters<TrashArgs>,
  ) -> Result<CallToolResult, McpError> {
    writable!(self);
    if !confirm {
      return Ok(tool_error(McpError::invalid_params(
        "refusing to trash without confirm=true".to_string(),
        None,
      )));
    }
    let trashed = self
      .delete(&format!("/api/workspaces/{workspace_id}/views/{view_id}"))
      .await;
    // Changes which documents exist/read back; see `cache` for why it's all of it.
    self.cache_clear();
    action_ack(trashed, "trashed", view_id)
  }

  #[tool(
    description = "Trash MANY pages/folders at once (soft delete, recoverable). Requires \
                   confirm=true. Prefer this over calling mica_trash_view N times — a cleanup \
                   sweep is one call, not one per page. Each id carries its whole subtree, so \
                   pass the folder rather than listing its children. The reply reports what was \
                   ACTUALLY trashed (`affected`, counting subtrees) and which requested ids were \
                   `skipped` because they were already gone.",
    annotations(
      title = "Trash pages (bulk)",
      read_only_hint = false,
      destructive_hint = true
    )
  )]
  pub(crate) async fn mica_trash_views(
    &self,
    Parameters(BatchViewsArgs {
      workspace_id,
      view_ids,
      confirm,
    }): Parameters<BatchViewsArgs>,
  ) -> Result<CallToolResult, McpError> {
    writable!(self);
    if !confirm {
      return Ok(tool_error(McpError::invalid_params(
        format!(
          "refusing to trash {} view(s) without confirm=true",
          view_ids.len()
        ),
        None,
      )));
    }
    let done = self
      .post(
        &format!("/api/workspaces/{workspace_id}/views/batch-trash"),
        json!({ "view_ids": view_ids }),
      )
      .await;
    // Changes which documents exist/read back; see `cache` for why it's all of it.
    self.cache_clear();
    tool_result(done)
  }

  #[tool(
    description = "Restore MANY pages/folders from the recycle bin in one call — the undo for \
                   mica_trash_views. Each id brings back the subtree that was trashed with it.",
    annotations(title = "Restore pages (bulk)", read_only_hint = false)
  )]
  pub(crate) async fn mica_restore_views(
    &self,
    Parameters(BatchRestoreArgs {
      workspace_id,
      view_ids,
    }): Parameters<BatchRestoreArgs>,
  ) -> Result<CallToolResult, McpError> {
    writable!(self);
    let done = self
      .post(
        &format!("/api/workspaces/{workspace_id}/views/batch-restore"),
        json!({ "view_ids": view_ids }),
      )
      .await;
    // Changes which documents exist/read back; see `cache` for why it's all of it.
    self.cache_clear();
    tool_result(done)
  }

  #[tool(
    description = "Move MANY pages/folders under one parent in a single call, in the given order. \
                   Only a FOLDER can hold children; passing a page as the parent is refused with \
                   a reason. Validated per id before anything is written, so a bad id leaves the \
                   tree untouched rather than half-moved.",
    annotations(title = "Move pages (bulk)", read_only_hint = false)
  )]
  pub(crate) async fn mica_move_views(
    &self,
    Parameters(BatchMoveArgs {
      workspace_id,
      view_ids,
      parent_view_id,
    }): Parameters<BatchMoveArgs>,
  ) -> Result<CallToolResult, McpError> {
    writable!(self);
    tool_result(
      self
        .post(
          &format!("/api/workspaces/{workspace_id}/views/batch-move"),
          json!({ "view_ids": view_ids, "parent_view_id": parent_view_id }),
        )
        .await,
    )
  }

  #[tool(
    description = "PERMANENTLY delete everything in the recycle bin. This is NOT recoverable — \
                   unlike mica_trash_views, nothing survives it. Requires confirm=true. Returns \
                   how many views and documents were destroyed.",
    annotations(
      title = "Empty recycle bin",
      read_only_hint = false,
      destructive_hint = true
    )
  )]
  pub(crate) async fn mica_empty_trash(
    &self,
    Parameters(ConfirmArg {
      workspace_id,
      confirm,
    }): Parameters<ConfirmArg>,
  ) -> Result<CallToolResult, McpError> {
    writable!(self);
    if !confirm {
      return Ok(tool_error(McpError::invalid_params(
        "refusing to empty the recycle bin without confirm=true — this cannot be undone"
          .to_string(),
        None,
      )));
    }
    let done = self
      .delete(&format!("/api/workspaces/{workspace_id}/trash"))
      .await;
    // Changes which documents exist/read back; see `cache` for why it's all of it.
    self.cache_clear();
    tool_result(done)
  }

  #[tool(
    description = "PERMANENTLY delete ONE trashed page/folder (and its subtree) from the recycle \
                   bin. Not recoverable. Requires confirm=true. Use mica_empty_trash to clear the \
                   whole bin.",
    annotations(
      title = "Purge one from bin",
      read_only_hint = false,
      destructive_hint = true
    )
  )]
  pub(crate) async fn mica_purge_view(
    &self,
    Parameters(TrashArgs {
      workspace_id,
      view_id,
      confirm,
    }): Parameters<TrashArgs>,
  ) -> Result<CallToolResult, McpError> {
    writable!(self);
    if !confirm {
      return Ok(tool_error(McpError::invalid_params(
        "refusing to purge without confirm=true — this cannot be undone".to_string(),
        None,
      )));
    }
    let purged = self
      .delete(&format!("/api/workspaces/{workspace_id}/trash/{view_id}"))
      .await;
    // Changes which documents exist/read back; see `cache` for why it's all of it.
    self.cache_clear();
    action_ack(purged, "purged", view_id)
  }

  #[tool(
    description = "PERMANENTLY delete MANY pages/folders from the recycle bin in one call — the \
                   rung between mica_purge_view (one) and mica_empty_trash (everything). Not \
                   recoverable. Requires confirm=true.\n\
                   ONLY acts on pages already IN the recycle bin: trash them first (\
                   mica_trash_views), then purge. Ids that are not trashed come back in \
                   `skipped` and are left alone — this tool cannot destroy a live page, on \
                   purpose. `affected` counts what actually went, subtrees included.",
    annotations(
      title = "Purge pages (bulk)",
      read_only_hint = false,
      destructive_hint = true
    )
  )]
  pub(crate) async fn mica_purge_views(
    &self,
    Parameters(BatchViewsArgs {
      workspace_id,
      view_ids,
      confirm,
    }): Parameters<BatchViewsArgs>,
  ) -> Result<CallToolResult, McpError> {
    writable!(self);
    if !confirm {
      return Ok(tool_error(McpError::invalid_params(
        format!(
          "refusing to permanently delete {} view(s) without confirm=true — this cannot be undone",
          view_ids.len()
        ),
        None,
      )));
    }
    let done = self
      .post(
        &format!("/api/workspaces/{workspace_id}/views/batch-purge"),
        json!({ "view_ids": view_ids }),
      )
      .await;
    // Changes which documents exist/read back; see `cache` for why it's all of it.
    self.cache_clear();
    tool_result(done)
  }
}
