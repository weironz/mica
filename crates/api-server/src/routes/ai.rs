use axum::{Json, extract::State, http::HeaderMap};
use mica_app_core::AppState;
use mica_infra::{AiConfig, AiProvider, ApiError, ApiResult};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::routes::auth::{admin_id_from_headers, user_id_from_headers};

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
  /// The VENDOR in use (`deepseek`, `zhipu`, …). What the dropdown selects.
  provider_id: String,
  /// Every vendor that has a stored config, so the dropdown can mark them.
  /// The single-row schema could not answer this, which is how the screen came
  /// to show "key configured" for a provider that had never had one.
  configured_providers: Vec<ConfiguredProvider>,
  /// Whether the CALLER may change these settings. The dialog uses it to show
  /// the fields read-only instead of letting someone fill in a key and then
  /// eat a 403 on save.
  #[serde(default)]
  can_edit: bool,
  provider: String,
  base_url: String,
  model: String,
  max_tokens: u32,
  has_key: bool,
  /// Last 4 characters of the stored key, or empty. Enough to tell "already
  /// configured, and it is the key I think it is" apart from "never set" —
  /// which a bare boolean rendered as a row of dots could not do.
  key_hint: String,
}

#[derive(Debug, Serialize)]
pub struct ConfiguredProvider {
  provider_id: String,
  has_key: bool,
  model: String,
  active: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAiSettingsRequest {
  /// The vendor this write is about. Switching providers is a write with only
  /// this field: the server answers with THAT vendor's stored config.
  provider_id: Option<String>,
  provider: Option<String>,
  /// The provider endpoint. Honoured again now that this route is admin-only —
  /// see `update_settings` for why it used to be discarded and what changed.
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
  let user_id = user_id_from_headers(&state, &headers).await?;
  let config = state.ai.read().await.clone();
  let can_edit = sqlx::query_scalar::<_, bool>("SELECT is_admin FROM users WHERE id = $1")
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .unwrap_or(false);
  let mut response = settings_response(config.as_ref());
  response.can_edit = can_edit;
  response.configured_providers = configured_providers(&state).await;
  Ok(Json(response))
}

/// `PATCH /api/ai/settings` — choose the provider / base URL / model / key.
pub async fn update_settings(
  State(state): State<AppState>,
  headers: HeaderMap,
  Json(payload): Json<UpdateAiSettingsRequest>,
) -> ApiResult<Json<AiSettingsResponse>> {
  // Instance-wide settings holding the operator's provider key — being signed
  // in is not enough. See `admin_id_from_headers`.
  let admin_id = admin_id_from_headers(&state, &headers).await?;

  let mut guard = state.ai.write().await;
  let current = guard.clone();

  // Which vendor this write is about. Absent means "the one in use" — an older
  // client that only ever knew about a single config.
  let provider_id = payload
    .provider_id
    .map(|value| value.trim().to_lowercase())
    .filter(|value| !value.is_empty())
    .or_else(|| current.as_ref().map(|c| c.provider_id.clone()))
    .unwrap_or_else(|| "custom".to_string());

  // The BASELINE is that vendor's own stored row — not whatever is running.
  // Getting this wrong is the whole bug 0022 exists for: inheriting the active
  // config meant selecting Zhipu handed it DeepSeek's key, and the screen then
  // reported a configured key for a provider that had never had one.
  let stored = mica_infra::AiConfig::load_provider(&state.db, &provider_id).await;
  let base = stored.as_ref().or_else(|| {
    current
      .as_ref()
      .filter(|c| c.provider_id == provider_id)
  });

  let provider = match payload.provider.as_deref() {
    Some(value) => AiProvider::parse(value)
      .ok_or_else(|| ApiError::BadRequest(format!("unknown AI protocol: {value}")))?,
    None => base.map(|c| c.provider).unwrap_or(AiProvider::OpenAi),
  };

  let base_url = payload
    .base_url
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty())
    .or_else(|| base.map(|c| c.base_url.clone()))
    .unwrap_or_else(|| default_base_url(provider));

