
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{Block, DocumentSnapshotPayload};
use crate::export_md::*;
use crate::html::*;
use crate::inline::*;

/// The block scanner's mutable state — everything `import_markdown_inner`
/// carries ACROSS loop iterations. Every field has an owner (the construct
/// family that writes it) and an invariant documented at the field; the
/// decomposition (docs/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §8)
/// exists to make these invariants explicit instead of reader-side lore.
pub(crate) struct BlockState {
  /// Stack of open list items' CONTENT columns (tabs count as 4): a new item
  /// is a child only when its marker reaches the parent's content column —
  /// `- foo` / ` - bar` are siblings. Other blocks reset the stack.
  ///
  /// Invariant: strictly the columns of the items currently open at each
  /// nesting level, bottom-up; anything not list-item-shaped clears it.
  pub(crate) list_stack: Vec<usize>,
  /// The most recently pushed paragraph-like block (paragraph or list/todo
  /// item), kept open for multi-line continuation. Owner: paragraph/list
  /// handlers.
  pub(crate) open_item: Option<OpenItem>,
  /// (block_idx, level, marker char) of the last pushed list ITEM — anchors
  /// loose marking, `<ol start>` and marker-change breaks. Invariant: points
  /// at a real list block while `list_stack` is non-empty; cleared with it.
  pub(crate) last_list: Option<(usize, usize, char)>,
  /// A blank line was seen inside a list: if another item follows, the list
  /// turns loose. Invariant: only ever SET while `list_stack` is non-empty
  /// (a blank outside any list never arms it); consumed by the next list
  /// push or the block that ends the list context.
  pub(crate) pending_loose: bool,
  /// A blockquote group is currently open (its lines have been seen).
  pub(crate) quote_active: bool,
  /// A blank just closed a quote group: the NEXT quote at the same spot is a
  /// SEPARATE `<blockquote>` and carries `data.qbreak`. Invariant: only
  /// meaningful between quote groups — cleared when the next quote consumes
  /// it.
  pub(crate) quote_boundary: bool,
  /// A content-less `>` group (or a body-less alert marker) waiting to
  /// become an empty quote block if no content arrives. Invariant: None
  /// while `quote_active` (content arrived first would have consumed it).
  pub(crate) pending_empty_quote: Option<usize>,
  /// A `> [!NOTE]` marker line just opened a GFM alert: the type waits here
  /// and attaches to the group's FIRST content block's `data.alert`.
  /// Invariant: consumed the moment a block is pushed for this group.
  pub(crate) pending_alert: Option<&'static str>,
  /// The deepest open list item already holds container children
  /// (code/quote/divider blocks carrying `data.li`): later indented
  /// paragraphs become child paragraphs instead of `\n\n` text joins.
  pub(crate) item_children: bool,
}

/// The continuation target of the open paragraph-like block: raw source (to
/// re-join) plus the pre-inline data (so the join can re-parse cleanly).
pub(crate) struct OpenItem {
  pub(crate) block_idx: usize,
  pub(crate) kind: String,
  pub(crate) content_col: usize,
  pub(crate) raw: String,
  pub(crate) base: Value,
  pub(crate) had_blank: bool,
  pub(crate) qdepth: usize,
}

impl BlockState {
  pub(crate) fn new() -> Self {
    BlockState {
      list_stack: Vec::new(),
      open_item: None,
      last_list: None,
      pending_loose: false,
      quote_active: false,
      quote_boundary: false,
      pending_empty_quote: None,
      pending_alert: None,
      item_children: false,
    }
  }

  /// The list context is over: every list-family field resets together.
  /// (The owners call this instead of hand-clearing a subset — the subsets
  /// were exactly where the implicit invariants used to live.)
  pub(crate) fn end_list_context(&mut self) {
    self.list_stack.clear();
    self.open_item = None;
    self.last_list = None;
    self.pending_loose = false;
  }

  /// Debug-only assertion of the cross-field invariants documented above.
  /// Run once per loop iteration in debug builds: a refactor that breaks an
  /// invariant fails the FIRST corpus input that exercises it, not a user.
  pub(crate) fn debug_validate(&self) {
    debug_assert!(
      !self.pending_loose || !self.list_stack.is_empty(),
      "pending_loose is armed only inside a list (list_stack non-empty)"
    );
    // The alert flow legally holds both (`> [!NOTE]` arms the empty-quote
    // slot AND opens the group in one step), so the real invariant is about
    // the CONTENT side: once real content has pushed a block for the group,
    // no stale empty-quote slot may remain armed.
    debug_assert!(
      self.pending_empty_quote.map_or(true, |d| d >= 1),
      "pending_empty_quote carries the quote depth (>= 1)"
    );
  }
}

