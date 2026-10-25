//! Surgical text edits over the stored text/UTF-16 marks, including table cells.

use serde_json::{Value, json};

use crate::export_md::decode_table_cell;
use crate::inline::{RefDefs, apply_inline_marks};
use crate::{Block, import_markdown_fragment};

/// Accept a single block's Markdown as a find string as well as visible text.
/// This removes inline delimiters and list/heading markers, never joins blocks.
pub fn markdown_find_text(find: &str) -> Option<String> {
  let parsed = import_markdown_fragment(find, "find_root");
  let root = parsed.blocks.iter().find(|b| b.id == "find_root")?;
  if root.children.len() != 1 {
    return None;
  }
  let block = parsed.blocks.iter().find(|b| b.id == root.children[0])?;
  if !matches!(
    block.kind.as_str(),
    "paragraph" | "heading" | "bulleted_list" | "numbered_list" | "todo" | "quote"
  ) || block.text.is_empty()
    || block.text == find
  {
    return None;
  }
  Some(block.text.clone())
}

/// Count the same non-overlapping occurrences the edit will change.
pub fn block_text_matches(block: &Block, find: &str) -> usize {
  if find.is_empty() {
    return 0;
  }
  if block.kind != "table" {
    return block.text.matches(find).count();
  }
  block
    .data
    .get("rows")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(Value::as_array)
    .flatten()
    .map(|cell| decode_table_cell(cell).0.matches(find).count())
    .sum()
}

/// A short visible-text label for identifying blocks without counting positions.
pub fn block_text_preview(block: &Block, limit: usize) -> String {
  let text = if block.kind == "table" {
    block
      .data
      .get("rows")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .filter_map(Value::as_array)
      .flatten()
      .scan(0, |length, cell| {
        if *length >= limit {
          return None;
        }
        let text = decode_table_cell(cell).0;
        *length += text.chars().count() + 1;
        Some(text)
      })
      .collect::<Vec<_>>()
      .join(" ")
  } else {
    block.text.clone()
  };
  text
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
    .chars()
    .take(limit)
    .collect()
}

/// Return a modified copy; block identity, children and unrelated data survive.
/// Markdown applies only to the replacement's inline syntax. Code stays literal.
pub fn replace_block_text(block: &Block, find: &str, replacement: &str, markdown: bool) -> Block {
  let mut edited = block.clone();
  if find.is_empty() {
    return edited;
  }
  if block.kind == "table" {
    if let Some(rows) = edited.data.get_mut("rows").and_then(Value::as_array_mut) {
      for row in rows {
        if let Some(cells) = row.as_array_mut() {
          for cell in cells {
            let (text, data) = decode_table_cell(cell);
            if !text.contains(find) {
              continue;
            }
            let (text, data) = replace_inline(&text, &data, find, replacement, markdown);
            // Strings are legacy raw Markdown, so use the object form even for
            // plain cells: literal backticks must not be reparsed on next read.
            let mut updated = cell.as_object().cloned().unwrap_or_default();
            updated.insert("text".into(), Value::String(text));
            updated.insert(
              "marks".into(),
              data.get("marks").cloned().unwrap_or(json!([])),
            );
            *cell = Value::Object(updated);
          }
        }
      }
    }
  } else if matches!(block.kind.as_str(), "code" | "code_block" | "math_block") {
    edited.text = block.text.replace(find, replacement);
  } else {
    (edited.text, edited.data) =
      replace_inline(&block.text, &block.data, find, replacement, markdown);
  }
  edited
}

fn replace_inline(
  text: &str,
  data: &Value,
  find: &str,
  replacement: &str,
  markdown: bool,
) -> (String, Value) {
  let (insert, insert_data) = if markdown {
    apply_inline_marks(replacement.to_string(), Value::Null, &RefDefs::new())
  } else {
    (replacement.to_string(), Value::Null)
  };
  let insert_marks = insert_data.get("marks").and_then(Value::as_array);
  let mut marks = data
    .get("marks")
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
  let mut result = text.to_string();
  // Right to left keeps every remaining match's byte and UTF-16 offsets valid.
  for (start, matched) in text
    .match_indices(find)
    .collect::<Vec<_>>()
    .into_iter()
    .rev()
  {
    let lo = text[..start].encode_utf16().count() as u64;
    let hi = lo + matched.encode_utf16().count() as u64;
    let n = insert.encode_utf16().count() as u64;
    let mut shifted = Vec::new();
    for mut mark in marks {
      let Some((s, e)) = mark
        .get("start")
        .and_then(Value::as_u64)
        .zip(mark.get("end").and_then(Value::as_u64))
      else {
        continue;
      };
      let (s, e) = if e <= lo {
        (s, e)
      } else if s >= hi {
        (s - (hi - lo) + n, e - (hi - lo) + n)
      } else if s <= lo && e >= hi {
        // An explicitly formatted replacement supersedes an exact old mark;
        // enclosing formatting (e.g. a whole linked sentence) remains intact.
        if s == lo && e == hi && insert_marks.is_some() {
          continue;
        }
        (s, e - (hi - lo) + n)
      } else if s < lo {
        (s, lo)
      } else if e > hi {
        (lo + n, e - (hi - lo) + n)
      } else {
        continue;
      };
      if s < e {
        mark["start"] = json!(s);
        mark["end"] = json!(e);
        shifted.push(mark);
      }
    }
    if let Some(insert_marks) = insert_marks {
      for mark in insert_marks {
        let mut mark = mark.clone();
        mark["start"] = json!(lo + mark["start"].as_u64().unwrap_or(0));
        mark["end"] = json!(lo + mark["end"].as_u64().unwrap_or(0));
        shifted.push(mark);
      }
    }
    marks = shifted;
    result.replace_range(start..start + matched.len(), &insert);
  }
  let mut data = data.clone();
  if !marks.is_empty() || data.get("marks").is_some() {
    if !data.is_object() {
      data = json!({});
    }
    marks.sort_by_key(|m| (m["start"].as_u64(), m["end"].as_u64()));
    data["marks"] = Value::Array(marks);
  }
  (result, data)
}
