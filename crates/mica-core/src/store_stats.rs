//! Local SQLite usage statistics & writing activity persistence (`store` feature).
//!
//! Keeps all SQLite queries for Usage Statistics (Issue #191) separate from
//! `store.rs` so core CRDT persistence stays concise. Uses the exact same
//! [`crate::usage_stats`] DTOs and UTC hourly-to-local daily folding as the
//! cloud backend so local and cloud dashboards speak one contract.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, OptionalExtension};

use crate::store::{LocalStore, StoreError};
use crate::usage_stats::{
  fold_hourly_to_daily, format_unix_rfc3339, ActivitySource, HourlyActivityRow, UsageHealth,
  UsageStatsActivity, UsageStatsOverview, UsageSummary, WorkspaceUsageItem,
};

/// Optional per-workspace file/blob metrics supplied by the desktop shell when
/// querying local usage overview (images/attachments stored outside SQLite).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LocalWorkspaceMedia {
  pub image_count: u64,
  pub attachment_count: u64,
  pub storage_bytes: u64,
}

fn now_unix_secs() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0)
}

impl LocalStore {
  /// Read the stored `word_count` for a document snapshot, if present.
  pub fn doc_word_count(&self, origin: &str, doc_id: &str) -> Result<Option<i64>, StoreError> {
    let count: Option<Option<i64>> = self
      .conn
      .query_row(
        "SELECT word_count FROM doc_snapshot WHERE origin=?1 AND doc_id=?2",
        params![origin, doc_id],
        |r| r.get(0),
      )
      .optional()?;
    Ok(count.flatten())
  }

  /// Record a writing activity event into `local_writing_activity_hourly` using
  /// the current UTC hour bucket (or an explicit `timestamp_secs` for tests).
  #[allow(clippy::too_many_arguments)]
  pub fn record_writing_activity_at(
    &self,
    origin: &str,
    workspace_id: &str,
    document_id: &str,
    source: ActivitySource,
    is_created: bool,
    edit_events: u64,
    words_added: u64,
    words_deleted: u64,
    timestamp_secs: i64,
  ) -> Result<(), StoreError> {
    let bucket_hour = timestamp_secs - timestamp_secs.rem_euclid(3_600);
    self.conn.execute(
      "INSERT INTO local_writing_activity_hourly(
           origin, workspace_id, bucket_hour, document_id, source,
           is_created, edit_events, words_added, words_deleted
       ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
       ON CONFLICT(origin, bucket_hour, document_id, source) DO UPDATE SET
           workspace_id  = excluded.workspace_id,
           is_created    = MAX(local_writing_activity_hourly.is_created, excluded.is_created),
           edit_events   = local_writing_activity_hourly.edit_events + excluded.edit_events,
           words_added   = local_writing_activity_hourly.words_added + excluded.words_added,
           words_deleted = local_writing_activity_hourly.words_deleted + excluded.words_deleted",
      params![
        origin,
        workspace_id,
        bucket_hour,
        document_id,
        source.as_str(),
        if is_created { 1i64 } else { 0i64 },
        edit_events as i64,
        words_added as i64,
        words_deleted as i64,
      ],
    )?;
    Ok(())
  }

  /// Convenience wrapper using the current wall-clock UTC timestamp.
  #[allow(clippy::too_many_arguments)]
  pub fn record_writing_activity(
    &self,
    origin: &str,
    workspace_id: &str,
    document_id: &str,
    source: ActivitySource,
    is_created: bool,
    edit_events: u64,
    words_added: u64,
    words_deleted: u64,
  ) -> Result<(), StoreError> {
    self.record_writing_activity_at(
      origin,
      workspace_id,
      document_id,
      source,
      is_created,
      edit_events,
      words_added,
      words_deleted,
      now_unix_secs(),
    )
  }

