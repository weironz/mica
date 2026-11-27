//! Document editor key routing, hidden IME sink diffing, rich clipboard paste,
//! caret word boundaries, and slash-command state transitions.

use std::{cell::RefCell, rc::Rc};

use mica_editor::{
  DocPosition, DocSelection, EditorCommand,
  utf16::{utf16_len, utf16_next_boundary, utf16_prev_boundary, utf16_slice},
};
use nsh_core::{
  element::{ClipboardItem, clipboard_string, set_clipboard_item},
  prelude::*,
};

use crate::{
  editor_bridge::MicaDocumentBridge,
  editor_positioning::move_caret_vertical_preserving_x,
  editor_slash::{SlashItem, extract_slash_query, filter_slash_items},
  editor_view::{ImeSinkState, SLASH_MENU_WIDTH, SINK_LIMIT, SlashMenuSignals},
  text_geometry::VisibleLayoutCache,
};

#[derive(Clone, Copy)]
pub(crate) enum EditorKeyAction {
  Enter,
  SoftBreak,
  Backspace,
  Delete,
  Indent(i32),
  ArrowLeft { shift: bool, word: bool },
  ArrowRight { shift: bool, word: bool },
  ArrowUp { shift: bool },
  ArrowDown { shift: bool },
  Home { shift: bool },
  End { shift: bool },
  SelectAll,
  Copy,
  Cut,
  PastePlain,
  Undo,
  Redo,
  ToggleMark(&'static str),
  OpenLink,
  Escape,
}

fn common_prefix_bytes(a: &str, b: &str) -> usize {
  a.char_indices()
    .zip(b.chars())
    .take_while(|((_, ca), cb)| ca == cb)
    .map(|((i, c), _)| i + c.len_utf8())
    .last()
    .unwrap_or(0)
}

pub(crate) fn empty_ime_sink(sink_state: &Rc<RefCell<ImeSinkState>>, sink_value: Signal<String>) {
  let mut st = sink_state.borrow_mut();
  if st.composing || st.text.is_empty() {
    return;
  }
  st.emptied = Some(std::mem::take(&mut st.text));
  drop(st);
  sink_value.set(String::new());
}

pub(crate) fn apply_sink_text_diff(
  text: &str,
  sink_state: &Rc<RefCell<ImeSinkState>>,
  sink_value: Signal<String>,
  bridge: &MicaDocumentBridge,
  slash: SlashMenuSignals,
  ai_enabled: Signal<bool>,
) {
  let (deleted, inserted) = {
    let mut st = sink_state.borrow_mut();
    let base = match st.emptied.take() {
      Some(old) if text != old && text.starts_with(old.as_str()) => old,
      Some(_) => String::new(),
      None => std::mem::take(&mut st.text),
    };
    let prefix = common_prefix_bytes(&base, text);
    let deleted = base[prefix..].chars().count();
    let inserted = text[prefix..].to_owned();
    st.text = text.to_owned();
    (deleted, inserted)
  };
  sink_value.set(text.to_owned());

  // Never drop keystrokes if the bridge somehow had no active selection yet.
  if (deleted > 0 || !inserted.is_empty()) && bridge.selection().is_none() {
    let last = bridge.block_len().saturating_sub(1);
    let end_off = bridge
      .block_at(last)
      .map(|b| utf16_len(&b.text))
      .unwrap_or(0);
    bridge.set_selection(DocSelection::caret(last, end_off));
  }

  for _ in 0..deleted {
    bridge.dispatch(EditorCommand::DeleteBackward);
  }
  if !inserted.is_empty() {
    let mut parts = inserted.split('\n').peekable();
    while let Some(part) = parts.next() {
      if !part.is_empty() {
        bridge.dispatch(EditorCommand::InsertText {
          text: part.to_owned(),
        });
      }
      if parts.peek().is_some() {
        bridge.dispatch(EditorCommand::SplitBlock);
      }
    }
  }
  if sink_state.borrow().text.len() > SINK_LIMIT {
    empty_ime_sink(sink_state, sink_value);
  }

  sync_slash_palette_from_caret(bridge, slash, ai_enabled);
}

pub(crate) fn write_rich_clipboard(payload: mica_editor::ClipboardPayload) {
  let mut item = ClipboardItem::default();
  item.text = if !payload.markdown.is_empty() {
    payload.markdown
  } else {
    payload.plain
  };
  if !payload.html.is_empty() {
    item.html = Some(payload.html);
  }
  set_clipboard_item(item);
}

/// If the caret is collapsed at `(block 0, offset 0)` of a plain paragraph and
/// the page title input is visible, moves focus cleanly back onto the title.
pub(crate) fn should_backspace_to_title(
  show_page_title: bool,
  selection: Option<&DocSelection>,
  first_block: Option<&mica_editor::EditorBlock>,
) -> bool {
  if !show_page_title {
    return false;
  }
  let Some(sel) = selection else {
    return false;
  };
  if !sel.is_collapsed() {
    return false;
  }
  let pos = sel.focus_pos();
  if pos.node != 0 || pos.offset != 0 {
    return false;
  }
  let Some(blk) = first_block else {
    return false;
  };
  blk.kind == "paragraph"
}

pub(crate) fn try_backspace_to_title(
  bridge: &MicaDocumentBridge,
  show_page_title: Signal<bool>,
  page_title: Signal<String>,
  title_ref: ElementRef,
  sink_ref: ElementRef,
  editor_focused: Signal<bool>,
) -> bool {
  let sel = bridge.selection();
  let first = bridge.block_at(0);
  if !should_backspace_to_title(show_page_title.get_untracked(), sel.as_ref(), first.as_ref()) {
    return false;
  }
  // Exact inverse of Title Enter (aligned with `origin/main` 2026-08-28 fix):
  // when line 0 is an empty paragraph with following blocks, remove that empty
  // first line so the body rises up, AND place the caret at the END of the
  // page title.
  if first.as_ref().is_some_and(|b| b.text.is_empty())
    && bridge.block_count_signal().get_untracked() >= 2
  {
    bridge.dispatch(EditorCommand::DeleteBackward);
  }
  let end = page_title.get_untracked().encode_utf16().count() as u32;
  sink_ref.blur();
  editor_focused.set(false);
  bridge.clear_selection();
  title_ref.focus();
  title_ref.set_selection(end, end);
  true
}

/// Unified clipboard paste handler:
/// 1. Pulls Windows `CF_HTML` when NSH's Windows backend leaves `p.html` empty.
/// 2. Prefers data `<table>` HTML over companion preview bitmaps (Excel/Sheets).
/// 3. Pastes clipboard image bytes (`p.files`) or copied image files (`p.uris`).
/// 4. Falls through to Markdown / rich HTML paste (`bridge.paste_clipboard`).
pub(crate) fn handle_clipboard_paste(
  p: &PasteEvent,
  bridge: &Rc<MicaDocumentBridge>,
  image_ctx: &crate::image_blocks::ImageContext,
  slash: SlashMenuSignals,
  ai_enabled: Signal<bool>,
) {
  let html = if p.html.is_empty() {
    crate::clipboard_platform::read_windows_cf_html(&p.text).unwrap_or_default()
  } else {
    p.html.clone()
  };

  let is_data_table_html = !html.is_empty()
    && crate::clipboard_platform::clipboard_html_is_data_table(&html);

  // 1. Pasted image bytes (screenshot / copy image from browser or app):
  //    uploads to cloud storage (getting `file_id`) when signed in, or stores in
  //    Desktop local CAS, matching `v0.14.11`.
  if !is_data_table_html && !p.files.is_empty() {
    let mut inserted_any = false;
    for f in &p.files {
      if f.data.is_empty() {
        continue;
      }
      let alt = std::path::Path::new(&f.name)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != "image")
        .unwrap_or("")
        .to_string();
      crate::image_blocks::insert_image_from_bytes(
        bridge.clone(),
        image_ctx.clone(),
        f.data.clone(),
        f.name.clone(),
        f.mime.clone(),
        alt,
      );
      inserted_any = true;
    }
    if inserted_any {
      sync_slash_palette_from_caret(bridge, slash, ai_enabled);
      return;
    }
  }

