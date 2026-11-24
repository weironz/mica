//! Document selection, word boundary selection, active-mark queries, and
//! remote-merge selection reconciliation (UTF-16 offsets).
//!
//! Mirrors `model.dart` (`DocPosition`, `DocSelection`, `wordBoundsAt`),
//! `selection_marks.dart`, and `selection_reconcile.dart`.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::marks::{EditorMark, range_has_mark, snap_out_of_math_run};
use crate::model::EditorBlock;
use crate::utf16::{is_low_surrogate, snap_utf16_boundary, utf16_len};

/// A caret location: flat block index + UTF-16 code-unit offset inside that block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DocPosition {
  pub node: usize,
  pub offset: u32,
}

impl DocPosition {
  pub const fn new(node: usize, offset: u32) -> Self {
    Self { node, offset }
  }

  pub const fn with_offset(self, offset: u32) -> Self {
    Self {
      node: self.node,
      offset,
    }
  }
}

/// Document-wide selection spanning text carets, in-block ranges, cross-block
/// selections, atomic block selections, and table cell / area selections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocSelection {
  Caret {
    block_index: usize,
    offset: u32,
  },
  TextRange {
    block_index: usize,
    anchor: u32,
    focus: u32,
  },
  CrossBlock {
    anchor: DocPosition,
    focus: DocPosition,
  },
  BlockAtom {
    block_index: usize,
  },
  TableCell {
    block_index: usize,
    row: usize,
    col: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end_row: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end_col: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anchor: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    focus: Option<u32>,
  },
}

/// Canonical ordered view of a [`DocSelection`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizedSelection {
  Text {
    start: DocPosition,
    end: DocPosition,
  },
  BlockAtom {
    block_index: usize,
  },
  TableArea {
    block_index: usize,
    r0: usize,
    c0: usize,
    r1: usize,
    c1: usize,
    cell_anchor: Option<u32>,
    cell_focus: Option<u32>,
  },
}

impl DocSelection {
  pub const fn caret(block_index: usize, offset: u32) -> Self {
    Self::Caret {
      block_index,
      offset,
    }
  }

  /// Build the canonical selection variant for an `(anchor, focus)` pair.
  pub fn from_positions(anchor: DocPosition, focus: DocPosition) -> Self {
    if anchor == focus {
      Self::Caret {
        block_index: anchor.node,
        offset: anchor.offset,
      }
    } else if anchor.node == focus.node {
      Self::TextRange {
        block_index: anchor.node,
        anchor: anchor.offset,
        focus: focus.offset,
      }
    } else {
      Self::CrossBlock { anchor, focus }
    }
  }

  pub const fn block_atom(block_index: usize) -> Self {
    Self::BlockAtom { block_index }
  }

  pub const fn table_cell(block_index: usize, row: usize, col: usize) -> Self {
    Self::TableCell {
      block_index,
      row,
      col,
      end_row: None,
      end_col: None,
      anchor: None,
      focus: None,
    }
  }

  pub const fn table_area(block_index: usize, r0: usize, c0: usize, r1: usize, c1: usize) -> Self {
    Self::TableCell {
      block_index,
      row: r0,
      col: c0,
      end_row: Some(r1),
      end_col: Some(c1),
      anchor: None,
      focus: None,
    }
  }

  /// Ordered selection representation (start <= end for text, r0 <= r1 and c0 <= c1 for tables).
  pub fn normalized(&self) -> NormalizedSelection {
    match *self {
      Self::Caret {
        block_index,
        offset,
      } => {
        let p = DocPosition::new(block_index, offset);
        NormalizedSelection::Text { start: p, end: p }
      }
      Self::TextRange {
        block_index,
        anchor,
        focus,
      } => {
        let lo = anchor.min(focus);
        let hi = anchor.max(focus);
        NormalizedSelection::Text {
          start: DocPosition::new(block_index, lo),
          end: DocPosition::new(block_index, hi),
        }
      }
      Self::CrossBlock { anchor, focus } => {
        if anchor <= focus {
          NormalizedSelection::Text {
            start: anchor,
            end: focus,
          }
        } else {
          NormalizedSelection::Text {
            start: focus,
            end: anchor,
          }
        }
      }
      Self::BlockAtom { block_index } => NormalizedSelection::BlockAtom { block_index },
      Self::TableCell {
        block_index,
        row,
        col,
        end_row,
        end_col,
        anchor,
        focus,
      } => {
        let er = end_row.unwrap_or(row);
        let ec = end_col.unwrap_or(col);
        NormalizedSelection::TableArea {
          block_index,
          r0: row.min(er),
          c0: col.min(ec),
          r1: row.max(er),
          c1: col.max(ec),
          cell_anchor: anchor,
          cell_focus: focus,
        }
      }
    }
  }

