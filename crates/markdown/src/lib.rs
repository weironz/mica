//! Mica's Markdown engine — the block model ("AST") plus parsing and
//! rendering between it and Markdown/HTML. Pure and I/O-free; document
//! *operations* (insert/update/move) live in `mica-app-core`, archive-level
//! interchange in `mica-interchange`. See docs/architecture.md.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentSnapshotPayload {
  pub schema_version: i32,
  pub root_block_id: String,
  pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
  pub id: String,
  #[serde(rename = "type")]
  pub kind: String,
  #[serde(default)]
  pub text: String,
  /// Block-specific attributes (image `url`, heading `level`, todo `checked`).
  /// Defaults to `null` and is omitted from output when empty for compactness.
  #[serde(default, skip_serializing_if = "Value::is_null")]
  pub data: Value,
  #[serde(default)]
  pub children: Vec<String>,
}


#[derive(Debug, thiserror::Error)]
pub enum DocumentOperationError {
  #[error("unsupported document schema version: {0}")]
  UnsupportedSchemaVersion(i32),

  #[error("block not found: {0}")]
  BlockNotFound(String),

  #[error("block already exists: {0}")]
  BlockAlreadyExists(String),

  #[error("root block cannot be deleted or moved")]
  CannotMoveRoot,

  #[error("parent block cannot be a descendant of the moved block")]
  ParentIsDescendant,

  #[error("block type is required")]
  EmptyBlockType,
}

pub type DocumentOperationResult<T> = Result<T, DocumentOperationError>;



pub fn payload_from_value(value: Value) -> DocumentOperationResult<DocumentSnapshotPayload> {
  let payload = serde_json::from_value::<DocumentSnapshotPayload>(value)
    .map_err(|_| DocumentOperationError::UnsupportedSchemaVersion(0))?;
  if payload.schema_version != 1 {
    return Err(DocumentOperationError::UnsupportedSchemaVersion(
      payload.schema_version,
    ));
  }

  Ok(payload)
}


pub fn export_markdown(snapshot: &DocumentSnapshotPayload) -> DocumentOperationResult<String> {
  export_markdown_with_assets(snapshot, &BTreeMap::new())
}

/// Like [export_markdown] but rewrites each image block whose `file_id` is in
/// [images] to that asset path (e.g. `assets/photo.png`) — used for ZIP export
/// where image bytes are bundled alongside the Markdown.
pub fn export_markdown_with_assets(
  snapshot: &DocumentSnapshotPayload,
  images: &BTreeMap<String, String>,
) -> DocumentOperationResult<String> {
  if snapshot.schema_version != 1 {
    return Err(DocumentOperationError::UnsupportedSchemaVersion(
      snapshot.schema_version,
    ));
  }

  let mut lines = Vec::new();
  let root_index = block_index(snapshot, &snapshot.root_block_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(snapshot.root_block_id.clone()))?;
  let root = &snapshot.blocks[root_index];
  if !root.text.trim().is_empty() {
    append_markdown_block_content(root, 0, &mut lines, images);
  }

  append_markdown_children(snapshot, &snapshot.root_block_id, 0, &mut lines, images)?;

  while lines.last().is_some_and(|line: &String| line.is_empty()) {
    lines.pop();
  }

  Ok(lines.join("\n"))
}

pub fn export_html(snapshot: &DocumentSnapshotPayload) -> DocumentOperationResult<String> {
  if snapshot.schema_version != 1 {
    return Err(DocumentOperationError::UnsupportedSchemaVersion(
      snapshot.schema_version,
    ));
  }

  let mut html = String::new();
  let root_index = block_index(snapshot, &snapshot.root_block_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(snapshot.root_block_id.clone()))?;
  let root = &snapshot.blocks[root_index];
  if !root.text.trim().is_empty() {
    html.push_str(&format!("<p>{}</p>\n", escape_html(root.text.trim())));
  }

  append_html_children(snapshot, &snapshot.root_block_id, &mut html)?;

  Ok(html.trim_end().to_string())
}