pub(crate) fn import_markdown_inner(
  markdown: &str,
  root_block_id: &str,
  parse_front_matter: bool,
) -> DocumentSnapshotPayload {
  // YAML front matter: a leading `---` fence with a later `---`/`...` close.
  // We don't parse the YAML — just lift the raw inner text off the block
  // stream so it can't degrade into thematic breaks / paragraphs, and stash
  // it on the root for verbatim round-trip on export.
  let (front_matter, body) = if parse_front_matter {
    split_front_matter(markdown)
  } else {
    (None, markdown)
  };

  let raw_lines: Vec<&str> = body.lines().collect();
  let mut blocks: Vec<Block> = Vec::new();
  let mut root_children: Vec<String> = Vec::new();

  // Pass 1: link reference definitions (`[label]: dest "title"`, possibly
  // spanning lines) — they resolve case-insensitively and vanish from the
  // output. Definitions inside fences don't count, and a definition can't
  // interrupt a paragraph.
  let (defs, def_lines, quote_def_lines) = collect_ref_definitions(&raw_lines);

  let mut index = 0;
  // The scanner's cross-iteration state, with its invariants documented at
  // the field (BlockState above). The loop body addresses it as `st.field`.
  let mut st = BlockState::new();
  while index < raw_lines.len() {
    st.debug_validate();
    if def_lines.contains(&index) {
      index += 1;
      continue;
    }
    if let Some(&qd) = quote_def_lines.get(&index) {
      // The definition was consumed; the line still opens/continues its
      // blockquote as a contentless `>` marker (spec ex. 218).
      if !st.quote_active && st.pending_empty_quote.is_none() {
        st.pending_empty_quote = Some(qd);
      }
      st.open_item = None;
      st.quote_active = true;
      index += 1;
      continue;
    }
    let expanded = expand_marker_tabs(raw_lines[index]);
    let line = expanded.trim_end();
    let content = line.trim_start();
    // Paragraph text keeps the source line's trailing spaces — hard-break
    // detection happens at inline-parse time (it must see code spans), so
    // the block layer must not destroy the evidence.
    let body = expanded.trim_end_matches(['\n', '\r']).trim_start();

    if content.is_empty() {
      // Blank lines between items keep the list context, end the open
      // paragraph (items merely note the blank — an indented line can still
      // continue them), and make the list loose if it continues.
      match st.open_item.as_mut() {
        Some(open)
          if matches!(open.kind.as_str(), "bulleted_list" | "numbered_list" | "todo") =>
        {
          open.had_blank = true
        }
        _ => st.open_item = None,
      }
      st.pending_loose = st.pending_loose || !st.list_stack.is_empty();
      // A content-less `>` group becomes an empty quote block; a blank
      // after a quote separates blockquotes (the next gets `qbreak`).
      if let Some(d) = st.pending_empty_quote.take() {
        let mut data = if d > 1 { json!({ "quote": d }) } else { Value::Null };
        if st.quote_boundary {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(a) = st.pending_alert.take() {
          data_insert(&mut data, "alert", json!(a));
        }
        push_block(&mut blocks, &mut root_children, "quote", String::new(), data);
      }
      if blocks.last().is_some_and(|b| quote_depth_of(b) > 0) {
        st.quote_boundary = true;
      }
      st.quote_active = false;
      index += 1;
      continue;
    }
    let mut col: usize = 0;
    for c in line[..line.len() - content.len()].chars() {
      col = if c == '\t' { (col / 4 + 1) * 4 } else { col + 1 };
    }

    // A 4+-column line cannot start a new block while a top-level
    // paragraph-like block is open — it is lazy continuation text.
    if col >= 4
      && st.list_stack.is_empty()
      && let Some(open) = st.open_item.as_mut()
      && !open.had_blank
    {
      open.raw.push('\n');
      open.raw.push_str(body);
      let (joined, joined_data) = apply_inline_marks(open.raw.clone(), open.base.clone(), &defs);
      blocks[open.block_idx].text = joined;
      blocks[open.block_idx].data = joined_data;
      index += 1;
      continue;
    }

    // Setext underline of the open (possibly multi-line) paragraph: the
    // whole continued paragraph becomes the heading.
    if let Some(open) = &st.open_item
      && open.kind == "paragraph"
      && let Some(level) = setext_level(line)
    {
      let (text, data) = apply_inline_marks(open.raw.clone(), json!({ "level": level }), &defs);
      let idx = open.block_idx;
      blocks[idx].kind = "heading".to_string();
      blocks[idx].text = text;
      blocks[idx].data = data;
      st.open_item = None;
      index += 1;
      continue;
    }

    // List-item container children: at or past the open item's content
    // column, fences / indented code / dividers (and, below, quotes and
    // paragraphs) belong INSIDE the item — flat blocks carrying `data.li`.
    if let Some(&cc) = st.list_stack.last()
      && col >= cc
      && strip_quote_markers(content).0 == 0
      && !matches!(
        classify_markdown_line(content).0,
        "bulleted_list" | "numbered_list" | "todo"
      )
    {
      let level = st.list_stack.len() - 1;
      // Fenced code child.
      if let Some((fence_char, fence_len, info)) = fence_open(content) {
        let language =
          unescape_md(info.split_whitespace().next().unwrap_or_default());
        let mut code_lines = Vec::new();
        index += 1;
        while index < raw_lines.len() {
          let l = raw_lines[index];
          let lt = l.trim_start();
          if lt.is_empty() {
            code_lines.push(String::new());
            index += 1;
            continue;
          }
          let lcol = leading_columns(l);
          if lcol < cc {
            break; // the item ended — so does the fence
          }
          if fence_close(lt, fence_char, fence_len) {
            index += 1;
            break;
          }
          // Content sheds up to the OPENING fence's indentation.
          code_lines.push(deindent_columns(l, col));
          index += 1;
        }
        // Trailing blank-only lines belong between blocks, not the code.
        while code_lines.last().is_some_and(|l| l.is_empty()) {
          code_lines.pop();
        }
        let mut data = if language.is_empty() {
          json!({})
        } else {
          json!({ "language": language })
        };
        data_insert(&mut data, "li", json!(level));
        if st.pending_loose {
          if let Some((prev_idx, _, _)) = st.last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          st.pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), data);
        st.item_children = true;
        st.open_item = None;
        continue;
      }
      // Indented code child (4+ columns past the content column).
      if col >= cc + 4 {
        let mut code_lines: Vec<String> = Vec::new();
        let mut blanks: Vec<String> = Vec::new();
        while index < raw_lines.len() {
          let l = raw_lines[index];
          if l.trim().is_empty() {
            blanks.push(deindent_columns(l, cc + 4));
            index += 1;
            continue;
          }
          if leading_columns(l) < cc + 4 {
            break;
          }
          code_lines.append(&mut blanks);
          code_lines.push(deindent_columns(l, cc + 4));
          index += 1;
        }
        let data = json!({ "li": level });
        if st.pending_loose {
          if let Some((prev_idx, _, _)) = st.last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          st.pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), data);
        st.item_children = true;
        st.open_item = None;
        continue;
      }
      // Setext underline of the OPEN item's text: the text becomes a
      // heading CHILD (`- Bar\n  ---` → <li><h2>Bar</h2>, spec ex. 300).
      // Checked before the divider — with an open texted item, `---` is
      // an underline, not an <hr> (same precedence as at top level).
      if let Some(hl) = setext_level(content)
        && let Some(open) = st.open_item.as_ref()
        && matches!(open.kind.as_str(), "bulleted_list" | "numbered_list" | "todo")
        && !open.raw.is_empty()
        && !open.had_blank
      {
        let raw_text = open.raw.clone();
        let idx = open.block_idx;
        let (it, id) = apply_inline_marks(String::new(), open.base.clone(), &defs);
        blocks[idx].text = it;
        blocks[idx].data = id;
        let (ht, mut hd) = apply_inline_marks(raw_text, json!({ "level": hl }), &defs);
        data_insert(&mut hd, "li", json!(level));
        push_block(&mut blocks, &mut root_children, "heading", ht, hd);
        st.item_children = true;
        st.open_item = None;
        index += 1;
        continue;
      }
      // Divider child.
      if is_divider(content) {
        let data = json!({ "li": level });
        if st.pending_loose {
          if let Some((prev_idx, _, _)) = st.last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          st.pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, "divider", String::new(), data);
        st.item_children = true;
        st.open_item = None;
        index += 1;
        continue;
      }
      // Paragraph child: once the item holds children, indented paragraph
      // lines become child blocks (order matters — text renders first).
      if st.item_children
        && classify_markdown_line(content).0 == "paragraph"
        && st.open_item.is_none()
      {
        let data = json!({ "li": level });
        if st.pending_loose {
          if let Some((prev_idx, _, _)) = st.last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          st.pending_loose = false;
        }
        let base = data.clone();
        let (text2, data2) = apply_inline_marks(content.to_string(), data, &defs);
        push_block(&mut blocks, &mut root_children, "paragraph", text2, data2);
        st.open_item = Some(OpenItem {
          block_idx: blocks.len() - 1,
          kind: "paragraph".to_string(),
          content_col: cc,
          raw: body.to_string(),
          base,
          had_blank: false,
          qdepth: 0,
        });
        index += 1;
        continue;
      }
      // Anything else falls through to the regular machinery.
    }

    if col < 4
      && let Some((fence_char, fence_len, info)) = fence_open(content)
    {
      st.list_stack.clear();
      st.open_item = None;
      st.last_list = None;
      st.pending_loose = false;
      // Only the first word of the info string is the language.
      let language =
        unescape_md(info.split_whitespace().next().unwrap_or_default());
      let mut code_lines = Vec::new();
      index += 1;
      while index < raw_lines.len() {
        let l = raw_lines[index];
        let lt = l.trim_start();
        let lcol = l.len() - lt.len();
        if lcol < 4 && fence_close(lt, fence_char, fence_len) {
          index += 1;
          break;
        }
        // Content lines shed up to the opening fence's indentation.
        code_lines.push(if col > 0 {
          deindent_columns(l, col)
        } else {
          l.to_string()
        });
        index += 1;
      }
      if language == "math" {
        // GitHub's ```math fence normalizes to a math block.
        push_block(
          &mut blocks,
          &mut root_children,
          "math_block",
          code_lines.join("\n"),
          Value::Null,
        );
        continue;
      }
      let data = if language.is_empty() {
        Value::Null
      } else {
        json!({ "language": language })
      };
      push_block(
        &mut blocks,
        &mut root_children,
        "code_block",
        code_lines.join("\n"),
        data,
      );
      continue;
    }

    // Math block: `$$ … $$` or `\[ … \]`, single- or multi-line. The
    // LaTeX source is carried verbatim in a `math_block`; export always
    // writes the canonical `$$` form. (No spec exists — this follows the
    // Pandoc/GitHub dollar convention; see docs/editor-engine.md.)
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
        index += 1;
        while index < raw_lines.len() {
          let l = raw_lines[index].trim();
          if l == closer {
            index += 1;
            closed = true;
            break;
          }
          source.push(raw_lines[index].trim_end().to_string());
          index += 1;
        }
      } else if closed {
        index += 1;
      }
      if closed {
        push_block(
          &mut blocks,
          &mut root_children,
          "math_block",
          source.join("\n"),
          Value::Null,
        );
        st.list_stack.clear();
        st.open_item = None;
        st.last_list = None;
        st.pending_loose = false;
        st.item_children = false;
        continue;
      }
      // Unclosed: fall through and let the line parse normally.
    }

    // Footnote definition: `[^label]: content` at the line start (GFM). The
    // block carries the inline-parsed content as text and the label in
    // `data.label`; export restores the `[^label]: ` leader. Continuation
    // lines indented 4+ columns join the definition (a paragraph break inside
    // a footnote keeps the same block — we join with a newline, matching how
    // multi-line paragraphs carry their breaks).
    if col < 4
      && st.list_stack.is_empty()
      && st.open_item.is_none()
      && let Some((label, first)) = parse_footnote_def(content)
    {
      let mut body: Vec<String> = if first.is_empty() {
        Vec::new()
      } else {
        vec![first]
      };
      index += 1;
      while index < raw_lines.len() {
        let l = raw_lines[index];
        let lt = l.trim_start();
        if lt.is_empty() {
          // A blank line ends the definition unless an indented line resumes
          // it; peek ahead, and if so keep the blank as a paragraph break.
          let resumes = raw_lines
            .get(index + 1)
            .is_some_and(|n| !n.trim().is_empty() && column_of(n) >= 4);
          if !resumes {
            break;
          }
          body.push(String::new());
          index += 1;
          continue;
        }
        if column_of(l) < 4 {
          break; // a non-indented line starts a new block
        }
        body.push(deindent_columns(l, 4));
        index += 1;
      }
      let (text, data) =
        apply_inline_marks(body.join("\n"), json!({ "label": label }), &defs);
      push_block(&mut blocks, &mut root_children, "footnote_def", text, data);
      st.list_stack.clear();
      st.open_item = None;
      st.last_list = None;
      st.pending_loose = false;
      st.item_children = false;
      continue;
    }

    // HTML block (CommonMark types 1–7) → a raw html code block: the
    // source is the content (AFFiNE-style degrade), `data.raw` makes both
    // exporters write it back verbatim. Type 7 can't interrupt a paragraph.
    if col < 4
      && let Some(html_kind) = html_block_start(content)
      // Outside any open item's content column, an HTML block INTERRUPTS
      // the list (spec ex. 308/309: `<!-- -->` splits two lists).
      && (st.list_stack.is_empty() || col < *st.list_stack.last().unwrap())
      && !(html_kind == 7 && st.open_item.is_some())
    {
      st.list_stack.clear();
      let mut html_lines: Vec<String> = vec![raw_lines[index].to_string()];
      let ends_by_marker = html_kind <= 5;
      let mut done = ends_by_marker && html_block_ends(html_kind, content);
      index += 1;
      while index < raw_lines.len() && !done {
        let l = raw_lines[index];
        if ends_by_marker {
          html_lines.push(l.to_string());
          done = html_block_ends(html_kind, l);
          index += 1;
        } else {
          if l.trim().is_empty() {
            break; // types 6–7 end at a blank line
          }
          html_lines.push(l.to_string());
          index += 1;
        }
      }
      push_block(
        &mut blocks,
        &mut root_children,
        "code_block",
        html_lines.join("\n"),
        json!({ "language": "html", "raw": true }),
      );
      st.open_item = None;
      st.last_list = None;
      st.pending_loose = false;
      st.item_children = false;
      continue;
    }

    // GFM pipe table: a `|`-row followed by a `| --- |` separator whose
    // cell count MATCHES the header row (spec rule).
    if content.contains('|')
      && index + 1 < raw_lines.len()
      && is_table_separator(raw_lines[index + 1].trim())
      && split_table_row(raw_lines[index + 1].trim()).len() == split_table_row(content).len()
    {
      st.list_stack.clear();
      st.open_item = None;
      st.last_list = None;
      st.pending_loose = false;
      let header = split_table_row(content);
      let width = header.len().max(1);
      // Column alignment from the separator's colons.
      let aligns: Vec<String> = split_table_row(raw_lines[index + 1].trim())
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
      index += 2;
      while index < raw_lines.len() {
        let row = raw_lines[index].trim();
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
        index += 1;
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
      push_block(&mut blocks, &mut root_children, "table", String::new(), data);
      continue;
    }

    // Indented code block: 4+ columns at top level (inside a list that
    // indentation means nesting instead, handled by the stack above).
    if col >= 4
      && ((st.list_stack.is_empty() && st.open_item.is_none())
        || st.open_item
          .as_ref()
          .is_some_and(|o| o.had_blank && col < o.content_col))
    {
      st.list_stack.clear();
      st.open_item = None;
      st.last_list = None;
      st.pending_loose = false;
      let mut code_lines: Vec<String> = Vec::new();
      let mut pending_blanks: Vec<String> = Vec::new();
      while index < raw_lines.len() {
        let l = raw_lines[index];
        if l.trim().is_empty() {
          // Blank-ish lines keep whatever indentation they have past the
          // 4-column code margin.
          pending_blanks.push(deindent_columns(l, 4));
          index += 1;
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
        index += 1;
      }
      push_block(
        &mut blocks,
        &mut root_children,
        "code_block",
        code_lines.join("\n"),
        Value::Null,
      );
      continue;
    }

    // Horizontal rule (`---`, `***`, `___`) → divider block.
    if is_divider(content) {
      st.list_stack.clear();
      st.open_item = None;
      st.last_list = None;
      st.pending_loose = false;
      push_block(&mut blocks, &mut root_children, "divider", String::new(), Value::Null);
      index += 1;
      continue;
    }

    // Block quote: strip `>` markers (each takes one optional following
    // space; up to 3 spaces may sit between nested markers) → depth + rest.
    let (qdepth, qrest) = strip_quote_markers(content);
    if qdepth > 0 {
      // A quote at or past the open item's content column nests INSIDE the
      // item (data.li) — the list context survives.
      let li_ctx = match st.list_stack.last() {
        Some(&cc) if col >= cc => Some(st.list_stack.len() - 1),
        _ => None,
      };
      if li_ctx.is_none() {
        st.list_stack.clear();
        st.last_list = None;
      } else if st.pending_loose {
        if let Some((prev_idx, _, _)) = st.last_list {
          data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
        }
      }
      st.pending_loose = false;
      let qrest_trim = qrest.trim_start();

      // GFM alert marker: `> [!NOTE]` (etc.) as the FIRST line of a top-level
      // blockquote turns the group into a callout. Eat the marker line (it is
      // not body), remember the type for the group's first content block, and
      // arm `st.pending_empty_quote` so a body-less `> [!NOTE]` still round-trips
      // as an empty alert. Restricted to depth 1, top level (`li_ctx` none) to
      // match GitHub — nested/list-embedded `[!NOTE]` stays literal text.
      if qdepth == 1
        && li_ctx.is_none()
        && !st.quote_active
        && let Some(atype) = alert_type_of(qrest_trim)
      {
        st.pending_alert = Some(atype);
        st.pending_empty_quote = Some(1);
        st.open_item = None;
        st.quote_active = true;
        index += 1;
        continue;
      }

      // `>` with nothing after: a paragraph break inside the quote — or,
      // if the group never gets content, an empty blockquote.
      if qrest_trim.is_empty() {
        // A blank inside a quoted LIST ITEM keeps the item open — an
        // indented continuation may follow (spec ex. 259).
        if let Some(open) = st.open_item.as_mut()
          && matches!(open.kind.as_str(), "bulleted_list" | "numbered_list" | "todo")
          && open.qdepth == qdepth
        {
          open.had_blank = true;
          st.quote_active = true;
          index += 1;
          continue;
        }
        if !st.quote_active && st.pending_empty_quote.is_none() {
          st.pending_empty_quote = Some(qdepth);
        }
        st.open_item = None;
        st.quote_active = true;
        index += 1;
        continue;
      }

      // Lazy/marked continuation: a plain paragraph line at the same or a
      // shallower marker depth keeps the open quoted paragraph going.
      let (kind, text, mut data) = classify_markdown_line(qrest_trim);
      if kind == "paragraph"
        && !qrest_trim.starts_with("```")
        && let Some(open) = st.open_item.as_mut()
        && open.qdepth >= qdepth
        && !open.had_blank
      {
        open.raw.push('\n');
        open.raw.push_str(strip_quote_markers(body).1.trim_start());
        let (joined, joined_data) =
          apply_inline_marks(open.raw.clone(), open.base.clone(), &defs);
        blocks[open.block_idx].text = joined;
        blocks[open.block_idx].data = joined_data;
        st.quote_active = true;
        index += 1;
        continue;
      }

      st.pending_empty_quote = None;
      let qbreak = st.quote_boundary;
      st.quote_boundary = false;

      // Fenced code inside the quote: runs while the markers do.
      if let Some(language) = qrest_trim.strip_prefix("```") {
        let language = language.trim().to_string();
        let mut code_lines = Vec::new();
        index += 1;
        while index < raw_lines.len() {
          let lcontent = raw_lines[index].trim_end().trim_start();
          let (d2, r2) = strip_quote_markers(lcontent);
          if d2 < qdepth {
            break; // the quote ended — so does the fence
          }
          if r2.trim_start().starts_with("```") {
            index += 1;
            break;
          }
          code_lines.push(r2.to_string());
          index += 1;
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
        if let Some(a) = st.pending_alert.take() {
          data_insert(&mut data, "alert", json!(a));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          st.item_children = true;
        }
        push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), data);
        st.open_item = None;
        st.quote_active = true;
        continue;
      }

      let qcol = qrest.len() - qrest_trim.len();
      // After a `>` gap, a line at the quoted item's content column is its
      // SECOND paragraph — the item turns loose (spec ex. 259).
      if let Some(open) = st.open_item.as_mut()
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
          apply_inline_marks(open.raw.clone(), open.base.clone(), &defs);
        blocks[open.block_idx].text = joined;
        blocks[open.block_idx].data = joined_data;
        st.quote_active = true;
        index += 1;
        continue;
      }
      // Indented code inside the quote (per marked line).
      if qcol >= 4 {
        let mut data = json!({ "quote": qdepth });
        if qbreak {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(a) = st.pending_alert.take() {
          data_insert(&mut data, "alert", json!(a));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          st.item_children = true;
        }
        push_block(
          &mut blocks,
          &mut root_children,
          "code_block",
          deindent_columns(qrest, 4),
          data,
        );
        st.open_item = None;
        st.quote_active = true;
        index += 1;
        continue;
      }

      if is_divider(qrest_trim) {
        let mut data = json!({ "quote": qdepth });
        if qbreak {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(a) = st.pending_alert.take() {
          data_insert(&mut data, "alert", json!(a));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          st.item_children = true;
        }
        push_block(&mut blocks, &mut root_children, "divider", String::new(), data);
        st.open_item = None;
        st.quote_active = true;
        index += 1;
        continue;
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
      if let Some(a) = st.pending_alert.take() {
        data_insert(&mut data, "alert", json!(a));
      }
      if let Some(level) = li_ctx {
        data_insert(&mut data, "li", json!(level));
        st.item_children = true;
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
          let (it, id) = apply_inline_marks(String::new(), data, &defs);
          push_block(&mut blocks, &mut root_children, kind, it, id);
          let raw = inner_rest.trim_start().to_string();
          let mut qdata = json!({ "quote": qdepth + inner_depth, "li": 0 });
          if qbreak {
            data_insert(&mut qdata, "qbreak", json!(true));
          }
          let base = qdata.clone();
          let (qt, qd) = apply_inline_marks(raw.clone(), qdata, &defs);
          push_block(&mut blocks, &mut root_children, "quote", qt, qd);
          st.open_item = Some(OpenItem {
            block_idx: blocks.len() - 1,
            kind: "quote".to_string(),
            content_col: 0,
            raw,
            base,
            had_blank: false,
            qdepth: qdepth + inner_depth,
          });
          st.item_children = true;
          st.quote_active = true;
          index += 1;
          continue;
        }
      }
      let raw = text.clone();
      let base = data.clone();
      let (text, data) = if kind == "image" {
        (parse_inline_with(&text, &defs).text, data)
      } else {
        apply_inline_marks(text, data, &defs)
      };
      push_block(&mut blocks, &mut root_children, kind, text, data);
      st.open_item = if matches!(kind, "quote" | "bulleted_list" | "numbered_list" | "todo") {
        Some(OpenItem {
          block_idx: blocks.len() - 1,
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
      st.quote_active = true;
      index += 1;
      continue;
    }

    let (kind, text, data) = classify_markdown_line(content);
    // An indented (4+ columns) marker cannot start a list at top level —
    // the line is paragraph continuation or code, never a new item.
    let (kind, text, data) = if matches!(kind, "bulleted_list" | "numbered_list" | "todo")
      && col >= 4
      // ... at top level, or indented past EVERY open item's content column
      // (spec ex. 312: the 4-space `- e` is lazy text of item `d`).
      && st.list_stack.iter().all(|&cc| col < cc)
    {
      ("paragraph", content.to_string(), Value::Null)
    } else {
      (kind, text, data)
    };

    // Continuation (CommonMark): a paragraph line joins the open block with
    // a soft break (lazy lines included); after a blank, a line indented to
    // the item's content column starts a second paragraph inside the item
    // (the list turns loose). An empty list item or an ordered marker other
    // than `1.` cannot interrupt a paragraph — those lines stay text.
    if let Some(open) = st.open_item.as_mut() {
      let weak_item = matches!(kind, "bulleted_list" | "numbered_list" | "todo")
        && open.kind == "paragraph"
        && !open.had_blank
        && (text.is_empty() || data.get("start").is_some());
      if kind == "paragraph" || weak_item {
        if open.had_blank {
          if col >= open.content_col && !open.raw.is_empty() && !st.item_children {
            open.had_blank = false;
            open.raw.push_str("\n\n");
            open.raw.push_str(body);
            data_insert(&mut open.base, "loose", json!(true));
            st.pending_loose = false;
            let (joined, joined_data) =
              apply_inline_marks(open.raw.clone(), open.base.clone(), &defs);
            blocks[open.block_idx].text = joined;
            blocks[open.block_idx].data = joined_data;
            index += 1;
            continue;
          }
          // The blank closed the DEEP item — but the line may still sit
          // inside an ANCESTOR item's content column: it becomes that
          // item's continuation paragraph, rendered after the sublist
          // (spec ex. 325). Flat model: a paragraph child (`data.li`).
          if kind == "paragraph"
            && let Some(anc) = st.list_stack.iter().rposition(|&cc| col >= cc)
            && anc + 1 < st.list_stack.len()
          {
            st.list_stack.truncate(anc + 1);
            // Owner: the last list block at that level.
            let owner = blocks.iter().rposition(|b| {
              list_tag_for(&b.kind).is_some()
                && li_of(b).is_none()
                && list_indent(b) == anc
            });
            if let Some(owner) = owner {
              data_insert(&mut blocks[owner].data, "loose", json!(true));
              let (pt, mut pd) = apply_inline_marks(body.to_string(), Value::Null, &defs);
              data_insert(&mut pd, "li", json!(anc));
              push_block(&mut blocks, &mut root_children, "paragraph", pt, pd);
              st.pending_loose = false;
              st.item_children = true;
              st.open_item = None;
              index += 1;
              continue;
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
          let (joined, joined_data) =
            apply_inline_marks(open.raw.clone(), open.base.clone(), &defs);
          blocks[open.block_idx].text = joined;
          blocks[open.block_idx].data = joined_data;
          index += 1;
          continue;
        }
      }
    }

    // List/todo items: nesting level from the indentation stack, loose and
    // <ol start> bookkeeping, then the item stays open for continuations.
    if matches!(kind, "bulleted_list" | "numbered_list" | "todo") {
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
      let mut kind = kind;
      let mut text = text;
      let mut data = data;
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
        while st.list_stack.last().is_some_and(|&cc| col < cc) {
          st.list_stack.pop();
        }
        let level = st.list_stack.len();
        let ccol = col + marker_width + extra;
        st.list_stack.push(ccol);
        let mut idata = Value::Null;
        if level > 0 {
          data_insert(&mut idata, "indent", json!(level));
        }
        if kind == "numbered_list"
          && let Some(st) = data.get("start")
        {
          data_insert(&mut idata, "start", st.clone());
        }
        if st.pending_loose {
          data_insert(&mut idata, "loose", json!(true));
          st.pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, kind, String::new(), idata);
        st.last_list = Some((blocks.len() - 1, level, marker_char_of(kind, &content_owned)));
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
      while st.list_stack.last().is_some_and(|&cc| col < cc) {
        st.list_stack.pop();
      }
      let level = st.list_stack.len();
      st.list_stack.push(content_col);
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
      let continues_run = st.last_list.is_some_and(|(idx, lv, mk)| {
        lv == level && blocks[idx].kind == kind && mk == marker_char
      });
      let same_level_break = !continues_run
        && st.last_list.is_some_and(|(idx, lv, _)| lv == level && blocks[idx].kind == kind);
      if same_level_break {
        // Record the changed bullet/delimiter so export keeps the break.
        data_insert(&mut data, "marker", json!(marker_char.to_string()));
      }
      // The blank line belongs to whichever list the boundary sits in:
      // same-or-shallower level → this item is loose; deeper level → the
      // blank separated a parent's text from its sublist, so the parent is.
      if st.pending_loose {
        match st.last_list {
          Some((prev_idx, prev_level, _)) if level > prev_level => {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          _ => data_insert(&mut data, "loose", json!(true)),
        }
        st.pending_loose = false;
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
        let (item_text, item_data) = apply_inline_marks(String::new(), data, &defs);
        push_block(&mut blocks, &mut root_children, kind, item_text, item_data);
        let block_idx = blocks.len() - 1;
        st.last_list = Some((block_idx, level, marker_char));
        while st.list_stack.len() > level {
          st.list_stack.pop();
        }
        st.list_stack.push(content_col);
        st.item_children = true;
        st.open_item = None;
        if let Some((fence_char, fence_len, info)) = starts_fence {
          let language =
            unescape_md(info.split_whitespace().next().unwrap_or_default());
          let mut code_lines = Vec::new();
          let mut closed = false;
          index += 1;
          while index < raw_lines.len() {
            let l = raw_lines[index];
            let lt = l.trim_start();
            if lt.is_empty() {
              code_lines.push(String::new());
              index += 1;
              continue;
            }
            if leading_columns(l) < content_col {
              break;
            }
            if fence_close(lt, fence_char, fence_len) {
              closed = true;
              index += 1;
              break;
            }
            code_lines.push(deindent_columns(l, content_col));
            index += 1;
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
          push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), cdata);
          continue;
        }
        if starts_divider {
          push_block(
            &mut blocks,
            &mut root_children,
            "divider",
            String::new(),
            json!({ "li": level }),
          );
          index += 1;
          continue;
        }
        if let Some(hl) = starts_heading {
          let htext = strip_atx_closing(text[hl..].trim_start()).to_string();
          let (htext, mut hdata) = apply_inline_marks(htext, json!({ "level": hl }), &defs);
          data_insert(&mut hdata, "li", json!(level));
          push_block(&mut blocks, &mut root_children, "heading", htext, hdata);
          index += 1;
          continue;
        }
        // starts_code: the kept extra spaces are an indented code child.
        push_block(
          &mut blocks,
          &mut root_children,
          "code_block",
          deindent_columns(&text, 4),
          json!({ "li": level }),
        );
        index += 1;
        continue;
      }
      let raw = text.clone();
      let base = data.clone();
      let (text, data) = apply_inline_marks(text, data, &defs);
      push_block(&mut blocks, &mut root_children, kind, text, data);
      let block_idx = blocks.len() - 1;
      st.last_list = Some((block_idx, level, marker_char));
      st.item_children = false;
      st.open_item = Some(OpenItem {
        block_idx,
        kind: kind.to_string(),
        content_col,
        raw,
        base,
        had_blank: false,
        qdepth: 0,
      });
      index += 1;
      continue;
    }

    st.list_stack.clear();
    st.last_list = None;
    st.pending_loose = false;
    st.item_children = false;
    let (text, data) = if kind == "image" {
      // The alt is plain text — inline markup flattens (spec alt rule).
      (parse_inline_with(&text, &defs).text, data)
    } else {
      apply_inline_marks(text, data, &defs)
    };
    let raw = if kind == "paragraph" {
      Some(body.to_string())
    } else {
      None
    };
    push_block(&mut blocks, &mut root_children, kind, text, data);
    // Paragraphs stay open for lazy continuation and setext underlines.
    st.open_item = raw.map(|raw| OpenItem {
      block_idx: blocks.len() - 1,
      kind: "paragraph".to_string(),
      content_col: 0,
      raw,
      base: Value::Null,
      had_blank: false,
      qdepth: 0,
    });
    index += 1;
  }

  if let Some(d) = st.pending_empty_quote.take() {
    let mut data = if d > 1 { json!({ "quote": d }) } else { Value::Null };
    if st.quote_boundary {
      data_insert(&mut data, "qbreak", json!(true));
    }
    if let Some(a) = st.pending_alert.take() {
      data_insert(&mut data, "alert", json!(a));
    }
    push_block(&mut blocks, &mut root_children, "quote", String::new(), data);
  }

  // Promote a paragraph that is exactly one image (e.g. a reference-form
  // `![alt][label]` on its own line) to an image block; markup inside the
  // alt flattens, the same as the direct `![alt](url)` fast path.
  for b in &mut blocks {
    if b.kind != "paragraph" {
      continue;
    }
    let full = b.text.encode_utf16().count();
    if full == 0 {
      continue;
    }
    let marks = b.data.get("marks").and_then(Value::as_array);
    // A whole-line image promotes to a standalone image block — unless it is
    // wrapped in a link (`[![alt](img)](url)`), which must stay an inline
    // anchored image inside the paragraph.
    let wrapped_in_link = marks.is_some_and(|ms| {
      ms.iter().any(|m| {
        m.get("type").and_then(Value::as_str) == Some("link")
          && m.get("start").and_then(Value::as_u64) == Some(0)
          && m.get("end").and_then(Value::as_u64) == Some(full as u64)
      })
    });
    let promoted = (!wrapped_in_link)
      .then_some(marks)
      .flatten()
      .and_then(|ms| {
        ms.iter().find(|m| {
          m.get("type").and_then(Value::as_str) == Some("image")
            && m.get("start").and_then(Value::as_u64) == Some(0)
            && m.get("end").and_then(Value::as_u64) == Some(full as u64)
        })
      });
    if let Some(image) = promoted {
      let url = image.get("href").and_then(Value::as_str).unwrap_or_default();
      let mut data = json!({ "url": url });
      if let Some(t) = image.get("title").and_then(Value::as_str) {
        data_insert(&mut data, "title", json!(t));
      }
      b.kind = "image".to_string();
      b.data = data;
    }
  }

  let mut root_data = Value::Null;
  if let Some(fm) = front_matter {
    data_insert(&mut root_data, "front_matter", json!(fm));
  }
  let root = Block {
    id: root_block_id.to_string(),
    kind: "paragraph".to_string(),
    text: String::new(),
    data: root_data,
    children: root_children,
  };
  let mut all_blocks = Vec::with_capacity(blocks.len() + 1);
  all_blocks.push(root);
  all_blocks.extend(blocks);

  DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id: root_block_id.to_string(),
    blocks: all_blocks,
  }
}

/// Detect a leading YAML front matter block: the *first* line is exactly `---`
/// and some later line is exactly `---` or `...`. Returns `(Some(inner), body)`
/// where `inner` is the verbatim text between the fences (fence lines excluded,
/// no surrounding newlines) and `body` is the remaining Markdown after the
/// close fence. When there's no well-formed front matter the whole input is the
/// body — an unterminated `---` is then just an ordinary first line, so the
/// parser will treat it as a thematic break / setext underline as usual.
pub(crate) fn split_front_matter(markdown: &str) -> (Option<String>, &str) {
  // The opener must own the entire first line (no indentation, no trailing
  // content); `---x` or ` ---` are not front matter.
  let mut lines = markdown.split('\n');
  let first = lines.next().unwrap_or_default();
  if first.trim_end_matches('\r') != "---" {
    return (None, markdown);
  }

  // Byte cursor advances line by line so we can slice the body verbatim
  // (split('\n') drops the separators we still need to account for).
  let inner_start = first.len() + 1; // past the opener + its '\n'
  let mut cursor = inner_start;
  for line in lines {
    let stripped = line.trim_end_matches('\r');
    if stripped == "---" || stripped == "..." {
      // inner = everything between opener and this close fence, minus the
      // trailing '\n' that precedes the fence (none for an empty body).
      let inner_end = cursor.saturating_sub(1).max(inner_start);
      let inner = &markdown[inner_start..inner_end.min(markdown.len())];
      // Body starts after the close fence line and its '\n' (if any).
      let after_fence = cursor + line.len();
      let body = if after_fence < markdown.len() {
        &markdown[after_fence + 1..]
      } else {
        ""
      };
      return (Some(inner.to_string()), body);
    }
    cursor += line.len() + 1; // line plus the '\n' that split() consumed
  }

  // No close fence — not front matter; let the parser see the raw text.
  (None, markdown)
}

/// A Markdown thematic break: 3+ of the same `-`, `*`, or `_` (already trimmed).
pub(crate) fn is_divider(content: &str) -> bool {
  // Spec thematic break: 3+ of the same -/*/_ with optional spaces/tabs
  // between (`- - -`, `_  _  _`).
  let mut marker = 0u8;
  let mut count = 0usize;
  for b in content.bytes() {
    match b {
      b' ' | b'\t' => {}
      b'-' | b'*' | b'_' => {
        if marker == 0 {
          marker = b;
        } else if b != marker {
          return false;
        }
        count += 1;
      }
      _ => return false,
    }
  }
  count >= 3
}

/// Leading indentation of a line in columns (tabs = 4-column stops).
/// How many columns one leading whitespace character is worth, or None when the
/// character is not indentation at all.
///
/// THE single answer to "how far is this line indented". Three functions used
/// to answer it separately — `column_of`, `leading_columns` and
/// `deindent_columns` — and they disagreed about U+00A0: `trim_start()` (which
/// decides a line *is* indented, via `char::is_whitespace`) counted it, these
/// did not. A no-break-space-indented paragraph after a list therefore looked
/// indented to the dispatcher and unindented to the measurement, the loop
/// re-fed itself the same line forever, and `import_markdown` allocated until
/// the process aborted — taking the desktop app down mid-import. Thirty bytes
/// did it, and CJK documents indent with NBSP routinely.
///
/// Keep every indent measurement going through here. A fourth copy is how this
/// comes back.
pub(crate) fn indent_width(c: char, col: usize) -> Option<usize> {
  match c {
    ' ' | '\u{a0}' => Some(col + 1),
    '\t' => Some((col / 4 + 1) * 4),
    _ => None,
  }
}

pub(crate) fn leading_columns(line: &str) -> usize {
  let mut col = 0usize;
  for c in line.chars() {
    match indent_width(c, col) {
      Some(next) => col = next,
      None => break,
    }
  }
  col
}

/// Strip [columns] of leading indentation, treating tabs as 4-column stops
/// (a tab that overshoots emits the remainder as spaces).
pub(crate) fn deindent_columns(line: &str, columns: usize) -> String {
  let mut col = 0usize;
  for (i, c) in line.char_indices() {
    match indent_width(c, col) {
      Some(next) => col = next,
      None => return line[i..].to_string(),
    }
    if col >= columns {
      let mut rest = " ".repeat(col - columns);
      rest.push_str(&line[i + c.len_utf8()..]);
      return rest;
    }
  }
  String::new()
}

/// The visual column where a line's content begins (tabs to 4-stops).
pub(crate) fn column_of(line: &str) -> usize {
  let mut col = 0usize;
  for c in line.chars() {
    match indent_width(c, col) {
      Some(next) => col = next,
      None => break,
    }
  }
  col
}

/// A GFM footnote definition leader `[^label]: rest`. Returns the label (no
/// caret) and the remaining first-line content (may be empty). The label
/// holds no whitespace or brackets — same shape as the inline reference.
pub(crate) fn parse_footnote_def(content: &str) -> Option<(String, String)> {
  let chars: Vec<char> = content.chars().collect();
  if chars.first() != Some(&'[') || chars.get(1) != Some(&'^') {
    return None;
  }
  let close = matching_bracket(&chars, 0)?;
  if close < 3 || chars.get(close + 1) != Some(&':') {
    return None;
  }
  let label: String = chars[2..close].iter().collect();
  if label.is_empty()
    || label.chars().any(|c| c.is_whitespace() || matches!(c, '[' | ']' | '^'))
  {
    return None;
  }
  let rest: String = chars[close + 2..].iter().collect();
  Some((label, rest.trim_start().to_string()))
}

/// `===`/`---` underline (≤3 leading spaces) → setext heading level, if any.
pub(crate) fn setext_level(line: &str) -> Option<usize> {
  let lead = line.len() - line.trim_start().len();
  if lead > 3 {
    return None;
  }
  let t = line.trim();
  if t.is_empty() {
    return None;
  }
  if t.bytes().all(|b| b == b'=') {
    return Some(1);
  }
  if t.bytes().all(|b| b == b'-') {
    return Some(2);
  }
  None
}

pub(crate) fn is_table_separator(line: &str) -> bool {
  let cells = split_table_row(line);
  !cells.is_empty()
    && cells.iter().all(|cell| {
      let t = cell.trim();
      !t.is_empty() && t.contains('-') && t.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')
    })
}

pub(crate) fn split_table_row(line: &str) -> Vec<String> {
  let mut s = line.trim();
  s = s.strip_prefix('|').unwrap_or(s);
  s = s.strip_suffix('|').unwrap_or(s);
  let mut cells = Vec::new();
  let mut buffer = String::new();
  let mut chars = s.chars().peekable();
  while let Some(ch) = chars.next() {
    if ch == '\\' && chars.peek() == Some(&'|') {
      buffer.push('|');
      chars.next();
    } else if ch == '|' {
      cells.push(buffer.trim().to_string());
      buffer.clear();
    } else {
      buffer.push(ch);
    }
  }
  cells.push(buffer.trim().to_string());
  cells
}

/// Map a single non-empty, non-fence Markdown line to a `(kind, text, data)`
/// triple for block construction.
pub(crate) fn classify_markdown_line(content: &str) -> (&'static str, String, Value) {
  if let Some(level) = heading_prefix_level(content) {
    let text = strip_atx_closing(content[level..].trim_start()).to_string();
    return ("heading", text, json!({ "level": level }));
  }
  if let Some(rest) = content.strip_prefix("- [ ] ") {
    return ("todo", rest.to_string(), json!({ "checked": false }));
  }
  if let Some(rest) = content
    .strip_prefix("- [x] ")
    .or_else(|| content.strip_prefix("- [X] "))
  {
    return ("todo", rest.to_string(), json!({ "checked": true }));
  }
  if let Some((alt, url, title)) = parse_markdown_image(content) {
    let data = match title {
      Some(t) => json!({ "url": url, "title": t }),
      None => json!({ "url": url }),
    };
    return ("image", alt, data);
  }
  if let Some(rest) = content
    .strip_prefix("- ")
    .or_else(|| content.strip_prefix("* "))
    .or_else(|| content.strip_prefix("+ "))
  {
    return ("bulleted_list", rest.to_string(), Value::Null);
  }
  if matches!(content, "-" | "*" | "+") {
    // A bare marker is an empty list item.
    return ("bulleted_list", String::new(), Value::Null);
  }
  if let Some((start, rest)) = numbered_list_marker(content) {
    let data = if start == 1 {
      Value::Null
    } else {
      json!({ "start": start })
    };
    return ("numbered_list", rest.to_string(), data);
  }
  if let Some(rest) = content.strip_prefix("> ") {
    return ("quote", rest.to_string(), Value::Null);
  }

  ("paragraph", content.to_string(), Value::Null)
}

pub(crate) fn push_block(
  blocks: &mut Vec<Block>,
  root_children: &mut Vec<String>,
  kind: &str,
  text: String,
  data: Value,
) {
  let id = format!("block_{}", Uuid::new_v4().simple());
  root_children.push(id.clone());
  blocks.push(Block {
    id,
    kind: kind.to_string(),
    text,
    data,
    children: Vec::new(),
  });
}

pub(crate) fn heading_prefix_level(content: &str) -> Option<usize> {
  let hashes = content.chars().take_while(|&c| c == '#').count();
  if (1..=6).contains(&hashes)
    && (content.len() == hashes || content[hashes..].starts_with([' ', '\t']))
  {
    Some(hashes)
  } else {
    None
  }
}

/// Strip an ATX closing sequence: trailing `#`s preceded by a space (or
/// making up the whole text).
pub(crate) fn strip_atx_closing(text: &str) -> &str {
  let t = text.trim_end();
  let trimmed = t.trim_end_matches('#');
  if trimmed.len() == t.len() {
    return t;
  }
  if trimmed.is_empty() {
    return "";
  }
  match trimmed.strip_suffix(' ') {
    Some(rest) => rest.trim_end(),
    None => t,
  }
}

/// Ordered-list marker: 1–9 digits + `.` or `)` + space (or end-of-line for
/// an empty item). Returns the start number and the content after the marker.
pub(crate) fn numbered_list_marker(content: &str) -> Option<(u64, &str)> {
  let digits_end = content
    .find(|c: char| !c.is_ascii_digit())
    .unwrap_or(content.len());
  if digits_end == 0 || digits_end > 9 {
    return None;
  }
  let start: u64 = content[..digits_end].parse().ok()?;
  let rest = &content[digits_end..];
  match rest.as_bytes() {
    [] => None,
    [b'.'] | [b')'] => Some((start, "")),
    [b'.', b' ', ..] | [b')', b' ', ..] => Some((start, &rest[2..])),
    _ => None,
  }
}

/// Block-level HTML tag names (CommonMark type-6 start condition).
pub(crate) const HTML_BLOCK_TAGS: &[&str] = &[
  "address", "article", "aside", "base", "basefont", "blockquote", "body",
  "caption", "center", "col", "colgroup", "dd", "details", "dialog", "dir",
  "div", "dl", "dt", "fieldset", "figcaption", "figure", "footer", "form",
  "frame", "frameset", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header",
  "hr", "html", "iframe", "legend", "li", "link", "main", "menu", "menuitem",
  "nav", "noframes", "ol", "optgroup", "option", "p", "param", "search",
  "section", "summary", "table", "tbody", "td", "template", "tfoot", "th",
  "thead", "title", "tr", "track", "ul",
];

/// CommonMark HTML block start conditions. Returns the kind (1–7), which
/// picks the end condition; type 7 cannot interrupt a paragraph.
pub(crate) fn html_block_start(content: &str) -> Option<u8> {
  let rest = content.strip_prefix('<')?;
  if rest.starts_with("!--") {
    return Some(2);
  }
  if rest.starts_with('?') {
    return Some(3);
  }
  if rest.starts_with("![CDATA[") {
    return Some(5);
  }
  if let Some(d) = rest.strip_prefix('!') {
    if d.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
      return Some(4);
    }
    return None;
  }
  let (closing, name_rest) = match rest.strip_prefix('/') {
    Some(r) => (true, r),
    None => (false, rest),
  };
  let name_len = name_rest
    .chars()
    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
    .count();
  if name_len == 0 || !name_rest.chars().next().unwrap().is_ascii_alphabetic() {
    return None;
  }
  let name: String = name_rest[..name_len].to_ascii_lowercase();
  let after = &name_rest[name_len..];
  let boundary = after.is_empty() || after.starts_with([' ', '\t', '>']);
  if !closing
    && matches!(name.as_str(), "pre" | "script" | "style" | "textarea")
    && boundary
  {
    return Some(1);
  }
  if HTML_BLOCK_TAGS.contains(&name.as_str())
    && (boundary || (!closing && after.starts_with("/>")))
  {
    return Some(6);
  }
  // Type 7: a COMPLETE open/closing tag (any other name) alone on the line.
  let chars: Vec<char> = content.chars().collect();
  if let Some(end) = inline_html_end(&chars, 0)
    && chars[1] != '!'
    && chars[1] != '?'
    && !matches!(name.as_str(), "pre" | "script" | "style" | "textarea")
    && chars[end..].iter().all(|&c| c == ' ' || c == '\t')
  {
    return Some(7);
  }
  None
}

/// Does this line end the HTML block of the given kind (types 1–5)?
pub(crate) fn html_block_ends(kind: u8, line: &str) -> bool {
  let lower = line.to_ascii_lowercase();
  match kind {
    1 => {
      lower.contains("</pre>")
        || lower.contains("</script>")
        || lower.contains("</style>")
        || lower.contains("</textarea>")
    }
    2 => line.contains("-->"),
    3 => line.contains("?>"),
    4 => line.contains('>'),
    5 => line.contains("]]>"),
    _ => false,
  }
}

/// Inline raw HTML at chars[i] == '<': open tag, closing tag, comment,
/// processing instruction, declaration or CDATA. Returns the exclusive end.
pub(crate) fn inline_html_end(chars: &[char], i: usize) -> Option<usize> {
  if chars.get(i) != Some(&'<') {
    return None;
  }
  let starts = |from: usize, pat: &str| -> bool {
    pat.chars().enumerate().all(|(k, c)| chars.get(from + k) == Some(&c))
  };
  let find = |from: usize, pat: &str| -> Option<usize> {
    let pat: Vec<char> = pat.chars().collect();
    let mut j = from;
    while j + pat.len() <= chars.len() {
      if chars[j..j + pat.len()] == pat[..] {
        return Some(j);
      }
      j += 1;
    }
    None
  };
  // Comment (<!--> and <!---> count as empty comments).
  if starts(i + 1, "!--") {
    if starts(i + 4, ">") {
      return Some(i + 5);
    }
    if starts(i + 4, "->") {
      return Some(i + 6);
    }
    return find(i + 4, "-->").map(|p| p + 3);
  }
  if starts(i + 1, "![CDATA[") {
    return find(i + 9, "]]>").map(|p| p + 3);
  }
  if chars.get(i + 1) == Some(&'?') {
    return find(i + 2, "?>").map(|p| p + 2);
  }
  if chars.get(i + 1) == Some(&'!') {
    if !chars.get(i + 2).is_some_and(|c| c.is_ascii_alphabetic()) {
      return None;
    }
    return find(i + 2, ">").map(|p| p + 1);
  }
  let (closing, mut p) = if chars.get(i + 1) == Some(&'/') {
    (true, i + 2)
  } else {
    (false, i + 1)
  };
  if !chars.get(p).is_some_and(|c| c.is_ascii_alphabetic()) {
    return None;
  }
  while chars.get(p).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '-') {
    p += 1;
  }
  let skip_ws = |mut q: usize| -> usize {
    while chars.get(q).is_some_and(|c| c.is_whitespace()) {
      q += 1;
    }
    q
  };
  if closing {
    let q = skip_ws(p);
    return (chars.get(q) == Some(&'>')).then_some(q + 1);
  }
  loop {
    let q = skip_ws(p);
    match chars.get(q) {
      Some('>') => return Some(q + 1),
      Some('/') if chars.get(q + 1) == Some(&'>') => return Some(q + 2),
      _ => {}
    }
    if q == p {
      return None; // an attribute needs whitespace before it
    }
    // Attribute name.
    if !chars.get(q).is_some_and(|c| c.is_ascii_alphabetic() || *c == '_' || *c == ':') {
      return None;
    }
    let mut r = q + 1;
    while chars
      .get(r)
      .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
    {
      r += 1;
    }
    // Optional value.
    let eq = skip_ws(r);
    if chars.get(eq) == Some(&'=') {
      let v = skip_ws(eq + 1);
      match chars.get(v) {
        Some(&quote) if quote == '"' || quote == '\'' => {
          let mut w = v + 1;
          while chars.get(w).is_some_and(|&c| c != quote) {
            w += 1;
          }
          chars.get(w)?;
          p = w + 1;
        }
        Some(_) => {
          let mut w = v;
          while chars
            .get(w)
            .is_some_and(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '=' | '<' | '>' | '`'))
          {
            w += 1;
          }
          if w == v {
            return None;
          }
          p = w;
        }
        None => return None,
      }
    } else {
      p = r;
    }
  }
}

/// A fence opener: 3+ backticks or tildes; a backtick fence's info string
/// may not contain backticks. Returns (fence char, run length, info).
pub(crate) fn fence_open(content: &str) -> Option<(u8, usize, &str)> {
  let b = content.as_bytes();
  let c = *b.first()?;
  if c != b'`' && c != b'~' {
    return None;
  }
  let mut n = 0;
  while n < b.len() && b[n] == c {
    n += 1;
  }
  if n < 3 {
    return None;
  }
  let info = &content[n..];
  if c == b'`' && info.contains('`') {
    return None;
  }
  Some((c, n, info))
}

/// A fence closer: a run of the opening char at least as long, nothing else.
pub(crate) fn fence_close(content: &str, fence_char: u8, fence_len: usize) -> bool {
  let b = content.trim_end().as_bytes();
  b.len() >= fence_len && b.iter().all(|&x| x == fence_char)
}

/// Strip leading `>` quote markers: each consumes one optional following
/// space, and up to 3 spaces may precede the next nested marker. Returns the
/// marker depth and the remaining content.
/// A `>` / list marker followed by a TAB: expand the marker-trailing
/// whitespace into spaces using ABSOLUTE columns (tabs advance to 4-column
/// stops from line start), so the byte-oriented marker/indent logic sees
/// what the spec sees (ex. 6: `>\t\tfoo` is code "  foo" in a quote; ex. 7:
/// `-\t\tfoo` is code in a list item). Other lines pass through untouched.
pub(crate) fn expand_marker_tabs(line: &str) -> String {
  let b = line.as_bytes();
  let mut out = String::new();
  let mut col = 0usize;
  let mut i = 0usize;
  let mut changed = false;
  loop {
    // leading spaces
    let ws_start = i;
    while i < b.len() && b[i] == b' ' {
      i += 1;
      col += 1;
    }
    // marker: `>` (chainable) or a single list marker
    // `>` and a single bullet marker are different constructs that happen to be
    // the same width; one arm, not two identical ones.
    let marker_len = if b.get(i) == Some(&b'>')
      || (matches!(b.get(i), Some(b'-' | b'*' | b'+'))
        && matches!(b.get(i + 1), Some(b'\t' | b' ')))
    {
      1
    } else if b.get(i).is_some_and(u8::is_ascii_digit) {
      let mut j = i;
      while j < b.len() && b[j].is_ascii_digit() && j - i < 9 {
        j += 1;
      }
      if matches!(b.get(j), Some(b'.' | b')'))
        && matches!(b.get(j + 1), Some(b'\t' | b' ')) {
        j + 1 - i
      } else {
        0
      }
    } else {
      0
    };
    if marker_len == 0 {
      break;
    }
    out.push_str(&line[ws_start..i + marker_len]);
    i += marker_len;
    col += marker_len;
    // expand the whitespace run after the marker if it contains a tab
    let run_start = i;
    let mut run_has_tab = false;
    let mut run_col = col;
    while i < b.len() && matches!(b[i], b' ' | b'\t') {
      if b[i] == b'\t' {
        run_has_tab = true;
        run_col = (run_col / 4 + 1) * 4;
      } else {
        run_col += 1;
      }
      i += 1;
    }
    if run_has_tab {
      for _ in col..run_col {
        out.push(' ');
      }
      changed = true;
    } else {
      out.push_str(&line[run_start..i]);
    }
    col = run_col;
    // only `>` chains; a list marker ends the scan
    if b[run_start.saturating_sub(marker_len)] != b'>' {
      break;
    }
  }
  if !changed {
    return line.to_string();
  }
  out.push_str(&line[i..]);
  out
}

pub(crate) fn strip_quote_markers(content: &str) -> (usize, &str) {
  let b = content.as_bytes();
  let mut i = 0;
  let mut depth = 0;
  loop {
    let mut j = i;
    let mut spaces = 0;
    while j < b.len() && b[j] == b' ' {
      j += 1;
      spaces += 1;
    }
    if spaces > 3 || j >= b.len() || b[j] != b'>' {
      break;
    }
    j += 1;
    if j < b.len() && b[j] == b' ' {
      j += 1;
    }
    depth += 1;
    i = j;
  }
  (depth, &content[i..])
}

/// Quote nesting depth of a block: the `quote` kind is depth ≥ 1, any other
/// kind inside a quote carries `data.quote`.
pub(crate) fn quote_depth_of(block: &Block) -> usize {
  let d = block.data.get("quote").and_then(Value::as_u64).unwrap_or(0) as usize;
  if block.kind == "quote" { d.max(1) } else { d }
}

/// GFM alert (callout) type of a blockquote's first line: `[!NOTE]` and the
/// four siblings, case-insensitive per the GitHub spec, with the line
/// carrying nothing but the marker. `None` for any other line — a plain
/// blockquote is unchanged. Only these 5 standard kinds round-trip to a
/// portable `> [!TYPE]` marker, so no custom titles/icons (a dialect with no
/// standard representation).
pub(crate) fn alert_type_of(line: &str) -> Option<&'static str> {
  let inner = line.trim().strip_prefix("[!")?.strip_suffix(']')?;
  match inner.to_ascii_lowercase().as_str() {
    "note" => Some("note"),
    "tip" => Some("tip"),
    "important" => Some("important"),
    "warning" => Some("warning"),
    "caution" => Some("caution"),
    _ => None,
  }
}

