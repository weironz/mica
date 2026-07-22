use axum::{Json, extract::State, http::HeaderMap};
use mica_app_core::AppState;
use mica_infra::{AiConfig, AiProvider, ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::routes::auth::user_id_from_headers;

#[derive(Debug, Deserialize)]
pub struct AiCompleteRequest {
  prompt: String,
  /// Optional system instruction overriding the default.
  system: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AiCompleteResponse {
  text: String,
}

#[derive(Debug, Serialize)]
pub struct AiSettingsResponse {
  configured: bool,
  provider: String,
  base_url: String,
  model: String,
  max_tokens: u32,
  has_key: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAiSettingsRequest {
  provider: Option<String>,
  /// Accepted for backward compatibility but DELIBERATELY IGNORED: `base_url` is
  /// operator-controlled only (see `update_settings`). A user-supplied value
  /// would redirect the operator's API key, so old clients that still send it
  /// are silently no-op'd rather than rejected.
  #[allow(dead_code)]
  base_url: Option<String>,
  model: Option<String>,
  /// Write-only; omit to keep the existing key, send "" to clear it.
  api_key: Option<String>,
  max_tokens: Option<u32>,
}

const DEFAULT_SYSTEM: &str = "You are a writing assistant inside a Markdown document editor. \
Respond with clean GitHub-Flavored Markdown only — no preamble, no code fences around the whole \
answer, no commentary. Use headings, lists, and tables where helpful.";

/// `POST /api/ai/complete` — generate Markdown from a prompt. Requires a signed-in
/// user; returns 503 when the server has no AI provider configured.
pub async fn complete(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<AiCompleteRequest>,
) -> ApiResult<Json<AiCompleteResponse>> {
  let _user_id = user_id_from_headers(&state, &headers).await?;

  let config = state
    .ai
    .read()
    .await
    .clone()
    .ok_or_else(|| ApiError::Unavailable("AI is not configured on this server".to_string()))?;

  let prompt = payload.prompt.trim();
  if prompt.is_empty() {
    return Err(ApiError::BadRequest("prompt cannot be empty".to_string()));
  }

  let system = payload
    .system
    .as_deref()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .unwrap_or(DEFAULT_SYSTEM);

  let text = generate(&config, system, prompt).await?;
  Ok(Json(AiCompleteResponse { text }))
}

/// `GET /api/ai/settings` — current AI configuration (never returns the key).
pub async fn get_settings(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<AiSettingsResponse>> {
  let _user_id = user_id_from_headers(&state, &headers).await?;
  let config = state.ai.read().await.clone();
  Ok(Json(settings_response(config.as_ref())))
}

/// `PATCH /api/ai/settings` — choose the provider / base URL / model / key.
pub async fn update_settings(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<UpdateAiSettingsRequest>,
) -> ApiResult<Json<AiSettingsResponse>> {
  let _user_id = user_id_from_headers(&state, &headers).await?;

  let mut guard = state.ai.write().await;
  let current = guard.clone();

  let provider = match payload.provider.as_deref() {
    Some(value) => AiProvider::parse(value)
      .ok_or_else(|| ApiError::BadRequest(format!("unknown AI provider: {value}")))?,
    None => current
      .as_ref()
      .map(|c| c.provider)
      .unwrap_or(AiProvider::OpenAi),
  };

  // `base_url` is OPERATOR-controlled only. `state.ai` is a process-wide
  // singleton that carries the operator's provider API key; letting a normal
  // user point `base_url` at their own host would exfiltrate that key (and open
  // an SSRF). So the request body's `base_url` is deliberately ignored (old
  // clients that still send it are not rejected — just no-op'd): keep the
  // env/config endpoint when the provider is unchanged, else the provider
  // default on a switch.
  let base_url = match current.as_ref() {
    Some(c) if c.provider == provider => c.base_url.clone(),
    _ => default_base_url(provider),
  };

  let model = payload
    .model
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty())
    .or_else(|| current.as_ref().map(|c| c.model.clone()))
    .unwrap_or_else(|| default_model(provider));

  let api_key = match payload.api_key {
    Some(value) => value.trim().to_string(),
    None => current
      .as_ref()
      .map(|c| c.api_key.clone())
      .unwrap_or_default(),
  };

  let max_tokens = payload
    .max_tokens
    .or_else(|| current.as_ref().map(|c| c.max_tokens))
    .unwrap_or(2048);

  let anthropic_version = current
    .as_ref()
    .map(|c| c.anthropic_version.clone())
    .filter(|value| !value.is_empty())
    .unwrap_or_else(|| "2023-06-01".to_string());

  let config = AiConfig {
    provider,
    api_key,
    model,
    base_url,
    max_tokens,
    anthropic_version,
  };
  let response = settings_response(Some(&config));
  *guard = Some(config);
  Ok(Json(response))
}

fn settings_response(config: Option<&AiConfig>) -> AiSettingsResponse {
  match config {
    Some(config) => AiSettingsResponse {
      configured: true,
      provider: config.provider.as_str().to_string(),
      base_url: config.base_url.clone(),
      model: config.model.clone(),
      max_tokens: config.max_tokens,
      has_key: config.has_key(),
    },
    None => AiSettingsResponse {
      configured: false,
      provider: AiProvider::OpenAi.as_str().to_string(),
      base_url: String::new(),
      model: String::new(),
      max_tokens: 2048,
      has_key: false,
    },
  }
}

fn default_base_url(provider: AiProvider) -> String {
  match provider {
    AiProvider::Anthropic => "https://api.anthropic.com".to_string(),
    AiProvider::OpenAi => "https://api.deepseek.com".to_string(),
  }
}

fn default_model(provider: AiProvider) -> String {
  match provider {
    AiProvider::Anthropic => "claude-sonnet-4-6".to_string(),
    AiProvider::OpenAi => "deepseek-chat".to_string(),
  }
}

async fn generate(config: &AiConfig, system: &str, prompt: &str) -> ApiResult<String> {
  match config.provider {
    AiProvider::Anthropic => generate_anthropic(config, system, prompt).await,
    AiProvider::OpenAi => generate_openai(config, system, prompt).await,
  }
}

async fn generate_anthropic(config: &AiConfig, system: &str, prompt: &str) -> ApiResult<String> {
  let body = json!({
    "model": config.model,
    "max_tokens": config.max_tokens,
    "system": system,
    "messages": [ { "role": "user", "content": prompt } ],
  });

  let payload = send(
    reqwest::Client::new()
      .post(config.endpoint())
      .header("x-api-key", &config.api_key)
      .header("anthropic-version", &config.anthropic_version)
      .header("content-type", "application/json")
      .json(&body),
  )
  .await?;

  // { content: [ { type: "text", text: "..." }, ... ] }
  let text = payload
    .get("content")
    .and_then(|content| content.as_array())
    .map(|blocks| {
      blocks
        .iter()
        .filter(|block| block.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("")
    })
    .unwrap_or_default();

  finish(text)
}

async fn generate_openai(config: &AiConfig, system: &str, prompt: &str) -> ApiResult<String> {
  let body = json!({
    "model": config.model,
    "max_tokens": config.max_tokens,
    "stream": false,
    "messages": [
      { "role": "system", "content": system },
      { "role": "user", "content": prompt },
    ],
  });

  let mut request = reqwest::Client::new()
    .post(config.endpoint())
    .header("content-type", "application/json")
    .json(&body);
  if config.has_key() {
    request = request.header("authorization", format!("Bearer {}", config.api_key));
  }

  let payload = send(request).await?;

  // { choices: [ { message: { content: "..." } } ] }
  let text = payload
    .get("choices")
    .and_then(|choices| choices.as_array())
    .and_then(|choices| choices.first())
    .and_then(|choice| choice.get("message"))
    .and_then(|message| message.get("content"))
    .and_then(|content| content.as_str())
    .unwrap_or_default()
    .to_string();

  finish(text)
}

async fn send(request: reqwest::RequestBuilder) -> ApiResult<serde_json::Value> {
  let response = request
    .send()
    .await
    .map_err(|error| ApiError::Unavailable(format!("AI request failed: {error}")))?;
  let status = response.status();
  let payload: serde_json::Value = response
    .json()
    .await
    .map_err(|error| ApiError::Internal(format!("invalid AI response: {error}")))?;

  if !status.is_success() {
    let message = payload
      .get("error")
      .and_then(|error| error.get("message").or(Some(error)))
      .and_then(|message| message.as_str())
      .unwrap_or("AI provider returned an error");
    return Err(ApiError::Unavailable(format!(
      "AI provider error ({status}): {message}"
    )));
  }
  Ok(payload)
}

fn finish(text: String) -> ApiResult<String> {
  if text.trim().is_empty() {
    return Err(ApiError::Internal(
      "AI returned an empty response".to_string(),
    ));
  }
  Ok(text)
}
