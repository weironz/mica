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
    handler::server::{
        router::tool::ToolRouter,
        wrapper::Parameters,
    },
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
}

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
fn ok_json(value: Value) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

#[tool_router]
impl MicaMcp {
    #[tool(
        description = "List all Mica workspaces (id, name, role) the token can access.",
        annotations(read_only_hint = true)
    )]
    async fn mica_list_workspaces(&self) -> Result<CallToolResult, McpError> {
        ok_json(self.get("/api/workspaces").await?)
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
        ok_json(
            self.get(&format!("/api/workspaces/{workspace_id}/views"))
                .await?,
        )
    }

    #[tool(
        description = "Search a workspace's pages by title.",
        annotations(read_only_hint = true)
    )]
    async fn mica_search(
        &self,
        Parameters(SearchArgs { workspace_id, query }): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let q = urlencode(&query);
        ok_json(
            self.get(&format!("/api/workspaces/{workspace_id}/search?q={q}"))
                .await?,
        )
    }

    #[tool(
        description = "Read a document's content as Markdown.",
        annotations(read_only_hint = true)
    )]
    async fn mica_read_document(
        &self,
        Parameters(DocArg { workspace_id, document_id }): Parameters<DocArg>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(
            self.get(&format!(
                "/api/workspaces/{workspace_id}/documents/{document_id}/export/markdown"
            ))
            .await?,
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
        Parameters(DocArg { workspace_id, document_id }): Parameters<DocArg>,
    ) -> Result<CallToolResult, McpError> {
        ok_json(
            self.get(&format!(
                "/api/workspaces/{workspace_id}/documents/{document_id}/outline"
            ))
            .await?,
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
        self.ensure_writable()?;
        // Markdown → the import endpoint (parses content server-side); empty →
        // the plain create endpoint.
        if let Some(markdown) = markdown {
            reject_mangled_latex(&markdown)?;
            let body = json!({
                "name": name,
                "markdown": markdown,
                "parent_view_id": parent_view_id,
            });
            ok_json(
                self.post(
                    &format!("/api/workspaces/{workspace_id}/documents/import/markdown"),
                    body,
                )
                .await?,
            )
        } else {
            let body = json!({ "name": name, "parent_view_id": parent_view_id });
            ok_json(
                self.post(&format!("/api/workspaces/{workspace_id}/documents"), body)
                    .await?,
            )
        }
    }

    #[tool(
        description = "Write into an EXISTING document. mode: append (after current content, \
                       the safe default), replace_all (rewrite), insert_at (place after \
                       `anchor` from mica_get_outline — a local edit), find_replace (swap \
                       `find`→`replace`). Content is Markdown; the server derives the ops.",
        annotations(title = "Write document", read_only_hint = false, destructive_hint = true)
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
        self.ensure_writable()?;
        // Every field that carries authored content, not just `markdown`:
        // find_replace writes through `replace`, and a swapped-in formula is
        // mangled by the same under-escaping.
        for text in [markdown.as_deref(), replace.as_deref()].into_iter().flatten() {
            reject_mangled_latex(text)?;
        }
        let body = json!({
            "mode": mode,
            "markdown": markdown.unwrap_or_default(),
            "anchor": anchor,
            "find": find,
            "replace": replace,
        });
        ok_json(
            self.patch(
                &format!("/api/workspaces/{workspace_id}/documents/{document_id}/markdown"),
                body,
            )
            .await?,
        )
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
        self.ensure_writable()?;
        let body = json!({ "parent_view_id": parent_view_id });
        ok_json(
            self.post(
                &format!("/api/workspaces/{workspace_id}/views/{view_id}/move"),
                body,
            )
            .await?,
        )
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
        self.ensure_writable()?;
        if !confirm {
            return Err(McpError::invalid_params(
                "refusing to trash without confirm=true".to_string(),
                None,
            ));
        }
        ok_json(
            self.delete(&format!("/api/workspaces/{workspace_id}/views/{view_id}"))
                .await?,
        )
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
        ok_json(
            self.get(&format!("/api/workspaces/{workspace_id}/export/markdown"))
                .await?,
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
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
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
        assert_eq!(tools.len(), 10, "all ten tools must be listed");
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
}
