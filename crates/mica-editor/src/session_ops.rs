//! Block transforms, indentation, LIS block moves, table operations, and
//! structured paste handlers for [`crate::EditorSession`].

use serde_json::{Map, Value, json};

use crate::commands::PasteInput;
use crate::diff_ops::{compute_move_blocks_target, diff_block_ops};
use crate::marks::{marks_from_data, marks_to_json, shift_marks};
use crate::model::{
  BlockSpec, DocOp, EditorBlock, MAX_NESTING_DEPTH, normalize_data_object,
};
use crate::paste::{
  html_nodes_to_markdown, looks_like_image_url, looks_like_url, normalize_line_endings,
  parse_paste_blocks, paste_merges_inline, pasted_formula_source, retag_mislabeled_fences,
  single_line_paste_inline, unwrap_nested_fences,
};
use crate::selection::{DocPosition, DocSelection, NormalizedSelection};
use crate::session::{DispatchCtx, EditorSession};
use crate::table::{TableModel, TableNavResult, TableOp};
use crate::utf16::{
  utf16_find_char_from, utf16_len, utf16_replace_range, utf16_rfind_char_before, utf16_slice,
};

impl EditorSession {
  // ── Indent / Outdent / Code line shifting ────────────────────────────────

  pub(crate) fn cmd_indent(&mut self, delta: i32, ctx: &mut DispatchCtx) {
    if delta == 0 || self.blocks.is_empty() {
      return;
    }
    let Some(sel) = self.selection.clone() else {
      return;
    };

    // Single-block selection inside a code block: Tab/Shift+Tab shifts code lines
    // (gh#169), except a collapsed Tab at offset 0 of a free-standing code block
    // that can attach to the preceding list item.
    if !sel.is_multi_node() {
      let idx = sel.focus_node();
      if idx < self.blocks.len() && self.blocks[idx].is_code() {
        let shift = delta < 0;
        let can_attach = sel.is_collapsed()
          && !shift
          && sel.focus_pos().offset == 0
          && self.can_attach_to_item(idx);
        if !can_attach {
          if sel.is_collapsed() && !shift {
            self.splice_range(&sel, "  ", &[], true, true, ctx);
            return;
          }
          if self.shift_code_lines(idx, &sel, delta, ctx) {
            return;
          }
          // Shift+Tab with nothing left to take off detaches an attached code block.
          if !(shift && self.blocks[idx].li_level().is_some()) {
            return;
          }
        }
      }
    }

    let (lo, hi) = match sel.normalized() {
      NormalizedSelection::BlockAtom { block_index }
      | NormalizedSelection::TableArea { block_index, .. } => (block_index, block_index),
      NormalizedSelection::Text { start, end } => (start.node, end.node),
    };
    let last = self.blocks.len() - 1;
    let lo = lo.min(last);
    let hi = hi.clamp(lo, last);

    for i in lo..=hi {
      if !self.blocks[i].is_list_kind() {
        if !EditorBlock::is_attachable_kind(&self.blocks[i].kind) {
          continue;
        }
        if delta > 0 && self.blocks[i].li_level().is_none() {
          if let Some(owner) = self.owning_item_indent_at(i) {
            self.blocks[i]
              .data_map_mut()
              .insert("li".to_string(), json!(owner));
            ctx.ops.push(Self::update_text_data_op(&self.blocks[i]));
          }
        } else if delta < 0 && self.blocks[i].li_level().is_some() {
          self.blocks[i].data_map_mut().remove("li");
          ctx.ops.push(Self::update_text_data_op(&self.blocks[i]));
        }
        continue;
      }

      let mut max_indent = 0u32;
      for p in (0..i).rev() {
        if self.blocks[p].is_list_kind() {
          max_indent = (self.blocks[p].indent() + 1).min(MAX_NESTING_DEPTH);
          break;
        }
        if self.blocks[p].li_level().is_some() {
          continue;
        }
        break;
      }
      let cur_indent = self.blocks[i].indent();
      let next = (cur_indent as i32 + delta).clamp(0, max_indent as i32) as u32;
      if next == cur_indent {
        continue;
      }
      if next > 0 {
        self.blocks[i]
          .data_map_mut()
          .insert("indent".to_string(), json!(next));
      } else {
        self.blocks[i].data_map_mut().remove("indent");
      }
      ctx.ops.push(Self::update_text_data_op(&self.blocks[i]));
    }
    if !ctx.ops.is_empty() {
      ctx.typing_block = None;
    }
  }

