//! Virtualized block renderers for Mica's pure-Rust `mica-editor` + `virtual_list`
//! document view (`AGENTS.md` Route 2-B).
//!
//! Key guarantees:
//! - **Zero non-virtualized `RichEditor` dependency**: every block renders inside
//!   a recycled `virtual_list` cell subscribed ONLY to its own per-block `Signal<u64>`.
//! - **Scoped selection & caret overlay**: moving the caret or dragging selection
//!   never rebuilds text spans, code blocks, boards, or tables.
//! - **Top-level `overlay()` popovers**: code-block language picker, AI menu, and
//!   more menu render in NSH's top overlay layer anchored to their trigger buttons,
//!   so embedded whiteboards or sibling blocks never clip or cover them.
//! - **Lazy heavy editors**: code blocks render lightweight monospace text when
//!   unfocused and mount `nsh_base::editor()` (`min_rows(1)`) on click/focus.
//! - **Single-layer cards**: zero double-bordered quotes, zero top gray strip on
//!   embedded boards (snug 20px auto-fit + draggable resize handles), and fullscreen
//!   zoom lightbox modals for Mermaid diagrams and images.

use std::{
  cell::{Cell, RefCell},
  rc::Rc,
  time::Duration,
};

use mica_editor::{
  DocPosition, DocSelection, EditorBlock,
  marks::EditorMark,
  utf16::{utf16_len, utf16_slice},
};
use nsh_base::{EditorScrollBeyond, EditorStyles, button, editor, input, text_view::PartStyle};
use nsh_core::{
  Color, PropertyValue, StyleDecl, StyleKey, SubmitBehavior,
  color::Rgba,
  element::{
    Align, ElementRef, ImageSrc, Placement, Style, Svg, element_ref, open_url, overlay,
    set_clipboard, svg, text_measurer,
  },
  geometry::{Bounds, Point, Points, px},
  prelude::*,
};
use serde_json::Value;

use crate::{
  editor_bridge::{MicaDocumentBridge, block_marks, effective_block_kind},
  mica_icons,
  mica_theme::palette,
  text_geometry::{
    VisibleLayoutCache, block_typography, caret_rect_for_offset, offset_at_point,
    resolve_lines_for_block, selection_rects_for_range,
  },
};

pub use crate::table_view::compute_adaptive_table_layout;

type TextMeasureFn = Rc<dyn Fn(&str, &str) -> Points>;

pub(crate) const TRANSPARENT_RGBA: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
const MATH_LIGHT_BG: Rgba = Rgba {
  r: 0xF5 as f32 / 255.0,
  g: 0xF3 as f32 / 255.0,
  b: 0xFF as f32 / 255.0,
  a: 1.0,
};

// ── Public Context Callbacks & Lightbox State ────────────────────────────────

/// Legacy context handle allowing embedded `board` views to open the workspace whiteboard.
#[derive(Clone)]
pub struct OpenBoardCallback(pub Rc<dyn Fn()>);

/// Block-aware context handle passing `(block_index, board_json)` when opening an
/// embedded `board` block for 3-way merge editing in the full whiteboard modal.
#[derive(Clone)]
pub struct OpenBoardWithBlockCallback(pub Rc<dyn Fn(usize, String)>);

/// Preset actions offered by a code block's floating "Ask AI" menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeAiAction {
  Explain,
  Improve,
  Fix,
  Custom,
}

/// Optional context handle invoked when the user triggers a code-block AI action.
/// Receives `(action, block_index, code_source)`.
#[derive(Clone)]
pub struct CodeAiCallback(pub Rc<dyn Fn(CodeAiAction, usize, String)>);

/// Ambient callback invoked when the user clicks an internal `mica://page/<uuid>`
/// link inside the editor or a table cell.
#[derive(Clone)]
pub struct OpenMicaPageCallback(pub Rc<dyn Fn(uuid::Uuid)>);

/// Validated target for an inline hyperlink click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
  MicaPage(uuid::Uuid),
  ExternalUrl(String),
}

/// Validates and classifies a raw hyperlink target (`https://`, `http://`,
/// `mailto:`, `tel:`, `www.*`, or internal `mica://page/<uuid>`). Rejects
/// empty strings and unsafe schemes such as `javascript:` or `data:`.
pub fn resolve_link_target(raw: &str) -> Option<LinkTarget> {
  let trimmed = raw.trim();
  if trimmed.is_empty() {
    return None;
  }
  let lower = trimmed.to_ascii_lowercase();
  if lower.starts_with("mica://page/") {
    let rest = &trimmed["mica://page/".len()..];
    let id_part = rest.split(['?', '#', '/']).next().unwrap_or("");
    return uuid::Uuid::parse_str(id_part).ok().map(LinkTarget::MicaPage);
  }
  if lower.starts_with("https://")
    || lower.starts_with("http://")
    || lower.starts_with("mailto:")
    || lower.starts_with("tel:")
  {
    return Some(LinkTarget::ExternalUrl(trimmed.to_owned()));
  }
  if lower.starts_with("www.") {
    return Some(LinkTarget::ExternalUrl(format!("https://{trimmed}")));
  }
  None
}

/// Opens a validated inline hyperlink: internal `mica://page/<uuid>` links jump
/// directly within Mica when `open_page` is provided, while safe external URLs
/// launch via `nsh_core::open_url`.
pub(crate) fn open_editor_link(
  raw_href: &str,
  open_page: Option<&OpenMicaPageCallback>,
) -> bool {
  match resolve_link_target(raw_href) {
    Some(LinkTarget::MicaPage(id)) => {
      if let Some(cb) = open_page {
        (cb.0)(id);
        true
      } else {
        false
      }
    }
    Some(LinkTarget::ExternalUrl(url)) => {
      drop(open_url(url));
      true
    }
    None => false,
  }
}

/// Pixel-exact hit test for inline `link` marks at text-local `(lx, ly)`:
/// returns the resolved raw href only when the pointer lands inside the visual
/// bounding box of a link mark (so clicking blank margin space past the end of
/// a link places the caret normally without opening the URL).
#[allow(clippy::too_many_arguments)]
pub(crate) fn hit_link_in_lines(
  lines: &[nsh_core::TextLine],
  text: &str,
  marks: &[EditorMark],
  lx: f32,
  ly: f32,
  font: &str,
  line_h: f32,
  measure: &impl Fn(&str, &str) -> Points,
) -> Option<String> {
  if marks.is_empty() || lx < -2.0 || ly < -2.0 {
    return None;
  }
  for m in marks {
    if m.kind != "link" || m.end <= m.start {
      continue;
    }
    let rects = selection_rects_for_range(lines, m.start, m.end, font, line_h, measure);
    let inside = rects.into_iter().any(|(rx, ry, rw, rh)| {
      lx >= rx - 1.0 && lx <= rx + rw + 2.0 && ly >= ry - 1.0 && ly <= ry + rh + 1.0
    });
    if !inside {
      continue;
    }
    let raw = m
      .href
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .unwrap_or_else(|| utf16_slice(text, m.start, m.end).trim());
    if resolve_link_target(raw).is_some() {
      return Some(raw.to_owned());
    }
  }
  None
}

/// Content shown inside the fullscreen pan/zoom lightbox modal.
#[derive(Clone, Debug, PartialEq)]
pub enum LightboxContent {
  Mermaid { block_index: usize, source: String },
  Image { url: String, alt: String, natural_size: Option<(f32, f32)> },
}

/// Rectangular multi-cell selection inside a table block: `(block_id, anchor, focus)`.
pub type TableCellSelection = (String, (usize, usize), (usize, usize));

/// Shared reactive environment passed into every recycled `virtual_list` block cell.
#[derive(Clone)]
pub struct BlockRenderEnv {
  pub bridge: Rc<MicaDocumentBridge>,
  pub dark_mode: Signal<bool>,
  pub layouts: Rc<RefCell<VisibleLayoutCache>>,
  pub drag_anchor: Signal<Option<DocPosition>>,
  pub lightbox: Signal<Option<LightboxContent>>,
  pub image_ctx: crate::image_blocks::ImageContext,
  pub focused_code_block: Signal<Option<String>>,
  pub focused_table_cell: Signal<Option<(String, usize, usize)>>,
  pub table_selection: Signal<Option<TableCellSelection>>,
  pub table_drag_anchor: Signal<Option<(String, usize, usize)>>,
  pub body_blank_marquee_ready: Signal<bool>,
  pub marquee_origin_page: Signal<Option<(f32, f32)>>,
  pub last_click: Signal<(usize, u32, f64, u8)>,
  pub page_width: Signal<f32>,
  pub editor_focused: Signal<bool>,
  pub caret_blink_on: Signal<bool>,
  pub sink_clear_rev: Signal<u64>,
  pub marquee_selected: Signal<Option<(usize, usize)>>,
  pub inline_math_edit: Signal<Option<crate::editor_positioning::InlineMathEdit>>,
  /// Records `(page_x, page_y)` when an inner `text_box` / `code_text_box`
  /// already handled the current `on_mouse_move` so the enclosing full-width
  /// row handler never overwrites the exact character hit with approximate
  /// margin coordinates.
  pub inner_drag_hit: Rc<Cell<Option<(f32, f32)>>>,
  /// `true` when the latest pointer-down originated from touch or stylus.
  pub last_pointer_is_touch: Signal<bool>,
  /// `true` when the active selection was started via touch (long-press /
  /// touch double-tap), displaying teardrop handles and compact action bar.
  pub touch_selection_active: Signal<bool>,
  /// Multi-tap counter for touch pointers (Web `nsh.js` reports `clicks = 1`
  /// for touch events).
  pub touch_tap_tracker: Rc<RefCell<crate::touch_selection::TouchTapTracker>>,
}

fn is_non_prose_block(block: &EditorBlock) -> bool {
  crate::touch_selection::is_atomic_selection_kind(effective_block_kind(block))
}

/// Computes the `(text_left, text_top)` origin of `block`'s text container in
/// row-local coordinates when dragging across margins, list prefixes, quote
/// borders, or code-block card padding where the inner text box did not fire.
pub(crate) fn row_text_origin_offset(
  block: &EditorBlock,
  content_left: f32,
  text_frame_origin: (f32, f32),
) -> (f32, f32) {
  let kind = effective_block_kind(block);
  let li_inset = list_continuation_inset_px(block);
  let indent = block.data.get("indent").and_then(Value::as_u64).unwrap_or(0).min(8) as f32;
  match kind {
    "code_block" => {
      let line_numbers = block.data.get("lineNumbers").and_then(Value::as_bool).unwrap_or(false);
      let ln_w = if line_numbers { 28.0 } else { 0.0 };
      let has_title = block
        .data
        .get("title")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
      let title_h = if has_title { 22.0 } else { 0.0 };
      // `node-view-code-block` (`my_2` = 8px) + card (`px(14)`, `py(12)`) + optional line numbers/title
      (content_left + li_inset + 14.0 + ln_w, 20.0 + title_h)
    }
    "quote" => {
      let has_callout = block
        .data
        .get("alert")
        .and_then(Value::as_str)
        .and_then(CalloutKind::from_attr)
        .is_some();
      let top_extra = if has_callout { 36.0 } else { 8.0 };
      (content_left + li_inset + 17.0, top_extra)
    }
    "bulleted_list" | "todo" => (content_left + indent * 24.0 + 26.0, text_frame_origin.1.max(2.0)),
    "numbered_list" => (content_left + indent * 24.0 + 28.0, text_frame_origin.1.max(2.0)),
    _ => (
      content_left + li_inset + indent * 24.0 + text_frame_origin.0,
      text_frame_origin.1,
    ),
  }
}

#[inline]
pub(crate) fn live_index(at: Position) -> usize {
  nsh_core::element::untracked(|| at.index())
}

// ── Virtual List Row Entry Point ─────────────────────────────────────────────

/// Renders one recycled `virtual_list` row cell for block position `at`.
/// Subscribes ONLY to `cell_block_id` (short-circuits when a block merely shifts
/// index on Enter/Backspace!) and that single block's `block_rev` signal.
pub fn render_virtual_block_row(env: BlockRenderEnv, at: Position) -> impl IntoElement {
  let bridge = env.bridge.clone();
  let bridge_id = env.bridge.clone();
  let bridge_drag = env.bridge.clone();
  let bridge_sel = env.bridge.clone();
  let layouts_reg = env.layouts.clone();
  let layouts_drag = env.layouts.clone();
  let drag_anchor = env.drag_anchor;
  let table_drag_guard = env.table_drag_anchor;
  let blank_marquee_guard = env.body_blank_marquee_ready;
  let marquee_origin_page_row = env.marquee_origin_page;
  let br_row_clear = env.bridge.clone();
  let focused_code_row = env.focused_code_block;
  let focused_cell_row = env.focused_table_cell;
  let table_sel_row = env.table_selection;
  let page_width = env.page_width;
  let dark_mode = env.dark_mode;
  let marquee_selected = env.marquee_selected;
  let inner_drag_hit_row = env.inner_drag_hit.clone();
  let touch_sel_row = env.touch_selection_active;
  let structure_rev = bridge.structure_rev_signal();
  let row_ref = element_ref();
  let row_width = signal(960.0_f32);
  let drag_measure: TextMeasureFn = Rc::new(text_measurer());

  // Memoized cell identity: NEVER runs on typing (`structure_rev` stays flat);
  // on Enter/Backspace/splice, unchanged rows that merely shift index keep the
  // exact same `block_id`, so `cell_block_id` short-circuits and NEVER rebuilds
  // sibling Mermaid diagrams, tables, code blocks, or paragraphs!
  let cell_block_id = computed(move || {
    structure_rev.get();
    bridge_id.block_id_at(at.index())
  });

  // Lightweight zero-DOM index sync so `layouts.row_refs` stays current across
  // Enter/Backspace shifts without triggering any element tree rebuild.
  {
    let layouts_sync = layouts_reg.clone();
    effect(move || {
      let idx = at.index();
      if let Some(bid) = cell_block_id.get() {
        layouts_sync.borrow_mut().register_row_ref(&bid, idx, row_ref);
      }
    });
  }

  // Memoized boolean gate for the non-prose block selection highlight overlay.
  // Short-circuits to `false` on every collapsed caret move, Enter, or Backspace.
  let show_block_overlay = computed(move || {
    let idx = at.index();
    let in_marquee = marquee_selected
      .get()
      .is_some_and(|(lo, hi)| idx >= lo && idx <= hi);
    if in_marquee {
      return true;
    }
    if let Some(sel) = bridge_sel.selection_signal().get() {
      !sel.is_collapsed()
        && idx >= sel.start_pos().node
        && idx <= sel.end_pos().node
        && bridge_sel
          .block_at(idx)
          .is_some_and(|b| is_non_prose_block(&b))
    } else {
      false
    }
  });

  view()
    .element_ref(&row_ref)
    .w_full()
    .items_center()
    .on_layout(move |b: &Bounds<Points>| {
      if (row_width.get_untracked() - b.size.width.0).abs() > 0.5 {
        row_width.set(b.size.width.0);
      }
    })
    // Pressing in the left/right blank margins (outside the prose text area)
    // arms block marquee drag; presses inside the prose column itself always
    // stay in normal text/line selection mode.
    .on_press_down(move |e: &PressEvent| {
      br_row_clear.clear_selection();
      marquee_selected.set(None);
      drag_anchor.set(None);
      touch_sel_row.set(false);
      focused_code_row.set(None);
      focused_cell_row.set(None);
      table_sel_row.set(None);
      table_drag_guard.set(None);
      let rw = row_width.get_untracked();
      let eff = page_width.get_untracked();
      let col_w = if crate::page_width::is_full_width(eff) {
        rw
      } else {
        rw.min(eff)
      };
      let col_left = ((rw - col_w) * 0.5).max(0.0);
      let col_right = col_left + col_w;
      let in_side_margin = crate::block_marquee::starts_marquee_in_margin(
        e.position.x.0,
        0.0,
        rw,
        col_left,
        col_right,
        f32::INFINITY,
      );
      blank_marquee_guard.set(in_side_margin);
      marquee_origin_page_row.set(
        in_side_margin.then_some((e.page_position.x.0, e.page_position.y.0)),
      );
    })
    // Unified full-width cross-block drag handler: covers left/right margins AND
    // every block kind (prose, tables, dividers, code blocks, images, boards,
    // Mermaid, math) so upward/downward drag selection NEVER stalls!
    .on_mouse_move(move |e: &MouseEvent| {
      let page_xy = (e.page_position.x.0, e.page_position.y.0);
      if inner_drag_hit_row.take() == Some(page_xy) {
        // Inner `text_box` / `code_text_box` already resolved the exact glyph
        // for this pointer move; never overwrite it with outer margin math.
        return;
      }
      if e.pressed_button != Some(MouseButton::Left) || blank_marquee_guard.get_untracked() {
        return;
      }
      let idx = at.index();
      let Some(block) = bridge_drag.block_at(idx) else {
        return;
      };
      // Seamless table-to-document drag upgrade: when dragging started inside a
      // table cell and the pointer leaves the table upward or downward into a
      // neighboring block, smoothly upgrade to cross-block document selection
      // anchored on the table so selection NEVER stalls at the table boundary!
      if let Some((ref t_bid, _, _)) = table_drag_guard.get_untracked() {
        if t_bid == &block.id {
          return;
        }
        let Some(table_idx) = bridge_drag.index_of_block_id(t_bid) else {
          table_drag_guard.set(None);
          return;
        };
        let table_end = bridge_drag
          .block_at(table_idx)
          .map(|b| utf16_len(&b.text))
          .unwrap_or(0);
        let upgraded_anchor = if idx < table_idx {
          DocPosition::new(table_idx, table_end)
        } else {
          DocPosition::new(table_idx, 0)
        };
        table_drag_guard.set(None);
        table_sel_row.set(None);
        focused_cell_row.set(None);
        drag_anchor.set(Some(upgraded_anchor));
      }
      let Some(anchor) = drag_anchor.get_untracked() else {
        return;
      };
      let focus_pos = if is_non_prose_block(&block) {
        if idx < anchor.node {
          DocPosition::new(idx, 0)
        } else if idx > anchor.node {
          DocPosition::new(idx, utf16_len(&block.text))
        } else {
          anchor
        }
      } else {
        let rw = row_width.get_untracked();
        let eff = page_width.get_untracked();
        let col_w = if crate::page_width::is_full_width(eff) {
          rw
        } else {
          rw.min(eff)
        };
        let col_left = ((rw - col_w) * 0.5).max(0.0);
        let content_left = col_left + 24.0;
        let content_right = (col_left + col_w - 24.0).max(content_left + 1.0);
        let block_u16_len = utf16_len(&block.text);
        let off = {
          let cache = layouts_drag.borrow();
          let info_opt = cache.blocks.get(&block.id);
          let text_frame_origin = info_opt
            .map(|i| (i.text_frame.origin.x.0, i.text_frame.origin.y.0))
            .unwrap_or((0.0, 0.0));
          let (text_left, text_top) =
            row_text_origin_offset(&block, content_left, text_frame_origin);
          let fallback_w = (content_right - text_left).max(160.0);
          let (_, fallback_lh, _, fallback_font) = block_typography(
            &block.kind,
            block.data.get("level").and_then(Value::as_u64).unwrap_or(1),
            16.0,
            "",
          );
          let eff_font = info_opt
            .map(|i| i.font.as_str())
            .filter(|f| !f.is_empty())
            .unwrap_or(fallback_font.as_str());
          let eff_lh = info_opt
            .map(|i| i.line_height)
            .filter(|&h| h > 1.0)
            .unwrap_or(fallback_lh);
          let lines = resolve_lines_for_block(
            info_opt,
            &block.text,
            fallback_w,
            eff_font,
            eff_lh,
            &|f, t| drag_measure(f, t),
          );
          let local_x = e.position.x.0 - text_left;
          let local_y = (e.position.y.0 - text_top).max(0.0);
          if e.position.x.0 <= text_left {
            offset_at_point(&lines, 0.0, local_y, eff_font, &|f, t| drag_measure(f, t))
              .min(block_u16_len)
          } else if e.position.x.0 >= content_right {
            offset_at_point(&lines, f32::MAX * 0.25, local_y, eff_font, &|f, t| {
              drag_measure(f, t)
            })
            .min(block_u16_len)
          } else {
            offset_at_point(&lines, local_x, local_y, eff_font, &|f, t| drag_measure(f, t))
              .min(block_u16_len)
          }
        };
        DocPosition::new(idx, off)
      };
      let next = DocSelection::from_positions(anchor, focus_pos);
      if bridge_drag.selection().as_ref() != Some(&next) {
        bridge_drag.set_selection(next);
      }
    })
    .child(
      view()
        .w_full()
        .max_w(move || {
          let eff = page_width.get();
          if crate::page_width::is_full_width(eff) {
            Length::from(relative(1.0))
          } else {
            Length::from(px(eff))
          }
        })
        .px_6()
        .relative()
        .child({
          // Keep `code_block` mounted across in-block typing / Enter / Backspace so
          // rapid Backspace repeats never tear down and rebuild the whole code card,
          // toolbar, overlays, or `text_ref`. Other block kinds keep their exact
          // per-block `rev_sig` rebuild path; kind or `li` changes still switch cleanly.
          let br_id_check = bridge.clone();
          let cell_render_key = computed(move || {
            let block_id = cell_block_id.get()?;
            let rev_sig = br_id_check.block_rev_signal(&block_id);
            let rev = rev_sig.get();
            let block = br_id_check.block_by_id(&block_id)?;
            let is_code = effective_block_kind(&block) == "code_block";
            let li_bits = list_continuation_inset_px(&block).to_bits();
            Some((block_id, is_code, li_bits, if is_code { 0 } else { rev }))
          });
          dynamic(move || {
            let Some((block_id, is_code, _, _)) = cell_render_key.get() else {
              return view().h(px(1.0)).into_element();
            };
            let idx_now = live_index(at);
            layouts_reg.borrow_mut().register_row_ref(&block_id, idx_now, row_ref);
            if is_code {
              let rev_sig = bridge.block_rev_signal(&block_id);
              let li_inset = bridge
                .block_by_id(&block_id)
                .map(|b| list_continuation_inset_px(&b))
                .unwrap_or(0.0);
              let inner = render_code_block_view(&env, at, block_id, rev_sig).into_element();
              if li_inset > 0.0 {
                view()
                  .test_id("list-nested-block")
                  .w_full()
                  .pl(px(li_inset))
                  .child(inner)
                  .into_element()
              } else {
                inner
              }
            } else {
              let Some(block) = bridge.block_by_id(&block_id) else {
                return view().h(px(1.0)).into_element();
              };
              render_block_body(&env, at, block)
            }
          })
        })
        // Scoped non-interactive selection highlight for marquee block selection
        // AND cross-block range selections spanning non-prose blocks (tables,
        // dividers, code blocks, images, boards, Mermaid, math).
        .child(when(move || show_block_overlay.get(), move || {
          view()
            .test_id("block-selection-overlay")
            .absolute()
            .left(px(20.0))
            .right(px(20.0))
            .top(px(2.5))
            .bottom(px(2.5))
            .rounded(px(4.0))
            .bg(move || palette(dark_mode.get()).selection_bg)
            .pointer_events(PointerEvents::None)
        })),
    )
}

