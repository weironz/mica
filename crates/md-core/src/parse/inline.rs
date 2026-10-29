use serde_json::{Value, json};

use crate::Block;
use crate::block::*;
use crate::entities::*;
use crate::scan_memo::{ScanMemo, next_math_closer_table, next_pair_table};
use crate::span_walk::{SpanTies, SpanWalker};

/// One past the closing backtick run of the code span opening at `start` (the
/// index of its first backtick), or None when the run never closes — CommonMark
/// leaves an unclosed run as literal text.
///
/// Deliberately a second copy of the run-matching in the inline-code branch
/// rather than a shared helper: that branch also strips padding and folds
/// newlines, and it carries the conformance score. This only needs the extent.
/// Keep the two in step.
pub(crate) fn code_span_close(chars: &[char], start: usize) -> Option<usize> {
  let mut n = 0;
  while start + n < chars.len() && chars[start + n] == '`' {
    n += 1;
  }
  let mut j = start + n;
  while j < chars.len() {
    if chars[j] == '`' {
      let mut m = 0;
      while j + m < chars.len() && chars[j + m] == '`' {
        m += 1;
      }
      if m == n {
        return Some(j + m);
      }
      j += m;
    } else {
      j += 1;
    }
  }
  None
}

/// Delimiter-inclusive char spans of every inline math run in `src` — both the
/// `$ … $` and `\( … \)` forms — judged by exactly the rules the parser uses.
///
/// Public so callers outside the parser can ask "is this offset inside a
/// formula?" without restating the Pandoc rules and drifting from them (the MCP
/// write guard needs it to tell a corrupted `\times` from an ordinary tab).
/// Mirrors Dart `mathRunSpans` in `lib/editor/marks.dart` — CLAUDE.md #2 keeps
/// the two sides in step, with this one authoritative.
pub fn math_run_spans(src: &str) -> Vec<(usize, usize)> {
  let chars: Vec<char> = src.chars().collect();
  let mut out = Vec::new();
  // One \(\)→`\)` next-table (built lazily) and the shared failure memos:
  // repeated unclosed `\(`/`$`/backticks rescanned the whole suffix before
  // (quadratic; issue #61 — the MCP write guard calls this per write).
  let mut scans = ScanMemo::new();
  let mut close_paren: Option<Vec<u32>> = None;
  let mut i = 0;
  while i < chars.len() {
    // `\( … \)` first — the escape arm below would otherwise eat the `\(`.
    if chars[i] == '\\'
      && chars.get(i + 1) == Some(&'(')
      && let Some(close) = {
        let table = close_paren.get_or_insert_with(|| next_pair_table(&chars, '\\', ')'));
        (table[i + 2] != u32::MAX).then_some(table[i + 2] as usize)
      }
    {
      out.push((i, close + 2));
      i = close + 2;
      continue;
    }
    // A code span binds tighter than math (§6.1) — step over it whole so a `$`
    // inside cannot open a run.
    if chars[i] == '`'
      && let Some(after) = scans.code_span_close(&chars, i)
    {
      i = after;
      continue;
    }
    if chars[i] == '$'
      && chars.get(i + 1) != Some(&'$')
      && let Some(close) = find_math_closer(&chars, i + 1, &mut scans)
    {
      out.push((i, close + 1));
      i = close + 1;
      continue;
    }
    i += 1;
  }
  out
}

/// A valid `$` math closer per the Pandoc rules: content non-empty with
/// non-space edges, closer not followed by a digit, no newline inside, and no
/// crossing into a code span. `memo` carries the failure knowledge forward:
/// every `$` opener used to rescan the whole suffix when no closer exists
/// (issue #61); see [`ScanMemo`] for why the recorded frontier is exact.
pub(crate) fn find_math_closer(
  chars: &[char],
  content_start: usize,
  memo: &mut ScanMemo,
) -> Option<usize> {
  if memo.math_dead_through >= content_start {
    return None; // a previous failure already settled this whole stretch
  }
  if let Some(table) = &memo.math_valid_closer
    && table[content_start] == u32::MAX
  {
    // No position ahead is even VALID as a closer — and the closer checks are
    // all position facts — so no scan from here can succeed, however the code
    // spans fall. This is what keeps `$a``a`b `×n (every closer-candidate
    // invalid, a code span between every opener) linear.
    return None;
  }
  if chars.get(content_start).is_none_or(|c| c.is_whitespace()) {
    return None;
  }
  // Record what a failure at/after `horizon` proves for later starts.
  fn record(chars: &[char], content_start: usize, horizon: usize, memo: &mut ScanMemo) {
    let table = memo
      .math_valid_closer
      .get_or_insert_with(|| next_math_closer_table(chars));
    if table[content_start] == u32::MAX {
      memo.math_dead_through = usize::MAX; // no valid closer anywhere ahead
    } else {
      memo.math_dead_through = memo.math_dead_through.max(horizon);
    }
  }
  // First backtick whose run CLOSES ahead: up to it the walk is char-for-char,
  // so a failure settles later starts only that far (see [`ScanMemo`]).
  let mut skip_open: Option<usize> = None;
  let mut j = content_start;
  while j < chars.len() {
    let c = chars[j];
    if c == '\n' {
      record(chars, content_start, skip_open.unwrap_or(j), memo);
      return None; // inline math stays on one line
    }
    if c == '`' {
      // Code spans bind tighter than math — CommonMark 0.31.2 §6.1 gives them
      // higher precedence than every inline construct but HTML tags and
      // autolinks — so a `$` inside one is literal and cannot close us. Step
      // over the span whole; scanning through it is what let `` `$HOME` `` be
      // eaten by an earlier `$`. An unclosed run is literal text: fall through
      // and keep scanning, exactly as CommonMark reads it.
      if let Some(after) = memo.code_span_close(chars, j) {
        if chars[j..after].contains(&'\n') {
          record(chars, content_start, skip_open.unwrap_or(j), memo);
          return None; // the span folds a line break; inline math may not
        }
        if skip_open.is_none() {
          skip_open = Some(j);
        }
        j = after;
        continue;
      }
    }
    if c == '$' && j > content_start {
      if chars[j - 1].is_whitespace() || chars[j - 1] == '\\' {
        j += 1;
        continue;
      }
      if chars.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
        j += 1;
        continue;
      }
      return Some(j);
    }
    j += 1;
  }
  record(chars, content_start, skip_open.unwrap_or(usize::MAX), memo);
  None
}

/// GFM extended autolink starting at chars[i] (the caller checks the word
/// boundary): bare `http(s)://…`, `www.…` (href gains `http://`) or a bare
/// email. Returns (consumed chars, href). `email_dead` memoises the bare-email
/// charset scan: without it `_a`×n rescans the local part to the end from
/// every boundary (issue #61); [`ScanMemo::email_dead`] states why it is exact.
pub(crate) fn extended_autolink(
  chars: &[char],
  i: usize,
  email_dead: &mut Option<(usize, usize)>,
) -> Option<(usize, String)> {
  // Only the first 8 chars decide the prefix. This used to collect and
  // lowercase the WHOLE remaining text first — at every word boundary — which
  // made one inline parse quadratic, and the paragraph-continuation re-parse
  // on every line made a plain multi-line paragraph cubic (2000 short lines:
  // seconds).
  let head: String = chars[i..chars.len().min(i + 8)]
    .iter()
    .collect::<String>()
    .to_ascii_lowercase();
  for (prefix, implied) in [
    ("https://", ""),
    ("http://", ""),
    ("ftp://", ""),
    ("www.", "http://"),
  ] {
    if head.starts_with(prefix) {
      // Take everything up to whitespace or '<', then trim trailing
      // punctuation per the GFM rules.
      let stop = chars[i..]
        .iter()
        .position(|&c| c.is_whitespace() || c == '<')
        .map_or(chars.len(), |k| i + k);
      let rest: String = chars[i..stop].iter().collect();
      let end = trim_autolink_end(&rest);
      let candidate = &rest[..end];
      let domain = &candidate[if implied.is_empty() { prefix.len() } else { 0 }..];
      let domain = domain.split(['/', '?', '#']).next().unwrap_or_default();
      if !valid_autolink_domain(domain) {
        return None;
      }
      let count = candidate.chars().count();
      if count == 0 {
        return None;
      }
      return Some((count, format!("{implied}{candidate}")));
    }
  }
  // Bare email: local@domain.tld — `+` allowed before the @ only; the last
  // character must be alphanumeric.
  if chars[i].is_ascii_alphanumeric() {
    if let Some((from, through)) = *email_dead
      && from < i
      && i <= through
    {
      return None; // same stop, same `@`, same domain verdict — proven equal
    }
    let mut j = i;
    while chars
      .get(j)
      .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
    {
      j += 1;
    }
    let mut found = None;
    if chars.get(j) == Some(&'@') {
      let local_len = j - i;
      let mut k = j + 1;
      while chars
        .get(k)
        .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
      {
        k += 1;
      }
      // Trailing `.` stays outside the link; trailing `-`/`_` invalidate.
      while k > j + 1 && chars[k - 1] == '.' {
        k -= 1;
      }
      if local_len > 0
        && k > j + 1
        && chars[k - 1].is_ascii_alphanumeric()
        && chars[j + 1..k].contains(&'.')
      {
        let text: String = chars[i..k].iter().collect();
        found = Some((k - i, format!("mailto:{text}")));
      }
    }
    if found.is_none() {
      *email_dead = Some((i, j));
    }
    return found;
  }
  None
}

