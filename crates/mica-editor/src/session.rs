//! Headless block editor session (`EditorSession`) over `mica_core::MicaDoc`
//! and `mica_md`.
//!
//! Single document authority: every edit mutates the owned `MicaDoc` through
//! tracked CRDT transactions, synchronizes the lightweight flat `Vec<EditorBlock>`
//! cache, and returns an [`EditorPatch`] containing incremental `dirty_blocks`,
//! `crdt_update`, legacy-compatible `doc_ops`, `selection`, `active_marks`,
//! `clipboard_out`, and UI `effects`.

use std::collections::{BTreeMap, HashMap, HashSet};

use mica_core::MicaDoc;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::commands::{
  AiEditPreview, AiSnapshot, EditorCommand, EditorPatch, FindMatch, FindReplaceOp,
};
use crate::diff_ops::{diff_block_ops, diff_block_patches};
use crate::history::{EditorHistoryTracker, HistorySelection};
use crate::marks::{
  EditorMark, apply_mark, concat_marks, marks_to_json, math_run_at, math_run_ending_at,
  math_run_starting_at, range_has_mark, shift_marks, split_marks,
};
use crate::model::{
  BlockPatch, BlockSpec, ClipboardPayload, DocOp, EditorBlock, EditorEffect, block_data_eq,
  normalize_data_object,
};
use crate::paste::{
  build_clipboard_payload, document_md_blocks, parse_paste_blocks, retag_mislabeled_fences,
  unwrap_nested_fences,
};
use crate::rules::{
  InputRuleMatch, detect_page_link_trigger, detect_slash_trigger, match_input_rule,
};
use crate::selection::{
  ActiveMarks, DocPosition, DocSelection, NormalizedSelection, active_marks_for_selection,
  inline_selection_ranges, reconcile_selection,
};
use crate::utf16::{
  common_prefix_utf16, common_suffix_utf16, diff_edit_range, inserted_text_utf16,
  snap_utf16_boundary, utf16_len, utf16_next_boundary, utf16_prev_boundary, utf16_replace_range,
  utf16_slice,
};

/// Cross-block IME preedit tracking state (mirrors `prose_input.dart`).
#[derive(Debug, Default)]
pub(crate) struct ProseInputState {
  pub range: Option<DocSelection>,
  pub base_text: Option<String>,
  pub composing: bool,
}

impl ProseInputState {
  pub fn reset(&mut self) {
    self.range = None;
    self.base_text = None;
    self.composing = false;
  }
}

/// Internal per-command execution accumulator passed across session helpers.
#[derive(Default)]
pub(crate) struct DispatchCtx {
  pub ops: Vec<DocOp>,
  /// When `Some(block_id)`, the command is a plain-text typing edit that may
  /// coalesce into the open typing history batch for `block_id`.
  pub typing_block: Option<String>,
  /// True when the command already mutated `self.doc` directly (Undo / Redo / Remote).
  pub doc_mutated_directly: bool,
  /// True for `ApplyRemoteUpdate` (do not echo a local `crdt_update` back).
  pub is_remote: bool,
  /// Precomputed incremental `dirty_blocks` for direct-doc commands (`Undo` /
  /// `Redo` / `ApplyRemoteUpdate`), avoiding a whole-document clone + diff.
  pub direct_dirty_blocks: Option<Vec<BlockPatch>>,
  pub clipboard_out: Option<ClipboardPayload>,
  pub effects: Vec<EditorEffect>,
  pub find_matches: Option<Vec<FindMatch>>,
  pub replaced_count: Option<usize>,
  pub exported_markdown: Option<String>,
  pub error: Option<String>,
}

/// Pure-Rust headless block editor kernel backed directly by `MicaDoc`.
pub struct EditorSession {
  pub(crate) doc: MicaDoc,
  pub(crate) root_block_id: String,
  pub(crate) blocks: Vec<EditorBlock>,
  pub(crate) selection: Option<DocSelection>,
  pub(crate) content_revision: u64,
  pub(crate) history: EditorHistoryTracker,
  pub(crate) prose_input: ProseInputState,
  pub(crate) slash_open: bool,
  pub(crate) page_link_open: bool,
}

impl EditorSession {
  /// Wrap an existing [`MicaDoc`] as the single authoritative document.
  /// Never writes to `doc` during construction.
  pub fn from_doc(mut doc: MicaDoc) -> Self {
    let root_id = {
      let r = doc.root_block_id();
      if r.is_empty() { "root".to_string() } else { r }
    };
    let blocks = read_body_blocks(&doc, &root_id);
    let selection = default_initial_selection(&blocks);
    doc.track_changes();
    let _ = doc.take_changed_block_ids();
    Self {
      doc,
      root_block_id: root_id,
      blocks,
      selection,
      content_revision: 0,
      history: EditorHistoryTracker::new(),
      prose_input: ProseInputState::default(),
      slash_open: false,
      page_link_open: false,
    }
  }

  /// Parse Markdown through `mica_md` (single Markdown source of truth) and
  /// open a session over the resulting [`MicaDoc`].
  pub fn from_markdown(markdown: &str) -> Self {
    Self::from_markdown_with_client_id(markdown, None)
  }

  /// Like [`Self::from_markdown`], with an explicit 53-bit yrs `client_id`.
  pub fn from_markdown_with_client_id(markdown: &str, client_id: Option<u64>) -> Self {
    let mut parsed = mica_md::parse(markdown, &mica_md::ParseOptions::default());
    // An empty Markdown document must still start with one editable paragraph
    // in `MicaDoc` so initial caret placement and Undo never drop to 0 blocks.
    let root_has_children = parsed
      .blocks
      .iter()
      .find(|b| b.id == parsed.root_block_id)
      .is_some_and(|b| !b.children.is_empty());
    if !root_has_children {
      let empty_id = uuid::Uuid::new_v4().to_string();
      if let Some(root) = parsed
        .blocks
        .iter_mut()
        .find(|b| b.id == parsed.root_block_id)
      {
        root.children.push(empty_id.clone());
      } else {
        parsed.blocks.push(mica_core::Block {
          id: parsed.root_block_id.clone(),
          kind: "page".to_string(),
          text: String::new(),
          data: Value::Null,
          children: vec![empty_id.clone()],
        });
      }
      parsed
        .blocks
        .push(mica_core::Block::new(empty_id, "paragraph"));
    }
    let doc = MicaDoc::from_blocks_with_client_id(&parsed.root_block_id, &parsed.blocks, client_id);
    Self::from_doc(doc)
  }

  /// Build a session from a flat list of body [`EditorBlock`]s under `root_id`.
  pub fn from_blocks(root_id: impl Into<String>, blocks: Vec<EditorBlock>) -> Self {
    Self::from_blocks_with_client_id(root_id, blocks, None)
  }

  /// Like [`Self::from_blocks`], with an explicit 53-bit yrs `client_id`.
  pub fn from_blocks_with_client_id(
    root_id: impl Into<String>,
    blocks: Vec<EditorBlock>,
    client_id: Option<u64>,
  ) -> Self {
    let root = root_id.into();
    let child_ids: Vec<String> = blocks.iter().map(|b| b.id.clone()).collect();
    let mut core_blocks = Vec::with_capacity(blocks.len() + 1);
    core_blocks.push(mica_core::Block {
      id: root.clone(),
      kind: "page".to_string(),
      text: String::new(),
      data: Value::Null,
      children: child_ids,
    });
    core_blocks.extend(blocks.iter().map(EditorBlock::to_core_block));
    let doc = MicaDoc::from_blocks_with_client_id(&root, &core_blocks, client_id);
    Self::from_doc(doc)
  }

  // ── Read accessors ───────────────────────────────────────────────────────

  pub fn doc(&self) -> &MicaDoc {
    &self.doc
  }

  pub fn doc_mut(&mut self) -> &mut MicaDoc {
    &mut self.doc
  }

  pub fn root_block_id(&self) -> &str {
    &self.root_block_id
  }

  pub fn blocks(&self) -> &[EditorBlock] {
    &self.blocks
  }

  pub fn selection(&self) -> Option<&DocSelection> {
    self.selection.as_ref()
  }

  pub fn content_revision(&self) -> u64 {
    self.content_revision
  }

  pub fn can_undo(&self) -> bool {
    self.history.can_undo(&self.doc)
  }

  pub fn can_redo(&self) -> bool {
    self.history.can_redo(&self.doc)
  }

  pub fn active_marks(&self) -> ActiveMarks {
    active_marks_for_selection(&self.blocks, self.selection.as_ref())
  }

  pub fn encode_state(&self) -> Vec<u8> {
    self.doc.encode_state()
  }

  pub fn state_vector(&self) -> Vec<u8> {
    self.doc.state_vector()
  }

  pub fn capture_ai_snapshot(&self) -> AiSnapshot {
    AiSnapshot {
      root_block_id: self.root_block_id.clone(),
      blocks: self.blocks.clone(),
    }
  }

  /// Export the live document body as Markdown via `mica_md`.
  pub fn export_markdown(&self, assets: &BTreeMap<String, String>) -> String {
    if self.blocks.is_empty() {
      return String::new();
    }
    mica_md::serialize_body(
      &document_md_blocks(&self.blocks),
      &mica_md::SerializeOptions {
        assets: assets.clone(),
      },
    )
  }

  /// Export the live document body as HTML via `mica_md`.
  pub fn export_html(
    &self,
    image_srcs: &BTreeMap<String, String>,
    profile: mica_md::HtmlProfile,
  ) -> String {
    if self.blocks.is_empty() {
      return String::new();
    }
    mica_md::render_html_body(
      &document_md_blocks(&self.blocks),
      &mica_md::HtmlOptions {
        image_srcs: image_srcs.clone(),
        profile,
      },
    )
  }

  // ── Unified command entrypoints ──────────────────────────────────────────

  /// JSON passthrough for FFI / WASM callers.
  pub fn dispatch_json(&mut self, cmd_json: &str) -> Result<String, String> {
    let cmd: EditorCommand =
      serde_json::from_str(cmd_json).map_err(|e| format!("invalid command JSON: {e}"))?;
    let patch = self.dispatch(cmd);
    serde_json::to_string(&patch).map_err(|e| format!("failed to serialize patch: {e}"))
  }