  pub fn anchor_pos(&self) -> DocPosition {
    match *self {
      Self::Caret {
        block_index,
        offset,
      } => DocPosition::new(block_index, offset),
      Self::TextRange {
        block_index,
        anchor,
        ..
      } => DocPosition::new(block_index, anchor),
      Self::CrossBlock { anchor, .. } => anchor,
      Self::BlockAtom { block_index } | Self::TableCell { block_index, .. } => {
        DocPosition::new(block_index, 0)
      }
    }
  }

  pub fn focus_pos(&self) -> DocPosition {
    match *self {
      Self::Caret {
        block_index,
        offset,
      } => DocPosition::new(block_index, offset),
      Self::TextRange {
        block_index, focus, ..
      } => DocPosition::new(block_index, focus),
      Self::CrossBlock { focus, .. } => focus,
      Self::BlockAtom { block_index } | Self::TableCell { block_index, .. } => {
        DocPosition::new(block_index, 0)
      }
    }
  }

  pub fn start_pos(&self) -> DocPosition {
    let a = self.anchor_pos();
    let f = self.focus_pos();
    if a <= f { a } else { f }
  }

  pub fn end_pos(&self) -> DocPosition {
    let a = self.anchor_pos();
    let f = self.focus_pos();
    if a <= f { f } else { a }
  }

  pub fn focus_node(&self) -> usize {
    self.focus_pos().node
  }

  pub fn is_collapsed(&self) -> bool {
    match *self {
      Self::Caret { .. } | Self::BlockAtom { .. } => true,
      Self::TextRange { anchor, focus, .. } => anchor == focus,
      Self::CrossBlock { anchor, focus } => anchor == focus,
      Self::TableCell {
        row,
        col,
        end_row,
        end_col,
        anchor,
        focus,
        ..
      } => end_row.is_none_or(|r| r == row) && end_col.is_none_or(|c| c == col) && anchor == focus,
    }
  }

  pub fn is_multi_node(&self) -> bool {
    match *self {
      Self::CrossBlock { anchor, focus } => anchor.node != focus.node,
      _ => false,
    }
  }

  /// Clamp selection indices/offsets to `blocks` and snap off surrogate pairs
  /// and interior of inline `math` runs.
  pub fn clamp_and_snap(&self, blocks: &[EditorBlock]) -> Option<Self> {
    if blocks.is_empty() {
      return None;
    }
    let last_idx = blocks.len() - 1;
    match *self {
      Self::BlockAtom { block_index } => {
        let idx = block_index.min(last_idx);
        if blocks[idx].is_atomic() {
          Some(Self::BlockAtom { block_index: idx })
        } else {
          Some(Self::Caret {
            block_index: idx,
            offset: 0,
          })
        }
      }
      Self::TableCell {
        block_index,
        row,
        col,
        end_row,
        end_col,
        anchor,
        focus,
      } => {
        let idx = block_index.min(last_idx);
        if blocks[idx].kind != "table" {
          return Some(Self::Caret {
            block_index: idx,
            offset: 0,
          });
        }
        Some(Self::TableCell {
          block_index: idx,
          row,
          col,
          end_row,
          end_col,
          anchor,
          focus,
        })
      }
      Self::Caret {
        block_index,
        offset,
      } => {
        let idx = block_index.min(last_idx);
        let b = &blocks[idx];
        if b.is_atomic() {
          return Some(Self::BlockAtom { block_index: idx });
        }
        let clamped = snap_utf16_boundary(&b.text, offset.min(b.utf16_len()), false);
        let snapped = snap_out_of_math_run(&b.marks(), clamped, None);
        Some(Self::Caret {
          block_index: idx,
          offset: snapped,
        })
      }
      Self::TextRange {
        block_index,
        anchor,
        focus,
      } => {
        let idx = block_index.min(last_idx);
        let b = &blocks[idx];
        if b.is_atomic() {
          return Some(Self::BlockAtom { block_index: idx });
        }
        let len = b.utf16_len();
        let marks = b.marks();
        let forward = anchor <= focus;
        let a_u16 = snap_utf16_boundary(&b.text, anchor.min(len), !forward);
        let f_u16 = snap_utf16_boundary(&b.text, focus.min(len), forward);
        if a_u16 == f_u16 {
          let snapped = snap_out_of_math_run(&marks, f_u16, None);
          return Some(Self::Caret {
            block_index: idx,
            offset: snapped,
          });
        }
        let a = snap_out_of_math_run(&marks, a_u16, Some(!forward));
        let f = snap_out_of_math_run(&marks, f_u16, Some(forward));
        Some(Self::from_positions(
          DocPosition::new(idx, a),
          DocPosition::new(idx, f),
        ))
      }
      Self::CrossBlock { anchor, focus } => {
        let a_idx = anchor.node.min(last_idx);
        let f_idx = focus.node.min(last_idx);
        let forward = (a_idx, anchor.offset) <= (f_idx, focus.offset);
        let snap_pos = |idx: usize, off: u32, pref_end: bool| -> DocPosition {
          let b = &blocks[idx];
          if b.is_atomic() {
            return DocPosition::new(idx, 0);
          }
          let clamped = snap_utf16_boundary(&b.text, off.min(b.utf16_len()), pref_end);
          let snapped = snap_out_of_math_run(&b.marks(), clamped, Some(pref_end));
          DocPosition::new(idx, snapped)
        };
        let a = snap_pos(a_idx, anchor.offset, !forward);
        let f = snap_pos(f_idx, focus.offset, forward);
        Some(Self::from_positions(a, f))
      }
    }
  }
}

