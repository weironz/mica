//! The engine's public API: Markdown ⇄ blocks, inline text, and HTML.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Value, json};

use crate::html::html::html_inline;
use crate::ids::BlockIds;
use crate::model::{Block, Document, DocumentSnapshotPayload, InlineText, Mark, MdError};
use crate::serialize::export_md::{block_data_str, decode_table_cell};

/// Whole-document parse plus front matter, or a fragment where a leading `---`
/// is a thematic break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseMode {
  /// Whole document: leading `---` fence is YAML front matter.
  Document,
  /// Fragment: leading `---` is always a thematic break.
  Fragment,
}

/// Options for [`parse`].
#[derive(Debug, Clone)]
pub struct ParseOptions {
  /// Whole document or fragment.
  pub mode: ParseMode,
  /// Id for the root block.
  pub root_id: String,
  /// How generated block ids are formed.
  pub ids: BlockIds,
}

impl Default for ParseOptions {
  fn default() -> Self {
    ParseOptions {
      mode: ParseMode::Document,
      root_id: "root".to_string(),
      #[cfg(feature = "uuid")]
      ids: BlockIds::Uuid,
      #[cfg(not(feature = "uuid"))]
      ids: BlockIds::Sequential {
        prefix: "b".to_string(),
      },
    }
  }
}

/// Parse Markdown into a block document.
pub fn parse(markdown: &str, options: &ParseOptions) -> Document {
  crate::parse::block::import_markdown_inner(
    markdown,
    &options.root_id,
    options.mode == ParseMode::Document,
    &options.ids,
  )
}

/// Parse Markdown into a block document (whole-document semantics).
pub fn import_markdown(markdown: &str, root_block_id: &str) -> DocumentSnapshotPayload {
  parse(
    markdown,
    &ParseOptions {
      root_id: root_block_id.to_string(),
      ..Default::default()
    },
  )
}

/// Parse a Markdown fragment: a leading `---` is a thematic break, never front
/// matter.
pub fn import_markdown_fragment(markdown: &str, root_block_id: &str) -> DocumentSnapshotPayload {
  parse(
    markdown,
    &ParseOptions {
      mode: ParseMode::Fragment,
      root_id: root_block_id.to_string(),
      ..Default::default()
    },
  )
}

/// Parse only the body: the root's children, with their `children` cleared.
/// Front matter is dropped. Use for paste and AI insertion.
pub fn parse_body(markdown: &str, mode: ParseMode) -> Vec<Block> {
  let doc = parse(
    markdown,
    &ParseOptions {
      mode,
      root_id: "root".to_string(),
      ids: BlockIds::Sequential {
        prefix: "b".to_string(),
      },
    },
  );
  let mut out = Vec::new();
  if let Some(root) = doc.blocks.iter().find(|b| b.id == doc.root_block_id) {
    for id in &root.children {
      if let Some(block) = doc.blocks.iter().find(|b| &b.id == id) {
        let mut block = block.clone();
        block.children.clear();
        out.push(block);
      }
    }
  }
  out
}

/// Parse inline Markdown into text plus marks (UTF-16 offsets).
pub fn parse_inline(source: &str) -> InlineText {
  let parsed =
    crate::parse::inline::parse_inline_with(source, &crate::parse::inline::RefDefs::new());
  let mut marks: Vec<Mark> = parsed
    .marks
    .into_iter()
    .map(|m| Mark {
      start: m.start,
      end: m.end,
      kind: m.kind,
      href: m.href,
      title: m.title,
    })
    .collect();
  marks.sort_by_key(|m| (m.start, m.end, m.kind.clone()));
  InlineText {
    text: parsed.text,
    marks,
  }
}

/// Decode a stored table cell (string source, or `{text, marks}` object).
pub fn decode_cell(cell: &Value) -> InlineText {
  let (text, data) = decode_table_cell(cell);
  let marks = data
    .get("marks")
    .and_then(Value::as_array)
    .map(|marks| {
      marks
        .iter()
        .filter_map(|m| serde_json::from_value::<Mark>(m.clone()).ok())
        .collect::<Vec<_>>()
    })
    .unwrap_or_default();
  InlineText { text, marks }
}

/// Encode an inline text as a stored table cell: object when marked, escaped
/// Markdown source string when not.
pub fn encode_cell(cell: &InlineText) -> Value {
  if cell.marks.is_empty() {
    return Value::String(serialize_inline(&cell.text, &[]));
  }
  json!({ "text": cell.text, "marks": cell.marks })
}

