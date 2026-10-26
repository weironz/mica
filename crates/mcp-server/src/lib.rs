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
  handler::server::router::tool::ToolRouter,
  model::{Implementation, ServerCapabilities, ServerInfo},
  tool_handler,
  transport::stdio,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

/// Outbound-call bounds (#45): a bare `reqwest::Client::new()` waits forever,
/// so one wedged connection would have parked the MCP tool call (and the agent
/// waiting on it) with no error at all. Connect and total bounds cover every
/// tool uniformly; the total is generous because export/markdown reads a whole
/// document. `MICA_HTTP_TIMEOUT_SECS` overrides the total for slow links.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

fn request_timeout() -> Duration {
  std::env::var("MICA_HTTP_TIMEOUT_SECS")
    .ok()
    .and_then(|v| v.trim().parse::<u64>().ok())
    .filter(|secs| *secs > 0)
    .map(Duration::from_secs)
    .unwrap_or(DEFAULT_REQUEST_TIMEOUT)
}

#[derive(Clone)]
struct MicaMcp {
  http: reqwest::Client,
  base: String,
  pat: String,
  read_only: bool,
  /// Warm read cache: document_id → its full Markdown (Phase 2). An agent working
  /// a workspace re-reads the same docs (scan → revisit a section); serving those
  /// from memory is the "like a local dir" win. SAFE because the agent is the
  /// sole editor during its work — every write through `write_markdown`
  /// invalidates the doc, so the cache is always coherent with the agent's own
  /// edits. It does NOT poll for edits made ELSEWHERE (none happen concurrently,
  /// per design); `refresh: true` on a read forces a refetch if ever needed.
  /// Shared across clones via Arc so the single stdio process has one cache.
  ///
  /// "Every write" means every tool that changes what a document reads as, not
  /// just the Markdown writes: restoring a version rewrites the page, and
  /// trash/restore/purge/transfer change whether it exists at all. Those act on
  /// view ids, which don't map to a document id without a lookup, so they
  /// drop the whole cache ([cache_clear]) — they are rare, a refetch is cheap.
  cache: Arc<Mutex<HashMap<Uuid, String>>>,
  tool_router: ToolRouter<Self>,
}

impl MicaMcp {
  fn new(base: String, pat: String, read_only: bool) -> Self {
    Self {
      http: reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(request_timeout())
        .build()
        .expect("building the MCP HTTP client (TLS backend init)"),
      base: base.trim_end_matches('/').to_string(),
      pat,
      read_only,
      cache: Arc::new(Mutex::new(HashMap::new())),
      tool_router: Self::full_tool_router(),
    }
  }

  /// The complete tool router: each `tools_*.rs` module builds its own via
  /// `#[tool_router]` (the macro only collects `#[tool]` methods from the impl
  /// block it annotates), so the pieces are merged here. Forgetting a merge is
  /// caught by the CI MCP-conformance job, not by compilation.
  fn full_tool_router() -> ToolRouter<Self> {
    let mut router = Self::read_router();
    router.merge(Self::write_router());
    router.merge(Self::organize_router());
    router.merge(Self::trash_router());
    router.merge(Self::admin_router());
    router
  }

  /// Cached full Markdown for a doc, if warm. The guard is dropped before any
  /// await (we clone out) so it never crosses an async point.
  fn cache_get(&self, document_id: Uuid) -> Option<String> {
    self.cache.lock().ok()?.get(&document_id).cloned()
  }
  fn cache_put(&self, document_id: Uuid, markdown: &str) {
    if let Ok(mut c) = self.cache.lock() {
      c.insert(document_id, markdown.to_string());
    }
  }
  /// Drop a doc's cached copy — called after any write so the next read is fresh.
  fn cache_invalidate(&self, document_id: Uuid) {
    if let Ok(mut c) = self.cache.lock() {
      c.remove(&document_id);
    }
  }
  /// Drop every cached doc — after a tool that changes documents it names only
  /// by VIEW id (trash, restore, purge, transfer, workspace delete).
  fn cache_clear(&self) {
    if let Ok(mut c) = self.cache.lock() {
      c.clear();
    }
  }

