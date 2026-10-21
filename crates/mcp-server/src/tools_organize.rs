//! Mica MCP server — a thin proxy that exposes the Mica REST API as MCP tools so
//! an AI (Claude Desktop / Code / any MCP client) can list, read, create, and
//! write Mica documents. It calls the API over HTTP with a PAT; it holds no DB
//! or storage access of its own. "Backup/restore" here is just export/import —
//! real backups are the job of external tools pointed at the exports.
//!
//! Shipped as `mica-cli mcp` (this crate is a library; the CLI resolves the
//! server URL + PAT through its usual chain — env, flags, saved login — and
//! calls [serve_stdio]). Read-only mode makes every write tool refuse at call
//! time while the read tools stay listed.
use rmcp::{
  ErrorData as McpError,
  handler::server::wrapper::Parameters,
  model::CallToolResult, tool, tool_router,
};
use serde_json::{Value, json};

use crate::*;
use crate::args::*;
use crate::results::*;

#[tool_router(router = organize_router, vis = "pub(crate)")]
impl MicaMcp {
  #[tool(
    description = "Move a page under a new parent folder (or to the top level).",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_move_document(
    &self,
    Parameters(MoveDocArgs {
      workspace_id,
      view_id,
      parent_view_id,
    }): Parameters<MoveDocArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let body = json!({ "parent_view_id": parent_view_id });
    let moved = self
      .post(
        &format!("/api/workspaces/{workspace_id}/views/{view_id}/move"),
        body,
      )
      .await;
    action_ack(moved, "moved", view_id)
  }

  #[tool(
    description = "Move or COPY a page (its whole subtree) or a folder to ANOTHER workspace on this \
                   server. copy=true keeps the source; otherwise the source is MOVED (trashed to the \
                   recycle bin after copying). Images are copied into the destination workspace. Note: \
                   the transferred page gets a NEW id, so version-history checkpoints do NOT carry \
                   over, and any links to pages left behind in the source will dangle. Ids come from \
                   mica_list_workspaces and mica_list_pages.",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_transfer_view(
    &self,
    Parameters(TransferArgs {
      workspace_id,
      view_id,
      dest_workspace_id,
      parent_view_id,
      copy,
    }): Parameters<TransferArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let body = json!({
      "dest_workspace_id": dest_workspace_id,
      "parent_view_id": parent_view_id,
      "remove_source": !copy,
    });
    let done = self
      .post(
        &format!("/api/workspaces/{workspace_id}/views/{view_id}/transfer"),
        body,
      )
      .await;
    // A move trashes the source; see `cache` for why it's all of it.
    self.cache_clear();
    action_ack(done, if copy { "copied" } else { "moved" }, view_id)
  }

  #[tool(
    description = "Duplicate a page (its whole subtree) or a folder IN PLACE — the copy lands in \
                   the SAME workspace, beside the original by default (or under parent_view_id if \
                   given). Images are shared with the original (same file, no re-upload). Like \
                   transfer, the copy gets NEW ids, so version-history checkpoints do NOT carry \
                   over. Omit name for a \"{source} 副本\" default; the name is deduped against \
                   siblings. Ids come from mica_list_pages.",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_clone_document(
    &self,
    Parameters(CloneArgs {
      workspace_id,
      view_id,
      parent_view_id,
      name,
    }): Parameters<CloneArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let body = json!({
      "parent_view_id": parent_view_id,
      "name": name,
    });
    let done = self
      .post(
        &format!("/api/workspaces/{workspace_id}/views/{view_id}/clone"),
        body,
      )
      .await;
    action_ack(done, "cloned", view_id)
  }

  #[tool(
    description = "Reorder a folder's children (or the workspace's top level) in ONE call: pass \
                   the complete list of child view ids in the order you want. Use this to SORT — \
                   the per-page move tool would take one call per item. Get the current children \
                   and their ids from mica_list_pages.",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_reorder(
    &self,
    Parameters(ReorderArgs {
      workspace_id,
      parent_view_id,
      ordered_view_ids,
    }): Parameters<ReorderArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let count = ordered_view_ids.len();
    let body = json!({
      "parent_view_id": parent_view_id,
      "ordered_view_ids": ordered_view_ids,
    });
    let done = self
      .post(
        &format!("/api/workspaces/{workspace_id}/views/reorder"),
        body,
      )
      .await;
    // The endpoint already answers with a count; forward what happened, not a tree.
    match done {
      Ok(_) => tool_result(Ok(
        json!({ "ok": true, "action": "reordered", "count": count }),
      )),
      Err(error) => Ok(tool_error(error)),
    }
  }

  #[tool(
    description = "Rename a page or a folder (changes its display name only, not its content).",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_rename(
    &self,
    Parameters(RenameArgs {
      workspace_id,
      view_id,
      name,
    }): Parameters<RenameArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let renamed = self
      .patch(
        &format!("/api/workspaces/{workspace_id}/views/{view_id}"),
        json!({ "name": name }),
      )
      .await;
    match renamed {
      Ok(_) => tool_result(Ok(
        json!({ "ok": true, "action": "renamed", "view_id": view_id, "name": name }),
      )),
      Err(error) => Ok(tool_error(error)),
    }
  }

