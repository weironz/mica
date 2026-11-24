//! Command and patch protocol for [`crate::EditorSession`].
//!
//! Designed for zero-friction native Rust calls, FFI, and JSON-serialized
//! WASM / bridge transport (`dispatch` / `dispatch_json`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{BlockPatch, BlockSpec, ClipboardPayload, DocOp, EditorBlock, EditorEffect};
use crate::selection::{ActiveMarks, DocSelection};
use crate::table::TableOp;

/// Structured paste payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum PasteInput {
  /// Plain clipboard text (may trigger bare-URL linking, display math block,
  /// single-line inline marks, or multi-line Markdown fragment parsing).
  Plain { text: String },
  /// Literal plain-text paste (Shift+Ctrl+V: no Markdown parsing, no URL linking).
  PlainLiteral { text: String },
  /// Markdown source text (rich flag controls inline merge vs block replacement).
  Markdown {
    markdown: String,
    #[serde(default)]
    plain: String,
    #[serde(default = "default_true")]
    rich: bool,
  },
  /// Parsed HTML tree (`mica_md::HtmlNode` JSON shape) + plain fallback.
  Html {
    html_node_json: Value,
    #[serde(default)]
    plain: String,
  },
}

fn default_true() -> bool {
  true
}

/// One in-page find match (UTF-16 offsets within a block).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindMatch {
  pub block_index: usize,
  pub block_id: String,
  pub start: u32,
  pub end: u32,
}

/// Find/replace operations dispatched via [`EditorCommand::FindReplace`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FindReplaceOp {
  /// Find all non-overlapping matches in the document.
  FindAll {
    query: String,
    #[serde(default)]
    case_sensitive: bool,
  },
  /// Replace a single UTF-16 range `[start, end)` in `block_index`.
  ReplaceOne {
    block_index: usize,
    start: u32,
    end: u32,
    replacement: String,
  },
  /// Replace every non-overlapping match across the document in one undo batch.
  ReplaceAll {
    query: String,
    replacement: String,
    #[serde(default)]
    case_sensitive: bool,
  },
}

/// Frozen snapshot of the editor body captured before generating an AI edit preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiSnapshot {
  pub root_block_id: String,
  pub blocks: Vec<EditorBlock>,
}

/// One targeted block-range replacement proposed by AI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiBlockEdit {
  pub block_ids: Vec<String>,
  #[serde(default)]
  pub before: String,
  #[serde(default)]
  pub after: String,
  pub replacement: Vec<BlockSpec>,
}

/// Collection of non-overlapping AI block edits to apply atomically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiEditPreview {
  pub edits: Vec<AiBlockEdit>,
}