  /// A doc's full Markdown — from the warm cache when present (unless `refresh`),
  /// else fetched from the export endpoint and cached. The one seam both read
  /// tools share, so caching is uniform.
  async fn full_markdown(
    &self,
    workspace_id: Uuid,
    document_id: Uuid,
    refresh: bool,
  ) -> Result<String, McpError> {
    if !refresh && let Some(cached) = self.cache_get(document_id) {
      return Ok(cached);
    }
    let value = self
      .get(&format!(
        "/api/workspaces/{workspace_id}/documents/{document_id}/export/markdown"
      ))
      .await?;
    let full = unwrap_markdown(value);
    self.cache_put(document_id, &full);
    Ok(full)
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
    // A 200 does NOT mean the body arrived: a proxy can cut the response short,
    // and `unwrap_or_default()` used to turn that into an empty page that the
    // read tools then cached. Every caller gets the same error semantics.
    let text = resp.text().await.map_err(|e| {
      McpError::internal_error(format!("Mica response body could not be read: {e}"), None)
    })?;
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
  /// GET with query pairs, encoded by reqwest itself — the pair list is
  /// percent-encoded properly (no hand-rolled `urlencode`), and an empty list
  /// is a plain GET with no bare `?` appended.
  async fn get_with_query(&self, path: &str, query: &[(&str, String)]) -> Result<Value, McpError> {
    let mut request = self.http.get(self.url(path));
    if !query.is_empty() {
      request = request.query(query);
    }
    self.send(request).await
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

  /// Fetch raw bytes (not JSON) plus the served content type, refusing anything
  /// over `max_bytes`. Used only for blobs, which are the one thing here that
  /// is not text. The cap is enforced BEFORE the body lands — Content-Length
  /// first, then the stream as it arrives — so an oversized blob is refused
  /// early instead of buffered whole and rejected after the fact.
  async fn get_bytes(
    &self,
    url: &str,
    max_bytes: usize,
  ) -> Result<(Vec<u8>, Option<String>), McpError> {
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
    if let Some(len) = resp.content_length()
      && len as usize > max_bytes
    {
      return Err(McpError::internal_error(
        format!("Mica blob is {len} bytes — over the {max_bytes}-byte limit"),
        None,
      ));
    }
    let mime = resp
      .headers()
      .get(reqwest::header::CONTENT_TYPE)
      .and_then(|v| v.to_str().ok())
      .map(|v| v.split(';').next().unwrap_or(v).trim().to_string());
    let mut bytes = Vec::new();
    let mut resp = resp;
    while let Some(chunk) = resp
      .chunk()
      .await
      .map_err(|e| McpError::internal_error(format!("Mica blob read failed: {e}"), None))?
    {
      bytes.extend_from_slice(&chunk);
      if bytes.len() > max_bytes {
        return Err(McpError::internal_error(
          format!(
            "Mica blob is over {max_bytes} bytes (stopped at {})",
            bytes.len()
          ),
          None,
        ));
      }
    }
    Ok((bytes, mime))
  }
}

/// Refuse the enclosing tool when this server runs read-only
/// ([MicaMcp::ensure_writable]). A macro because the plain form is three lines
/// of early-return boilerplate in front of every one of the ~24 mutating
/// tools: the guard must stay the FIRST thing a write does — ahead of
/// validation, ahead of any HTTP call — so the refusal is unconditional and
/// uniform. `$crate::` paths keep it independent of the caller's imports.
/// Declared before the `mod`s below: textual scoping, like the std `#[macro_use]`.
macro_rules! writable {
  ($self:ident) => {
    if let Err(error) = $self.ensure_writable() {
      return Ok($crate::results::tool_error(error));
    }
  };
}

mod args;
mod markdown_windows;
mod results;
mod tools_admin;
mod tools_organize;
mod tools_read;
mod tools_trash;
mod tools_write;

use markdown_windows::*;
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
      // The ONE place an agent reliably reads "how to use this efficiently" — it
      // is shown at connect, above the per-tool schemas. KEEP IT IN SYNC: when a
      // tool or a high-value param is added/changed, teach the workflow here, or
      // the capability ships discoverable-but-unused (optional params get skipped
      // unless this guide makes the benefit salient). This guide is what makes
      // range reads / in-place edits / optimistic concurrency actually get used.
      instructions: Some(
        "Mica note-workspace MCP. Documents are Markdown — the server derives block ops, so \
         you write Markdown, not block JSON.\n\
         \n\
         Navigate: mica_list_workspaces -> mica_search -> mica_read_document / \
         mica_get_outline. Reach for mica_list_pages only when you genuinely need the SHAPE of \
         the whole workspace; on a large one it is the most expensive call here.\n\
         \n\
         LOCATING things, including where to PUT a new page: mica_search matches pages AND \
         folders by name (plus page body text), and every hit carries `parent_view_id` and \
         `is_folder`. To file a page beside an existing one, search for that one and reuse its \
         `parent_view_id` — one hit instead of a whole tree. Only fall back to mica_list_pages \
         when a name is not enough to find the spot.\n\
         \n\
         A user may hand you a PATH like `workspace/folder/page` (the app's \"copy path\", whose \
         first segment is the WORKSPACE). There is no resolve-a-path tool and you do not need \
         one: search the LAST segment, then confirm the hit by walking `parent_view_id` upward \
         and checking those names against the earlier segments. Do that check rather than \
         trusting a lone hit — page names repeat, and a name may itself contain a slash.\n\
         \n\
         READ efficiently, do not pull whole pages: mica_list_pages returns the WHOLE \
         workspace tree (folders + pages, ids + names) in ONE call. mica_read_document takes \
         offset+limit (a line window) or section (a heading name) to return just a slice. \
         mica_get_outline returns headings + block ids + type/preview labels + the document `seq`, cheaply. To scan \
         MANY docs, mica_read_documents reads them all in one call — mode=\"outline\" for a \
         cheap structure survey, then read in full only the few you need.\n\
         \n\
         EDIT in place, do not rewrite the page. mica_update_document modes: append (after the \
         end — safe default); insert_at (place after `anchor`, which is a block id OR a \
         heading's text from the outline); find_replace (match visible text or a single block's \
         Markdown, including inline code, lists and table cells; replace parses inline Markdown, \
         or set replacement_format=\"plain_text\" for literal text. It cannot cross blocks/cells. \
         Set unique=true for a single precise edit, checked by the server); delete (REMOVE the block at \
         `anchor` and its subtree); replace_all (only when you \
         truly mean to rewrite the whole page). For MANY edits to one doc, mica_apply_edits \
         runs them all in ONE call (one turn instead of N).\n\
         \n\
         To REMOVE content, reach for delete, never replace_all. Re-sending a whole page to \
         drop one block rewrites every OTHER block as a side effect, and a page you did not \
         author byte-for-byte will not survive the trip: non-breaking spaces become spaces, \
         control characters inside code samples vanish, trailing whitespace is trimmed. Those \
         losses are invisible in the diff you can see and permanent in the document.\n\
         \n\
         REORGANISE in bulk. Whenever you are about to call a per-page tool in a loop, stop and \
         use the batch form — it is one request, not N, and it is the difference between a \
         cleanup that finishes and one that runs out of context: mica_trash_views (many pages to \
         the bin), mica_restore_views (the undo), mica_move_views (re-parent many under one \
         folder, in order). Each takes `view_ids`. A folder id carries its whole subtree, so \
         pass the folder rather than enumerating its children. They report what they ACTUALLY \
         touched — `affected` counts subtrees too, and `skipped` lists requested ids that were \
         already gone; do not read the count you sent as the count that happened.\n\
         \n\
         The recycle bin: mica_list_trash to see it, mica_restore_views to undo in bulk, and — \
         PERMANENT, not recoverable — mica_purge_view (one subtree), mica_purge_views (many, the \
         batch form) or mica_empty_trash (all of it). Trashing is safe and reversible; purging is \
         neither. The purge tools act ONLY on pages already in the bin, so the sequence for a \
         real cleanup is always trash first, then purge; ids that were never trashed come back in \
         `skipped` rather than being destroyed.\n\
         \n\
         DELETING A WHOLE WORKSPACE is a different act, not a bigger cleanup: mica_delete_workspace \
         destroys every page, version, comment, share link and file in it, there is no workspace \
         recycle bin, and nothing survives it. It needs confirm=true AND expect_name matching the \
         workspace's exact current name — a guard that exists because the wrong id here cannot be \
         undone. If what you actually want is to empty a workspace, that is mica_trash_views.\n\
         \n\
         Write acks return `seq` and `last_block_id` (the new end anchor) — chain further edits \
         from those instead of re-reading. For safe concurrent editing, pass the outline/ack \
         `seq` back as `expected_seq`; a stale write is refused (409) so you re-read first."
          .to_string(),
      ),
      ..Default::default()
    }
  }
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
mod contract_tests;

