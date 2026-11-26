//! Inline rich-text marks over plain text (UTF-16 code-unit offsets).
//!
//! Semantically equivalent to `clients/mica_flutter/lib/editor/marks.dart`,
//! preserving every invariant including zero-width links, non-inclusive leading
//! edges (`link`, `math`, `footnote`; gh#192), distinct link titles (#150),
//! atomic inline `math` runs, and first-character link click exclusion.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One inline mark over UTF-16 range `[start, end)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EditorMark {
  /// Inclusive UTF-16 start offset.
  pub start: u32,
  /// Exclusive UTF-16 end offset.
  pub end: u32,
  /// Mark type (`bold`, `italic`, `code`, `strike`, `highlight`, `link`, `math`, `footnote`, `html`, `image`).
  #[serde(rename = "type")]
  pub kind: String,
  /// Optional destination for `link` / `image` / `footnote`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub href: Option<String>,
  /// Optional title for `link` / `image`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
}

impl EditorMark {
  pub fn new(start: u32, end: u32, kind: impl Into<String>) -> Self {
    Self {
      start,
      end,
      kind: kind.into(),
      href: None,
      title: None,
    }
  }

  pub fn with_href(mut self, href: Option<String>) -> Self {
    self.href = href;
    self
  }

  pub fn with_title(mut self, title: Option<String>) -> Self {
    self.title = title;
    self
  }

  pub fn with_range(&self, start: u32, end: u32) -> Self {
    Self {
      start,
      end,
      kind: self.kind.clone(),
      href: self.href.clone(),
      title: self.title.clone(),
    }
  }

  pub fn shifted(&self, delta: u32) -> Self {
    Self {
      start: self.start.saturating_add(delta),
      end: self.end.saturating_add(delta),
      kind: self.kind.clone(),
      href: self.href.clone(),
      title: self.title.clone(),
    }
  }
}

impl From<mica_md::Mark> for EditorMark {
  fn from(m: mica_md::Mark) -> Self {
    Self {
      start: m.start as u32,
      end: m.end as u32,
      kind: m.kind,
      href: m.href,
      title: m.title,
    }
  }
}

impl From<&EditorMark> for mica_md::Mark {
  fn from(m: &EditorMark) -> Self {
    Self {
      start: m.start as usize,
      end: m.end as usize,
      kind: m.kind.clone(),
      href: m.href.clone(),
      title: m.title.clone(),
    }
  }
}

impl From<mica_core::Mark> for EditorMark {
  fn from(m: mica_core::Mark) -> Self {
    Self {
      start: m.start,
      end: m.end,
      kind: m.ty,
      href: m.href,
      title: m.title,
    }
  }
}

impl From<&EditorMark> for mica_core::Mark {
  fn from(m: &EditorMark) -> Self {
    Self {
      start: m.start,
      end: m.end,
      ty: m.kind.clone(),
      href: m.href.clone(),
      title: m.title.clone(),
    }
  }
}

/// Extract marks from a block's `data.marks` JSON array.
///
/// Zero-width ranges are legal for `link` (`[](/url)`), matching `marks.dart`.
pub fn marks_from_data(data: &Value) -> Vec<EditorMark> {
  let Some(arr) = data.get("marks").and_then(Value::as_array) else {
    return Vec::new();
  };
  let mut out = Vec::new();
  for m in arr {
    let Some(obj) = m.as_object() else { continue };
    let (Some(start), Some(end), Some(kind)) = (
      obj.get("start").and_then(Value::as_u64),
      obj.get("end").and_then(Value::as_u64),
      obj.get("type").and_then(Value::as_str),
    ) else {
      continue;
    };
    if end > start || (kind == "link" && end == start) {
      out.push(EditorMark {
        start: start as u32,
        end: end as u32,
        kind: kind.to_string(),
        href: obj.get("href").and_then(Value::as_str).map(String::from),
        title: obj.get("title").and_then(Value::as_str).map(String::from),
      });
    }
  }
  out
}

/// Serialize marks into the JSON array stored under `data.marks`.
pub fn marks_to_json(marks: &[EditorMark]) -> Value {
  Value::Array(
    marks
      .iter()
      .map(|m| {
        let mut obj = serde_json::Map::new();
        obj.insert("start".into(), m.start.into());
        obj.insert("end".into(), m.end.into());
        obj.insert("type".into(), m.kind.clone().into());
        if let Some(h) = &m.href {
          obj.insert("href".into(), h.clone().into());
        }
        if let Some(t) = &m.title {
          obj.insert("title".into(), t.clone().into());
        }
        Value::Object(obj)
      })
      .collect(),
  )
}

/// Whether `[from, to)` is fully covered by a mark of `kind`.
pub fn range_has_mark(marks: &[EditorMark], from: u32, to: u32, kind: &str) -> bool {
  marks
    .iter()
    .any(|m| m.kind == kind && m.start <= from && m.end >= to)
}

