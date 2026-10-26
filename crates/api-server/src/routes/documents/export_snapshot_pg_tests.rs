// Postgres-backed reproduction of gh#155: the tree ZIP export reads every page
// twice — once to build the asset inventory and decide which pages have a body,
// once to render. Those are two separate reads of a live database, and nothing
// stops a page from gaining an image in between: the render then saw an image
// the inventory had never heard of and `continue`d past it, so the archive
// shipped a page whose Markdown pointed at an asset the ZIP did not contain.
//
// The window is entered through the export's own test seam — inventory taken,
// nothing rendered — and the write in between is the real REST markdown route.
//
//   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//   cargo test -p mica-api-server an_image_added_after_the_inventory_pass
use super::{MarkdownUpdateMode, UpdateMarkdownRequest, update_document_markdown};
use axum::{
  Json,
  extract::{Path, State},
  http::{HeaderMap, header::AUTHORIZATION},
};
use mica_app_core::AppState;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

const JWT_SECRET: &str = "export-snapshot-pg-test-secret";
const BLOB: &[u8] = b"archive-png-bytes";

async fn pool() -> Option<PgPool> {
  let url = mica_infra::test_database_url()?;
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
    .bind(format!("{user}@export-snapshot.test"))
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

/// A page with a body and no images — the state pass 1 sees.
async fn seed_page(db: &PgPool, ws: Uuid, user: Uuid, name: &str) -> (Uuid, Uuid) {
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
      {"id": "root", "type": "page", "text": "", "children": ["p1"]},
      {"id": "p1", "type": "paragraph", "text": "exported body", "data": {}},
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
       VALUES($1,$2,$3,'document'::object_type,$4,'0',$5)",
  )
  .bind(view)
  .bind(ws)
  .bind(doc)
  .bind(name)
  .bind(user)
  .execute(db)
  .await
  .unwrap();
  (view, doc)
}

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
    bucket: "export-test".to_string(),
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

/// Read a request head; returns (method, unread body bytes).
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

/// A loopback object store that answers every presigned GET with [BLOB].
///
/// A plain thread, not a tokio listener: this crate does not enable tokio's
/// `net`/`io-util` features.
fn spawn_blob_store() -> String {
  use std::io::{Read as _, Write as _};
  let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fake store");
  let endpoint = format!("http://{}", listener.local_addr().unwrap());
  std::thread::spawn(move || {
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
      let payload: &[u8] = if method == "GET" { BLOB } else { b"" };
      let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        payload.len()
      );
      let _ = socket.write_all(head.as_bytes());
      let _ = socket.write_all(payload);
    }
  });
  endpoint
}

/// An image inserted into a page AFTER the inventory pass must ship its bytes:
/// the export either includes the asset its Markdown references, or fails. It
/// must never quietly write the link and omit the file.
#[tokio::test]
async fn an_image_added_after_the_inventory_pass_still_ships_its_bytes() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;
  let ws = seed_workspace(&db, user).await;
  let (view, doc) = seed_page(&db, ws, user, "Page").await;
  let file = seed_file(&db, ws, user).await;
  let _ = view;

  let endpoint = spawn_blob_store();
  let state = test_state(&db, &endpoint);
  let write_state = test_state(&db, &endpoint);
  let write_headers = bearer_headers(user);

  let bytes = super::stream_tree_bytes_between_passes(&state, ws, None, || async move {
    // The window: pass 1 is done, nothing has been rendered. A real REST write
    // inserts an already-uploaded, never-inventoried image into the page.
    let _ = update_document_markdown(
      State(write_state),
      write_headers,
      Path((ws, doc)),
      Json(UpdateMarkdownRequest {
        mode: MarkdownUpdateMode::Append,
        markdown: format!("![pixel](/api/workspaces/{ws}/files/{file}/blob/pixel.png)"),
        anchor: None,
        find: None,
        replace: None,
        replacement_format: "markdown".to_string(),
        unique: false,
        expected_seq: None,
      }),
    )
    .await
    .expect("the write must land inside the window");
  })
  .await;

  let entries = mica_interchange::read_zip(&bytes).expect("the archive must be a readable zip");
  let asset = entries.iter().find(|e| e.name == "assets/pixel.png");
  let asset = asset.unwrap_or_else(|| {
    panic!(
      "the archive must ship the asset its page references; got {:?}",
      entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>()
    )
  });
  assert_eq!(
    asset.bytes, BLOB,
    "the asset entry must hold the uploaded bytes"
  );

  let page = entries
    .iter()
    .find(|e| e.name == "Page.md")
    .expect("the page must be exported");
  let markdown = String::from_utf8_lossy(&page.bytes);
  assert!(
    markdown.contains("assets/pixel.png"),
    "the page must point at the bundled asset: {markdown}"
  );
}