#[cfg(test)]
mod tools_tests;

#[cfg(test)]
mod io_failure_tests;

#[cfg(test)]
mod tests {
  use super::*;
  use crate::markdown_windows::{
    pick_anchor, reject_mangled_latex, section_markdown, unwrap_markdown, window_markdown,
  };

  #[test]
  fn warm_cache_put_get_invalidate() {
    let m = MicaMcp::new("http://x".into(), "t".into(), false);
    let d1 = Uuid::new_v4();
    assert!(m.cache_get(d1).is_none(), "cold cache misses");
    m.cache_put(d1, "# hi");
    assert_eq!(m.cache_get(d1).as_deref(), Some("# hi"), "warm hit");
    // A write invalidates so the next read is fresh.
    m.cache_invalidate(d1);
    assert!(m.cache_get(d1).is_none(), "invalidated → miss");
  }

  #[test]
  fn window_no_bounds_returns_full_unchanged() {
    let doc = "line1\nline2\nline3";
    assert_eq!(window_markdown(doc, None, None, None), doc);
  }

  #[test]
  fn window_offset_limit_slices_and_headers() {
    let doc = "a\nb\nc\nd\ne";
    // lines 2..=3 (1-based offset 2, limit 2)
    assert_eq!(
      window_markdown(doc, Some(2), Some(2), None),
      "[lines 2-3 of 5]\nb\nc"
    );
    // offset past the middle, no limit → to the end
    assert_eq!(
      window_markdown(doc, Some(4), None, None),
      "[lines 4-5 of 5]\nd\ne"
    );
    // limit past the end clamps
    assert_eq!(
      window_markdown(doc, Some(4), Some(99), None),
      "[lines 4-5 of 5]\nd\ne"
    );
  }