  /// Execute one [`EditorCommand`] against the session and return an [`EditorPatch`].
  pub fn dispatch(&mut self, cmd: EditorCommand) -> EditorPatch {
    let prev_sel = self.selection.clone();
    let before_hist_sel = HistorySelection::capture(&self.blocks, self.selection.as_ref());
    let sv_before = self.doc.state_vector();
    let mut ctx = DispatchCtx::default();

    match cmd {
      EditorCommand::InsertText { text } => self.cmd_insert_text(&text, &mut ctx),
      EditorCommand::ImeUpdate {
        text,
        base,
        extent,
        composing_valid,
        composing_collapsed,
        shift_pressed,
      } => self.cmd_ime_update(
        &text,
        base,
        extent,
        composing_valid,
        composing_collapsed,
        shift_pressed,
        &mut ctx,
      ),
      EditorCommand::CommitComposition => {
        self.prose_input.reset();
        self.history.flush_typing_batch(&mut self.doc);
      }
      EditorCommand::DeleteBackward => self.cmd_delete_backward(&mut ctx),
      EditorCommand::DeleteForward => self.cmd_delete_forward(&mut ctx),
      EditorCommand::SplitBlock => self.cmd_split_block(&mut ctx),
      EditorCommand::SetSelection { selection } => {
        self.prose_input.reset();
        let next = selection.and_then(|s| s.clamp_and_snap(&self.blocks));
        if next != self.selection {
          self.selection = next;
          self.refresh_trigger_effects(&mut ctx);
        }
      }
      EditorCommand::ToggleMark { kind, href } => {
        self.cmd_toggle_mark(&kind, href.as_deref(), &mut ctx);
      }
      EditorCommand::SetLink {
        block_index,
        start,
        end,
        href,
        text,
      } => {
        self.cmd_set_link(
          block_index,
          start,
          end,
          href.as_deref(),
          text.as_deref(),
          &mut ctx,
        );
      }
      EditorCommand::Indent { delta } => self.cmd_indent(delta, &mut ctx),
      EditorCommand::TransformBlock { kind, data } => {
        self.cmd_transform_block(&kind, data, &mut ctx);
      }
      EditorCommand::BatchTransform { kind, data } => {
        self.cmd_batch_transform(&kind, data, &mut ctx);
      }
      EditorCommand::MoveBlocks {
        block_ids,
        target_index,
      } => {
        self.cmd_move_blocks(&block_ids, target_index, &mut ctx);
      }
      EditorCommand::DeleteSelectedBlocks => self.cmd_delete_selected_blocks(&mut ctx),
      EditorCommand::TableOp { block_index, op } => {
        self.cmd_table_op(block_index, op, &mut ctx);
      }
      EditorCommand::Paste { input } => self.cmd_paste(input, &mut ctx),
      EditorCommand::CopySelection { image_urls } => {
        if let Some(sel) = &self.selection {
          ctx.clipboard_out = build_clipboard_payload(&self.blocks, sel, &image_urls);
        }
      }
      EditorCommand::CutSelection { image_urls } => {
        if let Some(sel) = &self.selection {
          ctx.clipboard_out = build_clipboard_payload(&self.blocks, sel, &image_urls);
          if ctx.clipboard_out.is_some() {
            self.delete_current_selection(&mut ctx);
          }
        }
      }
      EditorCommand::FindReplace { op } => self.cmd_find_replace(op, &mut ctx),
      EditorCommand::ApplyAiEdits { snapshot, preview } => {
        self.cmd_apply_ai_edits(&snapshot, &preview, &mut ctx);
      }
      EditorCommand::Undo => self.cmd_undo_redo(false, &mut ctx),
      EditorCommand::Redo => self.cmd_undo_redo(true, &mut ctx),
      EditorCommand::ApplyRemoteUpdate { update } => {
        self.cmd_apply_remote_update(&update, &mut ctx);
      }
      EditorCommand::ExportMarkdown { assets } => {
        ctx.exported_markdown = Some(self.export_markdown(&assets));
      }
      EditorCommand::ToggleTodo { block_index } => self.cmd_toggle_todo(block_index, &mut ctx),
      EditorCommand::ApplySlashCommand {
        start,
        end,
        kind,
        data,
      } => self.cmd_apply_slash_command(start, end, &kind, data, &mut ctx),
      EditorCommand::SetCodeLanguage {
        block_index,
        language,
      } => self.cmd_set_code_language(block_index, language.as_deref(), &mut ctx),
      EditorCommand::SetInlineMath {
        block_index,
        start,
        end,
        source,
      } => self.cmd_set_inline_math(block_index, start, end, &source, &mut ctx),
      EditorCommand::InsertDivider => self.cmd_insert_divider(&mut ctx),
      EditorCommand::InsertImage {
        file_id,
        url,
        name,
        alt,
        align,
      } => self.cmd_insert_image(file_id, url, name, &alt, align, &mut ctx),
      EditorCommand::SetBlockText { block_index, text } => {
        if let Some(b) = self.blocks.get_mut(block_index)
          && b.text != text
        {
          let id = b.id.clone();
          b.text = text;
          ctx.ops.push(Self::update_text_data_op(b));
          ctx.typing_block = Some(id);
        }
      }
      EditorCommand::AppendEmptyParagraph => {
        self.prose_input.reset();
        let last_is_empty_para = self
          .blocks
          .last()
          .is_some_and(|b| b.kind == "paragraph" && b.text.is_empty());
        if last_is_empty_para {
          let last = self.blocks.len() - 1;
          self.collapse_to(DocPosition::new(last, 0));
        } else {
          let at = self.blocks.len();
          let created = EditorBlock::new(self.gen_block_id(), "paragraph", "");
          ctx.ops.push(self.insert_op(&created, at));
          self.blocks.push(created);
          self.collapse_to(DocPosition::new(at, 0));
          ctx.typing_block = None;
        }
      }
      EditorCommand::PrependEmptyParagraph => {
        self.prose_input.reset();
        let sole_empty_para = self.blocks.len() == 1
          && self.blocks[0].kind == "paragraph"
          && self.blocks[0].text.is_empty();
        if sole_empty_para {
          self.collapse_to(DocPosition::new(0, 0));
        } else {
          let created = EditorBlock::new(self.gen_block_id(), "paragraph", "");
          ctx.ops.push(self.insert_op(&created, 0));
          self.blocks.insert(0, created);
          self.collapse_to(DocPosition::new(0, 0));
          ctx.typing_block = None;
        }
      }
      EditorCommand::FlushHistoryBatch => {
        self.history.flush_typing_batch(&mut self.doc);
      }
    }

    // Commit collected `DocOp`s into the authoritative `MicaDoc`.
    if !ctx.doc_mutated_directly && !ctx.ops.is_empty() {
      self.ensure_root_in_doc();
      let after_blocks = &self.blocks;
      let after_sel = &self.selection;
      let root_id = self.root_block_id.as_str();
      let ops = &ctx.ops;
      if let Some(typing_id) = ctx.typing_block.as_deref() {
        self.history.record_typing_edit(
          &mut self.doc,
          typing_id,
          before_hist_sel,
          |doc| apply_ops_to_doc(doc, root_id, ops),
          || HistorySelection::capture(after_blocks, after_sel.as_ref()),
        );
      } else {
        self.history.record_structural_edit(
          &mut self.doc,
          before_hist_sel,
          |doc| apply_ops_to_doc(doc, root_id, ops),
          || HistorySelection::capture(after_blocks, after_sel.as_ref()),
        );
      }
      // Drain our own local mutations from the change tracker so the next
      // Undo / Redo / ApplyRemoteUpdate sees only the blocks IT touches.
      let _ = self.doc.take_changed_block_ids();
    }

    let dirty_blocks = if let Some(direct) = ctx.direct_dirty_blocks.take() {
      direct
    } else if ctx.ops.is_empty() {
      Vec::new()
    } else {
      patches_from_ops(&self.blocks, &ctx.ops, self.selection.as_ref())
    };
    let content_changed = !dirty_blocks.is_empty();
    if content_changed {
      self.content_revision += 1;
    }

    let crdt_update = if !ctx.is_remote && (content_changed || !ctx.ops.is_empty()) {
      self
        .doc
        .encode_diff(&sv_before)
        .ok()
        .filter(|bytes| !bytes.is_empty())
    } else {
      None
    };

    if (content_changed || self.selection != prev_sel)
      && let Some(sel) = &self.selection
      && let Some(b) = self.blocks.get(sel.focus_node())
    {
      let offset = sel.focus_pos().offset;
      if !ctx
        .effects
        .iter()
        .any(|e| matches!(e, EditorEffect::ScrollIntoView { .. }))
      {
        ctx.effects.push(EditorEffect::ScrollIntoView {
          block_id: b.id.clone(),
          offset,
        });
      }
    }

    EditorPatch {
      content_revision: self.content_revision,
      content_changed,
      dirty_blocks,
      selection: self.selection.clone(),
      active_marks: self.active_marks(),
      can_undo: self.can_undo(),
      can_redo: self.can_redo(),
      crdt_update,
      doc_ops: ctx.ops,
      clipboard_out: ctx.clipboard_out,
      effects: ctx.effects,
      find_matches: ctx.find_matches,
      replaced_count: ctx.replaced_count,
      exported_markdown: ctx.exported_markdown,
      error: ctx.error,
    }
  }

  // ── Shared primitives ────────────────────────────────────────────────────

  pub(crate) fn gen_block_id(&self) -> String {
    format!("b_{}", Uuid::new_v4().simple())
  }

  pub(crate) fn insert_op(&self, block: &EditorBlock, index: usize) -> DocOp {
    DocOp::InsertBlock {
      parent_id: self.root_block_id.clone(),
      index,
      block: block.to_core_block(),
    }
  }

  pub(crate) fn update_text_data_op(block: &EditorBlock) -> DocOp {
    DocOp::UpdateBlock {
      block_id: block.id.clone(),
      kind: None,
      text: Some(block.text.clone()),
      data: Some(block.data.clone()),
    }
  }

  pub(crate) fn update_full_op(block: &EditorBlock) -> DocOp {
    DocOp::UpdateBlock {
      block_id: block.id.clone(),
      kind: Some(block.kind.clone()),
      text: Some(block.text.clone()),
      data: Some(block.data.clone()),
    }
  }

