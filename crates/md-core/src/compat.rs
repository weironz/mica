//! Mica compatibility entries: the pre-SDK call shapes this engine grew up
//! with. They are whole-document snapshot wrappers over the same primitives
//! the SDK proper exposes ([`crate::parse`], [`crate::serialize`],
//! [`crate::render_html`]); nothing here is privileged. New embedders should
//! prefer those primitives directly.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::api::{ParseOptions, export_safe_tree};
use crate::model::{DocumentSnapshotPayload, MdError};

/// Parse Markdown into a block document (whole-document semantics).
pub fn import_markdown(markdown: &str, root_block_id: &str) -> DocumentSnapshotPayload {
  crate::api::parse(
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
  crate::api::parse(
    markdown,
    &ParseOptions {
      mode: crate::api::ParseMode::Fragment,
      root_id: root_block_id.to_string(),
      ..Default::default()
    },
  )
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
  crate::api::append_footnotes_section(snapshot, &mut html);

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
