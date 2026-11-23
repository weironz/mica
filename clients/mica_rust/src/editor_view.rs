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

use std::{cell::RefCell, rc::Rc, time::Duration};

use mica_editor::{
  DocPosition, DocSelection, EditorCommand, FindMatch, TableNavDirection, TableOp,
  utf16::{utf16_len, utf16_next_boundary, utf16_prev_boundary, utf16_slice},
};
use nsh_base::{ScrollbarMode, button, input, scrollbar};
use nsh_core::{
  SubmitBehavior,
  color::Rgba,
  element::{
    ClipboardItem, ElementRef, ImageSrc, KeyOptions, Spawned, Svg, clipboard_string, element_ref,
    on_key_with, set_clipboard, set_clipboard_item, set_interval, svg, virtual_list,
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
    move_caret_vertical_preserving_x, schedule_bubble_anchor_update, schedule_ime_sink_pos_update,
    schedule_slash_anchor_update,
  },
  editor_slash::{SlashItem, extract_slash_query, filter_slash_items},
  mica_icons,
  mica_theme::palette,
  text_geometry::VisibleLayoutCache,
};

pub use crate::editor_bridge::OutlineItem;

const SLASH_MENU_WIDTH: f32 = 248.0;
const SLASH_MENU_MAX_H: f32 = 288.0;
const SINK_LIMIT: usize = 64;
const TRANSPARENT_RGBA: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// Hidden native IME sink buffer tracker (mirrors NSH's battle-tested sink state).
#[derive(Default)]
struct ImeSinkState {
  text: String,
  emptied: Option<String>,
  composing: bool,
  held: Option<String>,
}

pub struct EditorHeaderProps {
  pub workspace_name: Signal<String>,
  pub folder_name: Signal<String>,
  pub page_title: Signal<String>,
  pub page_id_str: Signal<String>,
  pub is_favorite: Signal<bool>,
  pub right_panel_open: Signal<bool>,
  pub on_rename_title: Rc<dyn Fn(String)>,
  pub on_open_board: Rc<dyn Fn()>,
  pub on_toggle_favorite: Rc<dyn Fn()>,
  pub on_toggle_right_panel: Rc<dyn Fn()>,
  pub on_trash_page: Rc<dyn Fn()>,
  pub on_export_markdown: Rc<dyn Fn()>,
  pub on_export_html: Rc<dyn Fn()>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  // ── 1:1 Parity Props ─────────────────────────────────────────────────────
  pub is_cloud_signed_in: Signal<bool>,
  pub show_format_bar: Signal<bool>,
  pub show_page_title: Signal<bool>,
  pub show_properties: Signal<bool>,
  pub comments_visible: Signal<bool>,
  pub page_width: Signal<f32>,
  pub ai_enabled: Signal<bool>,
  pub on_commit_page_width: Rc<dyn Fn(f32)>,
  pub on_share: Rc<dyn Fn()>,
  pub on_toggle_comments: Rc<dyn Fn()>,
  pub on_toggle_properties: Rc<dyn Fn()>,
  pub on_export_zip: Rc<dyn Fn()>,
  pub on_export_pdf: Rc<dyn Fn()>,
  pub on_import_markdown_picker: Rc<dyn Fn()>,
  pub on_open_version_history: Rc<dyn Fn()>,
  pub on_inline_ai: Rc<dyn Fn(String)>,
  pub on_insert_image: Rc<dyn Fn()>,
  pub property_panel_slot: Option<Rc<dyn Fn() -> AnyElement>>,
  pub backlinks_panel_slot: Option<Rc<dyn Fn() -> AnyElement>>,
}

#[derive(Clone, Copy)]
struct SlashMenuSignals {
  open: Signal<bool>,
  query: Signal<String>,
  index: Signal<usize>,
  anchor: Signal<(f32, f32)>,
  opened_by_btn: Signal<bool>,
  box_size: Signal<(f32, f32)>,
}

