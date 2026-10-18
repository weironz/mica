//! Handler functions for the block scanner — the decomposition of what used
//! to be `import_markdown_inner`'s 1,300-line loop body
//! (docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §8, stage 2).
//!
//! The families preserve the original loop's parsing decisions and address
//! `ctx.st.<field>` through the shared [ScanCtx].
//! A handler returns `Some(())` when it consumed the line (the loop
//! `continue`s); `None` means "not mine — fall through to the next family",
//! preserving the loop's original ordering, which IS the precedence order.
//! `list_item` instead uses `ControlFlow` to return an unconsumed classification
//! by ownership; the remaining families need no full text/data copy.
//!
//! The golden-master test (tests/golden_master.rs) pins all of this
//! byte-for-byte: 741 corpus inputs through both entry points.

use std::ops::ControlFlow;

use serde_json::{Value, json};

use crate::block::{
  BlockState, OpenItem, alert_type_of, classify_markdown_line, column_of,
  data_insert, deindent_columns, fence_close, fence_open, heading_prefix_level,
  html_block_ends, html_block_start, is_divider, is_table_separator, leading_columns, parse_footnote_def, push_block, quote_depth_of, setext_level,
  split_table_row, strip_quote_markers, strip_atx_closing,
};
use crate::export_md::encode_table_cell;
use crate::html::{li_of, list_indent, list_tag_for};
use crate::inline::{RefDefs, apply_inline_marks, parse_inline_with, unescape_md};
use crate::Block;

/// What the scan loop and its handlers share per invocation: the document
/// being scanned and the two accumulators. `index` is passed by &mut.
/// The classify result carried between families.
pub(crate) struct ClassifiedLine {
  pub(crate) kind: &'static str,
  pub(crate) text: String,
  pub(crate) data: Value,
}

pub(crate) struct ScanCtx<'a> {
  pub(crate) lines: &'a [&'a str],
  pub(crate) blocks: &'a mut Vec<Block>,
  pub(crate) root_children: &'a mut Vec<String>,
  pub(crate) defs: &'a RefDefs,
  pub(crate) index: &'a mut usize,
  pub(crate) st: &'a mut BlockState,
}

impl<'a> ScanCtx<'a> {
  /// Push a block, appending its id to the root's children.
  pub(crate) fn push(&mut self, kind: &str, text: impl Into<String>, data: Value) {
    push_block(self.blocks, self.root_children, kind, text.into(), data);
  }
}

/// The line is blank: keep the list context (items note the blank for loose
/// marking), end the open paragraph, flush any armed content-less `>` group
/// as an empty quote block, and arm the quote boundary.
///
/// Blank lines between items keep the list context, end the open paragraph
/// (items merely note the blank — an indented line can still continue
/// them), and make the list loose if it continues.
pub(crate) fn blank_line(ctx: &mut ScanCtx) {
  match ctx.st.open_item.as_mut() {
    Some(open) if matches!(open.kind.as_str(), "bulleted_list" | "numbered_list" | "todo") => {
      open.had_blank = true
    }
    _ => ctx.st.open_item = None,
  }
  ctx.st.pending_loose = ctx.st.pending_loose || !ctx.st.list_stack.is_empty();
  // A content-less `>` group becomes an empty quote block; a blank after a
  // quote separates blockquotes (the next gets `qbreak`).
  if let Some(d) = ctx.st.pending_empty_quote.take() {
    let mut data = if d > 1 { json!({ "quote": d }) } else { Value::Null };
    if ctx.st.quote_boundary {
      data_insert(&mut data, "qbreak", json!(true));
    }
    if let Some(a) = ctx.st.pending_alert.take() {
      data_insert(&mut data, "alert", json!(a));
    }
    ctx.push("quote", String::new(), data);
  }
  if ctx.blocks.last().is_some_and(|b| quote_depth_of(b) > 0) {
    ctx.st.quote_boundary = true;
  }
  ctx.st.quote_active = false;
  *ctx.index += 1;
}
/// (family moved from the scan loop)
pub(crate) fn lazy_code_continuation(ctx: &mut ScanCtx, _line: &str, _content: &str, body: &str, col: usize) -> Option<()> {
    // A 4+-column line cannot start a new block while a top-level
    // paragraph-like block is open — it is lazy continuation text.
    if col >= 4
      && ctx.st.list_stack.is_empty()
      && let Some(open) = ctx.st.open_item.as_mut()
      && !open.had_blank
    {
      open.raw.push('\n');
      open.raw.push_str(body);
      defer_inline(ctx.blocks, &mut ctx.st.stale_inline, open.block_idx, &open.raw);
      (*ctx.index) += 1;
      return Some(());
    }
  None // fall through: the family guard rejected the line
}

/// A continuation line joined an open block: store the RAW joined source and
/// leave the inline parse for [finish_deferred_inline], once, after the scan.
///
/// Re-parsing the whole block on every continuation line made an n-line
/// paragraph cost n full inline parses — quadratic in the paragraph, which
/// on top of a quadratic inline scanner made a plain 4000-line paragraph take
/// seconds. Any later handler that writes this block's text itself (setext,
/// loose re-join, …) computes it from `open.raw` and clears the entry.
fn defer_inline(
  blocks: &mut [Block],
  stale: &mut std::collections::BTreeSet<usize>,
  idx: usize,
  raw: &str,
) {
  blocks[idx].text.clear();
  blocks[idx].text.push_str(raw);
  stale.insert(idx);
}

/// Parse the inlines of every block [defer_inline] left raw. Its base data is
/// the block's CURRENT data minus the stale `marks` — what the per-line
/// re-parse rebuilt from, including any key a handler set directly on the
/// block after its last continuation.
pub(crate) fn finish_deferred_inline(
  blocks: &mut [Block],
  stale: std::collections::BTreeSet<usize>,
  defs: &RefDefs,
) {
  for idx in stale {
    let block = &mut blocks[idx];
    let mut base = std::mem::take(&mut block.data);
    if let Value::Object(map) = &mut base {
      map.remove("marks");
      if map.is_empty() {
        base = Value::Null;
      }
    }
    let (text, data) = apply_inline_marks(std::mem::take(&mut block.text), base, defs);
    block.text = text;
    block.data = data;
  }
}

