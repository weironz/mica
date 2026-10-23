use axum::{
  extract::{
    Query, State,
    ws::{Message, WebSocket, WebSocketUpgrade},
  },
  http::HeaderMap,
  response::Response,
};
use futures_util::StreamExt;
use mica_app_core::AppState;
use mica_infra::{AiConfig, AiProvider, ApiError, ApiResult};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::routes::ai::{DEFAULT_SYSTEM, build_request};
use crate::routes::auth::{ensure_user_exists, token_from_request, user_id_from_token};

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
  if !crate::origin::origin_allowed(&state.config, &headers) {
    return Err(ApiError::Forbidden);
  }
  let token = token_from_request(&headers, query.token.as_deref()).ok_or(ApiError::Unauthorized)?;
  let user_id = user_id_from_token(&state, &token)?;
  // Same check as the REST endpoint: this socket streams completions, so it
  // spends the operator's credit for as long as it stays open, and a deleted
  // account's access token is still a valid JWT until it expires.
  ensure_user_exists(&state.db, user_id).await?;
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
  let body = build_request(config, system, prompt, true);

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
  let mut buffer: Vec<u8> = Vec::new();
  while let Some(chunk) = stream.next().await {
    let bytes = chunk.map_err(|error| format!("stream error: {error}"))?;
    buffer.extend_from_slice(&bytes);

    for line in drain_complete_lines(&mut buffer) {
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

/// Take every COMPLETE line out of `buffer`, decoded; a partial last line
/// stays for the next chunk.
///
/// Decoding happens per line, never per network chunk: a chunk boundary can
/// fall inside a multi-byte character, and decoding each chunk on its own
/// turned a split CJK character into U+FFFD — routine for Chinese output.
/// `\n` is a single byte that never occurs inside a UTF-8 sequence, so a
/// complete line is always whole characters.
fn drain_complete_lines(buffer: &mut Vec<u8>) -> Vec<String> {
  let Some(last) = buffer.iter().rposition(|&b| b == b'\n') else {
    return Vec::new();
  };
  let rest = buffer.split_off(last + 1);
  let complete = std::mem::replace(buffer, rest);
  complete
    .split(|&b| b == b'\n')
    .map(|line| String::from_utf8_lossy(line).into_owned())
    .collect()
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

#[cfg(test)]
mod tests {
  use super::drain_complete_lines;

  /// THE BUG: each network chunk was decoded on its own, so a character split
  /// across two chunks became U+FFFD.
  #[test]
  fn a_character_split_across_chunks_survives() {
    let line = "data: 中文\n".as_bytes();
    let (a, b) = line.split_at(7); // inside the 3-byte `中`
    let mut buffer = Vec::new();
    buffer.extend_from_slice(a);
    assert!(
      drain_complete_lines(&mut buffer).is_empty(),
      "no complete line yet"
    );
    buffer.extend_from_slice(b);
    let lines = drain_complete_lines(&mut buffer);
    assert_eq!(
      lines.iter().filter(|l| !l.is_empty()).collect::<Vec<_>>(),
      ["data: 中文"]
    );
    assert!(buffer.is_empty());
  }

  #[test]
  fn a_partial_line_waits_for_the_rest() {
    let mut buffer = b"data: a\ndata: b".to_vec();
    assert_eq!(drain_complete_lines(&mut buffer), ["data: a", ""]);
    assert_eq!(buffer, b"data: b");
  }
}