  fn owning_item_indent_at(&self, i: usize) -> Option<u32> {
    for p in (0..i).rev() {
      let n = &self.blocks[p];
      if n.is_list_kind() {
        return Some(n.indent());
      }
      if n.li_level().is_some() {
        continue;
      }
      return None;
    }
    None
  }

  fn can_attach_to_item(&self, i: usize) -> bool {
    let Some(node) = self.blocks.get(i) else {
      return false;
    };
    !node.is_list_kind()
      && node.li_level().is_none()
      && EditorBlock::is_attachable_kind(&node.kind)
      && self.owning_item_indent_at(i).is_some()
  }

  fn shift_code_lines(
    &mut self,
    idx: usize,
    sel: &DocSelection,
    delta: i32,
    ctx: &mut DispatchCtx,
  ) -> bool {
    let text = self.blocks[idx].text.clone();
    let len = utf16_len(&text);
    let s = sel.start_pos().offset.min(len);
    let mut e = sel.end_pos().offset.clamp(s, len);
    if e > s && utf16_slice(&text, e - 1, e) == "\n" {
      e -= 1;
    }

    let first_start = if s == 0 {
      0
    } else {
      utf16_rfind_char_before(&text, '\n', s - 1).map_or(0, |p| p + 1)
    };
    let mut starts = vec![first_start];
    let mut probe = first_start;
    while let Some(nl) = utf16_find_char_from(&text, '\n', probe) {
      if nl >= e {
        break;
      }
      starts.push(nl + 1);
      probe = nl + 1;
    }

    let mut edits: Vec<(u32, i32)> = Vec::new();
    let mut out = String::with_capacity(text.len() + starts.len() * 2);
    let mut cursor = 0u32;
    for &at in &starts {
      out.push_str(utf16_slice(&text, cursor, at));
      cursor = at;
      if delta > 0 {
        out.push_str("  ");
        edits.push((at, 2));
        continue;
      }
      let rest = utf16_slice(&text, at, len);
      let removed = if rest.starts_with('\t') {
        1u32
      } else {
        rest.chars().take(2).take_while(|&c| c == ' ').count() as u32
      };
      cursor += removed;
      if removed > 0 {
        edits.push((at, -(removed as i32)));
      }
    }
    if edits.is_empty() {
      return false;
    }
    out.push_str(utf16_slice(&text, cursor, len));

    let map_off = |o: u32| -> u32 {
      let mut d: i64 = 0;
      for &(at, n) in &edits {
        if n > 0 {
          if o >= at {
            d += n as i64;
          }
        } else {
          let rem = (-n) as u32;
          if o >= at + rem {
            d -= rem as i64;
          } else if o > at {
            d -= (o - at) as i64;
          }
        }
      }
      (o as i64 + d).max(0) as u32
    };

    let marks: Vec<_> = self.blocks[idx]
      .marks()
      .into_iter()
      .filter_map(|m| {
        let ms = map_off(m.start);
        let me = map_off(m.end);
        (me > ms).then(|| m.with_range(ms, me))
      })
      .collect();
    self.blocks[idx].text = out;
    self.blocks[idx].set_marks(&marks);
    ctx.ops.push(Self::update_text_data_op(&self.blocks[idx]));
    ctx.typing_block = None;

    let new_anchor = map_off(sel.anchor_pos().offset);
    let new_focus = map_off(sel.focus_pos().offset);
    self.selection = DocSelection::from_positions(
      DocPosition::new(idx, new_anchor),
      DocPosition::new(idx, new_focus),
    )
    .clamp_and_snap(&self.blocks);
    true
  }

  // ── Transform & Batch Transform ──────────────────────────────────────────