/// Options for serialization.
#[derive(Debug, Clone, Default)]
pub struct SerializeOptions {
  /// file_id → Markdown image destination.
  pub assets: BTreeMap<String, String>,
}

/// Serialize a document to Markdown.
pub fn serialize(doc: &Document, options: &SerializeOptions) -> Result<String, MdError> {
  export_markdown_with_assets(doc, &options.assets)
}

/// Serialize a block fragment (no front matter).
pub fn serialize_body(blocks: &[Block], options: &SerializeOptions) -> String {
  let doc = body_document(blocks);
  export_markdown_with_assets(&doc, &options.assets).unwrap_or_default()
}

/// Render plain text plus marks back to inline Markdown.
pub fn serialize_inline(text: &str, marks: &[Mark]) -> String {
  let data = if marks.is_empty() {
    Value::Null
  } else {
    json!({ "marks": marks })
  };
  crate::parse::inline::render_inline(&Block {
    id: String::new(),
    kind: "paragraph".to_string(),
    text: text.to_string(),
    data,
    children: Vec::new(),
  })
}

/// HTML dialect. In S1 `Clipboard` renders identically to `Export`; the
/// clipboard dialect is implemented by S3 T3.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HtmlProfile {
  /// Export / share-page HTML.
  #[default]
  Export,
  /// Clipboard text/html (S3 implementation; same as `Export` for now).
  Clipboard,
}

/// Options for HTML rendering.
#[derive(Debug, Clone, Default)]
pub struct HtmlOptions {
  /// file_id → `<img src>`.
  pub image_srcs: BTreeMap<String, String>,
  /// Output dialect.
  pub profile: HtmlProfile,
}

/// Render a document to HTML.
pub fn render_html(doc: &Document, options: &HtmlOptions) -> Result<String, MdError> {
  let mut doc = doc.clone();
  set_image_srcs(&mut doc, &options.image_srcs);
  export_html(&doc)
}

/// Render a block fragment to HTML.
pub fn render_html_body(blocks: &[Block], options: &HtmlOptions) -> String {
  render_html(&body_document(blocks), options).unwrap_or_default()
}

/// Delimiter-inclusive `char` spans of inline math runs.
pub use crate::parse::inline::math_run_spans;

/// Weave `$…$` / `\(…\)` runs into `math` marks, leaving all other text
/// verbatim. Offsets in the result are UTF-16.
pub fn parse_inline_math(source: &str) -> InlineText {
  let chars: Vec<char> = source.chars().collect();
  let spans = math_run_spans(source);
  let mut out = String::with_capacity(source.len());
  let mut marks = Vec::new();
  let mut last = 0usize;
  let mut utf16 = 0usize;
  let push_verbatim = |out: &mut String, utf16: &mut usize, slice: &[char]| {
    out.extend(slice);
    *utf16 += slice.iter().map(|c| c.len_utf16()).sum::<usize>();
  };
  for (start, end) in spans {
    // `$$…$$` is a display block; an opener preceded by `$` is not inline math.
    if chars[start] == '$' && start > 0 && chars[start - 1] == '$' {
      continue;
    }
    push_verbatim(&mut out, &mut utf16, &chars[last..start]);
    let (lo, hi) = if chars[start] == '\\' {
      (start + 2, end - 2)
    } else {
      (start + 1, end - 1)
    };
    let raw: String = chars[lo..hi].iter().collect();
    let inner = if chars[start] == '\\' {
      raw.trim().to_string()
    } else {
      raw
    };
    if inner.is_empty() {
      push_verbatim(&mut out, &mut utf16, &chars[start..end]);
      last = end;
      continue;
    }
    let n: usize = inner.chars().map(|c| c.len_utf16()).sum();
    out.push_str(&inner);
    marks.push(Mark {
      start: utf16,
      end: utf16 + n,
      kind: "math".to_string(),
      href: None,
      title: None,
    });
    utf16 += n;
    last = end;
  }
  push_verbatim(&mut out, &mut utf16, &chars[last..]);
  InlineText { text: out, marks }
}

/// Build a synthetic document whose root's children are `blocks`.
fn body_document(blocks: &[Block]) -> DocumentSnapshotPayload {
  let mut doc_blocks = Vec::with_capacity(blocks.len() + 1);
  let children: Vec<String> = (0..blocks.len()).map(|i| format!("\u{1}b{i}")).collect();
  doc_blocks.push(Block {
    id: "\u{1}root".to_string(),
    kind: "page".to_string(),
    text: String::new(),
    data: Value::Null,
    children: children.clone(),
  });
  for (i, block) in blocks.iter().enumerate() {
    let mut block = block.clone();
    block.id = children[i].clone();
    block.children.clear();
    doc_blocks.push(block);
  }
  DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id: "\u{1}root".to_string(),
    blocks: doc_blocks,
  }
}

