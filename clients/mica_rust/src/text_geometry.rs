//! Pure-Rust UTF-16 ↔ visual coordinate geometry for virtualized text blocks.
//!
//! Uses `nsh_core::TextLine` snapshots emitted by `EventKind::TextLayout` on
//! visible `virtual_list` rows together with `nsh_core::element::text_measurer()`
//! to compute exact caret rectangles, intra/cross-block selection rectangles,
//! click/drag UTF-16 hit-test offsets, and vertical arrow navigation without
//! ever laying out off-screen blocks.

use std::{
  borrow::Cow,
  collections::{HashMap, hash_map::DefaultHasher},
  hash::{Hash, Hasher},
};

use mica_editor::utf16::{utf16_len, utf16_slice};
use nsh_core::{
  TextLine,
  element::ElementRef,
  geometry::{Bounds, Points},
};

/// Visual layout geometry captured for a currently mounted text block.
#[derive(Clone, Debug, Default)]
pub struct BlockTextLayout {
  /// Wrapped visual lines reported by `EventKind::TextLayout` (UTF-16 offsets).
  pub lines: Vec<TextLine>,
  /// CSS font shorthand matching the rendered block (passed to `text_measurer`).
  pub font: String,
  /// Fallback single-line height in logical pixels when `lines` is empty.
  pub line_height: f32,
  /// Text box bounds relative to its parent block container.
  pub text_frame: Bounds<Points>,
  /// Block row bounds (for cross-block hit-testing and caret/IME positioning).
  pub row_frame: Bounds<Points>,
}

#[derive(Clone, Debug)]
struct SynthesizedLinesCache {
  text_len: usize,
  text_hash: u64,
  width_bits: u32,
  font: String,
  line_height_bits: u32,
  lines: Vec<TextLine>,
}

fn hash_str(s: &str) -> u64 {
  let mut h = DefaultHasher::new();
  s.hash(&mut h);
  h.finish()
}

/// Shared visible-only layout cache keyed by `block_id`.
/// Only blocks mounted inside `virtual_list` populate entries here.
#[derive(Default)]
pub struct VisibleLayoutCache {
  pub blocks: HashMap<String, BlockTextLayout>,
  /// Live `ElementRef` for each currently mounted `virtual_list` row (`block_id -> (index, ref)`).
  pub row_refs: HashMap<String, (usize, ElementRef)>,
  /// Live `ElementRef` for each currently mounted prose text box (`block_id -> ref`).
  pub text_refs: HashMap<String, ElementRef>,
  /// Per-block memoized fallback lines when native `EventKind::TextLayout` is
  /// stale (e.g. immediately after Backspace / typing before next layout tick).
  synth_cache: HashMap<String, SynthesizedLinesCache>,
}

impl VisibleLayoutCache {
  /// Updates `lines` for `block_id`, returning `true` iff the line geometry changed.
  /// Never lets a transient empty `TextLayout([])` overwrite already-valid lines.
  pub fn update_lines(
    &mut self,
    block_id: &str,
    lines: Vec<TextLine>,
    font: String,
    line_height: f32,
  ) -> bool {
    let entry = self.blocks.entry(block_id.to_owned()).or_default();
    if lines.is_empty() && !entry.lines.is_empty() {
      return false;
    }
    let changed = entry.lines != lines
      || entry.font != font
      || (entry.line_height - line_height).abs() > 0.1;
    if changed {
      entry.lines = lines;
      entry.font = font;
      entry.line_height = line_height;
      self.synth_cache.remove(block_id);
    }
    changed
  }

  /// Stores block typography eagerly on mount so fallback line synthesis has
  /// exact font & line-height metrics even before `EventKind::TextLayout` fires.
  pub fn update_typography(&mut self, block_id: &str, font: &str, line_height: f32) {
    let entry = self.blocks.entry(block_id.to_owned()).or_default();
    if entry.font != font {
      entry.font = font.to_owned();
    }
    if (entry.line_height - line_height).abs() > 0.1 {
      entry.line_height = line_height;
    }
  }

  /// Updates `text_frame` for `block_id`, returning `true` when the measured
  /// width changed meaningfully while `lines` is still empty (so fallback line
  /// wrapping can re-render to the exact column width).
  pub fn update_text_frame(&mut self, block_id: &str, frame: Bounds<Points>) -> bool {
    let entry = self.blocks.entry(block_id.to_owned()).or_default();
    let width_changed = (entry.text_frame.size.width.0 - frame.size.width.0).abs() > 0.5;
    entry.text_frame = frame;
    width_changed && entry.lines.is_empty()
  }

  pub fn update_row_frame(&mut self, block_id: &str, frame: Bounds<Points>) {
    let entry = self.blocks.entry(block_id.to_owned()).or_default();
    entry.row_frame = frame;
  }

  pub fn register_row_ref(&mut self, block_id: &str, index: usize, r: ElementRef) {
    self.row_refs.insert(block_id.to_owned(), (index, r));
  }

  pub fn register_text_ref(&mut self, block_id: &str, r: ElementRef) {
    self.text_refs.insert(block_id.to_owned(), r);
  }

  pub fn snapshot_row_refs(&self) -> Vec<(String, usize, ElementRef)> {
    self
      .row_refs
      .iter()
      .map(|(id, &(idx, r))| (id.clone(), idx, r))
      .collect()
  }