  pub(crate) fn cmd_transform_block(
    &mut self,
    kind: &str,
    data: Option<Value>,
    ctx: &mut DispatchCtx,
  ) {
    let Some(sel) = self.selection.clone() else {
      return;
    };
    let idx = sel.focus_node();
    if idx >= self.blocks.len() {
      return;
    }
    let existing_marks = self.blocks[idx].data_map().get("marks").cloned();
    let mut new_data = normalize_data_object(data.unwrap_or_else(|| json!({})));
    if EditorBlock::is_inline_text_kind(kind)
      && let Value::Object(m) = &mut new_data
      && !m.contains_key("marks")
      && let Some(marks) = existing_marks
    {
      m.insert("marks".to_string(), marks);
    }
    let b = &mut self.blocks[idx];
    b.kind = kind.to_string();
    b.data = new_data;
    ctx.ops.push(Self::update_full_op(b));
    ctx.typing_block = None;
    self.selection = self
      .selection
      .as_ref()
      .and_then(|s| s.clamp_and_snap(&self.blocks));
  }

  pub(crate) fn cmd_batch_transform(
    &mut self,
    kind: &str,
    data: Option<Value>,
    ctx: &mut DispatchCtx,
  ) {
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
    let last = self.blocks.len() - 1;
    let lo = lo.min(last);
    let hi = hi.clamp(lo, last);

    if kind == "code_block" {
      let mut texts = Vec::new();
      let mut first_idx = None;
      let mut remove_ids = Vec::new();
      for (i, blk) in self.blocks.iter().enumerate().take(hi + 1).skip(lo) {
        if blk.is_atomic() {
          continue;
        }
        if first_idx.is_none() {
          first_idx = Some(i);
        } else {
          remove_ids.push(blk.id.clone());
        }
        texts.push(blk.text.clone());
      }
      let Some(fi) = first_idx else {
        return;
      };
      let merged = texts.join("\n");
      let merged_len = utf16_len(&merged);
      let base_data = normalize_data_object(data.unwrap_or_else(|| json!({})));
      let b = &mut self.blocks[fi];
      b.kind = "code_block".to_string();
      b.text = merged;
      b.data = base_data;
      ctx.ops.push(Self::update_full_op(b));
      self.blocks.retain(|n| !remove_ids.contains(&n.id));
      for id in remove_ids {
        ctx.ops.push(DocOp::DeleteBlock { block_id: id });
      }
      ctx.typing_block = None;
      self.collapse_to(DocPosition::new(fi, merged_len));
      return;
    }

    let extra = normalize_data_object(data.unwrap_or_else(|| json!({})));
    for b in self.blocks.iter_mut().take(hi + 1).skip(lo) {
      if b.is_atomic() || b.is_code() {
        continue;
      }
      let mut nd = Map::new();
      if let Some(marks) = b.data_map().get("marks") {
        nd.insert("marks".to_string(), marks.clone());
      }
      if let Value::Object(extra_map) = &extra {
        for (k, v) in extra_map {
          nd.insert(k.clone(), v.clone());
        }
      }
      b.kind = kind.to_string();
      b.data = Value::Object(nd);
      ctx.ops.push(Self::update_full_op(b));
    }
    if !ctx.ops.is_empty() {
      ctx.typing_block = None;
    }
  }

  // ── MoveBlocks (LIS minimal move_block ops) ──────────────────────────────

  pub(crate) fn cmd_move_blocks(
    &mut self,
    block_ids: &[String],
    target_index: usize,
    ctx: &mut DispatchCtx,
  ) {
    let Some(reordered) = compute_move_blocks_target(&self.blocks, block_ids, target_index) else {
      return;
    };
    let ops = diff_block_ops(&self.root_block_id, &self.blocks, &reordered);
    if ops.is_empty() {
      return;
    }
    self.blocks = reordered;
    ctx.ops = ops;
    ctx.typing_block = None;
    if let Some(first_moved_id) = block_ids.first()
      && let Some(new_idx) = self.blocks.iter().position(|b| &b.id == first_moved_id)
    {
      if self.blocks[new_idx].is_atomic() {
        self.selection = Some(DocSelection::block_atom(new_idx));
      } else {
        self.collapse_to(DocPosition::new(new_idx, 0));
      }
    }
  }

