//! Gold-file contract tests (#85) for the auth DTOs. `contracts/user.json` is
//! what `GET /api/auth/me` answers; the cli (`User`) and Flutter
//! (`User.fromJson`) suites decode the same file.

use chrono::TimeZone;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::routes::contract_gold::check;

#[test]
fn gold_user() {
  check(
    "user",
    &json!(UserResponse {
      id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
      email: "owner@mica.test".to_string(),
      display_name: "Gold Owner".to_string(),
      created_at: Utc.with_ymd_and_hms(2026, 10, 24, 1, 0, 0).unwrap(),
      // None serializes as null — the client distinguishes "no picture" from
      // "unknown version", so the field is always present.
      avatar_version: Some("v3-20261024".to_string()),
    }),
  );
}
