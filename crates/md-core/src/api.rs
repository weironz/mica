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
  // Resolve root children to positions while the blocks are still borrowed,
  // then move the blocks out by position. The previous loop was O(n²) (`find`
  // per child) and deep-cloned every block; P0 measured both as real cost at
  // 3k blocks. No ids are cloned into the map, and the map is dropped before
  // the move so the borrow checker allows ownership transfer.
  let index: std::collections::HashMap<&str, usize> = doc
    .blocks
    .iter()
    .enumerate()
    .map(|(position, block)| (block.id.as_str(), position))
    .collect();
  let Some(&root_position) = index.get(doc.root_block_id.as_str()) else {
    return Vec::new();
  };
  let order: Vec<usize> = doc.blocks[root_position]
    .children
    .iter()
    .filter_map(|id| index.get(id.as_str()).copied())
    .collect();
  drop(index);
  let mut slots: Vec<Option<Block>> = doc.blocks.into_iter().map(Some).collect();
  let mut out = Vec::with_capacity(order.len());
  for position in order {
    if let Some(mut block) = slots[position].take() {
      block.children.clear();
      out.push(block);
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
    // Mark-free cells are stored as inline SOURCE, which readers re-parse.
    // Keep that compact form only when the re-read is exact: a bare URL would
    // gain a link mark and edge whitespace would be trimmed, so those take the
    // object form (I3; S4 decision, option b — here so every writer agrees).
    let source = Value::String(serialize_inline(&cell.text, &[]));
    if decode_cell(&source) == *cell {
      return source;
    }
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
  crate::compat::export_markdown_with_assets(doc, &options.assets)
}

/// Serialize a block fragment (no front matter).
pub fn serialize_body(blocks: &[Block], options: &SerializeOptions) -> String {
  let doc = body_document(blocks);
  crate::compat::export_markdown_with_assets(&doc, &options.assets).unwrap_or_default()
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

/// HTML dialect. Clipboard preserves source math/Mermaid and Mica paste
/// attributes; Export keeps the existing feature-dependent rich rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HtmlProfile {
  /// Export / share-page HTML.
  #[default]
  Export,
  /// Clipboard text/html, independent of the rich `render` feature.
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
  crate::compat::set_image_srcs(&mut doc, &options.image_srcs);
  match options.profile {
    HtmlProfile::Export => crate::compat::export_html(&doc),
    HtmlProfile::Clipboard => crate::html::clipboard::render(&doc),
  }
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

/// GFM footnotes collected to one trailing section.
pub(crate) fn append_footnotes_section(snapshot: &DocumentSnapshotPayload, out: &mut String) {
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
  use crate::DocumentSnapshotPayload;
  use crate::compat::{export_html, export_markdown};

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
