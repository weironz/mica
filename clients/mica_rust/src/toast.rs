//! Unified root-level transient toast overlay (`app-toast`).
//!
//! Mounted once at the root of `render_mica_app` (`z_index(80)`,
//! `PointerEvents::None`) so editor status flashes, image actions, and
//! whiteboard notifications share one pill, never overlap, never shift header
//! layout, and auto-dismiss via the app-root `Owner` even after closing transient
//! context menus or popovers.

use std::time::Duration;

use nsh_core::{PointerEvents, geometry::px, prelude::*};

use crate::mica_theme::palette;

const TOAST_TTL_MS: u64 = 2200;

/// Shared toast state installed once at the app root via `provide_context`.
#[derive(Clone, Copy)]
pub struct ToastHost {
  owner: Owner,
  entry: Signal<Option<(String, u64)>>,
}

/// Install a root [`ToastHost`] into the reactive context tree.
pub fn install_toast_host() -> ToastHost {
  let host = ToastHost {
    owner: Owner::current(),
    entry: signal(None),
  };
  provide_context(host);
  host
}

/// Write a new toast message with a monotonically bumped generation token.
fn push_toast_entry(entry: Signal<Option<(String, u64)>>, msg: String) -> u64 {
  let next_gen = entry
    .try_get()
    .flatten()
    .map(|(_, g)| g.wrapping_add(1))
    .unwrap_or(1);
  let _ = entry.try_set(Some((msg, next_gen)));
  next_gen
}

/// Clear the active toast ONLY if its generation matches `expected_gen`.
fn clear_toast_if_gen(entry: Signal<Option<(String, u64)>>, expected_gen: u64) {
  if entry
    .try_get()
    .flatten()
    .is_some_and(|(_, g)| g == expected_gen)
  {
    let _ = entry.try_set(None);
  }
}

impl ToastHost {
  /// Show a short transient message at the bottom-center of the window.
  pub fn show(&self, msg: impl Into<String>) {
    let text_msg = msg.into();
    let next_gen = push_toast_entry(self.entry, text_msg);
    let entry = self.entry;
    self.owner.run(move || {
      drop(spawn(async move {
        timer(Duration::from_millis(TOAST_TTL_MS)).await;
        clear_toast_if_gen(entry, next_gen);
      }));
    });
  }
}

/// Convenience helper for callers executing directly inside the reactive tree.
pub fn show_toast(msg: impl Into<String>) -> bool {
  if let Some(host) = use_context::<ToastHost>() {
    host.show(msg);
    true
  } else {
    false
  }
}

/// Root-mounted floating bottom-center toast pill (`test_id("app-toast")`).
pub fn toast_overlay(host: ToastHost, dark_mode: Signal<bool>) -> impl IntoElement {
  let entry = host.entry;
  when(move || entry.get().is_some(), move || {
    view()
      .absolute()
      .bottom(px(20.0))
      .left_0()
      .right_0()
      .items_center()
      .z_index(80)
      .pointer_events(PointerEvents::None)
      .child(
        view()
          .test_id("app-toast")
          .px_4()
          .py_2()
          .max_w(px(420.0))
          .rounded(px(8.0))
          .items_center()
          .justify_center()
          .bg(move || palette(dark_mode.get()).surface_overlay)
          .border_1()
          .border_color(move || palette(dark_mode.get()).border_normal)
          .shadow_md()
          .child(
            text(move || entry.get().map(|(m, _)| m).unwrap_or_default())
              .text_size(px(12.5))
              .text_color(move || palette(dark_mode.get()).text_primary),
          ),
      )
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn with_runtime(f: impl FnOnce()) {
    let rt = nsh_core::element::Runtime::default();
    let _g = rt.enter();
    f();
  }

  #[test]
  fn generation_guard_prevents_stale_timer_from_clearing_newer_toast() {
    with_runtime(|| {
      let entry: Signal<Option<(String, u64)>> = signal(None);
      let gen1 = push_toast_entry(entry, "第一条提示".to_string());
      assert_eq!(entry.get_untracked(), Some(("第一条提示".to_string(), gen1)));

      let gen2 = push_toast_entry(entry, "第二条提示".to_string());
      assert_ne!(gen1, gen2);
      assert_eq!(entry.get_untracked(), Some(("第二条提示".to_string(), gen2)));

      // An expired timer carrying `gen1` must NOT clear `gen2`.
      clear_toast_if_gen(entry, gen1);
      assert_eq!(entry.get_untracked(), Some(("第二条提示".to_string(), gen2)));

      // Only the matching `gen2` timer clears it.
      clear_toast_if_gen(entry, gen2);
      assert_eq!(entry.get_untracked(), None);
    });
  }
}