/// Left inset in pixels for a non-list block attached as a child of a list item
/// (`data.li = level`), aligning its left edge with the owning list item's text
/// column (matches `origin/main` Flutter `26.0 + 24.0 * li`).
pub(crate) fn list_continuation_inset_px(block: &EditorBlock) -> f32 {
  block
    .li_level()
    .map(|lv| 26.0 + 24.0 * lv as f32)
    .unwrap_or(0.0)
}

fn render_block_body(env: &BlockRenderEnv, at: Position, block: EditorBlock) -> AnyElement {
  let li_inset = list_continuation_inset_px(&block);
  let inner = match effective_block_kind(&block) {
    "code_block" => {
      let rev_sig = env.bridge.block_rev_signal(&block.id);
      render_code_block_view(env, at, block.id, rev_sig).into_element()
    }
    "board" => render_board_block_view(env, at, block).into_element(),
    "mermaid" => render_mermaid_block_view(env, at, block).into_element(),
    "math" | "math_block" => render_math_block_view(env, at, block).into_element(),
    "image" => render_image_block_view(env, at, block).into_element(),
    "table" => crate::table_view::render_table_block_view(env, at, block).into_element(),
    "divider" => render_divider_block_view(env, at).into_element(),
    _ => render_prose_block_view(env, at, block).into_element(),
  };
  if li_inset > 0.0 {
    view()
      .test_id("list-nested-block")
      .w_full()
      .pl(px(li_inset))
      .child(inner)
      .into_element()
  } else {
    inner
  }
}

// ── 1. Prose Blocks (paragraph / heading / lists / todo / single-layer quote) ─