  let model = payload
    .model
    .map(|value| value.trim().to_string())
    .or_else(|| base.map(|c| c.model.clone()))
    .unwrap_or_default();

  // Absent keeps this vendor's key; "" clears it. It can never pick up another
  // vendor's key, because `base` is scoped to this vendor.
  let api_key = payload
    .api_key
    .map(|value| value.trim().to_string())
    .or_else(|| base.map(|c| c.api_key.clone()))
    .unwrap_or_default();

  let max_tokens = payload
    .max_tokens
    .or_else(|| base.map(|c| c.max_tokens))
    .unwrap_or(2048);

  let config = AiConfig {
    provider_id,
    provider,
    api_key,
    model,
    base_url,
    max_tokens,
    anthropic_version: match provider {
      AiProvider::Anthropic => "2023-06-01".to_string(),
      AiProvider::OpenAi => String::new(),
    },
  };
  config.save(&state.db, Some(admin_id)).await?;
  let mut response = settings_response(Some(&config));
  response.can_edit = true;
  response.configured_providers = configured_providers(&state).await;
  *guard = Some(config);
  Ok(Json(response))
}

/// The stored providers, for the dropdown. `has_key` is per VENDOR here, which
/// is the point: it used to be a property of the instance and therefore said
/// "configured" no matter which provider was selected.
async fn configured_providers(state: &AppState) -> Vec<ConfiguredProvider> {
  mica_infra::AiConfig::list(&state.db)
    .await
    .into_iter()
    .map(|(provider_id, model, api_key, active)| ConfiguredProvider {
      provider_id,
      has_key: !api_key.trim().is_empty(),
      model,
      active,
    })
    .collect()
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
      key_hint: key_hint(&config.api_key),
      can_edit: false,
      provider_id: config.provider_id.clone(),
      configured_providers: Vec::new(),
    },
    None => AiSettingsResponse {
      configured: false,
      provider: AiProvider::OpenAi.as_str().to_string(),
      base_url: String::new(),
      model: String::new(),
      max_tokens: 2048,
      has_key: false,
      key_hint: String::new(),
      can_edit: false,
      provider_id: String::new(),
      configured_providers: Vec::new(),
    },
  }
}

/// The tail of a key, for recognising it without revealing it. Short keys get
/// nothing rather than most of themselves.
fn key_hint(key: &str) -> String {
  let key = key.trim();
  if key.chars().count() < 8 {
    return String::new();
  }
  key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect()
}

fn default_base_url(provider: AiProvider) -> String {
  match provider {
    AiProvider::Anthropic => "https://api.anthropic.com".to_string(),
    AiProvider::OpenAi => "https://api.deepseek.com".to_string(),
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

/// Candidate URLs for a provider's OpenAI-compatible model list, tried in order.
///
/// Ported from cc-switch (`src-tauri/src/services/model_fetch.rs`,
/// `build_models_url_candidates`), which is the one project I found that had
/// already paid for this knowledge. The reason it is a PROBE and not a per-
/// provider path table is the interesting part: the table is unmaintainable.
/// Every vendor's OpenAI-compatible surface sits at a slightly different depth
/// — some at the root, some already carrying a version segment, some behind an
/// Anthropic-compat sub-path — and the set of vendors changes faster than
/// anyone updates a table. Deriving the candidates from the base URL means a
/// provider nobody has heard of still works.
///
/// The rules, each of which exists because a real endpoint needed it:
///   * a base already ending in a version segment (`/v1`, Zhipu's
///     `/api/coding/paas/v4`) takes `{base}/models` — appending `/v1` again
///     gives `.../paas/v4/v1/models`, a 404;
///   * a non-`/v1` version segment keeps `{base}/v1/models` as a second guess;
///   * anything else takes `{base}/v1/models`;
///   * a base ending in a known Anthropic-compat sub-path also tries the root
///     without it, longest suffix first (`/api/anthropic` must not match as
///     `/anthropic` and leave a stranded `/api`).
fn models_url_candidates(base_url: &str) -> Vec<String> {
  /// Longest first, so the longer path wins the match.
  const COMPAT_SUFFIXES: &[&str] = &[
    "/api/claudecode",
    "/api/anthropic",
    "/apps/anthropic",
    "/api/coding",
    "/claudecode",
    "/anthropic",
    "/coding",
    "/claude",
  ];

  let base = base_url.trim().trim_end_matches('/');
  if base.is_empty() {
    return Vec::new();
  }

  let ends_with_version = base
    .rsplit('/')
    .next()
    .and_then(|last| last.strip_prefix('v'))
    .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));

  let mut out: Vec<String> = Vec::new();
  if ends_with_version {
    out.push(format!("{base}/models"));
    if !base.ends_with("/v1") {
      out.push(format!("{base}/v1/models"));
    }
  } else {
    out.push(format!("{base}/v1/models"));
  }

  if let Some(root) = COMPAT_SUFFIXES
    .iter()
    .find_map(|suffix| base.strip_suffix(*suffix))
  {
    let root = root.trim_end_matches('/');
    if root.contains("://") {
      out.push(format!("{root}/v1/models"));
      out.push(format!("{root}/models"));
    }
  }

  out.dedup();
  out
}

