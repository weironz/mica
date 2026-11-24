//! Paste-only heuristics and structured Markdown/HTML/plain clipboard helpers.
//!
//! All Markdown parsing, Markdown serialization, HTML conversion, and language
//! detection delegate directly to `mica_md` (`md-core`). Only paste-only
//! repair heuristics (ported from `paste_repair.dart`, decision D7) live here.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value, json};

use crate::marks::{EditorMark, clip_marks, marks_to_json};
use crate::model::{BlockSpec, ClipboardPayload, EditorBlock};
use crate::selection::{DocSelection, NormalizedSelection};
use crate::table::TableModel;
use crate::utf16::{utf16_len, utf16_slice};

/// Normalize `\r\n` and `\r` to `\n` so the document model stays `\n`-only.
pub fn normalize_line_endings(s: &str) -> String {
  s.replace("\r\n", "\n").replace('\r', "\n")
}

struct FenceHeader<'a> {
  indent: &'a str,
  ch: char,
  len: usize,
  rest: &'a str,
}

fn parse_fence_line(line: &str) -> Option<FenceHeader<'_>> {
  let bytes = line.as_bytes();
  let mut indent_len = 0usize;
  while indent_len < 3 && indent_len < bytes.len() && bytes[indent_len] == b' ' {
    indent_len += 1;
  }
  let after_indent = &line[indent_len..];
  let first = after_indent.chars().next()?;
  if first != '`' && first != '~' {
    return None;
  }
  let fence_len = after_indent.chars().take_while(|&c| c == first).count();
  if fence_len < 3 {
    return None;
  }
  let rest = &after_indent[fence_len..];
  // Info strings on backtick fences cannot contain backticks (CommonMark).
  if first == '`' && rest.contains('`') {
    return None;
  }
  Some(FenceHeader {
    indent: &line[..indent_len],
    ch: first,
    len: fence_len,
    rest,
  })
}

fn parse_fence_label_line(line: &str) -> Option<(&str, &str, &str)> {
  let hdr = parse_fence_line(line)?;
  let trimmed = hdr.rest.trim();
  if trimmed
    .chars()
    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '#' | '.' | '-'))
  {
    let fence_end = hdr.indent.len() + hdr.len;
    Some((hdr.indent, &line[hdr.indent.len()..fence_end], trimmed))
  } else {
    None
  }
}

/// Correct a pasted code fence whose language label the code structurally
/// contradicts (e.g. Python labelled ```` ```bash ```` by highlight.js).
///
/// Paste-only heuristic ported from `paste_repair.dart::retagMislabeledFences`;
/// delegates language canonicalization and strong signature detection to
/// `mica_md::highlight`.
pub fn retag_mislabeled_fences(md: &str) -> String {
  let mut lines: Vec<String> = md.split('\n').map(String::from).collect();
  let mut i = 0usize;
  while i < lines.len() {
    let Some((indent, fence, label)) = parse_fence_label_line(&lines[i])
      .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
    else {
      i += 1;
      continue;
    };
    let fence_ch = fence.chars().next().unwrap_or('`');
    let fence_len = fence.len();
    let mut j = i + 1;
    let mut body = Vec::new();
    while j < lines.len() {
      if let Some(c) = parse_fence_line(&lines[j])
        && c.rest.trim().is_empty()
        && c.ch == fence_ch
        && c.len >= fence_len
      {
        break;
      }
      body.push(lines[j].as_str());
      j += 1;
    }
    if !label.is_empty() {
      let claimed = mica_md::highlight::canonical_code_language(&label);
      if let Some(strong) = mica_md::highlight::strong_language_signature(&body.join("\n"))
        && strong != claimed
      {
        lines[i] = format!("{indent}{fence}{strong}");
      }
    }
    i = j + 1;
  }
  lines.join("\n")
}