  // 2. Pasted local image file URIs (copied from Explorer / Finder)
  if !is_data_table_html && !p.uris.is_empty() {
    let mut inserted_any = false;
    for u in &p.uris {
      if let Some((uri, alt)) = crate::clipboard_platform::image_uri_from_pasted_file_uri(u) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = crate::clipboard_platform::file_uri_to_local_path(&uri)
          && let Ok(bytes) = std::fs::read(&path)
          && !bytes.is_empty()
        {
          let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("image.png")
            .to_string();
          let mime = crate::clipboard_platform::mime_for_image(&bytes, &name).to_string();
          crate::image_blocks::insert_image_from_bytes(
            bridge.clone(),
            image_ctx.clone(),
            bytes,
            name,
            mime,
            alt,
          );
          inserted_any = true;
          continue;
        }
        bridge.insert_custom_block("image", &alt, Some(&uri));
        inserted_any = true;
      }
    }
    if inserted_any {
      sync_slash_palette_from_caret(bridge, slash, ai_enabled);
      return;
    }
  }

  // 3. Same-workspace Mica blob URL: reuse `file_id` directly.
  if html.is_empty()
    && crate::image_blocks::try_handle_pasted_image_url(&p.text, bridge, image_ctx)
  {
    sync_slash_palette_from_caret(bridge, slash, ai_enabled);
    return;
  }

  // 4. Markdown / rich HTML / plain text paste
  if !p.text.is_empty() || !html.is_empty() {
    bridge.paste_clipboard(&p.text, &html, false);
    sync_slash_palette_from_caret(bridge, slash, ai_enabled);
  }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_editor_key(
  action: EditorKeyAction,
  bridge: &Rc<MicaDocumentBridge>,
  layouts: &Rc<RefCell<VisibleLayoutCache>>,
  preferred_caret_x: Signal<Option<f32>>,
  slash: SlashMenuSignals,
  ai_enabled: Signal<bool>,
  link_prompt_open: Signal<bool>,
  link_draft: Signal<String>,
  on_inline_ai: &Rc<dyn Fn(String)>,
  on_insert_image: &Rc<dyn Fn()>,
  image_ctx: &crate::image_blocks::ImageContext,
  show_page_title: Signal<bool>,
  page_title: Signal<String>,
  title_ref: ElementRef,
  sink_ref: ElementRef,
  editor_focused: Signal<bool>,
) {
  // Any action other than Up/Down clears the remembered vertical visual column.
  if !matches!(
    action,
    EditorKeyAction::ArrowUp { .. } | EditorKeyAction::ArrowDown { .. }
  ) && preferred_caret_x.get_untracked().is_some()
  {
    preferred_caret_x.set(None);
  }

  // 1. Intercept ArrowUp / ArrowDown / Enter / Escape when slash menu is open.
  if slash.open.get_untracked() {
    let items = filter_slash_items(&slash.query.get_untracked(), ai_enabled.get_untracked());
    if items.is_empty() {
      close_slash(slash);
    } else {
      match action {
        EditorKeyAction::ArrowDown { shift: false } => {
          let next = (slash.index.get_untracked() + 1) % items.len();
          slash.index.set(next);
          return;
        }
        EditorKeyAction::ArrowUp { shift: false } => {
          let cur = slash.index.get_untracked().min(items.len() - 1);
          let next = (cur + items.len() - 1) % items.len();
          slash.index.set(next);
          return;
        }
        EditorKeyAction::Enter => {
          let idx = slash.index.get_untracked().min(items.len() - 1);
          let item = *items[idx];
          close_slash(slash);
          execute_slash_item(bridge, item, on_inline_ai, on_insert_image);
          return;
        }
        EditorKeyAction::Escape => {
          close_slash(slash);
          return;
        }
        _ => {}
      }
    }
  }

  match action {
    EditorKeyAction::Enter => {
      bridge.dispatch(EditorCommand::SplitBlock);
      sync_slash_palette_from_caret(bridge, slash, ai_enabled);
    }
    EditorKeyAction::SoftBreak => {
      let in_code_block = bridge
        .selection()
        .and_then(|s| bridge.block_at(s.focus_pos().node))
        .is_some_and(|b| crate::editor_bridge::effective_block_kind(&b) == "code_block");
      if in_code_block {
        bridge.dispatch(EditorCommand::SplitBlock);
      } else {
        bridge.dispatch(EditorCommand::InsertText {
          text: "\\\n".to_owned(),
        });
      }
    }
    EditorKeyAction::Backspace => {
      if let Some((img_idx, _, _)) = crate::image_blocks::selected_single_image_block(bridge, image_ctx) {
        bridge.delete_block_and_focus_neighbor(img_idx);
        sync_slash_palette_from_caret(bridge, slash, ai_enabled);
        return;
      }
      if try_backspace_to_title(
        bridge,
        show_page_title,
        page_title,
        title_ref,
        sink_ref,
        editor_focused,
      ) {
        return;
      }
      bridge.dispatch(EditorCommand::DeleteBackward);
      sync_slash_palette_from_caret(bridge, slash, ai_enabled);
    }
    EditorKeyAction::Delete => {
      if let Some((img_idx, _, _)) = crate::image_blocks::selected_single_image_block(bridge, image_ctx) {
        bridge.delete_block_and_focus_neighbor(img_idx);
        sync_slash_palette_from_caret(bridge, slash, ai_enabled);
        return;
      }
      if let Some(sel) = bridge.selection()
        && sel.is_collapsed()
        && let Some(blk) = bridge.block_at(sel.focus_pos().node)
        && crate::editor_bridge::effective_block_kind(&blk) == "code_block"
        && blk.text.is_empty()
      {
        bridge.delete_block_and_focus_neighbor(sel.focus_pos().node);
        sync_slash_palette_from_caret(bridge, slash, ai_enabled);
        return;
      }
      bridge.dispatch(EditorCommand::DeleteForward);
      sync_slash_palette_from_caret(bridge, slash, ai_enabled);
    }
    EditorKeyAction::Indent(delta) => {
      bridge.dispatch(EditorCommand::Indent { delta });
    }
    EditorKeyAction::SelectAll => {
      if let Some(sel) = bridge.selection()
        && !sel.is_multi_node()
        && let Some(blk) = bridge.block_at(sel.focus_pos().node)
        && crate::editor_bridge::effective_block_kind(&blk) == "code_block"
      {
        let end = utf16_len(&blk.text);
        let covers_all = sel.start_pos().offset == 0 && sel.end_pos().offset >= end;
        if end > 0 && !covers_all {
          let idx = sel.focus_pos().node;
          bridge.set_selection(DocSelection::from_positions(
            DocPosition::new(idx, 0),
            DocPosition::new(idx, end),
          ));
          return;
        }
      }
      bridge.select_all();
    }
    EditorKeyAction::Copy => {
      if let Some((_, _, resolved)) = crate::image_blocks::selected_single_image_block(bridge, image_ctx) {
        crate::image_blocks::copy_image_from_user_action(resolved, image_ctx.clone());
        return;
      }
      let img_urls = crate::image_blocks::build_document_image_urls(bridge, image_ctx);
      if let Some(payload) = bridge.copy_selection_with_images(img_urls) {
        write_rich_clipboard(payload);
      }
    }
    EditorKeyAction::Cut => {
      if let Some((_, bid, resolved)) = crate::image_blocks::selected_single_image_block(bridge, image_ctx) {
        crate::image_blocks::cut_image_block(bridge.clone(), bid, resolved, image_ctx.clone());
        return;
      }
      let img_urls = crate::image_blocks::build_document_image_urls(bridge, image_ctx);
      if let Some(payload) = bridge.cut_selection_with_images(img_urls) {
        write_rich_clipboard(payload);
      }
    }
    EditorKeyAction::PastePlain => {
      let br_plain = bridge.clone();
      spawn(async move {
        let plain = clipboard_string().await;
        if !plain.is_empty() {
          br_plain.paste_clipboard(&plain, "", true);
          sync_slash_palette_from_caret(&br_plain, slash, ai_enabled);
        }
      });
    }
    EditorKeyAction::Undo => bridge.undo(),
    EditorKeyAction::Redo => bridge.redo(),
    EditorKeyAction::ToggleMark(m) => bridge.toggle_mark(m),
    EditorKeyAction::OpenLink => {
      link_draft.set(bridge.current_link_href().unwrap_or_default());
      link_prompt_open.set(true);
    }
    EditorKeyAction::Escape => {
      if let Some(sel) = bridge.selection()
        && !sel.is_collapsed()
      {
        let focus = sel.focus_pos();
        bridge.set_selection(DocSelection::caret(focus.node, focus.offset));
      }
    }
    EditorKeyAction::ArrowLeft { shift, word } => {
      move_caret_horizontal(bridge, -1, shift, word);
    }
    EditorKeyAction::ArrowRight { shift, word } => {
      move_caret_horizontal(bridge, 1, shift, word);
    }
    EditorKeyAction::ArrowUp { shift } => {
      move_caret_vertical_preserving_x(bridge, layouts, preferred_caret_x, -1, shift);
    }
    EditorKeyAction::ArrowDown { shift } => {
      move_caret_vertical_preserving_x(bridge, layouts, preferred_caret_x, 1, shift);
    }
    EditorKeyAction::Home { shift } => {
      if let Some(sel) = bridge.selection() {
        let focus = sel.focus_pos();
        let next = DocPosition::new(focus.node, 0);
        let anchor = if shift { sel.anchor_pos() } else { next };
        bridge.set_selection(DocSelection::from_positions(anchor, next));
      }
    }
    EditorKeyAction::End { shift } => {
      if let Some(sel) = bridge.selection() {
        let focus = sel.focus_pos();
        let end_off = bridge
          .block_at(focus.node)
          .map(|b| utf16_len(&b.text))
          .unwrap_or(0);
        let next = DocPosition::new(focus.node, end_off);
        let anchor = if shift { sel.anchor_pos() } else { next };
        bridge.set_selection(DocSelection::from_positions(anchor, next));
      }
    }
  }
}

