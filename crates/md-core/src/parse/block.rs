use std::ops::ControlFlow;

use serde_json::{Value, json};

use crate::block_handlers::*;
use crate::html::*;
use crate::ids::BlockIds;
use crate::inline::*;
use crate::scan_memo::ScanMemo;
use crate::{Block, DocumentSnapshotPayload};

/// The block scanner's mutable state — everything `import_markdown_inner`
/// carries ACROSS loop iterations. Every field has an owner (the construct
/// family that writes it) and an invariant documented at the field; the
/// decomposition (docs/reviews/code-optimization/OPTIMIZATION_PLAN_ROUND3.md §8)
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
  /// Blocks whose `text` holds RAW continued source awaiting one inline parse
  /// at the end of the scan (`block_handlers::defer_inline`).
  pub(crate) stale_inline: std::collections::BTreeSet<usize>,
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
      stale_inline: std::collections::BTreeSet::new(),
    }
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
      self.pending_empty_quote.is_none_or(|d| d >= 1),
      "pending_empty_quote carries the quote depth (>= 1)"
    );
  }

  /// The block-pointer invariants, checked against the live accumulators:
  /// every state slot that names a block index must name a real block of
  /// the kind it claims. A handler that pushes and forgets to update the
  /// slot (or updates it to a stale index) trips here, in debug, on the
  /// first corpus input that exercises it — not in a user's document.
  pub(crate) fn debug_validate_blocks(&self, blocks: &[Block]) {
    if let Some(open) = &self.open_item {
      debug_assert!(
        open.block_idx < blocks.len(),
        "open_item points past the block list"
      );
      debug_assert!(
        match open.kind.as_str() {
          "quote" => blocks[open.block_idx].kind == "quote",
          "paragraph" => blocks[open.block_idx].kind == "paragraph",
          "bulleted_list" | "numbered_list" | "todo" =>
            list_tag_for(&blocks[open.block_idx].kind).is_some(),
          other => {
            debug_assert!(false, "open_item carries an unknown kind {other}");
            true
          }
        },
        "open_item kind {} disagrees with block {}",
        open.kind,
        blocks[open.block_idx].kind
      );
    }
    if let Some((idx, _level, marker)) = self.last_list {
      debug_assert!(idx < blocks.len(), "last_list points past the block list");
      debug_assert!(
        list_tag_for(&blocks[idx].kind).is_some(),
        "last_list points at a non-list block"
      );
      debug_assert!(
        blocks[idx]
          .data
          .get("marker")
          .and_then(Value::as_str)
          .map(|m| m.starts_with(marker))
          .unwrap_or(true),
        "last_list marker disagrees with the block's recorded marker"
      );
    }
  }
}

