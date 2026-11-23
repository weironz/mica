//! Rich WYSIWYG table block renderer aligned with Mica's legacy table UX (`CAP-05`):
//! - **Zero visual shift on cell click**: focused & unfocused cells share the exact same
//!   wrapped rich `text()` (`build_inline_runs` + `EventKind::TextLayout`), inheriting row
//!   background (`surface_raised` on header) and font weight (`SEMIBOLD` on header) while
//!   overlaying a crisp blinking caret, inline selection highlight, and a transparent
//!   caret-anchored IME `input()` proxy. Inline marks (`**bold**`, `` `code` ``, etc.) are
//!   preserved and shifted across edits via `splice_cell_marks`.
//! - **Full-height column resize handles across all rows**: every inner vertical divider
//!   (`1..col_count`) and the rightmost table edge (`col_count`) spans the full table grid
//!   height with `Cursor::ColResize`, live accent preview line, and persisted `SetColumnWidths`
//!   / `SetTableWidth`.
//! - **Proportional horizontal scrollbar**: tables wider than the page column (or manually
//!   widened past viewport) scroll smoothly horizontally with a visible `Axis::Horizontal`
//!   scrollbar beneath the table.
//! - **Legacy table chrome**: top-left corner anchor menu (`⋮⋮`), per-column top anchor pill
//!   with `overlay()` column menu, per-row left anchor pill with `overlay()` row menu, and
//!   right/bottom expand (`+`) bars hugging the outside of the grid.

use std::{cell::RefCell, rc::Rc, time::Duration};

use mica_editor::{
  EditorBlock, TableCell, TableModel, TableNavDirection, TableOp,
  marks::EditorMark,
  utf16::utf16_len,
};
use nsh_base::{ScrollbarMode, button, scrollbar};
use nsh_core::{
  SubmitBehavior, TextLine,
  color::Rgba,
  element::{
    Align, Composition, ElementRef, EventKind, EventPayload, Placement, PointerEvents, Selection,
    element_ref, input, overlay, scroll, set_clipboard, text, text_measurer, view,
  },
  geometry::{Axis, Bounds, Points, px},
  keymap::When,
  prelude::*,
};

use crate::{
  block_views::{
    BlockRenderEnv, TRANSPARENT_RGBA, build_inline_runs, live_index, render_inline_span,
  },
  editor_bridge::MicaDocumentBridge,
  mica_theme::palette,
  text_geometry::{caret_rect_for_offset, offset_at_point, selection_rects_for_range},
};

const LEFT_GUTTER_PX: f32 = 16.0;
const TOP_GUTTER_PX: f32 = 16.0;
const RIGHT_ADD_W_PX: f32 = 20.0;
const BOTTOM_ADD_H_PX: f32 = 18.0;
const CELL_FONT_PX: f32 = 13.5;
const CELL_LINE_H_PX: f32 = 21.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableMenuKind {
  Table,
  Row(usize),
  Col(usize),
}

fn visual_cell_units(text: &str) -> f32 {
  text
    .lines()
    .map(|line| {
      line
        .chars()
        .map(|c| if c.is_ascii() { 1.0_f32 } else { 1.85_f32 })
        .sum::<f32>()
    })
    .fold(0.0_f32, f32::max)
}

/// Computes content-adaptive column layout for a GFM table:
/// - Respects non-uniform user-adjusted `model.widths` when present.
/// - Otherwise measures every column's visual width (CJK-aware) so narrow key
///   columns (`项`, `编号`, `优先级`) stay compact while long content columns
///   automatically receive proportional space.
/// - Returns `(use_horizontal_scroll, col_shares, col_pixels)`.
pub fn compute_adaptive_table_layout(model: &TableModel) -> (bool, Vec<f32>, Vec<f32>) {
  let col_count = model.rows.first().map_or(0, Vec::len).max(1);
  let has_custom_widths = model.widths.len() == col_count && {
    let min_w = model.widths.iter().copied().fold(f64::INFINITY, f64::min);
    let max_w = model.widths.iter().copied().fold(0.0_f64, f64::max);
    (max_w - min_w) > 0.05
  };

  let mut raw_units = vec![4.5_f32; col_count];
  for (r_idx, row) in model.rows.iter().enumerate() {
    for (c_idx, cell) in row.iter().enumerate().take(col_count) {
      let u = visual_cell_units(cell.text.trim());
      let weighted = if r_idx == 0 { u * 1.08 } else { u };
      if weighted > raw_units[c_idx] {
        raw_units[c_idx] = weighted;
      }
    }
  }

  let natural_pixels: Vec<f32> = raw_units
    .iter()
    .map(|&u| (u * 8.2 + 28.0).clamp(72.0, 320.0))
    .collect();
  let total_natural_px: f32 = natural_pixels.iter().sum();
  let wide_table = col_count > 5 || (col_count > 4 && total_natural_px > 820.0);

  let mut weights: Vec<f32> = if has_custom_widths {
    model
      .widths
      .iter()
      .map(|&w| (w as f32).max(0.05))
      .collect()
  } else {
    raw_units
      .iter()
      .map(|&u| {
        let clamped = u.clamp(4.0, 64.0);
        if clamped <= 16.0 {
          clamped
        } else {
          16.0 + (clamped - 16.0).powf(0.85) * 1.35
        }
      })
      .collect()
  };

  let sum: f32 = weights.iter().sum::<f32>().max(0.001);
  let min_share = (0.09_f32).min(0.65 / col_count as f32);
  let max_share = (0.86_f32).max(1.0 / col_count as f32);
  for w in &mut weights {
    *w = (*w / sum).clamp(min_share, max_share);
  }
  let norm: f32 = weights.iter().sum::<f32>().max(0.001);
  let shares: Vec<f32> = weights.into_iter().map(|w| w / norm).collect();

  // Keep `col_pixels` in sync with custom column widths so wide-table resizing persists visually.
  let col_pixels: Vec<f32> = if has_custom_widths {
    shares
      .iter()
      .map(|&s| (s * total_natural_px).max(72.0))
      .collect()
  } else {
    natural_pixels
  };

  (wide_table, shares, col_pixels)
}

