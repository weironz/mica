//! Collaboration-surface tools: comments, members, uploaded files.
//!
//! The server routes existed long before these tools did; the 2026-08-26
//! inventory found MCP could neither see comments nor remove a member or a
//! file. Each removal here has no recycle bin behind it, so each one takes
//! `confirm: true` like the other permanent deletes.
use rmcp::{
  ErrorData as McpError, handler::server::wrapper::Parameters, model::CallToolResult, tool,
  tool_router,
};
use serde_json::json;

use crate::args::*;
use crate::results::*;
use crate::*;

/// Refuse a permanent removal that was not explicitly confirmed. Returned as a
/// tool error (actionable feedback), never as a protocol error.
fn unconfirmed(what: &str) -> CallToolResult {
  tool_error(McpError::invalid_params(
    format!("refusing to {what} without confirm=true — this cannot be undone"),
    None,
  ))
}

fn removed(
  result: Result<serde_json::Value, McpError>,
  key: &str,
  id: Uuid,
) -> Result<CallToolResult, McpError> {
  match result {
    Ok(_) => tool_result(Ok(
      json!({ "ok": true, "action": "deleted", key: id.to_string() }),
    )),
    Err(error) => Ok(tool_error(error)),
  }
}

#[tool_router(router = collab_router, vis = "pub(crate)")]
impl MicaMcp {
  #[tool(
    description = "List a page's comment threads: each thread's id, the quoted text it is \
                   anchored to (anchor null = the text was deleted, the thread is orphaned), \
                   resolved state and its replies. Use the thread id with mica_delete_comment.",
    annotations(title = "List comments", read_only_hint = true)
  )]
  pub(crate) async fn mica_list_comments(
    &self,
    Parameters(DocArg {
      workspace_id,
      document_id,
    }): Parameters<DocArg>,
  ) -> Result<CallToolResult, McpError> {
    tool_result(
      self
        .get(&format!(
          "/api/workspaces/{workspace_id}/documents/{document_id}/comments"
        ))
        .await,
    )
  }

  #[tool(
    description = "PERMANENTLY delete a comment thread and all its replies. Not recoverable. \
                   Requires confirm=true. You can delete threads you started; deleting someone \
                   else's needs edit rights in the workspace. The page text is not touched.",
    annotations(
      title = "Delete comment thread",
      read_only_hint = false,
      destructive_hint = true
    )
  )]
  pub(crate) async fn mica_delete_comment(
    &self,
    Parameters(CommentThreadArgs {
      workspace_id,
      document_id,
      thread_id,
      confirm,
    }): Parameters<CommentThreadArgs>,
  ) -> Result<CallToolResult, McpError> {
    writable!(self);
    if !confirm {
      return Ok(unconfirmed("delete a comment thread"));
    }
    let done = self
      .delete(&format!(
        "/api/workspaces/{workspace_id}/documents/{document_id}/comments/{thread_id}"
      ))
      .await;
    removed(done, "thread_id", thread_id)
  }

  #[tool(
    description = "List a workspace's members: user id, email, display name, role and join \
                   date. Use the user id with mica_remove_member.",
    annotations(title = "List members", read_only_hint = true)
  )]
  pub(crate) async fn mica_list_members(
    &self,
    Parameters(WorkspaceArg { workspace_id }): Parameters<WorkspaceArg>,
  ) -> Result<CallToolResult, McpError> {
    tool_result(
      self
        .get(&format!("/api/workspaces/{workspace_id}/members"))
        .await,
    )
  }

  #[tool(
    description = "Remove a member from a workspace: they lose access at once. Their pages and \
                   comments stay. Requires confirm=true and member-management rights; the \
                   workspace owner cannot be removed. Re-adding takes a new invitation from the \
                   app. Returns the remaining member list.",
    annotations(
      title = "Remove member",
      read_only_hint = false,
      destructive_hint = true
    )
  )]
  pub(crate) async fn mica_remove_member(
    &self,
    Parameters(MemberArgs {
      workspace_id,
      user_id,
      confirm,
    }): Parameters<MemberArgs>,
  ) -> Result<CallToolResult, McpError> {
    writable!(self);
    if !confirm {
      return Ok(unconfirmed("remove a member"));
    }
    tool_result(
      self
        .delete(&format!("/api/workspaces/{workspace_id}/members/{user_id}"))
        .await,
    )
  }

  #[tool(
    description = "PERMANENTLY delete an uploaded file (image or attachment) now. Takes the \
                   file_id from an href's `/files/{file_id}/blob/…`. Every page still linking \
                   to it shows a broken image afterwards — edit those links out first. You \
                   rarely need this: files no page references are reclaimed automatically. \
                   Requires confirm=true and edit rights.",
    annotations(title = "Delete file", read_only_hint = false, destructive_hint = true)
  )]
  pub(crate) async fn mica_delete_file(
    &self,
    Parameters(DeleteFileArgs {
      workspace_id,
      file_id,
      confirm,
    }): Parameters<DeleteFileArgs>,
  ) -> Result<CallToolResult, McpError> {
    writable!(self);
    if !confirm {
      return Ok(unconfirmed("delete a file"));
    }
    let done = self
      .delete(&format!("/api/workspaces/{workspace_id}/files/{file_id}"))
      .await;
    removed(done, "file_id", file_id)
  }
}
