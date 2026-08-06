//! Finding a comment's saved `quote` again, when its sticky anchor died.
//!
//! A thread anchors with yrs sticky indexes ([`crate::doc::CommentAnchor`]). Those
//! survive concurrent editing, but not the *destruction of the text object they
//! point into*: `MicaDoc::set_blocks` (every REST/MCP write and every version
//! restore) removes each block entry and writes a fresh `TextRef`, so anchors on a
//! document die even when not one character changed. Deleting the quoted sentence
//! kills them too — that one is *supposed* to orphan the thread.
//!
//! Telling those apart is exactly what the saved `quote` is for: look for that
//! text in the document as it is now, and re-anchor only when the match is
//! CONFIDENT. Everything here is therefore biased towards refusing:
//!
//! - an exact (case/whitespace-insensitive) hit wins, and among several hits only
//!   the one in the thread's original block is trusted — otherwise ambiguity
//!   means "stay an orphan";
//! - the fuzzy tier tolerates small edits (≤25% of the quote) but refuses when
//!   two separate places in the document match equally well.
//!
//! **A wrong anchor is worse than none**: an orphaned thread is visibly orphaned
//! and still readable against its quote, while a mis-anchored one silently
//! attributes a discussion to words nobody commented on. Never widen these rules
//! to "find something".
//!
//! Matching happens here, in Rust, because this is where anchors are resolved
//! (the list endpoint calls [`crate::doc::MicaDoc::resolve_range`]); the client is
//! handed finished offsets and never learns what a sticky index is.

use crate::doc::CommentRange;
use crate::Block;

/// Below this length a fuzzy hit is noise — "the API" is within one edit of far
/// too much prose. Short quotes still get the exact tier.
const MIN_FUZZY_CHARS: usize = 8;

/// The fuzzy tier is O(text × quote). Past this many DP cells (a ~30k-character
/// document against a 300-character quote) it is skipped rather than made to run
/// on every listing of a long document; the exact tier still applies.
const MAX_FUZZY_CELLS: usize = 8_000_000;

/// Marks a character that is a block boundary, not document text.
const BOUNDARY: usize = usize::MAX;

/// Where one character of the flattened document came from.
#[derive(Clone, Copy)]
struct Span {
    /// Index into `block_ids`, or [`BOUNDARY`].
    block: usize,
    /// UTF-16 offsets of this character within its block — the units the editor,
    /// the anchors and Dart strings all use.
    start: u32,
    end: u32,
}

/// The document's text flattened into one searchable sequence, with a map back to
/// (block id, UTF-16 offset).
///
/// Blocks are joined by a single boundary character, mirroring how the client
/// builds a quote from a multi-block selection (`\n` between blocks), so a quote
/// that spans blocks is matched the same way as one inside a block.
///
/// Build it ONCE per listing and reuse it: `from_blocks` is linear, but the
/// document read that feeds it is not free.
pub struct QuoteIndex {
    block_ids: Vec<String>,
    /// Normalized characters (lowercased, every whitespace kind folded to a
    /// space) so matching ignores casing and re-wrapping.
    chars: Vec<char>,
    /// Parallel to `chars`.
    spans: Vec<Span>,
}

impl QuoteIndex {
    /// Flatten blocks in document order (as returned by `MicaDoc::to_blocks`).
    pub fn from_blocks(blocks: &[Block]) -> Self {
        let mut block_ids = Vec::with_capacity(blocks.len());
        let mut chars = Vec::new();
        let mut spans = Vec::new();
        for (i, b) in blocks.iter().enumerate() {
            // A boundary per block, empty blocks included: dropping it for empty
            // blocks would shift a cross-block quote out of alignment.
            if i > 0 {
                chars.push(' ');
                spans.push(Span {
                    block: BOUNDARY,
                    start: 0,
                    end: 0,
                });
            }
            let bi = block_ids.len();
            block_ids.push(b.id.clone());
            let mut off = 0u32;
            for ch in b.text.chars() {
                let w = ch.len_utf16() as u32;
                chars.push(normalize(ch));
                spans.push(Span {
                    block: bi,
                    start: off,
                    end: off + w,
                });
                off += w;
            }
        }
        Self {
            block_ids,
            chars,
            spans,
        }
    }

    /// The range `quote` occupies in the document now, or `None` when there is no
    /// confident match.
    ///
    /// `prefer_block` is the thread's original start block: when the same text
    /// occurs several times, the one where the comment used to live is the only
    /// non-guess, and an occurrence anywhere else is only taken when it is unique.
    pub fn find(&self, quote: &str, prefer_block: Option<&str>) -> Option<CommentRange> {
        let pat = normalize_quote(quote);
        if pat.is_empty() {
            return None;
        }
        self.find_exact(&pat, prefer_block)
            .or_else(|| self.find_fuzzy(&pat))
    }