/// Shifts & preserves UTF-16 inline [`EditorMark`]s when a cell's plain text is edited
/// from `old_text` to `new_text`, avoiding formatting loss in WYSIWYG cell editing.
pub fn splice_cell_marks(
  old_text: &str,
  old_marks: &[EditorMark],
  new_text: &str,
) -> Vec<EditorMark> {
  if old_marks.is_empty() || new_text.is_empty() {
    return Vec::new();
  }
  if old_text == new_text {
    return old_marks.to_vec();
  }

  let old_chars: Vec<(usize, char)> = old_text.char_indices().collect();
  let new_chars: Vec<(usize, char)> = new_text.char_indices().collect();

  let mut prefix_c = 0usize;
  while prefix_c < old_chars.len()
    && prefix_c < new_chars.len()
    && old_chars[prefix_c].1 == new_chars[prefix_c].1
  {
    prefix_c += 1;
  }

  let mut suffix_c = 0usize;
  while suffix_c < (old_chars.len() - prefix_c)
    && suffix_c < (new_chars.len() - prefix_c)
    && old_chars[old_chars.len() - 1 - suffix_c].1 == new_chars[new_chars.len() - 1 - suffix_c].1
  {
    suffix_c += 1;
  }

  let old_prefix_bytes = if prefix_c < old_chars.len() {
    old_chars[prefix_c].0
  } else {
    old_text.len()
  };
  let old_del_end_idx = old_chars.len() - suffix_c;
  let old_del_end_bytes = if old_del_end_idx < old_chars.len() {
    old_chars[old_del_end_idx].0
  } else {
    old_text.len()
  };
  let new_ins_end_idx = new_chars.len() - suffix_c;
  let new_ins_end_bytes = if new_ins_end_idx < new_chars.len() {
    new_chars[new_ins_end_idx].0
  } else {
    new_text.len()
  };

  let prefix_u16 = utf16_len(&old_text[..old_prefix_bytes]);
  let old_del_end_u16 = utf16_len(&old_text[..old_del_end_bytes]);
  let new_ins_end_u16 = utf16_len(&new_text[..new_ins_end_bytes]);
  let delta = new_ins_end_u16 as isize - old_del_end_u16 as isize;
  let new_total_u16 = utf16_len(new_text);

  let mut out = Vec::with_capacity(old_marks.len());
  for m in old_marks {
    let mut next = m.clone();
    if m.end <= prefix_u16 {
      // Strictly before edit: keep untouched.
    } else if m.start >= old_del_end_u16 {
      next.start = (m.start as isize + delta).clamp(0, new_total_u16 as isize) as u32;
      next.end = (m.end as isize + delta).clamp(0, new_total_u16 as isize) as u32;
    } else {
      // Overlapping edit: anchor start if before edit, shift end by delta.
      next.start = if m.start <= prefix_u16 {
        m.start
      } else {
        new_ins_end_u16.min(new_total_u16)
      };
      next.end = if m.end <= old_del_end_u16 {
        prefix_u16.min(new_total_u16)
      } else {
        (m.end as isize + delta).clamp(0, new_total_u16 as isize) as u32
      };
    }
    if next.start < next.end {
      out.push(next);
    }
  }
  out
}

/// Computes resized adjacent column shares when dragging divider `c` (`1..col_count`) by `dx`.
pub fn apply_column_divider_drag(shares: &[f32], col: usize, dx: f32, grid_w: f32) -> Option<Vec<f64>> {
  if col == 0 || col >= shares.len() || dx.abs() < 3.0 {
    return None;
  }
  let tw = grid_w.max(120.0);
  let delta_frac = (dx / tw) as f64;
  let mut next: Vec<f64> = shares.iter().map(|&w| w as f64).collect();
  let min_w = (60.0_f64 / tw as f64).clamp(0.06, 0.25);
  let pair = next[col - 1] + next[col];
  let left_w = (next[col - 1] + delta_frac).clamp(min_w, (pair - min_w).max(min_w));
  next[col - 1] = left_w;
  next[col] = (pair - left_w).max(min_w);
  Some(next)
}

fn table_menu_item(
  tid: &str,
  label: &str,
  danger: bool,
  dark_mode: Signal<bool>,
  on_click: impl Fn() + 'static,
) -> impl IntoElement {
  let label_owned = label.to_owned();
  button()
    .test_id(tid)
    .w_full()
    .h(px(28.0))
    .px_2()
    .rounded(px(5.0))
    .flex_row()
    .items_center()
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(
      text(label_owned)
        .text_size(px(12.5))
        .text_color(move || {
          let p = palette(dark_mode.get());
          if danger { p.danger } else { p.text_primary }
        }),
    )
}

fn menu_divider(dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w_full()
    .h(px(1.0))
    .my_1()
    .bg(move || palette(dark_mode.get()).border_subtle)
}

fn grip_dots_svg(color: &str) -> impl IntoElement {
  nsh_core::element::svg(nsh_core::element::ImageSrc::svg_markup(&format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 16 16" fill="{color}"><circle cx="5.5" cy="3.5" r="1.25"/><circle cx="10.5" cy="3.5" r="1.25"/><circle cx="5.5" cy="8" r="1.25"/><circle cx="10.5" cy="8" r="1.25"/><circle cx="5.5" cy="12.5" r="1.25"/><circle cx="10.5" cy="12.5" r="1.25"/></svg>"##
  )))
  .w(px(12.0))
  .h(px(12.0))
}

