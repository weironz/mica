//! Pure-Rust virtualized document editor view built on `crates/mica-editor` +
//! NSH native `virtual_list` (`AGENTS.md` Route 2-B):
//! - Responsive page header: selectable breadcrumb + real path copy (left) ·
//!   toggleable formatting toolbar with live active states (center/2nd row) ·
//!   last-edited, right-sidebar toggle, whiteboard, share (cloud), copy-link,
//!   favorite star, trash, and full `⋯` page menu (right).
//! - Full-width `virtual_list` whose scrollbar sits cleanly on the right edge
//!   of the window, with `.header(...)` (scrolling 30px SemiBold page title +
//!   `PropertyPanel`), per-block virtualized rows subscribed ONLY to per-block
//!   `Signal<u64>` (NEVER rebuilding siblings or `virtual_list` keys on typing),
//!   `.footer(...)` (`BacklinksPanel` + bottom click-to-focus breathing room),
//!   single hidden native IME proxy `input()`, top-level `overlay()` code-block
//!   popovers, fullscreen Mermaid/Image zoom lightbox modal, caret-anchored
//!   22-item `/` slash menu, floating selection bubble menu, `Ctrl+F` find bar,
//!   and floating bottom-right word count badge.

use std::{
  cell::{Cell, RefCell},
  rc::Rc,
  time::Duration,
};

use mica_editor::{
  DocPosition, DocSelection, EditorCommand, FindMatch, TableNavDirection, TableOp,
  utf16::utf16_len,
};
use nsh_base::{ScrollbarMode, button, input, scrollbar};
use nsh_core::{
  PointerType, SubmitBehavior, TouchEvent, TouchPhase,
  color::Rgba,
  element::{
    ElementRef, KeyOptions, Spawned, element_ref, on_key_with, set_clipboard, set_interval,
    text_measurer, virtual_list,
  },
  geometry::{Bounds, Points, px},
  keymap::When,
  prelude::*,
};

use crate::{
  block_marquee::{MARQUEE_DRAG_SLOP, RectF, starts_marquee_in_margin},
  block_views::{
    BlockRenderEnv, LightboxContent, OpenBoardCallback, TableCellSelection, render_lightbox_modal,
    render_virtual_block_row,
  },
  editor_bridge::{MicaDocumentBridge, ScrollMode},
  editor_positioning::{
    compute_keep_visible_scroll_target, schedule_bubble_anchor_update, schedule_ime_sink_pos_update,
    schedule_slash_anchor_update, schedule_touch_handles_update,
  },
  mica_icons,
  mica_theme::palette,
  text_geometry::{VisibleLayoutCache, caret_rect_for_offset},
  touch_selection::{
    TouchHandleAnchors, TouchHandlesProps, TouchTapTracker, touch_selection_handles_layer,
  },
};

pub use crate::editor_bridge::OutlineItem;

pub(crate) const SLASH_MENU_WIDTH: f32 = 248.0;
pub(crate) const SLASH_MENU_MAX_H: f32 = 288.0;
pub(crate) const SINK_LIMIT: usize = 64;
const TRANSPARENT_RGBA: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// Ambient trigger incremented whenever a brand-new blank page is created so
/// `editor_view` immediately focuses the page title input (`page-title-input`).
#[derive(Clone, Copy)]
pub struct FocusTitleSignal(pub Signal<u64>);

/// Hidden native IME sink buffer tracker (mirrors NSH's battle-tested sink state).
#[derive(Default)]
pub(crate) struct ImeSinkState {
  pub(crate) text: String,
  pub(crate) emptied: Option<String>,
  pub(crate) composing: bool,
  pub(crate) held: Option<String>,
}

pub struct EditorHeaderProps {
  pub workspace_name: Signal<String>,
  pub folder_name: Signal<String>,
  pub page_title: Signal<String>,
  pub page_id_str: Signal<String>,
  pub is_favorite: Signal<bool>,
  pub right_panel_open: Signal<bool>,
  pub on_rename_title: Rc<dyn Fn(String)>,
  pub on_title_focus_change: Option<Rc<dyn Fn(bool)>>,
  pub on_open_board: Rc<dyn Fn()>,
  pub on_toggle_favorite: Rc<dyn Fn()>,
  pub on_toggle_right_panel: Rc<dyn Fn()>,
  pub on_trash_page: Rc<dyn Fn()>,
  pub on_export_markdown: Rc<dyn Fn()>,
  pub on_export_html: Rc<dyn Fn()>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  // ── 1:1 Parity Props ─────────────────────────────────────────────────────
  pub is_cloud_signed_in: Signal<bool>,
  pub sync_phase: Signal<mica_sync::SyncPhase>,
  pub sync_badge: Signal<String>,
  pub show_format_bar: Signal<bool>,
  pub show_page_title: Signal<bool>,
  pub show_properties: Signal<bool>,
  pub comments_visible: Signal<bool>,
  pub page_width: Signal<f32>,
  pub ai_enabled: Signal<bool>,
  pub on_share: Rc<dyn Fn()>,
  pub on_toggle_comments: Rc<dyn Fn()>,
  pub on_toggle_properties: Rc<dyn Fn()>,
  pub on_export_zip: Rc<dyn Fn()>,
  pub on_export_pdf: Rc<dyn Fn()>,
  pub on_import_markdown_picker: Rc<dyn Fn()>,
  pub on_open_version_history: Rc<dyn Fn()>,
  pub on_inline_ai: Rc<dyn Fn(String)>,
  pub on_insert_image: Rc<dyn Fn()>,
  pub image_ctx: crate::image_blocks::ImageContext,
  pub property_panel_slot: Option<Rc<dyn Fn() -> AnyElement>>,
  pub backlinks_panel_slot: Option<Rc<dyn Fn() -> AnyElement>>,
}

#[derive(Clone, Copy)]
pub(crate) struct SlashMenuSignals {
  pub(crate) open: Signal<bool>,
  pub(crate) query: Signal<String>,
  pub(crate) index: Signal<usize>,
  pub(crate) anchor: Signal<(f32, f32)>,
  pub(crate) opened_by_btn: Signal<bool>,
  pub(crate) box_size: Signal<(f32, f32)>,
}