/// GFM trailing-punctuation trimming for extended autolinks.
pub(crate) fn trim_autolink_end(s: &str) -> usize {
  let mut end = s.len();
  loop {
    let kept = &s[..end];
    let Some(last) = kept.chars().last() else {
      return 0;
    };
    if matches!(
      last,
      '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~' | '\'' | '"'
    ) {
      end -= last.len_utf8();
      continue;
    }
    if last == ')' {
      let opens = kept.matches('(').count();
      let closes = kept.matches(')').count();
      if closes > opens {
        end -= 1;
        continue;
      }
      return end;
    }
    if last == ';' {
      // A trailing entity reference (`&amp;`) drops off entirely.
      if let Some(amp) = kept.rfind('&') {
        let body = &kept[amp + 1..end - 1];
        if !body.is_empty() && body.chars().all(|c| c.is_ascii_alphanumeric()) {
          end = amp;
          continue;
        }
      }
      return end;
    }
    return end;
  }
}

/// GFM autolink domain: alphanumeric/`-`/`_` segments joined by `.`, at
/// least one dot, and no underscore in the last two segments.
pub(crate) fn valid_autolink_domain(domain: &str) -> bool {
  if domain.is_empty()
    || !domain
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
  {
    return false;
  }
  let segments: Vec<&str> = domain.split('.').collect();
  if segments.len() < 2 || segments.iter().any(|s| s.is_empty()) {
    return false;
  }
  !segments[segments.len().saturating_sub(2)..]
    .iter()
    .any(|s| s.contains('_'))
}

/// Parse inline Markdown (`**b**`, `*i*`/`_i_`, `` `c` ``, `~~s~~`, `[t](url)`)
/// in [text] into clean text plus marks merged into [data] under `"marks"`.
/// Offsets are UTF-16 code-unit indices to match the Flutter client.
pub(crate) fn apply_inline_marks(text: String, data: Value, defs: &RefDefs) -> (String, Value) {
  let parsed = parse_inline_with(&text, defs);
  if parsed.marks.is_empty() {
    return (parsed.text, data);
  }
  let marks: Vec<Value> = parsed
    .marks
    .iter()
    .map(|m| {
      let mut obj = serde_json::Map::new();
      obj.insert("start".into(), json!(m.start));
      obj.insert("end".into(), json!(m.end));
      obj.insert("type".into(), json!(m.kind));
      if let Some(href) = &m.href {
        obj.insert("href".into(), json!(href));
      }
      if let Some(title) = &m.title {
        obj.insert("title".into(), json!(title));
      }
      Value::Object(obj)
    })
    .collect();
  let mut obj = match data {
    Value::Object(map) => map,
    _ => serde_json::Map::new(),
  };
  obj.insert("marks".into(), Value::Array(marks));
  (parsed.text, Value::Object(obj))
}

pub(crate) struct ParsedInline {
  pub(crate) text: String,
  pub(crate) marks: Vec<InlineMark>,
}

/// Link reference definitions: normalized label → (destination, title).
pub(crate) type RefDefs = std::collections::HashMap<String, (String, Option<String>)>;

/// Scan all link reference definitions, fence-aware and multi-line: the
/// destination may sit on the line after `[label]:`, and a (quoted) title
/// may follow on its own line(s). Returns the defs and the set of consumed
/// line indices.
pub(crate) fn collect_ref_definitions(
  raw_lines: &[&str],
) -> (
  RefDefs,
  std::collections::HashSet<usize>,
  std::collections::HashMap<usize, usize>,
) {
  let mut defs = RefDefs::new();
  let mut def_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();
  // Definitions inside blockquotes (`> [foo]: /url`) — line → quote depth.
  // They define globally; the line itself renders as a contentless `>`
  // (spec ex. 218: the quote ends up empty, the def still resolves).
  let mut quote_def_lines: std::collections::HashMap<usize, usize> =
    std::collections::HashMap::new();
  let mut in_fence: Option<(u8, usize)> = None;
  let mut prev_para = false; // an open paragraph means a def can't start
  let mut i = 0;
  while i < raw_lines.len() {
    let line = raw_lines[i].trim_end();
    let content = line.trim_start();
    let col = line.len() - content.len();
    if let Some((c, n)) = in_fence {
      if fence_close(content, c, n) {
        in_fence = None;
      }
      prev_para = false;
      i += 1;
      continue;
    }
    if col < 4
      && let Some((c, n, _)) = fence_open(content)
    {
      in_fence = Some((c, n));
      prev_para = false;
      i += 1;
      continue;
    }
    if content.is_empty() {
      prev_para = false;
      i += 1;
      continue;
    }
    if !prev_para
      && col < 4
      && content.starts_with('[')
      && let Some((label, dest, title, used)) = parse_ref_definition_multi(raw_lines, i)
    {
      defs.entry(normalize_label(&label)).or_insert((dest, title));
      for k in i..i + used {
        def_lines.insert(k);
      }
      i += used;
      continue; // a definition doesn't open a paragraph
    }
    // A single-line definition inside a blockquote (multi-line defs would
    // need marker-stripped continuation — not a real-world shape).
    if !prev_para && col < 4 && content.starts_with('>') {
      let (qd, qrest) = strip_quote_markers(content);
      let qrest = qrest.trim_start();
      if qd > 0
        && qrest.starts_with('[')
        && let Some((label, dest, title, used)) = parse_ref_definition_multi(&[qrest], 0)
        && used == 1
      {
        defs.entry(normalize_label(&label)).or_insert((dest, title));
        quote_def_lines.insert(i, qd);
        i += 1;
        continue;
      }
    }
    // Only true paragraph text can lazily absorb a following def line. Single
    // line blocks (ATX headings, thematic breaks) don't, so a def on the next
    // line still counts — `# [Foo]\n[foo]: /url` defines `foo`.
    prev_para = heading_prefix_level(content).is_none() && !is_divider(content);
    i += 1;
  }
  (defs, def_lines, quote_def_lines)
}

/// `[label]:` at lines[i]; destination on the same or next line; optional
/// quoted title after the destination (same line needs whitespace between)
/// or on following line(s) — titles may span lines. Returns the parsed def
/// and how many lines it consumed.
pub(crate) fn parse_ref_definition_multi(
  raw_lines: &[&str],
  i: usize,
) -> Option<(String, String, Option<String>, usize)> {
  // The label may wrap across lines (`[Foo\n  bar]: /url`): join consecutive
  // lines until the `]:` closer appears, counting how many we spanned.
  let first = raw_lines[i].trim_start();
  let mut joined = first.to_string();
  let mut label_lines = 1;
  let (close, chars) = loop {
    let chars: Vec<char> = joined.chars().collect();
    if let Some(close) = matching_bracket(&chars, 0)
      && chars.get(close + 1) == Some(&':')
    {
      break (close, chars);
    }
    // No closer yet — pull in the next line (label-only blank lines are not
    // allowed inside a definition label).
    let next = raw_lines.get(i + label_lines)?;
    if next.trim().is_empty() {
      return None;
    }
    joined.push('\n');
    joined.push_str(next.trim());
    label_lines += 1;
    if label_lines > 8 {
      return None; // a runaway label is not a definition
    }
  };
  let label: String = chars[1..close].iter().collect();
  if label.trim().is_empty() {
    return None;
  }
  // `[^label]:` is a GFM footnote definition, a block of its own — never a
  // link reference definition. Leave it for the main block scanner.
  if label.starts_with('^') {
    return None;
  }
  for (k, &c) in chars[1..close].iter().enumerate() {
    if (c == '[' || c == ']') && (k == 0 || chars[k] != '\\') {
      return None;
    }
  }
  let after_colon: String = chars[close + 2..].iter().collect();
  let after_colon = after_colon.trim();
  let mut used = label_lines;
  let dest_line = if after_colon.is_empty() {
    // destination on the line after the (possibly multi-line) label
    let l2 = raw_lines.get(i + label_lines)?.trim();
    if l2.is_empty() {
      return None;
    }
    used = label_lines + 1;
    l2.to_string()
  } else {
    after_colon.to_string()
  };
  let (dest, rest, had_ws) = parse_def_dest(&dest_line)?;
  if !rest.is_empty() {
    // a same-line title needs whitespace after the destination
    if !had_ws {
      return None;
    }
    let title = parse_def_title(raw_lines, i + used - 1, &rest)?;
    return Some((label, dest, Some(title.0), used + title.1));
  }
  // maybe a title on the following line(s)
  if let Some(next) = raw_lines.get(i + used) {
    let nt = next.trim();
    if !nt.is_empty()
      && matches!(nt.as_bytes()[0], b'"' | b'\'' | b'(')
      && let Some(title) = parse_def_title(raw_lines, i + used, nt)
    {
      return Some((label, dest, Some(title.0), used + 1 + title.1));
    }
  }
  Some((label, dest, None, used))
}

/// Destination: `<angle form>` or a run of non-whitespace. Returns the
/// unescaped dest, the rest of the line (trimmed), and whether whitespace
/// separated them.
pub(crate) fn parse_def_dest(s: &str) -> Option<(String, String, bool)> {
  if let Some(inner) = s.strip_prefix('<') {
    let gt = inner.find('>')?;
    let rest = &inner[gt + 1..];
    let had_ws = rest.is_empty() || rest.starts_with([' ', '\t']);
    return Some((unescape_md(&inner[..gt]), rest.trim().to_string(), had_ws));
  }
  let end = s.find([' ', '\t']).unwrap_or(s.len());
  if end == 0 {
    return None;
  }
  Some((
    unescape_md(&s[..end]),
    s[end..].trim().to_string(),
    end < s.len(),
  ))
}