/// Renders a GFM `table` block with WYSIWYG cell editing, full-height column resize
/// handles across all rows, horizontal scrollbar, and legacy top/left anchor menus +
/// right/bottom `+` expand bars.
pub(crate) fn render_table_block_view(
  env: &BlockRenderEnv,
  at: Position,
  block: EditorBlock,
) -> impl IntoElement {
  let init_idx = live_index(at);
  let dark_mode = env.dark_mode;
  let bridge = env.bridge.clone();
  let focused_cell = env.focused_table_cell;
  let table_selection = env.table_selection;
  let table_drag_anchor = env.table_drag_anchor;
  let marquee_selected = env.marquee_selected;
  let drag_anchor = env.drag_anchor;
  let editor_focused = env.editor_focused;
  let block_id = block.id.clone();

  let model = TableModel::from_block_data(&block.data);
  let row_count = model.rows.len().max(1);
  let col_count = model.rows.first().map_or(0, Vec::len).max(1);
  let (wide_table, col_shares, col_pixels) = compute_adaptive_table_layout(&model);
  let total_wide_px: f32 = col_pixels.iter().sum();
  let explicit_model_width = (model.table_width > 60.0).then_some(model.table_width as f32);
  let min_readable_px: f32 = if col_count >= 4 {
    col_pixels
      .iter()
      .map(|&w| w.min(180.0))
      .sum::<f32>()
      .max(col_count as f32 * 110.0)
  } else {
    0.0
  };
  let model_aligns: Vec<String> = (0..col_count)
    .map(|c| model.align_for(c).to_owned())
    .collect();

  // Interactive chrome state
  let hovered_table = signal(false);
  let hovered_cell: Signal<Option<(usize, usize)>> = signal(None);
  let hovered_row_handle: Signal<Option<usize>> = signal(None);
  let hovered_col_handle: Signal<Option<usize>> = signal(None);
  let hovered_resize_col: Signal<Option<usize>> = signal(None);
  let live_resize: Signal<Option<(usize, f32)>> = signal(None);
  let col_drag_start = signal(0.0_f32);
  let open_menu: Signal<Option<TableMenuKind>> = signal(None);
  let viewport_w = signal(720.0_f32);

  // Dismiss any open table/row/column overlay menu on Escape or when focusing a cell.
  effect(move || {
    if open_menu.get().is_none() {
      return;
    }
    on_key_with(
      "escape",
      KeyOptions {
        when: When::Always,
        ..KeyOptions::default()
      },
      move |_| {
        open_menu.set(None);
      },
    );
  });
  effect(move || {
    if focused_cell.get().is_some() && open_menu.get_untracked().is_some() {
      open_menu.set(None);
    }
  });

  // Local caret blink signal active while any cell of THIS table has focus.
  let table_caret_blink = signal(true);
  let blink_reset_rev = signal(0u64);
  {
    let bid_blink = block_id.clone();
    let task_slot: Rc<RefCell<Option<Spawned<()>>>> = Rc::new(RefCell::new(None));
    let task_cleanup = task_slot.clone();
    cleanup(move || {
      if let Some(t) = task_cleanup.borrow_mut().take() {
        t.cancel();
      }
    });
    effect(move || {
      let _ = blink_reset_rev.get();
      let active_here = focused_cell
        .get()
        .as_ref()
        .is_some_and(|(id, _, _)| id == &bid_blink);
      if let Some(prev) = task_slot.borrow_mut().take() {
        prev.cancel();
      }
      if !active_here {
        table_caret_blink.set(false);
        return;
      }
      table_caret_blink.set(true);
      let task = set_interval(Duration::from_millis(530), move || {
        table_caret_blink.update(|b| *b = !*b);
      });
      *task_slot.borrow_mut() = Some(task);
    });
  }

  let h_scroll_ref = element_ref();
  let corner_ref = element_ref();
  let col_handle_refs: Vec<ElementRef> = (0..col_count).map(|_| element_ref()).collect();
  let row_handle_refs: Vec<ElementRef> = (0..row_count).map(|_| element_ref()).collect();

  // Exact reactive pixel width of the table grid so top handles, all rows, and
  // full-height column resize dividers share sub-pixel identical column x-offsets.
  let grid_w = computed(move || {
    let avail = (viewport_w.get() - LEFT_GUTTER_PX - RIGHT_ADD_W_PX).max(240.0);
    if let Some(w) = explicit_model_width {
      w.max(200.0)
    } else if wide_table {
      total_wide_px.max(avail)
    } else if min_readable_px > 0.0 {
      min_readable_px.max(avail)
    } else {
      avail
    }
  });

  // Cumulative column x-fractions `[0.0, s0, s0+s1, ..., 1.0]`
  let mut cum_shares = Vec::with_capacity(col_count + 1);
  cum_shares.push(0.0_f32);
  for &s in &col_shares {
    cum_shares.push(cum_shares.last().copied().unwrap_or(0.0) + s);
  }
  if let Some(last) = cum_shares.last_mut() {
    *last = 1.0;
  }

  // ── 1. Top Gutter Row: Top-left corner menu handle + per-column anchor pills ──
  let mut top_gutter = view()
    .flex_row()
    .items_center()
    .h(px(TOP_GUTTER_PX))
    .w(move || px(LEFT_GUTTER_PX + grid_w.get() + RIGHT_ADD_W_PX));

  // Top-left corner anchor (`⋮⋮`) -> opens Table Menu
  top_gutter = top_gutter.child(
    view()
      .w(px(LEFT_GUTTER_PX))
      .h(px(TOP_GUTTER_PX))
      .flex_shrink_0()
      .items_center()
      .justify_center()
      .child(
        button()
          .element_ref(&corner_ref)
          .test_id("table-corner-handle")
          .accessibility_label("表格操作菜单")
          .w(px(16.0))
          .h(px(16.0))
          .rounded(px(4.0))
          .items_center()
          .justify_center()
          .cursor(Cursor::Pointer)
          .opacity(move || {
            if hovered_table.get() || open_menu.get() == Some(TableMenuKind::Table) {
              1.0_f32
            } else {
              0.0_f32
            }
          })
          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
          .on_press(move || {
            focused_cell.set(None);
            open_menu.set(if open_menu.get_untracked() == Some(TableMenuKind::Table) {
              None
            } else {
              Some(TableMenuKind::Table)
            });
          })
          .child(dynamic(move || {
            grip_dots_svg(palette(dark_mode.get()).text_muted_hex)
          })),
      ),
  );

  // Per-column top anchor pills aligned directly above each column `c`
  for c in 0..col_count {
    let share_c = col_shares[c];
    let bid_col_click = block_id.clone();
    top_gutter = top_gutter.child(
      view()
        .w(move || px(grid_w.get() * share_c))
        .h(px(TOP_GUTTER_PX))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .on_mouse_enter(move |_: &MouseEvent| hovered_col_handle.set(Some(c)))
        .on_mouse_leave(move |_: &MouseEvent| {
          if hovered_col_handle.get_untracked() == Some(c) {
            hovered_col_handle.set(None);
          }
        })
        .child(
          button()
            .element_ref(&col_handle_refs[c])
            .test_id(&format!("table-col-handle-{c}"))
            .accessibility_label(format!("第 {} 列操作", c + 1))
            .w(px(28.0))
            .h(px(12.0))
            .rounded(px(4.0))
            .items_center()
            .justify_center()
            .cursor(Cursor::Pointer)
            .on_press(move || {
              focused_cell.set(None);
              let next = if open_menu.get_untracked() == Some(TableMenuKind::Col(c)) {
                None
              } else {
                table_selection.set(Some((
                  bid_col_click.clone(),
                  (0, c),
                  (row_count.saturating_sub(1), c),
                )));
                Some(TableMenuKind::Col(c))
              };
              open_menu.set(next);
            })
            .child(
              view()
                .w(px(22.0))
                .h(px(4.0))
                .rounded(px(2.0))
                .bg(move || {
                  let p = palette(dark_mode.get());
                  let active = open_menu.get() == Some(TableMenuKind::Col(c));
                  let self_hov = hovered_col_handle.get() == Some(c);
                  let col_hov = hovered_cell.get().is_some_and(|(_, hc)| hc == c);
                  if active || self_hov {
                    p.accent
                  } else if col_hov {
                    p.border_strong
                  } else {
                    TRANSPARENT_RGBA
                  }
                }),
            ),
        ),
    );
  }
  top_gutter = top_gutter.child(view().w(px(RIGHT_ADD_W_PX)).flex_shrink_0());

  // ── 2. Middle Section: Left row handles + Bordered Grid + Full-height resize dividers + Right `+` bar ──
  let mut rows_col = view()
    .relative()
    .flex_col()
    .w(move || px(LEFT_GUTTER_PX + grid_w.get()))
    .flex_shrink_0();

  for (r, row_handle_ref) in row_handle_refs.iter().enumerate().take(row_count) {
    let is_header = r == 0;
    let is_last_row = r + 1 == row_count;
    let bid_row_click = block_id.clone();

    let mut row_cells = view()
      .w(move || px(grid_w.get()))
      .flex_row()
      .items_stretch()
      .flex_shrink_0()
      .border_l_1()
      .border_r_1()
      .border_color(move || palette(dark_mode.get()).border_normal)
      .bg(move || {
        let p = palette(dark_mode.get());
        if is_header { p.surface_raised } else { p.surface_base }
      });

    if r == 0 {
      row_cells = row_cells
        .border_t_1()
        .rounded_t(px(6.0))
        .overflow_hidden();
    } else {
      row_cells = row_cells.border_t_1();
    }
    if is_last_row {
      row_cells = row_cells
        .border_b_1()
        .rounded_b(px(6.0))
        .overflow_hidden();
    }

    for c in 0..col_count {
      let cell_obj = model
        .rows
        .get(r)
        .and_then(|row| row.get(c))
        .cloned()
        .unwrap_or_default();
      let share_c = col_shares[c];
      let col_align = model_aligns[c].clone();

      let cell_el = render_table_cell(
        TableCellCtx {
          init_idx,
          at,
          row: r,
          col: c,
          row_count,
          is_header,
          share_c,
          col_align,
          cell_obj,
          block_id: block_id.clone(),
          bridge: bridge.clone(),
          dark_mode,
          grid_w,
          focused_cell,
          table_selection,
          table_drag_anchor,
          marquee_selected,
          drag_anchor,
          editor_focused,
          hovered_cell,
          table_caret_blink,
          blink_reset_rev,
        },
      );
      row_cells = row_cells.child(cell_el);
    }

    // Row container: left row anchor pill + bordered cell strip (`items_stretch` guarantees
    // the left pill automatically stays vertically centered on multi-line wrapped rows!).
    let row_wrapper = view()
      .w(move || px(LEFT_GUTTER_PX + grid_w.get()))
      .flex_row()
      .items_stretch()
      .child(
        view()
          .w(px(LEFT_GUTTER_PX))
          .flex_shrink_0()
          .items_center()
          .justify_center()
          .on_mouse_enter(move |_: &MouseEvent| hovered_row_handle.set(Some(r)))
          .on_mouse_leave(move |_: &MouseEvent| {
            if hovered_row_handle.get_untracked() == Some(r) {
              hovered_row_handle.set(None);
            }
          })
          .child(
            button()
              .element_ref(row_handle_ref)
              .test_id(&format!("table-row-handle-{r}"))
              .accessibility_label(format!("第 {} 行操作", r + 1))
              .w(px(12.0))
              .h(px(26.0))
              .rounded(px(4.0))
              .items_center()
              .justify_center()
              .cursor(Cursor::Pointer)
              .on_press(move || {
                focused_cell.set(None);
                let next = if open_menu.get_untracked() == Some(TableMenuKind::Row(r)) {
                  None
                } else {
                  table_selection.set(Some((
                    bid_row_click.clone(),
                    (r, 0),
                    (r, col_count.saturating_sub(1)),
                  )));
                  Some(TableMenuKind::Row(r))
                };
                open_menu.set(next);
              })
              .child(
                view()
                  .w(px(4.0))
                  .h(px(16.0))
                  .rounded(px(2.0))
                  .bg(move || {
                    let p = palette(dark_mode.get());
                    let active = open_menu.get() == Some(TableMenuKind::Row(r));
                    let self_hov = hovered_row_handle.get() == Some(r);
                    let row_hov = hovered_cell.get().is_some_and(|(hr, _)| hr == r);
                    if active || self_hov {
                      p.accent
                    } else if row_hov {
                      p.border_strong
                    } else {
                      TRANSPARENT_RGBA
                    }
                  }),
              ),
          ),
      )
      .child(row_cells);

    rows_col = rows_col.child(row_wrapper);
  }

  // Full-height column resize handles across ALL rows (`1..=col_count`):
  // - `1..col_count`: inner column borders (`SetColumnWidths`)
  // - `col_count`: rightmost table border (`SetTableWidth`, enabling horizontal scroll!)
  let mut resize_layer = view()
    .absolute()
    .left(px(LEFT_GUTTER_PX))
    .right_0()
    .top_0()
    .bottom_0()
    .z_index(16)
    .pointer_events(PointerEvents::BoxNone);

  for (c, &frac) in cum_shares.iter().enumerate().take(col_count + 1).skip(1) {
    let shares_r = col_shares.clone();
    let br_r = bridge.clone();
    let is_outer_right = c == col_count;

    resize_layer = resize_layer.child(
      view()
        .test_id(&format!("table-col-resize-{init_idx}-{c}"))
        .absolute()
        .top_0()
        .bottom_0()
        .left(move || {
          let base = grid_w.get() * frac;
          let live_dx = match live_resize.get() {
            Some((rc, dx)) if rc == c => dx,
            _ => 0.0,
          };
          px((base + live_dx - 4.0).max(-4.0))
        })
        .w(px(8.0))
        .items_center()
        .cursor(Cursor::ColResize)
        .on_mouse_enter(move |_: &MouseEvent| hovered_resize_col.set(Some(c)))
        .on_mouse_leave(move |_: &MouseEvent| {
          if hovered_resize_col.get_untracked() == Some(c) {
            hovered_resize_col.set(None);
          }
        })
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_responder_grant(move |e: &ResponderEvent| {
          col_drag_start.set(f32::from(e.page.x));
          live_resize.set(Some((c, 0.0)));
        })
        .on_responder_move(move |e: &ResponderEvent| {
          let dx = f32::from(e.page.x) - col_drag_start.get_untracked();
          live_resize.set(Some((c, dx)));
        })
        .on_responder_release(move |e: &ResponderEvent| {
          let dx = f32::from(e.page.x) - col_drag_start.get_untracked();
          live_resize.set(None);
          if dx.abs() < 3.0 {
            return;
          }
          let tw = grid_w.get_untracked();
          if is_outer_right {
            let next_w = (tw + dx).clamp(240.0, 2400.0);
            br_r.table_op(live_index(at), TableOp::SetTableWidth { width: next_w as f64 });
          } else if let Some(next) = apply_column_divider_drag(&shares_r, c, dx, tw) {
            br_r.table_op(live_index(at), TableOp::SetColumnWidths { widths: next });
          }
        })
        .child(
          // Crisp 2px full-height vertical guide bar visible while hovering OR dragging ANY row
          view()
            .w(px(2.0))
            .h_full()
            .bg(move || {
              let active = hovered_resize_col.get() == Some(c)
                || live_resize.get().is_some_and(|(rc, _)| rc == c);
              if active {
                let mut a = palette(dark_mode.get()).accent;
                a.a = 0.72;
                a
              } else {
                TRANSPARENT_RGBA
              }
            }),
        ),
    );
  }
  rows_col = rows_col.child(resize_layer);

  // Right `+` vertical expand-column bar hugging the right edge of the table grid
  let br_add_col = bridge.clone();
  let add_col_hovered = signal(false);
  let right_add_bar = button()
    .test_id("table-add-col-bar")
    .accessibility_label("右侧新增一列")
    .w(px(16.0))
    .ml(px(4.0))
    .flex_shrink_0()
    .rounded(px(4.0))
    .items_center()
    .justify_center()
    .cursor(Cursor::Pointer)
    .on_mouse_enter(move |_: &MouseEvent| add_col_hovered.set(true))
    .on_mouse_leave(move |_: &MouseEvent| add_col_hovered.set(false))
    .opacity(move || {
      let show = hovered_table.get() || add_col_hovered.get();
      if show { 1.0_f32 } else { 0.0_f32 }
    })
    .bg(move || {
      let dark = dark_mode.get();
      if add_col_hovered.get() {
        if dark {
          Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.14 }
        } else {
          Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.09 }
        }
      } else if dark {
        Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.06 }
      } else {
        Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.04 }
      }
    })
    .on_press(move || {
      open_menu.set(None);
      br_add_col.table_op(live_index(at), TableOp::InsertColumn { at: col_count });
    })
    .child(
      text("+")
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || palette(dark_mode.get()).text_muted),
    );

  let middle_row = view()
    .flex_row()
    .items_stretch()
    .w(move || px(LEFT_GUTTER_PX + grid_w.get() + RIGHT_ADD_W_PX))
    .child(rows_col)
    .child(right_add_bar);

  // ── 3. Bottom `+` horizontal expand-row bar hugging the bottom edge of the grid ──
  let br_add_row = bridge.clone();
  let add_row_hovered = signal(false);
  let bottom_add_row = view()
    .flex_row()
    .items_center()
    .mt(px(4.0))
    .h(px(BOTTOM_ADD_H_PX))
    .w(move || px(LEFT_GUTTER_PX + grid_w.get() + RIGHT_ADD_W_PX))
    .child(view().w(px(LEFT_GUTTER_PX)).flex_shrink_0())
    .child(
      button()
        .test_id("table-add-row-bar")
        .accessibility_label("底部新增一行")
        .w(move || px(grid_w.get()))
        .h(px(16.0))
        .flex_shrink_0()
        .rounded(px(4.0))
        .items_center()
        .justify_center()
        .cursor(Cursor::Pointer)
        .on_mouse_enter(move |_: &MouseEvent| add_row_hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| add_row_hovered.set(false))
        .opacity(move || {
          let show = hovered_table.get() || add_row_hovered.get();
          if show { 1.0_f32 } else { 0.0_f32 }
        })
        .bg(move || {
          let dark = dark_mode.get();
          if add_row_hovered.get() {
            if dark {
              Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.14 }
            } else {
              Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.09 }
            }
          } else if dark {
            Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.06 }
          } else {
            Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.04 }
          }
        })
        .on_press(move || {
          open_menu.set(None);
          br_add_row.table_op(live_index(at), TableOp::InsertRow { at: row_count });
        })
        .child(
          text("+")
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .text_color(move || palette(dark_mode.get()).text_muted),
        ),
    )
    .child(view().w(px(RIGHT_ADD_W_PX)).flex_shrink_0());

  let table_canvas = view()
    .flex_col()
    .w(move || px(LEFT_GUTTER_PX + grid_w.get() + RIGHT_ADD_W_PX))
    .child(top_gutter)
    .child(middle_row)
    .child(bottom_add_row);

  // Horizontal scroll wrapper + proportional horizontal scrollbar (auto-hides when
  // the table fits inside the viewport, appears immediately when wider than viewport).
  let scrollable_body = view()
    .w_full()
    .flex_col()
    .child(
      scroll()
        .element_ref(&h_scroll_ref)
        .horizontal()
        .scrollbar(false)
        .w_full()
        .child(table_canvas),
    )
    .child(
      scrollbar()
        .test_id("table-horizontal-scrollbar")
        .scroll(h_scroll_ref)
        .axis(Axis::Horizontal)
        .mode(ScrollbarMode::Always)
        .min_thumb_length(px(40.0))
        .thumb_style(move |s| {
          let dark = dark_mode.get();
          let color = if dark {
            Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.28 }
          } else {
            Rgba { r: 0.09, g: 0.11, b: 0.15, a: 0.26 }
          };
          s.bg(color).rounded(px(4.0))
        })
        .w_full()
        .h(px(10.0))
        .py(px(2.0)),
    );

  // ── 4. Top-level `overlay()` Menus (Table / Row / Column) ──
  let br_menus = bridge.clone();
  let col_refs_ov = col_handle_refs.clone();
  let row_refs_ov = row_handle_refs.clone();

  view()
    .test_id("node-view-table")
    .relative()
    .w_full()
    .my_2()
    .on_layout(move |b: &Bounds<Points>| {
      if b.size.width.0 > 120.0 && (viewport_w.get_untracked() - b.size.width.0).abs() > 1.0 {
        viewport_w.set(b.size.width.0);
      }
    })
    .on_mouse_enter(move |_: &MouseEvent| hovered_table.set(true))
    .on_mouse_leave(move |_: &MouseEvent| {
      hovered_table.set(false);
      hovered_cell.set(None);
    })
    .child(scrollable_body)
    .child(when(move || open_menu.get().is_some(), move || {
      let kind = open_menu.get_untracked().unwrap_or(TableMenuKind::Table);
      let br_m = br_menus.clone();
      match kind {
        TableMenuKind::Table => {
          let (b1, b2, b3, b4, b5, b6) = (
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
          );
          let menu = view()
            .test_id("table-corner-menu")
            .w(px(156.0))
            .p_1()
            .rounded(px(8.0))
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .child(table_menu_item("table-align-left", "整表左对齐", false, dark_mode, move || {
              open_menu.set(None);
              b1.table_op(live_index(at), TableOp::SetAlign { align: "left".to_owned() });
            }))
            .child(table_menu_item("table-align-center", "整表居中", false, dark_mode, move || {
              open_menu.set(None);
              b2.table_op(live_index(at), TableOp::SetAlign { align: "center".to_owned() });
            }))
            .child(table_menu_item("table-align-right", "整表右对齐", false, dark_mode, move || {
              open_menu.set(None);
              b3.table_op(live_index(at), TableOp::SetAlign { align: "right".to_owned() });
            }))
            .child(menu_divider(dark_mode))
            .child(table_menu_item("table-autofit", "自适应列宽", false, dark_mode, move || {
              open_menu.set(None);
              b4.table_op(live_index(at), TableOp::ResetColumnWidths);
              b4.table_op(live_index(at), TableOp::SetTableWidth { width: 0.0 });
            }))
            .child(table_menu_item("table-copy-all", "复制表格", false, dark_mode, move || {
              open_menu.set(None);
              if let Some(payload) = b5.copy_table_area(
                live_index(at),
                0,
                0,
                row_count.saturating_sub(1),
                col_count.saturating_sub(1),
              ) {
                let text_out = if !payload.markdown.is_empty() {
                  payload.markdown
                } else {
                  payload.plain
                };
                set_clipboard(text_out);
              }
            }))
            .child(menu_divider(dark_mode))
            .child(table_menu_item("table-delete", "删除表格", true, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b6.delete_block_at(live_index(at));
            }));
          overlay()
            .anchor(&corner_ref)
            .placement(Placement::Bottom)
            .align(Align::Start)
            .offset(px(4.0))
            .child(menu)
        }
        TableMenuKind::Row(r) => {
          let anchor_ref = row_refs_ov.get(r).copied().unwrap_or(corner_ref);
          let (b1, b2, b3, b4, b5, b6) = (
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
          );
          let mut menu = view()
            .test_id("table-row-menu")
            .w(px(156.0))
            .p_1()
            .rounded(px(8.0))
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal);
          if r > 0 {
            menu = menu.child(table_menu_item("table-row-move-up", "向上移动一行", false, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b1.table_op(live_index(at), TableOp::MoveRow { row: r, delta: -1 });
            }));
          }
          if r + 1 < row_count {
            menu = menu.child(table_menu_item("table-row-move-down", "向下移动一行", false, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b2.table_op(live_index(at), TableOp::MoveRow { row: r, delta: 1 });
            }));
          }
          menu = menu
            .child(table_menu_item("table-row-insert-above", "上方插入行", false, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b3.table_op(live_index(at), TableOp::InsertRow { at: r });
            }))
            .child(table_menu_item("table-row-insert-below", "下方插入行", false, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b4.table_op(live_index(at), TableOp::InsertRow { at: r + 1 });
            }))
            .child(menu_divider(dark_mode))
            .child(table_menu_item("table-row-clear", "清空整行内容", false, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b5.table_op(
                live_index(at),
                TableOp::ClearCells {
                  r0: r,
                  c0: 0,
                  r1: r,
                  c1: col_count.saturating_sub(1),
                },
              );
            }));
          if row_count > 1 {
            menu = menu.child(table_menu_item("table-row-delete", "删除此行", true, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b6.table_op(live_index(at), TableOp::DeleteRow { at: r });
            }));
          }
          overlay()
            .anchor(&anchor_ref)
            .placement(Placement::Right)
            .align(Align::Start)
            .offset(px(4.0))
            .child(menu)
        }
        TableMenuKind::Col(c) => {
          let anchor_ref = col_refs_ov.get(c).copied().unwrap_or(corner_ref);
          let (b1, b2, b3, b4, b5, b6, b7, b8, b9) = (
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
            br_m.clone(),
          );
          let mut menu = view()
            .test_id("table-col-menu")
            .w(px(156.0))
            .p_1()
            .rounded(px(8.0))
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal);
          if c > 0 {
            menu = menu.child(table_menu_item("table-col-move-left", "向左移动一列", false, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b1.table_op(live_index(at), TableOp::MoveColumn { col: c, delta: -1 });
            }));
          }
          if c + 1 < col_count {
            menu = menu.child(table_menu_item("table-col-move-right", "向右移动一列", false, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b2.table_op(live_index(at), TableOp::MoveColumn { col: c, delta: 1 });
            }));
          }
          menu = menu
            .child(table_menu_item("table-col-insert-left", "左侧插入列", false, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b3.table_op(live_index(at), TableOp::InsertColumn { at: c });
            }))
            .child(table_menu_item("table-col-insert-right", "右侧插入列", false, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b4.table_op(live_index(at), TableOp::InsertColumn { at: c + 1 });
            }))
            .child(menu_divider(dark_mode))
            .child(table_menu_item("table-col-align-left", "列左对齐", false, dark_mode, move || {
              open_menu.set(None);
              b5.table_op(
                live_index(at),
                TableOp::SetColumnAlign { col: c, align: "left".to_owned() },
              );
            }))
            .child(table_menu_item("table-col-align-center", "列居中对齐", false, dark_mode, move || {
              open_menu.set(None);
              b6.table_op(
                live_index(at),
                TableOp::SetColumnAlign { col: c, align: "center".to_owned() },
              );
            }))
            .child(table_menu_item("table-col-align-right", "列右对齐", false, dark_mode, move || {
              open_menu.set(None);
              b7.table_op(
                live_index(at),
                TableOp::SetColumnAlign { col: c, align: "right".to_owned() },
              );
            }))
            .child(menu_divider(dark_mode))
            .child(table_menu_item("table-col-clear", "清空整列内容", false, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b8.table_op(
                live_index(at),
                TableOp::ClearCells {
                  r0: 0,
                  c0: c,
                  r1: row_count.saturating_sub(1),
                  c1: c,
                },
              );
            }));
          if col_count > 1 {
            menu = menu.child(table_menu_item("table-col-delete", "删除此列", true, dark_mode, move || {
              open_menu.set(None);
              table_selection.set(None);
              b9.table_op(live_index(at), TableOp::DeleteColumn { at: c });
            }));
          }
          overlay()
            .anchor(&anchor_ref)
            .placement(Placement::Bottom)
            .align(Align::Center)
            .offset(px(4.0))
            .child(menu)
        }
      }
    }))
}