pub fn editor_view(
  bridge: Rc<MicaDocumentBridge>,
  header: Rc<EditorHeaderProps>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  // Allow embedded `board` views to open the full whiteboard.
  provide_context(OpenBoardCallback(header.on_open_board.clone()));

  let page_menu_open = signal(false);
  let page_more_btn_ref: ElementRef = element_ref();
  let has_text_selection = signal(false);
  let copied_path = signal(false);
  let copied_path_gen = signal(0u64);
  let copied_link = signal(false);

  // Caret-anchored slash menu state
  let slash = SlashMenuSignals {
    open: signal(false),
    query: signal(String::new()),
    index: signal(0usize),
    anchor: signal((12.0f32, 8.0f32)),
    opened_by_btn: signal(false),
    box_size: signal((740.0f32, 620.0f32)),
  };

  // Inline link prompt state (used by both format bar & bubble menu)
  let link_prompt_open = signal(false);
  let link_draft = signal(String::new());

  // In-page find bar (`Ctrl+F`) state
  let find_open = signal(false);
  let find_query = signal(String::new());
  let find_matches: Signal<Vec<FindMatch>> = signal(Vec::new());
  let find_index = signal(0usize);

  // Virtualized block environment + native IME sink state
  let layouts = Rc::new(RefCell::new(VisibleLayoutCache::default()));
  let drag_anchor: Signal<Option<DocPosition>> = signal(None);
  let lightbox: Signal<Option<LightboxContent>> = signal(None);
  let focused_code_block: Signal<Option<String>> = signal(None);
  let focused_table_cell: Signal<Option<(String, usize, usize)>> = signal(None);
  let table_selection: Signal<Option<TableCellSelection>> = signal(None);
  let table_drag_anchor: Signal<Option<(String, usize, usize)>> = signal(None);
  let body_blank_marquee_ready = signal(false);
  let marquee_origin_page: Signal<Option<(f32, f32)>> = signal(None);
  let last_click: Signal<(usize, u32, f64, u8)> = signal((0, 0, 0.0, 0));
  let inner_drag_hit: Rc<Cell<Option<(f32, f32)>>> = Rc::new(Cell::new(None));
  let last_pointer_is_touch = signal(false);
  let touch_selection_active = signal(false);
  let touch_handle_dragging = signal(false);
  let touch_handle_anchors: Signal<TouchHandleAnchors> = signal(TouchHandleAnchors::default());
  let touch_tap_tracker = Rc::new(RefCell::new(TouchTapTracker::default()));
  let paper_page_origin: Signal<(f32, f32)> = signal((0.0, 0.0));

  let list_ref = element_ref();
  let sink_ref = element_ref();
  let paper_ref = element_ref();
  let title_ref = element_ref();
  let editor_focused = signal(false);
  let suppress_next_enter = signal(false);
  let body_ctx_menu: Signal<Option<(f32, f32)>> = signal(None);
  let caret_blink_on = signal(true);
  let blink_task: Rc<RefCell<Option<Spawned<()>>>> = Rc::new(RefCell::new(None));
  let sink_value = signal(String::new());
  let sink_clear_rev = signal(0u64);
  let sink_state = Rc::new(RefCell::new(ImeSinkState::default()));
  let marquee_row_rects: Rc<RefCell<Vec<(usize, RectF)>>> = Rc::new(RefCell::new(Vec::new()));
  let scroll_offset_y = signal(0.0_f32);
  let scroll_content_h = signal(0.0_f32);
  let scroll_viewport_h = signal(0.0_f32);
  let autoscroll_speed = signal(0.0_f32);
  let autoscroll_busy = signal(false);
  let autoscroll_last_xy = signal((0.0_f32, 0.0_f32));
  let bubble_anchor: Signal<(f32, f32)> = signal((120.0, 48.0));
  let bubble_recalc_rev = signal(0u64);
  let ime_sink_pos: Signal<(f32, f32)> = signal((0.0, 0.0));
  let preferred_caret_x: Signal<Option<f32>> = signal(None);

  let ai_enabled = header.ai_enabled;
  let on_inline_ai_key = header.on_inline_ai.clone();
  let on_insert_image_key = header.on_insert_image.clone();

  // Sync `has_text_selection`, slash menu anchor, IME sink coordinates, and
  // bubble menu coordinates reactively when selection changes.
  // IMPORTANT: NEVER clear `sink_value` here! Clearing the native Win32/macOS Edit
  // sink on every typed character races with `WM_CHAR`/`EN_CHANGE`, producing doubled
  // characters (`ttest`) and dropped symbols (like backtick `` ` ``).
  {
    let sel_sig = bridge.selection_signal();
    let b_bub = bridge.clone();
    let l_bub = layouts.clone();
    let pw_bub = header.page_width;
    let pref_bub = paper_ref;
    effect(move || {
      let _ = bubble_recalc_rev.get();
      let sel = sel_sig.get();
      let is_range = sel.as_ref().is_some_and(|s| !s.is_collapsed());
      has_text_selection.set(is_range);
      if copied_link.get_untracked() {
        copied_link.set(false);
      }
      if is_range && slash.open.get_untracked() {
        close_slash(slash);
      }
      if let Some(s) = sel {
        if !s.is_collapsed() {
          schedule_bubble_anchor_update(
            &b_bub,
            &l_bub,
            pref_bub,
            pw_bub.get_untracked(),
            s.clone(),
            bubble_anchor,
          );
          schedule_touch_handles_update(&b_bub, &l_bub, pref_bub, s, touch_handle_anchors);
        } else {
          if touch_selection_active.get_untracked() && !touch_handle_dragging.get_untracked() {
            touch_selection_active.set(false);
          }
          touch_handle_anchors.set(TouchHandleAnchors::default());
          // Keep hidden native IME proxy pinned beside the active caret so OS
          // CJK candidate windows open right next to where the user is typing.
          schedule_ime_sink_pos_update(&b_bub, &l_bub, pref_bub, s.focus_pos(), ime_sink_pos);
        }
      } else {
        if touch_selection_active.get_untracked() && !touch_handle_dragging.get_untracked() {
          touch_selection_active.set(false);
        }
        touch_handle_anchors.set(TouchHandleAnchors::default());
      }
    });
  }

  // Anchor `/` slash menu directly below/above the active caret via live measure()
  // whenever the slash menu opens or the caret moves while it is open (CAP-04-04).
  {
    let b_sl = bridge.clone();
    let l_sl = layouts.clone();
    let pref_sl = paper_ref;
    let sel_sl = bridge.selection_signal();
    effect(move || {
      if !slash.open.get() {
        return;
      }
      let _ = sel_sl.get();
      schedule_slash_anchor_update(
        &b_sl,
        &l_sl,
        pref_sl,
        SLASH_MENU_WIDTH,
        SLASH_MENU_MAX_H,
        slash.anchor,
      );
    });
  }

  // Caret blink timer: solid immediately upon focus or selection move, then blinks
  // smoothly at 530ms intervals while focused; cancelled cleanly on blur / unmount.
  {
    let sel_sig = bridge.selection_signal();
    let bt = blink_task.clone();
    effect(move || {
      let focused = editor_focused.get();
      let sel = sel_sig.get();
      if let Some(prev) = bt.borrow_mut().take() {
        prev.cancel();
      }
      let collapsed = sel.as_ref().is_some_and(|s| s.is_collapsed());
      if !focused || !collapsed {
        caret_blink_on.set(false);
        return;
      }
      caret_blink_on.set(true);
      let task = set_interval(Duration::from_millis(530), move || {
        caret_blink_on.update(|b| *b = !*b);
      });
      *bt.borrow_mut() = Some(task);
    });
  }

  // Clear hidden IME sink ONLY on explicit non-typing cursor repositions (click).
  {
    let st_clear = sink_state.clone();
    effect(move || {
      if sink_clear_rev.get() > 0 {
        empty_ime_sink(&st_clear, sink_value);
      }
    });
  }

  // Prune stale layout/row refs AND clear orphaned code-block / table-cell focus
  // whenever block structure changes (delete / page switch) so stale focus from
  // a previous page never blocks prose shortcuts on the newly opened page!
  {
    let struct_rev = bridge.structure_rev_signal();
    let b_prune = bridge.clone();
    let l_prune = layouts.clone();
    effect(move || {
      let _ = struct_rev.get();
      let ids = b_prune.block_ids();
      let live: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
      l_prune.borrow_mut().prune_to_live(&live);
      if let Some(fid) = focused_code_block.get_untracked()
        && !live.contains(fid.as_str())
      {
        focused_code_block.set(None);
      }
      if let Some((tid, _, _)) = focused_table_cell.get_untracked()
        && !live.contains(tid.as_str())
      {
        focused_table_cell.set(None);
      }
      if let Some((tid, _, _)) = table_selection.get_untracked()
        && !live.contains(tid.as_str())
      {
        table_selection.set(None);
      }
      if let Some((tid, _, _)) = table_drag_anchor.get_untracked()
        && !live.contains(tid.as_str())
      {
        table_drag_anchor.set(None);
      }
    });
  }

  // Table multi-cell selection shortcuts (registered ONLY while a rectangular
  // table selection is active so they never interfere with single-cell editing).
  {
    let b_tbl = bridge.clone();
    effect(move || {
      if table_selection.get().is_none() {
        return;
      }
      {
        let b_copy = b_tbl.clone();
        on_key_with(
          "mod+c",
          KeyOptions {
            when: When::Always,
            ..KeyOptions::default()
          },
          move |_| {
            if let Some((bid, a, f)) = table_selection.get_untracked()
              && let Some(idx) = b_copy.index_of_block_id(&bid)
              && let Some(payload) = b_copy.copy_table_area(
                idx,
                a.0.min(f.0),
                a.1.min(f.1),
                a.0.max(f.0),
                a.1.max(f.1),
              )
            {
              write_rich_clipboard(payload);
            }
          },
        );
      }
      {
        let b_cut = b_tbl.clone();
        on_key_with(
          "mod+x",
          KeyOptions {
            when: When::Always,
            ..KeyOptions::default()
          },
          move |_| {
            if let Some((bid, a, f)) = table_selection.get_untracked()
              && let Some(idx) = b_cut.index_of_block_id(&bid)
            {
              let (r0, r1) = (a.0.min(f.0), a.0.max(f.0));
              let (c0, c1) = (a.1.min(f.1), a.1.max(f.1));
              if let Some(payload) = b_cut.copy_table_area(idx, r0, c0, r1, c1) {
                write_rich_clipboard(payload);
              }
              b_cut.table_op(idx, TableOp::ClearCells { r0, c0, r1, c1 });
            }
          },
        );
      }
      for key in ["delete", "backspace"] {
        let b_clr = b_tbl.clone();
        on_key_with(
          key,
          KeyOptions {
            when: When::Always,
            ..KeyOptions::default()
          },
          move |_| {
            if let Some((bid, a, f)) = table_selection.get_untracked()
              && let Some(idx) = b_clr.index_of_block_id(&bid)
            {
              let (r0, r1) = (a.0.min(f.0), a.0.max(f.0));
              let (c0, c1) = (a.1.min(f.1), a.1.max(f.1));
              b_clr.table_op(idx, TableOp::ClearCells { r0, c0, r1, c1 });
            }
          },
        );
      }
    });
  }

  // Focused table cell keyboard navigation (`Tab`, `Shift+Tab`, `Up`, `Down`, `Escape`)
  // implementing `CAP-05-06`: moves smoothly across cells, inserts a trailing row on
  // bottom-right `Tab`, and exits to neighboring prose blocks at table boundaries.
  {
    let b_tcell = bridge.clone();
    effect(move || {
      if focused_table_cell.get().is_none() {
        return;
      }
      let nav_keys: &[(&str, TableNavDirection)] = &[
        ("tab", TableNavDirection::Tab),
        ("shift+tab", TableNavDirection::ShiftTab),
        ("up", TableNavDirection::Up),
        ("down", TableNavDirection::Down),
      ];
      for &(stroke, dir) in nav_keys {
        let b_nav = b_tcell.clone();
        on_key_with(
          stroke,
          KeyOptions {
            when: When::Always,
            for_focus: true,
            ..KeyOptions::default()
          },
          move |_| {
            let Some((bid, r, c)) = focused_table_cell.get_untracked() else {
              return;
            };
            let Some(idx) = b_nav.index_of_block_id(&bid) else {
              focused_table_cell.set(None);
              return;
            };
            let Some(blk) = b_nav.block_at(idx) else {
              focused_table_cell.set(None);
              return;
            };
            let model = mica_editor::TableModel::from_block_data(&blk.data);
            match model.navigate(r, c, dir) {
              mica_editor::TableNavResult::Cell { row, col } => {
                focused_table_cell.set(Some((bid, row, col)));
              }
              mica_editor::TableNavResult::InsertRowAndFocus { row, col } => {
                b_nav.table_op(
                  idx,
                  TableOp::Navigate {
                    row: r,
                    col: c,
                    direction: dir,
                  },
                );
                focused_table_cell.set(Some((bid, row, col)));
              }
              mica_editor::TableNavResult::ExitBefore => {
                focused_table_cell.set(None);
                if idx > 0 {
                  let prev_len = b_nav
                    .block_at(idx - 1)
                    .map(|b| utf16_len(&b.text))
                    .unwrap_or(0);
                  b_nav.focus_block(idx - 1, prev_len);
                } else {
                  b_nav.focus_block(0, 0);
                }
              }
              mica_editor::TableNavResult::ExitAfter => {
                focused_table_cell.set(None);
                if idx + 1 < b_nav.block_len() {
                  b_nav.focus_block(idx + 1, 0);
                } else {
                  b_nav.append_empty_paragraph_and_focus();
                }
              }
            }
          },
        );
      }
      {
        let b_esc = b_tcell.clone();
        on_key_with(
          "escape",
          KeyOptions {
            when: When::Always,
            for_focus: true,
            ..KeyOptions::default()
          },
          move |_| {
            focused_table_cell.set(None);
            b_esc.request_focus();
          },
        );
      }
    });
  }

  // Release hidden prose IME sink whenever focus enters a code block or table cell,
  // so `Backspace` / `Delete` / arrows belong exclusively to the nested editor!
  {
    let sref = sink_ref;
    let st_blur = sink_state.clone();
    effect(move || {
      if focused_code_block.get().is_some() || focused_table_cell.get().is_some() {
        empty_ime_sink(&st_blur, sink_value);
        sref.blur();
        editor_focused.set(false);
      }
    });
  }

  // Focus native IME sink whenever `bridge.request_focus()` increments `focus_sink_rev`.
  {
    let focus_rev = bridge.focus_sink_rev_signal();
    let sref = sink_ref;
    let st_focus = sink_state.clone();
    effect(move || {
      let rev = focus_rev.get();
      if rev > 0 {
        focused_code_block.set(None);
        focused_table_cell.set(None);
        empty_ime_sink(&st_focus, sink_value);
        sref.focus();
      }
    });
  }

  // Focus the page title input directly when creating a brand-new blank page.
  if let Some(FocusTitleSignal(title_focus_rev)) = use_context::<FocusTitleSignal>() {
    let tref = title_ref;
    let sref = sink_ref;
    let st_blur = sink_state.clone();
    let show_title = header.show_page_title;
    let br_fallback = bridge.clone();
    effect(move || {
      let rev = title_focus_rev.get();
      if rev > 0 {
        focused_code_block.set(None);
        focused_table_cell.set(None);
        empty_ime_sink(&st_blur, sink_value);
        sref.blur();
        editor_focused.set(false);
        if show_title.get_untracked() {
          tref.focus();
          crate::focus_utils::focus_on_next_tick(tref, || true);
        } else {
          br_fallback.request_focus();
        }
      }
    });
  }

  // Scroll `virtual_list` according to ScrollMode:
  // - `Jump` (page switch, outline jump, find match): align target cleanly into view.
  // - `KeepVisible` (Enter, Backspace, typing, arrows): measure the active caret's
  //   vertical span (falling back to the row bounds for atomic blocks) against the
  //   viewport and nudge `scroll_offset_y` minimally ONLY when the caret actually
  //   leaves the visible bounds — NEVER yanking a tall clipped code block's top
  //   back into view when pressing Enter at the bottom!
  {
    let scroll_req = bridge.scroll_request_signal();
    let b_scroll = bridge.clone();
    let l_scroll = layouts.clone();
    let lref = list_ref;
    let pref = paper_ref;
    let pw_scroll = header.page_width;
    effect(move || {
      if let Some((idx, mode, _seq)) = scroll_req.get() {
        match mode {
          ScrollMode::Jump => {
            if idx == 0 {
              lref.scroll_to_offset(px(0.0), false);
            } else {
              lref.scroll_to_index(idx as u32, Percentage(0.2), px(0.0), false);
            }
          }
          ScrollMode::KeepVisible => {
            let b_sc = b_scroll.clone();
            let l_sc = l_scroll.clone();
            spawn(async move {
              // Yield one tick so newly split rows (e.g. pressing Enter) mount into
              // `virtual_list` and register their `row_ref` before deciding whether
              // scrolling is actually needed, and coalesce rapid key-repeat bursts.
              timer(Duration::from_millis(16)).await;
              if scroll_req.get_untracked().map(|(_, _, s)| s) != Some(_seq) {
                return;
              }
              let bid_opt = b_sc.block_id_at(idx);
              let (row_ref_opt, text_ref_opt) = {
                let cache = l_sc.borrow();
                let r = bid_opt
                  .as_deref()
                  .and_then(|bid| cache.row_refs.get(bid).map(|(_, r)| *r));
                let t = bid_opt
                  .as_deref()
                  .and_then(|bid| cache.text_refs.get(bid).copied());
                (r, t)
              };
              let Some(rref) = row_ref_opt else {
                // Target row is genuinely outside the mounted virtual window -> bring it in.
                if idx == 0 {
                  lref.scroll_to_offset(px(0.0), false);
                } else {
                  lref.scroll_to_index(idx as u32, Percentage(0.5), px(0.0), false);
                }
                return;
              };
              let Some(pm) = pref.measure().await else {
                return;
              };
              if scroll_req.get_untracked().map(|(_, _, s)| s) != Some(_seq) {
                return;
              }
              let vh = pm.frame.size.height.0;
              if vh <= 1.0 {
                return;
              }

              // Prefer exact active caret vertical bounds when editing inside a
              // text or code block so tall clipped blocks never jump upward!
              let caret_span: Option<(f32, f32, bool)> = if let (Some(bid), Some(tref), Some(sel)) =
                (bid_opt.as_deref(), text_ref_opt, b_sc.selection())
                && sel.focus_node() == idx
                && let Some(tm) = tref.measure().await
              {
                let blk = b_sc.block_at(idx);
                let blk_text = blk.as_ref().map(|b| b.text.as_str()).unwrap_or("");
                let is_code = blk
                  .as_ref()
                  .is_some_and(|b| crate::editor_bridge::effective_block_kind(b) == "code_block");
                let default_font = if is_code { "14px monospace" } else { "16px sans-serif" };
                let default_lh = if is_code { 22.0_f32 } else { 26.0_f32 };
                let eff_pw = pw_scroll.get_untracked();
                let fallback_w = if crate::page_width::is_full_width(eff_pw) {
                  820.0
                } else {
                  (eff_pw - 64.0).max(200.0)
                };
                let measure = text_measurer();
                let (_, cy, ch) = {
                  let cache = l_sc.borrow();
                  let (font, lh) = cache
                    .blocks
                    .get(bid)
                    .map(|i| {
                      (
                        if i.font.is_empty() {
                          default_font.to_owned()
                        } else {
                          i.font.clone()
                        },
                        if i.line_height > 1.0 {
                          i.line_height
                        } else {
                          default_lh
                        },
                      )
                    })
                    .unwrap_or_else(|| (default_font.to_owned(), default_lh));
                  let lines = cache.resolve_lines_cached(
                    bid,
                    blk_text,
                    fallback_w,
                    &font,
                    lh,
                    &measure,
                  );
                  caret_rect_for_offset(&lines, sel.focus_pos().offset, &font, lh, &measure)
                };
                let rel_top = (tm.page.y.0 - pm.page.y.0) + cy;
                Some((rel_top, rel_top + ch.max(default_lh), cy < 4.0))
              } else {
                None
              };

              if scroll_req.get_untracked().map(|(_, _, s)| s) != Some(_seq) {
                return;
              }

              let (rel_top, rel_bottom, at_first_line) = if let Some(span) = caret_span {
                span
              } else {
                let Some(rm) = rref.measure().await else {
                  return;
                };
                if scroll_req.get_untracked().map(|(_, _, s)| s) != Some(_seq) {
                  return;
                }
                let rt = rm.page.y.0 - pm.page.y.0;
                (rt, rt + rm.frame.size.height.0, true)
              };

              if idx == 0 && at_first_line && rel_top < 12.0 {
                lref.scroll_to_offset(px(0.0), false);
                return;
              }
              let cur_scroll = scroll_offset_y.get_untracked();
              if let Some(next_y) =
                compute_keep_visible_scroll_target(cur_scroll, vh, rel_top, rel_bottom)
              {
                lref.scroll_to_offset(px(next_y), false);
              }
            });
          }
        }
      }
    });
  }

  // Focused editor keyboard bindings (automatically unregistered when focus moves to
  // title input, code block editor, table cell, find bar, search, or whiteboard!).
  let show_page_title_key = header.show_page_title;
  let page_title_key = header.page_title;
  let img_ctx_key = header.image_ctx.clone();
  {
    let b_keys = bridge.clone();
    let l_keys = layouts.clone();
    let st_keys = sink_state.clone();
    let on_ai_k = on_inline_ai_key.clone();
    let on_img_k = on_insert_image_key.clone();
    let img_ctx_k = img_ctx_key.clone();
    effect(move || {
      if !editor_focused.get()
        || focused_code_block.get().is_some()
        || focused_table_cell.get().is_some()
      {
        return;
      }
      let bindings: &[(&str, bool, EditorKeyAction)] = &[
        ("enter", true, EditorKeyAction::Enter),
        ("shift+enter", true, EditorKeyAction::SoftBreak),
        ("backspace", true, EditorKeyAction::Backspace),
        ("delete", true, EditorKeyAction::Delete),
        ("tab", false, EditorKeyAction::Indent(1)),
        ("shift+tab", false, EditorKeyAction::Indent(-1)),
        ("left", true, EditorKeyAction::ArrowLeft { shift: false, word: false }),
        ("shift+left", true, EditorKeyAction::ArrowLeft { shift: true, word: false }),
        ("mod+left", true, EditorKeyAction::ArrowLeft { shift: false, word: true }),
        ("mod+shift+left", true, EditorKeyAction::ArrowLeft { shift: true, word: true }),
        ("right", true, EditorKeyAction::ArrowRight { shift: false, word: false }),
        ("shift+right", true, EditorKeyAction::ArrowRight { shift: true, word: false }),
        ("mod+right", true, EditorKeyAction::ArrowRight { shift: false, word: true }),
        ("mod+shift+right", true, EditorKeyAction::ArrowRight { shift: true, word: true }),
        ("up", true, EditorKeyAction::ArrowUp { shift: false }),
        ("shift+up", true, EditorKeyAction::ArrowUp { shift: true }),
        ("down", true, EditorKeyAction::ArrowDown { shift: false }),
        ("shift+down", true, EditorKeyAction::ArrowDown { shift: true }),
        ("home", false, EditorKeyAction::Home { shift: false }),
        ("shift+home", false, EditorKeyAction::Home { shift: true }),
        ("end", false, EditorKeyAction::End { shift: false }),
        ("shift+end", false, EditorKeyAction::End { shift: true }),
        ("mod+a", false, EditorKeyAction::SelectAll),
        ("mod+c", false, EditorKeyAction::Copy),
        ("mod+x", false, EditorKeyAction::Cut),
        ("mod+shift+v", false, EditorKeyAction::PastePlain),
        ("mod+z", true, EditorKeyAction::Undo),
        ("mod+y", true, EditorKeyAction::Redo),
        ("mod+shift+z", true, EditorKeyAction::Redo),
        ("mod+b", false, EditorKeyAction::ToggleMark("bold")),
        ("mod+i", false, EditorKeyAction::ToggleMark("italic")),
        ("mod+e", false, EditorKeyAction::ToggleMark("code")),
        ("mod+shift+s", false, EditorKeyAction::ToggleMark("strike")),
        ("mod+shift+h", false, EditorKeyAction::ToggleMark("highlight")),
        ("mod+k", false, EditorKeyAction::OpenLink),
        ("escape", false, EditorKeyAction::Escape),
      ];
      for &(stroke, repeat, action) in bindings {
        let br = b_keys.clone();
        let lay = l_keys.clone();
        let st = st_keys.clone();
        let ai_cb = on_ai_k.clone();
        let img_cb = on_img_k.clone();
        let img_ctx_c = img_ctx_k.clone();
        on_key_with(
          stroke,
          KeyOptions {
            when: When::Always,
            repeat,
            for_focus: true,
            ..KeyOptions::default()
          },
          move |_| {
            if st.borrow().composing {
              return;
            }
            if matches!(action, EditorKeyAction::Enter) && suppress_next_enter.get_untracked() {
              suppress_next_enter.set(false);
              return;
            }
            // Empty IME sink before structural/navigation commands so the next
            // typed character diffs cleanly against an empty buffer.
            empty_ime_sink(&st, sink_value);
            handle_editor_key(
              action,
              &br,
              &lay,
              preferred_caret_x,
              slash,
              ai_enabled,
              link_prompt_open,
              link_draft,
              &ai_cb,
              &img_cb,
              &img_ctx_c,
              show_page_title_key,
              page_title_key,
              title_ref,
              sink_ref,
              editor_focused,
            );
          },
        );
      }
    });
  }

  // `Ctrl+F` opens in-page find bar.
  on_key_with(
    "mod+f",
    KeyOptions {
      when: When::Always,
      ..KeyOptions::default()
    },
    move |_| {
      page_menu_open.set(false);
      close_slash(slash);
      find_open.set(true);
    },
  );

  // `Shift+Enter` steps backwards when the in-page find bar is open.
  {
    let b_find_prev_key = bridge.clone();
    on_key_with(
      "shift+enter",
      KeyOptions {
        when: When::Always,
        pass_through: true,
        ..KeyOptions::default()
      },
      move |_| {
        if find_open.get_untracked() {
          step_find_match(&b_find_prev_key, find_matches, find_index, -1);
        }
      },
    );
  }

  let marquee = crate::block_marquee::BlockMarqueeState::new();
  let marquee_origin: Signal<Option<(f32, f32)>> = signal(None);
  let paper_bounds: Signal<(f32, f32)> = signal((960.0, 720.0));

  // Continuous 60fps edge auto-scroll timer (CAP-02-03): while dragging a text
  // selection or marquee box near/past the top or bottom viewport edge, scrolls
  // `virtual_list` smoothly by exact pixels and extends the active selection
  // even when the pointer is held stationary or moved into the top toolbar!
  {
    let br_auto = bridge.clone();
    let layouts_auto = layouts.clone();
    let m_auto = marquee.clone();
    let lref_auto = list_ref;
    let pref_auto = paper_ref;
    let rects_auto = marquee_row_rects.clone();
    // Gate the interval ONLY on a boolean memo so continuous speed changes
    // during mouse movement NEVER cancel and restart the 16ms timer!
    let autoscroll_active = computed(move || autoscroll_speed.get().abs() >= 0.1);
    effect(move || {
      if !autoscroll_active.get() {
        return;
      }
      let br_t = br_auto.clone();
      let layouts_t = layouts_auto.clone();
      let m_t = m_auto.clone();
      let rects_t = rects_auto.clone();
      let task = set_interval(Duration::from_millis(16), move || {
        let speed = autoscroll_speed.get_untracked();
        if speed.abs() < 0.1 {
          return;
        }
        let has_drag =
          drag_anchor.get_untracked().is_some() || touch_handle_dragging.get_untracked();
        let has_marquee = m_t.is_active_untracked();
        if !has_drag && !has_marquee {
          autoscroll_speed.set(0.0);
          return;
        }
        let cur_y = scroll_offset_y.get_untracked();
        let ch = scroll_content_h.get_untracked();
        let vh = scroll_viewport_h.get_untracked().max(paper_bounds.get_untracked().1);
        let next_y = if ch > 0.0 {
          (cur_y + speed).clamp(0.0, (ch - vh).max(0.0))
        } else {
          (cur_y + speed).max(0.0)
        };
        let delta_y = next_y - cur_y;
        if delta_y.abs() >= 0.5 {
          scroll_offset_y.set(next_y);
          lref_auto.scroll_to_offset(px(next_y), false);
          if let Some((ox, oy)) = marquee_origin.get_untracked() {
            marquee_origin.set(Some((ox, oy - delta_y)));
          }
          if let Some((px0, py0)) = marquee_origin_page.get_untracked() {
            marquee_origin_page.set(Some((px0, py0 - delta_y)));
          }
        }
        if autoscroll_busy.get_untracked() {
          return;
        }
        if let Some(anchor) = drag_anchor.get_untracked() {
          let count = br_t.block_len();
          if count == 0 {
            return;
          }
          if speed < 0.0 && next_y <= 1.0 {
            let next = DocSelection::from_positions(anchor, DocPosition::new(0, 0));
            if br_t.selection().as_ref() != Some(&next) {
              br_t.set_selection(next);
            }
            return;
          }
          if speed > 0.0 && ch > 0.0 && next_y >= (ch - vh).max(0.0) - 1.0 {
            let last = count - 1;
            let end_off = br_t.block_at(last).map(|b| utf16_len(&b.text)).unwrap_or(0);
            let next = DocSelection::from_positions(anchor, DocPosition::new(last, end_off));
            if br_t.selection().as_ref() != Some(&next) {
              br_t.set_selection(next);
            }
            return;
          }
          autoscroll_busy.set(true);
          let row_refs = layouts_t.borrow().snapshot_row_refs();
          let br_m = br_t.clone();
          spawn(async move {
            let Some(pm) = pref_auto.measure().await else {
              autoscroll_busy.set(false);
              return;
            };
            let mut best: Option<(usize, f32)> = None;
            for (_bid, idx, rref) in row_refs {
              if let Some(rm) = rref.measure().await {
                let top = rm.page.y.0 - pm.page.y.0;
                let bottom = top + rm.frame.size.height.0;
                if bottom > 0.0 && top < pm.frame.size.height.0 {
                  best = match best {
                    None => Some((idx, top)),
                    Some((b_idx, b_top)) => {
                      if (speed < 0.0 && top < b_top) || (speed > 0.0 && top > b_top) {
                        Some((idx, top))
                      } else {
                        Some((b_idx, b_top))
                      }
                    }
                  };
                }
              }
            }
            if let Some((target_idx, _)) = best
              && let Some(cur_anchor) = drag_anchor.get_untracked()
            {
              let target_off = if speed < 0.0 {
                0
              } else {
                br_m.block_at(target_idx).map(|b| utf16_len(&b.text)).unwrap_or(0)
              };
              let next = DocSelection::from_positions(cur_anchor, DocPosition::new(target_idx, target_off));
              if br_m.selection().as_ref() != Some(&next) {
                br_m.set_selection(next);
              }
            }
            autoscroll_busy.set(false);
          });
        } else if has_marquee {
          let (cx, cy) = autoscroll_last_xy.get_untracked();
          if let Some((ox, oy)) = marquee_origin.get_untracked() {
            let rect = RectF::from_points(ox, oy, cx, cy);
            let rects = rects_t.borrow();
            m_t.update_drag(rect, &rects);
          }
        }
      });
      cleanup(move || task.cancel());
    });
  }

  let m_esc = marquee.clone();
  let b_esc_find = bridge.clone();
  on_key_with(
    "escape",
    KeyOptions {
      when: When::Always,
      pass_through: true,
      ..KeyOptions::default()
    },
    move |_| {
      if lightbox.get_untracked().is_some() {
        lightbox.set(None);
        return;
      }
      close_slash(slash);
      page_menu_open.set(false);
      link_prompt_open.set(false);
      if find_open.get_untracked() {
        find_open.set(false);
        b_esc_find.clear_find_highlights();
      }
      m_esc.clear();
      marquee_origin.set(None);
      body_blank_marquee_ready.set(false);
      table_selection.set(None);
      table_drag_anchor.set(None);
    },
  );

  // Marquee multi-block keyboard actions: registered ONLY while a marquee block
  // selection is active so they NEVER interfere with code blocks / table cells!
  {
    let b_marquee = bridge.clone();
    let m_keys = marquee.clone();
    effect(move || {
      if m_keys.selected_range.get().is_none() {
        return;
      }
      for key in ["delete", "backspace"] {
        let b_del = b_marquee.clone();
        let m_del = m_keys.clone();
        on_key_with(
          key,
          KeyOptions {
            when: When::Always,
            ..KeyOptions::default()
          },
          move |_| {
            if let Some((lo, hi)) = m_del.selected_range.get_untracked() {
              b_del.delete_block_range(lo, hi);
              m_del.clear();
            }
          },
        );
      }
      {
        let b_copy = b_marquee.clone();
        let m_copy = m_keys.clone();
        on_key_with(
          "mod+c",
          KeyOptions {
            when: When::Always,
            ..KeyOptions::default()
          },
          move |_| {
            if let Some((lo, hi)) = m_copy.selected_range.get_untracked()
              && let Some(payload) = b_copy.copy_block_range_payload(lo, hi)
            {
              write_rich_clipboard(payload);
            }
          },
        );
      }
      {
        let b_cut = b_marquee.clone();
        let m_cut = m_keys.clone();
        on_key_with(
          "mod+x",
          KeyOptions {
            when: When::Always,
            ..KeyOptions::default()
          },
          move |_| {
            if let Some((lo, hi)) = m_cut.selected_range.get_untracked() {
              if let Some(payload) = b_cut.copy_block_range_payload(lo, hi) {
                write_rich_clipboard(payload);
              }
              b_cut.delete_block_range(lo, hi);
              m_cut.clear();
            }
          },
        );
      }
    });
  }

  let active_kind = bridge.active_kind_signal();
  let active_level = bridge.active_level_signal();
  let active_marks = bridge.active_marks_signal();
  let can_undo = bridge.can_undo_signal();
  let can_redo = bridge.can_redo_signal();
  let last_edited = bridge.last_edited_signal();
  let status_flash = bridge.status_flash_signal();
  let word_count = bridge.word_count_signal();
  let structure_rev = bridge.structure_rev_signal();

  let is_fav = header.is_favorite;
  let right_open = header.right_panel_open;
  let show_format_bar = header.show_format_bar;
  let show_page_title = header.show_page_title;
  let show_properties = header.show_properties;
  let is_cloud = header.is_cloud_signed_in;
  let sync_phase = header.sync_phase;
  let sync_badge = header.sync_badge;
  let page_width = header.page_width;

  let last_flash_gen = Cell::new(0u64);
  effect(move || {
    if let Some((msg, toast_gen)) = status_flash.get()
      && toast_gen != last_flash_gen.get()
    {
      last_flash_gen.set(toast_gen);
      crate::toast::show_toast(msg);
    }
  });

  let inline_math_edit: Signal<Option<crate::editor_positioning::InlineMathEdit>> = signal(None);
  let image_ctx_env = header.image_ctx.clone();
  let image_ctx_paste = header.image_ctx.clone();
  let image_ctx_menu = header.image_ctx.clone();
  let image_ctx_cap = header.image_ctx.clone();
  let block_env = BlockRenderEnv {
    bridge: bridge.clone(),
    dark_mode,
    layouts: layouts.clone(),
    drag_anchor,
    lightbox,
    image_ctx: image_ctx_env,
    focused_code_block,
    focused_table_cell,
    table_selection,
    table_drag_anchor,
    body_blank_marquee_ready,
    marquee_origin_page,
    last_click,
    page_width,
    editor_focused,
    caret_blink_on,
    sink_clear_rev,
    marquee_selected: marquee.selected_range,
    inline_math_edit,
    inner_drag_hit,
    last_pointer_is_touch,
    touch_selection_active,
    touch_tap_tracker,
  };

  let b_copy_path = bridge.clone();
  let b_copy_link = bridge.clone();
  let b_undo = bridge.clone();
  let b_redo = bridge.clone();
  let b_para = bridge.clone();
  let b_h1 = bridge.clone();
  let b_h2 = bridge.clone();
  let b_h3 = bridge.clone();
  let b_bold = bridge.clone();
  let b_italic = bridge.clone();
  let b_strike = bridge.clone();
  let b_highlight = bridge.clone();
  let b_code = bridge.clone();
  let b_link_btn = bridge.clone();
  let b_list = bridge.clone();
  let b_ordered = bridge.clone();
  let b_todo = bridge.clone();
  let b_quote = bridge.clone();
  let b_code_blk = bridge.clone();
  let b_div = bridge.clone();
  let b_tbl = bridge.clone();
  let b_slash_btn = bridge.clone();
  let b_slash_menu = bridge.clone();
  let b_copy_md = bridge.clone();
  let b_title_enter = bridge.clone();
  let b_body = bridge.clone();
  let b_link_pop = bridge.clone();
  let b_find = bridge.clone();
  let b_lightbox = bridge.clone();

  let h_title_val = header.clone();
  let h_title_chg = header.clone();
  let h_board = header.clone();
  let h_share = header.clone();
  let h_fav = header.clone();
  let h_right = header.clone();
  let h_copy_id = header.clone();
  let h_copy_link_id = header.clone();
  let h_img_bar = header.clone();
  let h_menu = header.clone();
  let h_slash = header.clone();
  let h_bubble = header.clone();
  let prop_slot = header.property_panel_slot.clone();
  let backlinks_slot = header.backlinks_panel_slot.clone();
  let header_width = signal(1000.0_f32);
  let header_single_row = computed(move || header_width.get() >= 1240.0);
  let header_compact = computed(move || header_width.get() < 640.0);
  let format_bar_scroll = computed(move || header_width.get() < 720.0);

  let render_format_bar: Rc<dyn Fn() -> AnyElement> = {
    let b_undo = b_undo.clone();
    let b_redo = b_redo.clone();
    let b_para = b_para.clone();
    let b_h1 = b_h1.clone();
    let b_h2 = b_h2.clone();
    let b_h3 = b_h3.clone();
    let b_bold = b_bold.clone();
    let b_italic = b_italic.clone();
    let b_strike = b_strike.clone();
    let b_highlight = b_highlight.clone();
    let b_code = b_code.clone();
    let b_link_btn = b_link_btn.clone();
    let b_list = b_list.clone();
    let b_ordered = b_ordered.clone();
    let b_todo = b_todo.clone();
    let b_outdent = bridge.clone();
    let b_indent = bridge.clone();
    let b_quote = b_quote.clone();
    let b_code_blk = b_code_blk.clone();
    let b_div = b_div.clone();
    let b_tbl = b_tbl.clone();
    let b_slash_btn = b_slash_btn.clone();
    let h_img = h_img_bar.clone();
    Rc::new(move || {
      let b_undo = b_undo.clone();
      let b_redo = b_redo.clone();
      let b_para = b_para.clone();
      let b_h1 = b_h1.clone();
      let b_h2 = b_h2.clone();
      let b_h3 = b_h3.clone();
      let b_bold = b_bold.clone();
      let b_italic = b_italic.clone();
      let b_strike = b_strike.clone();
      let b_highlight = b_highlight.clone();
      let b_code = b_code.clone();
      let b_link_btn = b_link_btn.clone();
      let b_list = b_list.clone();
      let b_ordered = b_ordered.clone();
      let b_todo = b_todo.clone();
      let b_outdent = b_outdent.clone();
      let b_indent = b_indent.clone();
      let b_quote = b_quote.clone();
      let b_code_blk = b_code_blk.clone();
      let b_div = b_div.clone();
      let b_tbl = b_tbl.clone();
      let b_slash_btn = b_slash_btn.clone();
      let h_img = h_img.clone();
      view()
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap(px(1.0))
        .child(toolbar_btn_dim(
          "ed-undo",
          "撤销 (Ctrl+Z)",
          dark_mode,
          computed(move || can_undo.get()),
          move || b_undo.undo(),
          mica_icons::undo,
        ))
        .child(toolbar_btn_dim(
          "ed-redo",
          "重做 (Ctrl+Y)",
          dark_mode,
          computed(move || can_redo.get()),
          move || b_redo.redo(),
          mica_icons::redo,
        ))
        .child(toolbar_sep(dark_mode))
        .child(toolbar_btn_active(
          "ed-para",
          "正文段落",
          dark_mode,
          computed(move || active_kind.get() == "paragraph"),
          move || b_para.set_block_type("paragraph", None),
          mica_icons::align_text,
        ))
        .child(toolbar_btn_active(
          "ed-h1",
          "一级标题",
          dark_mode,
          computed(move || active_kind.get() == "heading" && active_level.get() == 1),
          move || toggle_block_type(&b_h1, "heading", Some(1)),
          |c| mica_icons::heading_box("1", c),
        ))
        .child(toolbar_btn_active(
          "ed-h2",
          "二级标题",
          dark_mode,
          computed(move || active_kind.get() == "heading" && active_level.get() == 2),
          move || toggle_block_type(&b_h2, "heading", Some(2)),
          |c| mica_icons::heading_box("2", c),
        ))
        .child(toolbar_btn_active(
          "ed-h3",
          "三级标题",
          dark_mode,
          computed(move || active_kind.get() == "heading" && active_level.get() == 3),
          move || toggle_block_type(&b_h3, "heading", Some(3)),
          |c| mica_icons::heading_box("3", c),
        ))
        .child(toolbar_sep(dark_mode))
        .child(toolbar_btn_active(
          "ed-bold",
          "加粗 (Ctrl+B)",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "bold" || m == "strong")),
          move || b_bold.toggle_mark("bold"),
          mica_icons::bold_icon,
        ))
        .child(toolbar_btn_active(
          "ed-italic",
          "斜体 (Ctrl+I)",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "italic" || m == "em")),
          move || b_italic.toggle_mark("italic"),
          mica_icons::italic_icon,
        ))
        .child(toolbar_btn_active(
          "ed-strike",
          "删除线 (Ctrl+Shift+S)",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "strike")),
          move || b_strike.toggle_mark("strike"),
          mica_icons::strikethrough_icon,
        ))
        .child(toolbar_btn_active(
          "ed-highlight",
          "高亮 (Ctrl+Shift+H)",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "highlight")),
          move || b_highlight.toggle_mark("highlight"),
          mica_icons::highlight_icon,
        ))
        .child(toolbar_btn_active(
          "ed-code",
          "行内代码",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "code")),
          move || b_code.toggle_mark("code"),
          mica_icons::code_icon,
        ))
        .child(toolbar_btn_active(
          "ed-link",
          "链接 (Ctrl+K)",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "link")),
          move || {
            link_draft.set(b_link_btn.current_link_href().unwrap_or_default());
            link_prompt_open.set(!link_prompt_open.get_untracked());
          },
          mica_icons::link_icon,
        ))
        .child(toolbar_sep(dark_mode))
        .child(toolbar_btn_active(
          "ed-list",
          "无序列表",
          dark_mode,
          computed(move || matches!(active_kind.get().as_str(), "bulleted_list" | "bulletList")),
          move || toggle_block_type(&b_list, "bulleted_list", None),
          mica_icons::list_bullet,
        ))
        .child(toolbar_btn_active(
          "ed-ordered",
          "有序列表",
          dark_mode,
          computed(move || matches!(active_kind.get().as_str(), "numbered_list" | "orderedList")),
          move || toggle_block_type(&b_ordered, "numbered_list", None),
          mica_icons::list_ordered,
        ))
        .child(toolbar_btn_active(
          "ed-todo",
          "待办清单",
          dark_mode,
          computed(move || matches!(active_kind.get().as_str(), "todo" | "taskItem")),
          move || toggle_block_type(&b_todo, "todo", None),
          mica_icons::todo_box,
        ))
        .child(toolbar_btn(
          "ed-outdent",
          "减少缩进 (Shift+Tab)",
          dark_mode,
          signal(false),
          move || {
            b_outdent.dispatch(EditorCommand::Indent { delta: -1 });
            b_outdent.request_focus();
          },
          mica_icons::indent_decrease,
        ))
        .child(toolbar_btn(
          "ed-indent",
          "增加缩进 (Tab)",
          dark_mode,
          signal(false),
          move || {
            b_indent.dispatch(EditorCommand::Indent { delta: 1 });
            b_indent.request_focus();
          },
          mica_icons::indent_increase,
        ))
        .child(toolbar_btn_active(
          "ed-quote",
          "引用块",
          dark_mode,
          computed(move || matches!(active_kind.get().as_str(), "quote" | "blockquote")),
          move || toggle_block_type(&b_quote, "quote", None),
          mica_icons::quote_icon,
        ))
        .child(toolbar_btn_active(
          "ed-code-block",
          "代码块",
          dark_mode,
          computed(move || active_kind.get() == "code_block"),
          move || b_code_blk.insert_custom_block("code_block", "", None),
          terminal_icon,
        ))
        .child(toolbar_sep(dark_mode))
        .child(toolbar_btn(
          "ed-divider",
          "插入分割线",
          dark_mode,
          signal(false),
          move || b_div.insert_divider(),
          mica_icons::divider_icon,
        ))
        .child(toolbar_btn(
          "ed-table-quick",
          "插入表格",
          dark_mode,
          signal(false),
          move || b_tbl.insert_table(2, 2),
          mica_icons::table_icon,
        ))
        .child(toolbar_btn(
          "ed-image",
          "插入图片",
          dark_mode,
          signal(false),
          move || (h_img.on_insert_image)(),
          image_icon,
        ))
        .child(toolbar_btn(
          "ed-slash-open",
          "插入块菜单 (/)",
          dark_mode,
          slash.open,
          move || {
            if slash.open.get_untracked() {
              close_slash(slash);
            } else {
              slash.query.set(String::new());
              slash.index.set(0);
              slash.opened_by_btn.set(true);
              update_slash_anchor(&b_slash_btn, slash);
              slash.open.set(true);
            }
          },
          mica_icons::plus,
        ))
        .into_element()
    })
  };
  let fmt_bar_inline = render_format_bar.clone();
  let fmt_bar_second = render_format_bar;

  let m_root_up = marquee.clone();
  let m_root_move = marquee.clone();
  view()
    .flex_1()
    .min_w(px(0.0))
    .min_h(px(0.0))
    .h_full()
    .overflow_hidden()
    .relative()
    .bg(move || palette(dark_mode.get()).surface_base)
    .on_mouse_move(move |e: &MouseEvent| {
      let active_drag = drag_anchor.get_untracked().is_some() || m_root_move.is_active_untracked();
      if !active_drag {
        return;
      }
      if e.pressed_button != Some(MouseButton::Left) {
        // Keep `drag_anchor` intact until an authoritative `MouseUp` or
        // window-level `TouchPhase::End` arrives, preventing transient Web
        // button/toolbar crossings from aborting an active selection drag.
        autoscroll_speed.set(0.0);
        return;
      }
      // When pointer drags UP into the 44px page header/toolbar above `mica-rich-editor`,
      // keep scrolling upward continuously!
      if e.position.y.0 < 48.0 {
        autoscroll_speed.set(-28.0);
      }
    })
    .on_mouse_up(move |_: &MouseEvent| {
      autoscroll_speed.set(0.0);
      if drag_anchor.get_untracked().is_some() {
        drag_anchor.set(None);
        if has_text_selection.get_untracked() {
          bubble_recalc_rev.update(|v| *v = v.wrapping_add(1));
        }
      }
      if marquee_origin.get_untracked().is_some()
        || marquee_origin_page.get_untracked().is_some()
      {
        marquee_origin.set(None);
        marquee_origin_page.set(None);
        m_root_up.end_drag();
      }
      body_blank_marquee_ready.set(false);
      table_drag_anchor.set(None);
    })
    .on_layout(move |b: &Bounds<Points>| {
      let w = b.size.width.0;
      if w > 0.0 && (header_width.get_untracked() - w).abs() > 2.0 {
        spawn(async move {
          if (header_width.get_untracked() - w).abs() > 2.0 {
            header_width.set(w);
          }
        });
      }
    })
    .child(
      // 1. Responsive page header (single 44px row on wide viewports; drops format bar to a clean 36px 2nd row when narrower so breadcrumb + copy-path never gets squeezed)
      view()
        .w_full()
        .border_b_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .bg(move || palette(dark_mode.get()).surface_base)
        .child(
          view()
            .w_full()
            .h(px(44.0))
            .px_3()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
        .child(
          // Left breadcrumb path (left-click draggable text selection!) + copy-path button
          view()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .flex_row()
            .items_center()
            .gap_1()
            .child(
              view()
                .test_id("ed-breadcrumb-selectable")
                .selectable(true)
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_1()
                .child(when(move || !header_compact.get() && !right_open.get(), {
                  let hb = header.clone();
                  move || {
                    let hb1 = hb.clone();
                    let hb2 = hb.clone();
                    view()
                      .flex_row()
                      .items_center()
                      .gap_1()
                      .child(
                        text(move || hb1.workspace_name.get())
                          .text_size(move || px((13.5 * crate::mica_theme::ui_scale_factor()).round()))
                          .text_color(move || palette(dark_mode.get()).text_muted)
                          .truncate(),
                      )
                      .child(when(
                        {
                          let hb_chk = hb2.clone();
                          move || !hb_chk.folder_name.get().trim().is_empty()
                        },
                        {
                          let hb_f = hb2.clone();
                          move || {
                            let hb_f2 = hb_f.clone();
                            view()
                              .flex_row()
                              .items_center()
                              .gap_1()
                              .child(
                                text(">")
                                  .text_size(move || px((12.5 * crate::mica_theme::ui_scale_factor()).round()))
                                  .text_color(move || palette(dark_mode.get()).text_ghost),
                              )
                              .child(
                                text(move || hb_f2.folder_name.get())
                                  .text_size(move || px((13.5 * crate::mica_theme::ui_scale_factor()).round()))
                                  .text_color(move || palette(dark_mode.get()).text_muted)
                                  .truncate(),
                              )
                          }
                        },
                      ))
                      .child(
                        text(">")
                          .text_size(move || px((12.5 * crate::mica_theme::ui_scale_factor()).round()))
                          .text_color(move || palette(dark_mode.get()).text_ghost),
                      )
                  }
                }))
                .child({
                  let hb = header.clone();
                  text(move || {
                    let t = hb.page_title.get();
                    if t.trim().is_empty() {
                      crate::mica_theme::tr("新页面", "New page").to_string()
                    } else {
                      t
                    }
                  })
                  .text_size(move || px((13.5 * crate::mica_theme::ui_scale_factor()).round()))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).text_primary)
                  .truncate()
                }),
            )
            .child(when(move || !header_compact.get(), {
              let h_copy_id = h_copy_id.clone();
              let b_copy_path = b_copy_path.clone();
              move || {
                let h_copy_id = h_copy_id.clone();
                let b_copy_path = b_copy_path.clone();
                crate::tooltip::with_tooltip(
              button()
                .test_id("ed-copy-path")
                .accessibility_label("复制页面路径")
                .w(px(28.0))
                .h(px(28.0))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  crate::tooltip::hide_tooltip();
                  // Match Flutter `_pagePath()`: copy human-readable containing path
                  // (`Workspace/Folder/...`, or full path segments joined with `/`),
                  // NOT `mica://page/<uuid>` (which belongs to the right-side copy-link button).
                  let ws = h_copy_id.workspace_name.get_untracked();
                  let folder = h_copy_id.folder_name.get_untracked();
                  let mut segs: Vec<String> = Vec::new();
                  if !ws.trim().is_empty() {
                    segs.push(ws.trim().to_string());
                  }
                  for part in folder.split(" > ") {
                    let p = part.trim();
                    if !p.is_empty() {
                      segs.push(p.to_string());
                    }
                  }
                  let path = segs.join("/");
                  if !path.is_empty() {
                    set_clipboard(path);
                    copied_path.set(true);
                    let next_gen = copied_path_gen.get_untracked().wrapping_add(1);
                    copied_path_gen.set(next_gen);
                    spawn(async move {
                      timer(std::time::Duration::from_millis(1600)).await;
                      if copied_path_gen.get_untracked() == next_gen {
                        copied_path.set(false);
                      }
                    });
                    b_copy_path.set_status_flash("已复制页面路径");
                  }
                })
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  if copied_path.get() {
                    mica_icons::check_icon("#16A34A")
                  } else {
                    mica_icons::copy_icon(p.text_muted_hex)
                  }
                })),
              "复制路径",
                )
              }
            })),
        )
        .child(when(move || show_format_bar.get() && header_single_row.get(), {
          let fb = fmt_bar_inline.clone();
          move || fb()
        }))
        .child(
          // Right cluster: last-edited + right-sidebar toggle + whiteboard + share (cloud) + copy link + favorite + trash + more
          view()
            .flex_shrink_0()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .child(when(move || !header_compact.get(), move || {
              crate::tooltip::with_dynamic_tooltip(
                view()
                  .test_id("ed-sync-status")
                  .w(px(28.0))
                  .h(px(28.0))
                  .flex_shrink_0()
                  .items_center()
                  .justify_center()
                  .rounded(px(5.0))
                  .child(dynamic(move || {
                    let p = palette(dark_mode.get());
                    let badge = sync_badge.get();
                    if !badge.trim().is_empty() {
                      match sync_phase.get() {
                        mica_sync::SyncPhase::Syncing => mica_icons::refresh_icon(p.accent_hex),
                        mica_sync::SyncPhase::Offline => mica_icons::cloud_off(p.warning_hex),
                        mica_sync::SyncPhase::Synced => mica_icons::cloud_check(p.text_muted_hex),
                      }
                    } else {
                      mica_icons::check_icon(p.text_faint_hex)
                    }
                  })),
                move || format_sync_status_tooltip(&sync_badge.get(), &last_edited.get()),
              )
            }))
            .child(toolbar_btn(
              "toggle-right-panel-btn",
              "页面大纲与信息",
              dark_mode,
              right_open,
              move || (h_right.on_toggle_right_panel)(),
              mica_icons::sidebar_toggle_right,
            ))
            .child(when(move || !header_compact.get(), {
              let hb = h_board.clone();
              move || {
                let hb = hb.clone();
                toolbar_btn(
                  "ed-open-board",
                  "无限白板",
                  dark_mode,
                  signal(false),
                  move || (hb.on_open_board)(),
                  mica_icons::whiteboard,
                )
              }
            }))
            .child(when(move || is_cloud.get(), {
              let hs = h_share.clone();
              move || {
                let hs = hs.clone();
                crate::tooltip::with_tooltip(
                  button()
                    .test_id("ed-share-btn")
                    .accessibility_label("共享页面")
                    .h(px(26.0))
                    .px_2()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .rounded(px(5.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      crate::tooltip::hide_tooltip();
                      (hs.on_share)();
                    })
                    .child(dynamic(move || {
                      lock_icon(palette(dark_mode.get()).text_muted_hex)
                    }))
                    .child(when(move || !header_compact.get(), move || {
                      text("共享")
                        .text_size(px(12.0))
                        .text_color(move || palette(dark_mode.get()).text_primary)
                    })),
                  "共享页面",
                )
              }
            }))
            .child(when(move || !header_compact.get(), {
              let hl = h_copy_link_id.clone();
              let bl = b_copy_link.clone();
              move || {
                let hl = hl.clone();
                let bl = bl.clone();
                crate::tooltip::with_tooltip(
              button()
                .test_id("ed-copy-link")
                .accessibility_label("复制链接")
                .w(px(28.0))
                .h(px(28.0))
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  crate::tooltip::hide_tooltip();
                  let id = hl.page_id_str.get_untracked();
                  set_clipboard(format!("mica://page/{id}"));
                  copied_link.set(true);
                  bl.set_status_flash("已复制链接");
                })
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  if copied_link.get() {
                    mica_icons::check_icon(p.success_hex)
                  } else {
                    mica_icons::link_icon(p.text_muted_hex)
                  }
                })),
              "复制页面链接",
                )
              }
            }))
            .child(crate::tooltip::with_dynamic_tooltip(
              button()
                .test_id("ed-star")
                .accessibility_label("收藏或取消收藏当前页面")
                .w(px(28.0))
                .h(px(28.0))
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  crate::tooltip::hide_tooltip();
                  (h_fav.on_toggle_favorite)();
                })
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  if is_fav.get() {
                    mica_icons::star_filled(p.favorite_hex)
                  } else {
                    mica_icons::star_icon(p.text_muted_hex)
                  }
                })),
              move || if is_fav.get_untracked() { "取消收藏" } else { "收藏" },
            ))
            .child(toolbar_btn_with_ref(
              "ed-more",
              "更多操作",
              &page_more_btn_ref,
              dark_mode,
              page_menu_open,
              move || page_menu_open.set(!page_menu_open.get_untracked()),
              mica_icons::more_dots,
            )),
        ),
        )
        .child(when(move || show_format_bar.get() && !header_single_row.get(), {
          let fb = fmt_bar_second.clone();
          move || {
            let fb = fb.clone();
            dynamic(move || {
              let scrollable = format_bar_scroll.get();
              let row = view()
                .w_full()
                .h(px(36.0))
                .px_3()
                .flex_row()
                .items_center()
                .border_t_1()
                .border_color(move || palette(dark_mode.get()).border_subtle);
              if scrollable {
                row.child(
                  scroll()
                    .test_id("ed-format-bar-scroll")
                    .horizontal()
                    .scrollbar(false)
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .child(view().h_full().flex_row().items_center().child(fb())),
                )
              } else {
                row.justify_center().child(fb())
              }
            })
          }
        })),
    )
    .child(when(move || page_menu_open.get(), move || {
      page_more_menu_view(
        h_menu.clone(),
        b_copy_md.clone(),
        page_menu_open,
        header_compact,
        &page_more_btn_ref,
        dark_mode,
      )
    }))
    .child(when(move || link_prompt_open.get(), move || {
      link_editor_popover(b_link_pop.clone(), link_draft, link_prompt_open, dark_mode)
    }))
    .child(when(move || find_open.get(), move || {
      find_bar_view(b_find.clone(), find_open, find_query, find_matches, find_index, dark_mode)
    }))
    .child({
      // 2. Full-width `virtual_list` editor canvas (scrollbar sits cleanly on the
      //    RIGHT EDGE of the window!) + hidden native IME proxy sink + centered
      //    header (page title + properties) / per-block virtualized rows / footer
      //    (backlinks + bottom click-to-focus) + margin block-marquee selection +
      //    floating selection bubble menu + slash menu + word count badge.
      let m_down = marquee.clone();
      let m_move = marquee.clone();
      let m_up = marquee.clone();
      let m_touch = marquee.clone();
      let m_overlay = marquee.clone();
      let layouts_handles = layouts.clone();
      let br_handles = b_body.clone();

      let br_count = b_body.clone();
      let br_keys = b_body.clone();
      let br_down = b_body.clone();
      let br_move_clear = b_body.clone();
      let br_title_focus = b_body.clone();
      let br_footer = b_body.clone();
      let br_hdr_drag = b_body.clone();
      let br_ftr_drag = b_body.clone();
      let br_bubble = b_body.clone();
      let h_bub = h_bubble.clone();
      let m_title_clear = marquee.clone();
      let m_footer_chk = marquee.clone();
      let layouts_marquee = layouts.clone();
      let rects_down = marquee_row_rects.clone();
      let rects_move = marquee_row_rects.clone();
      let img_ctx_cm = h_bub.image_ctx.clone();
      let inline_rename_ctx = use_context::<crate::sidebar::SidebarInlineRenameSignal>();

      // Hidden native IME proxy sink closures
      let br_sink_focus = b_body.clone();
      let br_chg = b_body.clone();
      let st_chg = sink_state.clone();
      let br_comp = b_body.clone();
      let st_comp = sink_state.clone();
      let br_key_press = b_body.clone();
      let st_key_press = sink_state.clone();
      let br_paste = b_body.clone();
      let st_paste = sink_state.clone();

      let env_row = block_env.clone();
      let h_title_v = h_title_val.clone();
      let h_title_c = h_title_chg.clone();
      let b_title_e = b_title_enter.clone();
      let prop_slot_hdr = prop_slot.clone();
      let backlinks_slot_ftr = backlinks_slot.clone();
      let in_margin_press = signal(false);

      // Snapshot visible block row rects whenever a child inside the body column
      // (blank tail of a short line, empty paragraph, block gap/gutter, or bottom
      // append zone) arms `marquee_origin_page`, because `PressDown` does not bubble
      // up to `mica-rich-editor` in `nsh-core`.
      {
        let layouts_child_m = layouts_marquee.clone();
        let rects_child_m = rects_down.clone();
        let m_child_clear = marquee.clone();
        let pref_child = paper_ref;
        effect(move || {
          if marquee_origin_page.get().is_none() {
            return;
          }
          if m_child_clear.is_active_untracked() {
            m_child_clear.clear();
          }
          let (vw, _) = paper_bounds.get_untracked();
          let eff = page_width.get_untracked();
          let room = (vw - 24.0).max(0.0);
          let col = crate::page_width::rendered_column_width(eff, room);
          let col_left = ((vw - col) * 0.5).max(0.0);
          let col_right = col_left + col;
          let content_l = col_left + 20.0;
          let content_r = (col_right - 20.0).max(content_l + 1.0);
          let row_refs = layouts_child_m.borrow().snapshot_row_refs();
          let rects_slot = rects_child_m.clone();
          spawn(async move {
            let Some(pm) = pref_child.measure().await else {
              return;
            };
            let mut measured = Vec::with_capacity(row_refs.len());
            for (_bid, idx, rref) in row_refs {
              if let Some(rm) = rref.measure().await {
                let top = rm.page.y.0 - pm.page.y.0;
                let bottom = top + rm.frame.size.height.0;
                if bottom > top {
                  measured.push((idx, RectF::new(content_l, top, content_r, bottom)));
                }
              }
            }
            measured.sort_by_key(|(idx, _)| *idx);
            *rects_slot.borrow_mut() = measured;
          });
        });
      }

      view()
        .element_ref(&paper_ref)
        .test_id("mica-rich-editor")
        .flex_1()
        .w_full()
        .min_h(px(0.0))
        .overflow_hidden()
        .relative()
        .bg(move || palette(dark_mode.get()).surface_base)
        .on_layout(move |b: &Bounds<Points>| {
          paper_bounds.set((b.size.width.0, b.size.height.0));
          slash.box_size.set((b.size.width.0, b.size.height.0));
          if has_text_selection.get_untracked() {
            bubble_recalc_rev.update(|v| *v = v.wrapping_add(1));
          }
        })
        .on_context_menu(move |e: &nsh_core::ContextMenuEvent| {
          if img_ctx_cm.context_menu.get_untracked().is_some() {
            return;
          }
          body_ctx_menu.set(Some((e.page_position.x.0, e.page_position.y.0)));
        })
        .on_press_down(move |e: &PressEvent| {
          if let Some(ref inline_ren) = inline_rename_ctx {
            inline_ren.request_commit();
          }
          let lx = e.position.x.0;
          let ly = e.position.y.0;
          let (vw, vh) = paper_bounds.get_untracked();
          let eff = page_width.get_untracked();
          let room = (vw - 24.0).max(0.0);
          let col = crate::page_width::rendered_column_width(eff, room);
          let col_left = ((vw - col) * 0.5).max(0.0);
          let col_right = col_left + col;

          let margin_hit = starts_marquee_in_margin(lx, ly, vw, col_left, col_right, vh);
          in_margin_press.set(margin_hit);

          // Always snapshot visible block row rects on pointer down so BOTH margin
          // drags AND body blank-space drags can upgrade seamlessly into a live
          // marquee box once movement exceeds MARQUEE_DRAG_SLOP.
          let pref = paper_ref;
          let row_refs = layouts_marquee.borrow().snapshot_row_refs();
          let rects_slot = rects_down.clone();
          let content_l = col_left + 20.0;
          let content_r = (col_right - 20.0).max(content_l + 1.0);
          spawn(async move {
            let Some(pm) = pref.measure().await else {
              return;
            };
            let mut measured = Vec::with_capacity(row_refs.len());
            for (_bid, idx, rref) in row_refs {
              if let Some(rm) = rref.measure().await {
                let top = rm.page.y.0 - pm.page.y.0;
                let bottom = top + rm.frame.size.height.0;
                if bottom > top {
                  measured.push((idx, RectF::new(content_l, top, content_r, bottom)));
                }
              }
            }
            measured.sort_by_key(|(idx, _)| *idx);
            *rects_slot.borrow_mut() = measured;
          });

          if margin_hit {
            // Clicking outside prose immediately releases any text, block, or table selection!
            m_down.clear();
            br_down.clear_selection();
            inline_math_edit.set(None);
            table_selection.set(None);
            table_drag_anchor.set(None);
            drag_anchor.set(None);
            touch_selection_active.set(false);
            focused_code_block.set(None);
            focused_table_cell.set(None);
            sink_ref.blur();
            editor_focused.set(false);
            marquee_origin.set(Some((lx, ly)));
          } else {
            if m_down.is_active_untracked() {
              m_down.clear();
            }
            marquee_origin.set(None);
          }
        })
        // Passive window-tracked pointer observer (never claims touches so
        // native scrolling and clicks stay untouched):
        // 1. Detects touch/pen vs mouse input for mobile long-press & double-tap
        //    word selection (`CAP-16-06`).
        // 2. Keeps Web mouse drag auto-scrolling and release cleanup alive even
        //    when the cursor leaves the browser viewport (`nsh.js` listens to
        //    `pointermove`/`pointerup` on `window` for `onTouch`!).
        .on_touch(move |e: &TouchEvent| {
          let Some(pt) = e.points.iter().find(|p| p.changed).or_else(|| e.points.first()) else {
            return;
          };
          let is_touch_ptr = matches!(pt.pointer_type, PointerType::Touch | PointerType::Pen);
          match e.phase {
            TouchPhase::Start => {
              last_pointer_is_touch.set(is_touch_ptr);
              let pref = paper_ref;
              spawn(async move {
                if let Some(pm) = pref.measure().await {
                  paper_page_origin.set((pm.page.x.0, pm.page.y.0));
                }
              });
            }
            TouchPhase::Move if !is_touch_ptr => {
              if drag_anchor.get_untracked().is_some() {
                let (ox, oy) = paper_page_origin.get_untracked();
                let (_, vh) = paper_bounds.get_untracked();
                let cy = pt.page.y.0 - oy;
                autoscroll_last_xy.set((pt.page.x.0 - ox, cy));
                autoscroll_speed.set(crate::editor_positioning::edge_autoscroll_speed(cy, vh));
              }
            }
            TouchPhase::End | TouchPhase::Cancel if !is_touch_ptr => {
              autoscroll_speed.set(0.0);
              if drag_anchor.get_untracked().is_some() {
                drag_anchor.set(None);
                if has_text_selection.get_untracked() {
                  bubble_recalc_rev.update(|v| *v = v.wrapping_add(1));
                }
              }
              if marquee_origin.get_untracked().is_some()
                || marquee_origin_page.get_untracked().is_some()
              {
                marquee_origin.set(None);
                marquee_origin_page.set(None);
                m_touch.end_drag();
              }
              in_margin_press.set(false);
              body_blank_marquee_ready.set(false);
              table_drag_anchor.set(None);
            }
            _ => {}
          }
        })
        .on_mouse_move(move |e: &MouseEvent| {
          if e.pressed_button != Some(MouseButton::Left) {
            return;
          }
          // Skip prose/marquee drag while user is dragging across table cells.
          if table_drag_anchor.get_untracked().is_some() {
            return;
          }
          // 1. Live marquee rectangle drag (works BOTH from outside margins AND from
          //    blank whitespace inside the body column such as empty lines, right of
          //    short lines, block gaps/padding, bullet gutters, or below the last block!)
          let allow_marquee =
            in_margin_press.get_untracked() || body_blank_marquee_ready.get_untracked();
          let origin_local = marquee_origin.get_untracked().or_else(|| {
            marquee_origin_page.get_untracked().map(|(px0, py0)| {
              (
                e.position.x.0 - (e.page_position.x.0 - px0),
                e.position.y.0 - (e.page_position.y.0 - py0),
              )
            })
          });
          if allow_marquee
            && let Some((ox, oy)) = origin_local
          {
            let cx = e.position.x.0;
            let cy = e.position.y.0;
            autoscroll_last_xy.set((cx, cy));
            if (cx - ox).abs() >= MARQUEE_DRAG_SLOP || (cy - oy).abs() >= MARQUEE_DRAG_SLOP {
              if drag_anchor.get_untracked().is_some() {
                drag_anchor.set(None);
                br_move_clear.clear_selection();
              }
              let rect = RectF::from_points(ox, oy, cx, cy);
              let rects = rects_move.borrow();
              m_move.update_drag(rect, &rects);
              let (_, vh) = paper_bounds.get_untracked();
              autoscroll_speed.set(crate::editor_positioning::edge_autoscroll_speed(cy, vh));
            }
            return;
          }
          // 2. Smooth 60fps edge auto-scroll velocity update during cross-block drag
          if drag_anchor.get_untracked().is_some() {
            let (_, vh) = paper_bounds.get_untracked();
            let cy = e.position.y.0;
            autoscroll_last_xy.set((e.position.x.0, cy));
            autoscroll_speed.set(crate::editor_positioning::edge_autoscroll_speed(cy, vh));
          } else if autoscroll_speed.get_untracked() != 0.0 {
            autoscroll_speed.set(0.0);
          }
        })
        .on_mouse_leave({
          let m_leave = marquee.clone();
          move |e: &MouseEvent| {
            let active_drag =
              drag_anchor.get_untracked().is_some() || m_leave.is_active_untracked();
            if e.pressed_button == Some(MouseButton::Left) && active_drag {
              let (_, vh) = paper_bounds.get_untracked();
              let speed = if e.position.y.0 <= vh * 0.5 { -24.0 } else { 24.0 };
              autoscroll_speed.set(speed);
            } else {
              autoscroll_speed.set(0.0);
            }
          }
        })
        .on_mouse_up(move |_: &MouseEvent| {
          autoscroll_speed.set(0.0);
          if drag_anchor.get_untracked().is_some() {
            drag_anchor.set(None);
            if has_text_selection.get_untracked() {
              bubble_recalc_rev.update(|v| *v = v.wrapping_add(1));
            }
          }
          if marquee_origin.get_untracked().is_some()
            || marquee_origin_page.get_untracked().is_some()
          {
            marquee_origin.set(None);
            marquee_origin_page.set(None);
            m_up.end_drag();
          }
          in_margin_press.set(false);
          body_blank_marquee_ready.set(false);
          table_drag_anchor.set(None);
        })
        .child(
          // Single hidden native IME proxy `input()` positioned beside the active
          // caret (`ime_sink_pos`) so Windows/macOS CJK candidate windows open right
          // under the user's typing position rather than at `(0, 0)` (CAP-01-05).
          input()
            .element_ref(&sink_ref)
            .test_id("mica-editor-ime-sink")
            .label("文档正文输入代理")
            .absolute()
            .left(move || px(ime_sink_pos.get().0))
            .top(move || px(ime_sink_pos.get().1))
            .w(px(240.0))
            .h(px(22.0))
            .pointer_events(PointerEvents::None)
            .p_0()
            .border(px(0.0))
            .bg(TRANSPARENT_RGBA)
            .text_color(TRANSPARENT_RGBA)
            .caret(false)
            .multiline(false)
            .submit_behavior(SubmitBehavior::Submit)
            .scrollbar(false)
            .auto_correct(false)
            .auto_capitalize(AutoCapitalize::None)
            .paste_behavior(PasteBehavior::Report)
            .value(move || sink_value.get())
            .on_focus(move || {
              editor_focused.set(true);
              if br_sink_focus.selection().is_none() && br_sink_focus.block_len() > 0 {
                crate::editor_keys::ensure_active_caret(&br_sink_focus);
              }
            })
            .on_blur(move || {
              editor_focused.set(false);
            })
            .on_change(move |text: &str| {
              if copied_link.get_untracked() {
                copied_link.set(false);
              }
              if st_chg.borrow().composing {
                st_chg.borrow_mut().held = Some(text.to_owned());
                sink_value.set(text.to_owned());
                return;
              }
              apply_sink_text_diff(
                text,
                &st_chg,
                sink_value,
                &br_chg,
                slash,
                ai_enabled,
              );
            })
            .on_composition_change(move |c: &Composition| {
              let held = {
                let mut st = st_comp.borrow_mut();
                st.composing = c.composing;
                if c.composing { None } else { st.held.take() }
              };
              if c.composing && !c.text.is_empty() {
                if let Some(sel) = br_comp.selection() {
                  let fp = sel.focus_pos();
                  br_comp.set_ime_preedit(Some((fp.node, fp.offset, c.text.clone())));
                }
              } else {
                br_comp.set_ime_preedit(None);
              }
              if let Some(committed) = held {
                apply_sink_text_diff(
                  &committed,
                  &st_comp,
                  sink_value,
                  &br_comp,
                  slash,
                  ai_enabled,
                );
              }
            })
            .on_key_press(move |k: &str| {
              if !st_key_press.borrow().composing && st_key_press.borrow().text.is_empty() {
                if k == "Backspace" {
                  if try_backspace_to_title(
                    &br_key_press,
                    show_page_title,
                    page_title_key,
                    title_ref,
                    sink_ref,
                    editor_focused,
                  ) {
                    return;
                  }
                  br_key_press.dispatch(EditorCommand::DeleteBackward);
                  sync_slash_palette_from_caret(&br_key_press, slash, ai_enabled);
                } else if k == "Delete" {
                  br_key_press.dispatch(EditorCommand::DeleteForward);
                  sync_slash_palette_from_caret(&br_key_press, slash, ai_enabled);
                }
              }
            })
            .on_paste(move |p: &PasteEvent| {
              if copied_link.get_untracked() {
                copied_link.set(false);
              }
              focused_code_block.set(None);
              focused_table_cell.set(None);
              table_selection.set(None);
              empty_ime_sink(&st_paste, sink_value);
              handle_clipboard_paste(p, &br_paste, &image_ctx_paste, slash, ai_enabled);
              br_paste.request_focus();
            }),
        )
        .child(
          // Full-width virtualized block list: `count` & `keys` subscribe ONLY to
          // `structure_rev`, NEVER to body typing signals!
          // Platform tiny 3px indicator is disabled in favor of NSH's proportional
          // adaptive `nsh_base::scrollbar()` mounted on the right window edge.
          virtual_list(
            move || {
              structure_rev.get();
              br_count.block_len()
            },
            move |at| render_virtual_block_row(env_row.clone(), at),
          )
          .keys(move || {
            structure_rev.get();
            br_keys.block_ids()
          })
          .element_ref(&list_ref)
          .estimated_item_size(px(36.0))
          .scrollbar(false)
          .on_scroll(move |e: &ScrollEvent| {
            scroll_offset_y.set(e.offset.y.0);
            if e.content_size.height.0 > 0.0 {
              scroll_content_h.set(e.content_size.height.0);
            }
            if e.size.height.0 > 0.0 {
              scroll_viewport_h.set(e.size.height.0);
            }
            if has_text_selection.get_untracked() {
              bubble_recalc_rev.update(|v| *v = v.wrapping_add(1));
            }
          })
          .flex_1()
          .w_full()
          .min_h(px(0.0))
          .h_full()
          .header(
            // Centered scrolling document header: snug 30px SemiBold page title + PropertyPanel
            view()
              .w_full()
              .items_center()
              .on_mouse_move(move |e: &MouseEvent| {
                if e.pressed_button == Some(MouseButton::Left) {
                  if let Some((ref t_bid, _, _)) = table_drag_anchor.get_untracked()
                    && let Some(t_idx) = br_hdr_drag.index_of_block_id(t_bid)
                  {
                    let t_end = br_hdr_drag
                      .block_at(t_idx)
                      .map(|b| utf16_len(&b.text))
                      .unwrap_or(0);
                    table_drag_anchor.set(None);
                    table_selection.set(None);
                    focused_table_cell.set(None);
                    drag_anchor.set(Some(DocPosition::new(t_idx, t_end)));
                  }
                  if let Some(anchor) = drag_anchor.get_untracked() {
                    let next = DocSelection::from_positions(anchor, DocPosition::new(0, 0));
                    if br_hdr_drag.selection().as_ref() != Some(&next) {
                      br_hdr_drag.set_selection(next);
                    }
                  }
                }
              })
              .child(
                view()
                  .w_full()
                  .max_w(move || {
                    let eff = page_width.get();
                    if crate::page_width::is_full_width(eff) {
                      px(100_000.0)
                    } else {
                      px(eff)
                    }
                  })
                  .px_6()
                  .pt_6()
                  .pb_1()
                  .child(when(move || show_page_title.get(), move || {
                    let h_val = h_title_v.clone();
                    let h_chg = h_title_c.clone();
                    let h_foc = h_title_v.on_title_focus_change.clone();
                    let h_blur = h_title_v.on_title_focus_change.clone();
                    let h_sub = h_title_v.on_title_focus_change.clone();
                    let b_enter = b_title_e.clone();
                    let b_tf = br_title_focus.clone();
                    let m_tc = m_title_clear.clone();
                    input()
                      .element_ref(&title_ref)
                      .test_id("page-title-input")
                      .placeholder("新页面")
                      .submit_behavior(SubmitBehavior::Submit)
                      .w_full()
                      .pb_1()
                      .text_size(px(30.0))
                      .font_weight(FontWeight::SEMIBOLD)
                      .text_color(move || palette(dark_mode.get()).text_primary)
                      .bg(move || palette(dark_mode.get()).surface_base)
                      .value(move || page_title_input_value(&h_val.page_title.get()))
                      .on_focus(move || {
                        if let Some(cb) = &h_foc {
                          cb(true);
                        }
                        b_tf.clear_selection();
                        m_tc.clear();
                        touch_selection_active.set(false);
                        table_selection.set(None);
                        focused_code_block.set(None);
                        focused_table_cell.set(None);
                        editor_focused.set(false);
                      })
                      .on_blur(move || {
                        if let Some(cb) = &h_blur {
                          cb(false);
                        }
                      })
                      .on_change(move |v: &str| (h_chg.on_rename_title)(v.to_owned()))
                      .on_submit(move |_: &str| {
                        if let Some(cb) = &h_sub {
                          cb(false);
                        }
                        suppress_next_enter.set(true);
                        b_enter.prepend_empty_paragraph_and_focus();
                        spawn(async move {
                          timer(Duration::from_millis(0)).await;
                          suppress_next_enter.set(false);
                        });
                      })
                  }))
                  .child(when(move || show_properties.get(), {
                    let slot = prop_slot_hdr.clone();
                    move || match &slot {
                      Some(render_fn) => render_fn(),
                      None => view().into_element(),
                    }
                  })),
              ),
          )
          .footer(
            // Centered scrolling footer: BacklinksPanel + bottom click-to-append-line breathing room
            view()
              .w_full()
              .items_center()
              .on_mouse_move(move |e: &MouseEvent| {
                if e.pressed_button == Some(MouseButton::Left) {
                  if let Some((ref t_bid, _, _)) = table_drag_anchor.get_untracked()
                    && let Some(t_idx) = br_ftr_drag.index_of_block_id(t_bid)
                  {
                    table_drag_anchor.set(None);
                    table_selection.set(None);
                    focused_table_cell.set(None);
                    drag_anchor.set(Some(DocPosition::new(t_idx, 0)));
                  }
                  if let Some(anchor) = drag_anchor.get_untracked() {
                    let last = br_ftr_drag.block_len().saturating_sub(1);
                    let end_off = br_ftr_drag
                      .block_at(last)
                      .map(|b| utf16_len(&b.text))
                      .unwrap_or(0);
                    let next = DocSelection::from_positions(anchor, DocPosition::new(last, end_off));
                    if br_ftr_drag.selection().as_ref() != Some(&next) {
                      br_ftr_drag.set_selection(next);
                    }
                  }
                }
              })
              .child(
                view()
                  .w_full()
                  .max_w(move || {
                    let eff = page_width.get();
                    if crate::page_width::is_full_width(eff) {
                      px(100_000.0)
                    } else {
                      px(eff)
                    }
                  })
                  .px_6()
                  .pt_4()
                  .pb_6()
                  .child(match backlinks_slot_ftr {
                    Some(render_fn) => render_fn(),
                    None => view().into_element(),
                  })
                  .child(
                    view()
                      .test_id("editor-bottom-append-zone")
                      .w_full()
                      .h(px(180.0))
                      .cursor(Cursor::Text)
                      .on_press_down(move |e: &PressEvent| {
                        body_blank_marquee_ready.set(true);
                        marquee_origin_page.set(Some((e.page_position.x.0, e.page_position.y.0)));
                      })
                      .on_press({
                        let br_click = br_footer.clone();
                        let m_chk = m_footer_chk.clone();
                        move || {
                          if !m_chk.is_active_untracked() {
                            focused_code_block.set(None);
                            focused_table_cell.set(None);
                            table_selection.set(None);
                            br_click.append_empty_paragraph_and_focus();
                          }
                        }
                      }),
                  ),
              ),
          ),
        )
        // Adaptive proportional vertical scrollbar anchored to the right window edge
        .child(
          scrollbar()
            .test_id("editor-vertical-scrollbar")
            .scroll(list_ref)
            .mode(ScrollbarMode::Always)
            .min_thumb_length(px(56.0))
            .thumb_style(move |s| {
              let dark = dark_mode.get();
              let color = if dark {
                Rgba { r: 1.0, g: 1.0, b: 1.0, a: 0.28 }
              } else {
                Rgba { r: 0.09, g: 0.11, b: 0.15, a: 0.28 }
              };
              s.bg(color).rounded(px(4.0))
            })
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(14.0))
            .px(px(3.0))
            .py(px(4.0))
            .z_index(16),
        )
        .child(touch_selection_handles_layer(TouchHandlesProps {
          bridge: br_handles,
          layouts: layouts_handles,
          paper_ref,
          paper_bounds,
          scroll_offset_y,
          autoscroll_speed,
          bubble_recalc_rev,
          has_text_selection,
          drag_anchor,
          touch_selection_active,
          touch_handle_dragging,
          touch_handle_anchors,
          dark_mode,
        }))
        .child(when(
          move || {
            has_text_selection.get()
              && drag_anchor.get().is_none()
              && !touch_handle_dragging.get()
          },
          {
          let br_b = br_bubble.clone();
          let h_b = h_bub.clone();
          move || {
            let br_c = br_b.clone();
            let h_c = h_b.clone();
            let narrow_sel = {
              let pw = paper_bounds.get().0;
              (pw > 0.0 && pw < 640.0) || touch_selection_active.get()
            };
            view()
              .absolute()
              .left(move || px(bubble_anchor.get().0))
              .top(move || px(bubble_anchor.get().1))
              .z_index(34)
              .child(if narrow_sel {
                crate::touch_selection::touch_selection_bar_view(br_c, dark_mode).into_element()
              } else {
                bubble_menu_view(
                  br_c,
                  h_c,
                  has_text_selection,
                  link_prompt_open,
                  link_draft,
                  dark_mode,
                )
                .into_element()
              })
          }
        }))
        .child(crate::editor_positioning::inline_math_popover_view(
          inline_math_edit,
          ime_sink_pos,
          br_bubble.clone(),
          dark_mode,
        ))
        .child(when(move || slash.open.get(), move || {
          caret_slash_menu_view(
            b_slash_menu.clone(),
            h_slash.clone(),
            slash,
            dark_mode,
          )
        }))
        .child(crate::block_marquee::marquee_overlay_layer(
          m_overlay,
          dark_mode,
        ))
        .child(when(
          move || !word_count.get().starts_with("0 词 · 0 字"),
          move || {
            // 3. Floating bottom-right word count badge (leaves clear room for window-edge scrollbar)
            view()
              .test_id("editor-word-count")
              .absolute()
              .right(px(18.0))
              .bottom(px(10.0))
              .z_index(12)
              .px_2()
              .py_1()
              .rounded(px(6.0))
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_subtle)
              .bg(move || palette(dark_mode.get()).surface_raised)
              .child(
                text(move || word_count.get())
                  .text_size(px(12.0))
                  .text_color(move || palette(dark_mode.get()).text_faint),
              )
          },
        ))
    })
    .child(render_lightbox_modal(lightbox, b_lightbox.clone(), dark_mode))
    .child(editor_context_menu_overlay(
      body_ctx_menu,
      b_lightbox.clone(),
      header.clone(),
      has_text_selection,
      is_cloud,
      paper_bounds,
      dark_mode,
    ))
    .child(crate::image_blocks::render_image_context_menu_overlay(
      image_ctx_menu,
      b_lightbox.clone(),
      lightbox,
      dark_mode,
    ))
    .child(crate::image_blocks::render_image_caption_modal(
      image_ctx_cap,
      b_lightbox,
      dark_mode,
    ))
}

