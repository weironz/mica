//! Reactive L3 bridge wrapping `crates/mica-editor::EditorSession` (L2 headless
//! pure-Rust block editor kernel backed directly by `mica_core::MicaDoc`).
//!
//! Architecture invariants (`AGENTS.md`):
//! - **Virtualization signal isolation**: `structure_rev` bumps ONLY when the
//!   top-level block count, IDs, or ordering change. `virtual_list` `count` and
//!   `keys` subscribe ONLY to `structure_rev`, NEVER to body typing signals.
//! - **Per-block fine-grained reactivity**: typing inside block `id` bumps ONLY
//!   `block_rev(id)` (`Signal<u64>`), leaving all other mounted blocks untouched.
//! - **Single CRDT + history stack**: every edit mutates the owned `MicaDoc`
//!   inside `EditorSession`, emitting incremental yrs updates and canonical
//!   Markdown through `crates/md-core`.

use std::{
  cell::RefCell,
  collections::{BTreeMap, HashMap},
  rc::Rc,
};

use mica_core::MicaDoc;
use mica_editor::{
  ClipboardPayload, DocPosition, DocSelection, EditorBlock, EditorCommand, EditorPatch,
  EditorSession, FindMatch, FindReplaceOp, PasteInput, TableOp,
  marks::EditorMark,
  model::BlockPatch,
  utf16::{utf16_len, utf16_slice},
};
use mica_md::{HtmlNode, HtmlProfile, html_to_markdown};
use nsh_core::prelude::{Owner, Signal, signal};
use serde_json::{Map, Value, json};

const DEFAULT_STATUS_TEXT: &str = "上次编辑 刚刚";

/// How `virtual_list` should react to a scroll request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollMode {
  /// Explicit navigation (page switch, outline jump, find match): align target into view.
  Jump,
  /// Editing/caret movement (Enter, Backspace, typing, arrows): keep target visible
  /// ONLY if it actually scrolled outside the viewport; NEVER jump when already visible!
  KeepVisible,
}

/// One heading entry in the live page outline (consumed by `right_sidebar.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineItem {
  pub block_index: usize,
  pub pos: usize,
  pub level: u64,
  pub text: String,
}

type PersistCallback = Rc<dyn Fn(String, Option<Vec<u8>>)>;

/// Reactive bridge connecting `mica_editor::EditorSession` to `virtual_list`
/// and all outer Mica chrome (header, outline, find bar, slash menu, version history).
pub struct MicaDocumentBridge {
  /// Root reactive owner that outlives all `virtual_list` recycled cells & `dynamic` runs.
  owner: Owner,
  session: RefCell<EditorSession>,
  /// Cached ordered block IDs for detecting structural diffs without cloning blocks.
  cached_ids: RefCell<Vec<String>>,
  /// O(1) `block_id -> index` map rebuilt only when `structure_rev` changes.
  id_to_index: RefCell<HashMap<String, usize>>,
  /// Per-block fine-grained revision signals (`block_id -> Signal<u64>`).
  block_revs: RefCell<HashMap<String, Signal<u64>>>,
  on_persist: RefCell<Option<PersistCallback>>,
  /// Lightweight dirty notification for debounced persistence (avoids synchronous
  /// full-document Markdown export & SQLite disk writes on every keystroke).
  on_dirty: RefCell<Option<Rc<dyn Fn()>>>,
  pending_persist: std::cell::Cell<bool>,

  // ── Reactive UI Signals ──────────────────────────────────────────────────
  /// Bumps ONLY when block count, IDs, or order change (`virtual_list` count/keys).
  structure_rev: Signal<u64>,
  /// Current selection / caret (`DocSelection`), paint-only.
  selection_sig: Signal<Option<DocSelection>>,
  /// Monotonic counter bumped when the editor body IME sink should focus & sync.
  focus_sink_rev: Signal<u64>,
  /// Request `(block_id, seq)` to immediately mount & focus a live code block editor.
  code_focus_request: Signal<Option<(String, u64)>>,
  /// Scroll-to-block request `(block_index, mode, seq)` consumed by `virtual_list`.
  scroll_request: Signal<Option<(usize, ScrollMode, u64)>>,
  /// Active CJK IME composition preedit `(block_index, start_u16, preedit_text)`.
  ime_preedit: Signal<Option<(usize, u32, String)>>,
  /// Active in-page find matches for inline highlighting.
  find_matches_sig: Signal<Vec<FindMatch>>,
  find_active_idx: Signal<usize>,

  // ── Header / Inspector Signals (API-compatible with legacy bridge) ───────
  word_count: Signal<String>,
  block_count: Signal<usize>,
  outline: Signal<Vec<OutlineItem>>,
  can_undo: Signal<bool>,
  can_redo: Signal<bool>,
  active_kind: Signal<String>,
  active_level: Signal<u64>,
  active_marks: Signal<Vec<String>>,
  last_edited: Signal<String>,
  front_matter: Signal<String>,
  session_rev: Signal<u64>,
}

impl MicaDocumentBridge {
  /// Constructs a live [`MicaDocumentBridge`] initialized from `markdown`.
  pub fn from_markdown(markdown: &str) -> Rc<Self> {
    Self::from_session(session_from_markdown(markdown))
  }

  /// Constructs a live [`MicaDocumentBridge`] from an encoded yrs CRDT state update.
  pub fn from_update(bytes: &[u8]) -> Result<Rc<Self>, String> {
    let doc = MicaDoc::from_update(bytes).map_err(|e| e.to_string())?;
    Ok(Self::from_session(EditorSession::from_doc(doc)))
  }

  fn from_session(session: EditorSession) -> Rc<Self> {
    let ids: Vec<String> = session.blocks().iter().map(|b| b.id.clone()).collect();
    let id_to_index: HashMap<String, usize> = ids
      .iter()
      .enumerate()
      .map(|(i, id)| (id.clone(), i))
      .collect();
    let word_count = signal(compute_word_count_label(session.blocks()));
    let block_count = signal(session.blocks().len());
    let outline = signal(compute_outline(session.blocks()));
    let can_undo = signal(session.can_undo());
    let can_redo = signal(session.can_redo());
    let (ak, al, am) = compute_active_state(&session);
    let fm = read_root_front_matter(session.doc(), session.root_block_id());
    let sel = session.selection().cloned();

    Rc::new(Self {
      owner: Owner::current(),
      session: RefCell::new(session),
      cached_ids: RefCell::new(ids),
      id_to_index: RefCell::new(id_to_index),
      block_revs: RefCell::new(HashMap::new()),
      on_persist: RefCell::new(None),
      on_dirty: RefCell::new(None),
      pending_persist: std::cell::Cell::new(false),
      structure_rev: signal(0u64),
      selection_sig: signal(sel),
      focus_sink_rev: signal(0u64),
      code_focus_request: signal(None),
      scroll_request: signal(None),
      ime_preedit: signal(None),
      find_matches_sig: signal(Vec::new()),
      find_active_idx: signal(0usize),
      word_count,
      block_count,
      outline,
      can_undo,
      can_redo,
      active_kind: signal(ak),
      active_level: signal(al),
      active_marks: signal(am),
      last_edited: signal(String::from(DEFAULT_STATUS_TEXT)),
      front_matter: signal(fm),
      session_rev: signal(0u64),
    })
  }

  pub fn set_on_persist(&self, cb: Rc<dyn Fn(String, Option<Vec<u8>>)>) {
    *self.on_persist.borrow_mut() = Some(cb);
  }

  /// Registers a lightweight dirty callback invoked synchronously on content
  /// edits instead of exporting Markdown synchronously on every keystroke.
  /// Callers should debounce and invoke [`Self::take_pending_markdown`] or
  /// [`Self::flush_pending_persist`].
  pub fn set_on_dirty(&self, cb: Rc<dyn Fn()>) {
    *self.on_dirty.borrow_mut() = Some(cb);
  }

  /// If edits are waiting to be persisted, clears the dirty flag and exports
  /// the current Markdown snapshot once.
  pub fn take_pending_markdown(&self) -> Option<String> {
    if self.pending_persist.replace(false) {
      Some(self.export_markdown())
    } else {
      None
    }
  }

  /// Synchronously flushes any pending debounced edits through `on_persist`.
  pub fn flush_pending_persist(&self) {
    if let Some(md) = self.take_pending_markdown() {
      let cb = self.on_persist.borrow().clone();
      if let Some(cb) = cb {
        cb(md, None);
      }
    }
  }

  // ── Core Virtual-List Read Accessors ─────────────────────────────────────

  /// Signal bumped ONLY on structural block changes (`virtual_list` `count` & `keys`).
  pub fn structure_rev_signal(&self) -> Signal<u64> {
    self.structure_rev
  }

  pub fn code_focus_request_signal(&self) -> Signal<Option<(String, u64)>> {
    self.code_focus_request
  }