/// A quoted title starting at `start` (the trimmed tail of line `line_idx`),
/// possibly spanning lines. Returns (title, extra lines consumed); fails if
/// non-whitespace follows the closing delimiter.
pub(crate) fn parse_def_title(
  raw_lines: &[&str],
  line_idx: usize,
  start: &str,
) -> Option<(String, usize)> {
  let open = start.chars().next()?;
  let close = match open {
    '"' => '"',
    '\'' => '\'',
    '(' => ')',
    _ => return None,
  };
  // same-line close?
  let body = &start[1..];
  if let Some(pos) = find_unescaped_char(body, close) {
    if !body[pos + close.len_utf8()..].trim().is_empty() {
      return None;
    }
    return Some((unescape_md(&body[..pos]), 0));
  }
  // spans lines: accumulate until a line containing the unescaped closer
  let mut acc = body.to_string();
  let mut extra = 0usize;
  loop {
    extra += 1;
    let l = raw_lines.get(line_idx + extra)?;
    let lt = l.trim_end();
    // A blank line may not interrupt a title — it voids the definition
    // (spec ex. 197: everything renders back as paragraphs).
    if lt.trim().is_empty() {
      return None;
    }
    if let Some(pos) = find_unescaped_char(lt, close) {
      if !lt[pos + close.len_utf8()..].trim().is_empty() {
        return None;
      }
      acc.push('\n');
      acc.push_str(&lt[..pos]);
      return Some((unescape_md(&acc), extra));
    }
    acc.push('\n');
    acc.push_str(lt);
  }
}

pub(crate) fn find_unescaped_char(s: &str, target: char) -> Option<usize> {
  let mut prev_backslash = false;
  for (idx, c) in s.char_indices() {
    if c == target && !prev_backslash {
      return Some(idx);
    }
    prev_backslash = c == '\\' && !prev_backslash;
  }
  None
}

pub(crate) fn normalize_label(label: &str) -> String {
  // Reference labels match under Unicode CASE FOLDING, not mere lowercase:
  // ẞ/ß and SS all collapse to "ss" (the only 1:N fold real labels hit).
  label
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase()
    .replace('ß', "ss")
}

/// Unescape backslash-escaped ASCII punctuation (destinations/titles).
pub(crate) fn unescape_md(s: &str) -> String {
  let chars: Vec<char> = s.chars().collect();
  let mut out = String::with_capacity(s.len());
  let mut i = 0;
  while i < chars.len() {
    if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
      out.push(chars[i + 1]);
      i += 2;
    } else if let Some((decoded, used)) = parse_entity(&chars, i) {
      // Entity references decode inside destinations/titles/info strings.
      out.push_str(&decoded);
      i += used;
    } else {
      out.push(chars[i]);
      i += 1;
    }
  }
  out
}

/// Parse an inline-link suffix `(dest "title")` starting right AFTER the `(`.
/// Returns (href, title, index just past the closing `)`).
pub(crate) fn parse_link_suffix(
  chars: &[char],
  mut i: usize,
) -> Option<(String, Option<String>, usize)> {
  // Only ASCII whitespace separates destination and title — U+00A0 and
  // friends are CONTENT (spec ex. 507: the whole thing becomes the href).
  fn md_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000B}' | '\u{000C}')
  }
  let n = chars.len();
  while i < n && md_ws(chars[i]) {
    i += 1;
  }
  // Destination: <may contain spaces> or bare with balanced parens.
  let dest: String;
  if i < n && chars[i] == '<' {
    let mut j = i + 1;
    // A backslash escapes the next char, so `<foo\>` is unterminated (the
    // `\>` is a literal `>`, not the closer) — keep scanning past it.
    while j < n && chars[j] != '>' && chars[j] != '\n' && chars[j] != '<' {
      j += if chars[j] == '\\' && j + 1 < n { 2 } else { 1 };
    }
    if j >= n || chars[j] != '>' {
      return None;
    }
    dest = unescape_md(&chars[i + 1..j].iter().collect::<String>());
    i = j + 1;
  } else {
    let mut depth = 0i32;
    let start = i;
    while i < n {
      let c = chars[i];
      if md_ws(c) {
        break;
      }
      if c == '\\' && i + 1 < n {
        i += 2;
        continue;
      }
      match c {
        '(' => depth += 1,
        ')' => {
          if depth == 0 {
            break;
          }
          depth -= 1;
        }
        _ => {}
      }
      i += 1;
    }
    if depth != 0 {
      return None;
    }
    dest = unescape_md(&chars[start..i].iter().collect::<String>());
  }
  while i < n && md_ws(chars[i]) {
    i += 1;
  }
  // Optional title: "..." / '...' / (...)
  let mut title = None;
  if i < n && matches!(chars[i], '"' | '\'' | '(') {
    let close = if chars[i] == '(' { ')' } else { chars[i] };
    let mut j = i + 1;
    let start = j;
    while j < n && chars[j] != close {
      // A backslash escapes the next char (so an escaped close delimiter does
      // not end the title); keep the raw span and decode it once at the end.
      if chars[j] == '\\' && j + 1 < n {
        j += 2;
        continue;
      }
      j += 1;
    }
    if j >= n {
      return None;
    }
    // Titles decode backslash escapes AND entity references (`&quot;` → `"`).
    title = Some(unescape_md(&chars[start..j].iter().collect::<String>()));
    i = j + 1;
    while i < n && md_ws(chars[i]) {
      i += 1;
    }
  }
  if i < n && chars[i] == ')' {
    Some((dest, title, i + 1))
  } else {
    None
  }
}

// NOTE: the uncached `label_contains_link` was removed — every caller now goes
// through `label_has_link_cached`, which shares one memo across the recursive
// parses (see LinkCache). Reintroducing an uncached path would silently restore
// the exponential blow-up on nested links.

/// If a code span, autolink, or raw inline HTML span starts at `chars[i]`,
/// return its exclusive end index. These spans bind tighter than link
/// brackets, so a bracket-matcher must skip over them as a unit.
pub(crate) fn inline_span_end(chars: &[char], i: usize) -> Option<usize> {
  inline_span_end_memo(chars, i, None)
}

fn inline_span_end_memo(chars: &[char], i: usize, memo: Option<&mut ScanMemo>) -> Option<usize> {
  let n = chars.len();
  match chars[i] {
    '`' => {
      // N-backtick run closes only on a run of exactly N.
      let mut run = 0;
      while i + run < n && chars[i + run] == '`' {
        run += 1;
      }
      let mut j = i + run;
      while j < n {
        if chars[j] == '`' {
          let mut m = 0;
          while j + m < n && chars[j + m] == '`' {
            m += 1;
          }
          if m == run {
            return Some(j + m);
          }
          j += m;
        } else {
          j += 1;
        }
      }
      None
    }
    '<' => {
      // Autolink first (it constrains the closing `>`), then raw HTML. An
      // autolink holds no whitespace and no `<`, so the scan stops there: it
      // used to run to the next `>` anywhere, i.e. to the END on text with no
      // `>`, from every `<` — and `matching_bracket` calls this at every
      // position, from every `[`, which made `[<[<[<…` cubic.
      let mut j = i + 1;
      while j < n && chars[j] != '>' && chars[j] != '<' && !chars[j].is_whitespace() {
        j += 1;
      }
      if j < n {
        let inner: String = chars[i + 1..j].iter().collect();
        if autolink_target(&inner).is_some() {
          return Some(j + 1);
        }
      }
      inline_html_end(chars, i, memo)
    }
    _ => None,
  }
}

/// [matching_bracket] answers memoised for one `chars`.
///
/// Every `[` the parser tries asks for its match, and an unmatched one scans to
/// the end of the text — so `[<[<[<…` (and plain `[[[[…`) cost a full scan per
/// bracket: quadratic. One scan from `open` visits every later `[` it passes on
/// a path that is the SAME path a scan from that `[` would take (the walk from
/// any position is deterministic), so a stack records their answers too —
/// exactly, matched or not. Each position is then scanned about once.
///
/// The same scan also records each matched `[`'s NESTING — how many bracket
/// levels open inside it — which is what `MAX_INLINE_NESTING` is checked
/// against. It is a property of the label text alone, so it cannot make a
/// label parse differently depending on where it was reached from (which is
/// what keeps the label cache, `LinkCache`, a pure memo).
#[derive(Default)]
pub(crate) struct BracketMemo {
  known: std::collections::HashMap<usize, Option<usize>>,
  nesting: std::collections::HashMap<usize, usize>,
}

impl BracketMemo {
  pub(crate) fn matching(&mut self, chars: &[char], open: usize) -> Option<usize> {
    if let Some(answer) = self.known.get(&open) {
      return *answer;
    }
    let n = chars.len();
    // (position of `[`, deepest nesting seen inside it so far)
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut j = open;
    let mut result = None;
    while j < n {
      let c = chars[j];
      if c == '\\' && j + 1 < n {
        j += 2;
        continue;
      }
      if let Some(end) = inline_span_end(chars, j) {
        j = end;
        continue;
      }
      if c == '[' {
        stack.push((j, 0));
      } else if c == ']'
        && let Some((o, inner)) = stack.pop()
      {
        self.known.insert(o, Some(j));
        self.nesting.insert(o, inner);
        if let Some(parent) = stack.last_mut() {
          parent.1 = parent.1.max(inner + 1);
        }
        if o == open {
          result = Some(j);
          break;
        }
      }
      j += 1;
    }
    if result.is_none() {
      for (o, _) in stack {
        self.known.insert(o, None);
      }
      self.known.insert(open, None);
    }
    result
  }

  /// Bracket levels nested inside the `[` at `open` (0 for `[a]`, 1 for
  /// `[[a]]`). Meaningful once `matching(open)` returned `Some`.
  pub(crate) fn nesting(&self, open: usize) -> usize {
    self.nesting.get(&open).copied().unwrap_or(0)
  }
}

