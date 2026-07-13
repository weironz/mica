//! Mica MCP server — a thin proxy that exposes the Mica REST API as MCP tools so
//! an AI (Claude Desktop / Code / any MCP client) can list, read, create, and
//! write Mica documents. It calls the API over HTTP with a PAT; it holds no DB
//! or storage access of its own. "Backup/restore" here is just export/import —
//! real backups are the job of external tools pointed at the exports.
//!
//! Config (env): `MICA_API_BASE_URL` (e.g. https://mica.cloudcele.com) and
//! `MICA_PAT` (a Mica personal access token). Optional `MICA_MCP_READ_ONLY=1`
//! registers only the read tools.
use anyhow::Context as _;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{
        router::tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    model::{Implementation, ServerInfo},
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
    tool_router: ToolRouter<Self>,
}

impl MicaMcp {
    fn from_env() -> anyhow::Result<Self> {
        let base = std::env::var("MICA_API_BASE_URL")
            .context("MICA_API_BASE_URL is required (e.g. https://mica.cloudcele.com)")?;
        let pat = std::env::var("MICA_PAT").context("MICA_PAT is required (a Mica access token)")?;
        Ok(Self {
            http: reqwest::Client::new(),
            base: base.trim_end_matches('/').to_string(),
            pat,
            tool_router: Self::tool_router(),
        })
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
            return Err(McpError::internal_error(
                format!("Mica API {status}: {text}"),
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

// ── Tools ───────────────────────────────────────────────────────────────────

#[tool_router]
impl MicaMcp {
    #[tool(description = "List all Mica workspaces (id, name, role) the token can access.")]
    async fn mica_list_workspaces(&self) -> Result<Json<Value>, McpError> {
        Ok(Json(self.get("/api/workspaces").await?))
    }

    #[tool(
        description = "List a workspace's page tree (documents + folders, with ids, names, \
                       parents). Use a page's object_id with the read/write tools."
    )]
    async fn mica_list_pages(
        &self,
        Parameters(WorkspaceArg { workspace_id }): Parameters<WorkspaceArg>,
    ) -> Result<Json<Value>, McpError> {
        Ok(Json(
            self.get(&format!("/api/workspaces/{workspace_id}/views"))
                .await?,
        ))
    }

    #[tool(description = "Search a workspace's pages by title.")]
    async fn mica_search(
        &self,
        Parameters(SearchArgs { workspace_id, query }): Parameters<SearchArgs>,
    ) -> Result<Json<Value>, McpError> {
        let q = urlencode(&query);
        Ok(Json(
            self.get(&format!("/api/workspaces/{workspace_id}/search?q={q}"))
                .await?,
        ))
    }

    #[tool(description = "Read a document's content as Markdown.")]
    async fn mica_read_document(
        &self,
        Parameters(DocArg { workspace_id, document_id }): Parameters<DocArg>,
    ) -> Result<Json<Value>, McpError> {
        Ok(Json(
            self.get(&format!(
                "/api/workspaces/{workspace_id}/documents/{document_id}/export/markdown"
            ))
            .await?,
        ))
    }

    #[tool(
        description = "Get a document's outline (headings + block ids in order). Call this \
                       before an anchored write so you can target a spot instead of rewriting \
                       the whole page."
    )]
    async fn mica_get_outline(
        &self,
        Parameters(DocArg { workspace_id, document_id }): Parameters<DocArg>,
    ) -> Result<Json<Value>, McpError> {
        Ok(Json(
            self.get(&format!(
                "/api/workspaces/{workspace_id}/documents/{document_id}/outline"
            ))
            .await?,
        ))
    }

    #[tool(description = "Create a new page (optionally with Markdown body) in a workspace.")]
    async fn mica_create_document(
        &self,
        Parameters(CreateDocArgs {
            workspace_id,
            name,
            markdown,
            parent_view_id,
        }): Parameters<CreateDocArgs>,
    ) -> Result<Json<Value>, McpError> {
        // Markdown → the import endpoint (parses content server-side); empty →
        // the plain create endpoint.
        if let Some(markdown) = markdown {
            let body = json!({
                "name": name,
                "markdown": markdown,
                "parent_view_id": parent_view_id,
            });
            Ok(Json(
                self.post(
                    &format!("/api/workspaces/{workspace_id}/documents/import/markdown"),
                    body,
                )
                .await?,
            ))
        } else {
            let body = json!({ "name": name, "parent_view_id": parent_view_id });
            Ok(Json(
                self.post(&format!("/api/workspaces/{workspace_id}/documents"), body)
                    .await?,
            ))
        }
    }
}

#[tool_handler]
impl ServerHandler for MicaMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Logs go to stderr — stdout is the MCP JSON-RPC channel and must stay clean.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let service = MicaMcp::from_env()?.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::urlencode;

    #[test]
    fn urlencode_escapes_reserved_keeps_unreserved() {
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("a&b=c/d?e"), "a%26b%3Dc%2Fd%3Fe");
        assert_eq!(urlencode("keep-_.~AZaz09"), "keep-_.~AZaz09");
    }
}
