//! Cross-language JSON wire format (§3). FFI and wasm forward here verbatim,
//! so both bindings answer byte-identically.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{
  HtmlOptions, HtmlProfile, ParseMode, SerializeOptions, decode_cell, encode_cell, parse_body,
  parse_inline, parse_inline_math, render_html_body, serialize_body, serialize_inline,
};
use crate::model::{Block, InlineText, Mark, MdError, SCHEMA_VERSION};

/// A block without id/children — the binding-facing shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockJson {
  /// Block kind.
  #[serde(rename = "type")]
  pub kind: String,
  /// Inline text or raw body.
  #[serde(default)]
  pub text: String,
  /// Block data; missing/null is normalized to `{}`.
  #[serde(default)]
  pub data: Value,
}

/// An inline text plus marks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InlineJson {
  /// Plain text.
  pub text: String,
  /// Marks in canonical order.
  #[serde(default)]
  pub marks: Vec<Mark>,
}

fn invalid(message: impl std::fmt::Display) -> MdError {
  MdError::InvalidInput(message.to_string())
}

fn decode_blocks(json: &str) -> Result<Vec<BlockJson>, MdError> {
  serde_json::from_str(json).map_err(invalid)
}

fn block_from_json(block: BlockJson, id: String) -> Block {
  Block {
    id,
    kind: block.kind,
    text: block.text,
    data: block.data,
    children: Vec::new(),
  }
}

fn block_to_json(block: Block) -> BlockJson {
  BlockJson {
    kind: block.kind,
    text: block.text,
    data: normalize_data(block.data),
  }
}

/// §3.4: `data` is always an object and its `marks` array is sorted by
/// `(start, end, type)`. Everything else in `data` is preserved.
fn normalize_data(mut data: Value) -> Value {
  if data.is_null() {
    return json!({});
  }
  if let Some(marks) = data.get_mut("marks").and_then(Value::as_array_mut) {
    // Sort by the same (start, end, type) key as before, but borrow the type
    // string instead of allocating one per comparison (`sort_by_key` would run
    // the closure repeatedly and allocate each time). Stable sort, so ties keep
    // their input order exactly as the previous implementation did.
    marks.sort_by(|a, b| {
      let key = |m: &Value| {
        (
          m.get("start").and_then(Value::as_u64).unwrap_or(0),
          m.get("end").and_then(Value::as_u64).unwrap_or(0),
        )
      };
      let (a_start, a_end) = key(a);
      let (b_start, b_end) = key(b);
      a_start.cmp(&b_start).then(a_end.cmp(&b_end)).then_with(|| {
        a.get("type")
          .and_then(Value::as_str)
          .unwrap_or("")
          .cmp(b.get("type").and_then(Value::as_str).unwrap_or(""))
      })
    });
  }
  // Only Value objects are canonicalized. Structs (BlockJson/InlineJson/Mark)
  // keep their declaration order, and arrays keep their semantic order. This
  // is a no-op for the default BTree-backed serde_json::Map, but also sorts
  // nested objects when another dependency enables preserve_order.
  data.sort_all_objects();
  data
}

fn inline_to_json(inline: InlineText) -> InlineJson {
  let mut marks = inline.marks;
  marks.sort_by_key(|m| (m.start, m.end, m.kind.clone()));
  InlineJson {
    text: inline.text,
    marks,
  }
}

/// Parse Markdown to `[BlockJson]`.
pub fn parse_body_json(markdown: &str, fragment: bool) -> Result<String, MdError> {
  let mode = if fragment {
    ParseMode::Fragment
  } else {
    ParseMode::Document
  };
  // Consume the parsed blocks instead of mapping over a borrow: `parse_body`
  // just produced them, so cloning every text/data value here was pure waste.
  let blocks: Vec<BlockJson> = parse_body(markdown, mode)
    .into_iter()
    .map(block_to_json)
    .collect();
  serde_json::to_string(&blocks).map_err(invalid)
}

/// Parse a batch of inline sources.
pub fn parse_inline_batch_json(sources_json: &str) -> Result<String, MdError> {
  let sources: Vec<String> = serde_json::from_str(sources_json).map_err(invalid)?;
  let out: Vec<InlineJson> = sources
    .iter()
    .map(|src| inline_to_json(parse_inline(src)))
    .collect();
  serde_json::to_string(&out).map_err(invalid)
}

/// Decode stored table cells to inline text.
pub fn decode_cells_json(cells_json: &str) -> Result<String, MdError> {
  let cells: Vec<Value> = serde_json::from_str(cells_json).map_err(invalid)?;
  let out: Vec<InlineJson> = cells
    .iter()
    .map(|cell| inline_to_json(decode_cell(cell)))
    .collect();
  serde_json::to_string(&out).map_err(invalid)
}