  /// Ensure all live documents under `origin` have up-to-date `word_count`,
  /// `char_count`, and `link_targets` projections (lazy backfill + stale fold).
  fn refresh_origin_stats_projections(&self, origin: &str) -> Result<(), StoreError> {
    let mut stmt = self.conn.prepare(
      "SELECT DISTINCT s.doc_id
             FROM local_view v
             JOIN doc_snapshot s ON s.doc_id = v.object_id AND s.origin = v.origin
             WHERE v.origin = ?1
               AND v.trashed = 0
               AND v.object_type <> 'folder'
               AND (s.projection_stale <> 0 OR s.word_count IS NULL OR s.char_count IS NULL)",
    )?;
    let pending: Vec<String> = stmt
      .query_map(params![origin], |r| r.get::<_, String>(0))?
      .collect::<Result<_, _>>()?;
    drop(stmt);
    for doc_id in pending {
      self.reindex_doc(origin, &doc_id)?;
    }
    Ok(())
  }

  /// Approximate SQLite database size in bytes (`page_count * page_size`).
  fn sqlite_storage_bytes(&self) -> u64 {
    let pages: i64 = self
      .conn
      .query_row("PRAGMA page_count", [], |r| r.get(0))
      .unwrap_or(0);
    let page_size: i64 = self
      .conn
      .query_row("PRAGMA page_size", [], |r| r.get(0))
      .unwrap_or(0);
    (pages.max(0) as u64).saturating_mul(page_size.max(0) as u64)
  }

