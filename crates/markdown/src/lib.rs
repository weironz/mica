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

  // Pass 1: link reference definitions (`[label]: dest "title"`, possibly
  // spanning lines) — they resolve case-insensitively and vanish from the
  // output. Definitions inside fences don't count, and a definition can't
  // interrupt a paragraph.
  let (defs, def_lines) = collect_ref_definitions(&raw_lines);

  let mut index = 0;
  // Stack of open list items' CONTENT columns (tabs count as 4): a new item
  // is a child only when its marker reaches the parent's content column —
  // `- foo` / ` - bar` are siblings. Other blocks reset the stack.
  let mut list_stack: Vec<usize> = Vec::new();
  // The most recently pushed paragraph-like block (paragraph or list/todo
  // item), kept open for multi-line continuation: indented or lazy lines
  // join its text with a soft break; for items, a blank + indented line
  // starts a second paragraph (joined with \n\n, list turns loose). `raw`
  // is the accumulated marker-stripped source, `base` the data before
  // inline marks so the joined text can re-parse cleanly.
  struct OpenItem {
    block_idx: usize,
    kind: String,
    content_col: usize,
    raw: String,
    base: Value,
    had_blank: bool,
    qdepth: usize,
    ends_hard: bool,
  }
  let mut open_item: Option<OpenItem> = None;
  // Last pushed list item (index, level, marker char) — anchors loose-list
  // marking, the "only a run's first number sets <ol start>" rule, and
  // marker-change list breaks (`-` vs `+`, `.` vs `)`).
  let mut last_list: Option<(usize, usize, char)> = None;
  // A blank line seen inside a list: if another item follows, the list is
  // loose (items render as paragraphs in HTML).
  let mut pending_loose = false;
  // Block quotes (flat model: blocks carry `data.quote` = depth, the HTML
  // exporter rebuilds nested <blockquote>): is a quote group currently
  // open, did a blank just close one (next quote gets `data.qbreak`), and
  // is a content-less `>` group waiting to become an empty quote block.
  let mut quote_active = false;
  let mut quote_boundary = false;
  let mut pending_empty_quote: Option<usize> = None;
  // Does the deepest open list item already hold container children
  // (code/quote/divider blocks carrying `data.li`)? Then later indented
  // paragraphs become child paragraphs instead of `\n\n` text joins.
  let mut item_children = false;
  while index < raw_lines.len() {
    if def_lines.contains(&index) {
      index += 1;
      continue;
    }
    let line = raw_lines[index].trim_end();
    let content = line.trim_start();
    // Two or more trailing spaces on the source line = a hard line break if
    // a continuation joins (canonicalized to a backslash break).
    let ends_hard = raw_lines[index].ends_with("  ");

    if content.is_empty() {
      // Blank lines between items keep the list context, end the open
      // paragraph (items merely note the blank — an indented line can still
      // continue them), and make the list loose if it continues.
      match open_item.as_mut() {
        Some(open)
          if matches!(open.kind.as_str(), "bulleted_list" | "numbered_list" | "todo") =>
        {
          open.had_blank = true
        }
        _ => open_item = None,
      }
      pending_loose = pending_loose || !list_stack.is_empty();
      // A content-less `>` group becomes an empty quote block; a blank
      // after a quote separates blockquotes (the next gets `qbreak`).
      if let Some(d) = pending_empty_quote.take() {
        let mut data = if d > 1 { json!({ "quote": d }) } else { Value::Null };
        if quote_boundary {
          data_insert(&mut data, "qbreak", json!(true));
        }
        push_block(&mut blocks, &mut root_children, "quote", String::new(), data);
      }
      if blocks.last().is_some_and(|b| quote_depth_of(b) > 0) {
        quote_boundary = true;
      }
      quote_active = false;
      index += 1;
      continue;
    }
    let mut col: usize = 0;
    for c in line[..line.len() - content.len()].chars() {
      col = if c == '\t' { (col / 4 + 1) * 4 } else { col + 1 };
    }

    // A 4+-column line cannot start a new block while a top-level
    // paragraph-like block is open — it is lazy continuation text.
    if col >= 4
      && list_stack.is_empty()
      && let Some(open) = open_item.as_mut()
      && !open.had_blank
    {
      open.raw.push_str(if open.ends_hard { "\\\n" } else { "\n" });
      open.raw.push_str(content);
      open.ends_hard = ends_hard;
      let (joined, joined_data) = apply_inline_marks(open.raw.clone(), open.base.clone(), &defs);
      blocks[open.block_idx].text = joined;
      blocks[open.block_idx].data = joined_data;
      index += 1;
      continue;
    }

    // Setext underline of the open (possibly multi-line) paragraph: the
    // whole continued paragraph becomes the heading.
    if let Some(open) = &open_item
      && open.kind == "paragraph"
      && let Some(level) = setext_level(line)
    {
      let (text, data) = apply_inline_marks(open.raw.clone(), json!({ "level": level }), &defs);
      let idx = open.block_idx;
      blocks[idx].kind = "heading".to_string();
      blocks[idx].text = text;
      blocks[idx].data = data;
      open_item = None;
      index += 1;
      continue;
    }

    // List-item container children: at or past the open item's content
    // column, fences / indented code / dividers (and, below, quotes and
    // paragraphs) belong INSIDE the item — flat blocks carrying `data.li`.
    if let Some(&cc) = list_stack.last()
      && col >= cc
      && strip_quote_markers(content).0 == 0
      && !matches!(
        classify_markdown_line(content).0,
        "bulleted_list" | "numbered_list" | "todo"
      )
    {
      let level = list_stack.len() - 1;
      // Fenced code child.
      if let Some((fence_char, fence_len, info)) = fence_open(content) {
        let language =
          unescape_md(info.trim().split_whitespace().next().unwrap_or_default());
        let mut code_lines = Vec::new();
        index += 1;
        while index < raw_lines.len() {
          let l = raw_lines[index];
          let lt = l.trim_start();
          if lt.is_empty() {
            code_lines.push(String::new());
            index += 1;
            continue;
          }
          let lcol = leading_columns(l);
          if lcol < cc {
            break; // the item ended — so does the fence
          }
          if fence_close(lt, fence_char, fence_len) {
            index += 1;
            break;
          }
          // Content sheds up to the OPENING fence's indentation.
          code_lines.push(deindent_columns(l, col));
          index += 1;
        }
        // Trailing blank-only lines belong between blocks, not the code.
        while code_lines.last().is_some_and(|l| l.is_empty()) {
          code_lines.pop();
        }
        let mut data = if language.is_empty() {
          json!({})
        } else {
          json!({ "language": language })
        };
        data_insert(&mut data, "li", json!(level));
        if pending_loose {
          if let Some((prev_idx, _, _)) = last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), data);
        item_children = true;
        open_item = None;
        continue;
      }
      // Indented code child (4+ columns past the content column).
      if col >= cc + 4 {
        let mut code_lines: Vec<String> = Vec::new();
        let mut blanks: Vec<String> = Vec::new();
        while index < raw_lines.len() {
          let l = raw_lines[index];
          if l.trim().is_empty() {
            blanks.push(deindent_columns(l, cc + 4));
            index += 1;
            continue;
          }
          if leading_columns(l) < cc + 4 {
            break;
          }
          code_lines.append(&mut blanks);
          code_lines.push(deindent_columns(l, cc + 4));
          index += 1;
        }
        let mut data = json!({ "li": level });
        if pending_loose {
          if let Some((prev_idx, _, _)) = last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), data);
        item_children = true;
        open_item = None;
        continue;
      }
      // Divider child.
      if is_divider(content) {
        let mut data = json!({ "li": level });
        if pending_loose {
          if let Some((prev_idx, _, _)) = last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, "divider", String::new(), data);
        item_children = true;
        open_item = None;
        index += 1;
        continue;
      }
      // Paragraph child: once the item holds children, indented paragraph
      // lines become child blocks (order matters — text renders first).
      if item_children
        && classify_markdown_line(content).0 == "paragraph"
        && open_item.is_none()
      {
        let mut data = json!({ "li": level });
        if pending_loose {
          if let Some((prev_idx, _, _)) = last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          pending_loose = false;
        }
        let base = data.clone();
        let (text2, data2) = apply_inline_marks(content.to_string(), data, &defs);
        push_block(&mut blocks, &mut root_children, "paragraph", text2, data2);
        open_item = Some(OpenItem {
          block_idx: blocks.len() - 1,
          kind: "paragraph".to_string(),
          content_col: cc,
          raw: content.to_string(),
          base,
          had_blank: false,
          qdepth: 0,
          ends_hard,
        });
        index += 1;
        continue;
      }
      // Anything else falls through to the regular machinery.
    }

    if col < 4
      && let Some((fence_char, fence_len, info)) = fence_open(content)
    {
      list_stack.clear();
      open_item = None;
      last_list = None;
      pending_loose = false;
      // Only the first word of the info string is the language.
      let language =
        unescape_md(info.trim().split_whitespace().next().unwrap_or_default());
      let mut code_lines = Vec::new();
      index += 1;
      while index < raw_lines.len() {
        let l = raw_lines[index];
        let lt = l.trim_start();
        let lcol = l.len() - lt.len();
        if lcol < 4 && fence_close(lt, fence_char, fence_len) {
          index += 1;
          break;
        }
        // Content lines shed up to the opening fence's indentation.
        code_lines.push(if col > 0 {
          deindent_columns(l, col)
        } else {
          l.to_string()
        });
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

    // HTML block (CommonMark types 1–7) → a raw html code block: the
    // source is the content (AFFiNE-style degrade), `data.raw` makes both
    // exporters write it back verbatim. Type 7 can't interrupt a paragraph.
    if col < 4
      && let Some(html_kind) = html_block_start(content)
      && list_stack.is_empty()
      && !(html_kind == 7 && open_item.is_some())
    {
      let mut html_lines: Vec<String> = vec![raw_lines[index].to_string()];
      let ends_by_marker = html_kind <= 5;
      let mut done = ends_by_marker && html_block_ends(html_kind, content);
      index += 1;
      while index < raw_lines.len() && !done {
        let l = raw_lines[index];
        if ends_by_marker {
          html_lines.push(l.to_string());
          done = html_block_ends(html_kind, l);
          index += 1;
        } else {
          if l.trim().is_empty() {
            break; // types 6–7 end at a blank line
          }
          html_lines.push(l.to_string());
          index += 1;
        }
      }
      push_block(
        &mut blocks,
        &mut root_children,
        "code_block",
        html_lines.join("\n"),
        json!({ "language": "html", "raw": true }),
      );
      open_item = None;
      last_list = None;
      pending_loose = false;
      item_children = false;
      continue;
    }

    // GFM pipe table: a `|`-row followed by a `| --- |` separator whose
    // cell count MATCHES the header row (spec rule).
    if content.contains('|')
      && index + 1 < raw_lines.len()
      && is_table_separator(raw_lines[index + 1].trim())
      && split_table_row(raw_lines[index + 1].trim()).len() == split_table_row(content).len()
    {
      list_stack.clear();
      open_item = None;
      last_list = None;
      pending_loose = false;
      let header = split_table_row(content);
      let width = header.len().max(1);
      // Column alignment from the separator's colons.
      let aligns: Vec<String> = split_table_row(raw_lines[index + 1].trim())
        .iter()
        .map(|cell| {
          let t = cell.trim();
          match (t.starts_with(':'), t.ends_with(':')) {
            (true, true) => "center",
            (false, true) => "right",
            (true, false) => "left",
            _ => "",
          }
          .to_string()
        })
        .collect();
      let mut rows: Vec<Vec<String>> = vec![header];
      index += 2;
      while index < raw_lines.len() {
        let row = raw_lines[index].trim();
        if row.is_empty() {
          break;
        }
        // A pipe-less line still belongs to the table unless it starts
        // another block (GFM: the table breaks at a blank line or the
        // beginning of another block-level structure).
        if !row.contains('|')
          && (classify_markdown_line(row).0 != "paragraph"
            || is_divider(row)
            || fence_open(row).is_some()
            || html_block_start(row).is_some()
            || strip_quote_markers(row).0 > 0)
        {
          break;
        }
        rows.push(split_table_row(row));
        index += 1;
      }
      // The header defines the column count: longer body rows truncate,
      // shorter ones pad (spec rule).
      for row in rows.iter_mut() {
        row.truncate(width);
        while row.len() < width {
          row.push(String::new());
        }
      }
      let mut data = json!({
        "rows": rows,
        "header": true,
        "align": "left",
        "widths": vec![1.0f64; width],
      });
      if aligns.iter().any(|a| !a.is_empty()) {
        data_insert(&mut data, "aligns", json!(aligns));
      }
      push_block(&mut blocks, &mut root_children, "table", String::new(), data);
      continue;
    }

    // Indented code block: 4+ columns at top level (inside a list that
    // indentation means nesting instead, handled by the stack above).
    if col >= 4
      && ((list_stack.is_empty() && open_item.is_none())
        || open_item
          .as_ref()
          .is_some_and(|o| o.had_blank && col < o.content_col))
    {
      list_stack.clear();
      open_item = None;
      last_list = None;
      pending_loose = false;
      let mut code_lines: Vec<String> = Vec::new();
      let mut pending_blanks: Vec<String> = Vec::new();
      while index < raw_lines.len() {
        let l = raw_lines[index];
        if l.trim().is_empty() {
          // Blank-ish lines keep whatever indentation they have past the
          // 4-column code margin.
          pending_blanks.push(deindent_columns(l, 4));
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
        code_lines.append(&mut pending_blanks);
        // Trailing spaces are code content — keep the line untrimmed.
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
      open_item = None;
      last_list = None;
      pending_loose = false;
      push_block(&mut blocks, &mut root_children, "divider", String::new(), Value::Null);
      index += 1;
      continue;
    }

    // Block quote: strip `>` markers (each takes one optional following
    // space; up to 3 spaces may sit between nested markers) → depth + rest.
    let (qdepth, qrest) = strip_quote_markers(content);
    if qdepth > 0 {
      // A quote at or past the open item's content column nests INSIDE the
      // item (data.li) — the list context survives.
      let li_ctx = match list_stack.last() {
        Some(&cc) if col >= cc => Some(list_stack.len() - 1),
        _ => None,
      };
      if li_ctx.is_none() {
        list_stack.clear();
        last_list = None;
      } else if pending_loose {
        if let Some((prev_idx, _, _)) = last_list {
          data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
        }
      }
      pending_loose = false;
      let qrest_trim = qrest.trim_start();

      // `>` with nothing after: a paragraph break inside the quote — or,
      // if the group never gets content, an empty blockquote.
      if qrest_trim.is_empty() {
        if !quote_active && pending_empty_quote.is_none() {
          pending_empty_quote = Some(qdepth);
        }
        open_item = None;
        quote_active = true;
        index += 1;
        continue;
      }

      // Lazy/marked continuation: a plain paragraph line at the same or a
      // shallower marker depth keeps the open quoted paragraph going.
      let (kind, text, mut data) = classify_markdown_line(qrest_trim);
      if kind == "paragraph"
        && !qrest_trim.starts_with("```")
        && let Some(open) = open_item.as_mut()
        && open.qdepth >= qdepth
        && !open.had_blank
      {
        open.raw.push_str(if open.ends_hard { "\\\n" } else { "\n" });
        open.raw.push_str(qrest_trim);
        open.ends_hard = ends_hard;
        let (joined, joined_data) =
          apply_inline_marks(open.raw.clone(), open.base.clone(), &defs);
        blocks[open.block_idx].text = joined;
        blocks[open.block_idx].data = joined_data;
        quote_active = true;
        index += 1;
        continue;
      }

      pending_empty_quote = None;
      let qbreak = quote_boundary;
      quote_boundary = false;

      // Fenced code inside the quote: runs while the markers do.
      if let Some(language) = qrest_trim.strip_prefix("```") {
        let language = language.trim().to_string();
        let mut code_lines = Vec::new();
        index += 1;
        while index < raw_lines.len() {
          let lcontent = raw_lines[index].trim_end().trim_start();
          let (d2, r2) = strip_quote_markers(lcontent);
          if d2 < qdepth {
            break; // the quote ended — so does the fence
          }
          if r2.trim_start().starts_with("```") {
            index += 1;
            break;
          }
          code_lines.push(r2.to_string());
          index += 1;
        }
        let mut data = if language.is_empty() {
          Value::Null
        } else {
          json!({ "language": language })
        };
        data_insert(&mut data, "quote", json!(qdepth));
        if qbreak {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          item_children = true;
        }
        push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), data);
        open_item = None;
        quote_active = true;
        continue;
      }

      // Indented code inside the quote (per marked line).
      let qcol = qrest.len() - qrest_trim.len();
      if qcol >= 4 {
        let mut data = json!({ "quote": qdepth });
        if qbreak {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          item_children = true;
        }
        push_block(
          &mut blocks,
          &mut root_children,
          "code_block",
          deindent_columns(qrest, 4),
          data,
        );
        open_item = None;
        quote_active = true;
        index += 1;
        continue;
      }

      if is_divider(qrest_trim) {
        let mut data = json!({ "quote": qdepth });
        if qbreak {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          item_children = true;
        }
        push_block(&mut blocks, &mut root_children, "divider", String::new(), data);
        open_item = None;
        quote_active = true;
        index += 1;
        continue;
      }

      // Quoted content block: plain text becomes the `quote` kind (depth in
      // `data.quote` past 1); any other kind carries `data.quote`.
      let kind = if kind == "paragraph" { "quote" } else { kind };
      if kind != "quote" || qdepth > 1 {
        data_insert(&mut data, "quote", json!(qdepth));
      }
      if qbreak {
        data_insert(&mut data, "qbreak", json!(true));
      }
      if let Some(level) = li_ctx {
        data_insert(&mut data, "li", json!(level));
        item_children = true;
      }
      let raw = text.clone();
      let base = data.clone();
      let (text, data) = if kind == "image" {
        (parse_inline_with(&text, &defs).text, data)
      } else {
        apply_inline_marks(text, data, &defs)
      };
      push_block(&mut blocks, &mut root_children, kind, text, data);
      open_item = if matches!(kind, "quote" | "bulleted_list" | "numbered_list" | "todo") {
        Some(OpenItem {
          block_idx: blocks.len() - 1,
          kind: kind.to_string(),
          content_col: 0,
          raw,
          base,
          had_blank: false,
          qdepth,
          ends_hard,
        })
      } else {
        None
      };
      quote_active = true;
      index += 1;
      continue;
    }

    let (kind, text, data) = classify_markdown_line(content);
    // An indented (4+ columns) marker cannot start a list at top level —
    // the line is paragraph continuation or code, never a new item.
    let (kind, text, data) = if matches!(kind, "bulleted_list" | "numbered_list" | "todo")
      && col >= 4
      && list_stack.is_empty()
    {
      ("paragraph", content.to_string(), Value::Null)
    } else {
      (kind, text, data)
    };
    let mut data = data;

    // Continuation (CommonMark): a paragraph line joins the open block with
    // a soft break (lazy lines included); after a blank, a line indented to
    // the item's content column starts a second paragraph inside the item
    // (the list turns loose). An empty list item or an ordered marker other
    // than `1.` cannot interrupt a paragraph — those lines stay text.
    if let Some(open) = open_item.as_mut() {
      let weak_item = matches!(kind, "bulleted_list" | "numbered_list" | "todo")
        && open.kind == "paragraph"
        && !open.had_blank
        && (text.is_empty() || data.get("start").is_some());
      if kind == "paragraph" || weak_item {
        if open.had_blank {
          if col >= open.content_col && !open.raw.is_empty() && !item_children {
            open.had_blank = false;
            open.raw.push_str("\n\n");
            open.raw.push_str(content);
            data_insert(&mut open.base, "loose", json!(true));
            pending_loose = false;
            let (joined, joined_data) =
              apply_inline_marks(open.raw.clone(), open.base.clone(), &defs);
            blocks[open.block_idx].text = joined;
            blocks[open.block_idx].data = joined_data;
            index += 1;
            continue;
          }
          // The blank closed the item; whatever follows is a new block.
        } else {
          if open.raw.is_empty() {
            open.raw = content.to_string();
          } else {
            open.raw.push_str(if open.ends_hard { "\\\n" } else { "\n" });
            open.raw.push_str(content);
          }
          open.ends_hard = ends_hard;
          let (joined, joined_data) =
            apply_inline_marks(open.raw.clone(), open.base.clone(), &defs);
          blocks[open.block_idx].text = joined;
          blocks[open.block_idx].data = joined_data;
          index += 1;
          continue;
        }
      }
    }

    // List/todo items: nesting level from the indentation stack, loose and
    // <ol start> bookkeeping, then the item stays open for continuations.
    if matches!(kind, "bulleted_list" | "numbered_list" | "todo") {
      // Content column: marker width plus up to 3 extra spaces consumed
      // (more than that means the item starts with indented code — the
      // spaces stay in the text and the column sits right after the
      // marker). A todo's task marker `[x] ` is CONTENT, not marker: only
      // the `- ` counts, so two-space-indented subtasks nest.
      let marker_width = if kind == "todo" {
        2
      } else {
        content.len() - text.len()
      };
      let extra = text.len() - text.trim_start_matches(' ').len();
      let (text, content_col) = if text.is_empty() {
        (text, col + marker_width + 1)
      } else if extra <= 3 {
        (text[extra..].to_string(), col + marker_width + extra)
      } else {
        (text, col + marker_width)
      };
      while list_stack.last().is_some_and(|&cc| col < cc) {
        list_stack.pop();
      }
      let level = list_stack.len();
      list_stack.push(content_col);
      if level > 0 {
        data_insert(&mut data, "indent", json!(level));
      }
      // The marker character: `-`/`*`/`+` for bullets and todos, the
      // delimiter (`.`/`)`) for ordered items.
      let marker_char = if kind == "numbered_list" {
        content.as_bytes()[content.bytes().position(|b| !b.is_ascii_digit()).unwrap()] as char
      } else {
        content.as_bytes()[0] as char
      };
      // Does this item continue the previous run (same level, same kind,
      // same marker)? A marker change starts a new list.
      let continues_run = last_list.is_some_and(|(idx, lv, mk)| {
        lv == level && blocks[idx].kind == kind && mk == marker_char
      });
      let same_level_break = !continues_run
        && last_list.is_some_and(|(idx, lv, _)| lv == level && blocks[idx].kind == kind);
      if same_level_break {
        // Record the changed bullet/delimiter so export keeps the break.
        data_insert(&mut data, "marker", json!(marker_char.to_string()));
      }
      // The blank line belongs to whichever list the boundary sits in:
      // same-or-shallower level → this item is loose; deeper level → the
      // blank separated a parent's text from its sublist, so the parent is.
      if pending_loose {
        match last_list {
          Some((prev_idx, prev_level, _)) if level > prev_level => {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          _ => data_insert(&mut data, "loose", json!(true)),
        }
        pending_loose = false;
      }
      // Only the number on the item that BEGINS an ordered run sets the
      // list's start; later numbers are ignored by the spec.
      if kind == "numbered_list" && data.get("start").is_some() && continues_run {
        data.as_object_mut().map(|m| m.remove("start"));
      }
      // An item whose text itself opens a container (`- ```'/`- ***`/code
      // by 4+ extra spaces) becomes an EMPTY item plus a child block.
      let starts_fence = fence_open(&text).map(|(c, n, info)| (c, n, info.to_string()));
      let starts_divider = is_divider(&text) && !text.is_empty();
      let starts_code = extra > 3;
      if starts_fence.is_some() || starts_divider || starts_code {
        let (item_text, item_data) = apply_inline_marks(String::new(), data, &defs);
        push_block(&mut blocks, &mut root_children, kind, item_text, item_data);
        let block_idx = blocks.len() - 1;
        last_list = Some((block_idx, level, marker_char));
        while list_stack.len() > level {
          list_stack.pop();
        }
        list_stack.push(content_col);
        item_children = true;
        open_item = None;
        if let Some((fence_char, fence_len, info)) = starts_fence {
          let language =
            unescape_md(info.trim().split_whitespace().next().unwrap_or_default());
          let mut code_lines = Vec::new();
          index += 1;
          while index < raw_lines.len() {
            let l = raw_lines[index];
            let lt = l.trim_start();
            if lt.is_empty() {
              code_lines.push(String::new());
              index += 1;
              continue;
            }
            if leading_columns(l) < content_col {
              break;
            }
            if fence_close(lt, fence_char, fence_len) {
              index += 1;
              break;
            }
            code_lines.push(deindent_columns(l, content_col));
            index += 1;
          }
          while code_lines.last().is_some_and(|l| l.is_empty()) {
            code_lines.pop();
          }
          let mut cdata = if language.is_empty() {
            json!({})
          } else {
            json!({ "language": language })
          };
          data_insert(&mut cdata, "li", json!(level));
          push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), cdata);
          continue;
        }
        if starts_divider {
          push_block(
            &mut blocks,
            &mut root_children,
            "divider",
            String::new(),
            json!({ "li": level }),
          );
          index += 1;
          continue;
        }
        // starts_code: the kept extra spaces are an indented code child.
        push_block(
          &mut blocks,
          &mut root_children,
          "code_block",
          deindent_columns(&text, 4),
          json!({ "li": level }),
        );
        index += 1;
        continue;
      }
      let raw = text.clone();
      let base = data.clone();
      let (text, data) = apply_inline_marks(text, data, &defs);
      push_block(&mut blocks, &mut root_children, kind, text, data);
      let block_idx = blocks.len() - 1;
      last_list = Some((block_idx, level, marker_char));
      item_children = false;
      open_item = Some(OpenItem {
        block_idx,
        kind: kind.to_string(),
        content_col,
        raw,
        base,
        had_blank: false,
        qdepth: 0,
        ends_hard,
      });
      index += 1;
      continue;
    }

    list_stack.clear();
    last_list = None;
    pending_loose = false;
    item_children = false;
    let (text, data) = if kind == "image" {
      // The alt is plain text — inline markup flattens (spec alt rule).
      (parse_inline_with(&text, &defs).text, data)
    } else {
      apply_inline_marks(text, data, &defs)
    };
    let raw = if kind == "paragraph" {
      Some(content.to_string())
    } else {
      None
    };
    push_block(&mut blocks, &mut root_children, kind, text, data);
    // Paragraphs stay open for lazy continuation and setext underlines.
    open_item = raw.map(|raw| OpenItem {
      block_idx: blocks.len() - 1,
      kind: "paragraph".to_string(),
      content_col: 0,
      raw,
      base: Value::Null,
      had_blank: false,
      qdepth: 0,
      ends_hard,
    });
    index += 1;
  }

  if let Some(d) = pending_empty_quote.take() {
    let mut data = if d > 1 { json!({ "quote": d }) } else { Value::Null };
    if quote_boundary {
      data_insert(&mut data, "qbreak", json!(true));
    }
    push_block(&mut blocks, &mut root_children, "quote", String::new(), data);
  }

  // Promote a paragraph that is exactly one image (e.g. a reference-form
  // `![alt][label]` on its own line) to an image block; markup inside the
  // alt flattens, the same as the direct `![alt](url)` fast path.
  for b in &mut blocks {
    if b.kind != "paragraph" {
      continue;
    }
    let full = b.text.encode_utf16().count();
    if full == 0 {
      continue;
    }
    let promoted = b.data.get("marks").and_then(Value::as_array).and_then(|ms| {
      ms.iter().find(|m| {
        m.get("type").and_then(Value::as_str) == Some("image")
          && m.get("start").and_then(Value::as_u64) == Some(0)
          && m.get("end").and_then(Value::as_u64) == Some(full as u64)
      })
    });
    if let Some(image) = promoted {
      let url = image.get("href").and_then(Value::as_str).unwrap_or_default();
      let mut data = json!({ "url": url });
      if let Some(t) = image.get("title").and_then(Value::as_str) {
        data_insert(&mut data, "title", json!(t));
      }
      b.kind = "image".to_string();
      b.data = data;
    }
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

/// Leading indentation of a line in columns (tabs = 4-column stops).
fn leading_columns(line: &str) -> usize {
  let mut col = 0usize;
  for c in line.chars() {
    match c {
      ' ' => col += 1,
      '\t' => col = (col / 4 + 1) * 4,
      _ => break,
    }
  }
  col
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
    let text = strip_atx_closing(content[level..].trim_start()).to_string();
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
  if let Some((alt, url, title)) = parse_markdown_image(content) {
    let data = match title {
      Some(t) => json!({ "url": url, "title": t }),
      None => json!({ "url": url }),
    };
    return ("image", alt, data);
  }
  if let Some(rest) = content
    .strip_prefix("- ")
    .or_else(|| content.strip_prefix("* "))
    .or_else(|| content.strip_prefix("+ "))
  {
    return ("bulleted_list", rest.to_string(), Value::Null);
  }
  if matches!(content, "-" | "*" | "+") {
    // A bare marker is an empty list item.
    return ("bulleted_list", String::new(), Value::Null);
  }
  if let Some((start, rest)) = numbered_list_marker(content) {
    let data = if start == 1 {
      Value::Null
    } else {
      json!({ "start": start })
    };
    return ("numbered_list", rest.to_string(), data);
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
  if (1..=6).contains(&hashes)
    && (content.len() == hashes || content[hashes..].starts_with([' ', '\t']))
  {
    Some(hashes)
  } else {
    None
  }
}

/// Strip an ATX closing sequence: trailing `#`s preceded by a space (or
/// making up the whole text).
fn strip_atx_closing(text: &str) -> &str {
  let t = text.trim_end();
  let trimmed = t.trim_end_matches('#');
  if trimmed.len() == t.len() {
    return t;
  }
  if trimmed.is_empty() {
    return "";
  }
  match trimmed.strip_suffix(' ') {
    Some(rest) => rest.trim_end(),
    None => t,
  }
}

/// Ordered-list marker: 1–9 digits + `.` or `)` + space (or end-of-line for
/// an empty item). Returns the start number and the content after the marker.
fn numbered_list_marker(content: &str) -> Option<(u64, &str)> {
  let digits_end = content
    .find(|c: char| !c.is_ascii_digit())
    .unwrap_or(content.len());
  if digits_end == 0 || digits_end > 9 {
    return None;
  }
  let start: u64 = content[..digits_end].parse().ok()?;
  let rest = &content[digits_end..];
  match rest.as_bytes() {
    [] => None,
    [b'.'] | [b')'] => Some((start, "")),
    [b'.', b' ', ..] | [b')', b' ', ..] => Some((start, &rest[2..])),
    _ => None,
  }
}

/// Curated named-entity table (the spec set plus common real-world names);
/// unknown entities stay literal, which is exactly the spec behavior.
fn named_entity(name: &str) -> Option<&'static str> {
  match name {
    "AElig" => Some("Æ"),
    "Alpha" => Some("Α"),
    "Beta" => Some("Β"),
    "ClockwiseContourIntegral" => Some("∲"),
    "Dagger" => Some("‡"),
    "Dcaron" => Some("Ď"),
    "Delta" => Some("Δ"),
    "DifferentialD" => Some("ⅆ"),
    "Gamma" => Some("Γ"),
    "HilbertSpace" => Some("ℋ"),
    "Lambda" => Some("Λ"),
    "OElig" => Some("Œ"),
    "Omega" => Some("Ω"),
    "Phi" => Some("Φ"),
    "Pi" => Some("Π"),
    "Psi" => Some("Ψ"),
    "Scaron" => Some("Š"),
    "Sigma" => Some("Σ"),
    "Theta" => Some("Θ"),
    "Yuml" => Some("Ÿ"),
    "aacute" => Some("á"),
    "acirc" => Some("â"),
    "acute" => Some("´"),
    "agrave" => Some("à"),
    "alpha" => Some("α"),
    "amp" => Some("&"),
    "apos" => Some("'"),
    "aring" => Some("å"),
    "asymp" => Some("≈"),
    "atilde" => Some("ã"),
    "auml" => Some("ä"),
    "beta" => Some("β"),
    "brvbar" => Some("¦"),
    "bull" => Some("•"),
    "cap" => Some("∩"),
    "ccedil" => Some("ç"),
    "cedil" => Some("¸"),
    "cent" => Some("¢"),
    "chi" => Some("χ"),
    "circ" => Some("ˆ"),
    "clubs" => Some("♣"),
    "copy" => Some("©"),
    "cup" => Some("∪"),
    "curren" => Some("¤"),
    "dagger" => Some("†"),
    "darr" => Some("↓"),
    "deg" => Some("°"),
    "delta" => Some("δ"),
    "diams" => Some("♦"),
    "divide" => Some("÷"),
    "eacute" => Some("é"),
    "ecirc" => Some("ê"),
    "egrave" => Some("è"),
    "empty" => Some("∅"),
    "emsp" => Some(" "),
    "ensp" => Some(" "),
    "epsilon" => Some("ε"),
    "equiv" => Some("≡"),
    "eta" => Some("η"),
    "euml" => Some("ë"),
    "euro" => Some("€"),
    "exist" => Some("∃"),
    "fnof" => Some("ƒ"),
    "forall" => Some("∀"),
    "frac12" => Some("½"),
    "frac14" => Some("¼"),
    "frac34" => Some("¾"),
    "frasl" => Some("⁄"),
    "gamma" => Some("γ"),
    "ge" => Some("≥"),
    "gt" => Some(">"),
    "harr" => Some("↔"),
    "hearts" => Some("♥"),
    "hellip" => Some("…"),
    "iacute" => Some("í"),
    "icirc" => Some("î"),
    "iexcl" => Some("¡"),
    "infin" => Some("∞"),
    "int" => Some("∫"),
    "iota" => Some("ι"),
    "iquest" => Some("¿"),
    "isin" => Some("∈"),
    "iuml" => Some("ï"),
    "kappa" => Some("κ"),
    "lambda" => Some("λ"),
    "lang" => Some("⟨"),
    "laquo" => Some("«"),
    "larr" => Some("←"),
    "lceil" => Some("⌈"),
    "ldquo" => Some("“"),
    "le" => Some("≤"),
    "lfloor" => Some("⌊"),
    "loz" => Some("◊"),
    "lsquo" => Some("‘"),
    "lt" => Some("<"),
    "macr" => Some("¯"),
    "mdash" => Some("—"),
    "micro" => Some("µ"),
    "middot" => Some("·"),
    "minus" => Some("−"),
    "mu" => Some("μ"),
    "nbsp" => Some(" "),
    "ndash" => Some("–"),
    "ne" => Some("≠"),
    "ngE" => Some("≧̸"),
    "not" => Some("¬"),
    "notin" => Some("∉"),
    "ntilde" => Some("ñ"),
    "nu" => Some("ν"),
    "oacute" => Some("ó"),
    "ocirc" => Some("ô"),
    "oelig" => Some("œ"),
    "omega" => Some("ω"),
    "oplus" => Some("⊕"),
    "ordf" => Some("ª"),
    "ordm" => Some("º"),
    "oslash" => Some("ø"),
    "otilde" => Some("õ"),
    "otimes" => Some("⊗"),
    "ouml" => Some("ö"),
    "para" => Some("¶"),
    "permil" => Some("‰"),
    "perp" => Some("⊥"),
    "phi" => Some("φ"),
    "pi" => Some("π"),
    "plusmn" => Some("±"),
    "pound" => Some("£"),
    "prod" => Some("∏"),
    "psi" => Some("ψ"),
    "quot" => Some("\""),
    "radic" => Some("√"),
    "rang" => Some("⟩"),
    "raquo" => Some("»"),
    "rarr" => Some("→"),
    "rceil" => Some("⌉"),
    "rdquo" => Some("”"),
    "reg" => Some("®"),
    "rfloor" => Some("⌋"),
    "rho" => Some("ρ"),
    "rsquo" => Some("’"),
    "scaron" => Some("š"),
    "sdot" => Some("⋅"),
    "sect" => Some("§"),
    "shy" => Some("­"),
    "sigma" => Some("σ"),
    "spades" => Some("♠"),
    "sub" => Some("⊂"),
    "sum" => Some("∑"),
    "sup" => Some("⊃"),
    "sup1" => Some("¹"),
    "sup2" => Some("²"),
    "sup3" => Some("³"),
    "szlig" => Some("ß"),
    "tau" => Some("τ"),
    "theta" => Some("θ"),
    "thinsp" => Some(" "),
    "tilde" => Some("˜"),
    "times" => Some("×"),
    "trade" => Some("™"),
    "uacute" => Some("ú"),
    "uarr" => Some("↑"),
    "ucirc" => Some("û"),
    "uml" => Some("¨"),
    "upsilon" => Some("υ"),
    "uuml" => Some("ü"),
    "xi" => Some("ξ"),
    "yen" => Some("¥"),
    "zeta" => Some("ζ"),
    "zwj" => Some("‍"),
    "zwnj" => Some("‌"),
    _ => None,
  }
}

/// Parse an entity/numeric character reference starting at `&` (chars[i]).
/// Returns (decoded string, chars consumed).
fn parse_entity(chars: &[char], i: usize) -> Option<(String, usize)> {
  if chars.get(i) != Some(&'&') {
    return None;
  }
  let semi = chars[i + 1..].iter().take(33).position(|&c| c == ';')? + i + 1;
  let body: String = chars[i + 1..semi].iter().collect();
  if let Some(rest) = body.strip_prefix('#') {
    let (digits, radix) = match rest.strip_prefix(['x', 'X']) {
      Some(h) => (h, 16),
      None => (rest, 10),
    };
    if digits.is_empty()
      || digits.len() > 7
      || !digits.chars().all(|c| c.is_digit(radix))
    {
      return None;
    }
    let n = u32::from_str_radix(digits, radix).ok()?;
    let c = if n == 0 { '\u{FFFD}' } else { char::from_u32(n).unwrap_or('\u{FFFD}') };
    return Some((c.to_string(), semi - i + 1));
  }
  named_entity(&body).map(|v| (v.to_string(), semi - i + 1))
}

/// GFM extended autolink starting at chars[i] (the caller checks the word
/// boundary): bare `http(s)://…`, `www.…` (href gains `http://`) or a bare
/// email. Returns (consumed chars, href).
fn extended_autolink(chars: &[char], i: usize) -> Option<(usize, String)> {
  let rest: String = chars[i..].iter().collect();
  let lower = rest.to_ascii_lowercase();
  for (prefix, implied) in [
    ("https://", ""),
    ("http://", ""),
    ("ftp://", ""),
    ("www.", "http://"),
  ] {
    if lower.starts_with(prefix) {
      // Take everything up to whitespace or '<', then trim trailing
      // punctuation per the GFM rules.
      let mut end = rest
        .char_indices()
        .find(|&(_, c)| c.is_whitespace() || c == '<')
        .map(|(k, _)| k)
        .unwrap_or(rest.len());
      end = trim_autolink_end(&rest[..end]);
      let candidate = &rest[..end];
      let domain = &candidate[if implied.is_empty() { prefix.len() } else { 0 }..];
      let domain = domain
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
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
    let mut j = i;
    while chars
      .get(j)
      .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
    {
      j += 1;
    }
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
        return Some((k - i, format!("mailto:{text}")));
      }
    }
  }
  None
}

