//! Pure-Rust Usage Statistics & Knowledge Observability Dashboard (Issue #191).
//!
//! Renders inside Settings → `使用统计` (`TAB_USAGE_STATS`) and is also reachable
//! directly from the bottom-left Account Menu (`使用统计`). Supports both cloud
//! mode (`/api/stats/overview` + `/api/stats/activity`) and local mode
//! (`VaultStore::usage_overview` + `VaultStore::usage_activity`), dark & light
//! themes, workspace drill-down, interactive trend bar chart, 180-day activity
//! heatmap, knowledge health pills, and a sortable workspace breakdown table.

use std::rc::Rc;

use mica_core::{
  DailyActivityPoint, HeatmapDayPoint, UsageStatsActivity, UsageStatsOverview, WorkspaceUsageItem,
};
use nsh_base::button;
use nsh_core::{
  color::{Rgba, rgba},
  geometry::px,
  prelude::*,
};

use crate::{
  mica_icons,
  mica_theme::{MicaPalette, palette, tr},
  settings_view::format_bytes_human,
};

/// Metric plotted by the Writing Trend bar chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendMetric {
  PagesCreated,
  PagesEdited,
  WordsAdded,
}

/// Sort column for the Workspaces Detail table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSortKey {
  Pages,
  Words,
  Storage,
  Updated,
}

/// Reactive state & callbacks passed from `lib.rs` into Settings → `使用统计`.
#[derive(Clone)]
pub struct UsageStatsProps {
  pub dark_mode: Signal<bool>,
  pub is_cloud: Signal<bool>,
  pub loading: Signal<bool>,
  pub error: Signal<Option<String>>,
  pub overview: Signal<Option<UsageStatsOverview>>,
  pub activity: Signal<Option<UsageStatsActivity>>,
  pub selected_workspace_id: Signal<Option<String>>,
  pub selected_range: Signal<String>,
  pub include_imports: Signal<bool>,
  pub on_refresh: Rc<dyn Fn()>,
  pub on_select_workspace: Rc<dyn Fn(Option<String>)>,
  pub on_select_range: Rc<dyn Fn(String)>,
  pub on_toggle_imports: Rc<dyn Fn(bool)>,
}

impl UsageStatsProps {
  /// Inert fallback props for legacy callers that do not wire usage stats.
  pub fn empty(dark_mode: Signal<bool>) -> Self {
    Self {
      dark_mode,
      is_cloud: signal(false),
      loading: signal(false),
      error: signal(None),
      overview: signal(None),
      activity: signal(None),
      selected_workspace_id: signal(None),
      selected_range: signal("30d".to_string()),
      include_imports: signal(false),
      on_refresh: Rc::new(|| {}),
      on_select_workspace: Rc::new(|_| {}),
      on_select_range: Rc::new(|_| {}),
      on_toggle_imports: Rc::new(|_| {}),
    }
  }
}

/// Format large counts with thin commas (`12,345`) for readable tabular KPIs.
pub fn format_count(n: u64) -> String {
  let digits = n.to_string();
  let mut out = String::with_capacity(digits.len() + digits.len() / 3);
  for (idx, ch) in digits.chars().rev().enumerate() {
    if idx > 0 && idx % 3 == 0 {
      out.push(',');
    }
    out.push(ch);
  }
  out.chars().rev().collect()
}

/// Format a signed word delta (`+120`, `-15`, `0`).
pub fn format_signed_count(n: i64) -> String {
  if n > 0 {
    format!("+{}", format_count(n as u64))
  } else if n < 0 {
    format!("-{}", format_count(n.unsigned_abs()))
  } else {
    "0".to_string()
  }
}

/// Compute integer percentage `part / total * 100` (returns `0` when `total == 0`).
pub fn percent_u64(part: u64, total: u64) -> u64 {
  part
    .saturating_mul(100)
    .saturating_add(total / 2)
    .checked_div(total)
    .unwrap_or(0)
}

/// Sort workspaces by `key` descending (ties broken by workspace name ascending).
pub fn sort_workspaces(
  items: &[WorkspaceUsageItem],
  key: WorkspaceSortKey,
) -> Vec<WorkspaceUsageItem> {
  let mut out = items.to_vec();
  out.sort_by(|a, b| {
    let ord = match key {
      WorkspaceSortKey::Pages => b.live_pages.cmp(&a.live_pages),
      WorkspaceSortKey::Words => b.words.cmp(&a.words),
      WorkspaceSortKey::Storage => b.storage_bytes.cmp(&a.storage_bytes),
      WorkspaceSortKey::Updated => b.updated_at.cmp(&a.updated_at),
    };
    ord.then_with(|| a.name.cmp(&b.name))
  });
  out
}

/// Split a flat 180-day heatmap slice into 7-day columns (oldest → newest).
pub fn heatmap_columns(days: &[HeatmapDayPoint]) -> Vec<Vec<HeatmapDayPoint>> {
  if days.is_empty() {
    return Vec::new();
  }
  let mut cols: Vec<Vec<HeatmapDayPoint>> = Vec::with_capacity(days.len().div_ceil(7));
  for chunk in days.chunks(7) {
    cols.push(chunk.to_vec());
  }
  cols
}