/// Link / image labels nested deeper than this are not parsed as links or
/// images: their brackets stay literal text (the content inside still parses).
/// Mirrors Dart `kMaxInlineNesting` (`clients/mica_flutter/lib/editor/
/// marks.dart`) — change both together.
///
/// Each recognized link/image label is parsed by a recursive call, so nesting
/// was recursion depth: `![![![…a](u)](u)](u)` 2000 deep (~10 KB) overflowed a
/// 2 MiB stack in release and aborted the process (10-19 #6b). The bound is
/// structural — a label is allowed only while fewer than this many bracket
/// levels open inside it (`BracketMemo::nesting`) — and every recursion goes
/// strictly deeper into the brackets, so recursion stays under it too.
/// CommonMark's suite nests a handful of levels at most.
pub(crate) const MAX_INLINE_NESTING: usize = 32;

/// Find the `]` matching the `[` at [open], honoring nesting and escapes.
///
/// Code spans, autolinks and raw inline HTML bind tighter than link brackets
/// (CommonMark §6.5), so a `]` inside one of those does not close the link:
/// we skip over such spans while scanning. `[foo`](/uri)`` therefore keeps
/// its backtick span intact instead of forming a bogus link.
pub(crate) fn matching_bracket(chars: &[char], open: usize) -> Option<usize> {
  let n = chars.len();
  let mut depth = 0i32;
  let mut j = open;
  while j < n {
    let c = chars[j];
    if c == '\\' && j + 1 < n {
      j += 2;
      continue;
    }
    // A code span / autolink / raw-HTML region swallows any brackets inside.
    if let Some(end) = inline_span_end(chars, j) {
      j = end;
      continue;
    }
    if c == '[' {
      depth += 1;
    } else if c == ']' {
      depth -= 1;
      if depth == 0 {
        return Some(j);
      }
    }
    j += 1;
  }
  None
}

/// Memo of parsed link/image LABELS, keyed by label text and SHARED across the
/// recursive parses of one top-level parse — by the nested-link check
/// (`label_has_link_cached`) AND by `push_link`, which turns the label into the
/// link's text. Both ask the same pure question: `parse_inline_memo(label,
/// autolinks off)`.
///
/// Without sharing, a label that DOES contain a link is parsed twice per nesting
/// level — once to answer the check, then again by the scan that falls through
/// the rejected outer brackets — giving T(n) = 2·T(n-1), i.e. exponential on
/// `[[[a](/u)](/u)](/u)`. That was fixed for the check alone; `push_link` still
/// parsed through a FRESH cache, so with links and images ALTERNATING
/// (`[![[![…a](u)](u)](u)](u)`) every image label re-ran the link checks below
/// it from scratch: P(k) = 2·P(k-2), 221 bytes = 1.3 s in release (P1-7).
pub(crate) type LinkCache = std::collections::HashMap<String, ParsedInline>;

/// Parse a link's (or image's) LABEL, through the shared [LinkCache].
///
/// Autolinks off: the label is about to become the link's TEXT, and text inside
/// a link never autolinks — that is the same rule `label_has_link_cached`
/// enforces, applied to the other half of the job. With it on, a URL-shaped
/// label got a second link mark of its own: `[https://a](https://b)` produced
/// TWO overlapping marks over the same range, one pointing at `a` (the
/// autolink) and one at `b` (the real destination). Which one wins is then a
/// question about mark order, and there is no answer to it that is right.
///
/// Pure function + pure cache: this cannot change any output, only how often a
/// label is re-parsed.
pub(crate) fn parse_label_cached<'c>(
  label: &str,
  defs: &RefDefs,
  cache: &'c mut LinkCache,
) -> &'c ParsedInline {
  if !cache.contains_key(label) {
    let parsed = parse_inline_memo(label, defs, cache, false);
    cache.insert(label.to_string(), parsed);
  }
  &cache[label]
}

/// Does this link-label content already contain a link (inline, reference, or
/// autolink)? CommonMark §6.3 forbids links nested inside link text, so when
/// this is true the surrounding brackets must stay literal. Images are fine
/// (`[![alt](img)](url)` is a valid linked image), so only `link` marks count.
///
/// Autolinks off (see `parse_label_cached`) is the whole point of this call.
/// CommonMark forbids a link inside a link, and this guard enforces it — but
/// it used to ask "does the label parse to anything with a link mark", and a
/// GFM *extended autolink* produces exactly that mark. So a label that was
/// itself a bare URL counted as a nested link, and `[https://x](https://x)`
/// — precisely what you get when you copy a link out of Mica and paste it
/// back — was refused and fell through to literal text. GitHub renders that
/// as an ordinary link; the nesting rule is about explicit link CONSTRUCTS,
/// not about text that would have autolinked had it been left alone. It never
/// is: once the label becomes a link's text, nothing autolinks inside it.
pub(crate) fn label_has_link_cached(label: &str, defs: &RefDefs, cache: &mut LinkCache) -> bool {
  parse_label_cached(label, defs, cache)
    .marks
    .iter()
    .any(|m| m.kind == "link")
}

/// Mirror of the Flutter `parseInline`: returns clean text (no markers) and the
/// marks over it, with offsets in UTF-16 code units.
pub(crate) fn parse_inline_with(src: &str, defs: &RefDefs) -> ParsedInline {
  let mut cache = LinkCache::new();
  parse_inline_memo(src, defs, &mut cache, true)
}

