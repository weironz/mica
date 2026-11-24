//! UTF-16 code-unit indexing utilities.
//!
//! `mica-core::MicaDoc` uses `OffsetKind::Utf16` and platform IMEs / Flutter
//! `TextPainter` address text in UTF-16 code units. Every offset in
//! `mica-editor` follows the same coordinate system, and every helper here
//! refuses to slice or park a boundary between the two halves of a UTF-16
//! surrogate pair (emoji, CJK Extension B, mathematical symbols outside BMP).

/// Length of `s` in UTF-16 code units.
#[inline]
pub fn utf16_len(s: &str) -> u32 {
  s.encode_utf16().count() as u32
}

/// Convert a UTF-16 offset into a UTF-8 byte index in `s`.
///
/// If `u16_off` lands strictly inside a 2-unit surrogate pair:
/// - `round_up == false` snaps back to the start of that scalar value;
/// - `round_up == true` snaps forward to the end of that scalar value.
pub fn utf16_to_byte(s: &str, u16_off: u32, round_up: bool) -> usize {
  let mut units = 0u32;
  for (byte_idx, ch) in s.char_indices() {
    if units == u16_off {
      return byte_idx;
    }
    let next = units + ch.len_utf16() as u32;
    if next > u16_off {
      return if round_up {
        byte_idx + ch.len_utf8()
      } else {
        byte_idx
      };
    }
    units = next;
  }
  s.len()
}

/// Snap a UTF-16 offset to a valid scalar-value boundary inside `s`.
///
/// Never lands between a high and low surrogate. Clamps to `[0, utf16_len(s)]`.
pub fn snap_utf16_boundary(s: &str, u16_off: u32, forward: bool) -> u32 {
  let mut units = 0u32;
  for ch in s.chars() {
    if units == u16_off {
      return units;
    }
    let next = units + ch.len_utf16() as u32;
    if next > u16_off {
      return if forward { next } else { units };
    }
    units = next;
  }
  units
}

/// Previous scalar-value boundary strictly before `offset` (in UTF-16 units).
/// Returns `0` when `offset == 0`.
pub fn utf16_prev_boundary(s: &str, offset: u32) -> u32 {
  let target = snap_utf16_boundary(s, offset, false);
  if target == 0 {
    return 0;
  }
  let mut prev = 0u32;
  let mut units = 0u32;
  for ch in s.chars() {
    if units >= target {
      break;
    }
    prev = units;
    units += ch.len_utf16() as u32;
  }
  prev
}

/// Next scalar-value boundary strictly after `offset` (in UTF-16 units).
/// Returns `utf16_len(s)` when at or past the end.
pub fn utf16_next_boundary(s: &str, offset: u32) -> u32 {
  let target = snap_utf16_boundary(s, offset, true);
  let mut units = 0u32;
  for ch in s.chars() {
    let next = units + ch.len_utf16() as u32;
    if units == target {
      return next;
    }
    units = next;
  }
  units
}

/// Slice `s` by UTF-16 `[start, end)`, clamping and snapping off surrogate halves.
pub fn utf16_slice(s: &str, start: u32, end: u32) -> &str {
  let total = utf16_len(s);
  let s_u16 = start.min(total);
  let e_u16 = end.clamp(s_u16, total);
  let lo = utf16_to_byte(s, s_u16, false);
  let hi = utf16_to_byte(s, e_u16, true).max(lo);
  &s[lo..hi]
}

/// Split `s` at UTF-16 offset `at`. Snaps forward if `at` lands mid-surrogate.
pub fn utf16_split_at(s: &str, at: u32) -> (&str, &str) {
  let byte_idx = utf16_to_byte(s, at, true);
  s.split_at(byte_idx)
}

/// Replace UTF-16 range `[start, end)` in `s` with `replacement`.
/// Returns `(new_string, actual_start_u16, actual_end_u16)` where start/end are
/// snapped off surrogate pairs.
pub fn utf16_replace_range(s: &str, start: u32, end: u32, replacement: &str) -> (String, u32, u32) {
  let total = utf16_len(s);
  let s_u16 = snap_utf16_boundary(s, start.min(total), false);
  let e_u16 = snap_utf16_boundary(s, end.clamp(s_u16, total), true);
  let lo = utf16_to_byte(s, s_u16, false);
  let hi = utf16_to_byte(s, e_u16, true);
  let mut out = String::with_capacity(lo + replacement.len() + (s.len() - hi));
  out.push_str(&s[..lo]);
  out.push_str(replacement);
  out.push_str(&s[hi..]);
  (out, s_u16, e_u16)
}

#[inline]
pub fn is_low_surrogate(u: u16) -> bool {
  (0xDC00..=0xDFFF).contains(&u)
}

/// Length of the longest common prefix of `a` and `b` in UTF-16 code units,
/// backed off if it would split a surrogate pair.
pub fn common_prefix_utf16(a: &str, b: &str) -> u32 {
  let au: Vec<u16> = a.encode_utf16().collect();
  let bu: Vec<u16> = b.encode_utf16().collect();
  let n = au.len().min(bu.len());
  let mut i = 0usize;
  while i < n && au[i] == bu[i] {
    i += 1;
  }
  if i > 0
    && ((i < au.len() && is_low_surrogate(au[i])) || (i < bu.len() && is_low_surrogate(bu[i])))
  {
    i -= 1;
  }
  i as u32
}

