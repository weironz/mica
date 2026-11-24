//! Two-pane Version History modal (`980×640`) + pure block-level diff engine —
//! 1:1 restoration of Flutter's `VersionHistoryDialog`, `version_diff.dart`,
//! and `version_data.dart`.
//!
//! Layout:
//! - Left pane (`flex_1`): diff legend bar (`新增` / `修改` / `删除` or `与上一版本无差异`)
//!   + scrollable read-only Markdown block preview with green/amber/red left accent
//!     bars & 10% tint washes (deleted blocks rendered as struck-through ghosts).
//! - Right pane (`300px`): header (`版本历史`, inline `保存检查点`, close `×`),
//!   grouped timeline (`今天` / `更早`), and bottom `恢复` action with 2-step
//!   inline confirmation.

use std::rc::Rc;

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Timelike};
use mica_md::{Block, ParseMode, parse_body};
use nsh_base::{button, input};
use nsh_core::{
  SubmitBehavior,
  color::Rgba,
  element::{ImageSrc, Svg, svg},
  geometry::{px, relative},
  prelude::*,
};
use serde_json::Value;

use crate::{mica_icons, mica_theme::palette};

/// Block-level diff classification for a version preview block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
  Unchanged,
  Added,
  Changed,
  Deleted,
}

/// One block in a version diff preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffBlock {
  pub kind: DiffKind,
  pub block_kind: String,
  pub text: String,
}

/// Timeline day-bucket for the version history list (`今天` / `更早`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionGroup {
  Today,
  Earlier,
}

/// Display metadata for one version entry in the timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionHistoryItem {
  pub id: String,
  pub label: String,
  pub is_named: bool,
  pub created_at: String,
  pub author: Option<String>,
  pub is_today: bool,
}

/// Reactive state & callbacks driving [`version_history_dialog`].
#[derive(Clone)]
pub struct VersionHistoryState {
  pub open: Signal<bool>,
  pub items: Signal<Vec<VersionHistoryItem>>,
  pub selected_id: Signal<Option<String>>,
  pub current_md: Signal<String>,
  pub prev_md: Signal<Option<String>>,
  pub loading: Signal<bool>,
  pub preview_loading: Signal<bool>,
  pub error: Signal<Option<String>>,
  pub on_select: Rc<dyn Fn(String)>,
  pub on_create_checkpoint: Rc<dyn Fn(String)>,
  pub on_restore: Rc<dyn Fn(String)>,
  pub on_close: Rc<dyn Fn()>,
}

const fn rgba_hex(hex: u32, a: f32) -> Rgba {
  Rgba {
    r: ((hex >> 16) & 0xff) as f32 / 255.0,
    g: ((hex >> 8) & 0xff) as f32 / 255.0,
    b: (hex & 0xff) as f32 / 255.0,
    a,
  }
}

const DIFF_ADDED_BAR: Rgba = rgba_hex(0x22C55E, 1.0);
const DIFF_ADDED_TINT: Rgba = rgba_hex(0x22C55E, 0.10);
const DIFF_CHANGED_BAR: Rgba = rgba_hex(0xF59E0B, 1.0);
const DIFF_CHANGED_TINT: Rgba = rgba_hex(0xF59E0B, 0.10);
const DIFF_DELETED_BAR: Rgba = rgba_hex(0xEF4444, 1.0);
const DIFF_DELETED_TINT: Rgba = rgba_hex(0xEF4444, 0.10);

/// Key-order-insensitive canonical encoding of a `serde_json::Value` so two
/// blocks carrying identical attributes in different key order never diff as
/// changed (1:1 with Flutter `version_diff.dart::_canonical`).
fn canonical_value(value: &Value) -> String {
  match value {
    Value::Null => "null".to_string(),
    Value::Bool(b) => b.to_string(),
    Value::Number(n) => n.to_string(),
    Value::String(s) => format!("{s:?}"),
    Value::Array(arr) => {
      let inner = arr.iter().map(canonical_value).collect::<Vec<_>>().join(",");
      format!("[{inner}]")
    }
    Value::Object(map) => {
      let mut keys: Vec<&String> = map.keys().collect();
      keys.sort();
      let inner = keys
        .into_iter()
        .map(|k| format!("{k}:{}", canonical_value(&map[k])))
        .collect::<Vec<_>>()
        .join(",");
      format!("{{{inner}}}")
    }
  }
}

fn block_display_text(block: &Block) -> String {
  if !block.text.is_empty() {
    return block.text.clone();
  }
  match block.kind.as_str() {
    "divider" => "———".to_string(),
    "image" => block
      .data
      .get("alt")
      .and_then(Value::as_str)
      .filter(|s| !s.is_empty())
      .or_else(|| block.data.get("url").and_then(Value::as_str))
      .map(|s| format!("[图片: {s}]"))
      .unwrap_or_else(|| "[图片]".to_string()),
    "table" => {
      if let Some(rows) = block.data.get("rows").and_then(Value::as_array) {
        let count = rows.len();
        format!("[表格 · {count} 行]")
      } else {
        "[表格]".to_string()
      }
    }
    _ => String::new(),
  }
}

