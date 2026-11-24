//! Usage Statistics DTOs and pure-Rust UTC hourly-to-local daily activity folding.
//!
//! Shared across:
//! - Backend HTTP API (`/api/stats/overview`, `/api/stats/activity`),
//! - Local SQLite store (`LocalStore::usage_overview`, `LocalStore::usage_activity`),
//! - Pure-Rust L3 desktop/web client (`clients/mica_rust`).
//!
//! All timestamps in the wire DTOs are RFC 3339 UTC strings (`YYYY-MM-DDTHH:MM:SSZ`)
//! and daily bucket dates are local-timezone `YYYY-MM-DD` strings computed with
//! integer UTC calendar arithmetic so `mica-core` stays dependency-free and
//! 100% `wasm32`-compatible.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Supported origin/source values for writing activity records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivitySource {
  Manual,
  Import,
  Transfer,
}

impl ActivitySource {
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Manual => "manual",
      Self::Import => "import",
      Self::Transfer => "transfer",
    }
  }

  pub fn from_db_str(s: &str) -> Self {
    match s {
      "import" => Self::Import,
      "transfer" => Self::Transfer,
      _ => Self::Manual,
    }
  }

  pub const fn is_bulk(self) -> bool {
    matches!(self, Self::Import | Self::Transfer)
  }
}

/// Top-level overview payload (`GET /api/stats/overview` and local equivalent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageStatsOverview {
  pub generated_at: String,
  pub summary: UsageSummary,
  pub health: UsageHealth,
  pub workspaces: Vec<WorkspaceUsageItem>,
}

/// Global KPI counters across all accessible workspaces.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSummary {
  pub workspace_count: u64,
  pub folder_count: u64,
  pub live_page_count: u64,
  pub trashed_page_count: u64,
  pub total_words: u64,
  pub total_chars: u64,
  pub image_count: u64,
  pub attachment_count: u64,
  pub storage_bytes: u64,
  /// `0` when quotas are disabled or in local mode (no quota cap).
  pub quota_bytes_total: u64,
}

/// Knowledge base health metrics across accessible live documents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageHealth {
  pub link_edges: u64,
  pub orphan_pages: u64,
  pub empty_pages: u64,
  pub public_shares: u64,
  pub unresolved_comments: u64,
}

/// Per-workspace usage breakdown row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceUsageItem {
  pub workspace_id: String,
  pub name: String,
  pub role: String,
  pub live_pages: u64,
  pub folders: u64,
  pub trashed_pages: u64,
  pub words: u64,
  pub chars: u64,
  pub image_count: u64,
  pub attachment_count: u64,
  pub storage_bytes: u64,
  pub quota_bytes: u64,
  pub link_edges: u64,
  pub orphan_pages: u64,
  pub empty_pages: u64,
  pub public_shares: u64,
  pub unresolved_comments: u64,
  pub updated_at: Option<String>,
}

/// Time-series & heatmap payload (`GET /api/stats/activity` and local equivalent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageStatsActivity {
  /// One of `"1d"`, `"7d"`, `"30d"`, `"90d"`, `"365d"`.
  pub range: String,
  pub tz_offset_minutes: i32,
  pub workspace_id: Option<String>,
  pub include_imports: bool,
  /// Earliest RFC 3339 UTC timestamp from which edit/word tracking exists, or `None`
  /// if no activity has been recorded yet.
  pub tracking_since: Option<String>,
  pub totals: UsageActivityTotals,
  pub daily: Vec<DailyActivityPoint>,
  pub heatmap: Vec<HeatmapDayPoint>,
}

/// Aggregated activity summary over the selected range (plus streaks over heatmap window).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageActivityTotals {
  pub pages_created: u64,
  /// Deduplicated distinct documents edited at least once within the range.
  pub pages_edited: u64,
  pub words_added: u64,
  pub words_deleted: u64,
  pub net_words: i64,
  pub active_days: u64,
  pub current_streak: u64,
  pub longest_streak: u64,
}

/// One local calendar day in the writing trend chart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyActivityPoint {
  /// Local date in `YYYY-MM-DD`.
  pub date: String,
  pub pages_created: u64,
  /// Deduplicated distinct documents edited on this local date.
  pub pages_edited: u64,
  pub words_added: u64,
  pub words_deleted: u64,
  pub net_words: i64,
}