  pub fn prune_to_live(&mut self, live_ids: &std::collections::HashSet<&str>) {
    self.blocks.retain(|id, _| live_ids.contains(id.as_str()));
    self.row_refs.retain(|id, _| live_ids.contains(id.as_str()));
    self.text_refs.retain(|id, _| live_ids.contains(id.as_str()));
    self.synth_cache.retain(|id, _| live_ids.contains(id.as_str()));
  }

  pub fn get(&self, block_id: &str) -> Option<&BlockTextLayout> {
    self.blocks.get(block_id)
  }

  pub fn clear(&mut self) {
    self.blocks.clear();
    self.row_refs.clear();
    self.text_refs.clear();
    self.synth_cache.clear();
  }

  /// Resolves visual `TextLine`s for `block_id`, using:
  /// 1. Native `EventKind::TextLayout` lines in O(1) when fresh and usable;
  /// 2. Per-block memoized synthesized lines in O(1) on repeat reads within the
  ///    same edit frame (e.g. IME sink + caret overlay + KeepVisible scroll);
  /// 3. Incremental line synthesis reusing unchanged lines from the previous
  ///    layout so single-line Backspace/typing in a 100-line code block makes
  ///    only ~1 `text_measurer` call instead of 100-400!
  pub fn resolve_lines_cached<F>(
    &mut self,
    block_id: &str,
    text: &str,
    fallback_width: f32,
    font: &str,
    line_height: f32,
    measure: &F,
  ) -> &[TextLine]
  where
    F: Fn(&str, &str) -> Points,
  {
    let native_usable = self
      .blocks
      .get(block_id)
      .is_some_and(|info| lines_usable_for_text(&info.lines, text));
    if native_usable {
      return &self.blocks[block_id].lines;
    }

    let (eff_font, eff_lh, eff_w) = match self.blocks.get(block_id) {
      Some(info) => (
        if !info.font.is_empty() {
          info.font.clone()
        } else {
          font.to_owned()
        },
        if info.line_height > 1.0 {
          info.line_height
        } else {
          line_height
        },
        if info.text_frame.size.width.0 > 24.0 {
          info.text_frame.size.width.0
        } else {
          fallback_width
        },
      ),
      None => (font.to_owned(), line_height, fallback_width),
    };

    let text_len = text.len();
    let text_hash = hash_str(text);
    let width_bits = eff_w.to_bits();
    let lh_bits = eff_lh.to_bits();

    let cache_hit = self.synth_cache.get(block_id).is_some_and(|c| {
      c.text_len == text_len
        && c.text_hash == text_hash
        && c.width_bits == width_bits
        && c.line_height_bits == lh_bits
        && c.font == eff_font
    });
    if cache_hit {
      return &self.synth_cache[block_id].lines;
    }

    let (prev_lines, prev_width_matches): (Option<&[TextLine]>, bool) =
      if let Some(prev) = self.synth_cache.get(block_id)
        && prev.font == eff_font
        && prev.line_height_bits == lh_bits
      {
        (Some(prev.lines.as_slice()), prev.width_bits == width_bits)
      } else if let Some(info) = self.blocks.get(block_id)
        && !info.lines.is_empty()
        && (info.font.is_empty() || info.font == eff_font)
      {
        let native_w_match = info.text_frame.size.width.0 > 24.0
          && (info.text_frame.size.width.0 - eff_w).abs() <= 0.5;
        (Some(info.lines.as_slice()), native_w_match)
      } else {
        (None, false)
      };

    let lines = synthesize_wrapped_lines_reusing(
      text,
      eff_w,
      &eff_font,
      eff_lh,
      prev_lines,
      prev_width_matches,
      measure,
    );
    self.synth_cache.insert(
      block_id.to_owned(),
      SynthesizedLinesCache {
        text_len,
        text_hash,
        width_bits,
        font: eff_font,
        line_height_bits: lh_bits,
        lines,
      },
    );
    &self.synth_cache[block_id].lines
  }
}

/// Typography parameters for a block kind + heading level under `base_px` / `font_family`.
/// Returns `(font_size_px, line_height_px, font_weight_num, css_font_shorthand)`.
pub fn block_typography(
  kind: &str,
  level: u64,
  base_px: f32,
  font_family: &str,
) -> (f32, f32, u16, String) {
  let base = base_px.clamp(13.0, 22.0);
  let scale = base / 16.0;
  let (size, lh_mult, weight): (f32, f32, u16) = match kind {
    "heading" => match level.clamp(1, 6) {
      1 => ((30.0 * scale).round(), 1.30, 600),
      2 => ((24.0 * scale).round(), 1.35, 600),
      3 => ((20.0 * scale).round(), 1.40, 600),
      4 => ((18.0 * scale).round(), 1.45, 600),
      5 => ((16.0 * scale).round(), 1.50, 600),
      _ => ((14.0 * scale).round(), 1.50, 500),
    },
    "quote" => (base, 1.65, 400),
    _ => (base, 1.65, 400),
  };
  let line_height = (size * lh_mult).round();
  let family = if font_family.trim().is_empty() {
    "system-ui, -apple-system, \"Segoe UI\", \"PingFang SC\", \"Microsoft YaHei\", sans-serif"
  } else {
    font_family
  };
  let css = format!("{weight} {size}px {family}");
  (size, line_height, weight, css)
}

