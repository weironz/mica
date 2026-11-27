//! Caret & selection visual anchor measurement for Mica's virtualized editor:
//! - Floating selection bubble menu positioning (above/below selected text)
//! - Caret-anchored `/` slash menu positioning (below/above caret)
//! - Hidden native IME proxy positioning (tracks active caret so OS CJK
//!   candidate windows open right beside the caret rather than at `(0, 0)`)
//! - Cross-block vertical arrow navigation preserving visual `preferred_x`

use std::{
  cell::{Cell, RefCell},
  rc::Rc,
  time::Duration,
};

use mica_editor::{DocPosition, DocSelection, EditorCommand, utf16::utf16_len};
use nsh_base::{button, input};
use nsh_core::{
  SubmitBehavior,
  element::{ElementRef, ImageSrc, svg, text_measurer},
  geometry::px,
  prelude::*,
};

use crate::{
  editor_bridge::MicaDocumentBridge,
  mica_theme::palette,
  text_geometry::{
    VisibleLayoutCache, caret_rect_for_offset, offset_at_point, resolve_lines_for_block,
    selection_rects_for_range, vertical_move_offset,
  },
};

/// Schedules a coalesced async measurement to place the hidden native IME input
/// sink at the active caret's local coordinates inside `paper_ref`, so
/// Windows/macOS CJK IME candidate windows follow the typing caret accurately
/// without flooding the platform measure bridge during rapid key-repeat bursts.
pub fn schedule_ime_sink_pos_update(
  bridge: &MicaDocumentBridge,
  layouts: &Rc<RefCell<VisibleLayoutCache>>,
  paper_ref: ElementRef,
  pos: DocPosition,
  ime_sink_pos: Signal<(f32, f32)>,
) {
  thread_local! {
    static IME_POS_GEN: Cell<u64> = const { Cell::new(0) };
  }
  let seq = IME_POS_GEN.with(|c| {
    let next = c.get().wrapping_add(1);
    c.set(next);
    next
  });
  let Some(bid) = bridge.block_id_at(pos.node) else {
    return;
  };
  let (text_ref, row_ref, caret_local) = {
    let mut cache = layouts.borrow_mut();
    let tref = cache.text_refs.get(&bid).copied();
    let rref = cache.row_refs.get(&bid).map(|(_, r)| *r);
    let measure = text_measurer();
    let block_text = bridge.block_at(pos.node).map(|b| b.text).unwrap_or_default();
    let meta = cache
      .blocks
      .get(&bid)
      .map(|info| (info.font.clone(), info.line_height));
    let caret = meta.map(|(font, lh)| {
      let lines = cache.resolve_lines_cached(
        &bid,
        &block_text,
        760.0,
        &font,
        lh.max(24.0),
        &measure,
      );
      caret_rect_for_offset(lines, pos.offset, &font, lh, &measure)
    });
    (tref, rref, caret)
  };
  if text_ref.is_none() && row_ref.is_none() {
    return;
  }
  spawn(async move {
    timer(Duration::from_millis(16)).await;
    if IME_POS_GEN.with(Cell::get) != seq {
      return;
    }
    let Some(pm) = paper_ref.measure().await else {
      return;
    };
    if IME_POS_GEN.with(Cell::get) != seq {
      return;
    }
    let vw = pm.frame.size.width.0.max(120.0);
    let vh = pm.frame.size.height.0.max(80.0);
    let anchor_m = if let Some(tref) = text_ref {
      tref.measure().await
    } else if let Some(rref) = row_ref {
      rref.measure().await
    } else {
      None
    };
    if IME_POS_GEN.with(Cell::get) != seq {
      return;
    }
    let Some(tm) = anchor_m else {
      return;
    };
    let base_x = tm.page.x.0 - pm.page.x.0;
    let base_y = tm.page.y.0 - pm.page.y.0;
    let (cx, cy) = match caret_local {
      Some((lx, ly, _)) => (base_x + lx, base_y + ly),
      None => (base_x, base_y),
    };
    let clamped_x = cx.clamp(4.0, (vw - 8.0).max(4.0));
    let clamped_y = cy.clamp(4.0, (vh - 24.0).max(4.0));
    ime_sink_pos.set((clamped_x, clamped_y));
  });
}