#[derive(Clone, PartialEq, Eq)]
struct BlockFingerprint {
  block_kind: String,
  text: String,
  canonical_data: String,
}

impl BlockFingerprint {
  fn from_block(block: &Block) -> Self {
    Self {
      block_kind: block.kind.clone(),
      text: block_display_text(block),
      canonical_data: canonical_value(&block.data),
    }
  }
}

/// Parse a Markdown document into plain `DiffKind::Unchanged` preview blocks
/// (used when previewing the oldest version with no predecessor).
pub fn parse_plain_diff_blocks(curr_md: &str) -> Vec<DiffBlock> {
  if curr_md.trim().is_empty() {
    return Vec::new();
  }
  parse_body(curr_md, ParseMode::Document)
    .iter()
    .map(|b| DiffBlock {
      kind: DiffKind::Unchanged,
      block_kind: b.kind.clone(),
      text: block_display_text(b),
    })
    .collect()
}

/// Pure block-level diff between two Markdown versions (`prev_md` → `curr_md`).
///
/// Uses LCS over exact block fingerprints (`kind` + `text` + canonical `data`)
/// to anchor unchanged blocks, then pairs blocks inside each gap as `Changed`,
/// splicing extra predecessor blocks in place as `Deleted` ghosts and extra
/// current blocks as `Added`.
pub fn diff_markdown_blocks(prev_md: &str, curr_md: &str) -> Vec<DiffBlock> {
  let prev_blocks: Vec<BlockFingerprint> = if prev_md.trim().is_empty() {
    Vec::new()
  } else {
    parse_body(prev_md, ParseMode::Document)
      .iter()
      .map(BlockFingerprint::from_block)
      .collect()
  };
  let curr_blocks: Vec<BlockFingerprint> = if curr_md.trim().is_empty() {
    Vec::new()
  } else {
    parse_body(curr_md, ParseMode::Document)
      .iter()
      .map(BlockFingerprint::from_block)
      .collect()
  };

  let n = prev_blocks.len();
  let m = curr_blocks.len();

  // Standard DP LCS table.
  let mut dp = vec![vec![0usize; m + 1]; n + 1];
  for i in (0..n).rev() {
    for j in (0..m).rev() {
      if prev_blocks[i] == curr_blocks[j] {
        dp[i][j] = dp[i + 1][j + 1] + 1;
      } else {
        dp[i][j] = dp[i + 1][j].max(dp[i][j + 1]);
      }
    }
  }

  // Reconstruct matched anchor pairs `(prev_idx, curr_idx)`.
  let mut anchors: Vec<(usize, usize)> = Vec::new();
  let (mut i, mut j) = (0usize, 0usize);
  while i < n && j < m {
    if prev_blocks[i] == curr_blocks[j] {
      anchors.push((i, j));
      i += 1;
      j += 1;
    } else if dp[i + 1][j] >= dp[i][j + 1] {
      i += 1;
    } else {
      j += 1;
    }
  }
  anchors.push((n, m)); // sentinel end

  let mut out = Vec::with_capacity(n.max(m));
  let (mut pi, mut cj) = (0usize, 0usize);

  for (anchor_p, anchor_c) in anchors {
    let prev_gap = &prev_blocks[pi..anchor_p];
    let curr_gap = &curr_blocks[cj..anchor_c];
    let paired = prev_gap.len().min(curr_gap.len());

    // Paired slots inside a gap represent in-place modifications (`Changed`).
    for changed in curr_gap.iter().take(paired) {
      out.push(DiffBlock {
        kind: DiffKind::Changed,
        block_kind: changed.block_kind.clone(),
        text: changed.text.clone(),
      });
    }
    // Excess current blocks in the gap are `Added`.
    for added in &curr_gap[paired..] {
      out.push(DiffBlock {
        kind: DiffKind::Added,
        block_kind: added.block_kind.clone(),
        text: added.text.clone(),
      });
    }
    // Excess predecessor blocks in the gap are `Deleted` ghosts.
    for deleted in &prev_gap[paired..] {
      out.push(DiffBlock {
        kind: DiffKind::Deleted,
        block_kind: deleted.block_kind.clone(),
        text: deleted.text.clone(),
      });
    }

    // Emit the matched anchor itself (if not the end sentinel).
    if anchor_p < n && anchor_c < m {
      out.push(DiffBlock {
        kind: DiffKind::Unchanged,
        block_kind: curr_blocks[anchor_c].block_kind.clone(),
        text: curr_blocks[anchor_c].text.clone(),
      });
      pi = anchor_p + 1;
      cj = anchor_c + 1;
    }
  }

  out
}