fn render_prose_block_view(env: &BlockRenderEnv, at: Position, block: EditorBlock) -> impl IntoElement {
  let dark_mode = env.dark_mode;
  let bridge = env.bridge.clone();
  let layouts = env.layouts.clone();
  let block_id = block.id.clone();
  let kind = block.kind.clone();
  let init_idx = live_index(at);
  let indent = block.data.get("indent").and_then(Value::as_u64).unwrap_or(0).min(8) as f32;
  let heading_level = if kind == "heading" {
    block.data.get("level").and_then(Value::as_u64).unwrap_or(1).clamp(1, 6)
  } else {
    0
  };
  let callout = if kind == "quote" {
    block.data.get("alert").and_then(Value::as_str).and_then(CalloutKind::from_attr)
  } else {
    None
  };
  let todo_checked = block.data.get("checked").and_then(Value::as_bool).unwrap_or(false);

  let (base_px, font_family) = use_context::<crate::mica_theme::UiAppearance>()
    .map(|a| (a.editor_font_px.get() as f32, a.editor_font_family.get()))
    .unwrap_or((16.0, ""));
  let (font_px, line_h, weight_num, css_font) = block_typography(&kind, heading_level, base_px, font_family);
  let font_weight = match weight_num {
    600..=900 => FontWeight::SEMIBOLD,
    500..=599 => FontWeight::MEDIUM,
    _ => FontWeight::NORMAL,
  };

  let (raw_mt, mb) = match kind.as_str() {
    "heading" => match heading_level {
      1 => (18.0, 6.0),
      2 => (14.0, 4.0),
      3 => (10.0, 4.0),
      _ => (8.0, 2.0),
    },
    "quote" => (6.0, 6.0),
    "bulleted_list" | "numbered_list" | "todo" => (2.0, 2.0),
    _ => (3.0, 4.0),
  };

  let marks = block_marks(&block);
  let text_str = block.text.clone();
  let text_ref = element_ref();
  {
    let mut cache = layouts.borrow_mut();
    cache.register_text_ref(&block_id, text_ref);
    cache.update_typography(&block_id, &css_font, line_h);
  }

  // Text container with exact `EventKind::TextLayout` + scoped selection/caret overlay
  let text_box = {
    let layouts_l = layouts.clone();
    let layouts_f = layouts.clone();
    let layouts_sel = layouts.clone();
    let layouts_click = layouts.clone();
    let layouts_move = layouts.clone();
    let bid_l = block_id.clone();
    let bid_f = block_id.clone();
    let bid_sel = block_id.clone();
    let bid_click = block_id.clone();
    let bid_move = block_id.clone();
    let font_l = css_font.clone();
    let font_sel = css_font.clone();
    let font_click = css_font.clone();
    let font_move = css_font.clone();
    let text_click = text_str.clone();
    let text_spans = text_str.clone();
    let br_click = bridge.clone();
    let br_move = bridge.clone();
    let br_sel = bridge.clone();
    let br_find = bridge.clone();
    let preedit_overlay_sig = bridge.ime_preedit_signal();
    let drag_anchor_d = env.drag_anchor;
    let drag_anchor_m = env.drag_anchor;
    let last_click = env.last_click;
    let focused_code = env.focused_code_block;
    let focused_cell = env.focused_table_cell;
    let table_sel = env.table_selection;
    let table_drag = env.table_drag_anchor;
    let marquee_sel_click = env.marquee_selected;
    let blank_marquee_ready = env.body_blank_marquee_ready;
    let marquee_origin_page = env.marquee_origin_page;
    let editor_focused = env.editor_focused;
    let caret_blink_on = env.caret_blink_on;
    let sink_clear_rev = env.sink_clear_rev;
    let inline_math_edit = env.inline_math_edit;
    let inner_drag_hit_m = env.inner_drag_hit.clone();
    let last_ptr_touch_down = env.last_pointer_is_touch;
    let last_ptr_touch_lp = env.last_pointer_is_touch;
    let touch_sel_down = env.touch_selection_active;
    let touch_sel_lp = env.touch_selection_active;
    let tap_tracker_down = env.touch_tap_tracker.clone();
    let layouts_lp = layouts.clone();
    let bid_lp = block_id.clone();
    let font_lp = css_font.clone();
    let text_lp = text_str.clone();
    let br_lp = bridge.clone();
    let drag_anchor_lp = env.drag_anchor;
    let focused_code_lp = env.focused_code_block;
    let focused_cell_lp = env.focused_table_cell;
    let table_sel_lp = env.table_selection;
    let table_drag_lp = env.table_drag_anchor;
    let marquee_sel_lp = env.marquee_selected;
    let blank_marquee_lp = env.body_blank_marquee_ready;
    let marquee_origin_lp = env.marquee_origin_page;
    let marks_click = marks.clone();
    let marks_move = marks.clone();
    let has_links = marks.iter().any(|m| m.kind == "link" && m.end > m.start);
    let open_page_cb = use_context::<OpenMicaPageCallback>();
    let hovering_link = signal(false);
    let pending_link_click: Rc<RefCell<Option<(String, f32, f32)>>> = Rc::new(RefCell::new(None));
    let pending_link_down = pending_link_click.clone();
    let pending_link_move = pending_link_click.clone();
    let pending_link_press = pending_link_click;
    let br_link_press = bridge.clone();
    let layout_rev = signal(0u64);
    let kind_color = kind.clone();
    let measure: TextMeasureFn = Rc::new(text_measurer());
    let measure_sel = measure.clone();
    let measure_click = measure.clone();
    let measure_lp = measure.clone();
    let measure_move = measure;

    let hit_page_w = env.page_width;
    let sel_text = text_str.clone();
    let sel_page_w = env.page_width;
    let text_move = text_str.clone();
    let hit_offset = move |layouts_ref: &Rc<RefCell<VisibleLayoutCache>>,
                           bid: &str,
                           block_text: &str,
                           font: &str,
                           meas: &TextMeasureFn,
                           lx: f32,
                           ly: f32|
          -> u32 {
      let cache = layouts_ref.borrow();
      let eff = hit_page_w.get_untracked();
      let fallback_w = if crate::page_width::is_full_width(eff) {
        820.0
      } else {
        (eff - 48.0).max(200.0)
      };
      let lines = resolve_lines_for_block(
        cache.blocks.get(bid),
        block_text,
        fallback_w,
        font,
        line_h,
        &|f, t| meas(f, t),
      );
      offset_at_point(&lines, lx, ly, font, &|f, t| meas(f, t))
    };
    let hit_link = move |layouts_ref: &Rc<RefCell<VisibleLayoutCache>>,
                         bid: &str,
                         block_text: &str,
                         block_marks: &[EditorMark],
                         font: &str,
                         meas: &TextMeasureFn,
                         lx: f32,
                         ly: f32|
          -> Option<String> {
      let cache = layouts_ref.borrow();
      let eff = hit_page_w.get_untracked();
      let fallback_w = if crate::page_width::is_full_width(eff) {
        820.0
      } else {
        (eff - 48.0).max(200.0)
      };
      let lines = resolve_lines_for_block(
        cache.blocks.get(bid),
        block_text,
        fallback_w,
        font,
        line_h,
        &|f, t| meas(f, t),
      );
      hit_link_in_lines(
        &lines,
        block_text,
        block_marks,
        lx,
        ly,
        font,
        line_h,
        &|f, t| meas(f, t),
      )
    };

    view()
      .element_ref(&text_ref)
      .test_id(&format!("block-text-{init_idx}"))
      .flex_1()
      .min_w(px(0.0))
      .min_h(px(line_h))
      .relative()
      .cursor(Cursor::Text)
      .style_when(hovering_link, |s| s.cursor(Cursor::Pointer))
      .on_layout(move |b: &Bounds<Points>| {
        if layouts_f.borrow_mut().update_text_frame(&bid_f, *b) {
          layout_rev.update(|v| *v = v.wrapping_add(1));
        }
      })
      .on_press_down(move |e: &PressEvent| {
        *pending_link_down.borrow_mut() = None;
        focused_code.set(None);
        focused_cell.set(None);
        table_sel.set(None);
        table_drag.set(None);
        marquee_sel_click.set(None);
        sink_clear_rev.update(|v| *v = v.wrapping_add(1));

        // Pressing inside any prose row always performs normal text/line
        // selection (never hijacked into a block marquee box — marquee is
        // reserved for the left/right margins and bottom blank area).
        blank_marquee_ready.set(false);
        marquee_origin_page.set(None);

        let off = hit_offset(&layouts_click, &bid_click, &text_click, &font_click, &measure_click, e.position.x.0, e.position.y.0);
        let is_touch = last_ptr_touch_down.get_untracked();
        let click_count = if is_touch {
          tap_tracker_down.borrow_mut().bump(
            e.page_position.x.0,
            e.page_position.y.0,
            crate::touch_selection::now_ms(),
          )
        } else {
          tap_tracker_down.borrow_mut().reset();
          e.clicks.clamp(1, 3)
        };
        let _ = last_click;
        let idx = live_index(at);

        if click_count == 3 {
          inline_math_edit.set(None);
          let end_u16 = utf16_len(&text_click);
          let sel = DocSelection::from_positions(DocPosition::new(idx, 0), DocPosition::new(idx, end_u16));
          drag_anchor_d.set(if is_touch { None } else { Some(sel.anchor_pos()) });
          touch_sel_down.set(is_touch && !sel.is_collapsed());
          br_click.set_selection(sel);
        } else if click_count == 2 {
          inline_math_edit.set(None);
          let (ws, we) = crate::touch_selection::touch_word_bounds(&text_click, off);
          let sel = DocSelection::from_positions(DocPosition::new(idx, ws), DocPosition::new(idx, we));
          drag_anchor_d.set(if is_touch { None } else { Some(sel.anchor_pos()) });
          touch_sel_down.set(is_touch && !sel.is_collapsed());
          br_click.set_selection(sel);
        } else {
          // Arm link navigation on a clean single click (Alt+Click places caret
          // inside the link for typing without opening; dragging cancels it).
          if has_links
            && !e.modifiers.alt
            && let Some(href) = hit_link(
              &layouts_click,
              &bid_click,
              &text_click,
              &marks_click,
              &font_click,
              &measure_click,
              e.position.x.0,
              e.position.y.0,
            )
          {
            *pending_link_down.borrow_mut() =
              Some((href, e.page_position.x.0, e.page_position.y.0));
          }
          if let Some(m) = marks_click.iter().find(|m| {
            m.kind == "math"
              && m.end > m.start
              && ((off > m.start && off < m.end) || (m.end - m.start == 1 && off == m.start))
          }) {
            inline_math_edit.set(Some(crate::editor_positioning::InlineMathEdit {
              block_index: idx,
              start: m.start,
              end: m.end,
              draft: utf16_slice(&text_click, m.start, m.end).to_owned(),
            }));
          } else {
            inline_math_edit.set(None);
          }
          let pos = DocPosition::new(idx, off);
          drag_anchor_d.set(if is_touch { None } else { Some(pos) });
          touch_sel_down.set(false);
          br_click.set_selection(DocSelection::caret(idx, off));
        }
        br_click.request_focus();
      })
      .on_press(move || {
        let Some((href, _, _)) = pending_link_press.borrow_mut().take() else {
          return;
        };
        if br_link_press.selection().is_none_or(|s| s.is_collapsed()) {
          open_editor_link(&href, open_page_cb.as_ref());
        }
      })
      .on_long_press(move |e: &PressEvent| {
        // Mobile / touch long-press selects a 1-2 char CJK word or full Latin word
        // and raises teardrop drag handles (never hijacking desktop mouse drag-select).
        if !last_ptr_touch_lp.get_untracked() {
          return;
        }
        focused_code_lp.set(None);
        focused_cell_lp.set(None);
        table_sel_lp.set(None);
        table_drag_lp.set(None);
        marquee_sel_lp.set(None);
        blank_marquee_lp.set(false);
        marquee_origin_lp.set(None);
        drag_anchor_lp.set(None);

        let off = hit_offset(&layouts_lp, &bid_lp, &text_lp, &font_lp, &measure_lp, e.position.x.0, e.position.y.0);
        let idx = live_index(at);
        let (ws, we) = crate::touch_selection::touch_word_bounds(&text_lp, off);
        if we > ws {
          let sel = DocSelection::from_positions(DocPosition::new(idx, ws), DocPosition::new(idx, we));
          touch_sel_lp.set(true);
          br_lp.set_selection(sel);
        }
      })
      .on_mouse_move(move |e: &MouseEvent| {
        if e.pressed_button != Some(MouseButton::Left) {
          if has_links && e.pressed_button.is_none() {
            let over = hit_link(
              &layouts_move,
              &bid_move,
              &text_move,
              &marks_move,
              &font_move,
              &measure_move,
              e.position.x.0,
              e.position.y.0,
            )
            .is_some();
            if hovering_link.get_untracked() != over {
              hovering_link.set(over);
            }
          }
          return;
        }
        let moved_past_slop = pending_link_move
          .borrow()
          .as_ref()
          .is_some_and(|(_, px0, py0)| {
            (e.page_position.x.0 - *px0).hypot(e.page_position.y.0 - *py0) > 4.0
          });
        if moved_past_slop {
          *pending_link_move.borrow_mut() = None;
        }
        if blank_marquee_ready.get_untracked() {
          return;
        }
        let Some(anchor) = drag_anchor_m.get_untracked() else {
          return;
        };
        inner_drag_hit_m.set(Some((e.page_position.x.0, e.page_position.y.0)));
        let idx = live_index(at);
        let off = hit_offset(&layouts_move, &bid_move, &text_move, &font_move, &measure_move, e.position.x.0, e.position.y.0);
        let next = DocSelection::from_positions(anchor, DocPosition::new(idx, off));
        if br_move.selection().as_ref() != Some(&next) {
          br_move.set_selection(next);
        }
      })
      .on_mouse_leave(move |_: &MouseEvent| {
        if hovering_link.get_untracked() {
          hovering_link.set(false);
        }
      })
      // 1. Scoped selection highlight & primary caret layer (NEVER rebuilds text spans!
      // Memoized `local_overlay` short-circuits all ~25 non-active visible rows to `None`.)
      .child({
        let br_ov_sel = br_sel.clone();
        let br_ov_find = br_find.clone();
        let marquee_sel_ov = env.marquee_selected;
        let local_overlay = computed(move || {
          let idx = at.index();
          let in_marquee = marquee_sel_ov
            .get()
            .is_some_and(|(lo, hi)| idx >= lo && idx <= hi);
          let sel_relevant = if in_marquee {
            // Suppress inline text highlight when the row's full-width block
            // selection pill is active so the two layers never double-paint.
            None
          } else {
            br_ov_sel.selection_signal().get().and_then(|sel| {
              let start_p = sel.start_pos();
              let end_p = sel.end_pos();
              let focus_p = sel.focus_pos();
              let has_range = !sel.is_collapsed() && idx >= start_p.node && idx <= end_p.node;
              let has_caret = sel.is_collapsed() && focus_p.node == idx;
              if has_range || has_caret {
                let focused = if has_caret { editor_focused.get() } else { false };
                let preedit = if has_caret && focused {
                  preedit_overlay_sig.get().filter(|(bi, _, comp)| *bi == idx && !comp.is_empty())
                } else {
                  None
                };
                if has_range || focused {
                  Some((sel, focused, preedit))
                } else {
                  None
                }
              } else {
                None
              }
            })
          };
          let find_all = br_ov_find.find_matches_signal().get();
          let has_find = find_all.iter().any(|fm| fm.block_index == idx);
          let find_active = if has_find { br_ov_find.find_active_idx_signal().get() } else { 0 };
          if sel_relevant.is_none() && !has_find {
            None
          } else {
            Some((idx, layout_rev.get(), dark_mode.get(), sel_relevant, has_find, find_active))
          }
        });
        dynamic(move || {
          let Some((idx, _lrev, dark, sel_relevant, has_find, find_active)) = local_overlay.get() else {
            return view()
              .absolute()
              .inset_0()
              .pointer_events(PointerEvents::None);
          };
          let p = palette(dark);
          let mut cache = layouts_sel.borrow_mut();
          let eff = sel_page_w.get_untracked();
          let fallback_w = if crate::page_width::is_full_width(eff) {
            820.0
          } else {
            (eff - 48.0).max(200.0)
          };
          let lines = cache.resolve_lines_cached(
            &bid_sel,
            &sel_text,
            fallback_w,
            &font_sel,
            line_h,
            &|f, t| measure_sel(f, t),
          );

          let mut layer = view()
            .absolute()
            .inset_0()
            .pointer_events(PointerEvents::None);

          // Find match highlights in this block
          if has_find {
            let find_all = br_find.find_matches_signal().get_untracked();
            for (mi, fm) in find_all.iter().enumerate() {
              if fm.block_index != idx {
                continue;
              }
              let bg = if mi == find_active {
                Rgba { r: 0.96, g: 0.62, b: 0.04, a: 0.42 }
              } else {
                Rgba { r: 0.98, g: 0.80, b: 0.08, a: 0.28 }
              };
              for (rx, ry, rw, rh) in selection_rects_for_range(
                lines,
                fm.start,
                fm.end,
                &font_sel,
                line_h,
                &|f, t| measure_sel(f, t),
              ) {
                layer = layer.child(
                  view()
                    .absolute()
                    .left(px(rx))
                    .top(px(ry))
                    .w(px(rw))
                    .h(px(rh))
                    .rounded(px(2.0))
                    .bg(bg),
                );
              }
            }
          }

          // Selection range & blinking caret (hidden when unfocused; zero-rebuild blink via reactive bg)
          if let Some((sel, focused, active_preedit)) = sel_relevant {
            let start_p = sel.start_pos();
            let end_p = sel.end_pos();
            let focus_p = sel.focus_pos();
            if !sel.is_collapsed()
              && idx >= start_p.node
              && idx <= end_p.node
            {
              let b_len = br_sel
                .block_at(idx)
                .map(|b| utf16_len(&b.text))
                .unwrap_or(0);
              let s_u16 = if idx == start_p.node { start_p.offset } else { 0 };
              let e_u16 = if idx == end_p.node { end_p.offset } else { b_len.max(1) };
              for (rx, ry, rw, rh) in selection_rects_for_range(
                lines,
                s_u16,
                e_u16,
                &font_sel,
                line_h,
                &|f, t| measure_sel(f, t),
              ) {
                layer = layer.child(
                  view()
                    .absolute()
                    .left(px(rx))
                    .top(px(ry + 1.5))
                    .w(px(rw))
                    .h(px((rh - 3.0).max(14.0)))
                    .rounded(px(3.0))
                    .bg(p.selection_bg),
                );
              }
            }
            if sel.is_collapsed() && focus_p.node == idx && focused {
              let (cx, cy, ch) = caret_rect_for_offset(
                lines,
                focus_p.offset,
                &font_sel,
                line_h,
                &|f, t| measure_sel(f, t),
              );
              if let Some((_, _, comp)) = active_preedit {
                layer = layer.child(
                  view()
                    .test_id("mica-ime-preedit")
                    .absolute()
                    .left(px(cx))
                    .top(px(cy))
                    .h(px(ch.max(line_h)))
                    .px(px(2.0))
                    .rounded(px(2.0))
                    .bg(p.surface_raised)
                    .border_b_1()
                    .border_color(p.accent)
                    .justify_center()
                    .child(
                      text(comp)
                        .text_size(px(font_px))
                        .line_height(px(line_h))
                        .text_color(p.text_primary),
                    ),
                );
              } else {
                layer = layer.child(
                  view()
                    .test_id("mica-active-caret")
                  .absolute()
                  .left(px(cx))
                  .top(px(cy + 1.0))
                  .w(px(2.0))
                  .h(px((ch - 2.0).max(14.0)))
                  .rounded(px(1.0))
                  .bg(move || {
                    if caret_blink_on.get() {
                      palette(dark_mode.get()).accent
                    } else {
                      TRANSPARENT_RGBA
                    }
                  }),
              );
              }
            }
          }
          layer
        })
      })
      // 2. Native rich inline spans with `EventKind::TextLayout`
      .child({
        let base_color_kind = kind_color;
        let mut t = text("")
          .w_full()
          .text_size(px(font_px))
          .line_height(px(line_h))
          .font_weight(font_weight)
          .text_color(move || {
            let p = palette(dark_mode.get());
            if (base_color_kind == "quote" && callout.is_none())
              || (base_color_kind == "heading" && heading_level == 6)
            {
              p.text_muted
            } else {
              p.text_primary
            }
          })
          .on(EventKind::TextLayout, move |p: &EventPayload| {
            if let EventPayload::TextLayout(lines) = p
              && layouts_l
                .borrow_mut()
                .update_lines(&bid_l, lines.clone(), font_l.clone(), line_h)
            {
              layout_rev.update(|v| *v = v.wrapping_add(1));
            }
          });

        let runs = build_inline_runs(&text_spans, &marks, None);
        if runs.is_empty() {
          // Keep 1 line height even on blank paragraphs (no placeholder hint text)
          t = t.child(text("\u{200B}"));
        } else {
          for run in runs {
            t = t.child(render_inline_span(run, font_px, dark_mode));
          }
        }
        t
      })
  };

  // Optional block prefix gutter (bullet dot, ordered number, todo checkbox)
  let row_content: AnyElement = match kind.as_str() {
    "bulleted_list" => {
      let glyph = match (indent as usize) % 3 {
        0 => "•",
        1 => "◦",
        _ => "▪",
      };
      view()
        .w_full()
        .flex_row()
        .items_start()
        .gap(px(8.0))
        .child(
          view()
            .w(px(18.0))
            .h(px(line_h))
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .child(
              text(glyph)
                .text_size(px(font_px))
                .text_color(move || palette(dark_mode.get()).text_muted),
            ),
        )
        .child(text_box)
        .into_element()
    }
    "numbered_list" => {
      let br_ord = bridge.clone();
      let struct_rev_ord = bridge.structure_rev_signal();
      view()
        .w_full()
        .flex_row()
        .items_start()
        .gap(px(6.0))
        .child(
          view()
            .min_w(px(22.0))
            .h(px(line_h))
            .flex_shrink_0()
            .items_end()
            .justify_center()
            .child(
              text(move || {
                struct_rev_ord.get();
                format!("{}.", br_ord.visible_ordinal_at(at.index()))
              })
              .text_size(px((font_px - 1.0).max(12.0)))
              .text_color(move || palette(dark_mode.get()).text_muted),
            ),
        )
        .child(text_box)
        .into_element()
    }
    "todo" => {
      let br_todo = bridge.clone();
      view()
        .w_full()
        .flex_row()
        .items_start()
        .gap(px(8.0))
        .child(
          view()
            .h(px(line_h))
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .child(
              button()
                .test_id(&format!("todo-checkbox-{init_idx}"))
                .accessibility_label("切换待办完成状态")
                .w(px(16.0))
                .h(px(16.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(move || {
                  let p = palette(dark_mode.get());
                  if todo_checked { p.accent } else { p.border_strong }
                })
                .bg(move || {
                  let p = palette(dark_mode.get());
                  if todo_checked { p.accent } else { p.surface_base }
                })
                .items_center()
                .justify_center()
                .on_press(move || br_todo.toggle_todo_at(live_index(at)))
                .child(when(move || todo_checked, move || mica_icons::check_icon("#FFFFFF"))),
            ),
        )
        .child(text_box)
        .into_element()
    }
    "quote" => {
      // SINGLE-LAYER quote bar (3px left border + optional GFM callout wash & header).
      // Continuous vertical bar across consecutive `quote` blocks in the same
      // quote group (broken only when `data.qbreak == true` or indent/callout changes).
      let br_q_pt = bridge.clone();
      let br_q_pb = bridge.clone();
      let struct_rev_q = bridge.structure_rev_signal();
      let mut q = view()
        .test_id("node-view-quote")
        .w_full()
        .border_l(px(3.0))
        .border_color(move || {
          let dark = dark_mode.get();
          match callout {
            Some(c) => c.accent(dark).0,
            None => palette(dark).quote_bar,
          }
        })
        .bg(move || {
          let dark = dark_mode.get();
          match callout {
            Some(c) => Rgba { a: 0.06, ..c.accent(dark).0 },
            None => TRANSPARENT_RGBA,
          }
        })
        .rounded_r(px(if callout.is_some() { 6.0 } else { 0.0 }))
        .pl(px(14.0))
        .pr(px(if callout.is_some() { 12.0 } else { 0.0 }))
        .pt(move || {
          struct_rev_q.get();
          let (from_prev, _) = quote_edges_at(&br_q_pt, at.index());
          if callout.is_some() {
            px(8.0)
          } else if from_prev {
            px(5.0)
          } else {
            px(2.0)
          }
        })
        .pb(move || {
          struct_rev_q.get();
          let (_, to_next) = quote_edges_at(&br_q_pb, at.index());
          if to_next {
            px(5.0)
          } else if callout.is_some() {
            px(8.0)
          } else {
            px(2.0)
          }
        });

      if let Some(c) = callout {
        q = q.child(
          view()
            .test_id("quote-callout-header")
            .w_full()
            .h(px(22.0))
            .mb(px(4.0))
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(dynamic(move || {
              let (_, hex) = c.accent(dark_mode.get());
              match c {
                CalloutKind::Note => mica_icons::info_icon(hex),
                CalloutKind::Tip => icon_lightbulb(hex),
                CalloutKind::Important => icon_important_badge(hex),
                CalloutKind::Warning => icon_warning_triangle(hex),
                CalloutKind::Caution => icon_caution_octagon(hex),
              }
            }))
            .child(
              text(c.label())
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || c.accent(dark_mode.get()).0),
            ),
        );
      }
      q.child(text_box).into_element()
    }
    _ => text_box.into_element(),
  };

  let layouts_row = layouts;
  let bid_row = block_id;
  let is_quote_row = kind == "quote";
  let br_mt = bridge.clone();
  let br_mb = bridge.clone();
  let struct_rev_m = bridge.structure_rev_signal();
  view()
    .test_id(&format!("block-row-{init_idx}"))
    .w_full()
    .mt(move || {
      let idx = at.index();
      if idx == 0 {
        return px(2.0);
      }
      if is_quote_row {
        struct_rev_m.get();
        let (from_prev, _) = quote_edges_at(&br_mt, idx);
        return px(if from_prev { 0.0 } else { raw_mt });
      }
      px(raw_mt)
    })
    .mb(move || {
      if is_quote_row {
        struct_rev_m.get();
        let (_, to_next) = quote_edges_at(&br_mb, at.index());
        return px(if to_next { 0.0 } else { mb });
      }
      px(mb)
    })
    .pl(px(indent * 24.0))
    .on_layout(move |b: &Bounds<Points>| {
      layouts_row.borrow_mut().update_row_frame(&bid_row, *b);
    })
    .child(row_content)
}

/// Determine whether a `quote` block joins seamlessly with the previous/next
/// `quote` block in the same group (no `qbreak`, same indent/list nesting, and
/// not starting a separate callout). Matches `v0.14.11` `render.dart`.
pub(crate) fn quote_group_edges(
  prev: Option<&EditorBlock>,
  cur: &EditorBlock,
  next: Option<&EditorBlock>,
) -> (bool, bool) {
  if cur.kind != "quote" {
    return (false, false);
  }
  let indent_of = |b: &EditorBlock| b.data.get("indent").and_then(Value::as_u64).unwrap_or(0);
  let li_of = |b: &EditorBlock| b.data.get("li").and_then(Value::as_u64);
  let qbreak_of = |b: &EditorBlock| b.data.get("qbreak").and_then(Value::as_bool).unwrap_or(false);
  let has_callout = |b: &EditorBlock| {
    b.data
      .get("alert")
      .and_then(Value::as_str)
      .and_then(CalloutKind::from_attr)
      .is_some()
  };

  let cur_ind = indent_of(cur);
  let cur_li = li_of(cur);
  let from_prev = !qbreak_of(cur)
    && !has_callout(cur)
    && prev.is_some_and(|p| p.kind == "quote" && indent_of(p) == cur_ind && li_of(p) == cur_li);
  let to_next = next.is_some_and(|n| {
    n.kind == "quote"
      && !qbreak_of(n)
      && !has_callout(n)
      && indent_of(n) == cur_ind
      && li_of(n) == cur_li
  });
  (from_prev, to_next)
}

fn quote_edges_at(bridge: &MicaDocumentBridge, idx: usize) -> (bool, bool) {
  let Some(cur) = bridge.block_at(idx) else {
    return (false, false);
  };
  let prev = idx.checked_sub(1).and_then(|i| bridge.block_at(i));
  let next = bridge.block_at(idx + 1);
  quote_group_edges(prev.as_ref(), &cur, next.as_ref())
}

#[derive(Clone)]
pub(crate) struct InlineRun {
  text: String,
  bold: bool,
  italic: bool,
  strike: bool,
  highlight: bool,
  underline: bool,
  code: bool,
  link: bool,
  math: bool,
  preedit: bool,
}

pub(crate) fn build_inline_runs(
  text: &str,
  marks: &[EditorMark],
  preedit: Option<&(usize, u32, String)>,
) -> Vec<InlineRun> {
  let total = utf16_len(text) as u32;
  if total == 0 && preedit.is_none() {
    return Vec::new();
  }
  let mut cuts = vec![0u32, total];
  for m in marks {
    let s = m.start.min(total);
    let e = m.end.min(total);
    if s < e {
      cuts.push(s);
      cuts.push(e);
    }
  }
  if let Some((_, poff, _)) = preedit {
    cuts.push((*poff).min(total));
  }
  cuts.sort_unstable();
  cuts.dedup();

  let mut runs = Vec::new();
  for win in cuts.windows(2) {
    let (a, b) = (win[0], win[1]);
    if let Some((_, poff, ptxt)) = preedit
      && *poff == a
      && !ptxt.is_empty()
    {
      runs.push(InlineRun {
        text: ptxt.clone(),
        bold: false,
        italic: false,
        strike: false,
        highlight: false,
        underline: true,
        code: false,
        link: false,
        math: false,
        preedit: true,
      });
    }
    if a == b {
      continue;
    }
    let slice = utf16_slice(text, a, b);
    if slice.is_empty() {
      continue;
    }
    let mut r = InlineRun {
      text: slice.to_owned(),
      bold: false,
      italic: false,
      strike: false,
      highlight: false,
      underline: false,
      code: false,
      link: false,
      math: false,
      preedit: false,
    };
    for m in marks {
      if m.start <= a && m.end >= b {
        match m.kind.as_str() {
          "bold" => r.bold = true,
          "italic" => r.italic = true,
          "strike" => r.strike = true,
          "highlight" => r.highlight = true,
          "underline" => r.underline = true,
          "code" => r.code = true,
          "link" => r.link = true,
          "math" => r.math = true,
          _ => {}
        }
      }
    }
    runs.push(r);
  }
  if let Some((_, poff, ptxt)) = preedit
    && *poff >= total
    && !ptxt.is_empty()
  {
    runs.push(InlineRun {
      text: ptxt.clone(),
      bold: false,
      italic: false,
      strike: false,
      highlight: false,
      underline: true,
      code: false,
      link: false,
      math: false,
      preedit: true,
    });
  }
  runs
}

pub(crate) fn render_inline_span(run: InlineRun, font_px: f32, dark_mode: Signal<bool>) -> impl IntoElement {
  let mut t = text(run.text);
  if run.bold {
    t = t.font_weight(FontWeight::BOLD);
  }
  if run.italic {
    t = t.italic();
  }
  if run.strike {
    t = t.line_through();
  }
  if run.underline || run.link || run.preedit {
    t = t.underline();
  }
  if run.code {
    t = t
      .text_size(px((font_px - 1.5).max(12.0)))
      .text_color(move || palette(dark_mode.get()).inline_code_ink)
      .bg(move || palette(dark_mode.get()).inline_code_bg);
  } else if run.math {
    t = t
      .text_color(move || palette(dark_mode.get()).accent)
      .bg(move || palette(dark_mode.get()).accent_soft);
  } else {
    if run.link {
      t = t.text_color(move || palette(dark_mode.get()).accent);
    }
    if run.highlight {
      t = t.bg(move || palette(dark_mode.get()).highlight_bg);
    }
  }
  t
}

// ── 2. Code Block (Lazy Editor + `min_rows(1)` + Top-Level `overlay()` Menus) ─

const CODE_LANGUAGES: &[&str] = &[
  "auto", "plain", "bash", "c", "cpp", "csharp", "css", "dart", "diff", "dockerfile",
  "go", "graphql", "html", "ini", "java", "javascript", "json", "kotlin", "lua",
  "makefile", "markdown", "mermaid", "mica-board", "nginx", "php", "powershell",
  "python", "ruby", "rust", "scala", "shell", "sql", "swift", "toml", "typescript",
  "xml", "yaml",
];

/// Resolves `(highlight_lang, toolbar_badge_label)` for a code block:
/// - Explicit user choice (non-empty and not `"auto"`) is preserved verbatim.
/// - Empty / `"auto"` runs `mica_md::highlight::detect_code_language` on a bounded
///   prefix of `source`: whenever a concrete language (other than `"plaintext"`) is
///   detected, both syntax highlighting and the badge show that detected language
///   instead of `"auto"`.
pub fn resolve_code_block_language_display(raw_lang: &str, source: &str) -> (String, String) {
  let trimmed = raw_lang.trim();
  if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("auto") {
    return (trimmed.to_string(), trimmed.to_string());
  }
  let sample = if source.len() > 4096 {
    let mut cut = 4096;
    while cut > 0 && !source.is_char_boundary(cut) {
      cut -= 1;
    }
    &source[..cut]
  } else {
    source
  };
  let detected = mica_md::highlight::detect_code_language(sample);
  if !detected.is_empty() && detected != "plaintext" {
    (detected.clone(), detected)
  } else {
    ("plaintext".to_string(), "auto".to_string())
  }
}

fn render_code_block_view(
  env: &BlockRenderEnv,
  at: Position,
  block_id: String,
  rev_sig: Signal<u64>,
) -> impl IntoElement {
  let dark_mode = env.dark_mode;
  let bridge = env.bridge.clone();
  let layouts = env.layouts.clone();
  let focused_code = env.focused_code_block;
  let drag_anchor = env.drag_anchor;
  let marquee_selected = env.marquee_selected;
  let table_sel = env.table_selection;
  let table_drag = env.table_drag_anchor;
  let focused_cell = env.focused_table_cell;
  let blank_marquee_ready = env.body_blank_marquee_ready;
  let marquee_origin_page = env.marquee_origin_page;
  let editor_focused = env.editor_focused;
  let caret_blink_on = env.caret_blink_on;
  let sink_clear_rev = env.sink_clear_rev;
  let ai_cb = use_context::<CodeAiCallback>();

  let hovered = signal(false);
  let lang_open = signal(false);
  let lang_query = signal(String::new());
  let ai_open = signal(false);
  let more_open = signal(false);
  let copied = signal(false);
  let copy_gen = signal(0u64);
  let ai_notice: Signal<Option<String>> = signal(None);
  let ai_notice_gen = signal(0u64);
  let layout_rev = signal(0u64);

  let lang_btn_ref: ElementRef = element_ref();
  let ai_btn_ref: ElementRef = element_ref();
  let more_btn_ref: ElementRef = element_ref();
  let text_ref = element_ref();
  layouts.borrow_mut().register_text_ref(&block_id, text_ref);

  let br_snap = bridge.clone();
  let bid_snap = block_id.clone();
  let live_block = computed(move || {
    rev_sig.get();
    br_snap.block_by_id(&bid_snap)
  });
  let live_src = computed(move || {
    live_block.get().map(|b| b.text).unwrap_or_default()
  });
  let live_meta = computed(move || {
    live_block
      .get()
      .map(|b| {
        let raw_lang = b
          .data
          .get("language")
          .and_then(Value::as_str)
          .unwrap_or("")
          .trim()
          .to_string();
        let wrap = b.data.get("wrap").and_then(Value::as_bool).unwrap_or(false);
        let line_numbers = b.data.get("lineNumbers").and_then(Value::as_bool).unwrap_or(false);
        let title = b
          .data
          .get("title")
          .and_then(Value::as_str)
          .unwrap_or("")
          .to_string();
        (raw_lang, wrap, line_numbers, title)
      })
      .unwrap_or_else(|| (String::new(), false, false, String::new()))
  });
  let live_lang = computed(move || {
    let (raw_lang, _, _, _) = live_meta.get();
    let src = live_src.get();
    resolve_code_block_language_display(&raw_lang, &src)
  });
  let live_wrap = computed(move || live_meta.get().1);
  let live_line_numbers = computed(move || live_meta.get().2);
  let live_title = computed(move || live_meta.get().3);

  let title_editing = signal(false);
  let title_draft = signal(live_title.get_untracked());

  let br_caret_chk = bridge.clone();
  let is_caret_in_block = computed(move || {
    let idx = at.index();
    br_caret_chk
      .selection_signal()
      .get()
      .is_some_and(|s| s.focus_node() == idx)
  });

  let br_lang = bridge.clone();
  let br_wrap = bridge.clone();
  let br_ln = bridge.clone();
  let br_title = bridge.clone();
  let br_del = bridge.clone();

  let code_font = "14px monospace";
  let code_line_h = 22.0_f32;

  let code_text_box = {
    let layouts_f = layouts.clone();
    let layouts_l = layouts.clone();
    let layouts_sel = layouts.clone();
    let layouts_click = layouts.clone();
    let layouts_move = layouts.clone();
    let bid_f = block_id.clone();
    let bid_l = block_id.clone();
    let bid_sel = block_id.clone();
    let bid_click = block_id.clone();
    let bid_move = block_id.clone();
    let br_click = bridge.clone();
    let br_press = bridge.clone();
    let br_move = bridge.clone();
    let br_sel = bridge.clone();
    let br_find = bridge.clone();
    let preedit_overlay_sig = bridge.ime_preedit_signal();
    let measure: TextMeasureFn = Rc::new(text_measurer());
    let measure_sel = measure.clone();
    let measure_click = measure.clone();
    let measure_lp = measure.clone();
    let measure_move = measure;
    let inner_drag_hit_code = env.inner_drag_hit.clone();
    let last_ptr_touch_code = env.last_pointer_is_touch;
    let last_ptr_touch_code_lp = env.last_pointer_is_touch;
    let touch_sel_code = env.touch_selection_active;
    let touch_sel_code_lp = env.touch_selection_active;
    let tap_tracker_code = env.touch_tap_tracker.clone();
    let layouts_code_lp = layouts.clone();
    let bid_code_lp = block_id.clone();
    let br_code_lp = bridge.clone();

    let code_page_w = env.page_width;
    let code_sel_page_w = env.page_width;
    let hit_code_offset = move |layouts_ref: &Rc<RefCell<VisibleLayoutCache>>,
                                bid: &str,
                                block_text: &str,
                                meas: &TextMeasureFn,
                                lx: f32,
                                ly: f32|
          -> u32 {
      let mut cache = layouts_ref.borrow_mut();
      let eff = code_page_w.get_untracked();
      let fallback_w = if live_wrap.get_untracked() {
        if crate::page_width::is_full_width(eff) {
          820.0
        } else {
          (eff - 80.0).max(200.0)
        }
      } else {
        f32::INFINITY
      };
      let lines = cache.resolve_lines_cached(
        bid,
        block_text,
        fallback_w,
        code_font,
        code_line_h,
        &|f, t| meas(f, t),
      );
      offset_at_point(lines, lx, ly, code_font, &|f, t| meas(f, t)).min(utf16_len(block_text))
    };

    view()
      .element_ref(&text_ref)
      .test_id("code-block-editor")
      .role(AccessibilityRole::Button)
      .label("代码块编辑器")
      .flex_1()
      .min_w(px(0.0))
      .min_h(px(code_line_h))
      .relative()
      .cursor(Cursor::Text)
      .on_layout(move |b: &Bounds<Points>| {
        // Account for `my_2()` (8px) on `node-view-code-block` so row-level
        // cross-block mouse dragging aligns with the exact code line coordinates.
        let adjusted = Bounds {
          origin: point(b.origin.x, px(b.origin.y.0 + 8.0)),
          size: b.size,
        };
        if layouts_f.borrow_mut().update_text_frame(&bid_f, adjusted) {
          layout_rev.update(|v| *v = v.wrapping_add(1));
        }
      })
      .on_press_down(move |e: &PressEvent| {
        focused_code.set(None);
        focused_cell.set(None);
        table_sel.set(None);
        table_drag.set(None);
        marquee_selected.set(None);
        blank_marquee_ready.set(false);
        marquee_origin_page.set(None);
        sink_clear_rev.update(|v| *v = v.wrapping_add(1));

        let text_click = live_src.get_untracked();
        let text_u16_len = utf16_len(&text_click);
        let off = hit_code_offset(&layouts_click, &bid_click, &text_click, &measure_click, e.position.x.0, e.position.y.0);
        let is_touch = last_ptr_touch_code.get_untracked();
        let click_count = if is_touch {
          tap_tracker_code.borrow_mut().bump(
            e.page_position.x.0,
            e.page_position.y.0,
            crate::touch_selection::now_ms(),
          )
        } else {
          tap_tracker_code.borrow_mut().reset();
          e.clicks.clamp(1, 3)
        };
        let idx = live_index(at);
        if click_count == 3 {
          let sel = DocSelection::from_positions(DocPosition::new(idx, 0), DocPosition::new(idx, text_u16_len));
          drag_anchor.set(if is_touch { None } else { Some(sel.anchor_pos()) });
          touch_sel_code.set(is_touch && !sel.is_collapsed());
          br_click.set_selection(sel);
        } else if click_count == 2 {
          let (ws, we) = crate::touch_selection::touch_word_bounds(&text_click, off);
          let sel = DocSelection::from_positions(DocPosition::new(idx, ws), DocPosition::new(idx, we));
          drag_anchor.set(if is_touch { None } else { Some(sel.anchor_pos()) });
          touch_sel_code.set(is_touch && !sel.is_collapsed());
          br_click.set_selection(sel);
        } else {
          let pos = DocPosition::new(idx, off);
          drag_anchor.set(if is_touch { None } else { Some(pos) });
          touch_sel_code.set(false);
          br_click.set_selection(DocSelection::caret(idx, off));
        }
        br_click.request_focus();
      })
      .on_long_press(move |e: &PressEvent| {
        if !last_ptr_touch_code_lp.get_untracked() {
          return;
        }
        focused_code.set(None);
        focused_cell.set(None);
        table_sel.set(None);
        table_drag.set(None);
        marquee_selected.set(None);
        blank_marquee_ready.set(false);
        marquee_origin_page.set(None);
        drag_anchor.set(None);
        let text_code_lp = live_src.get_untracked();
        let off = hit_code_offset(&layouts_code_lp, &bid_code_lp, &text_code_lp, &measure_lp, e.position.x.0, e.position.y.0);
        let idx = live_index(at);
        let (ws, we) = crate::touch_selection::touch_word_bounds(&text_code_lp, off);
        if we > ws {
          let sel = DocSelection::from_positions(DocPosition::new(idx, ws), DocPosition::new(idx, we));
          touch_sel_code_lp.set(true);
          br_code_lp.set_selection(sel);
        }
      })
      .on_press(move || {
        // Accessibility / headless test press fallback when `on_press_down` wasn't synthesized.
        let idx = live_index(at);
        if br_press.selection().is_none_or(|s| s.focus_node() != idx) {
          marquee_selected.set(None);
          focused_code.set(None);
          focused_cell.set(None);
          let end = utf16_len(&live_src.get_untracked());
          br_press.set_selection(DocSelection::caret(idx, end));
          br_press.request_focus();
        }
      })
      .on_mouse_move(move |e: &MouseEvent| {
        if e.pressed_button != Some(MouseButton::Left) || blank_marquee_ready.get_untracked() {
          return;
        }
        let Some(anchor) = drag_anchor.get_untracked() else {
          return;
        };
        inner_drag_hit_code.set(Some((e.page_position.x.0, e.page_position.y.0)));
        let idx = live_index(at);
        let text_code_move = live_src.get_untracked();
        let off = hit_code_offset(&layouts_move, &bid_move, &text_code_move, &measure_move, e.position.x.0, e.position.y.0);
        let next = DocSelection::from_positions(anchor, DocPosition::new(idx, off));
        if br_move.selection().as_ref() != Some(&next) {
          br_move.set_selection(next);
        }
      })
      // 1. Scoped selection highlight, find matches, IME preedit, and blinking caret
      .child({
        let br_ov_sel = br_sel.clone();
        let br_ov_find = br_find.clone();
        let local_overlay = computed(move || {
          let idx = at.index();
          let rev = rev_sig.get();
          let in_marquee = marquee_selected
            .get()
            .is_some_and(|(lo, hi)| idx >= lo && idx <= hi);
          let sel_relevant = if in_marquee {
            None
          } else {
            br_ov_sel.selection_signal().get().and_then(|sel| {
              let start_p = sel.start_pos();
              let end_p = sel.end_pos();
              let focus_p = sel.focus_pos();
              let has_range = !sel.is_collapsed() && idx >= start_p.node && idx <= end_p.node;
              let has_caret = sel.is_collapsed() && focus_p.node == idx;
              if has_range || has_caret {
                let focused = if has_caret { editor_focused.get() } else { false };
                let preedit = if has_caret && focused {
                  preedit_overlay_sig.get().filter(|(bi, _, comp)| *bi == idx && !comp.is_empty())
                } else {
                  None
                };
                if has_range || focused {
                  Some((sel, focused, preedit))
                } else {
                  None
                }
              } else {
                None
              }
            })
          };
          let find_all = br_ov_find.find_matches_signal().get();
          let has_find = find_all.iter().any(|fm| fm.block_index == idx);
          let find_active = if has_find { br_ov_find.find_active_idx_signal().get() } else { 0 };
          if sel_relevant.is_none() && !has_find {
            None
          } else {
            Some((idx, rev, layout_rev.get(), dark_mode.get(), sel_relevant, has_find, find_active))
          }
        });
        dynamic(move || {
          let Some((idx, _rev, _lrev, dark, sel_relevant, has_find, find_active)) = local_overlay.get() else {
            return view()
              .absolute()
              .inset_0()
              .pointer_events(PointerEvents::None);
          };
          let p = palette(dark);
          let code_sel_text = live_src.get_untracked();
          let text_u16_len = utf16_len(&code_sel_text);
          let mut cache = layouts_sel.borrow_mut();
          let eff = code_sel_page_w.get_untracked();
          let fallback_w = if live_wrap.get_untracked() {
            if crate::page_width::is_full_width(eff) {
              820.0
            } else {
              (eff - 80.0).max(200.0)
            }
          } else {
            f32::INFINITY
          };
          let lines = cache.resolve_lines_cached(
            &bid_sel,
            &code_sel_text,
            fallback_w,
            code_font,
            code_line_h,
            &|f, t| measure_sel(f, t),
          );

          let mut layer = view()
            .absolute()
            .inset_0()
            .pointer_events(PointerEvents::None);

          if has_find {
            let find_all = br_find.find_matches_signal().get_untracked();
            for (mi, fm) in find_all.iter().enumerate() {
              if fm.block_index != idx {
                continue;
              }
              let bg = if mi == find_active {
                Rgba { r: 0.96, g: 0.62, b: 0.04, a: 0.42 }
              } else {
                Rgba { r: 0.98, g: 0.80, b: 0.08, a: 0.28 }
              };
              for (rx, ry, rw, rh) in selection_rects_for_range(
                lines,
                fm.start,
                fm.end,
                code_font,
                code_line_h,
                &|f, t| measure_sel(f, t),
              ) {
                layer = layer.child(
                  view()
                    .absolute()
                    .left(px(rx))
                    .top(px(ry + 1.5))
                    .w(px(rw))
                    .h(px((rh - 3.0).max(14.0)))
                    .rounded(px(3.0))
                    .bg(bg),
                );
              }
            }
          }

          if let Some((sel, focused, active_preedit)) = sel_relevant {
            let start_p = sel.start_pos();
            let end_p = sel.end_pos();
            let focus_p = sel.focus_pos();
            if !sel.is_collapsed() && idx >= start_p.node && idx <= end_p.node {
              let s_u16 = if idx == start_p.node { start_p.offset } else { 0 };
              let e_u16 = if idx == end_p.node { end_p.offset } else { text_u16_len.max(1) };
              for (rx, ry, rw, rh) in selection_rects_for_range(
                lines,
                s_u16,
                e_u16,
                code_font,
                code_line_h,
                &|f, t| measure_sel(f, t),
              ) {
                layer = layer.child(
                  view()
                    .absolute()
                    .left(px(rx))
                    .top(px(ry + 1.5))
                    .w(px(rw))
                    .h(px((rh - 3.0).max(14.0)))
                    .rounded(px(3.0))
                    .bg(p.selection_bg),
                );
              }
            }
            if sel.is_collapsed() && focus_p.node == idx && focused {
              let (cx, cy, ch) = caret_rect_for_offset(
                lines,
                focus_p.offset,
                code_font,
                code_line_h,
                &|f, t| measure_sel(f, t),
              );
              if let Some((_, _, comp)) = active_preedit {
                layer = layer.child(
                  view()
                    .test_id("mica-ime-preedit")
                    .absolute()
                    .left(px(cx))
                    .top(px(cy))
                    .h(px(ch.max(code_line_h)))
                    .px(px(2.0))
                    .rounded(px(2.0))
                    .bg(p.surface_raised)
                    .border_b_1()
                    .border_color(p.accent)
                    .justify_center()
                    .child(
                      text(comp)
                        .text_size(px(14.0))
                        .line_height(px(code_line_h))
                        .font_family("monospace")
                        .text_color(p.text_primary),
                    ),
                );
              } else {
                layer = layer.child(
                  view()
                    .test_id("mica-active-caret")
                    .absolute()
                    .left(px(cx))
                    .top(px(cy + 1.5))
                    .w(px(2.0))
                    .h(px((ch - 3.0).max(14.0)))
                    .rounded(px(1.0))
                    .bg(move || {
                      if caret_blink_on.get() {
                        palette(dark_mode.get()).accent
                      } else {
                        TRANSPARENT_RGBA
                      }
                    }),
                );
              }
            }
          }
          layer
        })
      })
      // 2. Ghost placeholder when empty (pointer-events none so caret stays at (0, 0))
      .child(when(
        move || live_src.get().is_empty(),
        move || {
          view()
            .absolute()
            .left_0()
            .top_0()
            .pointer_events(PointerEvents::None)
            .child(
              text("输入代码...")
                .text_size(px(14.0))
                .line_height(px(code_line_h))
                .font_family("monospace")
                .text_color(move || palette(dark_mode.get()).text_ghost),
            )
        },
      ))
      // 3. Single unified syntax-highlighted code text (updates in-place on typing /
      //    Backspace without tearing down the outer card, toolbar, or overlays!)
      .child(dynamic(move || {
        let text_render = live_src.get();
        let (lang_hl, _) = live_lang.get();
        render_highlighted_code_text(&text_render, &lang_hl, dark_mode)
          .on(EventKind::TextLayout, {
            let layouts_tl = layouts_l.clone();
            let bid_tl = bid_l.clone();
            move |p: &EventPayload| {
              if let EventPayload::TextLayout(lines) = p
                && layouts_tl.borrow_mut().update_lines(
                  &bid_tl,
                  lines.clone(),
                  code_font.to_owned(),
                  code_line_h,
                )
              {
                layout_rev.update(|v| *v = v.wrapping_add(1));
              }
            }
          })
      }))
  };

  let touch_sel_card = env.touch_selection_active;
  let last_ptr_touch_card = env.last_pointer_is_touch;
  let br_card_down = bridge.clone();
  let code_row = view()
    .w_full()
    .flex_row()
    .items_start()
    .child(when(move || live_line_numbers.get(), move || {
      view()
        .min_w(px(28.0))
        .pr(px(12.0))
        .flex_shrink_0()
        .items_end()
        .pointer_events(PointerEvents::None)
        .child(
          text(move || {
            let line_count = live_src.get().bytes().filter(|&b| b == b'\n').count() + 1;
            (1..=line_count)
              .map(|n| n.to_string())
              .collect::<Vec<_>>()
              .join("\n")
          })
          .text_size(px(13.0))
          .line_height(px(code_line_h))
          .font_family("monospace")
          .text_color(move || palette(dark_mode.get()).text_faint),
        )
    }))
    .child(code_text_box);

  let layouts_row = layouts;
  let bid_row = block_id;

  view()
    .test_id("node-view-code-block")
    .relative()
    .w_full()
    .my_2()
    .on_layout(move |b: &Bounds<Points>| {
      layouts_row.borrow_mut().update_row_frame(&bid_row, *b);
    })
    .child(
      view()
        .relative()
        .w_full()
        .rounded(px(6.0))
        .bg(move || palette(dark_mode.get()).code_bg)
        .px(px(14.0))
        .py(px(12.0))
        .on_mouse_enter(move |_: &MouseEvent| {
          if drag_anchor.get_untracked().is_none()
            && table_drag.get_untracked().is_none()
            && !blank_marquee_ready.get_untracked()
          {
            hovered.set(true);
          }
        })
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .on_press_down(move |e: &PressEvent| {
          focused_code.set(None);
          focused_cell.set(None);
          table_sel.set(None);
          table_drag.set(None);
          marquee_selected.set(None);
          blank_marquee_ready.set(false);
          marquee_origin_page.set(None);
          touch_sel_card.set(false);
          sink_clear_rev.update(|v| *v = v.wrapping_add(1));
          let idx = live_index(at);
          let text_u16_len = utf16_len(&live_src.get_untracked());
          let off = if e.position.x.0 < 14.0 { 0 } else { text_u16_len };
          let pos = DocPosition::new(idx, off);
          drag_anchor.set(if last_ptr_touch_card.get_untracked() {
            None
          } else {
            Some(pos)
          });
          br_card_down.set_selection(DocSelection::caret(idx, off));
          br_card_down.request_focus();
        })
        .child(code_row)
        // Floating top-right hover/active toolbar (suppressed during drag-selection
        // so toolbar buttons never intercept DOM `elementFromPoint` hits mid-drag!)
        .child(when(
          move || {
            drag_anchor.get().is_none()
              && (hovered.get()
                || is_caret_in_block.get()
                || lang_open.get()
                || ai_open.get()
                || more_open.get()
                || copied.get())
          },
          move || {
            view()
              .test_id("code-block-toolbar")
              .absolute()
              .top(px(6.0))
              .right(px(8.0))
              .z_index(20)
              .flex_row()
              .items_center()
              .gap(px(4.0))
              .child(
                button()
                  .element_ref(&lang_btn_ref)
                  .test_id("code-lang-btn")
                  .accessibility_label("选择代码语言")
                  .h(px(24.0))
                  .px_2()
                  .rounded(px(5.0))
                  .flex_row()
                  .items_center()
                  .gap(px(3.0))
                  .bg(move || palette(dark_mode.get()).surface_hover)
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_normal))
                  .on_press(move || {
                    ai_open.set(false);
                    more_open.set(false);
                    lang_query.set(String::new());
                    lang_open.set(!lang_open.get_untracked());
                  })
                  .child(
                    text(move || format!("{} ▾", live_lang.get().1))
                      .text_size(px(12.0))
                      .text_color(move || palette(dark_mode.get()).text_muted),
                  ),
              )
              .child(
                button()
                  .element_ref(&ai_btn_ref)
                  .test_id("code-ai-btn")
                  .accessibility_label("AI 代码助手")
                  .w(px(24.0))
                  .h(px(24.0))
                  .rounded(px(5.0))
                  .items_center()
                  .justify_center()
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_normal))
                  .on_press(move || {
                    lang_open.set(false);
                    more_open.set(false);
                    ai_open.set(!ai_open.get_untracked());
                  })
                  .child(dynamic(move || {
                    mica_icons::sparkles_icon(palette(dark_mode.get()).text_muted_hex)
                  })),
              )
              .child(
                button()
                  .test_id("code-copy-btn")
                  .accessibility_label("复制代码")
                  .w(px(24.0))
                  .h(px(24.0))
                  .rounded(px(5.0))
                  .items_center()
                  .justify_center()
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_normal))
                  .on_press(move || copy_with_reset(live_src.get_untracked(), copied, copy_gen))
                  .child(dynamic(move || {
                    let dark = dark_mode.get();
                    if copied.get() {
                      mica_icons::check_icon(if dark { "#4ADE80" } else { "#16A34A" })
                    } else {
                      mica_icons::copy_icon(palette(dark).text_muted_hex)
                    }
                  })),
              )
              .child(
                button()
                  .element_ref(&more_btn_ref)
                  .test_id("code-more-btn")
                  .accessibility_label("代码块更多操作")
                  .w(px(24.0))
                  .h(px(24.0))
                  .rounded(px(5.0))
                  .items_center()
                  .justify_center()
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).border_normal))
                  .on_press(move || {
                    lang_open.set(false);
                    ai_open.set(false);
                    more_open.set(!more_open.get_untracked());
                  })
                  .child(dynamic(move || {
                    mica_icons::more_dots(palette(dark_mode.get()).text_muted_hex)
                  })),
              )
          },
        ))
        // Top-level `overlay()` Language Picker — NEVER occluded by boards below!
        .child(when(move || lang_open.get(), {
          let br_l = br_lang.clone();
          move || {
            let br_l2 = br_l.clone();
            overlay()
              .anchor(&lang_btn_ref)
              .placement(Placement::Bottom)
              .align(Align::End)
              .offset(px(4.0))
              .on_press_outside(move |_: &Point<Points>| lang_open.set(false))
              .on_cancel(move || lang_open.set(false))
              .child(
                view()
                  .test_id("code-lang-picker")
                  .w(px(210.0))
                  .max_h(px(280.0))
                  .p_2()
                  .gap_1()
                  .rounded(px(8.0))
                  .bg(move || palette(dark_mode.get()).surface_overlay)
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).border_normal)
                  .child(
                    input()
                      .test_id("code-lang-search")
                      .placeholder("搜索语言...")
                      .w_full()
                      .h(px(30.0))
                      .px_2()
                      .rounded(px(6.0))
                      .border_1()
                      .border_color(move || palette(dark_mode.get()).border_normal)
                      .text_size(px(12.5))
                      .text_color(move || palette(dark_mode.get()).text_primary)
                      .bg(move || palette(dark_mode.get()).surface_base)
                      .value(move || lang_query.get())
                      .on_change(move |v: &str| lang_query.set(v.to_owned())),
                  )
                  .child(scroll().w_full().h(px(220.0)).child(dynamic(move || {
                    let q = lang_query.get().trim().to_ascii_lowercase();
                    let cur_l2 = live_lang.get().1;
                    let p = palette(dark_mode.get());
                    let mut list = view().w_full();
                    let mut count = 0usize;
                    for &lang in CODE_LANGUAGES {
                      if !q.is_empty() && !lang.contains(&q) {
                        continue;
                      }
                      count += 1;
                      let is_cur = lang == cur_l2;
                      let br_item = br_l2.clone();
                      let lang_owned = lang.to_string();
                      list = list.child(
                        button()
                          .test_id(&format!("code-lang-{lang}"))
                          .w_full()
                          .h(px(30.0))
                          .px_2()
                          .rounded(px(5.0))
                          .flex_row()
                          .items_center()
                          .gap_2()
                          .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                          .on_press(move || {
                            lang_open.set(false);
                            let val = if lang_owned == "auto" {
                              Value::Null
                            } else {
                              Value::String(lang_owned.clone())
                            };
                            br_item.update_block_data_attr_at(live_index(at), "language", val);
                          })
                          .child(
                            view()
                              .w(px(16.0))
                              .items_center()
                              .justify_center()
                              .children(is_cur.then(|| mica_icons::check_icon(p.text_primary_hex))),
                          )
                          .child(
                            text(lang)
                              .text_size(px(12.5))
                              .font_weight(if is_cur { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                              .text_color(p.text_primary),
                          ),
                      );
                    }
                    if count == 0 {
                      list = list.child(
                        view()
                          .w_full()
                          .py_4()
                          .items_center()
                          .child(text("无匹配结果").text_size(px(12.0)).text_color(p.text_faint)),
                      );
                    }
                    list
                  }))),
              )
          }
        }))
        // Top-level `overlay()` Ask AI menu
        .child(when(move || ai_open.get(), {
          let ai_cb_m = ai_cb.clone();
          move || {
            let ai_cb_i = ai_cb_m.clone();
            let items: [(CodeAiAction, &str, &str); 4] = [
              (CodeAiAction::Explain, "解释代码", "code-ai-explain"),
              (CodeAiAction::Improve, "优化代码", "code-ai-improve"),
              (CodeAiAction::Fix, "查找问题", "code-ai-fix"),
              (CodeAiAction::Custom, "自定义提问", "code-ai-custom"),
            ];
            let mut menu = view()
              .test_id("code-ai-menu")
              .w(px(160.0))
              .p_1()
              .rounded(px(8.0))
              .bg(move || palette(dark_mode.get()).surface_overlay)
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal);
            for (action, label, tid) in items {
              let cb = ai_cb_i.clone();
              menu = menu.child(
                button()
                  .test_id(tid)
                  .w_full()
                  .h(px(30.0))
                  .px_2()
                  .rounded(px(5.0))
                  .flex_row()
                  .items_center()
                  .gap_2()
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || {
                    ai_open.set(false);
                    if let Some(c) = &cb {
                      (c.0)(action, live_index(at), live_src.get_untracked());
                    } else {
                      ai_notice.set(Some("请先在设置中配置 AI 模型".to_string()));
                      let g = ai_notice_gen.get_untracked().wrapping_add(1);
                      ai_notice_gen.set(g);
                      spawn(async move {
                        timer(Duration::from_millis(2200)).await;
                        if ai_notice_gen.get_untracked() == g {
                          ai_notice.set(None);
                        }
                      });
                    }
                  })
                  .child(text(label).text_size(px(12.5)).text_color(move || palette(dark_mode.get()).text_primary)),
              );
            }
            overlay()
              .anchor(&ai_btn_ref)
              .placement(Placement::Bottom)
              .align(Align::End)
              .offset(px(4.0))
              .on_press_outside(move |_: &Point<Points>| ai_open.set(false))
              .on_cancel(move || ai_open.set(false))
              .child(menu)
          }
        }))
        // Top-level `overlay()` More menu
        .child(when(move || more_open.get(), {
          let bw = br_wrap.clone();
          let bl = br_ln.clone();
          let bd = br_del.clone();
          move || {
            let bw = bw.clone();
            let bl = bl.clone();
            let bd = bd.clone();
            overlay()
              .anchor(&more_btn_ref)
              .placement(Placement::Bottom)
              .align(Align::End)
              .offset(px(4.0))
              .on_press_outside(move |_: &Point<Points>| more_open.set(false))
              .on_cancel(move || more_open.set(false))
              .child(
                view()
                  .test_id("code-more-menu")
                  .w(px(164.0))
                  .p_1()
                  .rounded(px(8.0))
                  .bg(move || palette(dark_mode.get()).surface_overlay)
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).border_normal)
                  .child(
                    button()
                      .test_id("code-menu-wrap")
                      .w_full()
                      .h(px(30.0))
                      .px_2()
                      .rounded(px(5.0))
                      .flex_row()
                      .items_center()
                      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                      .on_press(move || {
                        bw.update_block_data_attr_at(live_index(at), "wrap", Value::Bool(!live_wrap.get_untracked()));
                        more_open.set(false);
                      })
                      .child(
                        text(move || if live_wrap.get() { "取消换行" } else { "自动换行" })
                          .text_size(px(12.5))
                          .text_color(move || palette(dark_mode.get()).text_primary),
                      ),
                  )
                  .child(
                    button()
                      .test_id("code-menu-line-numbers")
                      .w_full()
                      .h(px(30.0))
                      .px_2()
                      .rounded(px(5.0))
                      .flex_row()
                      .items_center()
                      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                      .on_press(move || {
                        bl.update_block_data_attr_at(live_index(at), "lineNumbers", Value::Bool(!live_line_numbers.get_untracked()));
                        more_open.set(false);
                      })
                      .child(
                        text(move || if live_line_numbers.get() { "隐藏行号" } else { "显示行号" })
                          .text_size(px(12.5))
                          .text_color(move || palette(dark_mode.get()).text_primary),
                      ),
                  )
                  .child(
                    button()
                      .test_id("code-menu-title")
                      .w_full()
                      .h(px(30.0))
                      .px_2()
                      .rounded(px(5.0))
                      .flex_row()
                      .items_center()
                      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                      .on_press(move || {
                        more_open.set(false);
                        title_draft.set(live_title.get_untracked());
                        title_editing.set(true);
                      })
                      .child(
                        text(move || {
                          if live_title.get().trim().is_empty() {
                            "添加标题"
                          } else {
                            "编辑标题"
                          }
                        })
                        .text_size(px(12.5))
                        .text_color(move || palette(dark_mode.get()).text_primary),
                      ),
                  )
                  .child(
                    button()
                      .test_id("code-menu-delete")
                      .w_full()
                      .h(px(30.0))
                      .px_2()
                      .rounded(px(5.0))
                      .flex_row()
                      .items_center()
                      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).danger_wash))
                      .on_press(move || {
                        more_open.set(false);
                        bd.delete_block_at(live_index(at));
                      })
                      .child(text("删除代码块").text_size(px(12.5)).text_color(move || palette(dark_mode.get()).danger)),
                  ),
              )
          }
        }))
        .child(when(move || ai_notice.get().is_some(), move || {
          view()
            .absolute()
            .bottom(px(8.0))
            .right(px(8.0))
            .z_index(30)
            .px_2()
            .py_1()
            .rounded(px(5.0))
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .child(
              text(move || ai_notice.get().unwrap_or_default())
                .text_size(px(11.5))
                .text_color(move || palette(dark_mode.get()).text_muted),
            )
        })),
    )
    .child(when(move || title_editing.get(), {
      let br_t = br_title.clone();
      move || {
        let br_save = br_t.clone();
        let br_sub = br_t.clone();
        let br_rm = br_t.clone();
        view()
          .w_full()
          .pt_1()
          .flex_row()
          .items_center()
          .justify_center()
          .gap_2()
          .child(
            input()
              .test_id("code-title-input")
              .auto_focus(true)
              .submit_behavior(SubmitBehavior::Submit)
              .placeholder("输入代码块标题...")
              .w(px(260.0))
              .h(px(28.0))
              .px_2()
              .rounded(px(5.0))
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .bg(move || palette(dark_mode.get()).surface_base)
              .text_size(px(12.5))
              .text_color(move || palette(dark_mode.get()).text_primary)
              .value(move || title_draft.get())
              .on_change(move |v: &str| title_draft.set(v.to_owned()))
              .on_submit(move |v: &str| {
                let clean = v.trim();
                let val = if clean.is_empty() { Value::Null } else { Value::String(clean.to_owned()) };
                br_sub.update_block_data_attr_at(live_index(at), "title", val);
                title_editing.set(false);
              }),
          )
          .child(mini_chip("保存", dark_mode, move || {
            let cur = title_draft.get_untracked();
            let clean = cur.trim();
            let val = if clean.is_empty() { Value::Null } else { Value::String(clean.to_owned()) };
            br_save.update_block_data_attr_at(live_index(at), "title", val);
            title_editing.set(false);
          }))
          .child(mini_chip("移除", dark_mode, move || {
            title_draft.set(String::new());
            br_rm.update_block_data_attr_at(live_index(at), "title", Value::Null);
            title_editing.set(false);
          }))
      }
    }))
    .child(when(
      move || !live_title.get().trim().is_empty() && !title_editing.get(),
      move || {
        view()
          .test_id("code-block-caption")
          .w_full()
          .h(px(22.0))
          .items_center()
          .justify_center()
          .child(
            text(move || live_title.get())
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_muted),
          )
      },
    ))
}