  pub(crate) fn ensure_not_empty(&mut self, ctx: &mut DispatchCtx) {
    if !self.blocks.is_empty() {
      return;
    }
    let block = EditorBlock::new(self.gen_block_id(), "paragraph", "");
    ctx.ops.push(self.insert_op(&block, 0));
    self.blocks.push(block);
    self.selection = Some(DocSelection::caret(0, 0));
    ctx.typing_block = None;
  }

  fn ensure_root_in_doc(&mut self) {
    if self.doc.has_block(&self.root_block_id) {
      return;
    }
    let root = mica_core::Block {
      id: self.root_block_id.clone(),
      kind: "page".to_string(),
      text: String::new(),
      data: Value::Null,
      children: Vec::new(),
    };
    self.doc.set_blocks(&self.root_block_id, &[root]);
  }

  pub(crate) fn collapse_to(&mut self, pos: DocPosition) {
    let raw = DocSelection::caret(pos.node, pos.offset);
    self.selection = raw.clamp_and_snap(&self.blocks);
  }

  pub(crate) fn refresh_trigger_effects(&mut self, ctx: &mut DispatchCtx) {
    let (next_slash, next_page) = match self.selection.as_ref() {
      Some(DocSelection::Caret {
        block_index,
        offset,
      }) => match self.blocks.get(*block_index) {
        Some(block) => (
          detect_slash_trigger(block, *offset).map(|(s, e, q)| (block.id.clone(), s, e, q)),
          detect_page_link_trigger(block, *offset).map(|(s, e, q)| (block.id.clone(), s, e, q)),
        ),
        None => (None, None),
      },
      _ => (None, None),
    };

    if let Some((block_id, start_offset, end_offset, query)) = next_slash {
      self.slash_open = true;
      ctx.effects.push(EditorEffect::OpenSlashMenu {
        block_id,
        query,
        start_offset,
        end_offset,
      });
    } else if self.slash_open {
      self.slash_open = false;
      ctx.effects.push(EditorEffect::CloseSlashMenu);
    }

    if let Some((block_id, start_offset, end_offset, query)) = next_page {
      self.page_link_open = true;
      ctx.effects.push(EditorEffect::OpenPageLinkMenu {
        block_id,
        query,
        start_offset,
        end_offset,
      });
    } else if self.page_link_open {
      self.page_link_open = false;
      ctx.effects.push(EditorEffect::ClosePageLinkMenu);
    }
  }

  // ── Core splice primitive (single-block & cross-block) ───────────────────

  /// Replace `range` with `text` carrying `inserts` (UTF-16 offsets relative to
  /// `text`), shifting surviving marks and collapsing cross-block selections.
  /// Mirrors `controller.dart::spliceRange`.
  pub(crate) fn splice_range(
    &mut self,
    range: &DocSelection,
    text: &str,
    inserts: &[EditorMark],
    pin_marks_at_start: bool,
    typing_coalesce: bool,
    ctx: &mut DispatchCtx,
  ) -> bool {
    let NormalizedSelection::Text { start, end } = range.normalized() else {
      return false;
    };
    if end.node >= self.blocks.len() || start.node > end.node {
      return false;
    }

    let text_u16 = utf16_len(text);
    let build_inserted = |at: u32, limit: u32| -> Vec<EditorMark> {
      inserts
        .iter()
        .filter_map(|m| {
          let ms = at.saturating_add(m.start).min(limit);
          let me = at.saturating_add(m.end).min(limit);
          (me > ms).then(|| m.with_range(ms, me))
        })
        .collect()
    };

    if start.node == end.node {
      let idx = start.node;
      if self.blocks[idx].is_atomic() || self.blocks[idx].kind == "table" {
        return false;
      }
      let old_len = self.blocks[idx].utf16_len();
      let (new_text, s, e) =
        utf16_replace_range(&self.blocks[idx].text, start.offset, end.offset, text);
      let new_len = utf16_len(&new_text);
      let delta = new_len as i64 - old_len as i64;
      let mut marks = shift_marks(
        &self.blocks[idx].marks(),
        s,
        e,
        delta,
        new_len,
        pin_marks_at_start,
      );
      marks.extend(build_inserted(s, new_len));
      if !inserts.is_empty() {
        marks = crate::marks::normalize_marks(marks);
      }

      self.blocks[idx].text = new_text;
      self.blocks[idx].set_marks(&marks);
      let op = Self::update_text_data_op(&self.blocks[idx]);
      ctx.ops.push(op);
      if typing_coalesce {
        ctx.typing_block = Some(self.blocks[idx].id.clone());
      } else {
        ctx.typing_block = None;
      }
      self.collapse_to(DocPosition::new(idx, s + text_u16));
      return true;
    }

    // Cross-block splice: keep prefix of start block, suffix of end block, delete middle+end.
    let start_is_unit = self.blocks[start.node].is_unit_block();
    let end_is_unit = self.blocks[end.node].is_unit_block();
    let s = if start_is_unit {
      0
    } else {
      snap_utf16_boundary(
        &self.blocks[start.node].text,
        start.offset.min(self.blocks[start.node].utf16_len()),
        false,
      )
    };
    let e = if end_is_unit {
      0
    } else {
      snap_utf16_boundary(
        &self.blocks[end.node].text,
        end.offset.min(self.blocks[end.node].utf16_len()),
        true,
      )
    };
    let prefix = if start_is_unit {
      String::new()
    } else {
      utf16_slice(&self.blocks[start.node].text, 0, s).to_string()
    };
    let suffix = if end_is_unit {
      String::new()
    } else {
      let end_len = self.blocks[end.node].utf16_len();
      utf16_slice(&self.blocks[end.node].text, e, end_len).to_string()
    };
    let merged = format!("{prefix}{text}{suffix}");
    let merged_len = utf16_len(&merged);

    let (start_marks, _) = if start_is_unit {
      (Vec::new(), Vec::new())
    } else {
      split_marks(&self.blocks[start.node].marks(), s)
    };
    let mut merged_marks = start_marks;
    if !end_is_unit {
      let base = s + text_u16;
      for m in self.blocks[end.node].marks() {
        let ms =
          (m.start as i64 - e as i64 + base as i64).clamp(base as i64, merged_len as i64) as u32;
        let me =
          (m.end as i64 - e as i64 + base as i64).clamp(base as i64, merged_len as i64) as u32;
        if me > ms {
          merged_marks.push(m.with_range(ms, me));
        }
      }
    }
    merged_marks.extend(build_inserted(s, merged_len));
    merged_marks = crate::marks::normalize_marks(merged_marks);

    let removed_ids: Vec<String> = self.blocks[start.node + 1..=end.node]
      .iter()
      .map(|b| b.id.clone())
      .collect();

    if start_is_unit {
      self.blocks[start.node].kind = "paragraph".to_string();
      self.blocks[start.node].data = json!({});
    }
    self.blocks[start.node].text = merged;
    self.blocks[start.node].set_marks(&merged_marks);
    self.blocks.drain(start.node + 1..=end.node);

    ctx.ops.push(DocOp::UpdateBlock {
      block_id: self.blocks[start.node].id.clone(),
      kind: start_is_unit.then(|| "paragraph".to_string()),
      text: Some(self.blocks[start.node].text.clone()),
      data: Some(self.blocks[start.node].data.clone()),
    });
    for id in removed_ids {
      ctx.ops.push(DocOp::DeleteBlock { block_id: id });
    }
    ctx.typing_block = None;
    self.collapse_to(DocPosition::new(start.node, s + text_u16));
    true
  }

  // ── Text & IME commands ──────────────────────────────────────────────────

  fn cmd_insert_text(&mut self, text: &str, ctx: &mut DispatchCtx) {
    if text.is_empty() {
      return;
    }
    self.prose_input.reset();
    if self.blocks.is_empty() {
      self.ensure_not_empty(ctx);
    }
    let Some(sel) = self.selection.clone() else {
      return;
    };
    let normalized_text = text.replace("\r\n", "\n").replace('\r', "\n");

    // Atomic block at collapsed caret: insert a paragraph below.
    if sel.is_collapsed()
      && let Some(b) = self.blocks.get(sel.focus_node())
      && b.is_atomic()
    {
      let insert_at = sel.focus_node() + 1;
      let created = EditorBlock::new(self.gen_block_id(), "paragraph", &normalized_text);
      let caret_off = created.utf16_len();
      ctx.ops.push(self.insert_op(&created, insert_at));
      self.blocks.insert(insert_at, created);
      self.collapse_to(DocPosition::new(insert_at, caret_off));
      ctx.typing_block = None;
      return;
    }

    let single_block = !sel.is_multi_node();
    if self.splice_range(&sel, &normalized_text, &[], true, single_block, ctx) {
      if self.try_apply_input_rules(ctx) {
        ctx.typing_block = None;
        if self.slash_open {
          self.slash_open = false;
          ctx.effects.push(EditorEffect::CloseSlashMenu);
        }
        if self.page_link_open {
          self.page_link_open = false;
          ctx.effects.push(EditorEffect::ClosePageLinkMenu);
        }
      } else {
        self.refresh_trigger_effects(ctx);
      }
    }
  }