  pub fn request_focus_code_block(&self, block_id: String) {
    let seq = self
      .code_focus_request
      .get_untracked()
      .map(|(_, s)| s.wrapping_add(1))
      .unwrap_or(1);
    self.code_focus_request.set(Some((block_id, seq)));
  }

  /// Fine-grained per-block revision signal (`Signal<u64>`) bumped ONLY when
  /// block `block_id` itself changes. Created under the bridge's root `Owner`
  /// so recycled `virtual_list` cells / `dynamic` reruns NEVER dispose it.
  pub fn block_rev_signal(&self, block_id: &str) -> Signal<u64> {
    if let Some(&sig) = self.block_revs.borrow().get(block_id) {
      return sig;
    }
    let sig = self.owner.run(|| signal(0u64));
    let mut map = self.block_revs.borrow_mut();
    *map.entry(block_id.to_owned()).or_insert(sig)
  }

  /// Current number of top-level blocks (O(1)).
  pub fn block_len(&self) -> usize {
    self.session.borrow().blocks().len()
  }

  /// Ordered block IDs for `virtual_list::keys`.
  pub fn block_ids(&self) -> Vec<String> {
    self.cached_ids.borrow().clone()
  }

  /// O(1) lookup of the block ID currently at `index`.
  pub fn block_id_at(&self, index: usize) -> Option<String> {
    self.cached_ids.borrow().get(index).cloned()
  }

  /// Read a snapshot clone of block at `index`.
  pub fn block_at(&self, index: usize) -> Option<EditorBlock> {
    self.session.borrow().blocks().get(index).cloned()
  }

  /// O(1) lookup of a snapshot clone of block by `block_id`.
  pub fn block_by_id(&self, block_id: &str) -> Option<EditorBlock> {
    let idx = self.id_to_index.borrow().get(block_id).copied()?;
    self.session.borrow().blocks().get(idx).cloned()
  }

  /// O(1) lookup of current index of `block_id` (or `None` if removed).
  pub fn index_of_block_id(&self, block_id: &str) -> Option<usize> {
    self.id_to_index.borrow().get(block_id).copied()
  }

  /// Compute the visible 1-based ordinal for a `numbered_list` block at `index`.
  pub fn visible_ordinal_at(&self, index: usize) -> usize {
    let s = self.session.borrow();
    mica_editor::visible_ordinal(s.blocks(), index).unwrap_or(1) as usize
  }

  /// Borrow read-only access to the full block slice for lightweight queries.
  pub fn with_blocks<R>(&self, f: impl FnOnce(&[EditorBlock]) -> R) -> R {
    let s = self.session.borrow();
    f(s.blocks())
  }

  pub fn selection_signal(&self) -> Signal<Option<DocSelection>> {
    self.selection_sig
  }

  pub fn selection(&self) -> Option<DocSelection> {
    self.session.borrow().selection().cloned()
  }

  pub fn focus_sink_rev_signal(&self) -> Signal<u64> {
    self.focus_sink_rev
  }

  pub fn request_focus(&self) {
    self.focus_sink_rev.set(self.focus_sink_rev.get_untracked().wrapping_add(1));
  }

  pub fn scroll_request_signal(&self) -> Signal<Option<(usize, ScrollMode, u64)>> {
    self.scroll_request
  }

  pub fn request_scroll_to_block(&self, block_index: usize) {
    let seq = self
      .scroll_request
      .get_untracked()
      .map(|(_, _, n)| n.wrapping_add(1))
      .unwrap_or(1);
    self.scroll_request.set(Some((block_index, ScrollMode::Jump, seq)));
  }

  pub fn request_keep_block_visible(&self, block_index: usize) {
    let seq = self
      .scroll_request
      .get_untracked()
      .map(|(_, _, n)| n.wrapping_add(1))
      .unwrap_or(1);
    self.scroll_request.set(Some((block_index, ScrollMode::KeepVisible, seq)));
  }

  pub fn ime_preedit_signal(&self) -> Signal<Option<(usize, u32, String)>> {
    self.ime_preedit
  }

  pub fn set_ime_preedit(&self, preedit: Option<(usize, u32, String)>) {
    self.ime_preedit.set(preedit);
  }

  pub fn find_matches_signal(&self) -> Signal<Vec<FindMatch>> {
    self.find_matches_sig
  }

  pub fn find_active_idx_signal(&self) -> Signal<usize> {
    self.find_active_idx
  }

  pub fn set_find_highlights(&self, matches: Vec<FindMatch>, active_idx: usize) {
    self.find_matches_sig.set(matches);
    self.find_active_idx.set(active_idx);
  }

  pub fn clear_find_highlights(&self) {
    self.find_matches_sig.set(Vec::new());
    self.find_active_idx.set(0);
  }

  // ── Document Load & Remote CRDT Sync ─────────────────────────────────────

  /// Replaces the entire document with `markdown` (e.g. when switching pages).
  /// Clears stale per-block signals and bumps `structure_rev` once.
  pub fn load_markdown(&self, markdown: &str) {
    let next_session = session_from_markdown(markdown);
    let ids: Vec<String> = next_session.blocks().iter().map(|b| b.id.clone()).collect();
    let fm = read_root_front_matter(next_session.doc(), next_session.root_block_id());
    let sel = next_session.selection().cloned();
    let wc = compute_word_count_label(next_session.blocks());
    let bc = next_session.blocks().len();
    let ol = compute_outline(next_session.blocks());
    let (ak, al, am) = compute_active_state(&next_session);

    *self.session.borrow_mut() = next_session;
    {
      let mut idx_map = self.id_to_index.borrow_mut();
      idx_map.clear();
      idx_map.reserve(ids.len());
      for (i, id) in ids.iter().enumerate() {
        idx_map.insert(id.clone(), i);
      }
    }
    *self.cached_ids.borrow_mut() = ids;
    self.block_revs.borrow_mut().clear();
    self.pending_persist.set(false);

    self.ime_preedit.set(None);
    self.find_matches_sig.set(Vec::new());
    self.find_active_idx.set(0);
    self.selection_sig.set(sel);
    self.front_matter.set(fm);
    self.word_count.set(wc);
    self.block_count.set(bc);
    self.outline.set(ol);
    self.can_undo.set(false);
    self.can_redo.set(false);
    set_if_changed(self.active_kind, ak);
    set_if_changed(self.active_level, al);
    set_if_changed(self.active_marks, am);
    self.last_edited.set(String::from(DEFAULT_STATUS_TEXT));
    self.structure_rev.set(self.structure_rev.get_untracked().wrapping_add(1));
    self.session_rev.set(self.session_rev.get_untracked().wrapping_add(1));
    self.request_scroll_to_block(0);
  }

  /// Applies a remote peer's yrs CRDT update without echoing it back to `on_persist`.
  pub fn apply_remote_update(&self, update: &[u8]) -> Result<(), String> {
    let patch = self.dispatch_inner(
      EditorCommand::ApplyRemoteUpdate { update: update.to_vec() },
      false,
      None,
    );
    if let Some(err) = patch.error {
      Err(err)
    } else {
      Ok(())
    }
  }

  // ── Unified Command Dispatch ─────────────────────────────────────────────

  /// Executes an [`EditorCommand`] on the headless `EditorSession`, propagates
  /// minimal reactive updates (`structure_rev` ONLY on structural changes,
  /// `block_revs[id]` ONLY for dirty blocks), and triggers persistence.
  pub fn dispatch(&self, cmd: EditorCommand) -> EditorPatch {
    let scroll_mode = if matches!(
      cmd,
      EditorCommand::SplitBlock
        | EditorCommand::InsertText { .. }
        | EditorCommand::ImeUpdate { .. }
        | EditorCommand::DeleteBackward
        | EditorCommand::DeleteForward
        | EditorCommand::Paste { .. }
        | EditorCommand::Undo
        | EditorCommand::Redo
        | EditorCommand::InsertDivider
        | EditorCommand::InsertImage { .. }
        | EditorCommand::ApplySlashCommand { .. }
        | EditorCommand::AppendEmptyParagraph
    ) {
      Some(ScrollMode::KeepVisible)
    } else {
      None
    };
    self.dispatch_inner_opts(cmd, true, scroll_mode, false)
  }

  fn dispatch_inner(
    &self,
    cmd: EditorCommand,
    emit_persist: bool,
    scroll_mode: Option<ScrollMode>,
  ) -> EditorPatch {
    self.dispatch_inner_opts(cmd, emit_persist, scroll_mode, false)
  }