/// Returns `true` if any block in `blocks` carries a non-`Unchanged` diff kind.
pub fn has_any_diff(blocks: &[DiffBlock]) -> bool {
  blocks.iter().any(|b| b.kind != DiffKind::Unchanged)
}

fn parse_local_timestamp(created_at: &str) -> Option<DateTime<Local>> {
  if let Ok(dt) = DateTime::parse_from_rfc3339(created_at) {
    return Some(dt.with_timezone(&Local));
  }
  for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M"] {
    if let Ok(naive) = NaiveDateTime::parse_from_str(created_at, fmt)
      && let Some(local) = Local.from_local_datetime(&naive).earliest()
    {
      return Some(local);
    }
  }
  None
}

/// Classify a version timestamp into `(VersionGroup, relative_label)`:
/// - Today → `(Today, "HH:mm")`
/// - Yesterday → `(Earlier, "昨天 HH:mm")`
/// - Older → `(Earlier, "{month} 月 {day} 日")`
/// - Unparseable → `(Earlier, raw)`
pub fn format_version_stamp(created_at: &str, now_local: DateTime<Local>) -> (VersionGroup, String) {
  let Some(when) = parse_local_timestamp(created_at) else {
    return (VersionGroup::Earlier, created_at.to_string());
  };
  let today = now_local.date_naive();
  let date = when.date_naive();
  let delta_days = (today - date).num_days();
  let hhmm = format!("{:02}:{:02}", when.hour(), when.minute());

  if delta_days <= 0 {
    (VersionGroup::Today, hhmm)
  } else if delta_days == 1 {
    (VersionGroup::Earlier, format!("昨天 {hhmm}"))
  } else {
    (
      VersionGroup::Earlier,
      format!("{} 月 {} 日", when.month(), when.day()),
    )
  }
}

/// Convenience wrapper calling [`format_version_stamp`] against `Local::now()`.
pub fn format_version_stamp_now(created_at: &str) -> (VersionGroup, String) {
  format_version_stamp(created_at, Local::now())
}

/// Split timeline items into `今天` / `更早` sections preserving incoming order.
pub fn group_version_items(
  items: &[VersionHistoryItem],
) -> Vec<(VersionGroup, Vec<VersionHistoryItem>)> {
  let mut today = Vec::new();
  let mut earlier = Vec::new();
  for item in items {
    if item.is_today {
      today.push(item.clone());
    } else {
      earlier.push(item.clone());
    }
  }
  let mut sections = Vec::new();
  if !today.is_empty() {
    sections.push((VersionGroup::Today, today));
  }
  if !earlier.is_empty() {
    sections.push((VersionGroup::Earlier, earlier));
  }
  sections
}

fn display_version_name(item: &VersionHistoryItem) -> String {
  if item.is_named && !item.label.trim().is_empty() {
    item.label.trim().to_string()
  } else {
    "自动保存".to_string()
  }
}

fn version_icon_svg(size_px: f32, color: &str, inner: &str) -> Svg {
  let doc = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{inner}</svg>"##
  );
  svg(ImageSrc::svg_markup(&doc))
    .w(px(size_px))
    .h(px(size_px))
}

fn schedule_icon(color: &str) -> Svg {
  version_icon_svg(
    16.0,
    color,
    r##"<circle cx="10" cy="10" r="6.5"/><path d="M10 6.5V10L12.5 11.5"/>"##,
  )
}

fn bookmark_icon(color: &str) -> Svg {
  version_icon_svg(
    16.0,
    color,
    r##"<path d="M5.5 3.5H14.5V16.5L10 13.5L5.5 16.5Z"/>"##,
  )
}

fn bookmark_add_icon(color: &str) -> Svg {
  version_icon_svg(
    16.0,
    color,
    r##"<path d="M5.5 3.5H14.5V16.5L10 13.5L5.5 16.5Z"/><path d="M10 6.5V10.5M8 8.5H12"/>"##,
  )
}

