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
use uuid::Uuid;

#[tool_router(router = write_router, vis = "pub(crate)")]
impl MicaMcp {
  #[tool(
    description = "Create a new page (optionally with Markdown body) in a workspace.",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_create_document(
    &self,
    Parameters(CreateDocArgs {
      workspace_id,
      name,
      markdown,
      parent_view_id,
    }): Parameters<CreateDocArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    // Markdown → the import endpoint (parses content server-side); empty →
    // the plain create endpoint.
    if let Some(markdown) = markdown {
      if let Err(error) = reject_mangled_latex(&markdown) {
        return Ok(tool_error(error));
      }
      let body = json!({
          "name": name,
          "markdown": markdown,
          "parent_view_id": parent_view_id,
      });
      match self
        .post(
          &format!("/api/workspaces/{workspace_id}/documents/import/markdown"),
          body,
        )
        .await
      {
        Ok(value) => write_ack(value),
        Err(error) => Ok(tool_error(error)),
      }
    } else {
      let body = json!({ "name": name, "parent_view_id": parent_view_id });
      match self
        .post(&format!("/api/workspaces/{workspace_id}/documents"), body)
        .await
      {
        Ok(value) => write_ack(value),
        Err(error) => Ok(tool_error(error)),
      }
    }
  }

  #[tool(
    description = "Create a new Mica workspace. Returns the new workspace (id, name, role); \
                       use the id with the page tools.",
    annotations(read_only_hint = false)
  )]
  pub(crate) async fn mica_create_workspace(
    &self,
    Parameters(CreateWorkspaceArgs { name }): Parameters<CreateWorkspaceArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    match self.post("/api/workspaces", json!({ "name": name })).await {
      Ok(value) => write_ack(value),
      Err(error) => Ok(tool_error(error)),
    }
  }

  #[tool(
    description = "Write into an EXISTING doc. mode: append (safe default), \
                       replace_all (rewrite), insert_at (place after `anchor` — a block id \
                       or a heading's text, from mica_get_outline), find_replace (swap \
                       `find`→`replace`; set unique=true for a precise single edit), \
                       delete (remove the block at `anchor` and its subtree). \
                       To REMOVE something, use delete — not replace_all: resending a \
                       whole page to drop one block rewrites every other block too. \
                       Content is Markdown; the server derives the ops.",
    annotations(
      title = "Write document",
      read_only_hint = false,
      destructive_hint = true
    )
  )]
  pub(crate) async fn mica_update_document(
    &self,
    Parameters(UpdateDocArgs {
      workspace_id,
      document_id,
      mode,
      markdown,
      anchor,
      find,
      replace,
      unique,
      expected_seq,
    }): Parameters<UpdateDocArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    match self
      .write_markdown(
        workspace_id,
        document_id,
        &mode,
        markdown.as_deref(),
        anchor,
        find.as_deref(),
        replace.as_deref(),
        unique,
        expected_seq,
      )
      .await
    {
      Ok(value) => write_ack(value),
      Err(error) => Ok(tool_error(error)),
    }
  }

  /// The shared markdown-write path behind mica_update_document AND
  /// mica_apply_edits: latex guard, unique-match pre-check, heading-anchor
  /// resolution, then the PATCH. Returns the raw API value (for write_ack) or a
  /// tool error. One seam so a batch applies exactly what a single write does.
  #[allow(clippy::too_many_arguments)]
  pub(crate) async fn write_markdown(
    &self,
    workspace_id: Uuid,
    document_id: Uuid,
    mode: &str,
    markdown: Option<&str>,
    anchor: Option<String>,
    find: Option<&str>,
    replace: Option<&str>,
    unique: bool,
    expected_seq: Option<i64>,
  ) -> Result<Value, McpError> {
    // Every field that carries authored content, not just `markdown`:
    // find_replace writes through `replace`, and a swapped-in formula is
    // mangled by the same under-escaping.
    for text in [markdown, replace].into_iter().flatten() {
      reject_mangled_latex(text)?;
    }

    // unique find_replace: refuse unless the target occurs EXACTLY ONCE, so a
    // precise single edit never silently rewrites other matches (the Anthropic
    // str_replace discipline). Costs one read; opt-in via unique=true.
    if mode == "find_replace"
      && unique
      && let Some(find) = find.filter(|s| !s.is_empty())
    {
      let v = self
        .get(&format!(
          "/api/workspaces/{workspace_id}/documents/{document_id}/export/markdown"
        ))
        .await?;
      // Must UNWRAP, not stringify. The endpoint answers `{"markdown": "..."}`,
      // so `to_string()` counted matches against the JSON *encoding* of the
      // page — every newline a literal `\n`, every quote escaped. A `find`
      // spanning a line break then occurred 0 times and this guard rejected a
      // perfectly good edit. Same bug the read path was fixed for; this branch
      // predates the shared helper and was missed.
      let full = unwrap_markdown(v);
      let n = full.matches(find).count();
      if n != 1 {
        return Err(McpError::invalid_params(
          format!(
            "unique find_replace: {find:?} occurs {n} time(s); need exactly 1 — add \
             surrounding context to disambiguate, or set unique=false to replace all."
          ),
          None,
        ));
      }
    }

    // insert_at and delete both let the caller anchor by HEADING TEXT, not just
    // a block id — a heading survives edits above it. delete resolves STRICTLY:
    // a wrong insert position is visible and undoable, a wrong delete is gone.
    let anchor = if mode == "insert_at" || mode == "delete" {
      self
        .resolve_anchor(workspace_id, document_id, anchor, mode == "delete")
        .await?
    } else {
      anchor
    };

    let body = json!({
        "mode": mode,
        "markdown": markdown.unwrap_or_default(),
        "anchor": anchor,
        "find": find,
        "replace": replace,
        // Optimistic concurrency (Tier 1 #4): honoured once the API deploy lands;
        // ignored as an unknown field before that, so this is safe to always send.
        "expected_seq": expected_seq,
    });
    let result = self
      .patch(
        &format!("/api/workspaces/{workspace_id}/documents/{document_id}/markdown"),
        body,
      )
      .await;
    // The doc just changed — drop its warm copy so the next read is fresh
    // (keeps the cache coherent with the agent's own edits).
    if result.is_ok() {
      self.cache_invalidate(document_id);
    }
    result
  }

  /// insert_at anchor resolution: a `block_…` id passes through unchanged; None
  /// passes through (the API's append-at-end fallback); anything else is treated
  /// as heading text and resolved to the matching heading's block id via the
  /// outline, so a caller can say "under ## Setup" without knowing block ids.
  /// `strict` (delete) refuses an ambiguous match instead of taking the first
  /// one: "Setup" matching three headings is a question, not a target, and
  /// answering it by position deletes a section the caller never saw.
  pub(crate) async fn resolve_anchor(
    &self,
    ws: Uuid,
    doc: Uuid,
    anchor: Option<String>,
    strict: bool,
  ) -> Result<Option<String>, McpError> {
    let Some(anchor) = anchor else {
      return Ok(None);
    };
    if anchor.starts_with("block_") {
      return Ok(Some(anchor));
    }
    let outline = self
      .get(&format!("/api/workspaces/{ws}/documents/{doc}/outline"))
      .await?;
    pick_anchor(outline.get("headings"), &anchor, strict)
      .map(Some)
      .map_err(|message| McpError::invalid_params(message, None))
  }

  #[tool(
    description = "Apply a BATCH of edits to ONE document in a single call — one agent \
                       turn for a whole burst instead of N round-trips. `edits` run IN ORDER, \
                       each like mica_update_document (append / insert_at / find_replace / \
                       replace_all / delete); later edits see the effect of earlier ones. \
                       Deleting several blocks is one call: one delete per block id. \
                       NOT atomic: on \
                       a failure it stops and reports how many landed, so you can resume from \
                       there.",
    annotations(
      title = "Apply edits (batch)",
      read_only_hint = false,
      destructive_hint = true
    )
  )]
  pub(crate) async fn mica_apply_edits(
    &self,
    Parameters(ApplyEditsArgs {
      workspace_id,
      document_id,
      edits,
      expected_seq,
    }): Parameters<ApplyEditsArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    if edits.is_empty() {
      return Ok(tool_error(McpError::invalid_params(
        "edits is empty — pass at least one edit".to_string(),
        None,
      )));
    }
    let total = edits.len();
    let mut last: Option<Value> = None;
    for (i, e) in edits.into_iter().enumerate() {
      let EditOp {
        mode,
        markdown,
        anchor,
        find,
        replace,
        unique,
      } = e;
      // expected_seq guards only the batch START; within the batch the doc's seq
      // advances with each write, so pinning it per-op would 409 on edit #2.
      let seq = if i == 0 { expected_seq } else { None };
      match self
        .write_markdown(
          workspace_id,
          document_id,
          &mode,
          markdown.as_deref(),
          anchor,
          find.as_deref(),
          replace.as_deref(),
          unique,
          seq,
        )
        .await
      {
        Ok(value) => last = Some(value),
        Err(error) => {
          // Partial application: say how many landed and which op failed, as an
          // isError result the agent can read and resume from.
          return Ok(tool_error(McpError::internal_error(
            format!(
              "applied {i}/{total} edits; edit #{i} ({mode}) failed: {}",
              error.message
            ),
            None,
          )));
        }
      }
    }
    batch_ack(total, last)
  }

  // Images were the one thing an agent could neither put in nor get out. The
  // REST file layer (presign/complete/import-url/blob) existed all along but
  // only the Flutter client ever called it, so `![](https://…)` written
  // through MCP stayed an EXTERNAL link: the bytes never entered Mica, and
  // the page broke whenever the origin did.
  #[tool(
    description = "Store an image in this workspace from a public http(s) URL and return \
                       its file_id plus the exact Markdown to paste. Use this instead of \
                       writing `![](https://…)` directly: a bare external URL is only a link — \
                       Mica never holds the bytes, so the image dies with its origin. The \
                       returned `markdown` embeds the stored copy.",
    annotations(title = "Store image", read_only_hint = false)
  )]
  pub(crate) async fn mica_add_image(
    &self,
    Parameters(AddImageArgs { workspace_id, url }): Parameters<AddImageArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    let stored = self
      .post(
        &format!("/api/workspaces/{workspace_id}/files/import-url"),
        json!({ "url": url }),
      )
      .await;
    let value = match stored {
      Ok(value) => value,
      Err(error) => return Ok(tool_error(error)),
    };
    let file_id = value
      .pointer("/file/id")
      .and_then(Value::as_str)
      .unwrap_or_default()
      .to_string();
    let name = value
      .pointer("/file/original_name")
      .and_then(Value::as_str)
      .unwrap_or("image");
    // Hand back the finished snippet, not the parts. The alternative is the
    // model assembling a path from a spec it has to be told — a step that
    // buys nothing and can only go wrong.
    tool_result(Ok(json!({
        "ok": true,
        "action": "stored",
        "file_id": file_id,
        "name": name,
        "bytes": value.pointer("/file/byte_size").and_then(Value::as_i64),
        "markdown": format!(
            "![{name}](/api/workspaces/{workspace_id}/files/{file_id}/blob/{name})"
        ),
    })))
  }

  #[tool(
    description = "Fetch a stored image's actual pixels, so you can SEE it. Takes the \
                       file_id from an image href in a page's Markdown \
                       (`/files/{file_id}/blob/…`) or from mica_add_image. Reading the page \
                       gives you the link; this gives you the picture.",
    annotations(title = "Read image", read_only_hint = true)
  )]
  pub(crate) async fn mica_read_image(
    &self,
    Parameters(FileArgs {
      workspace_id,
      file_id,
    }): Parameters<FileArgs>,
  ) -> Result<CallToolResult, McpError> {
    let meta = match self
      .get(&format!("/api/workspaces/{workspace_id}/files/{file_id}"))
      .await
    {
      Ok(value) => value,
      Err(error) => return Ok(tool_error(error)),
    };
    let Some(download_url) = meta.pointer("/download_url").and_then(Value::as_str) else {
      return Ok(tool_error(McpError::internal_error(
        "Mica returned no download url for that file".to_string(),
        None,
      )));
    };
    let declared = meta
      .pointer("/file/mime_type")
      .and_then(Value::as_str)
      .unwrap_or("");
    let size = meta
      .pointer("/file/byte_size")
      .and_then(Value::as_i64)
      .unwrap_or(0);
    if size as usize > MAX_INLINE_IMAGE_BYTES {
      return Ok(tool_error(McpError::invalid_params(
        format!(
          "that image is {size} bytes — too large to inline (limit \
                     {MAX_INLINE_IMAGE_BYTES}). Open its href in a browser instead."
        ),
        None,
      )));
    }

    let (bytes, served) = match self.get_bytes(download_url).await {
      Ok(pair) => pair,
      Err(error) => return Ok(tool_error(error)),
    };
    // Trust the DECLARED type over the served one: object storage answers
    // `application/octet-stream` for anything it was handed without a type,
    // and a client that believes that renders nothing.
    let mime = if declared.starts_with("image/") {
      declared.to_string()
    } else {
      served.unwrap_or_else(|| "application/octet-stream".to_string())
    };
    if !mime.starts_with("image/") {
      return Ok(tool_error(McpError::invalid_params(
        format!("that file is {mime}, not an image — this tool only reads images"),
        None,
      )));
    }
    // The size check above used the RECORDED size; this one is the real
    // one. A row can lie; the bytes cannot.
    if bytes.len() > MAX_INLINE_IMAGE_BYTES {
      return Ok(tool_error(McpError::invalid_params(
        format!(
          "that image is {} bytes — too large to inline (limit {MAX_INLINE_IMAGE_BYTES})",
          bytes.len()
        ),
        None,
      )));
    }
    Ok(CallToolResult::success(vec![Content::image(
      base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes),
      mime,
    )]))
  }

}