  /// Anchor text is matched as a SUBSTRING so a caller can say "Setup" instead
  /// of the exact heading. For `insert_at` that generosity costs a position;
  /// for `delete` it would cost a section, so `delete` refuses the ambiguity.
  #[test]
  fn delete_refuses_an_anchor_that_matches_more_than_one_heading() {
    let headings = serde_json::json!([
      { "text": "Setup", "block_id": "block_a" },
      { "text": "Setup on Windows", "block_id": "block_b" },
    ]);

    // insert_at takes the first — inserting in the wrong place is visible and
    // undoable, and this is the behaviour callers already rely on.
    assert_eq!(
      pick_anchor(Some(&headings), "setup", false).unwrap(),
      "block_a"
    );

    let err = pick_anchor(Some(&headings), "setup", true).unwrap_err();
    assert!(err.contains("matches 2 headings"), "says how many: {err}");
    assert!(
      err.contains("block_a") && err.contains("block_b"),
      "lists them: {err}"
    );

    // Unambiguous text still resolves under strict.
    assert_eq!(
      pick_anchor(Some(&headings), "on windows", true).unwrap(),
      "block_b"
    );
  }

  #[test]
  fn an_anchor_that_matches_nothing_is_an_error_naming_the_mode() {
    let headings = serde_json::json!([{ "text": "Intro", "block_id": "block_a" }]);
    let err = pick_anchor(Some(&headings), "Setup", true).unwrap_err();
    assert!(err.starts_with("delete:"), "the error names delete: {err}");
    let err = pick_anchor(Some(&headings), "Setup", false).unwrap_err();
    assert!(
      err.starts_with("insert_at:"),
      "the error names insert_at: {err}"
    );
    // A doc with no headings at all must not panic or resolve to something.
    assert!(pick_anchor(None, "Setup", true).is_err());
  }

