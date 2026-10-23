//! Gold-file contract tests (#85): decode `/contracts/*.json` — produced by
//! the SERVER's DTO types — into the CLI's own, and assert every field.
//!
//! There is deliberately no shared DTO crate between server / cli / mcp /
//! Flutter (the web client needs a Dart implementation, so a Rust crate could
//! only cover three of the four). These golds are the drift guard instead: a
//! server-side rename fails the server's gold test AND this one in the same
//! CI run, instead of breaking `mica-cli` against a deployed server.

use serde_json::Value;
use uuid::Uuid;

use super::*;

/// The repo-root `contracts/` dir, from this crate's manifest dir.
fn gold(name: &str) -> String {
  let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
    .join("../../contracts")
    .join(format!("{name}.json"));
  std::fs::read_to_string(&path)
    .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
}

fn gold_json(name: &str) -> Value {
  serde_json::from_str(&gold(name)).expect("gold file is valid JSON")
}

#[test]
fn view_list_decodes_every_field() {
  let list: ViewListResponse =
    serde_json::from_str(&gold("view_list")).expect("decoding view list");
  assert_eq!(list.views.len(), 3, "three gold views");

  let folder = &list.views[0];
  assert_eq!(
    folder.id,
    Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap()
  );
  assert_eq!(folder.object_type, "folder");
  assert_eq!(folder.name, "Gold Folder");
  assert!(!folder.is_deleted);
  assert_eq!(folder.parent_view_id, None);
  assert_eq!(folder.state_bytes, None, "the folder row carries no stats");

  let stats = &list.views[1];
  assert_eq!(
    stats.object_id,
    Uuid::parse_str("66666666-6666-4666-8666-666666666666").unwrap()
  );
  assert_eq!(stats.object_type, "document");
  assert_eq!(stats.name, "With Stats");
  assert_eq!(
    stats.parent_view_id,
    Some(Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap())
  );
  assert_eq!(stats.state_bytes, Some(4096));

  let plain = &list.views[2];
  assert_eq!(plain.name, "Plain Page");
  assert_eq!(
    plain.state_bytes, None,
    "absent without with_stats, not null"
  );
}

#[test]
fn workspace_list_decodes_every_field() {
  let list: WorkspaceListResponse =
    serde_json::from_str(&gold("workspace_list")).expect("decoding workspace list");
  assert_eq!(list.workspaces.len(), 1);
  let ws = &list.workspaces[0];
  assert_eq!(
    ws.id,
    Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap()
  );
  assert_eq!(ws.name, "Gold Workspace");
  assert_eq!(
    ws.owner_id,
    Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap()
  );
  assert_eq!(ws.role, "owner");
  assert_eq!(ws.created_at, "2026-10-24T01:00:00Z");
  assert_eq!(ws.updated_at, "2026-10-24T02:00:00Z");
}

#[test]
fn user_decodes_every_field() {
  let user: User = serde_json::from_str(&gold("user")).expect("decoding user");
  assert_eq!(
    user.id,
    Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap()
  );
  assert_eq!(user.email, "owner@mica.test");
  assert_eq!(user.display_name, "Gold Owner");
  assert_eq!(user.created_at, "2026-10-24T01:00:00Z");
}

#[test]
fn batch_outcome_decodes_every_field() {
  let outcome: BatchOutcome =
    serde_json::from_str(&gold("batch_views")).expect("decoding batch outcome");
  assert_eq!(outcome.affected, 3);
  assert_eq!(
    outcome.skipped,
    vec![Uuid::parse_str("55555555-5555-4555-8555-555555555555").unwrap()]
  );
}

/// `read_markdown` and the batch survey navigate the raw JSON by key, not
/// through a struct — those string keys are as much the contract as any type.
#[test]
fn raw_navigation_keys_exist_where_the_client_walks_them() {
  let batch = gold_json("batch_read");
  let docs = batch["documents"].as_array().expect("documents array");
  let first = &docs[0];
  assert_eq!(
    first["document_id"].as_str(),
    Some("66666666-6666-4666-8666-666666666666")
  );
  assert_eq!(first["markdown"].as_str(), Some("# First\n\nbody"));
  let failed = &docs[1];
  assert_eq!(
    failed["error"].as_str().map(str::to_owned),
    Some("not found".to_owned()),
    "a per-doc failure is reported inline, never as a whole-request failure"
  );
  assert!(failed["markdown"].is_null(), "never both, on one row");

  let bootstrap = gold_json("document_bootstrap");
  let blocks = bootstrap["snapshot"]["payload"]["blocks"]
    .as_array()
    .expect("the blocks list `document_bootstrap`'s caller walks");
  assert_eq!(blocks.len(), 3);
}