#[derive(Debug, Deserialize)]
pub struct ListModelsQuery {
  /// Probe THESE settings instead of the saved ones, so an admin can pick a
  /// provider and see its models before committing the change. Omitted fields
  /// fall back to what is stored.
  base_url: Option<String>,
  api_key: Option<String>,
  provider: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListModelsResponse {
  models: Vec<String>,
  /// Which candidate answered. Shown in the UI because "it worked, but against
  /// a URL you did not expect" is a real outcome of probing.
  endpoint: String,
}

/// `GET /api/ai/models` — the provider's own model list.
///
/// Admin-only, and not merely because it is a settings screen: the probe spends
/// the operator's key against a URL the caller supplies. That is also why the
/// model list is never hardcoded — every vendor renames and retires models on
/// its own schedule, and a list baked into this binary is wrong the day after
/// it ships. Asking the provider is the only answer that stays true.
pub async fn list_models(
  State(state): State<AppState>,
  headers: HeaderMap,
  axum::extract::Query(query): axum::extract::Query<ListModelsQuery>,
) -> ApiResult<Json<ListModelsResponse>> {
  admin_id_from_headers(&state, &headers).await?;
  let saved = state.ai.read().await.clone();

  let base_url = query
    .base_url
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty())
    .or_else(|| saved.as_ref().map(|c| c.base_url.clone()))
    .unwrap_or_default();
  let api_key = query
    .api_key
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty())
    .or_else(|| saved.as_ref().map(|c| c.api_key.clone()))
    .unwrap_or_default();
  let provider = query
    .provider
    .as_deref()
    .and_then(AiProvider::parse)
    .or_else(|| saved.as_ref().map(|c| c.provider))
    .unwrap_or(AiProvider::OpenAi);

  if base_url.is_empty() {
    return Err(ApiError::BadRequest(
      "set a base URL before fetching models".to_string(),
    ));
  }

  let candidates = models_url_candidates(&base_url);
  if candidates.is_empty() {
    return Err(ApiError::BadRequest(format!(
      "cannot derive a model-list endpoint from {base_url}"
    )));
  }

  let client = reqwest::Client::new();
  let mut last_status: Option<String> = None;
  for url in &candidates {
    let mut request = client
      .get(url)
      .timeout(std::time::Duration::from_secs(15));
    if !api_key.is_empty() {
      request = match provider {
        AiProvider::Anthropic => request.header("x-api-key", &api_key),
        AiProvider::OpenAi => request.header("authorization", format!("Bearer {api_key}")),
      };
    }
    let response = match request.send().await {
      Ok(response) => response,
      // A transport error is about the host, not the path — trying the next
      // candidate would just wait out the same timeout again.
      Err(error) => {
        return Err(ApiError::Unavailable(format!(
          "could not reach {base_url}: {error}"
        )));
      }
    };

    let status = response.status();
    if status.is_success() {
      let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| ApiError::Unavailable(format!("unreadable model list: {error}")))?;
      let mut models: Vec<String> = body
        .get("data")
        .and_then(|data| data.as_array())
        .map(|entries| {
          entries
            .iter()
            .filter_map(|entry| entry.get("id").and_then(|id| id.as_str()))
            .map(str::to_string)
            .collect()
        })
        .unwrap_or_default();
      models.sort();
      models.dedup();
      return Ok(Json(ListModelsResponse {
        models,
        endpoint: url.clone(),
      }));
    }

    // Only a missing path is worth another guess. A 401 means the key is wrong
    // and every remaining candidate would say the same thing more slowly.
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
    {
      last_status = Some(status.to_string());
      continue;
    }
    return Err(ApiError::Unavailable(format!(
      "{url} answered {status}"
    )));
  }

  Err(ApiError::Unavailable(format!(
    "no model list at {} ({})",
    candidates.join(", "),
    last_status.unwrap_or_else(|| "no candidates".to_string())
  )))
}