  // ── Table operations ─────────────────────────────────────────────────────

  pub(crate) fn cmd_table_op(&mut self, block_index: usize, op: TableOp, ctx: &mut DispatchCtx) {
    if block_index >= self.blocks.len() || self.blocks[block_index].kind != "table" {
      return;
    }
    let mut table = TableModel::from_block_data(&self.blocks[block_index].data);
    let mut mutated = false;

    match op {
      TableOp::SetCell { row, col, cell } => {
        mutated = table.set_cell(row, col, cell);
      }
      TableOp::ClearCells { r0, c0, r1, c1 } => {
        mutated = table.clear_cells(r0, c0, r1, c1);
      }
      TableOp::InsertRow { at } => {
        table.insert_row(at);
        mutated = true;
      }
      TableOp::DeleteRow { at } => {
        mutated = table.delete_row(at);
      }
      TableOp::MoveRow { row, delta } => {
        mutated = table.move_row(row, delta);
      }
      TableOp::InsertColumn { at } => {
        table.insert_column(at);
        mutated = true;
      }
      TableOp::DeleteColumn { at } => {
        mutated = table.delete_column(at);
      }
      TableOp::MoveColumn { col, delta } => {
        mutated = table.move_column(col, delta);
      }
      TableOp::SetAlign { align } => {
        table.set_align(&align);
        mutated = true;
      }
      TableOp::SetColumnAlign { col, align } => {
        mutated = table.set_column_align(col, &align);
      }
      TableOp::SetColumnWidths { widths } => {
        table.set_column_widths(&widths);
        mutated = true;
      }
      TableOp::ResetColumnWidths => {
        table.reset_column_widths();
        mutated = true;
      }
      TableOp::SetTableWidth { width } => {
        table.set_table_width(width);
        mutated = true;
      }
      TableOp::Navigate {
        row,
        col,
        direction,
      } => {
        match table.navigate(row, col, direction) {
          TableNavResult::Cell { row: nr, col: nc } => {
            self.selection = Some(DocSelection::table_cell(block_index, nr, nc));
          }
          TableNavResult::InsertRowAndFocus { row: nr, col: nc } => {
            table.insert_row(nr);
            mutated = true;
            self.selection = Some(DocSelection::table_cell(block_index, nr, nc));
          }
          TableNavResult::ExitBefore => {
            if block_index > 0 {
              let prev = block_index - 1;
              self.collapse_to(DocPosition::new(prev, self.blocks[prev].utf16_len()));
            }
          }
          TableNavResult::ExitAfter => {
            if block_index + 1 < self.blocks.len() {
              self.collapse_to(DocPosition::new(block_index + 1, 0));
            } else {
              let created = EditorBlock::new(self.gen_block_id(), "paragraph", "");
              let insert_at = block_index + 1;
              ctx.ops.push(self.insert_op(&created, insert_at));
              self.blocks.insert(insert_at, created);
              self.collapse_to(DocPosition::new(insert_at, 0));
              ctx.typing_block = None;
            }
          }
        }
      }
      TableOp::CopyArea { r0, c0, r1, c1 } => {
        ctx.clipboard_out = Some(table.copy_area(r0, c0, r1, c1));
      }
      TableOp::CopyWhole => {
        ctx.clipboard_out = Some(table.copy_whole());
      }
    }

    if mutated {
      let mut next_data = self.blocks[block_index].data_map().clone();
      next_data.remove("aligns");
      next_data.remove("width");
      if let Value::Object(encoded) = table.to_block_data() {
        for (k, v) in encoded {
          next_data.insert(k, v);
        }
      }
      self.blocks[block_index].data = Value::Object(next_data);
      ctx.ops.push(DocOp::UpdateBlock {
        block_id: self.blocks[block_index].id.clone(),
        kind: None,
        text: None,
        data: Some(self.blocks[block_index].data.clone()),
      });
      ctx.typing_block = None;
    }
  }

  // ── Structured Paste ─────────────────────────────────────────────────────