  #[tool(
    description = "Create a folder — a pure container for organizing pages (no content of its own). \
                   Returns its view_id so you can move pages into it or nest folders under it.",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_create_folder(
    &self,
    Parameters(CreateFolderArgs {
      workspace_id,
      name,
      parent_view_id,
    }): Parameters<CreateFolderArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let created = self
      .post(
        &format!("/api/workspaces/{workspace_id}/folders"),
        json!({ "name": name, "parent_view_id": parent_view_id }),
      )
      .await;
    // Hand back the id — the whole point of creating a folder is to put things
    // in it, which needs its view_id.
    match created {
      Ok(value) => {
        let view_id = value
          .pointer("/view/id")
          .and_then(Value::as_str)
          .unwrap_or_default();
        tool_result(Ok(json!({
          "ok": true, "action": "created_folder", "view_id": view_id, "name": name
        })))
      }
      Err(error) => Ok(tool_error(error)),
    }
  }

  #[tool(
    description = "List the workspace's recycle bin (soft-deleted pages/folders) so you can find \
                   something to restore.",
    annotations(read_only_hint = true)
  )]
  pub(crate) async fn mica_list_trash(
    &self,
    Parameters(WorkspaceArg { workspace_id }): Parameters<WorkspaceArg>,
  ) -> Result<CallToolResult, McpError> {
    tool_result(
      self
        .get(&format!("/api/workspaces/{workspace_id}/trash"))
        .await,
    )
  }

  #[tool(
    description = "Restore a page or folder from the recycle bin back into the tree (undo a trash).",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_restore_view(
    &self,
    Parameters(ViewArg {
      workspace_id,
      view_id,
    }): Parameters<ViewArg>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let restored = self
      .post(
        &format!("/api/workspaces/{workspace_id}/views/{view_id}/restore"),
        json!({}),
      )
      .await;
    // Changes which documents exist/read back; see `cache` for why it's all of it.
    self.cache_clear();
    action_ack(restored, "restored", view_id)
  }

  #[tool(
    description = "List a document's named versions (restorable checkpoints), newest first. The \
                   raw edit log is omitted — these are the points you can roll back to.",
    annotations(read_only_hint = true)
  )]
  pub(crate) async fn mica_list_versions(
    &self,
    Parameters(DocArg {
      workspace_id,
      document_id,
    }): Parameters<DocArg>,
  ) -> Result<CallToolResult, McpError> {
    let history = self
      .get(&format!(
        "/api/workspaces/{workspace_id}/documents/{document_id}/history"
      ))
      .await;
    // Forward only the named `versions` — the `updates` op log is large and not
    // something a model acts on.
    match history {
      Ok(value) => {
        let versions = value.pointer("/versions").cloned().unwrap_or(json!([]));
        tool_result(Ok(json!({ "versions": versions })))
      }
      Err(error) => Ok(tool_error(error)),
    }
  }

  #[tool(
    description = "Pin the document's CURRENT state as a named, restorable version — a checkpoint \
                   before a risky edit. Returns the version_id.",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_create_version(
    &self,
    Parameters(CreateVersionArgs {
      workspace_id,
      document_id,
      name,
    }): Parameters<CreateVersionArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let created = self
      .post(
        &format!("/api/workspaces/{workspace_id}/documents/{document_id}/versions"),
        json!({ "name": name }),
      )
      .await;
    match created {
      Ok(value) => {
        let version_id = value
          .pointer("/version/id")
          .and_then(Value::as_str)
          .unwrap_or_default();
        tool_result(Ok(json!({
          "ok": true, "action": "created_version", "version_id": version_id, "name": name
        })))
      }
      Err(error) => Ok(tool_error(error)),
    }
  }

  #[tool(
    description = "Roll a document back to a named version (from mica_list_versions). History stays \
                   append-only — the restore is itself a new edit, so it is undoable.",
    annotations(
      title = "Restore version",
      read_only_hint = false,
      destructive_hint = true
    )
  )]
  pub(crate) async fn mica_restore_version(
    &self,
    Parameters(RestoreVersionArgs {
      workspace_id,
      document_id,
      version_id,
    }): Parameters<RestoreVersionArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let restored = self
      .post(
        &format!("/api/workspaces/{workspace_id}/documents/{document_id}/restore"),
        json!({ "version_id": version_id }),
      )
      .await;
    // The restore rewrote the page: a cached copy is now the pre-restore text,
    // and the next find_replace/insert_at would be planned against it. Dropped
    // whatever the outcome — a request that errored may still have landed.
    self.cache_invalidate(document_id);
    match restored {
      Ok(_) => tool_result(Ok(json!({
        "ok": true, "action": "restored_version", "version_id": version_id
      }))),
      Err(error) => Ok(tool_error(error)),
    }
  }

}