  /// Compute the full [`UsageStatsOverview`] for all local workspaces under
  /// `origin` (typically `"local"`).
  pub fn usage_overview(
    &self,
    origin: &str,
    media_by_workspace: &HashMap<String, LocalWorkspaceMedia>,
  ) -> Result<UsageStatsOverview, StoreError> {
    self.refresh_origin_stats_projections(origin)?;

    // 1. Workspaces list (ordered by position, id).
    let mut ws_stmt = self.conn.prepare(
      "SELECT id, name, role FROM local_workspace WHERE origin=?1 ORDER BY position, id",
    )?;
    let workspaces_raw: Vec<(String, String, String)> = ws_stmt
      .query_map(params![origin], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
      .collect::<Result<_, _>>()?;
    drop(ws_stmt);

    // 2. Per-workspace view & snapshot aggregates (`folder` is the only container;
    //    both `'document'` and `'page'` represent leaf pages).
    let mut agg_stmt = self.conn.prepare(
      "SELECT v.workspace_id,
              SUM(CASE WHEN v.trashed = 0 AND v.object_type <> 'folder' THEN 1 ELSE 0 END) AS live_pages,
              SUM(CASE WHEN v.trashed = 0 AND v.object_type = 'folder'  THEN 1 ELSE 0 END) AS folders,
              SUM(CASE WHEN v.trashed = 1 AND v.object_type <> 'folder' THEN 1 ELSE 0 END) AS trashed_pages,
              SUM(CASE WHEN v.trashed = 0 AND v.object_type <> 'folder' THEN COALESCE(s.word_count, 0) ELSE 0 END) AS words,
              SUM(CASE WHEN v.trashed = 0 AND v.object_type <> 'folder' THEN COALESCE(s.char_count, 0) ELSE 0 END) AS chars,
              SUM(CASE WHEN v.trashed = 0 AND v.object_type <> 'folder' AND COALESCE(s.word_count, 0) = 0 THEN 1 ELSE 0 END) AS empty_pages,
              SUM(CASE WHEN v.trashed = 0 AND v.object_type <> 'folder' THEN COALESCE(LENGTH(s.state), 0) ELSE 0 END) AS state_bytes,
              MAX(CASE WHEN v.trashed = 0 AND v.object_type <> 'folder' THEN s.updated_at ELSE NULL END) AS max_updated_ms
       FROM local_view v
       LEFT JOIN doc_snapshot s ON s.origin = v.origin AND s.doc_id = v.object_id
       WHERE v.origin = ?1
       GROUP BY v.workspace_id",
    )?;
    struct WsAgg {
      live_pages: u64,
      folders: u64,
      trashed_pages: u64,
      words: u64,
      chars: u64,
      empty_pages: u64,
      state_bytes: u64,
      max_updated_ms: Option<i64>,
    }
    let mut agg_map: HashMap<String, WsAgg> = HashMap::new();
    let agg_rows = agg_stmt.query_map(params![origin], |r| {
      Ok((
        r.get::<_, String>(0)?,
        WsAgg {
          live_pages: r.get::<_, i64>(1)?.max(0) as u64,
          folders: r.get::<_, i64>(2)?.max(0) as u64,
          trashed_pages: r.get::<_, i64>(3)?.max(0) as u64,
          words: r.get::<_, i64>(4)?.max(0) as u64,
          chars: r.get::<_, i64>(5)?.max(0) as u64,
          empty_pages: r.get::<_, i64>(6)?.max(0) as u64,
          state_bytes: r.get::<_, i64>(7)?.max(0) as u64,
          max_updated_ms: r.get::<_, Option<i64>>(8)?,
        },
      ))
    })?;
    for row in agg_rows {
      let (ws_id, agg) = row?;
      agg_map.insert(ws_id, agg);
    }
    drop(agg_stmt);

    // 3. Per-workspace link edges & linked-node sets (same contract as `graph_local`).
    let mut edge_stmt = self.conn.prepare(
      "SELECT src.workspace_id, src.id, tgt.id
       FROM local_view src
       JOIN doc_snapshot s ON s.doc_id = src.object_id AND s.origin = src.origin
       JOIN json_each(s.link_targets) j
       JOIN local_view tgt ON tgt.id = j.value
           AND tgt.trashed = 0
           AND tgt.origin = src.origin
           AND tgt.workspace_id = src.workspace_id
           AND tgt.object_type <> 'folder'
       WHERE src.trashed = 0
         AND src.origin = ?1
         AND src.object_type <> 'folder'
         AND s.link_targets IS NOT NULL
         AND src.id <> j.value",
    )?;
    let mut edges_by_ws: HashMap<String, u64> = HashMap::new();
    let mut linked_by_ws: HashMap<String, HashSet<String>> = HashMap::new();
    let edge_rows = edge_stmt.query_map(params![origin], |r| {
      Ok((
        r.get::<_, String>(0)?,
        r.get::<_, String>(1)?,
        r.get::<_, String>(2)?,
      ))
    })?;
    for row in edge_rows {
      let (ws_id, src, tgt) = row?;
      *edges_by_ws.entry(ws_id.clone()).or_default() += 1;
      let set = linked_by_ws.entry(ws_id).or_default();
      set.insert(src);
      set.insert(tgt);
    }
    drop(edge_stmt);

    let mut summary = UsageSummary::default();
    let mut health = UsageHealth::default();
    let mut workspaces = Vec::with_capacity(workspaces_raw.len());

    // Include workspaces present in `local_workspace` (plus any workspace_id in
    // `local_view` if `local_workspace` has not been populated yet in tests).
    let mut ordered_ws: Vec<(String, String, String)> = workspaces_raw;
    let seen_ws: HashSet<String> = ordered_ws.iter().map(|(id, _, _)| id.clone()).collect();
    for ws_id in agg_map.keys() {
      if !seen_ws.contains(ws_id) {
        ordered_ws.push((ws_id.clone(), "本地工作区".to_string(), "owner".to_string()));
      }
    }

    let mut total_media_bytes = 0u64;
    for (ws_id, name, role) in ordered_ws {
      let agg = agg_map.remove(&ws_id);
      let live_pages = agg.as_ref().map_or(0, |a| a.live_pages);
      let folders = agg.as_ref().map_or(0, |a| a.folders);
      let trashed_pages = agg.as_ref().map_or(0, |a| a.trashed_pages);
      let words = agg.as_ref().map_or(0, |a| a.words);
      let chars = agg.as_ref().map_or(0, |a| a.chars);
      let empty_pages = agg.as_ref().map_or(0, |a| a.empty_pages);
      let state_bytes = agg.as_ref().map_or(0, |a| a.state_bytes);
      let updated_at = agg
        .as_ref()
        .and_then(|a| a.max_updated_ms)
        .map(|ms| format_unix_rfc3339(ms / 1000));

      let link_edges = edges_by_ws.get(&ws_id).copied().unwrap_or(0);
      let linked_nodes = linked_by_ws.get(&ws_id).map_or(0, |s| s.len() as u64);
      let orphan_pages = live_pages.saturating_sub(linked_nodes);

      let media = media_by_workspace.get(&ws_id).copied().unwrap_or_default();
      let ws_storage_bytes = media.storage_bytes.saturating_add(state_bytes);
      total_media_bytes = total_media_bytes.saturating_add(media.storage_bytes);

      summary.workspace_count += 1;
      summary.folder_count += folders;
      summary.live_page_count += live_pages;
      summary.trashed_page_count += trashed_pages;
      summary.total_words += words;
      summary.total_chars += chars;
      summary.image_count += media.image_count;
      summary.attachment_count += media.attachment_count;
      summary.storage_bytes = summary.storage_bytes.saturating_add(ws_storage_bytes);

      health.link_edges += link_edges;
      health.orphan_pages += orphan_pages;
      health.empty_pages += empty_pages;

      workspaces.push(WorkspaceUsageItem {
        workspace_id: ws_id,
        name,
        role,
        live_pages,
        folders,
        trashed_pages,
        words,
        chars,
        image_count: media.image_count,
        attachment_count: media.attachment_count,
        storage_bytes: ws_storage_bytes,
        quota_bytes: 0,
        link_edges,
        orphan_pages,
        empty_pages,
        public_shares: 0,
        unresolved_comments: 0,
        updated_at,
      });
    }

    // Ensure total storage reflects at least SQLite size + local blobs when DB is non-empty.
    let db_bytes = self.sqlite_storage_bytes();
    summary.storage_bytes = summary
      .storage_bytes
      .max(db_bytes.saturating_add(total_media_bytes));

    Ok(UsageStatsOverview {
      generated_at: format_unix_rfc3339(now_unix_secs()),
      summary,
      health,
      workspaces,
    })
  }

  /// Query local writing activity time series & 180-day heatmap folded into the
  /// caller's timezone (`tz_offset_minutes`).
  #[allow(clippy::too_many_arguments)]
  pub fn usage_activity_at(
    &self,
    origin: &str,
    range_label: &str,
    range_days: u32,
    tz_offset_minutes: i32,
    workspace_id: Option<&str>,
    include_imports: bool,
    now_utc_secs: i64,
  ) -> Result<UsageStatsActivity, StoreError> {
    let heatmap_days = 180u32;
    let lookback_days = i64::from(range_days.max(heatmap_days)) + 2;
    let since_hour = now_utc_secs - lookback_days * 86_400;

    let tracking_since: Option<i64> = match workspace_id {
      Some(ws_id) => self
        .conn
        .query_row(
          "SELECT MIN(bucket_hour) FROM local_writing_activity_hourly
           WHERE origin=?1 AND workspace_id=?2",
          params![origin, ws_id],
          |r| r.get(0),
        )
        .optional()?
        .flatten(),
      None => self
        .conn
        .query_row(
          "SELECT MIN(bucket_hour) FROM local_writing_activity_hourly WHERE origin=?1",
          params![origin],
          |r| r.get(0),
        )
        .optional()?
        .flatten(),
    };

    let rows: Vec<HourlyActivityRow> = match workspace_id {
      Some(ws_id) => {
        let mut stmt = self.conn.prepare(
          "SELECT bucket_hour, document_id, source, is_created, edit_events, words_added, words_deleted
           FROM local_writing_activity_hourly
           WHERE origin=?1 AND workspace_id=?2 AND bucket_hour >= ?3",
        )?;
        let iter = stmt.query_map(params![origin, ws_id, since_hour], map_hourly_row)?;
        iter.collect::<Result<Vec<_>, _>>()?
      }
      None => {
        let mut stmt = self.conn.prepare(
          "SELECT bucket_hour, document_id, source, is_created, edit_events, words_added, words_deleted
           FROM local_writing_activity_hourly
           WHERE origin=?1 AND bucket_hour >= ?2",
        )?;
        let iter = stmt.query_map(params![origin, since_hour], map_hourly_row)?;
        iter.collect::<Result<Vec<_>, _>>()?
      }
    };

    Ok(fold_hourly_to_daily(
      &rows,
      &[],
      tracking_since,
      range_label,
      range_days,
      heatmap_days,
      tz_offset_minutes,
      workspace_id.map(str::to_string),
      include_imports,
      now_utc_secs,
    ))
  }

  /// Convenience wrapper using wall-clock UTC time.
  pub fn usage_activity(
    &self,
    origin: &str,
    range_label: &str,
    range_days: u32,
    tz_offset_minutes: i32,
    workspace_id: Option<&str>,
    include_imports: bool,
  ) -> Result<UsageStatsActivity, StoreError> {
    self.usage_activity_at(
      origin,
      range_label,
      range_days,
      tz_offset_minutes,
      workspace_id,
      include_imports,
      now_unix_secs(),
    )
  }
}

fn map_hourly_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HourlyActivityRow> {
  let source_str: String = r.get(2)?;
  Ok(HourlyActivityRow {
    bucket_hour_secs: r.get(0)?,
    document_id: r.get(1)?,
    source: ActivitySource::from_db_str(&source_str),
    is_created: r.get::<_, i64>(3)? != 0,
    edit_events: r.get::<_, i64>(4)?.max(0) as u64,
    words_added: r.get::<_, i64>(5)?.max(0) as u64,
    words_deleted: r.get::<_, i64>(6)?.max(0) as u64,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::block::Block;
  use crate::doc::MicaDoc;
  use crate::store::{LocalView, LocalWorkspace};
  use serde_json::json;

  fn make_doc_with_text(text: &str) -> MicaDoc {
    MicaDoc::from_blocks(
      "root",
      &[
        Block {
          id: "root".into(),
          kind: "document".into(),
          text: String::new(),
          data: json!({}),
          children: vec!["b1".into()],
        },
        Block {
          id: "b1".into(),
          kind: "paragraph".into(),
          text: text.into(),
          data: json!({}),
          children: vec![],
        },
      ],
    )
  }

  #[test]
  fn overview_counts_words_empty_pages_orphans_and_links() {
    let store = LocalStore::open_in_memory().unwrap();
    store
      .save_workspace(&LocalWorkspace {
        id: "ws-1".into(),
        name: "个人知识库".into(),
        position: "0000000010".into(),
        origin: "local".into(),
        role: "owner".into(),
      })
      .unwrap();

    // Folder + 3 live pages (2 linked, 1 empty orphan) + 1 trashed page.
    store
      .save_view(&LocalView {
        id: "f1".into(),
        workspace_id: "ws-1".into(),
        parent_id: None,
        object_id: "f1".into(),
        name: "笔记".into(),
        position: "0000000010".into(),
        trashed: false,
        origin: "local".into(),
        object_type: "folder".into(),
      })
      .unwrap();
    for (id, name, trashed) in [
      ("v1", "页面一", false),
      ("v2", "页面二", false),
      ("v3", "空白页", false),
      ("v4", "已删除", true),
    ] {
      store
        .save_view(&LocalView {
          id: id.into(),
          workspace_id: "ws-1".into(),
          parent_id: Some("f1".into()),
          object_id: id.into(),
          name: name.into(),
          position: "0000000020".into(),
          trashed,
          origin: "local".into(),
          object_type: "document".into(),
        })
        .unwrap();
    }

    // v1 links to v2 and has "你好 world" (3 words)
    let d1 = MicaDoc::from_blocks(
      "root",
      &[
        Block {
          id: "root".into(),
          kind: "document".into(),
          text: String::new(),
          data: json!({}),
          children: vec!["b1".into()],
        },
        Block {
          id: "b1".into(),
          kind: "paragraph".into(),
          text: "你好 world".into(),
          data: json!({
            "marks": [
              { "type": "link", "start": 0, "end": 2, "href": "mica://page/v2" }
            ]
          }),
          children: vec![],
        },
      ],
    );
    store.save_doc("local", "v1", &d1).unwrap();

    // v2 has "Rust开发" (3 words)
    store
      .save_doc("local", "v2", &make_doc_with_text("Rust开发"))
      .unwrap();
    // v3 is empty (0 words)
    store
      .save_doc("local", "v3", &make_doc_with_text(""))
      .unwrap();
    // v4 is trashed with 100 words (must not be counted in live totals)
    store
      .save_doc("local", "v4", &make_doc_with_text("一二三四五"))
      .unwrap();

    let overview = store.usage_overview("local", &HashMap::new()).unwrap();
    // `LocalStore::open` seeds the default `"local"` workspace plus our `"ws-1"`.
    assert_eq!(overview.summary.workspace_count, 2);
    assert_eq!(overview.summary.folder_count, 1);
    assert_eq!(overview.summary.live_page_count, 3);
    assert_eq!(overview.summary.trashed_page_count, 1);
    assert_eq!(overview.summary.total_words, 6);
    assert_eq!(overview.health.empty_pages, 1);
    assert_eq!(overview.health.link_edges, 1);
    assert_eq!(overview.health.orphan_pages, 1);
    let ws1 = overview
      .workspaces
      .iter()
      .find(|w| w.workspace_id == "ws-1")
      .expect("ws-1 present");
    assert_eq!(ws1.words, 6);
    assert_eq!(ws1.orphan_pages, 1);
    assert_eq!(ws1.link_edges, 1);
  }

  #[test]
  fn activity_records_merges_hours_deduplicates_and_filters_imports() {
    let store = LocalStore::open_in_memory().unwrap();
    let now = 20_562 * 86_400 + 10 * 3_600; // 2026-04-19T10:00:00Z

    // Manual creation + two edits in the same hour and next hour on same doc
    store
      .record_writing_activity_at(
        "local",
        "ws-1",
        "doc-1",
        ActivitySource::Manual,
        true,
        1,
        50,
        0,
        now - 7_200,
      )
      .unwrap();
    store
      .record_writing_activity_at(
        "local",
        "ws-1",
        "doc-1",
        ActivitySource::Manual,
        false,
        2,
        30,
        10,
        now - 7_100,
      )
      .unwrap();
    store
      .record_writing_activity_at(
        "local",
        "ws-1",
        "doc-1",
        ActivitySource::Manual,
        false,
        1,
        20,
        5,
        now - 3_600,
      )
      .unwrap();
    // Bulk import in same workspace
    store
      .record_writing_activity_at(
        "local",
        "ws-1",
        "doc-imp",
        ActivitySource::Import,
        true,
        0,
        800,
        0,
        now - 1_800,
      )
      .unwrap();

    let excl = store
      .usage_activity_at("local", "7d", 7, 0, Some("ws-1"), false, now)
      .unwrap();
    assert_eq!(excl.totals.pages_created, 1);
    assert_eq!(excl.totals.pages_edited, 1);
    assert_eq!(excl.totals.words_added, 100);
    assert_eq!(excl.totals.words_deleted, 15);
    assert_eq!(excl.totals.net_words, 85);
    assert_eq!(excl.totals.active_days, 1);
    assert_eq!(excl.totals.current_streak, 1);

    let incl = store
      .usage_activity_at("local", "7d", 7, 0, Some("ws-1"), true, now)
      .unwrap();
    assert_eq!(incl.totals.pages_created, 2);
    assert_eq!(incl.totals.words_added, 900);
  }
}