/// (family moved from the scan loop)
pub(crate) fn setext_paragraph(ctx: &mut ScanCtx, line: &str, _content: &str, _body: &str, _col: usize) -> Option<()> {
    // Setext underline of the open (possibly multi-line) paragraph: the
    // whole continued paragraph becomes the heading.
    if let Some(open) = &ctx.st.open_item
      && open.kind == "paragraph"
      && let Some(level) = setext_level(line)
    {
      let (text, data) = apply_inline_marks(open.raw.clone(), json!({ "level": level }), ctx.defs);
      let idx = open.block_idx;
      ctx.st.stale_inline.remove(&idx);
      ctx.blocks[idx].kind = "heading".to_string();
      ctx.blocks[idx].text = text;
      ctx.blocks[idx].data = data;
      ctx.st.open_item = None;
      (*ctx.index) += 1;
      return Some(());
    }
  None // fall through: the family guard rejected the line
}

/// (family moved from the scan loop)
pub(crate) fn list_container_children(ctx: &mut ScanCtx, _line: &str, content: &str, body: &str, col: usize) -> Option<()> {
    // List-item container children: at or past the open item's content
    // column, fences / indented code / dividers (and, below, quotes and
    // paragraphs) belong INSIDE the item — flat ctx.blocks carrying `data.li`.
    if let Some(&cc) = ctx.st.list_stack.last()
      && col >= cc
      && strip_quote_markers(content).0 == 0
      && !matches!(
        classify_markdown_line(content).0,
        "bulleted_list" | "numbered_list" | "todo"
      )
    {
      let level = ctx.st.list_stack.len() - 1;
      // Fenced code child.
      if let Some((fence_char, fence_len, info)) = fence_open(content) {
        let language =
          unescape_md(info.split_whitespace().next().unwrap_or_default());
        let mut code_lines = Vec::new();
        (*ctx.index) += 1;
        while *ctx.index < ctx.lines.len() {
          let l = ctx.lines[*ctx.index];
          let lt = l.trim_start();
          if lt.is_empty() {
            code_lines.push(String::new());
            (*ctx.index) += 1;
            continue;
          }
          let lcol = leading_columns(l);
          if lcol < cc {
            break; // the item ended — so does the fence
          }
          if fence_close(lt, fence_char, fence_len) {
            (*ctx.index) += 1;
            break;
          }
          // Content sheds up to the OPENING fence's indentation.
          code_lines.push(deindent_columns(l, col));
          (*ctx.index) += 1;
        }
        // Trailing blank-only lines belong between ctx.blocks, not the code.
        while code_lines.last().is_some_and(|l| l.is_empty()) {
          code_lines.pop();
        }
        let mut data = if language.is_empty() {
          json!({})
        } else {
          json!({ "language": language })
        };
        data_insert(&mut data, "li", json!(level));
        if ctx.st.pending_loose {
          if let Some((prev_idx, _, _)) = ctx.st.last_list {
            data_insert(&mut ctx.blocks[prev_idx].data, "loose", json!(true));
          }
          ctx.st.pending_loose = false;
        }
        push_block(ctx.blocks, ctx.root_children, "code_block", code_lines.join("\n"), data);
        ctx.st.item_children = true;
        ctx.st.open_item = None;
        return Some(());
      }
      // Indented code child (4+ columns past the content column).
      if col >= cc + 4 {
        let mut code_lines: Vec<String> = Vec::new();
        let mut blanks: Vec<String> = Vec::new();
        while *ctx.index < ctx.lines.len() {
          let l = ctx.lines[*ctx.index];
          if l.trim().is_empty() {
            blanks.push(deindent_columns(l, cc + 4));
            (*ctx.index) += 1;
            continue;
          }
          if leading_columns(l) < cc + 4 {
            break;
          }
          code_lines.append(&mut blanks);
          code_lines.push(deindent_columns(l, cc + 4));
          (*ctx.index) += 1;
        }
        let data = json!({ "li": level });
        if ctx.st.pending_loose {
          if let Some((prev_idx, _, _)) = ctx.st.last_list {
            data_insert(&mut ctx.blocks[prev_idx].data, "loose", json!(true));
          }
          ctx.st.pending_loose = false;
        }
        push_block(ctx.blocks, ctx.root_children, "code_block", code_lines.join("\n"), data);
        ctx.st.item_children = true;
        ctx.st.open_item = None;
        return Some(());
      }
      // Setext underline of the OPEN item's text: the text becomes a
      // heading CHILD (`- Bar\n  ---` → <li><h2>Bar</h2>, spec ex. 300).
      // Checked before the divider — with an open texted item, `---` is
      // an underline, not an <hr> (same precedence as at top level).
      if let Some(hl) = setext_level(content)
        && let Some(open) = ctx.st.open_item.as_ref()
        && matches!(open.kind.as_str(), "bulleted_list" | "numbered_list" | "todo")
        && !open.raw.is_empty()
        && !open.had_blank
      {
        let raw_text = open.raw.clone();
        let idx = open.block_idx;
        ctx.st.stale_inline.remove(&idx);
        let (it, id) = apply_inline_marks(String::new(), open.base.clone(), ctx.defs);
        ctx.blocks[idx].text = it;
        ctx.blocks[idx].data = id;
        let (ht, mut hd) = apply_inline_marks(raw_text, json!({ "level": hl }), ctx.defs);
        data_insert(&mut hd, "li", json!(level));
        push_block(ctx.blocks, ctx.root_children, "heading", ht, hd);
        ctx.st.item_children = true;
        ctx.st.open_item = None;
        (*ctx.index) += 1;
        return Some(());
      }
      // Divider child.
      if is_divider(content) {
        let data = json!({ "li": level });
        if ctx.st.pending_loose {
          if let Some((prev_idx, _, _)) = ctx.st.last_list {
            data_insert(&mut ctx.blocks[prev_idx].data, "loose", json!(true));
          }
          ctx.st.pending_loose = false;
        }
        push_block(ctx.blocks, ctx.root_children, "divider", String::new(), data);
        ctx.st.item_children = true;
        ctx.st.open_item = None;
        (*ctx.index) += 1;
        return Some(());
      }
      // Paragraph child: once the item holds children, indented paragraph
      // lines become child ctx.blocks (order matters — text renders first).
      if ctx.st.item_children
        && classify_markdown_line(content).0 == "paragraph"
        && ctx.st.open_item.is_none()
      {
        let data = json!({ "li": level });
        if ctx.st.pending_loose {
          if let Some((prev_idx, _, _)) = ctx.st.last_list {
            data_insert(&mut ctx.blocks[prev_idx].data, "loose", json!(true));
          }
          ctx.st.pending_loose = false;
        }
        let base = data.clone();
        let (text2, data2) = apply_inline_marks(content.to_string(), data, ctx.defs);
        push_block(ctx.blocks, ctx.root_children, "paragraph", text2, data2);
        ctx.st.open_item = Some(OpenItem {
          block_idx: ctx.blocks.len() - 1,
          kind: "paragraph".to_string(),
          content_col: cc,
          raw: body.to_string(),
          base,
          had_blank: false,
          qdepth: 0,
        });
        (*ctx.index) += 1;
        return Some(());
      }
      // Anything else falls through to the regular machinery.
    }
  None // fall through: the family guard rejected the line
}

