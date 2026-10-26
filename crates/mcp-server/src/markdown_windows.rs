//! Markdown windowing (anchor / section reads) plus the mangled-LaTeX guard.
use rmcp::ErrorData as McpError;
use serde_json::Value;

/// The markdown string out of whatever `/export/markdown` answered with.
///
/// That endpoint returns `Json<MarkdownExportResponse>` — a JSON OBJECT
/// `{"markdown": "..."}` — not a bare JSON string. This used to be
/// `other => other.to_string()`, so `full` became the SERIALIZED object: a
/// single line in which every newline is the two characters `\` and `n`.
///
/// Nothing errored. It broke the three things that read `full` as lines:
///
///   * `section` never matched — no line starts with `#`, so every call came
///     back "[no heading contains ...]" even for headings `mica_get_outline`
///     had just listed.
///   * `offset`/`limit` always reported "[lines 1-1 of 1]" and returned the
///     whole blob, so the token-saving window saved nothing.
///   * a plain read handed the model `{"markdown":"..."}` instead of markdown.
///
/// The last one was visible in every response and still read as an envelope
/// rather than a bug, which is why it survived: a wrong answer that looks like
/// a format is the kind that does not get reported.
pub(crate) fn unwrap_markdown(value: Value) -> String {
  match value {
    Value::String(s) => s,
    Value::Object(mut map) => match map.remove("markdown") {
      Some(Value::String(s)) => s,
      // Some other object: keep the old behaviour rather than inventing one.
      Some(other) => other.to_string(),
      None => Value::Object(map).to_string(),
    },
    other => other.to_string(),
  }
}

/// Return only the requested slice of a document's markdown, with a one-line
/// header so the model knows what it is looking at (and that there is more).
/// `section` wins over `offset`/`limit`. Pure — unit-tested below.
pub(crate) fn window_markdown(
  full: &str,
  offset: Option<usize>,
  limit: Option<usize>,
  section: Option<&str>,
) -> String {
  if let Some(section) = section {
    return section_markdown(full, section);
  }
  if offset.is_none() && limit.is_none() {
    return full.to_string();
  }
  let lines: Vec<&str> = full.lines().collect();
  let total = lines.len();
  // 1-based offset; clamp into range.
  let start = offset.unwrap_or(1).max(1).min(total.max(1)) - 1;
  let end = match limit {
    Some(n) => (start + n).min(total),
    None => total,
  };
  let body = lines.get(start..end).unwrap_or(&[]).join("\n");
  format!(
    "[lines {}-{} of {}]\n{}",
    start + 1,
    end.max(start + 1).min(total.max(1)),
    total,
    body
  )
}

/// The markdown from the first heading whose text contains [needle] (case-
/// insensitive) through the line before the next heading of the same or higher
/// level. Empty (with a note) when no heading matches. Pure.
/// Which heading a piece of anchor text picks out of an outline — the pure half
/// of `resolve_anchor`, so the matching rules are testable without a server.
///
/// `strict` is for `delete`: more than one match is an error naming the
/// candidates, not the first one silently. Substring matching is generous by
/// design (a caller says "Setup", not the exact heading), and generosity that
/// is fine for choosing an insert position is not fine for choosing what to
/// destroy.
pub(crate) fn pick_anchor(
  headings: Option<&Value>,
  anchor: &str,
  strict: bool,
) -> Result<String, String> {
  let needle = anchor.to_lowercase();
  let hits: Vec<(&str, &str)> = headings
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|h| {
      let text = h.get("text").and_then(Value::as_str)?;
      let block_id = h.get("block_id").and_then(Value::as_str)?;
      text
        .to_lowercase()
        .contains(&needle)
        .then_some((text, block_id))
    })
    .collect();

  if strict && hits.len() > 1 {
    let listed = hits
      .iter()
      .map(|(text, id)| format!("{text:?} ({id})"))
      .collect::<Vec<_>>()
      .join(", ");
    return Err(format!(
      "delete: {anchor:?} matches {} headings — {listed}. Pass the block id of \
       the one you mean; deleting the wrong block is not undoable from here.",
      hits.len()
    ));
  }
  match hits.first() {
    Some((_, block_id)) => Ok((*block_id).to_string()),
    None => Err(format!(
      "{mode}: no heading contains {anchor:?}; pass a block id from \
       mica_get_outline, or the exact text of a heading.",
      mode = if strict { "delete" } else { "insert_at" }
    )),
  }
}

