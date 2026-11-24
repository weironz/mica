//! Core flat block document model, operation DTOs, and editor patch types.
//!
//! Mirrors `clients/mica_flutter/lib/editor/model.dart` while binding directly
//! to `mica_core::Block` (single schema authority from `mica_md::schema`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::marks::{EditorMark, marks_from_data, marks_to_json};
use crate::utf16::utf16_len;

/// Upper bound on every nesting depth (`data.indent`, `data.quote`, `data.li`).
/// Mirrors `kMaxNestingDepth` in Dart and `MAX_NESTING_DEPTH` in `md-core`.
pub const MAX_NESTING_DEPTH: u32 = 8;

/// Lightweight flat block cached in `EditorSession`, synchronized with `MicaDoc`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorBlock {
  pub id: String,
  #[serde(rename = "type")]
  pub kind: String,
  #[serde(default)]
  pub text: String,
  #[serde(default)]
  pub data: Value,
}

impl EditorBlock {
  pub fn new(id: impl Into<String>, kind: impl Into<String>, text: impl Into<String>) -> Self {
    Self {
      id: id.into(),
      kind: kind.into(),
      text: text.into(),
      data: Value::Object(Map::new()),
    }
  }

  pub fn with_data(mut self, data: Value) -> Self {
    self.data = normalize_data_object(data);
    self
  }

  /// Convert into the shared `mica_core::Block` DTO (flat: `children` empty).
  pub fn to_core_block(&self) -> mica_core::Block {
    mica_core::Block {
      id: self.id.clone(),
      kind: self.kind.clone(),
      text: self.text.clone(),
      data: if self.data_map().is_empty() {
        Value::Null
      } else {
        self.data.clone()
      },
      children: Vec::new(),
    }
  }

  /// Convert into a `mica_md::Block` for direct serialization / HTML rendering.
  pub fn to_md_block(&self) -> mica_md::Block {
    self.to_core_block()
  }

  pub fn data_map(&self) -> &Map<String, Value> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    match &self.data {
      Value::Object(m) => m,
      _ => EMPTY.get_or_init(Map::new),
    }
  }

  pub fn data_map_mut(&mut self) -> &mut Map<String, Value> {
    if !self.data.is_object() {
      self.data = Value::Object(Map::new());
    }
    match &mut self.data {
      Value::Object(m) => m,
      _ => unreachable!(),
    }
  }

  pub fn utf16_len(&self) -> u32 {
    utf16_len(&self.text)
  }

  pub fn is_code(&self) -> bool {
    self.kind == "code_block"
  }

  /// Void/atomic block kinds that hold no inline caret.
  pub fn is_atomic_kind(kind: &str) -> bool {
    matches!(kind, "table" | "divider" | "image" | "math_block")
  }

  pub fn is_atomic(&self) -> bool {
    Self::is_atomic_kind(&self.kind)
  }

  /// Edits as ONE unit even when the kind is textual (e.g. mermaid / mica-board
  /// diagram code block in preview mode).
  pub fn is_unit_block(&self) -> bool {
    if self.is_atomic() {
      return true;
    }
    if self.kind == "code_block" {
      let lang = self.data_map().get("language").and_then(Value::as_str);
      let view = self.data_map().get("view").and_then(Value::as_str);
      return matches!(lang, Some("mermaid" | "mica-board")) && view != Some("code");
    }
    false
  }

  /// List/todo nesting level (`0..=MAX_NESTING_DEPTH`).
  pub fn indent(&self) -> u32 {
    self
      .data_map()
      .get("indent")
      .and_then(Value::as_i64)
      .unwrap_or(0)
      .clamp(0, MAX_NESTING_DEPTH as i64) as u32
  }

  /// Quote nesting depth (`>= 1` when `kind == "quote"`).
  pub fn quote_depth(&self) -> u32 {
    let d = self
      .data_map()
      .get("quote")
      .and_then(Value::as_i64)
      .unwrap_or(0)
      .clamp(0, MAX_NESTING_DEPTH as i64) as u32;
    if self.kind == "quote" && d < 1 { 1 } else { d }
  }

  /// Container-child list item level (`data.li`), if nested inside a list item.
  pub fn li_level(&self) -> Option<u32> {
    self
      .data_map()
      .get("li")
      .and_then(Value::as_i64)
      .map(|v| v.clamp(0, MAX_NESTING_DEPTH as i64) as u32)
  }

  pub fn is_list_kind(&self) -> bool {
    matches!(
      self.kind.as_str(),
      "bulleted_list" | "numbered_list" | "todo"
    )
  }

  /// Kinds whose whole content is one run of inline text that accepts inline
  /// formatting marks.
  pub fn is_inline_text_kind(kind: &str) -> bool {
    matches!(
      kind,
      "paragraph" | "heading" | "quote" | "bulleted_list" | "numbered_list" | "todo"
    )
  }

  pub fn is_inline_text(&self) -> bool {
    Self::is_inline_text_kind(&self.kind)
  }

  /// Kinds that can attach inside a list item as container children (`data.li`).
  pub fn is_attachable_kind(kind: &str) -> bool {
    matches!(kind, "code_block" | "quote" | "paragraph" | "divider")
  }

  /// Kinds that continue into the same block kind on Enter (until empty).
  pub fn continues_on_enter(kind: &str) -> bool {
    matches!(kind, "bulleted_list" | "numbered_list" | "todo" | "quote")
  }

  pub fn heading_level(&self) -> u32 {
    self
      .data_map()
      .get("level")
      .and_then(Value::as_i64)
      .unwrap_or(1)
      .clamp(1, 6) as u32
  }

  pub fn todo_checked(&self) -> bool {
    self
      .data_map()
      .get("checked")
      .and_then(Value::as_bool)
      .unwrap_or(false)
  }

  pub fn marks(&self) -> Vec<EditorMark> {
    marks_from_data(&self.data)
  }

  pub fn set_marks(&mut self, marks: &[EditorMark]) {
    if marks.is_empty() {
      if self.data_map().contains_key("marks") {
        self.data_map_mut().remove("marks");
      }
    } else {
      self
        .data_map_mut()
        .insert("marks".to_string(), marks_to_json(marks));
    }
  }
}