/// Length of the longest common suffix of `a` and `b` in UTF-16 code units
/// starting at or after `prefix` (never overlapping `prefix`, never splitting a
/// surrogate pair).
pub fn common_suffix_utf16(a: &str, b: &str, prefix: u32) -> u32 {
  let au: Vec<u16> = a.encode_utf16().collect();
  let bu: Vec<u16> = b.encode_utf16().collect();
  let p = prefix as usize;
  let max = au.len().min(bu.len()).saturating_sub(p);
  let mut i = 0usize;
  while i < max && au[au.len() - 1 - i] == bu[bu.len() - 1 - i] {
    i += 1;
  }
  if i > 0 && (is_low_surrogate(au[au.len() - i]) || is_low_surrogate(bu[bu.len() - i])) {
    i -= 1;
  }
  i as u32
}

/// Compute the replaced range `(prefix, old_end)` in `old_text` for an edit to
/// `new_text`, biased toward the pre-edit selection `[prev_start, prev_end]`
/// when repeated characters make the raw prefix/suffix diff ambiguous
/// (mirrors `text_common.dart::diffEditRange`, gh#192).
pub fn diff_edit_range(
  old_text: &str,
  new_text: &str,
  prev_start: Option<u32>,
  prev_end: Option<u32>,
  next_caret: Option<u32>,
) -> (u32, u32) {
  let old_len = utf16_len(old_text);
  let new_len = utf16_len(new_text);
  let mut prefix = common_prefix_utf16(old_text, new_text);
  let mut old_end = old_len - common_suffix_utf16(old_text, new_text, prefix);

  if let (Some(ps), Some(pe)) = (prev_start, prev_end) {
    let s = snap_utf16_boundary(old_text, ps.min(old_len), false);
    let e = snap_utf16_boundary(old_text, pe.clamp(s, old_len), true);
    if s != e {
      if prefix > s {
        prefix = s;
      }
      if old_end < e {
        old_end = e;
      }
    } else {
      let delta = new_len as i64 - old_len as i64;
      if delta > 0
        && old_end == prefix
        && prefix > s
        && next_caret.is_none_or(|nc| nc as i64 == s as i64 + delta)
        && utf16_slice(old_text, s, old_len)
          == utf16_slice(new_text, (s as i64 + delta) as u32, new_len)
      {
        prefix = s;
        old_end = s;
      } else if delta < 0
        && let Some(nc) = next_caret
        && nc as i64 == s as i64 + delta
        && prefix > nc
        && utf16_slice(old_text, 0, nc) == utf16_slice(new_text, 0, nc)
        && utf16_slice(old_text, s, old_len) == utf16_slice(new_text, nc, new_len)
      {
        prefix = nc;
        old_end = s;
      }
    }
  }
  (prefix, old_end)
}

/// Extract the inserted text turning `old` into `next` (mirrors
/// `prose_input.dart::insertedText`).
pub fn inserted_text_utf16(
  old: &str,
  next: &str,
  sel_start: Option<u32>,
  sel_end: Option<u32>,
) -> String {
  let old_len = utf16_len(old);
  let next_len = utf16_len(next);
  if let (Some(s0), Some(e0)) = (sel_start, sel_end)
    && s0 < e0
  {
    let start = snap_utf16_boundary(old, s0.min(old_len), false);
    let end = snap_utf16_boundary(old, e0.clamp(start, old_len), true);
    let tail = old_len - end;
    if next_len >= start + tail
      && utf16_slice(next, 0, start) == utf16_slice(old, 0, start)
      && utf16_slice(next, next_len - tail, next_len) == utf16_slice(old, end, old_len)
    {
      return utf16_slice(next, start, next_len - tail).to_string();
    }
  }
  let prefix = common_prefix_utf16(old, next);
  let suffix = common_suffix_utf16(old, next, prefix);
  utf16_slice(next, prefix, next_len - suffix).to_string()
}

/// Find the UTF-16 offset of the first occurrence of ASCII char `needle` in `s`
/// at or after `from_u16`, or `None`.
pub fn utf16_find_char_from(s: &str, needle: char, from_u16: u32) -> Option<u32> {
  let mut units = 0u32;
  for ch in s.chars() {
    if units >= from_u16 && ch == needle {
      return Some(units);
    }
    units += ch.len_utf16() as u32;
  }
  None
}

/// Find the UTF-16 offset of the last occurrence of `needle` in `s` strictly
/// before or at `before_u16` (inclusive), or `None`.
pub fn utf16_rfind_char_before(s: &str, needle: char, max_u16: u32) -> Option<u32> {
  let mut found = None;
  let mut units = 0u32;
  for ch in s.chars() {
    if units > max_u16 {
      break;
    }
    if ch == needle {
      found = Some(units);
    }
    units += ch.len_utf16() as u32;
  }
  found
}