/// Schedules an async measurement to anchor the `/` slash command menu directly
/// below (or above, when near the bottom edge) the active caret.
pub fn schedule_slash_anchor_update(
  bridge: &MicaDocumentBridge,
  layouts: &Rc<RefCell<VisibleLayoutCache>>,
  paper_ref: ElementRef,
  menu_w: f32,
  menu_h: f32,
  anchor_sig: Signal<(f32, f32)>,
) {
  let Some(sel) = bridge.selection() else {
    return;
  };
  let pos = sel.focus_pos();
  let Some(bid) = bridge.block_id_at(pos.node) else {
    return;
  };
  let (text_ref, row_ref, caret_local) = {
    let cache = layouts.borrow();
    let tref = cache.text_refs.get(&bid).copied();
    let rref = cache.row_refs.get(&bid).map(|(_, r)| *r);
    let measure = text_measurer();
    let block_text = bridge.block_at(pos.node).map(|b| b.text).unwrap_or_default();
    let caret = cache.blocks.get(&bid).map(|info| {
      let lines = resolve_lines_for_block(
        Some(info),
        &block_text,
        760.0,
        &info.font,
        info.line_height.max(24.0),
        &measure,
      );
      caret_rect_for_offset(&lines, pos.offset, &info.font, info.line_height, &measure)
    });
    (tref, rref, caret)
  };
  spawn(async move {
    let Some(pm) = paper_ref.measure().await else {
      return;
    };
    let vw = pm.frame.size.width.0.max(menu_w + 32.0);
    let vh = pm.frame.size.height.0.max(menu_h + 32.0);
    let tm = if let Some(tref) = text_ref {
      tref.measure().await
    } else if let Some(rref) = row_ref {
      rref.measure().await
    } else {
      None
    };
    let (cx, caret_top, caret_bottom) = if let Some(m) = tm {
      let bx = m.page.x.0 - pm.page.x.0;
      let by = m.page.y.0 - pm.page.y.0;
      match caret_local {
        Some((lx, ly, lh)) => (bx + lx, by + ly, by + ly + lh.max(20.0)),
        None => (bx, by, by + 24.0),
      }
    } else {
      ((vw - menu_w) * 0.5, 56.0, 80.0)
    };
    let left = cx.clamp(12.0, (vw - menu_w - 12.0).max(12.0));
    let below = caret_bottom + 6.0;
    let above = caret_top - menu_h - 6.0;
    let top = if below + menu_h <= vh - 8.0 || above < 8.0 {
      below.clamp(8.0, (vh - menu_h - 8.0).max(8.0))
    } else {
      above.max(8.0)
    };
    anchor_sig.set((left, top));
  });
}

