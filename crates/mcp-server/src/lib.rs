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
  ErrorData as McpError, ServerHandler, ServiceExt,
  handler::server::{router::tool::ToolRouter, wrapper::Parameters},
  model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
  schemars, tool, tool_handler, tool_router,
  transport::stdio,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone)]
struct MicaMcp {
  http: reqwest::Client,
  base: String,
  pat: String,
  read_only: bool,
  tool_router: ToolRouter<Self>,
}

impl MicaMcp {
  fn new(base: String, pat: String, read_only: bool) -> Self {
    Self {
      http: reqwest::Client::new(),
      base: base.trim_end_matches('/').to_string(),
      pat,
      read_only,
      tool_router: Self::tool_router(),
    }
  }

  /// Guard every mutating tool: refuse when the server is started read-only.
  fn ensure_writable(&self) -> Result<(), McpError> {
    if self.read_only {
      return Err(McpError::invalid_request(
        "this Mica MCP server is running read-only (MICA_MCP_READ_ONLY)".to_string(),
        None,
      ));
    }
    Ok(())
  }

  async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, McpError> {
    let resp = req
      .bearer_auth(&self.pat)
      .send()
      .await
      .map_err(|e| McpError::internal_error(format!("Mica request failed: {e}"), None))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
      // Cap the forwarded body — never dump a large or server-internal
      // error payload back to the model.
      let body: String = text.chars().take(400).collect();
      return Err(McpError::internal_error(
        format!("Mica API {status}: {body}"),
        None,
      ));
    }
    // Most endpoints return JSON; a couple return a bare string.
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
  }

  fn url(&self, path: &str) -> String {
    format!("{}{}", self.base, path)
  }

  async fn get(&self, path: &str) -> Result<Value, McpError> {
    self.send(self.http.get(self.url(path))).await
  }
  async fn post(&self, path: &str, body: Value) -> Result<Value, McpError> {
    self.send(self.http.post(self.url(path)).json(&body)).await
  }
  async fn patch(&self, path: &str, body: Value) -> Result<Value, McpError> {
    self.send(self.http.patch(self.url(path)).json(&body)).await
  }
  async fn delete(&self, path: &str) -> Result<Value, McpError> {
    self.send(self.http.delete(self.url(path))).await
  }

  /// Fetch raw bytes (not JSON) plus the served content type. Used only for
  /// blobs, which are the one thing here that is not text.
  async fn get_bytes(&self, url: &str) -> Result<(Vec<u8>, Option<String>), McpError> {
    let resp = self
      .http
      .get(url)
      .send()
      .await
      .map_err(|e| McpError::internal_error(format!("Mica blob fetch failed: {e}"), None))?;
    let status = resp.status();
    if !status.is_success() {
      return Err(McpError::internal_error(
        format!("Mica blob fetch {status}"),
        None,
      ));
    }
    let mime = resp
      .headers()
      .get(reqwest::header::CONTENT_TYPE)
      .and_then(|v| v.to_str().ok())
      .map(|v| v.split(';').next().unwrap_or(v).trim().to_string());
    let bytes = resp
      .bytes()
      .await
      .map_err(|e| McpError::internal_error(format!("Mica blob read failed: {e}"), None))?;
    Ok((bytes.to_vec(), mime))
  }
}

/// Refuse to inline an image so large it would blow the context it is being
/// read into. MCP has no streaming for ImageContent — it is one base64 string
/// in one tool result — and base64 inflates by 4/3 before the model's encoder
/// ever sees it. A cap with a readable reason beats a 40MB reply that wedges
/// the conversation; the href still works in a browser.
const MAX_INLINE_IMAGE_BYTES: usize = 4 * 1024 * 1024;