impl From<mica_core::Block> for EditorBlock {
  fn from(b: mica_core::Block) -> Self {
    Self {
      id: b.id,
      kind: b.kind,
      text: b.text,
      data: normalize_data_object(b.data),
    }
  }
}

impl From<&mica_core::Block> for EditorBlock {
  fn from(b: &mica_core::Block) -> Self {
    Self {
      id: b.id.clone(),
      kind: b.kind.clone(),
      text: b.text.clone(),
      data: normalize_data_object(b.data.clone()),
    }
  }
}

/// Ensure block `data` is always a JSON object (converting `Null` to `{}`).
pub fn normalize_data_object(data: Value) -> Value {
  match data {
    Value::Object(m) => Value::Object(m),
    _ => Value::Object(Map::new()),
  }
}

/// Unidentified block spec used by paste, AI edits, and insertion APIs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockSpec {
  #[serde(rename = "type")]
  pub kind: String,
  #[serde(default)]
  pub text: String,
  #[serde(default)]
  pub data: Value,
}

impl BlockSpec {
  pub fn new(kind: impl Into<String>, text: impl Into<String>, data: Value) -> Self {
    Self {
      kind: kind.into(),
      text: text.into(),
      data: normalize_data_object(data),
    }
  }

  pub fn to_md_block(&self) -> mica_md::Block {
    mica_md::Block {
      id: String::new(),
      kind: self.kind.clone(),
      text: self.text.clone(),
      data: match &self.data {
        Value::Object(m) if m.is_empty() => Value::Null,
        other => other.clone(),
      },
      children: Vec::new(),
    }
  }
}

impl From<mica_md::Block> for BlockSpec {
  fn from(b: mica_md::Block) -> Self {
    Self {
      kind: b.kind,
      text: b.text,
      data: normalize_data_object(b.data),
    }
  }
}

/// Heading toggle target matching `model.dart::headingButtonTarget`:
/// pressing the active heading level returns to a paragraph.
pub fn heading_button_target(kind: Option<&str>, level: u32, pressed: u32) -> (String, Value) {
  let clamped = pressed.clamp(1, 6);
  if kind == Some("heading") && level == clamped {
    ("paragraph".to_string(), json!({}))
  } else {
    ("heading".to_string(), json!({ "level": clamped }))
  }
}

/// Legacy-compatible document operation emitted alongside `crdt_update` for
/// transitional FFI / host callers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocOp {
  InsertBlock {
    parent_id: String,
    index: usize,
    block: mica_core::Block,
  },
  UpdateBlock {
    block_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
  },
  DeleteBlock {
    block_id: String,
  },
  MoveBlock {
    block_id: String,
    parent_id: String,
    index: usize,
  },
}

/// Incremental UI/renderer patch describing changed blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BlockPatch {
  Upsert { index: usize, block: EditorBlock },
  Remove { block_id: String },
  Reorder { order: Vec<String> },
}

/// Multi-flavor clipboard payload emitted by Copy / Cut / Table area copy.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClipboardPayload {
  pub plain: String,
  pub markdown: String,
  pub html: String,
}

/// Side-effect hints returned by `EditorSession::dispatch` for the UI shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditorEffect {
  ScrollIntoView {
    block_id: String,
    offset: u32,
  },
  OpenSlashMenu {
    block_id: String,
    query: String,
    start_offset: u32,
    end_offset: u32,
  },
  CloseSlashMenu,
  OpenPageLinkMenu {
    block_id: String,
    query: String,
    start_offset: u32,
    end_offset: u32,
  },
  ClosePageLinkMenu,
  ShowToast {
    message: String,
  },
}

/// Errors produced by editor operations (remote update decode, AI conflicts, JSON).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EditorError {
  #[error("failed to apply remote CRDT update: {0}")]
  RemoteUpdate(String),
  #[error("AI edit conflict: document changed since snapshot")]
  AiConflict,
  #[error("invalid editor command JSON: {0}")]
  InvalidCommandJson(String),
}

/// Structural JSON equality treating missing `marks: []` and empty objects cleanly.
pub fn block_data_eq(a: &Value, b: &Value) -> bool {
  let clean = |v: &Value| -> Map<String, Value> {
    let mut m = match v {
      Value::Object(o) => o.clone(),
      _ => Map::new(),
    };
    if matches!(m.get("marks"), Some(Value::Array(arr)) if arr.is_empty()) {
      m.remove("marks");
    }
    m
  };
  clean(a) == clean(b)
}