/// Schedules an async measurement to position the floating selection bubble
/// toolbar directly ABOVE (or below, when near top edge) the selected text.
pub fn schedule_bubble_anchor_update(
  bridge: &MicaDocumentBridge,
  layouts: &Rc<RefCell<VisibleLayoutCache>>,
  paper_ref: ElementRef,
  page_width: f32,
  sel: DocSelection,
  bubble_anchor: Signal<(f32, f32)>,
) {
  let start_p = sel.start_pos();
  let end_p = sel.end_pos();
  let focus_p = sel.focus_pos();

  let pick_block = |idx: usize| -> Option<(usize, String, ElementRef, Option<ElementRef>)> {
    let bid = bridge.block_id_at(idx)?;
    let cache = layouts.borrow();
    let (_, row_ref) = *cache.row_refs.get(&bid)?;
    let text_ref = cache.text_refs.get(&bid).copied();
    Some((idx, bid, row_ref, text_ref))
  };

  let Some((target_idx, bid, row_ref, text_ref)) =
    pick_block(start_p.node).or_else(|| pick_block(focus_p.node))
  else {
    return;
  };

  let local_rect_opt = {
    let block_text = bridge
      .block_at(target_idx)
      .map(|b| b.text)
      .unwrap_or_default();
    let b_len = utf16_len(&block_text);
    let s_u16 = if target_idx == start_p.node { start_p.offset } else { 0 };
    let e_u16 = if target_idx == end_p.node { end_p.offset } else { b_len.max(1) };
    let cache = layouts.borrow();
    let measure = text_measurer();
    cache.blocks.get(&bid).and_then(|info| {
      let fallback_w = if crate::page_width::is_full_width(page_width) {
        820.0
      } else {
        (page_width - 48.0).max(200.0)
      };
      let lines = resolve_lines_for_block(
        Some(info),
        &block_text,
        fallback_w,
        &info.font,
        info.line_height.max(24.0),
        &measure,
      );
      let rects = selection_rects_for_range(
        &lines,
        s_u16,
        e_u16,
        &info.font,
        info.line_height,
        &measure,
      );
      rects.first().copied()
    })
  };

  spawn(async move {
    let Some(pm) = paper_ref.measure().await else {
      return;
    };
    let vw = pm.frame.size.width.0.max(260.0);
    let vh = pm.frame.size.height.0.max(200.0);
    let bubble_w: f32 = if vw < 640.0 { 236.0 } else { 468.0 };
    const BUBBLE_H: f32 = 40.0;

    let anchor_m = if let Some(tref) = text_ref {
      tref.measure().await
    } else {
      None
    };
    let (sel_center_x, sel_top, sel_bottom) = if let Some(tm) = anchor_m {
      let base_x = tm.page.x.0 - pm.page.x.0;
      let base_y = tm.page.y.0 - pm.page.y.0;
      if let Some((rx, ry, rw, rh)) = local_rect_opt {
        (base_x + rx + rw * 0.5, base_y + ry, base_y + ry + rh)
      } else {
        (
          base_x + tm.frame.size.width.0 * 0.5,
          base_y,
          base_y + tm.frame.size.height.0.min(36.0),
        )
      }
    } else if let Some(rm) = row_ref.measure().await {
      let base_y = rm.page.y.0 - pm.page.y.0;
      let _ = crate::page_width::rendered_column_width(page_width, (vw - 24.0).max(0.0));
      (vw * 0.5, base_y, base_y + rm.frame.size.height.0.min(48.0))
    } else {
      return;
    };

    let left = (sel_center_x - bubble_w * 0.5).clamp(12.0, (vw - bubble_w - 12.0).max(12.0));
    let above_top = sel_top - BUBBLE_H - 8.0;
    let below_top = sel_bottom + 8.0;
    let top = if above_top >= 8.0 {
      above_top.min((vh - BUBBLE_H - 8.0).max(8.0))
    } else {
      below_top.clamp(8.0, (vh - BUBBLE_H - 8.0).max(8.0))
    };
    bubble_anchor.set((left, top));
  });
}

