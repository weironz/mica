//! The text a search hit shows — ONE implementation for both worlds (the
//! cloud `/search` and the desktop's `search_local`), so a page reads the same
//! either side and a fix lands once.
//!
//! It used to exist twice, and the two copies had drifted into two different
//! bugs: the cloud one counted characters correctly but opened 40 characters
//! before the match, the local one sliced by a byte offset taken from a
//! lowercased COPY of the text — which panics (`İ`→`i̇`, `ẞ`→`ß` change byte
//! length) inside the FFI store's mutex (second-round review #3).
//!
//! What the window is shaped for: the client draws a snippet on ONE line with
//! an ellipsis. A long lead put the match past the visible edge — about 35 CJK
//! characters fit a line, and the lead alone was 40 — and the `\n` between
//! blocks ended the line before the match was reached. So the lead is short,
//! never reaches back into the previous block, and newlines are flattened.

/// Characters of context kept before the match, so it is not flush-left.
const LEAD: usize = 12;
/// Characters taken in total (from the window start).
const WINDOW: usize = 90;
/// Drawn where one block ends and the next begins inside a snippet.
const BLOCK_SEPARATOR: &str = " · ";

/// A one-line window of `text` around the first case-insensitive occurrence of
/// `needle`, or `None` when it does not occur (or either is empty).
///
/// Case folding is per character (`char::to_lowercase`), so it finds every
/// occurrence an ASCII-only fold (the cloud index, SQL `ILIKE`) does — mapping
/// characters one by one preserves containment — and indexes stay in the
/// ORIGINAL text, whatever the fold does to lengths.
pub fn snippet(text: &str, needle: &str) -> Option<String> {
  let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
  if needle.is_empty() || text.is_empty() {
    return None;
  }
  let chars: Vec<char> = text.chars().collect();
  // Folded text, and for each folded char the ORIGINAL char it came from.
  let mut folded: Vec<char> = Vec::with_capacity(chars.len());
  let mut origin: Vec<usize> = Vec::with_capacity(chars.len());
  for (i, c) in chars.iter().enumerate() {
    for l in c.to_lowercase() {
      folded.push(l);
      origin.push(i);
    }
  }
  let at = folded
    .windows(needle.len())
    .position(|w| w == needle.as_slice())?;
  let hit = origin[at];

  // Never open before the start of the hit's own block: a lead taken from the
  // previous block is text the hit has nothing to do with.
  let block_start = chars[..hit]
    .iter()
    .rposition(|&c| c == '\n')
    .map_or(0, |i| i + 1);
  let start = hit.saturating_sub(LEAD).max(block_start);
  let end = (start + WINDOW).min(chars.len());

  let mut out = String::new();
  if start > 0 {
    out.push('…');
  }
  let mut pending_space = false;
  let mut pending_break = false;
  for &c in &chars[start..end] {
    if c == '\n' {
      pending_break = true;
    } else if c.is_whitespace() {
      pending_space = true;
    } else {
      // Runs of whitespace collapse to one space, a block boundary (with
      // whatever whitespace surrounds it) to one separator; neither leads.
      if !out.is_empty() && !out.ends_with('…') {
        if pending_break {
          out.push_str(BLOCK_SEPARATOR);
        } else if pending_space {
          out.push(' ');
        }
      }
      pending_space = false;
      pending_break = false;
      out.push(c);
    }
  }
  if end < chars.len() {
    out.push('…');
  }
  Some(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  /// How much of a snippet one line of the result list shows (CJK).
  const VISIBLE: usize = 30;

  fn visible_part(s: &str) -> String {
    s.chars().take(VISIBLE).collect()
  }

  /// THE BUG: a 40-char lead pushed the match off the one visible line.
  #[test]
  fn the_match_is_on_the_visible_part_of_the_line() {
    let body = format!("{}部署流程在这里", "前文".repeat(100));
    let s = snippet(&body, "部署流程").unwrap();
    assert!(visible_part(&s).contains("部署流程"), "{s}");
  }

  /// THE BUG: `\n` between blocks ended the line before the match.
  #[test]
  fn the_lead_stays_inside_the_hit_block_and_newlines_flatten() {
    let body = "第一段很长的内容\n第二段\n  部署流程开始\n第四段";
    let s = snippet(body, "部署").unwrap();
    assert!(s.starts_with("…部署流程开始"), "{s}");
    assert!(!s.contains('\n'), "{s}");
    assert!(s.contains(" · 第四段"), "{s}");
  }

  /// Second-round review #3: folding that changes length (`İ`→`i̇`, `ẞ`→`ß`)
  /// before the hit used to slice the original at the folded offset — a panic.
  #[test]
  fn length_changing_case_folds_neither_panic_nor_shift_the_window() {
    let body = format!("{}needle tail", "İẞ".repeat(200));
    let s = snippet(&body, "NEEDLE").unwrap();
    assert!(s.contains("needle tail"), "{s}");
    assert!(visible_part(&s).contains("needle"), "{s}");
    assert_eq!(snippet("ab", "İ"), None);
  }

  #[test]
  fn case_insensitive_and_absent_and_empty() {
    assert_eq!(
      snippet("Hello World", "world").as_deref(),
      Some("Hello World")
    );
    assert_eq!(snippet("Hello", "xyz"), None);
    assert_eq!(snippet("", "a"), None);
    assert_eq!(snippet("abc", ""), None);
  }

  #[test]
  fn clipped_edges_get_an_ellipsis_and_whitespace_runs_collapse() {
    let body = format!("{}  x\t\t y{}", "a".repeat(50), "b".repeat(300));
    let s = snippet(&body, "x").unwrap();
    assert!(s.starts_with('…') && s.ends_with('…'), "{s}");
    assert!(s.contains("x y"), "{s}");
  }
}