/// Parse Markdown into a flat document snapshot. Each line maps to a top-level
/// block; structural nesting beyond fenced code is intentionally out of scope
/// for the MVP importer.
pub fn import_markdown(markdown: &str, root_block_id: &str) -> DocumentSnapshotPayload {
  let raw_lines: Vec<&str> = markdown.lines().collect();
  let mut blocks: Vec<Block> = Vec::new();
  let mut root_children: Vec<String> = Vec::new();

  // Pass 1: link reference definitions (`[label]: dest "title"`) — they can
  // sit anywhere, resolve case-insensitively, and vanish from the output.
  let mut defs = RefDefs::new();
  let mut def_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();
  for (idx, raw) in raw_lines.iter().enumerate() {
    if let Some((label, dest, title)) = parse_ref_definition(raw) {
      defs.entry(normalize_label(&label)).or_insert((dest, title));
      def_lines.insert(idx);
    }
  }

  let mut index = 0;
  // Leading-width stack mapping source indentation columns to nesting levels
  // (tolerates 2/3/4-space styles; tabs count as 4). Only list/todo items
  // nest; any other block resets the stack.
  let mut list_stack: Vec<usize> = Vec::new();
  while index < raw_lines.len() {
    if def_lines.contains(&index) {
      index += 1;
      continue;
    }
    let line = raw_lines[index].trim_end();
    let content = line.trim_start();

    if content.is_empty() {
      index += 1;
      continue; // blank lines between items keep the list context
    }
    let mut col: usize = 0;
    for c in line[..line.len() - content.len()].chars() {
      col = if c == '\t' { (col / 4 + 1) * 4 } else { col + 1 };
    }

    if let Some(language) = content.strip_prefix("```") {
      list_stack.clear();
      let language = language.trim().to_string();
      let mut code_lines = Vec::new();
      index += 1;
      while index < raw_lines.len() {
        if raw_lines[index].trim_start().starts_with("```") {
          index += 1;
          break;
        }
        code_lines.push(raw_lines[index].to_string());
        index += 1;
      }
      let data = if language.is_empty() {
        Value::Null
      } else {
        json!({ "language": language })
      };
      push_block(
        &mut blocks,
        &mut root_children,
        "code_block",
        code_lines.join("\n"),
        data,
      );
      continue;
    }

    // GFM pipe table: a `|`-row followed by a `| --- |` separator.
    if content.contains('|')
      && index + 1 < raw_lines.len()
      && is_table_separator(raw_lines[index + 1].trim())
    {
      list_stack.clear();
      let mut rows: Vec<Vec<String>> = vec![split_table_row(content)];
      index += 2;
      while index < raw_lines.len() {
        let row = raw_lines[index].trim();
        if row.is_empty() || !row.contains('|') {
          break;
        }
        rows.push(split_table_row(row));
        index += 1;
      }
      let width = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
      for row in rows.iter_mut() {
        while row.len() < width {
          row.push(String::new());
        }
      }
      push_block(
        &mut blocks,
        &mut root_children,
        "table",
        String::new(),
        // Same shape the client editor uses (render defaults included).
        json!({
          "rows": rows,
          "header": true,
          "align": "left",
          "widths": vec![1.0f64; width],
        }),
      );
      continue;
    }

    // Indented code block: 4+ columns at top level (inside a list that
    // indentation means nesting instead, handled by the stack above).
    if list_stack.is_empty() && col >= 4 {
      let mut code_lines: Vec<String> = Vec::new();
      let mut pending_blanks = 0usize;
      while index < raw_lines.len() {
        let l = raw_lines[index].trim_end();
        if l.trim().is_empty() {
          pending_blanks += 1;
          index += 1;
          continue;
        }
        let c = l.trim_start();
        let mut lcol = 0usize;
        for ch in l[..l.len() - c.len()].chars() {
          lcol = if ch == '\t' { (lcol / 4 + 1) * 4 } else { lcol + 1 };
        }
        if lcol < 4 {
          break;
        }
        for _ in 0..pending_blanks {
          code_lines.push(String::new());
        }
        pending_blanks = 0;
        code_lines.push(deindent_columns(l, 4));
        index += 1;
      }
      push_block(
        &mut blocks,
        &mut root_children,
        "code_block",
        code_lines.join("\n"),
        Value::Null,
      );
      continue;
    }

    // Horizontal rule (`---`, `***`, `___`) → divider block.
    if is_divider(content) {
      list_stack.clear();
      push_block(&mut blocks, &mut root_children, "divider", String::new(), Value::Null);
      index += 1;
      continue;
    }

    let (kind, text, data) = classify_markdown_line(content);

    // Setext heading: a paragraph line whose NEXT line underlines it with
    // `===` (h1) or `---` (h2).
    if kind == "paragraph"
      && index + 1 < raw_lines.len()
      && let Some(level) = setext_level(raw_lines[index + 1])
    {
      let (text, data) = apply_inline_marks(text, json!({ "level": level }), &defs);
      push_block(&mut blocks, &mut root_children, "heading", text, data);
      list_stack.clear();
      index += 2;
      continue;
    }
    let (text, mut data) = if kind == "image" {
      (text, data)
    } else {
      apply_inline_marks(text, data, &defs)
    };

    // Nesting level for list items from the indentation stack.
    if matches!(kind, "bulleted_list" | "numbered_list" | "todo") {
      while list_stack.last().is_some_and(|&top| col < top) {
        list_stack.pop();
      }
      if list_stack.last().is_none_or(|&top| col > top) {
        list_stack.push(col);
      }
      let level = list_stack.len().saturating_sub(1);
      if level > 0 {
        match &mut data {
          Value::Object(map) => {
            map.insert("indent".into(), json!(level));
          }
          other => *other = json!({ "indent": level }),
        }
      }
    } else {
      list_stack.clear();
    }

    push_block(&mut blocks, &mut root_children, kind, text, data);
    index += 1;
  }

  let root = Block {
    id: root_block_id.to_string(),
    kind: "paragraph".to_string(),
    text: String::new(),
    data: Value::Null,
    children: root_children,
  };
  let mut all_blocks = Vec::with_capacity(blocks.len() + 1);
  all_blocks.push(root);
  all_blocks.extend(blocks);

  DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id: root_block_id.to_string(),
    blocks: all_blocks,
  }
}

/// A Markdown thematic break: 3+ of the same `-`, `*`, or `_` (already trimmed).
fn is_divider(content: &str) -> bool {
  // Spec thematic break: 3+ of the same -/*/_ with optional spaces/tabs
  // between (`- - -`, `_  _  _`).
  let mut marker = 0u8;
  let mut count = 0usize;
  for b in content.bytes() {
    match b {
      b' ' | b'\t' => {}
      b'-' | b'*' | b'_' => {
        if marker == 0 {
          marker = b;
        } else if b != marker {
          return false;
        }
        count += 1;
      }
      _ => return false,
    }
  }
  count >= 3
}