fn render_highlighted_code_text(code: &str, lang: &str, dark_mode: Signal<bool>) -> Text {
  let p = palette(dark_mode.get());
  let mut root = text("")
    .w_full()
    .text_size(px(14.0))
    .line_height(px(22.0))
    .font_family("monospace")
    .text_color(p.text_primary);
  if code.is_empty() {
    return root;
  }
  // Skip tokenizing gigantic snippets (>12KB) to keep 60fps virtual scrolling.
  if code.len() > 12_000 {
    root = root.child(text(code.to_owned()));
    if code.ends_with('\n') {
      root = root.child(text("\u{200B}"));
    }
    return root;
  }
  let tokens = mica_md::highlight::highlight_code(code, lang);
  if tokens.is_empty() {
    root = root.child(text(code.to_owned()));
    if code.ends_with('\n') {
      root = root.child(text("\u{200B}"));
    }
    return root;
  }
  let total_u16 = utf16_len(code);
  let mut cursor = 0u32;
  for tok in tokens {
    let s = tok.start.min(total_u16);
    let e = tok.end.min(total_u16);
    if s > cursor {
      let plain = utf16_slice(code, cursor, s);
      if !plain.is_empty() {
        root = root.child(text(plain.to_owned()));
      }
    }
    if e > s {
      let seg = utf16_slice(code, s, e);
      if !seg.is_empty() {
        let color = match tok.role {
          mica_md::highlight::Role::Keyword => p.code_keyword,
          mica_md::highlight::Role::String => p.code_string,
          mica_md::highlight::Role::Comment => p.code_comment,
          mica_md::highlight::Role::Number => p.code_number,
          mica_md::highlight::Role::Function => p.code_function,
          mica_md::highlight::Role::Key => p.code_key,
        };
        root = root.child(text(seg.to_owned()).text_color(color));
      }
    }
    cursor = cursor.max(e);
  }
  if cursor < total_u16 {
    let tail = utf16_slice(code, cursor, total_u16);
    if !tail.is_empty() {
      root = root.child(text(tail.to_owned()));
    }
  }
  if code.ends_with('\n') {
    root = root.child(text("\u{200B}"));
  }
  root
}