/// GFM trailing-punctuation trimming for extended autolinks.
fn trim_autolink_end(s: &str) -> usize {
  let mut end = s.len();
  loop {
    let kept = &s[..end];
    let Some(last) = kept.chars().last() else { return 0 };
    if matches!(last, '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~' | '\'' | '"') {
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
fn valid_autolink_domain(domain: &str) -> bool {
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

/// Block-level HTML tag names (CommonMark type-6 start condition).
const HTML_BLOCK_TAGS: &[&str] = &[
  "address", "article", "aside", "base", "basefont", "blockquote", "body",
  "caption", "center", "col", "colgroup", "dd", "details", "dialog", "dir",
  "div", "dl", "dt", "fieldset", "figcaption", "figure", "footer", "form",
  "frame", "frameset", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header",
  "hr", "html", "iframe", "legend", "li", "link", "main", "menu", "menuitem",
  "nav", "noframes", "ol", "optgroup", "option", "p", "param", "search",
  "section", "summary", "table", "tbody", "td", "template", "tfoot", "th",
  "thead", "title", "tr", "track", "ul",
];

/// CommonMark HTML block start conditions. Returns the kind (1–7), which
/// picks the end condition; type 7 cannot interrupt a paragraph.
fn html_block_start(content: &str) -> Option<u8> {
  let rest = content.strip_prefix('<')?;
  if rest.starts_with("!--") {
    return Some(2);
  }
  if rest.starts_with('?') {
    return Some(3);
  }
  if rest.starts_with("![CDATA[") {
    return Some(5);
  }
  if let Some(d) = rest.strip_prefix('!') {
    if d.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
      return Some(4);
    }
    return None;
  }
  let (closing, name_rest) = match rest.strip_prefix('/') {
    Some(r) => (true, r),
    None => (false, rest),
  };
  let name_len = name_rest
    .chars()
    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
    .count();
  if name_len == 0 || !name_rest.chars().next().unwrap().is_ascii_alphabetic() {
    return None;
  }
  let name: String = name_rest[..name_len].to_ascii_lowercase();
  let after = &name_rest[name_len..];
  let boundary = after.is_empty() || after.starts_with([' ', '\t', '>']);
  if !closing
    && matches!(name.as_str(), "pre" | "script" | "style" | "textarea")
    && boundary
  {
    return Some(1);
  }
  if HTML_BLOCK_TAGS.contains(&name.as_str())
    && (boundary || (!closing && after.starts_with("/>")))
  {
    return Some(6);
  }
  // Type 7: a COMPLETE open/closing tag (any other name) alone on the line.
  let chars: Vec<char> = content.chars().collect();
  if let Some(end) = inline_html_end(&chars, 0)
    && chars[1] != '!'
    && chars[1] != '?'
    && !matches!(name.as_str(), "pre" | "script" | "style" | "textarea")
    && chars[end..].iter().all(|&c| c == ' ' || c == '\t')
  {
    return Some(7);
  }
  None
}

/// Does this line end the HTML block of the given kind (types 1–5)?
fn html_block_ends(kind: u8, line: &str) -> bool {
  let lower = line.to_ascii_lowercase();
  match kind {
    1 => {
      lower.contains("</pre>")
        || lower.contains("</script>")
        || lower.contains("</style>")
        || lower.contains("</textarea>")
    }
    2 => line.contains("-->"),
    3 => line.contains("?>"),
    4 => line.contains('>'),
    5 => line.contains("]]>"),
    _ => false,
  }
}

/// Inline raw HTML at chars[i] == '<': open tag, closing tag, comment,
/// processing instruction, declaration or CDATA. Returns the exclusive end.
fn inline_html_end(chars: &[char], i: usize) -> Option<usize> {
  if chars.get(i) != Some(&'<') {
    return None;
  }
  let starts = |from: usize, pat: &str| -> bool {
    pat.chars().enumerate().all(|(k, c)| chars.get(from + k) == Some(&c))
  };
  let find = |from: usize, pat: &str| -> Option<usize> {
    let pat: Vec<char> = pat.chars().collect();
    let mut j = from;
    while j + pat.len() <= chars.len() {
      if chars[j..j + pat.len()] == pat[..] {
        return Some(j);
      }
      j += 1;
    }
    None
  };
  // Comment (<!--> and <!---> count as empty comments).
  if starts(i + 1, "!--") {
    if starts(i + 4, ">") {
      return Some(i + 5);
    }
    if starts(i + 4, "->") {
      return Some(i + 6);
    }
    return find(i + 4, "-->").map(|p| p + 3);
  }
  if starts(i + 1, "![CDATA[") {
    return find(i + 9, "]]>").map(|p| p + 3);
  }
  if chars.get(i + 1) == Some(&'?') {
    return find(i + 2, "?>").map(|p| p + 2);
  }
  if chars.get(i + 1) == Some(&'!') {
    if !chars.get(i + 2).is_some_and(|c| c.is_ascii_alphabetic()) {
      return None;
    }
    return find(i + 2, ">").map(|p| p + 1);
  }
  let (closing, mut p) = if chars.get(i + 1) == Some(&'/') {
    (true, i + 2)
  } else {
    (false, i + 1)
  };
  if !chars.get(p).is_some_and(|c| c.is_ascii_alphabetic()) {
    return None;
  }
  while chars.get(p).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '-') {
    p += 1;
  }
  let skip_ws = |mut q: usize| -> usize {
    while chars.get(q).is_some_and(|c| c.is_whitespace()) {
      q += 1;
    }
    q
  };
  if closing {
    let q = skip_ws(p);
    return (chars.get(q) == Some(&'>')).then_some(q + 1);
  }
  loop {
    let q = skip_ws(p);
    match chars.get(q) {
      Some('>') => return Some(q + 1),
      Some('/') if chars.get(q + 1) == Some(&'>') => return Some(q + 2),
      _ => {}
    }
    if q == p {
      return None; // an attribute needs whitespace before it
    }
    // Attribute name.
    if !chars.get(q).is_some_and(|c| c.is_ascii_alphabetic() || *c == '_' || *c == ':') {
      return None;
    }
    let mut r = q + 1;
    while chars
      .get(r)
      .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
    {
      r += 1;
    }
    // Optional value.
    let eq = skip_ws(r);
    if chars.get(eq) == Some(&'=') {
      let v = skip_ws(eq + 1);
      match chars.get(v) {
        Some(&quote) if quote == '"' || quote == '\'' => {
          let mut w = v + 1;
          while chars.get(w).is_some_and(|&c| c != quote) {
            w += 1;
          }
          chars.get(w)?;
          p = w + 1;
        }
        Some(_) => {
          let mut w = v;
          while chars
            .get(w)
            .is_some_and(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '=' | '<' | '>' | '`'))
          {
            w += 1;
          }
          if w == v {
            return None;
          }
          p = w;
        }
        None => return None,
      }
    } else {
      p = r;
    }
  }
}

/// A fence opener: 3+ backticks or tildes; a backtick fence's info string
/// may not contain backticks. Returns (fence char, run length, info).
fn fence_open(content: &str) -> Option<(u8, usize, &str)> {
  let b = content.as_bytes();
  let c = *b.first()?;
  if c != b'`' && c != b'~' {
    return None;
  }
  let mut n = 0;
  while n < b.len() && b[n] == c {
    n += 1;
  }
  if n < 3 {
    return None;
  }
  let info = &content[n..];
  if c == b'`' && info.contains('`') {
    return None;
  }
  Some((c, n, info))
}

/// A fence closer: a run of the opening char at least as long, nothing else.
fn fence_close(content: &str, fence_char: u8, fence_len: usize) -> bool {
  let b = content.trim_end().as_bytes();
  b.len() >= fence_len && b.iter().all(|&x| x == fence_char)
}

/// Strip leading `>` quote markers: each consumes one optional following
/// space, and up to 3 spaces may precede the next nested marker. Returns the
/// marker depth and the remaining content.
fn strip_quote_markers(content: &str) -> (usize, &str) {
  let b = content.as_bytes();
  let mut i = 0;
  let mut depth = 0;
  loop {
    let mut j = i;
    let mut spaces = 0;
    while j < b.len() && b[j] == b' ' {
      j += 1;
      spaces += 1;
    }
    if spaces > 3 || j >= b.len() || b[j] != b'>' {
      break;
    }
    j += 1;
    if j < b.len() && b[j] == b' ' {
      j += 1;
    }
    depth += 1;
    i = j;
  }
  (depth, &content[i..])
}

/// Quote nesting depth of a block: the `quote` kind is depth ≥ 1, any other
/// kind inside a quote carries `data.quote`.
fn quote_depth_of(block: &Block) -> usize {
  let d = block.data.get("quote").and_then(Value::as_u64).unwrap_or(0) as usize;
  if block.kind == "quote" { d.max(1) } else { d }
}

/// Insert a key into a block's `data`, upgrading `Null` to an object.
fn data_insert(data: &mut Value, key: &str, value: Value) {
  match data {
    Value::Object(map) => {
      map.insert(key.into(), value);
    }
    other => *other = json!({ key: value }),
  }
}

/// Parse a line that is exactly `![alt](url "title")` into
/// `(alt, url, title)` — spec destination rules (angle brackets, balanced
/// parens, nested brackets in the alt).
fn parse_markdown_image(content: &str) -> Option<(String, String, Option<String>)> {
  let chars: Vec<char> = content.chars().collect();
  if chars.len() < 2 || chars[0] != '!' || chars[1] != '[' {
    return None;
  }
  let close = matching_bracket(&chars, 1)?;
  if chars.get(close + 1) != Some(&'(') {
    return None;
  }
  let (href, title, next) = parse_link_suffix(&chars, close + 2)?;
  if next != chars.len() {
    return None; // not the whole line — leave it to the inline parser
  }
  let alt: String = chars[2..close].iter().collect();
  Some((alt, href, title))
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
    if quote_depth_of(child) > 0 && li_of(child).is_none() {
      // Collect one quote group (a `qbreak` starts the next blockquote)
      // and rebuild the nested <blockquote> structure from `data.quote`.
      let run_start = index;
      index += 1;
      while index < child_ids.len() {
        let b = block_for(snapshot, &child_ids[index])?;
        if quote_depth_of(b) == 0 || b.data.get("qbreak").is_some() {
          break;
        }
        index += 1;
      }
      let items: Vec<&Block> = child_ids[run_start..index]
        .iter()
        .map(|id| block_for(snapshot, id))
        .collect::<DocumentOperationResult<_>>()?;
      render_quote_group(snapshot, &items, 1, out)?;
    } else if list_tag_for(&child.kind).is_some() {
      // Collect the maximal run of list items (any tag, any `data.indent`
      // level) plus their container children (`data.li`) and render the
      // nested <ul>/<ol> structure.
      let run_start = index;
      while index < child_ids.len() {
        let b = block_for(snapshot, &child_ids[index])?;
        let is_li_child = li_of(b).is_some();
        if (list_tag_for(&b.kind).is_none() && !is_li_child)
          || (quote_depth_of(b) > 0 && !is_li_child)
        {
          break;
        }
        index += 1;
      }
      let items: Vec<&Block> = child_ids[run_start..index]
        .iter()
        .map(|id| block_for(snapshot, id))
        .collect::<DocumentOperationResult<_>>()?;
      render_html_list(snapshot, &items, 0, out)?;
    } else {
      append_html_block(snapshot, child, out)?;
      index += 1;
    }
  }

  Ok(())
}