  pub(crate) fn cmd_paste(&mut self, input: PasteInput, ctx: &mut DispatchCtx) {
    self.prose_input.reset();
    if self.blocks.is_empty() {
      self.ensure_not_empty(ctx);
    }

    match input {
      PasteInput::PlainLiteral { text } => {
        let normalized = normalize_line_endings(&text);
        if normalized.is_empty() {
          return;
        }
        let Some(sel) = self.selection.clone() else {
          return;
        };
        let idx = sel.focus_node();
        if idx < self.blocks.len() && self.blocks[idx].is_code() && !sel.is_multi_node() {
          self.splice_range(&sel, &normalized, &[], false, false, ctx);
          return;
        }
        if !normalized.contains('\n') {
          self.insert_text_at_caret(&normalized, ctx);
        } else {
          let specs: Vec<BlockSpec> = normalized
            .split('\n')
            .map(|line| BlockSpec::new("paragraph", line, json!({})))
            .collect();
          self.insert_blocks_replacing_selection(&specs, ctx);
        }
      }
      PasteInput::Plain { text } => {
        let plain = normalize_line_endings(&text);
        self.paste_markdown_or_plain(&plain, &plain, false, ctx);
      }
      PasteInput::Markdown {
        markdown,
        plain,
        rich,
      } => {
        let md = normalize_line_endings(&markdown);
        let pl = normalize_line_endings(&plain);
        self.paste_markdown_or_plain(&md, if pl.is_empty() { &md } else { &pl }, rich, ctx);
      }
      PasteInput::Html {
        html_node_json,
        plain,
      } => {
        let node = mica_md::HtmlNode::from_json(&html_node_json);
        let md = normalize_line_endings(&html_nodes_to_markdown(&node));
        let pl = normalize_line_endings(&plain);
        if md.is_empty() && !pl.is_empty() {
          self.paste_markdown_or_plain(&pl, &pl, false, ctx);
        } else {
          self.paste_markdown_or_plain(&md, &pl, true, ctx);
        }
      }
    }
  }

  fn paste_markdown_or_plain(
    &mut self,
    raw_markdown: &str,
    plain: &str,
    rich: bool,
    ctx: &mut DispatchCtx,
  ) {
    let markdown = retag_mislabeled_fences(&unwrap_nested_fences(raw_markdown));
    let Some(sel) = self.selection.clone() else {
      return;
    };
    let idx = sel.focus_node();
    if idx >= self.blocks.len() {
      return;
    }
    let node_is_code = self.blocks[idx].is_code();
    let node_is_atomic = self.blocks[idx].is_atomic();
    let node_is_table = self.blocks[idx].kind == "table";

    // Inside a single code block: paste verbatim preserving newlines.
    if node_is_code && !sel.is_multi_node() {
      let raw = if !plain.is_empty() { plain } else { &markdown };
      if !raw.is_empty() {
        self.splice_range(&sel, raw, &[], false, false, ctx);
      }
      return;
    }

    let trimmed = plain.trim();

    // Bare image URL → insert image block directly.
    if !rich && looks_like_image_url(trimmed) {
      self.cmd_insert_image(None, Some(trimmed.to_string()), None, "", None, ctx);
      return;
    }

    // Bare URL → link selected text (Notion-style) or insert linked URL at caret.
    if !rich && looks_like_url(trimmed) && !node_is_atomic && !node_is_table {
      if !sel.is_collapsed() && !sel.is_multi_node() {
        self.cmd_set_link(
          idx,
          sel.start_pos().offset,
          sel.end_pos().offset,
          Some(trimmed),
          None,
          ctx,
        );
      } else {
        if !sel.is_collapsed() {
          self.delete_current_selection(ctx);
        }
        let cur = self.selection.as_ref().map_or(DocPosition::new(idx, 0), DocSelection::focus_pos);
        self.cmd_set_link(cur.node, cur.offset, cur.offset, Some(trimmed), Some(trimmed), ctx);
      }
      return;
    }

    // Single-line display formula (`$$…$$` or `\[…\]`) → `math_block`.
    if !node_is_atomic
      && !node_is_table
      && let Some(src) = pasted_formula_source(trimmed)
    {
      self.insert_blocks_after_focus(&[BlockSpec::new("math_block", src, json!({}))], ctx);
      return;
    }

    // Single plain line with trusted inline marks (gh#185) or inline math.
    if !rich && !node_is_atomic && !node_is_code && !node_is_table && !plain.contains('\n') {
      if let Some((text, marks)) = single_line_paste_inline(plain) {
        self.splice_range(&sel, &text, &marks, false, false, ctx);
        return;
      }
      // Literal single plain line.
      if !plain.is_empty() {
        self.insert_text_at_caret(plain, ctx);
      }
      return;
    }

    if markdown.trim().is_empty() {
      if !plain.is_empty() {
        self.insert_text_at_caret(plain, ctx);
      }
      return;
    }

    // Single-line rich fragment weaves inline if `paste_merges_inline` permits.
    if rich && !markdown.contains('\n') && !node_is_atomic && !node_is_code && !node_is_table {
      let specs = parse_paste_blocks(&markdown);
      if specs.len() == 1 && !sel.is_multi_node() {
        let from = sel.start_pos().offset;
        let to = sel.end_pos().offset;
        let spec = &specs[0];
        if paste_merges_inline(
          &spec.kind,
          &self.blocks[idx].kind,
          self.blocks[idx].utf16_len(),
          from,
          to,
        ) {
          let marks = marks_from_data(&spec.data);
          self.splice_range(&sel, &spec.text, &marks, false, false, ctx);
          return;
        }
      }
    }

    // Multi-line or block-level paste via `mica_md::parse_body`.
    if rich || markdown.contains('\n') {
      let specs = parse_paste_blocks(&markdown);
      if !specs.is_empty() {
        self.insert_blocks_replacing_selection(&specs, ctx);
      }
    } else if !plain.is_empty() {
      self.insert_text_at_caret(plain, ctx);
    }
  }