fn move_caret_horizontal(bridge: &MicaDocumentBridge, dir: i32, shift: bool, word: bool) {
  let Some(sel) = bridge.selection() else {
    return;
  };
  if !shift && !sel.is_collapsed() {
    let target = if dir < 0 { sel.start_pos() } else { sel.end_pos() };
    bridge.set_selection(DocSelection::caret(target.node, target.offset));
    return;
  }
  let focus = sel.focus_pos();
  let bi = focus.node;
  let off = focus.offset;
  let count = bridge.block_len();
  let Some(block) = bridge.block_at(bi) else {
    return;
  };
  let len_u16 = utf16_len(&block.text);
  let next_pos = if dir < 0 {
    if off > 0 {
      let target_off = if word {
        prev_word_boundary(&block.text, off)
      } else {
        utf16_prev_boundary(&block.text, off)
      };
      DocPosition::new(bi, target_off)
    } else if bi > 0 {
      let prev_len = bridge
        .block_at(bi - 1)
        .map(|b| utf16_len(&b.text))
        .unwrap_or(0);
      DocPosition::new(bi - 1, prev_len)
    } else {
      focus
    }
  } else if off < len_u16 {
    let target_off = if word {
      next_word_boundary(&block.text, off)
    } else {
      utf16_next_boundary(&block.text, off)
    };
    DocPosition::new(bi, target_off)
  } else if bi + 1 < count {
    DocPosition::new(bi + 1, 0)
  } else {
    focus
  };
  let anchor = if shift { sel.anchor_pos() } else { next_pos };
  bridge.set_selection(DocSelection::from_positions(anchor, next_pos));
  if next_pos.node != bi {
    bridge.request_keep_block_visible(next_pos.node);
  }
}