/// Format the dynamic hover tooltip label for the fixed-width top-right sync/save status icon.
pub(crate) fn format_sync_status_tooltip(sync_badge: &str, last_edited: &str) -> String {
  let badge = sync_badge.trim();
  let edited = last_edited.trim();
  if !badge.is_empty() {
    if edited.is_empty() {
      badge.to_string()
    } else {
      format!("{badge} · {edited}")
    }
  } else if edited.is_empty() {
    "本地已保存".to_string()
  } else {
    format!("本地已保存 · {edited}")
  }
}


/// Return `true` when `name` is an untitled sentinel (`""`, `"新页面"`,
/// `"未命名页面"`, `"无标题"`, `"Untitled"`) that should render as the muted
/// placeholder rather than pre-filled solid text in the page title input.
pub(crate) fn is_default_untitled_title(name: &str) -> bool {
  let trimmed = name.trim();
  trimmed.is_empty()
    || trimmed == "新页面"
    || trimmed == "未命名页面"
    || trimmed == "无标题"
    || trimmed == "Untitled"
}

/// Value bound to the page title `<input>`: untitled sentinels map to `""` so
/// the muted `"新页面"` placeholder is shown and typing replaces it directly.
pub(crate) fn page_title_input_value(name: &str) -> String {
  if is_default_untitled_title(name) {
    String::new()
  } else {
    name.to_string()
  }
}

