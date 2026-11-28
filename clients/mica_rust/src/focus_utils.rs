//! Deferred post-layout focus helpers for native platform text inputs.
//!
//! On Windows (`nsh-platform-windows`), calling `.auto_focus(true)` synchronously
//! inside a newly attached `virtual_list` `dynamic()` slot invokes `RichEdit::set_focus`
//! before layout assigns non-zero bounds, causing `msftedit.dll` `CxxThrowException`
//! (`0xc0000409`) aborts. Always focus dynamically mounted inputs after non-zero layout
//! via [`focus_once_after_layout`] or [`focus_on_next_tick`].

use std::{cell::Cell, time::Duration};

use nsh_core::{
  element::ElementRef,
  geometry::{Bounds, Points},
  prelude::*,
};

/// Defer `el_ref.focus()` to the next runtime tick while `should_focus()` remains true.
pub(crate) fn focus_on_next_tick(
  el_ref: ElementRef,
  should_focus: impl Fn() -> bool + 'static,
) {
  spawn(async move {
    timer(Duration::from_millis(0)).await;
    if should_focus() {
      el_ref.focus();
    }
  });
}

/// Focus `el_ref` once after `bounds` acquires non-zero dimensions inside `on_layout`.
pub(crate) fn focus_once_after_layout(
  bounds: &Bounds<Points>,
  did_focus: &Cell<bool>,
  el_ref: ElementRef,
  should_focus: impl Fn() -> bool + 'static,
) {
  if !did_focus.get() && bounds.size.width.0 > 0.0 && bounds.size.height.0 > 0.0 {
    did_focus.set(true);
    focus_on_next_tick(el_ref, should_focus);
  }
}
