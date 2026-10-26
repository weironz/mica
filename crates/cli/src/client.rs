//! Thin blocking HTTP client over the Mica REST API (`<server>/api/...`).
//!
//! Every capability the CLI exposes is a call here — the CLI is an API client,
//! not a second implementation, so it can never drift from what web/desktop see.
//! Auth is a JWT bearer token (`Authorization: Bearer <token>`).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(test)]
mod contract_tests;

/// Client-side cap for a `download_bytes` rehost fetch, matching the server's
/// default upload cap (`S3_MAX_UPLOAD_BYTES`, 25 MiB). The server still
/// enforces ITS configured limit when the bytes land; this one is about not
/// buffering an unbounded body over a hostile connection.
const MAX_DOWNLOAD_BYTES: u64 = 25 * 1024 * 1024;

/// Outbound-call bounds (#45): the blocking client used to carry NO timeout at
/// all on an ordinary call and a hard total on none of the big ones — a wedged
/// connection parked a command forever, while nothing stopped a stalled server
/// answer. Ordinary calls get connect + total bounds (the total is generous
/// for a REST call; exports override per request); uploads get connect only,
/// because a total bound there kills a large Notion import mid-body.
/// `MICA_HTTP_TIMEOUT_SECS` overrides the ordinary-call total for slow links.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn request_timeout() -> std::time::Duration {
  std::env::var("MICA_HTTP_TIMEOUT_SECS")
    .ok()
    .and_then(|v| v.trim().parse::<u64>().ok())
    .filter(|secs| *secs > 0)
    .map(std::time::Duration::from_secs)
    .unwrap_or(DEFAULT_REQUEST_TIMEOUT)
}

