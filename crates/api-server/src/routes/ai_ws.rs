use axum::{
  extract::{
    Query, State,
    ws::{Message, WebSocket, WebSocketUpgrade},
  },
  http::{HeaderMap, header::AUTHORIZATION},
  response::Response,
};
use futures_util::StreamExt;
use mica_app_core::AppState;
use mica_infra::{AiConfig, AiProvider, ApiError, ApiResult};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::routes::auth::user_id_from_token;

#[derive(Debug, Deserialize)]
pub struct AiConnectQuery {
  token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AiStreamRequest {
  prompt: String,
  #[serde(default)]
  system: Option<String>,
}

const DEFAULT_SYSTEM: &str = "You are a writing assistant inside a Markdown document editor. \
Respond with clean GitHub-Flavored Markdown only — no preamble, no code fences around the whole \
answer, no commentary. Use headings, lists, and tables where helpful.";

/// `GET /ws/ai` — stream an AI completion token-by-token so the client can show
/// the response as it is generated. The client sends one JSON text frame
/// `{ "prompt": "...", "system": "(optional)" }`; the server replies with
/// `{type:"delta",text}` frames, then `{type:"done"}` (or `{type:"error",message}`).
pub async fn ai_socket(
  State(state): State<AppState>,
  Query(query): Query<AiConnectQuery>,
  headers: HeaderMap,
  upgrade: WebSocketUpgrade,
) -> ApiResult<Response> {
  let token = token_from(&headers, &query).ok_or(ApiError::Unauthorized)?;
  let _user_id = user_id_from_token(&state, &token)?;
  Ok(upgrade.on_upgrade(move |socket| run(socket, state)))
}

async fn run(mut socket: WebSocket, state: AppState) {
  let first = match socket.recv().await {
    Some(Ok(Message::Text(text))) => text,
    _ => return,
  };

  let request: AiStreamRequest = match serde_json::from_str(first.as_str()) {
    Ok(request) => request,
    Err(_) => {
      send_error(&mut socket, "invalid request").await;
      return;
    }
  };

  let prompt = request.prompt.trim().to_string();
  if prompt.is_empty() {
    send_error(&mut socket, "prompt cannot be empty").await;
    return;
  }

  let config = state.ai.read().await.clone();
  let Some(config) = config else {
    send_error(&mut socket, "AI is not configured on this server").await;
    return;
  };

  let system = request
    .system
    .as_deref()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .unwrap_or(DEFAULT_SYSTEM)
    .to_string();

  match stream_completion(&mut socket, &config, &system, &prompt).await {
    Ok(()) => {
      let _ = socket
        .send(Message::Text(json!({"type": "done"}).to_string().into()))
        .await;
    }
    Err(message) => send_error(&mut socket, &message).await,
  }
  // Dropping `socket` when this future returns closes the connection.
}

async fn stream_completion(
  socket: &mut WebSocket,
  config: &AiConfig,
  system: &str,
  prompt: &str,
) -> Result<(), String> {
  let body = match config.provider {
    AiProvider::Anthropic => json!({
      "model": config.model,
      "max_tokens": config.max_tokens,
      "stream": true,
      "system": system,
      "messages": [ { "role": "user", "content": prompt } ],
    }),
    AiProvider::OpenAi => json!({
      "model": config.model,
      "max_tokens": config.max_tokens,
      "stream": true,
      "messages": [
        { "role": "system", "content": system },
        { "role": "user", "content": prompt },
      ],
    }),
  };

  let mut request = reqwest::Client::new()
    .post(config.endpoint())
    .header("content-type", "application/json")
    .json(&body);
  request = match config.provider {
    AiProvider::Anthropic => request
      .header("x-api-key", &config.api_key)
      .header("anthropic-version", &config.anthropic_version),
    AiProvider::OpenAi => {
      if config.has_key() {
        request.header("authorization", format!("Bearer {}", config.api_key))
      } else {
        request
      }
    }
  };

  let response = request
    .send()
    .await
    .map_err(|error| format!("AI request failed: {error}"))?;

  let status = response.status();
  if !status.is_success() {
    let body = response.text().await.unwrap_or_default();
    let snippet: String = body.chars().take(300).collect();
    return Err(format!("AI provider error ({status}): {snippet}"));
  }

  let mut stream = response.bytes_stream();
  let mut buffer = String::new();
  while let Some(chunk) = stream.next().await {
    let bytes = chunk.map_err(|error| format!("stream error: {error}"))?;
    buffer.push_str(&String::from_utf8_lossy(&bytes));

    while let Some(pos) = buffer.find('\n') {
      let line: String = buffer.drain(..=pos).collect();
      let line = line.trim();
      if line.is_empty() || !line.starts_with("data:") {
        continue;
      }
      let data = line[5..].trim();
      if data == "[DONE]" {
        return Ok(());
      }
      if let Some(text) = extract_delta(config.provider, data) {
        if !text.is_empty() {
          socket
            .send(Message::Text(
              json!({"type": "delta", "text": text}).to_string().into(),
            ))
            .await
            .map_err(|error| format!("socket closed: {error}"))?;
        }
      }
    }
  }
  Ok(())
}

fn extract_delta(provider: AiProvider, data: &str) -> Option<String> {
  let value: Value = serde_json::from_str(data).ok()?;
  match provider {
    AiProvider::OpenAi => value
      .get("choices")?
      .get(0)?
      .get("delta")?
      .get("content")?
      .as_str()
      .map(str::to_string),
    AiProvider::Anthropic => {
      if value.get("type").and_then(|t| t.as_str()) == Some("content_block_delta") {
        value
          .get("delta")?
          .get("text")?
          .as_str()
          .map(str::to_string)
      } else {
        None
      }
    }
  }
}

async fn send_error(socket: &mut WebSocket, message: &str) {
  let _ = socket
    .send(Message::Text(
      json!({"type": "error", "message": message})
        .to_string()
        .into(),
    ))
    .await;
}

fn token_from(headers: &HeaderMap, query: &AiConnectQuery) -> Option<String> {
  if let Some(token) = headers
    .get(AUTHORIZATION)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.strip_prefix("Bearer "))
  {
    return Some(token.to_string());
  }
  query
    .token
    .as_ref()
    .map(|token| token.trim().to_string())
    .filter(|token| !token.is_empty())
}
