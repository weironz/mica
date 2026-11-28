//! Narrow-viewport & mobile responsive shell chrome (`CAP-16-05`):
//! - Breakpoint `< 768px` collapses left/right sidebars into dismissible
//!   overlay drawers with backdrop so the editor keeps 100% reading/writing width.
//! - Replaces the desktop multi-tab strip with a compact tab switcher pill +
//!   dropdown list (`narrow-tab-switcher` / `narrow-tab-popover`).

use std::rc::Rc;

use nsh_base::button;
use nsh_core::{color::Rgba, geometry::px, prelude::*};
use uuid::Uuid;

use crate::{mica_icons, mica_theme::palette};

/// Viewport width threshold (in logical px) below which the shell enters
/// mobile / narrow-window responsive drawer mode (`CAP-16-05`).
pub const NARROW_BREAKPOINT: f32 = 768.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NarrowTabEntry {
  pub idx: usize,
  pub doc_id: Uuid,
  pub title: String,
  pub active: bool,
}

/// Compact narrow-viewport tab switcher button mounted in the 34px top bar.
pub fn narrow_tab_switcher_button(
  active_title: Signal<String>,
  tab_count: Signal<usize>,
  open: Signal<bool>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  button()
    .test_id("narrow-tab-switcher")
    .accessibility_label("切换打开的标签页")
    .h(px(26.0))
    .max_w(px(210.0))
    .min_w(px(0.0))
    .flex_shrink(1.0)
    .px_2()
    .flex_row()
    .items_center()
    .gap(px(6.0))
    .rounded(px(6.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_subtle)
    .bg(move || palette(dark_mode.get()).surface_base)
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(move || open.set(!open.get_untracked()))
    .child(
      text(move || {
        let t = active_title.get();
        if t.trim().is_empty() {
          "未命名页面".to_string()
        } else {
          t
        }
      })
      .text_size(px(12.0))
      .font_weight(FontWeight::MEDIUM)
      .text_color(move || palette(dark_mode.get()).text_primary)
      .truncate(),
    )
    .child(
      view()
        .px(px(5.0))
        .h(px(16.0))
        .rounded(px(8.0))
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).accent_soft)
        .child(
          text(move || tab_count.get().to_string())
            .text_size(px(10.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || palette(dark_mode.get()).accent),
        ),
    )
}

/// Dropdown list of open tabs for narrow viewports.
pub fn narrow_tab_popover_view(
  open: Signal<bool>,
  tabs: Signal<Vec<NarrowTabEntry>>,
  dark_mode: Signal<bool>,
  on_select: Rc<dyn Fn(usize, Uuid, String)>,
  on_close_tab: Rc<dyn Fn(usize)>,
  on_new_page: Rc<dyn Fn()>,
) -> impl IntoElement {
  when(move || open.get(), move || {
    let entries = tabs.get();
    let can_close = entries.len() > 1;
    let p = palette(dark_mode.get());
    let mut list = view()
      .test_id("narrow-tab-popover")
      .absolute()
      .top(px(36.0))
      .left(px(12.0))
      .w(px(260.0))
      .p(px(6.0))
      .gap(px(2.0))
      .rounded(px(10.0))
      .border_1()
      .border_color(p.border_normal)
      .bg(p.surface_overlay)
      .z_index(48);

    for item in entries {
      let sel_cb = on_select.clone();
      let close_cb = on_close_tab.clone();
      let idx = item.idx;
      let doc_id = item.doc_id;
      let title = if item.title.trim().is_empty() {
        "未命名页面".to_string()
      } else {
        item.title.clone()
      };
      let title_sel = title.clone();
      let is_active = item.active;
      let mut row = view()
        .w_full()
        .h(px(32.0))
        .px_2()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .rounded(px(6.0))
        .bg(if is_active { p.accent_soft } else { Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 } })
        .child(
          button()
            .test_id(&format!("narrow-tab-item-{idx}"))
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .flex_row()
            .items_center()
            .on_press(move || {
              open.set(false);
              sel_cb(idx, doc_id, title_sel.clone());
            })
            .child(
              text(title)
                .text_size(px(13.0))
                .font_weight(if is_active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                .text_color(if is_active { p.accent } else { p.text_primary })
                .truncate(),
            ),
        );
      if can_close {
        row = row.child(
          button()
            .test_id(&format!("narrow-tab-close-{idx}"))
            .accessibility_label("关闭标签页")
            .w(px(22.0))
            .h(px(22.0))
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || close_cb(idx))
            .child(mica_icons::close_sm(p.text_faint_hex)),
        );
      }
      list = list.child(row);
    }

    let np = on_new_page.clone();
    list = list.child(
      view()
        .w_full()
        .mt_1()
        .pt_1()
        .border_t_1()
        .border_color(p.border_subtle)
        .child(
          button()
            .test_id("narrow-tab-new-page")
            .w_full()
            .h(px(30.0))
            .px_2()
            .flex_row()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
            .on_press(move || {
              open.set(false);
              np();
            })
            .child(mica_icons::plus(p.text_muted_hex))
            .child(text("新建页面").text_size(px(12.5)).text_color(p.text_primary)),
        ),
    );
    list
  })
}

/// Overlay drawer container with semi-transparent dismissible backdrop for
/// narrow viewports (`< 768px`).
pub fn mobile_drawer_overlay<C: IntoElement + 'static>(
  drawer_test_id: &'static str,
  align_right: bool,
  open: Signal<bool>,
  dark_mode: Signal<bool>,
  content: C,
) -> impl IntoElement {
  let backdrop = button()
    .test_id(&format!("{drawer_test_id}-backdrop"))
    .accessibility_label("关闭抽屉")
    .flex_1()
    .h_full()
    .bg(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.36 })
    .on_press(move || open.set(false));

  let drawer = view()
    .test_id(drawer_test_id)
    .h_full()
    .max_w(px(300.0))
    .bg(move || palette(dark_mode.get()).surface_sunken)
    .border_color(move || palette(dark_mode.get()).border_normal)
    .child(content);

  let mut shell = view()
    .absolute()
    .top(px(34.0))
    .left_0()
    .right_0()
    .bottom_0()
    .flex_row()
    .z_index(42);

  if align_right {
    shell = shell.child(backdrop).child(drawer.border_l_1());
  } else {
    shell = shell.child(drawer.border_r_1()).child(backdrop);
  }
  shell
}