/// Schedules an async measurement to place the start (left) and end (right)
/// mobile teardrop selection handles at the exact visual edges of `sel` inside
/// `paper_ref` (`CAP-16-06`).
pub fn schedule_touch_handles_update(
  bridge: &MicaDocumentBridge,
  layouts: &Rc<RefCell<VisibleLayoutCache>>,
  paper_ref: ElementRef,
  sel: DocSelection,
  anchors: Signal<crate::touch_selection::TouchHandleAnchors>,
) {
  if sel.is_collapsed() {
    anchors.set(crate::touch_selection::TouchHandleAnchors::default());
    return;
  }
  let start_p = sel.start_pos();
  let end_p = sel.end_pos();

  struct HandleEndpointRef {
    row_ref: Option<ElementRef>,
    text_ref: Option<ElementRef>,
    local_pt: Option<(f32, f32)>,
  }

  let measure = text_measurer();
  let endpoint_info = |pos: DocPosition, is_start: bool| -> Option<HandleEndpointRef> {
    let bid = bridge.block_id_at(pos.node)?;
    let cache = layouts.borrow();
    let row_ref = cache.row_refs.get(&bid).map(|(_, r)| *r);
    let text_ref = cache.text_refs.get(&bid).copied();
    let block_text = bridge.block_at(pos.node).map(|b| b.text).unwrap_or_default();
    let b_len = utf16_len(&block_text);
    let local_pt = cache.blocks.get(&bid).and_then(|info| {
      let (s_u16, e_u16) = if start_p.node == end_p.node {
        (start_p.offset, end_p.offset)
      } else if is_start {
        (start_p.offset, b_len.max(start_p.offset.saturating_add(1)))
      } else {
        (0, end_p.offset.max(1))
      };
      let lines = resolve_lines_for_block(
        Some(info),
        &block_text,
        760.0,
        &info.font,
        info.line_height.max(24.0),
        &measure,
      );
      let rects = selection_rects_for_range(
        &lines,
        s_u16,
        e_u16,
        &info.font,
        info.line_height,
        &measure,
      );
      if is_start {
        rects.first().map(|&(rx, ry, _, rh)| (rx, ry + rh))
      } else {
        rects.last().map(|&(rx, ry, rw, rh)| (rx + rw, ry + rh))
      }
    });
    Some(HandleEndpointRef {
      row_ref,
      text_ref,
      local_pt,
    })
  };

  let start_ep = endpoint_info(start_p, true);
  let end_ep = endpoint_info(end_p, false);

  spawn(async move {
    let Some(pm) = paper_ref.measure().await else {
      return;
    };
    let vw = pm.frame.size.width.0.max(200.0);
    let vh = pm.frame.size.height.0.max(120.0);

    let resolve_ep = async |ep: Option<HandleEndpointRef>, is_start: bool| -> Option<(f32, f32)> {
      let HandleEndpointRef {
        row_ref,
        text_ref,
        local_pt,
      } = ep?;
      let (ex, ey) = if let Some(tref) = text_ref
        && let Some(tm) = tref.measure().await
      {
        let bx = tm.page.x.0 - pm.page.x.0;
        let by = tm.page.y.0 - pm.page.y.0;
        match local_pt {
          Some((lx, ly)) => (bx + lx, by + ly),
          None if is_start => (bx, by + tm.frame.size.height.0.min(28.0)),
          None => (bx + tm.frame.size.width.0, by + tm.frame.size.height.0),
        }
      } else if let Some(rref) = row_ref
        && let Some(rm) = rref.measure().await
      {
        let bx = rm.page.x.0 - pm.page.x.0;
        let by = rm.page.y.0 - pm.page.y.0;
        if is_start {
          (bx + 24.0, by + rm.frame.size.height.0)
        } else {
          (bx + (rm.frame.size.width.0 - 24.0).max(24.0), by + rm.frame.size.height.0)
        }
      } else {
        return None;
      };
      if ey < -12.0 || ey > vh + 12.0 {
        return None;
      }
      Some((ex.clamp(8.0, (vw - 8.0).max(8.0)), ey.clamp(0.0, vh)))
    };

    let start = resolve_ep(start_ep, true).await;
    let end = resolve_ep(end_ep, false).await;
    anchors.set(crate::touch_selection::TouchHandleAnchors { start, end });
  });
}

