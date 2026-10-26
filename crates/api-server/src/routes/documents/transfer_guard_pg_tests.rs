// Postgres-backed reproduction of gh#154: a cross-workspace MOVE reads every
// payload in phase 2, copies blobs in phase 3 (outside any transaction) and
// only then commits phase 4. A content write that lands during the blob copy
// is not visible to the phase-4 guard, which compares view-ID SETS only — so
// the move used to succeed carrying the pre-write payload, leaving the new
// text only in the source's recycle bin.
//
// The window is held open the same way the audit probe held it: the object
// store is a loopback server (here, in-process) that answers the presigned GET
// only when the test says so. Everything else is real — the route handler, a
// real `documents` write through the REST markdown route, a real database.
//
//   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//   cargo test -p mica-api-server a_content_write_during_the_blob_copy
use super::transfer::TransferRequest;
use super::{MarkdownUpdateMode, UpdateMarkdownRequest, transfer_view, update_document_markdown};
use axum::{
  Json,
  extract::{Path, State},
  http::{HeaderMap, header::AUTHORIZATION},
};
use mica_app_core::AppState;
use mica_infra::ApiError;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::oneshot;
use uuid::Uuid;

const JWT_SECRET: &str = "transfer-guard-pg-test-secret";
const BLOB: &[u8] = b"fake-png-bytes";

async fn pool() -> Option<PgPool> {
  let Ok(url) = std::env::var("DATABASE_URL") else {
    assert!(
      std::env::var("CI").is_err(),
      "DATABASE_URL is unset in CI — the postgres service block regressed; \
         these tests must not silently pass"
    );
    return None;
  };
  Some(
    PgPool::connect(&url)
      .await
      .expect("DATABASE_URL is set but the connection failed"),
  )
}

async fn seed_user(db: &PgPool) -> Uuid {
  let user = Uuid::new_v4();
  sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
    .bind(user)
    .bind(format!("{user}@transfer-guard.test"))
    .execute(db)
    .await
    .unwrap();
  user
}

async fn seed_workspace(db: &PgPool, user: Uuid) -> Uuid {
  let ws = Uuid::new_v4();
  sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'W',$2)")
    .bind(ws)
    .bind(user)
    .execute(db)
    .await
    .unwrap();
  sqlx::query(
    "INSERT INTO workspace_members(workspace_id,user_id,role,position) VALUES($1,$2,'owner',$3)",
  )
  .bind(ws)
  .bind(user)
  .bind(crate::routes::workspaces::pad_position(1))
  .execute(db)
  .await
  .unwrap();
  ws
}

/// A page whose base holds one paragraph plus one image block pointing at
/// [file_id] — the image is what makes the transfer pause in phase 3.
async fn seed_page_with_image(db: &PgPool, ws: Uuid, user: Uuid, file_id: Uuid) -> (Uuid, Uuid) {
  let doc = Uuid::new_v4();
  sqlx::query(
    "INSERT INTO documents(id,workspace_id,root_block_id,created_by) VALUES($1,$2,'root',$3)",
  )
  .bind(doc)
  .bind(ws)
  .bind(user)
  .execute(db)
  .await
  .unwrap();
  let payload: mica_markdown::DocumentSnapshotPayload = serde_json::from_value(json!({
    "schema_version": 1,
    "root_block_id": "root",
    "blocks": [
      {"id": "root", "type": "page", "text": "", "children": ["p1", "img"]},
      {"id": "p1", "type": "paragraph", "text": "before the move", "data": {}},
      {"id": "img", "type": "image", "text": "",
       "data": {"file_id": file_id.to_string(), "name": "pixel.png"}},
    ],
  }))
  .unwrap();
  let mut tx = db.begin().await.unwrap();
  mica_app_core::sync::seed_base_tx(&mut tx, doc, payload)
    .await
    .unwrap();
  tx.commit().await.unwrap();

  let view = Uuid::new_v4();
  sqlx::query(
    "INSERT INTO views(id,workspace_id,object_id,object_type,name,position,created_by) \
       VALUES($1,$2,$3,'document'::object_type,'Page','0',$4)",
  )
  .bind(view)
  .bind(ws)
  .bind(doc)
  .bind(user)
  .execute(db)
  .await
  .unwrap();
  (view, doc)
}

