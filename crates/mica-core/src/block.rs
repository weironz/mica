//! The flat block DTO — mirrors `crates/markdown::Block` exactly so the yrs
//! document model and the existing markdown / REST model speak the same shape.

use serde_json::Value;

/// One block. A document is a flat collection of these; the tree lives in each
/// block's `children` (ids) plus a designated root (see [`crate::doc`]).
///
/// `text` is clean plain text; inline marks and block attributes both live in
/// `data` (`data.marks` is the inline-format ranges, the rest are block attrs).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Block {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
    #[serde(default)]
    pub children: Vec<String>,
}

impl Block {
    pub fn new(id: impl Into<String>, kind: impl Into<String>) -> Self {
        Block {
            id: id.into(),
            kind: kind.into(),
            text: String::new(),
            data: Value::Null,
            children: Vec::new(),
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    pub fn with_children(mut self, children: Vec<String>) -> Self {
        self.children = children;
        self
    }
}