use crate::editor_keys::{
  EditorKeyAction, apply_sink_text_diff, close_slash, empty_ime_sink, handle_clipboard_paste,
  handle_editor_key, sync_slash_palette_from_caret, toggle_block_type, try_backspace_to_title,
  update_slash_anchor, write_rich_clipboard,
};
use crate::editor_overlays::{
  bubble_menu_view, caret_slash_menu_view, editor_context_menu_overlay, find_bar_view, image_icon,
  link_editor_popover, lock_icon, page_more_menu_view, step_find_match, terminal_icon,
  toolbar_btn, toolbar_btn_active, toolbar_btn_dim, toolbar_btn_with_ref, toolbar_sep,
};

#[cfg(test)]
mod tests {
  use crate::editor_keys::should_backspace_to_title;
  use mica_editor::{DocSelection, EditorBlock};

  #[test]
  fn backspace_to_title_matches_first_paragraph_at_offset_zero_and_skips_styled_blocks() {
    let sel0 = DocSelection::caret(0, 0);
    let sel1 = DocSelection::caret(0, 1);
    let empty_para = EditorBlock::new("b0", "paragraph", "");
    let non_empty_para = EditorBlock::new("b0", "paragraph", "Spectrum-X");
    let heading = EditorBlock::new("b0", "heading", "");

    // Plain paragraph at (0, 0) steps to title end (and `try_backspace_to_title`
    // drops an empty line 0 first when a following block exists).
    assert!(should_backspace_to_title(true, Some(&sel0), Some(&empty_para)));
    assert!(should_backspace_to_title(true, Some(&sel0), Some(&non_empty_para)));

    // Caret after offset 0 or hidden page title never jumps to title.
    assert!(!should_backspace_to_title(true, Some(&sel1), Some(&non_empty_para)));
    assert!(!should_backspace_to_title(false, Some(&sel0), Some(&empty_para)));

    // Styled block (heading/list) sheds style first instead of jumping to title.
    assert!(!should_backspace_to_title(true, Some(&sel0), Some(&heading)));
  }

