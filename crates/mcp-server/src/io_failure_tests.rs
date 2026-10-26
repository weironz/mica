//! Network-boundary tests for issues #114 and #115 (#117 is pure and lives in
//! `lib.rs`'s `tests` module).
//!
//! A fake Mica API on a loopback socket exercises the REAL reqwest path: the
//! response is truncated mid-body or the connection is dropped after the write
//! has landed. Both bugs lived exactly at that boundary — `resp.text()` folded
//! a read error into `""`, and the cache was only dropped when the request
//! reported success.

use std::sync::{Arc, Mutex};

use rmcp::handler::server::wrapper::Parameters;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::args::{ReadDocArgs, UpdateDocArgs};
use crate::*;
const WS: Uuid = Uuid::nil();
const DOC: Uuid = Uuid::nil();

/// What the fake server should do with one request.
enum Action {
  Json(u16, String),
  /// Send these headers (declaring [declared] bytes) but only [bytes] of body,
  /// then close — a proxy cutting the response short.
  Truncate {
    declared: usize,
    bytes: &'static [u8],
  },
  /// Close the connection without any response — the request was already
  /// committed server-side.
  Drop,
}

struct FakeApi {
  base: String,
  seen: Arc<Mutex<Vec<String>>>,
}

async fn fake_api(handler: impl Fn(&str) -> Action + Send + Sync + 'static) -> FakeApi {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
  let log = seen.clone();
  let handler = Arc::new(handler);
  tokio::spawn(async move {
    loop {
      let Ok((sock, _)) = listener.accept().await else {
        return;
      };
      let handler = handler.clone();
      let log = log.clone();
      tokio::spawn(async move {
        let mut sock = sock;
        let Some(request) = read_request(&mut sock).await else {
          return;
        };
        log.lock().unwrap().push(request.clone());
        match handler(&request) {
          Action::Json(status, body) => {
            let _ = sock
              .write_all(
                format!(
                  "HTTP/1.1 {} X\r\nContent-Type: application/json\r\n\
                   Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                  status,
                  body.len(),
                  body
                )
                .as_bytes(),
              )
              .await;
            let _ = sock.flush().await;
          }
          Action::Truncate { declared, bytes } => {
            let _ = sock
              .write_all(
                format!(
                  "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                   Content-Length: {}\r\nConnection: close\r\n\r\n",
                  declared
                )
                .as_bytes(),
              )
              .await;
            let _ = sock.write_all(bytes).await;
            let _ = sock.flush().await;
          }
          Action::Drop => {}
        }
      });
    }
  });
  FakeApi { base, seen }
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> Option<String> {
  let mut reader = BufReader::new(&mut *sock);
  let mut request_line = String::new();
  if reader.read_line(&mut request_line).await.ok()? == 0 {
    return None;
  }
  let mut content_length = 0usize;
  loop {
    let mut header = String::new();
    if reader.read_line(&mut header).await.ok()? == 0 || header == "\r\n" {
      break;
    }
    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
      content_length = value.trim().parse().unwrap_or(0);
    }
  }
  let mut body = vec![0u8; content_length];
  let _ = reader.read_exact(&mut body).await;
  Some(request_line.trim_end().to_string())
}

fn read_args() -> Parameters<ReadDocArgs> {
  Parameters(ReadDocArgs {
    workspace_id: WS,
    document_id: DOC,
    offset: None,
    limit: None,
    section: None,
    refresh: false,
  })
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
  format!("{:?}", result.content)
}

fn export_gets(seen: &Arc<Mutex<Vec<String>>>) -> usize {
  seen
    .lock()
    .unwrap()
    .iter()
    .filter(|line| line.contains("/export/markdown"))
    .count()
}