pub fn editor_view(
  bridge: Rc<MicaDocumentBridge>,
  header: Rc<EditorHeaderProps>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  // Allow embedded `board` views to open the full whiteboard.
  provide_context(OpenBoardCallback(header.on_open_board.clone()));

  let page_menu_open = signal(false);
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
  let code_caret_at_start = signal(false);
  let focused_table_cell: Signal<Option<(String, usize, usize)>> = signal(None);
  let table_selection: Signal<Option<TableCellSelection>> = signal(None);
  let table_drag_anchor: Signal<Option<(String, usize, usize)>> = signal(None);
  let body_blank_marquee_ready = signal(false);
  let last_click: Signal<(usize, u32, f64, u8)> = signal((0, 0, 0.0, 0));

  let list_ref = element_ref();
  let sink_ref = element_ref();
  let paper_ref = element_ref();
  let title_ref = element_ref();
  let editor_focused = signal(false);
  let caret_blink_on = signal(true);
  let blink_task: Rc<RefCell<Option<Spawned<()>>>> = Rc::new(RefCell::new(None));
  let sink_value = signal(String::new());
  let sink_clear_rev = signal(0u64);
  let sink_state = Rc::new(RefCell::new(ImeSinkState::default()));
  let marquee_row_rects: Rc<RefCell<Vec<(usize, RectF)>>> = Rc::new(RefCell::new(Vec::new()));
  let autoscroll_cooldown = signal(false);
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
            s,
            bubble_anchor,
          );
        } else {
          // Keep hidden native IME proxy pinned beside the active caret so OS
          // CJK candidate windows open right next to where the user is typing.
          schedule_ime_sink_pos_update(&b_bub, &l_bub, pref_bub, s.focus_pos(), ime_sink_pos);
        }
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
        code_caret_at_start.set(false);
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

  // Code block Backspace-at-offset-0 handler: when the caret sits at line 0, col 0
  // of a focused code block, pressing Backspace removes the code block cleanly
  // (undoable via Ctrl+Z) and places focus back on the neighboring block.
  {
    let b_code_del = bridge.clone();
    effect(move || {
      let Some(code_bid) = focused_code_block.get() else {
        return;
      };
      if !code_caret_at_start.get() {
        return;
      }
      let b_del = b_code_del.clone();
      on_key_with(
        "backspace",
        KeyOptions {
          when: When::Always,
          ..KeyOptions::default()
        },
        move |_| {
          if let Some(idx) = b_del.index_of_block_id(&code_bid) {
            focused_code_block.set(None);
            code_caret_at_start.set(false);
            b_del.delete_block_and_focus_neighbor(idx);
          }
        },
      );
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

  // Scroll `virtual_list` according to ScrollMode:
  // - `Jump` (page switch, outline jump, find match): align target cleanly into view.
  // - `KeepVisible` (Enter, Backspace, typing, arrows): measure target row against
  //   the viewport and scroll ONLY when it actually leaves the visible bounds, so
  //   normal editing NEVER jumps or scrolls content upward!
  {
    let scroll_req = bridge.scroll_request_signal();
    let b_scroll = bridge.clone();
    let l_scroll = layouts.clone();
    let lref = list_ref;
    let pref = paper_ref;
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
            let row_ref_opt = b_scroll
              .block_id_at(idx)
              .and_then(|bid| l_scroll.borrow().row_refs.get(&bid).map(|(_, r)| *r));
            let Some(rref) = row_ref_opt else {
              // Target row is outside the mounted virtual window -> bring it in.
              if idx == 0 {
                lref.scroll_to_offset(px(0.0), false);
              } else {
                lref.scroll_to_index(idx as u32, Percentage(0.5), px(0.0), false);
              }
              return;
            };
            spawn(async move {
              let (Some(pm), Some(rm)) = (pref.measure().await, rref.measure().await) else {
                return;
              };
              let vh = pm.frame.size.height.0;
              if vh <= 1.0 {
                return;
              }
              let rel_top = rm.page.y.0 - pm.page.y.0;
              let rel_bottom = rel_top + rm.frame.size.height.0;
              if rel_top < 12.0 {
                if idx == 0 {
                  lref.scroll_to_offset(px(0.0), false);
                } else {
                  lref.scroll_to_index(idx as u32, Percentage(0.1), px(0.0), false);
                }
              } else if rel_bottom > vh - 16.0 {
                lref.scroll_to_index(idx as u32, Percentage(0.85), px(0.0), false);
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
  {
    let b_keys = bridge.clone();
    let l_keys = layouts.clone();
    let st_keys = sink_state.clone();
    let on_ai_k = on_inline_ai_key.clone();
    let on_img_k = on_insert_image_key.clone();
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
        ("mod+k", false, EditorKeyAction::OpenLink),
        ("escape", false, EditorKeyAction::Escape),
      ];
      for &(stroke, repeat, action) in bindings {
        let br = b_keys.clone();
        let lay = l_keys.clone();
        let st = st_keys.clone();
        let ai_cb = on_ai_k.clone();
        let img_cb = on_img_k.clone();
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
              show_page_title_key,
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
  let word_count = bridge.word_count_signal();
  let structure_rev = bridge.structure_rev_signal();

  let is_fav = header.is_favorite;
  let right_open = header.right_panel_open;
  let show_format_bar = header.show_format_bar;
  let show_page_title = header.show_page_title;
  let show_properties = header.show_properties;
  let is_cloud = header.is_cloud_signed_in;
  let page_width = header.page_width;

  let inline_math_edit: Signal<Option<crate::editor_positioning::InlineMathEdit>> = signal(None);
  let block_env = BlockRenderEnv {
    bridge: bridge.clone(),
    dark_mode,
    layouts: layouts.clone(),
    drag_anchor,
    lightbox,
    focused_code_block,
    code_caret_at_start,
    focused_table_cell,
    table_selection,
    table_drag_anchor,
    body_blank_marquee_ready,
    last_click,
    page_width,
    editor_focused,
    caret_blink_on,
    sink_clear_rev,
    marquee_selected: marquee.selected_range,
    inline_math_edit,
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
  let h_trash = header.clone();
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
  let header_single_row = computed(move || header_width.get() >= 1180.0);

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
    let b_code = b_code.clone();
    let b_link_btn = b_link_btn.clone();
    let b_list = b_list.clone();
    let b_ordered = b_ordered.clone();
    let b_todo = b_todo.clone();
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
      let b_code = b_code.clone();
      let b_link_btn = b_link_btn.clone();
      let b_list = b_list.clone();
      let b_ordered = b_ordered.clone();
      let b_todo = b_todo.clone();
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
          "删除线",
          dark_mode,
          computed(move || active_marks.get().iter().any(|m| m == "strike")),
          move || b_strike.toggle_mark("strike"),
          mica_icons::strikethrough_icon,
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
          move || b_code_blk.insert_custom_block("code_block", "", Some("rust")),
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

  view()
    .flex_1()
    .min_w(px(0.0))
    .min_h(px(0.0))
    .h_full()
    .overflow_hidden()
    .relative()
    .bg(move || palette(dark_mode.get()).surface_base)
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
                .child(when(move || !right_open.get(), {
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
                      crate::mica_theme::tr("无标题", "Untitled").to_string()
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
            .child(
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
            ),
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
            .child(when(move || !right_open.get(), move || {
              text(move || last_edited.get())
                .text_size(move || px((13.0 * crate::mica_theme::ui_scale_factor()).round()))
                .text_color(move || palette(dark_mode.get()).text_faint)
            }))
            .child(toolbar_btn(
              "toggle-right-panel-btn",
              "切换右侧大纲与页面信息边栏",
              dark_mode,
              right_open,
              move || (h_right.on_toggle_right_panel)(),
              mica_icons::sidebar_toggle_right,
            ))
            .child(toolbar_btn(
              "ed-open-board",
              "打开无限白板",
              dark_mode,
              signal(false),
              move || (h_board.on_open_board)(),
              mica_icons::whiteboard,
            ))
            .child(when(move || is_cloud.get(), {
              let hs = h_share.clone();
              move || {
                let hs = hs.clone();
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
                  .on_press(move || (hs.on_share)())
                  .child(dynamic(move || {
                    lock_icon(palette(dark_mode.get()).text_muted_hex)
                  }))
                  .child(
                    text("共享")
                      .text_size(px(12.0))
                      .text_color(move || palette(dark_mode.get()).text_primary),
                  )
              }
            }))
            .child(
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
                  let id = h_copy_link_id.page_id_str.get_untracked();
                  set_clipboard(format!("mica://page/{id}"));
                  copied_link.set(true);
                  b_copy_link.set_status_flash("已复制链接");
                })
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  if copied_link.get() {
                    mica_icons::check_icon(p.success_hex)
                  } else {
                    mica_icons::link_icon(p.text_muted_hex)
                  }
                })),
            )
            .child(
              button()
                .test_id("ed-star")
                .accessibility_label("收藏或取消收藏当前页面")
                .w(px(28.0))
                .h(px(28.0))
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || (h_fav.on_toggle_favorite)())
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  if is_fav.get() {
                    mica_icons::star_filled(p.favorite_hex)
                  } else {
                    mica_icons::star_icon(p.text_muted_hex)
                  }
                })),
            )
            .child(toolbar_btn(
              "trash-page-btn",
              "移入回收站",
              dark_mode,
              signal(false),
              move || (h_trash.on_trash_page)(),
              mica_icons::trash_icon,
            ))
            .child(toolbar_btn(
              "ed-more",
              "页面菜单",
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
            view()
              .w_full()
              .h(px(36.0))
              .px_3()
              .flex_row()
              .items_center()
              .justify_center()
              .border_t_1()
              .border_color(move || palette(dark_mode.get()).border_subtle)
              .child(fb())
          }
        })),
    )
    .child(when(move || page_menu_open.get(), move || {
      page_more_menu_view(h_menu.clone(), b_copy_md.clone(), page_menu_open, dark_mode)
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
      //    (backlinks + bottom click-to-focus) + non-blocking page-width grips +
      //    margin block-marquee selection + floating selection bubble menu +
      //    slash menu + word count badge.
      let m_down = marquee.clone();
      let m_move = marquee.clone();
      let m_up = marquee.clone();
      let m_overlay = marquee.clone();
      let on_commit_width = header.on_commit_page_width.clone();

      let br_count = b_body.clone();
      let br_keys = b_body.clone();
      let br_down = b_body.clone();
      let br_move_clear = b_body.clone();
      let br_title_focus = b_body.clone();
      let br_footer = b_body.clone();
      let br_hdr_drag = b_body.clone();
      let br_ftr_drag = b_body.clone();
      let br_edge_drag = b_body.clone();
      let br_bubble = b_body.clone();
      let h_bub = h_bubble.clone();
      let m_title_clear = marquee.clone();
      let m_footer_chk = marquee.clone();
      let layouts_marquee = layouts.clone();
      let rects_down = marquee_row_rects.clone();
      let rects_move = marquee_row_rects.clone();

      // Hidden native IME proxy sink closures
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
        .on_press_down(move |e: &PressEvent| {
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
            focused_code_block.set(None);
            code_caret_at_start.set(false);
            focused_table_cell.set(None);
            sink_ref.blur();
            editor_focused.set(false);
            marquee_origin.set(Some((lx, ly)));
          } else {
            if m_down.is_active_untracked() {
              m_down.clear();
            }
            marquee_origin.set(Some((lx, ly)));
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
          //    short lines, or below the last block!)
          let allow_marquee =
            in_margin_press.get_untracked() || body_blank_marquee_ready.get_untracked();
          if allow_marquee
            && let Some((ox, oy)) = marquee_origin.get_untracked()
          {
            let cx = e.position.x.0;
            let cy = e.position.y.0;
            if (cx - ox).abs() >= MARQUEE_DRAG_SLOP || (cy - oy).abs() >= MARQUEE_DRAG_SLOP {
              if drag_anchor.get_untracked().is_some() {
                drag_anchor.set(None);
                br_move_clear.clear_selection();
              }
              let rect = RectF::from_points(ox, oy, cx, cy);
              let rects = rects_move.borrow();
              m_move.update_drag(rect, &rects);
            }
            return;
          }
          // 2. Edge auto-scroll during upward/downward cross-block drag selection
          if let Some(anchor) = drag_anchor.get_untracked() {
            let (_, vh) = paper_bounds.get_untracked();
            let cy = e.position.y.0;
            let dir: i32 = if cy < 44.0 {
              -1
            } else if vh > 88.0 && cy > vh - 44.0 {
              1
            } else {
              0
            };
            if dir != 0 && !autoscroll_cooldown.get_untracked() {
              autoscroll_cooldown.set(true);
              spawn(async move {
                timer(Duration::from_millis(90)).await;
                autoscroll_cooldown.set(false);
              });
              let count = br_edge_drag.block_len();
              if count > 0 {
                let cur_node = br_edge_drag
                  .selection()
                  .map(|s| s.focus_pos().node)
                  .unwrap_or(anchor.node);
                let target_idx = if dir < 0 {
                  cur_node.saturating_sub(1)
                } else {
                  (cur_node + 1).min(count - 1)
                };
                if target_idx != cur_node {
                  let target_off = if dir < 0 {
                    0
                  } else {
                    br_edge_drag
                      .block_at(target_idx)
                      .map(|b| utf16_len(&b.text))
                      .unwrap_or(0)
                  };
                  br_edge_drag.set_selection(DocSelection::from_positions(
                    anchor,
                    DocPosition::new(target_idx, target_off),
                  ));
                  if target_idx == 0 {
                    list_ref.scroll_to_offset(px(0.0), false);
                  } else {
                    let align = if dir < 0 { Percentage(0.08) } else { Percentage(0.88) };
                    list_ref.scroll_to_index(target_idx as u32, align, px(0.0), false);
                  }
                }
              }
            }
          }
        })
        .on_mouse_up(move |_: &MouseEvent| {
          if drag_anchor.get_untracked().is_some() {
            drag_anchor.set(None);
            if has_text_selection.get_untracked() {
              bubble_recalc_rev.update(|v| *v = v.wrapping_add(1));
            }
          }
          if marquee_origin.get_untracked().is_some() {
            marquee_origin.set(None);
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
            .auto_correct(false)
            .auto_capitalize(AutoCapitalize::None)
            .paste_behavior(PasteBehavior::Report)
            .value(move || sink_value.get())
            .on_focus(move || {
              editor_focused.set(true);
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
                focused_code_block,
                code_caret_at_start,
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
                  focused_code_block,
                  code_caret_at_start,
                );
              }
            })
            .on_key_press(move |k: &str| {
              if k == "Backspace"
                && !st_key_press.borrow().composing
                && st_key_press.borrow().text.is_empty()
              {
                if try_backspace_to_title(
                  &br_key_press,
                  show_page_title,
                  title_ref,
                  sink_ref,
                  editor_focused,
                ) {
                  return;
                }
                br_key_press.dispatch(EditorCommand::DeleteBackward);
                sync_slash_palette_from_caret(&br_key_press, slash, ai_enabled);
              }
            })
            .on_paste(move |p: &PasteEvent| {
              if copied_link.get_untracked() {
                copied_link.set(false);
              }
              focused_code_block.set(None);
              code_caret_at_start.set(false);
              focused_table_cell.set(None);
              table_selection.set(None);
              empty_ime_sink(&st_paste, sink_value);
              handle_clipboard_paste(p, &br_paste, slash, ai_enabled);
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
          .on_scroll(move |_: &ScrollEvent| {
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
                if e.pressed_button == Some(MouseButton::Left)
                  && let Some(anchor) = drag_anchor.get_untracked()
                {
                  let next = DocSelection::from_positions(anchor, DocPosition::new(0, 0));
                  if br_hdr_drag.selection().as_ref() != Some(&next) {
                    br_hdr_drag.set_selection(next);
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
                    let b_enter = b_title_e.clone();
                    let b_tf = br_title_focus.clone();
                    let m_tc = m_title_clear.clone();
                    input()
                      .element_ref(&title_ref)
                      .test_id("page-title-input")
                      .placeholder("无标题")
                      .submit_behavior(SubmitBehavior::Submit)
                      .w_full()
                      .pb_1()
                      .text_size(px(30.0))
                      .font_weight(FontWeight::SEMIBOLD)
                      .text_color(move || palette(dark_mode.get()).text_primary)
                      .bg(move || palette(dark_mode.get()).surface_base)
                      .value(move || h_val.page_title.get())
                      .on_focus(move || {
                        b_tf.clear_selection();
                        m_tc.clear();
                        table_selection.set(None);
                        focused_code_block.set(None);
                        focused_table_cell.set(None);
                        editor_focused.set(false);
                      })
                      .on_change(move |v: &str| (h_chg.on_rename_title)(v.to_owned()))
                      .on_submit(move |_: &str| {
                        b_enter.focus_block(0, 0);
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
                if e.pressed_button == Some(MouseButton::Left)
                  && let Some(anchor) = drag_anchor.get_untracked()
                {
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
                      .on_press_down(move |_: &PressEvent| {
                        body_blank_marquee_ready.set(true);
                      })
                      .on_press({
                        let br_click = br_footer.clone();
                        let m_chk = m_footer_chk.clone();
                        move || {
                          if !m_chk.is_active_untracked() {
                            focused_code_block.set(None);
                            code_caret_at_start.set(false);
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
        .child(page_width_edge_grips(
          page_width,
          paper_bounds,
          dark_mode,
          on_commit_width,
        ))
        .child(when(move || has_text_selection.get(), {
          let br_b = br_bubble.clone();
          let h_b = h_bub.clone();
          move || {
            let br_c = br_b.clone();
            let h_c = h_b.clone();
            let narrow_sel = {
              let pw = paper_bounds.get().0;
              pw > 0.0 && pw < 640.0
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
    .child(render_lightbox_modal(lightbox, b_lightbox, dark_mode))
}

#[derive(Clone, Copy)]
enum EditorKeyAction {
  Enter,
  SoftBreak,
  Backspace,
  Delete,
  Indent(i32),
  ArrowLeft { shift: bool, word: bool },
  ArrowRight { shift: bool, word: bool },
  ArrowUp { shift: bool },
  ArrowDown { shift: bool },
  Home { shift: bool },
  End { shift: bool },
  SelectAll,
  Copy,
  Cut,
  PastePlain,
  Undo,
  Redo,
  ToggleMark(&'static str),
  OpenLink,
  Escape,
}

fn common_prefix_bytes(a: &str, b: &str) -> usize {
  a.char_indices()
    .zip(b.chars())
    .take_while(|((_, ca), cb)| ca == cb)
    .map(|((i, c), _)| i + c.len_utf8())
    .last()
    .unwrap_or(0)
}

fn empty_ime_sink(sink_state: &Rc<RefCell<ImeSinkState>>, sink_value: Signal<String>) {
  let mut st = sink_state.borrow_mut();
  if st.composing || st.text.is_empty() {
    return;
  }
  st.emptied = Some(std::mem::take(&mut st.text));
  drop(st);
  sink_value.set(String::new());
}

#[allow(clippy::too_many_arguments)]
fn apply_sink_text_diff(
  text: &str,
  sink_state: &Rc<RefCell<ImeSinkState>>,
  sink_value: Signal<String>,
  bridge: &MicaDocumentBridge,
  slash: SlashMenuSignals,
  ai_enabled: Signal<bool>,
  focused_code_block: Signal<Option<String>>,
  code_caret_at_start: Signal<bool>,
) {
  let (deleted, inserted) = {
    let mut st = sink_state.borrow_mut();
    let base = match st.emptied.take() {
      Some(old) if text != old && text.starts_with(old.as_str()) => old,
      Some(_) => String::new(),
      None => std::mem::take(&mut st.text),
    };
    let prefix = common_prefix_bytes(&base, text);
    let deleted = base[prefix..].chars().count();
    let inserted = text[prefix..].to_owned();
    st.text = text.to_owned();
    (deleted, inserted)
  };
  sink_value.set(text.to_owned());

  // Never drop keystrokes if the bridge somehow had no active selection yet.
  if (deleted > 0 || !inserted.is_empty()) && bridge.selection().is_none() {
    let last = bridge.block_len().saturating_sub(1);
    let end_off = bridge
      .block_at(last)
      .map(|b| utf16_len(&b.text))
      .unwrap_or(0);
    bridge.set_selection(DocSelection::caret(last, end_off));
  }

  let prev_focus_kind = bridge
    .selection()
    .and_then(|s| bridge.block_at(s.focus_pos().node))
    .map(|b| b.kind);

  for _ in 0..deleted {
    bridge.dispatch(EditorCommand::DeleteBackward);
  }
  if !inserted.is_empty() {
    let mut parts = inserted.split('\n').peekable();
    while let Some(part) = parts.next() {
      if !part.is_empty() {
        bridge.dispatch(EditorCommand::InsertText {
          text: part.to_owned(),
        });
      }
      if parts.peek().is_some() {
        bridge.dispatch(EditorCommand::SplitBlock);
      }
    }
  }
  if sink_state.borrow().text.len() > SINK_LIMIT {
    empty_ime_sink(sink_state, sink_value);
  }

  // If a markdown input rule (e.g. typing ```` ``` ````) just converted the focused
  // prose block into a `code_block`, hand focus straight to its live code editor.
  if prev_focus_kind.as_deref() != Some("code_block")
    && let Some(sel) = bridge.selection()
    && let Some(blk) = bridge.block_at(sel.focus_pos().node)
    && blk.kind == "code_block"
  {
    empty_ime_sink(sink_state, sink_value);
    code_caret_at_start.set(blk.text.is_empty());
    focused_code_block.set(Some(blk.id));
  }

  sync_slash_palette_from_caret(bridge, slash, ai_enabled);
}

fn write_rich_clipboard(payload: mica_editor::ClipboardPayload) {
  let mut item = ClipboardItem::default();
  item.text = if !payload.markdown.is_empty() {
    payload.markdown
  } else {
    payload.plain
  };
  if !payload.html.is_empty() {
    item.html = Some(payload.html);
  }
  set_clipboard_item(item);
}

/// If the caret is collapsed at `(block 0, offset 0)` of a plain paragraph and
/// the page title input is visible, moves focus cleanly back onto the title.
fn try_backspace_to_title(
  bridge: &MicaDocumentBridge,
  show_page_title: Signal<bool>,
  title_ref: ElementRef,
  sink_ref: ElementRef,
  editor_focused: Signal<bool>,
) -> bool {
  if !show_page_title.get_untracked() {
    return false;
  }
  let Some(sel) = bridge.selection() else {
    return false;
  };
  if !sel.is_collapsed() {
    return false;
  }
  let pos = sel.focus_pos();
  if pos.node != 0 || pos.offset != 0 {
    return false;
  }
  let Some(blk) = bridge.block_at(0) else {
    return false;
  };
  if blk.kind != "paragraph" {
    return false;
  }
  sink_ref.blur();
  editor_focused.set(false);
  bridge.clear_selection();
  title_ref.focus();
  true
}

/// Unified clipboard paste handler:
/// 1. Pulls Windows `CF_HTML` when NSH's Windows backend leaves `p.html` empty.
/// 2. Prefers data `<table>` HTML over companion preview bitmaps (Excel/Sheets).
/// 3. Pastes clipboard image bytes (`p.files`) or copied image files (`p.uris`).
/// 4. Falls through to Markdown / rich HTML paste (`bridge.paste_clipboard`).
fn handle_clipboard_paste(
  p: &PasteEvent,
  bridge: &MicaDocumentBridge,
  slash: SlashMenuSignals,
  ai_enabled: Signal<bool>,
) {
  let html = if p.html.is_empty() {
    crate::clipboard_platform::read_windows_cf_html().unwrap_or_default()
  } else {
    p.html.clone()
  };

  let is_data_table_html = !html.is_empty()
    && crate::clipboard_platform::clipboard_html_is_data_table(&html);

  // 1. Pasted image bytes (screenshot / copy image from browser or app)
  if !is_data_table_html && !p.files.is_empty() {
    let mut inserted_any = false;
    for f in &p.files {
      if let Some(uri) = crate::clipboard_platform::persist_pasted_image(&f.data, &f.mime, &f.name) {
        let alt = std::path::Path::new(&f.name)
          .file_stem()
          .and_then(|s| s.to_str())
          .filter(|s| !s.is_empty() && *s != "image")
          .unwrap_or("");
        bridge.insert_custom_block("image", alt, Some(&uri));
        inserted_any = true;
      }
    }
    if inserted_any {
      sync_slash_palette_from_caret(bridge, slash, ai_enabled);
      return;
    }
  }

  // 2. Pasted local image file URIs (copied from Explorer / Finder)
  if !is_data_table_html && !p.uris.is_empty() {
    let mut inserted_any = false;
    for u in &p.uris {
      if let Some((uri, alt)) = crate::clipboard_platform::image_uri_from_pasted_file_uri(u) {
        bridge.insert_custom_block("image", &alt, Some(&uri));
        inserted_any = true;
      }
    }
    if inserted_any {
      sync_slash_palette_from_caret(bridge, slash, ai_enabled);
      return;
    }
  }

  // 3. Markdown / rich HTML / plain text paste
  if !p.text.is_empty() || !html.is_empty() {
    bridge.paste_clipboard(&p.text, &html, false);
    sync_slash_palette_from_caret(bridge, slash, ai_enabled);
  }
}

#[allow(clippy::too_many_arguments)]
fn handle_editor_key(
  action: EditorKeyAction,
  bridge: &Rc<MicaDocumentBridge>,
  layouts: &Rc<RefCell<VisibleLayoutCache>>,
  preferred_caret_x: Signal<Option<f32>>,
  slash: SlashMenuSignals,
  ai_enabled: Signal<bool>,
  link_prompt_open: Signal<bool>,
  link_draft: Signal<String>,
  on_inline_ai: &Rc<dyn Fn(String)>,
  on_insert_image: &Rc<dyn Fn()>,
  show_page_title: Signal<bool>,
  title_ref: ElementRef,
  sink_ref: ElementRef,
  editor_focused: Signal<bool>,
) {
  // Any action other than Up/Down clears the remembered vertical visual column.
  if !matches!(
    action,
    EditorKeyAction::ArrowUp { .. } | EditorKeyAction::ArrowDown { .. }
  ) && preferred_caret_x.get_untracked().is_some()
  {
    preferred_caret_x.set(None);
  }

  // 1. Intercept ArrowUp / ArrowDown / Enter / Escape when slash menu is open.
  if slash.open.get_untracked() {
    let items = filter_slash_items(&slash.query.get_untracked(), ai_enabled.get_untracked());
    if items.is_empty() {
      close_slash(slash);
    } else {
      match action {
        EditorKeyAction::ArrowDown { shift: false } => {
          let next = (slash.index.get_untracked() + 1) % items.len();
          slash.index.set(next);
          return;
        }
        EditorKeyAction::ArrowUp { shift: false } => {
          let cur = slash.index.get_untracked().min(items.len() - 1);
          let next = (cur + items.len() - 1) % items.len();
          slash.index.set(next);
          return;
        }
        EditorKeyAction::Enter => {
          let idx = slash.index.get_untracked().min(items.len() - 1);
          let item = *items[idx];
          close_slash(slash);
          execute_slash_item(bridge, item, on_inline_ai, on_insert_image);
          return;
        }
        EditorKeyAction::Escape => {
          close_slash(slash);
          return;
        }
        _ => {}
      }
    }
  }

  match action {
    EditorKeyAction::Enter => {
      bridge.dispatch(EditorCommand::SplitBlock);
      sync_slash_palette_from_caret(bridge, slash, ai_enabled);
    }
    EditorKeyAction::SoftBreak => {
      bridge.dispatch(EditorCommand::InsertText {
        text: "\\\n".to_owned(),
      });
    }
    EditorKeyAction::Backspace => {
      if try_backspace_to_title(bridge, show_page_title, title_ref, sink_ref, editor_focused) {
        return;
      }
      bridge.dispatch(EditorCommand::DeleteBackward);
      sync_slash_palette_from_caret(bridge, slash, ai_enabled);
    }
    EditorKeyAction::Delete => {
      bridge.dispatch(EditorCommand::DeleteForward);
      sync_slash_palette_from_caret(bridge, slash, ai_enabled);
    }
    EditorKeyAction::Indent(delta) => {
      bridge.dispatch(EditorCommand::Indent { delta });
    }
    EditorKeyAction::SelectAll => {
      bridge.select_all();
    }
    EditorKeyAction::Copy => {
      if let Some(payload) = bridge.copy_selection() {
        write_rich_clipboard(payload);
      }
    }
    EditorKeyAction::Cut => {
      if let Some(payload) = bridge.cut_selection() {
        write_rich_clipboard(payload);
      }
    }
    EditorKeyAction::PastePlain => {
      let br_plain = bridge.clone();
      spawn(async move {
        let plain = clipboard_string().await;
        if !plain.is_empty() {
          br_plain.paste_clipboard(&plain, "", true);
          sync_slash_palette_from_caret(&br_plain, slash, ai_enabled);
        }
      });
    }
    EditorKeyAction::Undo => bridge.undo(),
    EditorKeyAction::Redo => bridge.redo(),
    EditorKeyAction::ToggleMark(m) => bridge.toggle_mark(m),
    EditorKeyAction::OpenLink => {
      link_draft.set(bridge.current_link_href().unwrap_or_default());
      link_prompt_open.set(true);
    }
    EditorKeyAction::Escape => {
      if let Some(sel) = bridge.selection()
        && !sel.is_collapsed()
      {
        let focus = sel.focus_pos();
        bridge.set_selection(DocSelection::caret(focus.node, focus.offset));
      }
    }
    EditorKeyAction::ArrowLeft { shift, word } => {
      move_caret_horizontal(bridge, -1, shift, word);
    }
    EditorKeyAction::ArrowRight { shift, word } => {
      move_caret_horizontal(bridge, 1, shift, word);
    }
    EditorKeyAction::ArrowUp { shift } => {
      move_caret_vertical_preserving_x(bridge, layouts, preferred_caret_x, -1, shift);
    }
    EditorKeyAction::ArrowDown { shift } => {
      move_caret_vertical_preserving_x(bridge, layouts, preferred_caret_x, 1, shift);
    }
    EditorKeyAction::Home { shift } => {
      if let Some(sel) = bridge.selection() {
        let focus = sel.focus_pos();
        let next = DocPosition::new(focus.node, 0);
        let anchor = if shift { sel.anchor_pos() } else { next };
        bridge.set_selection(DocSelection::from_positions(anchor, next));
      }
    }
    EditorKeyAction::End { shift } => {
      if let Some(sel) = bridge.selection() {
        let focus = sel.focus_pos();
        let end_off = bridge
          .block_at(focus.node)
          .map(|b| utf16_len(&b.text))
          .unwrap_or(0);
        let next = DocPosition::new(focus.node, end_off);
        let anchor = if shift { sel.anchor_pos() } else { next };
        bridge.set_selection(DocSelection::from_positions(anchor, next));
      }
    }
  }
}

fn move_caret_horizontal(bridge: &MicaDocumentBridge, dir: i32, shift: bool, word: bool) {
  let Some(sel) = bridge.selection() else {
    return;
  };
  if !shift && !sel.is_collapsed() {
    let target = if dir < 0 { sel.start_pos() } else { sel.end_pos() };
    bridge.set_selection(DocSelection::caret(target.node, target.offset));
    return;
  }
  let focus = sel.focus_pos();
  let bi = focus.node;
  let off = focus.offset;
  let count = bridge.block_len();
  let Some(block) = bridge.block_at(bi) else {
    return;
  };
  let len_u16 = utf16_len(&block.text);
  let next_pos = if dir < 0 {
    if off > 0 {
      let target_off = if word {
        prev_word_boundary(&block.text, off)
      } else {
        utf16_prev_boundary(&block.text, off)
      };
      DocPosition::new(bi, target_off)
    } else if bi > 0 {
      let prev_len = bridge
        .block_at(bi - 1)
        .map(|b| utf16_len(&b.text))
        .unwrap_or(0);
      DocPosition::new(bi - 1, prev_len)
    } else {
      focus
    }
  } else if off < len_u16 {
    let target_off = if word {
      next_word_boundary(&block.text, off)
    } else {
      utf16_next_boundary(&block.text, off)
    };
    DocPosition::new(bi, target_off)
  } else if bi + 1 < count {
    DocPosition::new(bi + 1, 0)
  } else {
    focus
  };
  let anchor = if shift { sel.anchor_pos() } else { next_pos };
  bridge.set_selection(DocSelection::from_positions(anchor, next_pos));
  if next_pos.node != bi {
    bridge.request_keep_block_visible(next_pos.node);
  }
}

fn prev_word_boundary(text: &str, u16_off: u32) -> u32 {
  let prefix = utf16_slice(text, 0, u16_off);
  let chars: Vec<(usize, char)> = prefix.char_indices().collect();
  let mut i = chars.len();
  while i > 0 && chars[i - 1].1.is_whitespace() {
    i -= 1;
  }
  while i > 0 && !chars[i - 1].1.is_whitespace() {
    i -= 1;
  }
  let byte_idx = if i < chars.len() { chars[i].0 } else { prefix.len() };
  utf16_len(&prefix[..byte_idx])
}

fn next_word_boundary(text: &str, u16_off: u32) -> u32 {
  let total = utf16_len(text);
  let suffix = utf16_slice(text, u16_off, total);
  let chars: Vec<(usize, char)> = suffix.char_indices().collect();
  let mut i = 0;
  while i < chars.len() && !chars[i].1.is_whitespace() {
    i += 1;
  }
  while i < chars.len() && chars[i].1.is_whitespace() {
    i += 1;
  }
  let byte_idx = if i < chars.len() { chars[i].0 } else { suffix.len() };
  u16_off + utf16_len(&suffix[..byte_idx])
}

/// Non-blocking side resize grips overlaid along the left & right edges of the
/// centered page column so dragging width works without shrinking `virtual_list`
/// (keeping the scrollbar permanently on the RIGHT EDGE of the window!).
fn page_width_edge_grips(
  width: Signal<f32>,
  paper_bounds: Signal<(f32, f32)>,
  dark_mode: Signal<bool>,
  on_commit: Rc<dyn Fn(f32)>,
) -> impl IntoElement {
  let live_width: Signal<Option<f32>> = signal(None);
  let hovered_side = signal(0_i8);
  let drag_start_x = signal(0.0_f32);
  let drag_start_rendered = signal(crate::page_width::PAGE_WIDTH_DEFAULT);
  let last_click_ms = signal(0.0_f64);

  let build_edge = move |side: f32, commit_cb: Rc<dyn Fn(f32)>| {
    let side_i8: i8 = if side < 0.0 { -1 } else { 1 };
    let tid = if side < 0.0 {
      "page-width-grip-left"
    } else {
      "page-width-grip-right"
    };
    view()
      .test_id(tid)
      .absolute()
      .top_0()
      .bottom_0()
      .left(move || {
        let outer = paper_bounds.get().0;
        let eff = live_width.get().unwrap_or_else(|| width.get());
        let room = (outer - 24.0).max(0.0);
        let col = crate::page_width::rendered_column_width(eff, room);
        let left_edge = ((outer - col) * 0.5).max(0.0);
        if side < 0.0 {
          px((left_edge - 12.0).max(0.0))
        } else {
          px((left_edge + col).min((outer - 22.0).max(0.0)))
        }
      })
      .w(move || {
        if paper_bounds.get().0 >= crate::responsive_shell::NARROW_BREAKPOINT {
          px(12.0)
        } else {
          px(0.0)
        }
      })
      .z_index(10)
      .items_center()
      .justify_center()
      .cursor(Cursor::ColResize)
      .on_mouse_enter(move |_: &MouseEvent| hovered_side.set(side_i8))
      .on_mouse_leave(move |_: &MouseEvent| {
        if hovered_side.get_untracked() == side_i8 {
          hovered_side.set(0);
        }
      })
      .claim_touches(TouchClaim::on_start())
      .keep_touches(true)
      .on_responder_grant(move |e: &ResponderEvent| {
        let outer = paper_bounds.get_untracked().0;
        if outer < crate::responsive_shell::NARROW_BREAKPOINT {
          return;
        }
        let stored = width.get_untracked();
        let avail = (outer - 24.0).max(1.0);
        drag_start_x.set(f32::from(e.page.x));
        drag_start_rendered.set(crate::page_width::rendered_column_width(stored, avail));
        live_width.set(Some(stored));
      })
      .on_responder_move(move |e: &ResponderEvent| {
        if live_width.get_untracked().is_none() {
          return;
        }
        let dx = f32::from(e.page.x) - drag_start_x.get_untracked();
        let stored = width.get_untracked();
        let start_r = drag_start_rendered.get_untracked();
        let avail = (paper_bounds.get_untracked().0 - 24.0).max(1.0);
        let next = crate::page_width::compute_drag_width(stored, start_r, avail, side, dx);
        live_width.set(Some(next));
      })
      .on_responder_release(move |e: &ResponderEvent| {
        let moved = (f32::from(e.page.x) - drag_start_x.get_untracked()).abs();
        let finished = live_width.get_untracked();
        live_width.set(None);
        if moved < 4.0 {
          let now = e.timestamp;
          let prev = last_click_ms.get_untracked();
          if prev > 0.0 && now >= prev && (now - prev) <= 350.0 {
            last_click_ms.set(0.0);
            if (width.get_untracked() - crate::page_width::PAGE_WIDTH_DEFAULT).abs() > 0.5 {
              (commit_cb)(crate::page_width::PAGE_WIDTH_DEFAULT);
            }
          } else {
            last_click_ms.set(now);
          }
          return;
        }
        last_click_ms.set(0.0);
        if let Some(target) = finished
          && (target - width.get_untracked()).abs() > 0.5
        {
          (commit_cb)(target);
        }
      })
      .child(
        view()
          .w(px(2.0))
          .h(px(48.0))
          .rounded(px(2.0))
          .bg(move || {
            let active = live_width.get().is_some() || hovered_side.get() == side_i8;
            if active {
              palette(dark_mode.get()).accent
            } else {
              TRANSPARENT_RGBA
            }
          }),
      )
  };

  view()
    .absolute()
    .top_0()
    .bottom_0()
    .left_0()
    .right_0()
    .pointer_events(PointerEvents::BoxNone)
    .child(build_edge(-1.0, on_commit.clone()))
    .child(build_edge(1.0, on_commit))
    .child(when(move || live_width.get().is_some(), move || {
      view()
        .absolute()
        .top(px(12.0))
        .left_0()
        .right_0()
        .items_center()
        .pointer_events(PointerEvents::None)
        .z_index(25)
        .child(
          view()
            .px_2()
            .py_1()
            .rounded(px(6.0))
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .child(
              text(move || {
                let lw = live_width.get().unwrap_or_else(|| width.get());
                let avail = (paper_bounds.get().0 - 24.0).max(0.0);
                let rw = crate::page_width::rendered_column_width(lw, avail);
                crate::page_width::width_readout_label(lw, rw)
              })
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_primary),
            ),
        )
    }))
}

fn close_slash(slash: SlashMenuSignals) {
  slash.open.set(false);
  slash.query.set(String::new());
  slash.index.set(0);
  slash.opened_by_btn.set(false);
}

fn update_slash_anchor(_bridge: &MicaDocumentBridge, slash: SlashMenuSignals) {
  let (box_w, _box_h) = slash.box_size.get_untracked();
  let left = ((box_w - SLASH_MENU_WIDTH) * 0.5)
    .clamp(16.0, (box_w - SLASH_MENU_WIDTH - 16.0).max(16.0));
  slash.anchor.set((left, 64.0));
}

fn toggle_block_type(bridge: &MicaDocumentBridge, kind: &str, level: Option<u64>) {
  let cur_kind = bridge.active_kind_signal().get_untracked();
  let cur_level = bridge.active_level_signal().get_untracked();
  let same_kind = match kind {
    "bulleted_list" => matches!(cur_kind.as_str(), "bulleted_list" | "bulletList"),
    "numbered_list" => matches!(cur_kind.as_str(), "numbered_list" | "orderedList"),
    "todo" => matches!(cur_kind.as_str(), "todo" | "taskItem"),
    "quote" => matches!(cur_kind.as_str(), "quote" | "blockquote"),
    other => cur_kind == other,
  };
  let already_same = same_kind && level.is_none_or(|lvl| cur_level == lvl);
  if already_same && kind != "paragraph" {
    bridge.set_block_type("paragraph", None);
  } else {
    bridge.set_block_type(kind, level);
  }
  bridge.request_focus();
}

fn sync_slash_palette_from_caret(
  bridge: &MicaDocumentBridge,
  slash: SlashMenuSignals,
  ai_enabled: Signal<bool>,
) {
  let Some(sel) = bridge.selection() else {
    if slash.open.get_untracked() && !slash.opened_by_btn.get_untracked() {
      close_slash(slash);
    }
    return;
  };
  if !sel.is_collapsed() {
    if slash.open.get_untracked() && !slash.opened_by_btn.get_untracked() {
      close_slash(slash);
    }
    return;
  }
  let focus = sel.focus_pos();
  if let Some(block) = bridge.block_at(focus.node)
    && !matches!(block.kind.as_str(), "code_block" | "table" | "image" | "divider" | "math_block")
  {
    let before = utf16_slice(&block.text, 0, focus.offset);
    if let Some((_, query)) = extract_slash_query(before) {
      let items = filter_slash_items(query, ai_enabled.get_untracked());
      if items.is_empty() {
        close_slash(slash);
        return;
      }
      slash.opened_by_btn.set(false);
      slash.query.set(query.to_owned());
      let cur = slash.index.get_untracked();
      if cur >= items.len() {
        slash.index.set(0);
      }
      update_slash_anchor(bridge, slash);
      slash.open.set(true);
      return;
    }
  }
  if slash.open.get_untracked() && !slash.opened_by_btn.get_untracked() {
    close_slash(slash);
  }
}

fn execute_slash_item(
  bridge: &MicaDocumentBridge,
  item: SlashItem,
  on_inline_ai: &Rc<dyn Fn(String)>,
  on_insert_image: &Rc<dyn Fn()>,
) {
  if item.kind == "__ai__" {
    bridge.replace_slash_token_and_apply("__ai__", None, None, None);
    on_inline_ai(String::new());
    return;
  }
  if item.kind == "image" {
    bridge.replace_slash_token_and_apply("__ai__", None, None, None);
    on_insert_image();
    return;
  }
  bridge.replace_slash_token_and_apply(item.kind, item.level, item.alert, item.template);
}

/// Caret-anchored floating 22-item vertical slash menu (exact Flutter `editor.dart` layout).
fn caret_slash_menu_view(
  bridge: Rc<MicaDocumentBridge>,
  header: Rc<EditorHeaderProps>,
  slash: SlashMenuSignals,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let ai_enabled = header.ai_enabled;
  let on_ai = header.on_inline_ai.clone();
  let on_img = header.on_insert_image.clone();

  view()
    .test_id("slash-menu")
    .absolute()
    .left(move || px(slash.anchor.get().0))
    .top(move || px(slash.anchor.get().1))
    .w(px(SLASH_MENU_WIDTH))
    .max_h(px(SLASH_MENU_MAX_H))
    .py(px(6.0))
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .z_index(36)
    .child(
      scroll()
        .w_full()
        .max_h(px(SLASH_MENU_MAX_H - 12.0))
        .child(dynamic(move || {
          let q = slash.query.get();
          let active_idx = slash.index.get();
          let items = filter_slash_items(&q, ai_enabled.get());
          let p = palette(dark_mode.get());
          let mut list = view().w_full();
          for (i, item) in items.into_iter().enumerate() {
            let is_active = i == active_idx;
            let b = bridge.clone();
            let ai_cb = on_ai.clone();
            let img_cb = on_img.clone();
            let item_copy = *item;
            let icon_color = slash_icon_hex(item_copy.icon_key, &p);
            list = list.child(
              button()
                .test_id(item_copy.id)
                .accessibility_label(format!("插入 {}", item_copy.label))
                .w_full()
                .px(px(12.0))
                .py(px(7.0))
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .bg(if is_active {
                  p.surface_hover
                } else {
                  p.surface_overlay
                })
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || {
                  close_slash(slash);
                  execute_slash_item(&b, item_copy, &ai_cb, &img_cb);
                })
                .child(slash_item_icon(item_copy.icon_key, icon_color))
                .child(
                  text(item_copy.label)
                    .text_size(px(13.5))
                    .text_color(p.text_primary)
                    .truncate(),
                ),
            );
          }
          list
        })),
    )
}

fn slash_icon_hex(key: &str, p: &crate::mica_theme::MicaPalette) -> &'static str {
  match key {
    "sparkles" => p.accent_hex,
    "alert_note" => p.accent_hex,
    "alert_tip" => p.success_hex,
    "alert_important" => p.accent_hex,
    "alert_warning" => p.warning_hex,
    "alert_caution" => p.danger_hex,
    _ => p.text_muted_hex,
  }
}

fn slash_item_icon(key: &str, color: &'static str) -> Svg {
  match key {
    "sparkles" => mica_icons::sparkles_icon(color),
    "text" => mica_icons::align_text(color),
    "h1" => mica_icons::heading_box("1", color),
    "h2" => mica_icons::heading_box("2", color),
    "h3" => mica_icons::heading_box("3", color),
    "bullet" => mica_icons::list_bullet(color),
    "ordered" => mica_icons::list_ordered(color),
    "todo" => mica_icons::todo_box(color),
    "quote" => mica_icons::quote_icon(color),
    "alert_note" | "alert_tip" | "alert_important" | "alert_warning" | "alert_caution" => {
      mica_icons::info_icon(color)
    }
    "code" => mica_icons::code_icon(color),
    "math" => math_icon(color),
    "whiteboard" => mica_icons::whiteboard(color),
    "mindmap" | "flowchart" => mica_icons::graph_nodes(color),
    "table" => mica_icons::table_icon(color),
    "divider" => mica_icons::divider_icon(color),
    "image" => image_icon(color),
    _ => mica_icons::file_doc(color),
  }
}

/// Full `⋯` page menu matching `main:clients/mica_flutter/lib/ui/page_header_actions.dart`.
fn page_more_menu_view(
  header: Rc<EditorHeaderProps>,
  bridge: Rc<MicaDocumentBridge>,
  page_menu_open: Signal<bool>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let comments_vis = header.comments_visible;
  let props_vis = header.show_properties;
  let on_comments = header.on_toggle_comments.clone();
  let on_props = header.on_toggle_properties.clone();
  let on_zip = header.on_export_zip.clone();
  let on_html = header.on_export_html.clone();
  let on_pdf = header.on_export_pdf.clone();
  let on_import = header.on_import_markdown_picker.clone();
  let on_ver = header.on_open_version_history.clone();
  let on_snap = header.on_save_snapshot.clone();
  let on_trash = header.on_trash_page.clone();

  view()
    .test_id("page-more-menu")
    .absolute()
    .top(px(42.0))
    .right(px(12.0))
    .w(px(220.0))
    .p_1()
    .gap(px(2.0))
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .z_index(35)
    .child(menu_check_row(
      "page-menu-comments",
      "评论",
      comments_vis,
      dark_mode,
      move || {
        on_comments();
        page_menu_open.set(false);
      },
    ))
    .child(menu_check_row(
      "page-menu-properties",
      "属性",
      props_vis,
      dark_mode,
      move || {
        on_props();
        page_menu_open.set(false);
      },
    ))
    .child(menu_sep(dark_mode))
    .child(menu_row("page-menu-copy-md", "复制页面内容", dark_mode, move || {
      set_clipboard(bridge.export_markdown());
      bridge.set_status_flash("已复制页面内容");
      page_menu_open.set(false);
    }))
    .child(menu_sep(dark_mode))
    .child(menu_row("page-menu-export-zip", "导出(ZIP,含图片)", dark_mode, move || {
      on_zip();
      page_menu_open.set(false);
    }))
    .child(menu_row("page-menu-export-html", "导出为 HTML", dark_mode, move || {
      on_html();
      page_menu_open.set(false);
    }))
    .child(menu_row("page-menu-export-pdf", "导出为 PDF", dark_mode, move || {
      on_pdf();
      page_menu_open.set(false);
    }))
    .child(menu_sep(dark_mode))
    .child(menu_row("page-menu-import-md", "导入 Markdown…", dark_mode, move || {
      on_import();
      page_menu_open.set(false);
    }))
    .child(menu_sep(dark_mode))
    .child(menu_row("page-menu-version-history", "版本历史", dark_mode, move || {
      on_ver();
      page_menu_open.set(false);
    }))
    .child(menu_row("page-menu-snapshot", "保存检查点", dark_mode, move || {
      on_snap();
      page_menu_open.set(false);
    }))
    .child(menu_sep(dark_mode))
    .child(menu_row("page-menu-trash", "移入回收站", dark_mode, move || {
      on_trash();
      page_menu_open.set(false);
    }))
}

/// Floating inline link URL editor popover.
fn link_editor_popover(
  bridge: Rc<MicaDocumentBridge>,
  link_draft: Signal<String>,
  link_prompt_open: Signal<bool>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let b_apply = bridge.clone();
  let b_submit = bridge.clone();
  let b_remove = bridge;

  view()
    .test_id("link-editor-popover")
    .absolute()
    .top(px(46.0))
    .right(px(24.0))
    .z_index(38)
    .px_2()
    .py_1()
    .flex_row()
    .items_center()
    .gap_1()
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .child(dynamic(move || {
      mica_icons::link_icon(palette(dark_mode.get()).text_muted_hex)
    }))
    .child(
      input()
        .test_id("link-url-input")
        .auto_focus(true)
        .submit_behavior(SubmitBehavior::Submit)
        .placeholder("https://...")
        .w(px(220.0))
        .px_2()
        .py_1()
        .rounded(px(5.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_subtle)
        .text_size(px(12.5))
        .text_color(move || palette(dark_mode.get()).text_primary)
        .bg(move || palette(dark_mode.get()).surface_base)
        .value(move || link_draft.get())
        .on_change(move |v: &str| link_draft.set(v.to_owned()))
        .on_submit(move |v: &str| {
          b_submit.set_link_mark(v);
          link_prompt_open.set(false);
          b_submit.request_focus();
        }),
    )
    .child(
      button()
        .test_id("link-apply-btn")
        .accessibility_label("应用链接")
        .px_2()
        .py_1()
        .rounded(px(5.0))
        .bg(move || palette(dark_mode.get()).accent)
        .on_press(move || {
          b_apply.set_link_mark(&link_draft.get_untracked());
          link_prompt_open.set(false);
          b_apply.request_focus();
        })
        .child(
          text("应用")
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).on_primary),
        ),
    )
    .child(
      button()
        .test_id("link-remove-btn")
        .accessibility_label("移除链接")
        .px_2()
        .py_1()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || {
          b_remove.unset_link_mark();
          link_prompt_open.set(false);
          b_remove.request_focus();
        })
        .child(
          text("移除")
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).text_muted),
        ),
    )
    .child(
      button()
        .test_id("link-close-btn")
        .accessibility_label("关闭链接编辑")
        .w(px(22.0))
        .h(px(22.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || link_prompt_open.set(false))
        .child(dynamic(move || {
          mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
        })),
    )
}

fn step_find_match(
  bridge: &MicaDocumentBridge,
  find_matches: Signal<Vec<FindMatch>>,
  find_index: Signal<usize>,
  delta: isize,
) {
  let hits = find_matches.get_untracked();
  if hits.is_empty() {
    return;
  }
  let len = hits.len() as isize;
  let cur = (find_index.get_untracked() as isize).clamp(0, len - 1);
  let next = ((cur + delta).rem_euclid(len)) as usize;
  find_index.set(next);
  bridge.select_find_match(&hits[next], next);
}

/// Floating top-right in-page find bar (`Ctrl+F`), 1:1 with Flutter `_buildFindBar`.
fn find_bar_view(
  bridge: Rc<MicaDocumentBridge>,
  find_open: Signal<bool>,
  find_query: Signal<String>,
  find_matches: Signal<Vec<FindMatch>>,
  find_index: Signal<usize>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let b_chg = bridge.clone();
  let b_submit = bridge.clone();
  let b_prev = bridge.clone();
  let b_next = bridge.clone();
  let b_close_key = bridge.clone();
  let b_close_btn = bridge;

  view()
    .test_id("editor-find-bar")
    .absolute()
    .top(px(46.0))
    .right(px(14.0))
    .z_index(37)
    .px_2()
    .py_1()
    .flex_row()
    .items_center()
    .gap(px(4.0))
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .child(dynamic(move || {
      mica_icons::search(palette(dark_mode.get()).text_muted_hex)
    }))
    .child(
      input()
        .test_id("editor-find-input")
        .auto_focus(true)
        .submit_behavior(SubmitBehavior::Submit)
        .placeholder("页内查找")
        .w(px(172.0))
        .px_1()
        .py_1()
        .text_size(px(13.0))
        .text_color(move || palette(dark_mode.get()).text_primary)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .value(move || find_query.get())
        .on_change(move |q: &str| {
          find_query.set(q.to_owned());
          let hits = b_chg.find_matches(q);
          find_index.set(0);
          if let Some(first) = hits.first() {
            b_chg.select_find_match(first, 0);
          }
          find_matches.set(hits);
        })
        .on_submit(move |_: &str| {
          step_find_match(&b_submit, find_matches, find_index, 1);
        })
        .on_key_press(move |k: &str| {
          if k == "Escape" {
            find_open.set(false);
            b_close_key.clear_find_highlights();
            b_close_key.request_focus();
          }
        }),
    )
    .child(
      view()
        .test_id("editor-find-count")
        .w(px(48.0))
        .items_end()
        .child(
          text(move || {
            let q = find_query.get();
            if q.is_empty() {
              String::new()
            } else {
              let total = find_matches.get().len();
              if total == 0 {
                "无结果".to_string()
              } else {
                format!("{}/{}", find_index.get().min(total - 1) + 1, total)
              }
            }
          })
          .text_size(px(11.5))
          .text_color(move || palette(dark_mode.get()).text_faint),
        ),
    )
    .child(
      button()
        .test_id("editor-find-prev")
        .accessibility_label("上一个 (Shift+Enter)")
        .w(px(24.0))
        .h(px(24.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || step_find_match(&b_prev, find_matches, find_index, -1))
        .child(dynamic(move || {
          chevron_up_icon(palette(dark_mode.get()).text_muted_hex)
        })),
    )
    .child(
      button()
        .test_id("editor-find-next")
        .accessibility_label("下一个 (Enter)")
        .w(px(24.0))
        .h(px(24.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || step_find_match(&b_next, find_matches, find_index, 1))
        .child(dynamic(move || {
          mica_icons::chevron_down(palette(dark_mode.get()).text_muted_hex)
        })),
    )
    .child(
      button()
        .test_id("editor-find-close")
        .accessibility_label("关闭 (Esc)")
        .w(px(24.0))
        .h(px(24.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
        .on_press(move || {
          find_open.set(false);
          b_close_btn.clear_find_highlights();
          b_close_btn.request_focus();
        })
        .child(dynamic(move || {
          mica_icons::close_sm(palette(dark_mode.get()).text_muted_hex)
        })),
    )
}

/// Expanded floating selection bubble menu (marks + link + block transforms + AI rewrite).
fn bubble_menu_view(
  bridge: Rc<MicaDocumentBridge>,
  header: Rc<EditorHeaderProps>,
  has_text_selection: Signal<bool>,
  link_prompt_open: Signal<bool>,
  link_draft: Signal<String>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  when(move || has_text_selection.get(), move || {
    let active_marks = bridge.active_marks_signal();
    let active_kind = bridge.active_kind_signal();
    let active_level = bridge.active_level_signal();
    let ai_enabled = header.ai_enabled;
    let on_ai = header.on_inline_ai.clone();

    let b_bold = bridge.clone();
    let b_italic = bridge.clone();
    let b_strike = bridge.clone();
    let b_code = bridge.clone();
    let b_link = bridge.clone();
    let b_h1 = bridge.clone();
    let b_h2 = bridge.clone();
    let b_h3 = bridge.clone();
    let b_para = bridge.clone();
    let b_bullet = bridge.clone();
    let b_ordered = bridge.clone();
    let b_todo = bridge.clone();
    let b_quote = bridge.clone();
    let b_code_blk = bridge.clone();
    let b_ai = bridge.clone();

    view()
      .test_id("editor-bubble-menu")
      .flex_row()
      .items_center()
      .gap(px(2.0))
      .px_1()
      .py_1()
      .rounded(px(8.0))
      .border_1()
      .border_color(move || palette(dark_mode.get()).border_normal)
      .bg(move || palette(dark_mode.get()).surface_overlay)
      .child(bubble_btn(
        "bubble-bold",
        "加粗",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "bold" || m == "strong")),
        move || b_bold.toggle_mark("bold"),
        mica_icons::bold_icon,
      ))
      .child(bubble_btn(
        "bubble-italic",
        "斜体",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "italic" || m == "em")),
        move || b_italic.toggle_mark("italic"),
        mica_icons::italic_icon,
      ))
      .child(bubble_btn(
        "bubble-strike",
        "删除线",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "strike")),
        move || b_strike.toggle_mark("strike"),
        mica_icons::strikethrough_icon,
      ))
      .child(bubble_btn(
        "bubble-code",
        "行内代码",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "code")),
        move || b_code.toggle_mark("code"),
        mica_icons::code_icon,
      ))
      .child(bubble_btn(
        "bubble-link",
        "链接",
        dark_mode,
        computed(move || active_marks.get().iter().any(|m| m == "link")),
        move || {
          link_draft.set(b_link.current_link_href().unwrap_or_default());
          link_prompt_open.set(true);
        },
        mica_icons::link_icon,
      ))
      .child(toolbar_sep(dark_mode))
      .child(bubble_btn(
        "bubble-h1",
        "一级标题",
        dark_mode,
        computed(move || active_kind.get() == "heading" && active_level.get() == 1),
        move || toggle_block_type(&b_h1, "heading", Some(1)),
        |c| mica_icons::heading_box("1", c),
      ))
      .child(bubble_btn(
        "bubble-h2",
        "二级标题",
        dark_mode,
        computed(move || active_kind.get() == "heading" && active_level.get() == 2),
        move || toggle_block_type(&b_h2, "heading", Some(2)),
        |c| mica_icons::heading_box("2", c),
      ))
      .child(bubble_btn(
        "bubble-h3",
        "三级标题",
        dark_mode,
        computed(move || active_kind.get() == "heading" && active_level.get() == 3),
        move || toggle_block_type(&b_h3, "heading", Some(3)),
        |c| mica_icons::heading_box("3", c),
      ))
      .child(bubble_btn(
        "bubble-para",
        "正文",
        dark_mode,
        computed(move || active_kind.get() == "paragraph"),
        move || b_para.set_block_type("paragraph", None),
        mica_icons::align_text,
      ))
      .child(bubble_btn(
        "bubble-bullet",
        "无序列表",
        dark_mode,
        computed(move || matches!(active_kind.get().as_str(), "bulleted_list" | "bulletList")),
        move || toggle_block_type(&b_bullet, "bulleted_list", None),
        mica_icons::list_bullet,
      ))
      .child(bubble_btn(
        "bubble-ordered",
        "有序列表",
        dark_mode,
        computed(move || matches!(active_kind.get().as_str(), "numbered_list" | "orderedList")),
        move || toggle_block_type(&b_ordered, "numbered_list", None),
        mica_icons::list_ordered,
      ))
      .child(bubble_btn(
        "bubble-todo",
        "待办",
        dark_mode,
        computed(move || matches!(active_kind.get().as_str(), "todo" | "taskItem")),
        move || toggle_block_type(&b_todo, "todo", None),
        mica_icons::todo_box,
      ))
      .child(bubble_btn(
        "bubble-quote",
        "引用",
        dark_mode,
        computed(move || matches!(active_kind.get().as_str(), "quote" | "blockquote")),
        move || toggle_block_type(&b_quote, "quote", None),
        mica_icons::quote_icon,
      ))
      .child(bubble_btn(
        "bubble-code-block",
        "代码块",
        dark_mode,
        computed(move || active_kind.get() == "code_block"),
        move || b_code_blk.insert_custom_block("code_block", "", Some("rust")),
        terminal_icon,
      ))
      .child(when(move || ai_enabled.get(), {
        let on_ai = on_ai.clone();
        let b_ai = b_ai.clone();
        move || {
          let on_ai = on_ai.clone();
          let b_ai = b_ai.clone();
          view()
            .flex_row()
            .items_center()
            .child(toolbar_sep(dark_mode))
            .child(
              button()
                .test_id("bubble-ai")
                .accessibility_label("用 AI 改写所选")
                .h(px(26.0))
                .px_2()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .rounded(px(5.0))
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || on_ai(b_ai.selected_text()))
                .child(dynamic(move || {
                  mica_icons::sparkles_icon(palette(dark_mode.get()).accent_hex)
                }))
                .child(
                  text("用 AI 改写所选")
                    .text_size(px(11.5))
                    .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            )
        }
      }))
  })
}

fn bubble_btn<F, I>(
  tid: &'static str,
  a11y: &'static str,
  dark_mode: Signal<bool>,
  active: Memo<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  button()
    .test_id(tid)
    .accessibility_label(a11y)
    .w(px(28.0))
    .h(px(28.0))
    .items_center()
    .justify_center()
    .rounded(px(5.0))
    .bg(move || {
      let p = palette(dark_mode.get());
      if active.get() {
        p.surface_hover
      } else {
        p.surface_overlay
      }
    })
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      icon_fn(if active.get() {
        p.text_primary_hex
      } else {
        p.text_muted_hex
      })
    }))
}

fn local_stroked(size_px: f32, color: &str, inner: &str) -> Svg {
  let doc = format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="none" stroke="{color}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{inner}</svg>"##
  );
  svg(ImageSrc::svg_markup(&doc)).w(px(size_px)).h(px(size_px))
}

fn terminal_icon(color: &str) -> Svg {
  local_stroked(
    17.0,
    color,
    r##"<rect x="2.8" y="4" width="14.4" height="12" rx="2"/><path d="M6.2 8.2L8.6 10.2L6.2 12.2M10.5 12.2H13.8"/>"##,
  )
}

fn image_icon(color: &str) -> Svg {
  local_stroked(
    17.0,
    color,
    r##"<rect x="3" y="4" width="14" height="12" rx="2"/><circle cx="7.5" cy="8" r="1.2"/><path d="M3.8 14L7.8 10.2L10.8 13L13.2 10.8L16.4 14"/>"##,
  )
}

fn lock_icon(color: &str) -> Svg {
  local_stroked(
    14.0,
    color,
    r##"<rect x="4.5" y="9" width="11" height="7.5" rx="1.8"/><path d="M7 9V6.8A3 3 0 0 1 13 6.8V9"/>"##,
  )
}

fn chevron_up_icon(color: &str) -> Svg {
  local_stroked(12.0, color, r##"<path d="M5 12.5L10 7.5L15 12.5"/>"##)
}

fn math_icon(color: &str) -> Svg {
  local_stroked(
    15.0,
    color,
    r##"<path d="M14.5 5H6.5L11 10L6.5 15H14.5"/>"##,
  )
}

fn toolbar_btn<F, I>(
  tid: &'static str,
  a11y: &'static str,
  dark_mode: Signal<bool>,
  active: Signal<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  button()
    .test_id(tid)
    .accessibility_label(a11y)
    .w(px(28.0))
    .h(px(28.0))
    .items_center()
    .justify_center()
    .rounded(px(5.0))
    .bg(move || {
      let p = palette(dark_mode.get());
      if active.get() {
        p.surface_hover
      } else {
        p.surface_base
      }
    })
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      icon_fn(if active.get() {
        p.text_primary_hex
      } else {
        p.text_muted_hex
      })
    }))
}

fn toolbar_btn_active<F, I>(
  tid: &'static str,
  a11y: &'static str,
  dark_mode: Signal<bool>,
  active: Memo<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  button()
    .test_id(tid)
    .accessibility_label(a11y)
    .w(px(28.0))
    .h(px(28.0))
    .items_center()
    .justify_center()
    .rounded(px(5.0))
    .bg(move || {
      let p = palette(dark_mode.get());
      if active.get() {
        p.surface_hover
      } else {
        p.surface_base
      }
    })
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      icon_fn(if active.get() {
        p.text_primary_hex
      } else {
        p.text_muted_hex
      })
    }))
}

fn toolbar_btn_dim<F, I>(
  tid: &'static str,
  a11y: &'static str,
  dark_mode: Signal<bool>,
  enabled: Memo<bool>,
  on_click: impl Fn() + 'static,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  button()
    .test_id(tid)
    .accessibility_label(a11y)
    .w(px(28.0))
    .h(px(28.0))
    .items_center()
    .justify_center()
    .rounded(px(5.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      icon_fn(if enabled.get() {
        p.text_muted_hex
      } else {
        p.text_ghost_hex
      })
    }))
}

fn toolbar_sep(dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w(px(1.0))
    .h(px(14.0))
    .mx(px(2.0))
    .bg(move || palette(dark_mode.get()).border_normal)
}

fn menu_sep(dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w_full()
    .h(px(1.0))
    .my(px(2.0))
    .bg(move || palette(dark_mode.get()).border_subtle)
}

fn menu_row(
  tid: &'static str,
  label: &'static str,
  dark_mode: Signal<bool>,
  on_click: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .w_full()
    .px_3()
    .py_1()
    .rounded(px(6.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(
      text(label)
        .text_size(px(12.5))
        .text_color(move || palette(dark_mode.get()).text_primary),
    )
}

fn menu_check_row(
  tid: &'static str,
  label: &'static str,
  checked: Signal<bool>,
  dark_mode: Signal<bool>,
  on_click: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .w_full()
    .px_3()
    .py_1()
    .flex_row()
    .items_center()
    .justify_between()
    .rounded(px(6.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(
      text(move || {
        if checked.get() {
          format!("✓ {label}")
        } else {
          label.to_string()
        }
      })
      .text_size(px(12.5))
      .text_color(move || palette(dark_mode.get()).text_primary),
    )
}
