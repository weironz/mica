//! 440px Public Share modal — 1:1 restoration of Flutter's `ShareDialog`
//! (`share_dialog.dart`) and `InlineCopyButton` (`copy_button.dart`).
//!
//! Displays a `公开访问` toggle (`任何有链接的人都能只读查看，无需登录`), and when
//! enabled reveals the read-only `{origin}/s/{token}` URL inside a `surface_raised`
//! container with a one-click copy button that flashes a green check (`已复制`)
//! for 1600ms.

use std::{rc::Rc, time::Duration};

use nsh_base::button;
use nsh_core::{
  color::Rgba,
  geometry::{px, relative},
  prelude::*,
};

use crate::{mica_icons, mica_theme::palette};

/// Duration the green check confirmation (`已复制`) stays visible after copying
/// (matches Flutter `kCopyDoneDuration = Duration(milliseconds: 1600)`).
const COPY_DONE_DURATION: Duration = Duration::from_millis(1600);

/// Reactive state & callbacks driving [`share_dialog_modal`].
#[derive(Clone)]
pub struct ShareDialogState {
  pub open: Signal<bool>,
  pub loading: Signal<bool>,
  pub busy: Signal<bool>,
  pub shared: Signal<bool>,
  pub token: Signal<Option<String>>,
  pub origin: Signal<String>,
  pub error: Signal<Option<String>>,
  pub on_toggle_public: Rc<dyn Fn(bool)>,
  pub on_close: Rc<dyn Fn()>,
}

/// Build the public read-only share URL `{origin}/s/{token}`, stripping any
/// trailing slash on `origin`.
pub fn build_share_url(origin: &str, token: &str) -> String {
  let base = origin.trim().trim_end_matches('/');
  let tok = token.trim();
  if base.is_empty() {
    format!("/s/{tok}")
  } else {
    format!("{base}/s/{tok}")
  }
}