/// (family moved from the scan loop)
pub(crate) fn fenced_code(ctx: &mut ScanCtx, _line: &str, content: &str, _body: &str, col: usize) -> Option<()> {
    if col < 4
      && let Some((fence_char, fence_len, info)) = fence_open(content)
    {
      ctx.st.list_stack.clear();
      ctx.st.open_item = None;
      ctx.st.last_list = None;
      ctx.st.pending_loose = false;
      // Only the first word of the info string is the language.
      let language =
        unescape_md(info.split_whitespace().next().unwrap_or_default());
      let mut code_lines = Vec::new();
      (*ctx.index) += 1;
      while *ctx.index < ctx.lines.len() {
        let l = ctx.lines[*ctx.index];
        let lt = l.trim_start();
        let lcol = l.len() - lt.len();
        if lcol < 4 && fence_close(lt, fence_char, fence_len) {
          (*ctx.index) += 1;
          break;
        }
        // Content lines shed up to the opening fence's indentation.
        code_lines.push(if col > 0 {
          deindent_columns(l, col)
        } else {
          l.to_string()
        });
        (*ctx.index) += 1;
      }
      if language == "math" {
        // GitHub's ```math fence normalizes to a math block.
        push_block(
          ctx.blocks,
          ctx.root_children,
          "math_block",
          code_lines.join("\n"),
          Value::Null,
        );
        return Some(());
      }
      let data = if language.is_empty() {
        Value::Null
      } else {
        json!({ "language": language })
      };
      push_block(
        ctx.blocks,
        ctx.root_children,
        "code_block",
        code_lines.join("\n"),
        data,
      );
      return Some(());
    }

    // Math block: `$$ … $$` or `\[ … \]`, single- or multi-line. The
    // LaTeX source is carried verbatim in a `math_block`; export always
    // writes the canonical `$$` form. (No spec exists — this follows the
    // Pandoc/GitHub dollar convention; see docs/editor-engine.md.)
  None // fall through: the family guard rejected the line
}

/// (family moved from the scan loop)
pub(crate) fn math_block(ctx: &mut ScanCtx, _line: &str, content: &str, _body: &str, col: usize) -> Option<()> {
    if col < 4
      && let Some((open_rest, closer)) = match content {
        c if c.starts_with("$$") => Some((&c[2..], "$$")),
        c if c.starts_with("\\[") => Some((&c[2..], "\\]")),
        _ => None,
      }
    {
      let open_rest = open_rest.trim();
      let mut source: Vec<String> = Vec::new();
      let mut closed = false;
      if let Some(inner) = open_rest.strip_suffix(closer) {
        // Single-line form: $$x^2$$
        if !inner.trim().is_empty() {
          source.push(inner.trim().to_string());
          closed = true;
        }
      }
      if !closed && open_rest.is_empty() {
        (*ctx.index) += 1;
        while *ctx.index < ctx.lines.len() {
          let l = ctx.lines[*ctx.index].trim();
          if l == closer {
            (*ctx.index) += 1;
            closed = true;
            break;
          }
          source.push(ctx.lines[*ctx.index].trim_end().to_string());
          (*ctx.index) += 1;
        }
      } else if closed {
        (*ctx.index) += 1;
      }
      if closed {
        push_block(
          ctx.blocks,
          ctx.root_children,
          "math_block",
          source.join("\n"),
          Value::Null,
        );
        ctx.st.list_stack.clear();
        ctx.st.open_item = None;
        ctx.st.last_list = None;
        ctx.st.pending_loose = false;
        ctx.st.item_children = false;
        return Some(());
      }
      // Unclosed: fall through and let the line parse normally.
    }

    // Footnote definition: `[^label]: content` at the line start (GFM). The
    // block carries the inline-parsed content as text and the label in
    // `data.label`; export restores the `[^label]: ` leader. Continuation
    // lines indented 4+ columns join the definition (a paragraph break inside
    // a footnote keeps the same block — we join with a newline, matching how
    // multi-line paragraphs carry their breaks).
  None // fall through: the family guard rejected the line
}

/// (family moved from the scan loop)
pub(crate) fn footnote_def(ctx: &mut ScanCtx, _line: &str, content: &str, _body: &str, col: usize) -> Option<()> {
    if col < 4
      && ctx.st.list_stack.is_empty()
      && ctx.st.open_item.is_none()
      && let Some((label, first)) = parse_footnote_def(content)
    {
      let mut body: Vec<String> = if first.is_empty() {
        Vec::new()
      } else {
        vec![first]
      };
      (*ctx.index) += 1;
      while *ctx.index < ctx.lines.len() {
        let l = ctx.lines[*ctx.index];
        let lt = l.trim_start();
        if lt.is_empty() {
          // A blank line ends the definition unless an indented line resumes
          // it; peek ahead, and if so keep the blank as a paragraph break.
          let resumes = ctx.lines
            .get((*ctx.index) + 1)
            .is_some_and(|n| !n.trim().is_empty() && column_of(n) >= 4);
          if !resumes {
            break;
          }
          body.push(String::new());
          (*ctx.index) += 1;
          continue;
        }
        if column_of(l) < 4 {
          break; // a non-indented line starts a new block
        }
        body.push(deindent_columns(l, 4));
        (*ctx.index) += 1;
      }
      let (text, data) =
        apply_inline_marks(body.join("\n"), json!({ "label": label }), ctx.defs);
      push_block(ctx.blocks, ctx.root_children, "footnote_def", text, data);
      ctx.st.list_stack.clear();
      ctx.st.open_item = None;
      ctx.st.last_list = None;
      ctx.st.pending_loose = false;
      ctx.st.item_children = false;
      return Some(());
    }

    // HTML block (CommonMark types 1–7) → a raw html code block: the
    // source is the content (AFFiNE-style degrade), `data.raw` makes both
    // exporters write it back verbatim. Type 7 can't interrupt a paragraph.
  None // fall through: the family guard rejected the line
}