/// Character classes for double-click word selection (mirrors `model.dart::_CharClass`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
  Cjk,
  Alnum,
  Whitespace,
  Other,
}

fn char_class(ch: char) -> CharClass {
  let code = ch as u32;
  if matches!(code, 0x20 | 0x09 | 0x0A | 0x0D) {
    return CharClass::Whitespace;
  }
  if ch.is_ascii_alphanumeric() || ch == '_' {
    return CharClass::Alnum;
  }
  if (0x3400..=0x9FFF).contains(&code)
    || (0x3040..=0x30FF).contains(&code)
    || (0xAC00..=0xD7A3).contains(&code)
    || (0xF900..=0xFAFF).contains(&code)
    || code > 0x7F
  {
    return CharClass::Cjk;
  }
  CharClass::Other
}

/// Bounds `(start_u16, end_u16)` of the "word" containing UTF-16 `offset` in
/// `text` (for double-click selection).
///
/// Groups contiguous CJK/non-ASCII runs and contiguous ASCII identifier runs
/// (`[A-Za-z0-9_]`); whitespace and punctuation select a single scalar value
/// (including full 2-unit surrogate pairs so emoji are never split).
pub fn word_range_at(text: &str, offset: u32) -> (u32, u32) {
  if text.is_empty() {
    return (0, 0);
  }
  let mut chars: Vec<(u32, u32, CharClass)> = Vec::new();
  let mut u16_pos = 0u32;
  for ch in text.chars() {
    let next = u16_pos + ch.len_utf16() as u32;
    chars.push((u16_pos, next, char_class(ch)));
    u16_pos = next;
  }
  let probe_idx = if offset >= u16_pos {
    chars.len() - 1
  } else {
    chars
      .iter()
      .position(|&(s, e, _)| offset >= s && offset < e)
      .unwrap_or(chars.len() - 1)
  };
  let cls = chars[probe_idx].2;
  if matches!(cls, CharClass::Whitespace | CharClass::Other) {
    return (chars[probe_idx].0, chars[probe_idx].1);
  }
  let mut start = probe_idx;
  while start > 0 && chars[start - 1].2 == cls {
    start -= 1;
  }
  let mut end = probe_idx + 1;
  while end < chars.len() && chars[end].2 == cls {
    end += 1;
  }
  (chars[start].0, chars[end - 1].1)
}

/// Inline-text slice covered by a ranged selection (mirrors `selection_marks.dart`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSelectionRange {
  pub block_index: usize,
  pub start: u32,
  pub end: u32,
}