/// Returns `true` when cached `EventKind::TextLayout` lines cover `text` with
/// usable non-zero geometry. Rejects empty, truncated/stale (e.g. pre-edit
/// `"\u{200B}"`), or zero-width layouts so selection and drag hit-testing never
/// degrade to an 8px first-character stub or offset `0`.
pub fn lines_usable_for_text(lines: &[TextLine], text: &str) -> bool {
  if lines.is_empty() {
    return false;
  }
  let visible = text.trim_end_matches('\u{200B}');
  let u16_len = utf16_len(visible);
  if u16_len == 0 {
    return true;
  }
  let max_end = lines.iter().map(|l| l.end).max().unwrap_or(0);
  // Allow a trailing `\n` / `\r` difference of 1 UTF-16 unit (DirectWrite strips
  // trailing newlineLength from `line.end`), and a trailing `\u{200B}` (+1) ONLY
  // when `visible` ends with `\n`. Reject both shorter stale layouts (after insert)
  // AND longer stale layouts (after Backspace / Delete) immediately so caret
  // updates never stall waiting on an asynchronous TextLayout round-trip!
  let min_expected = visible
    .trim_end_matches(['\n', '\r'])
    .encode_utf16()
    .count() as u32;
  let max_expected = if visible.ends_with('\n') {
    u16_len + 1
  } else {
    u16_len
  };
  if max_end < min_expected || max_end > max_expected {
    return false;
  }
  let has_non_ws = visible.chars().any(|c| !c.is_whitespace());
  if has_non_ws {
    if !lines.iter().any(|l| l.width.0 > 0.5) {
      return false;
    }
    if !lines
      .iter()
      .any(|l| !l.text.trim_end_matches(['\n', '\r', '\u{200B}']).is_empty())
    {
      return false;
    }
  }
  let explicit_breaks = visible
    .trim_end_matches(['\n', '\r'])
    .bytes()
    .filter(|&b| b == b'\n')
    .count();
  lines.len() > explicit_breaks
}

/// Measures horizontal advance width of `text` in `font`, preserving trailing
/// whitespace width on engines (notably Windows DirectWrite) whose bare
/// `GetMetrics().width` excludes trailing spaces.
fn measure_text_advance<F>(font: &str, text: &str, measure: &F) -> f32
where
  F: Fn(&str, &str) -> Points,
{
  if text.is_empty() {
    return 0.0;
  }
  if text.ends_with(char::is_whitespace) {
    let mut probe = String::with_capacity(text.len() + 1);
    probe.push_str(text);
    probe.push('|');
    let with_sentinel = measure(font, &probe).0;
    let only_sentinel = measure(font, "|").0;
    let adv = (with_sentinel - only_sentinel).max(0.0);
    if adv > 0.0 {
      return adv;
    }
  }
  measure(font, text).0.max(0.0)
}

/// Pure-Rust wrapped visual line synthesizer backed by the platform's exact
/// `text_measurer`. Used ONLY as an on-demand fallback when `EventKind::TextLayout`
/// has not populated usable lines for a visible block (e.g. under NSH content
/// culling / pacing / recycling / window resize).
pub fn synthesize_wrapped_lines<F>(
  text: &str,
  max_width: f32,
  font: &str,
  line_height: f32,
  measure: &F,
) -> Vec<TextLine>
where
  F: Fn(&str, &str) -> Points,
{
  synthesize_wrapped_lines_reusing(text, max_width, font, line_height, None, false, measure)
}

