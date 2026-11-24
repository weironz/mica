//! Usage Statistics HTTP API (`/api/stats/overview`, `/api/stats/activity`, Issue #191).
//!
//! Strictly scoped to the authenticated user's accessible workspaces
//! (`workspace_members.user_id = $1`). All reads aggregate from indexed SQL
//! tables (`views`, `document_yrs_base`, `files`, `pending_file_uploads`,
//! `document_shares`, `comment_threads`, `writing_activity_doc_hourly`) with
//! zero CRDT decoding on the request path.

use std::collections::{HashMap, HashSet};

use axum::{
  Json,
  extract::{Query, State},
  http::HeaderMap,
};
use chrono::{DateTime, Utc};
use mica_app_core::AppState;
use mica_core::usage_stats::{
  ActivitySource, FallbackDocCreation, HourlyActivityRow, UsageHealth, UsageStatsActivity,
  UsageStatsOverview, UsageSummary, WorkspaceUsageItem, fold_hourly_to_daily, format_unix_rfc3339,
  parse_activity_range,
};
use mica_infra::{ApiError, ApiResult};
use serde::Deserialize;
use uuid::Uuid;

use crate::routes::auth::user_id_from_headers;
use crate::routes::documents::workspace_role;

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
  /// One of `"1d"`, `"7d"`, `"30d"`, `"90d"`, `"365d"`. Defaults to `"30d"`.
  pub range: Option<String>,
  /// Client timezone offset in minutes west/east of UTC (JS-style positive east,
  /// e.g. `+480` for `UTC+8`, `-300` for `UTC-5`). Valid range: `-720..=840`.
  pub tz_offset: Option<i32>,
  /// Optional workspace filter; caller must be a member of that workspace.
  pub workspace_id: Option<Uuid>,
  /// Whether to include `import` / `transfer` activity rows. Defaults to `false`.
  pub include_imports: Option<bool>,
}

/// Validate `ActivityQuery` parameters before hitting the database.
pub(crate) fn validate_activity_query(
  query: &ActivityQuery,
) -> ApiResult<(&'static str, u32, i32, bool)> {
  let (range_label, range_days) =
    parse_activity_range(query.range.as_deref()).ok_or_else(|| {
      ApiError::BadRequest("invalid range; expected one of 1d, 7d, 30d, 90d, 365d".into())
    })?;
  let tz_offset = query.tz_offset.unwrap_or(0);
  if !(-720..=840).contains(&tz_offset) {
    return Err(ApiError::BadRequest(
      "tz_offset must be between -720 and 840 minutes".into(),
    ));
  }
  let include_imports = query.include_imports.unwrap_or(false);
  Ok((range_label, range_days, tz_offset, include_imports))
}

