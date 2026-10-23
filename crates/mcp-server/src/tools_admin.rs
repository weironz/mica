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

#[tool_router(router = admin_router, vis = "pub(crate)")]
impl MicaMcp {
  #[tool(
    description = "PERMANENTLY delete a whole workspace — every page, folder, version, comment, \
                   share link and uploaded file in it. There is NO workspace recycle bin: this \
                   is the one call here that destroys content which was never deleted first, and \
                   nothing survives it. Owner only.\n\
                   Requires confirm=true AND expect_name set to the workspace's exact current \
                   name; a mismatch is refused without touching anything. To remove PAGES rather \
                   than the workspace, use mica_trash_views.",
    annotations(
      title = "Delete workspace",
      read_only_hint = false,
      destructive_hint = true,
      idempotent_hint = false
    )
  )]
  pub(crate) async fn mica_delete_workspace(
    &self,
    Parameters(DeleteWorkspaceArgs {
      workspace_id,
      expect_name,
      confirm,
    }): Parameters<DeleteWorkspaceArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    if !confirm {
      return Ok(tool_error(McpError::invalid_params(
        "refusing to delete a workspace without confirm=true — this cannot be undone".to_string(),
        None,
      )));
    }
    // Read the name back and compare BEFORE deleting anything. The check is
    // worth a round trip precisely because the alternative is unrecoverable:
    // there is no bin to fish it out of afterwards.
    let listed = match self.get("/api/workspaces").await {
      Ok(value) => value,
      Err(error) => return Ok(tool_error(error)),
    };
    let actual = listed
      .get("workspaces")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      // Compare as UUIDs, not strings: the API's casing/hyphenation is not the
      // caller's to match.
      .find(|w| {
        w.get("id")
          .and_then(Value::as_str)
          .and_then(|id| id.parse::<uuid::Uuid>().ok())
          == Some(workspace_id)
      })
      .and_then(|w| w.get("name").and_then(Value::as_str))
      .map(str::to_string);
    // Refusals come back as tool errors, not protocol errors: the model can act
    // on both of these (list the workspaces, fix the name) and `tool_result`'s
    // note says that is exactly the split.
    let Some(actual) = actual else {
      return Ok(tool_error(McpError::invalid_params(
        format!("no workspace {workspace_id} is visible to this token — nothing was deleted"),
        None,
      )));
    };
    if actual != expect_name {
      return Ok(tool_error(McpError::invalid_params(
        format!(
          "refusing to delete: workspace {workspace_id} is named {actual:?}, not \
           {expect_name:?}. Nothing was deleted. Check mica_list_workspaces — this guard exists \
           because the wrong id here cannot be undone."
        ),
        None,
      )));
    }
    let deleted = self.delete(&format!("/api/workspaces/{workspace_id}")).await;
    // Every cached page belonged to SOME workspace; this one's are gone.
    self.cache_clear();
    action_ack(deleted, "deleted", workspace_id)
  }

  #[tool(
    description = "Publish a document to a PUBLIC read-only web link and return the URL. Anyone \
                   with the link can read it (no login); re-sharing returns the same link. Use \
                   mica_unshare to turn it off.",
    annotations(title = "Share to web", read_only_hint = false)
  )]
  pub(crate) async fn mica_share(
    &self,
    Parameters(DocArg {
      workspace_id,
      document_id,
    }): Parameters<DocArg>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let shared = self
      .post(
        &format!("/api/workspaces/{workspace_id}/documents/{document_id}/share"),
        json!({}),
      )
      .await;
    match shared {
      Ok(value) => {
        let token = value
          .pointer("/token")
          .and_then(Value::as_str)
          .unwrap_or_default();
        // Hand back the full, openable URL — the token alone is not actionable.
        tool_result(Ok(json!({
          "ok": true,
          "shared": true,
          "url": format!("{}/s/{}", self.base, token),
        })))
      }
      Err(error) => Ok(tool_error(error)),
    }
  }

  #[tool(
    description = "Turn off a document's public link (the URL 404s immediately).",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_unshare(
    &self,
    Parameters(DocArg {
      workspace_id,
      document_id,
    }): Parameters<DocArg>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let done = self
      .delete(&format!(
        "/api/workspaces/{workspace_id}/documents/{document_id}/share"
      ))
      .await;
    match done {
      Ok(_) => tool_result(Ok(json!({ "ok": true, "shared": false }))),
      Err(error) => Ok(tool_error(error)),
    }
  }

  #[tool(
    description = "Export a whole workspace as Markdown (all pages, in tree order). Read-only \
                       — use it to snapshot content for review or to hand to an external backup.",
    annotations(read_only_hint = true)
  )]
  pub(crate) async fn mica_export_workspace(
    &self,
    Parameters(WorkspaceArg { workspace_id }): Parameters<WorkspaceArg>,
  ) -> Result<CallToolResult, McpError> {
    tool_result(
      self
        .get(&format!("/api/workspaces/{workspace_id}/export/markdown"))
        .await,
    )
  }
}