  #[allow(clippy::too_many_arguments)]
  fn cmd_ime_update(
    &mut self,
    text: &str,
    base: u32,
    extent: u32,
    composing_valid: bool,
    composing_collapsed: bool,
    shift_pressed: bool,
    ctx: &mut DispatchCtx,
  ) {
    if self.blocks.is_empty() {
      self.ensure_not_empty(ctx);
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let is_composing = composing_valid && !composing_collapsed;

    // Cross-block IME preedit / commit (exact port of prose_input.dart).
    let active_cross = self.prose_input.range.clone().or_else(|| {
      self
        .selection
        .as_ref()
        .filter(|s| s.is_multi_node())
        .cloned()
    });
    if let Some(range) = active_cross {
      let focus_idx = self.selection.as_ref().map_or(0, DocSelection::focus_node);
      if focus_idx >= self.blocks.len() {
        self.prose_input.reset();
        return;
      }
      self.prose_input.range = Some(range.clone());
      let base_text = self
        .prose_input
        .base_text
        .get_or_insert_with(|| self.blocks[focus_idx].text.clone())
        .clone();

      if is_composing {
        self.prose_input.composing = true;
        self.set_focused_text(&text, base, extent, ctx);
        return;
      }

      let inserted = inserted_text_utf16(&base_text, &text, None, None);
      let was_composing = self.prose_input.composing;
      self.prose_input.reset();
      if was_composing && inserted.is_empty() && text == base_text {
        let focus_off = range.focus_pos().offset;
        self.set_focused_text(&base_text, focus_off, focus_off, ctx);
        self.selection = range.clamp_and_snap(&self.blocks);
        return;
      }
      let cur_text = self.blocks[focus_idx].text.clone();
      let range_end = range.end_pos();
      let map_pos = |p: DocPosition| -> DocPosition {
        if p.node != focus_idx {
          p
        } else {
          DocPosition::new(
            p.node,
            map_preedit_offset(p.offset, &base_text, &cur_text, p == range_end),
          )
        }
      };
      let mapped =
        DocSelection::from_positions(map_pos(range.anchor_pos()), map_pos(range.focus_pos()));
      self.splice_range(&mapped, &inserted, &[], true, false, ctx);
      return;
    }

    let Some(sel) = self.selection.clone() else {
      return;
    };
    let idx = sel.focus_node();
    if idx >= self.blocks.len() || self.blocks[idx].is_atomic() {
      return;
    }

    if !is_composing {
      let old_text = self.blocks[idx].text.clone();
      let old_breaks = old_text.matches('\n').count();
      let new_breaks = text.matches('\n').count();
      let is_code = self.blocks[idx].is_code();

      // 2+ new newlines at once in a non-code block → structured Markdown paste.
      if !is_code && !shift_pressed && new_breaks >= old_breaks + 2 {
        let repaired = retag_mislabeled_fences(&unwrap_nested_fences(&text));
        let specs = parse_paste_blocks(&repaired);
        if !specs.is_empty() {
          self.replace_focused_with_blocks(&specs, ctx);
          return;
        }
      }

      // Single inserted newline in non-code block without Shift → Enter block split.
      let (sel_s, sel_e) = if !sel.is_multi_node() && !sel.is_collapsed() {
        (Some(sel.start_pos().offset), Some(sel.end_pos().offset))
      } else {
        (None, None)
      };
      let inserted = inserted_text_utf16(&old_text, &text, sel_s, sel_e);
      if inserted == "\n" && !is_code && !shift_pressed {
        let tlen = utf16_len(&text);
        let caret = base.min(tlen);
        let nl_at = if caret > 0 && utf16_slice(&text, caret - 1, caret) == "\n" {
          caret - 1
        } else {
          crate::utf16::utf16_find_char_from(&text, '\n', 0).unwrap_or(caret)
        };
        let before = utf16_slice(&text, 0, nl_at).to_string();
        let after = utf16_slice(&text, nl_at + 1, tlen).to_string();
        self.apply_newline_split(&before, &after, ctx);
        return;
      }

      // Single newline typed inside a code block → auto-indent or language claim.
      let tlen = utf16_len(&text);
      let olen = utf16_len(&old_text);
      if is_code
        && text.contains('\n')
        && base == extent
        && base > 0
        && tlen == olen + 1
        && utf16_slice(&text, base - 1, base) == "\n"
      {
        self.insert_code_newline(base, ctx);
        return;
      }
    }

    self.set_focused_text(&text, base, extent, ctx);
    if !is_composing {
      if self.try_apply_input_rules(ctx) {
        ctx.typing_block = None;
        if self.slash_open {
          self.slash_open = false;
          ctx.effects.push(EditorEffect::CloseSlashMenu);
        }
        if self.page_link_open {
          self.page_link_open = false;
          ctx.effects.push(EditorEffect::ClosePageLinkMenu);
        }
      } else {
        self.refresh_trigger_effects(ctx);
      }
    }
  }

  fn set_focused_text(&mut self, text: &str, sel_start: u32, sel_end: u32, ctx: &mut DispatchCtx) {
    let Some(sel) = self.selection.clone() else {
      return;
    };
    let idx = sel.focus_node();
    if idx >= self.blocks.len() || self.blocks[idx].is_atomic() {
      return;
    }
    let old = self.blocks[idx].text.clone();
    let new_len = utf16_len(text);
    if old == text {
      self.selection = DocSelection::from_positions(
        DocPosition::new(idx, sel_start.min(new_len)),
        DocPosition::new(idx, sel_end.min(new_len)),
      )
      .clamp_and_snap(&self.blocks);
      return;
    }
    let same_node = !sel.is_multi_node() && sel.focus_node() == idx;
    let (prefix, old_end) = diff_edit_range(
      &old,
      text,
      same_node.then(|| sel.start_pos().offset),
      same_node.then(|| sel.end_pos().offset),
      (sel_start == sel_end).then(|| sel_end.min(new_len)),
    );
    let inserted_end =
      (old_end as i64 + new_len as i64 - utf16_len(&old) as i64).max(prefix as i64) as u32;
    let inserted_slice = utf16_slice(text, prefix, inserted_end);
    let replace_sel = DocSelection::from_positions(
      DocPosition::new(idx, prefix),
      DocPosition::new(idx, old_end),
    );
    self.splice_range(&replace_sel, inserted_slice, &[], true, true, ctx);

    // If typing landed inside a math run's leading edge, trim the swallowed prefix out.
    let caret = sel_end.min(new_len);
    let current_marks = self.blocks[idx].marks();
    if let Some(swallowed) = math_run_at(&current_marks, caret).cloned() {
      let adjusted: Vec<EditorMark> = current_marks
        .into_iter()
        .filter_map(|m| {
          if m == swallowed {
            (m.end > caret).then(|| m.with_range(caret, m.end))
          } else {
            Some(m)
          }
        })
        .collect();
      self.blocks[idx].set_marks(&adjusted);
      if let Some(last) = ctx.ops.last_mut() {
        *last = Self::update_text_data_op(&self.blocks[idx]);
      }
    }

    self.selection = DocSelection::from_positions(
      DocPosition::new(idx, sel_start.min(new_len)),
      DocPosition::new(idx, sel_end.min(new_len)),
    )
    .clamp_and_snap(&self.blocks);
  }

  /// Apply block or inline keyboard input rules on the focused block.
  pub(crate) fn try_apply_input_rules(&mut self, ctx: &mut DispatchCtx) -> bool {
    let Some(DocSelection::Caret {
      block_index,
      offset,
    }) = self.selection
    else {
      return false;
    };
    let Some(block) = self.blocks.get(block_index) else {
      return false;
    };
    let Some(rule) = match_input_rule(block, offset) else {
      return false;
    };

    match rule {
      InputRuleMatch::ConvertBlock {
        kind,
        data,
        text,
        marks,
      } => {
        let prev_li = self.blocks[block_index].li_level();
        let prev_is_list = self.blocks[block_index].is_list_kind();
        let attach_owner = if kind == "code_block" && prev_is_list {
          self.owning_item_indent_at(block_index)
        } else {
          None
        };
        let mut new_data = normalize_data_object(data);
        if let Value::Object(m) = &mut new_data {
          if !marks.is_empty() {
            m.insert("marks".to_string(), marks_to_json(&marks));
          }
          if EditorBlock::is_attachable_kind(&kind) {
            if let Some(lv) = prev_li.or(attach_owner) {
              m.insert("li".to_string(), json!(lv));
            }
          }
        }
        let b = &mut self.blocks[block_index];
        b.kind = kind;
        b.text = text;
        b.data = new_data;
        ctx.ops.push(Self::update_full_op(b));
        ctx.typing_block = None;
        self.collapse_to(DocPosition::new(block_index, 0));
        true
      }
      InputRuleMatch::InsertDivider => {
        self.blocks[block_index].text.clear();
        self.cmd_insert_divider(ctx);
        true
      }
      InputRuleMatch::ConvertToMathBlock { source } => {
        let b = &mut self.blocks[block_index];
        b.kind = "math_block".to_string();
        b.text = source;
        b.data = json!({});
        ctx.ops.push(Self::update_full_op(b));
        let after = block_index + 1;
        if after >= self.blocks.len() || self.blocks[after].is_atomic() {
          let p = EditorBlock::new(self.gen_block_id(), "paragraph", "");
          ctx.ops.push(self.insert_op(&p, after));
          self.blocks.insert(after, p);
        }
        ctx.typing_block = None;
        self.collapse_to(DocPosition::new(after, 0));
        true
      }
      InputRuleMatch::InlineMark {
        text,
        marks,
        new_caret,
      } => {
        let b = &mut self.blocks[block_index];
        b.text = text;
        b.set_marks(&marks);
        ctx.ops.push(Self::update_text_data_op(b));
        ctx.typing_block = None;
        self.collapse_to(DocPosition::new(block_index, new_caret));
        true
      }
    }
  }

  // ── Split / Delete / Merge ───────────────────────────────────────────────

  fn cmd_split_block(&mut self, ctx: &mut DispatchCtx) {
    self.prose_input.reset();
    let Some(sel) = self.selection.clone() else {
      return;
    };
    if !sel.is_collapsed() {
      self.delete_current_selection(ctx);
    }
    let Some(sel) = self.selection.clone() else {
      return;
    };
    let idx = sel.focus_node();
    if idx >= self.blocks.len() {
      return;
    }

    // Atomic block: insert an empty paragraph below.
    if self.blocks[idx].is_atomic() {
      let created = EditorBlock::new(self.gen_block_id(), "paragraph", "");
      ctx.ops.push(self.insert_op(&created, idx + 1));
      self.blocks.insert(idx + 1, created);
      self.collapse_to(DocPosition::new(idx + 1, 0));
      ctx.typing_block = None;
      return;
    }

    // Code block: auto-indent soft newline (or claim language tag).
    if self.blocks[idx].is_code() {
      let at = sel.focus_pos().offset.min(self.blocks[idx].utf16_len());
      self.insert_code_newline(at + 1, ctx);
      return;
    }

    let node = self.blocks[idx].clone();
    let len = node.utf16_len();
    let at = snap_utf16_boundary(&node.text, sel.focus_pos().offset.min(len), false);

    // Enter on empty nested list item outdents one level first; at indent 0
    // (or for quote/todo at indent 0), exits to a plain paragraph.
    if node.text.is_empty() && EditorBlock::continues_on_enter(&node.kind) {
      if node.is_list_kind() && node.indent() > 0 {
        self.cmd_indent(-1, ctx);
        return;
      }
      let b = &mut self.blocks[idx];
      b.kind = "paragraph".to_string();
      b.data = json!({});
      ctx.ops.push(Self::update_full_op(b));
      self.collapse_to(DocPosition::new(idx, 0));
      ctx.typing_block = None;
      return;
    }

    // Enter at offset 0 of non-empty block: insert empty paragraph ABOVE,
    // preserving the formatted block below.
    if at == 0 && !node.text.is_empty() {
      let created = EditorBlock::new(self.gen_block_id(), "paragraph", "");
      ctx.ops.push(self.insert_op(&created, idx));
      self.blocks.insert(idx, created);
      self.collapse_to(DocPosition::new(idx + 1, 0));
      ctx.typing_block = None;
      return;
    }

    let before = utf16_slice(&node.text, 0, at).to_string();
    let after = utf16_slice(&node.text, at, len).to_string();
    let continues = EditorBlock::continues_on_enter(&node.kind);
    let new_kind = if continues {
      node.kind.clone()
    } else {
      "paragraph".to_string()
    };
    let mut new_data = if continues {
      continuation_data(&node)
    } else {
      json!({})
    };
    let (before_marks, after_marks) = split_marks(&node.marks(), at);
    if !after_marks.is_empty()
      && let Value::Object(m) = &mut new_data
    {
      m.insert("marks".to_string(), marks_to_json(&after_marks));
    }

    let created = EditorBlock::new(self.gen_block_id(), new_kind, after).with_data(new_data);
    self.blocks[idx].text = before;
    self.blocks[idx].set_marks(&before_marks);
    ctx.ops.push(Self::update_text_data_op(&self.blocks[idx]));
    ctx.ops.push(self.insert_op(&created, idx + 1));
    self.blocks.insert(idx + 1, created);
    ctx.typing_block = None;
    self.collapse_to(DocPosition::new(idx + 1, 0));
  }

  fn apply_newline_split(&mut self, before: &str, after: &str, ctx: &mut DispatchCtx) {
    let Some(sel) = self.selection.clone() else {
      return;
    };
    let idx = sel.focus_node();
    if idx >= self.blocks.len() {
      return;
    }
    let node = self.blocks[idx].clone();

    if before.is_empty() && after.is_empty() && EditorBlock::continues_on_enter(&node.kind) {
      let b = &mut self.blocks[idx];
      b.kind = "paragraph".to_string();
      b.text.clear();
      b.data = json!({});
      ctx.ops.push(Self::update_full_op(b));
      self.collapse_to(DocPosition::new(idx, 0));
      ctx.typing_block = None;
      return;
    }

    // Empty first line of multi-line quote/list leaves the group Typora-style.
    if before.is_empty()
      && let Some(remainder) = after.strip_prefix('\n')
      && EditorBlock::continues_on_enter(&node.kind)
    {
      let rem_len = utf16_len(remainder);
      let marks = shift_marks(&node.marks(), 0, 1, -1, rem_len, true);
      self.blocks[idx].text = remainder.to_string();
      self.blocks[idx].set_marks(&marks);
      let created = EditorBlock::new(self.gen_block_id(), "paragraph", "");
      ctx.ops.push(self.insert_op(&created, idx));
      ctx.ops.push(Self::update_text_data_op(&self.blocks[idx]));
      self.blocks.insert(idx, created);
      self.collapse_to(DocPosition::new(idx, 0));
      ctx.typing_block = None;
      return;
    }

    if before.is_empty() && !after.is_empty() && after == node.text {
      let created = EditorBlock::new(self.gen_block_id(), "paragraph", "");
      ctx.ops.push(self.insert_op(&created, idx));
      self.blocks.insert(idx, created);
      self.collapse_to(DocPosition::new(idx + 1, 0));
      ctx.typing_block = None;
      return;
    }

    // Remap marks across the IME edit (`node.text -> before+after`) before splitting (issue #131).
    let flat = format!("{before}{after}");
    let flat_len = utf16_len(&flat);
    let old_len = node.utf16_len();
    let prefix = common_prefix_utf16(&node.text, &flat);
    let old_end = old_len - common_suffix_utf16(&node.text, &flat, prefix);
    let remapped = shift_marks(
      &node.marks(),
      prefix,
      old_end,
      flat_len as i64 - old_len as i64,
      flat_len,
      true,
    );
    let before_u16 = utf16_len(before).min(flat_len);
    let (before_marks, after_marks) = split_marks(&remapped, before_u16);

    let continues = EditorBlock::continues_on_enter(&node.kind);
    let mut new_data = if continues {
      continuation_data(&node)
    } else {
      json!({})
    };
    if !after_marks.is_empty()
      && let Value::Object(m) = &mut new_data
    {
      m.insert("marks".to_string(), marks_to_json(&after_marks));
    }
    let created = EditorBlock::new(
      self.gen_block_id(),
      if continues {
        node.kind.clone()
      } else {
        "paragraph".to_string()
      },
      after,
    )
    .with_data(new_data);

    self.blocks[idx].text = before.to_string();
    self.blocks[idx].set_marks(&before_marks);
    ctx.ops.push(Self::update_text_data_op(&self.blocks[idx]));
    ctx.ops.push(self.insert_op(&created, idx + 1));
    self.blocks.insert(idx + 1, created);
    ctx.typing_block = None;
    self.collapse_to(DocPosition::new(idx + 1, 0));
  }

  fn insert_code_newline(&mut self, caret_after_nl: u32, ctx: &mut DispatchCtx) {
    let Some(sel) = self.selection.clone() else {
      return;
    };
    let idx = sel.focus_node();
    if idx >= self.blocks.len() || !self.blocks[idx].is_code() {
      return;
    }
    let len = self.blocks[idx].utf16_len();
    let at = caret_after_nl.saturating_sub(1).min(len);
    let before = utf16_slice(&self.blocks[idx].text, 0, at).to_string();
    let after = utf16_slice(&self.blocks[idx].text, at, len).to_string();

    // Claim language tag if first line is a known code language (` ```yaml⏎ `).
    let tag = before.trim().to_ascii_lowercase();
    let has_lang = self.blocks[idx].data_map().contains_key("language");
    if after.is_empty()
      && !before.contains('\n')
      && !has_lang
      && tag != "auto"
      && mica_md::highlight::code_languages()
        .iter()
        .any(|l| l == &tag)
    {
      self.blocks[idx].text.clear();
      self.blocks[idx]
        .data_map_mut()
        .insert("language".to_string(), Value::String(tag));
      ctx.ops.push(Self::update_text_data_op(&self.blocks[idx]));
      ctx.typing_block = None;
      self.collapse_to(DocPosition::new(idx, 0));
      return;
    }

    let line_start = before.rfind('\n').map_or(0, |p| p + 1);
    let line = &before[line_start..];
    let indent_len_bytes = line.len() - line.trim_start_matches([' ', '\t']).len();
    let indent = &line[..indent_len_bytes];
    let next_text = format!("{before}\n{indent}{after}");
    let next_pos = at + 1 + utf16_len(indent);
    self.blocks[idx].text = next_text;
    ctx.ops.push(Self::update_text_data_op(&self.blocks[idx]));
    ctx.typing_block = Some(self.blocks[idx].id.clone());
    self.collapse_to(DocPosition::new(idx, next_pos));
  }

  fn cmd_delete_backward(&mut self, ctx: &mut DispatchCtx) {
    self.prose_input.reset();
    let Some(sel) = self.selection.clone() else {
      return;
    };
    if !sel.is_collapsed() {
      self.delete_current_selection(ctx);
      return;
    }
    let idx = sel.focus_node();
    if idx >= self.blocks.len() {
      return;
    }
    if self.blocks[idx].is_atomic() {
      self.remove_atomic_block(idx, ctx);
      return;
    }

    let caret = sel.focus_pos().offset.min(self.blocks[idx].utf16_len());
    // Atomic math run ending at caret deletes the whole formula.
    if let Some(run) = math_run_ending_at(&self.blocks[idx].marks(), caret).cloned() {
      self.cmd_set_inline_math(idx, run.start, run.end, "", ctx);
      return;
    }

    if caret > 0 {
      let prev = utf16_prev_boundary(&self.blocks[idx].text, caret);
      let del_range =
        DocSelection::from_positions(DocPosition::new(idx, prev), DocPosition::new(idx, caret));
      self.splice_range(&del_range, "", &[], true, true, ctx);
      self.refresh_trigger_effects(ctx);
      return;
    }

    // Offset 0: structural backward merge (mirrors `controller.dart::mergeBackward`).
    self.merge_backward(idx, ctx);
  }

  fn merge_backward(&mut self, idx: usize, ctx: &mut DispatchCtx) {
    let cur = self.blocks[idx].clone();

    // Nested list item at offset 0: outdent first before shedding marker.
    if cur.is_list_kind() && cur.indent() > 0 {
      self.cmd_indent(-1, ctx);
      return;
    }

    // Styled empty block → plain paragraph.
    if cur.text.is_empty() && cur.kind != "paragraph" {
      let b = &mut self.blocks[idx];
      b.kind = "paragraph".to_string();
      b.data = json!({});
      ctx.ops.push(Self::update_full_op(b));
      ctx.typing_block = None;
      return;
    }

    // List item sheds its marker first (Typora/Notion).
    if cur.is_list_kind() {
      let b = &mut self.blocks[idx];
      b.kind = "paragraph".to_string();
      let map = b.data_map_mut();
      map.remove("checked");
      map.remove("indent");
      map.remove("start");
      map.remove("marker");
      ctx.ops.push(Self::update_full_op(b));
      ctx.typing_block = None;
      return;
    }

    // Empty first line drops when there is a block below (2026-08-28 fix).
    if idx == 0 {
      if self.blocks.len() >= 2 && cur.text.is_empty() && !cur.is_atomic() {
        let removed_id = self.blocks.remove(0).id;
        ctx.ops.push(DocOp::DeleteBlock {
          block_id: removed_id,
        });
        ctx.typing_block = None;
        self.collapse_to(DocPosition::new(0, 0));
      }
      return;
    }

    let prev = self.blocks[idx - 1].clone();

    // Empty line below atomic block: drop the blank line and focus the atomic block
    // (2-press picture protection, 2026-08-12 fix).
    if prev.is_atomic() && cur.text.is_empty() {
      let removed_id = self.blocks.remove(idx).id;
      ctx.ops.push(DocOp::DeleteBlock {
        block_id: removed_id,
      });
      ctx.typing_block = None;
      self.selection = Some(DocSelection::block_atom(idx - 1));
      return;
    }

    // Non-empty line below atomic block: remove the atomic block, keeping text.
    if prev.is_atomic() {
      let removed_id = self.blocks.remove(idx - 1).id;
      ctx.ops.push(DocOp::DeleteBlock {
        block_id: removed_id,
      });
      ctx.typing_block = None;
      self.collapse_to(DocPosition::new(idx - 1, 0));
      return;
    }

    // Empty line above: remove it whole so current block keeps its kind (e.g. heading).
    if prev.text.is_empty() {
      let removed_id = self.blocks.remove(idx - 1).id;
      ctx.ops.push(DocOp::DeleteBlock {
        block_id: removed_id,
      });
      ctx.typing_block = None;
      self.collapse_to(DocPosition::new(idx - 1, 0));
      return;
    }

    // Merge current block's text + marks into previous block.
    let junction = prev.utf16_len();
    let merged_text = format!("{}{}", prev.text, cur.text);
    let merged_marks = concat_marks(&prev.marks(), &cur.marks(), junction);
    self.blocks[idx - 1].text = merged_text;
    self.blocks[idx - 1].set_marks(&merged_marks);
    let removed_id = self.blocks.remove(idx).id;

    ctx
      .ops
      .push(Self::update_text_data_op(&self.blocks[idx - 1]));
    ctx.ops.push(DocOp::DeleteBlock {
      block_id: removed_id,
    });
    ctx.typing_block = None;
    self.collapse_to(DocPosition::new(idx - 1, junction));
  }

  fn cmd_delete_forward(&mut self, ctx: &mut DispatchCtx) {
    self.prose_input.reset();
    let Some(sel) = self.selection.clone() else {
      return;
    };
    if !sel.is_collapsed() {
      self.delete_current_selection(ctx);
      return;
    }
    let idx = sel.focus_node();
    if idx >= self.blocks.len() {
      return;
    }
    if self.blocks[idx].is_atomic() {
      self.remove_atomic_block(idx, ctx);
      return;
    }

    let len = self.blocks[idx].utf16_len();
    let caret = sel.focus_pos().offset.min(len);
    if let Some(run) = math_run_starting_at(&self.blocks[idx].marks(), caret).cloned() {
      self.cmd_set_inline_math(idx, run.start, run.end, "", ctx);
      return;
    }

    if caret < len {
      let next = utf16_next_boundary(&self.blocks[idx].text, caret);
      let del_range =
        DocSelection::from_positions(DocPosition::new(idx, caret), DocPosition::new(idx, next));
      self.splice_range(&del_range, "", &[], true, true, ctx);
      return;
    }

    if idx + 1 >= self.blocks.len() {
      return;
    }

    // Empty plain paragraph with a block below: drop the empty line itself so
    // the following block rises with its kind/formatting intact (and atomic
    // blocks like images are selected rather than deleted on first press).
    if self.blocks[idx].text.is_empty()
      && self.blocks[idx].kind == "paragraph"
      && !self.blocks[idx].is_atomic()
    {
      let next_is_atomic = self.blocks[idx + 1].is_atomic();
      let removed_id = self.blocks.remove(idx).id;
      ctx.ops.push(DocOp::DeleteBlock {
        block_id: removed_id,
      });
      ctx.typing_block = None;
      if next_is_atomic {
        self.selection = Some(DocSelection::block_atom(idx));
      } else {
        self.collapse_to(DocPosition::new(idx, 0));
      }
      return;
    }

    if self.blocks[idx + 1].is_atomic() {
      let removed_id = self.blocks.remove(idx + 1).id;
      ctx.ops.push(DocOp::DeleteBlock {
        block_id: removed_id,
      });
      ctx.typing_block = None;
      self.collapse_to(DocPosition::new(idx, len));
      return;
    }

    let next_block = self.blocks.remove(idx + 1);
    let merged_text = format!("{}{}", self.blocks[idx].text, next_block.text);
    let merged_marks = concat_marks(&self.blocks[idx].marks(), &next_block.marks(), len);
    self.blocks[idx].text = merged_text;
    self.blocks[idx].set_marks(&merged_marks);
    ctx.ops.push(Self::update_text_data_op(&self.blocks[idx]));
    ctx.ops.push(DocOp::DeleteBlock {
      block_id: next_block.id,
    });
    ctx.typing_block = None;
    self.collapse_to(DocPosition::new(idx, len));
  }

  pub(crate) fn delete_current_selection(&mut self, ctx: &mut DispatchCtx) -> bool {
    let Some(sel) = self.selection.clone() else {
      return false;
    };
    match sel.normalized() {
      NormalizedSelection::BlockAtom { block_index } => {
        if block_index < self.blocks.len() {
          self.remove_atomic_block(block_index, ctx);
          return true;
        }
        false
      }
      NormalizedSelection::TableArea {
        block_index,
        r0,
        c0,
        r1,
        c1,
        ..
      } => {
        self.cmd_table_op(
          block_index,
          crate::table::TableOp::ClearCells { r0, c0, r1, c1 },
          ctx,
        );
        true
      }
      NormalizedSelection::Text { start, end } => {
        if start == end {
          if let Some(b) = self.blocks.get(start.node)
            && b.is_atomic()
          {
            self.remove_atomic_block(start.node, ctx);
            return true;
          }
          return false;
        }
        self.splice_range(&sel, "", &[], true, false, ctx)
      }
    }
  }

  pub(crate) fn remove_atomic_block(&mut self, idx: usize, ctx: &mut DispatchCtx) {
    if idx >= self.blocks.len() {
      return;
    }
    let removed = self.blocks.remove(idx);
    ctx.ops.push(DocOp::DeleteBlock {
      block_id: removed.id,
    });
    ctx.typing_block = None;
    if self.blocks.is_empty() {
      self.ensure_not_empty(ctx);
      return;
    }
    let target = idx.saturating_sub(1).min(self.blocks.len() - 1);
    let offset = if idx > 0 {
      self.blocks[target].utf16_len()
    } else {
      0
    };
    self.collapse_to(DocPosition::new(target, offset));
  }

  fn cmd_delete_selected_blocks(&mut self, ctx: &mut DispatchCtx) {
    self.prose_input.reset();
    let Some(sel) = self.selection.clone() else {
      return;
    };
    if self.blocks.is_empty() {
      return;
    }
    let (lo, hi) = match sel.normalized() {
      NormalizedSelection::BlockAtom { block_index }
      | NormalizedSelection::TableArea { block_index, .. } => (block_index, block_index),
      NormalizedSelection::Text { start, end } => (start.node, end.node),
    };
    let from = lo.min(self.blocks.len() - 1);
    let to = hi.clamp(from, self.blocks.len() - 1);
    let removed: Vec<EditorBlock> = self.blocks.drain(from..=to).collect();
    for b in removed {
      ctx.ops.push(DocOp::DeleteBlock { block_id: b.id });
    }
    ctx.typing_block = None;
    if self.blocks.is_empty() {
      self.ensure_not_empty(ctx);
      return;
    }
    let target = from.saturating_sub(1).min(self.blocks.len() - 1);
    let offset = if from > 0 {
      self.blocks[target].utf16_len()
    } else {
      0
    };
    self.collapse_to(DocPosition::new(target, offset));
  }

  // ── Formatting, links, undo/redo, remote merge ───────────────────────────

  fn cmd_toggle_mark(&mut self, kind: &str, href: Option<&str>, ctx: &mut DispatchCtx) {
    let Some(sel) = self.selection.clone() else {
      return;
    };
    let ranges = inline_selection_ranges(&self.blocks, &sel);
    if ranges.is_empty() {
      return;
    }
    let add = !ranges
      .iter()
      .all(|r| range_has_mark(&self.blocks[r.block_index].marks(), r.start, r.end, kind));
    if kind == "link" && add && href.is_none_or(str::is_empty) {
      return;
    }
    for r in ranges {
      let b = &mut self.blocks[r.block_index];
      let next_marks =
        crate::marks::apply_or_remove_mark(&b.marks(), r.start, r.end, kind, href, None, add);
      b.set_marks(&next_marks);
      ctx.ops.push(Self::update_text_data_op(b));
    }
    ctx.typing_block = None;
  }

  pub(crate) fn cmd_set_link(
    &mut self,
    block_index: usize,
    start: u32,
    end: u32,
    href: Option<&str>,
    replacement_text: Option<&str>,
    ctx: &mut DispatchCtx,
  ) {
    if block_index >= self.blocks.len() {
      return;
    }
    let b = &self.blocks[block_index];
    if b.is_atomic() || b.is_code() || b.kind == "table" {
      return;
    }
    let len = b.utf16_len();
    let s = snap_utf16_boundary(&b.text, start.min(len), false);
    let e = snap_utf16_boundary(&b.text, end.clamp(s, len), true);

    if let Some(label) = replacement_text {
      let Some(url) = href.filter(|u| !u.is_empty()) else {
        return;
      };
      let label_u16 = utf16_len(label);
      let (new_text, rs, re) = utf16_replace_range(&b.text, s, e, label);
      let new_len = utf16_len(&new_text);
      let shifted = shift_marks(
        &b.marks(),
        rs,
        re,
        new_len as i64 - len as i64,
        new_len,
        false,
      );
      let next_marks = apply_mark(&shifted, rs, rs + label_u16, "link", Some(url), None);
      let bm = &mut self.blocks[block_index];
      bm.text = new_text;
      bm.set_marks(&next_marks);
      ctx.ops.push(Self::update_text_data_op(bm));
      ctx.typing_block = None;
      self.collapse_to(DocPosition::new(block_index, rs + label_u16));
      return;
    }

    let prev_marks = b.marks();
    let mut next_marks = crate::marks::remove_mark(&prev_marks, s, e, "link");
    if let Some(url) = href.filter(|u| !u.is_empty()) {
      next_marks = apply_mark(&next_marks, s, e, "link", Some(url), None);
    }
    if next_marks == prev_marks {
      return;
    }
    let bm = &mut self.blocks[block_index];
    bm.set_marks(&next_marks);
    ctx.ops.push(Self::update_text_data_op(bm));
    ctx.typing_block = None;
  }

  fn cmd_undo_redo(&mut self, redo: bool, ctx: &mut DispatchCtx) {
    self.prose_input.reset();
    ctx.doc_mutated_directly = true;
    let _ = self.doc.take_changed_block_ids();
    let Some(restored_sel) = self.history.step(&mut self.doc, redo) else {
      ctx.direct_dirty_blocks = Some(Vec::new());
      return;
    };
    if let Some(changed_ids) = self.doc.take_changed_block_ids() {
      // Fast path: tree shape did not change — re-read only the touched blocks.
      let body_ids: Vec<String> = changed_ids
        .into_iter()
        .filter(|id| id != &self.root_block_id)
        .collect();
      let mut patches = Vec::with_capacity(body_ids.len());
      if !body_ids.is_empty() {
        let updated = self.doc.blocks_by_id(&body_ids);
        let mut ops = Vec::with_capacity(updated.len());
        for core_block in &updated {
          if let Some(idx) = self.blocks.iter().position(|b| b.id == core_block.id) {
            let next = EditorBlock::from(core_block);
            let old = &self.blocks[idx];
            if old.kind != next.kind
              || old.text != next.text
              || !block_data_eq(&old.data, &next.data)
            {
              ops.push(DocOp::UpdateBlock {
                block_id: next.id.clone(),
                kind: (old.kind != next.kind).then(|| next.kind.clone()),
                text: (old.text != next.text).then(|| next.text.clone()),
                data: Some(next.data.clone()),
              });
              self.blocks[idx] = next.clone();
              patches.push(BlockPatch::Upsert {
                index: idx,
                block: next,
              });
            }
          }
        }
        ctx.ops = ops;
      }
      ctx.direct_dirty_blocks = Some(patches);
    } else {
      // Slow path: tree structure changed (split/merge/delete/paste/move).
      let mut next_blocks = read_body_blocks(&self.doc, &self.root_block_id);
      if next_blocks.is_empty() {
        next_blocks.push(EditorBlock::new(self.gen_block_id(), "paragraph", ""));
      }
      let prev_blocks = std::mem::replace(&mut self.blocks, next_blocks);
      ctx.ops = diff_block_ops(&self.root_block_id, &prev_blocks, &self.blocks);
      ctx.direct_dirty_blocks = Some(diff_block_patches(&prev_blocks, &self.blocks));
    }
    self.selection = restored_sel
      .restore(&self.blocks)
      .or_else(|| {
        self
          .selection
          .as_ref()
          .and_then(|s| s.clamp_and_snap(&self.blocks))
      })
      .or_else(|| default_initial_selection(&self.blocks));
  }

  fn cmd_apply_remote_update(&mut self, update: &[u8], ctx: &mut DispatchCtx) {
    self.prose_input.reset();
    self.history.flush_typing_batch(&mut self.doc);
    ctx.doc_mutated_directly = true;
    ctx.is_remote = true;

    let _ = self.doc.take_changed_block_ids();
    if let Err(e) = self.doc.apply_update(update) {
      ctx.error = Some(e.to_string());
      ctx.direct_dirty_blocks = Some(Vec::new());
      return;
    }
    let root = self.doc.root_block_id();
    let root_changed = !root.is_empty() && root != self.root_block_id;
    if root_changed {
      self.root_block_id = root;
    }
    if !root_changed
      && let Some(changed_ids) = self.doc.take_changed_block_ids()
    {
      // Fast path: remote update only modified block contents, not tree shape.
      let body_ids: Vec<String> = changed_ids
        .into_iter()
        .filter(|id| id != &self.root_block_id)
        .collect();
      let mut patches = Vec::with_capacity(body_ids.len());
      let mut old_texts: HashMap<usize, String> = HashMap::new();
      if !body_ids.is_empty() {
        let updated = self.doc.blocks_by_id(&body_ids);
        for core_block in &updated {
          if let Some(idx) = self.blocks.iter().position(|b| b.id == core_block.id) {
            let next = EditorBlock::from(core_block);
            let old = &self.blocks[idx];
            if old.kind != next.kind
              || old.text != next.text
              || !block_data_eq(&old.data, &next.data)
            {
              old_texts.insert(idx, old.text.clone());
              self.blocks[idx] = next.clone();
              patches.push(BlockPatch::Upsert {
                index: idx,
                block: next,
              });
            }
          }
        }
      }
      if !old_texts.is_empty() {
        let sel_touches_changed = self.selection.as_ref().is_some_and(|s| {
          old_texts.contains_key(&s.anchor_pos().node) || old_texts.contains_key(&s.focus_node())
        });
        if sel_touches_changed {
          let prev_pairs: Vec<(String, String)> = self
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| {
              let txt = old_texts
                .get(&i)
                .cloned()
                .unwrap_or_else(|| b.text.clone());
              (b.id.clone(), txt)
            })
            .collect();
          self.selection = reconcile_selection(&prev_pairs, self.selection.as_ref(), &self.blocks)
            .or_else(|| default_initial_selection(&self.blocks));
        }
      }
      ctx.direct_dirty_blocks = Some(patches);
      return;
    }

    let next_blocks = read_body_blocks(&self.doc, &self.root_block_id);
    let prev_blocks = std::mem::replace(&mut self.blocks, next_blocks);
    let prev_pairs: Vec<(String, String)> = prev_blocks
      .iter()
      .map(|b| (b.id.clone(), b.text.clone()))
      .collect();
    self.selection = reconcile_selection(&prev_pairs, self.selection.as_ref(), &self.blocks)
      .or_else(|| default_initial_selection(&self.blocks));
    ctx.direct_dirty_blocks = Some(diff_block_patches(&prev_blocks, &self.blocks));
  }