/// `GET /api/stats/overview`
///
/// Returns global KPIs, knowledge base health indicators, and per-workspace
/// breakdowns across every workspace the authenticated user belongs to.
pub async fn overview(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<UsageStatsOverview>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let quota_per_ws = state.config.workspace_quota_bytes.max(0) as u64;

  // 1. Accessible workspaces + per-workspace page/folder/word aggregates.
  #[allow(clippy::type_complexity)]
  let ws_rows: Vec<(
    Uuid,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    Option<DateTime<Utc>>,
  )> = sqlx::query_as(
    r#"
    SELECT
      w.id,
      w.name,
      wm.role::text AS role,
      COALESCE(COUNT(*) FILTER (WHERE v.is_deleted = false AND v.object_type::text = 'document'), 0)::bigint AS live_pages,
      COALESCE(COUNT(*) FILTER (WHERE v.is_deleted = false AND v.object_type::text = 'folder'), 0)::bigint AS folders,
      COALESCE(COUNT(*) FILTER (WHERE v.is_deleted = true  AND v.object_type::text = 'document'), 0)::bigint AS trashed_pages,
      COALESCE(SUM(CASE WHEN v.is_deleted = false AND v.object_type::text = 'document' THEN COALESCE(b.word_count, 0) ELSE 0 END), 0)::bigint AS words,
      COALESCE(SUM(CASE WHEN v.is_deleted = false AND v.object_type::text = 'document' THEN COALESCE(b.char_count, 0) ELSE 0 END), 0)::bigint AS chars,
      COALESCE(COUNT(*) FILTER (WHERE v.is_deleted = false AND v.object_type::text = 'document' AND COALESCE(b.word_count, 0) = 0), 0)::bigint AS empty_pages,
      MAX(CASE WHEN v.is_deleted = false AND v.object_type::text = 'document' THEN d.updated_at ELSE NULL END) AS max_updated_at
    FROM workspaces w
    INNER JOIN workspace_members wm ON wm.workspace_id = w.id
    LEFT JOIN views v ON v.workspace_id = w.id
    LEFT JOIN documents d ON d.id = v.object_id AND v.object_type::text = 'document'
    LEFT JOIN document_yrs_base b ON b.document_id = d.id
    WHERE wm.user_id = $1
    GROUP BY w.id, w.name, wm.role, wm.position, w.created_at
    ORDER BY wm.position ASC, w.created_at ASC
    "#,
  )
  .bind(user_id)
  .fetch_all(&state.db)
  .await?;

  if ws_rows.is_empty() {
    return Ok(Json(UsageStatsOverview {
      generated_at: format_unix_rfc3339(Utc::now().timestamp()),
      summary: UsageSummary::default(),
      health: UsageHealth::default(),
      workspaces: Vec::new(),
    }));
  }

  let ws_ids: Vec<Uuid> = ws_rows.iter().map(|r| r.0).collect();

  // 2. Per-workspace files & storage (registered files + pending upload reservations,
  //    using the exact same storage definition as `store::workspace_bytes_used`).
  let file_rows: Vec<(Uuid, i64, i64, i64)> = sqlx::query_as(
    r#"
    SELECT
      w.id,
      COALESCE((SELECT COUNT(*) FROM files f WHERE f.workspace_id = w.id AND f.mime_type LIKE 'image/%'), 0)::bigint AS image_count,
      COALESCE((SELECT COUNT(*) FROM files f WHERE f.workspace_id = w.id AND f.mime_type NOT LIKE 'image/%'), 0)::bigint AS attachment_count,
      (
        COALESCE((SELECT SUM(f.byte_size) FROM files f WHERE f.workspace_id = w.id), 0)
        + COALESCE((
            SELECT SUM(p.byte_size)
            FROM pending_file_uploads p
            WHERE p.workspace_id = w.id
              AND NOT EXISTS (SELECT 1 FROM files f WHERE f.object_key = p.object_key)
          ), 0)
      )::bigint AS storage_bytes
    FROM workspaces w
    WHERE w.id = ANY($1)
    "#,
  )
  .bind(&ws_ids)
  .fetch_all(&state.db)
  .await?;
  let file_map: HashMap<Uuid, (u64, u64, u64)> = file_rows
    .into_iter()
    .map(|(id, imgs, atts, bytes)| {
      (
        id,
        (imgs.max(0) as u64, atts.max(0) as u64, bytes.max(0) as u64),
      )
    })
    .collect();

  // 3. Per-workspace directed page-link edges (both ends live documents in the same workspace).
  let edge_rows: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
    r#"
    SELECT src.workspace_id, src.id, tgt.id
    FROM views src
    INNER JOIN document_yrs_base b ON b.document_id = src.object_id
    CROSS JOIN LATERAL unnest(COALESCE(b.link_targets, ARRAY[]::uuid[])) AS target_id
    INNER JOIN views tgt
      ON tgt.id = target_id
     AND tgt.workspace_id = src.workspace_id
     AND tgt.is_deleted = false
     AND tgt.object_type::text = 'document'
    WHERE src.workspace_id = ANY($1)
      AND src.is_deleted = false
      AND src.object_type::text = 'document'
      AND src.id <> tgt.id
    "#,
  )
  .bind(&ws_ids)
  .fetch_all(&state.db)
  .await?;
  let mut edges_by_ws: HashMap<Uuid, u64> = HashMap::new();
  let mut linked_by_ws: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
  for (ws_id, src_id, tgt_id) in edge_rows {
    *edges_by_ws.entry(ws_id).or_default() += 1;
    let set = linked_by_ws.entry(ws_id).or_default();
    set.insert(src_id);
    set.insert(tgt_id);
  }

  // 4. Active public shares & unresolved comment threads per workspace.
  let share_rows: Vec<(Uuid, i64)> = sqlx::query_as(
    r#"
    SELECT ds.workspace_id, COUNT(*)::bigint
    FROM document_shares ds
    INNER JOIN views v
      ON v.workspace_id = ds.workspace_id
     AND v.object_id = ds.document_id
     AND v.object_type::text = 'document'
     AND v.is_deleted = false
    WHERE ds.workspace_id = ANY($1)
      AND ds.revoked_at IS NULL
    GROUP BY ds.workspace_id
    "#,
  )
  .bind(&ws_ids)
  .fetch_all(&state.db)
  .await?;
  let share_map: HashMap<Uuid, u64> = share_rows
    .into_iter()
    .map(|(ws, c)| (ws, c.max(0) as u64))
    .collect();

  let comment_rows: Vec<(Uuid, i64)> = sqlx::query_as(
    r#"
    SELECT v.workspace_id, COUNT(ct.id)::bigint
    FROM comment_threads ct
    INNER JOIN views v
      ON v.object_id = ct.document_id
     AND v.object_type::text = 'document'
     AND v.is_deleted = false
    WHERE v.workspace_id = ANY($1)
      AND ct.status <> 'resolved'
    GROUP BY v.workspace_id
    "#,
  )
  .bind(&ws_ids)
  .fetch_all(&state.db)
  .await?;
  let comment_map: HashMap<Uuid, u64> = comment_rows
    .into_iter()
    .map(|(ws, c)| (ws, c.max(0) as u64))
    .collect();

  let mut summary = UsageSummary::default();
  let mut health = UsageHealth::default();
  let mut workspaces = Vec::with_capacity(ws_rows.len());

  for (
    ws_id,
    name,
    role,
    live_pages,
    folders,
    trashed_pages,
    words,
    chars,
    empty_pages,
    max_updated,
  ) in ws_rows
  {
    let live_pages = live_pages.max(0) as u64;
    let folders = folders.max(0) as u64;
    let trashed_pages = trashed_pages.max(0) as u64;
    let words = words.max(0) as u64;
    let chars = chars.max(0) as u64;
    let empty_pages = empty_pages.max(0) as u64;
    let (image_count, attachment_count, storage_bytes) =
      file_map.get(&ws_id).copied().unwrap_or((0, 0, 0));
    let link_edges = edges_by_ws.get(&ws_id).copied().unwrap_or(0);
    let linked_nodes = linked_by_ws.get(&ws_id).map_or(0, |s| s.len() as u64);
    let orphan_pages = live_pages.saturating_sub(linked_nodes);
    let public_shares = share_map.get(&ws_id).copied().unwrap_or(0);
    let unresolved_comments = comment_map.get(&ws_id).copied().unwrap_or(0);

    summary.workspace_count += 1;
    summary.folder_count += folders;
    summary.live_page_count += live_pages;
    summary.trashed_page_count += trashed_pages;
    summary.total_words += words;
    summary.total_chars += chars;
    summary.image_count += image_count;
    summary.attachment_count += attachment_count;
    summary.storage_bytes = summary.storage_bytes.saturating_add(storage_bytes);
    summary.quota_bytes_total = summary.quota_bytes_total.saturating_add(quota_per_ws);

    health.link_edges += link_edges;
    health.orphan_pages += orphan_pages;
    health.empty_pages += empty_pages;
    health.public_shares += public_shares;
    health.unresolved_comments += unresolved_comments;

    workspaces.push(WorkspaceUsageItem {
      workspace_id: ws_id.to_string(),
      name,
      role,
      live_pages,
      folders,
      trashed_pages,
      words,
      chars,
      image_count,
      attachment_count,
      storage_bytes,
      quota_bytes: quota_per_ws,
      link_edges,
      orphan_pages,
      empty_pages,
      public_shares,
      unresolved_comments,
      updated_at: max_updated.map(|t| format_unix_rfc3339(t.timestamp())),
    });
  }

  Ok(Json(UsageStatsOverview {
    generated_at: format_unix_rfc3339(Utc::now().timestamp()),
    summary,
    health,
    workspaces,
  }))
}

