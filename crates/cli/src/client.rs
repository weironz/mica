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

#[derive(Debug, Deserialize)]
struct WorkspaceListResponse {
  workspaces: Vec<Workspace>,
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
      bail!("unauthorized (401) — run `mica-cli auth login` or set MICA_TOKEN");
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

  /// A workspace exported as a ZIP of Markdown pages + `assets/` images —
  /// exactly what the web "export.zip" button produces (shared server code).
  pub fn export_workspace_zip(&self, workspace_id: Uuid) -> Result<Vec<u8>> {
    let resp = self
      .authed(self.http.get(self.url(&format!("/workspaces/{workspace_id}/export.zip"))))
      .send()?;
    Ok(Self::ok(resp)?.bytes()?.to_vec())
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