/// Unwrap the LLM double-fence wrapper (a bare fence immediately followed by
/// an equal-char, equal-length fence). Ported from
/// `paste_repair.dart::unwrapNestedFences`.
pub fn unwrap_nested_fences(md: &str) -> String {
  let lines: Vec<&str> = md.split('\n').collect();
  let close_of = |line: &str| -> Option<(char, usize)> {
    let hdr = parse_fence_line(line)?;
    hdr.rest.trim().is_empty().then_some((hdr.ch, hdr.len))
  };

  let mut drop_indices: HashSet<usize> = HashSet::new();
  let mut open: Option<(char, usize)> = None;
  let mut i = 0usize;
  while i < lines.len() {
    let Some(hdr) = parse_fence_line(lines[i]) else {
      i += 1;
      continue;
    };
    let bare = hdr.rest.trim().is_empty();
    if let Some((och, olen)) = open {
      if bare && hdr.ch == och && hdr.len >= olen {
        open = None;
      }
      i += 1;
      continue;
    }
    if bare
      && i + 1 < lines.len()
      && let Some(inner) = parse_fence_line(lines[i + 1])
      && inner.ch == hdr.ch
      && inner.len == hdr.len
    {
      let mut close = None;
      for (j, line) in lines.iter().enumerate().skip(i + 2) {
        if let Some((fch, flen)) = close_of(line)
          && fch == hdr.ch
          && flen >= inner.len
        {
          close = Some(j);
          break;
        }
      }
      if let Some(close_idx) = close {
        drop_indices.insert(i);
        for (j, line) in lines.iter().enumerate().skip(close_idx + 1) {
          if line.trim().is_empty() {
            continue;
          }
          if let Some((fch, flen)) = close_of(line)
            && fch == hdr.ch
            && flen == hdr.len
          {
            drop_indices.insert(j);
          }
          break;
        }
        i = close_idx + 1;
        continue;
      }
    }
    open = Some((hdr.ch, hdr.len));
    i += 1;
  }
  if drop_indices.is_empty() {
    return md.to_string();
  }
  lines
    .into_iter()
    .enumerate()
    .filter_map(|(k, l)| (!drop_indices.contains(&k)).then_some(l))
    .collect::<Vec<_>>()
    .join("\n")
}

/// Whether a single pasted plain-text line should keep the inline marks parsed
/// by `mica_md::parse_inline` (gh#185; ported from `paste_repair.dart`).
pub fn single_line_paste_keeps_marks<'a>(mark_kinds: impl IntoIterator<Item = &'a str>) -> bool {
  let mut any = false;
  for kind in mark_kinds {
    if !matches!(kind, "code" | "bold" | "strike" | "link" | "math") {
      return false;
    }
    any = true;
  }
  any
}

/// Parse a single plain-text pasted line into `(text, marks)` via `mica_md`
/// when trusted by [`single_line_paste_keeps_marks`], falling back to inline
/// math (`$…$` / `\(…\)`). Returns `None` to paste literally.
pub fn single_line_paste_inline(line: &str) -> Option<(String, Vec<EditorMark>)> {
  let parsed = mica_md::parse_inline(line);
  if single_line_paste_keeps_marks(parsed.marks.iter().map(|m| m.kind.as_str())) {
    return Some((
      parsed.text,
      parsed.marks.into_iter().map(EditorMark::from).collect(),
    ));
  }
  let math = mica_md::parse_inline_math(line);
  if !math.marks.is_empty() {
    return Some((
      math.text,
      math.marks.into_iter().map(EditorMark::from).collect(),
    ));
  }
  None
}

/// Bare LaTeX source when a pasted single line is entirely a display formula
/// (`$$…$$` or `\[…\]`), or `None`.
pub fn pasted_formula_source(line: &str) -> Option<String> {
  let trimmed = line.trim();
  let inner = trimmed
    .strip_prefix("$$")
    .and_then(|s| s.strip_suffix("$$"))
    .or_else(|| {
      trimmed
        .strip_prefix(r"\[")
        .and_then(|s| s.strip_suffix(r"\]"))
    })?;
  let src = inner.trim();
  if src.is_empty() || src.contains('\n') {
    None
  } else {
    Some(src.to_string())
  }
}