/// Clip `marks` to `[from, to)` and rebase to 0.
///
/// Partially overlapping marks survive on their overlap; zero-width links
/// landing inside `[from, to]` survive (matching `marks.dart::clipMarks`).
pub fn clip_marks(marks: &[EditorMark], from: u32, to: u32) -> Vec<EditorMark> {
  let mut clipped = Vec::new();
  for m in marks {
    let start = m.start.clamp(from, to);
    let end = m.end.clamp(from, to);
    let empty = end == start;
    if end > start || (empty && m.kind == "link" && m.start >= from && m.end <= to) {
      clipped.push(EditorMark {
        start: start - from,
        end: end - from,
        kind: m.kind.clone(),
        href: m.href.clone(),
        title: m.title.clone(),
      });
    }
  }
  clipped
}

/// The `math` run a caret at `caret` sits STRICTLY inside, or `None`.
pub fn math_run_at(marks: &[EditorMark], caret: u32) -> Option<&EditorMark> {
  marks
    .iter()
    .find(|m| m.kind == "math" && caret > m.start && caret < m.end)
}

/// The `math` run ending exactly at `caret` (for atomic Backspace).
pub fn math_run_ending_at(marks: &[EditorMark], caret: u32) -> Option<&EditorMark> {
  marks
    .iter()
    .find(|m| m.kind == "math" && m.end == caret && m.end > m.start)
}

/// The `math` run starting exactly at `caret` (for atomic Delete).
pub fn math_run_starting_at(marks: &[EditorMark], caret: u32) -> Option<&EditorMark> {
  marks
    .iter()
    .find(|m| m.kind == "math" && m.start == caret && m.end > m.start)
}

/// Snap an offset strictly inside a `math` run to one of its edges.
///
/// - `pref_end == Some(true)` → `run.end`
/// - `pref_end == Some(false)` → `run.start`
/// - `pref_end == None` → nearer edge (ties snap to `run.start`).
pub fn snap_out_of_math_run(marks: &[EditorMark], offset: u32, pref_end: Option<bool>) -> u32 {
  let Some(run) = math_run_at(marks, offset) else {
    return offset;
  };
  if let Some(end_pref) = pref_end {
    return if end_pref { run.end } else { run.start };
  }
  if offset - run.start <= run.end - offset {
    run.start
  } else {
    run.end
  }
}

/// Add a mark of `kind` over `[from, to)`, trimming overlapping same-kind marks
/// and normalizing adjacent identical runs.
pub fn apply_mark(
  marks: &[EditorMark],
  from: u32,
  to: u32,
  kind: &str,
  href: Option<&str>,
  title: Option<&str>,
) -> Vec<EditorMark> {
  apply_or_remove_mark(marks, from, to, kind, href, title, true)
}

/// Remove marks of `kind` from `[from, to)`, splitting any mark that straddles
/// the range.
pub fn remove_mark(marks: &[EditorMark], from: u32, to: u32, kind: &str) -> Vec<EditorMark> {
  apply_or_remove_mark(marks, from, to, kind, None, None, false)
}

/// Add (`add == true`) or remove (`add == false`) `kind` over `[from, to)`.
pub fn apply_or_remove_mark(
  marks: &[EditorMark],
  from: u32,
  to: u32,
  kind: &str,
  href: Option<&str>,
  title: Option<&str>,
  add: bool,
) -> Vec<EditorMark> {
  if to <= from {
    return marks.to_vec();
  }
  let mut result = Vec::with_capacity(marks.len() + usize::from(add));
  for m in marks {
    if m.kind != kind || m.end <= from || m.start >= to {
      result.push(m.clone());
      continue;
    }
    if m.start < from {
      result.push(m.with_range(m.start, from));
    }
    if m.end > to {
      result.push(m.with_range(to, m.end));
    }
  }
  if add {
    result.push(EditorMark {
      start: from,
      end: to,
      kind: kind.to_string(),
      href: href.map(String::from),
      title: title.map(String::from),
    });
  }
  normalize_marks(result)
}

/// Mark types that never swallow plain text inserted at their leading edge
/// (typing before a link, inline formula, or footnote produces unmarked text;
/// gh#192).
#[inline]
fn non_inclusive_leading_edge(kind: &str) -> bool {
  matches!(kind, "link" | "math" | "footnote")
}