/// Strip [columns] of leading indentation, treating tabs as 4-column stops
/// (a tab that overshoots emits the remainder as spaces).
fn deindent_columns(line: &str, columns: usize) -> String {
  let mut col = 0usize;
  for (i, c) in line.char_indices() {
    match c {
      ' ' => col += 1,
      '\t' => col = (col / 4 + 1) * 4,
      _ => return line[i..].to_string(),
    }
    if col >= columns {
      let mut rest = " ".repeat(col - columns);
      rest.push_str(&line[i + c.len_utf8()..]);
      return rest;
    }
  }
  String::new()
}

/// `===`/`---` underline (≤3 leading spaces) → setext heading level, if any.
fn setext_level(line: &str) -> Option<usize> {
  let lead = line.len() - line.trim_start().len();
  if lead > 3 {
    return None;
  }
  let t = line.trim();
  if t.is_empty() {
    return None;
  }
  if t.bytes().all(|b| b == b'=') {
    return Some(1);
  }
  if t.bytes().all(|b| b == b'-') {
    return Some(2);
  }
  None
}

fn is_table_separator(line: &str) -> bool {
  let cells = split_table_row(line);
  !cells.is_empty()
    && cells.iter().all(|cell| {
      let t = cell.trim();
      !t.is_empty() && t.contains('-') && t.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')
    })
}

fn split_table_row(line: &str) -> Vec<String> {
  let mut s = line.trim();
  s = s.strip_prefix('|').unwrap_or(s);
  s = s.strip_suffix('|').unwrap_or(s);
  let mut cells = Vec::new();
  let mut buffer = String::new();
  let mut chars = s.chars().peekable();
  while let Some(ch) = chars.next() {
    if ch == '\\' && chars.peek() == Some(&'|') {
      buffer.push('|');
      chars.next();
    } else if ch == '|' {
      cells.push(buffer.trim().to_string());
      buffer.clear();
    } else {
      buffer.push(ch);
    }
  }
  cells.push(buffer.trim().to_string());
  cells
}

/// Map a single non-empty, non-fence Markdown line to a `(kind, text, data)`
/// triple for block construction.
fn classify_markdown_line(content: &str) -> (&'static str, String, Value) {
  if let Some(level) = heading_prefix_level(content) {
    let text = content[level..].trim_start().to_string();
    return ("heading", text, json!({ "level": level }));
  }
  if let Some(rest) = content.strip_prefix("- [ ] ") {
    return ("todo", rest.to_string(), json!({ "checked": false }));
  }
  if let Some(rest) = content
    .strip_prefix("- [x] ")
    .or_else(|| content.strip_prefix("- [X] "))
  {
    return ("todo", rest.to_string(), json!({ "checked": true }));
  }
  if let Some(image) = parse_markdown_image(content) {
    return ("image", image.0, json!({ "url": image.1 }));
  }
  if let Some(rest) = content
    .strip_prefix("- ")
    .or_else(|| content.strip_prefix("* "))
  {
    return ("bulleted_list", rest.to_string(), Value::Null);
  }
  if let Some(rest) = numbered_list_rest(content) {
    return ("numbered_list", rest.to_string(), Value::Null);
  }
  if let Some(rest) = content.strip_prefix("> ") {
    return ("quote", rest.to_string(), Value::Null);
  }

  ("paragraph", content.to_string(), Value::Null)
}

fn push_block(
  blocks: &mut Vec<Block>,
  root_children: &mut Vec<String>,
  kind: &str,
  text: String,
  data: Value,
) {
  let id = format!("block_{}", Uuid::new_v4().simple());
  root_children.push(id.clone());
  blocks.push(Block {
    id,
    kind: kind.to_string(),
    text,
    data,
    children: Vec::new(),
  });
}

fn heading_prefix_level(content: &str) -> Option<usize> {
  let hashes = content.chars().take_while(|&c| c == '#').count();
  if (1..=6).contains(&hashes) && content[hashes..].starts_with(' ') {
    Some(hashes)
  } else {
    None
  }
}

fn numbered_list_rest(content: &str) -> Option<&str> {
  let digits_end = content.find(|c: char| !c.is_ascii_digit())?;
  if digits_end == 0 {
    return None;
  }
  content[digits_end..].strip_prefix(". ")
}

/// Parse a line that is exactly `![alt](url)` into `(alt, url)`.
fn parse_markdown_image(content: &str) -> Option<(String, String)> {
  let rest = content.strip_prefix("![")?;
  let alt_end = rest.find("](")?;
  let alt = &rest[..alt_end];
  let after = &rest[alt_end + 2..];
  let url = after.strip_suffix(')')?;
  Some((alt.to_string(), url.to_string()))
}

fn append_html_children(
  snapshot: &DocumentSnapshotPayload,
  parent_id: &str,
  out: &mut String,
) -> DocumentOperationResult<()> {
  let parent_index = block_index(snapshot, parent_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(parent_id.to_string()))?;
  let child_ids = snapshot.blocks[parent_index].children.clone();

  let mut index = 0;
  while index < child_ids.len() {
    let child = block_for(snapshot, &child_ids[index])?;
    match list_tag_for(&child.kind) {
      // Group a run of same-kind list items into a single <ul>/<ol>.
      Some(tag) => {
        out.push_str(&format!("<{tag}>\n"));
        while index < child_ids.len() {
          let item = block_for(snapshot, &child_ids[index])?;
          if list_tag_for(&item.kind) != Some(tag) {
            break;
          }
          out.push_str(&format!("<li>{}", escape_html(item.text.trim())));
          if !item.children.is_empty() {
            out.push('\n');
            append_html_children(snapshot, &item.id, out)?;
          }
          out.push_str("</li>\n");
          index += 1;
        }
        out.push_str(&format!("</{tag}>\n"));
      }
      None => {
        append_html_block(snapshot, child, out)?;
        index += 1;
      }
    }
  }

  Ok(())
}