/// Whether `text` is a bare `http://` or `https://` URL (no whitespace).
pub fn looks_like_url(text: &str) -> bool {
  if text.is_empty() || text.chars().any(char::is_whitespace) {
    return false;
  }
  let Some(rest) = text
    .strip_prefix("https://")
    .or_else(|| text.strip_prefix("http://"))
  else {
    return false;
  };
  !rest.is_empty() && !rest.starts_with('/')
}

/// Whether `text` is a bare image URL ending in a common image extension.
pub fn looks_like_image_url(text: &str) -> bool {
  if !looks_like_url(text) {
    return false;
  }
  let path = text.split('?').next().unwrap_or(text);
  let lower = path.to_ascii_lowercase();
  [
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".avif",
  ]
  .iter()
  .any(|ext| lower.ends_with(ext))
}

/// Whether a single-block rich paste should weave inline into the target block
/// (mirrors `editor_hooks.dart::pasteMergesInline`).
pub fn paste_merges_inline(
  spec_kind: &str,
  target_kind: &str,
  target_len_u16: u32,
  sel_from: u32,
  sel_to: u32,
) -> bool {
  if !EditorBlock::is_inline_text_kind(spec_kind) {
    return false;
  }
  if spec_kind == "paragraph" {
    return true;
  }
  let emptied = sel_to.saturating_sub(sel_from) >= target_len_u16;
  !emptied || target_kind != "paragraph"
}

/// Convert a parsed HTML DOM tree to Markdown via `mica_md::html_to_markdown`.
pub fn html_nodes_to_markdown(body: &mica_md::HtmlNode) -> String {
  mica_md::html_to_markdown(body)
}

/// Parse a pasted Markdown fragment into flat [`BlockSpec`]s via `mica_md`.
pub fn parse_paste_blocks(markdown: &str) -> Vec<BlockSpec> {
  mica_md::parse_body(markdown, mica_md::ParseMode::Fragment)
    .into_iter()
    .map(BlockSpec::from)
    .collect()
}

/// Language label to emit on a code block leaving Mica (clipboard / interchange).
/// Mirrors `controller.dart::_copyLanguage`.
pub fn copy_language(block: &EditorBlock) -> String {
  let pinned = block
    .data_map()
    .get("language")
    .and_then(Value::as_str)
    .map(str::trim)
    .unwrap_or("");
  if !pinned.is_empty() && pinned != "auto" {
    return pinned.to_string();
  }
  let detected = mica_md::highlight::detect_code_language(&block.text);
  if detected == "plaintext" {
    String::new()
  } else {
    detected
  }
}

/// Visible ordinal of a `numbered_list` item, walking back to the head of its
/// run (same indent / quoteDepth / `li`), skipping over deeper nested list
/// children that belong to preceding sibling items.
pub fn visible_ordinal(blocks: &[EditorBlock], index: usize) -> Option<u64> {
  let node = blocks.get(index)?;
  if node.kind != "numbered_list" {
    return None;
  }
  let target_indent = node.indent();
  let target_quote = node.quote_depth();
  let target_li = node.li_level();
  let restarts = |at: &EditorBlock| {
    let m = at.data_map();
    m.contains_key("start") || m.contains_key("marker")
  };

  let mut head = index;
  let mut siblings_before: u64 = 0;
  let mut cursor = index;
  while cursor > 0 && !restarts(&blocks[head]) {
    let prev = &blocks[cursor - 1];
    if prev.quote_depth() != target_quote {
      break;
    }
    // Deeper nested children belong to the preceding sibling item — skip them.
    // A continuation block (`data.li = Some(l)`) belongs to an item at indent
    // `l`: skip when `l >= target_indent`, break when `l < target_indent` (an
    // outer item's continuation terminates an inner sub-list).
    if let Some(li) = prev.li_level() {
      if li >= target_indent {
        cursor -= 1;
        continue;
      }
      break;
    }
    if prev.indent() > target_indent {
      cursor -= 1;
      continue;
    }
    if prev.kind == "numbered_list"
      && prev.indent() == target_indent
      && prev.li_level() == target_li
    {
      siblings_before += 1;
      head = cursor - 1;
      cursor -= 1;
    } else {
      break;
    }
  }
  let base = blocks[head]
    .data_map()
    .get("start")
    .and_then(Value::as_u64)
    .unwrap_or(1);
  let ordinal = base.saturating_add(siblings_before);
  Some(if ordinal <= 999_999_999 { ordinal } else { 1 })
}