/// Shift mark offsets for a text edit replacing `[edit_start, edit_old_end)`
/// with a net change of `delta` UTF-16 code units, yielding total length
/// `new_len`.
///
/// Exact port of `marks.dart::shiftMarks`.
pub fn shift_marks(
  marks: &[EditorMark],
  edit_start: u32,
  edit_old_end: u32,
  delta: i64,
  new_len: u32,
  pin_at_start: bool,
) -> Vec<EditorMark> {
  let pure_insert = edit_start == edit_old_end && delta > 0;
  let edit_start_i = edit_start as i64;
  let edit_old_end_i = edit_old_end as i64;
  let new_len_i = new_len as i64;

  let mut result = Vec::with_capacity(marks.len());
  for m in marks {
    let ms = m.start as i64;
    let me = m.end as i64;
    let (s, e) = if pure_insert {
      if me <= edit_start_i {
        (ms, me)
      } else if ms > edit_start_i
        || (ms == edit_start_i && (!pin_at_start || non_inclusive_leading_edge(&m.kind)))
      {
        (ms + delta, me + delta)
      } else {
        (ms, me + delta)
      }
    } else {
      let move_pt = |x: i64| -> i64 {
        if pin_at_start {
          if x <= edit_start_i {
            return x;
          }
          if x >= edit_old_end_i {
            return x + delta;
          }
          edit_start_i
        } else {
          if x >= edit_old_end_i {
            return x + delta;
          }
          if x > edit_start_i {
            return edit_start_i;
          }
          x
        }
      };
      (move_pt(ms), move_pt(me))
    };
    let cs = s.clamp(0, new_len_i) as u32;
    let ce = e.clamp(0, new_len_i) as u32;
    if ce > cs {
      result.push(m.with_range(cs, ce));
    }
  }
  normalize_marks(result)
}

/// Split marks at UTF-16 offset `at`: left gets `[0, at)`, right gets `[at, ..)`
/// rebased to 0. Marks straddling `at` are divided cleanly.
pub fn split_marks(marks: &[EditorMark], at: u32) -> (Vec<EditorMark>, Vec<EditorMark>) {
  let mut before = Vec::new();
  let mut after = Vec::new();
  for m in marks {
    if m.start < at {
      let end = m.end.min(at);
      if end > m.start {
        before.push(m.with_range(m.start, end));
      }
    }
    if m.end > at {
      let start = m.start.max(at);
      if m.end > start {
        after.push(m.with_range(start - at, m.end - at));
      }
    }
  }
  (before, after)
}

/// Concatenate marks of two joined texts where `b` starts at UTF-16 `junction`.
pub fn concat_marks(a: &[EditorMark], b: &[EditorMark], junction: u32) -> Vec<EditorMark> {
  let mut combined = Vec::with_capacity(a.len() + b.len());
  combined.extend(a.iter().cloned());
  combined.extend(b.iter().map(|m| m.shifted(junction)));
  normalize_marks(combined)
}

/// Sort and coalesce overlapping/touching marks of the same kind + metadata
/// (`href` AND `title`, preserving issue #150). Drops empty ranges (`end <= start`).
pub fn normalize_marks(mut marks: Vec<EditorMark>) -> Vec<EditorMark> {
  if marks.is_empty() {
    return marks;
  }
  marks.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.start.cmp(&b.start)));
  let mut out: Vec<EditorMark> = Vec::with_capacity(marks.len());
  for m in marks {
    if m.end <= m.start {
      continue;
    }
    if let Some(last) = out.last_mut()
      && last.kind == m.kind
      && last.href == m.href
      && last.title == m.title
      && last.end >= m.start
    {
      if m.end > last.end {
        last.end = m.end;
      }
      continue;
    }
    out.push(m);
  }
  out
}

/// Order-independent value equality of two mark lists.
pub fn marks_equal(a: &[EditorMark], b: &[EditorMark]) -> bool {
  if a.len() != b.len() {
    return false;
  }
  if a.is_empty() {
    return true;
  }
  let cmp = |x: &EditorMark, y: &EditorMark| {
    (
      x.start,
      x.end,
      x.kind.as_str(),
      x.href.as_deref(),
      x.title.as_deref(),
    )
      .cmp(&(
        y.start,
        y.end,
        y.kind.as_str(),
        y.href.as_deref(),
        y.title.as_deref(),
      ))
  };
  let mut xa = a.to_vec();
  let mut xb = b.to_vec();
  xa.sort_by(cmp);
  xb.sort_by(cmp);
  xa == xb
}

/// Whether a click landing at caret `offset` should activate link `mark`.
///
/// For links longer than 1 UTF-16 unit, `offset == mark.start` is reserved for
/// placing the caret before the link (2026-08-12 fix); 1-unit links remain
/// clickable at `mark.start`.
pub fn link_click_hit(mark: &EditorMark, offset: u32) -> bool {
  if mark.kind != "link" || mark.href.is_none() {
    return false;
  }
  if offset >= mark.end {
    return false;
  }
  let first_interior = if mark.end.saturating_sub(mark.start) > 1 {
    mark.start + 1
  } else {
    mark.start
  };
  offset >= first_interior
}