/// All commands accepted by [`crate::EditorSession::dispatch`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum EditorCommand {
  /// Insert plain text at the caret (or replacing selection), running input rules.
  InsertText { text: String },
  /// Platform IME buffer update for the focused block (handles composition,
  /// cross-block composition, Enter split, code auto-indent, multi-line paste,
  /// and keyboard input rules).
  ImeUpdate {
    text: String,
    base: u32,
    extent: u32,
    #[serde(default)]
    composing_valid: bool,
    #[serde(default = "default_true")]
    composing_collapsed: bool,
    #[serde(default)]
    shift_pressed: bool,
  },
  /// Explicitly finish an active IME composition session and close typing coalescing.
  CommitComposition,
  /// Backspace (deletes selection, or math atom, or sheds list marker, or merges
  /// with previous block, or deletes previous/current char).
  DeleteBackward,
  /// Forward Delete (deletes selection, or math atom, or merges next block, or
  /// deletes next char).
  DeleteForward,
  /// Enter / Split block at caret (or exit empty list/todo/quote, or insert
  /// code auto-indent newline).
  SplitBlock,
  /// Move caret / update selection (paint-only; does not bump `content_revision`).
  SetSelection { selection: Option<DocSelection> },
  /// Toggle an inline mark across the current selection (`bold`, `italic`,
  /// `code`, `strike`, `link`, `math`).
  ToggleMark {
    kind: String,
    #[serde(default)]
    href: Option<String>,
  },
  /// Set, edit, or remove (`href: None`) a link over `[start, end)` of `block_index`.
  /// When `text` is `Some(label)`, also replaces the link label text.
  SetLink {
    block_index: usize,
    start: u32,
    end: u32,
    #[serde(default)]
    href: Option<String>,
    #[serde(default)]
    text: Option<String>,
  },
  /// Indent (`delta > 0`) or outdent (`delta < 0`) selected list items, attach/
  /// detach container children (`data.li`), or shift code-block lines.
  Indent { delta: i32 },
  /// Transform the focused block's kind (and optional data).
  TransformBlock {
    kind: String,
    #[serde(default)]
    data: Option<Value>,
  },
  /// Transform all non-atomic blocks in the current selection (`code_block`
  /// merges them into one code block; other kinds preserve inline marks).
  BatchTransform {
    kind: String,
    #[serde(default)]
    data: Option<Value>,
  },
  /// Move `block_ids` to `target_index` using LIS-minimal `move_block` ops.
  MoveBlocks {
    block_ids: Vec<String>,
    target_index: usize,
  },
  /// Delete all blocks covered by the current selection (or atomic block at caret),
  /// keeping the document non-empty.
  DeleteSelectedBlocks,
  /// Table mutation, navigation, or area copy on table block `block_index`.
  TableOp { block_index: usize, op: TableOp },
  /// Structured paste (plain, literal, Markdown, or HTML tree).
  Paste { input: PasteInput },
  /// Copy the current selection to `clipboard_out` (plain + Markdown + HTML).
  CopySelection {
    #[serde(default)]
    image_urls: BTreeMap<String, String>,
  },
  /// Copy and delete the current selection in one command.
  CutSelection {
    #[serde(default)]
    image_urls: BTreeMap<String, String>,
  },
  /// In-page find / replace.
  FindReplace { op: FindReplaceOp },
  /// Apply a validated AI block-edit preview atomically as one undo step.
  ApplyAiEdits {
    snapshot: AiSnapshot,
    preview: AiEditPreview,
  },
  /// Undo the latest local editor batch (CRDT forward-op compatible).
  Undo,
  /// Redo the latest undone local editor batch.
  Redo,
  /// Merge a remote peer's yrs v1 update, preserving local undo/redo and
  /// reconciling the selection.
  ApplyRemoteUpdate { update: Vec<u8> },
  /// Export the current document body as Markdown via `mica_md`.
  ExportMarkdown {
    #[serde(default)]
    assets: BTreeMap<String, String>,
  },
  /// Toggle checked state on a `todo` block.
  ToggleTodo { block_index: usize },
  /// Apply a slash-menu item replacing `[start, end)` in the focused block.
  ApplySlashCommand {
    start: u32,
    end: u32,
    kind: String,
    #[serde(default)]
    data: Value,
  },
  /// Set or clear (`None` / `"auto"`) a code block's language label.
  SetCodeLanguage {
    block_index: usize,
    #[serde(default)]
    language: Option<String>,
  },
  /// Replace an inline math run `[start, end)` with `source` (empty deletes it).
  SetInlineMath {
    block_index: usize,
    start: u32,
    end: u32,
    source: String,
  },
  /// Insert a divider at the caret and ensure a trailing editable paragraph.
  InsertDivider,
  /// Insert an image block at the caret and ensure a trailing editable paragraph.
  InsertImage {
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    alt: String,
    #[serde(default)]
    align: Option<String>,
  },
  /// Replace the raw text/source of block `block_index` (e.g. code block,
  /// Mermaid diagram, math LaTeX, or embedded whiteboard JSON) without running
  /// prose input rules, preserving marks/data and recording a CRDT undo step.
  SetBlockText { block_index: usize, text: String },
  /// Ensure the document ends with an editable empty paragraph (inserting one
  /// if the final block is non-empty or non-paragraph, e.g. table/code/image)
  /// and collapse the caret onto it.
  AppendEmptyParagraph,
  /// Explicitly close any open typing-coalescing history batch (e.g. on 400ms timer).
  FlushHistoryBatch,
}

/// Unified result returned by every [`crate::EditorSession::dispatch`] call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorPatch {
  pub content_revision: u64,
  pub content_changed: bool,
  pub dirty_blocks: Vec<BlockPatch>,
  pub selection: Option<DocSelection>,
  pub active_marks: ActiveMarks,
  pub can_undo: bool,
  pub can_redo: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub crdt_update: Option<Vec<u8>>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub doc_ops: Vec<DocOp>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub clipboard_out: Option<ClipboardPayload>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub effects: Vec<EditorEffect>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub find_matches: Option<Vec<FindMatch>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub replaced_count: Option<usize>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub exported_markdown: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
}