  // ── Find & Replace / AI Edits ────────────────────────────────────────────

  fn cmd_find_replace(&mut self, op: FindReplaceOp, ctx: &mut DispatchCtx) {
    match op {
      FindReplaceOp::FindAll {
        query,
        case_sensitive,
      } => {
        ctx.find_matches = Some(find_all_matches(&self.blocks, &query, case_sensitive));
      }
      FindReplaceOp::ReplaceOne {
        block_index,
        start,
        end,
        replacement,
      } => {
        if block_index < self.blocks.len() && !self.blocks[block_index].is_atomic() {
          let len = self.blocks[block_index].utf16_len();
          if start <= end && end <= len {
            let range = DocSelection::from_positions(
              DocPosition::new(block_index, start),
              DocPosition::new(block_index, end),
            );
            if self.splice_range(&range, &replacement, &[], true, false, ctx) {
              ctx.replaced_count = Some(1);
            }
          }
        }
      }
      FindReplaceOp::ReplaceAll {
        query,
        replacement,
        case_sensitive,
      } => {
        if query.is_empty() {
          ctx.replaced_count = Some(0);
          return;
        }
        let matches = find_all_matches(&self.blocks, &query, case_sensitive);
        if matches.is_empty() {
          ctx.replaced_count = Some(0);
          return;
        }
        let mut by_block: BTreeMap<usize, Vec<(u32, u32)>> = BTreeMap::new();
        for m in &matches {
          by_block
            .entry(m.block_index)
            .or_default()
            .push((m.start, m.end));
        }
        let mut count = 0usize;
        for (idx, spans) in by_block {
          let b = &mut self.blocks[idx];
          if b.is_atomic() {
            continue;
          }
          let mut text = b.text.clone();
          let mut marks = b.marks();
          for &(s, e) in spans.iter().rev() {
            let old_len = utf16_len(&text);
            let (next_text, rs, re) = utf16_replace_range(&text, s, e, &replacement);
            let new_len = utf16_len(&next_text);
            marks = shift_marks(
              &marks,
              rs,
              re,
              new_len as i64 - old_len as i64,
              new_len,
              true,
            );
            text = next_text;
            count += 1;
          }
          b.text = text;
          b.set_marks(&marks);
          ctx.ops.push(Self::update_text_data_op(b));
        }
        ctx.typing_block = None;
        ctx.replaced_count = Some(count);
        self.selection = self
          .selection
          .as_ref()
          .and_then(|s| s.clamp_and_snap(&self.blocks));
      }
    }
  }

