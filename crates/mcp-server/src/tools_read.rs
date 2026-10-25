//! The read tools: workspaces, page tree, search, document reads, outline.
use rmcp::{
  ErrorData as McpError,
  handler::server::wrapper::Parameters,
  model::{CallToolResult, Content},
  tool, tool_router,
};
use serde_json::{Value, json};

use crate::args::*;
use crate::markdown_windows::*;
use crate::results::*;
use crate::*;

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
    // Pairs left out when unset, matching the old hand-built query string:
    // the server treats an absent pair and a defaulted one the same here, and
    // omitting keeps the URL clean.
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(parent) = parent_view_id {
      query.push(("parent_view_id", parent.to_string()));
    }
    if let Some(depth) = depth {
      query.push(("depth", depth.to_string()));
    }
    if let Some(limit) = limit {
      query.push(("limit", limit.to_string()));
    }
    if let Some(offset) = offset {
      query.push(("offset", offset.to_string()));
    }
    if with_stats {
      query.push(("with_stats", "true".to_string()));
    }
    let listed = self
      .get_with_query(&format!("/api/workspaces/{workspace_id}/views"), &query)
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
    // Folders are opt-in server-side because the APP's search dialog opens a hit
    // as a page. An agent is the caller that both wants them and can tell them
    // apart (`is_folder`), so this layer is where the opt-in belongs.
    tool_result(
      self
        .get_with_query(
          &format!("/api/workspaces/{workspace_id}/search"),
          &[("q", query), ("include_folders", "true".to_string())],
        )
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
    let full = match self.full_markdown(workspace_id, document_id, refresh).await {
      Ok(full) => full,
      Err(error) => return Ok(tool_error(error)),
    };
    let out = window_markdown(&full, offset, limit, section.as_deref());
    Ok(CallToolResult::success(vec![Content::text(out)]))
  }

  #[tool(
    description = "Read MANY documents in ONE call — scan a whole workspace fast instead of \
                       N reads. mode: \"outline\" (headings + block ids with type/preview + seq per doc — cheap, \
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
    if document_ids.len() > MAX_BATCH_DOCUMENTS {
      return Ok(tool_error(McpError::invalid_params(
        format!(
          "at most {MAX_BATCH_DOCUMENTS} document_ids per call, got {}",
          document_ids.len()
        ),
        None,
      )));
    }
    let outline = mode.as_deref() == Some("outline");
    if !outline {
      return self.batch_read_full(workspace_id, &document_ids).await;
    }
    // Outline has no server batch route, so it stays one fetch per doc — but
    // joined in small waves rather than awaited one at a time, and still
    // cheap: always fresh, carries seq.
    const OUTLINE_CONCURRENCY: usize = 32;
    let mut out = Vec::with_capacity(document_ids.len());
    for chunk in document_ids.chunks(OUTLINE_CONCURRENCY) {
      let results = futures_util::future::join_all(chunk.iter().map(|id| {
        let path = format!("/api/workspaces/{workspace_id}/documents/{id}/outline");
        async move { self.get(&path).await }
      }))
      .await;
      for (id, fetched) in chunk.iter().zip(results) {
        let mut row = serde_json::Map::new();
        row.insert("document_id".to_string(), json!(id));
        match fetched {
          Ok(v) => {
            row.insert("outline".to_string(), v);
          }
          Err(error) => {
            row.insert("error".to_string(), json!(error.message.to_string()));
          }
        }
        out.push(Value::Object(row));
      }
    }
    tool_result(Ok(json!({ "documents": out })))
  }

  /// The "full" mode of [MicaMcp::mica_read_documents]: the server's
  /// batch-read turns N export/markdown round trips into ONE request. The
  /// endpoint rejects duplicate ids, so dedup on the way out and map results
  /// back onto the caller's list as given — order and repeats preserved.
  async fn batch_read_full(
    &self,
    workspace_id: Uuid,
    document_ids: &[Uuid],
  ) -> Result<CallToolResult, McpError> {
    let mut unique: Vec<Uuid> = Vec::with_capacity(document_ids.len());
    let mut seen = std::collections::HashSet::with_capacity(document_ids.len());
    for id in document_ids {
      if seen.insert(*id) {
        unique.push(*id);
      }
    }
    let value = match self
      .post(
        &format!("/api/workspaces/{workspace_id}/documents/batch-read"),
        json!({ "document_ids": unique }),
      )
      .await
    {
      Ok(value) => value,
      Err(error) => return Ok(tool_error(error)),
    };
    let mut by_id: std::collections::HashMap<Uuid, (Option<String>, Option<String>)> =
      std::collections::HashMap::with_capacity(unique.len());
    if let Some(items) = value.get("documents").and_then(Value::as_array) {
      for item in items {
        let Some(id) = item
          .get("document_id")
          .and_then(|v| serde_json::from_value::<Uuid>(v.clone()).ok())
        else {
          continue;
        };
        let markdown = item
          .get("markdown")
          .and_then(Value::as_str)
          .map(str::to_string);
        let error = item
          .get("error")
          .and_then(Value::as_str)
          .map(str::to_string);
        // Warm the cache exactly as the single-read path does, so a follow-up
        // read anywhere stays local.
        if let Some(md) = &markdown {
          self.cache_put(id, md);
        }
        by_id.insert(id, (markdown, error));
      }
    }
    let mut out = Vec::with_capacity(document_ids.len());
    for id in document_ids {
      let mut row = serde_json::Map::new();
      row.insert("document_id".to_string(), json!(id));
      match by_id.get(id) {
        Some((Some(markdown), _)) => {
          row.insert("markdown".to_string(), json!(markdown));
        }
        Some((None, Some(error))) => {
          row.insert("error".to_string(), json!(error));
        }
        _ => {
          row.insert(
            "error".to_string(),
            json!("the server returned no result for that id"),
          );
        }
      }
      out.push(Value::Object(row));
    }
    tool_result(Ok(json!({ "documents": out })))
  }

  #[tool(
    description = "Get a document's outline (headings + block ids in order, plus blocks \
                       with type and a short visible-text preview) and its \
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
