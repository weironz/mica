//! Code block view: warm-gray surface, hover toolbar, top-level overlay
//! language/AI/more pickers, lazy `nsh_base::editor()` focus mounting, and
//! incremental syntax highlighting.

use std::{cell::RefCell, rc::Rc, time::Duration};

use mica_editor::{
  DocPosition, DocSelection,
  utf16::{utf16_len, utf16_slice},
};
use nsh_base::{button, input};
use nsh_core::{
  SubmitBehavior,
  color::Rgba,
  element::{Align, ElementRef, Placement, element_ref, overlay, text_measurer},
  geometry::{Bounds, Point, Points, px},
  prelude::*,
};
use serde_json::Value;

use crate::{
  block_views::{
    BlockRenderEnv, CodeAiAction, CodeAiCallback, TRANSPARENT_RGBA, TextMeasureFn, copy_with_reset,
    live_index, mini_chip,
  },
  mica_icons,
  mica_theme::palette,
  text_geometry::{VisibleLayoutCache, caret_rect_for_offset, offset_at_point, selection_rects_for_range},
};

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

pub(crate) fn render_code_block_view(
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
      let cache = layouts_ref.borrow();
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
      offset_at_point(&lines, lx, ly, code_font, &|f, t| meas(f, t)).min(utf16_len(block_text))
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
          let cache = layouts_sel.borrow();
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
          let lines: &[nsh_core::TextLine] = &lines;

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
