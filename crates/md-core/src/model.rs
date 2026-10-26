//! The block document model: a root block plus a flat block table.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The document schema version this engine reads and writes.
pub const SCHEMA_VERSION: i32 = 1;

/// A document: one root block plus a flat block table. Field names and serde
/// shape match the payload Mica has always stored, so this is the same type,
/// moved rather than copied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentSnapshotPayload {
  /// Schema version; only [`SCHEMA_VERSION`] is accepted by the exporters.
  pub schema_version: i32,
  /// Id of the root block (usually `"root"`).
  pub root_block_id: String,
  /// Every block, root included, in document order.
  pub blocks: Vec<Block>,
}

/// The document type; the historical name is kept for callers.
pub type Document = DocumentSnapshotPayload;

/// One flat block. Structure that would be nesting in other models lives in
/// `data` (`indent`, `quote`, `li`, …); `children` is only used by the root or
/// by blocks the editor explicitly nests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
  /// Stable block id.
  pub id: String,
  /// Block kind; see [`crate::schema::kinds`].
  #[serde(rename = "type")]
  pub kind: String,
  /// Inline text, or the raw body for code/math blocks.
  #[serde(default)]
  pub text: String,
  /// Block-specific attributes (image `url`, heading `level`, …).
  /// Defaults to `null` and is omitted from output when empty.
  #[serde(default, skip_serializing_if = "Value::is_null")]
  pub data: Value,
  /// Child block ids.
  #[serde(default)]
  pub children: Vec<String>,
}

impl Block {
  /// A block with just an id and kind — the seed the builders below grow.
  pub fn new(id: impl Into<String>, kind: impl Into<String>) -> Self {
    Block {
      id: id.into(),
      kind: kind.into(),
      text: String::new(),
      data: Value::Null,
      children: Vec::new(),
    }
  }

  /// Set the block's inline text.
  pub fn with_text(mut self, text: impl Into<String>) -> Self {
    self.text = text.into();
    self
  }

  /// Set the block's data object.
  pub fn with_data(mut self, data: Value) -> Self {
    self.data = data;
    self
  }

  /// Set the block's child ids.
  pub fn with_children(mut self, children: Vec<String>) -> Self {
    self.children = children;
    self
  }
}

/// An inline mark over a `[start, end)` range of the plain text.
///
/// Offsets are **UTF-16 code units**, matching Dart and JS strings. Marks are
/// stored in `(start, end, kind)` order — the canonical order (§3.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
  /// Inclusive UTF-16 start.
  pub start: usize,
  /// Exclusive UTF-16 end.
  pub end: usize,
  /// Mark type; see [`crate::schema::marks`].
  #[serde(rename = "type")]
  pub kind: String,
  /// Link destination, for `link`/`image`/`footnote`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub href: Option<String>,
  /// Link title, when present.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
}

/// Plain text plus inline marks.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InlineText {
  /// The plain text.
  pub text: String,
  /// Marks over the text, in canonical order.
  pub marks: Vec<Mark>,
}

/// Errors the engine itself can report.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MdError {
  /// The document's schema version is not supported.
  #[error("unsupported document schema version: {0}")]
  UnsupportedSchemaVersion(i32),
  /// A block referenced by id is not in the document.
  #[error("block not found: {0}")]
  BlockNotFound(String),
  /// The wire input could not be parsed.
  #[error("invalid input: {0}")]
  InvalidInput(String),
}