/// Inline marks rendered to nested HTML (<strong>/<em>/<code>/<del>/<a>),
/// mirroring the markdown render_span structure.
fn html_inline(block: &Block) -> String {
  if matches!(block.kind.as_str(), "code_block" | "code" | "table") {
    return escape_html(&block.text);
  }
  let marks = marks_from_block(block);
  if marks.is_empty() {
    return escape_html(block.text.trim());
  }
  let units: Vec<u16> = block.text.encode_utf16().collect();
  let refs: Vec<&InlineMark> = marks.iter().collect();
  html_span(&units, 0, units.len(), &refs)
}

fn html_span(units: &[u16], lo: usize, hi: usize, marks: &[&InlineMark]) -> String {
  let mut out = String::new();
  let mut pos = lo;
  while pos < hi {
    let next = marks
      .iter()
      .enumerate()
      .filter_map(|(i, m)| {
        let s = m.start.max(pos);
        let e = m.end.min(hi);
        (e > s).then_some((s, e, i))
      })
      .min_by_key(|&(s, e, _)| (s, usize::MAX - e));
    let Some((s, e, picked)) = next else {
      out.push_str(&escape_html(&String::from_utf16_lossy(&units[pos..hi])));
      break;
    };
    out.push_str(&escape_html(&String::from_utf16_lossy(&units[pos..s])));
    let m = marks[picked];
    if m.kind == "code" {
      let raw = String::from_utf16_lossy(&units[s..e]);
      out.push_str(&format!("<code>{}</code>", escape_html(&raw)));
      pos = e;
      continue;
    }
    let inner: Vec<&InlineMark> = marks
      .iter()
      .enumerate()
      .filter(|&(i, x)| i != picked && x.end.min(e) > x.start.max(s))
      .map(|(_, x)| *x)
      .collect();
    let body = html_span(units, s, e, &inner);
    out.push_str(&match m.kind.as_str() {
      "bold" => format!("<strong>{body}</strong>"),
      "italic" => format!("<em>{body}</em>"),
      "strike" => format!("<del>{body}</del>"),
      "link" => match &m.title {
        Some(t) => format!(
          "<a href=\"{}\" title=\"{}\">{body}</a>",
          escape_html(m.href.as_deref().unwrap_or("")),
          escape_html(t)
        ),
        None => format!(
          "<a href=\"{}\">{body}</a>",
          escape_html(m.href.as_deref().unwrap_or(""))
        ),
      },
      _ => body,
    });
    pos = e;
  }
  out
}

fn append_html_block(
  snapshot: &DocumentSnapshotPayload,
  block: &Block,
  out: &mut String,
) -> DocumentOperationResult<()> {
  let text = html_inline(block);
  match block.kind.as_str() {
    "heading" => {
      let level = heading_level(block, 0);
      out.push_str(&format!("<h{level}>{text}</h{level}>\n"));
      append_html_children(snapshot, &block.id, out)?;
    }
    "todo" => {
      let checked = if block_data_bool(block, "checked") {
        " checked"
      } else {
        ""
      };
      out.push_str(&format!(
        "<div class=\"todo\"><input type=\"checkbox\" disabled{checked}> {text}</div>\n"
      ));
      append_html_children(snapshot, &block.id, out)?;
    }
    "quote" => {
      out.push_str(&format!("<blockquote>{text}</blockquote>\n"));
      append_html_children(snapshot, &block.id, out)?;
    }
    "code_block" | "code" => {
      let lang = block_data_str(block, "language").unwrap_or("");
      let class = if lang.is_empty() {
        String::new()
      } else {
        format!(" class=\"language-{}\"", escape_html(lang))
      };
      let body = if block.text.is_empty() {
        String::new()
      } else {
        format!("{}\n", escape_html(&block.text))
      };
      out.push_str(&format!("<pre><code{class}>{body}</code></pre>\n"));
      append_html_children(snapshot, &block.id, out)?;
    }
    "divider" => {
      out.push_str("<hr />\n");
    }
    "image" => {
      let url = escape_html(block_data_str(block, "url").unwrap_or_default());
      let alt = escape_html(if block.text.trim().is_empty() {
        "image"
      } else {
        block.text.trim()
      });
      out.push_str(&format!("<img src=\"{url}\" alt=\"{alt}\">\n"));
    }
    _ => {
      if !text.is_empty() {
        out.push_str(&format!("<p>{text}</p>\n"));
      }
      append_html_children(snapshot, &block.id, out)?;
    }
  }

  Ok(())
}

fn block_for<'a>(
  snapshot: &'a DocumentSnapshotPayload,
  block_id: &str,
) -> DocumentOperationResult<&'a Block> {
  let index = block_index(snapshot, block_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(block_id.to_string()))?;
  Ok(&snapshot.blocks[index])
}

fn list_tag_for(kind: &str) -> Option<&'static str> {
  match kind {
    "bulleted_list" | "bullet_list" => Some("ul"),
    "numbered_list" | "number_list" => Some("ol"),
    _ => None,
  }
}

