//! Per-parse scan memoization for the inline parser (issue #61).
//!
//! Several inline arms answer "is there a closing X ahead?" by scanning to the
//! end of the text, every time, from every position. On adversarial input
//! (unclosed `\(` repeated, `_a` repeated, a `<` with no `>` anywhere) that
//! rescan is quadratic. The parser's main loop only moves FORWARD, so each
//! entry here records what ONE failed scan proved, and later positions reuse
//! the fact instead of rescanning:
//!
//! * [`next_pair_table`] — one backward pass answers every `\)` lookup in O(1).
//! * `math_dead_through` — a failed `$`-closer scan that never SKIPPED over a
//!   closed code span visited every position it passed, so any later start on
//!   the same stretch rescans the same suffix and fails too (every closer
//!   check is a position fact). A scan that DID skip is not a plain suffix
//!   walk from a later start, so it memoises only up to the first skipped
//!   span's opening backtick (`skip_open`): from there to that backtick the
//!   walk is still identical, and beyond it nothing is claimed.
//! * `code_dead` — "no backtick run of length N at or after F" is a fact about
//!   run lengths alone; the smallest such F per length settles every later
//!   `code_span_close` whose searched range is inside the dead zone.
//! * `email_dead` — the bare-email local-part charset never changes, so a scan
//!   from `s` that stopped at `t` and failed stops at the same `t` from EVERY
//!   start in `(s, t]` and fails there identically (`@`, the domain scan, and
//!   the trailing-trim are all at or after `t`). Without this, `_a`×n rescans
//!   the local part to the end from every boundary.
//! * `gt_dead_from` — no `>` at or after this index: one test settles both the
//!   `<` autolink scan and every inline-html terminator search (they all need
//!   a `>`), letting the whole `<` arm fall through in O(1).
//! * `html_dead` — per-terminator ("-->", "]]>", "?>") absence frontier for
//!   the `find` scans inside `inline_html_end` (its `>` search feeds
//!   `gt_dead_from` instead).

use std::collections::HashMap;

/// `table[i]` = first index ≥ i where the two-char sequence `a``b` occurs, or
/// u32::MAX. One O(n) backward pass; identical answers to a forward scan from
/// each position (which is what the unclosed `\(` arm did per `\(`).
pub(crate) fn next_pair_table(chars: &[char], a: char, b: char) -> Vec<u32> {
  let mut table = vec![u32::MAX; chars.len() + 1];
  let mut next = u32::MAX;
  for j in (0..chars.len()).rev() {
    if chars[j] == a && chars.get(j + 1) == Some(&b) {
      next = j as u32;
    }
    table[j] = next;
  }
  table
}

/// `table[i]` = first index ≥ i that is even VALID as a `$`-math closer per
/// the Pandoc context rules (prev not whitespace/backslash, next not a digit)
/// — position facts, checked exactly the way [`find_math_closer`] does. When
/// no such index exists at or after a content start, NO math scan from there
/// can ever succeed, however the code spans fall — one backward pass upgrades
/// "the suffix has no closers" to an O(1) fact.
pub(crate) fn next_math_closer_table(chars: &[char]) -> Vec<u32> {
  let mut table = vec![u32::MAX; chars.len() + 1];
  let mut next = u32::MAX;
  for j in (1..chars.len()).rev() {
    if chars[j] == '$'
      && !chars[j - 1].is_whitespace()
      && chars[j - 1] != '\\'
      && !chars.get(j + 1).is_some_and(|c| c.is_ascii_digit())
    {
      next = j as u32;
    }
    table[j] = next;
  }
  table
}

#[derive(Default)]
pub(crate) struct ScanMemo {
  /// `$`-math content starts known to fail: a failed scan proves every start
  /// in (its own start, `math_dead_through`] fails. See module docs.
  pub(crate) math_dead_through: usize,
  /// Lazily built [`next_math_closer_table`]; an empty suffix there means no
  /// scan can ever succeed, however code spans fall.
  pub(crate) math_valid_closer: Option<Vec<u32>>,
  /// Backtick run length → smallest index from which NO closing run of that
  /// length exists (recorded at `start + run` on a `code_span_close` failure).
  pub(crate) code_dead: HashMap<usize, usize>,
  /// Failed bare-email scan: (its start, the charset stop index). Starts in
  /// that open interval fail identically — again, see module docs.
  pub(crate) email_dead: Option<(usize, usize)>,
  /// No `>` at or after this index.
  pub(crate) gt_dead_from: usize,
  /// Inline-HTML terminator → smallest `from` whose search found nothing; a
  /// later search starting inside the dead zone finds nothing either.
  pub(crate) html_dead: HashMap<&'static str, usize>,
}

impl ScanMemo {
  pub(crate) fn new() -> Self {
    ScanMemo {
      gt_dead_from: usize::MAX,
      ..Default::default()
    }
  }

  /// [`crate::inline::code_span_close`], memoised on the run length: a
  /// failure at `start` means no run of that length anywhere at or after
  /// `start + n`, and a later call searches a subset of that same range.
  pub(crate) fn code_span_close(&mut self, chars: &[char], start: usize) -> Option<usize> {
    let mut n = 0;
    while start + n < chars.len() && chars[start + n] == '`' {
      n += 1;
    }
    if self
      .code_dead
      .get(&n)
      .is_some_and(|&from| from <= start + n)
    {
      return None;
    }
    let close = crate::inline::code_span_close(chars, start);
    if close.is_none() {
      let entry = self.code_dead.entry(n).or_insert(usize::MAX);
      *entry = (*entry).min(start + n);
    }
    close
  }
}