/// Centered two-pane `980×640` Version History modal dialog.
///
/// Renders nothing when `!state.open.get()`.
pub fn version_history_dialog(
  state: VersionHistoryState,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let creating_checkpoint = signal(false);
  let checkpoint_draft = signal(String::new());
  let confirm_restore_id: Signal<Option<String>> = signal(None);

  let open_sig = state.open;
  when(
    move || open_sig.get(),
    move || {
      let st_bg = state.clone();
      let st_left = state.clone();
      let st_right = state.clone();

      view()
        .test_id("version-history-backdrop")
        .absolute()
        .inset_0()
        .z_index(55)
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("version-history-backdrop-dismiss")
            .accessibility_label("关闭版本历史")
            .absolute()
            .inset_0()
            .on_press(move || {
              creating_checkpoint.set(false);
              confirm_restore_id.set(None);
              (st_bg.on_close)();
            }),
        )
        .child(
          view()
            .test_id("version-history-dialog")
            .w(px(980.0))
            .h(px(640.0))
            .max_w(relative(0.94))
            .max_h(relative(0.92))
            .flex_row()
            .rounded(px(12.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .overflow_hidden()
            .z_index(56)
            .child(preview_pane(st_left, dark_mode))
            .child(
              view()
                .w(px(1.0))
                .h_full()
                .flex_shrink_0()
                .bg(move || palette(dark_mode.get()).border_normal),
            )
            .child(timeline_pane(
              st_right,
              dark_mode,
              creating_checkpoint,
              checkpoint_draft,
              confirm_restore_id,
            )),
        )
    },
  )
}

fn preview_pane(state: VersionHistoryState, dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .test_id("version-preview-pane")
    .flex_1()
    .min_w(px(0.0))
    .h_full()
    .bg(move || palette(dark_mode.get()).surface_base)
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      let selected = state.selected_id.get();

      if selected.is_none() {
        return view()
          .flex_1()
          .h_full()
          .items_center()
          .justify_center()
          .p_6()
          .child(
            text("还没有版本。\n点右上角「保存检查点」把当前状态存下来。")
              .text_size(px(13.0))
              .text_color(p.text_muted)
              .text_center(),
          )
          .into_element();
      }

      if state.preview_loading.get() {
        return view()
          .flex_1()
          .h_full()
          .items_center()
          .justify_center()
          .child(
            text("正在加载版本预览…")
              .text_size(px(13.0))
              .text_color(p.text_muted),
          )
          .into_element();
      }

      if let Some(err) = state.error.get() {
        return view()
          .flex_1()
          .h_full()
          .items_center()
          .justify_center()
          .p_6()
          .child(
            text(format!("加载失败: {err}"))
              .text_size(px(13.0))
              .text_color(p.danger),
          )
          .into_element();
      }

      let curr_md = state.current_md.get();
      let prev_md = state.prev_md.get();
      let (has_prev, blocks) = match prev_md {
        Some(prev) => (true, diff_markdown_blocks(&prev, &curr_md)),
        None => (false, parse_plain_diff_blocks(&curr_md)),
      };
      let diff_present = has_any_diff(&blocks);

      let mut pane = view().flex_1().h_full();

      // Legend bar: omitted for oldest version (no predecessor), explicit
      // "no differences" note when identical, or green/amber/red legend.
      if has_prev && !diff_present {
        pane = pane.child(
          view()
            .test_id("version-diff-none-bar")
            .w_full()
            .px(px(32.0))
            .py(px(6.0))
            .bg(p.surface_raised)
            .border_b_1()
            .border_color(p.border_subtle)
            .child(
              text("与上一版本无差异")
                .text_size(px(12.0))
                .text_color(p.text_muted),
            ),
        );
      } else if has_prev && diff_present {
        pane = pane.child(
          view()
            .test_id("version-diff-legend-bar")
            .w_full()
            .px(px(32.0))
            .py(px(6.0))
            .flex_row()
            .items_center()
            .gap(px(14.0))
            .bg(p.surface_raised)
            .border_b_1()
            .border_color(p.border_subtle)
            .child(legend_dot(DIFF_ADDED_BAR, "新增", p.text_muted))
            .child(legend_dot(DIFF_CHANGED_BAR, "修改", p.text_muted))
            .child(legend_dot(DIFF_DELETED_BAR, "删除", p.text_muted)),
        );
      }

      let mut body = view().w_full().max_w(px(720.0)).gap(px(6.0));
      if blocks.is_empty() {
        body = body.child(
          text("（空文档）")
            .text_size(px(13.0))
            .text_color(p.text_ghost),
        );
      } else {
        for (idx, blk) in blocks.into_iter().enumerate() {
          body = body.child(diff_block_row(idx, blk, dark_mode));
        }
      }

      pane = pane.child(
        scroll().flex_1().min_h(px(0.0)).child(
          view()
            .w_full()
            .px(px(32.0))
            .py(px(24.0))
            .items_center()
            .child(body),
        ),
      );

      pane.into_element()
    }))
}

fn legend_dot(color: Rgba, label: &'static str, text_color: Rgba) -> impl IntoElement {
  view()
    .flex_row()
    .items_center()
    .gap(px(5.0))
    .child(view().w(px(10.0)).h(px(10.0)).rounded(px(2.0)).bg(color))
    .child(text(label).text_size(px(12.0)).text_color(text_color))
}

