//! Comprehensive unit & persistence tests for `storage.rs` (`VaultStore`) and
//! `net.rs` (`CloudConnector`).

#[path = "../src/storage.rs"]
mod storage;

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

#[test]
fn cloud_connector_health_and_login_against_local_http_server() {
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
  let summary = connector.check_health().expect("health check should succeed");
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
}

#[test]
fn cloud_connector_validation_serde_and_memory_sync_engine() {
  let cfg = CloudConfig {
    server_url: "https://mica.example".to_string(),
    access_token: Some("tok".to_string()),
    email: Some("bob@mica.example".to_string()),
  };
  let round_trip: CloudConfig =
    serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
  assert_eq!(round_trip, cfg);

  let mut empty_conn = CloudConnector::new(CloudConfig::default());
  assert!(empty_conn.check_health().is_err());
  assert!(empty_conn.login("a@b.c", "pw").is_err());

  empty_conn.set_server_url("http://127.0.0.1:1");
  assert!(empty_conn.login("", "pw").is_err());
  assert!(empty_conn.login("a@b.c", "").is_err());

  let mut engine = empty_conn.create_doc_sync("doc-mem", 77, "Bob");
  let effects = CloudConnector::push_local_update(&mut engine, sample_update("Memory diff"));
  assert!(!effects.is_empty());
  assert!(!engine.outbox_empty());
}