fn prev_word_boundary(text: &str, u16_off: u32) -> u32 {
  let prefix = utf16_slice(text, 0, u16_off);
  let chars: Vec<(usize, char)> = prefix.char_indices().collect();
  let mut i = chars.len();
  while i > 0 && chars[i - 1].1.is_whitespace() {
    i -= 1;
  }
  while i > 0 && !chars[i - 1].1.is_whitespace() {
    i -= 1;
  }
  let byte_idx = if i < chars.len() { chars[i].0 } else { prefix.len() };
  utf16_len(&prefix[..byte_idx])
}

fn next_word_boundary(text: &str, u16_off: u32) -> u32 {
  let total = utf16_len(text);
  let suffix = utf16_slice(text, u16_off, total);
  let chars: Vec<(usize, char)> = suffix.char_indices().collect();
  let mut i = 0;
  while i < chars.len() && !chars[i].1.is_whitespace() {
    i += 1;
  }
  while i < chars.len() && chars[i].1.is_whitespace() {
    i += 1;
  }
  let byte_idx = if i < chars.len() { chars[i].0 } else { suffix.len() };
  u16_off + utf16_len(&suffix[..byte_idx])
}

pub(crate) fn close_slash(slash: SlashMenuSignals) {
  slash.open.set(false);
  slash.query.set(String::new());
  slash.index.set(0);
  slash.opened_by_btn.set(false);
}