/// Render one quote group (flat blocks carrying `data.quote` depths) as
/// nested `<blockquote>`: deeper runs recurse, `quote`-kind blocks are the
/// paragraphs, list items group into lists, anything else renders normally.
fn render_quote_group(
  snapshot: &DocumentSnapshotPayload,
  items: &[&Block],
  depth: usize,
  out: &mut String,
) -> DocumentOperationResult<()> {
  out.push_str("<blockquote>\n");
  let mut i = 0;
  while i < items.len() {
    if quote_depth_of(items[i]) > depth {
      let s = i;
      while i < items.len() && quote_depth_of(items[i]) > depth {
        i += 1;
      }
      render_quote_group(snapshot, &items[s..i], depth + 1, out)?;
      continue;
    }
    if list_tag_for(&items[i].kind).is_some() {
      let s = i;
      while i < items.len()
        && quote_depth_of(items[i]) == depth
        && list_tag_for(&items[i].kind).is_some()
      {
        i += 1;
      }
      render_html_list(snapshot, &items[s..i], 0, out)?;
      continue;
    }
    if items[i].kind == "quote" {
      let text = html_inline(items[i]);
      if !text.is_empty() {
        out.push_str(&format!("<p>{text}</p>\n"));
      }
    } else {
      append_html_block(snapshot, items[i], out)?;
    }
    i += 1;
  }
  out.push_str("</blockquote>\n");
  Ok(())
}