/// Capitalized label for an alert type (`note` → `Note`), the visible title
/// GitHub renders on the callout.
pub(crate) fn alert_title(kind: &str) -> &'static str {
  match kind {
    "tip" => "Tip",
    "important" => "Important",
    "warning" => "Warning",
    "caution" => "Caution",
    _ => "Note",
  }
}

/// Insert a key into a block's `data`, upgrading `Null` to an object.
pub(crate) fn data_insert(data: &mut Value, key: &str, value: Value) {
  match data {
    Value::Object(map) => {
      map.insert(key.into(), value);
    }
    other => *other = json!({ key: value }),
  }
}

/// Parse a line that is exactly `![alt](url "title")` into
/// `(alt, url, title)` — spec destination rules (angle brackets, balanced
/// parens, nested brackets in the alt).
pub(crate) fn parse_markdown_image(content: &str) -> Option<(String, String, Option<String>)> {
  let chars: Vec<char> = content.chars().collect();
  if chars.len() < 2 || chars[0] != '!' || chars[1] != '[' {
    return None;
  }
  let close = matching_bracket(&chars, 1)?;
  if chars.get(close + 1) != Some(&'(') {
    return None;
  }
  let (href, title, next) = parse_link_suffix(&chars, close + 2)?;
  if next != chars.len() {
    return None; // not the whole line — leave it to the inline parser
  }
  let alt: String = chars[2..close].iter().collect();
  Some((alt, href, title))
}