fn diff_block_row(idx: usize, block: DiffBlock, dark_mode: Signal<bool>) -> impl IntoElement {
  let _p = palette(dark_mode.get_untracked());
  let (tint, bar) = match block.kind {
    DiffKind::Added => (Some(DIFF_ADDED_TINT), Some(DIFF_ADDED_BAR)),
    DiffKind::Changed => (Some(DIFF_CHANGED_TINT), Some(DIFF_CHANGED_BAR)),
    DiffKind::Deleted => (Some(DIFF_DELETED_TINT), Some(DIFF_DELETED_BAR)),
    DiffKind::Unchanged => (None, None),
  };
  let is_deleted = block.kind == DiffKind::Deleted;

  let mut row = view()
    .test_id(&format!("version-diff-block-{idx}"))
    .w_full()
    .flex_row()
    .items_stretch()
    .rounded(px(4.0))
    .overflow_hidden();

  if let Some(bg_tint) = tint {
    row = row.bg(bg_tint);
  }
  if let Some(bar_color) = bar {
    row = row.child(view().w(px(3.0)).flex_shrink_0().bg(bar_color));
  }

  let content = render_preview_block(&block, is_deleted, dark_mode);
  row.child(
    view()
      .flex_1()
      .min_w(px(0.0))
      .px(px(10.0))
      .py(px(4.0))
      .child(content),
  )
}