// ── Tool parameters ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WorkspaceArg {
  /// The workspace id (from `mica_list_workspaces`).
  workspace_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchArgs {
  workspace_id: String,
  /// Free-text query matched against page titles.
  query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DocArg {
  workspace_id: String,
  /// The document's object id (a page view's `object_id`).
  document_id: String,
}

/// Control characters a JSON unescape produces from a LaTeX command, paired with
/// the escape that made them, and an example of the command each one eats.
/// `\times` under-escaped in JSON is not an error — `\t` is a legal escape — so
/// it decodes silently to TAB + "imes" and the formula is destroyed with no
/// diagnostic anywhere.
///
/// These four are safe to condemn: none has a legitimate reason to appear inside
/// a formula. `\n` and `\r` collide the same way (`\nabla`, `\rho`) but are
/// deliberately absent — a real newline is ordinary markdown and we cannot tell
/// the two apart. Their damage is at least visible: a newline breaks the math
/// run outright, so the page shows literal `$…$` rather than quietly wrong math.
const JSON_ESCAPE_COLLISIONS: [(char, &str, &str); 4] = [
  ('\u{0009}', r"\t", r"\times"),
  ('\u{000C}', r"\f", r"\frac"),
  ('\u{000B}', r"\v", r"\vec"),
  ('\u{0008}', r"\b", r"\beta"),
];

/// Reject markdown whose LaTeX was mangled by an under-escaped JSON string.
///
/// We are not the ones corrupting it — the server stores faithfully what it is
/// given, and a correctly escaped `\\times` round-trips fine. But this arrives
/// as valid JSON that is silently wrong, and the caller is usually a model
/// hand-writing escapes, which gets LaTeX wrong often enough that "AI writes its
/// answer into a page" (formulas and all) cannot be trusted without a check.
/// Refusing with a precise message lets the caller retry correctly; storing it
/// loses the formula forever, which is the failure this guard exists to stop.
///
/// Two rules, both chosen to be false-positive-free rather than thorough:
///  - FF / VT / BS anywhere: no legitimate markdown contains them.
///  - TAB *inside a math run only*: a tab is ordinary indentation elsewhere, but
///    inside `$…$` it is meaningless to LaTeX and means `\t…` was eaten.
fn reject_mangled_latex(markdown: &str) -> Result<(), McpError> {
  for (ch, escape, example) in JSON_ESCAPE_COLLISIONS {
    if !markdown.contains(ch) {
      continue;
    }
    // A tab is ordinary indentation outside a formula, so it only condemns
    // itself within one. The other three have no innocent reading anywhere.
    if ch == '\u{0009}' && !tab_inside_math(markdown) {
      continue;
    }
    return Err(mangled_latex_error(escape, example));
  }
  Ok(())
}

fn tab_inside_math(markdown: &str) -> bool {
  let chars: Vec<char> = markdown.chars().collect();
  mica_markdown::math_run_spans(markdown)
    .into_iter()
    .any(|(start, end)| chars[start..end].contains(&'\u{0009}'))
}

fn mangled_latex_error(escape: &str, example: &str) -> McpError {
  McpError::invalid_params(
    format!(
      "The markdown contains a raw control character where a LaTeX command should be: \
             `{escape}` was read as a JSON escape, so `{example}` arrived as a control character \
             instead of a command. The backslashes in the JSON string were under-escaped — in \
             JSON a literal backslash must be doubled. Send \"$\\\\times$\", not \"$\\times$\" \
             (the latter is a tab). Nothing was written; resend with every backslash doubled."
    ),
    None,
  )
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateDocArgs {
  workspace_id: String,
  /// Page title.
  name: String,
  /// Initial body as Markdown. Omit for an empty page.
  #[serde(default)]
  markdown: Option<String>,
  /// Parent view id to nest under (a folder). Omit for a top-level page.
  #[serde(default)]
  parent_view_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UpdateDocArgs {
  workspace_id: String,
  /// The document's object id (a page's `object_id` from mica_list_pages) —
  /// NOT its view id (that's for move/trash).
  document_id: String,
  /// One of: "append" (add after current content — safe default),
  /// "replace_all" (rewrite the page), "insert_at" (insert after `anchor`),
  /// "find_replace" (swap text; uses `find`/`replace`, not `markdown`).
  mode: String,
  /// Markdown to write (append/replace_all/insert_at). Omit for find_replace.
  #[serde(default)]
  markdown: Option<String>,
  /// insert_at: the block id to insert after (from `mica_get_outline`).
  #[serde(default)]
  anchor: Option<String>,
  /// find_replace: the text to find and its replacement.
  #[serde(default)]
  find: Option<String>,
  #[serde(default)]
  replace: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MoveDocArgs {
  workspace_id: String,
  /// The page's VIEW id (from `mica_list_pages`), not its object_id.
  view_id: String,
  /// New parent view id (a folder), or omit/null to move to the top level.
  #[serde(default)]
  parent_view_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReorderArgs {
  workspace_id: String,
  /// The folder whose children to reorder (a folder's view id), or omit/null
  /// for the workspace's top level.
  #[serde(default)]
  parent_view_id: Option<String>,
  /// The COMPLETE list of that parent's child VIEW ids, in the order you want
  /// them. Pass every child — leaving one out keeps it at a stale position that
  /// interleaves with the sorted ones.
  ordered_view_ids: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RenameArgs {
  workspace_id: String,
  /// The VIEW id to rename (a page OR a folder), from `mica_list_pages`.
  view_id: String,
  /// The new name.
  name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateFolderArgs {
  workspace_id: String,
  /// The new folder's name.
  name: String,
  /// Parent folder view id, or omit/null for the workspace's top level.
  #[serde(default)]
  parent_view_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ViewArg {
  workspace_id: String,
  /// The VIEW id (page or folder).
  view_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateVersionArgs {
  workspace_id: String,
  /// The document's object id (a page view's `object_id`).
  document_id: String,
  /// A label for this checkpoint (e.g. "before the rewrite").
  name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RestoreVersionArgs {
  workspace_id: String,
  /// The document's object id.
  document_id: String,
  /// The version id to restore to (from `mica_list_versions`).
  version_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AddImageArgs {
  workspace_id: String,
  /// A public http(s) URL. The server fetches it once and keeps the bytes;
  /// identical bytes deduplicate to one stored file.
  url: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FileArgs {
  workspace_id: String,
  /// The file's id — the uuid in an image href's `/files/{file_id}/blob/…`.
  file_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TrashArgs {
  workspace_id: String,
  /// The VIEW id to trash (its whole subtree goes to the recycle bin).
  view_id: String,
  /// Must be true to proceed — a guard against accidental deletion.
  confirm: bool,
}

// ── Tools ───────────────────────────────────────────────────────────────────

/// Return an API payload as MCP text content.
///
/// Deliberately NOT `Json<Value>`, which is what the tools used to return: rmcp
/// derives an `outputSchema` from the return type, and `Value` is "any", so it
/// emitted `{"title": "AnyValue"}` with no `"type": "object"`. The MCP spec
/// requires an object schema there, so a validating client REJECTS the whole
/// tools/list — Claude Code logged `tools/list failed … expected "object"` and
/// registered nothing, even though the handshake and every tool were fine.
/// `outputSchema` is optional; the honest move for a proxy returning whatever
/// the REST API said is to not declare one at all.
fn tool_result(result: Result<Value, McpError>) -> Result<CallToolResult, McpError> {
  match result {
    Ok(value) => {
      let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
      Ok(CallToolResult::success(vec![Content::text(text)]))
    }
    // An API failure is a TOOL error, not a protocol error. The spec splits
    // them by who can act on them: JSON-RPC errors are for structural faults
    // the model cannot fix (unknown tool, malformed request), while anything
    // the model could react to — API failures, validation, business rules —
    // must come back as `isError: true` content so it can read the reason and
    // retry. Reporting a 403 or a bad-input message as a protocol error hides
    // it from the very reader it was written for.
    Err(error) => Ok(tool_error(error)),
  }
}

/// Turn an internal error into a tool-execution error the model can read.
fn tool_error(error: McpError) -> CallToolResult {
  CallToolResult::error(vec![Content::text(error.message.to_string())])
}

/// Answer a write with WHAT HAPPENED, not with the document itself.
///
/// A tool result is the model's context — that is MCP's whole design — and it is
/// re-sent on every later turn, so a fat result is not a one-off cost, it
/// compounds for the rest of the conversation. The REST API returns the full
/// snapshot because the EDITOR needs the blocks to render; a model needs the id
/// and nothing else. Forwarding it verbatim turned a 558-byte page into a 6.4KB
/// reply (~2k tokens), and a real page into far more — measured, not guessed.
///
/// Falls back to the whole payload when nothing is recognisable: if an endpoint
/// changes shape, the caller should see too much rather than a silent "ok".
fn write_ack(value: Value) -> Result<CallToolResult, McpError> {
  let mut ack = serde_json::Map::new();
  for (at, key) in [
    ("/document/id", "document_id"),
    ("/view/id", "view_id"),
    ("/view/name", "name"),
  ] {
    if let Some(found) = value.pointer(at).and_then(Value::as_str) {
      ack.insert(key.to_string(), json!(found));
    }
  }
  // A count, not the blocks: enough to confirm the write landed and roughly
  // how big it is, without shipping the tree.
  if let Some(blocks) = value
    .pointer("/snapshot/payload/blocks")
    .and_then(Value::as_array)
  {
    ack.insert("blocks".to_string(), json!(blocks.len()));
  }
  if let Some(seq) = value.pointer("/update/seq").and_then(Value::as_i64) {
    ack.insert("seq".to_string(), json!(seq));
  }
  if ack.is_empty() {
    return tool_result(Ok(value));
  }
  ack.insert("ok".to_string(), json!(true));
  tool_result(Ok(Value::Object(ack)))
}

/// Answer a write whose endpoint replies with a LIST rather than the thing
/// written. Move and trash both return the workspace's remaining views, because
/// the sidebar re-renders from that — but the model already knows the id it
/// acted on (it supplied it), so echoing the tree back is pure cost: 6.4KB on a
/// 14-page workspace to report one deletion. `action` says which side of the
/// call happened, so "did it land?" needs no follow-up read.
fn action_ack(
  result: Result<Value, McpError>,
  action: &str,
  view_id: &str,
) -> Result<CallToolResult, McpError> {
  match result {
    Ok(_) => tool_result(Ok(json!({
        "ok": true,
        "action": action,
        "view_id": view_id,
    }))),
    Err(error) => Ok(tool_error(error)),
  }
}

/// Keep only the fields a model navigates by; drop the storage bookkeeping.
///
/// The `/views` endpoint answers the Flutter client, which needs every column
/// (position for lexo-ordering, icon, timestamps, is_deleted…). A model needs
/// none of that — just enough to pick a page and hand its ids to another tool.
/// On a 14-page workspace the full list was 6.3KB, more than half of it
/// `workspace_id` (identical on every row, and supplied by the caller in the
/// first place), `created_by`, `position`, and timestamps. This is the read-side
/// twin of `write_ack`: the API stays whole and honest, the proxy fits the
/// answer to its audience.
fn slim_pages(value: Value) -> Value {
  let Some(views) = value.get("views").and_then(Value::as_array) else {
    return value;
  };
  let slim: Vec<Value> = views
    .iter()
    .map(|v| {
      let mut row = serde_json::Map::new();
      // id → the view id, for move/trash. object_id → the document id, for
      // read/outline/update. object_type → folder vs document. The rest is what
      // a human-facing tree needs and a model does not.
      for key in ["id", "object_id", "object_type", "name", "parent_view_id"] {
        if let Some(found) = v.get(key)
          && !found.is_null()
        {
          row.insert(key.to_string(), found.clone());
        }
      }
      // icon only when set — an emoji is a cheap, useful disambiguator; a null
      // is just noise.
      if let Some(icon) = v.get("icon").filter(|i| !i.is_null()) {
        row.insert("icon".to_string(), icon.clone());
      }
      Value::Object(row)
    })
    .collect();
  json!({ "pages": slim })
}

#[tool_router]
impl MicaMcp {
  #[tool(
    description = "List all Mica workspaces (id, name, role) the token can access.",
    annotations(read_only_hint = true)
  )]
  async fn mica_list_workspaces(&self) -> Result<CallToolResult, McpError> {
    tool_result(self.get("/api/workspaces").await)
  }

  #[tool(
    description = "List a workspace's page tree (documents + folders, with ids, names, \
                       parents). Use a page's object_id with the read/write tools.",
    annotations(read_only_hint = true)
  )]
  async fn mica_list_pages(
    &self,
    Parameters(WorkspaceArg { workspace_id }): Parameters<WorkspaceArg>,
  ) -> Result<CallToolResult, McpError> {
    let listed = self
      .get(&format!("/api/workspaces/{workspace_id}/views"))
      .await;
    tool_result(listed.map(slim_pages))
  }

  // The description is the model's ONLY knowledge of what this can do, and it
  // used to say "by title" — which was false, and quietly expensive: the
  // endpoint has always scanned body text too, so every agent was told the
  // index did not exist and fell back to reading whole pages to find a
  // phrase. Say what it actually does, and say what it costs.
  #[tool(
    description = "Find pages by TITLE **and body text**. Returns each hit's view_id, \
                       object_id, page name, and a snippet of the matching body text \
                       (`title_match` tells you which side matched). Prefer this over reading \
                       pages when looking for something: a hit costs a fraction of a whole \
                       document, and the snippet is often answer enough on its own.",
    annotations(read_only_hint = true)
  )]
  async fn mica_search(
    &self,
    Parameters(SearchArgs {
      workspace_id,
      query,
    }): Parameters<SearchArgs>,
  ) -> Result<CallToolResult, McpError> {
    let q = urlencode(&query);
    tool_result(
      self
        .get(&format!("/api/workspaces/{workspace_id}/search?q={q}"))
        .await,
    )
  }

  #[tool(
    description = "Read a document's content as Markdown.",
    annotations(read_only_hint = true)
  )]
  async fn mica_read_document(
    &self,
    Parameters(DocArg {
      workspace_id,
      document_id,
    }): Parameters<DocArg>,
  ) -> Result<CallToolResult, McpError> {
    tool_result(
      self
        .get(&format!(
          "/api/workspaces/{workspace_id}/documents/{document_id}/export/markdown"
        ))
        .await,
    )
  }

  #[tool(
    description = "Get a document's outline (headings + block ids in order). Call this \
                       before an anchored write so you can target a spot instead of rewriting \
                       the whole page.",
    annotations(read_only_hint = true)
  )]
  async fn mica_get_outline(
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

  #[tool(
    description = "Create a new page (optionally with Markdown body) in a workspace.",
    annotations(read_only_hint = false)
  )]
  async fn mica_create_document(
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
    description = "Write into an EXISTING document. mode: append (after current content, \
                       the safe default), replace_all (rewrite), insert_at (place after \
                       `anchor` from mica_get_outline — a local edit), find_replace (swap \
                       `find`→`replace`). Content is Markdown; the server derives the ops.",
    annotations(
      title = "Write document",
      read_only_hint = false,
      destructive_hint = true
    )
  )]
  async fn mica_update_document(
    &self,
    Parameters(UpdateDocArgs {
      workspace_id,
      document_id,
      mode,
      markdown,
      anchor,
      find,
      replace,
    }): Parameters<UpdateDocArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    // Every field that carries authored content, not just `markdown`:
    // find_replace writes through `replace`, and a swapped-in formula is
    // mangled by the same under-escaping.
    for text in [markdown.as_deref(), replace.as_deref()]
      .into_iter()
      .flatten()
    {
      if let Err(error) = reject_mangled_latex(text) {
        return Ok(tool_error(error));
      }
    }
    let body = json!({
        "mode": mode,
        "markdown": markdown.unwrap_or_default(),
        "anchor": anchor,
        "find": find,
        "replace": replace,
    });
    match self
      .patch(
        &format!("/api/workspaces/{workspace_id}/documents/{document_id}/markdown"),
        body,
      )
      .await
    {
      Ok(value) => write_ack(value),
      Err(error) => Ok(tool_error(error)),
    }
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
  async fn mica_add_image(
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
  async fn mica_read_image(
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

  #[tool(
    description = "Move a page under a new parent folder (or to the top level).",
    annotations(read_only_hint = false)
  )]
  async fn mica_move_document(
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
    action_ack(moved, "moved", &view_id)
  }

  #[tool(
    description = "Reorder a folder's children (or the workspace's top level) in ONE call: pass \
                   the complete list of child view ids in the order you want. Use this to SORT — \
                   the per-page move tool would take one call per item. Get the current children \
                   and their ids from mica_list_pages.",
    annotations(read_only_hint = false)
  )]
  async fn mica_reorder(
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
  async fn mica_rename(
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
  async fn mica_create_folder(
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
  async fn mica_list_trash(
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
  async fn mica_restore_view(
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
    action_ack(restored, "restored", &view_id)
  }

  #[tool(
    description = "List a document's named versions (restorable checkpoints), newest first. The \
                   raw edit log is omitted — these are the points you can roll back to.",
    annotations(read_only_hint = true)
  )]
  async fn mica_list_versions(
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
  async fn mica_create_version(
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
  async fn mica_restore_version(
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
    match restored {
      Ok(_) => tool_result(Ok(json!({
        "ok": true, "action": "restored_version", "version_id": version_id
      }))),
      Err(error) => Ok(tool_error(error)),
    }
  }

  #[tool(
    description = "Move a page (and its subtree) to the recycle bin — a SOFT delete, \
                       recoverable in the app. Requires confirm=true. Permanent deletion is \
                       not exposed here.",
    annotations(title = "Trash page", read_only_hint = false, destructive_hint = true)
  )]
  async fn mica_trash_view(
    &self,
    Parameters(TrashArgs {
      workspace_id,
      view_id,
      confirm,
    }): Parameters<TrashArgs>,
  ) -> Result<CallToolResult, McpError> {
    if let Err(error) = self.ensure_writable() {
      return Ok(tool_error(error));
    }
    if !confirm {
      return Err(McpError::invalid_params(
        "refusing to trash without confirm=true".to_string(),
        None,
      ));
    }
    let trashed = self
      .delete(&format!("/api/workspaces/{workspace_id}/views/{view_id}"))
      .await;
    action_ack(trashed, "trashed", &view_id)
  }

  #[tool(
    description = "Publish a document to a PUBLIC read-only web link and return the URL. Anyone \
                   with the link can read it (no login); re-sharing returns the same link. Use \
                   mica_unshare to turn it off.",
    annotations(title = "Share to web", read_only_hint = false)
  )]
  async fn mica_share(
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
  async fn mica_unshare(
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
  async fn mica_export_workspace(
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

#[tool_handler]
impl ServerHandler for MicaMcp {
  fn get_info(&self) -> ServerInfo {
    ServerInfo {
      // Declaring `tools` is NOT optional and NOT implied by having them.
      // `#[tool_handler]` implements tools/list and tools/call, but a
      // client reads `capabilities` during initialize and never calls a
      // method the server did not advertise — so with the default (empty)
      // capabilities the server connects, reports healthy, and exposes
      // exactly nothing. Every tool below is dead without this line.
      capabilities: ServerCapabilities::builder().enable_tools().build(),
      server_info: Implementation {
        name: "mica-mcp".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        ..Default::default()
      },
      instructions: Some(
        "Mica note-workspace MCP server. List/read/create/write documents and \
                 export/import workspaces over the Mica REST API. Workflow: list workspaces \
                 → list pages → read/outline → create/update. Writes take Markdown; the \
                 server derives the block ops."
          .to_string(),
      ),
      ..Default::default()
    }
  }
}

/// Minimal percent-encoding for a query value (space + a few reserved chars).
fn urlencode(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for b in s.bytes() {
    match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
      _ => out.push_str(&format!("%{b:02X}")),
    }
  }
  out
}

/// Serve the Mica MCP server over stdio until the client disconnects.
///
/// A LIBRARY entry rather than a binary: `mica-cli mcp` is the shipped front
/// door (one artifact per platform instead of two — CI only ever published
/// mica-cli anyway, so a standalone mica-mcp binary was a build users could
/// not download). The caller resolves [base]/[pat] from its own config chain;
/// this crate no longer reads the environment.
pub async fn serve_stdio(base: String, pat: String, read_only: bool) -> anyhow::Result<()> {
  // Logs go to stderr — stdout is the MCP JSON-RPC channel and must stay clean.
  // try_init, not init: the embedding CLI may set up tracing itself one day,
  // and a second init would panic mid-handshake.
  let _ = tracing_subscriber::fmt()
    .with_writer(std::io::stderr)
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
    )
    .try_init();

  let service = MicaMcp::new(base, pat, read_only).serve(stdio()).await?;
  service.waiting().await?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::{reject_mangled_latex, urlencode};

  #[test]
  fn urlencode_escapes_reserved_keeps_unreserved() {
    assert_eq!(urlencode("hello world"), "hello%20world");
    assert_eq!(urlencode("a&b=c/d?e"), "a%26b%3Dc%2Fd%3Fe");
    assert_eq!(urlencode("keep-_.~AZaz09"), "keep-_.~AZaz09");
  }

  /// Byte-for-byte what reached the database from a real MCP write of
  /// `$\eta = 2 \times \frac{N-1}{N}$` with under-escaped backslashes. Spelled
  /// with \u{..} rather than Rust's own `\t`, because `"\times"` in Rust
  /// source IS tab+"imes" — the very confusion that caused the bug, and not
  /// something a test about it should re-enact.
  #[test]
  fn rejects_the_corruption_that_actually_shipped() {
    let mangled = concat!(
      "**A**: For $N$ nodes it is $\\eta = 2 ",
      "\u{0009}imes \u{000C}rac{N-1}{N}$, approaching $2$."
    );
    // Precondition: this really is the shape we saw — \eta intact, the
    // other two commands eaten down to a control char.
    assert!(mangled.contains(r"\eta"));
    assert!(!mangled.contains(r"\times") && !mangled.contains(r"\frac"));
    assert!(reject_mangled_latex(mangled).is_err());
  }

  #[test]
  fn rejects_each_collision_char() {
    // \f, \v, \b are impossible in real markdown — rejected anywhere.
    assert!(reject_mangled_latex("a \u{000C}rac b").is_err()); // \frac
    assert!(reject_mangled_latex("a \u{000B}ec b").is_err()); // \vec
    assert!(reject_mangled_latex("a \u{0008}eta b").is_err()); // \beta
    // A tab only condemns itself inside a formula.
    assert!(reject_mangled_latex("$x \u{0009}imes y$").is_err()); // \times
  }

  /// The guard must never fire on content someone legitimately wrote, or it
  /// blocks the very workflow it protects.
  #[test]
  fn accepts_legitimate_content() {
    // Correctly escaped LaTeX — the whole point.
    assert!(reject_mangled_latex(r"$\eta = 2 \times \frac{N-1}{N}$").is_ok());
    // A tab outside math is ordinary indentation / code.
    assert!(reject_mangled_latex("- item\n\u{0009}continued\n\n\u{0009}code();").is_ok());
    assert!(reject_mangled_latex("| a\u{0009}| b |\n|---|---|").is_ok());
    // Currency is not math (no valid closer), so its tabs stay innocent.
    assert!(reject_mangled_latex("costs $5\u{0009}and $10").is_ok());
    // Plain prose, newlines and CR are untouched.
    assert!(reject_mangled_latex("line one\r\nline two\n").is_ok());
  }

  /// A `$` inside a code span never opens a run (§6.1), so a tab after it is
  /// not "inside math" — proves the guard uses the parser's real rules.
  #[test]
  fn a_tab_after_a_dollar_in_a_code_span_is_not_math() {
    assert!(reject_mangled_latex("`$HOME`\tand `$PATH`\tare paths").is_ok());
  }
}

#[cfg(test)]
mod handshake_tests {
  /// A write must answer "what happened", never hand the document back. The
  /// REST payload carries the whole block tree for the editor's benefit; a
  /// 558-byte page measured 6.4KB (~2k tokens) of tool result, re-sent every
  /// later turn. Pinned on SIZE, because the regression is silent: forwarding
  /// more is always "correct", just ruinous.
  #[test]
  fn a_write_ack_carries_ids_not_the_document() {
    let blocks: Vec<Value> = (0..40)
      .map(|i| json!({"id": format!("block_{i}"), "text": "x".repeat(200)}))
      .collect();
    let rest = json!({
        "document": {"id": "doc-1", "root_block_id": "block_0", "current_seq": 0},
        "snapshot": {"payload": {"blocks": blocks}},
        "view": {"id": "view-1", "name": "Notes", "position": "0000000010"},
    });
    let fat = serde_json::to_string(&rest).unwrap().len();

    let out = write_ack(rest).expect("ack");
    let text = format!("{:?}", out.content);
    assert!(text.contains("doc-1") && text.contains("view-1") && text.contains("Notes"));
    assert!(
      text.contains("40"),
      "block COUNT, so the write is confirmable"
    );
    assert!(!text.contains("block_39"), "no block ids");
    assert!(!text.contains(&"x".repeat(200)), "no block text");
    assert!(
      text.len() * 20 < fat,
      "ack must be a fraction of the payload: {} vs {fat}",
      text.len()
    );
  }

  /// The page list is what a model reads to navigate, and it was 6.3KB on 14
  /// pages — more than half of it storage bookkeeping the model never uses. The
  /// slim keeps exactly the navigation fields and drops the rest, and the whole
  /// point is that `workspace_id` (identical on every row, and supplied by the
  /// caller) is gone.
  #[test]
  fn list_pages_keeps_navigation_fields_and_drops_bookkeeping() {
    let full = json!({
      "views": [
        {
          "id": "view-1",
          "workspace_id": "ws-1",
          "parent_view_id": null,
          "object_id": "doc-1",
          "object_type": "document",
          "name": "Notes",
          "icon": "📄",
          "position": "0000000010",
          "is_deleted": false,
          "created_by": "user-1",
          "created_at": "2026-07-16T00:00:00Z",
          "updated_at": "2026-07-16T00:00:00Z"
        },
        {
          "id": "view-2",
          "workspace_id": "ws-1",
          "parent_view_id": "view-1",
          "object_id": "obj-2",
          "object_type": "folder",
          "name": "Sub",
          "icon": null,
          "position": "0000000020",
          "is_deleted": false,
          "created_by": "user-1",
          "created_at": "2026-07-16T00:00:00Z",
          "updated_at": "2026-07-16T00:00:00Z"
        }
      ]
    });
    let fat = serde_json::to_string(&full).unwrap().len();

    let slim = slim_pages(full);
    let text = serde_json::to_string(&slim).unwrap();
    // Every navigation field a tool consumes survives.
    for kept in [
      "view-1", "doc-1", "document", "Notes", "📄", "view-2", "folder",
    ] {
      assert!(text.contains(kept), "dropped a needed field: {kept}");
    }
    // parent linkage is kept (it is the tree), but only when present — the root
    // page's null parent is elided, not echoed.
    assert!(
      text.contains("\"parent_view_id\":\"view-1\""),
      "child keeps parent"
    );
    // The bookkeeping is gone. workspace_id is the marquee case: same on every
    // row and the caller passed it in.
    for gone in [
      "ws-1",
      "user-1",
      "0000000010",
      "is_deleted",
      "created_at",
      "updated_at",
    ] {
      assert!(!text.contains(gone), "should have dropped: {gone}");
    }
    // A null icon is noise, not information.
    assert!(!text.contains("\"icon\":null"), "null icon elided");
    assert!(
      text.len() * 2 < fat,
      "slim must be well under half: {} vs {fat}",
      text.len()
    );
  }

  /// If the endpoint stops returning `{views: [...]}`, forward what it did send
  /// rather than silently return an empty page list.
  #[test]
  fn list_pages_forwards_an_unexpected_shape() {
    let odd = json!({"surprise": "shape"});
    assert_eq!(slim_pages(odd.clone()), odd);
  }

  /// Move and trash reply with the workspace's REMAINING views (the sidebar
  /// rebuilds from that list), so `write_ack` finds no id to lift and falls
  /// back to forwarding the lot — a whole page tree to report one deletion.
  /// These two take the id from the caller instead and never read the body.
  #[test]
  fn move_and_trash_ack_the_action_not_the_page_tree() {
    let views: Vec<Value> = (0..14)
      .map(|i| {
        json!({
            "id": format!("view_{i}"),
            "workspace_id": "ws-1",
            "created_by": "user-1",
            "name": format!("Page {i}"),
            "position": "0000000010",
        })
      })
      .collect();
    let tree = json!({ "views": views });
    let fat = serde_json::to_string(&tree).unwrap().len();

    let out = action_ack(Ok(tree), "trashed", "view-9").expect("ack");
    let text = format!("{:?}", out.content);
    assert!(text.contains("view-9"), "the id acted on");
    assert!(text.contains("trashed"), "WHICH action landed");
    assert!(!text.contains("view_3"), "no sibling views");
    assert!(!text.contains("Page 7"), "no sibling names");
    assert!(
      text.len() * 5 < fat,
      "ack must be a fraction of the tree: {} vs {fat}",
      text.len()
    );
  }

  /// A failed move must not report `ok` — the ack ignores the body, so the
  /// error is the ONLY thing separating "moved" from "did nothing".
  #[test]
  fn a_failed_action_is_an_error_not_an_ok() {
    let out = action_ack(
      Err(McpError::internal_error("Mica API 403".to_string(), None)),
      "moved",
      "view-9",
    )
    .expect("result");
    assert_eq!(out.is_error, Some(true));
    assert!(format!("{:?}", out.content).contains("403"));
  }

  /// Images were the one thing an agent could neither store nor see: writing
  /// `![](https://…)` left the bytes outside Mica, and reading a page gave
  /// back `![](pasted-image.png)` — every uploaded image, in every workspace,
  /// under the same unresolvable name. Both halves must be reachable, and
  /// `mica_add_image`'s whole point is handing back paste-ready Markdown.
  #[test]
  fn the_image_tools_are_listed_and_carry_their_arguments() {
    let router = MicaMcp::tool_router();
    let listed = router.list_all();
    let names: Vec<&str> = listed.iter().map(|t| t.name.as_ref()).collect();
    assert!(names.contains(&"mica_add_image"), "store: {names:?}");
    assert!(names.contains(&"mica_read_image"), "read back: {names:?}");

    let add = listed
      .into_iter()
      .find(|t| t.name == "mica_add_image")
      .expect("listed");
    let props = add.input_schema.get("properties").expect("properties");
    assert!(props.get("url").is_some() && props.get("workspace_id").is_some());
    // The description must promise the paste-ready snippet — that promise is
    // the only reason a model reaches for this instead of writing the raw
    // URL, which is the exact mistake it exists to prevent.
    let desc = add.description.unwrap_or_default().to_lowercase();
    assert!(desc.contains("markdown"), "must offer the snippet: {desc}");
  }

  /// Reordering was impossible through MCP: `mica_move_document` dropped the
  /// `position` the move endpoint accepts, so "sort this folder" had no path.
  /// `mica_reorder` closes that gap, and its whole value is doing it in ONE call
  /// with the ordered list — the description must say so, and it must carry the
  /// ordered ids.
  #[test]
  fn reorder_is_listed_and_takes_an_ordered_list() {
    let router = MicaMcp::tool_router();
    let listed = router.list_all();
    let names: Vec<&str> = listed.iter().map(|t| t.name.as_ref()).collect();
    assert!(
      names.contains(&"mica_reorder"),
      "reorder missing: {names:?}"
    );

    let t = listed
      .into_iter()
      .find(|t| t.name == "mica_reorder")
      .expect("listed");
    let props = t.input_schema.get("properties").expect("properties");
    assert!(
      props.get("ordered_view_ids").is_some(),
      "must take the ordered ids: {props:?}"
    );
    let desc = t.description.unwrap_or_default().to_lowercase();
    assert!(
      desc.contains("one call") || desc.contains("sort"),
      "must sell the one-call sort: {desc}"
    );
  }

  /// The organize + history toolkit an agent needs to actually tidy a knowledge
  /// base — every one was a gap the app had but MCP didn't expose. If any is
  /// dropped, "rename this", "make a folder", "roll back" silently can't be done.
  #[test]
  fn the_organize_and_history_tools_are_all_listed() {
    let names: Vec<String> = MicaMcp::tool_router()
      .list_all()
      .iter()
      .map(|t| t.name.to_string())
      .collect();
    for expected in [
      "mica_rename",
      "mica_create_folder",
      "mica_list_trash",
      "mica_restore_view",
      "mica_list_versions",
      "mica_create_version",
      "mica_restore_version",
    ] {
      assert!(
        names.iter().any(|n| n == expected),
        "{expected} missing from {names:?}"
      );
    }
  }

  /// If an endpoint changes shape, show the caller everything rather than
  /// swallow the answer into a confident, empty "ok".
  #[test]
  fn an_unrecognised_payload_is_forwarded_whole() {
    let out = write_ack(json!({"surprise": "shape"})).expect("ack");
    assert!(format!("{:?}", out.content).contains("surprise"));
  }

  use super::*;
  use rmcp::ServerHandler;

  /// The bug this pins shipped and reached a real client: `get_info` filled
  /// `capabilities` from `Default` (i.e. empty), so initialize answered
  /// `capabilities: {}`. Clients read that and register nothing — Claude Code
  /// logged `hasTools:false` and exposed zero tools while still reporting the
  /// server "✓ Connected". Every tool in this file was dead.
  ///
  /// It survived because the probe used to "verify" it called `tools/list`
  /// explicitly, which `#[tool_handler]` answers regardless of what was
  /// advertised. A real client never asks for a capability the server did not
  /// declare — so the check has to be on the DECLARATION, not on whether
  /// tools/list happens to work.
  #[test]
  fn initialize_advertises_the_tools_capability() {
    let info = MicaMcp::new("https://example.test".into(), "pat".into(), false).get_info();
    assert!(
      info.capabilities.tools.is_some(),
      "server must advertise `tools` or every client registers zero tools"
    );
  }

  /// Read-only must not hide the tools: writes refuse at call time (see
  /// `ensure_writable`) but stay listed, so the model can see them and be told
  /// why rather than silently lacking them.
  #[test]
  fn read_only_still_advertises_tools() {
    let info = MicaMcp::new("https://example.test".into(), "pat".into(), true).get_info();
    assert!(info.capabilities.tools.is_some());
  }
  /// The second half of the same outage. With `tools` finally advertised, the
  /// client fetched tools/list and REJECTED it wholesale:
  ///   `path: ["tools",0,"outputSchema","type"] — expected "object"`.
  /// rmcp derives outputSchema from the return type, and the tools returned
  /// `Json<Value>`; `Value` is "any", so it emitted `{"title":"AnyValue"}` with
  /// no `"type"`. MCP requires an object schema there — one bad tool and the
  /// whole list is thrown away.
  ///
  /// outputSchema is optional, so the fix is to declare none (see `ok_json`).
  /// Pinned on the WIRE shape, because that is what the client validates; a
  /// test that only called a tool would pass while every client saw zero tools.
  #[test]
  fn no_tool_declares_an_output_schema() {
    let router = MicaMcp::tool_router();
    let tools = router.list_all();
    assert_eq!(tools.len(), 22, "every tool must be listed");
    for t in &tools {
      assert!(
        t.output_schema.is_none(),
        "{}: an outputSchema derived from Value is not an object schema,                  and one invalid entry makes a client discard the entire tools/list",
        t.name
      );
    }
  }

  /// Inputs, by contrast, MUST be object schemas — that half was always right,
  /// and this keeps it that way.
  #[test]
  fn every_tool_input_schema_is_an_object() {
    for t in MicaMcp::tool_router().list_all() {
      assert_eq!(
        t.input_schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "{}: inputSchema must be an object schema",
        t.name
      );
    }
  }
  /// The spec splits errors by who can act on them: JSON-RPC errors are for
  /// structural faults the model cannot fix; anything it could react to —
  /// API failures, validation, business rules — must come back as
  /// `isError: true` content so it can read the reason and retry (SEP-1303).
  ///
  /// This was wrong in all three places, and the LaTeX guard was the sharpest
  /// case: it exists precisely so the caller re-sends with doubled
  /// backslashes, yet it reported through the one channel meant for errors
  /// the model is NOT expected to act on.
  #[tokio::test]
  async fn a_read_only_refusal_is_a_tool_error_not_a_protocol_error() {
    let server = MicaMcp::new("https://example.invalid".into(), "pat".into(), true);
    let result = server
      .mica_create_document(Parameters(CreateDocArgs {
        workspace_id: "w".into(),
        name: "n".into(),
        markdown: None,
        parent_view_id: None,
      }))
      .await
      .expect("read-only must not surface as a protocol error");
    assert_eq!(result.is_error, Some(true));
  }

  #[tokio::test]
  async fn mangled_latex_is_a_tool_error_the_model_can_read() {
    let server = MicaMcp::new("https://example.invalid".into(), "pat".into(), false);
    // tab + "imes" — exactly what an under-escaped `\times` decodes to.
    let mangled = format!("$x {}imes y$", '\u{0009}');
    let result = server
      .mica_create_document(Parameters(CreateDocArgs {
        workspace_id: "w".into(),
        name: "n".into(),
        markdown: Some(mangled),
        parent_view_id: None,
      }))
      .await
      .expect("validation must not surface as a protocol error");
    assert_eq!(result.is_error, Some(true));
    let text = format!("{:?}", result.content);
    assert!(
      text.contains("doubled"),
      "the message must tell the caller how to fix it, got: {text}"
    );
  }
}
