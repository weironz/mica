//! Mobile / touch-friendly word selection rules and compact action bar
//! (1:1 Rust port of `main:clients/mica_flutter/lib/editor/touch_selection.dart`,
//! implementing `CAP-16-06`).
//!
//! Why CJK uses a 1-2 character rule on touch:
//! Chinese / Japanese / Korean scripts have no spaces between words, so a
//! naive word-boundary scan grabs the entire clause up to the next punctuation.
//! Selecting the tapped character plus one neighbor (grapheme/UTF-16 safe)
//! makes touch long-press and double-tap predictable and easy to refine.

use std::rc::Rc;

use mica_editor::{
  EditorCommand,
  utf16::{utf16_len, utf16_slice},
};
use nsh_base::button;
use nsh_core::{
  element::{clipboard_string, set_clipboard},
  geometry::px,
  prelude::*,
};

use crate::{editor_bridge::MicaDocumentBridge, mica_theme::palette};

fn is_wide_ideograph(ch: char) -> bool {
  let c = ch as u32;
  (0x3400..=0x9FFF).contains(&c)
    || (0x3040..=0x30FF).contains(&c)
    || (0xAC00..=0xD7A3).contains(&c)
    || (0xF900..=0xFAFF).contains(&c)
    || (0x20000..=0x2FA1F).contains(&c)
}

fn is_wide_punctuation(ch: char) -> bool {
  let c = ch as u32;
  (0x2000..=0x206F).contains(&c)
    || (0x3000..=0x303F).contains(&c)
    || (0xFE30..=0xFE4F).contains(&c)
    || (0xFF00..=0xFF0F).contains(&c)
    || (0xFF1A..=0xFF20).contains(&c)
    || (0xFF3B..=0xFF40).contains(&c)
    || (0xFF5B..=0xFF65).contains(&c)
}

/// Computes the UTF-16 `(start, end)` range selected by a touch double-tap or
/// long-press at UTF-16 unit offset `glyph_u16`:
/// - Wide CJK / kana / hangul: tapped char + next wide char (or previous wide
///   char at the end of a run, or just that 1 char if isolated).
/// - Wide CJK punctuation (`，。、“”（）【】`): selected alone.
/// - Latin words & numbers: full word (`mica_editor::word_range_at`).
/// - Surrogate-pair safe: never splits emoji or CJK Ext-B characters.
pub fn touch_word_bounds(text: &str, glyph_u16: u32) -> (u32, u32) {
  let total_u16 = utf16_len(text);
  if total_u16 == 0 {
    return (0, 0);
  }
  let g = glyph_u16.min(total_u16.saturating_sub(1));

  // Collect scalar chars with their UTF-16 [start, end) spans.
  let mut chars: Vec<(char, u32, u32)> = Vec::with_capacity(text.len());
  let mut cur = 0u32;
  for ch in text.chars() {
    let next = cur + ch.len_utf16() as u32;
    chars.push((ch, cur, next));
    cur = next;
  }

  let at = chars
    .iter()
    .position(|&(_, s, e)| g >= s && g < e)
    .unwrap_or(chars.len() - 1);
  let (ch, s, e) = chars[at];

  if is_wide_punctuation(ch) {
    return (s, e);
  }
  if is_wide_ideograph(ch) {
    // CJK / Kana / Hangul: take tapped char + 1 neighbor in the same wide run.
    if at + 1 < chars.len() && is_wide_ideograph(chars[at + 1].0) {
      return (s, chars[at + 1].2);
    }
    if at > 0 && is_wide_ideograph(chars[at - 1].0) {
      return (chars[at - 1].1, e);
    }
    return (s, e);
  }
  // Latin / numeric / underscore word: scan contiguous non-wide word chars so
  // words never bleed into adjacent CJK or emoji.
  let is_latin_word = |c: char| !is_wide_ideograph(c) && (c.is_alphanumeric() || c == '_');
  if is_latin_word(ch) {
    let mut lo = at;
    while lo > 0 && is_latin_word(chars[lo - 1].0) {
      lo -= 1;
    }
    let mut hi = at;
    while hi + 1 < chars.len() && is_latin_word(chars[hi + 1].0) {
      hi += 1;
    }
    return (chars[lo].1, chars[hi].2);
  }
  // Emoji, symbols, ASCII punctuation, whitespace: select the single scalar
  // (never splitting UTF-16 surrogate pairs).
  (s, e)
}