/// Centered `440px` Share dialog modal.
///
/// Renders nothing when `!state.open.get()`.
pub fn share_dialog_modal(state: ShareDialogState, dark_mode: Signal<bool>) -> impl IntoElement {
  let copied = signal(false);
  let copy_gen = signal(0u64);
  let open_sig = state.open;

  when(
    move || open_sig.get(),
    move || {
      let modal_owner = Owner::current();
      let st_bg = state.clone();
      let st_close_x = state.clone();
      let st_close_btn = state.clone();
      let st_body = state.clone();

      view()
        .test_id("share-dialog-backdrop")
        .absolute()
        .inset_0()
        .z_index(55)
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("share-dialog-backdrop-dismiss")
            .accessibility_label("关闭分享弹窗")
            .absolute()
            .inset_0()
            .on_press(move || {
              copied.set(false);
              (st_bg.on_close)();
            }),
        )
        .child(
          view()
            .test_id("share-dialog-modal")
            .w(px(440.0))
            .max_w(relative(0.92))
            .rounded(px(12.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .overflow_hidden()
            .z_index(56)
            .child(
              // Header
              view()
                .px_5()
                .py_3()
                .flex_row()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .child(
                  text("分享")
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                )
                .child(
                  button()
                    .test_id("share-dialog-close-x")
                    .accessibility_label("关闭")
                    .w(px(26.0))
                    .h(px(26.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      copied.set(false);
                      (st_close_x.on_close)();
                    })
                    .child(dynamic(move || {
                      mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                    })),
                ),
            )
            .child(
              // Body
              view()
                .px_5()
                .py_4()
                .child(dynamic(move || {
                  let p = palette(dark_mode.get());
                  if st_body.loading.get() {
                    return view()
                      .test_id("share-dialog-loading")
                      .h(px(80.0))
                      .items_center()
                      .justify_center()
                      .child(text("加载中…").text_size(px(13.0)).text_color(p.text_muted))
                      .into_element();
                  }

                  let is_shared = st_body.shared.get();
                  let is_busy = st_body.busy.get();
                  let token_opt = st_body.token.get();
                  let origin = st_body.origin.get();
                  let error_opt = st_body.error.get();
                  let on_toggle = st_body.on_toggle_public.clone();

                  let mut content = view().w_full().gap_3();

                  // Public access switch row
                  content = content.child(
                    button()
                      .test_id("share-public-toggle")
                      .accessibility_label("切换公开访问")
                      .disabled(is_busy)
                      .w_full()
                      .py_1()
                      .rounded(px(6.0))
                      .flex_row()
                      .items_center()
                      .justify_between()
                      .gap_3()
                      .opacity(if is_busy { 0.6_f32 } else { 1.0_f32 })
                      .on_press(move || {
                        copied.set(false);
                        on_toggle(!is_shared);
                      })
                      .child(
                        view()
                          .flex_1()
                          .min_w(px(0.0))
                          .gap(px(2.0))
                          .child(
                            text("公开访问")
                              .text_size(px(14.0))
                              .font_weight(FontWeight::MEDIUM)
                              .text_color(p.text_primary),
                          )
                          .child(
                            text("任何有链接的人都能只读查看，无需登录")
                              .text_size(px(12.0))
                              .text_color(p.text_muted),
                          ),
                      )
                      .child(pill_switch(is_shared, p.accent, p.border_strong)),
                  );

                  // Read-only public URL box + copy button with green check flash
                  if is_shared
                    && let Some(tok) = token_opt.filter(|t| !t.trim().is_empty())
                  {
                      let url = build_share_url(&origin, &tok);
                      let url_for_copy = url.clone();
                      let is_copied = copied.get();

                      content = content.child(
                        view()
                          .test_id("share-url-box")
                          .w_full()
                          .px_3()
                          .py_2()
                          .rounded(px(6.0))
                          .border_1()
                          .border_color(p.border_subtle)
                          .bg(p.surface_raised)
                          .flex_row()
                          .items_center()
                          .gap_2()
                          .child(
                            scroll()
                              .horizontal()
                              .scrollbar(false)
                              .flex_1()
                              .min_w(px(0.0))
                              .child(
                                text(url)
                                  .test_id("share-url-text")
                                  .selectable(true)
                                  .text_size(px(12.5))
                                  .text_color(p.text_primary),
                              ),
                          )
                          .child(
                            button()
                              .test_id("share-copy-btn")
                              .accessibility_label(if is_copied { "已复制" } else { "复制链接" })
                              .px_2()
                              .py(px(4.0))
                              .rounded(px(5.0))
                              .flex_row()
                              .items_center()
                              .gap(px(4.0))
                              .flex_shrink_0()
                              .hover(move |s| {
                                s.bg(palette(dark_mode.get_untracked()).surface_hover)
                              })
                              .on_press(move || {
                                set_clipboard(url_for_copy.clone());
                                let next_gen = copy_gen.get_untracked().wrapping_add(1);
                                copy_gen.set(next_gen);
                                copied.set(true);
                                // Run under `modal_owner` so rebuilding this `dynamic(...)`
                                // body for `copied = true` does not cancel the reset timer.
                                modal_owner.run(move || {
                                  drop(set_timeout(COPY_DONE_DURATION, move || {
                                    if copy_gen.get_untracked() == next_gen {
                                      copied.set(false);
                                    }
                                  }));
                                });
                              })
                              .child(if is_copied {
                                mica_icons::check_icon(p.success_hex)
                              } else {
                                mica_icons::copy_icon(p.text_faint_hex)
                              })
                              .child(
                                text(if is_copied { "已复制" } else { "复制链接" })
                                  .text_size(px(12.0))
                                  .font_weight(if is_copied {
                                    FontWeight::MEDIUM
                                  } else {
                                    FontWeight::NORMAL
                                  })
                                  .text_color(if is_copied {
                                    p.copy_done
                                  } else {
                                    p.text_muted
                                  }),
                              ),
                          ),
                      );
                  }

                  if let Some(err) = error_opt {
                    content = content.child(
                      view()
                        .test_id("share-dialog-error")
                        .w_full()
                        .px_3()
                        .py_2()
                        .rounded(px(6.0))
                        .bg(p.danger_wash)
                        .child(
                          text(format!("操作失败：{err}"))
                            .text_size(px(12.0))
                            .text_color(p.danger),
                        ),
                    );
                  }

                  content.into_element()
                })),
            )
            .child(
              // Footer
              view()
                .px_5()
                .py_3()
                .flex_row()
                .justify_end()
                .border_t_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .child(
                  button()
                    .test_id("share-dialog-close-btn")
                    .px_3()
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || {
                      copied.set(false);
                      (st_close_btn.on_close)();
                    })
                    .child(
                      text("关闭")
                        .text_size(px(13.0))
                        .text_color(move || palette(dark_mode.get()).text_primary),
                    ),
                ),
            ),
        )
    },
  )
}

fn pill_switch(on: bool, active_bg: Rgba, inactive_bg: Rgba) -> impl IntoElement {
  let thumb_white = Rgba {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
  };
  let mut track = view()
    .w(px(40.0))
    .h(px(22.0))
    .px(px(2.0))
    .flex_row()
    .items_center()
    .flex_shrink_0()
    .rounded_full()
    .bg(if on { active_bg } else { inactive_bg });
  if on {
    track = track.justify_end();
  } else {
    track = track.justify_start();
  }
  track.child(
    view()
      .w(px(18.0))
      .h(px(18.0))
      .rounded_full()
      .bg(thumb_white),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn build_share_url_normalizes_trailing_slash() {
    assert_eq!(
      build_share_url("https://mica.example.com/", "abc123"),
      "https://mica.example.com/s/abc123"
    );
    assert_eq!(
      build_share_url("https://mica.example.com", "xyz789"),
      "https://mica.example.com/s/xyz789"
    );
    assert_eq!(build_share_url("", "tok"), "/s/tok");
  }
}
