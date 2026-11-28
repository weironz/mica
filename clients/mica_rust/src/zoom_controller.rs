//! Desktop & Web global UI zoom controller (`Ctrl+Wheel` / `Ctrl+=` / `Ctrl+-` / `Ctrl+0`).
//!
//! Clamps zoom in `[0.75, 1.50]` (75%..=150%) with `5%` discrete steps, persists
//! to `VaultStore` (`uiZoomPercent`), and drives a transient percentage pill.

use std::rc::Rc;
use std::time::Duration;

use nsh_core::{WheelDelta, WheelEvent, prelude::*};

use crate::{mica_theme::palette, storage::VaultStore};

pub const ZOOM_MIN: f32 = 0.75;
pub const ZOOM_MAX: f32 = 1.50;
pub const ZOOM_STEP: f32 = 0.05;
const ZOOM_PREF_KEY: &str = "uiZoomPercent";

#[derive(Clone)]
pub struct ZoomController {
  pub factor: Signal<f32>,
  pub badge_visible: Signal<bool>,
  badge_gen: Signal<u64>,
  vault: Rc<VaultStore>,
}

fn snap_zoom(raw: f32) -> f32 {
  let clamped = raw.clamp(ZOOM_MIN, ZOOM_MAX);
  ((clamped * 20.0).round() / 20.0).clamp(ZOOM_MIN, ZOOM_MAX)
}

impl ZoomController {
  pub fn new(vault: Rc<VaultStore>) -> Self {
    let initial = vault
      .get_pref(ZOOM_PREF_KEY)
      .and_then(|s| s.trim().parse::<u32>().ok())
      .map(|pct| snap_zoom(pct as f32 / 100.0))
      .unwrap_or(1.0);
    Self {
      factor: signal(initial),
      badge_visible: signal(false),
      badge_gen: signal(0u64),
      vault,
    }
  }

  pub fn percent(&self) -> u32 {
    (self.factor.get() * 100.0).round() as u32
  }

  pub fn step_by(&self, steps: i32) {
    if steps == 0 {
      return;
    }
    let cur = self.factor.get_untracked();
    let next = snap_zoom(cur + (steps as f32) * ZOOM_STEP);
    self.apply_factor(next);
  }

  pub fn reset(&self) {
    self.apply_factor(1.0);
  }

  fn apply_factor(&self, next: f32) {
    if (self.factor.get_untracked() - next).abs() > 0.001 {
      self.factor.set(next);
      let pct = (next * 100.0).round() as u32;
      let _ = self.vault.set_pref(ZOOM_PREF_KEY, &pct.to_string());
    }
    self.flash_badge();
  }

  fn flash_badge(&self) {
    self.badge_visible.set(true);
    let gen_sig = self.badge_gen;
    let vis_sig = self.badge_visible;
    let next_gen = gen_sig.get_untracked().wrapping_add(1);
    gen_sig.set(next_gen);
    spawn(async move {
      timer(Duration::from_millis(1400)).await;
      if gen_sig.get_untracked() == next_gen {
        vis_sig.set(false);
      }
    });
  }

  /// Handle a root-level wheel event when `Ctrl` / `Cmd` is held. Returns `true`
  /// if it triggered a zoom step.
  pub fn handle_wheel(&self, ev: &WheelEvent) -> bool {
    if !ev.modifiers.ctrl && !ev.modifiers.cmd {
      return false;
    }
    let dy = match ev.delta {
      WheelDelta::Points(pt) => f32::from(pt.y),
      WheelDelta::Lines(pt) => pt.y * 24.0,
    };
    if dy.abs() < 0.01 {
      return false;
    }
    // Negative wheel delta (`y < 0`) is scroll-up -> zoom in (+1 step).
    let steps = if dy < 0.0 { 1 } else { -1 };
    self.step_by(steps);
    true
  }
}

pub fn zoom_badge_overlay(zoom: &ZoomController, dark_mode: Signal<bool>) -> impl IntoElement {
  let vis = zoom.badge_visible;
  let factor = zoom.factor;
  when(move || vis.get(), move || {
    view()
      .test_id("zoom-level-badge")
      .absolute()
      .top(px(44.0))
      .right(px(16.0))
      .z_index(58)
      .px_3()
      .py_1()
      .rounded(px(999.0))
      .border_1()
      .border_color(move || palette(dark_mode.get()).border_normal)
      .bg(move || palette(dark_mode.get()).surface_overlay)
      .shadow_md()
      .child(
        text(move || format!("{}%", (factor.get() * 100.0).round() as u32))
          .text_size(px(12.5))
          .font_weight(FontWeight::MEDIUM)
          .text_color(move || palette(dark_mode.get()).text_primary),
      )
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn snap_zoom_clamps_and_quantizes_to_five_percent_steps() {
    assert!((snap_zoom(0.1) - ZOOM_MIN).abs() < 1e-5);
    assert!((snap_zoom(3.0) - ZOOM_MAX).abs() < 1e-5);
    assert!((snap_zoom(1.07) - 1.05).abs() < 1e-5);
    assert!((snap_zoom(0.92) - 0.90).abs() < 1e-5);
  }
}
