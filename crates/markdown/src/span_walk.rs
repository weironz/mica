//! A forward mark sweep behind `html_span` / `render_span` (issue #61).
//!
//! Both renderers walk `pos` through the text; at every step they pick the
//! next mark — earliest clipped start, widest clipped end, a renderer-specific
//! tie-break — then recurse into the marks overlapping the pick. They used to
//! rescan the WHOLE marks slice for each pick and again for each overlap set:
//! O(marks) per emitted span, O(marks²) for a paragraph of sibling marks.
//! This walker answers the same questions off one sort per call: marks
//! activate as `pos` passes their start, die when `pos` passes their end, and
//! the first un-activated mark — stored in selection order — is the next pick
//! when nothing active qualifies. Selection and overlap semantics are
//! unchanged term for term.

use std::cmp::Ordering;

use crate::inline::InlineMark;

/// Tie-break flavour. The Markdown renderer sorts terminal kinds (code/math/
/// html/footnote) LAST on an exact-range tie so they nest inside the nestable
/// one; the HTML renderer keys ties on the mark index alone.
#[derive(Clone, Copy)]
pub(crate) enum SpanTies {
  Html,
  Markdown,
}

/// One emission step: mark `idx` covers `[s, e)` (clipped to the walk range).
pub(crate) struct Pick<'m> {
  pub(crate) s: usize,
  pub(crate) e: usize,
  pub(crate) idx: usize,
  pub(crate) mark: &'m InlineMark,
}

pub(crate) struct SpanWalker<'m> {
  marks: &'m [&'m InlineMark],
  /// Slice indices in selection order: start ASC, then [`cmp_candidates`].
  order: Vec<usize>,
  /// First `order` slot not yet activated; everything before it has start ≤
  /// the last `pos` seen.
  next: usize,
  /// Activated indices. Entries `pos` has passed (end ≤ pos) drop out on the
  /// next [`SpanWalker::next_pick`] and never come back (`pos` only grows).
  active: Vec<usize>,
  hi: usize,
  ties: SpanTies,
}

/// `Less` = `a` sorts before `b` = `a` wins the pick at equal (clipped) start:
/// widest clipped end, then the renderer tie-break, then the HIGHER slice
/// index (the parser appends equal-range outer marks last).
fn cmp_candidates(
  marks: &[&InlineMark],
  hi: usize,
  ties: SpanTies,
  a: usize,
  b: usize,
) -> Ordering {
  let (ma, mb) = (marks[a], marks[b]);
  mb.end
    .min(hi)
    .cmp(&ma.end.min(hi))
    .then_with(|| match ties {
      SpanTies::Markdown => {
        crate::inline::renders_terminal(&ma.kind).cmp(&crate::inline::renders_terminal(&mb.kind))
      }
      SpanTies::Html => Ordering::Equal,
    })
    .then_with(|| {
      if matches!(ties, SpanTies::Markdown) && ma.start == ma.end && mb.start == mb.end {
        a.cmp(&b)
      } else {
        b.cmp(&a)
      }
    })
}

impl<'m> SpanWalker<'m> {
  pub(crate) fn new(marks: &'m [&'m InlineMark], hi: usize, ties: SpanTies) -> Self {
    let mut order: Vec<usize> = (0..marks.len()).collect();
    order.sort_by(|&a, &b| {
      marks[a]
        .start
        .cmp(&marks[b].start)
        .then_with(|| cmp_candidates(marks, hi, ties, a, b))
    });
    SpanWalker {
      marks,
      order,
      next: 0,
      active: Vec::new(),
      hi,
      ties,
    }
  }

  /// The next mark to emit at scan position `pos`; `pos` only ever advances.
  pub(crate) fn next_pick(&mut self, pos: usize) -> Option<Pick<'m>> {
    while self.next < self.order.len() && self.marks[self.order[self.next]].start <= pos {
      let idx = self.order[self.next];
      self.active.push(idx);
      self.next += 1;
    }
    // Active marks all clip to start == pos, so only the end and the
    // tie-break decide; dead ones (end ≤ pos) are purged for good.
    let marks = self.marks;
    let hi = self.hi;
    let ties = self.ties;
    let mut best: Option<usize> = None;
    self.active.retain(|&a| {
      let empty_link = matches!(ties, SpanTies::Markdown)
        && marks[a].kind == "link"
        && marks[a].start == pos
        && marks[a].end == pos;
      if marks[a].end.min(hi) <= pos && !empty_link {
        return false;
      }
      if best.is_none_or(|b| cmp_candidates(marks, hi, ties, a, b) == Ordering::Less) {
        best = Some(a);
      }
      true
    });
    if let Some(a) = best {
      let m = self.marks[a];
      if m.start == m.end {
        self.active.retain(|&idx| idx != a);
      }
      return Some(Pick {
        s: pos,
        e: m.end.min(self.hi),
        idx: a,
        mark: m,
      });
    }
    // No active candidate: pending starts only grow from here and the sort IS
    // the selection order, so the first qualified pending mark is the pick.
    while self.next < self.order.len() {
      let a = self.order[self.next];
      let m = self.marks[a];
      let empty_link = matches!(self.ties, SpanTies::Markdown)
        && m.kind == "link"
        && m.start == m.end
        && m.start <= self.hi;
      if m.end.min(self.hi) <= m.start && !empty_link {
        self.next += 1;
        continue;
      }
      if empty_link {
        self.next += 1;
      }
      return Some(Pick {
        s: m.start,
        e: m.end.min(self.hi),
        idx: a,
        mark: m,
      });
    }
    None
  }

  /// The marks overlapping `(s, e)` — the `inner` recursion set both renderers
  /// built by rescanning the whole slice — in original slice order, the pick
  /// itself excluded. Valid right after the [`SpanWalker::next_pick`] that
  /// returned `pick`.
  pub(crate) fn inner(&self, pick: &Pick) -> Vec<&'m InlineMark> {
    let overlaps = |m: &InlineMark| {
      m.end.min(pick.e) > m.start.max(pick.s)
        || (matches!(self.ties, SpanTies::Markdown)
          && m.kind == "link"
          && m.start == m.end
          && m.start >= pick.s
          && m.start < pick.e)
    };
    let mut idxs: Vec<usize> = Vec::new();
    for &a in &self.active {
      if a != pick.idx && overlaps(self.marks[a]) {
        idxs.push(a);
      }
    }
    for &a in &self.order[self.next..] {
      let m = self.marks[a];
      if m.start >= pick.e {
        break;
      }
      if a != pick.idx && overlaps(m) {
        idxs.push(a);
      }
    }
    idxs.sort_unstable();
    idxs.into_iter().map(|a| self.marks[a]).collect()
  }
}
