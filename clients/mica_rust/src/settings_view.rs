//! Settings, theme appearance, cloud account, and export panel.

use std::rc::Rc;

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;

pub struct SettingsCallbacks {
  pub dark_mode: Signal<bool>,
  pub locale_zh: Signal<bool>,
  pub server_url: Signal<String>,
  pub cloud_status: Signal<String>,
  pub export_preview: Signal<String>,
  pub on_toggle_theme: Rc<dyn Fn()>,
  pub on_toggle_locale: Rc<dyn Fn()>,
  pub on_check_server: Rc<dyn Fn(String)>,
  pub on_export_markdown: Rc<dyn Fn()>,
  pub on_export_html: Rc<dyn Fn()>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  pub on_close: Rc<dyn Fn()>,
}

pub fn settings_panel_view(cb: Rc<SettingsCallbacks>) -> impl IntoElement {
  let cb_close = cb.clone();
  let cb_theme = cb.clone();
  let cb_theme_lbl = cb.clone();
  let cb_loc = cb.clone();
  let cb_loc_lbl = cb.clone();
  let cb_url_val = cb.clone();
  let cb_url_chg = cb.clone();
  let cb_check = cb.clone();
  let cb_status = cb.clone();
  let cb_md = cb.clone();
  let cb_html = cb.clone();
  let cb_snap = cb.clone();
  let cb_exp = cb.clone();

  view()
    .test_id("settings-panel")
    .flex_1()
    .p_4()
    .gap_4()
    .bg(move || theme().background)
    .child(
      view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(
          text("Settings & Sync")
            .text_base()
            .text_color(move || theme().foreground),
        )
        .child(
          button()
            .test_id("settings-close-btn")
            .px_2()
            .py_1()
            .border_1()
            .border_color(move || theme().border)
            .rounded(move || theme().radius)
            .on_press(move || (cb_close.on_close)())
            .child(
              text("Close")
                .text_xs()
                .text_color(move || theme().foreground),
            ),
        ),
    )
    .child(
      // Appearance & Language
      view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_2()
        .p_3()
        .border_1()
        .border_color(move || theme().border)
        .rounded(move || theme().radius)
        .child(
          button()
            .test_id("settings-toggle-theme")
            .px_3()
            .py_1()
            .border_1()
            .border_color(move || theme().border)
            .rounded(move || theme().radius)
            .bg(move || theme().secondary)
            .on_press(move || (cb_theme.on_toggle_theme)())
            .child(
              text(move || {
                if cb_theme_lbl.dark_mode.get() {
                  "Theme: Dark"
                } else {
                  "Theme: Light"
                }
              })
              .text_xs()
              .text_color(move || theme().secondary_foreground),
            ),
        )
        .child(
          button()
            .test_id("settings-toggle-locale")
            .px_3()
            .py_1()
            .border_1()
            .border_color(move || theme().border)
            .rounded(move || theme().radius)
            .bg(move || theme().secondary)
            .on_press(move || (cb_loc.on_toggle_locale)())
            .child(
              text(move || {
                if cb_loc_lbl.locale_zh.get() {
                  "Language: 中文"
                } else {
                  "Language: English"
                }
              })
              .text_xs()
              .text_color(move || theme().secondary_foreground),
            ),
        ),
    )
    .child(
      // Cloud Server Sync
      view()
        .p_3()
        .gap_2()
        .border_1()
        .border_color(move || theme().border)
        .rounded(move || theme().radius)
        .child(
          text("Cloud Server (mica-sync + mica-client)")
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
        .child(
          view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(
              input()
                .test_id("settings-server-url")
                .flex_1()
                .px_2()
                .py_1()
                .border_1()
                .border_color(move || theme().border)
                .rounded(move || theme().radius)
                .text_color(move || theme().foreground)
                .value(move || cb_url_val.server_url.get())
                .on_change(move |v: &str| cb_url_chg.server_url.set(v.to_owned())),
            )
            .child(
              button()
                .test_id("settings-check-health")
                .px_3()
                .py_1()
                .border_1()
                .border_color(move || theme().border)
                .rounded(move || theme().radius)
                .bg(move || theme().primary)
                .on_press(move || {
                  let url = cb_check.server_url.get_untracked();
                  (cb_check.on_check_server)(url);
                })
                .child(
                  text("Check Health")
                    .text_xs()
                    .text_color(move || theme().primary_foreground),
                ),
            ),
        )
        .child(
          text(move || cb_status.cloud_status.get())
            .text_xs()
            .text_color(move || theme().muted_foreground),
        ),
    )
    .child(
      // Export & Version Snapshot
      view()
        .p_3()
        .gap_2()
        .border_1()
        .border_color(move || theme().border)
        .rounded(move || theme().radius)
        .child(
          text("Document Export & Local Version History")
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
        .child(
          view()
            .flex_row()
            .flex_wrap()
            .gap_2()
            .child(
              button()
                .test_id("settings-export-md")
                .px_3()
                .py_1()
                .border_1()
                .border_color(move || theme().border)
                .rounded(move || theme().radius)
                .bg(move || theme().secondary)
                .on_press(move || (cb_md.on_export_markdown)())
                .child(
                  text("Export Markdown")
                    .text_xs()
                    .text_color(move || theme().secondary_foreground),
                ),
            )
            .child(
              button()
                .test_id("settings-export-html")
                .px_3()
                .py_1()
                .border_1()
                .border_color(move || theme().border)
                .rounded(move || theme().radius)
                .bg(move || theme().secondary)
                .on_press(move || (cb_html.on_export_html)())
                .child(
                  text("Export HTML")
                    .text_xs()
                    .text_color(move || theme().secondary_foreground),
                ),
            )
            .child(
              button()
                .test_id("settings-save-snapshot")
                .px_3()
                .py_1()
                .border_1()
                .border_color(move || theme().border)
                .rounded(move || theme().radius)
                .bg(move || theme().secondary)
                .on_press(move || (cb_snap.on_save_snapshot)())
                .child(
                  text("Save Snapshot")
                    .text_xs()
                    .text_color(move || theme().secondary_foreground),
                ),
            ),
        )
        .child(
          text(move || cb_exp.export_preview.get())
            .text_xs()
            .text_color(move || theme().muted_foreground),
        ),
    )
    .child(
      // Shortcuts summary
      view()
        .p_3()
        .gap_1()
        .border_1()
        .border_color(move || theme().border)
        .rounded(move || theme().radius)
        .child(
          text("Shortcuts: Ctrl/Cmd+Z Undo · Ctrl/Cmd+Shift+Z Redo · Ctrl/Cmd+K Search · / Slash Menu")
            .text_xs()
            .text_color(move || theme().muted_foreground),
        ),
    )
}