/// Inline-text intersections of `selection` across `blocks`. Atomic and code
/// blocks are excluded even when a cross-block selection crosses them.
pub fn inline_selection_ranges(
  blocks: &[EditorBlock],
  selection: &DocSelection,
) -> Vec<InlineSelectionRange> {
  let NormalizedSelection::Text { start, end } = selection.normalized() else {
    return Vec::new();
  };
  if start == end || end.node >= blocks.len() {
    return Vec::new();
  }
  let mut out = Vec::new();
  for (i, block) in blocks
    .iter()
    .enumerate()
    .take(end.node + 1)
    .skip(start.node)
  {
    if !block.is_inline_text() {
      continue;
    }
    let len = block.utf16_len();
    let s = if i == start.node {
      start.offset.min(len)
    } else {
      0
    };
    let e = if i == end.node {
      end.offset.min(len)
    } else {
      len
    };
    if s < e {
      out.push(InlineSelectionRange {
        block_index: i,
        start: s,
        end: e,
      });
    }
  }
  out
}

/// Active inline marks at the current selection (for toolbar / format-bar state).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActiveMarks {
  pub kinds: BTreeSet<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub link_href: Option<String>,
}

impl ActiveMarks {
  pub fn has(&self, kind: &str) -> bool {
    self.kinds.contains(kind)
  }
}

/// Compute active marks for the current selection.
///
/// - For a non-collapsed text range: a mark is active iff EVERY inline-text
///   slice in the selection is covered by that mark (mirrors
///   `selection_marks.dart::inlineSelectionHasMark`).
/// - For a collapsed caret: reports marks covering the caret position
///   (`start < caret && end >= caret` for inclusive marks, `start < caret && end > caret`
///   for non-inclusive `link`/`math`/`footnote`).
pub fn active_marks_for_selection(
  blocks: &[EditorBlock],
  selection: Option<&DocSelection>,
) -> ActiveMarks {
  let Some(sel) = selection else {
    return ActiveMarks::default();
  };
  match sel.normalized() {
    NormalizedSelection::BlockAtom { .. } | NormalizedSelection::TableArea { .. } => {
      ActiveMarks::default()
    }
    NormalizedSelection::Text { start, end } if start == end => {
      let Some(block) = blocks.get(start.node) else {
        return ActiveMarks::default();
      };
      if !block.is_inline_text() {
        return ActiveMarks::default();
      }
      let caret = start.offset;
      let marks = block.marks();
      let mut out = ActiveMarks::default();
      for m in &marks {
        let non_inc = matches!(m.kind.as_str(), "link" | "math" | "footnote");
        let active = if non_inc {
          caret > m.start && caret < m.end
        } else {
          (caret > m.start && caret <= m.end) || (caret == 0 && m.start == 0 && m.end > 0)
        };
        if active {
          out.kinds.insert(m.kind.clone());
          if m.kind == "link" && out.link_href.is_none() {
            out.link_href = m.href.clone();
          }
        }
      }
      out
    }
    NormalizedSelection::Text { .. } => {
      let ranges = inline_selection_ranges(blocks, sel);
      if ranges.is_empty() {
        return ActiveMarks::default();
      }
      let per_range_marks: Vec<Vec<EditorMark>> = ranges
        .iter()
        .map(|r| blocks[r.block_index].marks())
        .collect();
      let candidate_kinds: BTreeSet<&str> =
        per_range_marks[0].iter().map(|m| m.kind.as_str()).collect();
      let mut out = ActiveMarks::default();
      for kind in candidate_kinds {
        if ranges
          .iter()
          .zip(&per_range_marks)
          .all(|(r, ms)| range_has_mark(ms, r.start, r.end, kind))
        {
          out.kinds.insert(kind.to_string());
          if kind == "link" {
            out.link_href = per_range_marks[0]
              .iter()
              .find(|m| m.kind == "link" && m.start <= ranges[0].start && m.end >= ranges[0].end)
              .and_then(|m| m.href.clone());
          }
        }
      }
      out
    }
  }
}