/// One local calendar day in the 180-day activity heatmap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeatmapDayPoint {
  /// Local date in `YYYY-MM-DD`.
  pub date: String,
  /// Combined activity score (`pages_created * 3 + pages_edited + words_added / 50`).
  pub score: u64,
  /// Intensity level `0..=4`.
  pub level: u8,
  pub pages_created: u64,
  pub pages_edited: u64,
  pub words_added: u64,
}

/// Raw hourly activity row loaded from Postgres (`writing_activity_doc_hourly`)
/// or SQLite (`local_writing_activity_hourly`) before timezone folding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HourlyActivityRow {
  /// UTC hour-aligned Unix epoch seconds (`ts - ts % 3600`).
  pub bucket_hour_secs: i64,
  pub document_id: String,
  pub source: ActivitySource,
  pub is_created: bool,
  pub edit_events: u64,
  pub words_added: u64,
  pub words_deleted: u64,
}

/// Historical document creation event (used as fallback for `pages_created` prior to
/// `tracking_since` when `writing_activity_doc_hourly` did not exist yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackDocCreation {
  pub created_at_secs: i64,
  pub document_id: String,
}

/// Parse a `range` query string into `(canonical_label, range_days)`.
pub fn parse_activity_range(raw: Option<&str>) -> Option<(&'static str, u32)> {
  match raw.unwrap_or("30d").trim() {
    "1d" => Some(("1d", 1)),
    "7d" => Some(("7d", 7)),
    "30d" => Some(("30d", 30)),
    "90d" => Some(("90d", 90)),
    "365d" => Some(("365d", 365)),
    _ => None,
  }
}

/// Clamp or validate `tz_offset` minutes (`-720..=840`, UTC-12 to UTC+14).
pub fn clamp_tz_offset_minutes(offset: i32) -> i32 {
  offset.clamp(-720, 840)
}

/// Format Unix epoch seconds into RFC 3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn format_unix_rfc3339(secs: i64) -> String {
  let days = secs.div_euclid(86_400);
  let rem = secs.rem_euclid(86_400);
  let (year, month, day) = civil_from_days(days);
  let hour = rem / 3_600;
  let minute = (rem % 3_600) / 60;
  let second = rem % 60;
  format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Format a Unix day number (`secs.div_euclid(86_400)`) into `YYYY-MM-DD`.
pub fn format_unix_day(days: i64) -> String {
  let (year, month, day) = civil_from_days(days);
  format!("{year:04}-{month:02}-{day:02}")
}

/// Howard Hinnant's civil-from-days algorithm: maps days since `1970-01-01`
/// to Gregorian `(year, month 1..=12, day 1..=31)`.
pub fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
  let z = days_since_epoch + 719_468;
  let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
  let doe = (z - era * 146_097) as u64; // [0, 146096]
  let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
  let y = yoe as i64 + era * 400;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
  let mp = (5 * doy + 2) / 153; // [0, 11]
  let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
  let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
  let year = y + if m <= 2 { 1 } else { 0 };
  (year, m as u32, d as u32)
}

