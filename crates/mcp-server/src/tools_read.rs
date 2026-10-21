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
  model::{CallToolResult, Content}, tool, tool_router,
};
use serde_json::{Value, json};

use crate::*;
use crate::args::*;
use crate::markdown_windows::*;
use crate::results::*;

#[tool_router(router = read_router, vis = "pub(crate)")]
impl MicaMcp {
  #[tool(
    description = "List all Mica workspaces (id, name, role) the token can access.",
    annotations(read_only_hint = true)
  )]
  pub(crate) async fn mica_list_workspaces(&self) -> Result<CallToolResult, McpError> {
    tool_result(self.get("/api/workspaces").await)
  }

  #[tool(
    description = "List a workspace's page tree (documents + folders, with ids, names, \
                       parents). Use a page's object_id with the read/write tools.\n\
                       NARROW IT: on a large workspace the whole tree is a big result that can \
                       be truncated in transit, leaving you with no structure at all. Pass \
                       parent_view_id to list one folder, depth=1 for just its direct children, \
                       or limit/offset to page through. with_stats=true adds `state_bytes` per \
                       page — SMALL means nearly empty (that is how you find the stubs without \
                       reading every page), but large does NOT mean long: deleted text leaves \
                       weight behind, so never read it as a word count.",
    annotations(read_only_hint = true)
  )]
  pub(crate) async fn mica_list_pages(
    &self,
    Parameters(ListPagesArgs {
      workspace_id,
      parent_view_id,
      depth,
      limit,
      offset,
      with_stats,
    }): Parameters<ListPagesArgs>,
  ) -> Result<CallToolResult, McpError> {
    let mut qs: Vec<String> = Vec::new();
    if let Some(parent) = parent_view_id {
      qs.push(format!("parent_view_id={parent}"));
    }
    if let Some(depth) = depth {
      qs.push(format!("depth={depth}"));
    }
    if let Some(limit) = limit {
      qs.push(format!("limit={limit}"));
    }
    if let Some(offset) = offset {
      qs.push(format!("offset={offset}"));
    }
    if with_stats {
      qs.push("with_stats=true".to_string());
    }
    let query = if qs.is_empty() {
      String::new()
    } else {
      format!("?{}", qs.join("&"))
    };
    let listed = self
      .get(&format!("/api/workspaces/{workspace_id}/views{query}"))
      .await;
    // `slim_pages` keeps state_bytes when it is there — see its field list.
    tool_result(listed.map(slim_pages))
  }

  // The description is the model's ONLY knowledge of what this can do, and it
  // used to say "by title" — which was false, and quietly expensive: the
  // endpoint has always scanned body text too, so every agent was told the
  // index did not exist and fell back to reading whole pages to find a
  // phrase. Say what it actually does, and say what it costs.
  #[tool(
    description = "Find pages AND FOLDERS by TITLE **and body text**. Returns each hit's \
                       view_id, object_id, name, a snippet of the matching body text \
                       (`title_match` tells you which side matched), `parent_view_id` (the \
                       containing folder, null at the workspace root), and `is_folder`. Prefer \
                       this over reading pages when looking for something: a hit costs a \
                       fraction of a whole document, and the snippet is often answer enough on \
                       its own. ALSO reach for it instead of mica_list_pages when you need to \
                       FILE a page somewhere — search for anything already sitting in that spot \
                       and reuse its `parent_view_id`, which costs one hit instead of a whole \
                       workspace tree. Folders match by name only (they carry no body).",
    annotations(read_only_hint = true)
  )]
  pub(crate) async fn mica_search(
    &self,
    Parameters(SearchArgs {
      workspace_id,
      query,
    }): Parameters<SearchArgs>,
  ) -> Result<CallToolResult, McpError> {
    let q = urlencode(&query);
    // Folders are opt-in server-side because the APP's search dialog opens a hit
    // as a page. An agent is the caller that both wants them and can tell them
    // apart (`is_folder`), so this layer is where the opt-in belongs.
    tool_result(
      self
        .get(&format!(
          "/api/workspaces/{workspace_id}/search?q={q}&include_folders=true"
        ))
        .await,
    )
  }

  #[tool(
    description = "Read a document as Markdown. For long pages, pass offset+limit \
                       (a line window) or section (a heading name) to pull just a slice \
                       and save tokens instead of the whole doc.",
    annotations(read_only_hint = true)
  )]
  pub(crate) async fn mica_read_document(
    &self,
    Parameters(ReadDocArgs {
      workspace_id,
      document_id,
      offset,
      limit,
      section,
      refresh,
    }): Parameters<ReadDocArgs>,
  ) -> Result<CallToolResult, McpError> {
    // Full markdown from the warm cache (or fetched + cached), then sliced — the
    // window is what saves tokens on a big page; the cache is what makes a
    // re-read local.
    let full = match self
      .full_markdown(workspace_id, document_id, refresh)
      .await
    {
      Ok(full) => full,
      Err(error) => return Ok(tool_error(error)),
    };
    let out = window_markdown(&full, offset, limit, section.as_deref());
    Ok(CallToolResult::success(vec![Content::text(out)]))
  }

  #[tool(
    description = "Read MANY documents in ONE call — scan a whole workspace fast instead of \
                       N reads. mode: \"outline\" (headings + block ids + seq per doc — cheap, \
                       survey structure across docs) or \"full\" (each doc's whole Markdown). \
                       Per-doc errors report inline; a bad id doesn't fail the batch.",
    annotations(read_only_hint = true)
  )]
  pub(crate) async fn mica_read_documents(
    &self,
    Parameters(ReadDocumentsArgs {
      workspace_id,
      document_ids,
      mode,
    }): Parameters<ReadDocumentsArgs>,
  ) -> Result<CallToolResult, McpError> {
    if document_ids.is_empty() {
      return Ok(tool_error(McpError::invalid_params(
        "document_ids is empty — pass at least one id".to_string(),
        None,
      )));
    }
    let outline = mode.as_deref() == Some("outline");
    let mut out = Vec::with_capacity(document_ids.len());
    for id in &document_ids {
      let mut row = serde_json::Map::new();
      row.insert("document_id".to_string(), json!(id));
      // outline stays a direct fetch (cheap, always fresh, carries seq); full
      // markdown rides the warm cache like the single read.
      let fetched = if outline {
        self
          .get(&format!(
            "/api/workspaces/{workspace_id}/documents/{id}/outline"
          ))
          .await
          .map(|v| ("outline", v))
      } else {
        self
          .full_markdown(workspace_id, *id, false)
          .await
          .map(|md| ("markdown", json!(md)))
      };
      match fetched {
        Ok((key, v)) => {
          row.insert(key.to_string(), v);
        }
        Err(error) => {
          row.insert("error".to_string(), json!(error.message.to_string()));
        }
      }
      out.push(Value::Object(row));
    }
    tool_result(Ok(json!({ "documents": out })))
  }

  #[tool(
    description = "Get a document's outline (headings + block ids in order) plus its \
                       current `seq`. Call this before an anchored write so you can target a \
                       spot instead of rewriting the whole page — and pass the `seq` back as \
                       `expected_seq` to make the write conflict-checked.",
    annotations(read_only_hint = true)
  )]
  pub(crate) async fn mica_get_outline(
    &self,
    Parameters(DocArg {
      workspace_id,
      document_id,
    }): Parameters<DocArg>,
  ) -> Result<CallToolResult, McpError> {
    tool_result(
      self
        .get(&format!(
          "/api/workspaces/{workspace_id}/documents/{document_id}/outline"
        ))
        .await,
    )
  }

}