/// #114. HTTP 200 with a body that stops early is not a successful empty page.
/// It must be a tool error, and it must NOT seed the warm cache.
#[tokio::test]
async fn a_truncated_200_body_is_an_error_and_is_not_cached() {
  let api = fake_api(|request| {
    if request.contains("/export/markdown") {
      Action::Truncate {
        declared: 100,
        bytes: b"{\"markdow",
      }
    } else {
      Action::Json(404, r#"{"error":"unexpected"}"#.to_string())
    }
  })
  .await;
  let server = MicaMcp::new(api.base.clone(), "pat".into(), false);

  let first = server.mica_read_document(read_args()).await.unwrap();
  assert_eq!(
    first.is_error,
    Some(true),
    "a body read failure must be isError, got {}",
    text_of(&first)
  );
  assert!(
    text_of(&first).contains("body"),
    "the error names the failed body read, got {}",
    text_of(&first)
  );

  let second = server.mica_read_document(read_args()).await.unwrap();
  assert_eq!(second.is_error, Some(true), "still broken, not blank");
  assert_eq!(
    export_gets(&api.seen),
    2,
    "the failed body must not be cached — the second read has to re-GET"
  );
}

/// #114's other half: a genuinely empty — but COMPLETE — body still succeeds,
/// and because it is a complete success it may be cached.
#[tokio::test]
async fn a_legal_empty_markdown_body_still_succeeds() {
  let api = fake_api(|request| {
    if request.contains("/export/markdown") {
      Action::Json(200, r#"{"markdown":""}"#.to_string())
    } else {
      Action::Json(404, r#"{"error":"unexpected"}"#.to_string())
    }
  })
  .await;
  let server = MicaMcp::new(api.base.clone(), "pat".into(), false);

  let first = server.mica_read_document(read_args()).await.unwrap();
  assert_ne!(first.is_error, Some(true), "empty is a legal page");
  assert_eq!(
    first.content.len(),
    1,
    "one empty text block comes back, not an error"
  );

  let second = server.mica_read_document(read_args()).await.unwrap();
  assert_ne!(second.is_error, Some(true));
}

/// #115. The PATCH commits, then the response is lost. The tool reports the
/// request failure (no retry), but the stale cached copy must be dropped so the
/// next read confirms the server's state.
#[tokio::test]
async fn a_write_whose_response_is_lost_invalidates_the_cache() {
  let document = Arc::new(Mutex::new("old document text\n".to_string()));
  let doc_for_server = document.clone();
  let api = fake_api(move |request| {
    if request.contains("/markdown") && request.starts_with("PATCH ") {
      // The write lands, then the connection dies.
      let mut text = doc_for_server.lock().unwrap();
      text.push_str("new document text\n");
      return Action::Drop;
    }
    if request.contains("/export/markdown") {
      let text = doc_for_server.lock().unwrap().clone();
      return Action::Json(200, serde_json::json!({ "markdown": text }).to_string());
    }
    Action::Json(404, r#"{"error":"unexpected"}"#.to_string())
  })
  .await;
  let server = MicaMcp::new(api.base.clone(), "pat".into(), false);

  let before = server.mica_read_document(read_args()).await.unwrap();
  assert_ne!(before.is_error, Some(true));
  assert!(text_of(&before).contains("old document text"));

  let write = server
    .mica_update_document(Parameters(UpdateDocArgs {
      workspace_id: WS,
      document_id: DOC,
      mode: "append".to_string(),
      markdown: Some("new document text\n".to_string()),
      anchor: None,
      find: None,
      replace: None,
      replacement_format: "markdown".to_string(),
      unique: false,
      expected_seq: None,
    }))
    .await
    .unwrap();
  assert_eq!(
    write.is_error,
    Some(true),
    "the lost response is still a reported failure (and is NOT retried)"
  );

  let after = server.mica_read_document(read_args()).await.unwrap();
  assert_ne!(after.is_error, Some(true));
  assert!(
    text_of(&after).contains("new document text"),
    "the next read must confirm the committed write, got {}",
    text_of(&after)
  );
  assert_eq!(
    export_gets(&api.seen),
    2,
    "the cache was invalidated even though the write reported an error"
  );
}