/// Above a fenced code block: the container's content line with up to three
/// leading spaces and any blockquote markers removed. `None` for an indented
/// (>=4 spaces) line — that is an indented code block, not a fence.
fn fence_line(line: &str) -> Option<&str> {
  let mut rest = line;
  loop {
    let indent = rest.len() - rest.trim_start_matches(' ').len();
    if indent > 3 {
      return None;
    }
    rest = &rest[indent..];
    match rest.strip_prefix('>') {
      Some(after) => rest = after.strip_prefix(' ').unwrap_or(after),
      None => return Some(rest),
    }
  }
}

/// An opening fence: three or more backticks or tildes, after container
/// prefixes. Everything up to the matching close is code, so no `#` inside it
/// can start or end a section.
fn fence_open(line: &str) -> Option<(char, usize)> {
  let rest = fence_line(line)?;
  let marker = rest.chars().next()?;
  if marker != '`' && marker != '~' {
    return None;
  }
  let len = rest.chars().take_while(|c| *c == marker).count();
  (len >= 3).then_some((marker, len))
}

/// Only the SAME marker at the SAME length or longer, with nothing but
/// whitespace after it, closes a fence — a ``` inside ```` is content.
fn fence_close(line: &str, marker: char, open_len: usize) -> bool {
  let Some(rest) = fence_line(line) else {
    return false;
  };
  let len = rest.chars().take_while(|c| *c == marker).count();
  len >= open_len && rest[len..].trim().is_empty()
}

pub(crate) fn section_markdown(full: &str, needle: &str) -> String {
  let needle_lc = needle.to_lowercase();
  let lines: Vec<&str> = full.lines().collect();
  let heading_level = |line: &str| -> Option<usize> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && line.chars().nth(hashes) == Some(' ') {
      Some(hashes)
    } else {
      None
    }
  };
  // A heading is only a heading OUTSIDE a fence. Markdown is scanned once,
  // keeping the fence state, rather than re-parsing the exported text with a
  // rule that disagrees with the engine (issue #117).
  let mut headings: Vec<Option<usize>> = Vec::with_capacity(lines.len());
  let mut fence: Option<(char, usize)> = None;
  for line in &lines {
    if let Some((marker, open_len)) = fence {
      headings.push(None);
      if fence_close(line, marker, open_len) {
        fence = None;
      }
      continue;
    }
    if let Some((marker, open_len)) = fence_open(line) {
      fence = Some((marker, open_len));
      headings.push(None);
      continue;
    }
    headings.push(heading_level(line));
  }
  let mut start = None;
  let mut start_level = 0;
  for (i, level) in headings.iter().enumerate() {
    if let Some(level) = level
      && lines[i].to_lowercase().contains(&needle_lc)
    {
      start = Some(i);
      start_level = *level;
      break;
    }
  }
  let Some(start) = start else {
    return format!("[no heading contains {needle:?}; use mica_get_outline to list headings]");
  };
  let mut end = lines.len();
  for (i, level) in headings.iter().enumerate().skip(start + 1) {
    if level.is_some_and(|lvl| lvl <= start_level) {
      end = i;
      break;
    }
  }
  lines[start..end].join("\n")
}

/// Reject markdown whose LaTeX was mangled by an under-escaped JSON string.
///
/// The check itself lives in `mica_markdown::latex_guard` (issue #83): the server
/// enforces it on every markdown write path, and this wrapper is only the early
/// rejection at the tool boundary — same rules, expressed as a tool error instead
/// of an HTTP 400.
pub(crate) fn reject_mangled_latex(markdown: &str) -> Result<(), McpError> {
  match mica_markdown::mangled_latex_collision(markdown) {
    Some((escape, example)) => Err(mangled_latex_error(escape, example)),
    None => Ok(()),
  }
}

pub(crate) fn mangled_latex_error(escape: &str, example: &str) -> McpError {
  McpError::invalid_params(mica_markdown::mangled_latex_message(escape, example), None)
}
