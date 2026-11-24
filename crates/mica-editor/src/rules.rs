//! Keyboard input rules (block shortcuts, inline mark triggers, slash & `[[` triggers).
//!
//! Mirrors `controller.dart::applyInputRules` and `_applyInlineRule`, with
//! full UTF-16 offset safety (never splitting surrogate pairs).

use serde_json::{Value, json};

use crate::marks::{EditorMark, apply_mark, shift_marks};
use crate::model::EditorBlock;
use crate::utf16::{utf16_len, utf16_slice};

/// Outcome of matching a keyboard input rule on the focused block.
#[derive(Debug, Clone, PartialEq)]
pub enum InputRuleMatch {
  /// Convert the block in place, stripping the leading marker and leaving `rest`
  /// with caret at 0.
  ConvertBlock {
    kind: String,
    data: Value,
    text: String,
    marks: Vec<EditorMark>,
  },
  /// Convert the block into a `divider` and ensure a trailing editable paragraph.
  InsertDivider,
  /// Convert the block into an atomic `math_block` with `source` and ensure a
  /// trailing editable paragraph.
  ConvertToMathBlock { source: String },
  /// Strip inline delimiters and apply a mark over the inner run; place caret at
  /// `new_caret`.
  InlineMark {
    text: String,
    marks: Vec<EditorMark>,
    new_caret: u32,
  },
}

/// Evaluate block and inline keyboard input rules on `block` at UTF-16 `caret`.
pub fn match_input_rule(block: &EditorBlock, caret: u32) -> Option<InputRuleMatch> {
  if block.is_atomic() || block.kind == "table" {
    return None;
  }
  let text = block.text.as_str();
  let total_u16 = utf16_len(text);
  let caret = caret.min(total_u16);

  let convert = |kind: &str, data: Value, strip_u16: u32| -> InputRuleMatch {
    let rest = utf16_slice(text, strip_u16, total_u16).to_string();
    let rest_len = total_u16 - strip_u16;
    let marks = shift_marks(
      &block.marks(),
      0,
      strip_u16,
      -(strip_u16 as i64),
      rest_len,
      true,
    );
    InputRuleMatch::ConvertBlock {
      kind: kind.to_string(),
      data,
      text: rest,
      marks,
    }
  };

  // Heading: `# ` .. `###### ` (works on paragraphs AND existing headings to re-level).
  if block.kind == "paragraph" || block.kind == "heading" {
    for lvl in (1u32..=6).rev() {
      let strip = lvl + 1;
      if caret == strip && text.starts_with(&format!("{} ", "#".repeat(lvl as usize))) {
        return Some(convert("heading", json!({ "level": lvl }), strip));
      }
    }
  }

  if block.kind == "paragraph" {
    // Direct `- [ ] ` / `- [x] ` todo markers (e.g. typed fast or composed).
    for (pat, checked) in [
      ("- [ ] ", false),
      ("- [] ", false),
      ("- [x] ", true),
      ("- [X] ", true),
    ] {
      let pat_u16 = utf16_len(pat);
      if caret == pat_u16 && text.starts_with(pat) {
        return Some(convert("todo", json!({ "checked": checked }), pat_u16));
      }
    }

    // Bulleted list: `- `, `* `, `+ `, `• ` (note `•` is U+2022 = 1 UTF-16 unit).
    for pat in ["- ", "* ", "+ ", "• "] {
      let pat_u16 = utf16_len(pat);
      if caret == pat_u16 && text.starts_with(pat) {
        return Some(convert("bulleted_list", json!({}), pat_u16));
      }
    }

    // Block quote: `> `.
    if caret == 2 && text.starts_with("> ") {
      return Some(convert("quote", json!({}), 2));
    }

    // Numbered list: `1. `, `2. `, …
    if let Some((num, strip_u16)) = match_numbered_prefix(text)
      && caret == strip_u16
    {
      let data = if num != 1 {
        json!({ "start": num })
      } else {
        json!({})
      };
      return Some(convert("numbered_list", data, strip_u16));
    }

    // Code fence: ``` or ~~~
    if caret == 3 && (text == "```" || text == "~~~") {
      return Some(convert("code_block", json!({}), 3));
    }

    // Display math block: $$...$$
    if caret == total_u16 && total_u16 > 4 && text.starts_with("$$") && text.ends_with("$$") {
      let inner = &text[2..text.len() - 2];
      let trimmed = inner.trim();
      if !trimmed.is_empty() && !trimmed.contains("$$") {
        return Some(InputRuleMatch::ConvertToMathBlock {
          source: trimmed.to_string(),
        });
      }
    }

    // Divider: --- / *** / ___
    if caret == 3 && (text == "---" || text == "***" || text == "___") {
      return Some(InputRuleMatch::InsertDivider);
    }
  } else if block.kind == "bulleted_list" {
    // Chained todo rule after `- ` converted to bulleted_list: `[ ] ` / `[x] `.
    for (pat, checked) in [
      ("[ ] ", false),
      ("[] ", false),
      ("[x] ", true),
      ("[X] ", true),
    ] {
      let pat_u16 = utf16_len(pat);
      if caret == pat_u16 && text.starts_with(pat) {
        let mut data = serde_json::Map::new();
        data.insert("checked".to_string(), Value::Bool(checked));
        if block.indent() > 0 {
          data.insert("indent".to_string(), json!(block.indent()));
        }
        return Some(convert("todo", Value::Object(data), pat_u16));
      }
    }
  }

  // Inline rules (never inside code blocks).
  if block.kind != "code_block" {
    return match_inline_rule(block, caret);
  }
  None
}