pub(crate) fn update_slash_anchor(_bridge: &MicaDocumentBridge, slash: SlashMenuSignals) {
  let (box_w, _box_h) = slash.box_size.get_untracked();
  let left = ((box_w - SLASH_MENU_WIDTH) * 0.5)
    .clamp(16.0, (box_w - SLASH_MENU_WIDTH - 16.0).max(16.0));
  slash.anchor.set((left, 64.0));
}

pub(crate) fn toggle_block_type(bridge: &MicaDocumentBridge, kind: &str, level: Option<u64>) {
  let cur_kind = bridge.active_kind_signal().get_untracked();
  let cur_level = bridge.active_level_signal().get_untracked();
  let same_kind = match kind {
    "bulleted_list" => matches!(cur_kind.as_str(), "bulleted_list" | "bulletList"),
    "numbered_list" => matches!(cur_kind.as_str(), "numbered_list" | "orderedList"),
    "todo" => matches!(cur_kind.as_str(), "todo" | "taskItem"),
    "quote" => matches!(cur_kind.as_str(), "quote" | "blockquote"),
    other => cur_kind == other,
  };
  let already_same = same_kind && level.is_none_or(|lvl| cur_level == lvl);
  if already_same && kind != "paragraph" {
    bridge.set_block_type("paragraph", None);
  } else {
    bridge.set_block_type(kind, level);
  }
  bridge.request_focus();
}