  /// `/export/markdown` answers with an OBJECT. Taking `.to_string()` of it
  /// yielded the serialized JSON, whose newlines are the literal characters
  /// `\` + `n` — so `full` was one line and every windowed read silently
  /// degraded: `section` matched nothing, `offset`/`limit` reported
  /// "[lines 1-1 of 1]", and a plain read returned `{"markdown":"..."}`.
  #[test]
  fn the_markdown_is_unwrapped_out_of_the_export_object() {
    let body = serde_json::json!({ "markdown": "# Title

body" });
    let full = unwrap_markdown(body);
    assert_eq!(
      full,
      "# Title

body"
    );
    assert_eq!(full.lines().count(), 3, "must be real newlines, not \n");
  }

  /// The windows are what the fix exists for — assert them, not just the
  /// unwrap, so a regression in either layer is caught here.
  #[test]
  fn a_window_over_the_export_object_is_a_real_slice() {
    let body = serde_json::json!({ "markdown": "# Title

## Setup

step

## Next

more" });
    let full = unwrap_markdown(body);
    assert_eq!(
      section_markdown(&full, "Setup"),
      "## Setup

step
"
    );
    assert!(
      window_markdown(&full, Some(1), Some(2), None).starts_with("[lines 1-2 of 9]"),
      "got {:?}",
      window_markdown(&full, Some(1), Some(2), None)
    );
  }

  /// A bare JSON string still works (older servers), and an object without the
  /// field keeps the old visible-but-honest behaviour rather than returning "".
  #[test]
  fn unwrap_falls_back_instead_of_inventing_an_empty_document() {
    assert_eq!(unwrap_markdown(serde_json::json!("plain")), "plain");
    let odd = serde_json::json!({ "unexpected": 1 });
    assert_eq!(unwrap_markdown(odd), "{\"unexpected\":1}");
  }

  #[test]
  fn section_returns_heading_through_next_same_or_higher() {
    let doc = "# Title\n\nintro\n\n## Setup\n\nstep one\n\n### Sub\n\ndeep\n\n## Usage\n\nrun it";
    // ## Setup through just before ## Usage (## Sub is deeper, stays in)
    assert_eq!(
      section_markdown(doc, "setup"),
      "## Setup\n\nstep one\n\n### Sub\n\ndeep\n"
    );
    // A higher-level heading (# Title) captures everything under it until EOF
    // here (no other level-1), so it spans the whole doc.
    assert!(section_markdown(doc, "Title").starts_with("# Title"));
    assert!(section_markdown(doc, "Title").contains("## Usage"));
  }

  #[test]
  fn section_miss_is_a_readable_note_not_a_panic() {
    let out = section_markdown("# Only\n\nbody", "nope");
    assert!(out.contains("no heading contains"));
  }

  /// The collision rules themselves (and their fixtures — the real shipped
  /// corruption, every collision char, the legitimate content that must pass)
  /// moved to `mica_markdown::latex_guard` with the guard (issue #83). What
  /// stays here: the wrapper turns a collision into an error, and stays silent
  /// on clean input.
  #[test]
  fn the_wrapper_maps_a_collision_to_an_error() {
    assert_eq!(
      reject_mangled_latex("a \u{000C}rac b")
        .err()
        .map(|e| e.message.to_string()),
      Some(mica_markdown::mangled_latex_message(r"\f", r"\frac"))
    );
    assert!(reject_mangled_latex(r"$\eta = 2 \times \frac{N-1}{N}$").is_ok());
  }

  /// #117: a `#` inside a fenced code block is CONTENT, not a heading. The
  /// section reader used to treat it as the next same-level heading, cutting
  /// Guide off mid-fence and inventing a "Phantom" section.
  #[test]
  fn a_fenced_heading_is_content_not_a_section_boundary() {
    let doc =
      "# Guide\n\nBefore.\n\n```markdown\n# Phantom\nexample\n```\n\nAfter.\n\n# Next\n\nTail.";
    assert_eq!(
      section_markdown(doc, "Guide"),
      "# Guide\n\nBefore.\n\n```markdown\n# Phantom\nexample\n```\n\nAfter.\n"
    );
    assert!(
      section_markdown(doc, "Phantom").contains("no heading contains"),
      "a heading inside a fence is not a section"
    );
    assert_eq!(section_markdown(doc, "Next"), "# Next\n\nTail.");
  }

  /// The scanner has to be the Markdown one, not "a line that starts with #":
  /// tilde fences, longer backtick fences (a ``` inside ```` is content), and
  /// fences inside a blockquote all hide headings.
  #[test]
  fn every_fence_flavour_hides_its_headings() {
    let doc = "# Top\n\n~~~\n# Tilde\n~~~\n\n````\n```\n# Long\n```\n````\n\n> ```\n> # Quoted\n> ```\n\n## Real\n\nbody";
    let top = section_markdown(doc, "Top");
    assert!(
      top.contains("# Tilde"),
      "tilde fenced content stays: {top:?}"
    );
    assert!(
      top.contains("# Long"),
      "longer fence keeps ``` inside: {top:?}"
    );
    assert!(top.contains("# Quoted"), "blockquoted fence stays: {top:?}");
    for phantom in ["Tilde", "Long", "Quoted"] {
      assert!(
        section_markdown(doc, phantom).contains("no heading contains"),
        "{phantom:?} is code, not a heading"
      );
    }
    assert_eq!(section_markdown(doc, "Real"), "## Real\n\nbody");
  }
}

#[cfg(test)]
mod handshake_tests {
  use crate::args::CreateDocArgs;
  use crate::results::*;
  use rmcp::handler::server::wrapper::Parameters;
  use serde_json::json;
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
    let router = MicaMcp::full_tool_router();
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
    let router = MicaMcp::full_tool_router();
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
    let names: Vec<String> = MicaMcp::full_tool_router()
      .list_all()
      .iter()
      .map(|t| t.name.to_string())
      .collect();
    for expected in [
      "mica_rename",
      "mica_create_folder",
      "mica_clone_document",
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
    let router = MicaMcp::full_tool_router();
    let tools = router.list_all();
    assert_eq!(
      tools.len(),
      34,
      "every tool must be listed — if you added one, bump this AND add it to the \
       get_info instructions, or it ships discoverable-but-unused"
    );
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
    for t in MicaMcp::full_tool_router().list_all() {
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
        workspace_id: Uuid::nil(),
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
        workspace_id: Uuid::nil(),
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

/// Wire-level tests: a real MCP `tools/call` (JSON-RPC over an in-memory pipe)
/// against a fake Mica API on a local TcpListener that records every request
/// line. This is the layer P1-1 lived in — no test above ever looked at the URL
/// a tool actually sends, so a path built from a model-supplied string went
/// unchallenged.
#[cfg(test)]
mod wire_tests {
  use super::*;
  use serde_json::json;
  use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

  const WS: &str = "11111111-1111-4111-8111-111111111111";
  const DOC: &str = "22222222-2222-4222-8222-222222222222";
  const VIEW: &str = "33333333-3333-4333-8333-333333333333";
  const VICTIM: &str = "99999999-9999-4999-8999-999999999999";

  type Log = Arc<Mutex<Vec<String>>>;

  /// A fake API: answers every request `200 {}` and records "METHOD /path?query".
  async fn fake_api() -> (String, Log) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Log = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
      loop {
        let Ok((sock, _)) = listener.accept().await else {
          return;
        };
        let log = log.clone();
        tokio::spawn(async move {
          let mut reader = BufReader::new(sock);
          let mut request_line = String::new();
          if reader.read_line(&mut request_line).await.unwrap_or(0) == 0 {
            return;
          }
          let mut content_length = 0usize;
          loop {
            let mut header = String::new();
            if reader.read_line(&mut header).await.unwrap_or(0) == 0 || header == "\r\n" {
              break;
            }
            if let Some(v) = header.to_ascii_lowercase().strip_prefix("content-length:") {
              content_length = v.trim().parse().unwrap_or(0);
            }
          }
          let mut body = vec![0u8; content_length];
          let _ = reader.read_exact(&mut body).await;
          let mut parts = request_line.split_whitespace();
          let method = parts.next().unwrap_or("").to_string();
          let target = parts.next().unwrap_or("").to_string();
          log.lock().unwrap().push(format!("{method} {target}"));
          let _ = reader
            .get_mut()
            .write_all(
              b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                Content-Length: 2\r\nConnection: close\r\n\r\n{}",
            )
            .await;
        });
      }
    });
    (base, seen)
  }

  /// One MCP session over an in-memory pipe, driven with raw JSON-RPC lines —
  /// exactly what a client sends, so argument deserialization is the real one.
  struct Session {
    to_server: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    from_server: BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    next_id: u64,
  }

  impl Session {
    async fn start(server: MicaMcp) -> Self {
      let (client_io, server_io) = tokio::io::duplex(1 << 20);
      tokio::spawn(async move {
        let (r, w) = tokio::io::split(server_io);
        if let Ok(running) = server.serve((r, w)).await {
          let _ = running.waiting().await;
        }
      });
      let (r, w) = tokio::io::split(client_io);
      let mut s = Session {
        to_server: w,
        from_server: BufReader::new(r),
        next_id: 1,
      };
      s.request(
        "initialize",
        json!({
          "protocolVersion": "2025-03-26",
          "capabilities": {},
          "clientInfo": { "name": "wire-test", "version": "0" }
        }),
      )
      .await;
      s.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .await;
      s
    }

    async fn send(&mut self, msg: Value) {
      let mut line = msg.to_string();
      line.push('\n');
      self.to_server.write_all(line.as_bytes()).await.unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
      let id = self.next_id;
      self.next_id += 1;
      self
        .send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        .await;
      loop {
        let mut line = String::new();
        let n = self.from_server.read_line(&mut line).await.unwrap();
        assert!(n > 0, "server closed the pipe");
        let msg: Value = serde_json::from_str(&line).unwrap();
        if msg.get("id") == Some(&json!(id)) {
          return msg;
        }
      }
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Value {
      self
        .request(
          "tools/call",
          json!({ "name": tool, "arguments": arguments }),
        )
        .await
    }
  }

  /// A refusal is either a JSON-RPC error (bad params never reached the tool)
  /// or an `isError` tool result. Anything else means the call went through.
  fn refused(reply: &Value) -> bool {
    reply.get("error").is_some() || reply.pointer("/result/isError") == Some(&json!(true))
  }

  /// P1-1. Each of these used to be spliced straight into the request URL;
  /// reqwest resolves `..` and drops everything after `#`, so the first one
  /// turned "unshare a document" into `DELETE /api/workspaces/<victim>` —
  /// around every confirm/expect_name guard. Every id the API routes on is a
  /// UUID, so anything else must be refused before a single byte is sent.
  #[tokio::test]
  async fn a_path_shaped_id_is_refused_without_any_request() {
    let (base, seen) = fake_api().await;
    let mut mcp = Session::start(MicaMcp::new(base, "pat".into(), false)).await;

    let attacks = [
      (
        "mica_unshare",
        json!({ "workspace_id": WS, "document_id": format!("../../../workspaces/{VICTIM}#") }),
      ),
      (
        "mica_unshare",
        json!({ "workspace_id": format!("{VICTIM}#"), "document_id": DOC }),
      ),
      (
        "mica_get_outline",
        json!({ "workspace_id": WS, "document_id": format!("{DOC}/../../{VICTIM}") }),
      ),
      (
        "mica_trash_view",
        json!({
          "workspace_id": WS,
          "view_id": format!("../../../workspaces/{VICTIM}#"),
          "confirm": true
        }),
      ),
      (
        "mica_list_pages",
        json!({ "workspace_id": WS, "parent_view_id": format!("{VIEW}&depth=1#") }),
      ),
      (
        "mica_read_documents",
        json!({ "workspace_id": WS, "document_ids": [format!("../{VICTIM}")], "mode": "outline" }),
      ),
      (
        "mica_restore_version",
        json!({ "workspace_id": WS, "document_id": DOC, "version_id": "../x" }),
      ),
      (
        "mica_read_image",
        json!({ "workspace_id": WS, "file_id": format!("x/../../{VICTIM}") }),
      ),
    ];
    for (tool, args) in attacks {
      let reply = mcp.call(tool, args.clone()).await;
      let sent = seen.lock().unwrap().clone();
      assert!(
        sent.is_empty(),
        "{tool} {args} must send nothing, sent {sent:?}"
      );
      assert!(
        refused(&reply),
        "{tool} {args} must be refused, got {reply}"
      );
    }

    // Not vacuous: the same harness DOES observe a legitimate call.
    let reply = mcp
      .call(
        "mica_unshare",
        json!({ "workspace_id": WS, "document_id": DOC }),
      )
      .await;
    assert!(!refused(&reply), "a real uuid must go through: {reply}");
    assert_eq!(seen.lock().unwrap().len(), 1, "the harness sees requests");
  }

  /// A missing confirmation is actionable tool feedback, with no API side effects.
  #[tokio::test]
  async fn confirmation_refusals_are_tool_errors_without_any_request() {
    let (base, seen) = fake_api().await;
    let server = MicaMcp::new(base, "pat".into(), false);
    let probe = server.clone();
    let doc: Uuid = DOC.parse().unwrap();
    probe.cache_put(doc, "cached page");
    let mut mcp = Session::start(server).await;

    let guarded = [
      (
        "mica_delete_workspace",
        json!({ "workspace_id": WS, "expect_name": "My workspace" }),
        "refusing to delete a workspace without confirm=true — this cannot be undone",
        true,
      ),
      (
        "mica_trash_view",
        json!({ "workspace_id": WS, "view_id": VIEW }),
        "refusing to trash without confirm=true",
        false,
      ),
      (
        "mica_trash_views",
        json!({ "workspace_id": WS, "view_ids": [VIEW, DOC] }),
        "refusing to trash 2 view(s) without confirm=true",
        true,
      ),
      (
        "mica_empty_trash",
        json!({ "workspace_id": WS }),
        "refusing to empty the recycle bin without confirm=true — this cannot be undone",
        true,
      ),
      (
        "mica_purge_view",
        json!({ "workspace_id": WS, "view_id": VIEW }),
        "refusing to purge without confirm=true — this cannot be undone",
        false,
      ),
      (
        "mica_purge_views",
        json!({ "workspace_id": WS, "view_ids": [VIEW, DOC] }),
        "refusing to permanently delete 2 view(s) without confirm=true — this cannot be undone",
        true,
      ),
    ];
    for (tool, mut args, reason, defaults_to_false) in guarded {
      // Only the tools with a serde default accept an omitted confirmation.
      for omitted in [false, true] {
        if omitted && !defaults_to_false {
          continue;
        }
        if omitted {
          args.as_object_mut().unwrap().remove("confirm");
        } else {
          args["confirm"] = json!(false);
        }
        let reply = mcp.call(tool, args.clone()).await;
        assert!(reply.get("error").is_none(), "{tool}: {reply}");
        assert_eq!(
          reply.pointer("/result/isError"),
          Some(&json!(true)),
          "{tool}: {reply}"
        );
        assert_eq!(
          reply
            .pointer("/result/content/0/text")
            .and_then(Value::as_str),
          Some(reason),
          "{tool}: {reply}"
        );
        assert!(
          seen.lock().unwrap().is_empty(),
          "{tool} must send no API requests"
        );
        assert_eq!(
          probe.cache_get(doc).as_deref(),
          Some("cached page"),
          "{tool}"
        );
      }
    }
  }

  /// The guard for the NEXT id someone adds: every `*_id` / `*_ids` argument of
  /// every tool is schema'd as a uuid string — which is only true when the
  /// field is a `Uuid`, i.e. when it cannot carry a path. A new `String` id
  /// fails here instead of shipping P1-1 again.
  #[test]
  fn every_id_argument_is_a_uuid() {
    let mut checked = 0;
    for t in MicaMcp::full_tool_router().list_all() {
      let Some(props) = t.input_schema.get("properties").and_then(Value::as_object) else {
        continue;
      };
      for (name, schema) in props {
        let schema = if name.ends_with("_ids") {
          schema.get("items").unwrap_or(&Value::Null)
        } else if name.ends_with("_id") {
          schema
        } else {
          continue;
        };
        assert_eq!(
          schema.get("format").and_then(Value::as_str),
          Some("uuid"),
          "{}.{name} must be a Uuid, not a free string: {schema}",
          t.name
        );
        assert_eq!(schema.get("type").and_then(Value::as_str), Some("string"));
        checked += 1;
      }
    }
    assert!(
      checked > 50,
      "walked the real schemas, not an empty list: {checked}"
    );
  }

  /// C11. The warm cache is only safe if every tool that changes what a page
  /// reads as drops it. Restoring a version rewrites the page; trash, restore,
  /// purge and transfer change whether it exists. Any of them leaving the old
  /// text cached means the next anchored edit is planned against a page that
  /// is no longer there.
  #[tokio::test]
  async fn tools_that_change_documents_drop_the_read_cache() {
    let (base, seen) = fake_api().await;
    let server = MicaMcp::new(base, "pat".into(), false);
    let probe = server.clone(); // shares the Arc'd cache
    let mut mcp = Session::start(server).await;
    let doc: Uuid = DOC.parse().unwrap();
    let ver = "44444444-4444-4444-8444-444444444444";
    let dest = "55555555-5555-4555-8555-555555555555";

    // Control: a read-only tool leaves it warm, so a miss below is the tool's
    // doing and not a probe that never shared the cache.
    probe.cache_put(doc, "stale");
    let reply = mcp
      .call("mica_list_trash", json!({ "workspace_id": WS }))
      .await;
    assert!(!refused(&reply), "{reply}");
    assert_eq!(probe.cache_get(doc).as_deref(), Some("stale"));

    let changers = [
      (
        "mica_restore_version",
        json!({ "workspace_id": WS, "document_id": DOC, "version_id": ver }),
      ),
      (
        "mica_trash_view",
        json!({ "workspace_id": WS, "view_id": VIEW, "confirm": true }),
      ),
      (
        "mica_trash_views",
        json!({ "workspace_id": WS, "view_ids": [VIEW], "confirm": true }),
      ),
      (
        "mica_restore_view",
        json!({ "workspace_id": WS, "view_id": VIEW }),
      ),
      (
        "mica_restore_views",
        json!({ "workspace_id": WS, "view_ids": [VIEW] }),
      ),
      (
        "mica_purge_view",
        json!({ "workspace_id": WS, "view_id": VIEW, "confirm": true }),
      ),
      (
        "mica_purge_views",
        json!({ "workspace_id": WS, "view_ids": [VIEW], "confirm": true }),
      ),
      (
        "mica_empty_trash",
        json!({ "workspace_id": WS, "confirm": true }),
      ),
      (
        "mica_transfer_view",
        json!({ "workspace_id": WS, "view_id": VIEW, "dest_workspace_id": dest }),
      ),
    ];
    for (tool, args) in changers {
      probe.cache_put(doc, "stale");
      let before = seen.lock().unwrap().len();
      let reply = mcp.call(tool, args).await;
      assert!(!refused(&reply), "{tool}: {reply}");
      assert_eq!(
        seen.lock().unwrap().len(),
        before + 1,
        "{tool} reached the API"
      );
      assert!(
        probe.cache_get(doc).is_none(),
        "{tool} left a stale cached page"
      );
    }
  }

  /// The positive half: a well-formed UUID builds exactly the documented path.
  #[tokio::test]
  async fn a_uuid_builds_the_exact_path() {
    let (base, seen) = fake_api().await;
    let mut mcp = Session::start(MicaMcp::new(base, "pat".into(), false)).await;

    let reply = mcp
      .call(
        "mica_unshare",
        json!({ "workspace_id": WS, "document_id": DOC }),
      )
      .await;
    assert!(!refused(&reply), "{reply}");
    let reply = mcp
      .call(
        "mica_list_pages",
        json!({ "workspace_id": WS, "parent_view_id": VIEW, "depth": 1 }),
      )
      .await;
    assert!(!refused(&reply), "{reply}");
    assert_eq!(
      *seen.lock().unwrap(),
      vec![
        format!("DELETE /api/workspaces/{WS}/documents/{DOC}/share"),
        format!("GET /api/workspaces/{WS}/views?parent_view_id={VIEW}&depth=1"),
      ]
    );
  }
}