fn render_preview_block(
  block: &DiffBlock,
  is_deleted: bool,
  dark_mode: Signal<bool>,
) -> AnyElement {
  let p = palette(dark_mode.get_untracked());
  let base_color = if is_deleted {
    p.text_muted
  } else {
    p.text_primary
  };

  match block.block_kind.as_str() {
    "divider" => view()
      .w_full()
      .py(px(8.0))
      .child(view().w_full().h(px(1.0)).bg(p.border_normal))
      .into_element(),
    "heading" => {
      let mut t = text(block.text.clone())
        .text_size(px(18.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(base_color);
      if is_deleted {
        t = t.line_through();
      }
      t.into_element()
    }
    "quote" => {
      let mut t = text(block.text.clone())
        .text_size(px(14.0))
        .text_color(p.text_muted);
      if is_deleted {
        t = t.line_through();
      }
      view()
        .w_full()
        .pl(px(12.0))
        .border_l(px(3.0))
        .border_color(p.quote_bar)
        .child(t)
        .into_element()
    }
    "code_block" | "math_block" => {
      let mut t = text(block.text.clone())
        .text_size(px(12.5))
        .text_color(base_color);
      if is_deleted {
        t = t.line_through();
      }
      view()
        .w_full()
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(6.0))
        .bg(p.code_bg)
        .child(t)
        .into_element()
    }
    "bulleted_list" => {
      let mut t = text(format!("• {}", block.text))
        .text_size(px(14.0))
        .text_color(base_color);
      if is_deleted {
        t = t.line_through();
      }
      view().pl(px(8.0)).child(t).into_element()
    }
    "numbered_list" => {
      let mut t = text(format!("1. {}", block.text))
        .text_size(px(14.0))
        .text_color(base_color);
      if is_deleted {
        t = t.line_through();
      }
      view().pl(px(8.0)).child(t).into_element()
    }
    "todo" => {
      let mut t = text(format!("☐ {}", block.text))
        .text_size(px(14.0))
        .text_color(base_color);
      if is_deleted {
        t = t.line_through();
      }
      view().pl(px(8.0)).child(t).into_element()
    }
    _ => {
      let mut t = text(block.text.clone())
        .text_size(px(14.0))
        .text_color(base_color);
      if is_deleted {
        t = t.line_through();
      }
      t.into_element()
    }
  }
}

fn timeline_pane(
  state: VersionHistoryState,
  dark_mode: Signal<bool>,
  creating_checkpoint: Signal<bool>,
  checkpoint_draft: Signal<String>,
  confirm_restore_id: Signal<Option<String>>,
) -> impl IntoElement {
  let st_close = state.clone();
  let st_create = state.clone();
  let st_list = state.clone();
  let st_footer = state;

  view()
    .test_id("version-timeline-pane")
    .w(px(300.0))
    .h_full()
    .flex_shrink_0()
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .child(
      // Header: title + save checkpoint + close
      view()
        .px_4()
        .py_3()
        .flex_row()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .child(
          text("版本历史")
            .text_size(px(15.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || palette(dark_mode.get()).text_primary),
        )
        .child(
          view()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .child(
              button()
                .test_id("version-save-checkpoint-btn")
                .accessibility_label("保存检查点")
                .px_2()
                .h(px(26.0))
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  let next = !creating_checkpoint.get_untracked();
                  if next {
                    checkpoint_draft.set(String::new());
                  }
                  creating_checkpoint.set(next);
                })
                .child(dynamic(move || {
                  bookmark_add_icon(palette(dark_mode.get()).text_muted_hex)
                }))
                .child(
                  text("保存检查点")
                    .text_size(px(12.0))
                    .text_color(move || palette(dark_mode.get()).text_muted),
                ),
            )
            .child(
              button()
                .test_id("version-history-close-btn")
                .accessibility_label("关闭")
                .w(px(26.0))
                .h(px(26.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  creating_checkpoint.set(false);
                  confirm_restore_id.set(None);
                  (st_close.on_close)();
                })
                .child(dynamic(move || {
                  mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                })),
            ),
        ),
    )
    .child(when(
      move || creating_checkpoint.get(),
      move || {
        let on_create_submit = st_create.on_create_checkpoint.clone();
        let on_create_btn = st_create.on_create_checkpoint.clone();

        view()
          .test_id("version-checkpoint-inline-form")
          .px_3()
          .py_2()
          .gap_2()
          .bg(move || palette(dark_mode.get()).surface_raised)
          .border_b_1()
          .border_color(move || palette(dark_mode.get()).border_subtle)
          .child(
            input()
              .test_id("version-checkpoint-name-input")
              .auto_focus(true)
              .submit_behavior(SubmitBehavior::Submit)
              .placeholder("给这个版本起个名字,如「改版前」")
              .w_full()
              .px_2()
              .py(px(6.0))
              .rounded(px(6.0))
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .bg(move || palette(dark_mode.get()).surface_base)
              .text_size(px(12.5))
              .text_color(move || palette(dark_mode.get()).text_primary)
              .value(move || checkpoint_draft.get())
              .on_change(move |v: &str| checkpoint_draft.set(v.to_owned()))
              .on_key_press(move |k: &str| {
                if k == "Escape" {
                  checkpoint_draft.set(String::new());
                  creating_checkpoint.set(false);
                }
              })
              .on_submit(move |raw: &str| {
                let name = raw.trim().to_string();
                if name.is_empty() {
                  return;
                }
                checkpoint_draft.set(String::new());
                creating_checkpoint.set(false);
                on_create_submit(name);
              }),
          )
          .child(
            view()
              .flex_row()
              .justify_end()
              .gap_2()
              .child(
                button()
                  .test_id("version-checkpoint-cancel")
                  .px_2()
                  .py_1()
                  .rounded(px(6.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || {
                    checkpoint_draft.set(String::new());
                    creating_checkpoint.set(false);
                  })
                  .child(
                    text("取消")
                      .text_size(px(12.0))
                      .text_color(move || palette(dark_mode.get()).text_muted),
                  ),
              )
              .child(
                button()
                  .test_id("version-checkpoint-confirm")
                  .px_3()
                  .py_1()
                  .rounded(px(6.0))
                  .bg(move || palette(dark_mode.get()).accent)
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
                  .on_press(move || {
                    let name = checkpoint_draft.get_untracked().trim().to_string();
                    if name.is_empty() {
                      return;
                    }
                    checkpoint_draft.set(String::new());
                    creating_checkpoint.set(false);
                    on_create_btn(name);
                  })
                  .child(
                    text("保存")
                      .text_size(px(12.0))
                      .font_weight(FontWeight::MEDIUM)
                      .text_color(move || palette(dark_mode.get()).on_primary),
                  ),
              ),
          )
      },
    ))
    .child(
      // Scrollable grouped timeline
      scroll().flex_1().min_h(px(0.0)).child(dynamic(move || {
        let p = palette(dark_mode.get());
        if st_list.loading.get() {
          return view()
            .w_full()
            .py(px(40.0))
            .items_center()
            .child(text("加载中…").text_size(px(12.5)).text_color(p.text_muted))
            .into_element();
        }
        let items = st_list.items.get();
        if items.is_empty() {
          return view()
            .w_full()
            .px_4()
            .py(px(40.0))
            .items_center()
            .child(
              text("还没有版本。\n点右上角「保存检查点」把当前状态存下来。")
                .text_size(px(12.5))
                .text_color(p.text_muted)
                .text_center(),
            )
            .into_element();
        }

        let selected_id = st_list.selected_id.get();
        let sections = group_version_items(&items);
        let mut list = view().w_full().px_2().pb_2();

        for (group, section_items) in sections {
          let heading = match group {
            VersionGroup::Today => "今天",
            VersionGroup::Earlier => "更早",
          };
          list = list.child(
            view().pl(px(6.0)).pt(px(10.0)).pb(px(4.0)).child(
              text(heading)
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.text_faint),
            ),
          );

          for item in section_items {
            let is_selected = selected_id.as_deref() == Some(item.id.as_str());
            let on_select = st_list.on_select.clone();
            let item_id = item.id.clone();
            let (_, stamp) = format_version_stamp_now(&item.created_at);

            let title = if item.is_named {
              display_version_name(&item)
            } else {
              stamp.clone()
            };
            let mut sub_parts = Vec::new();
            if item.is_named {
              sub_parts.push(stamp);
            }
            if let Some(ref author) = item.author {
              let trimmed = author.trim();
              if !trimmed.is_empty() {
                sub_parts.push(trimmed.to_string());
              }
            }
            let subtitle = if sub_parts.is_empty() {
              None
            } else {
              Some(sub_parts.join("  ·  "))
            };

            let icon_color = if is_selected {
              p.text_primary_hex
            } else {
              p.text_faint_hex
            };
            let border_col = if is_selected {
              Rgba {
                r: p.text_primary.r,
                g: p.text_primary.g,
                b: p.text_primary.b,
                a: 0.35,
              }
            } else {
              Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
              }
            };

            let mut text_col = view().flex_1().min_w(px(0.0)).gap(px(2.0)).child(
              text(title)
                .text_size(px(13.0))
                .font_weight(if item.is_named {
                  FontWeight::MEDIUM
                } else {
                  FontWeight::NORMAL
                })
                .text_color(p.text_primary)
                .truncate(),
            );
            if let Some(sub) = subtitle {
              text_col = text_col.child(
                text(sub)
                  .text_size(px(11.5))
                  .text_color(p.text_faint)
                  .truncate(),
              );
            }

            let mut row_btn = button()
              .test_id(&format!("version-row-{}", item.id))
              .accessibility_label(format!("预览版本 {}", display_version_name(&item)))
              .w_full()
              .mb(px(6.0))
              .px(px(10.0))
              .py(px(9.0))
              .rounded(px(8.0))
              .border_1()
              .border_color(border_col)
              .flex_row()
              .items_center()
              .gap(px(9.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                confirm_restore_id.set(None);
                on_select(item_id.clone());
              })
              .child(if item.is_named {
                bookmark_icon(icon_color)
              } else {
                schedule_icon(icon_color)
              })
              .child(text_col);

            if is_selected {
              row_btn = row_btn.bg(p.surface_hover);
            }

            list = list.child(row_btn);
          }
        }

        list.into_element()
      })),
    )
    .child(
      // Footer restore bar with 2-step confirmation
      view()
        .test_id("version-history-footer")
        .w_full()
        .p_3()
        .border_t_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .child(dynamic(move || {
          let p = palette(dark_mode.get());
          let selected_id = st_footer.selected_id.get();
          let items = st_footer.items.get();
          let selected_item = selected_id
            .as_deref()
            .and_then(|sid| items.iter().find(|v| v.id == sid))
            .cloned();

          if let Some(confirm_id) = confirm_restore_id.get() {
            let name = items
              .iter()
              .find(|v| v.id == confirm_id)
              .map(display_version_name)
              .unwrap_or_else(|| "所选版本".to_string());
            let on_restore = st_footer.on_restore.clone();
            let on_close = st_footer.on_close.clone();

            return view()
              .test_id("version-restore-confirm-box")
              .w_full()
              .gap_2()
              .child(
                text(format!("把当前页恢复到「{name}」？"))
                  .text_size(px(12.5))
                  .text_color(p.text_primary),
              )
              .child(
                view()
                  .flex_row()
                  .justify_end()
                  .gap_2()
                  .child(
                    button()
                      .test_id("version-restore-cancel")
                      .px_3()
                      .py(px(6.0))
                      .rounded(px(6.0))
                      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                      .on_press(move || confirm_restore_id.set(None))
                      .child(text("取消").text_size(px(12.5)).text_color(p.text_muted)),
                  )
                  .child(
                    button()
                      .test_id("version-restore-confirm")
                      .px_3()
                      .py(px(6.0))
                      .rounded(px(6.0))
                      .bg(p.accent)
                      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).accent_hover))
                      .on_press(move || {
                        confirm_restore_id.set(None);
                        on_restore(confirm_id.clone());
                        on_close();
                      })
                      .child(
                        text("确认恢复")
                          .text_size(px(12.5))
                          .font_weight(FontWeight::MEDIUM)
                          .text_color(p.on_primary),
                      ),
                  ),
              )
              .into_element();
          }

          let can_restore = selected_item.is_some();
          let sel_id_for_press = selected_item.map(|v| v.id);
          view()
            .w_full()
            .flex_row()
            .justify_end()
            .child(
              button()
                .test_id("version-restore-btn")
                .accessibility_label("恢复所选版本")
                .disabled(!can_restore)
                .px_4()
                .py(px(6.0))
                .rounded(px(6.0))
                .bg(if can_restore { p.accent } else { p.surface_sunken })
                .hover(move |s| {
                  let pal = palette(dark_mode.get_untracked());
                  s.bg(if can_restore {
                    pal.accent_hover
                  } else {
                    pal.surface_sunken
                  })
                })
                .on_press(move || {
                  if let Some(ref sid) = sel_id_for_press {
                    confirm_restore_id.set(Some(sid.clone()));
                  }
                })
                .child(
                  text("恢复")
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if can_restore {
                      p.on_primary
                    } else {
                      p.text_ghost
                    }),
                ),
            )
            .into_element()
        })),
    )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn diff_unchanged_documents_marks_every_block_unchanged() {
    let md = "# Title\n\nFirst paragraph.\n\nSecond paragraph.";
    let diff = diff_markdown_blocks(md, md);
    assert_eq!(diff.len(), 3);
    assert!(diff.iter().all(|b| b.kind == DiffKind::Unchanged));
    assert!(!has_any_diff(&diff));
  }

  #[test]
  fn diff_detects_added_blocks() {
    let prev = "# Title\n\nFirst.";
    let curr = "# Title\n\nFirst.\n\nAdded paragraph.";
    let diff = diff_markdown_blocks(prev, curr);
    assert_eq!(diff.len(), 3);
    assert_eq!(diff[0].kind, DiffKind::Unchanged);
    assert_eq!(diff[1].kind, DiffKind::Unchanged);
    assert_eq!(diff[2].kind, DiffKind::Added);
    assert_eq!(diff[2].text, "Added paragraph.");
    assert!(has_any_diff(&diff));
  }

  #[test]
  fn diff_detects_changed_blocks_between_anchors() {
    let prev = "# Title\n\nOld sentence.\n\nFooter.";
    let curr = "# Title\n\nUpdated sentence.\n\nFooter.";
    let diff = diff_markdown_blocks(prev, curr);
    assert_eq!(diff.len(), 3);
    assert_eq!(diff[0].kind, DiffKind::Unchanged);
    assert_eq!(diff[1].kind, DiffKind::Changed);
    assert_eq!(diff[1].text, "Updated sentence.");
    assert_eq!(diff[2].kind, DiffKind::Unchanged);
  }

  #[test]
  fn diff_detects_deleted_blocks_as_ghosts() {
    let prev = "# Title\n\nRemoved line.\n\nFooter.";
    let curr = "# Title\n\nFooter.";
    let diff = diff_markdown_blocks(prev, curr);
    assert_eq!(diff.len(), 3);
    assert_eq!(diff[0].kind, DiffKind::Unchanged);
    assert_eq!(diff[1].kind, DiffKind::Deleted);
    assert_eq!(diff[1].text, "Removed line.");
    assert_eq!(diff[2].kind, DiffKind::Unchanged);
  }

  #[test]
  fn diff_handles_empty_predecessor_and_empty_current() {
    let added = diff_markdown_blocks("", "# Hello\n\nWorld");
    assert_eq!(added.len(), 2);
    assert!(added.iter().all(|b| b.kind == DiffKind::Added));

    let deleted = diff_markdown_blocks("# Hello\n\nWorld", "");
    assert_eq!(deleted.len(), 2);
    assert!(deleted.iter().all(|b| b.kind == DiffKind::Deleted));
  }

  #[test]
  fn format_version_stamp_classifies_today_yesterday_and_earlier() {
    let now = Local.with_ymd_and_hms(2026, 7, 22, 16, 0, 0).unwrap();

    let (g_today, s_today) = format_version_stamp("2026-07-22 15:08:00", now);
    assert_eq!(g_today, VersionGroup::Today);
    assert_eq!(s_today, "15:08");

    let (g_yest, s_yest) = format_version_stamp("2026-07-21 22:03:00", now);
    assert_eq!(g_yest, VersionGroup::Earlier);
    assert_eq!(s_yest, "昨天 22:03");

    let (g_old, s_old) = format_version_stamp("2026-06-05 09:30:00", now);
    assert_eq!(g_old, VersionGroup::Earlier);
    assert_eq!(s_old, "6 月 5 日");

    let (g_bad, s_bad) = format_version_stamp("not-a-date", now);
    assert_eq!(g_bad, VersionGroup::Earlier);
    assert_eq!(s_bad, "not-a-date");
  }

  #[test]
  fn group_version_items_splits_today_and_earlier_preserving_order() {
    let items = vec![
      VersionHistoryItem {
        id: "v1".into(),
        label: "最新".into(),
        is_named: true,
        created_at: "2026-07-22 15:08:00".into(),
        author: None,
        is_today: true,
      },
      VersionHistoryItem {
        id: "v2".into(),
        label: "".into(),
        is_named: false,
        created_at: "2026-07-21 22:03:00".into(),
        author: Some("Alice".into()),
        is_today: false,
      },
      VersionHistoryItem {
        id: "v3".into(),
        label: "初稿".into(),
        is_named: true,
        created_at: "2026-07-22 09:00:00".into(),
        author: None,
        is_today: true,
      },
    ];
    let grouped = group_version_items(&items);
    assert_eq!(grouped.len(), 2);
    assert_eq!(grouped[0].0, VersionGroup::Today);
    assert_eq!(grouped[0].1.len(), 2);
    assert_eq!(grouped[0].1[0].id, "v1");
    assert_eq!(grouped[0].1[1].id, "v3");
    assert_eq!(grouped[1].0, VersionGroup::Earlier);
    assert_eq!(grouped[1].1.len(), 1);
    assert_eq!(grouped[1].1[0].id, "v2");
  }
}