pub(crate) fn parse_inline_memo(
  src: &str,
  defs: &RefDefs,
  cache: &mut LinkCache,
  // Whether GFM extended autolinks (bare `www.`/`http(s)://`/email) are
  // recognized. Off ONLY while testing a link label for nested links — see
  // `label_has_link_cached`.
  allow_autolink: bool,
) -> ParsedInline {
  // Line-ending canonicalization (§6.7/6.9/6.12) with span awareness: a run
  // of 2+ trailing spaces before '\n' OUTSIDE code spans / autolinks / raw
  // inline HTML is a hard break — canonicalized to the "\\\n" stored-text
  // convention (html_text renders it as <br/>); soft breaks just drop the
  // trailing spaces. INSIDE those spans every byte is content: `code  ⏎span`
  // keeps its spaces (the code-span handler maps '\n' to ' ') and raw HTML
  // keeps its literal line break. The block layer no longer pre-judges hard
  // breaks — it can't see code spans.
  let src_chars: Vec<char> = src.chars().collect();
  let mut chars: Vec<char> = Vec::with_capacity(src_chars.len());
  {
    // Normalization scans the original buffer before the main parser builds
    // its own memo. Reuse failed HTML terminator scans here too; otherwise
    // repeated unclosed comments still rescan the whole suffix in this pass.
    let mut scans = ScanMemo::new();
    let n = src_chars.len();
    let mut i = 0;
    while i < n {
      let c = src_chars[i];
      // Escaped char (incl. "\\`"): never a span opener; copy as-is. A
      // backslash directly before '\n' falls through — it IS a hard break.
      if c == '\\' && i + 1 < n && src_chars[i + 1] != '\n' {
        chars.push(c);
        chars.push(src_chars[i + 1]);
        i += 2;
        continue;
      }
      if c == '`' || c == '<' {
        if let Some(end) = inline_span_end_memo(&src_chars, i, Some(&mut scans)) {
          chars.extend_from_slice(&src_chars[i..end]);
          i = end;
          continue;
        }
      }
      if c == '\n' {
        // Hard break = 2+ SPACES immediately before the newline; any other
        // trailing whitespace (tabs included) just drops with a soft break.
        let mut spaces = 0;
        while spaces < chars.len() && chars[chars.len() - 1 - spaces] == ' ' {
          spaces += 1;
        }
        let mut ws = spaces;
        while ws < chars.len() && matches!(chars[chars.len() - 1 - ws], ' ' | '\t') {
          ws += 1;
        }
        chars.truncate(chars.len() - ws);
        if spaces >= 2 {
          chars.push('\\');
        }
        chars.push('\n');
        i += 1;
        continue;
      }
      chars.push(c);
      i += 1;
    }
    // End-of-text trailing whitespace is outside spans by construction.
    while matches!(chars.last(), Some(' ') | Some('\t')) {
      chars.pop();
    }
  }
  // Bracket matches for THIS `chars`, shared by every lookup in this parse.
  let mut brackets = BracketMemo::default();
  // Unclosed-construct scans memoised for THIS parse (issue #61): unclosed
  // `\(`, `$`, backticks and `<` used to rescan the suffix from every opener.
  let mut scans = ScanMemo::new();
  let mut close_paren: Option<Vec<u32>> = None;
  let mut out = String::new();
  let mut out_len: usize = 0; // UTF-16 length of `out`
  let mut marks: Vec<InlineMark> = Vec::new();
  let mut delims: Vec<Delim> = Vec::new();
  let mut i = 0;

  // Find the next index of `needle` (as chars) starting at `from`.
  let find_from = |from: usize, needle: &[char]| -> Option<usize> {
    if needle.is_empty() {
      return None;
    }
    let mut j = from;
    while j + needle.len() <= chars.len() {
      if chars[j..j + needle.len()] == *needle {
        return Some(j);
      }
      j += 1;
    }
    None
  };

  // Emit a resolved link or image into out/marks (text parsed recursively,
  // through the label cache the nested-link check shares).
  #[allow(clippy::too_many_arguments)]
  let push_link = |kind: &str,
                   label: &str,
                   href: String,
                   title: Option<String>,
                   out: &mut String,
                   out_len: &mut usize,
                   marks: &mut Vec<InlineMark>,
                   cache: &mut LinkCache| {
    let start = *out_len;
    let parsed = parse_label_cached(label, defs, cache);
    out.push_str(&parsed.text);
    *out_len += parsed.text.encode_utf16().count();
    for m in &parsed.marks {
      marks.push(InlineMark {
        start: m.start + start,
        end: m.end + start,
        kind: m.kind.clone(),
        href: m.href.clone(),
        title: m.title.clone(),
      });
    }
    marks.push(InlineMark {
      start,
      end: *out_len,
      kind: kind.to_string(),
      href: Some(href),
      title,
    });
  };

  while i < chars.len() {
    // Inline math, LaTeX form: \( … \) (normalized to `$ … $` on write).
    // Checked before the escape arm, which would otherwise eat the `\(`.
    if chars[i] == '\\'
      && chars.get(i + 1) == Some(&'(')
      && let Some(close) = {
        let table = close_paren.get_or_insert_with(|| next_pair_table(&chars, '\\', ')'));
        (table[i + 2] != u32::MAX).then_some(table[i + 2] as usize)
      }
    {
      let inner: String = chars[i + 2..close].iter().collect();
      let inner = inner.trim().to_string();
      if !inner.is_empty() {
        let start = out_len;
        out.push_str(&inner);
        out_len += inner.encode_utf16().count();
        marks.push(InlineMark {
          start,
          end: out_len,
          kind: "math".into(),
          href: None,
          title: None,
        });
        i = close + 2;
        continue;
      }
    }
    // Inline math, dollar form: `$ … $` — the opener may not be followed
    // by whitespace, the closer may not be preceded by whitespace or
    // followed by a digit (the Pandoc rules; keeps `$5 and $10` literal).
    if chars[i] == '$' && chars.get(i + 1) != Some(&'$') {
      if let Some(close) = find_math_closer(&chars, i + 1, &mut scans) {
        let inner: String = chars[i + 1..close].iter().collect();
        let start = out_len;
        out.push_str(&inner);
        out_len += inner.encode_utf16().count();
        marks.push(InlineMark {
          start,
          end: out_len,
          kind: "math".into(),
          href: None,
          title: None,
        });
        i = close + 1;
        continue;
      }
    }
    // Backslash escape: `\*` is a literal `*` (any ASCII punctuation).
    if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
      out.push(chars[i + 1]);
      out_len += chars[i + 1].len_utf16();
      i += 2;
      continue;
    }
    // Entity / numeric character reference: decodes to plain TEXT (the
    // result can't open emphasis or any structure).
    if chars[i] == '&'
      && let Some((decoded, used)) = parse_entity(&chars, i)
    {
      out.push_str(&decoded);
      out_len += decoded.encode_utf16().count();
      i += used;
      continue;
    }
    // Autolink: <https://…> or <user@host> — the URL is both text and target.
    // With no `>` anywhere at/after here, BOTH sub-scans below are known to
    // fail (each needs one): the whole arm falls through in O(1).
    if chars[i] == '<' && i < scans.gt_dead_from {
      if i + 1 < scans.gt_dead_from {
        if let Some(close) = find_from(i + 1, &['>']) {
          let inner: String = chars[i + 1..close].iter().collect();
          let href = autolink_target(&inner);
          if let Some(href) = href {
            let start = out_len;
            out.push_str(&inner);
            out_len += inner.encode_utf16().count();
            marks.push(InlineMark {
              start,
              end: out_len,
              kind: "link".into(),
              href: Some(href),
              title: None,
            });
            i = close + 1;
            continue;
          }
        } else {
          scans.gt_dead_from = i + 1;
        }
      }
      // Raw inline HTML: a valid tag/comment/PI/declaration/CDATA shape
      // passes through verbatim under an `html` mark.
      if let Some(end) = inline_html_end(&chars, i, Some(&mut scans)) {
        let raw: String = chars[i..end].iter().collect();
        let start = out_len;
        out.push_str(&raw);
        out_len += raw.encode_utf16().count();
        marks.push(InlineMark {
          start,
          end: out_len,
          kind: "html".into(),
          href: None,
          title: None,
        });
        i = end;
        continue;
      }
    }
    // Image: ![alt](dest "title") | ![alt][label] | ![alt][] | ![alt] —
    // same bridge as links; the alt keeps its inner marks (HTML flattens
    // them to plain text, markdown re-renders them).
    // Past `MAX_INLINE_NESTING` the brackets stay literal (see there).
    if chars[i] == '!'
      && i + 1 < chars.len()
      && chars[i + 1] == '['
      && let Some(close) = brackets.matching(&chars, i + 1)
      && brackets.nesting(i + 1) < MAX_INLINE_NESTING
    {
      let label: String = chars[i + 2..close].iter().collect();
      if !label.is_empty() {
        if close + 1 < chars.len() && chars[close + 1] == '(' {
          if let Some((href, title, next)) = parse_link_suffix(&chars, close + 2) {
            push_link(
              "image",
              &label,
              href,
              title,
              &mut out,
              &mut out_len,
              &mut marks,
              cache,
            );
            i = next;
            continue;
          }
        }
        if !defs.is_empty() {
          let (ref_label, next) = if close + 1 < chars.len() && chars[close + 1] == '[' {
            match brackets.matching(&chars, close + 1) {
              Some(end2) => {
                let second: String = chars[close + 2..end2].iter().collect();
                if second.is_empty() {
                  (label.clone(), end2 + 1) // collapsed ![alt][]
                } else {
                  (second, end2 + 1) // full ![alt][label]
                }
              }
              None => (label.clone(), close + 1),
            }
          } else {
            (label.clone(), close + 1) // shortcut ![alt]
          };
          if let Some((dest, title)) = defs.get(&normalize_label(&ref_label)) {
            push_link(
              "image",
              &label,
              dest.clone(),
              title.clone(),
              &mut out,
              &mut out_len,
              &mut marks,
              cache,
            );
            i = next;
            continue;
          }
        }
      }
    }
    // Footnote reference: `[^label]` (GFM). The brackets and caret are
    // stripped to bare TEXT (the label) under a `footnote` mark carrying the
    // label as `href` — the same delimiter-strip + mark shape inline math
    // uses. Labels hold no whitespace or brackets; checked before the link
    // arm so `[^x]` never parses as a shortcut link. Whether a matching
    // definition exists is decided at render time, not here (an undefined
    // reference round-trips to `[^x]` regardless and degrades to literal HTML).
    if chars[i] == '['
      && chars.get(i + 1) == Some(&'^')
      && let Some(close) = brackets.matching(&chars, i)
      && close > i + 2
    {
      let label: String = chars[i + 2..close].iter().collect();
      if !label
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '[' | ']' | '^'))
      {
        let start = out_len;
        out.push_str(&label);
        out_len += label.encode_utf16().count();
        marks.push(InlineMark {
          start,
          end: out_len,
          kind: "footnote".into(),
          href: Some(label),
          title: None,
        });
        i = close + 1;
        continue;
      }
    }
    // Link: [text](dest "title") | [text][label] | [text][] | [shortcut]
    if chars[i] == '['
      && let Some(close) = brackets.matching(&chars, i)
      && brackets.nesting(i) < MAX_INLINE_NESTING
    {
      let label: String = chars[i + 1..close].iter().collect();
      // A link's text may not itself contain a link (CommonMark §6.3): if
      // the label holds a top-level `[…](…)`/`[…][…]`/autolink, the OUTER
      // brackets stay literal and the inner link wins. `[foo [bar](/uri)]`
      // is therefore `[foo <a>bar</a>]`, not a nested anchor.
      // The nested-link check is (a) computed LAZILY — it only ever gates the
      // two link forms below, so computing it up-front made EVERY bracket pay
      // a recursive re-parse — and (b) memoized through a cache SHARED across
      // the whole parse (see `LinkCache`), which is what removes the second
      // re-parse of a label that DOES contain a link.
      //
      // Both together turn parsing from exponential to flat on nested input.
      // Measured (release, `import_markdown`), before -> after:
      //   `[[[[a]]]]`            depth 24:   ~6s   -> 0ms
      //   `[[[a](/u)](/u)](/u)`  depth 20:  834ms  -> 0ms   (depth 32: 0ms)
      //   `[![[![…a](u)](u)](u)](u)` depth 40: 1317ms -> 0ms (the cache now
      //   also feeds `push_link`; it used to cover the check only — P1-7)
      // This matters because `import_markdown` runs on user-supplied markdown
      // (import + MCP), so ~170 bytes used to pin a core for minutes.
      //
      // Output is unchanged: laziness is `!A && X` -> `X && !A` with both pure
      // (differential-fuzzed, ~2.8M inputs, zero divergence), and the memo is a
      // pure cache over a pure function. Pinned by tests/nested_bracket_perf.rs.
      // Inline form — empty text is allowed (`[](/url)` → empty anchor).
      if close + 1 < chars.len() && chars[close + 1] == '(' {
        if let Some((href, title, next)) = parse_link_suffix(&chars, close + 2) {
          if !label_has_link_cached(&label, defs, cache) {
            push_link(
              "link",
              &label,
              href,
              title,
              &mut out,
              &mut out_len,
              &mut marks,
              cache,
            );
            i = next;
            continue;
          }
        }
      }
      if !label.is_empty() {
        // Reference forms (full / collapsed / shortcut).
        if !defs.is_empty() {
          let (ref_label, next) = if close + 1 < chars.len() && chars[close + 1] == '[' {
            match brackets.matching(&chars, close + 1) {
              Some(end2) => {
                let second: String = chars[close + 2..end2].iter().collect();
                if second.is_empty() {
                  (label.clone(), end2 + 1) // collapsed [text][]
                } else {
                  (second, end2 + 1) // full [text][label]
                }
              }
              None => (label.clone(), close + 1),
            }
          } else {
            (label.clone(), close + 1) // shortcut [text]
          };
          if let Some((dest, title)) = defs.get(&normalize_label(&ref_label)) {
            if !label_has_link_cached(&label, defs, cache) {
              push_link(
                "link",
                &label,
                dest.clone(),
                title.clone(),
                &mut out,
                &mut out_len,
                &mut marks,
                cache,
              );
              i = next;
              continue;
            }
          }
        }
      }
    }
    // Emphasis delimiters (* and _): record the run with its flanking
    // properties; pairing happens after the scan (spec algorithm).
    if chars[i] == '*' || chars[i] == '_' {
      let c = chars[i];
      let mut j = i;
      while j < chars.len() && chars[j] == c {
        j += 1;
      }
      let count = j - i;
      let prev = if i == 0 { None } else { Some(chars[i - 1]) };
      let next = chars.get(j).copied();
      let (can_open, can_close) = flanking(c, prev, next);
      let start = out_len;
      for _ in 0..count {
        out.push(c);
      }
      out_len += count; // BMP chars: 1 UTF-16 unit each
      delims.push(Delim {
        c,
        start,
        cur_start: start,
        count,
        orig: count,
        can_open,
        can_close,
      });
      i = j;
      continue;
    }
    // GFM extended autolinks: bare www./http(s):// URLs and emails at a
    // word boundary (start, whitespace, or `*`/`_`/`~`/`(`).
    if allow_autolink
      && (i == 0 || chars[i - 1].is_whitespace() || matches!(chars[i - 1], '*' | '_' | '~' | '('))
      && let Some((len, href)) = extended_autolink(&chars, i, &mut scans.email_dead)
    {
      let text: String = chars[i..i + len].iter().collect();
      let start = out_len;
      out.push_str(&text);
      out_len += text.encode_utf16().count();
      marks.push(InlineMark {
        start,
        end: out_len,
        kind: "link".into(),
        href: Some(href),
        title: None,
      });
      i += len;
      continue;
    }
    // Strike: ~~...~~
    // GFM strikethrough: one or two tildes close on a run of the same
    // length (three or more stay literal).
    if chars[i] == '~' {
      let mut n = 0;
      while i + n < chars.len() && chars[i + n] == '~' {
        n += 1;
      }
      if n <= 2 {
        // A SINGLE tilde may not open or close INTRAWORD — our one deliberate
        // divergence from GitHub here.
        //
        // GitHub strikes `1~2 … 1~2` (verified 2026-08-04 against its own
        // renderer: `1<del>2个 Token，… ≈ 1</del>2个 Token`), and so did we.
        // On GitHub that is survivable because the SOURCE survives — you see
        // the tildes and can escape them. Mica is WYSIWYG: the tildes are
        // consumed into a mark and the writer cannot get them back by editing
        // the text, so a range like "1~2 个 Token" written twice in one
        // paragraph silently eats everything between. Range notation is far
        // more common in prose than single-tilde strikethrough, which Mica
        // never even emits (the exporter always writes `~~`), so the trade is
        // one-sided.
        //
        // Exactly the shape of CommonMark's intraword `_` rule, and for the
        // same reason: a delimiter with word characters on BOTH sides is
        // almost always literal. `~struck~` (spaces around it) still works and
        // `~~` is untouched. Mirrored in Dart `_parseInlineMemo`.
        let intraword = |at: usize, run: usize| {
          at > 0
            && is_word_char(chars[at - 1])
            && chars.get(at + run).copied().is_some_and(is_word_char)
        };
        let mut j = i + n;
        let mut close = None;
        while j < chars.len() && !(n == 1 && intraword(i, n)) {
          if chars[j] == '`'
            && let Some(after) = scans.code_span_close(&chars, j)
          {
            j = after;
            continue;
          }
          if chars[j] == '~' && (j == 0 || chars[j - 1] != '\\') {
            let mut m = 0;
            while j + m < chars.len() && chars[j + m] == '~' {
              m += 1;
            }
            if m == n && j > i + n && !(n == 1 && intraword(j, m)) {
              close = Some(j);
              break;
            }
            j += m;
          } else {
            j += 1;
          }
        }
        if let Some(end) = close {
          let inner: String = chars[i + n..end].iter().collect();
          push_inner(
            &inner,
            "strike",
            None,
            defs,
            &mut out,
            &mut out_len,
            &mut marks,
          );
          i = end + n;
          continue;
        }
      }
      // A run that does not open (3+, or no closer) is literal AS A WHOLE.
      // Falling through emitted ONE tilde and re-scanned the rest as a shorter
      // run, so `~~~a~~` struck `a` (GFM keeps it all literal) and `~~a~`
      // struck too — and the Dart mirror, which already wrote the whole run,
      // disagreed with this engine.
      for _ in 0..n {
        out.push('~');
      }
      out_len += n;
      i += n;
      continue;
    }
    // Inline code: an N-backtick run closes only on a run of exactly N;
    // line endings become spaces; one leading+trailing space strips when
    // both exist and the content isn't all spaces. No escapes inside.
    if chars[i] == '`' {
      let mut n = 0;
      while i + n < chars.len() && chars[i + n] == '`' {
        n += 1;
      }
      // No run of length N anywhere ahead? An earlier failure already proved
      // it (run lengths are position facts) — backtick runs unclosed used to
      // cost one full suffix scan each (issue #61).
      let mut close = None;
      if !scans.code_dead.get(&n).is_some_and(|&from| from <= i + n) {
        let mut j = i + n;
        while j < chars.len() {
          if chars[j] == '`' {
            let mut m = 0;
            while j + m < chars.len() && chars[j + m] == '`' {
              m += 1;
            }
            if m == n {
              close = Some(j);
              break;
            }
            j += m;
          } else {
            j += 1;
          }
        }
        if close.is_none() {
          let entry = scans.code_dead.entry(n).or_insert(usize::MAX);
          *entry = (*entry).min(i + n);
        }
      }
      if let Some(end) = close {
        let mut inner: String = chars[i + n..end]
          .iter()
          .map(|&c| if c == '\n' { ' ' } else { c })
          .collect();
        if inner.len() >= 2
          && inner.starts_with(' ')
          && inner.ends_with(' ')
          && !inner.bytes().all(|b| b == b' ')
        {
          inner = inner[1..inner.len() - 1].to_string();
        }
        let start = out_len;
        out.push_str(&inner);
        out_len += inner.encode_utf16().count();
        marks.push(InlineMark {
          start,
          end: out_len,
          kind: "code".into(),
          href: None,
          title: None,
        });
        i = end + n;
        continue;
      }
      // No closer: the run is literal text.
      for _ in 0..n {
        out.push('`');
      }
      out_len += n;
      i += n;
      continue;
    }
    out.push(chars[i]);
    out_len += chars[i].len_utf16();
    i += 1;
  }

  process_emphasis(&mut out, &mut marks, &mut delims);

  ParsedInline { text: out, marks }
}