/// Fold raw UTC hourly activity rows (plus optional historical creation fallbacks)
/// into timezone-aligned daily points, a 180-day heatmap, deduplicated edit totals,
/// and streak metrics.
#[allow(clippy::too_many_arguments)]
pub fn fold_hourly_to_daily(
  rows: &[HourlyActivityRow],
  fallback_creations: &[FallbackDocCreation],
  tracking_since_secs: Option<i64>,
  range_label: &str,
  range_days: u32,
  heatmap_days: u32,
  tz_offset_minutes: i32,
  workspace_id: Option<String>,
  include_imports: bool,
  now_utc_secs: i64,
) -> UsageStatsActivity {
  let tz_offset_minutes = clamp_tz_offset_minutes(tz_offset_minutes);
  let tz_offset_secs = i64::from(tz_offset_minutes) * 60;
  let today_local_day = (now_utc_secs + tz_offset_secs).div_euclid(86_400);

  let range_days = range_days.max(1);
  let heatmap_days = heatmap_days.max(1);
  let range_start_day = today_local_day - i64::from(range_days - 1);
  let heatmap_start_day = today_local_day - i64::from(heatmap_days - 1);
  let window_min_day = range_start_day.min(heatmap_start_day);

  #[derive(Default)]
  struct DayAcc {
    created_docs: HashSet<String>,
    edited_docs: HashSet<String>,
    words_added: u64,
    words_deleted: u64,
  }

  let mut by_day: HashMap<i64, DayAcc> = HashMap::new();
  let mut tracked_created_docs: HashSet<String> = HashSet::new();

  for row in rows {
    if !include_imports && row.source.is_bulk() {
      continue;
    }
    let local_day = (row.bucket_hour_secs + tz_offset_secs).div_euclid(86_400);
    if local_day < window_min_day || local_day > today_local_day {
      continue;
    }
    let acc = by_day.entry(local_day).or_default();
    if row.is_created {
      acc.created_docs.insert(row.document_id.clone());
      tracked_created_docs.insert(row.document_id.clone());
    }
    if row.edit_events > 0 {
      acc.edited_docs.insert(row.document_id.clone());
    }
    acc.words_added = acc.words_added.saturating_add(row.words_added);
    acc.words_deleted = acc.words_deleted.saturating_add(row.words_deleted);
  }

  // Fallback for historical `pages_created` before activity tracking existed:
  // only count docs not already recorded as `is_created = true` in the hourly table.
  for doc in fallback_creations {
    if tracked_created_docs.contains(&doc.document_id) {
      continue;
    }
    if let Some(since) = tracking_since_secs {
      if doc.created_at_secs >= since {
        continue;
      }
    }
    let local_day = (doc.created_at_secs + tz_offset_secs).div_euclid(86_400);
    if local_day < window_min_day || local_day > today_local_day {
      continue;
    }
    let acc = by_day.entry(local_day).or_default();
    acc.created_docs.insert(doc.document_id.clone());
  }

  // Build daily trend series (zero-filled across `range_days`).
  let mut daily = Vec::with_capacity(range_days as usize);
  let mut range_created_total = 0u64;
  let mut range_edited_docs: HashSet<String> = HashSet::new();
  let mut range_words_added = 0u64;
  let mut range_words_deleted = 0u64;
  let mut active_days_in_range = 0u64;

  for day in range_start_day..=today_local_day {
    let (pages_created, pages_edited, words_added, words_deleted) =
      if let Some(acc) = by_day.get(&day) {
        for doc_id in &acc.edited_docs {
          range_edited_docs.insert(doc_id.clone());
        }
        (
          acc.created_docs.len() as u64,
          acc.edited_docs.len() as u64,
          acc.words_added,
          acc.words_deleted,
        )
      } else {
        (0, 0, 0, 0)
      };

    range_created_total = range_created_total.saturating_add(pages_created);
    range_words_added = range_words_added.saturating_add(words_added);
    range_words_deleted = range_words_deleted.saturating_add(words_deleted);
    if pages_created > 0 || pages_edited > 0 || words_added > 0 {
      active_days_in_range += 1;
    }

    let net_words = (words_added as i128 - words_deleted as i128)
      .clamp(i64::MIN as i128, i64::MAX as i128) as i64;

    daily.push(DailyActivityPoint {
      date: format_unix_day(day),
      pages_created,
      pages_edited,
      words_added,
      words_deleted,
      net_words,
    });
  }

  // Build 180-day heatmap + streaks.
  let mut raw_heat = Vec::with_capacity(heatmap_days as usize);
  let mut max_score = 0u64;
  for day in heatmap_start_day..=today_local_day {
    let (pages_created, pages_edited, words_added) = if let Some(acc) = by_day.get(&day) {
      (
        acc.created_docs.len() as u64,
        acc.edited_docs.len() as u64,
        acc.words_added,
      )
    } else {
      (0, 0, 0)
    };
    let score = pages_created
      .saturating_mul(3)
      .saturating_add(pages_edited)
      .saturating_add(words_added / 50);
    max_score = max_score.max(score);
    raw_heat.push((day, score, pages_created, pages_edited, words_added));
  }

  let mut heatmap = Vec::with_capacity(raw_heat.len());
  let mut longest_streak = 0u64;
  let mut running_streak = 0u64;
  for (day, score, pages_created, pages_edited, words_added) in raw_heat {
    let level = intensity_level(score, max_score);
    if score > 0 {
      running_streak += 1;
      longest_streak = longest_streak.max(running_streak);
    } else {
      running_streak = 0;
    }
    heatmap.push(HeatmapDayPoint {
      date: format_unix_day(day),
      score,
      level,
      pages_created,
      pages_edited,
      words_added,
    });
  }

  // Current streak: consecutive active days ending today, OR ending yesterday if
  // today has no activity yet (matches GitHub/Notion streak expectations).
  let current_streak = compute_current_streak(&heatmap);

  let net_words = (range_words_added as i128 - range_words_deleted as i128)
    .clamp(i64::MIN as i128, i64::MAX as i128) as i64;

  UsageStatsActivity {
    range: range_label.to_string(),
    tz_offset_minutes,
    workspace_id,
    include_imports,
    tracking_since: tracking_since_secs.map(format_unix_rfc3339),
    totals: UsageActivityTotals {
      pages_created: range_created_total,
      pages_edited: range_edited_docs.len() as u64,
      words_added: range_words_added,
      words_deleted: range_words_deleted,
      net_words,
      active_days: active_days_in_range,
      current_streak,
      longest_streak,
    },
    daily,
    heatmap,
  }
}