  fn dispatch_inner_opts(
    &self,
    cmd: EditorCommand,
    emit_persist: bool,
    scroll_mode: Option<ScrollMode>,
    suppress_block_revs: bool,
  ) -> EditorPatch {
    let may_touch_front_matter = matches!(
      cmd,
      EditorCommand::Undo | EditorCommand::Redo | EditorCommand::ApplyRemoteUpdate { .. }
    );
    let patch = {
      let mut s = self.session.borrow_mut();
      s.dispatch(cmd)
    };

    // 1. Check whether top-level block structure (IDs / order / count) changed
    // without cloning all block IDs on plain in-block edits.
    let (structure_changed, wc, bc, ol, can_u, can_r, ak, al, am, sel, fm) = {
      let s = self.session.borrow();
      let blocks = s.blocks();
      let structure_changed = {
        let mut cached = self.cached_ids.borrow_mut();
        let same = blocks.len() == cached.len()
          && blocks.iter().zip(cached.iter()).all(|(b, c)| b.id == *c);
        if same {
          false
        } else {
          cached.clear();
          cached.extend(blocks.iter().map(|b| b.id.clone()));
          let mut idx_map = self.id_to_index.borrow_mut();
          idx_map.clear();
          idx_map.reserve(cached.len());
          for (i, id) in cached.iter().enumerate() {
            idx_map.insert(id.clone(), i);
          }
          true
        }
      };
      let wc = if patch.content_changed {
        Some(compute_word_count_label(blocks))
      } else {
        None
      };
      let bc = blocks.len();
      let ol = if patch.content_changed {
        Some(compute_outline(blocks))
      } else {
        None
      };
      let (ak, al, am) = compute_active_state(&s);
      let fm = if patch.content_changed && may_touch_front_matter {
        Some(read_root_front_matter(s.doc(), s.root_block_id()))
      } else {
        None
      };
      (
        structure_changed,
        wc,
        bc,
        ol,
        s.can_undo(),
        s.can_redo(),
        ak,
        al,
        am,
        s.selection().cloned(),
        fm,
      )
    };

    if structure_changed {
      // Prune signals for deleted block IDs so the map stays O(live_blocks).
      {
        let idx_map = self.id_to_index.borrow();
        self.block_revs.borrow_mut().retain(|id, _| idx_map.contains_key(id));
      }
      self.structure_rev.set(self.structure_rev.get_untracked().wrapping_add(1));
    }

    // 2. Bump ONLY the per-block signals for dirty blocks (never re-renders siblings!).
    // Collect first and drop the `RefCell` borrow BEFORE calling `.set(...)` so
    // synchronous reactive observers can safely call `block_rev_signal(...)`.
    if !suppress_block_revs && !patch.dirty_blocks.is_empty() {
      let to_bump: Vec<Signal<u64>> = {
        let revs = self.block_revs.borrow();
        patch
          .dirty_blocks
          .iter()
          .filter_map(|bp| {
            let id = match bp {
              BlockPatch::Upsert { block, .. } => Some(block.id.as_str()),
              BlockPatch::Remove { block_id } => Some(block_id.as_str()),
              BlockPatch::Reorder { .. } => None,
            };
            id.and_then(|k| revs.get(k).copied())
          })
          .collect()
      };
      for sig in to_bump {
        sig.set(sig.get_untracked().wrapping_add(1));
      }
    }

    // 3. Update selection & lightweight header signals.
    self.selection_sig.set(sel.clone());
    set_if_changed(self.can_undo, can_u);
    set_if_changed(self.can_redo, can_r);
    set_if_changed(self.active_kind, ak);
    set_if_changed(self.active_level, al);
    set_if_changed(self.active_marks, am);
    set_if_changed(self.block_count, bc);

    if let Some(wc_str) = wc {
      set_if_changed(self.word_count, wc_str);
    }
    if let Some(ol_vec) = ol {
      set_if_changed(self.outline, ol_vec);
    }
    if let Some(fm_str) = fm {
      set_if_changed(self.front_matter, fm_str);
    }

    if let Some(mode) = scroll_mode
      && let Some(ref s) = sel
    {
      match mode {
        ScrollMode::Jump => self.request_scroll_to_block(s.focus_node()),
        ScrollMode::KeepVisible => self.request_keep_block_visible(s.focus_node()),
      }
    }

    if patch.content_changed {
      set_if_changed(self.last_edited, String::from(DEFAULT_STATUS_TEXT));
      self.session_rev.set(self.session_rev.get_untracked().wrapping_add(1));
      if emit_persist {
        let dirty_cb = self.on_dirty.borrow().clone();
        if let Some(dirty_cb) = dirty_cb {
          self.pending_persist.set(true);
          dirty_cb();
        } else {
          let cb = self.on_persist.borrow().clone();
          if let Some(cb) = cb {
            let md = self.export_markdown();
            cb(md, patch.crdt_update.clone());
          }
        }
      }
    }

    patch
  }

  // ── High-Level Editing & Formatting Operations ───────────────────────────

  pub fn undo(&self) {
    self.dispatch(EditorCommand::Undo);
  }

  pub fn redo(&self) {
    self.dispatch(EditorCommand::Redo);
  }

  pub fn toggle_mark(&self, mark_name: &str) {
    self.dispatch(EditorCommand::ToggleMark {
      kind: mark_name.to_owned(),
      href: None,
    });
  }

  pub fn set_block_type(&self, kind: &str, level: Option<u64>) {
    let canonical = match kind {
      "bulletList" | "bullet_list" => "bulleted_list",
      "orderedList" | "number_list" => "numbered_list",
      "taskItem" => "todo",
      "blockquote" => "quote",
      "codeBlock" | "code" => "code_block",
      "horizontalRule" => "divider",
      other => other,
    };
    match canonical {
      "divider" => {
        self.insert_divider();
        return;
      }
      "table" => {
        self.insert_table(3, 3);
        return;
      }
      "math" | "math_block" | "mermaid" | "board" | "image" => {
        self.insert_custom_block(canonical, "", None);
        return;
      }
      _ => {}
    }
    let data = match canonical {
      "heading" => Some(json!({ "level": level.unwrap_or(1).clamp(1, 6) })),
      "todo" => Some(json!({ "checked": false })),
      "code_block" => Some(json!({ "language": "" })),
      _ => None,
    };
    let is_multi = self.selection().is_some_and(|s| s.is_multi_node());
    if is_multi {
      self.dispatch(EditorCommand::BatchTransform {
        kind: canonical.to_owned(),
        data,
      });
    } else {
      self.dispatch(EditorCommand::TransformBlock {
        kind: canonical.to_owned(),
        data,
      });
    }
    if canonical == "code_block"
      && let Some(sel) = self.selection()
      && let Some(blk) = self.block_at(sel.focus_pos().node)
      && effective_block_kind(&blk) == "code_block"
    {
      self.request_focus_code_block(blk.id);
    }
  }

  pub fn insert_divider(&self) {
    self.dispatch(EditorCommand::InsertDivider);
  }

  pub fn insert_table(&self, rows: usize, cols: usize) {
    let r = rows.max(2);
    let c = cols.max(2);
    let mut table_rows = Vec::with_capacity(r);
    for _ in 0..r {
      table_rows.push(Value::Array(vec![Value::String(String::new()); c]));
    }
    let aligns = vec![Value::String(String::new()); c];
    let data = json!({ "rows": table_rows, "aligns": aligns });
    self.insert_block_after_caret("table", "", data);
  }

  pub fn insert_custom_block(&self, kind: &str, text: &str, language: Option<&str>) {
    match kind {
      "divider" => self.insert_divider(),
      "image" => {
        let (url, alt) = match language {
          Some(u) if !u.is_empty() => (Some(u.to_owned()), text.to_owned()),
          _ => (Some(text.to_owned()), String::new()),
        };
        self.dispatch(EditorCommand::InsertImage {
          file_id: None,
          url,
          name: None,
          alt,
          align: Some("left".to_owned()),
        });
      }
      "math" | "math_block" => {
        let src = if text.trim().is_empty() { r"E = mc^2" } else { text };
        self.insert_block_after_caret("math_block", src, Value::Null);
      }
      "mermaid" => {
        let src = if text.trim().is_empty() {
          "graph LR\n  A[Mica] --> B[Pure Rust]"
        } else {
          text
        };
        self.insert_block_after_caret("code_block", src, json!({ "language": "mermaid" }));
      }
      "board" | "mica-board" => {
        let src = if text.trim().is_empty() {
          mica_board::starter_board_json("whiteboard")
        } else {
          text.to_owned()
        };
        self.insert_block_after_caret("code_block", &src, json!({ "language": "mica-board" }));
      }
      "code_block" | "code" => {
        let lang = language.unwrap_or_default();
        self.insert_block_after_caret("code_block", text, json!({ "language": lang }));
        if let Some(sel) = self.selection()
          && let Some(blk) = self.block_at(sel.focus_pos().node)
          && effective_block_kind(&blk) == "code_block"
        {
          self.request_focus_code_block(blk.id);
        }
      }
      other => {
        self.insert_block_after_caret(other, text, Value::Null);
      }
    }
  }