fn list_indent(block: &Block) -> usize {
  block.data.get("indent").and_then(Value::as_u64).unwrap_or(0) as usize
}

/// Is this block a container child of a list item, and at which level?
fn li_of(block: &Block) -> Option<usize> {
  block.data.get("li").and_then(Value::as_u64).map(|v| v as usize)
}

/// The effective list level a block sits at: items by `data.indent`,
/// container children by `data.li`.
fn li_level(block: &Block) -> usize {
  match li_of(block) {
    Some(l) => l,
    None => list_indent(block),
  }
}

/// Render a flat run of list items (nesting via `data.indent`) plus their
/// container children (`data.li`) as spec-shaped nested `<ul>`/`<ol>`: a tag
/// change at the same level starts a new list, deeper items and children
/// nest inside the preceding `<li>`, a loose list (any item carrying
/// `data.loose`) wraps item text in `<p>`, and `data.start` on the first
/// item of an ordered run becomes `<ol start="n">`.
fn render_html_list(
  snapshot: &DocumentSnapshotPayload,
  items: &[&Block],
  level: usize,
  out: &mut String,
) -> DocumentOperationResult<()> {
  let mut i = 0;
  while i < items.len() {
    if li_level(items[i]) > level || (li_of(items[i]) == Some(level) && i == 0) {
      // Orphan deeper entries (no parent at this level): render a level
      // down; a leading direct child renders bare.
      let s = i;
      while i < items.len()
        && (li_level(items[i]) > level || (li_of(items[i]) == Some(level) && i == s))
      {
        i += 1;
      }
      render_html_list(snapshot, &items[s..i], level + 1, out)?;
      continue;
    }
    let tag = list_tag_for(&items[i].kind).unwrap_or("ul");
    // Extent of this list: items at `level` with the same tag, plus any
    // deeper items or container children in between.
    let mut li_heads: Vec<usize> = Vec::new();
    let mut j = i;
    while j < items.len() {
      if li_level(items[j]) == level && li_of(items[j]).is_none() {
        if list_tag_for(&items[j].kind) != Some(tag) {
          break;
        }
        // A marker/delimiter change recorded at import starts a new list.
        if !li_heads.is_empty()
          && (items[j].data.get("start").is_some() || items[j].data.get("marker").is_some())
        {
          break;
        }
        li_heads.push(j);
      }
      j += 1;
    }
    let loose = li_heads.iter().any(|&k| block_data_bool(items[k], "loose"));
    let start_attr = match items[i].data.get("start").and_then(Value::as_u64) {
      Some(n) if tag == "ol" && n != 1 => format!(" start=\"{n}\""),
      _ => String::new(),
    };
    out.push_str(&format!("<{tag}{start_attr}>\n"));
    for (k, &head) in li_heads.iter().enumerate() {
      let end = li_heads.get(k + 1).copied().unwrap_or(j);
      let mut body = html_inline(items[head]);
      // GFM task list items render a disabled checkbox before the text.
      if items[head].kind == "todo" {
        let checked = if block_data_bool(items[head], "checked") {
          "checked=\"\" "
        } else {
          ""
        };
        body = format!("<input {checked}disabled=\"\" type=\"checkbox\"> {body}");
      }
      let has_children = head + 1 < end;
      if loose && body.is_empty() && !has_children {
        out.push_str("<li></li>\n");
      } else if loose {
        out.push_str("<li>\n");
        for para in body.split("\n\n").filter(|p| !p.is_empty()) {
          out.push_str(&format!("<p>{para}</p>\n"));
        }
        if has_children {
          render_li_children(snapshot, &items[head + 1..end], level, out)?;
        }
        out.push_str("</li>\n");
      } else {
        out.push_str("<li>");
        out.push_str(&body);
        if has_children {
          out.push('\n');
          render_li_children(snapshot, &items[head + 1..end], level, out)?;
        }
        out.push_str("</li>\n");
      }
    }
    out.push_str(&format!("</{tag}>\n"));
    i = j;
  }
  Ok(())
}