fn match_numbered_prefix(text: &str) -> Option<(u64, u32)> {
  let bytes = text.as_bytes();
  let mut i = 0usize;
  while i < bytes.len() && bytes[i].is_ascii_digit() {
    i += 1;
  }
  if i == 0 || i > 9 || i + 1 >= bytes.len() {
    return None;
  }
  if bytes[i] == b'.' && bytes[i + 1] == b' ' {
    let num: u64 = text[..i].parse().ok()?;
    Some((num, (i + 2) as u32))
  } else {
    None
  }
}

fn match_inline_rule(block: &EditorBlock, caret: u32) -> Option<InputRuleMatch> {
  let text = block.text.as_str();
  let total_u16 = utf16_len(text);
  let before = utf16_slice(text, 0, caret);

  let mut matched: Option<(u32, u32, u32, &str, Option<String>)> = None;

  // 1. Link: `[label](url)` when `)` is at `caret - 1`.
  if before.ends_with(')')
    && let Some((m_start, i_start, i_end, href)) = match_inline_link(before)
  {
    matched = Some((m_start, i_start, i_end, "link", Some(href)));
  }

  // 2. Two-char wrappers: `**bold**`, `~~strike~~`.
  if matched.is_none() && caret >= 4 {
    for (delim, kind) in [("**", "bold"), ("~~", "strike")] {
      if let Some(prefix_without_close) = before.strip_suffix(delim) {
        if let Some(open_byte) = prefix_without_close.rfind(delim) {
          let inner = &prefix_without_close[open_byte + delim.len()..];
          if !inner.is_empty() && !inner.trim().is_empty() {
            let match_start = utf16_len(&before[..open_byte]);
            let inner_start = match_start + 2;
            let inner_end = caret - 2;
            matched = Some((match_start, inner_start, inner_end, kind, None));
            break;
          }
        }
      }
    }
  }

  // 3. One-char wrappers: `` `code` ``, `*italic*`, `_italic_`, `~strike~`, `$math$`.
  if matched.is_none() && caret >= 3 {
    for (ch, kind) in [
      ('`', "code"),
      ('*', "italic"),
      ('_', "italic"),
      ('~', "strike"),
      ('$', "math"),
    ] {
      if !before.ends_with(ch) {
        continue;
      }
      let without_close = &before[..before.len() - ch.len_utf8()];
      // Do not match the second char of a doubled marker (`**`, `__`, `~~`, `$$`, `` `` ``).
      if without_close.ends_with(ch) {
        continue;
      }
      // For `$math$`, ignore escaped `\$` as closing delimiter.
      if ch == '$' && without_close.ends_with('\\') {
        continue;
      }
      let Some(open_byte) = without_close.rfind(ch) else {
        continue;
      };
      // Skip if the opener is preceded by the same char (part of a two-char wrapper).
      if before[..open_byte].ends_with(ch) {
        continue;
      }
      if ch == '$' && before[..open_byte].ends_with('\\') {
        continue;
      }
      let inner = &without_close[open_byte + ch.len_utf8()..];
      if inner.is_empty() || inner.trim().is_empty() {
        continue;
      }
      let match_start = utf16_len(&before[..open_byte]);
      let inner_start = match_start + 1;
      let inner_end = caret - 1;
      matched = Some((match_start, inner_start, inner_end, kind, None));
      break;
    }
  }

  let (match_start, inner_start, inner_end, kind, href) = matched?;
  let inner = utf16_slice(text, inner_start, inner_end);
  let inner_u16 = utf16_len(inner);

  let mut new_text = String::with_capacity(text.len());
  new_text.push_str(utf16_slice(text, 0, match_start));
  new_text.push_str(inner);
  new_text.push_str(utf16_slice(text, caret, total_u16));
  let new_len = utf16_len(&new_text);

  // Shift existing marks for the two marker deletions (right side first so left
  // offsets stay valid), then add the new mark over the converted run.
  let mut marks = block.marks();
  marks = shift_marks(
    &marks,
    inner_end,
    caret,
    inner_end as i64 - caret as i64,
    total_u16 - (caret - inner_end),
    true,
  );
  marks = shift_marks(
    &marks,
    match_start,
    inner_start,
    match_start as i64 - inner_start as i64,
    new_len,
    true,
  );
  let mark_end = match_start + inner_u16;
  marks = apply_mark(&marks, match_start, mark_end, kind, href.as_deref(), None);

  Some(InputRuleMatch::InlineMark {
    text: new_text,
    marks,
    new_caret: mark_end,
  })
}