/// Remap `prev_sel` from `prev_blocks` `(id, text)` onto `next_blocks` after a
/// remote CRDT merge or reconcile.
///
/// Follows block identity across remote insertions and reorders, falls back to
/// the nearest surviving successor then predecessor when a block is deleted,
/// and remaps UTF-16 text offsets using `mica_core::utf16_text_diff` (fast path)
/// plus bounded Myers diff (multi-edit remote merges; mirrors
/// `selection_reconcile.dart`).
pub fn reconcile_selection(
  prev_blocks: &[(String, String)],
  prev_sel: Option<&DocSelection>,
  next_blocks: &[EditorBlock],
) -> Option<DocSelection> {
  let sel = prev_sel?;
  if next_blocks.is_empty() || prev_blocks.is_empty() {
    return None;
  }
  let indices: HashMap<&str, usize> = next_blocks
    .iter()
    .enumerate()
    .map(|(i, b)| (b.id.as_str(), i))
    .collect();

  let mut edit_cache: HashMap<usize, Vec<TextEdit>> = HashMap::new();

  let mut remap_pos = |pos: DocPosition| -> DocPosition {
    let old_idx = pos.node.min(prev_blocks.len() - 1);
    let (old_id, old_text) = &prev_blocks[old_idx];
    if let Some(&next_idx) = indices.get(old_id.as_str()) {
      let next_text = &next_blocks[next_idx].text;
      let edits = edit_cache
        .entry(old_idx)
        .or_insert_with(|| compute_text_edits(old_text, next_text));
      let old_len = utf16_len(old_text);
      let remapped = remap_offset(edits, pos.offset.min(old_len));
      let clamped = snap_utf16_boundary(next_text, remapped.min(utf16_len(next_text)), false);
      return DocPosition::new(next_idx, clamped);
    }
    for (cand_id, _) in &prev_blocks[old_idx + 1..] {
      if let Some(&succ) = indices.get(cand_id.as_str()) {
        return DocPosition::new(succ, 0);
      }
    }
    for (cand_id, _) in prev_blocks[..old_idx].iter().rev() {
      if let Some(&pred) = indices.get(cand_id.as_str()) {
        return DocPosition::new(pred, next_blocks[pred].utf16_len());
      }
    }
    DocPosition::new(old_idx.min(next_blocks.len() - 1), 0)
  };

  let mapped = match *sel {
    DocSelection::BlockAtom { block_index } => {
      let p = remap_pos(DocPosition::new(block_index, 0));
      if next_blocks[p.node].is_atomic() {
        DocSelection::BlockAtom {
          block_index: p.node,
        }
      } else {
        DocSelection::Caret {
          block_index: p.node,
          offset: p.offset,
        }
      }
    }
    DocSelection::TableCell {
      block_index,
      row,
      col,
      end_row,
      end_col,
      anchor,
      focus,
    } => {
      let p = remap_pos(DocPosition::new(block_index, 0));
      if next_blocks[p.node].kind == "table" {
        DocSelection::TableCell {
          block_index: p.node,
          row,
          col,
          end_row,
          end_col,
          anchor,
          focus,
        }
      } else {
        DocSelection::Caret {
          block_index: p.node,
          offset: p.offset,
        }
      }
    }
    DocSelection::Caret {
      block_index,
      offset,
    } => {
      let p = remap_pos(DocPosition::new(block_index, offset));
      DocSelection::Caret {
        block_index: p.node,
        offset: p.offset,
      }
    }
    DocSelection::TextRange {
      block_index,
      anchor,
      focus,
    } => {
      let a = remap_pos(DocPosition::new(block_index, anchor));
      let f = remap_pos(DocPosition::new(block_index, focus));
      DocSelection::from_positions(a, f)
    }
    DocSelection::CrossBlock { anchor, focus } => {
      let a = remap_pos(anchor);
      let f = remap_pos(focus);
      DocSelection::from_positions(a, f)
    }
  };
  mapped.clamp_and_snap(next_blocks)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextEdit {
  start: u32,
  old_end: u32,
  new_end: u32,
}

fn remap_offset(edits: &[TextEdit], offset: u32) -> u32 {
  let mut delta: i64 = 0;
  for edit in edits {
    if offset < edit.start {
      break;
    }
    // Right affinity: an insertion exactly at the caret goes before the caret.
    if offset >= edit.old_end {
      delta = edit.new_end as i64 - edit.old_end as i64;
    } else {
      // Deleted/replaced text maps to the start of this replacement.
      return (edit.start as i64 + delta).max(0) as u32;
    }
  }
  (offset as i64 + delta).max(0) as u32
}

/// Infer UTF-16 edits turning `before` into `after`.
///
/// Uses `mica_core::utf16_text_diff` for the common prefix/suffix trim (which
/// also keeps surrogate pairs intact), then runs bounded Myers diff when both
/// sides have changed interior text so independent remote edits in one block
/// remap accurately.
fn compute_text_edits(before: &str, after: &str) -> Vec<TextEdit> {
  let Some(single) = mica_core::utf16_text_diff(before, after) else {
    return Vec::new();
  };
  let start = single.at;
  let old_end = start + single.del;
  let new_end = start + utf16_len(&single.ins);
  let fallback = vec![TextEdit {
    start,
    old_end,
    new_end,
  }];
  if start == old_end || start == new_end {
    return fallback;
  }

  let a: Vec<u16> = before.encode_utf16().collect();
  let b: Vec<u16> = after.encode_utf16().collect();
  let start_usize = start as usize;
  let old_len = (old_end - start) as usize;
  let new_len = (new_end - start) as usize;

  const MAX_DISTANCE: usize = 128;
  const MAX_WORK: usize = 200_000;
  if old_len.abs_diff(new_len) > MAX_DISTANCE {
    return fallback;
  }

  let mut trace: Vec<Vec<usize>> = Vec::new();
  let mut work = 0usize;
  for distance in 0..=MAX_DISTANCE {
    let mut row = vec![0usize; 2 * distance + 1];
    let d_i = distance as isize;
    let mut k = -d_i;
    while k <= d_i {
      work += 1;
      if work > MAX_WORK {
        return fallback;
      }
      let idx = (k + d_i) as usize;
      let mut x = if distance == 0 {
        0
      } else {
        let prev = &trace[distance - 1];
        let insert = k == -d_i || (k != d_i && prev[idx - 2] < prev[idx]);
        if insert { prev[idx] } else { prev[idx - 2] + 1 }
      };
      let mut y = (x as isize - k) as usize;
      while x < old_len && y < new_len {
        work += 1;
        if work > MAX_WORK {
          return fallback;
        }
        if a[start_usize + x] != b[start_usize + y] {
          break;
        }
        x += 1;
        y += 1;
      }
      row[idx] = x;
      if x == old_len && y == new_len {
        trace.push(row);
        let mut matches: Vec<(usize, usize, usize)> = Vec::new();
        let mut cx = x;
        let mut cy = y;
        for d in (1..=distance).rev() {
          let prev = &trace[d - 1];
          let ck = cx as isize - cy as isize;
          let cd = d as isize;
          let cidx = (ck + cd) as usize;
          let insert = ck == -cd || (ck != cd && prev[cidx - 2] < prev[cidx]);
          let prev_k = if insert { ck + 1 } else { ck - 1 };
          let prev_x = prev[(prev_k + cd - 1) as usize];
          let prev_y = (prev_x as isize - prev_k) as usize;
          let snake_x = prev_x + usize::from(!insert);
          let snake_y = prev_y + usize::from(insert);
          if cx > snake_x {
            matches.push((snake_x, snake_y, cx - snake_x));
          }
          cx = prev_x;
          cy = prev_y;
        }
        if cx > 0 {
          matches.push((0, 0, cx));
        }
        let mut edits = Vec::new();
        let mut old_cur = 0usize;
        let mut new_cur = 0usize;
        for &(ms, ns, mlen) in matches.iter().rev() {
          if old_cur != ms || new_cur != ns {
            // Avoid landing an edit boundary mid-surrogate.
            let mut s0 = start_usize + old_cur;
            let mut oe0 = start_usize + ms;
            let mut ne0 = start_usize + ns;
            if s0 > 0 && s0 < a.len() && is_low_surrogate(a[s0]) {
              s0 -= 1;
            }
            if oe0 < a.len() && is_low_surrogate(a[oe0]) {
              oe0 += 1;
            }
            if ne0 < b.len() && is_low_surrogate(b[ne0]) {
              ne0 += 1;
            }
            edits.push(TextEdit {
              start: s0 as u32,
              old_end: oe0 as u32,
              new_end: ne0 as u32,
            });
          }
          old_cur = ms + mlen;
          new_cur = ns + mlen;
        }
        if old_cur != old_len || new_cur != new_len {
          edits.push(TextEdit {
            start: (start_usize + old_cur) as u32,
            old_end,
            new_end,
          });
        }
        return edits;
      }
      k += 2;
    }
    trace.push(row);
  }
  fallback
}