/// Inside one `<li>`: direct container children (`data.li` == level) render
/// as their blocks (quote runs rebuild <blockquote>); anything deeper is a
/// nested list run.
fn render_li_children(
  snapshot: &DocumentSnapshotPayload,
  items: &[&Block],
  level: usize,
  out: &mut String,
) -> DocumentOperationResult<()> {
  let mut i = 0;
  while i < items.len() {
    let b = items[i];
    if li_of(b) == Some(level) {
      if quote_depth_of(b) > 0 {
        let s = i;
        while i < items.len() {
          let q = items[i];
          if li_of(q) != Some(level) || quote_depth_of(q) == 0 {
            break;
          }
          if i > s && q.data.get("qbreak").is_some() {
            break;
          }
          i += 1;
        }
        render_quote_group(snapshot, &items[s..i], 1, out)?;
        continue;
      }
      if b.kind == "paragraph" {
        let text = html_inline(b);
        if !text.is_empty() {
          out.push_str(&format!("<p>{text}</p>\n"));
        }
      } else {
        append_html_block(snapshot, b, out)?;
      }
      i += 1;
      continue;
    }
    // Deeper: a nested list run (items and their children).
    let s = i;
    while i < items.len() && (li_level(items[i]) > level || li_of(items[i]) == Some(level + 1)) {
      i += 1;
    }
    render_html_list(snapshot, &items[s..i], level + 1, out)?;
  }
  Ok(())
}