  fn insert_block_after_caret(&self, kind: &str, text: &str, data: Value) {
    let sel = match self.selection() {
      Some(s) => s,
      None => {
        let last = self.block_len().saturating_sub(1);
        let end_off = self.block_at(last).map(|b| utf16_len(&b.text)).unwrap_or(0);
        let fallback = DocSelection::caret(last, end_off);
        self.dispatch_inner(
          EditorCommand::SetSelection {
            selection: Some(fallback.clone()),
          },
          false,
          None,
        );
        fallback
      }
    };
    let cur_idx = sel.focus_node();
    let cur_empty = self
      .block_at(cur_idx)
      .is_some_and(|b| b.kind == "paragraph" && b.text.is_empty());
    if cur_empty {
      self.dispatch(EditorCommand::ApplySlashCommand {
        start: 0,
        end: 0,
        kind: kind.to_owned(),
        data,
      });
      if !text.is_empty() {
        self.update_block_source_at(cur_idx, text);
      }
    } else {
      let end_off = self
        .block_at(cur_idx)
        .map(|b| utf16_len(&b.text))
        .unwrap_or(0);
      self.dispatch(EditorCommand::SetSelection {
        selection: Some(DocSelection::caret(cur_idx, end_off)),
      });
      self.dispatch(EditorCommand::SplitBlock);
      let new_idx = self
        .selection()
        .map(|s| s.focus_node())
        .unwrap_or(cur_idx + 1);
      self.dispatch(EditorCommand::ApplySlashCommand {
        start: 0,
        end: 0,
        kind: kind.to_owned(),
        data,
      });
      if !text.is_empty() {
        self.update_block_source_at(new_idx, text);
      }
    }
  }

  pub fn set_quote_alert(&self, alert: &str) {
    let data = if alert.trim().is_empty() {
      json!({})
    } else {
      json!({ "alert": alert.trim() })
    };
    self.dispatch(EditorCommand::TransformBlock {
      kind: "quote".to_owned(),
      data: Some(data),
    });
  }

  /// Replaces the `/query` token ending at the caret (preserving preceding text)
  /// and applies the chosen slash command block/callout/board template.
  pub fn replace_slash_token_and_apply(
    &self,
    kind: &str,
    level: Option<u64>,
    alert: Option<&str>,
    template: Option<&str>,
  ) {
    let sel = self.selection().unwrap_or_else(|| DocSelection::caret(0, 0));
    let focus = sel.focus_pos();
    let bi = focus.node;
    let caret = focus.offset;
    let (slash_start, slash_end) = self
      .block_at(bi)
      .and_then(|b| {
        let prefix = utf16_slice(&b.text, 0, caret);
        let slash_byte = prefix.rfind('/')?;
        let before = &prefix[..slash_byte];
        if !before.is_empty() && !before.ends_with(|c: char| c.is_whitespace()) {
          return None;
        }
        let s_u16 = utf16_len(before);
        Some((s_u16, caret))
      })
      .unwrap_or((caret, caret));

    let delete_slash = || {
      if slash_end > slash_start {
        self.dispatch(EditorCommand::SetSelection {
          selection: Some(DocSelection::from_positions(
            DocPosition::new(bi, slash_start),
            DocPosition::new(bi, slash_end),
          )),
        });
        self.dispatch(EditorCommand::DeleteBackward);
      }
    };

    match kind {
      "heading" => {
        delete_slash();
        self.set_block_type("heading", level);
      }
      "bulleted_list" | "numbered_list" | "todo" => {
        delete_slash();
        self.set_block_type(kind, None);
      }
      "quote" => {
        delete_slash();
        self.set_quote_alert(alert.unwrap_or(""));
      }
      "code_block" => {
        delete_slash();
        self.insert_custom_block("code_block", "", Some("rust"));
        // `insert_custom_block("code_block", ...)` already requested focus on
        // the live code block editor — do NOT steal focus back to the prose sink!
        return;
      }
      "math" | "math_block" => {
        delete_slash();
        self.insert_custom_block("math", r"E = mc^2", None);
      }
      "mermaid" => {
        delete_slash();
        self.insert_custom_block("mermaid", "graph LR\n  A[Mica] --> B[Pure Rust]", None);
      }
      "board" => {
        delete_slash();
        let starter = mica_board::starter_board_json(template.unwrap_or("whiteboard"));
        self.insert_custom_block("board", &starter, None);
      }
      "table" => {
        delete_slash();
        self.insert_table(2, 2);
      }
      "divider" => {
        delete_slash();
        self.insert_divider();
      }
      "image" => {
        delete_slash();
        self.insert_custom_block("image", "", Some(""));
      }
      _ => {
        delete_slash();
        self.set_block_type("paragraph", None);
      }
    }
    self.request_focus();
  }

  /// Updates the raw text/source of block at `block_index` (e.g. embedded board
  /// 3-way merged JSON or external replace) and refreshes that block's view.
  pub fn update_block_source_at(&self, block_index: usize, new_source: &str) {
    self.dispatch_inner_opts(
      EditorCommand::SetBlockText {
        block_index,
        text: new_source.to_owned(),
      },
      true,
      None,
      false,
    );
  }

  /// Like [`Self::update_block_source_at`], but skips bumping `block_rev` for the
  /// actively focused inline editor (`nsh_base::editor()` inside code/mermaid/math)
  /// so keystrokes never destroy & remount the live editor!
  pub fn update_block_source_live(&self, block_index: usize, new_source: &str) {
    self.dispatch_inner_opts(
      EditorCommand::SetBlockText {
        block_index,
        text: new_source.to_owned(),
      },
      true,
      None,
      true,
    );
  }

  /// Updates a single JSON `data` key on block at `block_index` (e.g. `language`,
  /// `wrap`, `lineNumbers`, `title`, `width`, `height`, `align`, `alert`).
  pub fn update_block_data_attr_at(&self, block_index: usize, key: &str, value: Value) {
    let block_info = self.block_at(block_index);
    let Some(block) = block_info else {
      return;
    };
    if key == "language" && block.kind == "code_block" {
      let lang = value.as_str().map(str::to_owned);
      self.dispatch(EditorCommand::SetCodeLanguage {
        block_index,
        language: lang,
      });
      return;
    }
    let mut map = match block.data {
      Value::Object(m) => m,
      _ => Map::new(),
    };
    if value.is_null() {
      map.remove(key);
    } else {
      map.insert(key.to_owned(), value);
    }
    let next_data = if map.is_empty() { Value::Null } else { Value::Object(map) };
    let prev_sel = self.selection();
    let _ = self.dispatch_inner(
      EditorCommand::SetSelection {
        selection: Some(DocSelection::caret(block_index, 0)),
      },
      false,
      None,
    );
    self.dispatch_inner(
      EditorCommand::TransformBlock {
        kind: block.kind,
        data: Some(next_data),
      },
      true,
      None,
    );
    if let Some(sel) = prev_sel {
      let _ = self.dispatch_inner(
        EditorCommand::SetSelection { selection: Some(sel) },
        false,
        None,
      );
    }
  }

  pub fn delete_block_at(&self, block_index: usize) {
    self.delete_block_range(block_index, block_index);
  }

  /// Deletes the block at `block_index` (e.g. pressing Backspace at offset 0 of
  /// a code block) and places the caret cleanly on the neighboring block.
  pub fn delete_block_and_focus_neighbor(&self, block_index: usize) {
    let count = self.block_len();
    if count == 0 || block_index >= count {
      return;
    }
    self.delete_block_at(block_index);
    let new_count = self.block_len();
    if new_count == 0 {
      return;
    }
    let target_idx = if block_index > 0 {
      (block_index - 1).min(new_count - 1)
    } else {
      0
    };
    let target_off = if block_index > 0 {
      self
        .block_at(target_idx)
        .map(|b| utf16_len(&b.text))
        .unwrap_or(0)
    } else {
      0
    };
    self.dispatch_inner(
      EditorCommand::SetSelection {
        selection: Some(DocSelection::caret(target_idx, target_off)),
      },
      false,
      Some(ScrollMode::KeepVisible),
    );
    self.request_focus();
  }

  pub fn toggle_todo_at(&self, block_index: usize) {
    self.dispatch(EditorCommand::ToggleTodo { block_index });
  }

  pub fn table_op(&self, block_index: usize, op: TableOp) -> EditorPatch {
    self.dispatch(EditorCommand::TableOp { block_index, op })
  }

  /// Like [`Self::table_op`], but suppresses self-remounting while typing inside a focused cell.
  pub fn table_op_live(&self, block_index: usize, op: TableOp) -> EditorPatch {
    self.dispatch_inner_opts(
      EditorCommand::TableOp { block_index, op },
      true,
      None,
      true,
    )
  }