// ── 3. Embedded Board (Single-Layer, Snug Auto-Fit, Draggable Resize Handles) ─

fn render_board_block_view(env: &BlockRenderEnv, at: Position, block: EditorBlock) -> impl IntoElement {
  let dark_mode = env.dark_mode;
  let bridge = env.bridge.clone();
  let open_board = use_context::<OpenBoardCallback>();
  let open_board_block = use_context::<OpenBoardWithBlockCallback>();

  let source = block.text.clone();
  let persisted_w = block.data.get("width").and_then(Value::as_f64).filter(|&v| v > 0.0).map(|v| v as f32);
  let persisted_h = block.data.get("height").and_then(Value::as_f64).filter(|&v| v > 0.0).map(|v| v as f32);
  let natural_aspect = crate::board_view::board_natural_aspect(&source);

  let hovered = signal(false);
  let container_w = signal(720.0_f32);
  let resizing_size: Signal<Option<(f32, f32)>> = signal(None);
  let drag_start = signal((0.0_f32, 0.0_f32, 720.0_f32, 360.0_f32));
  let last_handle_click_ms = signal(0.0_f64);

  let effective_width = computed(move || {
    let max_w = container_w.get().max(240.0);
    if let Some((w, _)) = resizing_size.get() {
      return w.clamp(220.0, max_w);
    }
    persisted_w.map(|w| w.clamp(220.0, max_w)).unwrap_or(max_w)
  });

  let effective_height = computed(move || {
    if let Some((_, h)) = resizing_size.get() {
      return h.clamp(180.0, 960.0);
    }
    if let Some(h) = persisted_h {
      return h.clamp(180.0, 960.0);
    }
    (effective_width.get() * natural_aspect).clamp(320.0, 540.0)
  });

  let has_custom_size = persisted_w.is_some() || persisted_h.is_some();

  let open_click: Rc<dyn Fn()> = {
    let obb = open_board_block.clone();
    let ob = open_board.clone();
    let src = source.clone();
    Rc::new(move || {
      if let Some(cb) = &obb {
        (cb.0)(live_index(at), src.clone());
      } else if let Some(cb) = &ob {
        (cb.0)();
      }
    })
  };
  let open_btn = open_click.clone();

  let commit_size: Rc<dyn Fn(Option<f32>, Option<f32>)> = {
    let br = bridge.clone();
    Rc::new(move |w_opt, h_opt| {
      let cur_idx = live_index(at);
      br.update_block_data_attr_at(
        cur_idx,
        "width",
        w_opt.map(|v| Value::from(v.round() as i64)).unwrap_or(Value::Null),
      );
      br.update_block_data_attr_at(
        cur_idx,
        "height",
        h_opt.map(|v| Value::from(v.round() as i64)).unwrap_or(Value::Null),
      );
    })
  };
  let (cb_reset, cb_bottom, cb_right, cb_corner) = (
    commit_size.clone(),
    commit_size.clone(),
    commit_size.clone(),
    commit_size,
  );

  let preview_src = source;
  view()
    .test_id("node-view-board")
    .w_full()
    .my_2()
    .on_layout(move |b: &Bounds<Points>| {
      let w = b.size.width.0;
      if w > 120.0 && (container_w.get_untracked() - w).abs() > 2.0 {
        container_w.set(w);
      }
    })
    .child(
      view()
        .relative()
        .w(move || {
          let max_w = container_w.get().max(240.0);
          let ew = effective_width.get();
          if (max_w - ew).abs() <= 2.0 {
            Length::from(relative(1.0))
          } else {
            Length::from(px(ew))
          }
        })
        .h(move || Length::from(px(effective_height.get())))
        .rounded(px(8.0))
        .border_1()
        .border_color(move || {
          let p = palette(dark_mode.get());
          if resizing_size.get().is_some() { p.accent } else { p.border_normal }
        })
        .bg(move || palette(dark_mode.get()).surface_base)
        .overflow_hidden()
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .child(
          button()
            .test_id("board-preview-open-btn")
            .accessibility_label("点击画板进入编辑")
            .w_full()
            .h_full()
            .on_press(move || open_click())
            .child(
              canvas(move || {
                let sz = (effective_width.get(), effective_height.get());
                crate::board_view::paint_board_preview(&preview_src, dark_mode.get(), sz)
              })
              .w_full()
              .h_full(),
            ),
        )
        // Hover-only top-right floating chip (zero gray header strip!)
        .child(when(move || hovered.get(), move || {
          let ob = open_btn.clone();
          let cr = cb_reset.clone();
          view()
            .absolute()
            .top(px(8.0))
            .right(px(8.0))
            .z_index(20)
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(when(move || has_custom_size, {
              let cr2 = cr.clone();
              move || {
                let cr3 = cr2.clone();
                button()
                  .test_id("board-reset-size-btn")
                  .px_2()
                  .py_1()
                  .rounded(px(6.0))
                  .bg(move || palette(dark_mode.get()).surface_overlay)
                  .border_1()
                  .border_color(move || palette(dark_mode.get()).border_normal)
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || cr3(None, None))
                  .child(text("自适应").text_size(px(12.0)).text_color(move || palette(dark_mode.get()).text_muted))
              }
            }))
            .child(
              button()
                .test_id("board-open-full-btn")
                .accessibility_label("编辑内嵌白板")
                .px_2()
                .py_1()
                .rounded(px(6.0))
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .bg(move || palette(dark_mode.get()).surface_overlay)
                .border_1()
                .border_color(move || palette(dark_mode.get()).border_normal)
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || ob())
                .child(dynamic(move || mica_icons::whiteboard(palette(dark_mode.get()).text_muted_hex)))
                .child(
                  text("编辑白板")
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            )
        }))
        // Bottom resize handle
        .child(
          view()
            .test_id("board-resize-bottom")
            .absolute()
            .left(px(16.0))
            .right(px(16.0))
            .bottom(px(0.0))
            .h(px(10.0))
            .z_index(25)
            .items_center()
            .justify_center()
            .cursor(Cursor::RowResize)
            .claim_touches(TouchClaim::on_start())
            .keep_touches(true)
            .on_responder_grant(move |e: &ResponderEvent| {
              let (w, h) = (effective_width.get_untracked(), effective_height.get_untracked());
              drag_start.set((f32::from(e.page.x), f32::from(e.page.y), w, h));
              resizing_size.set(Some((w, h)));
            })
            .on_responder_move(move |e: &ResponderEvent| {
              let (_, sy, sw, sh) = drag_start.get_untracked();
              resizing_size.set(Some((sw, (sh + (f32::from(e.page.y) - sy)).clamp(180.0, 960.0))));
            })
            .on_responder_release(move |e: &ResponderEvent| {
              let (_, sy, _, sh) = drag_start.get_untracked();
              let moved = (f32::from(e.page.y) - sy).abs();
              let fin = resizing_size.get_untracked();
              resizing_size.set(None);
              if moved < 4.0 {
                let now = e.timestamp;
                let prev = last_handle_click_ms.get_untracked();
                if prev > 0.0 && now >= prev && (now - prev) <= 350.0 {
                  last_handle_click_ms.set(0.0);
                  cb_bottom(None, None);
                } else {
                  last_handle_click_ms.set(now);
                }
                return;
              }
              if let Some((_, nh)) = fin && (nh - sh).abs() > 1.0 {
                cb_bottom(persisted_w, Some(nh));
              }
            })
            .on_responder_terminate(move |_: &ResponderEvent| resizing_size.set(None))
            .child(view().w(px(40.0)).h(px(4.0)).rounded(px(2.0)).bg(move || {
              let p = palette(dark_mode.get());
              if resizing_size.get().is_some() { p.accent } else if hovered.get() { p.border_strong } else { TRANSPARENT_RGBA }
            })),
        )
        // Right resize handle
        .child(
          view()
            .test_id("board-resize-right")
            .absolute()
            .top(px(16.0))
            .bottom(px(16.0))
            .right(px(0.0))
            .w(px(10.0))
            .z_index(25)
            .items_center()
            .justify_center()
            .cursor(Cursor::ColResize)
            .claim_touches(TouchClaim::on_start())
            .keep_touches(true)
            .on_responder_grant(move |e: &ResponderEvent| {
              let (w, h) = (effective_width.get_untracked(), effective_height.get_untracked());
              drag_start.set((f32::from(e.page.x), f32::from(e.page.y), w, h));
              resizing_size.set(Some((w, h)));
            })
            .on_responder_move(move |e: &ResponderEvent| {
              let (sx, _, sw, sh) = drag_start.get_untracked();
              let max_w = container_w.get_untracked().max(240.0);
              let nw = (sw + (f32::from(e.page.x) - sx)).clamp(220.0, max_w);
              let nh = (nw * (sh / sw.max(1.0)).clamp(0.25, 2.5)).clamp(180.0, 960.0);
              resizing_size.set(Some((nw, nh)));
            })
            .on_responder_release(move |e: &ResponderEvent| {
              let (sx, _, sw, _) = drag_start.get_untracked();
              let moved = (f32::from(e.page.x) - sx).abs();
              let fin = resizing_size.get_untracked();
              resizing_size.set(None);
              if moved < 4.0 {
                let now = e.timestamp;
                let prev = last_handle_click_ms.get_untracked();
                if prev > 0.0 && now >= prev && (now - prev) <= 350.0 {
                  last_handle_click_ms.set(0.0);
                  cb_right(None, None);
                } else {
                  last_handle_click_ms.set(now);
                }
                return;
              }
              if let Some((nw, nh)) = fin && (nw - sw).abs() > 1.0 {
                let max_w = container_w.get_untracked().max(240.0);
                let pw = if (max_w - nw).abs() <= 4.0 { None } else { Some(nw) };
                cb_right(pw, Some(nh));
              }
            })
            .on_responder_terminate(move |_: &ResponderEvent| resizing_size.set(None))
            .child(view().w(px(4.0)).h(px(36.0)).rounded(px(2.0)).bg(move || {
              let p = palette(dark_mode.get());
              if resizing_size.get().is_some() { p.accent } else if hovered.get() { p.border_strong } else { TRANSPARENT_RGBA }
            })),
        )
        // Bottom-right corner resize handle
        .child(
          view()
            .test_id("board-resize-corner")
            .absolute()
            .right(px(0.0))
            .bottom(px(0.0))
            .w(px(16.0))
            .h(px(16.0))
            .z_index(28)
            .items_center()
            .justify_center()
            .cursor(Cursor::NwseResize)
            .claim_touches(TouchClaim::on_start())
            .keep_touches(true)
            .on_responder_grant(move |e: &ResponderEvent| {
              let (w, h) = (effective_width.get_untracked(), effective_height.get_untracked());
              drag_start.set((f32::from(e.page.x), f32::from(e.page.y), w, h));
              resizing_size.set(Some((w, h)));
            })
            .on_responder_move(move |e: &ResponderEvent| {
              let (sx, sy, sw, sh) = drag_start.get_untracked();
              let max_w = container_w.get_untracked().max(240.0);
              let nw = (sw + (f32::from(e.page.x) - sx)).clamp(220.0, max_w);
              let nh = (sh + (f32::from(e.page.y) - sy)).clamp(180.0, 960.0);
              resizing_size.set(Some((nw, nh)));
            })
            .on_responder_release(move |e: &ResponderEvent| {
              let (sx, sy, _, _) = drag_start.get_untracked();
              let moved = (f32::from(e.page.x) - sx).hypot(f32::from(e.page.y) - sy);
              let fin = resizing_size.get_untracked();
              resizing_size.set(None);
              if moved < 4.0 {
                let now = e.timestamp;
                let prev = last_handle_click_ms.get_untracked();
                if prev > 0.0 && now >= prev && (now - prev) <= 350.0 {
                  last_handle_click_ms.set(0.0);
                  cb_corner(None, None);
                } else {
                  last_handle_click_ms.set(now);
                }
                return;
              }
              if let Some((nw, nh)) = fin {
                let max_w = container_w.get_untracked().max(240.0);
                let pw = if (max_w - nw).abs() <= 4.0 { None } else { Some(nw) };
                cb_corner(pw, Some(nh));
              }
            })
            .on_responder_terminate(move |_: &ResponderEvent| resizing_size.set(None))
            .child(
              view()
                .w(px(8.0))
                .h(px(8.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(move || {
                  let p = palette(dark_mode.get());
                  if hovered.get() || resizing_size.get().is_some() { p.accent } else { TRANSPARENT_RGBA }
                })
                .bg(move || {
                  let p = palette(dark_mode.get());
                  if resizing_size.get().is_some() { p.accent } else if hovered.get() { p.surface_base } else { TRANSPARENT_RGBA }
                }),
            ),
        )
        .child(when(move || resizing_size.get().is_some(), move || {
          view()
            .test_id("board-resize-readout")
            .absolute()
            .right(px(16.0))
            .bottom(px(16.0))
            .z_index(30)
            .px_2()
            .py_1()
            .rounded(px(5.0))
            .bg(move || palette(dark_mode.get()).text_primary)
            .pointer_events(PointerEvents::None)
            .child(
              text(move || {
                let (w, h) = resizing_size.get().unwrap_or((0.0, 0.0));
                format!("{} × {}", w.round() as i32, h.round() as i32)
              })
              .text_size(px(11.5))
              .font_weight(FontWeight::MEDIUM)
              .text_color(move || palette(dark_mode.get()).surface_base),
            )
        })),
    )
}

// ── 4. Mermaid Block (Click-to-Zoom Fullscreen Lightbox + Source Toggle) ─────

fn render_mermaid_block_view(env: &BlockRenderEnv, at: Position, block: EditorBlock) -> impl IntoElement {
  let dark_mode = env.dark_mode;
  let bridge = env.bridge.clone();
  let lightbox = env.lightbox;
  let focused_code = env.focused_code_block;
  let block_id = block.id.clone();
  let source = block.text.clone();
  let src_zoom = source.clone();
  let src_zoom_btn = source.clone();
  let src_copy = source.clone();
  let src_edit = source.clone();

  let editing = signal(false);
  let hovered = signal(false);
  let copied = signal(false);
  let copy_gen = signal(0u64);
  let preview_rev = crate::preview_render::preview_rev();

  view()
    .test_id("node-view-mermaid")
    .relative()
    .w_full()
    .my_2()
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_subtle)
    .bg(move || palette(dark_mode.get()).surface_raised)
    .overflow_hidden()
    .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
    .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
    .child(
      // Clicking the Mermaid diagram opens the fullscreen pan/zoom Lightbox Modal!
      // Render via `image(ImageSrc::svg_markup(...))` (resvg + system fonts) rather
      // than `svg(...)`, because NSH's vector `svg()` drawing path drops `<text>` nodes.
      button()
        .test_id("mermaid-preview-btn")
        .accessibility_label("点击放大查看 Mermaid 图表")
        .w_full()
        .p_4()
        .items_center()
        .justify_center()
        .cursor(Cursor::ZoomIn)
        .on_press(move || {
          lightbox.set(Some(LightboxContent::Mermaid {
            block_index: live_index(at),
            source: src_zoom.clone(),
          }));
        })
        .child(dynamic(move || {
          let _ = preview_rev.get();
          if let Some(svg_markup) = render_mermaid_svg_for_theme(&source, dark_mode.get()) {
            let (svg_w, svg_h) = parse_svg_aspect_size(&svg_markup, 860.0, 460.0);
            image(ImageSrc::svg_markup(&svg_markup))
              .w(px(svg_w))
              .h(px(svg_h))
              .into_element()
          } else {
            text(source.clone())
              .text_size(px(12.5))
              .text_color(move || palette(dark_mode.get()).text_muted)
              .into_element()
          }
        })),
    )
    .child(when(
      move || hovered.get() || editing.get() || copied.get(),
      move || {
        let sc = src_copy.clone();
        let sz2 = src_zoom_btn.clone();
        view()
          .absolute()
          .top(px(8.0))
          .right(px(8.0))
          .z_index(20)
          .flex_row()
          .items_center()
          .gap(px(4.0))
          .child(
            button()
              .test_id("mermaid-zoom-btn")
              .accessibility_label("全屏缩放查看 Mermaid")
              .px_2()
              .h(px(24.0))
              .rounded(px(5.0))
              .bg(move || palette(dark_mode.get()).surface_overlay)
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                lightbox.set(Some(LightboxContent::Mermaid {
                  block_index: live_index(at),
                  source: sz2.clone(),
                }));
              })
              .child(text("放大").text_size(px(11.5)).text_color(move || palette(dark_mode.get()).text_primary)),
          )
          .child(
            button()
              .test_id("mermaid-copy-btn")
              .accessibility_label("复制 Mermaid 源码")
              .w(px(24.0))
              .h(px(24.0))
              .rounded(px(5.0))
              .items_center()
              .justify_center()
              .bg(move || palette(dark_mode.get()).surface_overlay)
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || copy_with_reset(sc.clone(), copied, copy_gen))
              .child(dynamic(move || {
                let dark = dark_mode.get();
                if copied.get() {
                  mica_icons::check_icon(if dark { "#4ADE80" } else { "#16A34A" })
                } else {
                  mica_icons::copy_icon(palette(dark).text_muted_hex)
                }
              })),
          )
          .child(
            button()
              .test_id("mermaid-toggle-edit")
              .accessibility_label("切换 Mermaid 源码编辑器")
              .px_2()
              .h(px(24.0))
              .rounded(px(5.0))
              .bg(move || palette(dark_mode.get()).surface_overlay)
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press({
                let bid_m = block_id.clone();
                move || {
                  let next = !editing.get_untracked();
                  editing.set(next);
                  if next {
                    focused_code.set(Some(bid_m.clone()));
                  } else if focused_code.get_untracked().as_deref() == Some(bid_m.as_str()) {
                    focused_code.set(None);
                  }
                }
              })
              .child(
                text(move || if editing.get() { "收起源码" } else { "源码" })
                  .text_size(px(11.5))
                  .text_color(move || palette(dark_mode.get()).text_primary),
              ),
          )
      },
    ))
    .child(when(move || editing.get(), {
      let br_m = bridge.clone();
      let se = src_edit.clone();
      move || {
        let br_m2 = br_m.clone();
        let se2 = se.clone();
        view()
          .w_full()
          .px(px(14.0))
          .py(px(10.0))
          .border_t_1()
          .border_color(move || palette(dark_mode.get()).border_subtle)
          .bg(move || palette(dark_mode.get()).code_bg)
          .child(
            editor()
              .test_id("mermaid-source-editor")
              .accessibility_label("Mermaid 源码编辑器")
              .w_full()
              .default_value(Some(SharedString::from(se2)))
              .auto_focus(true)
              .placeholder("graph LR\n  A[Mica] --> B[Pure Rust]")
              .line_numbers(false)
              .soft_wrap(false)
              .folding(false)
              .font_size(px(13.0))
              .line_height(px(20.0))
              .min_rows(Some(3u32))
              .max_rows(Some(18u32))
              .scroll_beyond_last_line(EditorScrollBeyond::None)
              .styles(move || code_editor_styles(dark_mode.get()))
              .on_change(move |v: &SharedString| br_m2.update_block_source_at(live_index(at), v.as_ref())),
          )
      }
    }))
}