/// Inline marks rendered to nested HTML (<strong>/<em>/<code>/<del>/<a>),
/// mirroring the markdown render_span structure.
/// Escape inline TEXT for HTML and render hard breaks (`\` + newline in
/// the block text) as `<br />`.
fn html_text(seg: &str) -> String {
  escape_html(seg).replace("\\\n", "<br />\n")
}

fn html_inline(block: &Block) -> String {
  if matches!(block.kind.as_str(), "code_block" | "code" | "table") {
    return escape_html(&block.text);
  }
  let marks = marks_from_block(block);
  if marks.is_empty() {
    return html_text(block.text.trim());
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
      out.push_str(&html_text(&String::from_utf16_lossy(&units[pos..hi])));
      break;
    };
    out.push_str(&html_text(&String::from_utf16_lossy(&units[pos..s])));
    let m = marks[picked];
    if m.kind == "code" {
      let raw = String::from_utf16_lossy(&units[s..e]);
      out.push_str(&format!("<code>{}</code>", escape_html(&raw)));
      pos = e;
      continue;
    }
    if m.kind == "html" {
      // Raw inline HTML passes through unescaped (GFM tagfilter applied).
      out.push_str(&tagfilter(&String::from_utf16_lossy(&units[s..e])));
      pos = e;
      continue;
    }
    if m.kind == "image" {
      // The alt attribute is the PLAIN text of the span — inner marks flatten.
      let alt = String::from_utf16_lossy(&units[s..e]);
      let title_attr = match &m.title {
        Some(t) => format!(" title=\"{}\"", escape_html(t)),
        None => String::new(),
      };
      out.push_str(&format!(
        "<img src=\"{}\" alt=\"{}\"{title_attr} />",
        escape_html(&escape_href(m.href.as_deref().unwrap_or(""))),
        escape_html(&alt)
      ));
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
          escape_html(&escape_href(m.href.as_deref().unwrap_or(""))),
          escape_html(t)
        ),
        None => format!(
          "<a href=\"{}\">{body}</a>",
          escape_html(&escape_href(m.href.as_deref().unwrap_or("")))
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
      if text.is_empty() {
        out.push_str("<blockquote>\n</blockquote>\n");
      } else {
        out.push_str(&format!("<blockquote>\n<p>{text}</p>\n</blockquote>\n"));
      }
      append_html_children(snapshot, &block.id, out)?;
    }
    "code_block" | "code" => {
      if block_data_bool(block, "raw") {
        // A raw HTML block passes through (GFM tagfilter applied).
        out.push_str(&tagfilter(&block.text));
        out.push('\n');
        return Ok(());
      }
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
    "table" => {
      let Some(rows) = block.data.get("rows").and_then(Value::as_array) else {
        return Ok(());
      };
      let aligns = block.data.get("aligns").and_then(Value::as_array);
      let attr = |c: usize| -> String {
        match aligns
          .and_then(|a| a.get(c))
          .and_then(Value::as_str)
          .unwrap_or("")
        {
          "" => String::new(),
          a => format!(" align=\"{a}\""),
        }
      };
      let cell_html = |raw: &str| -> String {
        let (text, data) = apply_inline_marks(raw.to_string(), Value::Null, &RefDefs::new());
        html_inline(&Block {
          id: String::new(),
          kind: "paragraph".to_string(),
          text,
          data,
          children: Vec::new(),
        })
      };
      let row_cells = |row: &Value| -> Vec<String> {
        row
          .as_array()
          .map(|cells| {
            cells
              .iter()
              .map(|c| c.as_str().unwrap_or("").trim().to_string())
              .collect()
          })
          .unwrap_or_default()
      };
      out.push_str("<table>\n<thead>\n<tr>\n");
      if let Some(head) = rows.first() {
        for (c, cell) in row_cells(head).iter().enumerate() {
          out.push_str(&format!("<th{}>{}</th>\n", attr(c), cell_html(cell)));
        }
      }
      out.push_str("</tr>\n</thead>\n");
      if rows.len() > 1 {
        out.push_str("<tbody>\n");
        for row in rows.iter().skip(1) {
          out.push_str("<tr>\n");
          for (c, cell) in row_cells(row).iter().enumerate() {
            out.push_str(&format!("<td{}>{}</td>\n", attr(c), cell_html(cell)));
          }
          out.push_str("</tr>\n");
        }
        out.push_str("</tbody>\n");
      }
      out.push_str("</table>\n");
    }
    "divider" => {
      out.push_str("<hr />\n");
    }
    "image" => {
      let url = escape_html(&escape_href(block_data_str(block, "url").unwrap_or_default()));
      let alt = escape_html(block.text.trim());
      let title_attr = match block_data_str(block, "title") {
        Some(t) => format!(" title=\"{}\"", escape_html(t)),
        None => String::new(),
      };
      out.push_str(&format!(
        "<p><img src=\"{url}\" alt=\"{alt}\"{title_attr} /></p>\n"
      ));
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
    "bulleted_list" | "bullet_list" | "todo" => Some("ul"),
    "numbered_list" | "number_list" => Some("ol"),
    _ => None,
  }
}

/// GFM tagfilter: escape the opening `<` of the nine disallowed raw-HTML
/// tags so they render inert while everything else passes through.
fn tagfilter(s: &str) -> String {
  const BAD: &[&str] = &[
    "title", "textarea", "style", "xmp", "iframe", "noembed", "noframes",
    "script", "plaintext",
  ];
  let bytes = s.as_bytes();
  let mut out = String::with_capacity(s.len());
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'<' {
      let mut j = i + 1;
      if bytes.get(j) == Some(&b'/') {
        j += 1;
      }
      let name_start = j;
      while bytes.get(j).is_some_and(|b| b.is_ascii_alphabetic()) {
        j += 1;
      }
      let name = s[name_start..j].to_ascii_lowercase();
      let boundary = matches!(bytes.get(j), None | Some(b' ' | b'\t' | b'\n' | b'>' | b'/'));
      if BAD.contains(&name.as_str()) && boundary {
        out.push_str("&lt;");
        i += 1;
        continue;
      }
    }
    out.push(s.as_bytes()[i] as char);
    i += 1;
  }
  out
}