#[cfg(test)]
mod models_endpoint_tests {
  use super::models_url_candidates;

  /// The plain case: a bare host takes the OpenAI convention.
  #[test]
  fn a_bare_host_gets_v1_models() {
    assert_eq!(
      models_url_candidates("https://api.deepseek.com"),
      vec!["https://api.deepseek.com/v1/models"]
    );
    // A trailing slash must not produce a double slash.
    assert_eq!(
      models_url_candidates("https://api.deepseek.com/"),
      vec!["https://api.deepseek.com/v1/models"]
    );
  }

  /// A base that already carries its version must not get a second one:
  /// `.../paas/v4/v1/models` is a 404, and it was the reported failure that put
  /// this rule in cc-switch in the first place.
  #[test]
  fn a_versioned_base_takes_models_directly() {
    assert_eq!(
      models_url_candidates("https://open.bigmodel.cn/api/coding/paas/v4"),
      vec![
        "https://open.bigmodel.cn/api/coding/paas/v4/models",
        // Kept as a second guess only because /v4 is not /v1 — some gateways
        // really do nest a /v1 under their own version.
        "https://open.bigmodel.cn/api/coding/paas/v4/v1/models",
      ]
    );
    // /v1 needs no second guess: it would be the same URL.
    assert_eq!(
      models_url_candidates("https://api.moonshot.cn/v1"),
      vec!["https://api.moonshot.cn/v1/models"]
    );
  }

  /// An Anthropic-compat sub-path usually has no model list of its own, so the
  /// root is worth trying too.
  #[test]
  fn a_compat_subpath_also_probes_the_root() {
    assert_eq!(
      models_url_candidates("https://api.deepseek.com/anthropic"),
      vec![
        "https://api.deepseek.com/anthropic/v1/models",
        "https://api.deepseek.com/v1/models",
        "https://api.deepseek.com/models",
      ]
    );
  }

  /// Longest suffix first — matching `/anthropic` inside `/api/anthropic` would
  /// leave a stranded `https://host/api` root and probe a URL nobody serves.
  #[test]
  fn the_longest_compat_suffix_wins() {
    assert_eq!(
      models_url_candidates("https://api.z.ai/api/anthropic"),
      vec![
        "https://api.z.ai/api/anthropic/v1/models",
        "https://api.z.ai/v1/models",
        "https://api.z.ai/models",
      ]
    );
  }

  /// A local model server is a first-class case, not an edge one.
  #[test]
  fn a_local_endpoint_works_like_any_other() {
    assert_eq!(
      models_url_candidates("http://localhost:11434/v1"),
      vec!["http://localhost:11434/v1/models"]
    );
  }

  #[test]
  fn an_empty_base_has_no_candidates() {
    assert!(models_url_candidates("   ").is_empty());
  }
}