  /// Copies a rectangular sub-grid `(r0..=r1, c0..=c1)` of table block `block_index`.
  pub fn copy_table_area(
    &self,
    block_index: usize,
    r0: usize,
    c0: usize,
    r1: usize,
    c1: usize,
  ) -> Option<ClipboardPayload> {
    let block = self.block_at(block_index)?;
    if block.kind != "table" {
      return None;
    }
    let model = mica_editor::TableModel::from_block_data(&block.data);
    let payload = model.copy_area(r0, c0, r1, c1);
    if payload.plain.is_empty() && payload.markdown.is_empty() && payload.html.is_empty() {
      None
    } else {
      Some(payload)
    }
  }

  /// Sets selection to `block_index` at UTF-16 `offset` and scrolls it into view.
  pub fn focus_block(&self, block_index: usize, offset: u32) {
    let clamped_idx = block_index.min(self.block_len().saturating_sub(1));
    let max_off = self
      .block_at(clamped_idx)
      .map(|b| utf16_len(&b.text))
      .unwrap_or(0);
    self.dispatch_inner(
      EditorCommand::SetSelection {
        selection: Some(DocSelection::caret(clamped_idx, offset.min(max_off))),
      },
      false,
      Some(ScrollMode::Jump),
    );
    self.request_focus();
  }

  /// Ensures the document ends with an editable empty paragraph (appending one
  /// if the last block has content or is non-paragraph) and focuses it.
  pub fn append_empty_paragraph_and_focus(&self) {
    self.dispatch(EditorCommand::AppendEmptyParagraph);
    self.request_focus();
  }

  /// Compatibility alias for `lib.rs` / `right_sidebar.rs` outline jump (`pos == block_index`).
  pub fn focus_pos(&self, block_index: usize) {
    self.focus_block(block_index, 0);
  }

  pub fn set_selection(&self, sel: DocSelection) {
    self.dispatch_inner(
      EditorCommand::SetSelection { selection: Some(sel) },
      false,
      None,
    );
  }

  /// Clears any active document selection / caret (e.g. clicking outside prose).
  pub fn clear_selection(&self) {
    if self.selection().is_some() {
      self.dispatch_inner(
        EditorCommand::SetSelection { selection: None },
        false,
        None,
      );
    }
  }

  pub fn select_all(&self) {
    let count = self.block_len();
    if count == 0 {
      return;
    }
    let last_idx = count - 1;
    let last_len = self
      .block_at(last_idx)
      .map(|b| utf16_len(&b.text))
      .unwrap_or(0);
    self.set_selection(DocSelection::from_positions(
      DocPosition::new(0, 0),
      DocPosition::new(last_idx, last_len),
    ));
  }

  // ── Structured Clipboard Paste / Copy / Cut ──────────────────────────────

  /// Pastes clipboard `text` and/or `html` through `crates/mica-editor::paste`
  /// and `crates/md-core` (single Markdown source of truth).
  pub fn paste_clipboard(&self, text: &str, html: &str, plain_literal: bool) -> bool {
    if text.is_empty() && html.is_empty() {
      return false;
    }
    if plain_literal {
      let patch = self.dispatch(EditorCommand::Paste {
        input: PasteInput::PlainLiteral { text: text.to_owned() },
      });
      return patch.content_changed;
    }

    let normalized = normalize_pasted_text(text);
    let text_is_md = looks_like_mica_markdown(&normalized);
    let html_frag = crate::clipboard_platform::extract_cf_html_fragment(html)
      .unwrap_or_default();
    let html_has_semantics = !html_frag.trim().is_empty() && has_semantic_html_tags(&html_frag);
    let html_is_table = crate::clipboard_platform::clipboard_html_is_data_table(&html_frag);

    // 1. Prefer canonical Markdown text directly when the plain clipboard flavor
    // already carries explicit Markdown syntax (e.g. copied from Mica itself,
    // VS Code, Typora, or Obsidian) unless HTML has a richer data <table> that
    // plain text lacks. This guarantees 100% lossless round-trip for code
    // languages, Mermaid, boards, callouts, todos, marks, and tables.
    if text_is_md && !(html_is_table && !normalized.contains("---")) {
      let patch = self.dispatch(EditorCommand::Paste {
        input: PasteInput::Markdown {
          markdown: normalized.clone(),
          plain: text.to_owned(),
          rich: true,
        },
      });
      if patch.content_changed {
        return true;
      }
    }

    // 2. Convert semantic HTML (e.g. Windows CF_HTML copied from v0.14.11
    // Flutter Mica, browsers, Word, Feishu, Excel) through `crates/md-core`.
    if html_has_semantics {
      let body = parse_html_fragment_to_node(&html_frag);
      let md = html_to_markdown(&body);
      let trimmed = md.trim();
      if !trimmed.is_empty() {
        let patch = self.dispatch(EditorCommand::Paste {
          input: PasteInput::Markdown {
            markdown: trimmed.to_owned(),
            plain: text.to_owned(),
            rich: true,
          },
        });
        if patch.content_changed {
          return true;
        }
      }
    }

    // 3. Fallback: Markdown-normalized plain text or plain paste.
    let input = if text_is_md {
      PasteInput::Markdown {
        markdown: normalized,
        plain: text.to_owned(),
        rich: true,
      }
    } else {
      PasteInput::Plain { text: text.to_owned() }
    };
    let patch = self.dispatch(EditorCommand::Paste { input });
    patch.content_changed
  }

  pub fn copy_selection(&self) -> Option<ClipboardPayload> {
    let patch = self.dispatch_inner(
      EditorCommand::CopySelection { image_urls: BTreeMap::new() },
      false,
      None,
    );
    patch.clipboard_out
  }

  pub fn cut_selection(&self) -> Option<ClipboardPayload> {
    let patch = self.dispatch(EditorCommand::CutSelection {
      image_urls: BTreeMap::new(),
    });
    patch.clipboard_out
  }

  // ── Block Range, Links, Find & Front Matter ──────────────────────────────

  pub fn delete_block_range(&self, lo: usize, hi: usize) {
    let count = self.block_len();
    if count == 0 || lo >= count {
      return;
    }
    let hi = hi.min(count - 1);
    let end_off = self
      .block_at(hi)
      .map(|b| utf16_len(&b.text))
      .unwrap_or(0);
    self.dispatch_inner(
      EditorCommand::SetSelection {
        selection: Some(DocSelection::from_positions(
          DocPosition::new(lo, 0),
          DocPosition::new(hi, end_off),
        )),
      },
      false,
      None,
    );
    self.dispatch(EditorCommand::DeleteSelectedBlocks);
  }

  pub fn copy_block_range_payload(&self, lo: usize, hi: usize) -> Option<ClipboardPayload> {
    let count = self.block_len();
    if count == 0 || lo >= count {
      return None;
    }
    let hi = hi.min(count - 1);
    let end_off = self
      .block_at(hi)
      .map(|b| utf16_len(&b.text))
      .unwrap_or(0);
    let prev = self.selection();
    self.dispatch_inner(
      EditorCommand::SetSelection {
        selection: Some(DocSelection::from_positions(
          DocPosition::new(lo, 0),
          DocPosition::new(hi, end_off),
        )),
      },
      false,
      None,
    );
    let payload = self.copy_selection();
    if let Some(p) = prev {
      self.set_selection(p);
    }
    payload
  }

  pub fn copy_block_range_markdown(&self, lo: usize, hi: usize) -> String {
    self
      .copy_block_range_payload(lo, hi)
      .map(|c| c.markdown)
      .unwrap_or_default()
  }

  pub fn set_link_mark(&self, href: &str) {
    let href = href.trim();
    if href.is_empty() {
      self.unset_link_mark();
      return;
    }
    let Some(sel) = self.selection() else {
      return;
    };
    let start = sel.start_pos();
    let end = sel.end_pos();
    if start.node == end.node && start.offset < end.offset {
      self.dispatch(EditorCommand::SetLink {
        block_index: start.node,
        start: start.offset,
        end: end.offset,
        href: Some(href.to_owned()),
        text: None,
      });
    } else {
      self.dispatch(EditorCommand::ToggleMark {
        kind: "link".to_owned(),
        href: Some(href.to_owned()),
      });
    }
  }

  pub fn unset_link_mark(&self) {
    let Some(sel) = self.selection() else {
      return;
    };
    let start = sel.start_pos();
    let end = sel.end_pos();
    if start.node == end.node && start.offset < end.offset {
      self.dispatch(EditorCommand::SetLink {
        block_index: start.node,
        start: start.offset,
        end: end.offset,
        href: None,
        text: None,
      });
    } else {
      self.dispatch(EditorCommand::ToggleMark {
        kind: "link".to_owned(),
        href: None,
      });
    }
  }

  pub fn current_link_href(&self) -> Option<String> {
    let s = self.session.borrow();
    s.active_marks().link_href
  }

