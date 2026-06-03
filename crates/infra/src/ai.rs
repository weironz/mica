use std::{env, fs};

/// Which API dialect to speak to the model provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiProvider {
  /// Anthropic Messages API (`/v1/messages`, `x-api-key`).
  Anthropic,
  /// OpenAI-compatible Chat Completions (`/chat/completions`, `Bearer`).
  /// Covers OpenAI, DeepSeek, and local servers (Ollama, LM Studio, vLLM, …).
  OpenAi,
}

impl AiProvider {
  pub fn as_str(self) -> &'static str {
    match self {
      AiProvider::Anthropic => "anthropic",
      AiProvider::OpenAi => "openai",
    }
  }

  pub fn parse(value: &str) -> Option<Self> {
    match value.trim().to_lowercase().as_str() {
      "anthropic" | "claude" => Some(AiProvider::Anthropic),
      "openai" | "deepseek" | "local" | "openai-compatible" | "ollama" => Some(AiProvider::OpenAi),
      _ => None,
    }
  }
}

/// Active AI provider configuration. Built from the environment / `deepseek.conf`
/// at startup, then mutable at runtime through the AI settings endpoint. When
/// absent the AI endpoints return `503`.
#[derive(Debug, Clone)]
pub struct AiConfig {
  pub provider: AiProvider,
  pub api_key: String,
  pub model: String,
  /// Provider base URL, e.g. `https://api.anthropic.com`, `https://api.deepseek.com`,
  /// or `http://localhost:11434/v1` for a local model.
  pub base_url: String,
  pub max_tokens: u32,
  pub anthropic_version: String,
}

impl AiConfig {
  /// Resolve the initial config from the environment and `deepseek.conf`.
  pub fn from_env() -> Option<Self> {
    let _ = dotenvy::dotenv();
    let max_tokens = env::var("AI_MAX_TOKENS")
      .ok()
      .and_then(|value| value.parse::<u32>().ok())
      .unwrap_or(2048);

    // 1. Anthropic, if a key is present.
    if let Some(api_key) = first_non_empty(&["ANTHROPIC_API_KEY"]) {
      return Some(Self {
        provider: AiProvider::Anthropic,
        api_key,
        model: first_non_empty(&["AI_MODEL", "ANTHROPIC_MODEL"])
          .unwrap_or_else(|| "claude-sonnet-4-6".to_string()),
        base_url: first_non_empty(&["AI_BASE_URL", "ANTHROPIC_BASE_URL"])
          .unwrap_or_else(|| "https://api.anthropic.com".to_string()),
        max_tokens,
        anthropic_version: first_non_empty(&["ANTHROPIC_VERSION"])
          .unwrap_or_else(|| "2023-06-01".to_string()),
      });
    }

    // 2. DeepSeek (OpenAI-compatible), via env or the deepseek.conf file.
    if let Some(api_key) = deepseek_key() {
      return Some(Self {
        provider: AiProvider::OpenAi,
        api_key,
        model: first_non_empty(&["AI_MODEL", "DEEPSEEK_MODEL"])
          .unwrap_or_else(|| "deepseek-chat".to_string()),
        base_url: first_non_empty(&["AI_BASE_URL", "DEEPSEEK_BASE_URL"])
          .unwrap_or_else(|| "https://api.deepseek.com".to_string()),
        max_tokens,
        anthropic_version: String::new(),
      });
    }

    // 3. Generic OpenAI-compatible (incl. local models that may need no key).
    let openai_key = first_non_empty(&["OPENAI_API_KEY", "AI_API_KEY"]);
    let base_url = first_non_empty(&["AI_BASE_URL", "OPENAI_BASE_URL"]);
    if openai_key.is_some() || base_url.is_some() {
      return Some(Self {
        provider: AiProvider::OpenAi,
        api_key: openai_key.unwrap_or_default(),
        model: first_non_empty(&["AI_MODEL"]).unwrap_or_else(|| "gpt-4o-mini".to_string()),
        base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        max_tokens,
        anthropic_version: String::new(),
      });
    }

    None
  }

  /// Endpoint URL for the configured provider.
  pub fn endpoint(&self) -> String {
    let base = self.base_url.trim_end_matches('/');
    match self.provider {
      AiProvider::Anthropic => format!("{base}/v1/messages"),
      AiProvider::OpenAi => format!("{base}/chat/completions"),
    }
  }

  pub fn has_key(&self) -> bool {
    !self.api_key.trim().is_empty()
  }
}

fn deepseek_key() -> Option<String> {
  if let Some(key) = first_non_empty(&["DEEPSEEK_API_KEY"]) {
    return Some(key);
  }
  let path = env::var("DEEPSEEK_CONF").unwrap_or_else(|_| "deepseek.conf".to_string());
  let contents = fs::read_to_string(path).ok()?;
  let key = contents.trim();
  if key.is_empty() {
    None
  } else {
    Some(key.to_string())
  }
}

fn first_non_empty(keys: &[&str]) -> Option<String> {
  for key in keys {
    if let Ok(value) = env::var(key) {
      let trimmed = value.trim();
      if !trimmed.is_empty() {
        return Some(trimmed.to_string());
      }
    }
  }
  None
}