  fn insert_text_at_caret(&mut self, text: &str, ctx: &mut DispatchCtx) {
    let Some(sel) = self.selection.clone() else {
      return;
    };
    let idx = sel.focus_node();
    if idx >= self.blocks.len() {
      return;
    }
    if self.blocks[idx].is_atomic() || self.blocks[idx].kind == "table" {
      self.insert_blocks_after_focus(&[BlockSpec::new("paragraph", text, json!({}))], ctx);
      return;
    }
    self.splice_range(&sel, text, &[], false, false, ctx);
  }

  fn insert_blocks_after_focus(&mut self, specs: &[BlockSpec], ctx: &mut DispatchCtx) {
    if specs.is_empty() {
      return;
    }
    let focus_idx = self
      .selection
      .as_ref()
      .map_or_else(|| self.blocks.len().saturating_sub(1), DocSelection::focus_node)
      .min(self.blocks.len().saturating_sub(1));

    let mut first = 0usize;
    if focus_idx < self.blocks.len()
      && self.blocks[focus_idx].text.is_empty()
      && self.blocks[focus_idx].kind == "paragraph"
    {
      let spec = &specs[0];
      let b = &mut self.blocks[focus_idx];
      b.kind = spec.kind.clone();
      b.text = spec.text.clone();
      b.data = normalize_data_object(spec.data.clone());
      ctx.ops.push(Self::update_full_op(b));
      first = 1;
    }

    let mut insert_at = if self.blocks.is_empty() {
      0
    } else {
      focus_idx + 1
    };
    for spec in &specs[first..] {
      let created = EditorBlock::new(self.gen_block_id(), &spec.kind, &spec.text)
        .with_data(spec.data.clone());
      ctx.ops.push(self.insert_op(&created, insert_at));
      self.blocks.insert(insert_at, created);
      insert_at += 1;
    }
    ctx.typing_block = None;
    let last = insert_at.saturating_sub(1).min(self.blocks.len() - 1);
    self.collapse_to(DocPosition::new(last, self.blocks[last].utf16_len()));
  }