/// (family moved from the scan loop)
pub(crate) fn html_block(ctx: &mut ScanCtx, _line: &str, content: &str, _body: &str, col: usize) -> Option<()> {
    if col < 4
      && let Some(html_kind) = html_block_start(content)
      // Outside any open item's content column, an HTML block INTERRUPTS
      // the list (spec ex. 308/309: `<!-- -->` splits two lists).
      && (ctx.st.list_stack.is_empty() || col < *ctx.st.list_stack.last().unwrap())
      && !(html_kind == 7 && ctx.st.open_item.is_some())
    {
      ctx.st.list_stack.clear();
      let mut html_lines: Vec<String> = vec![ctx.lines[*ctx.index].to_string()];
      let ends_by_marker = html_kind <= 5;
      let mut done = ends_by_marker && html_block_ends(html_kind, content);
      (*ctx.index) += 1;
      while *ctx.index < ctx.lines.len() && !done {
        let l = ctx.lines[*ctx.index];
        if ends_by_marker {
          html_lines.push(l.to_string());
          done = html_block_ends(html_kind, l);
          (*ctx.index) += 1;
        } else {
          if l.trim().is_empty() {
            break; // types 6–7 end at a blank line
          }
          html_lines.push(l.to_string());
          (*ctx.index) += 1;
        }
      }
      push_block(
        ctx.blocks,
        ctx.root_children,
        "code_block",
        html_lines.join("\n"),
        json!({ "language": "html", "raw": true }),
      );
      ctx.st.open_item = None;
      ctx.st.last_list = None;
      ctx.st.pending_loose = false;
      ctx.st.item_children = false;
      return Some(());
    }

    // GFM pipe table: a `|`-row followed by a `| --- |` separator whose
    // cell count MATCHES the header row (spec rule).
  None // fall through: the family guard rejected the line
}

/// (family moved from the scan loop)
pub(crate) fn table(ctx: &mut ScanCtx, _line: &str, content: &str, _body: &str, _col: usize) -> Option<()> {
    if content.contains('|')
      && (*ctx.index) + 1 < ctx.lines.len()
      && is_table_separator(ctx.lines[(*ctx.index) + 1].trim())
      && split_table_row(ctx.lines[(*ctx.index) + 1].trim()).len() == split_table_row(content).len()
    {
      ctx.st.list_stack.clear();
      ctx.st.open_item = None;
      ctx.st.last_list = None;
      ctx.st.pending_loose = false;
      let header = split_table_row(content);
      let width = header.len().max(1);
      // Column alignment from the separator's colons.
      let aligns: Vec<String> = split_table_row(ctx.lines[(*ctx.index) + 1].trim())
        .iter()
        .map(|cell| {
          let t = cell.trim();
          match (t.starts_with(':'), t.ends_with(':')) {
            (true, true) => "center",
            (false, true) => "right",
            (true, false) => "left",
            _ => "",
          }
          .to_string()
        })
        .collect();
      let mut rows: Vec<Vec<String>> = vec![header];
      (*ctx.index) += 2;
      while *ctx.index < ctx.lines.len() {
        let row = ctx.lines[*ctx.index].trim();
        if row.is_empty() {
          break;
        }
        // A pipe-less line still belongs to the table unless it starts
        // another block (GFM: the table breaks at a blank line or the
        // beginning of another block-level structure).
        if !row.contains('|')
          && (classify_markdown_line(row).0 != "paragraph"
            || is_divider(row)
            || fence_open(row).is_some()
            || html_block_start(row).is_some()
            || strip_quote_markers(row).0 > 0)
        {
          break;
        }
        rows.push(split_table_row(row));
        (*ctx.index) += 1;
      }
      // The header defines the column count: longer body rows truncate,
      // shorter ones pad (spec rule).
      for row in rows.iter_mut() {
        row.truncate(width);
        while row.len() < width {
          row.push(String::new());
        }
      }
      // Upgrade each cell from raw inline-Markdown source to the unified
      // marks-over-plain-text form: a cell carrying inline marks (**bold**,
      // `code`, links, …) becomes {"text","marks"} — the SAME representation
      // paragraphs use (crates/markdown is the authority; Dart mirrors it) —
      // while a mark-free cell stays a plain string, so simple tables are
      // byte-identical to before and the grid never bloats. Legacy string
      // cells still round-trip: `decode_table_cell` re-parses them on read.
      let cell_rows: Vec<Vec<Value>> = rows
        .iter()
        .map(|row| row.iter().map(|raw| encode_table_cell(raw)).collect())
        .collect();
      let mut data = json!({
        "rows": cell_rows,
        "header": true,
        "align": "left",
        "widths": vec![1.0f64; width],
      });
      if aligns.iter().any(|a| !a.is_empty()) {
        data_insert(&mut data, "aligns", json!(aligns));
      }
      push_block(ctx.blocks, ctx.root_children, "table", String::new(), data);
      return Some(());
    }

    // Indented code block: 4+ columns at top level (inside a list that
    // indentation means nesting instead, handled by the stack above).
  None // fall through: the family guard rejected the line
}

/// (family moved from the scan loop)
pub(crate) fn indented_code_and_divider(ctx: &mut ScanCtx, _line: &str, content: &str, _body: &str, col: usize) -> Option<()> {
    if col >= 4
      && ((ctx.st.list_stack.is_empty() && ctx.st.open_item.is_none())
        || ctx.st.open_item
          .as_ref()
          .is_some_and(|o| o.had_blank && col < o.content_col))
    {
      ctx.st.list_stack.clear();
      ctx.st.open_item = None;
      ctx.st.last_list = None;
      ctx.st.pending_loose = false;
      let mut code_lines: Vec<String> = Vec::new();
      let mut pending_blanks: Vec<String> = Vec::new();
      while *ctx.index < ctx.lines.len() {
        let l = ctx.lines[*ctx.index];
        if l.trim().is_empty() {
          // Blank-ish lines keep whatever indentation they have past the
          // 4-column code margin.
          pending_blanks.push(deindent_columns(l, 4));
          (*ctx.index) += 1;
          continue;
        }
        let c = l.trim_start();
        let mut lcol = 0usize;
        for ch in l[..l.len() - c.len()].chars() {
          lcol = if ch == '\t' { (lcol / 4 + 1) * 4 } else { lcol + 1 };
        }
        if lcol < 4 {
          break;
        }
        code_lines.append(&mut pending_blanks);
        // Trailing spaces are code content — keep the line untrimmed.
        code_lines.push(deindent_columns(l, 4));
        (*ctx.index) += 1;
      }
      push_block(
        ctx.blocks,
        ctx.root_children,
        "code_block",
        code_lines.join("\n"),
        Value::Null,
      );
      return Some(());
    }

    // Horizontal rule (`---`, `***`, `___`) → divider block.
    if is_divider(content) {
      ctx.st.list_stack.clear();
      ctx.st.open_item = None;
      ctx.st.last_list = None;
      ctx.st.pending_loose = false;
      push_block(ctx.blocks, ctx.root_children, "divider", String::new(), Value::Null);
      (*ctx.index) += 1;
      return Some(());
    }

    // Block quote: strip `>` markers (each takes one optional following
    // space; up to 3 spaces may sit between nested markers) → depth + rest.
    let (_qdepth, _qrest) = strip_quote_markers(content);
  None // fall through: the family guard rejected the line
}