/// One run of `*`/`_` delimiters, tracked in output (UTF-16) coordinates.
pub(crate) struct Delim {
  c: char,
  start: usize,     // original run start in `out`
  cur_start: usize, // advances as the closer side is consumed
  count: usize,     // remaining delimiter characters
  orig: usize,      // original run length (rule-of-3)
  can_open: bool,
  can_close: bool,
}

pub(crate) fn is_md_punct(c: char) -> bool {
  // CommonMark 0.31: "Unicode punctuation" = general categories P* AND S*
  // (symbols — currency, math, arrows — count too). std has no category
  // tables, so cover the blocks that occur in real text; extend on demand.
  c.is_ascii_punctuation()
    || matches!(c,
      '\u{00A1}'..='\u{00A9}'   // ¡¢£¤¥¦§¨© Latin-1 punct/symbols
        | '\u{00AB}'..='\u{00B1}' // «¬®¯°± (skip ª, a letter)
        | '\u{00B4}' | '\u{00B6}'..='\u{00B8}' | '\u{00BB}' | '\u{00BF}'
        | '\u{00D7}' | '\u{00F7}' // × ÷
        | '\u{2000}'..='\u{206F}' // general punctuation
        | '\u{20A0}'..='\u{20CF}' // currency symbols (€ …)
        | '\u{2100}'..='\u{2BFF}' // letterlike, arrows, math, misc symbols
        | '\u{3000}'..='\u{303F}' // CJK punctuation
        | '\u{FE30}'..='\u{FE4F}' // CJK compat forms
        | '\u{FF01}'..='\u{FF0F}' // fullwidth punct
        | '\u{FF1A}'..='\u{FF20}' | '\u{FF3B}'..='\u{FF40}' | '\u{FF5B}'..='\u{FF65}')
}

/// A CJK character (BMP): ideographs, kana, hangul, bopomofo, AND CJK
/// punctuation (`。、！？「」…`). Used to make emphasis CJK-friendly — see
/// [`flanking`]. Kept byte-identical to the Dart `_isCjk` (marks.dart).
pub(crate) fn is_cjk(c: char) -> bool {
  matches!(c,
    '\u{1100}'..='\u{11FF}'   // Hangul Jamo
      | '\u{2E80}'..='\u{2EFF}' // CJK Radicals Supplement
      | '\u{3000}'..='\u{303F}' // CJK Symbols and Punctuation
      | '\u{3040}'..='\u{30FF}' // Hiragana + Katakana
      | '\u{3100}'..='\u{312F}' // Bopomofo
      | '\u{3130}'..='\u{318F}' // Hangul Compatibility Jamo
      | '\u{31C0}'..='\u{31EF}' // CJK Strokes
      | '\u{3200}'..='\u{33FF}' // Enclosed CJK + CJK Compatibility
      | '\u{3400}'..='\u{4DBF}' // CJK Ext A
      | '\u{4E00}'..='\u{9FFF}' // CJK Unified Ideographs
      | '\u{A000}'..='\u{A4CF}' // Yi
      | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
      | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
      | '\u{FE30}'..='\u{FE4F}' // CJK Compatibility Forms
      | '\u{FF00}'..='\u{FFEF}') // Halfwidth and Fullwidth Forms
}