  fn insert_blocks_replacing_selection(&mut self, specs: &[BlockSpec], ctx: &mut DispatchCtx) {
    if specs.is_empty() {
      return;
    }
    if let Some(sel) = self.selection.clone()
      && !sel.is_collapsed()
    {
      self.delete_current_selection(ctx);
      if let Some(after_sel) = &self.selection
        && let Some(leftover) = self.blocks.get(after_sel.focus_node())
        && leftover.text.is_empty()
      {
        self.replace_focused_with_blocks(specs, ctx);
        return;
      }
    }
    self.insert_blocks_after_focus(specs, ctx);
  }

  // ── Auxiliary block commands ─────────────────────────────────────────────

  pub(crate) fn cmd_toggle_todo(&mut self, block_index: usize, ctx: &mut DispatchCtx) {
    if block_index >= self.blocks.len() || self.blocks[block_index].kind != "todo" {
      return;
    }
    let next = !self.blocks[block_index].todo_checked();
    self.blocks[block_index]
      .data_map_mut()
      .insert("checked".to_string(), Value::Bool(next));
    ctx.ops.push(Self::update_text_data_op(&self.blocks[block_index]));
    ctx.typing_block = None;
  }

  pub(crate) fn cmd_apply_slash_command(
    &mut self,
    start: u32,
    end: u32,
    kind: &str,
    data: Value,
    ctx: &mut DispatchCtx,
  ) {
    let Some(sel) = self.selection.clone() else {
      return;
    };
    let idx = sel.focus_node();
    if idx >= self.blocks.len() {
      return;
    }
    let old_len = self.blocks[idx].utf16_len();
    let (new_text, s, e) = utf16_replace_range(&self.blocks[idx].text, start, end, "");
    let new_len = utf16_len(&new_text);
    let shifted_marks = shift_marks(
      &self.blocks[idx].marks(),
      s,
      e,
      new_len as i64 - old_len as i64,
      new_len,
      true,
    );
    let mut next_data = normalize_data_object(data);
    if EditorBlock::is_inline_text_kind(kind)
      && !shifted_marks.is_empty()
      && let Value::Object(m) = &mut next_data
    {
      m.insert("marks".to_string(), marks_to_json(&shifted_marks));
    }
    let b = &mut self.blocks[idx];
    b.kind = kind.to_string();
    b.text = new_text;
    b.data = next_data;
    ctx.ops.push(Self::update_full_op(b));
    ctx.typing_block = None;

    if self.slash_open {
      self.slash_open = false;
      ctx.effects.push(crate::model::EditorEffect::CloseSlashMenu);
    }

    if self.blocks[idx].is_atomic() {
      let after = idx + 1;
      if after >= self.blocks.len() || self.blocks[after].is_atomic() {
        let p = EditorBlock::new(self.gen_block_id(), "paragraph", "");
        ctx.ops.push(self.insert_op(&p, after));
        self.blocks.insert(after, p);
      }
      self.collapse_to(DocPosition::new(after, 0));
    } else {
      self.collapse_to(DocPosition::new(idx, s));
    }
  }

  pub(crate) fn cmd_set_code_language(
    &mut self,
    block_index: usize,
    language: Option<&str>,
    ctx: &mut DispatchCtx,
  ) {
    if block_index >= self.blocks.len() || self.blocks[block_index].kind != "code_block" {
      return;
    }
    match language.map(str::trim) {
      None | Some("" | "auto") => {
        self.blocks[block_index].data_map_mut().remove("language");
      }
      Some(lang) => {
        self.blocks[block_index]
          .data_map_mut()
          .insert("language".to_string(), Value::String(lang.to_string()));
      }
    }
    ctx.ops.push(Self::update_text_data_op(&self.blocks[block_index]));
    ctx.typing_block = None;
  }