fn heatmap_cell_color(level: u8, dark: bool) -> Rgba {
  if dark {
    match level {
      0 => rgba(0x2629_30FF),
      1 => rgba(0x164E_3BFF),
      2 => rgba(0x1E7B_57FF),
      3 => rgba(0x2EA6_72FF),
      _ => rgba(0x4ADE_94FF),
    }
  } else {
    match level {
      0 => rgba(0xEBEE_F2FF),
      1 => rgba(0xB8E6_CDFF),
      2 => rgba(0x6FCF_9EFF),
      3 => rgba(0x32A8_6CFF),
      _ => rgba(0x1679_4BFF),
    }
  }
}

fn bar_fill_color(active: bool, dark: bool) -> Rgba {
  if dark {
    if active {
      rgba(0x7CB3_FFFF)
    } else {
      rgba(0x5B9B_F5CC)
    }
  } else if active {
    rgba(0x1F65_D6FF)
  } else {
    rgba(0x3B82_F6CC)
  }
}

fn short_updated_date(ts: Option<&str>) -> String {
  match ts {
    Some(s) if s.len() >= 10 => s[..10].to_string(),
    Some(s) if !s.is_empty() => s.to_string(),
    _ => "—".to_string(),
  }
}

/// Main Usage Statistics panel rendered inside Settings right detail pane.
pub fn usage_stats_panel(props: UsageStatsProps) -> impl IntoElement {
  let dm = props.dark_mode;
  let trend_metric = signal(TrendMetric::WordsAdded);
  let sort_key = signal(WorkspaceSortKey::Pages);
  let hovered_bar_idx: Signal<Option<usize>> = signal(None);
  let hovered_heat: Signal<Option<HeatmapDayPoint>> = signal(None);

  let p_toolbar = props.clone();
  let p_error = props.clone();
  let p_kpis = props.clone();
  let p_trend = props.clone();
  let p_heat = props.clone();
  let p_health = props.clone();
  let p_table = props.clone();

  view()
    .test_id("usage-stats-panel")
    .w_full()
    .gap(px(18.0))
    // 1. Top Toolbar: workspace filter chips + mode badge + refresh button
    .child(toolbar_row(p_toolbar))
    // 2. Error banner (with retry)
    .child(when(
      {
        let err = p_error.error;
        move || err.get().is_some()
      },
      {
        let err = p_error.error;
        let retry = p_error.on_refresh.clone();
        move || {
          let retry_cb = retry.clone();
          view()
            .test_id("usage-stats-error")
            .w_full()
            .px(px(14.0))
            .py(px(10.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(move || palette(dm.get()).danger)
            .bg(move || palette(dm.get()).surface_raised)
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .child(
              text(move || err.get().unwrap_or_default())
                .text_size(px(12.5))
                .text_color(move || palette(dm.get()).danger),
            )
            .child(
              button()
                .test_id("usage-stats-retry-btn")
                .accessibility_label("重试加载统计")
                .px(px(10.0))
                .py(px(5.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(move || palette(dm.get()).border_normal)
                .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
                .on_press(move || retry_cb())
                .child(
                  text(move || tr("重试", "Retry"))
                    .text_size(px(12.0))
                    .text_color(move || palette(dm.get()).text_primary),
                ),
            )
        }
      },
    ))
    // 3. 5 KPI Cards
    .child(kpi_cards_section(p_kpis))
    // 4. Writing Trend Card
    .child(writing_trend_card(p_trend, trend_metric, hovered_bar_idx))
    // 5. Activity Heatmap (180 days)
    .child(heatmap_card(p_heat, hovered_heat))
    // 6. Knowledge Health Card
    .child(health_card(p_health))
    // 7. Workspaces Breakdown Table
    .child(workspaces_table_card(p_table, sort_key))
}

fn toolbar_row(props: UsageStatsProps) -> impl IntoElement {
  let dm = props.dark_mode;
  let overview = props.overview;
  let selected_ws = props.selected_workspace_id;
  let loading = props.loading;
  let is_cloud = props.is_cloud;
  let on_select_ws = props.on_select_workspace.clone();
  let on_refresh = props.on_refresh.clone();

  view()
    .w_full()
    .flex_row()
    .flex_wrap()
    .items_center()
    .justify_between()
    .gap(px(10.0))
    .child(
      // Workspace scope pills
      dynamic(move || {
        let ov = overview.get();
        let active_ws = selected_ws.get();
        let dark = dm.get();
        let pal = palette(dark);
        let on_sel_all = on_select_ws.clone();

        let mut row = view().flex_row().flex_wrap().items_center().gap(px(6.0));
        let all_active = active_ws.is_none();
        row = row.child(
          button()
            .test_id("usage-stats-ws-all")
            .accessibility_label("全部工作区")
            .px(px(10.0))
            .py(px(5.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(if all_active { pal.accent } else { pal.border_subtle })
            .bg(if all_active { pal.accent_soft } else { pal.surface_raised })
            .on_press(move || on_sel_all(None))
            .child(
              text(tr("全部工作区", "All Workspaces"))
                .text_size(px(12.0))
                .font_weight(if all_active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                .text_color(if all_active { pal.accent } else { pal.text_primary }),
            ),
        );

        if let Some(ov) = ov {
          for ws in ov.workspaces {
            let is_active = active_ws.as_deref() == Some(ws.workspace_id.as_str());
            let ws_id = ws.workspace_id.clone();
            let ws_tid = format!("usage-stats-ws-{}", ws.workspace_id);
            let on_sel = on_select_ws.clone();
            row = row.child(
              button()
                .test_id(&ws_tid)
                .accessibility_label(ws.name.clone())
                .px(px(10.0))
                .py(px(5.0))
                .rounded(px(7.0))
                .border_1()
                .border_color(if is_active { pal.accent } else { pal.border_subtle })
                .bg(if is_active { pal.accent_soft } else { pal.surface_raised })
                .on_press(move || on_sel(Some(ws_id.clone())))
                .child(
                  text(ws.name)
                    .text_size(px(12.0))
                    .font_weight(if is_active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                    .text_color(if is_active { pal.accent } else { pal.text_primary }),
                ),
            );
          }
        }
        row
      }),
    )
    .child(
      // Right: mode badge + refresh button
      view()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
          view()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .bg(move || palette(dm.get()).surface_raised)
            .child(
              text(move || {
                if is_cloud.get() {
                  tr("云端实时统计", "Cloud Stats")
                } else {
                  tr("本地离线统计", "Local Stats")
                }
              })
              .text_size(px(11.5))
              .text_color(move || palette(dm.get()).text_muted),
            ),
        )
        .child(
          button()
            .test_id("usage-stats-refresh-btn")
            .accessibility_label("刷新使用统计")
            .px(px(10.0))
            .py(px(5.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(move || palette(dm.get()).border_subtle)
            .bg(move || palette(dm.get()).surface_raised)
            .hover(move |s| s.bg(palette(dm.get_untracked()).surface_hover))
            .on_press(move || on_refresh())
            .flex_row()
            .items_center()
            .gap(px(5.0))
            .child(dynamic(move || mica_icons::refresh_icon(palette(dm.get()).text_muted_hex)))
            .child(
              text(move || {
                if loading.get() {
                  tr("刷新中…", "Refreshing…")
                } else {
                  tr("刷新", "Refresh")
                }
              })
              .text_size(px(12.0))
              .text_color(move || palette(dm.get()).text_primary),
            ),
        ),
    )
}

fn kpi_cards_section(props: UsageStatsProps) -> impl IntoElement {
  let dm = props.dark_mode;
  let overview = props.overview;
  let selected_ws = props.selected_workspace_id;
  let is_cloud = props.is_cloud;

  dynamic(move || {
    let dark = dm.get();
    let pal = palette(dark);
    let cloud = is_cloud.get();
    let ov = overview.get();
    let ws_filter = selected_ws.get();

    // Compute scoped KPIs (global or single drilled-in workspace).
    let (
      ws_headline,
      ws_sub,
      live_pages,
      total_words,
      total_chars,
      image_count,
      attachment_count,
      storage_bytes,
      quota_bytes,
    ) = if let Some(ov) = ov {
      if let Some(ref target_id) = ws_filter
        && let Some(ws) = ov.workspaces.iter().find(|w| &w.workspace_id == target_id)
      {
        (
          ws.name.clone(),
          format!("{} 个文件夹", format_count(ws.folders)),
          ws.live_pages,
          ws.words,
          ws.chars,
          ws.image_count,
          ws.attachment_count,
          ws.storage_bytes,
          ws.quota_bytes,
        )
      } else {
        (
          format_count(ov.summary.workspace_count),
          format!("{} 个文件夹", format_count(ov.summary.folder_count)),
          ov.summary.live_page_count,
          ov.summary.total_words,
          ov.summary.total_chars,
          ov.summary.image_count,
          ov.summary.attachment_count,
          ov.summary.storage_bytes,
          ov.summary.quota_bytes_total,
        )
      }
    } else {
      ("0".into(), "0 个文件夹".into(), 0, 0, 0, 0, 0, 0, 0)
    };

    let avg_words = total_words.checked_div(live_pages).unwrap_or(0);
    let (storage_sub, quota_ratio_opt) = if !cloud {
      (tr("本地存储 · 无配额上限", "Local vault · No quota").to_string(), None)
    } else if ws_filter.is_some() && quota_bytes > 0 {
      let pct = percent_u64(storage_bytes, quota_bytes).min(100);
      let ratio = (storage_bytes as f64 / quota_bytes as f64).clamp(0.0, 1.0) as f32;
      (format!("配额 {} · 已用 {pct}%", format_bytes_human(quota_bytes)), Some(ratio))
    } else if let Some(ref ov_ref) = overview.get_untracked()
      && let Some(per_ws_quota) = ov_ref.workspaces.iter().map(|w| w.quota_bytes).find(|&q| q > 0)
    {
      let peak_bytes = ov_ref.workspaces.iter().map(|w| w.storage_bytes).max().unwrap_or(0);
      let peak_pct = percent_u64(peak_bytes, per_ws_quota).min(100);
      let ratio = (peak_bytes as f64 / per_ws_quota as f64).clamp(0.0, 1.0) as f32;
      (
        format!("单区配额 {} · 峰值 {peak_pct}%", format_bytes_human(per_ws_quota)),
        Some(ratio),
      )
    } else {
      (tr("无存储配额上限", "No storage quota").to_string(), None)
    };

    view()
      .test_id("usage-stats-kpis")
      .w_full()
      .flex_row()
      .flex_wrap()
      .gap(px(10.0))
      .child(kpi_card(
        "usage-stats-kpi-workspaces",
        tr("工作区", "Workspaces"),
        &ws_headline,
        &ws_sub,
        None,
        pal,
      ))
      .child(kpi_card(
        "usage-stats-kpi-pages",
        tr("活跃页面", "Live Pages"),
        &format_count(live_pages),
        &format!("篇均 {} 词", format_count(avg_words)),
        None,
        pal,
      ))
      .child(kpi_card(
        "usage-stats-kpi-words",
        tr("总字数", "Total Words"),
        &format_count(total_words),
        &format!("{} 非空白字符", format_count(total_chars)),
        None,
        pal,
      ))
      .child(kpi_card(
        "usage-stats-kpi-media",
        tr("图片 / 附件", "Media"),
        &format_count(image_count + attachment_count),
        &format!("{} 图 · {} 附件", format_count(image_count), format_count(attachment_count)),
        None,
        pal,
      ))
      .child(kpi_card(
        "usage-stats-kpi-storage",
        tr("已用存储", "Storage"),
        &format_bytes_human(storage_bytes),
        &storage_sub,
        quota_ratio_opt,
        pal,
      ))
  })
}

fn kpi_card(
  tid: &'static str,
  label: &str,
  value: &str,
  subtitle: &str,
  progress: Option<f32>,
  pal: MicaPalette,
) -> impl IntoElement {
  let mut card = view()
    .test_id(tid)
    .flex_1()
    .min_w(px(148.0))
    .px(px(14.0))
    .py(px(12.0))
    .rounded(px(10.0))
    .border_1()
    .border_color(pal.border_subtle)
    .bg(pal.surface_raised)
    .gap(px(4.0))
    .child(text(label.to_string()).text_size(px(12.0)).text_color(pal.text_muted))
    .child(
      text(value.to_string())
        .truncate()
        .text_size(px(20.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(pal.text_primary),
    )
    .child(text(subtitle.to_string()).text_size(px(11.5)).text_color(pal.text_faint));

  if let Some(ratio) = progress {
    let fill_color = if ratio >= 0.85 { pal.danger } else { pal.accent };
    card = card.child(
      view()
        .mt(px(4.0))
        .w_full()
        .h(px(5.0))
        .rounded(px(3.0))
        .bg(pal.surface_sunken)
        .overflow_hidden()
        .child(view().h_full().w(relative(ratio)).rounded(px(3.0)).bg(fill_color)),
    );
  }
  card
}

fn writing_trend_card(
  props: UsageStatsProps,
  trend_metric: Signal<TrendMetric>,
  hovered_bar_idx: Signal<Option<usize>>,
) -> impl IntoElement {
  let dm = props.dark_mode;
  let activity = props.activity;
  let selected_range = props.selected_range;
  let include_imports = props.include_imports;
  let on_select_range = props.on_select_range.clone();
  let on_toggle_imports = props.on_toggle_imports.clone();

  view()
    .test_id("usage-stats-trend")
    .w_full()
    .p(px(16.0))
    .rounded(px(10.0))
    .border_1()
    .border_color(move || palette(dm.get()).border_subtle)
    .bg(move || palette(dm.get()).surface_raised)
    .gap(px(12.0))
    // Card header: title + range selector + metric selector + include-imports checkbox
    .child(
      view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
          text(move || tr("写作趋势", "Writing Trend"))
            .text_size(px(14.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || palette(dm.get()).text_primary),
        )
        .child(
          view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            // Range pills
            .child(range_pills(dm, selected_range, on_select_range))
            // Metric pills
            .child(metric_pills(dm, trend_metric))
            // Include imports toggle
            .child(
              button()
                .test_id("usage-stats-include-imports")
                .accessibility_label("包含导入与转移记录")
                .px(px(8.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(move || {
                  if include_imports.get() {
                    palette(dm.get()).accent
                  } else {
                    palette(dm.get()).border_subtle
                  }
                })
                .bg(move || {
                  if include_imports.get() {
                    palette(dm.get()).accent_soft
                  } else {
                    palette(dm.get()).surface_base
                  }
                })
                .on_press(move || on_toggle_imports(!include_imports.get_untracked()))
                .child(
                  text(move || {
                    if include_imports.get() {
                      tr("☑ 含导入/转移", "☑ Imports")
                    } else {
                      tr("☐ 含导入/转移", "☐ Imports")
                    }
                  })
                  .text_size(px(11.5))
                  .text_color(move || {
                    if include_imports.get() {
                      palette(dm.get()).accent
                    } else {
                      palette(dm.get()).text_muted
                    }
                  }),
                ),
            ),
        ),
    )
    // Summary strip
    .child(dynamic(move || {
      let pal = palette(dm.get());
      let Some(act) = activity.get() else {
        return view().child(
          text(tr("暂无写作活跃记录，创建或编辑页面后将自动累计。", "No writing activity recorded yet."))
            .text_size(px(12.0))
            .text_color(pal.text_muted),
        );
      };
      let t = &act.totals;
      let summary_text = format!(
        "新建 {} 篇 · 编辑 {} 篇 · 新增 +{} 词 · 净增 {} 词 · 活跃 {} 天",
        format_count(t.pages_created),
        format_count(t.pages_edited),
        format_count(t.words_added),
        format_signed_count(t.net_words),
        format_count(t.active_days),
      );
      let mut row = view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(text(summary_text).text_size(px(12.5)).text_color(pal.text_muted));
      if let Some(since) = act.tracking_since.as_deref() {
        let day_only = if since.len() >= 10 { &since[..10] } else { since };
        row = row.child(
          text(format!("编辑与字数变化自 {day_only} 起统计"))
            .text_size(px(11.5))
            .text_color(pal.text_faint),
        );
      }
      row
    }))
    // Interactive bar chart stage
    .child(dynamic(move || {
      let dark = dm.get();
      let pal = palette(dark);
      let metric = trend_metric.get();
      let hovered = hovered_bar_idx.get();
      let Some(act) = activity.get() else {
        return view()
          .w_full()
          .h(px(110.0))
          .items_center()
          .justify_center()
          .rounded(px(8.0))
          .bg(pal.surface_base)
          .child(text(tr("等待数据…", "Loading…")).text_size(px(12.0)).text_color(pal.text_faint));
      };
      if act.daily.is_empty() {
        return view().w_full().h(px(80.0));
      }

      let values: Vec<u64> = act
        .daily
        .iter()
        .map(|d| metric_value(d, metric))
        .collect();
      let max_val = values.iter().copied().max().unwrap_or(0);

      let mut bars_row = view()
        .w_full()
        .h(px(120.0))
        .px(px(6.0))
        .pt(px(10.0))
        .pb(px(4.0))
        .rounded(px(8.0))
        .bg(pal.surface_base)
        .border_1()
        .border_color(pal.border_subtle)
        .flex_row()
        .items_end()
        .gap(px(if act.daily.len() > 100 { 1.0 } else { 3.0 }));

      for (idx, val) in values.iter().copied().enumerate() {
        let is_hovered = hovered == Some(idx);
        let h_px = if max_val == 0 || val == 0 {
          3.0
        } else {
          ((val as f32 / max_val as f32) * 98.0).max(6.0)
        };
        let fill = if val == 0 {
          pal.surface_sunken
        } else {
          bar_fill_color(is_hovered, dark)
        };

        bars_row = bars_row.child(
          view()
            .flex_1()
            .min_w(px(1.5))
            .h_full()
            .justify_end()
            .on_mouse_enter(move |_: &MouseEvent| hovered_bar_idx.set(Some(idx)))
            .child(view().w_full().h(px(h_px)).rounded(px(2.5)).bg(fill)),
        );
      }

      // Axis labels (first / middle / last date) + hover tooltip readout
      let first_date = act.daily.first().map(|d| d.date.clone()).unwrap_or_default();
      let mid_date = act
        .daily
        .get(act.daily.len() / 2)
        .map(|d| d.date.clone())
        .unwrap_or_default();
      let last_date = act.daily.last().map(|d| d.date.clone()).unwrap_or_default();

      let hover_detail = hovered.and_then(|i| act.daily.get(i)).map(|d| {
        format!(
          "{} · 新建 {} 篇 · 编辑 {} 篇 · 新增 +{} 词 · 净增 {} 词",
          d.date,
          format_count(d.pages_created),
          format_count(d.pages_edited),
          format_count(d.words_added),
          format_signed_count(d.net_words),
        )
      });

      view()
        .w_full()
        .gap(px(6.0))
        .child(bars_row)
        .child(
          view()
            .w_full()
            .flex_row()
            .items_center()
            .justify_between()
            .child(text(first_date).text_size(px(11.0)).text_color(pal.text_faint))
            .child(
              text(hover_detail.unwrap_or(mid_date))
                .text_size(px(11.5))
                .text_color(if hovered.is_some() { pal.text_primary } else { pal.text_faint }),
            )
            .child(text(last_date).text_size(px(11.0)).text_color(pal.text_faint)),
        )
    }))
}

fn metric_value(d: &DailyActivityPoint, metric: TrendMetric) -> u64 {
  match metric {
    TrendMetric::PagesCreated => d.pages_created,
    TrendMetric::PagesEdited => d.pages_edited,
    TrendMetric::WordsAdded => d.words_added,
  }
}

fn range_pills(
  dm: Signal<bool>,
  selected_range: Signal<String>,
  on_select_range: Rc<dyn Fn(String)>,
) -> impl IntoElement {
  const RANGES: &[(&str, &str, &str)] = &[
    ("1d", "今天", "1D"),
    ("7d", "7 天", "7D"),
    ("30d", "30 天", "30D"),
    ("90d", "90 天", "90D"),
    ("365d", "1 年", "1Y"),
  ];
  let mut row = view()
    .flex_row()
    .items_center()
    .gap(px(3.0))
    .p(px(2.0))
    .rounded(px(7.0))
    .bg(move || palette(dm.get()).surface_base)
    .border_1()
    .border_color(move || palette(dm.get()).border_subtle);

  for &(key, zh, en) in RANGES {
    let on_sel = on_select_range.clone();
    let tid = format!("usage-stats-range-{key}");
    row = row.child(
      button()
        .test_id(&tid)
        .accessibility_label(zh)
        .px(px(7.0))
        .py(px(3.0))
        .rounded(px(5.0))
        .bg(move || {
          if selected_range.get() == key {
            palette(dm.get()).accent_soft
          } else {
            rgba(0x0000_0000)
          }
        })
        .on_press(move || on_sel(key.to_string()))
        .child(
          text(move || tr(zh, en))
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
              if selected_range.get() == key {
                palette(dm.get()).accent
              } else {
                palette(dm.get()).text_muted
              }
            }),
        ),
    );
  }
  row
}

fn metric_pills(dm: Signal<bool>, trend_metric: Signal<TrendMetric>) -> impl IntoElement {
  const METRICS: &[(TrendMetric, &str, &str, &str)] = &[
    (TrendMetric::PagesCreated, "usage-stats-metric-created", "新建页面", "Created"),
    (TrendMetric::PagesEdited, "usage-stats-metric-edited", "编辑页面", "Edited"),
    (TrendMetric::WordsAdded, "usage-stats-metric-words", "新增字数", "Words"),
  ];
  let mut row = view()
    .flex_row()
    .items_center()
    .gap(px(3.0))
    .p(px(2.0))
    .rounded(px(7.0))
    .bg(move || palette(dm.get()).surface_base)
    .border_1()
    .border_color(move || palette(dm.get()).border_subtle);

  for &(m, tid, zh, en) in METRICS {
    row = row.child(
      button()
        .test_id(tid)
        .accessibility_label(zh)
        .px(px(7.0))
        .py(px(3.0))
        .rounded(px(5.0))
        .bg(move || {
          if trend_metric.get() == m {
            palette(dm.get()).surface_hover
          } else {
            rgba(0x0000_0000)
          }
        })
        .on_press(move || trend_metric.set(m))
        .child(
          text(move || tr(zh, en))
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || {
              if trend_metric.get() == m {
                palette(dm.get()).text_primary
              } else {
                palette(dm.get()).text_muted
              }
            }),
        ),
    );
  }
  row
}

fn heatmap_card(
  props: UsageStatsProps,
  hovered_heat: Signal<Option<HeatmapDayPoint>>,
) -> impl IntoElement {
  let dm = props.dark_mode;
  let activity = props.activity;

  view()
    .test_id("usage-stats-heatmap")
    .w_full()
    .p(px(16.0))
    .rounded(px(10.0))
    .border_1()
    .border_color(move || palette(dm.get()).border_subtle)
    .bg(move || palette(dm.get()).surface_raised)
    .gap(px(10.0))
    .child(
      view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
          text(move || tr("活跃热力图（近半年）", "Activity Heatmap (180 Days)"))
            .text_size(px(14.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || palette(dm.get()).text_primary),
        )
        .child(
          text(move || {
            if let Some(act) = activity.get() {
              let active_180 = act.heatmap.iter().filter(|d| d.score > 0).count() as u64;
              format!(
                "活跃 {} 天 · 当前连续 {} 天 · 最长连续 {} 天",
                format_count(active_180),
                format_count(act.totals.current_streak),
                format_count(act.totals.longest_streak),
              )
            } else {
              "活跃 0 天 · 最长连续 0 天".to_string()
            }
          })
          .text_size(px(12.0))
          .text_color(move || palette(dm.get()).text_muted),
        ),
    )
    .child(dynamic(move || {
      let dark = dm.get();
      let pal = palette(dark);
      let Some(act) = activity.get() else {
        return view().w_full().h(px(90.0));
      };
      let cols = heatmap_columns(&act.heatmap);

      let mut grid = view().w_full().flex_row().items_start().gap(px(3.0));
      for col in cols {
        let mut col_view = view().gap(px(3.0));
        for pt in col {
          let bg = heatmap_cell_color(pt.level, dark);
          let pt_hover = pt.clone();
          col_view = col_view.child(
            view()
              .w(px(11.0))
              .h(px(11.0))
              .rounded(px(2.5))
              .bg(bg)
              .on_mouse_enter(move |_: &MouseEvent| hovered_heat.set(Some(pt_hover.clone()))),
          );
        }
        grid = grid.child(col_view);
      }

      // Legend + hover detail strip
      let mut legend = view().flex_row().items_center().gap(px(4.0)).child(
        text(tr("少", "Less")).text_size(px(11.0)).text_color(pal.text_faint),
      );
      for lvl in 0..=4u8 {
        legend = legend.child(
          view()
            .w(px(10.0))
            .h(px(10.0))
            .rounded(px(2.0))
            .bg(heatmap_cell_color(lvl, dark)),
        );
      }
      legend = legend.child(text(tr("多", "More")).text_size(px(11.0)).text_color(pal.text_faint));

      view()
        .w_full()
        .gap(px(8.0))
        .child(grid)
        .child(
          view()
            .w_full()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
              text(move || {
                if let Some(pt) = hovered_heat.get() {
                  format!(
                    "{} · 新建 {} 篇 · 编辑 {} 篇 · 新增 +{} 词",
                    pt.date,
                    format_count(pt.pages_created),
                    format_count(pt.pages_edited),
                    format_count(pt.words_added),
                  )
                } else {
                  tr("悬停格子查看当日详情", "Hover a cell for daily details").to_string()
                }
              })
              .text_size(px(11.5))
              .text_color(pal.text_muted),
            )
            .child(legend),
        )
    }))
}

fn health_card(props: UsageStatsProps) -> impl IntoElement {
  let dm = props.dark_mode;
  let overview = props.overview;
  let selected_ws = props.selected_workspace_id;

  dynamic(move || {
    let pal = palette(dm.get());
    let ov = overview.get();
    let ws_filter = selected_ws.get();

    let (live_pages, link_edges, orphan_pages, empty_pages, trashed_pages, public_shares, unresolved_comments) =
      if let Some(ov) = ov {
        if let Some(ref target_id) = ws_filter
          && let Some(ws) = ov.workspaces.iter().find(|w| &w.workspace_id == target_id)
        {
          (
            ws.live_pages,
            ws.link_edges,
            ws.orphan_pages,
            ws.empty_pages,
            ws.trashed_pages,
            ws.public_shares,
            ws.unresolved_comments,
          )
        } else {
          (
            ov.summary.live_page_count,
            ov.health.link_edges,
            ov.health.orphan_pages,
            ov.health.empty_pages,
            ov.summary.trashed_page_count,
            ov.health.public_shares,
            ov.health.unresolved_comments,
          )
        }
      } else {
        (0, 0, 0, 0, 0, 0, 0)
      };

    let orphan_pct = percent_u64(orphan_pages, live_pages);

    view()
      .test_id("usage-stats-health")
      .w_full()
      .p(px(16.0))
      .rounded(px(10.0))
      .border_1()
      .border_color(pal.border_subtle)
      .bg(pal.surface_raised)
      .gap(px(10.0))
      .child(
        text(tr("知识库健康度", "Knowledge Health"))
          .text_size(px(14.0))
          .font_weight(FontWeight::SEMIBOLD)
          .text_color(pal.text_primary),
      )
      .child(
        view()
          .w_full()
          .flex_row()
          .flex_wrap()
          .gap(px(8.0))
          .child(health_pill(tr("双向链接", "Links"), &format!("{} 条", format_count(link_edges)), pal))
          .child(health_pill(
            tr("孤岛页面", "Orphan Pages"),
            &format!("{} 篇 ({orphan_pct}%)", format_count(orphan_pages)),
            pal,
          ))
          .child(health_pill(tr("空页面", "Empty Pages"), &format!("{} 篇", format_count(empty_pages)), pal))
          .child(health_pill(tr("回收站", "Trash"), &format!("{} 篇", format_count(trashed_pages)), pal))
          .child(health_pill(
            tr("公开分享", "Public Shares"),
            &format!("{} 个", format_count(public_shares)),
            pal,
          ))
          .child(health_pill(
            tr("未解决评论", "Open Comments"),
            &format!("{} 条", format_count(unresolved_comments)),
            pal,
          )),
      )
  })
}

fn health_pill(
  label: &str,
  value: &str,
  pal: MicaPalette,
) -> impl IntoElement {
  view()
    .px(px(10.0))
    .py(px(6.0))
    .rounded(px(8.0))
    .border_1()
    .border_color(pal.border_subtle)
    .bg(pal.surface_base)
    .flex_row()
    .items_center()
    .gap(px(6.0))
    .child(text(label.to_string()).text_size(px(12.0)).text_color(pal.text_muted))
    .child(
      text(value.to_string())
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(pal.text_primary),
    )
}

fn workspaces_table_card(
  props: UsageStatsProps,
  sort_key: Signal<WorkspaceSortKey>,
) -> impl IntoElement {
  let dm = props.dark_mode;
  let overview = props.overview;
  let selected_ws = props.selected_workspace_id;
  let on_select_ws = props.on_select_workspace.clone();

  view()
    .test_id("usage-stats-table")
    .w_full()
    .p(px(16.0))
    .rounded(px(10.0))
    .border_1()
    .border_color(move || palette(dm.get()).border_subtle)
    .bg(move || palette(dm.get()).surface_raised)
    .gap(px(10.0))
    .child(
      view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .child(
          text(move || tr("工作区明细", "Workspaces Breakdown"))
            .text_size(px(14.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || palette(dm.get()).text_primary),
        )
        .child(
          text(move || tr("点击工作区行可下钻筛选上方统计", "Click a row to drill down"))
            .text_size(px(11.5))
            .text_color(move || palette(dm.get()).text_faint),
        ),
    )
    // Sortable column header row
    .child(
      view()
        .w_full()
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(6.0))
        .bg(move || palette(dm.get()).surface_base)
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
          view()
            .flex_1()
            .min_w(px(140.0))
            .child(text(move || tr("工作区名称", "Workspace")).text_size(px(11.5)).text_color(move || palette(dm.get()).text_muted)),
        )
        .child(sort_header_btn("usage-stats-sort-pages", "页面", "Pages", 72.0, WorkspaceSortKey::Pages, sort_key, dm))
        .child(sort_header_btn("usage-stats-sort-words", "字数", "Words", 86.0, WorkspaceSortKey::Words, sort_key, dm))
        .child(
          view()
            .w(px(74.0))
            .items_end()
            .child(text(move || tr("图/附件", "Media")).text_size(px(11.5)).text_color(move || palette(dm.get()).text_muted)),
        )
        .child(sort_header_btn("usage-stats-sort-storage", "存储", "Storage", 84.0, WorkspaceSortKey::Storage, sort_key, dm))
        .child(sort_header_btn("usage-stats-sort-updated", "最近更新", "Updated", 92.0, WorkspaceSortKey::Updated, sort_key, dm)),
    )
    // Table rows
    .child(dynamic(move || {
      let pal = palette(dm.get());
      let active_ws = selected_ws.get();
      let key = sort_key.get();
      let Some(ov) = overview.get() else {
        return view().py(px(12.0)).items_center().child(
          text(tr("暂无工作区数据", "No workspaces yet")).text_size(px(12.0)).text_color(pal.text_faint),
        );
      };
      let sorted = sort_workspaces(&ov.workspaces, key);
      let mut list = view().w_full().gap(px(4.0));
      for ws in sorted {
        let is_selected = active_ws.as_deref() == Some(ws.workspace_id.as_str());
        let ws_id = ws.workspace_id.clone();
        let on_sel = on_select_ws.clone();
        let row_tid = format!("usage-stats-row-{}", ws.workspace_id);
        list = list.child(
          button()
            .test_id(&row_tid)
            .accessibility_label(ws.name.clone())
            .w_full()
            .px(px(10.0))
            .py(px(8.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(if is_selected { pal.accent } else { rgba(0x0000_0000) })
            .bg(if is_selected { pal.accent_soft } else { pal.surface_base })
            .hover(move |s| s.bg(pal.surface_hover))
            .on_press(move || {
              if is_selected {
                on_sel(None);
              } else {
                on_sel(Some(ws_id.clone()));
              }
            })
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(
              view()
                .flex_1()
                .min_w(px(140.0))
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                  text(ws.name)
                    .truncate()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(pal.text_primary),
                )
                .child(
                  text(format!("({} 文件夹)", ws.folders))
                    .text_size(px(11.0))
                    .text_color(pal.text_faint),
                ),
            )
            .child(table_num_cell(&format_count(ws.live_pages), 72.0, pal.text_primary))
            .child(table_num_cell(&format_count(ws.words), 86.0, pal.text_primary))
            .child(table_num_cell(
              &format!("{}/{}", ws.image_count, ws.attachment_count),
              74.0,
              pal.text_muted,
            ))
            .child(table_num_cell(&format_bytes_human(ws.storage_bytes), 84.0, pal.text_primary))
            .child(table_num_cell(&short_updated_date(ws.updated_at.as_deref()), 92.0, pal.text_muted)),
        );
      }
      list
    }))
}

fn sort_header_btn(
  tid: &'static str,
  zh: &'static str,
  en: &'static str,
  width: f32,
  key: WorkspaceSortKey,
  sort_key: Signal<WorkspaceSortKey>,
  dm: Signal<bool>,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(zh)
    .w(px(width))
    .items_end()
    .on_press(move || sort_key.set(key))
    .child(
      text(move || {
        let base = tr(zh, en);
        if sort_key.get() == key {
          format!("{base} ↓")
        } else {
          base.to_string()
        }
      })
      .text_size(px(11.5))
      .font_weight(FontWeight::MEDIUM)
      .text_color(move || {
        if sort_key.get() == key {
          palette(dm.get()).accent
        } else {
          palette(dm.get()).text_muted
        }
      }),
    )
}

fn table_num_cell(val: &str, width: f32, color: Rgba) -> impl IntoElement {
  view()
    .w(px(width))
    .items_end()
    .child(text(val.to_string()).text_size(px(12.0)).text_color(color))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn formats_numbers_and_signed_deltas_cleanly() {
    assert_eq!(format_count(0), "0");
    assert_eq!(format_count(999), "999");
    assert_eq!(format_count(1_234), "1,234");
    assert_eq!(format_count(12_345_678), "12,345,678");
    assert_eq!(format_signed_count(42), "+42");
    assert_eq!(format_signed_count(-1_200), "-1,200");
    assert_eq!(format_signed_count(0), "0");
    assert_eq!(percent_u64(1, 3), 33);
    assert_eq!(percent_u64(0, 0), 0);
  }

  #[test]
  fn sorts_workspaces_by_pages_words_and_storage() {
    let mk = |id: &str, name: &str, pages: u64, words: u64, bytes: u64| WorkspaceUsageItem {
      workspace_id: id.into(),
      name: name.into(),
      role: "owner".into(),
      live_pages: pages,
      folders: 0,
      trashed_pages: 0,
      words,
      chars: words,
      image_count: 0,
      attachment_count: 0,
      storage_bytes: bytes,
      quota_bytes: 0,
      link_edges: 0,
      orphan_pages: 0,
      empty_pages: 0,
      public_shares: 0,
      unresolved_comments: 0,
      updated_at: None,
    };
    let items = vec![
      mk("a", "Alpha", 5, 100, 5000),
      mk("b", "Beta", 20, 50, 1000),
      mk("c", "Gamma", 10, 900, 2000),
    ];
    assert_eq!(sort_workspaces(&items, WorkspaceSortKey::Pages)[0].workspace_id, "b");
    assert_eq!(sort_workspaces(&items, WorkspaceSortKey::Words)[0].workspace_id, "c");
    assert_eq!(sort_workspaces(&items, WorkspaceSortKey::Storage)[0].workspace_id, "a");
  }

  #[test]
  fn heatmap_splits_180_days_into_26_week_columns() {
    let days: Vec<HeatmapDayPoint> = (0..180)
      .map(|i| HeatmapDayPoint {
        date: format!("2026-01-{:02}", (i % 28) + 1),
        score: i as u64,
        level: (i % 5) as u8,
        pages_created: 0,
        pages_edited: 1,
        words_added: 10,
      })
      .collect();
    let cols = heatmap_columns(&days);
    assert_eq!(cols.len(), 26);
    assert_eq!(cols[0].len(), 7);
    assert_eq!(cols[25].len(), 5);
  }
}