/// Register the uploaded image the page references; the object key must sit
/// under the source workspace's prefix for the copy plan to accept it.
async fn seed_file(db: &PgPool, ws: Uuid, user: Uuid) -> Uuid {
  let file = Uuid::new_v4();
  sqlx::query(
    "INSERT INTO files(id,workspace_id,uploaded_by,object_key,original_name,mime_type,byte_size) \
       VALUES($1,$2,$3,$4,'pixel.png','image/png',$5)",
  )
  .bind(file)
  .bind(ws)
  .bind(user)
  .bind(format!("workspaces/{ws}/{file}.png"))
  .bind(BLOB.len() as i64)
  .execute(db)
  .await
  .unwrap();
  file
}

fn test_state(db: &PgPool, endpoint: &str) -> AppState {
  let mut config = mica_infra::AppConfig::from_env().unwrap();
  config.jwt_secret = JWT_SECRET.into();
  config.environment = mica_infra::Environment::Test;
  let mut state = AppState::new(config, db.clone(), Arc::new(mica_infra::LogMailer));
  state.storage = Some(Arc::new(mica_infra::S3Config {
    endpoint: endpoint.to_string(),
    internal_endpoint: None,
    region: "us-east-1".to_string(),
    bucket: "guard-test".to_string(),
    access_key: "x".to_string(),
    secret_key: "x".to_string(),
    presign_ttl_seconds: 900,
    max_upload_bytes: 0,
    public_base_url: None,
    force_path_style: true,
  }));
  state
}

fn bearer_headers(user: Uuid) -> HeaderMap {
  #[derive(serde::Serialize)]
  struct TestClaims {
    sub: String,
    exp: usize,
  }
  let claims = TestClaims {
    sub: user.to_string(),
    exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
  };
  let token = jsonwebtoken::encode(
    &jsonwebtoken::Header::default(),
    &claims,
    &jsonwebtoken::EncodingKey::from_secret(JWT_SECRET.as_bytes()),
  )
  .unwrap();
  let mut headers = HeaderMap::new();
  headers.insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
  headers
}

/// Read a request head off the socket; returns (method, unread body bytes).
fn read_head(socket: &mut std::net::TcpStream) -> Option<(String, usize)> {
  use std::io::Read as _;
  let mut buf: Vec<u8> = Vec::new();
  let mut chunk = [0u8; 1024];
  loop {
    if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
      let head = String::from_utf8_lossy(&buf[..end]).to_string();
      let already = buf.len() - (end + 4);
      let method = head
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
      let length = head
        .lines()
        .find_map(|line| {
          let (key, value) = line.split_once(':')?;
          key
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
        })
        .unwrap_or(0);
      return Some((method, length.saturating_sub(already)));
    }
    let read = socket.read(&mut chunk).ok()?;
    if read == 0 {
      return None;
    }
    buf.extend_from_slice(&chunk[..read]);
  }
}

/// A loopback object store: it serves the blob GET and accepts the PUT of the
/// destination copy. The GET is HELD until the test releases it, which is the
/// deterministic window between the transfer's payload scan and its commit.
///
/// A plain blocked thread, not a tokio listener: this crate does not enable
/// tokio's `net`/`io-util` features, and the wait must stay off the runtime.
///
/// Returns (endpoint, get_seen, release).
fn spawn_blob_store() -> (String, oneshot::Receiver<()>, oneshot::Sender<()>) {
  use std::io::{Read as _, Write as _};
  let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fake store");
  let endpoint = format!("http://{}", listener.local_addr().unwrap());
  let (seen_tx, seen_rx) = oneshot::channel();
  let (release_tx, release_rx) = oneshot::channel();
  std::thread::spawn(move || {
    let mut seen = Some(seen_tx);
    // `blocking_recv` consumes the receiver, so it lives in an Option: the
    // first held GET waits, every later one is served immediately.
    let mut release = Some(release_rx);
    // One request at a time: the transfer copies one blob, and its PUT can only
    // start after the held GET has been answered.
    for stream in listener.incoming() {
      let Ok(mut socket) = stream else {
        continue;
      };
      let Some((method, body_left)) = read_head(&mut socket) else {
        continue;
      };
      if body_left > 0 {
        let mut body = vec![0u8; body_left];
        let _ = socket.read_exact(&mut body);
      }
      if method == "GET" {
        if let Some(tx) = seen.take() {
          let _ = tx.send(());
        }
        if let Some(rx) = release.take() {
          let _ = rx.blocking_recv();
        }
      }
      let payload: &[u8] = if method == "GET" { BLOB } else { b"" };
      let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        payload.len()
      );
      let _ = socket.write_all(head.as_bytes());
      let _ = socket.write_all(payload);
    }
  });
  (endpoint, seen_rx, release_tx)
}