  fn cmd_apply_ai_edits(
    &mut self,
    snapshot: &AiSnapshot,
    preview: &AiEditPreview,
    ctx: &mut DispatchCtx,
  ) {
    if snapshot.root_block_id != self.root_block_id || preview.edits.is_empty() {
      ctx.error = Some("AI edit conflict: empty preview or mismatched root".to_string());
      return;
    }
    let original: HashMap<&str, &EditorBlock> =
      snapshot.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
    let positions: HashMap<&str, usize> = self
      .blocks
      .iter()
      .enumerate()
      .map(|(i, b)| (b.id.as_str(), i))
      .collect();
    let mut occupied: HashSet<&str> = HashSet::new();
    let mut ranges: Vec<(usize, usize, &crate::commands::AiBlockEdit)> = Vec::new();

    // Validate EVERY targeted range before mutating anything (exact semantics of
    // `controller.dart::applyAiEdits`).
    for edit in &preview.edits {
      if edit.block_ids.is_empty() {
        ctx.error = Some("AI edit conflict: empty block_ids".to_string());
        return;
      }
      let Some(&start) = positions.get(edit.block_ids[0].as_str()) else {
        ctx.error = Some("AI edit conflict: target block missing".to_string());
        return;
      };
      for (offset, id) in edit.block_ids.iter().enumerate() {
        let Some(&before) = original.get(id.as_str()) else {
          ctx.error = Some("AI edit conflict: snapshot missing target block".to_string());
          return;
        };
        let idx = start + offset;
        if !occupied.insert(id.as_str())
          || idx >= self.blocks.len()
          || self.blocks[idx].id != *id
          || self.blocks[idx].kind != before.kind
          || self.blocks[idx].text != before.text
          || !block_data_eq(&self.blocks[idx].data, &before.data)
        {
          ctx.error = Some("AI edit conflict: document changed since snapshot".to_string());
          return;
        }
      }
      ranges.push((start, start + edit.block_ids.len(), edit));
    }

    let mut target = self.blocks.clone();
    ranges.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (start, end, edit) in ranges {
      let replacement: Vec<EditorBlock> = edit
        .replacement
        .iter()
        .enumerate()
        .map(|(i, spec)| {
          let id = if i < edit.block_ids.len() {
            edit.block_ids[i].clone()
          } else {
            self.gen_block_id()
          };
          EditorBlock::new(id, &spec.kind, &spec.text).with_data(spec.data.clone())
        })
        .collect();
      target.splice(start..end, replacement);
    }
    if target.is_empty() {
      ctx.error = Some("AI edit conflict: result cannot be empty".to_string());
      return;
    }
    let ops = diff_block_ops(&self.root_block_id, &self.blocks, &target);
    if ops.is_empty() {
      return;
    }
    self.blocks = target;
    ctx.ops = ops;
    ctx.typing_block = None;
    self.selection = self
      .selection
      .as_ref()
      .and_then(|s| s.clamp_and_snap(&self.blocks))
      .or_else(|| default_initial_selection(&self.blocks));
  }

