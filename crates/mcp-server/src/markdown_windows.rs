//! Mica MCP server — a thin proxy that exposes the Mica REST API as MCP tools so
//! an AI (Claude Desktop / Code / any MCP client) can list, read, create, and
//! write Mica documents. It calls the API over HTTP with a PAT; it holds no DB
//! or storage access of its own. "Backup/restore" here is just export/import —
//! real backups are the job of external tools pointed at the exports.
//!
//! Shipped as `mica-cli mcp` (this crate is a library; the CLI resolves the
//! server URL + PAT through its usual chain — env, flags, saved login — and
//! calls [serve_stdio]). Read-only mode makes every write tool refuse at call
//! time while the read tools stay listed.
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
pub(crate) fn pick_anchor(headings: Option<&Value>, anchor: &str, strict: bool) -> Result<String, String> {
  let needle = anchor.to_lowercase();
  let hits: Vec<(&str, &str)> = headings
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|h| {
      let text = h.get("text").and_then(Value::as_str)?;
      let block_id = h.get("block_id").and_then(Value::as_str)?;
      text.to_lowercase().contains(&needle).then_some((text, block_id))
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
  let mut start = None;
  let mut start_level = 0;
  for (i, line) in lines.iter().enumerate() {
    if let Some(level) = heading_level(line)
      && line.to_lowercase().contains(&needle_lc)
    {
      start = Some(i);
      start_level = level;
      break;
    }
  }
  let Some(start) = start else {
    return format!("[no heading contains {needle:?}; use mica_get_outline to list headings]");
  };
  let mut end = lines.len();
  for (i, line) in lines.iter().enumerate().skip(start + 1) {
    if heading_level(line).is_some_and(|lvl| lvl <= start_level) {
      end = i;
      break;
    }
  }
  lines[start..end].join("\n")
}

/// Control characters a JSON unescape produces from a LaTeX command, paired with
/// the escape that made them, and an example of the command each one eats.
/// `\times` under-escaped in JSON is not an error — `\t` is a legal escape — so
/// it decodes silently to TAB + "imes" and the formula is destroyed with no
/// diagnostic anywhere.
///
/// These four are safe to condemn: none has a legitimate reason to appear inside
/// a formula. `\n` and `\r` collide the same way (`\nabla`, `\rho`) but are
/// deliberately absent — a real newline is ordinary markdown and we cannot tell
/// the two apart. Their damage is at least visible: a newline breaks the math
/// run outright, so the page shows literal `$…$` rather than quietly wrong math.
pub(crate) const JSON_ESCAPE_COLLISIONS: [(char, &str, &str); 4] = [
  ('\u{0009}', r"\t", r"\times"),
  ('\u{000C}', r"\f", r"\frac"),
  ('\u{000B}', r"\v", r"\vec"),
  ('\u{0008}', r"\b", r"\beta"),
];

/// Reject markdown whose LaTeX was mangled by an under-escaped JSON string.
///
/// We are not the ones corrupting it — the server stores faithfully what it is
/// given, and a correctly escaped `\\times` round-trips fine. But this arrives
/// as valid JSON that is silently wrong, and the caller is usually a model
/// hand-writing escapes, which gets LaTeX wrong often enough that "AI writes its
/// answer into a page" (formulas and all) cannot be trusted without a check.
/// Refusing with a precise message lets the caller retry correctly; storing it
/// loses the formula forever, which is the failure this guard exists to stop.
///
/// Two rules, both chosen to be false-positive-free rather than thorough:
///  - FF / VT / BS anywhere: no legitimate markdown contains them.
///  - TAB *inside a math run only*: a tab is ordinary indentation elsewhere, but
///    inside `$…$` it is meaningless to LaTeX and means `\t…` was eaten.
pub(crate) fn reject_mangled_latex(markdown: &str) -> Result<(), McpError> {
  for (ch, escape, example) in JSON_ESCAPE_COLLISIONS {
    if !markdown.contains(ch) {
      continue;
    }
    // A tab is ordinary indentation outside a formula, so it only condemns
    // itself within one. The other three have no innocent reading anywhere.
    if ch == '\u{0009}' && !tab_inside_math(markdown) {
      continue;
    }
    return Err(mangled_latex_error(escape, example));
  }
  Ok(())
}

pub(crate) fn tab_inside_math(markdown: &str) -> bool {
  let chars: Vec<char> = markdown.chars().collect();
  mica_markdown::math_run_spans(markdown)
    .into_iter()
    .any(|(start, end)| chars[start..end].contains(&'\u{0009}'))
}

pub(crate) fn mangled_latex_error(escape: &str, example: &str) -> McpError {
  McpError::invalid_params(
    format!(
      "The markdown contains a raw control character where a LaTeX command should be: \
             `{escape}` was read as a JSON escape, so `{example}` arrived as a control character \
             instead of a command. The backslashes in the JSON string were under-escaped — in \
             JSON a literal backslash must be doubled. Send \"$\\\\times$\", not \"$\\times$\" \
             (the latter is a tab). Nothing was written; resend with every backslash doubled."
    ),
    None,
  )
}
