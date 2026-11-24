//! Comprehensive unit & persistence tests for `storage.rs` (`VaultStore`) and
//! `net.rs` (`CloudConnector`).

#[allow(dead_code)]
#[path = "../src/storage.rs"]
mod storage;

#[allow(dead_code)]
#[path = "../src/net.rs"]
mod net;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use chrono::Utc;
use mica_api_types::{AuthResponse, User, View};
use mica_client::WorldOrigin;
use mica_core::{Block, MicaDoc};
use mica_sync::{SyncEffect, SyncEvent, SyncPhase};
use uuid::Uuid;

use crate::net::{CloudConfig, CloudConnector};
use crate::storage::{DEFAULT_ORIGIN, VaultStore};

fn make_view(
  ws: Uuid,
  doc: Uuid,
  name: &str,
  object_type: &str,
  parent: Option<Uuid>,
  pos: &str,
) -> View {
  let now = Utc::now();
  View {
    id: Uuid::new_v4(),
    workspace_id: ws,
    parent_view_id: parent,
    object_id: doc,
    object_type: object_type.to_string(),
    name: name.to_string(),
    icon: None,
    position: pos.to_string(),
    is_deleted: false,
    created_by: Uuid::new_v4(),
    created_at: now,
    updated_at: now,
    state_bytes: None,
  }
}

fn temp_vault_path(tag: &str) -> std::path::PathBuf {
  std::env::temp_dir()
    .join(format!("mica_rust_test_{tag}_{}", Uuid::new_v4()))
    .join("vault.sqlite")
}

fn sample_update(text: &str) -> Vec<u8> {
  let doc = MicaDoc::from_blocks(
    "root",
    &[
      Block::new("root", "page").with_children(vec!["b1".to_string()]),
      Block::new("b1", "paragraph").with_text(text),
    ],
  );
  doc.encode_state()
}

