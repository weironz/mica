//! Thin blocking HTTP client over the Mica REST API (`<server>/api/...`).
//!
//! Every capability the CLI exposes is a call here — the CLI is an API client,
//! not a second implementation, so it can never drift from what web/desktop see.
//! Auth is a JWT bearer token (`Authorization: Bearer <token>`).

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct Client {
  base: String,
  token: Option<String>,
  http: reqwest::blocking::Client,
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
  /// count is what separates "imported everything" from "imported most of it".
  #[serde(default)]
  pub skipped: Vec<String>,
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
    let http = reqwest::blocking::Client::builder()
      .user_agent(concat!("mica-cli/", env!("CARGO_PKG_VERSION")))
      .build()
      .context("building HTTP client")?;
    Ok(Self { base, token, http })
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
          .http
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

  /// Download an image over THIS machine's network (no auth). The whole point
  /// of CLI re-hosting: the client reaches hosts (AppFlowy, imgur) that a
  /// CN-hosted server 403s.
  ///
  /// The URL comes from a document, i.e. from any collaborator, and the bytes
  /// go back into the workspace — so without limits this is an SSRF on the
  /// operator's own network: plant `http://192.168.1.1/…` as an image, wait
  /// for someone to run the sweep, read the router's admin page in the
  /// workspace. Hence the same rules the server's `import-url` follows: only
  /// public addresses, resolved once and pinned (no DNS rebinding), redirects
  /// followed by hand so every hop is checked, an image or nothing, and a size
  /// cap matching what `rehost-image` accepts.
  pub fn download_bytes(&self, url: &str) -> Result<Vec<u8>> {
    const MAX_HOPS: usize = 5;
    let mut current = reqwest::Url::parse(url).with_context(|| format!("bad url {url}"))?;
    for _ in 0..=MAX_HOPS {
      let resp = fetch_public(&current)?;
      if resp.status().is_redirection() {
        let location = resp
          .headers()
          .get(reqwest::header::LOCATION)
          .and_then(|v| v.to_str().ok())
          .with_context(|| format!("{current} redirected without a Location"))?;
        current = current
          .join(location)
          .with_context(|| format!("{current} redirected to a bad url"))?;
        continue;
      }
      let resp = Self::ok(resp)?;
      let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase())
        .unwrap_or_default();
      use std::io::Read as _;
      let mut bytes = Vec::new();
      resp
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("downloading {current}"))?;
      if bytes.len() as u64 > MAX_IMAGE_BYTES {
        bail!("{current} is larger than {} MiB", MAX_IMAGE_BYTES >> 20);
      }
      if !mime.starts_with("image/") && !looks_like_raster_image(&bytes) {
        bail!("{current} is not an image (content-type {mime:?})");
      }
      return Ok(bytes);
    }
    bail!("{url}: more than {MAX_HOPS} redirects")
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
    let resp = self.authed(self.http.post(self.url("/auth/tokens"))).json(&body).send()?;
    Self::ok(resp)?.json().context("decoding created token")
  }

  pub fn list_tokens(&self) -> Result<Vec<TokenInfo>> {
    let resp = self.authed(self.http.get(self.url("/auth/tokens"))).send()?;
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

/// The `rehost-image` endpoint's body limit.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// One request to `url`, only if its host resolves exclusively to public
/// addresses — and connecting to exactly those (`resolve_to_addrs`), so a
/// second DNS answer cannot swap in a private one. Redirects are not followed;
/// the caller does that, through here again.
fn fetch_public(url: &reqwest::Url) -> Result<reqwest::blocking::Response> {
  if !matches!(url.scheme(), "http" | "https") {
    bail!("refusing {url}: only http(s) urls are fetched");
  }
  let host = url.host_str().with_context(|| format!("{url} has no host"))?;
  let addrs = url
    .socket_addrs(|| url.port_or_known_default())
    .with_context(|| format!("resolving {host}"))?;
  if addrs.is_empty()
    || addrs
      .iter()
      .any(|a| mica_interchange::net::is_blocked_addr(a.ip()))
  {
    bail!("refusing {url}: it resolves to a private or loopback address");
  }
  reqwest::blocking::Client::builder()
    .user_agent(concat!("mica-cli/", env!("CARGO_PKG_VERSION")))
    .redirect(reqwest::redirect::Policy::none())
    .resolve_to_addrs(host, &addrs)
    .connect_timeout(std::time::Duration::from_secs(10))
    .timeout(std::time::Duration::from_secs(120))
    .build()
    .context("building HTTP client")?
    .get(url.clone())
    .send()
    .with_context(|| format!("downloading {url}"))
}

/// For hosts that serve images as `application/octet-stream` (or no type at
/// all): the magic bytes of the raster formats a document image can be.
/// Deliberately not SVG — that is markup, and an internal page could pass as it.
fn looks_like_raster_image(b: &[u8]) -> bool {
  b.starts_with(b"\x89PNG\r\n\x1a\n")
    || b.starts_with(&[0xFF, 0xD8, 0xFF])
    || b.starts_with(b"GIF87a")
    || b.starts_with(b"GIF89a")
    || (b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP")
    // BMP: "BM" alone is too weak a signature; its reserved words are zero.
    || (b.len() >= 14 && b.starts_with(b"BM") && b[6..10] == [0, 0, 0, 0])
    || b.starts_with(&[0, 0, 1, 0])
    || (b.len() >= 12 && &b[4..8] == b"ftyp" && matches!(&b[8..12], b"avif" | b"avis" | b"heic" | b"heix" | b"mif1"))
}

#[cfg(test)]
mod download_tests {
  use super::*;

  /// Issue #32: a document image pointing into the operator's LAN (or at the
  /// cloud metadata address) is refused before any request is made.
  #[test]
  fn private_targets_are_refused_without_a_request() {
    for url in [
      "http://127.0.0.1:9/x.png",
      "http://192.168.1.1/admin",
      "http://169.254.169.254/latest/meta-data/",
      "http://[::1]/x.png",
      "http://localhost:9/x.png",
      "file:///etc/passwd",
    ] {
      let err = fetch_public(&reqwest::Url::parse(url).unwrap()).unwrap_err();
      let msg = format!("{err:#}");
      assert!(msg.contains("refusing"), "{url}: {msg}");
    }
  }

  #[test]
  fn raster_sniff_accepts_images_and_rejects_markup() {
    assert!(looks_like_raster_image(b"\x89PNG\r\n\x1a\n...."));
    assert!(looks_like_raster_image(&[0xFF, 0xD8, 0xFF, 0xE0]));
    assert!(looks_like_raster_image(b"RIFF\0\0\0\0WEBPVP8 "));
    assert!(!looks_like_raster_image(b"<html><body>router admin</body></html>"));
    assert!(!looks_like_raster_image(b"{\"AccessKeyId\":\"x\"}"));
    assert!(!looks_like_raster_image(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
  }
}