pub fn render_mermaid_svg_for_theme(src: &str, dark_mode: bool) -> Option<String> {
  crate::preview_render::render_mermaid_cached(src, dark_mode)
}

pub fn parse_svg_aspect_size(svg_markup: &str, max_w: f32, max_h: f32) -> (f32, f32) {
  if let Some(vb_pos) = svg_markup.find("viewBox=\"") {
    let rest = &svg_markup[vb_pos + 9..];
    if let Some(end) = rest.find('"') {
      let nums: Vec<f32> = rest[..end]
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f32>().ok())
        .collect();
      if nums.len() == 4 && nums[2] > 1.0 && nums[3] > 1.0 {
        let (vw, vh) = (nums[2], nums[3]);
        // Preserve exact aspect ratio so rasterized SVG text is never stretched.
        let scale = (max_w / vw).min(max_h / vh).min(1.5);
        return ((vw * scale).max(60.0), (vh * scale).max(36.0));
      }
    }
  }
  (max_w.min(580.0), max_h.min(240.0))
}

// ── 5. Math Block (RaTeX SVG + Click-to-Edit LaTeX) ──────────────────────────

fn render_math_block_view(env: &BlockRenderEnv, at: Position, block: EditorBlock) -> impl IntoElement {
  let dark_mode = env.dark_mode;
  let bridge = env.bridge.clone();
  let source = block.text.clone();
  let editing = signal(false);
  let draft = signal(source.clone());

  view()
    .test_id("node-view-math")
    .w_full()
    .my_2()
    .rounded(px(8.0))
    .bg(move || if dark_mode.get() { palette(true).surface_raised } else { MATH_LIGHT_BG })
    .p_3()
    .child(
      button()
        .test_id("math-preview-btn")
        .w_full()
        .items_center()
        .justify_center()
        .on_press(move || editing.set(!editing.get_untracked()))
        .child(dynamic(move || {
          let _ = crate::preview_render::preview_rev().get();
          let latex = if source.trim().is_empty() { r"E = mc^2" } else { source.trim() };
          let fg = if dark_mode.get() { "#f4f4f5" } else { "#18181b" };
          if let Some(m) = crate::preview_render::render_math_cached(latex, true, fg) {
            let w = (m.width * 22.0).clamp(60.0, 560.0) as f32;
            let h = (m.height * 22.0).clamp(28.0, 160.0) as f32;
            svg(ImageSrc::svg_markup(&m.svg)).w(px(w)).h(px(h)).into_element()
          } else {
            text(latex.to_string())
              .text_size(px(14.0))
              .text_color(move || palette(dark_mode.get()).text_primary)
              .into_element()
          }
        })),
    )
    .child(when(move || editing.get(), {
      let br_m = bridge.clone();
      move || {
        let br_m2 = br_m.clone();
        view()
          .w_full()
          .pt_2()
          .child(
            input()
              .test_id("math-latex-input")
              .placeholder(r"输入 LaTeX 公式，例如 \sum_{i=1}^n i^2")
              .submit_behavior(SubmitBehavior::Submit)
              .w_full()
              .h(px(32.0))
              .px_2()
              .rounded(px(6.0))
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .bg(move || palette(dark_mode.get()).surface_base)
              .text_size(px(13.0))
              .text_color(move || palette(dark_mode.get()).text_primary)
              .value(move || draft.get())
              .on_change(move |v: &str| {
                draft.set(v.to_owned());
                br_m2.update_block_source_at(live_index(at), v);
              })
              .on_submit(move |_: &str| editing.set(false)),
          )
      }
    }))
}