fn escape_html(input: &str) -> String {
  let mut out = String::with_capacity(input.len());
  for ch in input.chars() {
    match ch {
      '&' => out.push_str("&amp;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      '"' => out.push_str("&quot;"),
      '\'' => out.push_str("&#39;"),
      other => out.push(other),
    }
  }
  out
}


fn append_markdown_children(
  snapshot: &DocumentSnapshotPayload,
  parent_id: &str,
  depth: usize,
  lines: &mut Vec<String>,
  images: &BTreeMap<String, String>,
) -> DocumentOperationResult<()> {
  let parent_index = block_index(snapshot, parent_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(parent_id.to_string()))?;
  let child_ids = snapshot.blocks[parent_index].children.clone();

  for child_id in child_ids {
    let child_index = block_index(snapshot, &child_id)
      .ok_or_else(|| DocumentOperationError::BlockNotFound(child_id.clone()))?;
    append_markdown_block(snapshot, &snapshot.blocks[child_index], depth, lines, images)?;
  }

  Ok(())
}

fn append_markdown_block(
  snapshot: &DocumentSnapshotPayload,
  block: &Block,
  depth: usize,
  lines: &mut Vec<String>,
  images: &BTreeMap<String, String>,
) -> DocumentOperationResult<()> {
  match block.kind.as_str() {
    "heading" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth, lines, images)?;
    }
    "todo" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth + 1, lines, images)?;
    }
    "bulleted_list" | "bullet_list" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth + 1, lines, images)?;
    }
    "numbered_list" | "number_list" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth + 1, lines, images)?;
    }
    "quote" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth, lines, images)?;
    }
    "code_block" | "code" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth, lines, images)?;
    }
    "image" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth, lines, images)?;
    }
    _ => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth, lines, images)?;
    }
  }

  Ok(())
}

fn append_markdown_block_content(
  block: &Block,
  depth: usize,
  lines: &mut Vec<String>,
  images: &BTreeMap<String, String>,
) {
  // List/todo nesting from `data.indent` (4 spaces per level — valid
  // CommonMark continuation for both `- ` and `1. ` parents); other kinds
  // keep the children-tree depth indent.
  let level = block
    .data
    .get("indent")
    .and_then(Value::as_u64)
    .unwrap_or(0) as usize;
  let indent = if matches!(block.kind.as_str(), "bulleted_list" | "bullet_list" | "numbered_list" | "number_list" | "todo") {
    "    ".repeat(level)
  } else {
    "  ".repeat(depth)
  };
  let text = block.text.trim_end();
  // Inline marks (bold/italic/code/strike/link) rendered back to Markdown.
  let rich = render_inline(block);
  let rich = rich.trim_end();
  match block.kind.as_str() {
    "heading" => {
      let level = heading_level(block, depth);
      lines.push(format!("{} {}", "#".repeat(level), rich));
      lines.push(String::new());
    }
    "todo" => {
      let marker = if block_data_bool(block, "checked") {
        "x"
      } else {
        " "
      };
      lines.push(format!("{indent}- [{marker}] {rich}"));
    }
    "bulleted_list" | "bullet_list" => lines.push(format!("{indent}- {rich}")),
    "numbered_list" | "number_list" => lines.push(format!("{indent}1. {rich}")),
    "quote" => {
      lines.push(format!("> {rich}"));
      lines.push(String::new());
    }
    "code_block" | "code" => {
      let language = block_data_str(block, "language").unwrap_or("");
      lines.push(format!("```{language}"));
      lines.extend(block.text.lines().map(str::to_string));
      lines.push("```".to_string());
      lines.push(String::new());
    }
    "image" => {
      // Prefer a bundled asset path (ZIP export) for an uploaded image; else
      // fall back to its original filename, then a raw external `url`.
      let asset = block_data_str(block, "file_id").and_then(|id| images.get(id));
      let target: String = match asset {
        Some(path) => path.clone(),
        None => block_data_str(block, "name")
          .filter(|s| !s.is_empty())
          .or_else(|| block_data_str(block, "url"))
          .unwrap_or_default()
          .to_string(),
      };
      // Keep an empty alt empty — `![](url)` must round-trip unchanged.
      lines.push(format!("![{text}]({target})"));
      lines.push(String::new());
    }
    "table" => {
      append_table_markdown(block, lines);
    }
    "divider" => {
      lines.push("---".to_string());
      lines.push(String::new());
    }
    _ => {
      if !text.is_empty() {
        let rich = escape_block_leader(rich.to_string());
        lines.push(format!("{indent}{rich}"));
        lines.push(String::new());
      }
    }
  }
}