/// Incremental version of [`synthesize_wrapped_lines`] that reuses unchanged
/// logical lines from `prev_lines` in O(1) without calling `measure`, so editing
/// one line of a large code block never re-measures every unchanged line.
pub fn synthesize_wrapped_lines_reusing<F>(
  text: &str,
  max_width: f32,
  font: &str,
  line_height: f32,
  prev_lines: Option<&[TextLine]>,
  prev_width_matches: bool,
  measure: &F,
) -> Vec<TextLine>
where
  F: Fn(&str, &str) -> Points,
{
  let lh = line_height.max(18.0);
  let make_line = |slice: &str, start: u32, end: u32, y: f32, width: f32| TextLine {
    text: slice.to_owned(),
    start,
    end,
    x: Points(0.0),
    y: Points(y),
    width: Points(width.max(0.0)),
    height: Points(lh),
    ascender: Points(lh * 0.78),
    descender: Points(lh * 0.22),
    cap_height: Points(lh * 0.60),
    x_height: Points(lh * 0.44),
  };

  let clean = text.trim_end_matches('\u{200B}');
  if clean.is_empty() {
    return vec![make_line("", 0, 0, 0.0, 0.0)];
  }

  let wrap_w = if max_width.is_finite() && max_width > 24.0 {
    Some(max_width)
  } else {
    None
  };

  // Index reusable paragraphs from `prev_lines`:
  // - Single-line paragraphs: exact trimmed text -> previous `TextLine`
  // - Wrapped multi-line paragraphs (only when wrap width is unchanged):
  //   concatenated trimmed text -> slice of wrapped `TextLine`s.
  let mut prev_single: HashMap<&str, &TextLine> = HashMap::new();
  let mut prev_wrapped: HashMap<String, &[TextLine]> = HashMap::new();
  if let Some(prev) = prev_lines
    && !prev.is_empty()
  {
    let mut i = 0usize;
    while i < prev.len() {
      let mut j = i;
      while j + 1 < prev.len()
        && prev[j + 1].start == prev[j].end
        && !prev[j].text.trim_end_matches('\u{200B}').ends_with('\n')
      {
        j += 1;
      }
      if i == j {
        let trimmed = prev[i].text.trim_end_matches(['\n', '\r', '\u{200B}']);
        if !trimmed.is_empty()
          && !trimmed.ends_with(char::is_whitespace)
          && prev[i].width.0 > 0.5
          && wrap_w.is_none_or(|limit| prev[i].width.0 <= limit + 0.5)
        {
          prev_single.entry(trimmed).or_insert(&prev[i]);
        }
      } else if prev_width_matches {
        let mut joined = String::new();
        for line in &prev[i..=j] {
          joined.push_str(line.text.trim_end_matches(['\n', '\r', '\u{200B}']));
        }
        if !joined.is_empty() && !joined.ends_with(char::is_whitespace) {
          prev_wrapped.entry(joined).or_insert(&prev[i..=j]);
        }
      }
      i = j + 1;
    }
  }

  let mut out = Vec::new();
  let mut line_idx = 0usize;
  let mut para_u16_start = 0u32;

  for para in clean.split('\n') {
    let para_u16_len = utf16_len(para);
    if para.is_empty() {
      out.push(make_line("", para_u16_start, para_u16_start, line_idx as f32 * lh, 0.0));
      line_idx += 1;
      para_u16_start = para_u16_start.saturating_add(1);
      continue;
    }

    if let Some(reused) = prev_single.get(para) {
      let mut line = (*reused).clone();
      line.text = para.to_owned();
      line.start = para_u16_start;
      line.end = para_u16_start + para_u16_len;
      line.x = Points(0.0);
      line.y = Points(line_idx as f32 * lh);
      line.height = Points(lh);
      out.push(line);
      line_idx += 1;
      para_u16_start = para_u16_start.saturating_add(para_u16_len + 1);
      continue;
    }

    if !prev_wrapped.is_empty()
      && let Some(reused_slice) = prev_wrapped.get(para)
    {
      let mut seg_u16 = para_u16_start;
      for reused in *reused_slice {
        let trimmed = reused.text.trim_end_matches(['\n', '\r', '\u{200B}']);
        let seg_len = utf16_len(trimmed);
        let mut line = reused.clone();
        line.text = trimmed.to_owned();
        line.start = seg_u16;
        line.end = seg_u16 + seg_len;
        line.x = Points(0.0);
        line.y = Points(line_idx as f32 * lh);
        line.height = Points(lh);
        out.push(line);
        line_idx += 1;
        seg_u16 = seg_u16.saturating_add(seg_len);
      }
      para_u16_start = para_u16_start.saturating_add(para_u16_len + 1);
      continue;
    }

    let full_w = measure_text_advance(font, para, measure);
    let Some(limit) = wrap_w else {
      out.push(make_line(
        para,
        para_u16_start,
        para_u16_start + para_u16_len,
        line_idx as f32 * lh,
        full_w,
      ));
      line_idx += 1;
      para_u16_start = para_u16_start.saturating_add(para_u16_len + 1);
      continue;
    };

    if full_w <= limit + 0.5 {
      out.push(make_line(
        para,
        para_u16_start,
        para_u16_start + para_u16_len,
        line_idx as f32 * lh,
        full_w,
      ));
      line_idx += 1;
      para_u16_start = para_u16_start.saturating_add(para_u16_len + 1);
      continue;
    }

    // Collect char byte offsets and cumulative UTF-16 offsets for O(log N) wrap binary search.
    let mut byte_offs: Vec<usize> = Vec::with_capacity(para.len() + 1);
    let mut u16_offs: Vec<u32> = Vec::with_capacity(para.len() + 1);
    let mut acc_u16 = 0u32;
    for (bi, ch) in para.char_indices() {
      byte_offs.push(bi);
      u16_offs.push(acc_u16);
      acc_u16 += ch.len_utf16() as u32;
    }
    byte_offs.push(para.len());
    u16_offs.push(acc_u16);
    let n_chars = byte_offs.len() - 1;

    let mut cur = 0usize;
    while cur < n_chars {
      let rem = &para[byte_offs[cur]..];
      let rem_w = measure_text_advance(font, rem, measure);
      if rem_w <= limit + 0.5 {
        let s_u16 = para_u16_start + u16_offs[cur];
        let e_u16 = para_u16_start + u16_offs[n_chars];
        out.push(make_line(rem, s_u16, e_u16, line_idx as f32 * lh, rem_w));
        line_idx += 1;
        break;
      }

      // Binary search for the longest prefix `cur..hi` fitting within `limit`.
      let mut lo = cur + 1;
      let mut hi = n_chars;
      while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        let seg = &para[byte_offs[cur]..byte_offs[mid]];
        if measure_text_advance(font, seg, measure) <= limit + 0.5 {
          lo = mid;
        } else {
          hi = mid - 1;
        }
      }
      let mut cut = lo.max(cur + 1);

      // Prefer breaking after whitespace when breaking mid-ASCII-word so wrapped
      // lines closely match DirectWrite / CoreText / CSS word wrapping.
      if cut < n_chars {
        let next_ch = para[byte_offs[cut]..].chars().next().unwrap_or(' ');
        if next_ch.is_ascii_alphanumeric() {
          let min_back = cur + ((cut - cur) * 2 / 5).max(1);
          for k in (min_back..cut).rev() {
            let prev_ch = para[byte_offs[k - 1]..byte_offs[k]].chars().next().unwrap_or('a');
            if prev_ch.is_ascii_whitespace() || matches!(prev_ch, '-' | '/' | '，' | '。' | '、') {
              cut = k;
              break;
            }
          }
        }
      }

      let seg = &para[byte_offs[cur]..byte_offs[cut]];
      let seg_w = measure_text_advance(font, seg, measure);
      let s_u16 = para_u16_start + u16_offs[cur];
      let e_u16 = para_u16_start + u16_offs[cut];
      out.push(make_line(seg, s_u16, e_u16, line_idx as f32 * lh, seg_w));
      line_idx += 1;
      cur = cut;
    }

    para_u16_start = para_u16_start.saturating_add(para_u16_len + 1);
  }

  out
}

