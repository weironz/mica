//! Gold-file contract tests (#85) for the workspace DTOs. The golds live in
//! `/contracts` and are decoded by the cli (`Workspace`) and Flutter
//! (`Workspace.fromJson`) test suites. Field renames fail all sides at once.

use chrono::TimeZone;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::routes::contract_gold::check;

fn at(hour: u32) -> chrono::DateTime<Utc> {
  Utc.with_ymd_and_hms(2026, 10, 24, hour, 0, 0).unwrap()
}

#[test]
fn gold_workspace_list() {
  check(
    "workspace_list",
    &json!(WorkspaceListResponse {
      workspaces: vec![Workspace {
        id: Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap(),
        name: "Gold Workspace".to_string(),
        owner_id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
        role: "owner".to_string(),
        created_at: at(1),
        updated_at: at(2),
        // `page_count` is `#[serde(default)]`: the list route fills it, the
        // single-workspace routes leave it out entirely — the gold pins the
        // LIST shape, where it is present.
        page_count: 12,
      }],
    }),
  );
}

#[test]
fn gold_workspace_members() {
  check(
    "workspace_members",
    &json!(WorkspaceMemberListResponse {
      members: vec![
        WorkspaceMember {
          user_id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
          email: "owner@mica.test".to_string(),
          display_name: "Gold Owner".to_string(),
          role: "owner".to_string(),
          joined_at: at(1),
          avatar_key: None,
        },
        WorkspaceMember {
          user_id: Uuid::parse_str("88888888-8888-4888-8888-888888888888").unwrap(),
          email: "editor@mica.test".to_string(),
          display_name: "Gold Editor".to_string(),
          role: "editor".to_string(),
          joined_at: at(3),
          // Serialized as the derived `avatar_version`, never the raw key:
          // "avatars/v3-20261024.png" → "v3-20261024" (last segment, stem,
          // ≤16 chars — see routes/avatar.rs).
          avatar_key: Some("avatars/v3-20261024.png".to_string()),
        },
      ],
    }),
  );
}