// ── 6. Image Block (Delegated to `crate::image_blocks`) ──────────────────────

fn render_image_block_view(env: &BlockRenderEnv, at: Position, block: EditorBlock) -> impl IntoElement {
  crate::image_blocks::render_image_block_view(env, at, block)
}

fn mini_chip(label: &'static str, dark_mode: Signal<bool>, on_press: impl Fn() + 'static) -> impl IntoElement {
  button()
    .px_2()
    .py_1()
    .rounded(px(4.0))
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_press)
    .child(text(label).text_size(px(11.5)).text_color(move || palette(dark_mode.get()).text_primary))
}

// ── 7. Table Block (Content-Adaptive Columns + Free Multi-Cell Drag Select) ──

// ── 8. Divider Block ─────────────────────────────────────────────────────────

fn render_divider_block_view(env: &BlockRenderEnv, at: Position) -> impl IntoElement {
  let dark_mode = env.dark_mode;
  let br = env.bridge.clone();
  let marquee_selected = env.marquee_selected;
  button()
    .test_id("node-view-divider")
    .w_full()
    .py_4()
    .justify_center()
    .on_press(move || {
      marquee_selected.set(None);
      br.focus_block(live_index(at), 0);
    })
    .child(view().w_full().h(px(1.5)).bg(move || palette(dark_mode.get()).quote_bar))
}

// ── 9. Floating Zoom & Pan Lightbox Dialog (Mermaid & Images) ────────────────