  /// Helper used by `paste` and multi-line IME paste.
  pub(crate) fn replace_focused_with_blocks(&mut self, specs: &[BlockSpec], ctx: &mut DispatchCtx) {
    if specs.is_empty() {
      return;
    }
    if self.blocks.is_empty() {
      self.ensure_not_empty(ctx);
    }
    let idx = self
      .selection
      .as_ref()
      .map_or(0, DocSelection::focus_node)
      .min(self.blocks.len() - 1);

    let first = &specs[0];
    let b = &mut self.blocks[idx];
    b.kind = first.kind.clone();
    b.text = first.text.clone();
    b.data = normalize_data_object(first.data.clone());
    ctx.ops.push(Self::update_full_op(b));

    let mut insert_at = idx + 1;
    for spec in &specs[1..] {
      let created =
        EditorBlock::new(self.gen_block_id(), &spec.kind, &spec.text).with_data(spec.data.clone());
      ctx.ops.push(self.insert_op(&created, insert_at));
      self.blocks.insert(insert_at, created);
      insert_at += 1;
    }
    ctx.typing_block = None;
    let last = insert_at.saturating_sub(1).min(self.blocks.len() - 1);
    self.collapse_to(DocPosition::new(last, self.blocks[last].utf16_len()));
  }
}