/// (family moved from the scan loop)
pub(crate) fn quote_group(ctx: &mut ScanCtx, _line: &str, content: &str, body: &str, col: usize) -> Option<()> {
    // Block quote: strip `>` markers (each takes one optional following
    // space; up to 3 spaces may sit between nested markers) → depth + rest.
    let (qdepth, qrest) = strip_quote_markers(content);

    if qdepth > 0 {
      // A quote at or past the open item's content column nests INSIDE the
      // item (data.li) — the list context survives.
      let li_ctx = match ctx.st.list_stack.last() {
        Some(&cc) if col >= cc => Some(ctx.st.list_stack.len() - 1),
        _ => None,
      };
      if li_ctx.is_none() {
        ctx.st.list_stack.clear();
        ctx.st.last_list = None;
      } else if ctx.st.pending_loose {
        if let Some((prev_idx, _, _)) = ctx.st.last_list {
          data_insert(&mut ctx.blocks[prev_idx].data, "loose", json!(true));
        }
      }
      ctx.st.pending_loose = false;
      let qrest_trim = qrest.trim_start();

      // GFM alert marker: `> [!NOTE]` (etc.) as the FIRST line of a top-level
      // blockquote turns the group into a callout. Eat the marker line (it is
      // not body), remember the type for the group's first content block, and
      // arm `ctx.st.pending_empty_quote` so a body-less `> [!NOTE]` still round-trips
      // as an empty alert. Restricted to depth 1, top level (`li_ctx` none) to
      // match GitHub — nested/list-embedded `[!NOTE]` stays literal text.
      if qdepth == 1
        && li_ctx.is_none()
        && !ctx.st.quote_active
        && let Some(atype) = alert_type_of(qrest_trim)
      {
        ctx.st.pending_alert = Some(atype);
        ctx.st.pending_empty_quote = Some(1);
        ctx.st.open_item = None;
        ctx.st.quote_active = true;
        (*ctx.index) += 1;
        return Some(());
      }

      // `>` with nothing after: a paragraph break inside the quote — or,
      // if the group never gets content, an empty blockquote.
      if qrest_trim.is_empty() {
        // A blank inside a quoted LIST ITEM keeps the item open — an
        // indented continuation may follow (spec ex. 259).
        if let Some(open) = ctx.st.open_item.as_mut()
          && matches!(open.kind.as_str(), "bulleted_list" | "numbered_list" | "todo")
          && open.qdepth == qdepth
        {
          open.had_blank = true;
          ctx.st.quote_active = true;
          (*ctx.index) += 1;
          return Some(());
        }
        if !ctx.st.quote_active && ctx.st.pending_empty_quote.is_none() {
          ctx.st.pending_empty_quote = Some(qdepth);
        }
        ctx.st.open_item = None;
        ctx.st.quote_active = true;
        (*ctx.index) += 1;
        return Some(());
      }

      // Lazy/marked continuation: a plain paragraph line at the same or a
      // shallower marker depth keeps the open quoted paragraph going.
      let (kind, text, mut data) = classify_markdown_line(qrest_trim);
      if kind == "paragraph"
        && !qrest_trim.starts_with("```")
        && let Some(open) = ctx.st.open_item.as_mut()
        && open.qdepth >= qdepth
        && !open.had_blank
      {
        open.raw.push('\n');
        open.raw.push_str(strip_quote_markers(body).1.trim_start());
        defer_inline(ctx.blocks, &mut ctx.st.stale_inline, open.block_idx, &open.raw);
        ctx.st.quote_active = true;
        (*ctx.index) += 1;
        return Some(());
      }

      ctx.st.pending_empty_quote = None;
      let qbreak = ctx.st.quote_boundary;
      ctx.st.quote_boundary = false;

      // Fenced code inside the quote: runs while the markers do.
      if let Some(language) = qrest_trim.strip_prefix("```") {
        let language = language.trim().to_string();
        let mut code_lines = Vec::new();
        (*ctx.index) += 1;
        while *ctx.index < ctx.lines.len() {
          let lcontent = ctx.lines[*ctx.index].trim_end().trim_start();
          let (d2, r2) = strip_quote_markers(lcontent);
          if d2 < qdepth {
            break; // the quote ended — so does the fence
          }
          if r2.trim_start().starts_with("```") {
            (*ctx.index) += 1;
            break;
          }
          code_lines.push(r2.to_string());
          (*ctx.index) += 1;
        }
        let mut data = if language.is_empty() {
          Value::Null
        } else {
          json!({ "language": language })
        };
        data_insert(&mut data, "quote", json!(qdepth));
        if qbreak {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(a) = ctx.st.pending_alert.take() {
          data_insert(&mut data, "alert", json!(a));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          ctx.st.item_children = true;
        }
        push_block(ctx.blocks, ctx.root_children, "code_block", code_lines.join("\n"), data);
        ctx.st.open_item = None;
        ctx.st.quote_active = true;
        return Some(());
      }

      let qcol = qrest.len() - qrest_trim.len();
      // After a `>` gap, a line at the quoted item's content column is its
      // SECOND paragraph — the item turns loose (spec ex. 259).
      if let Some(open) = ctx.st.open_item.as_mut()
        && matches!(open.kind.as_str(), "bulleted_list" | "numbered_list" | "todo")
        && open.had_blank
        && open.qdepth == qdepth
        && qcol >= open.content_col
      {
        open.had_blank = false;
        open.raw.push_str("\n\n");
        open.raw.push_str(qrest_trim);
        data_insert(&mut open.base, "loose", json!(true));
        let (joined, joined_data) =
          apply_inline_marks(open.raw.clone(), open.base.clone(), ctx.defs);
        ctx.st.stale_inline.remove(&open.block_idx);
        ctx.blocks[open.block_idx].text = joined;
        ctx.blocks[open.block_idx].data = joined_data;
        ctx.st.quote_active = true;
        (*ctx.index) += 1;
        return Some(());
      }
      // Indented code inside the quote (per marked line).
      if qcol >= 4 {
        let mut data = json!({ "quote": qdepth });
        if qbreak {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(a) = ctx.st.pending_alert.take() {
          data_insert(&mut data, "alert", json!(a));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          ctx.st.item_children = true;
        }
        push_block(
          ctx.blocks,
          ctx.root_children,
          "code_block",
          deindent_columns(qrest, 4),
          data,
        );
        ctx.st.open_item = None;
        ctx.st.quote_active = true;
        (*ctx.index) += 1;
        return Some(());
      }

      if is_divider(qrest_trim) {
        let mut data = json!({ "quote": qdepth });
        if qbreak {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(a) = ctx.st.pending_alert.take() {
          data_insert(&mut data, "alert", json!(a));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          ctx.st.item_children = true;
        }
        push_block(ctx.blocks, ctx.root_children, "divider", String::new(), data);
        ctx.st.open_item = None;
        ctx.st.quote_active = true;
        (*ctx.index) += 1;
        return Some(());
      }

      // Quoted content block: plain text becomes the `quote` kind (depth in
      // `data.quote` past 1); any other kind carries `data.quote`.
      let kind = if kind == "paragraph" { "quote" } else { kind };
      if kind != "quote" || qdepth > 1 {
        data_insert(&mut data, "quote", json!(qdepth));
      }
      if qbreak {
        data_insert(&mut data, "qbreak", json!(true));
      }
      if let Some(a) = ctx.st.pending_alert.take() {
        data_insert(&mut data, "alert", json!(a));
      }
      if let Some(level) = li_ctx {
        data_insert(&mut data, "li", json!(level));
        ctx.st.item_children = true;
      }
      // A quoted list item: track its content column (marker + up to 3
      // consumed spaces) so blanks/continuations can address it, and let a
      // `>`-opening text become a nested quote CHILD (`> 1. > q`).
      let mut text = text;
      let mut item_ccol = 0usize;
      if matches!(kind, "bulleted_list" | "numbered_list" | "todo") {
        let marker_width =
          if kind == "todo" { 2 } else { qrest_trim.len() - text.len() };
        let extra = text.len() - text.trim_start_matches(' ').len();
        if extra <= 3 {
          text = text[extra..].to_string();
          item_ccol = qcol + marker_width + extra;
        } else {
          item_ccol = qcol + marker_width;
        }
        let (inner_depth, inner_rest) = strip_quote_markers(&text);
        if inner_depth > 0 {
          // Empty item + quote child at ABSOLUTE depth; the child stays
          // open so lazy lines continue it (spec ex. 292/293).
          let (it, id) = apply_inline_marks(String::new(), data, ctx.defs);
          push_block(ctx.blocks, ctx.root_children, kind, it, id);
          let raw = inner_rest.trim_start().to_string();
          let mut qdata = json!({ "quote": qdepth + inner_depth, "li": 0 });
          if qbreak {
            data_insert(&mut qdata, "qbreak", json!(true));
          }
          let base = qdata.clone();
          let (qt, qd) = apply_inline_marks(raw.clone(), qdata, ctx.defs);
          push_block(ctx.blocks, ctx.root_children, "quote", qt, qd);
          ctx.st.open_item = Some(OpenItem {
            block_idx: ctx.blocks.len() - 1,
            kind: "quote".to_string(),
            content_col: 0,
            raw,
            base,
            had_blank: false,
            qdepth: qdepth + inner_depth,
          });
          ctx.st.item_children = true;
          ctx.st.quote_active = true;
          (*ctx.index) += 1;
          return Some(());
        }
      }
      let raw = text.clone();
      let base = data.clone();
      let (text, data) = if kind == "image" {
        (parse_inline_with(&text, ctx.defs).text, data)
      } else {
        apply_inline_marks(text, data, ctx.defs)
      };
      push_block(ctx.blocks, ctx.root_children, kind, text, data);
      ctx.st.open_item = if matches!(kind, "quote" | "bulleted_list" | "numbered_list" | "todo") {
        Some(OpenItem {
          block_idx: ctx.blocks.len() - 1,
          kind: kind.to_string(),
          content_col: item_ccol,
          raw,
          base,
          had_blank: false,
          qdepth,
        })
      } else {
        None
      };
      ctx.st.quote_active = true;
      (*ctx.index) += 1;
      return Some(());
    }

  None // fall through: the family guard rejected the line
}

/// (family moved from the scan loop)
pub(crate) fn open_item_continuations(
  ctx: &mut ScanCtx,
  body: &str,
  col: usize,
  c: &ClassifiedLine,
) -> Option<()> {
    // Classification and top-level indentation normalization already happened
    // in the scanner. Inspect by reference so a declined family copies nothing.
    let (kind, text, data) = (c.kind, &c.text, &c.data);
    // Continuation (CommonMark): a paragraph line joins the open block with
    // a soft break (lazy lines included); after a blank, a line indented to
    // the item's content column starts a second paragraph inside the item
    // (the list turns loose). An empty list item or an ordered marker other
    // than `1.` cannot interrupt a paragraph — those lines stay c.text.
    if let Some(open) = ctx.st.open_item.as_mut() {
      let weak_item = matches!(kind, "bulleted_list" | "numbered_list" | "todo")
        && open.kind == "paragraph"
        && !open.had_blank
        && (text.is_empty() || data.get("start").is_some());
      if kind == "paragraph" || weak_item {
        if open.had_blank {
          if col >= open.content_col && !open.raw.is_empty() && !ctx.st.item_children {
            open.had_blank = false;
            open.raw.push_str("\n\n");
            open.raw.push_str(body);
            data_insert(&mut open.base, "loose", json!(true));
            ctx.st.pending_loose = false;
            let (joined, joined_data) =
              apply_inline_marks(open.raw.clone(), open.base.clone(), ctx.defs);
            ctx.st.stale_inline.remove(&open.block_idx);
            ctx.blocks[open.block_idx].text = joined;
            ctx.blocks[open.block_idx].data = joined_data;
            (*ctx.index) += 1;
            return Some(());
          }
          // The blank closed the DEEP item — but the line may still sit
          // inside an ANCESTOR item's content column: it becomes that
          // item's continuation paragraph, rendered after the sublist
          // (spec ex. 325). Flat model: a paragraph child (`c.data.li`).
          if c.kind == "paragraph"
            && let Some(anc) = ctx.st.list_stack.iter().rposition(|&cc| col >= cc)
            && anc + 1 < ctx.st.list_stack.len()
          {
            ctx.st.list_stack.truncate(anc + 1);
            // Owner: the last list block at that level.
            let owner = ctx.blocks.iter().rposition(|b| {
              list_tag_for(&b.kind).is_some()
                && li_of(b).is_none()
                && list_indent(b) == anc
            });
            if let Some(owner) = owner {
              data_insert(&mut ctx.blocks[owner].data, "loose", json!(true));
              let (pt, mut pd) = apply_inline_marks(body.to_string(), Value::Null, ctx.defs);
              data_insert(&mut pd, "li", json!(anc));
              push_block(ctx.blocks, ctx.root_children, "paragraph", pt, pd);
              ctx.st.pending_loose = false;
              ctx.st.item_children = true;
              ctx.st.open_item = None;
              (*ctx.index) += 1;
              return Some(());
            }
          }
          // The blank closed the item; whatever follows is a new block.
        } else {
          if open.raw.is_empty() {
            open.raw = body.to_string();
          } else {
            open.raw.push('\n');
            open.raw.push_str(body);
          }
          defer_inline(ctx.blocks, &mut ctx.st.stale_inline, open.block_idx, &open.raw);
          (*ctx.index) += 1;
          return Some(());
        }
      }
    }

    // List/todo items: nesting level from the indentation stack, loose and
    // <ol start> bookkeeping, then the item stays open for continuations.
  None // fall through: the family guard rejected the line
}

/// Consume a list line, or return its original classification to the leaf
/// handler. Passing ownership back avoids cloning the text/data for every
/// paragraph, heading and image that this family declines.
pub(crate) fn list_item(
  ctx: &mut ScanCtx,
  content: &str,
  col: usize,
  c: ClassifiedLine,
) -> ControlFlow<(), ClassifiedLine> {
    if matches!(c.kind, "bulleted_list" | "numbered_list" | "todo") {
    let (mut kind, mut text, mut data) = (c.kind, c.text, c.data);
      // `- - foo` / `1. - 2. foo`: an item whose text is ITSELF a list
      // marker is a chain — every outer link becomes an EMPTY item one
      // level deeper (spec ex. 298/299); only the innermost text is real
      // content. The HTML exporter already renders empty-item nesting.
      let marker_char_of = |kind: &str, content: &str| -> char {
        if kind == "numbered_list" {
          content.as_bytes()[content.bytes().position(|b| !b.is_ascii_digit()).unwrap()] as char
        } else {
          content.as_bytes()[0] as char
        }
      };
      
      
      
      let mut content_owned = content.to_string();
      let mut col = col;
      loop {
        // A divider outranks a marker chain: `- * * *` is an <hr> child
        // (spec ex. 61), not three nested bullets.
        if text.is_empty() || is_divider(&text) {
          break;
        }
        let inner = classify_markdown_line(&text);
        if !matches!(inner.0, "bulleted_list" | "numbered_list" | "todo") {
          break;
        }
        let marker_width =
          if kind == "todo" { 2 } else { content_owned.len() - text.len() };
        let extra = text.len() - text.trim_start_matches(' ').len();
        if extra > 3 {
          break; // 4+ spaces = an indented-code child, not a chain
        }
        while ctx.st.list_stack.last().is_some_and(|&cc| col < cc) {
          ctx.st.list_stack.pop();
        }
        let level = ctx.st.list_stack.len();
        let ccol = col + marker_width + extra;
        ctx.st.list_stack.push(ccol);
        let mut idata = Value::Null;
        if level > 0 {
          data_insert(&mut idata, "indent", json!(level));
        }
        if kind == "numbered_list"
          && let Some(start) = data.get("start")
        {
          data_insert(&mut idata, "start", start.clone());
        }
        if ctx.st.pending_loose {
          data_insert(&mut idata, "loose", json!(true));
          ctx.st.pending_loose = false;
        }
        push_block(ctx.blocks, ctx.root_children, kind, String::new(), idata);
        ctx.st.last_list = Some((ctx.blocks.len() - 1, level, marker_char_of(kind, &content_owned)));
        content_owned = text[extra..].to_string();
        col = ccol;
        let (k3, t3, d3) = classify_markdown_line(&content_owned);
        kind = k3;
        text = t3;
        data = d3;
      }
      let content: &str = &content_owned;
      // Content column: marker width plus up to 3 extra spaces consumed
      // (more than that means the item starts with indented code — the
      // spaces stay in the text and the column sits right after the
      // marker). A todo's task marker `[x] ` is CONTENT, not marker: only
      // the `- ` counts, so two-space-indented subtasks nest.
      let marker_width = if kind == "todo" {
        2
      } else {
        content.len() - text.len()
      };
      let extra = text.len() - text.trim_start_matches(' ').len();
      let (text, content_col) = if text.is_empty() {
        (text, col + marker_width + 1)
      } else if extra <= 3 {
        (text[extra..].to_string(), col + marker_width + extra)
      } else {
        (text, col + marker_width)
      };
      while ctx.st.list_stack.last().is_some_and(|&cc| col < cc) {
        ctx.st.list_stack.pop();
      }
      let level = ctx.st.list_stack.len();
      ctx.st.list_stack.push(content_col);
      if level > 0 {
        data_insert(&mut data, "indent", json!(level));
      }
      // The marker character: `-`/`*`/`+` for bullets and todos, the
      // delimiter (`.`/`)`) for ordered items.
      let marker_char = if kind == "numbered_list" {
        content.as_bytes()[content.bytes().position(|b| !b.is_ascii_digit()).unwrap()] as char
      } else {
        content.as_bytes()[0] as char
      };
      // Does this item continue the previous run (same level, same kind,
      // same marker)? A marker change starts a new list.
      let continues_run = ctx.st.last_list.is_some_and(|(idx, lv, mk)| {
        lv == level && ctx.blocks[idx].kind == kind && mk == marker_char
      });
      let same_level_break = !continues_run
        && ctx.st.last_list.is_some_and(|(idx, lv, _)| lv == level && ctx.blocks[idx].kind == kind);
      if same_level_break {
        // Record the changed bullet/delimiter so export keeps the break.
        data_insert(&mut data, "marker", json!(marker_char.to_string()));
      }
      // The blank line belongs to whichever list the boundary sits in:
      // same-or-shallower level → this item is loose; deeper level → the
      // blank separated a parent's text from its sublist, so the parent is.
      if ctx.st.pending_loose {
        match ctx.st.last_list {
          Some((prev_idx, prev_level, _)) if level > prev_level => {
            data_insert(&mut ctx.blocks[prev_idx].data, "loose", json!(true));
          }
          _ => data_insert(&mut data, "loose", json!(true)),
        }
        ctx.st.pending_loose = false;
      }
      // Only the number on the item that BEGINS an ordered run sets the
      // list's start; later numbers are ignored by the spec.
      if kind == "numbered_list" && data.get("start").is_some() && continues_run {
        data.as_object_mut().map(|m| m.remove("start"));
      }
      // An item whose text itself opens a container (`- ```'/`- ***`/code
      // by 4+ extra spaces) becomes an EMPTY item plus a child block.
      let starts_fence = fence_open(&text).map(|(c, n, info)| (c, n, info.to_string()));
      let starts_divider = is_divider(&text) && !text.is_empty();
      let starts_code = extra > 3;
      let starts_heading = heading_prefix_level(&text);
      if starts_fence.is_some() || starts_divider || starts_code || starts_heading.is_some() {
        let (item_text, item_data) = apply_inline_marks(String::new(), data, ctx.defs);
        push_block(ctx.blocks, ctx.root_children, kind, item_text, item_data);
        let block_idx = ctx.blocks.len() - 1;
        ctx.st.last_list = Some((block_idx, level, marker_char));
        while ctx.st.list_stack.len() > level {
          ctx.st.list_stack.pop();
        }
        ctx.st.list_stack.push(content_col);
        ctx.st.item_children = true;
        ctx.st.open_item = None;
        if let Some((fence_char, fence_len, info)) = starts_fence {
          let language =
            unescape_md(info.split_whitespace().next().unwrap_or_default());
          let mut code_lines = Vec::new();
          let mut closed = false;
          (*ctx.index) += 1;
          while *ctx.index < ctx.lines.len() {
            let l = ctx.lines[*ctx.index];
            let lt = l.trim_start();
            if lt.is_empty() {
              code_lines.push(String::new());
              (*ctx.index) += 1;
              continue;
            }
            if leading_columns(l) < content_col {
              break;
            }
            if fence_close(lt, fence_char, fence_len) {
              closed = true;
              (*ctx.index) += 1;
              break;
            }
            code_lines.push(deindent_columns(l, content_col));
            (*ctx.index) += 1;
          }
          // Blank lines BEFORE a real closing fence are content (spec ex.
          // 318); only an unterminated fence sheds the overshoot.
          if !closed {
            while code_lines.last().is_some_and(|l| l.is_empty()) {
              code_lines.pop();
            }
          }
          let mut cdata = if language.is_empty() {
            json!({})
          } else {
            json!({ "language": language })
          };
          data_insert(&mut cdata, "li", json!(level));
          push_block(ctx.blocks, ctx.root_children, "code_block", code_lines.join("\n"), cdata);
          return ControlFlow::Break(());
        }
        if starts_divider {
          push_block(
            ctx.blocks,
            ctx.root_children,
            "divider",
            String::new(),
            json!({ "li": level }),
          );
          (*ctx.index) += 1;
          return ControlFlow::Break(());
        }
        if let Some(hl) = starts_heading {
          let htext = strip_atx_closing(text[hl..].trim_start()).to_string();
          let (htext, mut hdata) = apply_inline_marks(htext, json!({ "level": hl }), ctx.defs);
          data_insert(&mut hdata, "li", json!(level));
          push_block(ctx.blocks, ctx.root_children, "heading", htext, hdata);
          (*ctx.index) += 1;
          return ControlFlow::Break(());
        }
        // starts_code: the kept extra spaces are an indented code child.
        push_block(
          ctx.blocks,
          ctx.root_children,
          "code_block",
          deindent_columns(&text, 4),
          json!({ "li": level }),
        );
        (*ctx.index) += 1;
        return ControlFlow::Break(());
      }
      let raw = text.clone();
      let base = data.clone();
      let (text, data) = apply_inline_marks(text, data, ctx.defs);
      push_block(ctx.blocks, ctx.root_children, kind, text, data);
      let block_idx = ctx.blocks.len() - 1;
      ctx.st.last_list = Some((block_idx, level, marker_char));
      ctx.st.item_children = false;
      ctx.st.open_item = Some(OpenItem {
        block_idx,
        kind: kind.to_string(),
        content_col,
        raw,
        base,
        had_blank: false,
        qdepth: 0,
      });
      (*ctx.index) += 1;
      return ControlFlow::Break(());
    }

    ctx.st.list_stack.clear();
  ControlFlow::Continue(c)
}

/// (family moved from the scan loop)
pub(crate) fn leaf_block(ctx: &mut ScanCtx, body: &str, c: ClassifiedLine) -> Option<()> {
    ctx.st.last_list = None;
    let ClassifiedLine { kind, text, data } = c;
    ctx.st.pending_loose = false;
    ctx.st.item_children = false;
    let (text, data) = if kind == "image" {
      // The alt is plain text — inline markup flattens (spec alt rule).
      (parse_inline_with(&text, ctx.defs).text, data)
    } else {
      apply_inline_marks(text, data, ctx.defs)
    };
    let raw = if kind == "paragraph" {
      Some(body.to_string())
    } else {
      None
    };
    push_block(ctx.blocks, ctx.root_children, kind, text, data);
    // Paragraphs stay open for lazy continuation and setext underlines.
    ctx.st.open_item = raw.map(|raw| OpenItem {
      block_idx: ctx.blocks.len() - 1,
      kind: "paragraph".to_string(),
      content_col: 0,
      raw,
      base: Value::Null,
      had_blank: false,
      qdepth: 0,
    });
    (*ctx.index) += 1;
  Some(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn declined_families_transfer_large_classification_without_copying() {
    // The handler extraction used to clone BOTH payloads before each family,
    // even when the line was neither a continuation nor a list item.
    for kind in ["paragraph", "heading", "image"] {
      let mut blocks = Vec::new();
      let mut children = Vec::new();
      let defs = RefDefs::new();
      let mut index = 0;
      let mut st = BlockState::new();
      st.list_stack.push(2);
      let mut ctx = ScanCtx {
        lines: &[],
        blocks: &mut blocks,
        root_children: &mut children,
        defs: &defs,
        index: &mut index,
        st: &mut st,
      };
      let c = ClassifiedLine {
        kind,
        text: "a".repeat(1024 * 1024),
        data: json!({ "url": "b".repeat(1024 * 1024) }),
      };
      let text_ptr = c.text.as_ptr();
      let data_ptr = c.data["url"].as_str().unwrap().as_ptr();
      assert!(open_item_continuations(&mut ctx, "unused", 0, &c).is_none());
      let ControlFlow::Continue(c) = list_item(&mut ctx, "unused", 0, c) else {
        panic!("non-list classification must reach the leaf handler");
      };
      assert_eq!(c.text.as_ptr(), text_ptr);
      assert_eq!(c.data["url"].as_str().unwrap().as_ptr(), data_ptr);
      assert!(ctx.st.list_stack.is_empty());
      assert!(ctx.blocks.is_empty());
      assert_eq!(*ctx.index, 0);
    }
  }
}