/// A "word" character for the single-tilde intraword rule: neither whitespace
/// nor punctuation.
///
/// CJK counts as a WORD character here, unlike in [`flanking`] below, where the
/// CJK amendment treats it as a boundary. That amendment exists so
/// `**加粗。**后文` can close — it is about PUNCTUATION adjacency and has no
/// analogue for `~`. Counting CJK as word is what makes `一~二` behave like
/// `1~2`: the same range notation written the other way.
pub(crate) fn is_word_char(c: char) -> bool {
  !c.is_whitespace() && !is_md_punct(c)
}

/// Left/right flanking → (can_open, can_close), with the CJK-friendly amendment
/// (markdown-cjk-friendly). Plain CommonMark treats CJK punctuation (`。`) as
/// "punctuation", so `**加粗。**后文` can't close — a `。` before the `**` and a
/// letter after fail the flanking test. A Chinese sentence ends in `。`/`,` far
/// more often than a space, so this bit constantly (and broke round-trip: the
/// exporter emits `**…。**x` which then wouldn't re-parse). Fix: split
/// "punctuation" into NON-CJK punctuation (strict rule kept) and CJK
/// punctuation/characters, which instead RELAX flanking the way whitespace does
/// in Latin (a CJK char is a word boundary). ASCII inputs are unaffected
/// (`is_cjk` is false), so the CommonMark scoreboard stays 641/641. Mirrored in
/// the Dart `_flanking` (marks.dart).
pub(crate) fn flanking(c: char, prev: Option<char>, next: Option<char>) -> (bool, bool) {
  let prev_ws = prev.is_none_or(char::is_whitespace);
  let next_ws = next.is_none_or(char::is_whitespace);
  let prev_cjk = prev.is_some_and(is_cjk);
  let next_cjk = next.is_some_and(is_cjk);
  let prev_ncp = prev.is_some_and(is_md_punct) && !prev_cjk; // non-CJK punct
  let next_ncp = next.is_some_and(is_md_punct) && !next_cjk;

  let left = !next_ws && (!next_ncp || prev_ws || prev_ncp || prev_cjk);
  let right = !prev_ws && (!prev_ncp || next_ws || next_ncp || next_cjk);

  if c == '_' {
    (
      left && (!right || prev_ncp || prev_cjk),
      right && (!left || next_ncp || next_cjk),
    )
  } else {
    (left, right)
  }
}

/// The spec's process-emphasis: pair closers with the nearest valid opener
/// (same char, rule-of-3), strong before em, deleting used delimiter
/// characters and emitting bold/italic marks over the content between.
pub(crate) fn process_emphasis(
  out: &mut String,
  marks: &mut Vec<InlineMark>,
  delims: &mut [Delim],
) {
  if delims.is_empty() {
    return;
  }
  let mut deletions: Vec<(usize, usize)> = Vec::new(); // (start, len) in out coords

  let mut closer_i = 0;
  while closer_i < delims.len() {
    if delims[closer_i].count == 0 || !delims[closer_i].can_close {
      closer_i += 1;
      continue;
    }
    // Nearest opener walking back.
    let mut opener_i = None;
    let mut k = closer_i;
    while k > 0 {
      k -= 1;
      let o = &delims[k];
      if o.count == 0 || !o.can_open || o.c != delims[closer_i].c {
        continue;
      }
      // Rule of 3.
      let cl = &delims[closer_i];
      if (o.can_close || cl.can_open)
        && (o.orig + cl.orig) % 3 == 0
        && !(o.orig % 3 == 0 && cl.orig % 3 == 0)
      {
        continue;
      }
      opener_i = Some(k);
      break;
    }
    let Some(oi) = opener_i else {
      // No opener: if it can't also open, it is pure text now.
      closer_i += 1;
      continue;
    };

    let use_n = if delims[oi].count >= 2 && delims[closer_i].count >= 2 {
      2
    } else {
      1
    };
    // Opener consumes from its right edge; closer from its left edge.
    delims[oi].count -= use_n;
    let o_del = delims[oi].start + delims[oi].count;
    deletions.push((o_del, use_n));
    let c_del = delims[closer_i].cur_start;
    deletions.push((c_del, use_n));
    delims[closer_i].cur_start += use_n;
    delims[closer_i].count -= use_n;

    marks.push(InlineMark {
      start: o_del + use_n,
      end: c_del,
      kind: if use_n == 2 { "bold" } else { "italic" }.to_string(),
      href: None,
      title: None,
    });

    // Delimiters between opener and closer can never pair across.
    for d in delims[oi + 1..closer_i].iter_mut() {
      d.count = 0;
    }
    if delims[closer_i].count == 0 {
      closer_i += 1;
    }
  }

  if deletions.is_empty() {
    return;
  }
  deletions.sort_unstable();

  // Rebuild the text without the consumed delimiter characters and remap
  // every mark offset past the deletions.
  let units: Vec<u16> = out.encode_utf16().collect();
  let mut keep: Vec<u16> = Vec::with_capacity(units.len());
  let mut removed_before: Vec<usize> = Vec::with_capacity(units.len() + 1);
  let mut di = 0;
  let mut removed = 0usize;
  let mut skip_until = 0usize;
  for (idx, &u) in units.iter().enumerate() {
    removed_before.push(removed);
    if idx >= skip_until && di < deletions.len() && deletions[di].0 == idx {
      skip_until = idx + deletions[di].1;
      di += 1;
    }
    if idx < skip_until {
      removed += 1;
    } else {
      keep.push(u);
    }
  }
  removed_before.push(removed);

  *out = String::from_utf16_lossy(&keep);
  for m in marks.iter_mut() {
    m.start -= removed_before[m.start.min(removed_before.len() - 1)];
    m.end -= removed_before[m.end.min(removed_before.len() - 1)];
  }
  marks.retain(|m| m.end > m.start);
}

/// CommonMark autolink target for `<inner>`: an absolute URI (scheme:) maps
/// to itself, a bare email maps to `mailto:`; anything else is not an
/// autolink.
pub(crate) fn autolink_target(inner: &str) -> Option<String> {
  if inner.is_empty() || inner.chars().any(|c| c.is_whitespace() || c == '<') {
    return None;
  }
  let bytes = inner.as_bytes();
  // scheme: ALPHA (ALPHA/DIGIT/+/-/.)* ':'
  if let Some(colon) = inner.find(':') {
    let scheme = &inner[..colon];
    if scheme.len() >= 2
      && scheme.len() <= 32
      && scheme.as_bytes()[0].is_ascii_alphabetic()
      && scheme
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
      && colon + 1 < inner.len()
    {
      return Some(inner.to_string());
    }
  }
  // email: x@y.z (loose; backslash escapes never count)
  if inner.contains('\\') {
    return None;
  }
  if let Some(at) = inner.find('@') {
    let (local, host) = inner.split_at(at);
    let host = &host[1..];
    if !local.is_empty()
      && host.contains('.')
      && !host.ends_with('.')
      && bytes.iter().filter(|&&b| b == b'@').count() == 1
    {
      return Some(format!("mailto:{inner}"));
    }
  }
  None
}

/// Parse [inner] recursively and append it to [out], adding a [kind] mark over
/// the appended range.
pub(crate) fn push_inner(
  inner: &str,
  kind: &str,
  href: Option<String>,
  defs: &RefDefs,
  out: &mut String,
  out_len: &mut usize,
  marks: &mut Vec<InlineMark>,
) {
  let start = *out_len;
  let parsed = parse_inline_with(inner, defs);
  out.push_str(&parsed.text);
  *out_len += parsed.text.encode_utf16().count();
  for m in parsed.marks {
    marks.push(InlineMark {
      start: m.start + start,
      end: m.end + start,
      kind: m.kind,
      href: m.href,
      title: m.title,
    });
  }
  marks.push(InlineMark {
    start,
    end: *out_len,
    kind: kind.to_string(),
    href,
    title: None,
  });
}

/// An inline mark over a `[start, end)` range of a block's plain text.
pub(crate) struct InlineMark {
  pub(crate) start: usize,
  pub(crate) end: usize,
  pub(crate) kind: String,
  pub(crate) href: Option<String>,
  pub(crate) title: Option<String>,
}

pub(crate) fn marks_from_block(block: &Block) -> Vec<InlineMark> {
  let Some(raw) = block.data.get("marks").and_then(Value::as_array) else {
    return Vec::new();
  };
  // Mark offsets index into UTF-16 code units (the Flutter client's string
  // model); convert against this text's UTF-16 view when slicing.
  let units: Vec<u16> = block.text.encode_utf16().collect();
  let len = units.len();
  let mut marks = Vec::new();
  for m in raw {
    let Some(obj) = m.as_object() else { continue };
    let start = obj.get("start").and_then(Value::as_u64).map(|v| v as usize);
    let end = obj.get("end").and_then(Value::as_u64).map(|v| v as usize);
    let kind = obj.get("type").and_then(Value::as_str);
    if let (Some(start), Some(end), Some(kind)) = (start, end, kind) {
      let start = start.min(len);
      let end = end.min(len);
      // Empty-text links (`[](/url)`) are a legitimate zero-width mark; every
      // other kind needs real content to wrap.
      if end > start || kind == "link" {
        marks.push(InlineMark {
          start,
          end,
          kind: kind.to_string(),
          href: obj.get("href").and_then(Value::as_str).map(str::to_string),
          title: obj.get("title").and_then(Value::as_str).map(str::to_string),
        });
      }
    }
  }
  marks
}

pub(crate) fn escape_title(title: &str) -> String {
  let mut out = String::with_capacity(title.len());
  let mut chars = title.chars().peekable();
  while let Some(c) = chars.next() {
    if c == '&' {
      out.push_str("&amp;");
      continue;
    }
    if c == '"' || (c == '\\' && chars.peek().is_none_or(|c| c.is_ascii_punctuation())) {
      out.push('\\');
    }
    out.push(c);
  }
  out
}