/// Returns the UTF-16 slice selected by [`touch_word_bounds`] (helper for UI & tests).
pub fn touch_selected_slice(text: &str, glyph_u16: u32) -> &str {
  let (s, e) = touch_word_bounds(text, glyph_u16);
  utf16_slice(text, s, e)
}

/// Compact touch / narrow-viewport selection action bar (`剪切 / 复制 / 粘贴 / 全选`,
/// `CAP-16-06`). Uses generous 34px hit targets so fingers can tap accurately
/// without triggering accidental drag or horizontal overflow.
pub fn touch_selection_bar_view(
  bridge: Rc<MicaDocumentBridge>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let b_cut = bridge.clone();
  let b_copy = bridge.clone();
  let b_paste = bridge.clone();
  let b_all = bridge;

  let action_btn = move |tid: &'static str, label: &'static str, on_act: Rc<dyn Fn()>| {
    button()
      .test_id(tid)
      .h(px(34.0))
      .px_3()
      .items_center()
      .justify_center()
      .rounded(px(6.0))
      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
      .on_press(move || on_act())
      .child(
        text(label)
          .text_size(px(13.0))
          .font_weight(FontWeight::MEDIUM)
          .text_color(move || palette(dark_mode.get()).text_primary),
      )
  };

  view()
    .test_id("touch-selection-bar")
    .h(px(40.0))
    .px(px(4.0))
    .flex_row()
    .items_center()
    .gap(px(2.0))
    .rounded(px(10.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_overlay)
    .child(action_btn(
      "touch-sel-cut",
      "剪切",
      Rc::new(move || {
        if let Some(payload) = b_cut.copy_selection() {
          set_clipboard(payload.plain);
          b_cut.dispatch(EditorCommand::DeleteBackward);
          b_cut.set_status_flash("已剪切所选内容");
        }
      }),
    ))
    .child(action_btn(
      "touch-sel-copy",
      "复制",
      Rc::new(move || {
        if let Some(payload) = b_copy.copy_selection() {
          set_clipboard(payload.plain);
          b_copy.set_status_flash("已复制所选内容");
        }
      }),
    ))
    .child(action_btn(
      "touch-sel-paste",
      "粘贴",
      Rc::new(move || {
        let br = b_paste.clone();
        spawn(async move {
          let s = clipboard_string().await;
          if !s.is_empty() {
            br.paste_clipboard(&s, "", false);
          }
        });
      }),
    ))
    .child(action_btn(
      "touch-sel-all",
      "全选",
      Rc::new(move || {
        b_all.select_all();
      }),
    ))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn touch_word_bounds_handles_cjk_latin_punctuation_and_emoji() {
    // CJK mid-run selects 2 chars (not the entire sentence!)
    let s = "泰国算力交付清单，共32台";
    assert_eq!(touch_selected_slice(s, 0), "泰国");
    assert_eq!(touch_selected_slice(s, 2), "算力");
    // End of CJK run grabs the previous wide char too
    assert_eq!(touch_selected_slice(s, 7), "清单");
    // Full-width punctuation stands alone
    assert_eq!(touch_selected_slice(s, 8), "，");
    // Isolated single CJK char stays 1 char
    assert_eq!(touch_selected_slice(s, 9), "共");
    // ASCII number grabs whole run
    assert_eq!(touch_selected_slice(s, 10), "32");
    // Latin word grabs full word
    let en = "hello world";
    assert_eq!(touch_selected_slice(en, 1), "hello");
    // Emoji surrogate pair is never split
    let em = "🚀火箭";
    assert_eq!(touch_selected_slice(em, 0), "🚀");
    assert_eq!(touch_selected_slice(em, 1), "🚀");
    assert_eq!(touch_selected_slice(em, 2), "火箭");
  }
}