pub(crate) fn sync_slash_palette_from_caret(
  bridge: &MicaDocumentBridge,
  slash: SlashMenuSignals,
  ai_enabled: Signal<bool>,
) {
  let Some(sel) = bridge.selection() else {
    if slash.open.get_untracked() && !slash.opened_by_btn.get_untracked() {
      close_slash(slash);
    }
    return;
  };
  if !sel.is_collapsed() {
    if slash.open.get_untracked() && !slash.opened_by_btn.get_untracked() {
      close_slash(slash);
    }
    return;
  }
  let focus = sel.focus_pos();
  if let Some(block) = bridge.block_at(focus.node)
    && !matches!(block.kind.as_str(), "code_block" | "table" | "image" | "divider" | "math_block")
  {
    let before = utf16_slice(&block.text, 0, focus.offset);
    if let Some((_, query)) = extract_slash_query(before) {
      let items = filter_slash_items(query, ai_enabled.get_untracked());
      if items.is_empty() {
        close_slash(slash);
        return;
      }
      slash.opened_by_btn.set(false);
      slash.query.set(query.to_owned());
      let cur = slash.index.get_untracked();
      if cur >= items.len() {
        slash.index.set(0);
      }
      update_slash_anchor(bridge, slash);
      slash.open.set(true);
      return;
    }
  }
  if slash.open.get_untracked() && !slash.opened_by_btn.get_untracked() {
    close_slash(slash);
  }
}

pub(crate) fn execute_slash_item(
  bridge: &MicaDocumentBridge,
  item: SlashItem,
  on_inline_ai: &Rc<dyn Fn(String)>,
  on_insert_image: &Rc<dyn Fn()>,
) {
  if item.kind == "__ai__" {
    bridge.replace_slash_token_and_apply("__ai__", None, None, None);
    on_inline_ai(String::new());
    return;
  }
  if item.kind == "image" {
    bridge.replace_slash_token_and_apply("__ai__", None, None, None);
    on_insert_image();
    return;
  }
  bridge.replace_slash_token_and_apply(item.kind, item.level, item.alert, item.template);
}