/// Percent-encode a URL for an href attribute (cmark's houdini set: keep
/// alphanumerics and `-_.+!*'(),%#@?=;:/,&$~`; encode the rest per UTF-8
/// byte). HTML-escape the result separately.
fn escape_href(input: &str) -> String {
  const SAFE: &[u8] = b"-_.+!*'(),%#@?=;:/,&$~";
  let mut out = String::with_capacity(input.len());
  for &b in input.as_bytes() {
    if b.is_ascii_alphanumeric() || SAFE.contains(&b) {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}

fn escape_html(input: &str) -> String {
  let mut out = String::with_capacity(input.len());
  for ch in input.chars() {
    match ch {
      '&' => out.push_str("&amp;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      '"' => out.push_str("&quot;"),
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

  let is_item = |kind: &str| {
    matches!(kind, "bulleted_list" | "bullet_list" | "numbered_list" | "number_list" | "todo")
  };
  let mut prev_was_item = false;
  let mut prev_quote = 0usize;
  for child_id in child_ids {
    let child_index = block_index(snapshot, &child_id)
      .ok_or_else(|| DocumentOperationError::BlockNotFound(child_id.clone()))?;
    let child = &snapshot.blocks[child_index];
    let quote = quote_depth_of(child);
    let li_child = li_of(child);
    // A list runs until a blank line: separate it from whatever follows so
    // the next block can't be read back as a lazy continuation. Container
    // children (`data.li`) stay inside the run.
    if prev_was_item && !is_item(&child.kind) && li_child.is_none() {
      lines.push(String::new());
    }
    // A quote group ends at depth 0 or a recorded break — a plain blank
    // line keeps the next block out of the quote on re-import.
    if prev_quote > 0 && li_child.is_none() && (quote == 0 || child.data.get("qbreak").is_some()) {
      lines.push(String::new());
    }
    // A paragraph child needs a blank before it (that's what made it a
    // child paragraph rather than a text join on import).
    if li_child.is_some() && child.kind == "paragraph" {
      lines.push(String::new());
    }
    prev_was_item = is_item(&child.kind) || li_child.is_some();
    prev_quote = if li_child.is_some() { 0 } else { quote };
    let from = lines.len();
    append_markdown_block(snapshot, child, depth, lines, images)?;
    if quote > 0 {
      // Re-prefix every emitted line with the quote markers; blanks become
      // bare `>` so paragraph breaks stay inside the quote.
      let marker = "> ".repeat(quote);
      let bare = marker.trim_end().to_string();
      for l in lines[from..].iter_mut() {
        *l = if l.is_empty() {
          bare.clone()
        } else {
          format!("{marker}{l}")
        };
      }
    }
    if let Some(level) = li_child {
      // Container children sit at the owning item's content column.
      let indent = " ".repeat(level * 4 + 4);
      for l in lines[from..].iter_mut() {
        if !l.is_empty() {
          *l = format!("{indent}{l}");
        }
      }
      // Code/divider arms append a trailing blank — drop it so a following
      // sibling item doesn't read as loose.
      while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
      }
    }
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

/// Emit one list/todo item: a leading blank if the item is loose (so the
/// list re-imports loose), the marker line, then continuation lines indented
/// to the item's content column.
fn push_list_item(lines: &mut Vec<String>, block: &Block, indent: &str, marker: &str, rich: &str) {
  if block_data_bool(block, "loose") {
    lines.push(String::new());
  }
  let pad = " ".repeat(marker.len());
  for (i, l) in rich.split('\n').enumerate() {
    if i == 0 {
      lines.push(format!("{indent}{marker}{l}"));
    } else {
      lines.push(format!("{indent}{pad}{l}"));
    }
  }
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
      if rich.contains('\n') && level <= 2 {
        // Multi-line headings only exist in setext form.
        lines.extend(rich.split('\n').map(str::to_string));
        lines.push(if level == 1 { "===" } else { "---" }.to_string());
      } else {
        lines.push(format!("{} {}", "#".repeat(level), rich.replace('\n', " ")));
      }
      lines.push(String::new());
    }
    "todo" => {
      let marker = if block_data_bool(block, "checked") {
        "x"
      } else {
        " "
      };
      push_list_item(lines, block, &indent, &format!("- [{marker}] "), rich);
    }
    "bulleted_list" | "bullet_list" => {
      let marker = block_data_str(block, "marker").unwrap_or("-");
      push_list_item(lines, block, &indent, &format!("{marker} "), rich);
    }
    "numbered_list" | "number_list" => {
      // Only a run's first item carries `start`; the rest stay `1.` (valid
      // CommonMark — ordered numbering continues regardless). A recorded
      // `marker` restores the `)` delimiter that broke the previous run.
      let n = block.data.get("start").and_then(Value::as_u64).unwrap_or(1);
      let delim = block_data_str(block, "marker").unwrap_or(".");
      push_list_item(lines, block, &indent, &format!("{n}{delim} "), rich);
    }
    "quote" => {
      // Marker prefixes are added by the children walker (depth-aware).
      lines.extend(rich.split('\n').map(str::to_string));
      lines.push(String::new());
    }
    "code_block" | "code" => {
      if block_data_bool(block, "raw") {
        // A raw HTML block writes back verbatim — no fences — so foreign
        // viewers still render the HTML.
        lines.extend(block.text.lines().map(str::to_string));
        lines.push(String::new());
      } else {
        let language = block_data_str(block, "language").unwrap_or("");
        lines.push(format!("```{language}"));
        lines.extend(block.text.lines().map(str::to_string));
        lines.push("```".to_string());
        lines.push(String::new());
      }
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
      match block_data_str(block, "title") {
        Some(t) => lines.push(format!("![{text}]({target} \"{}\")", t.replace('"', "\\\""))),
        None => lines.push(format!("![{text}]({target})")),
      }
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

/// Scan all link reference definitions, fence-aware and multi-line: the
/// destination may sit on the line after `[label]:`, and a (quoted) title
/// may follow on its own line(s). Returns the defs and the set of consumed
/// line indices.
fn collect_ref_definitions(raw_lines: &[&str]) -> (RefDefs, std::collections::HashSet<usize>) {
  let mut defs = RefDefs::new();
  let mut def_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();
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
    prev_para = true;
    i += 1;
  }
  (defs, def_lines)
}

/// `[label]:` at lines[i]; destination on the same or next line; optional
/// quoted title after the destination (same line needs whitespace between)
/// or on following line(s) — titles may span lines. Returns the parsed def
/// and how many lines it consumed.
fn parse_ref_definition_multi(
  raw_lines: &[&str],
  i: usize,
) -> Option<(String, String, Option<String>, usize)> {
  let first = raw_lines[i].trim();
  let chars: Vec<char> = first.chars().collect();
  let close = matching_bracket(&chars, 0)?;
  if chars.get(close + 1) != Some(&':') {
    return None;
  }
  let label: String = chars[1..close].iter().collect();
  if label.trim().is_empty() {
    return None;
  }
  for (k, &c) in chars[1..close].iter().enumerate() {
    if (c == '[' || c == ']') && (k == 0 || chars[k] != '\\') {
      return None;
    }
  }
  let after_colon: String = chars[close + 2..].iter().collect();
  let after_colon = after_colon.trim();
  let mut used = 1;
  let dest_line = if after_colon.is_empty() {
    // destination on the next line
    let l2 = raw_lines.get(i + 1)?.trim();
    if l2.is_empty() {
      return None;
    }
    used = 2;
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
fn parse_def_dest(s: &str) -> Option<(String, String, bool)> {
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
fn parse_def_title(
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

fn find_unescaped_char(s: &str, target: char) -> Option<usize> {
  let mut prev_backslash = false;
  for (idx, c) in s.char_indices() {
    if c == target && !prev_backslash {
      return Some(idx);
    }
    prev_backslash = c == '\\' && !prev_backslash;
  }
  None
}

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

  // Emit a resolved link or image into out/marks (text parsed recursively).
  let push_link = |kind: &str,
                   label: &str,
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
      kind: kind.to_string(),
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
      // Raw inline HTML: a valid tag/comment/PI/declaration/CDATA shape
      // passes through verbatim under an `html` mark.
      if let Some(end) = inline_html_end(&chars, i) {
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
    if chars[i] == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
      if let Some(close) = matching_bracket(&chars, i + 1) {
        let label: String = chars[i + 2..close].iter().collect();
        if !label.is_empty() {
          if close + 1 < chars.len() && chars[close + 1] == '(' {
            if let Some((href, title, next)) = parse_link_suffix(&chars, close + 2) {
              push_link("image", &label, href, title, &mut out, &mut out_len, &mut marks);
              i = next;
              continue;
            }
          }
          if !defs.is_empty() {
            let (ref_label, next) = if close + 1 < chars.len() && chars[close + 1] == '[' {
              match matching_bracket(&chars, close + 1) {
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
              push_link("image", &label, dest.clone(), title.clone(), &mut out, &mut out_len, &mut marks);
              i = next;
              continue;
            }
          }
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
              push_link("link", &label, href, title, &mut out, &mut out_len, &mut marks);
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
              push_link("link", &label, dest.clone(), title.clone(), &mut out, &mut out_len, &mut marks);
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
    if (i == 0
      || chars[i - 1].is_whitespace()
      || matches!(chars[i - 1], '*' | '_' | '~' | '('))
      && let Some((len, href)) = extended_autolink(&chars, i)
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
        let mut j = i + n;
        let mut close = None;
        while j < chars.len() {
          if chars[j] == '~' && (j == 0 || chars[j - 1] != '\\') {
            let mut m = 0;
            while j + m < chars.len() && chars[j + m] == '~' {
              m += 1;
            }
            if m == n && j > i + n {
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
          push_inner(&inner, "strike", None, defs, &mut out, &mut out_len, &mut marks);
          i = end + n;
          continue;
        }
      }
    }
    // Inline code: an N-backtick run closes only on a run of exactly N;
    // line endings become spaces; one leading+trailing space strips when
    // both exist and the content isn't all spaces. No escapes inside.
    if chars[i] == '`' {
      let mut n = 0;
      while i + n < chars.len() && chars[i + n] == '`' {
        n += 1;
      }
      let mut j = i + n;
      let mut close = None;
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
    if scheme.len() >= 2
      && scheme.len() <= 32
      && scheme.as_bytes()[0].is_ascii_alphabetic()
      && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
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
  let chars: Vec<char> = text.chars().collect();
  for (i, &c) in chars.iter().enumerate() {
    // A backslash right before a newline IS a hard break — keep it raw.
    if c == '\\' && chars.get(i + 1) == Some(&'\n') {
      out.push(c);
      continue;
    }
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
    let mut lead = escape_inline(&String::from_utf16_lossy(&units[pos..s]));
    if marks[picked].kind == "link" && lead.ends_with('!') {
      lead.pop();
      lead.push_str("\\!");
    }
    out.push_str(&lead);
    let inner: Vec<&InlineMark> = marks
      .iter()
      .enumerate()
      .filter(|&(i, m)| i != picked && m.end.min(e) > m.start.max(s))
      .map(|(_, m)| *m)
      .collect();
    let m = marks[picked];
    if m.kind == "html" {
      // Raw inline HTML writes back verbatim.
      out.push_str(&String::from_utf16_lossy(&units[s..e]));
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
        || (raw.starts_with(' ') && raw.ends_with(' ') && !raw.trim().is_empty());
      if pad {
        out.push_str(&format!("{fence} {raw} {fence}"));
      } else {
        out.push_str(&format!("{fence}{raw}{fence}"));
      }
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
        // A title-less link whose text IS its target → autolink form; a
        // bare `www.` link writes back bare (GFM re-links it on read).
        if m.title.is_none() && plain.starts_with("www.") && href == format!("http://{plain}") {
          plain.to_string()
        } else if m.title.is_none() && (href == plain || href == format!("mailto:{plain}")) {
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
      "image" => {
        let href = m.href.as_deref().unwrap_or("");
        let dest = if href.contains(char::is_whitespace) {
          format!("<{href}>")
        } else {
          href.to_string()
        };
        match &m.title {
          Some(t) => format!("![{body}]({dest} \"{}\")", t.replace('"', "\\\"")),
          None => format!("![{body}]({dest})"),
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
  let aligns = block.data.get("aligns").and_then(Value::as_array);
  let sep: Vec<&str> = (0..cols)
    .map(|c| {
      match aligns
        .and_then(|a| a.get(c))
        .and_then(Value::as_str)
        .unwrap_or("")
      {
        "center" => ":---:",
        "right" => "---:",
        "left" => ":---",
        _ => "---",
      }
    })
    .collect();
  lines.push(format!("| {} |", sep.join(" | ")));
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