/// Serialize a document to Markdown.
pub fn export_markdown(snapshot: &DocumentSnapshotPayload) -> Result<String, MdError> {
  export_markdown_with_assets(snapshot, &BTreeMap::new())
}

/// Serialize a document, rewriting image destinations from `images`.
pub fn export_markdown_with_assets(
  snapshot: &DocumentSnapshotPayload,
  images: &BTreeMap<String, String>,
) -> Result<String, MdError> {
  if snapshot.schema_version != 1 {
    return Err(MdError::UnsupportedSchemaVersion(snapshot.schema_version));
  }
  let (safe, index) = export_safe_tree(snapshot);
  let snapshot = &*safe;

  let mut lines = Vec::new();
  let root_index = index
    .get(snapshot.root_block_id.as_str())
    .copied()
    .ok_or_else(|| MdError::BlockNotFound(snapshot.root_block_id.clone()))?;
  let root = &snapshot.blocks[root_index];

  if let Some(fm) = root.data.get("front_matter").and_then(Value::as_str) {
    lines.push("---".to_string());
    if !fm.is_empty() {
      lines.extend(fm.split('\n').map(str::to_string));
    }
    lines.push("---".to_string());
  }

  if !root.text.trim().is_empty() {
    crate::serialize::export_md::append_markdown_block_content(root, 0, &mut lines, images);
  }

  crate::serialize::export_md::append_markdown_children(
    snapshot,
    &index,
    &snapshot.root_block_id,
    0,
    &mut lines,
    images,
  )?;

  while lines.last().is_some_and(|line: &String| line.is_empty()) {
    lines.pop();
  }

  Ok(lines.join("\n"))
}

/// Render a document to its HTML body fragment.
pub fn export_html(snapshot: &DocumentSnapshotPayload) -> Result<String, MdError> {
  if snapshot.schema_version != 1 {
    return Err(MdError::UnsupportedSchemaVersion(snapshot.schema_version));
  }
  let (safe, index) = export_safe_tree(snapshot);
  let snapshot = &*safe;

  let mut html = String::new();
  let root_index = index
    .get(snapshot.root_block_id.as_str())
    .copied()
    .ok_or_else(|| MdError::BlockNotFound(snapshot.root_block_id.clone()))?;
  let root = &snapshot.blocks[root_index];
  if !root.text.trim().is_empty() {
    html.push_str(&format!(
      "<p>{}</p>\n",
      crate::serialize::export_md::escape_html(root.text.trim())
    ));
  }

  crate::html::html::append_html_children(snapshot, &index, &snapshot.root_block_id, &mut html)?;
  append_footnotes_section(snapshot, &mut html);

  Ok(html.trim_end().to_string())
}

/// Point each image block's `url` at the matching entry in `srcs`.
pub fn set_image_srcs(snapshot: &mut DocumentSnapshotPayload, srcs: &BTreeMap<String, String>) {
  for block in &mut snapshot.blocks {
    if block.kind != "image" {
      continue;
    }
    let Some(file_id) = block.data.get("file_id").and_then(Value::as_str) else {
      continue;
    };
    if let Some(src) = srcs.get(file_id) {
      if let Value::Object(map) = &mut block.data {
        map.insert("url".to_string(), Value::String(src.clone()));
      }
    }
  }
}

/// Split a leading YAML front matter block off `markdown`.
///
/// Exposed because the Mica adapter's page-title writer/reader must agree with
/// the parser on what counts as front matter rather than restate the rule.
pub fn split_front_matter(markdown: &str) -> (Option<String>, &str) {
  crate::parse::block::split_front_matter(markdown)
}

/// GFM footnotes collected to one trailing section.
fn append_footnotes_section(snapshot: &DocumentSnapshotPayload, out: &mut String) {
  let defs: Vec<&Block> = snapshot
    .blocks
    .iter()
    .filter(|b| b.kind == "footnote_def")
    .collect();
  if defs.is_empty() {
    return;
  }
  out.push_str("<section class=\"footnotes\">\n<ol>\n");
  for def in defs {
    let label = block_data_str(def, "label").unwrap_or("");
    let id = crate::serialize::export_md::escape_html(label);
    let body = html_inline(def);
    out.push_str(&format!(
      "<li id=\"fn-{id}\">\n<p>{body} <a href=\"#fnref-{id}\" class=\"footnote-backref\">↩</a></p>\n</li>\n"
    ));
  }
  out.push_str("</ol>\n</section>\n");
}