pub(crate) fn escape_destination(href: &str) -> String {
  let angle = href.contains(char::is_whitespace);
  let mut depth = 0i32;
  let mut balanced = true;
  for c in href.chars() {
    if c == '(' {
      depth += 1;
    }
    if c == ')' {
      depth -= 1;
      balanced &= depth >= 0;
    }
  }
  balanced &= depth == 0;
  let mut escaped = String::with_capacity(href.len());
  for c in href.chars() {
    match c {
      '&' => escaped.push_str("&amp;"),
      '(' | ')' if !angle && !balanced => {
        escaped.push('\\');
        escaped.push(c);
      }
      '\\' | '<' | '>' => {
        escaped.push('\\');
        escaped.push(c);
      }
      _ => escaped.push(c),
    }
  }
  if angle {
    format!("<{escaped}>")
  } else {
    escaped
  }
}

/// Render a block's text with its inline marks back to Markdown. Code blocks and
/// tables carry no inline marks and are emitted verbatim by their own arms.
/// Escape characters our (and CommonMark's) inline grammar would otherwise
/// interpret, so literal text survives a round-trip.
pub(crate) fn escape_inline(text: &str) -> String {
  let mut out = String::with_capacity(text.len());
  let chars: Vec<char> = text.chars().collect();
  for (i, &c) in chars.iter().enumerate() {
    // A backslash right before a newline IS a hard break — keep it raw.
    if c == '\\' && chars.get(i + 1) == Some(&'\n') {
      out.push(c);
      continue;
    }
    if c == '&' && parse_entity(&chars, i).is_some() {
      out.push_str("&amp;");
      continue;
    }
    if matches!(c, '\\' | '*' | '_' | '`' | '~' | '[' | ']' | '<' | '$') {
      out.push('\\');
    }
    out.push(c);
  }
  out
}

/// A paragraph whose text LOOKS like a block marker (`- x`, `> x`, `# x`,
/// `1. x`, `1) x`, `---`, `\[`) must escape its leader or it changes kind on
/// re-import. Applied to EVERY line of a paragraph/quote — a hard break puts a
/// later line at a line start just as much as the first.
pub(crate) fn escape_block_leader(line: String) -> String {
  let t = line.as_str();
  let compact: String = t.chars().filter(|c| *c != ' ').collect();
  let divider_like = compact.len() >= 3 && compact.chars().all(|c| c == '-');
  // A paragraph that would read as a setext underline gets escaped too.
  let setext_like = !t.is_empty() && (t.bytes().all(|b| b == b'=') || t.bytes().all(|b| b == b'-'));
  let divider_like = divider_like || setext_like;
  // A marker is followed by a space, a tab, or nothing at all (an empty item
  // / empty heading is still that block).
  let marker_then_gap = |rest: &str| rest.is_empty() || rest.starts_with([' ', '\t']);
  if t.starts_with('>')
    || ((t.starts_with('-') || t.starts_with('+')) && marker_then_gap(&t[1..]))
    || divider_like
    || (t.starts_with('#') && marker_then_gap(t.trim_start_matches('#')))
  {
    return format!("\\{line}");
  }
  // `\[` opens a display-math block (`\[ … \]`) on re-import, and
  // `escape_inline` writes every literal `[` as `\[` — so a paragraph that was
  // just `[TODO]` came back as a math block. An entity is plain text to the
  // inline parser and opens nothing. Only where `math_block` would actually
  // fire — `\[` alone on the line, or a line ending in `\]` — so every other
  // `\[…` line keeps its familiar form.
  if let Some(rest) = t.strip_prefix("\\[")
    && (rest.trim().is_empty() || rest.trim_end().ends_with("\\]"))
  {
    return format!("&#91;{rest}");
  }
  let digits = t.bytes().take_while(u8::is_ascii_digit).count();
  if (1..=9).contains(&digits)
    && let Some(delim @ ('.' | ')')) = t[digits..].chars().next()
    && marker_then_gap(&t[digits + 1..])
  {
    return format!("{}\\{delim}{}", &t[..digits], &t[digits + 1..]);
  }
  line
}

/// [escape_block_leader] on every line of a multi-line text.
pub(crate) fn escape_block_leaders(text: &str) -> String {
  text
    .split('\n')
    .map(|l| escape_block_leader(l.to_string()))
    .collect::<Vec<_>>()
    .join("\n")
}

pub(crate) fn render_inline(block: &Block) -> String {
  if matches!(block.kind.as_str(), "code_block" | "code" | "table") {
    return block.text.clone();
  }
  let marks = marks_from_block(block);
  if marks.is_empty() {
    return escape_inline(&block.text);
  }
  let units: Vec<u16> = block.text.encode_utf16().collect();
  let refs: Vec<&InlineMark> = marks.iter().collect();
  render_span(&units, 0, units.len(), &refs)
}

/// Marks whose rendering is TERMINAL: the branch writes the span literally and
/// cannot nest anything inside it, so it must never win an exact-range tie — it
/// would swallow the mark it coincides with. `[`a`](/x)` used to export as
/// `` `a` ``, dropping the link outright (and with it the URL).
pub(crate) fn renders_terminal(kind: &str) -> bool {
  matches!(kind, "code" | "math" | "html" | "footnote")
}

/// Render `[lo, hi)` of the text wrapping marks as properly NESTED markdown:
/// the outermost mark opens once over its whole range with inner marks
/// recursing inside (`**bold *italic* tail**`) — per-segment wrapping breaks
/// round-trips. Marks that only partially overlap an enclosing one are
/// clipped (markdown cannot express crossing ranges).
pub(crate) fn render_span(units: &[u16], lo: usize, hi: usize, marks: &[&InlineMark]) -> String {
  let mut out = String::new();
  let mut pos = lo;
  // The pick used to rescan the WHOLE slice per emitted span (and the overlap
  // set again): O(marks²) on a paragraph of sibling marks. The sweep keeps the
  // same selection — clipped start, widest end, nestable on exact-range ties,
  // outer (later-appended) mark last — behind one sort (issue #61).
  let mut walker = SpanWalker::new(marks, hi, SpanTies::Markdown);
  while pos <= hi {
    let Some(pick) = walker.next_pick(pos) else {
      out.push_str(&escape_inline(&String::from_utf16_lossy(&units[pos..hi])));
      break;
    };
    let (s, e) = (pick.s, pick.e);
    let m = pick.mark;
    let mut lead = escape_inline(&String::from_utf16_lossy(&units[pos..s]));
    if m.kind == "link" && lead.ends_with('!') {
      lead.pop();
      lead.push_str("\\!");
    }
    out.push_str(&lead);
    if m.kind == "html" {
      // Raw inline HTML writes back verbatim.
      out.push_str(&String::from_utf16_lossy(&units[s..e]));
      pos = e;
      continue;
    }
    if m.kind == "math" {
      let source = String::from_utf16_lossy(&units[s..e]);
      if source.contains('$') {
        out.push_str(&format!("\\({source}\\)"));
      } else {
        out.push_str(&format!("${source}$"));
      }
      pos = e;
      continue;
    }
    if m.kind == "footnote" {
      // The span text IS the label; the `[^…]` reference syntax is restored
      // from the mark's href (the label survives even if the span was edited).
      let label = m
        .href
        .clone()
        .unwrap_or_else(|| String::from_utf16_lossy(&units[s..e]));
      out.push_str(&format!("[^{label}]"));
      pos = e;
      continue;
    }
    if m.kind == "code" {
      // Code spans are literal — no escaping, no nested marks. The fence is
      // one backtick longer than any run inside; a space pads content that
      // starts/ends with a backtick or with stripped-on-read spaces.
      let raw = String::from_utf16_lossy(&units[s..e]);
      let mut longest = 0usize;
      let mut cur = 0usize;
      for c in raw.chars() {
        if c == '`' {
          cur += 1;
          longest = longest.max(cur);
        } else {
          cur = 0;
        }
      }
      let fence = "`".repeat(longest + 1);
      let pad = raw.starts_with('`')
        || raw.ends_with('`')
        || (raw.starts_with(' ') && raw.ends_with(' ') && !raw.bytes().all(|b| b == b' '));
      if pad {
        out.push_str(&format!("{fence} {raw} {fence}"));
      } else {
        out.push_str(&format!("{fence}{raw}{fence}"));
      }
      pos = e;
      continue;
    }
    let inner = walker.inner(&pick);
    let body = render_span(units, s, e, &inner);
    out.push_str(&match m.kind.as_str() {
      "bold" => format!("**{body}**"),
      "italic" => format!("*{body}*"),
      "strike" => format!("~~{body}~~"),
      "link" => {
        let href = m.href.as_deref().unwrap_or("");
        let plain = String::from_utf16_lossy(&units[s..e]);
        // A title-less link whose text IS its target → autolink form; a
        // bare `www.` link writes back bare (GFM re-links it on read). Both
        // shorthands write `plain`, so they DISCARD inner marks — only take
        // them when there are none to lose (`[`x`](x)` must stay bracketed).
        if m.title.is_none()
          && inner.is_empty()
          && plain.starts_with("www.")
          && href == format!("http://{plain}")
        {
          plain.to_string()
        } else if m.title.is_none()
          && !plain.is_empty()
          && inner.is_empty()
          && (href == plain || href == format!("mailto:{plain}"))
        {
          format!("<{plain}>")
        } else {
          let dest = escape_destination(href);
          match &m.title {
            Some(t) => format!("[{body}]({dest} \"{}\")", escape_title(t)),
            None => format!("[{body}]({dest})"),
          }
        }
      }
      "image" => {
        let href = m.href.as_deref().unwrap_or("");
        let dest = escape_destination(href);
        match &m.title {
          Some(t) => format!("![{body}]({dest} \"{}\")", escape_title(t)),
          None => format!("![{body}]({dest})"),
        }
      }
      _ => body,
    });
    pos = e;
  }
  out
}