/// Resolves visual `TextLine`s for a block: returns borrowed native
/// `EventKind::TextLayout` lines in O(1) when usable, or synthesizes wrapped
/// lines via `text_measurer` whenever `TextLayout` is missing, empty, or stale.
pub fn resolve_lines_for_block<'a, F>(
  cached: Option<&'a BlockTextLayout>,
  text: &str,
  fallback_width: f32,
  font: &str,
  line_height: f32,
  measure: &F,
) -> Cow<'a, [TextLine]>
where
  F: Fn(&str, &str) -> Points,
{
  if let Some(info) = cached
    && lines_usable_for_text(&info.lines, text)
  {
    return Cow::Borrowed(info.lines.as_slice());
  }
  let eff_font = cached
    .map(|i| i.font.as_str())
    .filter(|f| !f.is_empty())
    .unwrap_or(font);
  let eff_lh = cached
    .map(|i| i.line_height)
    .filter(|&h| h > 1.0)
    .unwrap_or(line_height);
  let eff_w = cached
    .map(|i| i.text_frame.size.width.0)
    .filter(|&w| w > 24.0)
    .unwrap_or(fallback_width);
  let prev_lines = cached.filter(|i| !i.lines.is_empty()).map(|i| i.lines.as_slice());
  let prev_w_match = cached.is_some_and(|i| {
    i.text_frame.size.width.0 > 24.0 && (i.text_frame.size.width.0 - eff_w).abs() <= 0.5
  });
  Cow::Owned(synthesize_wrapped_lines_reusing(
    text,
    eff_w,
    eff_font,
    eff_lh,
    prev_lines,
    prev_w_match,
    measure,
  ))
}

/// Measures the horizontal advance of UTF-16 prefix `prefix_u16` within `line`,
/// proportionally calibrated against the platform engine's exact `line.width` so
/// caret & click hit-testing match DirectWrite / CoreText / DOM subpixel metrics.
/// Explicitly preserves trailing whitespace width (DirectWrite omits trailing
/// spaces from `line.width`, which otherwise freezes the caret at `x = 0` on
/// indented blank lines).
fn measure_line_prefix<F>(line: &TextLine, offset_in_line_u16: u32, font: &str, measure: &F) -> f32
where
  F: Fn(&str, &str) -> Points,
{
  let visible = line.text.trim_end_matches(['\n', '\r', '\u{200B}']);
  if offset_in_line_u16 == 0 || visible.is_empty() {
    return 0.0;
  }
  let line_u16_len = utf16_len(visible);
  let has_trailing_ws = visible.ends_with(char::is_whitespace);
  if offset_in_line_u16 >= line_u16_len {
    if !has_trailing_ws && line.width.0 > 0.0 {
      return line.width.0;
    }
    return measure_text_advance(font, visible, measure).max(line.width.0.max(0.0));
  }
  let sub = utf16_slice(visible, 0, offset_in_line_u16);
  let measured = measure_text_advance(font, sub, measure);
  if !has_trailing_ws && line.width.0 > 0.0 {
    let full_measured = measure_text_advance(font, visible, measure);
    if full_measured > 0.5 {
      (measured * (line.width.0 / full_measured)).clamp(0.0, line.width.0)
    } else {
      measured.min(line.width.0)
    }
  } else {
    measured
  }
}

/// Find the visual line index containing UTF-16 offset `offset`.
fn line_index_for_offset(lines: &[TextLine], offset: u32) -> usize {
  if lines.is_empty() {
    return 0;
  }
  for (idx, line) in lines.iter().enumerate() {
    let is_last = idx + 1 == lines.len();
    if offset < line.end || (is_last && offset <= line.end) {
      return idx;
    }
    // If offset lands exactly on a hard break or wrap boundary, prefer the start
    // of the next line if the next line starts at `offset`.
    if offset == line.end && !is_last && lines[idx + 1].start == offset {
      return idx + 1;
    }
    if offset == line.end {
      return idx;
    }
  }
  lines.len() - 1
}

/// Compute `(x, y, height)` of the caret at UTF-16 `offset` in local text-box coordinates.
pub fn caret_rect_for_offset<F>(
  lines: &[TextLine],
  offset: u32,
  font: &str,
  fallback_line_h: f32,
  measure: &F,
) -> (f32, f32, f32)
where
  F: Fn(&str, &str) -> Points,
{
  let h_fallback = fallback_line_h.max(18.0);
  if lines.is_empty() {
    return (0.0, 0.0, h_fallback);
  }
  let idx = line_index_for_offset(lines, offset);
  let line = &lines[idx];
  let rel = offset.saturating_sub(line.start);
  let dx = measure_line_prefix(line, rel, font, measure);
  let h = if line.height.0 > 0.0 { line.height.0 } else { h_fallback };
  (line.x.0 + dx, line.y.0, h)
}