  pub(crate) fn cmd_set_inline_math(
    &mut self,
    block_index: usize,
    start: u32,
    end: u32,
    new_source: &str,
    ctx: &mut DispatchCtx,
  ) {
    if block_index >= self.blocks.len() {
      return;
    }
    let b = &self.blocks[block_index];
    let old_len = b.utf16_len();
    if start >= end || end > old_len {
      return;
    }
    // Validate the math run still covers `[start, end)` exactly (guards against stale dialogs).
    let has_run = b
      .marks()
      .iter()
      .any(|m| m.kind == "math" && m.start == start && m.end == end);
    if !has_run {
      return;
    }
    let (new_text, s, e) = utf16_replace_range(&b.text, start, end, new_source);
    let new_len = utf16_len(&new_text);
    let shifted = shift_marks(
      &b.marks(),
      s,
      e,
      new_len as i64 - old_len as i64,
      new_len,
      true,
    );
    let bm = &mut self.blocks[block_index];
    bm.text = new_text;
    bm.set_marks(&shifted);
    ctx.ops.push(Self::update_text_data_op(bm));
    ctx.typing_block = None;
    let caret = if new_source.is_empty() {
      s
    } else {
      s + utf16_len(new_source)
    };
    self.collapse_to(DocPosition::new(block_index, caret));
  }

  pub(crate) fn cmd_insert_divider(&mut self, ctx: &mut DispatchCtx) {
    if self.blocks.is_empty() {
      self.ensure_not_empty(ctx);
    }
    let idx = self
      .selection
      .as_ref()
      .map_or(0, DocSelection::focus_node)
      .min(self.blocks.len() - 1);

    let div_idx = if self.blocks[idx].text.is_empty() && self.blocks[idx].kind == "paragraph" {
      let b = &mut self.blocks[idx];
      b.kind = "divider".to_string();
      b.text.clear();
      b.data = json!({});
      ctx.ops.push(Self::update_full_op(b));
      idx
    } else {
      let d = EditorBlock::new(self.gen_block_id(), "divider", "");
      ctx.ops.push(self.insert_op(&d, idx + 1));
      self.blocks.insert(idx + 1, d);
      idx + 1
    };

    let after = div_idx + 1;
    if after >= self.blocks.len() || self.blocks[after].is_atomic() {
      let p = EditorBlock::new(self.gen_block_id(), "paragraph", "");
      ctx.ops.push(self.insert_op(&p, after));
      self.blocks.insert(after, p);
    }
    ctx.typing_block = None;
    self.collapse_to(DocPosition::new(after, 0));
  }

  pub(crate) fn cmd_insert_image(
    &mut self,
    file_id: Option<String>,
    url: Option<String>,
    name: Option<String>,
    alt: &str,
    align: Option<String>,
    ctx: &mut DispatchCtx,
  ) {
    if file_id.is_none() && url.is_none() {
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

    let mut data = Map::new();
    if let Some(fid) = &file_id {
      data.insert("file_id".to_string(), Value::String(fid.clone()));
    }
    if let Some(u) = &url {
      data.insert("url".to_string(), Value::String(u.clone()));
    }
    let resolved_name = name.unwrap_or_else(|| {
      url
        .as_deref()
        .and_then(|u| u.rsplit('/').next())
        .unwrap_or("")
        .to_string()
    });
    data.insert("name".to_string(), Value::String(resolved_name));
    if let Some(a) = align {
      data.insert("align".to_string(), Value::String(a));
    }
    let data_val = Value::Object(data);

    let img_idx = if self.blocks[idx].text.is_empty() && self.blocks[idx].kind == "paragraph" {
      let b = &mut self.blocks[idx];
      b.kind = "image".to_string();
      b.text = alt.to_string();
      b.data = data_val;
      ctx.ops.push(Self::update_full_op(b));
      idx
    } else {
      let img = EditorBlock::new(self.gen_block_id(), "image", alt).with_data(data_val);
      ctx.ops.push(self.insert_op(&img, idx + 1));
      self.blocks.insert(idx + 1, img);
      idx + 1
    };

    let after = img_idx + 1;
    if after >= self.blocks.len() || self.blocks[after].is_atomic() {
      let p = EditorBlock::new(self.gen_block_id(), "paragraph", "");
      ctx.ops.push(self.insert_op(&p, after));
      self.blocks.insert(after, p);
    }
    ctx.typing_block = None;
    self.collapse_to(DocPosition::new(after, 0));
  }
}