/// Encode inline texts as stored table cells.
pub fn encode_cells_json(inlines_json: &str) -> Result<String, MdError> {
  let inlines: Vec<InlineJson> = serde_json::from_str(inlines_json).map_err(invalid)?;
  let out: Vec<Value> = inlines
    .into_iter()
    .map(|mut inline| {
      inline
        .marks
        .sort_by_key(|m| (m.start, m.end, m.kind.clone()));
      let mut cell = encode_cell(&InlineText {
        text: inline.text,
        marks: inline.marks,
      });
      cell.sort_all_objects();
      cell
    })
    .collect();
  serde_json::to_string(&out).map_err(invalid)
}

/// Serialize a block fragment to Markdown.
pub fn serialize_body_json(blocks_json: &str, assets_json: &str) -> Result<String, MdError> {
  let blocks = decode_blocks(blocks_json)?;
  let assets: std::collections::BTreeMap<String, String> =
    serde_json::from_str(assets_json).map_err(invalid)?;
  let blocks: Vec<Block> = blocks
    .into_iter()
    .enumerate()
    .map(|(i, b)| block_from_json(b, format!("\u{1}b{i}")))
    .collect();
  Ok(serialize_body(&blocks, &SerializeOptions { assets }))
}

/// Serialize one inline text to Markdown.
pub fn serialize_inline_json(inline_json: &str) -> Result<String, MdError> {
  let inline: InlineJson = serde_json::from_str(inline_json).map_err(invalid)?;
  Ok(serialize_inline(&inline.text, &inline.marks))
}

/// Render a block fragment to HTML.
pub fn render_html_body_json(
  blocks_json: &str,
  image_srcs_json: &str,
  profile: &str,
) -> Result<String, MdError> {
  let blocks = decode_blocks(blocks_json)?;
  let image_srcs: std::collections::BTreeMap<String, String> =
    serde_json::from_str(image_srcs_json).map_err(invalid)?;
  let profile = match profile {
    "export" => HtmlProfile::Export,
    "clipboard" => HtmlProfile::Clipboard,
    other => return Err(invalid(format!("unknown HTML profile: {other}"))),
  };
  let blocks: Vec<Block> = blocks
    .into_iter()
    .enumerate()
    .map(|(i, b)| block_from_json(b, format!("\u{1}b{i}")))
    .collect();
  Ok(render_html_body(
    &blocks,
    &HtmlOptions {
      image_srcs,
      profile,
    },
  ))
}

/// Inline-math spans converted from `char` indices to UTF-16 indices.
pub fn math_run_spans_json(source: &str) -> String {
  let chars: Vec<char> = source.chars().collect();
  let mut prefix = Vec::with_capacity(chars.len() + 1);
  let mut utf16 = 0usize;
  prefix.push(0usize);
  for c in &chars {
    utf16 += c.len_utf16();
    prefix.push(utf16);
  }
  let spans: Vec<[usize; 2]> = crate::math_run_spans(source)
    .into_iter()
    .map(|(start, end)| [prefix[start], prefix[end]])
    .collect();
  serde_json::to_string(&spans).unwrap_or_else(|_| "[]".to_string())
}

/// Parse inline math to `InlineJson`.
pub fn parse_inline_math_json(source: &str) -> String {
  serde_json::to_string(&inline_to_json(parse_inline_math(source)))
    .unwrap_or_else(|_| r#"{"text":"","marks":[]}"#.to_string())
}

/// Engine build information.
pub fn engine_info_json() -> String {
  let mut info = json!({
    "crate_version": env!("CARGO_PKG_VERSION"),
    "schema_version": SCHEMA_VERSION,
    "source_fingerprint": env!("MICA_MD_SOURCE_HASH"),
  });
  info.sort_all_objects();
  info.to_string()
}

#[cfg(test)]
mod object_order_tests {
  use super::normalize_data;
  use serde_json::json;

  #[test]
  fn recursive_key_order_keeps_unknown_fields_nulls_and_array_order() {
    let value = json!({
      "unknown": {"z": null, "a": [{"z": 2, "a": 1}, null, 4]},
      "marks": [
        {"start": 2, "end": 3, "type": "bold", "extra": {"z": 2, "a": 1}},
        {"start": 0, "end": 1, "type": "italic"}
      ]
    });
    let result = normalize_data(value);
    assert_eq!(
      result.to_string(),
      concat!(
        "{\"marks\":[{\"end\":1,\"start\":0,\"type\":\"italic\"},",
        "{\"end\":3,\"extra\":{\"a\":1,\"z\":2},\"start\":2,\"type\":\"bold\"}],",
        "\"unknown\":{\"a\":[{\"a\":1,\"z\":2},null,4],\"z\":null}}"
      )
    );
    // Only top-level missing block data has the established null -> {} rule;
    // an unknown non-object value is not coerced or silently repaired.
    assert_eq!(normalize_data(json!([null, 3, 1])), json!([null, 3, 1]));
    assert_eq!(normalize_data(json!(false)), json!(false));
  }
}