  pub fn selected_text(&self) -> String {
    let s = self.session.borrow();
    let Some(sel) = s.selection() else {
      return String::new();
    };
    if sel.is_collapsed() {
      return String::new();
    }
    mica_editor::selection_clipboard_text(s.blocks(), sel, &BTreeMap::new())
  }

  pub fn find_matches(&self, query: &str) -> Vec<FindMatch> {
    if query.is_empty() {
      self.set_find_highlights(Vec::new(), 0);
      return Vec::new();
    }
    let patch = self.dispatch_inner(
      EditorCommand::FindReplace {
        op: FindReplaceOp::FindAll {
          query: query.to_owned(),
          case_sensitive: false,
        },
      },
      false,
      None,
    );
    let matches = patch.find_matches.unwrap_or_default();
    self.set_find_highlights(matches.clone(), 0);
    matches
  }

  pub fn select_find_match(&self, m: &FindMatch, active_idx: usize) {
    self.find_active_idx.set(active_idx);
    self.dispatch_inner(
      EditorCommand::SetSelection {
        selection: Some(DocSelection::from_positions(
          DocPosition::new(m.block_index, m.start),
          DocPosition::new(m.block_index, m.end),
        )),
      },
      false,
      Some(ScrollMode::Jump),
    );
  }

  pub fn front_matter(&self) -> String {
    let s = self.session.borrow();
    read_root_front_matter(s.doc(), s.root_block_id())
  }

  pub fn set_front_matter(&self, yaml: &str) {
    let trimmed = yaml.trim();
    let (crdt_bytes, changed) = {
      let mut s = self.session.borrow_mut();
      let root_id = s.root_block_id().to_owned();
      let cur = read_root_front_matter(s.doc(), &root_id);
      if cur == trimmed {
        (None, false)
      } else {
        let sv = s.state_vector();
        let mut root_data = s
          .doc()
          .to_blocks()
          .into_iter()
          .find(|b| b.id == root_id)
          .and_then(|b| b.data.as_object().cloned())
          .unwrap_or_default();
        if trimmed.is_empty() {
          root_data.remove("front_matter");
        } else {
          root_data.insert("front_matter".into(), Value::String(trimmed.to_owned()));
        }
        let next_val = if root_data.is_empty() {
          Value::Null
        } else {
          Value::Object(root_data)
        };
        s.doc_mut().set_block_data(&root_id, &next_val);
        let diff = s.doc().encode_diff(&sv).ok().filter(|d| !d.is_empty());
        (diff, true)
      }
    };
    if changed {
      self.front_matter.set(trimmed.to_owned());
      self.last_edited.set(String::from(DEFAULT_STATUS_TEXT));
      self.session_rev.set(self.session_rev.get_untracked().wrapping_add(1));
      let cb = self.on_persist.borrow().clone();
      if let Some(cb) = cb {
        cb(self.export_markdown(), crdt_bytes);
      }
    }
  }

  pub fn export_markdown(&self) -> String {
    self.session.borrow().export_markdown(&BTreeMap::new())
  }

  pub fn export_html(&self) -> String {
    self
      .session
      .borrow()
      .export_html(&BTreeMap::new(), HtmlProfile::Export)
  }

  // ── Signal Getters (API-compatible with all shell consumers) ─────────────

  pub fn front_matter_signal(&self) -> Signal<String> {
    self.front_matter
  }
  pub fn word_count_signal(&self) -> Signal<String> {
    self.word_count
  }
  pub fn block_count_signal(&self) -> Signal<usize> {
    self.block_count
  }
  pub fn outline_signal(&self) -> Signal<Vec<OutlineItem>> {
    self.outline
  }
  pub fn can_undo_signal(&self) -> Signal<bool> {
    self.can_undo
  }
  pub fn can_redo_signal(&self) -> Signal<bool> {
    self.can_redo
  }
  pub fn active_kind_signal(&self) -> Signal<String> {
    self.active_kind
  }
  pub fn active_level_signal(&self) -> Signal<u64> {
    self.active_level
  }
  pub fn active_marks_signal(&self) -> Signal<Vec<String>> {
    self.active_marks
  }
  pub fn last_edited_signal(&self) -> Signal<String> {
    self.last_edited
  }
  pub fn set_status_flash(&self, msg: impl Into<String>) {
    self.last_edited.set(msg.into());
  }
  pub fn session_rev_signal(&self) -> Signal<u64> {
    self.session_rev
  }
}

// ── Derived Document Metrics & Active State Helpers ──────────────────────────

fn session_from_markdown(markdown: &str) -> EditorSession {
  let session = EditorSession::from_markdown(markdown);
  if session.blocks().is_empty() {
    EditorSession::from_blocks(
      session.root_block_id().to_owned(),
      vec![EditorBlock::new(uuid::Uuid::new_v4().to_string(), "paragraph", "")],
    )
  } else {
    session
  }
}

fn set_if_changed<T: Clone + PartialEq + 'static>(sig: Signal<T>, val: T) {
  if sig.get_untracked() != val {
    sig.set(val);
  }
}

fn read_root_front_matter(doc: &MicaDoc, root_id: &str) -> String {
  doc
    .to_blocks()
    .into_iter()
    .find(|b| b.id == root_id)
    .and_then(|b| b.data.get("front_matter").and_then(Value::as_str).map(str::to_owned))
    .unwrap_or_default()
}

fn compute_outline(blocks: &[EditorBlock]) -> Vec<OutlineItem> {
  let mut items = Vec::new();
  for (idx, b) in blocks.iter().enumerate() {
    if b.kind == "heading" {
      let level = b.data.get("level").and_then(Value::as_u64).unwrap_or(1).clamp(1, 6);
      let label = b.text.trim();
      if !label.is_empty() {
        items.push(OutlineItem {
          block_index: idx,
          pos: idx,
          level,
          text: label.to_owned(),
        });
      }
    }
  }
  items
}

fn compute_word_count_label(blocks: &[EditorBlock]) -> String {
  let (mut chars, mut words) = (0usize, 0usize);
  for b in blocks {
    let raw = b.text.trim();
    if raw.is_empty() {
      continue;
    }
    let mut in_ascii = false;
    for ch in raw.chars() {
      if ch.is_whitespace() {
        in_ascii = false;
        continue;
      }
      chars += 1;
      if ch.is_ascii_alphanumeric() {
        if !in_ascii {
          words += 1;
          in_ascii = true;
        }
      } else {
        in_ascii = false;
        if ('\u{4E00}'..='\u{9FFF}').contains(&ch)
          || ('\u{3400}'..='\u{4DBF}').contains(&ch)
          || ('\u{3040}'..='\u{30FF}').contains(&ch)
          || ('\u{AC00}'..='\u{D7AF}').contains(&ch)
        {
          words += 1;
        }
      }
    }
  }
  format!("{words} 词 · {chars} 字")
}

fn compute_active_state(session: &EditorSession) -> (String, u64, Vec<String>) {
  let blocks = session.blocks();
  let sel_idx = session
    .selection()
    .map(|s| s.focus_node())
    .unwrap_or(0);
  let (kind, level) = blocks
    .get(sel_idx)
    .map(|b| {
      let lvl = if b.kind == "heading" {
        b.data.get("level").and_then(Value::as_u64).unwrap_or(1)
      } else {
        0
      };
      (b.kind.clone(), lvl)
    })
    .unwrap_or_else(|| ("paragraph".to_owned(), 0));

  let active = session.active_marks();
  let mut marks = Vec::new();
  if active.has("bold") {
    marks.push("bold".to_owned());
  }
  if active.has("italic") {
    marks.push("italic".to_owned());
  }
  if active.has("strike") {
    marks.push("strike".to_owned());
  }
  if active.has("code") {
    marks.push("code".to_owned());
  }
  if active.link_href.is_some() {
    marks.push("link".to_owned());
  }
  (kind, level, marks)
}

/// Canonical block kind helper (`code_block` with `language == "mermaid"` / `"mica-board"`
/// resolves to `"mermaid"` / `"board"`).
pub fn effective_block_kind(block: &EditorBlock) -> &str {
  if block.kind == "code_block" {
    let lang = block
      .data
      .get("language")
      .and_then(Value::as_str)
      .unwrap_or_default();
    if lang.eq_ignore_ascii_case("mermaid") {
      return "mermaid";
    }
    if lang.eq_ignore_ascii_case("mica-board") || lang.eq_ignore_ascii_case("board") {
      return "board";
    }
  }
  block.kind.as_str()
}

/// Extract normalized UTF-16 marks from an `EditorBlock`.
pub fn block_marks(block: &EditorBlock) -> Vec<EditorMark> {
  mica_editor::marks_from_data(&block.data)
}

// ── Markdown & HTML Clipboard Helpers ────────────────────────────────────────