/// Parse inline Markdown (`**b**`, `*i*`/`_i_`, `` `c` ``, `~~s~~`, `[t](url)`)
/// in [text] into clean text plus marks merged into [data] under `"marks"`.
/// Offsets are UTF-16 code-unit indices to match the Flutter client.
fn apply_inline_marks(text: String, data: Value, defs: &RefDefs) -> (String, Value) {
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

struct ParsedInline {
  text: String,
  marks: Vec<InlineMark>,
}

/// Link reference definitions: normalized label → (destination, title).
type RefDefs = std::collections::HashMap<String, (String, Option<String>)>;

/// Single-line link reference definition: ` [label]: dest "title"` → parts.
fn parse_ref_definition(raw: &str) -> Option<(String, String, Option<String>)> {
  let lead = raw.len() - raw.trim_start().len();
  if lead > 3 {
    return None;
  }
  let chars: Vec<char> = raw.trim().chars().collect();
  if chars.first() != Some(&'[') {
    return None;
  }
  let close = matching_bracket(&chars, 0)?;
  if chars.get(close + 1) != Some(&':') {
    return None;
  }
  let label: String = chars[1..close].iter().collect();
  if label.trim().is_empty() {
    return None;
  }
  let mut i = close + 2;
  while i < chars.len() && chars[i].is_whitespace() {
    i += 1;
  }
  if i >= chars.len() {
    return None; // dest on a later line: unsupported single-line form
  }
  // Destination (reuse the suffix parser's rules sans the closing paren).
  let dest: String;
  if chars[i] == '<' {
    let mut j = i + 1;
    while j < chars.len() && chars[j] != '>' {
      j += 1;
    }
    if j >= chars.len() {
      return None;
    }
    dest = unescape_md(&chars[i + 1..j].iter().collect::<String>());
    i = j + 1;
  } else {
    let start = i;
    while i < chars.len() && !chars[i].is_whitespace() {
      i += 1;
    }
    dest = unescape_md(&chars[start..i].iter().collect::<String>());
  }
  while i < chars.len() && chars[i].is_whitespace() {
    i += 1;
  }
  let mut title = None;
  if i < chars.len() && matches!(chars[i], '"' | '\'' | '(') {
    let closec = if chars[i] == '(' { ')' } else { chars[i] };
    let mut j = i + 1;
    let mut buf = String::new();
    while j < chars.len() && chars[j] != closec {
      if chars[j] == '\\' && j + 1 < chars.len() {
        buf.push(chars[j + 1]);
        j += 2;
        continue;
      }
      buf.push(chars[j]);
      j += 1;
    }
    if j >= chars.len() {
      return None;
    }
    title = Some(buf);
    i = j + 1;
  }
  while i < chars.len() && chars[i].is_whitespace() {
    i += 1;
  }
  if i != chars.len() {
    return None; // trailing junk → it's a paragraph, not a definition
  }
  Some((label, dest, title))
}

/// Case-fold and collapse internal whitespace, per the spec's label matching.
fn normalize_label(label: &str) -> String {
  label.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Unescape backslash-escaped ASCII punctuation (destinations/titles).
fn unescape_md(s: &str) -> String {
  let chars: Vec<char> = s.chars().collect();
  let mut out = String::with_capacity(s.len());
  let mut i = 0;
  while i < chars.len() {
    if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
      out.push(chars[i + 1]);
      i += 2;
    } else {
      out.push(chars[i]);
      i += 1;
    }
  }
  out
}

/// Parse an inline-link suffix `(dest "title")` starting right AFTER the `(`.
/// Returns (href, title, index just past the closing `)`).
fn parse_link_suffix(chars: &[char], mut i: usize) -> Option<(String, Option<String>, usize)> {
  let n = chars.len();
  while i < n && chars[i].is_whitespace() {
    i += 1;
  }
  // Destination: <may contain spaces> or bare with balanced parens.
  let dest: String;
  if i < n && chars[i] == '<' {
    let mut j = i + 1;
    while j < n && chars[j] != '>' && chars[j] != '\n' && chars[j] != '<' {
      j += 1;
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
      if c.is_whitespace() {
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
  while i < n && chars[i].is_whitespace() {
    i += 1;
  }
  // Optional title: "..." / '...' / (...)
  let mut title = None;
  if i < n && matches!(chars[i], '"' | '\'' | '(') {
    let close = if chars[i] == '(' { ')' } else { chars[i] };
    let mut j = i + 1;
    let mut buf = String::new();
    while j < n && chars[j] != close {
      if chars[j] == '\\' && j + 1 < n {
        buf.push(chars[j + 1]);
        j += 2;
        continue;
      }
      buf.push(chars[j]);
      j += 1;
    }
    if j >= n {
      return None;
    }
    title = Some(buf);
    i = j + 1;
    while i < n && chars[i].is_whitespace() {
      i += 1;
    }
  }
  if i < n && chars[i] == ')' { Some((dest, title, i + 1)) } else { None }
}

/// Find the `]` matching the `[` at [open], honoring nesting and escapes.
fn matching_bracket(chars: &[char], open: usize) -> Option<usize> {
  let mut depth = 0i32;
  let mut j = open;
  while j < chars.len() {
    let c = chars[j];
    if c == '\\' && j + 1 < chars.len() {
      j += 2;
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

/// Mirror of the Flutter `parseInline`: returns clean text (no markers) and the
/// marks over it, with offsets in UTF-16 code units.
fn parse_inline_with(src: &str, defs: &RefDefs) -> ParsedInline {
  let chars: Vec<char> = src.chars().collect();
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

  // Find the next unescaped occurrence of `needle` starting at `from`.
  let find_unescaped = |from: usize, needle: &[char]| -> Option<usize> {
    let mut j = from;
    while j + needle.len() <= chars.len() {
      if chars[j..j + needle.len()] == *needle && (j == 0 || chars[j - 1] != '\\') {
        return Some(j);
      }
      j += 1;
    }
    None
  };

  // Emit a resolved link into out/marks (text parsed recursively).
  let push_link = |label: &str,
                   href: String,
                   title: Option<String>,
                   out: &mut String,
                   out_len: &mut usize,
                   marks: &mut Vec<InlineMark>| {
    let start = *out_len;
    let parsed = parse_inline_with(label, defs);
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
      kind: "link".into(),
      href: Some(href),
      title,
    });
  };

  while i < chars.len() {
    // Backslash escape: `\*` is a literal `*` (any ASCII punctuation).
    if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
      out.push(chars[i + 1]);
      out_len += chars[i + 1].len_utf16();
      i += 2;
      continue;
    }
    // Autolink: <https://…> or <user@host> — the URL is both text and target.
    if chars[i] == '<' {
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
      }
    }
    // Link: [text](dest "title") | [text][label] | [text][] | [shortcut]
    if chars[i] == '[' {
      if let Some(close) = matching_bracket(&chars, i) {
        let label: String = chars[i + 1..close].iter().collect();
        if !label.is_empty() {
          // Inline form.
          if close + 1 < chars.len() && chars[close + 1] == '(' {
            if let Some((href, title, next)) = parse_link_suffix(&chars, close + 2) {
              push_link(&label, href, title, &mut out, &mut out_len, &mut marks);
              i = next;
              continue;
            }
          }
          // Reference forms (full / collapsed / shortcut).
          if !defs.is_empty() {
            let (ref_label, next) = if close + 1 < chars.len() && chars[close + 1] == '[' {
              match matching_bracket(&chars, close + 1) {
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
              push_link(&label, dest.clone(), title.clone(), &mut out, &mut out_len, &mut marks);
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
    // Strike: ~~...~~
    if chars[i] == '~' && i + 1 < chars.len() && chars[i + 1] == '~' {
      if let Some(end) = find_unescaped(i + 2, &['~', '~']) {
        if end > i + 2 {
          let inner: String = chars[i + 2..end].iter().collect();
          push_inner(&inner, "strike", None, defs, &mut out, &mut out_len, &mut marks);
          i = end + 2;
          continue;
        }
      }
    }
    // Inline code: `...`
    if chars[i] == '`' {
      if let Some(end) = find_from(i + 1, &['`']) {
        if end > i {
          let inner: String = chars[i + 1..end].iter().collect();
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
          i = end + 1;
          continue;
        }
      }
    }
    out.push(chars[i]);
    out_len += chars[i].len_utf16();
    i += 1;
  }

  process_emphasis(&mut out, &mut marks, &mut delims);

  ParsedInline { text: out, marks }
}

/// One run of `*`/`_` delimiters, tracked in output (UTF-16) coordinates.
struct Delim {
  c: char,
  start: usize,     // original run start in `out`
  cur_start: usize, // advances as the closer side is consumed
  count: usize,     // remaining delimiter characters
  orig: usize,      // original run length (rule-of-3)
  can_open: bool,
  can_close: bool,
}

fn is_md_punct(c: char) -> bool {
  c.is_ascii_punctuation()
    || matches!(c, '\u{2018}'..='\u{201F}' | '\u{2010}'..='\u{2027}')
}

/// Left/right flanking → (can_open, can_close) per the spec, including the
/// `_` intraword restrictions.
fn flanking(c: char, prev: Option<char>, next: Option<char>) -> (bool, bool) {
  let prev_ws = prev.is_none_or(char::is_whitespace);
  let next_ws = next.is_none_or(char::is_whitespace);
  let prev_punct = prev.is_some_and(is_md_punct);
  let next_punct = next.is_some_and(is_md_punct);

  let left = !next_ws && (!next_punct || prev_ws || prev_punct);
  let right = !prev_ws && (!prev_punct || next_ws || next_punct);

  if c == '_' {
    (left && (!right || prev_punct), right && (!left || next_punct))
  } else {
    (left, right)
  }
}

/// The spec's process-emphasis: pair closers with the nearest valid opener
/// (same char, rule-of-3), strong before em, deleting used delimiter
/// characters and emitting bold/italic marks over the content between.
fn process_emphasis(out: &mut String, marks: &mut Vec<InlineMark>, delims: &mut [Delim]) {
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

    let use_n = if delims[oi].count >= 2 && delims[closer_i].count >= 2 { 2 } else { 1 };
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
fn autolink_target(inner: &str) -> Option<String> {
  if inner.is_empty() || inner.chars().any(|c| c.is_whitespace() || c == '<') {
    return None;
  }
  let bytes = inner.as_bytes();
  // scheme: ALPHA (ALPHA/DIGIT/+/-/.)* ':'
  if let Some(colon) = inner.find(':') {
    let scheme = &inner[..colon];
    if !scheme.is_empty()
      && scheme.as_bytes()[0].is_ascii_alphabetic()
      && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
      && colon + 1 < inner.len()
    {
      return Some(inner.to_string());
    }
  }
  // email: x@y.z (loose)
  if let Some(at) = inner.find('@') {
    let (local, host) = inner.split_at(at);
    let host = &host[1..];
    if !local.is_empty() && host.contains('.') && !host.ends_with('.') && bytes.iter().filter(|&&b| b == b'@').count() == 1 {
      return Some(format!("mailto:{inner}"));
    }
  }
  None
}

/// Parse [inner] recursively and append it to [out], adding a [kind] mark over
/// the appended range.
fn push_inner(
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
struct InlineMark {
  start: usize,
  end: usize,
  kind: String,
  href: Option<String>,
  title: Option<String>,
}

fn marks_from_block(block: &Block) -> Vec<InlineMark> {
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
      if end > start {
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

/// Render a block's text with its inline marks back to Markdown. Code blocks and
/// tables carry no inline marks and are emitted verbatim by their own arms.
/// Escape characters our (and CommonMark's) inline grammar would otherwise
/// interpret, so literal text survives a round-trip.
fn escape_inline(text: &str) -> String {
  let mut out = String::with_capacity(text.len());
  for c in text.chars() {
    if matches!(c, '\\' | '*' | '_' | '`' | '~' | '[' | ']' | '<') {
      out.push('\\');
    }
    out.push(c);
  }
  out
}

/// A paragraph whose text LOOKS like a block marker (`- x`, `> x`, `# x`,
/// `1. x`, `---`) must escape its leader or it changes kind on re-import.
fn escape_block_leader(line: String) -> String {
  let t = line.as_str();
  let compact: String = t.chars().filter(|c| *c != ' ').collect();
  let divider_like = compact.len() >= 3 && compact.chars().all(|c| c == '-');
  // A paragraph that would read as a setext underline gets escaped too.
  let setext_like = !t.is_empty() && (t.bytes().all(|b| b == b'=') || t.bytes().all(|b| b == b'-'));
  let divider_like = divider_like || setext_like;
  let numbered = t
    .find(". ")
    .is_some_and(|dot| dot > 0 && t[..dot].bytes().all(|b| b.is_ascii_digit()));
  if t.starts_with("- ")
    || t.starts_with("+ ")
    || t.starts_with("> ")
    || divider_like
    || (t.starts_with('#') && t[1..].trim_start_matches('#').starts_with(' '))
  {
    return format!("\\{line}");
  }
  if numbered {
    let dot = t.find(". ").unwrap();
    return format!("{}\\. {}", &t[..dot], &t[dot + 2..]);
  }
  line
}

fn render_inline(block: &Block) -> String {
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

/// Render `[lo, hi)` of the text wrapping marks as properly NESTED markdown:
/// the outermost mark opens once over its whole range with inner marks
/// recursing inside (`**bold *italic* tail**`) — per-segment wrapping breaks
/// round-trips. Marks that only partially overlap an enclosing one are
/// clipped (markdown cannot express crossing ranges).
fn render_span(units: &[u16], lo: usize, hi: usize, marks: &[&InlineMark]) -> String {
  let mut out = String::new();
  let mut pos = lo;
  while pos < hi {
    // The next mark by clipped start; ties prefer the widest (outermost).
    let next = marks
      .iter()
      .enumerate()
      .filter_map(|(i, m)| {
        let s = m.start.max(pos);
        let e = m.end.min(hi);
        (e > s).then_some((s, e, i))
      })
      .min_by_key(|&(s, e, _)| (s, usize::MAX - e));
    let Some((s, e, picked)) = next else {
      out.push_str(&escape_inline(&String::from_utf16_lossy(&units[pos..hi])));
      break;
    };
    out.push_str(&escape_inline(&String::from_utf16_lossy(&units[pos..s])));
    let inner: Vec<&InlineMark> = marks
      .iter()
      .enumerate()
      .filter(|&(i, m)| i != picked && m.end.min(e) > m.start.max(s))
      .map(|(_, m)| *m)
      .collect();
    let m = marks[picked];
    if m.kind == "code" {
      // Code spans are literal — no escaping, no nested marks.
      let raw = String::from_utf16_lossy(&units[s..e]);
      out.push_str(&format!("`{raw}`"));
      pos = e;
      continue;
    }
    let body = render_span(units, s, e, &inner);
    out.push_str(&match m.kind.as_str() {
      "bold" => format!("**{body}**"),
      "italic" => format!("*{body}*"),
      "strike" => format!("~~{body}~~"),
      "link" => {
        let href = m.href.as_deref().unwrap_or("");
        let plain = String::from_utf16_lossy(&units[s..e]);
        // A title-less link whose text IS its target → autolink form.
        if m.title.is_none() && (href == plain || href == format!("mailto:{plain}")) {
          format!("<{plain}>")
        } else {
          let dest = if href.contains(char::is_whitespace) {
            format!("<{href}>")
          } else {
            href.to_string()
          };
          match &m.title {
            Some(t) => format!("[{body}]({dest} \"{}\")", t.replace('"', "\\\"")),
            None => format!("[{body}]({dest})"),
          }
        }
      }
      _ => body,
    });
    pos = e;
  }
  out
}

fn append_table_markdown(block: &Block, lines: &mut Vec<String>) {
  let Some(rows) = block.data.get("rows").and_then(Value::as_array) else {
    return;
  };
  let grid: Vec<Vec<String>> = rows
    .iter()
    .filter_map(|row| {
      row.as_array().map(|cells| {
        cells
          .iter()
          .map(|cell| {
            cell
              .as_str()
              .unwrap_or("")
              .replace('|', "\\|")
              .replace('\n', " ")
              .trim()
              .to_string()
          })
          .collect()
      })
    })
    .collect();
  if grid.is_empty() {
    return;
  }
  let cols = grid.iter().map(Vec::len).max().unwrap_or(0).max(1);
  let row_line = |cells: &[String]| -> String {
    let mut padded = cells.to_vec();
    while padded.len() < cols {
      padded.push(String::new());
    }
    format!("| {} |", padded.join(" | "))
  };
  lines.push(row_line(&grid[0]));
  lines.push(format!("| {} |", vec!["---"; cols].join(" | ")));
  for row in grid.iter().skip(1) {
    lines.push(row_line(row));
  }
  lines.push(String::new());
}

fn heading_level(block: &Block, depth: usize) -> usize {
  block
    .data
    .get("level")
    .and_then(Value::as_u64)
    .map(|level| level as usize)
    .unwrap_or(depth + 1)
    .clamp(1, 6)
}

fn block_data_str<'a>(block: &'a Block, key: &str) -> Option<&'a str> {
  block.data.get(key).and_then(Value::as_str)
}

fn block_data_bool(block: &Block, key: &str) -> bool {
  block
    .data
    .get(key)
    .and_then(Value::as_bool)
    .unwrap_or(false)
}



pub fn block_index(snapshot: &DocumentSnapshotPayload, block_id: &str) -> Option<usize> {
  snapshot
    .blocks
    .iter()
    .position(|block| block.id == block_id)
}

pub fn is_descendant(snapshot: &DocumentSnapshotPayload, block_id: &str, parent_id: &str) -> bool {
  let Some(index) = block_index(snapshot, parent_id) else {
    return false;
  };

  for child_id in &snapshot.blocks[index].children {
    if child_id == block_id || is_descendant(snapshot, block_id, child_id) {
      return true;
    }
  }

  false
}
