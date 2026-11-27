//! Atomic / non-prose block views inside virtualized documents:
//! - Embedded single-layer whiteboard card (`board`) with auto-fit & resize handles
//! - Mermaid diagram preview (`mermaid`) with host-theme SVG rendering
//! - RaTeX math block (`math`) with click-to-edit LaTeX

use std::rc::Rc;

use mica_editor::EditorBlock;
use nsh_base::{button, input};
use nsh_core::{
  SubmitBehavior,
  element::{ImageSrc, svg},
  geometry::px,
  prelude::*,
};
use serde_json::Value;

use crate::{
  block_views::{
    BlockRenderEnv, LightboxContent, MATH_LIGHT_BG, OpenBoardCallback, OpenBoardWithBlockCallback,
    TRANSPARENT_RGBA, copy_with_reset, live_index,
  },
  mica_icons,
  mica_theme::palette,
};

// ── 3. Embedded Board (Single-Layer, Snug Auto-Fit, Draggable Resize Handles) ─

pub(crate) fn render_board_block_view(env: &BlockRenderEnv, at: Position, block: EditorBlock) -> impl IntoElement {
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

pub(crate) fn render_mermaid_block_view(env: &BlockRenderEnv, at: Position, block: EditorBlock) -> impl IntoElement {
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
            input()
              .test_id("mermaid-source-editor")
              .label("Mermaid 源码编辑器")
              .w_full()
              .h(px(140.0))
              .multiline(true)
              .default_value(se2)
              .auto_focus(true)
              .placeholder("graph LR\n  A[Mica] --> B[Pure Rust]")
              .text_size(px(13.0))
              .line_height(px(20.0))
              .font_family("monospace")
              .text_color(move || palette(dark_mode.get()).text_primary)
              .bg(move || palette(dark_mode.get()).code_bg)
              .border(px(0.0))
              .on_change(move |v: &str| br_m2.update_block_source_at(live_index(at), v)),
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

pub(crate) fn render_math_block_view(env: &BlockRenderEnv, at: Position, block: EditorBlock) -> impl IntoElement {
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