/// Match trailing `[label](url)` at the end of `before` (which ends with `)`).
/// Returns `(match_start_u16, inner_start_u16, inner_end_u16, href)`.
fn match_inline_link(before: &str) -> Option<(u32, u32, u32, String)> {
  let body = before.strip_suffix(')')?;
  let open_paren = body.rfind("](")?;
  let url = &body[open_paren + 2..];
  if url.is_empty() || url.chars().any(|c| c.is_whitespace() || c == ')') {
    return None;
  }
  let before_close_bracket = &body[..open_paren];
  let open_bracket = before_close_bracket.rfind('[')?;
  let label = &before_close_bracket[open_bracket + 1..];
  if label.is_empty() || label.contains(']') {
    return None;
  }
  let match_start = utf16_len(&before[..open_bracket]);
  let inner_start = match_start + 1;
  let inner_end = inner_start + utf16_len(label);
  Some((match_start, inner_start, inner_end, url.to_string()))
}

/// Detect an active `/query` slash-menu trigger ending at `caret` in `block`.
/// Returns `(start_u16, end_u16, query)` where `start_u16` is the `/` position.
pub fn detect_slash_trigger(block: &EditorBlock, caret: u32) -> Option<(u32, u32, String)> {
  if block.is_atomic() || block.is_code() {
    return None;
  }
  let before = utf16_slice(&block.text, 0, caret);
  let slash_byte = before.rfind('/')?;
  // Must be at line start or immediately after whitespace.
  if slash_byte > 0 {
    let prev = before[..slash_byte].chars().next_back()?;
    if !prev.is_whitespace() {
      return None;
    }
  }
  let query = &before[slash_byte + 1..];
  if query.chars().any(char::is_whitespace) {
    return None;
  }
  let start = utf16_len(&before[..slash_byte]);
  Some((start, caret, query.to_string()))
}

/// Detect an active `[[query` page-link trigger ending at `caret` in `block`.
/// Returns `(start_u16, end_u16, query)` where `start_u16` is the first `[`.
pub fn detect_page_link_trigger(block: &EditorBlock, caret: u32) -> Option<(u32, u32, String)> {
  if block.is_atomic() || block.is_code() {
    return None;
  }
  let before = utf16_slice(&block.text, 0, caret);
  let open_byte = before.rfind("[[")?;
  let query = &before[open_byte + 2..];
  if query.contains(']') || query.contains('\n') {
    return None;
  }
  let start = utf16_len(&before[..open_byte]);
  Some((start, caret, query.to_string()))
}
