//! Gold-file contract tests (#85) for the view/document DTOs defined in this
//! module. The golds live in `/contracts` at the repo root and are decoded by
//! the cli, mcp-server and Flutter test suites — a field rename fails all four
//! sides at once. Fixed ids/timestamps on purpose: the clients assert them by
//! value, so every uuid and instant here is part of the contract.

use chrono::TimeZone;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::routes::contract_gold::check;

fn at(hour: u32) -> chrono::DateTime<Utc> {
  Utc.with_ymd_and_hms(2026, 10, 24, hour, 0, 0).unwrap()
}

fn user() -> Uuid {
  Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap()
}
fn ws() -> Uuid {
  Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap()
}
fn folder_view() -> Uuid {
  Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap()
}
fn page_view() -> Uuid {
  Uuid::parse_str("44444444-4444-4444-8444-444444444444").unwrap()
}
fn page_view_2() -> Uuid {
  Uuid::parse_str("55555555-5555-4555-8555-555555555555").unwrap()
}
fn doc() -> Uuid {
  Uuid::parse_str("66666666-6666-4666-8666-666666666666").unwrap()
}
fn doc_2() -> Uuid {
  Uuid::parse_str("77777777-7777-4777-8777-777777777777").unwrap()
}

/// A views row exactly as the SQL returns one: every column the `View` struct
/// knows, `FromRow` field for field.
fn view(id: Uuid, object_id: Uuid, object_type: &str, name: &str) -> View {
  View {
    id,
    workspace_id: ws(),
    parent_view_id: None,
    object_id,
    object_type: object_type.to_string(),
    name: name.to_string(),
    icon: None,
    position: "0000000001".to_string(),
    is_deleted: false,
    created_by: user(),
    created_at: at(1),
    updated_at: at(2),
    state_bytes: None,
  }
}

/// The `documents` row a fresh page owns.
fn document_record(id: Uuid) -> mica_app_core::store::DocumentRecord {
  mica_app_core::store::DocumentRecord {
    id,
    workspace_id: ws(),
    root_block_id: "block_root".to_string(),
    current_seq: 3,
    created_by: user(),
    created_at: at(1),
    updated_at: at(2),
  }
}

/// A two-paragraph payload — the shape every snapshot-carrying response embeds.
/// The mcp server walks `payload.blocks` for the root's last child, so the
/// block ids are load-bearing for its `last_block_id` ack field.
fn payload_blocks() -> serde_json::Value {
  json!({
    "root_block_id": "block_root",
    "blocks": [
      { "id": "block_root", "type": "page", "text": "",
        "data": {}, "children": ["block_a", "block_b"] },
      { "id": "block_a", "type": "paragraph", "text": "First paragraph",
        "data": {}, "children": [] },
      { "id": "block_b", "type": "paragraph", "text": "Last paragraph",
        "data": {}, "children": [] }
    ]
  })
}

#[test]
fn gold_view_list() {
  // Three rows spanning the optional-field surface: a folder with an icon, a
  // page whose listing asked for `with_stats`, and a bare page (icon and
  // state_bytes must be ABSENT there, not null — `skip_serializing_if`).
  let folder = View {
    icon: Some("📗".to_string()),
    ..view(folder_view(), folder_view(), "folder", "Gold Folder")
  };
  let stats_page = View {
    parent_view_id: Some(folder_view()),
    state_bytes: Some(4096),
    ..view(page_view(), doc(), "document", "With Stats")
  };
  let plain_page = View {
    parent_view_id: Some(folder_view()),
    ..view(page_view_2(), doc_2(), "document", "Plain Page")
  };
  check(
    "view_list",
    &json!(ViewListResponse {
      views: vec![folder, stats_page, plain_page],
    }),
  );
}

#[test]
fn gold_batch_views() {
  check(
    "batch_views",
    &json!(BatchViewsResponse {
      affected: 3,
      skipped: vec![page_view_2()],
    }),
  );
}

#[test]
fn gold_document_create() {
  check(
    "document_create",
    &json!(DocumentCreateResponse {
      document: document_record(doc()),
      view: view(page_view(), doc(), "document", "Created Page"),
    }),
  );
}

#[test]
fn gold_document_update() {
  check(
    "document_update",
    &json!(DocumentUpdateResponse {
      document: document_record(doc()),
      snapshot: store::AppliedContent {
        version_seq: 4,
        schema_version: 1,
        payload: payload_blocks(),
      },
      update: store::AppliedOperations {
        seq: 4,
        actor_id: user(),
        update_kind: "block_operations",
        payload: json!([{
          "type": "update_block", "block_id": "block_a",
          "text": "First paragraph edited", "data": {}
        }]),
      },
    }),
  );
}

#[test]
fn gold_document_bootstrap() {
  check(
    "document_bootstrap",
    &json!(DocumentBootstrapResponse {
      document: document_record(doc()),
      view: view(page_view(), doc(), "document", "Bootstrapped Page"),
      snapshot: store::AppliedContent {
        version_seq: 3,
        schema_version: 1,
        payload: payload_blocks(),
      },
    }),
  );
}