/// Map a raw daily activity score to GitHub-style intensity `0..=4`.
pub fn intensity_level(score: u64, max_score: u64) -> u8 {
  if score == 0 {
    return 0;
  }
  if max_score <= 4 {
    return (score as u8).min(4);
  }
  let ratio = (score as f64) / (max_score as f64);
  if ratio <= 0.25 {
    1
  } else if ratio <= 0.50 {
    2
  } else if ratio <= 0.75 {
    3
  } else {
    4
  }
}

fn compute_current_streak(heatmap: &[HeatmapDayPoint]) -> u64 {
  if heatmap.is_empty() {
    return 0;
  }
  let mut idx = heatmap.len();
  // If today (last entry) is still 0, allow continuing from yesterday.
  if heatmap[idx - 1].score == 0 {
    if idx == 1 {
      return 0;
    }
    idx -= 1;
  }
  let mut streak = 0u64;
  while idx > 0 {
    idx -= 1;
    if heatmap[idx].score > 0 {
      streak += 1;
    } else {
      break;
    }
  }
  streak
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn civil_date_and_rfc3339_formatting() {
    assert_eq!(format_unix_day(0), "1970-01-01");
    assert_eq!(format_unix_rfc3339(0), "1970-01-01T00:00:00Z");
    // 2026-03-01 00:00:00 UTC = 20513 days * 86400 = 1_772_323_200
    assert_eq!(format_unix_rfc3339(1_772_323_200), "2026-03-01T00:00:00Z");
    // Leap day 2024-02-29 12:34:56 UTC
    let leap = 19_782 * 86_400 + 12 * 3_600 + 34 * 60 + 56;
    assert_eq!(format_unix_rfc3339(leap), "2024-02-29T12:34:56Z");
  }

  #[test]
  fn timezone_folding_shifts_midnight_utc_plus_8_and_minus_5() {
    // UTC 2026-04-18 16:00:00 => Beijing (+480m) 2026-04-19 00:00:00, NY (-300m) 2026-04-18 11:00:00
    let utc_16h = 20_561 * 86_400 + 16 * 3_600; // 2026-04-18T16:00:00Z
    let now_utc = 20_562 * 86_400 + 2 * 3_600; // 2026-04-19T02:00:00Z
    let rows = vec![HourlyActivityRow {
      bucket_hour_secs: utc_16h,
      document_id: "doc-a".into(),
      source: ActivitySource::Manual,
      is_created: true,
      edit_events: 2,
      words_added: 120,
      words_deleted: 20,
    }];

    let bj = fold_hourly_to_daily(
      &rows,
      &[],
      Some(utc_16h),
      "7d",
      7,
      7,
      480,
      None,
      false,
      now_utc,
    );
    assert_eq!(bj.daily.len(), 7);
    let last_bj = bj.daily.last().unwrap();
    assert_eq!(last_bj.date, "2026-04-19");
    assert_eq!(last_bj.pages_created, 1);
    assert_eq!(last_bj.pages_edited, 1);
    assert_eq!(last_bj.words_added, 120);
    assert_eq!(last_bj.net_words, 100);

    let ny = fold_hourly_to_daily(
      &rows,
      &[],
      Some(utc_16h),
      "7d",
      7,
      7,
      -300,
      None,
      false,
      now_utc,
    );
    // At 2026-04-19T02:00:00Z, NY (-5h) is still 2026-04-18 21:00!
    let last_ny = ny.daily.last().unwrap();
    assert_eq!(last_ny.date, "2026-04-18");
    assert_eq!(last_ny.pages_created, 1);
    assert_eq!(last_ny.words_added, 120);
  }

  #[test]
  fn deduplicates_same_doc_across_hours_and_filters_imports() {
    let day = 20_562 * 86_400; // 2026-04-19T00:00:00Z
    let rows = vec![
      HourlyActivityRow {
        bucket_hour_secs: day + 3_600,
        document_id: "doc-1".into(),
        source: ActivitySource::Manual,
        is_created: false,
        edit_events: 3,
        words_added: 50,
        words_deleted: 10,
      },
      HourlyActivityRow {
        bucket_hour_secs: day + 2 * 3_600,
        document_id: "doc-1".into(),
        source: ActivitySource::Manual,
        is_created: false,
        edit_events: 5,
        words_added: 30,
        words_deleted: 0,
      },
      HourlyActivityRow {
        bucket_hour_secs: day + 3 * 3_600,
        document_id: "doc-import".into(),
        source: ActivitySource::Import,
        is_created: true,
        edit_events: 1,
        words_added: 5000,
        words_deleted: 0,
      },
    ];

    let without_imports = fold_hourly_to_daily(
      &rows,
      &[],
      Some(day),
      "1d",
      1,
      7,
      0,
      None,
      false,
      day + 10_000,
    );
    assert_eq!(without_imports.totals.pages_edited, 1);
    assert_eq!(without_imports.totals.pages_created, 0);
    assert_eq!(without_imports.totals.words_added, 80);
    assert_eq!(without_imports.totals.words_deleted, 10);
    assert_eq!(without_imports.totals.net_words, 70);

    let with_imports = fold_hourly_to_daily(
      &rows,
      &[],
      Some(day),
      "1d",
      1,
      7,
      0,
      None,
      true,
      day + 10_000,
    );
    assert_eq!(with_imports.totals.pages_edited, 2);
    assert_eq!(with_imports.totals.pages_created, 1);
    assert_eq!(with_imports.totals.words_added, 5080);
  }

  #[test]
  fn streaks_handle_active_yesterday_and_gaps() {
    let today = 20_562 * 86_400;
    let rows = vec![
      // 5 days ago + 4 days ago (2-day streak)
      HourlyActivityRow {
        bucket_hour_secs: today - 5 * 86_400 + 3_600,
        document_id: "d1".into(),
        source: ActivitySource::Manual,
        is_created: false,
        edit_events: 1,
        words_added: 10,
        words_deleted: 0,
      },
      HourlyActivityRow {
        bucket_hour_secs: today - 4 * 86_400 + 3_600,
        document_id: "d2".into(),
        source: ActivitySource::Manual,
        is_created: false,
        edit_events: 1,
        words_added: 10,
        words_deleted: 0,
      },
      // gap on day -3; then days -2 and -1 active, today 0 => current_streak = 2, longest = 2
      HourlyActivityRow {
        bucket_hour_secs: today - 2 * 86_400 + 3_600,
        document_id: "d3".into(),
        source: ActivitySource::Manual,
        is_created: true,
        edit_events: 1,
        words_added: 100,
        words_deleted: 0,
      },
      HourlyActivityRow {
        bucket_hour_secs: today - 86_400 + 3_600,
        document_id: "d4".into(),
        source: ActivitySource::Manual,
        is_created: false,
        edit_events: 1,
        words_added: 40,
        words_deleted: 0,
      },
    ];
    let res = fold_hourly_to_daily(&rows, &[], None, "7d", 7, 7, 0, None, false, today + 3_600);
    assert_eq!(res.totals.active_days, 4);
    assert_eq!(res.totals.current_streak, 2);
    assert_eq!(res.totals.longest_streak, 2);
  }
}