/// Moves the caret vertically (`dir = -1` Up, `+1` Down) while preserving the
/// horizontal visual column `preferred_x` across both wrapped lines AND
/// consecutive top-level blocks (CAP-01-03).
pub fn move_caret_vertical_preserving_x(
  bridge: &MicaDocumentBridge,
  layouts: &Rc<RefCell<VisibleLayoutCache>>,
  preferred_x: Signal<Option<f32>>,
  dir: i32,
  shift: bool,
) {
  let Some(sel) = bridge.selection() else {
    return;
  };
  let focus = sel.focus_pos();
  let bi = focus.node;
  let count = bridge.block_len();
  let Some(block) = bridge.block_at(bi) else {
    return;
  };

  let measure = text_measurer();
  let desired_x = preferred_x.get_untracked().or_else(|| {
    let cache = layouts.borrow();
    cache.get(&block.id).map(|layout| {
      let (cx, _, _) = caret_rect_for_offset(
        &layout.lines,
        focus.offset,
        &layout.font,
        layout.line_height,
        &measure,
      );
      cx
    })
  });
  if preferred_x.get_untracked().is_none()
    && let Some(dx) = desired_x
  {
    preferred_x.set(Some(dx));
  }

  // 1. Try moving within wrapped lines of the current block first.
  if let Some(layout) = layouts.borrow().get(&block.id)
    && let Some(next_off) = vertical_move_offset(
      &layout.lines,
      focus.offset,
      dir,
      desired_x,
      &layout.font,
      &measure,
    )
  {
    let next = DocPosition::new(bi, next_off);
    let anchor = if shift { sel.anchor_pos() } else { next };
    bridge.set_selection(DocSelection::from_positions(anchor, next));
    return;
  }

  // 2. Step into the previous / next block and land at the visually nearest
  //    column (`desired_x`) on its bottom line (moving up) or top line (moving down).
  let target_bi = if dir < 0 {
    bi.saturating_sub(1)
  } else {
    (bi + 1).min(count.saturating_sub(1))
  };
  if target_bi == bi {
    return;
  }
  let Some(target_block) = bridge.block_at(target_bi) else {
    return;
  };
  let target_len = utf16_len(&target_block.text);
  let target_off = if let (Some(dx), Some(t_layout)) =
    (desired_x, layouts.borrow().get(&target_block.id))
    && !t_layout.lines.is_empty()
  {
    let target_line = if dir < 0 {
      t_layout.lines.last().unwrap()
    } else {
      &t_layout.lines[0]
    };
    let mid_y = target_line.y.0 + target_line.height.0 * 0.5;
    offset_at_point(&t_layout.lines, dx, mid_y, &t_layout.font, &measure).min(target_len)
  } else {
    focus.offset.min(target_len)
  };
  let next = DocPosition::new(target_bi, target_off);
  let anchor = if shift { sel.anchor_pos() } else { next };
  bridge.set_selection(DocSelection::from_positions(anchor, next));
  bridge.request_keep_block_visible(target_bi);
}

/// Active inline math (`$...$`) click-to-edit state (`CAP-07-05`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineMathEdit {
  pub block_index: usize,
  pub start: u32,
  pub end: u32,
  pub draft: String,
}