fn has_semantic_html_tags(html: &str) -> bool {
  let lower = html.to_ascii_lowercase();
  [
    "<h1", "<h2", "<h3", "<h4", "<h5", "<h6", "<ul", "<ol", "<li", "<blockquote",
    "<table", "<strong", "<b>", "<b ", "<em", "<i>", "<i ", "<pre", "<code",
    "<img", "<a ", "<hr", "<del", "<s>", "<s ",
  ]
  .iter()
  .any(|t| lower.contains(t))
}

fn is_bare_image_url(line: &str) -> bool {
  let trimmed = line.trim();
  if !(trimmed.starts_with("http://") || trimmed.starts_with("https://"))
    || trimmed.contains(char::is_whitespace)
  {
    return false;
  }
  let without_query = trimmed
    .split(['?', '#'])
    .next()
    .unwrap_or(trimmed)
    .to_ascii_lowercase();
  [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp", ".avif"]
    .iter()
    .any(|ext| without_query.ends_with(ext))
}

fn normalize_pasted_text(raw: &str) -> String {
  let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
  let mut out = Vec::new();
  let mut in_fence = false;
  for line in normalized.lines() {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
      in_fence = !in_fence;
      out.push(line.to_owned());
      continue;
    }
    if !in_fence {
      if let Some(rest) = trimmed.strip_prefix("• ") {
        let indent = &line[..line.len() - trimmed.len()];
        out.push(format!("{indent}- {rest}"));
        continue;
      }
      if is_bare_image_url(line) {
        out.push(format!("![]({})", line.trim()));
        continue;
      }
    }
    out.push(line.to_owned());
  }
  out.join("\n")
}

fn looks_like_mica_markdown(text: &str) -> bool {
  for line in text.lines() {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
      return true;
    }
    if trimmed.starts_with('#') {
      let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
      if (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ') {
        return true;
      }
    }
    if trimmed == ">" || trimmed.starts_with("> ") {
      return true;
    }
    if trimmed.starts_with("- ")
      || trimmed.starts_with("* ")
      || trimmed.starts_with("+ ")
      || trimmed.starts_with("• ")
    {
      return true;
    }
    let digits = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0
      && matches!(trimmed.as_bytes().get(digits), Some(b'.' | b')'))
      && trimmed.as_bytes().get(digits + 1) == Some(&b' ')
    {
      return true;
    }
    let t = trimmed.trim_end();
    if t.len() >= 3
      && (t.bytes().all(|b| b == b'-')
        || t.bytes().all(|b| b == b'*')
        || t.bytes().all(|b| b == b'_'))
    {
      return true;
    }
    if t.contains('|')
      && t.contains("---")
      && t.bytes().all(|b| matches!(b, b'|' | b'-' | b':' | b' ' | b'\t'))
    {
      return true;
    }
    if is_bare_image_url(t) {
      return true;
    }
  }
  let has_pair = |delim: &str| -> bool {
    if let Some(i) = text.find(delim) {
      let rest = &text[i + delim.len()..];
      if let Some(j) = rest.find(delim) {
        return j > 0 && !rest[..j].contains('\n');
      }
    }
    false
  };
  if has_pair("**") || has_pair("__") || has_pair("~~") || has_pair("`") {
    return true;
  }
  if let Some(i) = text.find("](") {
    let before = &text[..i];
    let after = &text[i + 2..];
    if before.rfind('[').is_some_and(|b| !before[b..].contains('\n'))
      && after.find(')').is_some_and(|e| e > 0 && !after[..e].contains('\n'))
    {
      return true;
    }
  }
  false
}

/// Lightweight zero-dependency HTML fragment parser into `mica_md::HtmlNode`
/// (handles Windows `CF_HTML` fragment markers, common semantic tags, attributes, and entities).
fn parse_html_fragment_to_node(raw: &str) -> HtmlNode {
  let extracted = crate::clipboard_platform::extract_cf_html_fragment(raw);
  let fragment = extracted.as_deref().unwrap_or(raw);

  let mut stack: Vec<(String, BTreeMap<String, String>, Vec<HtmlNode>)> =
    vec![("body".into(), BTreeMap::new(), Vec::new())];
  let bytes = fragment.as_bytes();
  let mut i = 0usize;

  while i < bytes.len() {
    if bytes[i] == b'<' {
      // Skip comments / doctype
      if fragment[i..].starts_with("<!--") {
        if let Some(end) = fragment[i + 4..].find("-->") {
          i += 4 + end + 3;
          continue;
        }
        break;
      }
      if let Some(rel_close) = fragment[i + 1..].find('>') {
        let tag_inner = fragment[i + 1..i + 1 + rel_close].trim();
        i += rel_close + 2;
        if tag_inner.is_empty() || tag_inner.starts_with('!') || tag_inner.starts_with('?') {
          continue;
        }
        if let Some(close_name) = tag_inner.strip_prefix('/') {
          let cname = close_name.trim().to_ascii_lowercase();
          if stack.len() > 1
            && let Some(pos) = stack.iter().rposition(|(n, _, _)| *n == cname)
            && pos > 0
          {
            while stack.len() > pos {
              let (e, a, c) = stack.pop().unwrap();
              let el = HtmlNode::Element { e, a, c };
              if let Some(parent) = stack.last_mut() {
                parent.2.push(el);
              }
            }
          }
          continue;
        }
        let self_closing = tag_inner.ends_with('/');
        let body = tag_inner.trim_end_matches('/').trim();
        let (tag_name, attrs) = parse_tag_and_attrs(body);
        if tag_name.is_empty() {
          continue;
        }
        // Skip script/style contents
        if matches!(tag_name.as_str(), "script" | "style" | "head" | "meta" | "link") {
          if !self_closing && matches!(tag_name.as_str(), "script" | "style" | "head") {
            let needle = format!("</{tag_name}>");
            if let Some(end) = fragment[i..].to_ascii_lowercase().find(&needle) {
              i += end + needle.len();
            }
          }
          continue;
        }
        let is_void = self_closing
          || matches!(tag_name.as_str(), "br" | "hr" | "img" | "input" | "col" | "source");
        if is_void {
          let el = HtmlNode::Element {
            e: tag_name,
            a: attrs,
            c: Vec::new(),
          };
          if let Some(parent) = stack.last_mut() {
            parent.2.push(el);
          }
        } else {
          stack.push((tag_name, attrs, Vec::new()));
        }
        continue;
      }
    }
    let start = i;
    while i < bytes.len() && bytes[i] != b'<' {
      i += 1;
    }
    if i > start {
      let decoded = decode_html_entities(&fragment[start..i]);
      if !decoded.is_empty()
        && let Some(parent) = stack.last_mut()
      {
        parent.2.push(HtmlNode::Text { t: decoded });
      }
    }
  }

  while stack.len() > 1 {
    let (e, a, c) = stack.pop().unwrap();
    let el = HtmlNode::Element { e, a, c };
    if let Some(parent) = stack.last_mut() {
      parent.2.push(el);
    }
  }
  let (e, a, c) = stack.pop().unwrap();
  HtmlNode::Element { e, a, c }
}

fn parse_tag_and_attrs(s: &str) -> (String, BTreeMap<String, String>) {
  let mut attrs = BTreeMap::new();
  let mut parts = s.splitn(2, char::is_whitespace);
  let tag = parts.next().unwrap_or_default().to_ascii_lowercase();
  let mut rest = parts.next().unwrap_or_default().trim();
  while !rest.is_empty() {
    let name_end = rest
      .find(|c: char| c.is_whitespace() || c == '=')
      .unwrap_or(rest.len());
    let key = rest[..name_end].trim().to_ascii_lowercase();
    rest = rest[name_end..].trim_start();
    if let Some(after_eq) = rest.strip_prefix('=') {
      let after_eq = after_eq.trim_start();
      let (val, next_rest) = if let Some(q) = after_eq.strip_prefix('"') {
        match q.find('"') {
          Some(end) => (&q[..end], &q[end + 1..]),
          None => (q, ""),
        }
      } else if let Some(q) = after_eq.strip_prefix('\'') {
        match q.find('\'') {
          Some(end) => (&q[..end], &q[end + 1..]),
          None => (q, ""),
        }
      } else {
        let end = after_eq.find(char::is_whitespace).unwrap_or(after_eq.len());
        (&after_eq[..end], &after_eq[end..])
      };
      if !key.is_empty() {
        attrs.insert(key, decode_html_entities(val));
      }
      rest = next_rest.trim_start();
    } else if !key.is_empty() {
      attrs.insert(key, String::new());
    }
  }
  (tag, attrs)
}