/// Spawn a single-request local HTTP server on an ephemeral localhost port.
fn spawn_one_shot_http_server(
  status_line: &'static str,
  body: String,
) -> (String, thread::JoinHandle<String>) {
  let listener = TcpListener::bind("127.0.0.1:0").expect("bind localhost ephemeral port");
  let addr = listener.local_addr().expect("local addr");
  let handle = thread::spawn(move || {
    let (mut stream, _) = listener.accept().expect("accept client");
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut header_end = None;
    let mut content_len = 0usize;

    loop {
      let n = stream.read(&mut chunk).unwrap_or(0);
      if n == 0 {
        break;
      }
      raw.extend_from_slice(&chunk[..n]);
      if header_end.is_none()
        && let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n")
      {
        let end = pos + 4;
        header_end = Some(end);
        let headers_str = String::from_utf8_lossy(&raw[..end]);
        for line in headers_str.lines() {
          if let Some((k, v)) = line.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
          {
            content_len = v.trim().parse::<usize>().unwrap_or(0);
          }
        }
      }
      if let Some(end) = header_end
        && raw.len() >= end + content_len
      {
        break;
      }
    }

    let request = String::from_utf8_lossy(&raw).to_string();
    let response = format!(
      "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
      body.len(),
      body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    request
  });
  (format!("http://{addr}"), handle)
}

#[test]
fn sqlite_vault_persists_views_markdown_boards_versions_and_identity_across_restart() {
  let db_path = temp_vault_path("restart");
  let db_str = db_path.to_str().unwrap().to_string();

  let ws = Uuid::new_v4();
  let folder = make_view(ws, Uuid::new_v4(), "Projects", "folder", None, "0001");
  let doc_id = Uuid::new_v4();
  let page = make_view(ws, doc_id, "Roadmap", "page", Some(folder.id), "0002");

  // Use >12 blocks to verify DFS order from root is preserved (not lexicographic orphan order).
  let mut md_lines = vec!["# Mica Roadmap".to_string()];
  for i in 1..=14 {
    md_lines.push(format!("Paragraph block number {i:02}."));
  }
  let markdown = md_lines.join("\n\n");
  let board_json = r#"{"schema":1,"nodes":[{"id":"n1","text":"架构设计 Architecture"}],"edges":[]}"#;

  let first_client_id;
  let expected_version_count;
  {
    let vault = VaultStore::open_at(&db_str).expect("open sqlite vault");
    assert_eq!(vault.origin(), DEFAULT_ORIGIN);
    first_client_id = vault.client_id();
    assert!(first_client_id > 0);

    vault.save_view(DEFAULT_ORIGIN, &folder).unwrap();
    vault.save_view(DEFAULT_ORIGIN, &page).unwrap();
    vault.save_markdown(DEFAULT_ORIGIN, doc_id, &markdown).unwrap();
    vault
      .save_board_json(DEFAULT_ORIGIN, "board-alpha", board_json)
      .unwrap();
    vault
      .create_version_snapshot(DEFAULT_ORIGIN, doc_id, Some("Release 1.0"))
      .unwrap();
    vault
      .create_version_snapshot(DEFAULT_ORIGIN, doc_id, None)
      .unwrap();
    expected_version_count = vault.list_version_count(DEFAULT_ORIGIN, doc_id);
    assert!(expected_version_count >= 3);
  }

  // Reopen the SQLite file from scratch to simulate full process restart.
  {
    let reopened = VaultStore::open_at(&db_str).expect("reopen sqlite vault");
    assert_eq!(reopened.client_id(), first_client_id);

    let views = reopened.list_views(DEFAULT_ORIGIN);
    assert_eq!(views.len(), 2);
    assert_eq!(views[0].id, folder.id);
    assert_eq!(views[0].object_type, "folder");
    assert_eq!(views[1].id, page.id);
    assert_eq!(views[1].parent_view_id, Some(folder.id));
    assert_eq!(views[1].object_id, doc_id);

    let loaded_md = reopened
      .load_markdown(DEFAULT_ORIGIN, doc_id)
      .expect("markdown survives restart");
    assert!(loaded_md.starts_with("# Mica Roadmap"));
    let pos_02 = loaded_md.find("Paragraph block number 02.").unwrap();
    let pos_10 = loaded_md.find("Paragraph block number 10.").unwrap();
    let pos_14 = loaded_md.find("Paragraph block number 14.").unwrap();
    assert!(pos_02 < pos_10 && pos_10 < pos_14);

    let loaded_board = reopened
      .load_board_json(DEFAULT_ORIGIN, "board-alpha")
      .expect("board json survives restart");
    assert_eq!(loaded_board, board_json);

    assert_eq!(
      reopened.list_version_count(DEFAULT_ORIGIN, doc_id),
      expected_version_count
    );

    reopened.purge_view(DEFAULT_ORIGIN, page.id).unwrap();
    assert_eq!(reopened.list_views(DEFAULT_ORIGIN).len(), 1);
  }

  if let Some(dir) = db_path.parent() {
    let _ = std::fs::remove_dir_all(dir);
  }
}

#[test]
fn vault_isolates_origins_and_validates_inputs() {
  let vault = VaultStore::in_memory();
  let shared_doc = Uuid::new_v4();
  let cloud_origin = "https://cloud.mica.example";

  vault
    .save_markdown(DEFAULT_ORIGIN, shared_doc, "Local body")
    .unwrap();
  vault
    .save_markdown(cloud_origin, shared_doc, "Cloud body")
    .unwrap();
  assert_eq!(
    vault.load_markdown(DEFAULT_ORIGIN, shared_doc).as_deref(),
    Some("Local body")
  );
  assert_eq!(
    vault.load_markdown(cloud_origin, shared_doc).as_deref(),
    Some("Cloud body")
  );

  assert!(vault.save_board_json(DEFAULT_ORIGIN, "  ", "{}").is_err());
  assert!(vault.load_board_json(DEFAULT_ORIGIN, "missing").is_none());
  assert!(vault.load_markdown(DEFAULT_ORIGIN, Uuid::new_v4()).is_none());
  assert!(VaultStore::open_at("   ").is_err());

  let default_vault = VaultStore::open_default();
  assert_eq!(default_vault.origin(), DEFAULT_ORIGIN);
}

#[test]
fn vault_sqlite_sync_storage_integrates_with_cloud_connector_sync_engine() {
  let vault = VaultStore::in_memory();
  let doc_id = Uuid::new_v4().to_string();

  // Exercise both explicit sqlite helper and boxed cross-platform factory.
  let _direct_sqlite = vault.sqlite_sync_storage("https://sync.example", &doc_id);
  let storage = vault.create_sync_storage("https://sync.example", &doc_id);

  let conn = CloudConnector::new(CloudConfig {
    server_url: "https://sync.example/".to_string(),
    access_token: Some("tok-abc".to_string()),
    email: Some("sync@mica.test".to_string()),
  });

  let mut engine = conn.create_doc_sync_with_storage(&doc_id, vault.client_id(), "", storage);
  let connect_effects = engine.handle_event(SyncEvent::Connect);
  assert!(
    connect_effects
      .iter()
      .any(|e| matches!(e, SyncEffect::OpenSocket { .. }))
  );

  let push_effects = CloudConnector::push_local_update(&mut engine, sample_update("Offline edit"));
  assert!(!engine.outbox_empty());
  assert!(
    push_effects.is_empty()
      || push_effects
        .iter()
        .any(|e| matches!(e, SyncEffect::SyncPhaseChanged(_)))
  );
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
  use std::sync::{Arc, Condvar, Mutex};
  use std::task::{Context, Poll, Wake, Waker};
  struct CondWaker {
    ready: Mutex<bool>,
    cv: Condvar,
  }
  impl Wake for CondWaker {
    fn wake(self: Arc<Self>) {
      *self.ready.lock().unwrap() = true;
      self.cv.notify_all();
    }
  }
  let wake = Arc::new(CondWaker {
    ready: Mutex::new(false),
    cv: Condvar::new(),
  });
  let waker = Waker::from(wake.clone());
  let mut cx = Context::from_waker(&waker);
  let mut fut = std::pin::pin!(fut);
  loop {
    match fut.as_mut().poll(&mut cx) {
      Poll::Ready(out) => return out,
      Poll::Pending => {
        let mut g = wake.ready.lock().unwrap();
        while !*g {
          g = wake.cv.wait(g).unwrap();
        }
        *g = false;
      }
    }
  }
}

#[test]
fn cloud_connector_health_and_login_against_local_http_server() {
  block_on(async {
  // 1. Health check success
  let (health_url, health_handle) = spawn_one_shot_http_server(
    "200 OK",
    r#"{"status":"ok","service":"mica-api-server","version":"0.14.11"}"#.to_string(),
  );
  let connector = CloudConnector::new(CloudConfig {
    server_url: format!("{health_url}/"),
    access_token: None,
    email: None,
  });
  assert_eq!(connector.server_url(), health_url);
  assert_eq!(
    connector.endpoints().health(),
    format!("{health_url}/api/health")
  );
  let summary = connector.check_health().await.expect("health check should succeed");
  assert_eq!(summary, "ok (v0.14.11)");
  let req = health_handle.join().unwrap();
  assert!(req.starts_with("GET /api/health "));

  // 2. Login success + ClaimTracker transition + Logout reset
  let auth_payload = AuthResponse {
    access_token: "jwt-secret-token".to_string(),
    token_type: "Bearer".to_string(),
    expires_at: Utc::now(),
    refresh_token: "ref-token".to_string(),
    user: User {
      id: Uuid::new_v4(),
      email: "alice@mica.dev".to_string(),
      display_name: "Alice".to_string(),
      created_at: Utc::now(),
      avatar_version: None,
    },
  };
  let (login_url, login_handle) = spawn_one_shot_http_server(
    "200 OK",
    serde_json::to_string(&auth_payload).unwrap(),
  );
  let mut auth_conn = CloudConnector::new(CloudConfig {
    server_url: login_url.clone(),
    access_token: None,
    email: None,
  });
  let before_claim = auth_conn.claims().capture();
  assert_eq!(before_claim.origin, WorldOrigin::Local);
  assert!(!auth_conn.is_authenticated());
  assert_eq!(auth_conn.account_label(), "Local Vault");
  assert_eq!(auth_conn.sync_phase_label(), "Local Only");

  let signed_in_email = auth_conn
    .login("alice@mica.dev", "super-secret")
    .await
    .expect("login should succeed");
  assert_eq!(signed_in_email, "alice@mica.dev");
  assert!(auth_conn.is_authenticated());
  assert_eq!(auth_conn.account_label(), "alice@mica.dev");
  assert_eq!(auth_conn.sync_phase(), SyncPhase::Offline);
  assert_eq!(auth_conn.sync_phase_label(), "Offline");
  assert_eq!(
    auth_conn.config().access_token.as_deref(),
    Some("jwt-secret-token")
  );
  assert!(!auth_conn.claims().matches_account(&before_claim));
  assert_eq!(
    auth_conn.claims().capture().origin,
    WorldOrigin::Cloud {
      server_url: login_url.clone(),
    }
  );

  let login_req = login_handle.join().unwrap();
  assert!(login_req.starts_with("POST /api/auth/login "));
  assert!(login_req.contains("alice@mica.dev"));

  // Changing server URL clears auth so token never leaks to another server.
  auth_conn.set_server_url("https://other.mica.dev/");
  assert_eq!(auth_conn.server_url(), "https://other.mica.dev");
  assert!(!auth_conn.is_authenticated());
  assert_eq!(auth_conn.claims().capture().origin, WorldOrigin::Local);
  });
}

#[test]
fn cloud_connector_validation_serde_and_memory_sync_engine() {
  block_on(async {
  let cfg = CloudConfig {
    server_url: "https://mica.example".to_string(),
    access_token: Some("tok".to_string()),
    email: Some("bob@mica.example".to_string()),
  };
  let round_trip: CloudConfig =
    serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
  assert_eq!(round_trip, cfg);

  let mut empty_conn = CloudConnector::new(CloudConfig::default());
  assert!(empty_conn.check_health().await.is_err());
  assert!(empty_conn.login("a@b.c", "pw").await.is_err());

  empty_conn.set_server_url("http://127.0.0.1:1");
  assert!(empty_conn.login("", "pw").await.is_err());
  assert!(empty_conn.login("a@b.c", "").await.is_err());

  let mut engine = empty_conn.create_doc_sync("doc-mem", 77, "Bob");
  let effects = CloudConnector::push_local_update(&mut engine, sample_update("Memory diff"));
  assert!(!effects.is_empty());
  assert!(!engine.outbox_empty());
  });
}

fn read_full_http_request(stream: &mut std::net::TcpStream) -> String {
  let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
  let mut data = Vec::new();
  let mut buf = [0u8; 4096];
  let mut header_end: Option<usize> = None;
  let mut content_len: usize = 0;
  let mut is_chunked = false;
  loop {
    let n = match stream.read(&mut buf) {
      Ok(0) | Err(_) => break,
      Ok(n) => n,
    };
    data.extend_from_slice(&buf[..n]);
    if header_end.is_none()
      && let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n")
    {
      let end = pos + 4;
      header_end = Some(end);
      let head = String::from_utf8_lossy(&data[..end]);
      for line in head.lines() {
        if let Some((k, v)) = line.split_once(':') {
          if k.trim().eq_ignore_ascii_case("content-length") {
            content_len = v.trim().parse::<usize>().unwrap_or(0);
          } else if k.trim().eq_ignore_ascii_case("transfer-encoding")
            && v.to_ascii_lowercase().contains("chunked")
          {
            is_chunked = true;
          }
        }
      }
    }
    if let Some(end) = header_end {
      if is_chunked {
        if data[end..].windows(5).any(|w| w == b"0\r\n\r\n") {
          break;
        }
      } else if data.len() >= end + content_len {
        break;
      }
    }
  }
  String::from_utf8_lossy(&data).to_string()
}

fn spawn_multi_step_http_server(
  responses: Vec<(&'static str, String)>,
) -> (String, thread::JoinHandle<Vec<String>>) {
  let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
  let addr = listener.local_addr().unwrap();
  let handle = thread::spawn(move || {
    let mut requests = Vec::with_capacity(responses.len());
    for (status, body) in responses {
      let (mut stream, _) = listener.accept().expect("accept client");
      requests.push(read_full_http_request(&mut stream));
      let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
      );
      let _ = stream.write_all(reply.as_bytes());
      let _ = stream.flush();
      let _ = stream.shutdown(std::net::Shutdown::Both);
    }
    requests
  });
  (format!("http://{addr}"), handle)
}

#[test]
fn cloud_workspaces_views_documents_and_vault_mirror_end_to_end() {
  let now = Utc::now();
  let owner_id = Uuid::new_v4();
  let ws1_id = Uuid::new_v4();
  let ws2_id = Uuid::new_v4();
  let folder_id = Uuid::new_v4();
  let doc_id = Uuid::new_v4();
  let page_view_id = Uuid::new_v4();
  let ws2_doc_id = Uuid::new_v4();
  let ws2_view_id = Uuid::new_v4();

  let login_resp = serde_json::json!({
    "access_token": "jwt-access-1",
    "token_type": "Bearer",
    "expires_at": now,
    "refresh_token": "jwt-refresh-1",
    "user": {
      "id": owner_id,
      "email": "will@cloudcele.com",
      "display_name": "Will",
      "created_at": now
    }
  });
  let workspaces_resp = serde_json::json!({
    "workspaces": [
      {
        "id": ws1_id,
        "name": "云端研发工作区",
        "owner_id": owner_id,
        "role": "owner",
        "created_at": now,
        "updated_at": now,
        "page_count": 3
      },
      {
        "id": ws2_id,
        "name": "云端个人笔记",
        "owner_id": owner_id,
        "role": "editor",
        "created_at": now,
        "updated_at": now,
        "page_count": 1
      }
    ]
  });
  // Intentionally return child page BEFORE parent folder + server's `object_type: "document"`.
  let ws1_views_resp = serde_json::json!({
    "views": [
      {
        "id": page_view_id,
        "workspace_id": ws1_id,
        "parent_view_id": folder_id,
        "object_id": doc_id,
        "object_type": "document",
        "name": "架构总览",
        "position": "0000000020",
        "is_deleted": false,
        "created_by": owner_id,
        "created_at": now,
        "updated_at": now
      },
      {
        "id": folder_id,
        "workspace_id": ws1_id,
        "parent_view_id": null,
        "object_id": Uuid::new_v4(),
        "object_type": "folder",
        "name": "核心文档",
        "position": "0000000010",
        "is_deleted": false,
        "created_by": owner_id,
        "created_at": now,
        "updated_at": now
      }
    ]
  });
  let ws2_views_resp = serde_json::json!({
    "views": [
      {
        "id": ws2_view_id,
        "workspace_id": ws2_id,
        "parent_view_id": null,
        "object_id": ws2_doc_id,
        "object_type": "document",
        "name": "灵感随记",
        "position": "0000000010",
        "is_deleted": false,
        "created_by": owner_id,
        "created_at": now,
        "updated_at": now
      }
    ]
  });
  let doc_md_resp = serde_json::json!({ "markdown": "# 架构总览\n\n来自 mica.cloudcele.com 的正文" });
  let patch_ok = serde_json::json!({ "ok": true });
  let refresh_resp = serde_json::json!({
    "access_token": "jwt-access-2",
    "token_type": "Bearer",
    "expires_at": now,
    "refresh_token": "jwt-refresh-2",
    "user": {
      "id": owner_id,
      "email": "will@cloudcele.com",
      "display_name": "Will",
      "created_at": now
    }
  });

  let (base_url, handle) = spawn_multi_step_http_server(vec![
    ("200 OK", login_resp.to_string()),
    ("200 OK", workspaces_resp.to_string()),
    ("200 OK", ws1_views_resp.to_string()),
    ("200 OK", ws2_views_resp.to_string()),
    ("200 OK", doc_md_resp.to_string()),
    ("200 OK", patch_ok.to_string()),
    ("200 OK", refresh_resp.to_string()),
  ]);

  let mut conn = CloudConnector::new(CloudConfig {
    server_url: base_url.clone(),
    access_token: None,
    email: None,
  });
  block_on(async {
    assert_eq!(conn.login("will@cloudcele.com", "pw").await.unwrap(), "will@cloudcele.com");
    assert_eq!(conn.access_token(), Some("jwt-access-1"));
    assert_eq!(conn.refresh_token(), Some("jwt-refresh-1"));

    let workspaces = conn.list_workspaces().await.unwrap();
    assert_eq!(workspaces.len(), 2);
    assert_eq!(workspaces[0].name, "云端研发工作区");
    assert_eq!(workspaces[0].page_count, 3);
    assert_eq!(workspaces[1].name, "云端个人笔记");

    let ws1_views = conn.list_views(ws1_id).await.unwrap();
    let ws2_views = conn.list_views(ws2_id).await.unwrap();
    assert_eq!(ws1_views.len(), 2);
    assert_eq!(ws2_views.len(), 1);

    let md = conn.fetch_document_markdown(ws1_id, doc_id).await.unwrap();
    assert!(md.contains("来自 mica.cloudcele.com 的正文"));

    conn
      .save_document_markdown(ws1_id, doc_id, "# 架构总览\n\n已更新")
      .await
      .unwrap();

    conn.refresh_session().await.unwrap();
    assert_eq!(conn.access_token(), Some("jwt-access-2"));
    assert_eq!(conn.refresh_token(), Some("jwt-refresh-2"));

    // Mirror into VaultStore and verify credential persistence + per-workspace isolation.
    let vault = VaultStore::in_memory();
    vault
      .save_server_credential_with_refresh(
        &base_url,
        "will@cloudcele.com",
        conn.access_token().unwrap(),
        conn.refresh_token().unwrap(),
      )
      .unwrap();
    let (saved_email, saved_tok, saved_rt) = vault.saved_server_credential_full(&base_url).unwrap();
    assert_eq!(saved_email, "will@cloudcele.com");
    assert_eq!(saved_tok, "jwt-access-2");
    assert_eq!(saved_rt, "jwt-refresh-2");

    let ws_pairs: Vec<(Uuid, String)> = workspaces.iter().map(|w| (w.id, w.name.clone())).collect();
    vault.replace_origin_workspaces(&base_url, &ws_pairs).unwrap();
    assert_eq!(vault.list_workspaces(&base_url).len(), 2);

    vault.replace_workspace_views(&base_url, ws1_id, &ws1_views).unwrap();
    vault.replace_workspace_views(&base_url, ws2_id, &ws2_views).unwrap();

    // Multi-workspace isolation: ws1 and ws2 under the same cloud origin never mix.
    let stored_ws1 = vault.list_workspace_views(&base_url, ws1_id);
    let stored_ws2 = vault.list_workspace_views(&base_url, ws2_id);
    assert_eq!(stored_ws1.len(), 2);
    assert_eq!(stored_ws2.len(), 1);
    assert_eq!(stored_ws2[0].name, "灵感随记");
    assert!(vault.list_views(DEFAULT_ORIGIN).is_empty());
  });

  let reqs = handle.join().unwrap();
  assert!(reqs[1].starts_with("GET /api/workspaces "));
  assert!(reqs[1].to_ascii_lowercase().contains("authorization: bearer jwt-access-1"));
  assert!(reqs[4].starts_with(&format!("GET /api/workspaces/{ws1_id}/documents/{doc_id}/markdown ")));
  assert!(reqs[5].starts_with(&format!("PATCH /api/workspaces/{ws1_id}/documents/{doc_id}/markdown ")));
  assert!(
    reqs[5].contains("\"replace_all\""),
    "unexpected PATCH request: {}",
    reqs[5]
  );
  assert!(reqs[6].starts_with("POST /api/auth/refresh "));
}

#[test]
fn cloud_crud_mutations_and_unauthorized_detection() {
  assert!(CloudConnector::is_unauthorized_error("GET /api/workspaces failed: http status: 401"));
  assert!(CloudConnector::is_unauthorized_error("authentication required"));
  assert!(!CloudConnector::is_unauthorized_error("GET /api/workspaces failed: http status: 500"));

  let now = Utc::now();
  let owner_id = Uuid::new_v4();
  let ws_id = Uuid::new_v4();
  let folder_id = Uuid::new_v4();
  let page_view_id = Uuid::new_v4();
  let doc_id = Uuid::new_v4();

  let ws_json = serde_json::json!({
    "workspace": {
      "id": ws_id,
      "name": "新云端工作区",
      "owner_id": owner_id,
      "role": "owner",
      "created_at": now,
      "updated_at": now,
      "page_count": 0
    }
  });
  let folder_json = serde_json::json!({
    "view": {
      "id": folder_id,
      "workspace_id": ws_id,
      "parent_view_id": null,
      "object_id": Uuid::new_v4(),
      "object_type": "folder",
      "name": "目录 A",
      "position": "0000000010",
      "is_deleted": false,
      "created_by": owner_id,
      "created_at": now,
      "updated_at": now
    }
  });
  let doc_json = serde_json::json!({
    "view": {
      "id": page_view_id,
      "workspace_id": ws_id,
      "parent_view_id": folder_id,
      "object_id": doc_id,
      "object_type": "document",
      "name": "页面 1",
      "position": "0000000020",
      "is_deleted": false,
      "created_by": owner_id,
      "created_at": now,
      "updated_at": now
    }
  });
  let renamed_json = serde_json::json!({
    "view": {
      "id": page_view_id,
      "workspace_id": ws_id,
      "parent_view_id": folder_id,
      "object_id": doc_id,
      "object_type": "document",
      "name": "页面 1（改名）",
      "icon": "📄",
      "position": "0000000020",
      "is_deleted": false,
      "created_by": owner_id,
      "created_at": now,
      "updated_at": now
    }
  });
  let trash_json = serde_json::json!({ "views": [] });
  let usage_json = serde_json::json!({ "bytes_used": 2048, "quota_bytes": 1048576 });
  let ok_json = serde_json::json!({ "ok": true }).to_string();

  let (base_url, handle) = spawn_multi_step_http_server(vec![
    ("200 OK", ws_json.to_string()),
    ("200 OK", ok_json.clone()),
    ("200 OK", folder_json.to_string()),
    ("200 OK", doc_json.to_string()),
    ("200 OK", renamed_json.to_string()),
    ("200 OK", ok_json.clone()),
    ("200 OK", ok_json.clone()),
    ("200 OK", trash_json.to_string()),
    ("200 OK", ok_json.clone()),
    ("200 OK", ok_json.clone()),
    ("200 OK", usage_json.to_string()),
    ("200 OK", ok_json.clone()),
    ("200 OK", ok_json.clone()),
    ("200 OK", ok_json.clone()),
    ("200 OK", ok_json),
  ]);

  let conn = CloudConnector::new(CloudConfig {
    server_url: base_url,
    access_token: Some("tok-live".to_string()),
    email: Some("will@cloudcele.com".to_string()),
  })
  .with_refresh_token(Some("rt-live".to_string()));
  assert_eq!(conn.refresh_token(), Some("rt-live"));

  block_on(async {
    let created_ws = conn.create_workspace("新云端工作区").await.unwrap();
    assert_eq!(created_ws.id, ws_id);
    conn.update_workspace(ws_id, "改名后的工作区").await.unwrap();

    let created_folder = conn.create_folder(ws_id, "目录 A", None).await.unwrap();
    assert_eq!(created_folder.id, folder_id);
    let created_doc = conn.create_document(ws_id, "页面 1", Some(folder_id)).await.unwrap();
    assert_eq!(created_doc.object_id, doc_id);

    let renamed = conn
      .update_view(ws_id, page_view_id, "页面 1（改名）", Some("📄"))
      .await
      .unwrap();
    assert_eq!(renamed.name, "页面 1（改名）");

    conn.reorder_views(ws_id, Some(folder_id), &[page_view_id]).await.unwrap();
    conn.delete_view(ws_id, page_view_id).await.unwrap();
    assert!(conn.list_trash(ws_id).await.unwrap().is_empty());
    conn.restore_view(ws_id, page_view_id).await.unwrap();
    conn.purge_view(ws_id, page_view_id).await.unwrap();
    assert_eq!(conn.workspace_usage(ws_id).await.unwrap(), (2048, 1048576));
    conn.revoke_invitation(ws_id, "inv-1").await.unwrap();
    conn.update_member_role(ws_id, "usr-1", "admin").await.unwrap();
    conn.remove_member(ws_id, "usr-1").await.unwrap();
    conn.delete_workspace(ws_id).await.unwrap();
  });

  let reqs = handle.join().unwrap();
  assert!(reqs[0].starts_with("POST /api/workspaces "));
  assert!(reqs[1].starts_with(&format!("PATCH /api/workspaces/{ws_id} ")));
  assert!(reqs[2].starts_with(&format!("POST /api/workspaces/{ws_id}/folders ")));
  assert!(reqs[3].starts_with(&format!("POST /api/workspaces/{ws_id}/documents ")));
  assert!(reqs[4].starts_with(&format!("PATCH /api/workspaces/{ws_id}/views/{page_view_id} ")));
  assert!(reqs[5].starts_with(&format!("POST /api/workspaces/{ws_id}/views/reorder ")));
  assert!(reqs[5].contains("\"ordered_view_ids\""));
  assert!(reqs[6].starts_with(&format!("DELETE /api/workspaces/{ws_id}/views/{page_view_id} ")));
  assert!(reqs[7].starts_with(&format!("GET /api/workspaces/{ws_id}/trash ")));
  assert!(reqs[8].starts_with(&format!("POST /api/workspaces/{ws_id}/views/{page_view_id}/restore ")));
  assert!(reqs[9].starts_with(&format!("DELETE /api/workspaces/{ws_id}/trash/{page_view_id} ")));
  assert!(reqs[10].starts_with(&format!("GET /api/workspaces/{ws_id}/usage ")));
  assert!(reqs[11].starts_with(&format!("DELETE /api/workspaces/{ws_id}/invitations/inv-1 ")));
  assert!(reqs[12].starts_with(&format!("PATCH /api/workspaces/{ws_id}/members/usr-1 ")));
  assert!(reqs[13].starts_with(&format!("DELETE /api/workspaces/{ws_id}/members/usr-1 ")));
  assert!(reqs[14].starts_with(&format!("DELETE /api/workspaces/{ws_id} ")));
}

#[test]
fn usage_stats_vault_and_cloud_connector_endpoints() {
  // 1. Local VaultStore tracking + overview + activity folding.
  let vault = VaultStore::in_memory();
  let ws_id = Uuid::new_v4();
  let doc_id = Uuid::new_v4();
  vault
    .save_workspace_name(DEFAULT_ORIGIN, ws_id, "技术笔记")
    .unwrap();
  let view = View {
    id: doc_id,
    workspace_id: ws_id,
    parent_view_id: None,
    object_id: doc_id,
    object_type: "page".to_string(),
    name: "首页".to_string(),
    icon: None,
    position: "0000000010".to_string(),
    is_deleted: false,
    created_by: Uuid::nil(),
    created_at: Utc::now(),
    updated_at: Utc::now(),
    state_bytes: None,
  };
  vault.save_view(DEFAULT_ORIGIN, &view).unwrap();
  vault
    .save_markdown(DEFAULT_ORIGIN, doc_id, "你好 world")
    .unwrap();
  vault.record_doc_created(
    DEFAULT_ORIGIN,
    ws_id,
    doc_id,
    mica_core::ActivitySource::Manual,
    "你好 world",
  );
  vault
    .save_markdown_tracked(
      DEFAULT_ORIGIN,
      ws_id,
      doc_id,
      "你好 world Rust 统计",
      mica_core::ActivitySource::Manual,
    )
    .unwrap();

  let overview = vault.usage_overview(DEFAULT_ORIGIN).unwrap();
  assert_eq!(overview.summary.workspace_count, 1);
  assert_eq!(overview.summary.live_page_count, 1);
  assert_eq!(overview.summary.total_words, 6);
  let act = vault
    .usage_activity(DEFAULT_ORIGIN, "7d", 480, Some(&ws_id.to_string()), false)
    .unwrap();
  assert_eq!(act.totals.pages_created, 1);
  assert_eq!(act.totals.pages_edited, 1);
  assert_eq!(act.totals.words_added, 6);

  // 2. CloudConnector GET /api/stats/overview + GET /api/stats/activity.
  let ov_json = serde_json::to_string(&overview).unwrap();
  let act_json = serde_json::to_string(&act).unwrap();
  let (url, handle) = spawn_multi_step_http_server(vec![
    ("200 OK", ov_json),
    ("200 OK", act_json),
  ]);
  let conn = CloudConnector::new(CloudConfig {
    server_url: url,
    access_token: Some("tok-stats".to_string()),
    email: Some("stats@mica.test".to_string()),
  });
  let (remote_ov, remote_act) = block_on(async {
    let ov = conn.get_usage_overview().await.unwrap();
    let act = conn
      .get_usage_activity("7d", 480, Some(&ws_id.to_string()), false)
      .await
      .unwrap();
    (ov, act)
  });
  assert_eq!(remote_ov.summary.total_words, 6);
  assert_eq!(remote_act.totals.pages_created, 1);
  let reqs = handle.join().unwrap();
  assert!(reqs[0].starts_with("GET /api/stats/overview "));
  assert!(reqs[1].starts_with(&format!(
    "GET /api/stats/activity?range=7d&tz_offset=480&include_imports=false&workspace_id={ws_id} "
  )));
}