/// Interactive floating popover for inline LaTeX formulas (`$...$`):
/// shows a live RaTeX SVG preview alongside a LaTeX source editor with
/// `保存` / `删除` / `取消` actions (`CAP-07-05`).
pub fn inline_math_popover_view(
  edit_sig: Signal<Option<InlineMathEdit>>,
  anchor: Signal<(f32, f32)>,
  bridge: Rc<MicaDocumentBridge>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  when(move || edit_sig.get().is_some(), move || {
    let br_save = bridge.clone();
    let br_enter = bridge.clone();
    let br_del = bridge.clone();
    let br_cancel = bridge.clone();
    let (ax, ay) = anchor.get();
    let left = ax.clamp(12.0, 420.0);
    let top = (ay + 28.0).max(16.0);

    view()
      .test_id("inline-math-popover")
      .absolute()
      .left(px(left))
      .top(px(top))
      .w(px(320.0))
      .p(px(10.0))
      .gap(px(8.0))
      .rounded(px(10.0))
      .border_1()
      .border_color(move || palette(dark_mode.get()).border_normal)
      .bg(move || palette(dark_mode.get()).surface_overlay)
      .z_index(40)
      .child(
        // Live RaTeX inline SVG preview
        view()
          .w_full()
          .min_h(px(34.0))
          .px_2()
          .py_1()
          .rounded(px(6.0))
          .bg(move || palette(dark_mode.get()).surface_sunken)
          .items_center()
          .justify_center()
          .child(dynamic(move || {
            let _ = crate::preview_render::preview_rev().get();
            let src = edit_sig.get().map(|e| e.draft).unwrap_or_default();
            let p = palette(dark_mode.get());
            if src.trim().is_empty() {
              text("输入 LaTeX 行内公式")
                .text_size(px(12.0))
                .text_color(p.text_faint)
                .into_element()
            } else if let Some(m) = crate::preview_render::render_math_cached(
              src.trim(),
              false,
              p.text_primary_hex,
            ) {
              let w = (m.width * 18.0).clamp(16.0, 290.0) as f32;
              let h = (m.height * 18.0).clamp(18.0, 84.0) as f32;
              svg(ImageSrc::svg_markup(&m.svg)).w(px(w)).h(px(h)).into_element()
            } else {
              text(src)
                .text_size(px(13.0))
                .text_color(p.accent)
                .into_element()
            }
          })),
      )
      .child(
        input()
          .test_id("inline-math-input")
          .placeholder("例如: E = mc^2")
          .submit_behavior(SubmitBehavior::Submit)
          .w_full()
          .h(px(30.0))
          .px_2()
          .rounded(px(6.0))
          .border_1()
          .border_color(move || palette(dark_mode.get()).border_normal)
          .bg(move || palette(dark_mode.get()).surface_base)
          .text_size(px(12.5))
          .text_color(move || palette(dark_mode.get()).text_primary)
          .value(move || edit_sig.get().map(|e| e.draft).unwrap_or_default())
          .on_change(move |v: &str| {
            if let Some(mut cur) = edit_sig.get_untracked() {
              cur.draft = v.to_owned();
              edit_sig.set(Some(cur));
            }
          })
          .on_submit(move |v: &str| {
            if let Some(cur) = edit_sig.get_untracked() {
              br_enter.dispatch(EditorCommand::SetInlineMath {
                block_index: cur.block_index,
                start: cur.start,
                end: cur.end,
                source: v.trim().to_owned(),
              });
              edit_sig.set(None);
              br_enter.request_focus();
            }
          }),
      )
      .child(
        view()
          .w_full()
          .flex_row()
          .items_center()
          .justify_between()
          .child(
            button()
              .test_id("inline-math-delete")
              .h(px(26.0))
              .px_2()
              .rounded(px(5.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                if let Some(cur) = edit_sig.get_untracked() {
                  br_del.dispatch(EditorCommand::SetInlineMath {
                    block_index: cur.block_index,
                    start: cur.start,
                    end: cur.end,
                    source: String::new(),
                  });
                  edit_sig.set(None);
                  br_del.request_focus();
                }
              })
              .child(
                text("删除")
                  .text_size(px(12.0))
                  .text_color(move || palette(dark_mode.get()).danger),
              ),
          )
          .child(
            view()
              .flex_row()
              .items_center()
              .gap(px(6.0))
              .child(
                button()
                  .test_id("inline-math-cancel")
                  .h(px(26.0))
                  .px_2()
                  .rounded(px(5.0))
                  .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                  .on_press(move || {
                    edit_sig.set(None);
                    br_cancel.request_focus();
                  })
                  .child(
                    text("取消")
                      .text_size(px(12.0))
                      .text_color(move || palette(dark_mode.get()).text_muted),
                  ),
              )
              .child(
                button()
                  .test_id("inline-math-save")
                  .h(px(26.0))
                  .px_3()
                  .rounded(px(5.0))
                  .bg(move || palette(dark_mode.get()).accent)
                  .on_press(move || {
                    if let Some(cur) = edit_sig.get_untracked() {
                      br_save.dispatch(EditorCommand::SetInlineMath {
                        block_index: cur.block_index,
                        start: cur.start,
                        end: cur.end,
                        source: cur.draft.trim().to_owned(),
                      });
                      edit_sig.set(None);
                      br_save.request_focus();
                    }
                  })
                  .child(text("保存").text_size(px(12.0)).text_color(rgb(0xFFFFFF))),
              ),
          ),
      )
  })
}