/// Compute selection highlight rectangles `(x, y, width, height)` for UTF-16 range `[start, end)`.
pub fn selection_rects_for_range<F>(
  lines: &[TextLine],
  start: u32,
  end: u32,
  font: &str,
  fallback_line_h: f32,
  measure: &F,
) -> Vec<(f32, f32, f32, f32)>
where
  F: Fn(&str, &str) -> Points,
{
  let (lo, hi) = if start <= end { (start, end) } else { (end, start) };
  if lo == hi {
    return Vec::new();
  }
  let h_fallback = fallback_line_h.max(18.0);
  if lines.is_empty() {
    return vec![(0.0, 0.0, 8.0, h_fallback)];
  }
  let mut rects = Vec::new();
  for line in lines {
    if hi <= line.start || lo > line.end {
      continue;
    }
    let seg_start = lo.max(line.start).saturating_sub(line.start);
    let seg_end = hi.min(line.end).saturating_sub(line.start);
    if seg_start == seg_end && hi <= line.end {
      continue;
    }
    let x0 = line.x.0 + measure_line_prefix(line, seg_start, font, measure);
    let x1 = if hi > line.end {
      (line.x.0 + line.width.0 + 6.0).max(x0 + 6.0)
    } else {
      line.x.0 + measure_line_prefix(line, seg_end, font, measure)
    };
    let h = if line.height.0 > 0.0 { line.height.0 } else { h_fallback };
    let w = (x1 - x0).max(4.0);
    rects.push((x0, line.y.0, w, h));
  }
  rects
}

/// Map a local pointer coordinate `(local_x, local_y)` inside a text box to the nearest UTF-16 offset.
pub fn offset_at_point<F>(
  lines: &[TextLine],
  local_x: f32,
  local_y: f32,
  font: &str,
  measure: &F,
) -> u32
where
  F: Fn(&str, &str) -> Points,
{
  if lines.is_empty() {
    return 0;
  }
  // 1. Pick the visual line closest vertically to `local_y`.
  let mut target_idx = lines.len() - 1;
  if local_y <= lines[0].y.0 {
    target_idx = 0;
  } else {
    for (idx, line) in lines.iter().enumerate() {
      let bottom = line.y.0 + line.height.0.max(1.0);
      if local_y < bottom {
        target_idx = idx;
        break;
      }
    }
  }
  let line = &lines[target_idx];
  let visible = line.text.trim_end_matches(['\n', '\r', '\u{200B}']);
  let visual_end = line.start + utf16_len(visible);
  let rel_x = local_x - line.x.0;
  if rel_x <= 0.0 || visible.is_empty() {
    return line.start;
  }
  if line.width.0 > 0.0 && rel_x >= line.width.0 {
    return visual_end;
  }

  // 2. Walk Unicode scalar / UTF-16 boundaries in `visible` to find the nearest character edge.
  let mut best_u16 = 0u32;
  let mut prev_x = 0.0_f32;
  let mut cur_u16 = 0u32;
  for ch in visible.chars() {
    let next_u16 = cur_u16 + ch.len_utf16() as u32;
    let next_x = measure_line_prefix(line, next_u16, font, measure);
    let mid = (prev_x + next_x) * 0.5;
    if rel_x < mid {
      return line.start + cur_u16;
    }
    best_u16 = next_u16;
    prev_x = next_x;
    cur_u16 = next_u16;
  }
  (line.start + best_u16).min(visual_end)
}