pub(crate) fn import_markdown_inner(
  markdown: &str,
  root_block_id: &str,
  parse_front_matter: bool,
  ids: &BlockIds,
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
    st.debug_validate_blocks(&blocks);
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
      blank_line(&mut ScanCtx {
        lines: &raw_lines,
        blocks: &mut blocks,
        root_children: &mut root_children,
        defs: &defs,
        index: &mut index,
        st: &mut st,
      });
      continue;
    }
    let col = leading_columns(line);

    // The families in ORDER — this is the precedence order of the old loop
    // body, and order IS semantics here.
    macro_rules! family {
      ($name:ident) => {
        if $name(
          &mut ScanCtx {
            lines: &raw_lines,
            blocks: &mut blocks,
            root_children: &mut root_children,
            defs: &defs,
            index: &mut index,
            st: &mut st,
          },
          line,
          content,
          body,
          col,
        )
        .is_some()
        {
          continue;
        }
      };
    }
    family!(lazy_code_continuation);
    family!(setext_paragraph);
    family!(list_container_children);
    family!(fenced_code);
    family!(math_block);
    family!(footnote_def);
    family!(html_block);
    family!(table);
    family!(indented_code_and_divider);
    family!(quote_group);

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

    let mut ctx = ScanCtx {
      lines: &raw_lines,
      blocks: &mut blocks,
      root_children: &mut root_children,
      defs: &defs,
      index: &mut index,
      st: &mut st,
    };
    // Continuation (CommonMark): a paragraph line joins the open block with
    // a soft break (lazy lines included); after a blank, a line indented to
    // the item's content column starts a second paragraph inside the item
    // (the list turns loose). An empty list item or an ordered marker other
    // than `1.` cannot interrupt a paragraph — those lines stay text.
    // The classify result carries through the remaining families.
    let c = ClassifiedLine { kind, text, data };
    if open_item_continuations(&mut ctx, body, col, &c).is_some() {
      continue;
    }
    let ControlFlow::Continue(c) = list_item(&mut ctx, content, col, c) else {
      continue;
    };
    leaf_block(&mut ctx, body, c);
  }
  crate::block_handlers::finish_deferred_inline(
    &mut blocks,
    std::mem::take(&mut st.stale_inline),
    &defs,
  );
  if let Some(d) = st.pending_empty_quote.take() {
    let mut data = if d > 1 {
      json!({ "quote": d })
    } else {
      Value::Null
    };
    if st.quote_boundary {
      data_insert(&mut data, "qbreak", json!(true));
    }
    if let Some(a) = st.pending_alert.take() {
      data_insert(&mut data, "alert", json!(a));
    }
    push_block(
      &mut blocks,
      &mut root_children,
      "quote",
      String::new(),
      data,
    );
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
    // The parser appends enclosing marks after their children. A full-range
    // link BEFORE the image belongs to its alt; one AFTER wraps the image and
    // must survive as an anchored image inside a paragraph.
    let promoted = marks.and_then(|ms| {
      let image_index = ms.iter().rposition(|m| {
        m.get("type").and_then(Value::as_str) == Some("image")
          && m.get("start").and_then(Value::as_u64) == Some(0)
          && m.get("end").and_then(Value::as_u64) == Some(full as u64)
      })?;
      let wrapped_in_link = ms[image_index + 1..].iter().any(|m| {
        m.get("type").and_then(Value::as_str) == Some("link")
          && m.get("start").and_then(Value::as_u64) == Some(0)
          && m.get("end").and_then(Value::as_u64) == Some(full as u64)
      });
      (!wrapped_in_link).then_some(&ms[image_index])
    });
    if let Some(image) = promoted {
      let url = image
        .get("href")
        .and_then(Value::as_str)
        .unwrap_or_default();
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
  assign_ids(&mut blocks, &mut root_children, ids);
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

/// Replace the parser's temporary ids (`\u{1}<index>`) with the caller's ids
/// and rewrite every `children` reference. Temporary ids are unique and never
/// appear in real documents, so they cannot collide with user data.
fn assign_ids(blocks: &mut [Block], root_children: &mut [String], ids: &BlockIds) {
  // Only `push_block` appends to both collections, in lockstep, so position IS
  // the block index. Generate each final id exactly once (`Uuid` is random) and
  // give the block the value while the root's child list gets the one clone the
  // exporter needs.
  debug_assert_eq!(blocks.len(), root_children.len());
  for (n, block) in blocks.iter_mut().enumerate() {
    let final_id = ids.id_for(n);
    if let Some(child) = root_children.get_mut(n) {
      *child = final_id.clone();
    }
    block.id = final_id;
  }
  // Defensive generality: the current parser only gives the root children, so
  // the common path skips building a map entirely.
  if blocks.iter().any(|b| !b.children.is_empty()) {
    let finals: Vec<String> = blocks.iter().map(|b| b.id.clone()).collect();
    let final_for = |temp: &str| -> Option<&String> {
      let n: usize = temp.strip_prefix('\u{1}')?.parse().ok()?;
      finals.get(n)
    };
    for block in blocks.iter_mut() {
      for child in &mut block.children {
        if let Some(mapped) = final_for(child) {
          *child = mapped.clone();
        }
      }
    }
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
    '\t' => Some((col / 4 + 1) * 4),
    c if c.is_whitespace() => Some(col + 1),
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
    || label
      .chars()
      .any(|c| c.is_whitespace() || matches!(c, '[' | ']' | '^'))
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
  // An EMPTY to-do exports as `- [ ] ` and the trailing space is trimmed off
  // the line before it gets here, so the bare marker must count too — it used
  // to fall through to a bullet whose text was `[ ]`, losing the kind and the
  // checked state.
  if content == "- [ ]" {
    return ("todo", String::new(), json!({ "checked": false }));
  }
  if content == "- [x]" || content == "- [X]" {
    return ("todo", String::new(), json!({ "checked": true }));
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
  // A temporary, deterministic id; `assign_ids` replaces it at the end of the
  // parse. `\u{1}` cannot appear in a real block id.
  let id = format!("\u{1}{}", blocks.len());
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
  "address",
  "article",
  "aside",
  "base",
  "basefont",
  "blockquote",
  "body",
  "caption",
  "center",
  "col",
  "colgroup",
  "dd",
  "details",
  "dialog",
  "dir",
  "div",
  "dl",
  "dt",
  "fieldset",
  "figcaption",
  "figure",
  "footer",
  "form",
  "frame",
  "frameset",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "head",
  "header",
  "hr",
  "html",
  "iframe",
  "legend",
  "li",
  "link",
  "main",
  "menu",
  "menuitem",
  "nav",
  "noframes",
  "ol",
  "optgroup",
  "option",
  "p",
  "param",
  "search",
  "section",
  "summary",
  "table",
  "tbody",
  "td",
  "template",
  "tfoot",
  "th",
  "thead",
  "title",
  "tr",
  "track",
  "ul",
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
  if !closing && matches!(name.as_str(), "pre" | "script" | "style" | "textarea") && boundary {
    return Some(1);
  }
  if HTML_BLOCK_TAGS.contains(&name.as_str()) && (boundary || (!closing && after.starts_with("/>")))
  {
    return Some(6);
  }
  // Type 7: a COMPLETE open/closing tag (any other name) alone on the line.
  let chars: Vec<char> = content.chars().collect();
  if let Some(end) = inline_html_end(&chars, 0, None)
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
///
/// `memo` (the inline parser passes one per parse) memoises the terminator
/// searches: an unclosed `<!--` used to rescan the whole suffix from every
/// later `<` (issue #61). A failed search means "no terminator at or after
/// `from`", and later starts only move forward, so the frontier stays valid.
pub(crate) fn inline_html_end(
  chars: &[char],
  i: usize,
  mut memo: Option<&mut ScanMemo>,
) -> Option<usize> {
  if chars.get(i) != Some(&'<') {
    return None;
  }
  let starts = |from: usize, pat: &str| -> bool {
    pat
      .chars()
      .enumerate()
      .all(|(k, c)| chars.get(from + k) == Some(&c))
  };
  let mut find = |from: usize, pat: &'static str| -> Option<usize> {
    if let Some(m) = memo.as_deref_mut() {
      let dead = if pat == ">" {
        Some(m.gt_dead_from)
      } else {
        m.html_dead.get(pat).copied()
      };
      if dead.is_some_and(|f| f <= from) {
        return None;
      }
    }
    let needle: Vec<char> = pat.chars().collect();
    let mut j = from;
    while j + needle.len() <= chars.len() {
      if chars[j..j + needle.len()] == needle[..] {
        return Some(j);
      }
      j += 1;
    }
    if let Some(m) = memo.as_deref_mut() {
      if pat == ">" {
        m.gt_dead_from = m.gt_dead_from.min(from);
      } else {
        let entry = m.html_dead.entry(pat).or_insert(usize::MAX);
        *entry = (*entry).min(from);
      }
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
  while chars
    .get(p)
    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '-')
  {
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
    if !chars
      .get(q)
      .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_' || *c == ':')
    {
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
      if matches!(b.get(j), Some(b'.' | b')')) && matches!(b.get(j + 1), Some(b'\t' | b' ')) {
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

/// Upper bound on every nesting depth stored in block data — `data.quote`,
/// `data.li`, `data.indent` — as READ by the renderers and exporters. Mirrors
/// the Dart editor's `kMaxNestingDepth` (`clients/mica_flutter/lib/editor/
/// model.dart`); change both together. Non-destructive: the stored value is
/// untouched, only the depth the engines act on is bound — so both engines
/// render the same data the same way, and a hostile value can't drive
/// `render_quote_group`/`render_html_list` recursion (stack overflow) or a
/// `" ".repeat(depth)` (multi-GB allocation). P1-6 / 10-19 #6a.
pub(crate) const MAX_NESTING_DEPTH: u64 = 8;

/// A depth stored in `data[key]`, clamped to `MAX_NESTING_DEPTH`. `None` when
/// the key is absent or not a non-negative integer.
pub(crate) fn data_depth(block: &Block, key: &str) -> Option<usize> {
  block
    .data
    .get(key)
    .and_then(Value::as_u64)
    .map(|v| v.min(MAX_NESTING_DEPTH) as usize)
}

/// Quote nesting depth of a block: the `quote` kind is depth ≥ 1, any other
/// kind inside a quote carries `data.quote`. Clamped (`MAX_NESTING_DEPTH`).
pub(crate) fn quote_depth_of(block: &Block) -> usize {
  let d = data_depth(block, "quote").unwrap_or(0);
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