/// Selection slices `(block_index, from_u16, to_u16)` for copy/cut.
pub fn selection_slices(
  blocks: &[EditorBlock],
  selection: &DocSelection,
) -> Vec<(usize, u32, u32)> {
  match selection.normalized() {
    NormalizedSelection::BlockAtom { block_index } => {
      if let Some(b) = blocks.get(block_index) {
        vec![(block_index, 0, b.utf16_len())]
      } else {
        Vec::new()
      }
    }
    NormalizedSelection::TableArea { block_index, .. } => {
      if block_index < blocks.len() {
        vec![(block_index, 0, 0)]
      } else {
        Vec::new()
      }
    }
    NormalizedSelection::Text { start, end } => {
      if start.node >= blocks.len() || end.node >= blocks.len() {
        return Vec::new();
      }
      if start == end {
        if blocks[start.node].is_atomic() {
          return vec![(start.node, 0, blocks[start.node].utf16_len())];
        }
        return Vec::new();
      }
      if start.node == end.node {
        return vec![(start.node, start.offset, end.offset)];
      }
      let mut out = vec![(start.node, start.offset, blocks[start.node].utf16_len())];
      for (i, b) in blocks
        .iter()
        .enumerate()
        .take(end.node + 1)
        .skip(start.node + 1)
      {
        let to = if i == end.node {
          end.offset
        } else {
          b.utf16_len()
        };
        out.push((i, 0, to));
      }
      out
    }
  }
}

/// Convert the current selection into clipped `mica_md::Block`s for Markdown &
/// Clipboard HTML serialization (mirrors `controller.dart::selectionBlocks`).
pub fn selection_md_blocks(
  blocks: &[EditorBlock],
  selection: &DocSelection,
) -> Vec<mica_md::Block> {
  let mut out = Vec::new();
  for (idx, sl_from, sl_to) in selection_slices(blocks, selection) {
    let node = &blocks[idx];
    let len = node.utf16_len();
    let from = if node.is_atomic() {
      0
    } else {
      sl_from.min(len)
    };
    let to = if node.is_atomic() {
      len
    } else {
      sl_to.clamp(from, len)
    };
    let full = from == 0 && to == len;

    let mut kind = node.kind.clone();
    let mut data_map = if full {
      node.data_map().clone()
    } else {
      if kind != "code_block" {
        kind = "paragraph".to_string();
      }
      Map::new()
    };
    if node.kind == "code_block" {
      let lang = copy_language(node);
      if lang.is_empty() {
        data_map.remove("language");
      } else {
        data_map.insert("language".to_string(), Value::String(lang));
      }
    }
    if node.kind == "numbered_list"
      && let Some(ordinal) = visible_ordinal(blocks, idx)
    {
      data_map.insert("start".to_string(), json!(ordinal));
    }
    let clipped = clip_marks(&node.marks(), from, to);
    if clipped.is_empty() {
      data_map.remove("marks");
    } else {
      data_map.insert("marks".to_string(), marks_to_json(&clipped));
    }
    out.push(mica_md::Block {
      id: String::new(),
      kind,
      text: utf16_slice(&node.text, from, to).to_string(),
      data: if data_map.is_empty() {
        Value::Null
      } else {
        Value::Object(data_map)
      },
      children: Vec::new(),
    });
  }
  out
}