// ── Module-private helpers ───────────────────────────────────────────────────

/// Derive incremental [`BlockPatch`]es directly from the emitted [`DocOp`]s in
/// `O(|ops|)` time for pure in-place updates (avoiding whole-document clones
/// and hash-map diffs on every keystroke).
fn patches_from_ops(
  blocks: &[EditorBlock],
  ops: &[DocOp],
  selection: Option<&DocSelection>,
) -> Vec<BlockPatch> {
  let mut order_changed = false;
  for op in ops {
    if matches!(
      op,
      DocOp::InsertBlock { .. } | DocOp::DeleteBlock { .. } | DocOp::MoveBlock { .. }
    ) {
      order_changed = true;
      break;
    }
  }

  // Fast path: in-place block updates only (typing, marks, links, todo toggle, etc.).
  if !order_changed {
    if ops.len() == 1
      && let DocOp::UpdateBlock { block_id, .. } = &ops[0]
    {
      if let Some(sel) = selection {
        let hint = sel.focus_node();
        if let Some(b) = blocks.get(hint)
          && b.id == *block_id
        {
          return vec![BlockPatch::Upsert {
            index: hint,
            block: b.clone(),
          }];
        }
      }
      if let Some(idx) = blocks.iter().position(|b| b.id == *block_id) {
        return vec![BlockPatch::Upsert {
          index: idx,
          block: blocks[idx].clone(),
        }];
      }
      return Vec::new();
    }
    let mut updated_ids: HashSet<&str> = HashSet::with_capacity(ops.len());
    for op in ops {
      if let DocOp::UpdateBlock { block_id, .. } = op {
        updated_ids.insert(block_id.as_str());
      }
    }
    let mut patches = Vec::with_capacity(updated_ids.len());
    for (i, b) in blocks.iter().enumerate() {
      if updated_ids.remove(b.id.as_str()) {
        patches.push(BlockPatch::Upsert {
          index: i,
          block: b.clone(),
        });
        if updated_ids.is_empty() {
          break;
        }
      }
    }
    return patches;
  }

  // Structural path: inserts, deletes, or moves.
  let mut deleted_ids: Vec<&str> = Vec::new();
  let mut deleted_seen: HashSet<&str> = HashSet::new();
  let mut upserted_ids: HashSet<&str> = HashSet::new();
  for op in ops {
    match op {
      DocOp::DeleteBlock { block_id } => {
        if deleted_seen.insert(block_id.as_str()) {
          deleted_ids.push(block_id.as_str());
        }
      }
      DocOp::InsertBlock { block, .. } => {
        upserted_ids.insert(block.id.as_str());
      }
      DocOp::UpdateBlock { block_id, .. } => {
        upserted_ids.insert(block_id.as_str());
      }
      DocOp::MoveBlock { .. } => {}
    }
  }

  let mut upserts = Vec::with_capacity(upserted_ids.len());
  let mut order = Vec::with_capacity(blocks.len());
  let mut live_deleted_readded: HashSet<&str> = HashSet::new();
  for (i, b) in blocks.iter().enumerate() {
    let id = b.id.as_str();
    order.push(b.id.clone());
    if deleted_seen.contains(id) {
      live_deleted_readded.insert(id);
    }
    if upserted_ids.contains(id) {
      upserts.push(BlockPatch::Upsert {
        index: i,
        block: b.clone(),
      });
    }
  }

  let mut patches = Vec::with_capacity(deleted_ids.len() + upserts.len() + 1);
  for id in deleted_ids {
    if !live_deleted_readded.contains(id) {
      patches.push(BlockPatch::Remove {
        block_id: id.to_string(),
      });
    }
  }
  patches.extend(upserts);
  patches.push(BlockPatch::Reorder { order });
  patches
}

pub(crate) fn read_body_blocks(doc: &MicaDoc, root_id: &str) -> Vec<EditorBlock> {
  let all = doc.to_blocks();
  if all.is_empty() {
    return Vec::new();
  }
  let by_id: HashMap<&str, &mica_core::Block> = all.iter().map(|b| (b.id.as_str(), b)).collect();
  let Some(root) = by_id.get(root_id).copied().or_else(|| all.first()) else {
    return Vec::new();
  };
  let mut out = Vec::with_capacity(root.children.len());
  let mut seen = HashSet::new();
  seen.insert(root.id.as_str());
  for cid in &root.children {
    if seen.insert(cid.as_str())
      && let Some(&b) = by_id.get(cid.as_str())
    {
      out.push(EditorBlock::from(b));
    }
  }
  out
}

pub(crate) fn default_initial_selection(blocks: &[EditorBlock]) -> Option<DocSelection> {
  let first = blocks.first()?;
  if first.is_atomic() {
    Some(DocSelection::block_atom(0))
  } else {
    Some(DocSelection::caret(0, 0))
  }
}

fn continuation_data(node: &EditorBlock) -> Value {
  if node.kind == "todo" {
    let mut m = Map::new();
    m.insert("checked".to_string(), Value::Bool(false));
    if node.indent() > 0 {
      m.insert("indent".to_string(), json!(node.indent()));
    }
    return Value::Object(m);
  }
  let mut m = node.data_map().clone();
  // Stay in the SAME quote group (never inherit `qbreak`), and do not carry parent marks.
  m.remove("qbreak");
  m.remove("marks");
  m.remove("start");
  Value::Object(m)
}

fn apply_ops_to_doc(doc: &mut MicaDoc, root_id: &str, ops: &[DocOp]) {
  for op in ops {
    match op {
      DocOp::InsertBlock {
        parent_id,
        index,
        block,
      } => {
        let p = if parent_id.is_empty() {
          root_id
        } else {
          parent_id.as_str()
        };
        doc.insert_block(p, *index, block);
      }
      DocOp::UpdateBlock {
        block_id,
        kind,
        text,
        data,
      } => {
        doc.update_block_fields(block_id, kind.as_deref(), text.as_deref(), data.as_ref());
      }
      DocOp::DeleteBlock { block_id } => {
        doc.delete_block(block_id, false);
      }
      DocOp::MoveBlock {
        block_id,
        parent_id,
        index,
      } => {
        let p = if parent_id.is_empty() {
          root_id
        } else {
          parent_id.as_str()
        };
        doc.move_block(block_id, p, *index);
      }
    }
  }
}

fn map_preedit_offset(offset: u32, from: &str, to: &str, right_bias: bool) -> u32 {
  if from == to {
    return offset;
  }
  let from_len = utf16_len(from);
  let to_len = utf16_len(to);
  let prefix = common_prefix_utf16(from, to);
  let suffix = common_suffix_utf16(from, to, prefix);
  let old_end = from_len - suffix;
  let delta = to_len as i64 - from_len as i64;
  if right_bias {
    if offset < prefix {
      return offset;
    }
    if offset >= old_end {
      return (offset as i64 + delta).max(0) as u32;
    }
  } else {
    if offset <= prefix {
      return offset;
    }
    if offset > old_end {
      return (offset as i64 + delta).max(0) as u32;
    }
  }
  prefix
}

/// Non-overlapping find across all non-atomic blocks, returning UTF-16 ranges.
pub fn find_all_matches(
  blocks: &[EditorBlock],
  query: &str,
  case_sensitive: bool,
) -> Vec<FindMatch> {
  if query.is_empty() {
    return Vec::new();
  }
  let mut out = Vec::new();
  let needle_chars: Vec<char> = if case_sensitive {
    query.chars().collect()
  } else {
    query.chars().flat_map(char::to_lowercase).collect()
  };
  let nlen = needle_chars.len();
  if nlen == 0 {
    return out;
  }

  for (idx, block) in blocks.iter().enumerate() {
    if block.is_atomic() || block.text.is_empty() {
      continue;
    }
    // Map each folded char back to the original text's UTF-16 [start, end) span.
    let mut hay: Vec<(char, u32, u32)> = Vec::new();
    let mut u16_pos = 0u32;
    for ch in block.text.chars() {
      let next = u16_pos + ch.len_utf16() as u32;
      if case_sensitive {
        hay.push((ch, u16_pos, next));
      } else {
        for lc in ch.to_lowercase() {
          hay.push((lc, u16_pos, next));
        }
      }
      u16_pos = next;
    }
    let mut i = 0usize;
    while i + nlen <= hay.len() {
      if hay[i..i + nlen]
        .iter()
        .zip(&needle_chars)
        .all(|(&(c, _, _), &nc)| c == nc)
      {
        let start = hay[i].1;
        let end = hay[i + nlen - 1].2;
        out.push(FindMatch {
          block_index: idx,
          block_id: block.id.clone(),
          start,
          end,
        });
        i += nlen;
      } else {
        i += 1;
      }
    }
  }
  out
}
