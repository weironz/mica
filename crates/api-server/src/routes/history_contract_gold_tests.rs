//! Gold-file contract tests (#85) for the version-history DTOs. The mcp
//! server's `mica_list_versions` reads `versions` from `contracts/history.json`
//! and its `mica_create_version` reads `version/id` from
//! `contracts/version.json`; the Flutter history panel decodes the same rows.

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
fn gold_history() {
  check(
    "history",
    &json!(HistoryResponse {
      versions: vec![
        YrsVersionMeta {
          id: Uuid::parse_str("aaaaaaaa-0000-4000-8000-000000000001").unwrap(),
          document_id: Uuid::parse_str("66666666-6666-4666-8666-666666666666").unwrap(),
          rid: Some(42),
          label: Some("before the rewrite".to_string()),
          created_by: Some(Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap()),
          created_at: at(2),
        },
        YrsVersionMeta {
          id: Uuid::parse_str("aaaaaaaa-0000-4000-8000-000000000002").unwrap(),
          document_id: Uuid::parse_str("66666666-6666-4666-8666-666666666666").unwrap(),
          rid: Some(17),
          label: None,
          created_by: None,
          created_at: at(1),
        },
      ],
    }),
  );
}

#[test]
fn gold_version() {
  check(
    "version",
    &json!(VersionResponse {
      version: YrsVersionMeta {
        id: Uuid::parse_str("aaaaaaaa-0000-4000-8000-000000000001").unwrap(),
        document_id: Uuid::parse_str("66666666-6666-4666-8666-666666666666").unwrap(),
        rid: Some(42),
        label: Some("before the rewrite".to_string()),
        created_by: Some(Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap()),
        created_at: at(2),
      },
    }),
  );
}
