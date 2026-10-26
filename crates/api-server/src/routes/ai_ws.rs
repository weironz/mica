use axum::{
  extract::{
    Query, State,
    ws::{Message, WebSocket, WebSocketUpgrade},
  },
  http::HeaderMap,
  response::Response,
};
use futures_util::{SinkExt as _, StreamExt as _};
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
  Ok(crate::routes::ws::capped_socket(upgrade).on_upgrade(move |socket| run(socket, state)))
}

async fn run(socket: WebSocket, state: AppState) {
  let mut socket = socket;
  let first = match socket.next().await {
    Some(Ok(Message::Text(text))) => text,
    _ => return,
  };

  let request: AiStreamRequest = match serde_json::from_str(first.as_str()) {
    Ok(request) => request,
    Err(_) => {
      let _ = socket
        .send(Message::Text(
          json!({"type": "error", "message": "invalid request"})
            .to_string()
            .into(),
        ))
        .await;
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

  // Split so the upstream read loop can race the CLIENT's disappearance: the
  // receiver half is polled concurrently (#137), so a disconnect during a
  // heartbeat-only stretch cancels the upstream request instead of holding it
  // until the provider's own idle timeout.
  let (mut sink, mut downstream) = socket.split();
  let outcome = stream_completion(&mut sink, &mut downstream, &config, &system, &prompt).await;
  match outcome {
    Completion::Done => {
      let _ = sink
        .send(Message::Text(json!({"type": "done"}).to_string().into()))
        .await;
    }
    Completion::ClientGone => {} // nothing left to tell
    Completion::Error(message) => send_error_split(&mut sink, &message).await,
  }
}

/// How one completion ended. `Done` is the ONLY success: the provider said so
/// with its own finish frame (`[DONE]` / `message_stop`). An upstream EOF
/// without one is `Error` — "transport ended" is not "generation succeeded".
/// `ClientGone` means the downstream socket died, so there is nobody to tell.
enum Completion {
  Done,
  Error(String),
  ClientGone,
}

async fn stream_completion(
  sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
  downstream: &mut futures_util::stream::SplitStream<WebSocket>,
  config: &AiConfig,
  system: &str,
  prompt: &str,
) -> Completion {
  let body = build_request(config, system, prompt, true);

  let mut request = crate::http_client::ai_stream_client()
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

  let response = match request.send().await {
    Ok(response) => response,
    Err(error) => return Completion::Error(format!("AI request failed: {error}")),
  };

  let status = response.status();
  if !status.is_success() {
    let body = response.text().await.unwrap_or_default();
    let snippet: String = body.chars().take(300).collect();
    return Completion::Error(format!("AI provider error ({status}): {snippet}"));
  }

  let mut stream = response.bytes_stream();
  let mut buffer: Vec<u8> = Vec::new();
  loop {
    let chunk = tokio::select! {
      biased;
      downstream = downstream.next() => {
        match downstream {
          Some(Ok(_)) => continue,
          _ => return Completion::ClientGone,
        }
      }
      chunk = stream.next() => chunk,
    };
    let bytes = match chunk {
      Some(Ok(bytes)) => bytes,
      Some(Err(error)) => return Completion::Error(format!("stream error: {error}")),
      None => {
        return Completion::Error(
          "AI stream ended before the provider signaled completion".to_string(),
        );
      }
    };
    buffer.extend_from_slice(&bytes);

    for line in drain_complete_lines(&mut buffer) {
      let line = line.trim();
      if line.is_empty() || !line.starts_with("data:") {
        continue;
      }
      let data = line[5..].trim();
      if data == "[DONE]" {
        return Completion::Done;
      }
      let Ok(value) = serde_json::from_str::<Value>(data) else {
        continue;
      };
      if let Some(message) = provider_error(config.provider, &value) {
        return Completion::Error(message);
      }
      if is_provider_finish(config.provider, &value) {
        return Completion::Done;
      }
      if let Some(text) = extract_delta(config.provider, &value) {
        if !text.is_empty() {
          let sent = sink
            .send(Message::Text(
              json!({"type": "delta", "text": text}).to_string().into(),
            ))
            .await;
          if sent.is_err() {
            return Completion::ClientGone;
          }
        }
      }
    }
  }
}

/// The provider's own success-finish frame. `[DONE]` is handled literally
/// above (OpenAI sends it as a bare data line); this covers the structured
/// case — Anthropic's `{"type":"message_stop"}`.
fn is_provider_finish(provider: AiProvider, value: &Value) -> bool {
  match provider {
    AiProvider::Anthropic => value.get("type").and_then(|t| t.as_str()) == Some("message_stop"),
    AiProvider::OpenAi => false,
  }
}

/// An SSE error EVENT — the provider answered HTTP 200 and then said it
/// failed. Both dialects carry the message under `error.message`.
fn provider_error(provider: AiProvider, value: &Value) -> Option<String> {
  let message = match provider {
    AiProvider::Anthropic => {
      if value.get("type").and_then(|t| t.as_str()) == Some("error") {
        value.get("error")?.get("message")?.as_str()?
      } else {
        return None;
      }
    }
    AiProvider::OpenAi => value.get("error")?.get("message")?.as_str()?,
  };
  Some(format!("AI provider error: {message}"))
}

async fn send_error_split(
  sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
  message: &str,
) {
  let _ = sink
    .send(Message::Text(
      json!({"type": "error", "message": message})
        .to_string()
        .into(),
    ))
    .await;
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

fn extract_delta(provider: AiProvider, value: &Value) -> Option<String> {
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

#[cfg(test)]
mod terminal_tests {
  use super::*;
  use axum::{
    Router,
    routing::{get, post},
  };
  use futures_util::SinkExt;
  use std::{sync::Arc, time::Duration};
  use tokio_tungstenite::tungstenite::Message as ClientMessage;

  struct Fixture {
    admin: sqlx::PgPool,
    db: sqlx::PgPool,
    name: String,
    state: AppState,
    token: String,
  }

  impl Fixture {
    async fn new() -> Self {
      let url = std::env::var("DATABASE_URL").expect("AI socket tests require real Postgres");
      let admin = sqlx::PgPool::connect(&url).await.unwrap();
      let name = format!("mica_fix_h_ai_{}", uuid::Uuid::new_v4().simple());
      sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&admin)
        .await
        .unwrap();
      let (base, _) = url.rsplit_once('/').unwrap();
      let db = sqlx::PgPool::connect(&format!("{base}/{name}"))
        .await
        .unwrap();
      mica_infra::run_migrations(&db).await.unwrap();
      let user = uuid::Uuid::new_v4();
      sqlx::query("INSERT INTO users (id,email,display_name,password_hash) VALUES ($1,'ai@example.invalid','AI','unused')")
        .bind(user).execute(&db).await.unwrap();
      let mut config = mica_infra::AppConfig::from_env().unwrap();
      config.environment = mica_infra::Environment::Test;
      config.jwt_secret = "ai-terminal-regression-secret-0123456789".into();
      let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &json!({"sub": user.to_string(), "exp": 4102444800_u64}),
        &jsonwebtoken::EncodingKey::from_secret(config.jwt_secret.as_bytes()),
      )
      .unwrap();
      let state = AppState::new(config, db.clone(), Arc::new(mica_infra::LogMailer));
      Self {
        admin,
        db,
        name,
        state,
        token,
      }
    }

    async fn cleanup(self) {
      drop(self.state);
      self.db.close().await;
      sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {} WITH (FORCE)",
        self.name
      )))
      .execute(&self.admin)
      .await
      .unwrap();
      self.admin.close().await;
    }

    async fn serve(
      &self,
      provider: AiProvider,
      upstream: String,
    ) -> (String, tokio::task::JoinHandle<()>) {
      *self.state.ai.write().await = Some(AiConfig {
        provider_id: provider.as_str().into(),
        provider,
        api_key: "test".into(),
        model: "test".into(),
        base_url: upstream,
        max_tokens: 100,
        anthropic_version: "2023-06-01".into(),
      });
      let app = Router::new()
        .route("/ws/ai", get(ai_socket))
        .with_state(self.state.clone());
      let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
      let addr = listener.local_addr().unwrap();
      let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
      });
      (format!("ws://{addr}/ws/ai?token={}", self.token), server)
    }

    async fn frames(&self, provider: AiProvider, sse: &'static str) -> Vec<Value> {
      let path = match provider {
        AiProvider::OpenAi => "/chat/completions",
        AiProvider::Anthropic => "/v1/messages",
      };
      let upstream = Router::new().route(
        path,
        post(move || async move { ([("content-type", "text/event-stream")], sse) }),
      );
      let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
      let addr = listener.local_addr().unwrap();
      let upstream = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
      });
      let (url, server) = self.serve(provider, format!("http://{addr}")).await;
      let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
      socket
        .send(ClientMessage::text(r#"{"prompt":"rewrite"}"#))
        .await
        .unwrap();
      let frames = tokio::time::timeout(Duration::from_secs(5), async {
        let mut frames = Vec::new();
        while let Some(Ok(ClientMessage::Text(text))) = socket.next().await {
          let frame: Value = serde_json::from_str(&text).unwrap();
          let terminal = frame["type"] == "done" || frame["type"] == "error";
          frames.push(frame);
          if terminal {
            break;
          }
        }
        frames
      })
      .await
      .unwrap();
      drop(socket);
      server.abort();
      upstream.abort();
      let _ = server.await;
      let _ = upstream.await;
      frames
    }
  }

  #[tokio::test]
  async fn clean_upstream_eof_without_provider_finish_is_an_error() {
    let fixture = Fixture::new().await;
    let frames = fixture
      .frames(
        AiProvider::OpenAi,
        "data: {\"choices\":[{\"delta\":{\"content\":\"Partial result\"}}]}\n\n",
      )
      .await;
    fixture.cleanup().await;
    assert_eq!(frames[0], json!({"type":"delta","text":"Partial result"}));
    assert_eq!(
      frames[1]["type"], "error",
      "EOF cannot synthesize business done: {frames:?}"
    );
  }

  #[tokio::test]
  async fn openai_done_frame_is_the_only_success() {
    let fixture = Fixture::new().await;
    let frames = fixture
      .frames(
        AiProvider::OpenAi,
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n\n",
      )
      .await;
    fixture.cleanup().await;
    assert_eq!(frames[0], json!({"type":"delta","text":"Hi"}));
    assert_eq!(frames[1], json!({"type":"done"}));
    assert_eq!(frames.len(), 2);
  }

  #[tokio::test]
  async fn openai_sse_error_frame_propagates_as_error() {
    let fixture = Fixture::new().await;
    let frames = fixture.frames(AiProvider::OpenAi,
      "data: {\"choices\":[{\"delta\":{\"content\":\"Partial result\"}}]}\n\ndata: {\"error\":{\"message\":\"probe upstream failed\"}}\n\n").await;
    fixture.cleanup().await;
    assert_eq!(frames[1]["type"], "error");
    assert!(
      frames[1]["message"]
        .as_str()
        .unwrap()
        .contains("probe upstream failed")
    );
  }

  #[tokio::test]
  async fn anthropic_message_stop_is_success() {
    let fixture = Fixture::new().await;
    let frames = fixture.frames(AiProvider::Anthropic,
      "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\ndata: {\"type\":\"message_stop\"}\n\n").await;
    fixture.cleanup().await;
    assert_eq!(frames[0], json!({"type":"delta","text":"Hi"}));
    assert_eq!(frames[1], json!({"type":"done"}));
  }

  #[tokio::test]
  async fn anthropic_error_event_propagates_as_error() {
    let fixture = Fixture::new().await;
    let frames = fixture.frames(AiProvider::Anthropic,
      "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Partial result\"}}\n\ndata: {\"type\":\"error\",\"error\":{\"message\":\"overloaded\"}}\n\n").await;
    fixture.cleanup().await;
    assert_eq!(frames[1]["type"], "error");
    assert!(
      frames[1]["message"]
        .as_str()
        .unwrap()
        .contains("overloaded")
    );
  }

  /// #137: a client that walks away while the provider is sending nothing but
  /// comment heartbeats must not keep the upstream request open. The upstream
  /// body here yields `: ping` comments forever; the guard records the moment
  /// the server actually drops the response body (what closing the upstream
  /// connection does to it).
  #[tokio::test]
  async fn client_disconnect_during_heartbeat_only_stream_cancels_upstream() {
    use std::sync::atomic::{AtomicBool, Ordering};

    struct UpstreamGuard(Arc<AtomicBool>);
    impl Drop for UpstreamGuard {
      fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
      }
    }

    let fixture = Fixture::new().await;
    let dropped = Arc::new(AtomicBool::new(false));
    let observed = dropped.clone();
    let upstream = Router::new().route(
      "/chat/completions",
      post(move || {
        let dropped = dropped.clone();
        async move {
          let stream = futures_util::stream::unfold(UpstreamGuard(dropped), |guard| async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Some((Ok::<_, std::io::Error>(b": ping\n\n".to_vec()), guard))
          });
          axum::http::Response::builder()
            .header("content-type", "text/event-stream")
            .body(axum::body::Body::from_stream(stream))
            .unwrap()
        }
      }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
      axum::serve(listener, upstream).await.unwrap();
    });
    let (url, server) = fixture
      .serve(AiProvider::OpenAi, format!("http://{addr}"))
      .await;
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    socket
      .send(ClientMessage::text(r#"{"prompt":"rewrite"}"#))
      .await
      .unwrap();
    // Let a couple of comment-only heartbeats flow with no delta at all.
    tokio::time::sleep(Duration::from_millis(500)).await;
    drop(socket);
    let cancelled = tokio::time::timeout(Duration::from_secs(3), async {
      while !observed.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(50)).await;
      }
    })
    .await;
    server.abort();
    upstream.abort();
    let _ = server.await;
    let _ = upstream.await;
    fixture.cleanup().await;
    assert!(
      cancelled.is_ok(),
      "upstream request must be cancelled shortly after the client is gone"
    );
  }
}