    fn find_exact(&self, pat: &[char], prefer_block: Option<&str>) -> Option<CommentRange> {
        let (n, m) = (self.chars.len(), pat.len());
        if m > n {
            return None;
        }
        let mut hits = Vec::new();
        for i in 0..=(n - m) {
            if self.chars[i..i + m] == *pat {
                hits.push(i);
                // Past a handful it is ambiguous anyway; the only thing still
                // worth finding is a hit in the preferred block.
                if hits.len() >= 64 {
                    break;
                }
            }
        }
        let preferred = prefer_block.and_then(|want| {
            hits.iter()
                .copied()
                .find(|&i| self.block_id_at(i) == Some(want))
        });
        let start = match preferred {
            Some(i) => i,
            // Several matches and none in the original block: any pick is a coin
            // toss, so keep the thread an orphan.
            None if hits.len() != 1 => return None,
            None => hits[0],
        };
        self.range(start, start + m)
    }

    /// Approximate match: the quote may have been lightly edited since.
    ///
    /// Sellers' algorithm gives the edit distance from the quote to the best
    /// substring ending at each position; the best-scoring place wins only if it
    /// is close enough AND alone (two equally good places = ambiguous = orphan).
    fn find_fuzzy(&self, pat: &[char]) -> Option<CommentRange> {
        let (n, m) = (self.chars.len(), pat.len());
        if m < MIN_FUZZY_CHARS || n == 0 || n.saturating_mul(m) > MAX_FUZZY_CELLS {
            return None;
        }
        let max_dist = m / 4;

        let ends = sellers(&self.chars, pat);
        let best = *ends.iter().min()?;
        if best > max_dist {
            return None;
        }
        // Group equally-good end positions. One alignment can only shift by as
        // many characters as it took edits, so neighbours that close together are
        // the same match ending a character earlier or later; anything further
        // apart is a SECOND place in the document that fits just as well, and
        // then there is no honest way to choose.
        let window = best + 1;
        let mut regions = 0usize;
        let mut end = 0usize;
        let mut last: Option<usize> = None;
        for (i, &d) in ends.iter().enumerate() {
            if d != best {
                continue;
            }
            if last.is_none_or(|p| i - p > window) {
                regions += 1;
                end = i;
            }
            last = Some(i);
        }
        if regions != 1 {
            return None;
        }

        // Sellers finds where a match ENDS. Re-run it backwards over the text
        // before that point to find where the same match starts.
        let rev_text: Vec<char> = self.chars[..end].iter().rev().copied().collect();
        let rev_pat: Vec<char> = pat.iter().rev().copied().collect();
        let starts = sellers(&rev_text, &rev_pat);
        let mut best_len = 0usize;
        let mut best_dist = usize::MAX;
        for (len, &d) in starts.iter().enumerate().skip(1) {
            let closer_to_pattern =
                d == best_dist && len.abs_diff(m) < best_len.abs_diff(m);
            if d < best_dist || closer_to_pattern {
                best_dist = d;
                best_len = len;
            }
        }
        if best_len == 0 || best_dist > max_dist {
            return None;
        }
        self.range(end - best_len, end)
    }

    fn block_id_at(&self, i: usize) -> Option<&str> {
        let span = self.spans.get(i)?;
        if span.block == BOUNDARY {
            return None;
        }
        Some(&self.block_ids[span.block])
    }

    /// Half-open character range → a comment range, trimming block boundaries off
    /// both ends (a highlight must start and end on real text).
    fn range(&self, mut from: usize, mut to: usize) -> Option<CommentRange> {
        while from < to && self.spans[from].block == BOUNDARY {
            from += 1;
        }
        while to > from && self.spans[to - 1].block == BOUNDARY {
            to -= 1;
        }
        if from >= to {
            return None;
        }
        let start = self.spans[from];
        let end = self.spans[to - 1];
        Some(CommentRange {
            start_block: self.block_ids[start.block].clone(),
            start_offset: start.start,
            end_block: self.block_ids[end.block].clone(),
            end_offset: end.end,
        })
    }
}

/// Fold away the differences that must not stop a match: letter case, and which
/// whitespace character it is (the client joins a multi-block quote with `\n`
/// where the document has a block boundary).
fn normalize(ch: char) -> char {
    if ch.is_whitespace() {
        ' '
    } else {
        ch.to_lowercase().next().unwrap_or(ch)
    }
}

/// Normalize a stored quote into a search pattern, dropping the `…` the client
/// appends when it truncates a long selection — with the ellipsis the exact tier
/// could never hit. The match is then the quote's 300-character prefix, i.e. a
/// re-anchored long selection can come back shorter than it was; still better
/// than a dead thread, and the comment is on the text it names.
fn normalize_quote(quote: &str) -> Vec<char> {
    quote
        .trim()
        .trim_end_matches('…')
        .trim()
        .chars()
        .map(normalize)
        .collect()
}

/// Sellers' variant of edit distance: `out[i]` is the smallest number of edits
/// turning `pat` into some substring of `text` ending at `i` (the start is free).
fn sellers(text: &[char], pat: &[char]) -> Vec<usize> {
    let m = pat.len();
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    let mut out = Vec::with_capacity(text.len() + 1);
    out.push(m);
    for &t in text {
        cur[0] = 0;
        for j in 1..=m {
            let cost = usize::from(t != pat[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        out.push(cur[m]);
        std::mem::swap(&mut prev, &mut cur);
    }
    out
}