struct TableCellCtx {
  init_idx: usize,
  at: Position,
  row: usize,
  col: usize,
  row_count: usize,
  is_header: bool,
  share_c: f32,
  col_align: String,
  cell_obj: TableCell,
  block_id: String,
  bridge: Rc<MicaDocumentBridge>,
  dark_mode: Signal<bool>,
  grid_w: Memo<f32>,
  focused_cell: Signal<Option<(String, usize, usize)>>,
  table_selection: Signal<Option<crate::block_views::TableCellSelection>>,
  table_drag_anchor: Signal<Option<(String, usize, usize)>>,
  marquee_selected: Signal<Option<(usize, usize)>>,
  drag_anchor: Signal<Option<mica_editor::DocPosition>>,
  editor_focused: Signal<bool>,
  hovered_cell: Signal<Option<(usize, usize)>>,
  table_caret_blink: Signal<bool>,
  blink_reset_rev: Signal<u64>,
}

fn render_table_cell(ctx: TableCellCtx) -> impl IntoElement {
  let TableCellCtx {
    init_idx,
    at,
    row: r,
    col: c,
    row_count,
    is_header,
    share_c,
    col_align,
    cell_obj,
    block_id,
    bridge,
    dark_mode,
    grid_w,
    focused_cell,
    table_selection,
    table_drag_anchor,
    marquee_selected,
    drag_anchor,
    editor_focused,
    hovered_cell,
    table_caret_blink,
    blink_reset_rev,
  } = ctx;

  let init_len_u16 = utf16_len(&cell_obj.text);
  let live_text = signal(cell_obj.text);
  let live_marks = signal(cell_obj.marks);
  let cell_preedit: Signal<Option<(u32, String)>> = signal(None);
  let desired_sel: Signal<(u32, u32)> = signal((init_len_u16, init_len_u16));
  let sel_push_rev = signal(0u64);
  let visual_sel: Signal<(u32, u32)> = signal((init_len_u16, init_len_u16));
  let cell_text_drag_anchor: Signal<Option<u32>> = signal(None);
  let layout_rev = signal(0u64);
  let cell_layout: Rc<RefCell<(Vec<TextLine>, Bounds<Points>)>> =
    Rc::new(RefCell::new((Vec::new(), Bounds::default())));
  let cell_font = if is_header {
    "600 13.5px sans-serif"
  } else {
    "400 13.5px sans-serif"
  };

  let bid_chk = block_id.clone();
  let bid_sel_chk = block_id.clone();
  let is_active = computed(move || {
    focused_cell
      .get()
      .as_ref()
      .is_some_and(|(id, fr, fc)| id == &bid_chk && *fr == r && *fc == c)
  });
  let is_in_multi_sel = computed(move || {
    table_selection.get().as_ref().is_some_and(|(id, a, f)| {
      if id != &bid_sel_chk {
        return false;
      }
      let (r0, r1) = (a.0.min(f.0), a.0.max(f.0));
      let (c0, c1) = (a.1.min(f.1), a.1.max(f.1));
      r >= r0 && r <= r1 && c >= c0 && c <= c1
    })
  });

  // Compute live caret `(x, y, h)` inside the cell text container from real `TextLayout`.
  let cl_caret = cell_layout.clone();
  let caret_rect = computed(move || {
    let _ = layout_rev.get();
    let (_, end_u16) = visual_sel.get();
    let (ref lines, _) = *cl_caret.borrow();
    if lines.is_empty() {
      return (0.0_f32, 0.0_f32, CELL_LINE_H_PX);
    }
    let measurer = text_measurer();
    caret_rect_for_offset(lines, end_u16, cell_font, CELL_LINE_H_PX, &|f, s| {
      measurer(f, s)
    })
  });

  // Hit-test pointer `(page_x, page_y)` to exact UTF-16 offset in this cell.
  let hit_offset = {
    let cl_hit = cell_layout.clone();
    move |page_x: f32, page_y: f32| -> u32 {
      let max_off = utf16_len(&live_text.get_untracked());
      let (ref lines, frame) = *cl_hit.borrow();
      if lines.is_empty() {
        return max_off;
      }
      let lx = (page_x - frame.origin.x.0).max(0.0);
      let ly = (page_y - frame.origin.y.0).max(0.0);
      let measurer = text_measurer();
      offset_at_point(lines, lx, ly, cell_font, &|f, s| measurer(f, s)).min(max_off)
    }
  };

  let hit_down = hit_offset.clone();
  let hit_move = hit_offset;

  let bid_down = block_id.clone();
  let bid_move = block_id.clone();
  let bid_press = block_id.clone();
  let bid_nav = block_id;
  let br_down = bridge.clone();
  let br_chg = bridge.clone();
  let br_sub = bridge;

  // Inner text box: ALWAYS renders the exact same wrapped rich `text()` (zero layout
  // shift, zero white background, zero bold drop on click!), with scoped caret/selection
  // overlay + invisible caret-anchored IME `input()` proxy when active.
  let cl_tl = cell_layout.clone();
  let cl_frame = cell_layout.clone();
  let cl_sel = cell_layout;
  let align_disp = col_align;

  let text_box = view()
    .relative()
    .w_full()
    .on_layout(move |b: &Bounds<Points>| {
      cl_frame.borrow_mut().1 = *b;
    })
    // 1. Unified WYSIWYG rich cell text (identical in focused & unfocused states!)
    .child(dynamic(move || {
      let txt = live_text.get();
      let marks = live_marks.get();
      let pre = cell_preedit.get();
      let pre_tuple = pre.as_ref().map(|(off, s)| (0usize, *off, s.clone()));
      let cl_inner = cl_tl.clone();

      let mut t = text("")
        .w_full()
        .min_h(px(CELL_LINE_H_PX))
        .text_size(px(CELL_FONT_PX))
        .line_height(px(CELL_LINE_H_PX))
        .font_weight(if is_header {
          FontWeight::SEMIBOLD
        } else {
          FontWeight::NORMAL
        })
        .text_color(move || palette(dark_mode.get()).text_primary)
        .on(EventKind::TextLayout, move |p: &EventPayload| {
          if let EventPayload::TextLayout(lines) = p {
            cl_inner.borrow_mut().0 = lines.clone();
            layout_rev.update(|v| *v = v.wrapping_add(1));
          }
        });
      t = match align_disp.as_str() {
        "center" => t.text_center(),
        "right" => t.text_right(),
        _ => t,
      };
      let display_txt = if txt.ends_with('\n') {
        format!("{txt}\u{200B}")
      } else {
        txt
      };
      let runs = build_inline_runs(&display_txt, &marks, pre_tuple.as_ref());
      if runs.is_empty() {
        t = t.child(text("\u{200B}"));
      } else {
        for run in runs {
          t = t.child(render_inline_span(run, CELL_FONT_PX, dark_mode));
        }
      }
      t
    }))
    // 2. Scoped inline selection rects + blinking caret overlay (zero background change!)
    .child(when(move || is_active.get(), move || {
      let cl_ov = cl_sel.clone();
      view()
        .absolute()
        .inset_0()
        .z_index(4)
        .pointer_events(PointerEvents::None)
        .child(dynamic(move || {
          let _ = layout_rev.get();
          let (s, e) = visual_sel.get();
          let dark = dark_mode.get();
          let p = palette(dark);
          let mut layer = view().absolute().inset_0().pointer_events(PointerEvents::None);
          if s != e {
            let (ref lines, _) = *cl_ov.borrow();
            let measurer = text_measurer();
            let rects = selection_rects_for_range(
              lines,
              s.min(e),
              s.max(e),
              cell_font,
              CELL_LINE_H_PX,
              &|f, txt| measurer(f, txt),
            );
            for (rx, ry, rw, rh) in rects {
              let inset_y = 1.5_f32;
              layer = layer.child(
                view()
                  .absolute()
                  .left(px(rx))
                  .top(px(ry + inset_y))
                  .w(px(rw.max(4.0)))
                  .h(px((rh - inset_y * 2.0).max(14.0)))
                  .rounded(px(2.0))
                  .bg(p.selection_bg),
              );
            }
          } else if table_caret_blink.get() {
            let (cx, cy, ch) = caret_rect.get();
            let caret_h = (ch - 3.0).clamp(14.0, 20.0);
            let caret_y = cy + ((ch - caret_h) * 0.5).max(0.0);
            layer = layer.child(
              view()
                .test_id("mica-table-active-caret")
                .absolute()
                .left(px(cx))
                .top(px(caret_y))
                .w(px(1.5))
                .h(px(caret_h))
                .rounded(px(1.0))
                .bg(p.accent),
            );
          }
          layer
        }))
    }))
    // 3. Invisible caret-anchored IME input proxy (`test_id("table-cell-input-...")`)
    .child(when(move || is_active.get(), {
      let br_c = br_chg.clone();
      let br_s = br_sub.clone();
      let bid_n = bid_nav.clone();
      move || {
        let br_c2 = br_c.clone();
        let br_s2 = br_s.clone();
        let bid_n2 = bid_n.clone();
        let init_val = live_text.get_untracked();
        input()
          .test_id(&format!("table-cell-input-{init_idx}-{r}-{c}"))
          .auto_focus(true)
          .absolute()
          .left(move || px(caret_rect.get().0))
          .top(move || px(caret_rect.get().1))
          .w(px(1.0))
          .h(px(18.0))
          .pointer_events(PointerEvents::None)
          .p_0()
          .border(px(0.0))
          .bg(TRANSPARENT_RGBA)
          .text_color(TRANSPARENT_RGBA)
          .caret(false)
          .multiline(true)
          .scrollbar(false)
          .submit_behavior(SubmitBehavior::Submit)
          .default_value(init_val)
          .selected_range(move || {
            let _ = sel_push_rev.get();
            desired_sel.get()
          })
          .on_selection_change(move |sel: &Selection| {
            visual_sel.set((sel.start, sel.end));
            blink_reset_rev.update(|v| *v = v.wrapping_add(1));
          })
          .on_composition_change(move |comp: &Composition| {
            if comp.composing && !comp.text.is_empty() {
              let off = visual_sel.get_untracked().0;
              cell_preedit.set(Some((off, comp.text.clone())));
            } else {
              cell_preedit.set(None);
            }
          })
          .on_change(move |v: &str| {
            let prev = live_text.get_untracked();
            let next_marks = splice_cell_marks(&prev, &live_marks.get_untracked(), v);
            live_text.set(v.to_owned());
            live_marks.set(next_marks.clone());
            cell_preedit.set(None);
            blink_reset_rev.update(|v| *v = v.wrapping_add(1));
            br_c2.table_op_live(
              live_index(at),
              TableOp::SetCell {
                row: r,
                col: c,
                cell: TableCell::new(v, next_marks),
              },
            );
          })
          .on_submit(move |_: &str| {
            let next_r = r + 1;
            if next_r < row_count {
              focused_cell.set(Some((bid_n2.clone(), next_r, c)));
            } else {
              br_s2.table_op(
                live_index(at),
                TableOp::Navigate {
                  row: r,
                  col: c,
                  direction: TableNavDirection::Tab,
                },
              );
              focused_cell.set(Some((bid_n2.clone(), next_r, 0)));
            }
          })
      }
    }));

  let mut cell_box = view()
    .test_id(&format!("table-cell-{init_idx}-{r}-{c}"))
    .w(move || px(grid_w.get() * share_c))
    .flex_shrink_0()
    .min_w(px(0.0))
    .overflow_hidden()
    .px_3()
    .py_2()
    .cursor(Cursor::Text)
    .bg(move || {
      if is_in_multi_sel.get() {
        palette(dark_mode.get()).selection_bg
      } else {
        TRANSPARENT_RGBA
      }
    })
    .on_mouse_enter(move |_: &MouseEvent| {
      hovered_cell.set(Some((r, c)));
    })
    .on_press_down(move |e: &PressEvent| {
      drag_anchor.set(None);
      marquee_selected.set(None);
      br_down.clear_selection();
      editor_focused.set(false);
      table_selection.set(None);
      table_drag_anchor.set(Some((bid_down.clone(), r, c)));

      let off = hit_down(e.page_position.x.0, e.page_position.y.0);
      cell_text_drag_anchor.set(Some(off));
      desired_sel.set((off, off));
      visual_sel.set((off, off));
      sel_push_rev.update(|v| *v = v.wrapping_add(1));
      blink_reset_rev.update(|v| *v = v.wrapping_add(1));
      focused_cell.set(Some((bid_down.clone(), r, c)));
    })
    .on_mouse_move(move |e: &MouseEvent| {
      if e.pressed_button != Some(MouseButton::Left) {
        return;
      }
      let Some((ref anchor_bid, ar, ac)) = table_drag_anchor.get_untracked() else {
        return;
      };
      if anchor_bid != &bid_move {
        return;
      }
      if ar != r || ac != c || table_selection.get_untracked().is_some() {
        cell_text_drag_anchor.set(None);
        if focused_cell.get_untracked().is_some() {
          focused_cell.set(None);
        }
        let next = Some((bid_move.clone(), (ar, ac), (r, c)));
        if table_selection.get_untracked() != next {
          table_selection.set(next);
        }
      } else if let Some(a_off) = cell_text_drag_anchor.get_untracked() {
        // Intra-cell text selection drag while staying inside the same cell
        let cur_off = hit_move(e.page_position.x.0, e.page_position.y.0);
        let range = (a_off.min(cur_off), a_off.max(cur_off));
        if visual_sel.get_untracked() != range {
          desired_sel.set(range);
          visual_sel.set(range);
          sel_push_rev.update(|v| *v = v.wrapping_add(1));
        }
      }
    })
    .on_mouse_up(move |_: &MouseEvent| {
      table_drag_anchor.set(None);
      cell_text_drag_anchor.set(None);
    })
    .on_press(move || {
      // Headless `app.press("id:table-cell-...")` fallback
      if table_selection.get_untracked().is_none() {
        let end = utf16_len(&live_text.get_untracked());
        desired_sel.set((end, end));
        visual_sel.set((end, end));
        sel_push_rev.update(|v| *v = v.wrapping_add(1));
        blink_reset_rev.update(|v| *v = v.wrapping_add(1));
        focused_cell.set(Some((bid_press.clone(), r, c)));
      }
    })
    .child(text_box);

  if c > 0 {
    cell_box = cell_box
      .border_l_1()
      .border_color(move || palette(dark_mode.get()).border_subtle);
  }
  cell_box
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn adaptive_table_column_widths_give_long_content_more_space() {
    let model = TableModel::new(
      vec![
        vec![TableCell::plain("项"), TableCell::plain("值")],
        vec![
          TableCell::plain("版本"),
          TableCell::plain("Ubuntu 24.04.3 LTS / CUDA 13.0 / PyTorch 2.8 / NCCL 2.27"),
        ],
      ],
      true,
      "left",
      0.0,
      None,
      None,
    );
    let (wide, shares, _px) = compute_adaptive_table_layout(&model);
    assert!(!wide);
    assert_eq!(shares.len(), 2);
    assert!(
      shares[0] < 0.25 && shares[1] > 0.75,
      "short key column should stay compact (<25%), long value column >75%, got {:?}",
      shares
    );
  }

  #[test]
  fn splice_cell_marks_preserves_and_shifts_bold_marks_across_edits() {
    // "单机柜可用功率 (kW)" with bold over "单机柜可用功率" (u16 0..7)
    let old = "单机柜可用功率 (kW)";
    let marks = vec![EditorMark::new(0, 7, "bold")];

    // Insert prefix "A: " (3 u16) -> bold shifts to 3..10
    let m1 = splice_cell_marks(old, &marks, "A: 单机柜可用功率 (kW)");
    assert_eq!(m1.len(), 1);
    assert_eq!((m1[0].start, m1[0].end), (3, 10));

    // Append suffix " / A+B" after bold -> bold stays 0..7
    let m2 = splice_cell_marks(old, &marks, "单机柜可用功率 (kW) / A+B");
    assert_eq!(m2.len(), 1);
    assert_eq!((m2[0].start, m2[0].end), (0, 7));
  }

  #[test]
  fn apply_column_divider_drag_redistributes_adjacent_columns_cleanly() {
    let shares = vec![0.3, 0.3, 0.4];
    let next = apply_column_divider_drag(&shares, 1, 60.0, 600.0).unwrap();
    assert!((next[0] - 0.4).abs() < 1e-4);
    assert!((next[1] - 0.2).abs() < 1e-4);
    assert!((next[2] - 0.4).abs() < 1e-4);
    assert!(apply_column_divider_drag(&shares, 1, 1.0, 600.0).is_none());
  }
}