fn decode_html_entities(s: &str) -> String {
  if !s.contains('&') {
    return s.to_owned();
  }
  let mut out = String::with_capacity(s.len());
  let bytes = s.as_bytes();
  let mut i = 0usize;
  while i < bytes.len() {
    if bytes[i] == b'&'
      && let Some(semi) = s[i + 1..].find(';')
      && semi <= 10
    {
      let ent = &s[i + 1..i + 1 + semi];
      let decoded = match ent {
        "amp" => Some("&".to_owned()),
        "lt" => Some("<".to_owned()),
        "gt" => Some(">".to_owned()),
        "quot" => Some("\"".to_owned()),
        "apos" | "#39" => Some("'".to_owned()),
        "nbsp" => Some(" ".to_owned()),
        _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16)
          .ok()
          .and_then(char::from_u32)
          .map(|c| c.to_string()),
        _ if ent.starts_with('#') => ent[1..]
          .parse::<u32>()
          .ok()
          .and_then(char::from_u32)
          .map(|c| c.to_string()),
        _ => None,
      };
      if let Some(d) = decoded {
        out.push_str(&d);
        i += semi + 2;
        continue;
      }
    }
    let ch = s[i..].chars().next().unwrap();
    out.push(ch);
    i += ch.len_utf8();
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn with_runtime(f: impl FnOnce()) {
    let rt = nsh_core::element::Runtime::default();
    let _g = rt.enter();
    f();
  }

  #[test]
  fn typing_bumps_only_single_block_rev_never_structure_rev() {
    with_runtime(|| {
      let bridge = MicaDocumentBridge::from_markdown("# Title\n\nParagraph A\n\nParagraph B");
      assert_eq!(bridge.block_len(), 3);
      let s0 = bridge.structure_rev_signal().get_untracked();
      let id1 = bridge.block_at(1).unwrap().id;
      let id2 = bridge.block_at(2).unwrap().id;
      let rev1 = bridge.block_rev_signal(&id1);
      let rev2 = bridge.block_rev_signal(&id2);
      let r1_0 = rev1.get_untracked();
      let r2_0 = rev2.get_untracked();

      bridge.focus_block(1, 11);
      bridge.dispatch(EditorCommand::InsertText { text: "!".into() });

      assert_eq!(
        bridge.structure_rev_signal().get_untracked(),
        s0,
        "typing inside a block must NEVER bump structure_rev"
      );
      assert!(rev1.get_untracked() > r1_0, "edited block rev must bump");
      assert_eq!(rev2.get_untracked(), r2_0, "untouched sibling block rev must NOT bump");
      assert_eq!(bridge.block_at(1).unwrap().text, "Paragraph A!");

      // Splitting a block DOES bump structure_rev
      bridge.dispatch(EditorCommand::SplitBlock);
      assert!(bridge.structure_rev_signal().get_untracked() > s0);
      assert_eq!(bridge.block_len(), 4);
    });
  }

  #[test]
  fn markdown_and_html_paste_round_trip_headings_quotes_lists_tables_and_images() {
    with_runtime(|| {
      let bridge = MicaDocumentBridge::from_markdown("");
      let md_paste = concat!(
        "# HGX B300 清单\n\n",
        "> 用途：**对外待确认事项**\n\n",
        "## 0. 优先级总览\n\n",
        "| P0 阻塞项 | 收件方 |\n",
        "| --- | --- |\n",
        "| 单柜功率 | IDC |\n\n",
        "---\n\n",
        "• 日期：2026-11-04\n\n",
        "https://mica.cloudcele.com/api/workspaces/w1/files/f1/blob/pasted-image.png"
      );
      assert!(bridge.paste_clipboard(md_paste, "", false));
      let kinds: Vec<String> = bridge.with_blocks(|bs| bs.iter().map(|b| b.kind.clone()).collect());
      assert!(kinds.iter().any(|k| k == "heading"), "expected heading in {kinds:?}");
      assert!(kinds.iter().any(|k| k == "quote"), "expected quote in {kinds:?}");
      assert!(kinds.iter().any(|k| k == "table"), "expected table in {kinds:?}");
      assert!(kinds.iter().any(|k| k == "divider"), "expected divider in {kinds:?}");
      assert!(kinds.iter().any(|k| k == "bulleted_list"), "expected bulleted_list in {kinds:?}");
      assert!(kinds.iter().any(|k| k == "image"), "expected image in {kinds:?}");

      // HTML paste (including Windows CF_HTML fragment markers)
      let bridge2 = MicaDocumentBridge::from_markdown("");
      let html = r#"<!--StartFragment--><ul><li><strong>日期：</strong>2026-11-04</li></ul><h2>现象</h2><img src="https://example.com/a.png" alt="图1"><!--EndFragment-->"#;
      assert!(bridge2.paste_clipboard("fallback", html, false));
      let kinds2: Vec<String> = bridge2.with_blocks(|bs| bs.iter().map(|b| b.kind.clone()).collect());
      assert!(kinds2.iter().any(|k| k == "bulleted_list"), "expected bulleted_list in {kinds2:?}");
      assert!(kinds2.iter().any(|k| k == "heading"), "expected heading in {kinds2:?}");
      assert!(kinds2.iter().any(|k| k == "image"), "expected image in {kinds2:?}");
    });
  }

  #[test]
  fn large_document_loads_fast_and_undo_redo_stay_single_stack() {
    with_runtime(|| {
      let mut big = String::new();
      for i in 0..300 {
        big.push_str(&format!("## Section {i}\n\n- [ ] Task item {i} with **bold** and `code`\n\n"));
      }
      let t0 = std::time::Instant::now();
      let bridge = MicaDocumentBridge::from_markdown(&big);
      assert!(bridge.block_len() >= 600);
      assert!(
        t0.elapsed() < std::time::Duration::from_millis(500),
        "loading 600+ block markdown should be fast, took {:?}",
        t0.elapsed()
      );
      bridge.focus_block(0, 0);
      bridge.dispatch(EditorCommand::InsertText { text: "X".into() });
      assert!(bridge.can_undo_signal().get_untracked());
      bridge.undo();
      assert!(bridge.can_redo_signal().get_untracked());
      bridge.redo();
    });
  }

  #[test]
  fn rapid_enter_and_backspace_do_not_rebuild_siblings_or_sync_persist() {
    with_runtime(|| {
      let mut md = String::from("# Runbook\n\nIntro line\n\n```mermaid\nflowchart LR\n  A[阶段0] --> B[阶段1]\n```\n\n");
      for i in 0..40 {
        md.push_str(&format!("Paragraph {i} body content\n\n"));
      }
      let bridge = MicaDocumentBridge::from_markdown(&md);
      let initial_len = bridge.block_len();
      let mermaid_id = bridge.block_at(2).unwrap().id;
      let tail_id = bridge.block_at(initial_len - 1).unwrap().id;
      let mermaid_rev = bridge.block_rev_signal(&mermaid_id);
      let tail_rev = bridge.block_rev_signal(&tail_id);
      let m0 = mermaid_rev.get_untracked();
      let t0_rev = tail_rev.get_untracked();

      let dirty_count = Rc::new(std::cell::Cell::new(0usize));
      let persist_count = Rc::new(std::cell::Cell::new(0usize));
      {
        let dc = dirty_count.clone();
        bridge.set_on_dirty(Rc::new(move || dc.set(dc.get() + 1)));
      }
      {
        let pc = persist_count.clone();
        bridge.set_on_persist(Rc::new(move |_md, _crdt| pc.set(pc.get() + 1)));
      }

      bridge.focus_block(1, 5);
      let start = std::time::Instant::now();
      for _ in 0..30 {
        bridge.dispatch(EditorCommand::SplitBlock);
      }
      assert_eq!(bridge.block_len(), initial_len + 30);
      assert_eq!(bridge.index_of_block_id(&mermaid_id), Some(32));
      assert_eq!(bridge.block_by_id(&mermaid_id).unwrap().id, mermaid_id);

      for _ in 0..30 {
        bridge.dispatch(EditorCommand::DeleteBackward);
      }
      let elapsed = start.elapsed();

      assert_eq!(bridge.block_len(), initial_len);
      assert_eq!(bridge.index_of_block_id(&mermaid_id), Some(2));
      assert_eq!(
        mermaid_rev.get_untracked(),
        m0,
        "Mermaid sibling below Enter/Backspace must NEVER have its block_rev bumped"
      );
      assert_eq!(
        tail_rev.get_untracked(),
        t0_rev,
        "Tail sibling below Enter/Backspace must NEVER have its block_rev bumped"
      );
      assert_eq!(dirty_count.get(), 60);
      assert_eq!(
        persist_count.get(),
        0,
        "rapid Enter/Backspace must coalesce via on_dirty rather than exporting synchronously on every keystroke"
      );

      bridge.flush_pending_persist();
      assert_eq!(persist_count.get(), 1, "flush_pending_persist must export once");
      assert!(
        elapsed < std::time::Duration::from_millis(500),
        "60 structural Enter/Backspace ops should stay snappy, took {elapsed:?}"
      );
    });
  }
}