pub fn render_lightbox_modal(
  lightbox: Signal<Option<LightboxContent>>,
  bridge: Rc<MicaDocumentBridge>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let zoom = signal(1.0_f32);
  let pan = signal((0.0_f32, 0.0_f32));
  let dragging = signal(false);
  let drag_start_page = signal((0.0_f32, 0.0_f32));
  let drag_start_pan = signal((0.0_f32, 0.0_f32));
  let canvas_size = signal((960.0_f32, 580.0_f32));
  let show_source = signal(false);

  when(move || lightbox.get().is_some(), {
    let br = bridge.clone();
    move || {
      let Some(content) = lightbox.get() else {
        return view().into_element();
      };
      let br_s = br.clone();
      let title_text = match &content {
        LightboxContent::Mermaid { .. } => "Mermaid 图表预览",
        LightboxContent::Image { alt, .. } if !alt.trim().is_empty() => "图片预览",
        LightboxContent::Image { .. } => "图片预览",
      };
      let is_mermaid = matches!(content, LightboxContent::Mermaid { .. });
      let base_size = signal((780.0_f32, 500.0_f32));
      let apply_zoom = move |target: f32| {
        let old_z = zoom.get_untracked().max(0.01);
        let new_z = target.clamp(0.25, 6.0);
        let ratio = new_z / old_z;
        let (ox, oy) = pan.get_untracked();
        zoom.set(new_z);
        pan.set((ox * ratio, oy * ratio));
      };
      let close_dialog = move || {
        zoom.set(1.0);
        pan.set((0.0, 0.0));
        dragging.set(false);
        show_source.set(false);
        lightbox.set(None);
      };

      // Interactive zoom/pan canvas shared by Mermaid diagrams and images.
      let build_zoom_pan_stage = move |stage_content: AnyElement| {
        view()
          .test_id("lightbox-pan-stage")
          .flex_1()
          .w_full()
          .min_h(px(0.0))
          .relative()
          .overflow_hidden()
          .bg(move || palette(dark_mode.get()).surface_sunken)
          .cursor(Cursor::Grab)
          .on_layout(move |b: &Bounds<Points>| {
            let (w, h) = (b.size.width.0, b.size.height.0);
            if w > 10.0 && h > 10.0 {
              let (pw, ph) = canvas_size.get_untracked();
              if (pw - w).abs() > 2.0 || (ph - h).abs() > 2.0 {
                canvas_size.set((w, h));
              }
            }
          })
          .on_wheel(move |ev: &WheelEvent| {
            let dy = match ev.delta {
              WheelDelta::Points(pt) => f32::from(pt.y),
              WheelDelta::Lines(pt) => pt.y * 24.0,
            };
            if dy.abs() > 0.01 {
              let factor = if dy < 0.0 { 1.12 } else { 1.0 / 1.12 };
              apply_zoom(zoom.get_untracked() * factor);
            }
          })
          .claim_touches(TouchClaim::on_start())
          .keep_touches(true)
          .on_responder_grant(move |e: &ResponderEvent| {
            dragging.set(true);
            drag_start_page.set((f32::from(e.page.x), f32::from(e.page.y)));
            drag_start_pan.set(pan.get_untracked());
          })
          .on_responder_move(move |e: &ResponderEvent| {
            if !dragging.get_untracked() {
              return;
            }
            let (sx, sy) = drag_start_page.get_untracked();
            let (px0, py0) = drag_start_pan.get_untracked();
            pan.set((px0 + f32::from(e.page.x) - sx, py0 + f32::from(e.page.y) - sy));
          })
          .on_responder_release(move |_: &ResponderEvent| {
            dragging.set(false);
          })
          .child(
            view()
              .absolute()
              .left(move || {
                let (cw, _) = canvas_size.get();
                let (ox, _) = pan.get();
                let (bw, _) = base_size.get();
                px((cw - bw * zoom.get()) * 0.5 + ox)
              })
              .top(move || {
                let (_, ch) = canvas_size.get();
                let (_, oy) = pan.get();
                let (_, bh) = base_size.get();
                px((ch - bh * zoom.get()) * 0.5 + oy)
              })
              .pointer_events(PointerEvents::None)
              .child(stage_content),
          )
          .child(
            view()
              .absolute()
              .left(px(12.0))
              .bottom(px(10.0))
              .px_2()
              .py_1()
              .rounded(px(6.0))
              .bg(move || palette(dark_mode.get()).surface_overlay)
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_subtle)
              .pointer_events(PointerEvents::None)
              .child(
                text("滚轮缩放 · 按住拖拽平移")
                  .text_size(px(11.5))
                  .text_color(move || palette(dark_mode.get()).text_faint),
              ),
          )
      };

      view()
        .test_id("mica-lightbox-backdrop")
        .absolute()
        .inset_0()
        .z_index(65)
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("lightbox-backdrop-dismiss")
            .accessibility_label("关闭预览弹窗")
            .absolute()
            .inset_0()
            .on_press(close_dialog),
        )
        .child(
          view()
            .test_id("mica-lightbox-modal")
            .w(relative(0.86))
            .max_w(px(1080.0))
            .h(relative(0.82))
            .max_h(px(740.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .overflow_hidden()
            .shadow_md()
            .z_index(66)
            .child(
              // Floating dialog header bar
              view()
                .w_full()
                .h(px(46.0))
                .flex_shrink_0()
                .px_4()
                .flex_row()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .child(
                  text(title_text)
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                )
                .child(
                  view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                      button()
                        .test_id("lightbox-zoom-out")
                        .px_2()
                        .py_1()
                        .rounded(px(4.0))
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                        .on_press(move || apply_zoom(zoom.get_untracked() / 1.25))
                        .child(
                          text("−")
                            .text_size(px(12.0))
                            .text_color(move || palette(dark_mode.get()).text_primary),
                        ),
                    )
                    .child(
                      text(move || format!("{}%", (zoom.get() * 100.0).round() as i32))
                        .text_size(px(12.5))
                        .text_color(move || palette(dark_mode.get()).text_muted),
                    )
                    .child(
                      button()
                        .test_id("lightbox-zoom-in")
                        .px_2()
                        .py_1()
                        .rounded(px(4.0))
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                        .on_press(move || apply_zoom(zoom.get_untracked() * 1.25))
                        .child(
                          text("+")
                            .text_size(px(12.0))
                            .text_color(move || palette(dark_mode.get()).text_primary),
                        ),
                    )
                    .child(
                      button()
                        .test_id("lightbox-zoom-reset")
                        .px_2()
                        .py_1()
                        .rounded(px(4.0))
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                        .on_press(move || {
                          zoom.set(1.0);
                          pan.set((0.0, 0.0));
                        })
                        .child(
                          text("重置")
                            .text_size(px(11.5))
                            .text_color(move || palette(dark_mode.get()).text_primary),
                        ),
                    )
                    .child(when(move || is_mermaid, move || {
                      mini_chip("源码", dark_mode, move || {
                        show_source.set(!show_source.get_untracked());
                      })
                    }))
                    .child(
                      button()
                        .test_id("lightbox-close-btn")
                        .px_3()
                        .py_1()
                        .rounded(px(6.0))
                        .bg(move || palette(dark_mode.get()).accent)
                        .on_press(close_dialog)
                        .child(text("关闭").text_size(px(12.5)).text_color(nsh_core::color::white())),
                    ),
                ),
            )
            .child(match content {
              LightboxContent::Mermaid { block_index, source } => {
                let live_source = signal(source.clone());
                let init_svg = render_mermaid_svg_for_theme(&source, dark_mode.get_untracked());
                let (init_w, init_h) = init_svg
                  .as_deref()
                  .map(|s| parse_svg_aspect_size(s, 960.0, 520.0))
                  .unwrap_or((680.0, 320.0));
                base_size.set((init_w, init_h));

                let diagram_el = dynamic(move || {
                  let _ = crate::preview_render::preview_rev().get();
                  let current_src = live_source.get();
                  if let Some(svg_markup) = render_mermaid_svg_for_theme(&current_src, dark_mode.get()) {
                    let (w0, h0) = parse_svg_aspect_size(&svg_markup, 960.0, 520.0);
                    let (pw, ph) = base_size.get_untracked();
                    if (pw - w0).abs() > 1.0 || (ph - h0).abs() > 1.0 {
                      base_size.set((w0, h0));
                    }
                    image(ImageSrc::svg_markup(&svg_markup))
                      .w(move || px(w0 * zoom.get()))
                      .h(move || px(h0 * zoom.get()))
                      .into_element()
                  } else {
                    text(current_src)
                      .text_size(px(13.0))
                      .text_color(move || palette(dark_mode.get()).text_primary)
                      .into_element()
                  }
                })
                .into_element();

                view()
                  .flex_1()
                  .w_full()
                  .min_h(px(0.0))
                  .child(build_zoom_pan_stage(diagram_el))
                  .child(when(move || show_source.get(), move || {
                    let br_e = br_s.clone();
                    let se = live_source.get_untracked();
                    view()
                      .w_full()
                      .h(px(200.0))
                      .flex_shrink_0()
                      .p_3()
                      .border_t_1()
                      .border_color(move || palette(dark_mode.get()).border_normal)
                      .bg(move || palette(dark_mode.get()).code_bg)
                      .child(
                        editor()
                          .w_full()
                          .h_full()
                          .value(Some(SharedString::from(se)))
                          .font_size(px(13.0))
                          .line_height(px(20.0))
                          .styles(move || code_editor_styles(dark_mode.get()))
                          .on_change(move |v: &SharedString| {
                            br_e.update_block_source_at(block_index, v.as_ref());
                            live_source.set(v.to_string());
                          }),
                      )
                  }))
                  .into_element()
              }
              LightboxContent::Image { url, alt, natural_size } => {
                let fit_box = |nat: Option<(f32, f32)>| -> (f32, f32) {
                  if let Some((nw, nh)) = nat
                    && nw > 1.0
                    && nh > 1.0
                  {
                    let scale = (860.0 / nw).min(560.0 / nh).min(1.0);
                    return ((nw * scale).max(80.0), (nh * scale).max(60.0));
                  }
                  (780.0, 500.0)
                };
                base_size.set(fit_box(natural_size));
                let img_el = image(ImageSrc::from(url))
                  .w(move || px(base_size.get().0 * zoom.get()))
                  .h(move || px(base_size.get().1 * zoom.get()))
                  .on_load(move |sz: &nsh_core::geometry::Size<nsh_core::geometry::DevicePixels>| {
                    let nw = sz.width.0 as f32;
                    let nh = sz.height.0 as f32;
                    if nw > 1.0 && nh > 1.0 {
                      base_size.set(fit_box(Some((nw, nh))));
                    }
                  })
                  .into_element();
                view()
                  .flex_1()
                  .w_full()
                  .min_h(px(0.0))
                  .child(build_zoom_pan_stage(img_el))
                  .child(when({ let has_a = !alt.trim().is_empty(); move || has_a }, {
                    let a = alt.clone();
                    move || {
                      view()
                        .w_full()
                        .flex_shrink_0()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(move || palette(dark_mode.get()).border_subtle)
                        .items_center()
                        .child(
                          text(a.clone())
                            .text_size(px(12.5))
                            .text_color(move || palette(dark_mode.get()).text_muted),
                        )
                    }
                  }))
                  .into_element()
              }
            }),
        )
        .into_element()
    }
  })
}

// ── Callout, Code Editor Styles & Small Helpers ──────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalloutKind {
  Note,
  Tip,
  Important,
  Warning,
  Caution,
}

impl CalloutKind {
  pub fn from_attr(raw: &str) -> Option<Self> {
    match raw.trim().to_ascii_lowercase().as_str() {
      "note" => Some(Self::Note),
      "tip" => Some(Self::Tip),
      "important" => Some(Self::Important),
      "warning" => Some(Self::Warning),
      "caution" => Some(Self::Caution),
      _ => None,
    }
  }

  pub fn label(self) -> &'static str {
    match self {
      Self::Note => "Note",
      Self::Tip => "Tip",
      Self::Important => "Important",
      Self::Warning => "Warning",
      Self::Caution => "Caution",
    }
  }

  pub fn accent(self, dark: bool) -> (Rgba, &'static str) {
    let p = palette(dark);
    match self {
      Self::Note => (p.alert_note, if dark { "#58A6FF" } else { "#0969DA" }),
      Self::Tip => (p.alert_tip, if dark { "#3FB950" } else { "#1A7F37" }),
      Self::Important => (p.alert_important, if dark { "#A371F7" } else { "#8250DF" }),
      Self::Warning => (p.alert_warning, if dark { "#D29922" } else { "#9A6700" }),
      Self::Caution => (p.alert_caution, if dark { "#F85149" } else { "#CF222E" }),
    }
  }
}

fn code_editor_styles(dark: bool) -> EditorStyles {
  let p = palette(dark);
  let mut styles = EditorStyles::default();
  styles.content = Some(PartStyle(Style::from_decls([
    StyleDecl::Set(StyleKey::BackgroundColor, PropertyValue::Color(Some(Color::from(p.code_bg)))),
    StyleDecl::Set(StyleKey::Color, PropertyValue::Color(Some(Color::from(p.text_primary)))),
  ])));
  styles.gutter = Some(PartStyle(Style::from_decls([
    StyleDecl::Set(StyleKey::BackgroundColor, PropertyValue::Color(Some(Color::from(p.code_bg)))),
    StyleDecl::Set(StyleKey::Color, PropertyValue::Color(Some(Color::from(p.text_faint)))),
  ])));
  styles.selection = Some(PartStyle(Style::from_decls([
    StyleDecl::Set(StyleKey::BackgroundColor, PropertyValue::Color(Some(Color::from(p.selection_bg)))),
  ])));
  styles
}

fn copy_with_reset(text: String, copied: Signal<bool>, copy_gen: Signal<u64>) {
  set_clipboard(text);
  copied.set(true);
  let next_gen = copy_gen.get_untracked().wrapping_add(1);
  copy_gen.set(next_gen);
  spawn(async move {
    timer(Duration::from_millis(1600)).await;
    if copy_gen.get_untracked() == next_gen {
      copied.set(false);
    }
  });
}

fn stroked_icon(color: &str, body: &str) -> Svg {
  svg(ImageSrc::svg_markup(&format!(
    r##"<svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="{color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##
  )))
  .w(px(14.0))
  .h(px(14.0))
}

fn icon_lightbulb(c: &str) -> Svg {
  stroked_icon(c, r##"<path d="M9 18h6"/><path d="M10 22h4"/><path d="M12 2a7 7 0 0 0-4 12.7V17h8v-2.3A7 7 0 0 0 12 2z"/>"##)
}
fn icon_important_badge(c: &str) -> Svg {
  stroked_icon(c, r##"<circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>"##)
}
fn icon_warning_triangle(c: &str) -> Svg {
  stroked_icon(c, r##"<path d="M10.3 3.9L1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>"##)
}
fn icon_caution_octagon(c: &str) -> Svg {
  stroked_icon(c, r##"<polygon points="7.9 2 16.1 2 22 7.9 22 16.1 16.1 22 7.9 22 2 16.1 2 7.9 7.9 2"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/>"##)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn callout_kind_and_svg_viewbox_parsing() {
    assert_eq!(CalloutKind::from_attr("NOTE"), Some(CalloutKind::Note));
    assert_eq!(CalloutKind::from_attr("tip"), Some(CalloutKind::Tip));
    assert_eq!(CalloutKind::from_attr("unknown"), None);
    let (w, h) = parse_svg_aspect_size(r#"<svg viewBox="0 0 800 200"></svg>"#, 680.0, 320.0);
    assert!((w - 680.0).abs() < 1.0);
    assert!((h - 170.0).abs() < 1.0);
  }

  #[test]
  fn quote_group_edges_join_adjacent_quotes_and_break_on_qbreak_or_callout() {
    let q1 = EditorBlock::new("q1", "quote", "line 1");
    let q2 = EditorBlock::new("q2", "quote", "line 2");
    let mut q3_break = EditorBlock::new("q3", "quote", "separate quote");
    q3_break.data = serde_json::json!({ "qbreak": true });
    let mut q4_callout = EditorBlock::new("q4", "quote", "note");
    q4_callout.data = serde_json::json!({ "alert": "note" });

    assert_eq!(quote_group_edges(None, &q1, Some(&q2)), (false, true));
    assert_eq!(quote_group_edges(Some(&q1), &q2, Some(&q3_break)), (true, false));
    assert_eq!(quote_group_edges(Some(&q2), &q3_break, Some(&q4_callout)), (false, false));
    assert_eq!(quote_group_edges(Some(&q3_break), &q4_callout, None), (false, false));
  }

  #[test]
  fn list_continuation_inset_matches_list_text_column() {
    let plain = EditorBlock::new("b0", "code_block", "fn main() {}");
    assert_eq!(list_continuation_inset_px(&plain), 0.0);

    for (li, expected) in [(0u32, 26.0f32), (1, 50.0), (2, 74.0)] {
      let mut blk = EditorBlock::new("b1", "code_block", "let x = 1;");
      blk.data = serde_json::json!({ "li": li });
      assert_eq!(list_continuation_inset_px(&blk), expected);
    }
  }

  #[test]
  fn row_text_origin_offset_accounts_for_code_block_quote_and_list_padding() {
    let mut code = EditorBlock::new("c0", "code_block", "fn main() {}");
    assert_eq!(row_text_origin_offset(&code, 100.0, (0.0, 0.0)), (114.0, 20.0));

    code.data = serde_json::json!({ "lineNumbers": true, "li": 0, "title": "demo.rs" });
    assert_eq!(row_text_origin_offset(&code, 100.0, (0.0, 0.0)), (168.0, 42.0));

    let quote = EditorBlock::new("q0", "quote", "cited");
    assert_eq!(row_text_origin_offset(&quote, 100.0, (0.0, 0.0)), (117.0, 8.0));

    let mut bullet = EditorBlock::new("l0", "bulleted_list", "item");
    bullet.data = serde_json::json!({ "indent": 1 });
    assert_eq!(row_text_origin_offset(&bullet, 100.0, (0.0, 2.0)), (150.0, 2.0));
  }

  #[test]
  fn mermaid_cjk_svg_rasterizes_dark_text_pixels() {
    let src = "flowchart LR\n  P0[阶段0 准备与到货验收] --> P1[阶段1 上架与布线]";
    let svg = render_mermaid_svg_for_theme(src, false).expect("merman should render flowchart");
    assert!(svg.contains("阶段0"), "SVG should contain Chinese label text");
    assert!(svg.contains("<text"), "resvg-safe Mermaid SVG should contain <text> nodes");
    let raster = nsh_core::svg::rasterize(svg.as_bytes(), 900)
      .expect("nsh_core::svg::rasterize should rasterize Mermaid SVG");
    let dark_pixels = raster
      .pixels
      .as_chunks::<4>()
      .0
      .iter()
      .filter(|px| px[3] > 200 && px[0] < 80 && px[1] < 80 && px[2] < 80)
      .count();
    assert!(
      dark_pixels > 150,
      "Chinese Mermaid labels must render visible dark glyph pixels (got {dark_pixels} dark pixels on {}x{}, svg snippet: {})",
      raster.width,
      raster.height,
      &svg[..svg.len().min(600)]
    );
  }

  #[test]
  fn build_inline_runs_marks_highlight_and_combinations() {
    let marks = vec![
      EditorMark::new(2, 6, "highlight"),
      EditorMark::new(2, 4, "bold"),
    ];
    let runs = build_inline_runs("这是核心重点内容", &marks, None);
    assert_eq!(runs.len(), 4);
    assert_eq!(runs[0].text, "这是");
    assert!(!runs[0].highlight);
    assert_eq!(runs[1].text, "核心");
    assert!(runs[1].highlight && runs[1].bold);
    assert_eq!(runs[2].text, "重点");
    assert!(runs[2].highlight && !runs[2].bold);
    assert_eq!(runs[3].text, "内容");
    assert!(!runs[3].highlight);
  }

  #[test]
  fn resolve_link_target_validates_external_mica_and_rejects_unsafe_schemes() {
    assert_eq!(
      resolve_link_target("https://docs.nvidia.com/dgx/"),
      Some(LinkTarget::ExternalUrl("https://docs.nvidia.com/dgx/".to_owned()))
    );
    assert_eq!(
      resolve_link_target("www.example.com/docs"),
      Some(LinkTarget::ExternalUrl("https://www.example.com/docs".to_owned()))
    );
    let page_id = uuid::Uuid::new_v4();
    assert_eq!(
      resolve_link_target(&format!("mica://page/{page_id}")),
      Some(LinkTarget::MicaPage(page_id))
    );
    assert_eq!(resolve_link_target("javascript:alert(1)"), None);
    assert_eq!(resolve_link_target("data:text/html,hi"), None);
    assert_eq!(resolve_link_target("   "), None);
  }

  #[test]
  fn hit_link_in_lines_hits_link_rect_and_ignores_trailing_blank_margin() {
    let text = "https://docs.nvidia.com/dgx/";
    let mut link = EditorMark::new(0, utf16_len(text), "link");
    link.href = Some(text.to_owned());
    let marks = vec![link];
    let measure = |_: &str, s: &str| Points((s.chars().count() as f32) * 8.0);
    let lines = crate::text_geometry::synthesize_wrapped_lines(text, 600.0, "400 15px sans-serif", 24.0, &measure);
    // Inside the link text (width ~224px) -> hits link
    assert_eq!(
      hit_link_in_lines(&lines, text, &marks, 64.0, 10.0, "400 15px sans-serif", 24.0, &measure).as_deref(),
      Some(text)
    );
    // Blank whitespace well to the right of the link -> returns None so user can click end-of-line
    assert_eq!(
      hit_link_in_lines(&lines, text, &marks, 320.0, 10.0, "400 15px sans-serif", 24.0, &measure),
      None
    );
  }

  #[test]
  fn resolve_code_block_language_display_detects_bash_and_hides_auto() {
    let bash_cmd = "ping -D 8.8.8.8 | tee -a /tmp/fgt_upgrade_ping_$(date +%Y%m%d_%H%M).log";
    assert_eq!(
      resolve_code_block_language_display("", bash_cmd),
      ("bash".to_string(), "bash".to_string())
    );
    assert_eq!(
      resolve_code_block_language_display("auto", bash_cmd),
      ("bash".to_string(), "bash".to_string())
    );
    assert_eq!(
      resolve_code_block_language_display("", ""),
      ("plaintext".to_string(), "auto".to_string())
    );
    assert_eq!(
      resolve_code_block_language_display("python", bash_cmd),
      ("python".to_string(), "python".to_string())
    );
  }
}