/// Move vertically by `delta_lines` (`-1` up, `+1` down) within a wrapped multi-line block.
/// Returns `Some(new_u16_offset)` if the move stays inside this block, or `None` when the
/// caret should cross into the previous/next block.
pub fn vertical_move_offset<F>(
  lines: &[TextLine],
  cur_u16: u32,
  delta_lines: i32,
  desired_x: Option<f32>,
  font: &str,
  measure: &F,
) -> Option<u32>
where
  F: Fn(&str, &str) -> Points,
{
  if lines.len() <= 1 {
    return None;
  }
  let cur_idx = line_index_for_offset(lines, cur_u16) as i32;
  let next_idx = cur_idx + delta_lines;
  if next_idx < 0 || next_idx >= lines.len() as i32 {
    return None;
  }
  let target_x = desired_x.unwrap_or_else(|| {
    let (cx, _, _) = caret_rect_for_offset(lines, cur_u16, font, 24.0, measure);
    cx
  });
  let target_line = &lines[next_idx as usize];
  let mid_y = target_line.y.0 + target_line.height.0 * 0.5;
  Some(offset_at_point(lines, target_x, mid_y, font, measure))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fixed_measure(_font: &str, text: &str) -> Points {
    // ASCII = 8px, CJK/wide = 16px for deterministic layout tests
    let w: f32 = text
      .chars()
      .map(|c| if c.is_ascii() { 8.0 } else { 16.0 })
      .sum();
    Points(w)
  }

  fn mk_line(text: &str, start: u32, y: f32) -> TextLine {
    let len = utf16_len(text) as u32;
    let w = fixed_measure("", text);
    TextLine {
      text: text.to_string(),
      start,
      end: start + len,
      x: Points(0.0),
      y: Points(y),
      width: w,
      height: Points(24.0),
      ascender: Points(18.0),
      descender: Points(6.0),
      cap_height: Points(14.0),
      x_height: Points(10.0),
    }
  }

  #[test]
  fn caret_selection_and_hit_test_work_for_cjk_ascii_and_emoji_surrogates() {
    // "Hi你好😀" => UTF-16 lengths: H(1), i(1), 你(1), 好(1), 😀(2) => total 6
    let lines = vec![mk_line("Hi你好😀", 0, 0.0)];
    let (x0, y0, h0) = caret_rect_for_offset(&lines, 0, "", 24.0, &fixed_measure);
    assert_eq!((x0, y0, h0), (0.0, 0.0, 24.0));

    // After "Hi" (u16=2) => 16px
    let (x2, _, _) = caret_rect_for_offset(&lines, 2, "", 24.0, &fixed_measure);
    assert_eq!(x2, 16.0);

    // After "Hi你好" (u16=4) => 16 + 32 = 48px
    let (x4, _, _) = caret_rect_for_offset(&lines, 4, "", 24.0, &fixed_measure);
    assert_eq!(x4, 48.0);

    // Click near middle of "你" (x=22 => before midpoint 24 => offset 2; x=26 => offset 3)
    assert_eq!(offset_at_point(&lines, 22.0, 12.0, "", &fixed_measure), 2);
    assert_eq!(offset_at_point(&lines, 26.0, 12.0, "", &fixed_measure), 3);
    // Click past end => u16=6
    assert_eq!(offset_at_point(&lines, 999.0, 12.0, "", &fixed_measure), 6);

    // Selection across "你好" (u16=2..4) => x=16, w=32
    let rects = selection_rects_for_range(&lines, 2, 4, "", 24.0, &fixed_measure);
    assert_eq!(rects, vec![(16.0, 0.0, 32.0, 24.0)]);
  }

  #[test]
  fn multi_line_vertical_navigation_and_selection_span_lines() {
    let lines = vec![mk_line("hello ", 0, 0.0), mk_line("world", 6, 24.0)];
    // Down from offset 2 ("he|llo ") => offset 8 ("wo|rld")
    assert_eq!(
      vertical_move_offset(&lines, 2, 1, None, "", &fixed_measure),
      Some(8)
    );
    // Down from line 2 => None (crosses to next block)
    assert_eq!(
      vertical_move_offset(&lines, 8, 1, None, "", &fixed_measure),
      None
    );
    // Multi-line selection 2..9 produces 2 rects
    let rects = selection_rects_for_range(&lines, 2, 9, "", 24.0, &fixed_measure);
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].1, 0.0);
    assert_eq!(rects[1].1, 24.0);
  }

  #[test]
  fn multi_line_code_trailing_newline_and_zwsp_caret_and_hit_test() {
    // Code text `"line1\nline2\n"` rendered with trailing `"\u{200B}"` so line 2 has real geometry.
    let lines = vec![
      mk_line("line1\n", 0, 0.0),
      mk_line("line2\n", 6, 22.0),
      mk_line("\u{200B}", 12, 44.0),
    ];
    // Clicking past right edge of line 0 must land at offset 5 (before '\n'), NOT 6.
    assert_eq!(offset_at_point(&lines, 999.0, 10.0, "", &fixed_measure), 5);
    // Clicking past right edge of line 1 must land at offset 11 (before '\n'), NOT 12.
    assert_eq!(offset_at_point(&lines, 999.0, 30.0, "", &fixed_measure), 11);
    // Caret at end of `"line1\nline2\n"` (u16=12) lands on trailing empty line 2 at x=0, y=44.
    let (cx, cy, _) = caret_rect_for_offset(&lines, 12, "", 22.0, &fixed_measure);
    assert_eq!((cx, cy), (0.0, 44.0));
  }

  #[test]
  fn lines_usable_for_text_rejects_empty_truncated_and_zero_width_layouts() {
    assert!(!lines_usable_for_text(&[], "cloud镜像选择："));
    // Degenerate zero-width layout on non-empty text
    let mut zero = mk_line("cloud镜像选择：", 0, 0.0);
    zero.width = Points(0.0);
    assert!(!lines_usable_for_text(&[zero], "cloud镜像选择："));
    // Truncated stale layout (e.g. old "\u{200B}" entry)
    let zwsp = mk_line("\u{200B}", 0, 0.0);
    assert!(!lines_usable_for_text(&[zwsp], "cloud镜像选择："));
    // Valid full layout passes
    let good = mk_line("cloud镜像选择：", 0, 0.0);
    assert!(lines_usable_for_text(&[good], "cloud镜像选择："));
    // Empty paragraph accepts empty single-line layout
    let empty_line = mk_line("", 0, 0.0);
    assert!(lines_usable_for_text(&[empty_line], ""));
  }

  #[test]
  fn synthesize_wrapped_lines_and_resolve_cover_full_selection_and_hit_test() {
    let text = "cloudimg-amd64-disk-kvm.img 用于虚拟环境的精简内核，它可能会丢失一些模块/配置";
    // Empty cache -> falls back cleanly to wrapped lines instead of 8px stub!
    let empty_cache = BlockTextLayout {
      lines: Vec::new(),
      font: String::new(),
      line_height: 26.0,
      text_frame: Bounds {
        origin: nsh_core::geometry::point(Points(0.0), Points(0.0)),
        size: nsh_core::geometry::size(Points(320.0), Points(52.0)),
      },
      row_frame: Bounds::default(),
    };
    let resolved = resolve_lines_for_block(
      Some(&empty_cache),
      text,
      320.0,
      "",
      26.0,
      &fixed_measure,
    );
    assert!(
      resolved.len() >= 2,
      "long bullet text at 320px must wrap across >=2 lines, got {:?}",
      resolved
    );
    let total_u16 = utf16_len(text);
    let rects = selection_rects_for_range(&resolved, 0, total_u16, "", 26.0, &fixed_measure);
    assert_eq!(rects.len(), resolved.len());
    assert!(
      rects[0].2 > 150.0,
      "first line selection width must span the wrapped line (not 8px stub), got {:?}",
      rects
    );
    assert!(
      rects[1].2 > 40.0,
      "second line selection width must also be highlighted, got {:?}",
      rects
    );
    // Hit-testing middle of line 1 resolves a real mid-string offset, never stuck at 0!
    let hit = offset_at_point(&resolved, 160.0, 12.0, "", &fixed_measure);
    assert!(hit > 10 && hit < total_u16, "expected mid-line hit offset, got {hit}");
  }

  #[test]
  fn lines_usable_for_text_rejects_stale_longer_layout_after_backspace() {
    let stale = vec![mk_line("hello_world", 0, 0.0)];
    assert!(lines_usable_for_text(&stale, "hello_world"));
    assert!(
      !lines_usable_for_text(&stale, "hello_worl"),
      "1-char Backspace shrinking text must reject stale longer TextLayout immediately"
    );
    assert!(
      !lines_usable_for_text(&stale, "hello"),
      "multi-char Backspace must reject stale longer TextLayout"
    );
  }

  #[test]
  fn caret_on_whitespace_only_code_line_measures_trailing_spaces_even_when_line_width_is_zero() {
    // Simulate Windows DirectWrite where `line.width` and bare `measure("    ")`
    // exclude trailing spaces (return 0.0), while `measure("    |") - measure("|")`
    // reports the true 4-space advance width.
    let dwrite_like_measure = |_font: &str, s: &str| -> Points {
      let trimmed = s.trim_end_matches(char::is_whitespace);
      Points(trimmed.chars().count() as f32 * 8.4)
    };
    let mut ws_line = mk_line("    ", 0, 0.0);
    ws_line.width = Points(0.0);
    let (cx4, _, _) = caret_rect_for_offset(&[ws_line.clone()], 4, "14px monospace", 22.0, &dwrite_like_measure);
    let (cx2, _, _) = caret_rect_for_offset(&[ws_line], 2, "14px monospace", 22.0, &dwrite_like_measure);
    assert!(
      (cx4 - 33.6).abs() < 0.5,
      "caret after 4 spaces must render at col 4 (~33.6px), never frozen at x=0, got {cx4}"
    );
    assert!(
      (cx2 - 16.8).abs() < 0.5,
      "caret after 2 spaces must move visibly left to col 2 (~16.8px), got {cx2}"
    );
  }

  #[test]
  fn resolve_lines_cached_reuses_unchanged_lines_and_memoizes_same_frame_reads() {
    use std::cell::Cell;

    let mut lines_src: Vec<String> = (0..60).map(|i| format!("line_{i:02}: set option_{i} enable")).collect();
    let initial_text = lines_src.join("\n");
    let measure_calls = Cell::new(0usize);
    let counting_measure = |font: &str, s: &str| -> Points {
      measure_calls.set(measure_calls.get() + 1);
      fixed_measure(font, s)
    };

    let mut cache = VisibleLayoutCache::default();
    // Seed native layout from initial 60-line code block.
    let initial_lines = synthesize_wrapped_lines(&initial_text, 760.0, "14px monospace", 22.0, &fixed_measure);
    cache.update_lines("code-1", initial_lines, "14px monospace".into(), 22.0);
    cache.update_text_frame(
      "code-1",
      Bounds {
        origin: nsh_core::geometry::point(Points(0.0), Points(0.0)),
        size: nsh_core::geometry::size(Points(760.0), Points(1320.0)),
      },
    );

    // Backspace deletes 1 char on line 59 (leaving 59 of 60 lines unchanged).
    lines_src[59].pop();
    let after_bs = lines_src.join("\n");
    measure_calls.set(0);
    let resolved1 = cache.resolve_lines_cached("code-1", &after_bs, 760.0, "14px monospace", 22.0, &counting_measure);
    assert_eq!(resolved1.len(), 60);
    assert!(
      measure_calls.get() <= 2,
      "Backspace on 1 of 60 lines must reuse 59 cached lines (<=2 measure calls), got {}",
      measure_calls.get()
    );

    // Second read on the exact same frame (e.g. IME sink + caret overlay) hits O(1) cache!
    let calls_before_second = measure_calls.get();
    let resolved2 = cache.resolve_lines_cached("code-1", &after_bs, 760.0, "14px monospace", 22.0, &counting_measure);
    assert_eq!(resolved2.len(), 60);
    assert_eq!(
      measure_calls.get(),
      calls_before_second,
      "second same-frame read must hit synth_cache with 0 measure calls"
    );
  }
}