pub struct Client {
  base: String,
  token: Option<String>,
  http: reqwest::blocking::Client,
  /// Connect-bound but NOT total-bound: only `start_import` uses it, because an
  /// archive of hundreds of MB must not be cut off by the ordinary-call total
  /// (#45 — the 30s default "finished" server-side work looked like a failure).
  upload: reqwest::blocking::Client,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct User {
  pub id: Uuid,
  pub email: String,
  pub display_name: String,
  pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthResponse {
  pub access_token: String,
  #[allow(dead_code)]
  pub token_type: String,
  pub expires_at: String,
  pub user: User,
}

#[derive(Debug, Deserialize)]
struct MeResponse {
  user: User,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Workspace {
  pub id: Uuid,
  pub name: String,
  pub owner_id: Uuid,
  pub role: String,
  pub created_at: String,
  pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ExportManifest {
  pub version: u32,
  pub structure_revision: String,
  pub tree_revision: String,
  pub manifest: serde_json::Value,
  pub pages: Vec<ExportPageRevision>,
  pub files: Vec<ExportFileRevision>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ExportFileRevision {
  pub file_id: Uuid,
  pub revision: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ExportPageRevision {
  pub view_id: Uuid,
  pub document_id: Uuid,
  pub revision: String,
}

#[derive(Debug, Deserialize)]
struct WorkspaceListResponse {
  workspaces: Vec<Workspace>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct View {
  /// The VIEW id — what move / trash / restore address. Distinct from
  /// [View::object_id], the document a view points at, which is what the read
  /// and write endpoints take. Confusing the two is the easiest mistake here,
  /// so `page list` prints both.
  pub id: Uuid,
  pub object_id: Uuid,
  pub object_type: String,
  pub name: String,
  #[serde(default)]
  pub is_deleted: bool,
  #[serde(default)]
  pub parent_view_id: Option<Uuid>,
  /// Size of the stored CRDT state, present only when the listing asked for
  /// stats. Small means nearly empty; large does NOT mean long (deleted text
  /// leaves weight behind), so never read it as a word count.
  #[serde(default)]
  pub state_bytes: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ImportStart {
  job_id: Uuid,
}

/// A running (or finished) import, as the polling endpoint reports it.
#[derive(Debug, Deserialize, Serialize)]
pub struct ImportJobView {
  /// `running` | `done` | `error` | `cancelled`.
  pub status: String,
  pub total: usize,
  pub done: usize,
  pub workspace_id: Option<Uuid>,
  #[serde(default)]
  pub error: Option<String>,
  /// Archive entries no imported page references — dropped, not failed. The
  /// list is CAPPED by the server; `skipped_total` is the honest count.
  #[serde(default)]
  pub skipped: Vec<String>,
  /// How many were skipped in total (the list above is a sample).
  #[serde(default)]
  pub skipped_total: usize,
  /// External images the import promised to bring in and did not — each one is
  /// still a link pointing at somebody else's server.
  #[serde(default)]
  pub image_failures: Vec<ImageFailure>,
  /// How many failed in total (the list above is a sample).
  #[serde(default)]
  pub image_failures_total: usize,
}

/// One external image that stayed a link, as the server reports it (app-core
/// `ImageFailure`). `document_id` + `block_id` are exactly what a retry through
/// `rehost-image` needs, which is why they are carried here.
#[derive(Debug, Deserialize, Serialize)]
pub struct ImageFailure {
  pub url: String,
  /// The page it sits on — a list of urls with no page names is not actionable.
  #[serde(default)]
  pub page: String,
  pub document_id: Uuid,
  pub block_id: String,
  pub reason: String,
  /// False when a circuit breaker refused it without even making the request.
  #[serde(default)]
  pub attempted: bool,
}

/// What a batch call actually did. `affected` counts every view touched —
/// including descendants that came along with a folder — so it is normally
/// larger than the number of ids sent. `skipped` names the requested ids that
/// were not touched: already in that state, or gone.
#[derive(Debug, Deserialize, Serialize)]
pub struct BatchOutcome {
  pub affected: usize,
  #[serde(default)]
  pub skipped: Vec<Uuid>,
}

/// Filters for [Client::list_views_filtered]; all optional.
#[derive(Debug, Default)]
pub struct ViewFilter {
  pub parent_view_id: Option<Uuid>,
  pub depth: Option<i32>,
  pub limit: Option<i64>,
  pub offset: Option<i64>,
  pub with_stats: bool,
}

#[derive(Debug, Deserialize)]
struct ViewListResponse {
  views: Vec<View>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreatedToken {
  pub id: Uuid,
  pub name: String,
  pub scopes: Vec<String>,
  pub token: String,
  pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TokenInfo {
  pub id: Uuid,
  pub name: String,
  pub scopes: Vec<String>,
  pub created_at: String,
  pub last_used_at: Option<String>,
  pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenListResponse {
  tokens: Vec<TokenInfo>,
}

impl Client {
  pub fn new(base: impl Into<String>, token: Option<String>) -> Result<Self> {
    let base = base.into().trim_end_matches('/').to_string();
    let user_agent = concat!("mica-cli/", env!("CARGO_PKG_VERSION"));
    let http = reqwest::blocking::Client::builder()
      .user_agent(user_agent)
      .connect_timeout(CONNECT_TIMEOUT)
      .timeout(request_timeout())
      .build()
      .context("building HTTP client")?;
    let upload = reqwest::blocking::Client::builder()
      .user_agent(user_agent)
      .connect_timeout(CONNECT_TIMEOUT)
      .build()
      .context("building upload HTTP client")?;
    Ok(Self {
      base,
      token,
      http,
      upload,
    })
  }

  fn url(&self, path: &str) -> String {
    format!("{}/api{path}", self.base)
  }

  fn authed(&self, rb: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
    match &self.token {
      Some(token) => rb.bearer_auth(token),
      None => rb,
    }
  }

  /// Turn a non-2xx response into a helpful error (401 gets a login hint).
  fn ok(resp: reqwest::blocking::Response) -> Result<reqwest::blocking::Response> {
    let status = resp.status();
    if status.is_success() {
      return Ok(resp);
    }
    if status.as_u16() == 401 {
      // Names the accepted sources, and only those. It used to say "set
      // MICA_TOKEN" — a variable the `mcp` path did not even read — which sent
      // people to change one they had already set correctly.
      bail!(
        "unauthorized (401) — the token was rejected. Pass --token, set MICA_PAT, \
         or run `mica-cli auth login`"
      );
    }
    let body = resp.text().unwrap_or_default();
    bail!("request failed: {status} — {}", body.trim());
  }

  pub fn login(&self, email: &str, password: &str) -> Result<AuthResponse> {
    let resp = self
      .http
      .post(self.url("/auth/login"))
      .json(&serde_json::json!({ "email": email, "password": password }))
      .send()
      .with_context(|| format!("connecting to {}", self.base))?;
    // A 401 here means bad credentials, not a missing token — give the right hint.
    if resp.status().as_u16() == 401 {
      bail!("invalid email or password");
    }
    Self::ok(resp)?.json().context("decoding login response")
  }

  pub fn me(&self) -> Result<User> {
    let resp = self.authed(self.http.get(self.url("/auth/me"))).send()?;
    let me: MeResponse = Self::ok(resp)?.json()?;
    Ok(me.user)
  }

  pub fn list_workspaces(&self) -> Result<Vec<Workspace>> {
    let resp = self.authed(self.http.get(self.url("/workspaces"))).send()?;
    let list: WorkspaceListResponse = Self::ok(resp)?.json()?;
    Ok(list.workspaces)
  }

  pub fn export_manifest(&self, workspace_id: Uuid) -> Result<ExportManifest> {
    let resp = self
      .authed(
        self
          .http
          .get(self.url(&format!("/workspaces/{workspace_id}/export-manifest"))),
      )
      .timeout(std::time::Duration::from_secs(180))
      .send()?;
    Self::ok(resp)?
      .json()
      .context("decoding export manifest (server and CLI must support incremental export)")
  }

  /// Only one page and its images are buffered, never a workspace archive.
  /// The revision is derived from the exact state used to render this ZIP.
  pub fn export_page_zip(
    &self,
    workspace_id: Uuid,
    view_id: Uuid,
    skip_assets: &[Uuid],
  ) -> Result<(String, String, Vec<u8>)> {
    let resp = self
      .authed(self.http.get(self.url(&format!(
        "/workspaces/{workspace_id}/export-pages/{view_id}/export.zip"
      ))))
      .query(&[(
        "skip_assets",
        skip_assets
          .iter()
          .map(Uuid::to_string)
          .collect::<Vec<_>>()
          .join(","),
      )])
      .timeout(std::time::Duration::from_secs(600))
      .send()?;
    let mut resp = Self::ok(resp)?;
    let revision = resp
      .headers()
      .get("x-mica-export-revision")
      .context("page export is missing its content revision")?
      .to_str()?
      .to_string();
    if revision.is_empty() {
      bail!("page export has an empty revision");
    }
    let structure = resp
      .headers()
      .get("x-mica-export-structure-revision")
      .context("page export is missing its structure revision")?
      .to_str()?
      .to_string();
    let mut bytes = Vec::new();
    resp.copy_to(&mut bytes)?;
    Ok((revision, structure, bytes))
  }

  pub fn server(&self) -> &str {
    &self.base
  }

  /// Views (folders + document leaves) in a workspace. Trashed views are already
  /// filtered server-side.
  pub fn list_views(&self, workspace_id: Uuid) -> Result<Vec<View>> {
    let resp = self
      .authed(
        self
          .http
          .get(self.url(&format!("/workspaces/{workspace_id}/views"))),
      )
      .send()?;
    let list: ViewListResponse = Self::ok(resp)?.json()?;
    Ok(list.views)
  }

  /// Views, narrowed. The unfiltered call returns the whole tree, which on a
  /// large workspace is a lot of JSON to move in order to look at one folder.
  pub fn list_views_filtered(&self, workspace_id: Uuid, filter: &ViewFilter) -> Result<Vec<View>> {
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(parent) = filter.parent_view_id {
      query.push(("parent_view_id", parent.to_string()));
    }
    if let Some(depth) = filter.depth {
      query.push(("depth", depth.to_string()));
    }
    if let Some(limit) = filter.limit {
      query.push(("limit", limit.to_string()));
    }
    if let Some(offset) = filter.offset {
      query.push(("offset", offset.to_string()));
    }
    if filter.with_stats {
      query.push(("with_stats", "true".to_string()));
    }
    let resp = self
      .authed(
        self
          .http
          .get(self.url(&format!("/workspaces/{workspace_id}/views")))
          .query(&query),
      )
      .send()?;
    let list: ViewListResponse = Self::ok(resp)?.json()?;
    Ok(list.views)
  }

  /// What is in the recycle bin. Soft-deleted, so all of it is restorable.
  pub fn list_trash(&self, workspace_id: Uuid) -> Result<Vec<View>> {
    let resp = self
      .authed(
        self
          .http
          .get(self.url(&format!("/workspaces/{workspace_id}/trash"))),
      )
      .send()?;
    let list: ViewListResponse = Self::ok(resp)?.json()?;
    Ok(list.views)
  }

  /// One page's Markdown.
  pub fn read_markdown(&self, workspace_id: Uuid, document_id: Uuid) -> Result<String> {
    let resp = self
      .authed(self.http.get(self.url(&format!(
        "/workspaces/{workspace_id}/documents/{document_id}/markdown"
      ))))
      .send()?;
    let body: serde_json::Value = Self::ok(resp)?.json()?;
    Ok(
      body
        .get("markdown")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string(),
    )
  }

  /// Many pages' Markdown in one round trip. Per-document failures come back
  /// inline rather than failing the request, so one deleted page does not lose
  /// the rest of a survey.
  pub fn batch_read_documents(
    &self,
    workspace_id: Uuid,
    document_ids: &[Uuid],
  ) -> Result<serde_json::Value> {
    let resp = self
      .authed(
        self
          .http
          .post(self.url(&format!("/workspaces/{workspace_id}/documents/batch-read")))
          .json(&serde_json::json!({ "document_ids": document_ids })),
      )
      .send()?;
    Ok(Self::ok(resp)?.json()?)
  }

  fn batch_views(
    &self,
    workspace_id: Uuid,
    action: &str,
    body: serde_json::Value,
  ) -> Result<BatchOutcome> {
    let resp = self
      .authed(
        self
          .http
          .post(self.url(&format!("/workspaces/{workspace_id}/views/{action}")))
          .json(&body),
      )
      .send()?;
    Self::ok(resp)?.json().context("decoding batch result")
  }

  /// Soft-delete many views (each with its subtree) in one request.
  pub fn batch_trash_views(&self, workspace_id: Uuid, view_ids: &[Uuid]) -> Result<BatchOutcome> {
    self.batch_views(
      workspace_id,
      "batch-trash",
      serde_json::json!({ "view_ids": view_ids }),
    )
  }

  /// Bring many views back out of the recycle bin in one request.
  pub fn batch_restore_views(&self, workspace_id: Uuid, view_ids: &[Uuid]) -> Result<BatchOutcome> {
    self.batch_views(
      workspace_id,
      "batch-restore",
      serde_json::json!({ "view_ids": view_ids }),
    )
  }

  /// Re-parent many views under one folder (None = workspace root).
  pub fn batch_move_views(
    &self,
    workspace_id: Uuid,
    view_ids: &[Uuid],
    parent_view_id: Option<Uuid>,
  ) -> Result<BatchOutcome> {
    self.batch_views(
      workspace_id,
      "batch-move",
      serde_json::json!({ "view_ids": view_ids, "parent_view_id": parent_view_id }),
    )
  }

  /// Empty the recycle bin. PERMANENT — nothing here is recoverable after.
  pub fn empty_trash(&self, workspace_id: Uuid) -> Result<serde_json::Value> {
    let resp = self
      .authed(
        self
          .http
          .delete(self.url(&format!("/workspaces/{workspace_id}/trash"))),
      )
      // Purging a large bin can run well past the ordinary-call total while
      // the server is still working fine — and a client that gives up reports
      // a failure for work that actually completed (#45).
      .timeout(std::time::Duration::from_secs(300))
      .send()?;
    Ok(Self::ok(resp)?.json()?)
  }

  /// Hand an archive to the server and get the job that is now importing it.
  ///
  /// The upload is the only part that needs this process: the import itself
  /// runs server-side, keyed by the returned id, and outlives us leaving.
  pub fn start_import(
    &self,
    zip: Vec<u8>,
    name: Option<&str>,
    workspace_id: Option<Uuid>,
    parent_view_id: Option<Uuid>,
    notion: bool,
    rehost_external: bool,
  ) -> Result<Uuid> {
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(name) = name {
      query.push(("name", name.to_string()));
    }
    if let Some(ws) = workspace_id {
      query.push(("workspace_id", ws.to_string()));
    }
    if let Some(parent) = parent_view_id {
      query.push(("parent_view_id", parent.to_string()));
    }
    if notion {
      query.push(("notion", "true".to_string()));
    }
    // Only sent when turning it OFF — the server defaults it on.
    if !rehost_external {
      query.push(("rehost_external", "false".to_string()));
    }
    let resp = self
      .authed(
        self
          // The upload client, not the ordinary one: an archive of hundreds of
          // MB legitimately outlasts the ordinary call's total timeout (#45).
          .upload
          .post(self.url("/workspaces/import"))
          .query(&query)
          .body(zip),
      )
      .send()?;
    let started: ImportStart = Self::ok(resp)?.json().context("decoding import start")?;
    Ok(started.job_id)
  }

  /// One poll of a running import.
  pub fn import_job(&self, job_id: Uuid) -> Result<ImportJobView> {
    let resp = self
      .authed(self.http.get(self.url(&format!("/import/jobs/{job_id}"))))
      .send()?;
    Self::ok(resp)?.json().context("decoding import job")
  }

  /// A document's full bootstrap snapshot as raw JSON — the caller navigates
  /// `["snapshot"]["payload"]["blocks"]` (a flat block list).
  pub fn document_bootstrap(
    &self,
    workspace_id: Uuid,
    document_id: Uuid,
  ) -> Result<serde_json::Value> {
    let resp = self
      .authed(self.http.get(self.url(&format!(
        "/workspaces/{workspace_id}/documents/{document_id}/bootstrap"
      ))))
      .send()?;
    Ok(Self::ok(resp)?.json()?)
  }

  /// Download a URL's bytes over THIS machine's network (no auth). The whole
  /// point of CLI re-hosting: the client reaches hosts (AppFlowy, imgur) that a
  /// CN-hosted server 403s.
  ///
  /// SSRF-guarded — the SAME rule as the server's `fetch_and_store_image_url`,
  /// because the threat moved with the fetch: a collaborator who drops an
  /// internal URL into an image block must not get the machine running
  /// `rehost-images` to pull intranet content up into the workspace. So the
  /// URL is vetted BEFORE any request (http(s) only, public addresses only,
  /// every resolved answer checked) and the vetted addresses are PINNED onto a
  /// purpose-built client (`resolve_to_addrs`), closing the DNS-rebinding gap
  /// where a host answers the check publicly and the connection privately.
  /// Redirects stay OFF so a public URL cannot 30x-bounce onto an internal
  /// address. The body must be an image (`image/*` header, or sniff as one by
  /// magic number when the host mislabels it) and is size-capped while
  /// streaming, not after buffering.
  pub fn download_bytes(&self, url: &str) -> Result<Vec<u8>> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("parsing {url}"))?;
    let pinned = crate::ssrf::vetted_pinned_addrs(&parsed)?;
    // Safe: vetted_pinned_addrs already refused a host-less URL.
    let host = parsed.host_str().unwrap_or_default().to_string();

    // A dedicated client, not `self.http`: the API client follows redirects and
    // resolves freely, which is exactly what this fetch must never do.
    let client = reqwest::blocking::Client::builder()
      .user_agent(concat!("mica-cli/", env!("CARGO_PKG_VERSION")))
      .timeout(std::time::Duration::from_secs(8))
      .redirect(reqwest::redirect::Policy::none())
      .resolve_to_addrs(&host, &pinned)
      .build()
      .context("building the guarded download client")?;
    let resp = client
      .get(url)
      .send()
      .with_context(|| format!("downloading {url}"))?;
    // `Self::ok` only fails 401/4xx/5xx; a 3xx must not pass here — with
    // redirects off, following it is the refusal, not reqwest's default.
    if resp.status().is_redirection() {
      bail!(
        "image url returned {} — redirects are not followed",
        resp.status()
      );
    }
    let resp = Self::ok(resp)?;

    let header_mime = resp
      .headers()
      .get(reqwest::header::CONTENT_TYPE)
      .and_then(|v| v.to_str().ok())
      .map(|s| s.split(';').next().unwrap_or(s).trim().to_string());

    // Cheap up-front refusal when the server volunteers its size. NOT the
    // enforcement — a lying or chunked response can omit or understate it; the
    // capped read below is.
    if let Some(declared) = resp
      .headers()
      .get(reqwest::header::CONTENT_LENGTH)
      .and_then(|v| v.to_str().ok())
      .and_then(|s| s.parse::<u64>().ok())
      && declared > MAX_DOWNLOAD_BYTES
    {
      bail!("image is too large: {declared} bytes exceeds the {MAX_DOWNLOAD_BYTES} byte limit");
    }

    // Read at most MAX+1 bytes: the moment the cap is passed the fetch is
    // refused, instead of buffering a hostile body first and asking after.
    use std::io::Read;
    let mut bytes = Vec::new();
    resp
      .take(MAX_DOWNLOAD_BYTES + 1)
      .read_to_end(&mut bytes)
      .context("reading the image body")?;
    if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
      bail!("image is too large: body exceeds the {MAX_DOWNLOAD_BYTES} byte limit");
    }

    // Must actually be an image: an `image/*` content type, or bytes that
    // sniff as one when the host mislabels (an octet-stream PNG is still a
    // PNG). Anything else — HTML, JSON, an intranet error page — is refused.
    let from_header = header_mime
      .as_deref()
      .is_some_and(|m| m.starts_with("image/"));
    if !from_header && crate::ssrf::sniff_image_ext(&bytes).is_none() {
      bail!(
        "url is not an image (content-type: {})",
        header_mime.as_deref().unwrap_or("<none>")
      );
    }
    Ok(bytes)
  }

  /// Store already-downloaded image [bytes] into Mica and repoint image
  /// [block_id] in [document_id] at the new file (the `rehost-image` endpoint).
  /// Delete an image block whose source is gone. `url` is what the caller
  /// checked; the server refuses if the block no longer carries it.
  pub fn drop_image(
    &self,
    workspace_id: Uuid,
    document_id: Uuid,
    block_id: &str,
    url: &str,
  ) -> Result<()> {
    let resp = self
      .authed(
        self
          .http
          .post(self.url(&format!(
            "/workspaces/{workspace_id}/documents/{document_id}/drop-image"
          )))
          .query(&[("block_id", block_id), ("url", url)]),
      )
      .send()?;
    Self::ok(resp)?;
    Ok(())
  }

  pub fn rehost_image(
    &self,
    workspace_id: Uuid,
    document_id: Uuid,
    block_id: &str,
    file_name: &str,
    bytes: Vec<u8>,
  ) -> Result<()> {
    let resp = self
      .authed(
        self
          .http
          .post(self.url(&format!(
            "/workspaces/{workspace_id}/documents/{document_id}/rehost-image"
          )))
          .query(&[("block_id", block_id), ("file_name", file_name)])
          .body(bytes),
      )
      .send()?;
    Self::ok(resp)?;
    Ok(())
  }

  pub fn create_token(
    &self,
    name: &str,
    scopes: &[String],
    expires_in_days: Option<i64>,
  ) -> Result<CreatedToken> {
    let mut body = serde_json::json!({ "name": name, "scopes": scopes });
    if let Some(days) = expires_in_days {
      body["expires_in_days"] = serde_json::json!(days);
    }
    let resp = self
      .authed(self.http.post(self.url("/auth/tokens")))
      .json(&body)
      .send()?;
    Self::ok(resp)?.json().context("decoding created token")
  }

  pub fn list_tokens(&self) -> Result<Vec<TokenInfo>> {
    let resp = self
      .authed(self.http.get(self.url("/auth/tokens")))
      .send()?;
    let list: TokenListResponse = Self::ok(resp)?.json()?;
    Ok(list.tokens)
  }

  pub fn revoke_token(&self, id: Uuid) -> Result<()> {
    let resp = self
      .authed(self.http.delete(self.url(&format!("/auth/tokens/{id}"))))
      .send()?;
    Self::ok(resp)?;
    Ok(())
  }
}