/// `GET /api/stats/activity?range=30d&tz_offset=480&workspace_id=<opt>&include_imports=false`
///
/// Folds the authenticated user's UTC hourly writing activity buckets into the
/// caller's local calendar days, returning daily series, 180-day heatmap,
/// deduplicated edited-page counts, and writing streaks.
pub async fn activity(
  State(state): State<AppState>,
  headers: HeaderMap,
  Query(query): Query<ActivityQuery>,
) -> ApiResult<Json<UsageStatsActivity>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let (range_label, range_days, tz_offset, include_imports) = validate_activity_query(&query)?;

  if let Some(ws_id) = query.workspace_id {
    workspace_role(&state.db, ws_id, user_id)
      .await?
      .ok_or(ApiError::NotFound)?;
  }

  let heatmap_days = 180u32;
  let lookback_days = i64::from(range_days.max(heatmap_days)) + 2;
  let now_utc = Utc::now();
  let since_ts = now_utc - chrono::Duration::days(lookback_days);

  // Earliest tracked activity bucket for this user (and optional workspace).
  let tracking_since: Option<i64> = sqlx::query_scalar(
    r#"
    SELECT EXTRACT(EPOCH FROM MIN(bucket_hour))::bigint
    FROM writing_activity_doc_hourly
    WHERE user_id = $1
      AND ($2::uuid IS NULL OR workspace_id = $2)
    "#,
  )
  .bind(user_id)
  .bind(query.workspace_id)
  .fetch_one(&state.db)
  .await?;

  let raw_rows: Vec<(i64, Uuid, String, bool, i32, i32, i32)> = sqlx::query_as(
    r#"
    SELECT
      EXTRACT(EPOCH FROM bucket_hour)::bigint AS bucket_hour_secs,
      document_id,
      source,
      is_created,
      edit_events,
      words_added,
      words_deleted
    FROM writing_activity_doc_hourly
    WHERE user_id = $1
      AND ($2::uuid IS NULL OR workspace_id = $2)
      AND bucket_hour >= $3
    "#,
  )
  .bind(user_id)
  .bind(query.workspace_id)
  .bind(since_ts)
  .fetch_all(&state.db)
  .await?;

  let hourly_rows: Vec<HourlyActivityRow> = raw_rows
    .into_iter()
    .map(
      |(bucket_hour_secs, doc_id, source, is_created, edit_events, words_added, words_deleted)| {
        HourlyActivityRow {
          bucket_hour_secs,
          document_id: doc_id.to_string(),
          source: ActivitySource::from_db_str(&source),
          is_created,
          edit_events: edit_events.max(0) as u64,
          words_added: words_added.max(0) as u64,
          words_deleted: words_deleted.max(0) as u64,
        }
      },
    )
    .collect();

  // Historical `pages_created` fallback from `documents.created_at` prior to `tracking_since`.
  let fallback_rows: Vec<(i64, Uuid)> = sqlx::query_as(
    r#"
    SELECT EXTRACT(EPOCH FROM d.created_at)::bigint, d.id
    FROM documents d
    INNER JOIN workspace_members wm
      ON wm.workspace_id = d.workspace_id
     AND wm.user_id = $1
    WHERE d.created_by = $1
      AND ($2::uuid IS NULL OR d.workspace_id = $2)
      AND d.created_at >= $3
    "#,
  )
  .bind(user_id)
  .bind(query.workspace_id)
  .bind(since_ts)
  .fetch_all(&state.db)
  .await?;

  let fallback_creations: Vec<FallbackDocCreation> = fallback_rows
    .into_iter()
    .map(|(created_at_secs, id)| FallbackDocCreation {
      created_at_secs,
      document_id: id.to_string(),
    })
    .collect();

  let result = fold_hourly_to_daily(
    &hourly_rows,
    &fallback_creations,
    tracking_since,
    range_label,
    range_days,
    heatmap_days,
    tz_offset,
    query.workspace_id.map(|id| id.to_string()),
    include_imports,
    now_utc.timestamp(),
  );

  Ok(Json(result))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn validates_range_and_tz_offset_bounds() {
    let ok = validate_activity_query(&ActivityQuery {
      range: None,
      tz_offset: Some(480),
      workspace_id: None,
      include_imports: None,
    })
    .unwrap();
    assert_eq!(ok, ("30d", 30, 480, false));

    let ok_1y = validate_activity_query(&ActivityQuery {
      range: Some("365d".into()),
      tz_offset: Some(-720),
      workspace_id: None,
      include_imports: Some(true),
    })
    .unwrap();
    assert_eq!(ok_1y, ("365d", 365, -720, true));

    assert!(
      validate_activity_query(&ActivityQuery {
        range: Some("10y".into()),
        tz_offset: Some(0),
        workspace_id: None,
        include_imports: None,
      })
      .is_err()
    );

    assert!(
      validate_activity_query(&ActivityQuery {
        range: Some("7d".into()),
        tz_offset: Some(900),
        workspace_id: None,
        include_imports: None,
      })
      .is_err()
    );
  }
}