/// Deepest `children` nesting the exporters recurse into.
const MAX_EXPORT_DEPTH: usize = 128;

/// The block tree as the recursive exporters need it: every block reached
/// once, no deeper than [`MAX_EXPORT_DEPTH`].
pub(crate) fn export_safe_tree(
  snapshot: &DocumentSnapshotPayload,
) -> (Cow<'_, DocumentSnapshotPayload>, HashMap<&str, usize>) {
  let index: HashMap<&str, usize> = snapshot
    .blocks
    .iter()
    .enumerate()
    .map(|(i, b)| (b.id.as_str(), i))
    .collect();
  let Some(&root) = index.get(snapshot.root_block_id.as_str()) else {
    return (Cow::Borrowed(snapshot), index);
  };

  let well_formed = {
    let mut seen: HashSet<&str> = HashSet::from([snapshot.root_block_id.as_str()]);
    let mut stack = vec![(root, 0usize)];
    let mut ok = true;
    'walk: while let Some((i, depth)) = stack.pop() {
      for child in &snapshot.blocks[i].children {
        if !seen.insert(child.as_str()) || depth + 1 > MAX_EXPORT_DEPTH {
          ok = false;
          break 'walk;
        }
        if let Some(&c) = index.get(child.as_str()) {
          stack.push((c, depth + 1));
        }
      }
    }
    ok
  };
  if well_formed {
    return (Cow::Borrowed(snapshot), index);
  }

  let mut children: Vec<Vec<String>> = vec![Vec::new(); snapshot.blocks.len()];
  let mut seen: HashSet<&str> = HashSet::from([snapshot.root_block_id.as_str()]);
  let mut stack = vec![(root, 0usize)];
  while let Some((i, depth)) = stack.pop() {
    for child in &snapshot.blocks[i].children {
      if !seen.insert(child.as_str()) {
        continue;
      }
      let Some(&c) = index.get(child.as_str()) else {
        children[i].push(child.clone());
        continue;
      };
      children[i].push(child.clone());
      if depth + 1 < MAX_EXPORT_DEPTH {
        stack.push((c, depth + 1));
        continue;
      }
      let mut branch = vec![c];
      while let Some(b) = branch.pop() {
        if b != c {
          children[i].push(snapshot.blocks[b].id.clone());
        }
        for grandchild in snapshot.blocks[b].children.iter().rev() {
          if seen.insert(grandchild.as_str())
            && let Some(&g) = index.get(grandchild.as_str())
          {
            branch.push(g);
          }
        }
      }
    }
  }
  let mut repaired = snapshot.clone();
  for (block, kids) in repaired.blocks.iter_mut().zip(children) {
    block.children = kids;
  }
  (Cow::Owned(repaired), index)
}

#[cfg(test)]
mod tree_shape_tests {
  use super::{export_html, export_markdown};
  use crate::DocumentSnapshotPayload;

  fn doc(blocks: serde_json::Value) -> DocumentSnapshotPayload {
    serde_json::from_value(serde_json::json!({
      "schema_version": 1, "root_block_id": "root", "blocks": blocks,
    }))
    .unwrap()
  }

  /// THE BUG: `children` comes from a CRDT any member can write; a block that
  /// lists itself sent the recursive exporters into a stack overflow.
  #[test]
  fn a_cycle_in_children_exports_instead_of_overflowing() {
    let snapshot = doc(serde_json::json!([
      {"id": "root", "type": "page", "text": "", "data": {}, "children": ["a"]},
      {"id": "a", "type": "quote", "text": "loop", "data": {}, "children": ["a", "root"]},
    ]));
    assert!(export_html(&snapshot).unwrap().contains("loop"));
    assert!(export_markdown(&snapshot).unwrap().contains("loop"));
  }

  /// A chain far deeper than any real document keeps all its text.
  #[test]
  fn a_very_deep_chain_is_flattened_not_lost() {
    let n = 5000;
    let mut blocks = vec![serde_json::json!(
      {"id": "root", "type": "page", "text": "", "data": {}, "children": ["b0"]}
    )];
    for i in 0..n {
      let children = if i + 1 < n {
        vec![format!("b{}", i + 1)]
      } else {
        vec![]
      };
      blocks.push(serde_json::json!(
        {"id": format!("b{i}"), "type": "quote", "text": format!("t{i}"), "data": {}, "children": children}
      ));
    }
    let md = export_markdown(&doc(serde_json::Value::Array(blocks))).unwrap();
    assert!(md.contains("t0") && md.contains(&format!("t{}", n - 1)));
  }
}
