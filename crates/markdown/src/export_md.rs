use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::{Block, DocumentOperationError, DocumentOperationResult, DocumentSnapshotPayload};
use crate::block_index;
use crate::block::*;
use crate::html::*;
use crate::inline::*;

/// Percent-encode a URL for an href attribute (cmark's houdini set: keep
/// alphanumerics and `-_.+!*'(),%#@?=;:/,&$~`; encode the rest per UTF-8
/// byte). HTML-escape the result separately.
pub(crate) fn escape_href(input: &str) -> String {
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

pub(crate) fn escape_html(input: &str) -> String {
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


pub(crate) fn append_markdown_children(
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
    // A callout group head carries `data.alert`: emit the GFM marker line
    // `> [!TYPE]` before its content so the blockquote re-imports as an alert.
    // The marker sits at depth 1 (alerts are top-level); the block's own lines
    // get their `> ` prefix below.
    if let Some(a) = child.data.get("alert").and_then(Value::as_str) {
      lines.push(format!("> [!{}]", a.to_ascii_uppercase()));
    }
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

pub(crate) fn append_markdown_block(
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
pub(crate) fn push_list_item(lines: &mut Vec<String>, block: &Block, indent: &str, marker: &str, rich: &str) {
  if block_data_bool(block, "loose") {
    lines.push(String::new());
  }
  let pad = " ".repeat(marker.len());
  for (i, l) in rich.split('\n').enumerate() {
    if i == 0 {
      lines.push(format!("{indent}{marker}{l}"));
    } else {
      // A continuation line lands ON the item's content column, so a line that
      // merely LOOKS like a block leader starts a real block on re-import.
      // CommonMark ex. 312 imports `    - e` under `   - d` as the literal text
      // `d\n- e`; writing it back unescaped produced `  - e`, a nested item —
      // export(import(md)) stopped parsing to the blocks it came from. The
      // paragraph path has escaped its leader all along (that is how the `===`
      // setext drift was fixed); item continuations never did.
      lines.push(format!("{indent}{pad}{}", escape_block_leader(l.to_string())));
    }
  }
}

pub(crate) fn append_markdown_block_content(
  block: &Block,
  depth: usize,
  lines: &mut Vec<String>,
  images: &BTreeMap<String, String>,
) {
  // List/todo nesting from `data.indent` (4 spaces per level — valid
  // CommonMark continuation for both `- ` and `1. ` parents); other kinds
  // keep the children-tree depth indent. `list_indent` applies the same 0..=8
  // cap the Dart editor uses, so both engines indent identically (P1-4).
  let level = list_indent(block);
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
    "math_block" => {
      lines.push("$$".to_string());
      lines.extend(block.text.lines().map(str::to_string));
      lines.push("$$".to_string());
      lines.push(String::new());
    }
    "footnote_def" => {
      // `[^label]: content`; continuation lines indent 4 columns (GFM). The
      // content carries inline marks, so use the rich render, not raw text.
      let label = block_data_str(block, "label").unwrap_or("");
      let mut body = rich.lines();
      let first = body.next().unwrap_or("");
      lines.push(format!("[^{label}]: {first}"));
      for cont in body {
        lines.push(if cont.is_empty() {
          String::new()
        } else {
          format!("    {cont}")
        });
      }
      lines.push(String::new());
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


/// Encode a table cell from its raw inline-Markdown source into the unified
/// stored form: a cell with inline marks becomes `{"text","marks"}` (matching
/// paragraphs); a mark-free cell stays the plain source string (keeps simple
/// tables and the conformance golds byte-identical).
pub(crate) fn encode_table_cell(raw: &str) -> Value {
  let (text, data) = apply_inline_marks(raw.to_string(), Value::Null, &RefDefs::new());
  match data.get("marks").and_then(Value::as_array) {
    Some(marks) if !marks.is_empty() => json!({ "text": text, "marks": marks }),
    _ => Value::String(raw.to_string()),
  }
}

/// Decode a stored table cell to `(plain_text, data-with-marks)`. A cell is
/// either a JSON string — legacy / mark-free raw inline-Markdown source, parsed
/// on read so old documents keep rendering their `**bold**` — or the unified
/// `{"text","marks"}` object. `null`/other → empty (a missing cell must never
/// leak the literal word "null").
pub(crate) fn decode_table_cell(cell: &Value) -> (String, Value) {
  match cell {
    Value::String(s) => apply_inline_marks(s.clone(), Value::Null, &RefDefs::new()),
    Value::Object(map) => {
      let text = map.get("text").and_then(Value::as_str).unwrap_or("").to_string();
      let mut data = serde_json::Map::new();
      if let Some(marks) = map.get("marks")
        && marks.as_array().is_some_and(|a| !a.is_empty())
      {
        data.insert("marks".into(), marks.clone());
      }
      (text, Value::Object(data))
    }
    _ => (String::new(), Value::Null),
  }
}

/// Render a decoded cell's `(text, marks)` back to inline Markdown, then apply
/// GFM cell escaping: literal `|` → `\|`, and a soft line break (which a
/// single-line pipe row can't hold) → `<br>`.
pub(crate) fn table_cell_markdown(cell: &Value) -> String {
  let (text, data) = decode_table_cell(cell);
  let md = render_inline(&Block {
    id: String::new(),
    kind: "paragraph".to_string(),
    text,
    data,
    children: Vec::new(),
  });
  md.replace('|', "\\|").replace('\n', "<br>").trim().to_string()
}

pub(crate) fn append_table_markdown(block: &Block, lines: &mut Vec<String>) {
  let Some(rows) = block.data.get("rows").and_then(Value::as_array) else {
    return;
  };
  let grid: Vec<Vec<String>> = rows
    .iter()
    .filter_map(|row| {
      row
        .as_array()
        .map(|cells| cells.iter().map(table_cell_markdown).collect())
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

pub(crate) fn heading_level(block: &Block, depth: usize) -> usize {
  block
    .data
    .get("level")
    .and_then(Value::as_u64)
    .map(|level| level as usize)
    .unwrap_or(depth + 1)
    .clamp(1, 6)
}

pub(crate) fn block_data_str<'a>(block: &'a Block, key: &str) -> Option<&'a str> {
  block.data.get(key).and_then(Value::as_str)
}

pub(crate) fn block_data_bool(block: &Block, key: &str) -> bool {
  block
    .data
    .get(key)
    .and_then(Value::as_bool)
    .unwrap_or(false)
}