/// Computes the minimal new `scroll_offset_y` needed to keep a vertical target
/// span `[rel_top, rel_bottom]` (relative to the viewport top) visible inside a
/// viewport of height `viewport_h`, returning `None` when no scroll is needed.
///
/// Crucially, when editing near the bottom of a tall code block whose top half
/// is scrolled above the viewport, the caller passes the active **caret** span
/// rather than the whole block's bounds — so pressing Enter never yanks the
/// hidden top of the code block back into view!
pub fn compute_keep_visible_scroll_target(
  cur_scroll_y: f32,
  viewport_h: f32,
  rel_top: f32,
  rel_bottom: f32,
) -> Option<f32> {
  if viewport_h <= 1.0 {
    return None;
  }
  let top_pad = 12.0_f32;
  let bottom_pad = 16.0_f32;
  if rel_top >= top_pad && rel_bottom <= viewport_h - bottom_pad {
    return None;
  }
  let target_h = (rel_bottom - rel_top).max(0.0);
  // Fallback guard for tall atomic/unmeasured blocks already spanning across
  // the viewport: never yank their clipped top back into view.
  if target_h > (viewport_h - top_pad - bottom_pad)
    && rel_top < top_pad
    && rel_bottom > viewport_h - bottom_pad
  {
    return None;
  }
  let margin = 24.0_f32.min(viewport_h * 0.15);
  if rel_top < top_pad {
    let next = (cur_scroll_y + rel_top - margin).max(0.0);
    ((next - cur_scroll_y).abs() > 0.5).then_some(next)
  } else if rel_bottom > viewport_h - bottom_pad {
    let next = (cur_scroll_y + (rel_bottom - (viewport_h - margin))).max(0.0);
    ((next - cur_scroll_y).abs() > 0.5).then_some(next)
  } else {
    None
  }
}

pub use mica_editor::interaction::edge_autoscroll_speed;

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn edge_autoscroll_speed_ramps_smoothly_near_and_past_edges() {
    assert_eq!(edge_autoscroll_speed(300.0, 600.0), 0.0);
    let top_inner = edge_autoscroll_speed(50.0, 600.0);
    let top_outside = edge_autoscroll_speed(-40.0, 600.0);
    assert!(top_inner < 0.0 && top_outside < top_inner);
    let bot_inner = edge_autoscroll_speed(550.0, 600.0);
    let bot_outside = edge_autoscroll_speed(650.0, 600.0);
    assert!(bot_inner > 0.0 && bot_outside > bot_inner);
  }

  #[test]
  fn keep_visible_scroll_never_yanks_clipped_tall_code_block_upward() {
    // Tall code block top is scrolled 300px above viewport (`rel_top = -300`),
    // while active caret at bottom of code block sits at `[360, 382]` inside a
    // 700px viewport -> ZERO scroll!
    assert_eq!(compute_keep_visible_scroll_target(500.0, 700.0, 360.0, 382.0), None);

    // Pressing Enter pushes caret just past the bottom edge (`[690, 712]`):
    // scroll DOWN minimally by ~36px, NEVER upward!
    let next_down = compute_keep_visible_scroll_target(500.0, 700.0, 690.0, 712.0)
      .expect("caret past bottom edge should scroll down slightly");
    assert!(
      next_down > 500.0 && next_down < 560.0,
      "expected minimal downward nudge from 500.0, got {next_down}"
    );

    // Caret moving above top edge (`[-10, 12]`): scroll UP minimally by ~34px.
    let next_up = compute_keep_visible_scroll_target(500.0, 700.0, -10.0, 12.0)
      .expect("caret above top edge should scroll up slightly");
    assert!(
      next_up < 500.0 && next_up > 440.0,
      "expected minimal upward nudge from 500.0, got {next_up}"
    );

    // Fallback atomic block taller than viewport spanning `[-250, 820]`: ZERO jump!
    assert_eq!(compute_keep_visible_scroll_target(500.0, 700.0, -250.0, 820.0), None);
  }
}