  #[test]
  fn untitled_page_titles_render_empty_input_for_gray_placeholder() {
    use super::{is_default_untitled_title, page_title_input_value};

    for sentinel in ["", "   ", "新页面", "未命名页面", "无标题", "Untitled"] {
      assert!(is_default_untitled_title(sentinel), "expected sentinel for {sentinel:?}");
      assert_eq!(page_title_input_value(sentinel), "");
    }
    assert!(!is_default_untitled_title("纯Rust架构设计"));
    assert_eq!(page_title_input_value("纯Rust架构设计 "), "纯Rust架构设计 ");
  }

  #[test]
  fn sync_status_tooltip_formats_cloud_and_local_states() {
    use super::format_sync_status_tooltip;

    assert_eq!(
      format_sync_status_tooltip("已实时同步", "上次编辑 刚刚"),
      "已实时同步 · 上次编辑 刚刚"
    );
    assert_eq!(
      format_sync_status_tooltip("正在同步… · 2 人在线", "上次编辑 刚刚"),
      "正在同步… · 2 人在线 · 上次编辑 刚刚"
    );
    assert_eq!(format_sync_status_tooltip("离线缓存", ""), "离线缓存");
    assert_eq!(
      format_sync_status_tooltip("", "上次编辑 刚刚"),
      "本地已保存 · 上次编辑 刚刚"
    );
    assert_eq!(format_sync_status_tooltip("", "  "), "本地已保存");
  }
}