async fn live_views(db: &PgPool, ws: Uuid) -> Vec<(Uuid, String)> {
  sqlx::query_as::<_, (Uuid, String)>(
    "SELECT id, name FROM views WHERE workspace_id = $1 AND is_deleted = false ORDER BY name",
  )
  .bind(ws)
  .fetch_all(db)
  .await
  .unwrap()
}

async fn is_deleted(db: &PgPool, view: Uuid) -> bool {
  sqlx::query_scalar::<_, bool>("SELECT is_deleted FROM views WHERE id = $1")
    .bind(view)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn document_text(db: &PgPool, doc: Uuid) -> String {
  let payload = mica_app_core::store::current_payload(db, doc)
    .await
    .unwrap()
    .expect("document must have a payload");
  payload
    .blocks
    .iter()
    .map(|b| b.text.as_str())
    .collect::<Vec<_>>()
    .join("\n")
}

/// A content write that lands while the blob copy is in flight must refuse the
/// move with a 409: the destination tree does not land, the source is not
/// soft-deleted, and the new text stays exactly where its author wrote it.
#[tokio::test]
async fn a_content_write_during_the_blob_copy_refuses_the_move() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let src = seed_workspace(&db, user).await;
  let dest = seed_workspace(&db, user).await;
  let file = seed_file(&db, src, user).await;
  let (page, doc) = seed_page_with_image(&db, src, user, file).await;

  let seq_before: i64 = sqlx::query_scalar("SELECT current_seq FROM documents WHERE id = $1")
    .bind(doc)
    .fetch_one(&db)
    .await
    .unwrap();
  let (endpoint, get_seen, release) = spawn_blob_store();
  let state = test_state(&db, &endpoint);

  let request: TransferRequest = serde_json::from_value(json!({
    "dest_workspace_id": dest,
    "remove_source": true,
  }))
  .unwrap();
  let headers = bearer_headers(user);
  let moving = tokio::spawn(async move {
    transfer_view(State(state), headers, Path((src, page)), Json(request)).await
  });

  // Phase 2 has read the payload and phase 3 is fetching the image: the
  // commit cannot have happened yet.
  get_seen
    .await
    .expect("the transfer must reach the blob GET");

  let written = update_document_markdown(
    State(test_state(&db, &endpoint)),
    bearer_headers(user),
    Path((src, doc)),
    Json(UpdateMarkdownRequest {
      mode: MarkdownUpdateMode::Append,
      markdown: "written while the move was copying blobs".to_string(),
      anchor: None,
      find: None,
      replace: None,
      replacement_format: "markdown".to_string(),
      unique: false,
      expected_seq: None,
    }),
  )
  .await
  .expect("the REST write must land inside the window");
  let written = serde_json::to_value(&written.0).unwrap();
  assert_eq!(
    written["document"]["current_seq"].as_i64(),
    Some(seq_before + 1),
    "the write advanced the document's content version"
  );

  release.send(()).expect("the held GET is still waiting");

  let result = moving.await.expect("the transfer task must not panic");
  let error = result.expect_err("a move racing a content write must be refused");
  assert!(
    matches!(error, ApiError::Conflict(_)),
    "expected 409 for a moved-on document, got {error:?}"
  );

  assert!(
    live_views(&db, dest).await.is_empty(),
    "a refused move must not leave a destination tree behind"
  );
  assert!(
    !is_deleted(&db, page).await,
    "a refused move must not soft-delete the source"
  );
  let text = document_text(&db, doc).await;
  assert!(
    text.contains("before the move") && text.contains("written while the move was copying blobs"),
    "the source keeps its content, including the concurrent write: {text}"
  );
}