/// Literal plain text for `text/plain` clipboard flavor (mirrors
/// `controller.dart::selectionClipboardText`).
pub fn selection_clipboard_text(
  blocks: &[EditorBlock],
  selection: &DocSelection,
  image_urls: &BTreeMap<String, String>,
) -> String {
  if let NormalizedSelection::TableArea {
    block_index,
    r0,
    c0,
    r1,
    c1,
    ..
  } = selection.normalized()
    && let Some(block) = blocks.get(block_index)
    && block.kind == "table"
  {
    return TableModel::from_block_data(&block.data)
      .copy_area(r0, c0, r1, c1)
      .plain;
  }

  let mut parts = Vec::new();
  for (idx, sl_from, sl_to) in selection_slices(blocks, selection) {
    let node = &blocks[idx];
    match node.kind.as_str() {
      "table" => {
        let table = TableModel::from_block_data(&node.data);
        parts.push(table.copy_whole().plain);
      }
      "image" => {
        let file_id = node.data_map().get("file_id").and_then(Value::as_str);
        let text = file_id
          .and_then(|fid| image_urls.get(fid).map(String::as_str))
          .or_else(|| node.data_map().get("url").and_then(Value::as_str))
          .or_else(|| node.data_map().get("name").and_then(Value::as_str))
          .unwrap_or(&node.text);
        parts.push(text.to_string());
      }
      "divider" => parts.push("---".to_string()),
      _ => {
        let len = node.utf16_len();
        let from = sl_from.min(len);
        let to = sl_to.clamp(from, len);
        if node.is_code() || node.kind == "math_block" {
          parts.push(utf16_slice(&node.text, from, to).to_string());
        } else {
          // Strip stored prose hard-break marker `\` before `\n` unless inside
          // a literal `code` / `math` / `html` mark.
          let marks = node.marks();
          let chars: Vec<(u32, char)> = {
            let mut v = Vec::new();
            let mut u = 0u32;
            for ch in node.text.chars() {
              v.push((u, ch));
              u += ch.len_utf16() as u32;
            }
            v
          };
          let mut buf = String::new();
          for k in 0..chars.len() {
            let (u16_pos, ch) = chars[k];
            if u16_pos < from || u16_pos >= to {
              continue;
            }
            let hard_break = ch == '\\'
              && chars
                .get(k + 1)
                .is_some_and(|&(_, next_ch)| next_ch == '\n')
              && !marks.iter().any(|m| {
                u16_pos >= m.start
                  && u16_pos < m.end
                  && matches!(m.kind.as_str(), "code" | "math" | "html")
              });
            if !hard_break {
              buf.push(ch);
            }
          }
          parts.push(buf);
        }
      }
    }
  }
  parts.join("\n\n")
}

/// Build the full clipboard payload (`plain`, `markdown`, `html`) for `selection`.
pub fn build_clipboard_payload(
  blocks: &[EditorBlock],
  selection: &DocSelection,
  image_urls: &BTreeMap<String, String>,
) -> Option<ClipboardPayload> {
  if let NormalizedSelection::TableArea {
    block_index,
    r0,
    c0,
    r1,
    c1,
    ..
  } = selection.normalized()
    && let Some(block) = blocks.get(block_index)
    && block.kind == "table"
  {
    let payload = TableModel::from_block_data(&block.data).copy_area(r0, c0, r1, c1);
    return (!payload.plain.is_empty() || !payload.html.is_empty()).then_some(payload);
  }

  let md_blocks = selection_md_blocks(blocks, selection);
  if md_blocks.is_empty() {
    return None;
  }
  let plain = selection_clipboard_text(blocks, selection, image_urls);
  let markdown = mica_md::serialize_body(
    &md_blocks,
    &mica_md::SerializeOptions {
      assets: image_urls.clone(),
    },
  );
  let html = mica_md::render_html_body(
    &md_blocks,
    &mica_md::HtmlOptions {
      image_srcs: image_urls.clone(),
      profile: mica_md::HtmlProfile::Clipboard,
    },
  );
  if plain.is_empty() && markdown.is_empty() && html.is_empty() {
    None
  } else {
    Some(ClipboardPayload {
      plain,
      markdown,
      html,
    })
  }
}

/// Convert all document blocks to `mica_md::Block`s (preserving verbatim code
/// language for document export round-trip; `copy_language` is clipboard-only).
pub fn document_md_blocks(blocks: &[EditorBlock]) -> Vec<mica_md::Block> {
  blocks.iter().map(EditorBlock::to_md_block).collect()
}

/// Helper: check UTF-16 length of a string slice when calculating paste merges.
pub fn text_u16_len(s: &str) -> u32 {
  utf16_len(s)
}
