//! Mica's Markdown engine — the block model ("AST") plus parsing and
//! rendering between it and Markdown/HTML. Pure and I/O-free; document
//! *operations* (insert/update/move) live in `mica-app-core`, archive-level
//! interchange in `mica-interchange`. See docs/architecture.md.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentSnapshotPayload {
  pub schema_version: i32,
  pub root_block_id: String,
  pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
  pub id: String,
  #[serde(rename = "type")]
  pub kind: String,
  #[serde(default)]
  pub text: String,
  /// Block-specific attributes (image `url`, heading `level`, todo `checked`).
  /// Defaults to `null` and is omitted from output when empty for compactness.
  #[serde(default, skip_serializing_if = "Value::is_null")]
  pub data: Value,
  #[serde(default)]
  pub children: Vec<String>,
}


#[derive(Debug, thiserror::Error)]
pub enum DocumentOperationError {
  #[error("unsupported document schema version: {0}")]
  UnsupportedSchemaVersion(i32),

  #[error("block not found: {0}")]
  BlockNotFound(String),

  #[error("block already exists: {0}")]
  BlockAlreadyExists(String),

  #[error("root block cannot be deleted or moved")]
  CannotMoveRoot,

  #[error("parent block cannot be a descendant of the moved block")]
  ParentIsDescendant,

  #[error("block type is required")]
  EmptyBlockType,

  #[error("block id is required")]
  EmptyBlockId,

  #[error("a newly inserted block must be a leaf (empty children); attach children with their own inserts")]
  InsertBlockWithChildren,
}

pub type DocumentOperationResult<T> = Result<T, DocumentOperationError>;



pub fn payload_from_value(value: Value) -> DocumentOperationResult<DocumentSnapshotPayload> {
  let payload = serde_json::from_value::<DocumentSnapshotPayload>(value)
    .map_err(|_| DocumentOperationError::UnsupportedSchemaVersion(0))?;
  if payload.schema_version != 1 {
    return Err(DocumentOperationError::UnsupportedSchemaVersion(
      payload.schema_version,
    ));
  }

  Ok(payload)
}


pub fn export_markdown(snapshot: &DocumentSnapshotPayload) -> DocumentOperationResult<String> {
  export_markdown_with_assets(snapshot, &BTreeMap::new())
}

/// Like [export_markdown] but rewrites each image block whose `file_id` is in
/// [images] to that asset path (e.g. `assets/photo.png`) — used for ZIP export
/// where image bytes are bundled alongside the Markdown.
pub fn export_markdown_with_assets(
  snapshot: &DocumentSnapshotPayload,
  images: &BTreeMap<String, String>,
) -> DocumentOperationResult<String> {
  if snapshot.schema_version != 1 {
    return Err(DocumentOperationError::UnsupportedSchemaVersion(
      snapshot.schema_version,
    ));
  }

  let mut lines = Vec::new();
  let root_index = block_index(snapshot, &snapshot.root_block_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(snapshot.root_block_id.clone()))?;
  let root = &snapshot.blocks[root_index];

  // Front matter, if the importer stashed it, is restored verbatim ahead of
  // everything else, fenced with `---`. Stored as the raw inner text, so the
  // fences are ours to (re)apply — `import` of this output recovers the same
  // string (round-trip identity).
  if let Some(fm) = root.data.get("front_matter").and_then(Value::as_str) {
    lines.push("---".to_string());
    // Empty front matter has no inner lines — `"".split('\n')` would yield a
    // spurious blank one, breaking round-trip identity against `---\n---`.
    if !fm.is_empty() {
      lines.extend(fm.split('\n').map(str::to_string));
    }
    lines.push("---".to_string());
  }

  if !root.text.trim().is_empty() {
    append_markdown_block_content(root, 0, &mut lines, images);
  }

  append_markdown_children(snapshot, &snapshot.root_block_id, 0, &mut lines, images)?;

  while lines.last().is_some_and(|line: &String| line.is_empty()) {
    lines.pop();
  }

  Ok(lines.join("\n"))
}

pub fn export_html(snapshot: &DocumentSnapshotPayload) -> DocumentOperationResult<String> {
  if snapshot.schema_version != 1 {
    return Err(DocumentOperationError::UnsupportedSchemaVersion(
      snapshot.schema_version,
    ));
  }

  let mut html = String::new();
  let root_index = block_index(snapshot, &snapshot.root_block_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(snapshot.root_block_id.clone()))?;
  let root = &snapshot.blocks[root_index];
  if !root.text.trim().is_empty() {
    html.push_str(&format!("<p>{}</p>\n", escape_html(root.text.trim())));
  }

  append_html_children(snapshot, &snapshot.root_block_id, &mut html)?;
  append_footnotes_section(snapshot, &mut html);

  Ok(html.trim_end().to_string())
}

/// Wrap the [export_html] fragment in a standalone, self-contained HTML5
/// document: UTF-8, a `<title>`/`<h1>` from `title`, and an embedded stylesheet
/// giving readable typography for headings/code/tables/quotes/images. No
/// external requests — images render from whatever `src` the snapshot's image
/// blocks carry, so a caller wanting a portable file [set_image_srcs] to `data:`
/// URIs first (server/FFI both do). Shared by the download endpoint and the FFI
/// local export so both platforms emit byte-identical files.
pub fn export_html_document(
  snapshot: &DocumentSnapshotPayload,
  title: &str,
  content_width_px: u32,
) -> DocumentOperationResult<String> {
  let body = export_html(snapshot)?;
  let safe_title = escape_html(title.trim());
  // The content column matches the AUTHOR's editor page width (WYSIWYG): a doc
  // exports as wide as it was written, not a hardcoded guess. Clamped to a sane
  // range so a stray value can't produce a 1px or 5000px page.
  let width = content_width_px.clamp(640, 2400);
  // Kept deliberately small and dependency-free: a single embedded stylesheet,
  // system fonts (with CJK fallbacks), no JS. Mirrors the share page's look so
  // an exported file and a shared link read the same. Tables fill the column
  // (width:100%) like the editor, rather than shrinking to content.
  Ok(format!(
    "<!doctype html>\n<html lang=\"zh\">\n<head>\n<meta charset=\"utf-8\">\n\
     <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
     <title>{safe_title}</title>\n<style>\n\
     body{{max-width:{width}px;margin:2.5rem auto;padding:0 1.5rem;\
     font-family:-apple-system,BlinkMacSystemFont,'Segoe UI','Microsoft YaHei',\
     'PingFang SC',sans-serif;line-height:1.7;color:#1f2328;\
     -webkit-print-color-adjust:exact;print-color-adjust:exact;}}\n\
     img{{max-width:100%;height:auto;border-radius:6px;}}\n\
     pre{{background:#f6f8fa;padding:1rem;border-radius:6px;overflow:auto;}}\n\
     code{{background:#f6f8fa;padding:.15em .35em;border-radius:4px;\
     font-family:'Cascadia Code','Consolas','Courier New',monospace;}}\n\
     pre code{{background:none;padding:0;}}\n\
     blockquote{{margin:0;padding-left:1rem;border-left:3px solid #d0d7de;color:#57606a;}}\n\
     table{{width:100%;border-collapse:collapse;}}\n\
     td,th{{border:1px solid #d0d7de;padding:.4em .6em;}}\n\
     hr{{border:none;border-top:1px solid #d0d7de;margin:2rem 0;}}\n\
     h1{{margin-bottom:1.5rem;}}\n\
     .todo input{{margin-right:.4em;}}\n\
     .mermaid{{text-align:center;margin:1rem 0;}}\n\
     .mermaid svg{{max-width:100%;height:auto;}}\n\
     div.math{{margin:1rem 0;text-align:center;}}\n\
     .math-block{{max-width:100%;}}\n</style>\n</head>\n<body>\n\
     <h1>{safe_title}</h1>\n{body}\n</body>\n</html>\n"
  ))
}

/// Point each image block's `url` at the matching entry in `srcs` (keyed by the
/// block's `file_id`). Used before [export_html_document] to swap content-
/// addressed `file_id`s for `data:` URIs (self-contained export) or absolute
/// URLs. Blocks with no matching `file_id` keep their existing `url`.
pub fn set_image_srcs(snapshot: &mut DocumentSnapshotPayload, srcs: &BTreeMap<String, String>) {
  for block in &mut snapshot.blocks {
    if block.kind != "image" {
      continue;
    }
    let Some(file_id) = block.data.get("file_id").and_then(Value::as_str) else {
      continue;
    };
    if let Some(src) = srcs.get(file_id) {
      if let Value::Object(map) = &mut block.data {
        map.insert("url".to_string(), Value::String(src.clone()));
      }
    }
  }
}

// ── rich-export renderers (feature `render`): math → MathML, mermaid → SVG ──
//
// Both degrade to `None` when the feature is off, so `export_html` falls back to
// the raw source form and pure-parse consumers (MCP/CLI) pull no extra deps.

/// LaTeX → a self-contained inline `<svg>` via RaTeX (>99.5% KaTeX coverage —
/// the same family the editor's flutter_math renders, so export == on screen).
/// Glyphs are embedded outlines (no external fonts / JS). `None` on a parse
/// error → caller keeps the raw LaTeX. `display` picks Display vs Text style.
#[cfg(feature = "render")]
fn render_math(latex: &str, display: bool) -> Option<String> {
  use ratex_layout::{layout, to_display_list, LayoutOptions};
  use ratex_svg::{render_to_svg, SvgOptions};
  use ratex_types::math_style::MathStyle;

  let nodes = ratex_parser::parse(latex).ok()?;
  let opts = LayoutOptions {
    style: if display {
      MathStyle::Display
    } else {
      MathStyle::Text
    },
    ..Default::default()
  };
  let list = to_display_list(&layout(&nodes, &opts));
  // padding:0 so an inline formula adds no stray whitespace around itself.
  let svg = render_to_svg(
    &list,
    &SvgOptions {
      embed_glyphs: true,
      padding: 0.0,
      ..Default::default()
    },
  );
  // Size the SVG in `em` so math scales with the surrounding font, and (inline)
  // drop it by its `depth` so its baseline sits on the text baseline.
  let height_em = list.height + list.depth;
  let style = if display {
    format!("height:{height_em:.4}em")
  } else {
    format!("height:{height_em:.4}em;vertical-align:-{:.4}em", list.depth)
  };
  Some(restyle_math_svg(&svg, &style, display))
}

/// Rewrite RaTeX's `<svg>` opening tag: drop its fixed `pt` width/height (so the
/// `em` CSS height drives sizing and the viewBox keeps the aspect ratio) and add
/// our `style` + a class. Body/glyphs are untouched.
#[cfg(feature = "render")]
fn restyle_math_svg(svg: &str, style: &str, display: bool) -> String {
  let class = if display { "math-block" } else { "math-inline" };
  let view_box = svg
    .find("viewBox=\"")
    .and_then(|i| {
      let rest = &svg[i + 9..];
      rest.find('"').map(|j| &rest[..j])
    })
    .unwrap_or("0 0 1 1");
  let body_start = svg.find('>').map(|i| i + 1).unwrap_or(0);
  let body_end = svg.rfind("</svg>").unwrap_or(svg.len());
  let body = &svg[body_start..body_end];
  format!(
    "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{view_box}\" \
     class=\"{class}\" style=\"{style}\" role=\"math\">{body}</svg>"
  )
}
#[cfg(not(feature = "render"))]
fn render_math(_latex: &str, _display: bool) -> Option<String> {
  None
}

/// A Mermaid diagram → a self-contained inline `<svg>` via the headless merman
/// engine (the SAME engine the editor renders with). `id` scopes the SVG's
/// `<style>` so multiple diagrams on one page don't cross-style each other.
/// `None` on a syntax/render error (or feature off) → caller keeps the code.
#[cfg(feature = "render")]
pub fn render_mermaid_svg_with_id(source: &str, id: &str) -> Option<String> {
  use merman::render::{HeadlessRenderer, SvgPipeline};
  // `resvg-safe`: plain shapes/text, no `<foreignObject>`. The editor's raster
  // preview (flutter_svg) can't decode foreignObject, so it already renders
  // this pipeline — using it here too means export == on-screen, and browsers
  // render the safe subset fine.
  let pipeline = SvgPipeline::resvg_safe();
  HeadlessRenderer::new()
    .with_strict_parsing()
    .with_diagram_id(id)
    .render_svg_with_pipeline_sync(source, &pipeline)
    .ok()
    .flatten()
}
#[cfg(not(feature = "render"))]
pub fn render_mermaid_svg_with_id(_source: &str, _id: &str) -> Option<String> {
  None
}

/// Single-diagram convenience (the editor's live FFI preview): a stable id.
pub fn render_mermaid_svg(source: &str) -> Option<String> {
  render_mermaid_svg_with_id(source, "mica-mermaid")
}

/// A merman diagram id from a block id: only `[A-Za-z0-9_-]`, always leading
/// with a letter (a valid, collision-free CSS/SVG id per diagram).
fn mermaid_id(block_id: &str) -> String {
  let mut s = String::from("m");
  for c in block_id.chars() {
    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
      s.push(c);
    }
  }
  s
}

/// GFM footnotes are collected to a single trailing section: an ordered list,
/// one item per `footnote_def` (in document order), each item ending with a
/// backlink (`↩`) to its reference. Emits nothing when the document has none.
fn append_footnotes_section(snapshot: &DocumentSnapshotPayload, out: &mut String) {
  let defs: Vec<&Block> = snapshot
    .blocks
    .iter()
    .filter(|b| b.kind == "footnote_def")
    .collect();
  if defs.is_empty() {
    return;
  }
  out.push_str("<section class=\"footnotes\">\n<ol>\n");
  for def in defs {
    let label = block_data_str(def, "label").unwrap_or("");
    let id = escape_html(label);
    let body = html_inline(def);
    out.push_str(&format!(
      "<li id=\"fn-{id}\">\n<p>{body} <a href=\"#fnref-{id}\" class=\"footnote-backref\">↩</a></p>\n</li>\n"
    ));
  }
  out.push_str("</ol>\n</section>\n");
}

/// Parse Markdown into a flat document snapshot. Each line maps to a top-level
/// block; structural nesting beyond fenced code is intentionally out of scope
/// for the MVP importer.
pub fn import_markdown(markdown: &str, root_block_id: &str) -> DocumentSnapshotPayload {
  // YAML front matter: a leading `---` fence with a later `---`/`...` close.
  // We don't parse the YAML — just lift the raw inner text off the block
  // stream so it can't degrade into thematic breaks / paragraphs, and stash
  // it on the root for verbatim round-trip on export.
  let (front_matter, body) = split_front_matter(markdown);

  let raw_lines: Vec<&str> = body.lines().collect();
  let mut blocks: Vec<Block> = Vec::new();
  let mut root_children: Vec<String> = Vec::new();

  // Pass 1: link reference definitions (`[label]: dest "title"`, possibly
  // spanning lines) — they resolve case-insensitively and vanish from the
  // output. Definitions inside fences don't count, and a definition can't
  // interrupt a paragraph.
  let (defs, def_lines, quote_def_lines) = collect_ref_definitions(&raw_lines);

  let mut index = 0;
  // Stack of open list items' CONTENT columns (tabs count as 4): a new item
  // is a child only when its marker reaches the parent's content column —
  // `- foo` / ` - bar` are siblings. Other blocks reset the stack.
  let mut list_stack: Vec<usize> = Vec::new();
  // The most recently pushed paragraph-like block (paragraph or list/todo
  // item), kept open for multi-line continuation: indented or lazy lines
  // join its text with a soft break; for items, a blank + indented line
  // starts a second paragraph (joined with \n\n, list turns loose). `raw`
  // is the accumulated marker-stripped source, `base` the data before
  // inline marks so the joined text can re-parse cleanly.
  struct OpenItem {
    block_idx: usize,
    kind: String,
    content_col: usize,
    raw: String,
    base: Value,
    had_blank: bool,
    qdepth: usize,
  }
  let mut open_item: Option<OpenItem> = None;
  // Last pushed list item (index, level, marker char) — anchors loose-list
  // marking, the "only a run's first number sets <ol start>" rule, and
  // marker-change list breaks (`-` vs `+`, `.` vs `)`).
  let mut last_list: Option<(usize, usize, char)> = None;
  // A blank line seen inside a list: if another item follows, the list is
  // loose (items render as paragraphs in HTML).
  let mut pending_loose = false;
  // Block quotes (flat model: blocks carry `data.quote` = depth, the HTML
  // exporter rebuilds nested <blockquote>): is a quote group currently
  // open, did a blank just close one (next quote gets `data.qbreak`), and
  // is a content-less `>` group waiting to become an empty quote block.
  let mut quote_active = false;
  let mut quote_boundary = false;
  let mut pending_empty_quote: Option<usize> = None;
  // Does the deepest open list item already hold container children
  // (code/quote/divider blocks carrying `data.li`)? Then later indented
  // paragraphs become child paragraphs instead of `\n\n` text joins.
  let mut item_children = false;
  while index < raw_lines.len() {
    if def_lines.contains(&index) {
      index += 1;
      continue;
    }
    if let Some(&qd) = quote_def_lines.get(&index) {
      // The definition was consumed; the line still opens/continues its
      // blockquote as a contentless `>` marker (spec ex. 218).
      if !quote_active && pending_empty_quote.is_none() {
        pending_empty_quote = Some(qd);
      }
      open_item = None;
      quote_active = true;
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
      match open_item.as_mut() {
        Some(open)
          if matches!(open.kind.as_str(), "bulleted_list" | "numbered_list" | "todo") =>
        {
          open.had_blank = true
        }
        _ => open_item = None,
      }
      pending_loose = pending_loose || !list_stack.is_empty();
      // A content-less `>` group becomes an empty quote block; a blank
      // after a quote separates blockquotes (the next gets `qbreak`).
      if let Some(d) = pending_empty_quote.take() {
        let mut data = if d > 1 { json!({ "quote": d }) } else { Value::Null };
        if quote_boundary {
          data_insert(&mut data, "qbreak", json!(true));
        }
        push_block(&mut blocks, &mut root_children, "quote", String::new(), data);
      }
      if blocks.last().is_some_and(|b| quote_depth_of(b) > 0) {
        quote_boundary = true;
      }
      quote_active = false;
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
      && list_stack.is_empty()
      && let Some(open) = open_item.as_mut()
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
    if let Some(open) = &open_item
      && open.kind == "paragraph"
      && let Some(level) = setext_level(line)
    {
      let (text, data) = apply_inline_marks(open.raw.clone(), json!({ "level": level }), &defs);
      let idx = open.block_idx;
      blocks[idx].kind = "heading".to_string();
      blocks[idx].text = text;
      blocks[idx].data = data;
      open_item = None;
      index += 1;
      continue;
    }

    // List-item container children: at or past the open item's content
    // column, fences / indented code / dividers (and, below, quotes and
    // paragraphs) belong INSIDE the item — flat blocks carrying `data.li`.
    if let Some(&cc) = list_stack.last()
      && col >= cc
      && strip_quote_markers(content).0 == 0
      && !matches!(
        classify_markdown_line(content).0,
        "bulleted_list" | "numbered_list" | "todo"
      )
    {
      let level = list_stack.len() - 1;
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
        if pending_loose {
          if let Some((prev_idx, _, _)) = last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), data);
        item_children = true;
        open_item = None;
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
        if pending_loose {
          if let Some((prev_idx, _, _)) = last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), data);
        item_children = true;
        open_item = None;
        continue;
      }
      // Setext underline of the OPEN item's text: the text becomes a
      // heading CHILD (`- Bar\n  ---` → <li><h2>Bar</h2>, spec ex. 300).
      // Checked before the divider — with an open texted item, `---` is
      // an underline, not an <hr> (same precedence as at top level).
      if let Some(hl) = setext_level(content)
        && let Some(open) = open_item.as_ref()
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
        item_children = true;
        open_item = None;
        index += 1;
        continue;
      }
      // Divider child.
      if is_divider(content) {
        let data = json!({ "li": level });
        if pending_loose {
          if let Some((prev_idx, _, _)) = last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, "divider", String::new(), data);
        item_children = true;
        open_item = None;
        index += 1;
        continue;
      }
      // Paragraph child: once the item holds children, indented paragraph
      // lines become child blocks (order matters — text renders first).
      if item_children
        && classify_markdown_line(content).0 == "paragraph"
        && open_item.is_none()
      {
        let data = json!({ "li": level });
        if pending_loose {
          if let Some((prev_idx, _, _)) = last_list {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          pending_loose = false;
        }
        let base = data.clone();
        let (text2, data2) = apply_inline_marks(content.to_string(), data, &defs);
        push_block(&mut blocks, &mut root_children, "paragraph", text2, data2);
        open_item = Some(OpenItem {
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
      list_stack.clear();
      open_item = None;
      last_list = None;
      pending_loose = false;
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
        list_stack.clear();
        open_item = None;
        last_list = None;
        pending_loose = false;
        item_children = false;
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
      && list_stack.is_empty()
      && open_item.is_none()
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
      list_stack.clear();
      open_item = None;
      last_list = None;
      pending_loose = false;
      item_children = false;
      continue;
    }

    // HTML block (CommonMark types 1–7) → a raw html code block: the
    // source is the content (AFFiNE-style degrade), `data.raw` makes both
    // exporters write it back verbatim. Type 7 can't interrupt a paragraph.
    if col < 4
      && let Some(html_kind) = html_block_start(content)
      // Outside any open item's content column, an HTML block INTERRUPTS
      // the list (spec ex. 308/309: `<!-- -->` splits two lists).
      && (list_stack.is_empty() || col < *list_stack.last().unwrap())
      && !(html_kind == 7 && open_item.is_some())
    {
      list_stack.clear();
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
      open_item = None;
      last_list = None;
      pending_loose = false;
      item_children = false;
      continue;
    }

    // GFM pipe table: a `|`-row followed by a `| --- |` separator whose
    // cell count MATCHES the header row (spec rule).
    if content.contains('|')
      && index + 1 < raw_lines.len()
      && is_table_separator(raw_lines[index + 1].trim())
      && split_table_row(raw_lines[index + 1].trim()).len() == split_table_row(content).len()
    {
      list_stack.clear();
      open_item = None;
      last_list = None;
      pending_loose = false;
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
      let mut data = json!({
        "rows": rows,
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
      && ((list_stack.is_empty() && open_item.is_none())
        || open_item
          .as_ref()
          .is_some_and(|o| o.had_blank && col < o.content_col))
    {
      list_stack.clear();
      open_item = None;
      last_list = None;
      pending_loose = false;
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
      list_stack.clear();
      open_item = None;
      last_list = None;
      pending_loose = false;
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
      let li_ctx = match list_stack.last() {
        Some(&cc) if col >= cc => Some(list_stack.len() - 1),
        _ => None,
      };
      if li_ctx.is_none() {
        list_stack.clear();
        last_list = None;
      } else if pending_loose {
        if let Some((prev_idx, _, _)) = last_list {
          data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
        }
      }
      pending_loose = false;
      let qrest_trim = qrest.trim_start();

      // `>` with nothing after: a paragraph break inside the quote — or,
      // if the group never gets content, an empty blockquote.
      if qrest_trim.is_empty() {
        // A blank inside a quoted LIST ITEM keeps the item open — an
        // indented continuation may follow (spec ex. 259).
        if let Some(open) = open_item.as_mut()
          && matches!(open.kind.as_str(), "bulleted_list" | "numbered_list" | "todo")
          && open.qdepth == qdepth
        {
          open.had_blank = true;
          quote_active = true;
          index += 1;
          continue;
        }
        if !quote_active && pending_empty_quote.is_none() {
          pending_empty_quote = Some(qdepth);
        }
        open_item = None;
        quote_active = true;
        index += 1;
        continue;
      }

      // Lazy/marked continuation: a plain paragraph line at the same or a
      // shallower marker depth keeps the open quoted paragraph going.
      let (kind, text, mut data) = classify_markdown_line(qrest_trim);
      if kind == "paragraph"
        && !qrest_trim.starts_with("```")
        && let Some(open) = open_item.as_mut()
        && open.qdepth >= qdepth
        && !open.had_blank
      {
        open.raw.push('\n');
        open.raw.push_str(strip_quote_markers(body).1.trim_start());
        let (joined, joined_data) =
          apply_inline_marks(open.raw.clone(), open.base.clone(), &defs);
        blocks[open.block_idx].text = joined;
        blocks[open.block_idx].data = joined_data;
        quote_active = true;
        index += 1;
        continue;
      }

      pending_empty_quote = None;
      let qbreak = quote_boundary;
      quote_boundary = false;

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
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          item_children = true;
        }
        push_block(&mut blocks, &mut root_children, "code_block", code_lines.join("\n"), data);
        open_item = None;
        quote_active = true;
        continue;
      }

      let qcol = qrest.len() - qrest_trim.len();
      // After a `>` gap, a line at the quoted item's content column is its
      // SECOND paragraph — the item turns loose (spec ex. 259).
      if let Some(open) = open_item.as_mut()
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
        quote_active = true;
        index += 1;
        continue;
      }
      // Indented code inside the quote (per marked line).
      if qcol >= 4 {
        let mut data = json!({ "quote": qdepth });
        if qbreak {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          item_children = true;
        }
        push_block(
          &mut blocks,
          &mut root_children,
          "code_block",
          deindent_columns(qrest, 4),
          data,
        );
        open_item = None;
        quote_active = true;
        index += 1;
        continue;
      }

      if is_divider(qrest_trim) {
        let mut data = json!({ "quote": qdepth });
        if qbreak {
          data_insert(&mut data, "qbreak", json!(true));
        }
        if let Some(level) = li_ctx {
          data_insert(&mut data, "li", json!(level));
          item_children = true;
        }
        push_block(&mut blocks, &mut root_children, "divider", String::new(), data);
        open_item = None;
        quote_active = true;
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
      if let Some(level) = li_ctx {
        data_insert(&mut data, "li", json!(level));
        item_children = true;
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
          open_item = Some(OpenItem {
            block_idx: blocks.len() - 1,
            kind: "quote".to_string(),
            content_col: 0,
            raw,
            base,
            had_blank: false,
            qdepth: qdepth + inner_depth,
          });
          item_children = true;
          quote_active = true;
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
      open_item = if matches!(kind, "quote" | "bulleted_list" | "numbered_list" | "todo") {
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
      quote_active = true;
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
      && list_stack.iter().all(|&cc| col < cc)
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
    if let Some(open) = open_item.as_mut() {
      let weak_item = matches!(kind, "bulleted_list" | "numbered_list" | "todo")
        && open.kind == "paragraph"
        && !open.had_blank
        && (text.is_empty() || data.get("start").is_some());
      if kind == "paragraph" || weak_item {
        if open.had_blank {
          if col >= open.content_col && !open.raw.is_empty() && !item_children {
            open.had_blank = false;
            open.raw.push_str("\n\n");
            open.raw.push_str(body);
            data_insert(&mut open.base, "loose", json!(true));
            pending_loose = false;
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
            && let Some(anc) = list_stack.iter().rposition(|&cc| col >= cc)
            && anc + 1 < list_stack.len()
          {
            list_stack.truncate(anc + 1);
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
              pending_loose = false;
              item_children = true;
              open_item = None;
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
        while list_stack.last().is_some_and(|&cc| col < cc) {
          list_stack.pop();
        }
        let level = list_stack.len();
        let ccol = col + marker_width + extra;
        list_stack.push(ccol);
        let mut idata = Value::Null;
        if level > 0 {
          data_insert(&mut idata, "indent", json!(level));
        }
        if kind == "numbered_list"
          && let Some(st) = data.get("start")
        {
          data_insert(&mut idata, "start", st.clone());
        }
        if pending_loose {
          data_insert(&mut idata, "loose", json!(true));
          pending_loose = false;
        }
        push_block(&mut blocks, &mut root_children, kind, String::new(), idata);
        last_list = Some((blocks.len() - 1, level, marker_char_of(kind, &content_owned)));
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
      while list_stack.last().is_some_and(|&cc| col < cc) {
        list_stack.pop();
      }
      let level = list_stack.len();
      list_stack.push(content_col);
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
      let continues_run = last_list.is_some_and(|(idx, lv, mk)| {
        lv == level && blocks[idx].kind == kind && mk == marker_char
      });
      let same_level_break = !continues_run
        && last_list.is_some_and(|(idx, lv, _)| lv == level && blocks[idx].kind == kind);
      if same_level_break {
        // Record the changed bullet/delimiter so export keeps the break.
        data_insert(&mut data, "marker", json!(marker_char.to_string()));
      }
      // The blank line belongs to whichever list the boundary sits in:
      // same-or-shallower level → this item is loose; deeper level → the
      // blank separated a parent's text from its sublist, so the parent is.
      if pending_loose {
        match last_list {
          Some((prev_idx, prev_level, _)) if level > prev_level => {
            data_insert(&mut blocks[prev_idx].data, "loose", json!(true));
          }
          _ => data_insert(&mut data, "loose", json!(true)),
        }
        pending_loose = false;
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
        last_list = Some((block_idx, level, marker_char));
        while list_stack.len() > level {
          list_stack.pop();
        }
        list_stack.push(content_col);
        item_children = true;
        open_item = None;
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
      last_list = Some((block_idx, level, marker_char));
      item_children = false;
      open_item = Some(OpenItem {
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

    list_stack.clear();
    last_list = None;
    pending_loose = false;
    item_children = false;
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
    open_item = raw.map(|raw| OpenItem {
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

  if let Some(d) = pending_empty_quote.take() {
    let mut data = if d > 1 { json!({ "quote": d }) } else { Value::Null };
    if quote_boundary {
      data_insert(&mut data, "qbreak", json!(true));
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
fn split_front_matter(markdown: &str) -> (Option<String>, &str) {
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
fn is_divider(content: &str) -> bool {
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
fn leading_columns(line: &str) -> usize {
  let mut col = 0usize;
  for c in line.chars() {
    match c {
      ' ' => col += 1,
      '\t' => col = (col / 4 + 1) * 4,
      _ => break,
    }
  }
  col
}

/// Strip [columns] of leading indentation, treating tabs as 4-column stops
/// (a tab that overshoots emits the remainder as spaces).
fn deindent_columns(line: &str, columns: usize) -> String {
  let mut col = 0usize;
  for (i, c) in line.char_indices() {
    match c {
      ' ' => col += 1,
      '\t' => col = (col / 4 + 1) * 4,
      _ => return line[i..].to_string(),
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
fn column_of(line: &str) -> usize {
  let mut col = 0usize;
  for c in line.chars() {
    match c {
      ' ' => col += 1,
      '\t' => col = (col / 4 + 1) * 4,
      _ => break,
    }
  }
  col
}

/// A GFM footnote definition leader `[^label]: rest`. Returns the label (no
/// caret) and the remaining first-line content (may be empty). The label
/// holds no whitespace or brackets — same shape as the inline reference.
fn parse_footnote_def(content: &str) -> Option<(String, String)> {
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
fn setext_level(line: &str) -> Option<usize> {
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

fn is_table_separator(line: &str) -> bool {
  let cells = split_table_row(line);
  !cells.is_empty()
    && cells.iter().all(|cell| {
      let t = cell.trim();
      !t.is_empty() && t.contains('-') && t.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ')
    })
}

fn split_table_row(line: &str) -> Vec<String> {
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
fn classify_markdown_line(content: &str) -> (&'static str, String, Value) {
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

fn push_block(
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

fn heading_prefix_level(content: &str) -> Option<usize> {
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
fn strip_atx_closing(text: &str) -> &str {
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
fn numbered_list_marker(content: &str) -> Option<(u64, &str)> {
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

/// Curated named-entity table (the spec set plus common real-world names);
/// unknown entities stay literal, which is exactly the spec behavior.
///
/// `invisible_characters` is DENY-by-default and fires on `&shy;` (U+00AD, a
/// soft hyphen). Mapping an entity name to its character is this table's whole
/// job, so the "invisible" literal is the correct value, not a typo. The lint
/// aborted `cargo clippy` on the FIRST crate of the workspace, which is why the
/// other eight had never been linted at all — see docs/code-review-2026-07-20.md.
#[allow(clippy::invisible_characters)]
fn named_entity(name: &str) -> Option<&'static str> {
  match name {
    "AElig" => Some("Æ"),
    "Alpha" => Some("Α"),
    "Beta" => Some("Β"),
    "ClockwiseContourIntegral" => Some("∲"),
    "Dagger" => Some("‡"),
    "Dcaron" => Some("Ď"),
    "Delta" => Some("Δ"),
    "DifferentialD" => Some("ⅆ"),
    "Gamma" => Some("Γ"),
    "HilbertSpace" => Some("ℋ"),
    "Lambda" => Some("Λ"),
    "OElig" => Some("Œ"),
    "Omega" => Some("Ω"),
    "Phi" => Some("Φ"),
    "Pi" => Some("Π"),
    "Psi" => Some("Ψ"),
    "Scaron" => Some("Š"),
    "Sigma" => Some("Σ"),
    "Theta" => Some("Θ"),
    "Yuml" => Some("Ÿ"),
    "aacute" => Some("á"),
    "acirc" => Some("â"),
    "acute" => Some("´"),
    "agrave" => Some("à"),
    "alpha" => Some("α"),
    "amp" => Some("&"),
    "apos" => Some("'"),
    "aring" => Some("å"),
    "asymp" => Some("≈"),
    "atilde" => Some("ã"),
    "auml" => Some("ä"),
    "beta" => Some("β"),
    "brvbar" => Some("¦"),
    "bull" => Some("•"),
    "cap" => Some("∩"),
    "ccedil" => Some("ç"),
    "cedil" => Some("¸"),
    "cent" => Some("¢"),
    "chi" => Some("χ"),
    "circ" => Some("ˆ"),
    "clubs" => Some("♣"),
    "copy" => Some("©"),
    "cup" => Some("∪"),
    "curren" => Some("¤"),
    "dagger" => Some("†"),
    "darr" => Some("↓"),
    "deg" => Some("°"),
    "delta" => Some("δ"),
    "diams" => Some("♦"),
    "divide" => Some("÷"),
    "eacute" => Some("é"),
    "ecirc" => Some("ê"),
    "egrave" => Some("è"),
    "empty" => Some("∅"),
    "emsp" => Some(" "),
    "ensp" => Some(" "),
    "epsilon" => Some("ε"),
    "equiv" => Some("≡"),
    "eta" => Some("η"),
    "euml" => Some("ë"),
    "euro" => Some("€"),
    "exist" => Some("∃"),
    "fnof" => Some("ƒ"),
    "forall" => Some("∀"),
    "frac12" => Some("½"),
    "frac14" => Some("¼"),
    "frac34" => Some("¾"),
    "frasl" => Some("⁄"),
    "gamma" => Some("γ"),
    "ge" => Some("≥"),
    "gt" => Some(">"),
    "harr" => Some("↔"),
    "hearts" => Some("♥"),
    "hellip" => Some("…"),
    "iacute" => Some("í"),
    "icirc" => Some("î"),
    "iexcl" => Some("¡"),
    "infin" => Some("∞"),
    "int" => Some("∫"),
    "iota" => Some("ι"),
    "iquest" => Some("¿"),
    "isin" => Some("∈"),
    "iuml" => Some("ï"),
    "kappa" => Some("κ"),
    "lambda" => Some("λ"),
    "lang" => Some("⟨"),
    "laquo" => Some("«"),
    "larr" => Some("←"),
    "lceil" => Some("⌈"),
    "ldquo" => Some("“"),
    "le" => Some("≤"),
    "lfloor" => Some("⌊"),
    "loz" => Some("◊"),
    "lsquo" => Some("‘"),
    "lt" => Some("<"),
    "macr" => Some("¯"),
    "mdash" => Some("—"),
    "micro" => Some("µ"),
    "middot" => Some("·"),
    "minus" => Some("−"),
    "mu" => Some("μ"),
    "nbsp" => Some(" "),
    "ndash" => Some("–"),
    "ne" => Some("≠"),
    "ngE" => Some("≧̸"),
    "not" => Some("¬"),
    "notin" => Some("∉"),
    "ntilde" => Some("ñ"),
    "nu" => Some("ν"),
    "oacute" => Some("ó"),
    "ocirc" => Some("ô"),
    "oelig" => Some("œ"),
    "omega" => Some("ω"),
    "oplus" => Some("⊕"),
    "ordf" => Some("ª"),
    "ordm" => Some("º"),
    "oslash" => Some("ø"),
    "otilde" => Some("õ"),
    "otimes" => Some("⊗"),
    "ouml" => Some("ö"),
    "para" => Some("¶"),
    "permil" => Some("‰"),
    "perp" => Some("⊥"),
    "phi" => Some("φ"),
    "pi" => Some("π"),
    "plusmn" => Some("±"),
    "pound" => Some("£"),
    "prod" => Some("∏"),
    "psi" => Some("ψ"),
    "quot" => Some("\""),
    "radic" => Some("√"),
    "rang" => Some("⟩"),
    "raquo" => Some("»"),
    "rarr" => Some("→"),
    "rceil" => Some("⌉"),
    "rdquo" => Some("”"),
    "reg" => Some("®"),
    "rfloor" => Some("⌋"),
    "rho" => Some("ρ"),
    "rsquo" => Some("’"),
    "scaron" => Some("š"),
    "sdot" => Some("⋅"),
    "sect" => Some("§"),
    "shy" => Some("­"),
    "sigma" => Some("σ"),
    "spades" => Some("♠"),
    "sub" => Some("⊂"),
    "sum" => Some("∑"),
    "sup" => Some("⊃"),
    "sup1" => Some("¹"),
    "sup2" => Some("²"),
    "sup3" => Some("³"),
    "szlig" => Some("ß"),
    "tau" => Some("τ"),
    "theta" => Some("θ"),
    "thinsp" => Some(" "),
    "tilde" => Some("˜"),
    "times" => Some("×"),
    "trade" => Some("™"),
    "uacute" => Some("ú"),
    "uarr" => Some("↑"),
    "ucirc" => Some("û"),
    "uml" => Some("¨"),
    "upsilon" => Some("υ"),
    "uuml" => Some("ü"),
    "xi" => Some("ξ"),
    "yen" => Some("¥"),
    "zeta" => Some("ζ"),
    "zwj" => Some("‍"),
    "zwnj" => Some("‌"),
    _ => None,
  }
}

/// Parse an entity/numeric character reference starting at `&` (chars[i]).
/// Returns (decoded string, chars consumed).
fn parse_entity(chars: &[char], i: usize) -> Option<(String, usize)> {
  if chars.get(i) != Some(&'&') {
    return None;
  }
  let semi = chars[i + 1..].iter().take(33).position(|&c| c == ';')? + i + 1;
  let body: String = chars[i + 1..semi].iter().collect();
  if let Some(rest) = body.strip_prefix('#') {
    let (digits, radix) = match rest.strip_prefix(['x', 'X']) {
      Some(h) => (h, 16),
      None => (rest, 10),
    };
    if digits.is_empty()
      || digits.len() > 7
      || !digits.chars().all(|c| c.is_digit(radix))
    {
      return None;
    }
    let n = u32::from_str_radix(digits, radix).ok()?;
    let c = if n == 0 { '\u{FFFD}' } else { char::from_u32(n).unwrap_or('\u{FFFD}') };
    return Some((c.to_string(), semi - i + 1));
  }
  named_entity(&body).map(|v| (v.to_string(), semi - i + 1))
}

/// Find the two-char sequence `a``b` starting at `from`.
fn find_pair(chars: &[char], from: usize, a: char, b: char) -> Option<usize> {
  let mut j = from;
  while j + 1 < chars.len() {
    if chars[j] == a && chars[j + 1] == b {
      return Some(j);
    }
    j += 1;
  }
  None
}

/// One past the closing backtick run of the code span opening at `start` (the
/// index of its first backtick), or None when the run never closes — CommonMark
/// leaves an unclosed run as literal text.
///
/// Deliberately a second copy of the run-matching in the inline-code branch
/// rather than a shared helper: that branch also strips padding and folds
/// newlines, and it carries the conformance score. This only needs the extent.
/// Keep the two in step.
fn code_span_close(chars: &[char], start: usize) -> Option<usize> {
  let mut n = 0;
  while start + n < chars.len() && chars[start + n] == '`' {
    n += 1;
  }
  let mut j = start + n;
  while j < chars.len() {
    if chars[j] == '`' {
      let mut m = 0;
      while j + m < chars.len() && chars[j + m] == '`' {
        m += 1;
      }
      if m == n {
        return Some(j + m);
      }
      j += m;
    } else {
      j += 1;
    }
  }
  None
}

/// Delimiter-inclusive char spans of every inline math run in [src] — both the
/// `$ … $` and `\( … \)` forms — judged by exactly the rules the parser uses.
///
/// Public so callers outside the parser can ask "is this offset inside a
/// formula?" without restating the Pandoc rules and drifting from them (the MCP
/// write guard needs it to tell a corrupted `\times` from an ordinary tab).
/// Mirrors Dart `mathRunSpans` in `lib/editor/marks.dart` — CLAUDE.md #2 keeps
/// the two sides in step, with this one authoritative.
pub fn math_run_spans(src: &str) -> Vec<(usize, usize)> {
  let chars: Vec<char> = src.chars().collect();
  let mut out = Vec::new();
  let mut i = 0;
  while i < chars.len() {
    // `\( … \)` first — the escape arm below would otherwise eat the `\(`.
    if chars[i] == '\\'
      && chars.get(i + 1) == Some(&'(')
      && let Some(close) = find_pair(&chars, i + 2, '\\', ')')
    {
      out.push((i, close + 2));
      i = close + 2;
      continue;
    }
    // A code span binds tighter than math (§6.1) — step over it whole so a `$`
    // inside cannot open a run.
    if chars[i] == '`'
      && let Some(after) = code_span_close(&chars, i)
    {
      i = after;
      continue;
    }
    if chars[i] == '$'
      && chars.get(i + 1) != Some(&'$')
      && let Some(close) = find_math_closer(&chars, i + 1)
    {
      out.push((i, close + 1));
      i = close + 1;
      continue;
    }
    i += 1;
  }
  out
}

/// A valid `$` math closer per the Pandoc rules: content non-empty with
/// non-space edges, closer not followed by a digit, no newline inside, and no
/// crossing into a code span.
fn find_math_closer(chars: &[char], content_start: usize) -> Option<usize> {
  if chars.get(content_start).is_none_or(|c| c.is_whitespace()) {
    return None;
  }
  let mut j = content_start;
  while j < chars.len() {
    let c = chars[j];
    if c == '\n' {
      return None; // inline math stays on one line
    }
    if c == '`' {
      // Code spans bind tighter than math — CommonMark 0.31.2 §6.1 gives them
      // higher precedence than every inline construct but HTML tags and
      // autolinks — so a `$` inside one is literal and cannot close us. Step
      // over the span whole; scanning through it is what let `` `$HOME` `` be
      // eaten by an earlier `$`. An unclosed run is literal text: fall through
      // and keep scanning, exactly as CommonMark reads it.
      if let Some(after) = code_span_close(chars, j) {
        if chars[j..after].contains(&'\n') {
          return None; // the span folds a line break; inline math may not
        }
        j = after;
        continue;
      }
    }
    if c == '$' && j > content_start {
      if chars[j - 1].is_whitespace() || chars[j - 1] == '\\' {
        j += 1;
        continue;
      }
      if chars.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
        j += 1;
        continue;
      }
      return Some(j);
    }
    j += 1;
  }
  None
}

/// GFM extended autolink starting at chars[i] (the caller checks the word
/// boundary): bare `http(s)://…`, `www.…` (href gains `http://`) or a bare
/// email. Returns (consumed chars, href).
fn extended_autolink(chars: &[char], i: usize) -> Option<(usize, String)> {
  let rest: String = chars[i..].iter().collect();
  let lower = rest.to_ascii_lowercase();
  for (prefix, implied) in [
    ("https://", ""),
    ("http://", ""),
    ("ftp://", ""),
    ("www.", "http://"),
  ] {
    if lower.starts_with(prefix) {
      // Take everything up to whitespace or '<', then trim trailing
      // punctuation per the GFM rules.
      let mut end = rest
        .char_indices()
        .find(|&(_, c)| c.is_whitespace() || c == '<')
        .map(|(k, _)| k)
        .unwrap_or(rest.len());
      end = trim_autolink_end(&rest[..end]);
      let candidate = &rest[..end];
      let domain = &candidate[if implied.is_empty() { prefix.len() } else { 0 }..];
      let domain = domain
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
      if !valid_autolink_domain(domain) {
        return None;
      }
      let count = candidate.chars().count();
      if count == 0 {
        return None;
      }
      return Some((count, format!("{implied}{candidate}")));
    }
  }
  // Bare email: local@domain.tld — `+` allowed before the @ only; the last
  // character must be alphanumeric.
  if chars[i].is_ascii_alphanumeric() {
    let mut j = i;
    while chars
      .get(j)
      .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
    {
      j += 1;
    }
    if chars.get(j) == Some(&'@') {
      let local_len = j - i;
      let mut k = j + 1;
      while chars
        .get(k)
        .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
      {
        k += 1;
      }
      // Trailing `.` stays outside the link; trailing `-`/`_` invalidate.
      while k > j + 1 && chars[k - 1] == '.' {
        k -= 1;
      }
      if local_len > 0
        && k > j + 1
        && chars[k - 1].is_ascii_alphanumeric()
        && chars[j + 1..k].contains(&'.')
      {
        let text: String = chars[i..k].iter().collect();
        return Some((k - i, format!("mailto:{text}")));
      }
    }
  }
  None
}

/// GFM trailing-punctuation trimming for extended autolinks.
fn trim_autolink_end(s: &str) -> usize {
  let mut end = s.len();
  loop {
    let kept = &s[..end];
    let Some(last) = kept.chars().last() else { return 0 };
    if matches!(last, '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~' | '\'' | '"') {
      end -= last.len_utf8();
      continue;
    }
    if last == ')' {
      let opens = kept.matches('(').count();
      let closes = kept.matches(')').count();
      if closes > opens {
        end -= 1;
        continue;
      }
      return end;
    }
    if last == ';' {
      // A trailing entity reference (`&amp;`) drops off entirely.
      if let Some(amp) = kept.rfind('&') {
        let body = &kept[amp + 1..end - 1];
        if !body.is_empty() && body.chars().all(|c| c.is_ascii_alphanumeric()) {
          end = amp;
          continue;
        }
      }
      return end;
    }
    return end;
  }
}

/// GFM autolink domain: alphanumeric/`-`/`_` segments joined by `.`, at
/// least one dot, and no underscore in the last two segments.
fn valid_autolink_domain(domain: &str) -> bool {
  if domain.is_empty()
    || !domain
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
  {
    return false;
  }
  let segments: Vec<&str> = domain.split('.').collect();
  if segments.len() < 2 || segments.iter().any(|s| s.is_empty()) {
    return false;
  }
  !segments[segments.len().saturating_sub(2)..]
    .iter()
    .any(|s| s.contains('_'))
}

/// Block-level HTML tag names (CommonMark type-6 start condition).
const HTML_BLOCK_TAGS: &[&str] = &[
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
fn html_block_start(content: &str) -> Option<u8> {
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
fn html_block_ends(kind: u8, line: &str) -> bool {
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
fn inline_html_end(chars: &[char], i: usize) -> Option<usize> {
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
fn fence_open(content: &str) -> Option<(u8, usize, &str)> {
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
fn fence_close(content: &str, fence_char: u8, fence_len: usize) -> bool {
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
fn expand_marker_tabs(line: &str) -> String {
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

fn strip_quote_markers(content: &str) -> (usize, &str) {
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
fn quote_depth_of(block: &Block) -> usize {
  let d = block.data.get("quote").and_then(Value::as_u64).unwrap_or(0) as usize;
  if block.kind == "quote" { d.max(1) } else { d }
}

/// Insert a key into a block's `data`, upgrading `Null` to an object.
fn data_insert(data: &mut Value, key: &str, value: Value) {
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
fn parse_markdown_image(content: &str) -> Option<(String, String, Option<String>)> {
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

fn append_html_children(
  snapshot: &DocumentSnapshotPayload,
  parent_id: &str,
  out: &mut String,
) -> DocumentOperationResult<()> {
  let parent_index = block_index(snapshot, parent_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(parent_id.to_string()))?;
  let child_ids = snapshot.blocks[parent_index].children.clone();

  let mut index = 0;
  while index < child_ids.len() {
    let child = block_for(snapshot, &child_ids[index])?;
    if quote_depth_of(child) > 0 && li_of(child).is_none() {
      // Collect one quote group (a `qbreak` starts the next blockquote)
      // and rebuild the nested <blockquote> structure from `data.quote`.
      let run_start = index;
      index += 1;
      while index < child_ids.len() {
        let b = block_for(snapshot, &child_ids[index])?;
        if quote_depth_of(b) == 0 || b.data.get("qbreak").is_some() {
          break;
        }
        index += 1;
      }
      let items: Vec<&Block> = child_ids[run_start..index]
        .iter()
        .map(|id| block_for(snapshot, id))
        .collect::<DocumentOperationResult<_>>()?;
      render_quote_group(snapshot, &items, 1, out)?;
    } else if list_tag_for(&child.kind).is_some() {
      // Collect the maximal run of list items (any tag, any `data.indent`
      // level) plus their container children (`data.li`) and render the
      // nested <ul>/<ol> structure.
      let run_start = index;
      while index < child_ids.len() {
        let b = block_for(snapshot, &child_ids[index])?;
        let is_li_child = li_of(b).is_some();
        if (list_tag_for(&b.kind).is_none() && !is_li_child)
          || (quote_depth_of(b) > 0 && !is_li_child)
        {
          break;
        }
        index += 1;
      }
      let items: Vec<&Block> = child_ids[run_start..index]
        .iter()
        .map(|id| block_for(snapshot, id))
        .collect::<DocumentOperationResult<_>>()?;
      render_html_list(snapshot, &items, 0, out)?;
    } else {
      append_html_block(snapshot, child, out)?;
      index += 1;
    }
  }

  Ok(())
}

/// Render one quote group (flat blocks carrying `data.quote` depths) as
/// nested `<blockquote>`: deeper runs recurse, `quote`-kind blocks are the
/// paragraphs, list items group into lists, anything else renders normally.
fn render_quote_group(
  snapshot: &DocumentSnapshotPayload,
  items: &[&Block],
  depth: usize,
  out: &mut String,
) -> DocumentOperationResult<()> {
  out.push_str("<blockquote>\n");
  let mut i = 0;
  while i < items.len() {
    // A list run first — it swallows the items' container children
    // (`data.li` members, including DEEPER-quoted ones: `> 1. > q`).
    if quote_depth_of(items[i]) == depth && list_tag_for(&items[i].kind).is_some() {
      let s = i;
      while i < items.len()
        && ((quote_depth_of(items[i]) == depth && list_tag_for(&items[i].kind).is_some())
          || li_of(items[i]).is_some()
          || li_level(items[i]) > 0)
      {
        i += 1;
      }
      render_html_list(snapshot, &items[s..i], 0, out)?;
      continue;
    }
    if quote_depth_of(items[i]) > depth {
      let s = i;
      while i < items.len() && quote_depth_of(items[i]) > depth {
        i += 1;
      }
      render_quote_group(snapshot, &items[s..i], depth + 1, out)?;
      continue;
    }
    if items[i].kind == "quote" {
      let text = html_inline(items[i]);
      if !text.is_empty() {
        out.push_str(&format!("<p>{text}</p>\n"));
      }
    } else {
      append_html_block(snapshot, items[i], out)?;
    }
    i += 1;
  }
  out.push_str("</blockquote>\n");
  Ok(())
}

fn list_indent(block: &Block) -> usize {
  // Cap at 8 to mirror the Dart editor's read (model.dart `indent` getter,
  // `.clamp(0, 8)`). Without this cap the two engines DISAGREE on a block whose
  // `data.indent` was written past 8 by a non-editor client (MCP/REST/API-token,
  // or an import): Rust rendered N levels, Dart rendered 8 — same data, two
  // outputs, violating the round-trip/parity invariant. (P1-4.) Non-destructive,
  // like Dart's: the stored value is untouched, only the rendered depth is bound.
  (block.data.get("indent").and_then(Value::as_u64).unwrap_or(0) as usize).min(8)
}

/// Is this block a container child of a list item, and at which level?
fn li_of(block: &Block) -> Option<usize> {
  block.data.get("li").and_then(Value::as_u64).map(|v| v as usize)
}

/// The effective list level a block sits at: items by `data.indent`,
/// container children by `data.li`.
fn li_level(block: &Block) -> usize {
  match li_of(block) {
    Some(l) => l,
    None => list_indent(block),
  }
}

/// Render a flat run of list items (nesting via `data.indent`) plus their
/// container children (`data.li`) as spec-shaped nested `<ul>`/`<ol>`: a tag
/// change at the same level starts a new list, deeper items and children
/// nest inside the preceding `<li>`, a loose list (any item carrying
/// `data.loose`) wraps item text in `<p>`, and `data.start` on the first
/// item of an ordered run becomes `<ol start="n">`.
fn render_html_list(
  snapshot: &DocumentSnapshotPayload,
  items: &[&Block],
  level: usize,
  out: &mut String,
) -> DocumentOperationResult<()> {
  let mut i = 0;
  while i < items.len() {
    if li_level(items[i]) > level || (li_of(items[i]) == Some(level) && i == 0) {
      // Orphan deeper entries (no parent at this level): render a level
      // down; a leading direct child renders bare.
      let s = i;
      while i < items.len()
        && (li_level(items[i]) > level || (li_of(items[i]) == Some(level) && i == s))
      {
        i += 1;
      }
      render_html_list(snapshot, &items[s..i], level + 1, out)?;
      continue;
    }
    let tag = list_tag_for(&items[i].kind).unwrap_or("ul");
    // Extent of this list: items at `level` with the same tag, plus any
    // deeper items or container children in between.
    let mut li_heads: Vec<usize> = Vec::new();
    let mut j = i;
    while j < items.len() {
      if li_level(items[j]) == level && li_of(items[j]).is_none() {
        if list_tag_for(&items[j].kind) != Some(tag) {
          break;
        }
        // A marker/delimiter change recorded at import starts a new list.
        if !li_heads.is_empty()
          && (items[j].data.get("start").is_some() || items[j].data.get("marker").is_some())
        {
          break;
        }
        li_heads.push(j);
      }
      j += 1;
    }
    let loose = li_heads.iter().any(|&k| block_data_bool(items[k], "loose"));
    let start_attr = match items[i].data.get("start").and_then(Value::as_u64) {
      Some(n) if tag == "ol" && n != 1 => format!(" start=\"{n}\""),
      _ => String::new(),
    };
    out.push_str(&format!("<{tag}{start_attr}>\n"));
    for (k, &head) in li_heads.iter().enumerate() {
      let end = li_heads.get(k + 1).copied().unwrap_or(j);
      let mut body = html_inline(items[head]);
      // GFM task list items render a disabled checkbox before the text.
      if items[head].kind == "todo" {
        let checked = if block_data_bool(items[head], "checked") {
          "checked=\"\" "
        } else {
          ""
        };
        body = format!("<input {checked}disabled=\"\" type=\"checkbox\"> {body}");
      }
      let has_children = head + 1 < end;
      if loose && body.is_empty() && !has_children {
        out.push_str("<li></li>\n");
      } else if loose {
        out.push_str("<li>\n");
        for para in body.split("\n\n").filter(|p| !p.is_empty()) {
          out.push_str(&format!("<p>{para}</p>\n"));
        }
        if has_children {
          render_li_children(snapshot, &items[head + 1..end], level, true, out)?;
        }
        out.push_str("</li>\n");
      } else {
        out.push_str("<li>");
        out.push_str(&body);
        if has_children {
          out.push('\n');
          render_li_children(snapshot, &items[head + 1..end], level, false, out)?;
        }
        out.push_str("</li>\n");
      }
    }
    out.push_str(&format!("</{tag}>\n"));
    i = j;
  }
  Ok(())
}

/// Inside one `<li>`: direct container children (`data.li` == level) render
/// as their blocks (quote runs rebuild <blockquote>); anything deeper is a
/// nested list run.
fn render_li_children(
  snapshot: &DocumentSnapshotPayload,
  items: &[&Block],
  level: usize,
  loose: bool,
  out: &mut String,
) -> DocumentOperationResult<()> {
  let mut i = 0;
  while i < items.len() {
    let b = items[i];
    if li_of(b) == Some(level) {
      if quote_depth_of(b) > 0 {
        let s = i;
        while i < items.len() {
          let q = items[i];
          if li_of(q) != Some(level) || quote_depth_of(q) == 0 {
            break;
          }
          if i > s && q.data.get("qbreak").is_some() {
            break;
          }
          i += 1;
        }
        // Base depth = the group head's own depth: inside a quoted list
        // (`> 1. > q`) the child carries the ABSOLUTE depth, but the
        // enclosing <blockquote>s are already open around the list.
        render_quote_group(snapshot, &items[s..i], quote_depth_of(items[s]), out)?;
        continue;
      }
      if b.kind == "paragraph" {
        // In a TIGHT item a paragraph child renders bare (spec ex. 300:
        // `<h2>Bar</h2>\nbaz</li>`); only loose lists wrap it in <p>.
        let text = html_inline(b);
        if !text.is_empty() {
          if loose {
            out.push_str(&format!("<p>{text}</p>\n"));
          } else {
            out.push_str(&text);
            if i + 1 < items.len() {
              out.push('\n');
            }
          }
        }
      } else {
        append_html_block(snapshot, b, out)?;
      }
      i += 1;
      continue;
    }
    // Deeper: a nested list run (items and their children).
    let s = i;
    while i < items.len() && (li_level(items[i]) > level || li_of(items[i]) == Some(level + 1)) {
      i += 1;
    }
    render_html_list(snapshot, &items[s..i], level + 1, out)?;
  }
  Ok(())
}

/// Inline marks rendered to nested HTML (<strong>/<em>/<code>/<del>/<a>),
/// mirroring the markdown render_span structure.
/// Escape inline TEXT for HTML and render hard breaks (`\` + newline in
/// the block text) as `<br />`.
fn html_text(seg: &str) -> String {
  escape_html(seg).replace("\\\n", "<br />\n")
}

fn html_inline(block: &Block) -> String {
  if matches!(block.kind.as_str(), "code_block" | "code" | "table") {
    return escape_html(&block.text);
  }
  let marks = marks_from_block(block);
  if marks.is_empty() {
    // No trimming here: the block text is already edge-trimmed at construction,
    // and a decoded entity such as `&nbsp;`/`&#9;` is real content that must
    // survive at the line's start or end.
    return html_text(&block.text);
  }
  let units: Vec<u16> = block.text.encode_utf16().collect();
  // An empty-text link (`[](/url)`) is a zero-width mark the span walker skips
  // because it only steps over non-empty ranges; emit its empty anchor here.
  if units.is_empty()
    && let Some(m) = marks.iter().find(|m| m.kind == "link")
  {
    return match &m.title {
      Some(t) => format!(
        "<a href=\"{}\" title=\"{}\"></a>",
        escape_html(&escape_href(m.href.as_deref().unwrap_or(""))),
        escape_html(t)
      ),
      None => format!(
        "<a href=\"{}\"></a>",
        escape_html(&escape_href(m.href.as_deref().unwrap_or("")))
      ),
    };
  }
  let refs: Vec<&InlineMark> = marks.iter().collect();
  html_span(&units, 0, units.len(), &refs)
}

fn html_span(units: &[u16], lo: usize, hi: usize, marks: &[&InlineMark]) -> String {
  let mut out = String::new();
  let mut pos = lo;
  while pos < hi {
    let next = marks
      .iter()
      .enumerate()
      .filter_map(|(i, m)| {
        let s = m.start.max(pos);
        let e = m.end.min(hi);
        (e > s).then_some((s, e, i))
      })
      // Earliest start, then widest span. On an exact range tie (e.g. the
      // strong+em pair of `***foo***`, whose delimiters all collapse to the
      // same edges) the mark matched LAST in `process_emphasis` is the outer
      // one, so prefer the higher index to nest it on the outside.
      .min_by_key(|&(s, e, i)| (s, usize::MAX - e, usize::MAX - i));
    let Some((s, e, picked)) = next else {
      out.push_str(&html_text(&String::from_utf16_lossy(&units[pos..hi])));
      break;
    };
    out.push_str(&html_text(&String::from_utf16_lossy(&units[pos..s])));
    let m = marks[picked];
    if m.kind == "code" {
      let raw = String::from_utf16_lossy(&units[s..e]);
      out.push_str(&format!("<code>{}</code>", escape_html(&raw)));
      pos = e;
      continue;
    }
    if m.kind == "html" {
      // Raw inline HTML passes through unescaped (GFM tagfilter applied, then
      // an event-handler / `javascript:`-URL scrub for no-CSP consumers).
      out.push_str(&strip_unsafe_attrs(&tagfilter(&String::from_utf16_lossy(&units[s..e]))));
      pos = e;
      continue;
    }
    if m.kind == "math" {
      let latex = String::from_utf16_lossy(&units[s..e]);
      match render_math(&latex, false) {
        Some(mathml) => out.push_str(&mathml),
        None => out.push_str(&format!("<span class=\"math\">{}</span>", escape_html(&latex))),
      }
      pos = e;
      continue;
    }
    if m.kind == "footnote" {
      // GFM reference shape: superscript backlink-anchored to the definition.
      // The label is the href; the visible text is the label too.
      let label = m.href.clone().unwrap_or_else(|| String::from_utf16_lossy(&units[s..e]));
      let id = escape_html(&label);
      out.push_str(&format!(
        "<sup id=\"fnref-{id}\"><a href=\"#fn-{id}\">{id}</a></sup>"
      ));
      pos = e;
      continue;
    }
    if m.kind == "image" {
      // The alt attribute is the PLAIN text of the span — inner marks flatten.
      let alt = String::from_utf16_lossy(&units[s..e]);
      let title_attr = match &m.title {
        Some(t) => format!(" title=\"{}\"", escape_html(t)),
        None => String::new(),
      };
      out.push_str(&format!(
        "<img src=\"{}\" alt=\"{}\"{title_attr} />",
        escape_html(&escape_href(m.href.as_deref().unwrap_or(""))),
        escape_html(&alt)
      ));
      pos = e;
      continue;
    }
    let inner: Vec<&InlineMark> = marks
      .iter()
      .enumerate()
      .filter(|&(i, x)| i != picked && x.end.min(e) > x.start.max(s))
      .map(|(_, x)| *x)
      .collect();
    let body = html_span(units, s, e, &inner);
    out.push_str(&match m.kind.as_str() {
      "bold" => format!("<strong>{body}</strong>"),
      "italic" => format!("<em>{body}</em>"),
      "strike" => format!("<del>{body}</del>"),
      "link" => match &m.title {
        Some(t) => format!(
          "<a href=\"{}\" title=\"{}\">{body}</a>",
          escape_html(&escape_href(m.href.as_deref().unwrap_or(""))),
          escape_html(t)
        ),
        None => format!(
          "<a href=\"{}\">{body}</a>",
          escape_html(&escape_href(m.href.as_deref().unwrap_or("")))
        ),
      },
      _ => body,
    });
    pos = e;
  }
  out
}

fn append_html_block(
  snapshot: &DocumentSnapshotPayload,
  block: &Block,
  out: &mut String,
) -> DocumentOperationResult<()> {
  let text = html_inline(block);
  match block.kind.as_str() {
    "heading" => {
      let level = heading_level(block, 0);
      out.push_str(&format!("<h{level}>{text}</h{level}>\n"));
      append_html_children(snapshot, &block.id, out)?;
    }
    "todo" => {
      let checked = if block_data_bool(block, "checked") {
        " checked"
      } else {
        ""
      };
      out.push_str(&format!(
        "<div class=\"todo\"><input type=\"checkbox\" disabled{checked}> {text}</div>\n"
      ));
      append_html_children(snapshot, &block.id, out)?;
    }
    "quote" => {
      if text.is_empty() {
        out.push_str("<blockquote>\n</blockquote>\n");
      } else {
        out.push_str(&format!("<blockquote>\n<p>{text}</p>\n</blockquote>\n"));
      }
      append_html_children(snapshot, &block.id, out)?;
    }
    "code_block" | "code" => {
      if block_data_bool(block, "raw") {
        // A raw HTML block passes through. Type-1 blocks (script/style/pre/
        // textarea) are literal and bypass the tagfilter; every other kind
        // gets it (so loose `<title>`/`<xmp>` etc. are still neutralized).
        // The kind is re-derived from the block's own opening line rather than
        // persisted, keeping the cross-language block shape unchanged.
        let first_line = block.text.lines().next().unwrap_or("").trim_start();
        if html_block_start(first_line) == Some(1) {
          // Type-1 (script/style/pre/textarea) stays byte-for-byte verbatim to
          // preserve CommonMark round-trip fidelity — NOT scrubbed here; the
          // share response's strict CSP is what neutralizes it.
          out.push_str(&block.text);
        } else {
          out.push_str(&strip_unsafe_attrs(&tagfilter(&block.text)));
        }
        out.push('\n');
        return Ok(());
      }
      let lang = block_data_str(block, "language").unwrap_or("");
      // Mermaid renders to a self-contained inline SVG (feature `render`); a
      // syntax/render failure or the feature being off falls through to the
      // plain code block below, so the source is never lost.
      if lang == "mermaid" {
        if let Some(svg) = render_mermaid_svg_with_id(&block.text, &mermaid_id(&block.id)) {
          out.push_str("<div class=\"mermaid\">");
          out.push_str(&svg);
          out.push_str("</div>\n");
          append_html_children(snapshot, &block.id, out)?;
          return Ok(());
        }
      }
      let class = if lang.is_empty() {
        String::new()
      } else {
        format!(" class=\"language-{}\"", escape_html(lang))
      };
      let body = if block.text.is_empty() {
        String::new()
      } else {
        format!("{}\n", escape_html(&block.text))
      };
      out.push_str(&format!("<pre><code{class}>{body}</code></pre>\n"));
      append_html_children(snapshot, &block.id, out)?;
    }
    "table" => {
      let Some(rows) = block.data.get("rows").and_then(Value::as_array) else {
        return Ok(());
      };
      let aligns = block.data.get("aligns").and_then(Value::as_array);
      let attr = |c: usize| -> String {
        match aligns
          .and_then(|a| a.get(c))
          .and_then(Value::as_str)
          .unwrap_or("")
        {
          "" => String::new(),
          a => format!(" align=\"{a}\""),
        }
      };
      let cell_html = |raw: &str| -> String {
        let (text, data) = apply_inline_marks(raw.to_string(), Value::Null, &RefDefs::new());
        html_inline(&Block {
          id: String::new(),
          kind: "paragraph".to_string(),
          text,
          data,
          children: Vec::new(),
        })
      };
      let row_cells = |row: &Value| -> Vec<String> {
        row
          .as_array()
          .map(|cells| {
            cells
              .iter()
              .map(|c| c.as_str().unwrap_or("").trim().to_string())
              .collect()
          })
          .unwrap_or_default()
      };
      out.push_str("<table>\n<thead>\n<tr>\n");
      if let Some(head) = rows.first() {
        for (c, cell) in row_cells(head).iter().enumerate() {
          out.push_str(&format!("<th{}>{}</th>\n", attr(c), cell_html(cell)));
        }
      }
      out.push_str("</tr>\n</thead>\n");
      if rows.len() > 1 {
        out.push_str("<tbody>\n");
        for row in rows.iter().skip(1) {
          out.push_str("<tr>\n");
          for (c, cell) in row_cells(row).iter().enumerate() {
            out.push_str(&format!("<td{}>{}</td>\n", attr(c), cell_html(cell)));
          }
          out.push_str("</tr>\n");
        }
        out.push_str("</tbody>\n");
      }
      out.push_str("</table>\n");
    }
    "math_block" => {
      match render_math(&block.text, true) {
        Some(mathml) => out.push_str(&format!("<div class=\"math\">{mathml}</div>\n")),
        None => {
          out.push_str(&format!("<div class=\"math\">{}</div>\n", escape_html(&block.text)))
        }
      }
    }
    "footnote_def" => {
      // Definitions are not rendered in document order — they are gathered
      // into a single trailing <section class="footnotes"> by export_html.
    }
    "divider" => {
      out.push_str("<hr />\n");
    }
    "image" => {
      let url = escape_html(&escape_href(block_data_str(block, "url").unwrap_or_default()));
      let alt = escape_html(block.text.trim());
      let title_attr = match block_data_str(block, "title") {
        Some(t) => format!(" title=\"{}\"", escape_html(t)),
        None => String::new(),
      };
      out.push_str(&format!(
        "<p><img src=\"{url}\" alt=\"{alt}\"{title_attr} /></p>\n"
      ));
    }
    _ => {
      if !text.is_empty() {
        out.push_str(&format!("<p>{text}</p>\n"));
      }
      append_html_children(snapshot, &block.id, out)?;
    }
  }

  Ok(())
}

fn block_for<'a>(
  snapshot: &'a DocumentSnapshotPayload,
  block_id: &str,
) -> DocumentOperationResult<&'a Block> {
  let index = block_index(snapshot, block_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(block_id.to_string()))?;
  Ok(&snapshot.blocks[index])
}

fn list_tag_for(kind: &str) -> Option<&'static str> {
  match kind {
    "bulleted_list" | "bullet_list" | "todo" => Some("ul"),
    "numbered_list" | "number_list" => Some("ol"),
    _ => None,
  }
}

/// GFM tagfilter: escape the opening `<` of the nine disallowed raw-HTML
/// tags so they render inert while everything else passes through.
fn tagfilter(s: &str) -> String {
  const BAD: &[&str] = &[
    "title", "textarea", "style", "xmp", "iframe", "noembed", "noframes",
    "script", "plaintext",
  ];
  let bytes = s.as_bytes();
  let mut out = String::with_capacity(s.len());
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'<' {
      let mut j = i + 1;
      if bytes.get(j) == Some(&b'/') {
        j += 1;
      }
      let name_start = j;
      while bytes.get(j).is_some_and(|b| b.is_ascii_alphabetic()) {
        j += 1;
      }
      let name = s[name_start..j].to_ascii_lowercase();
      let boundary = matches!(bytes.get(j), None | Some(b' ' | b'\t' | b'\n' | b'>' | b'/'));
      if BAD.contains(&name.as_str()) && boundary {
        out.push_str("&lt;");
        i += 1;
        continue;
      }
    }
    out.push(s.as_bytes()[i] as char);
    i += 1;
  }
  out
}

/// Names of attributes whose value is fetched as a URL — the vectors for a
/// `javascript:`/`vbscript:` scheme injection. Lower-cased for comparison.
const URL_ATTRS: &[&str] = &[
  "href", "src", "xlink:href", "action", "formaction", "poster",
];

/// Is `name`+`value` a dangerous attribute that must be dropped?
///
/// Two rules: (1) any `on…` event-handler attribute (`on` + one-or-more ASCII
/// letters, case-insensitive — `onerror`, `OnLoad`, …); (2) a URL attribute
/// whose value's scheme is `javascript:` / `vbscript:`. For the scheme test we
/// strip ASCII whitespace and control bytes (leading AND embedded up to the
/// colon) before comparing, so `  JAVAScript:` and `java\tscript:` are both
/// caught. Entity-encoded schemes (`&#106;avascript:`) are NOT decoded here —
/// that residual vector is why the share response still carries a strict CSP.
fn is_unsafe_attr(name: &str, value: &str) -> bool {
  let nb = name.as_bytes();
  if nb.len() > 2
    && nb[0].eq_ignore_ascii_case(&b'o')
    && nb[1].eq_ignore_ascii_case(&b'n')
    && nb[2..].iter().all(u8::is_ascii_alphabetic)
  {
    return true;
  }
  let lname = name.to_ascii_lowercase();
  if URL_ATTRS.contains(&lname.as_str()) {
    // Probe = value with all ASCII whitespace/control bytes removed, lowercased.
    // A real URL never collapses to a `javascript:`/`vbscript:` prefix, so this
    // has no realistic false positive while closing the tab/newline bypass.
    let probe: String = value
      .bytes()
      .filter(|b| *b > 0x20 && *b != 0x7f)
      .map(|b| b.to_ascii_lowercase() as char)
      .take(11) // len("javascript:")
      .collect();
    if probe.starts_with("javascript:") || probe.starts_with("vbscript:") {
      return true;
    }
  }
  false
}

/// Defense-in-depth over raw HTML: strip event-handler (`on*`) attributes and
/// `javascript:`/`vbscript:` URL attributes from element tags. This is a
/// belt-and-suspenders layer for consumers WITHOUT a CSP (e.g. a locally saved
/// `.html` download); the share response's own strict CSP remains the primary
/// XSS defense. Written in the same hand-rolled byte-scanner style as
/// [tagfilter], with no regex dependency.
///
/// Only the INSIDE of an element tag is scanned. Text nodes are copied byte-for
/// byte (UTF-8 preserved by slicing at ASCII boundaries only). Comments
/// (`<!-- -->`), declarations (`<!…>`) and processing instructions (`<?…?>`)
/// pass through verbatim — never scanned for attributes.
fn strip_unsafe_attrs(s: &str) -> String {
  let b = s.as_bytes();
  let mut out = String::with_capacity(s.len());
  let mut i = 0;
  while i < b.len() {
    // Text run: copy verbatim up to the next '<' (keeps multibyte UTF-8 intact).
    if b[i] != b'<' {
      let start = i;
      while i < b.len() && b[i] != b'<' {
        i += 1;
      }
      out.push_str(&s[start..i]);
      continue;
    }
    // `<!-- … -->` comment: copy verbatim, do not scan.
    if s[i + 1..].starts_with("!--") {
      let mut k = i + 4;
      while k + 2 < b.len() && !(b[k] == b'-' && b[k + 1] == b'-' && b[k + 2] == b'>') {
        k += 1;
      }
      let end = if k + 2 < b.len() { k + 3 } else { b.len() };
      out.push_str(&s[i..end]);
      i = end;
      continue;
    }
    // `<! … >` declaration (e.g. DOCTYPE): copy verbatim to the next '>'.
    if b.get(i + 1) == Some(&b'!') {
      let mut k = i + 2;
      while k < b.len() && b[k] != b'>' {
        k += 1;
      }
      let end = (k + 1).min(b.len());
      out.push_str(&s[i..end]);
      i = end;
      continue;
    }
    // `<? … ?>` processing instruction: copy verbatim.
    if b.get(i + 1) == Some(&b'?') {
      let mut k = i + 2;
      while k + 1 < b.len() && !(b[k] == b'?' && b[k + 1] == b'>') {
        k += 1;
      }
      let end = if k + 1 < b.len() { k + 2 } else { b.len() };
      out.push_str(&s[i..end]);
      i = end;
      continue;
    }
    // A tag: `<` optional `/` then a name starting with an ASCII letter.
    let mut j = i + 1;
    let closing = b.get(j) == Some(&b'/');
    if closing {
      j += 1;
    }
    if !b.get(j).is_some_and(u8::is_ascii_alphabetic) {
      // Not a real tag start — emit the bare '<' as literal text.
      out.push('<');
      i += 1;
      continue;
    }
    // Emit `<`, optional `/`, then the tag name.
    out.push('<');
    if closing {
      out.push('/');
    }
    let name_start = j;
    while b
      .get(j)
      .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-')
    {
      j += 1;
    }
    out.push_str(&s[name_start..j]);
    // Attribute region: loop until the tag terminator (`>` / `/>` / EOF).
    loop {
      let ws_start = j;
      while b.get(j).is_some_and(u8::is_ascii_whitespace) {
        j += 1;
      }
      let ws = &s[ws_start..j];
      match b.get(j) {
        None => {
          out.push_str(ws);
          break;
        }
        Some(&b'>') => {
          out.push_str(ws);
          out.push('>');
          j += 1;
          break;
        }
        Some(&b'/') if b.get(j + 1) == Some(&b'>') => {
          out.push_str(ws);
          out.push_str("/>");
          j += 2;
          break;
        }
        _ => {}
      }
      // Parse one attribute: name [ = value ].
      let attr_start = j;
      while b
        .get(j)
        .is_some_and(|c| !c.is_ascii_whitespace() && !matches!(c, b'=' | b'>' | b'/'))
      {
        j += 1;
      }
      if j == attr_start {
        // A stray char (e.g. a lone '/') that isn't a tag end — emit and move on
        // so the scanner can't stall.
        out.push_str(ws);
        out.push(b[j] as char);
        j += 1;
        continue;
      }
      let name = &s[attr_start..j];
      let mut value = "";
      // Optional value, allowing whitespace around '=' (HTML-legal).
      let mut k = j;
      while b.get(k).is_some_and(u8::is_ascii_whitespace) {
        k += 1;
      }
      if b.get(k) == Some(&b'=') {
        k += 1;
        while b.get(k).is_some_and(u8::is_ascii_whitespace) {
          k += 1;
        }
        match b.get(k) {
          Some(&b'"') => {
            let vs = k + 1;
            k += 1;
            while b.get(k).is_some_and(|c| *c != b'"') {
              k += 1;
            }
            value = &s[vs..k.min(b.len())];
            if b.get(k) == Some(&b'"') {
              k += 1;
            }
          }
          Some(&b'\'') => {
            let vs = k + 1;
            k += 1;
            while b.get(k).is_some_and(|c| *c != b'\'') {
              k += 1;
            }
            value = &s[vs..k.min(b.len())];
            if b.get(k) == Some(&b'\'') {
              k += 1;
            }
          }
          _ => {
            let vs = k;
            while b.get(k).is_some_and(|c| !c.is_ascii_whitespace() && *c != b'>') {
              k += 1;
            }
            value = &s[vs..k];
          }
        }
        j = k;
      }
      let attr_text = &s[attr_start..j];
      if is_unsafe_attr(name, value) {
        // Drop: skip the leading whitespace AND the attribute entirely.
      } else {
        out.push_str(ws);
        out.push_str(attr_text);
      }
    }
    // Advance the outer cursor past the tag the attribute loop just consumed
    // (`j` walked to the `>` / `/>` / EOF). Without this the outer `while` would
    // re-scan the same tag forever.
    i = j;
  }
  out
}

/// Percent-encode a URL for an href attribute (cmark's houdini set: keep
/// alphanumerics and `-_.+!*'(),%#@?=;:/,&$~`; encode the rest per UTF-8
/// byte). HTML-escape the result separately.
fn escape_href(input: &str) -> String {
  const SAFE: &[u8] = b"-_.+!*'(),%#@?=;:/,&$~";
  let mut out = String::with_capacity(input.len());
  for &b in input.as_bytes() {
    if b.is_ascii_alphanumeric() || SAFE.contains(&b) {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}

fn escape_html(input: &str) -> String {
  let mut out = String::with_capacity(input.len());
  for ch in input.chars() {
    match ch {
      '&' => out.push_str("&amp;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      '"' => out.push_str("&quot;"),
      other => out.push(other),
    }
  }
  out
}


fn append_markdown_children(
  snapshot: &DocumentSnapshotPayload,
  parent_id: &str,
  depth: usize,
  lines: &mut Vec<String>,
  images: &BTreeMap<String, String>,
) -> DocumentOperationResult<()> {
  let parent_index = block_index(snapshot, parent_id)
    .ok_or_else(|| DocumentOperationError::BlockNotFound(parent_id.to_string()))?;
  let child_ids = snapshot.blocks[parent_index].children.clone();

  let is_item = |kind: &str| {
    matches!(kind, "bulleted_list" | "bullet_list" | "numbered_list" | "number_list" | "todo")
  };
  let mut prev_was_item = false;
  let mut prev_quote = 0usize;
  for child_id in child_ids {
    let child_index = block_index(snapshot, &child_id)
      .ok_or_else(|| DocumentOperationError::BlockNotFound(child_id.clone()))?;
    let child = &snapshot.blocks[child_index];
    let quote = quote_depth_of(child);
    let li_child = li_of(child);
    // A list runs until a blank line: separate it from whatever follows so
    // the next block can't be read back as a lazy continuation. Container
    // children (`data.li`) stay inside the run.
    if prev_was_item && !is_item(&child.kind) && li_child.is_none() {
      lines.push(String::new());
    }
    // A quote group ends at depth 0 or a recorded break — a plain blank
    // line keeps the next block out of the quote on re-import.
    if prev_quote > 0 && li_child.is_none() && (quote == 0 || child.data.get("qbreak").is_some()) {
      lines.push(String::new());
    }
    // A paragraph child needs a blank before it (that's what made it a
    // child paragraph rather than a text join on import).
    if li_child.is_some() && child.kind == "paragraph" {
      lines.push(String::new());
    }
    prev_was_item = is_item(&child.kind) || li_child.is_some();
    prev_quote = if li_child.is_some() { 0 } else { quote };
    let from = lines.len();
    append_markdown_block(snapshot, child, depth, lines, images)?;
    if quote > 0 {
      // Re-prefix every emitted line with the quote markers; blanks become
      // bare `>` so paragraph breaks stay inside the quote.
      let marker = "> ".repeat(quote);
      let bare = marker.trim_end().to_string();
      for l in lines[from..].iter_mut() {
        *l = if l.is_empty() {
          bare.clone()
        } else {
          format!("{marker}{l}")
        };
      }
    }
    if let Some(level) = li_child {
      // Container children sit at the owning item's content column.
      let indent = " ".repeat(level * 4 + 4);
      for l in lines[from..].iter_mut() {
        if !l.is_empty() {
          *l = format!("{indent}{l}");
        }
      }
      // Code/divider arms append a trailing blank — drop it so a following
      // sibling item doesn't read as loose.
      while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
      }
    }
  }

  Ok(())
}

fn append_markdown_block(
  snapshot: &DocumentSnapshotPayload,
  block: &Block,
  depth: usize,
  lines: &mut Vec<String>,
  images: &BTreeMap<String, String>,
) -> DocumentOperationResult<()> {
  match block.kind.as_str() {
    "heading" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth, lines, images)?;
    }
    "todo" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth + 1, lines, images)?;
    }
    "bulleted_list" | "bullet_list" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth + 1, lines, images)?;
    }
    "numbered_list" | "number_list" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth + 1, lines, images)?;
    }
    "quote" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth, lines, images)?;
    }
    "code_block" | "code" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth, lines, images)?;
    }
    "image" => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth, lines, images)?;
    }
    _ => {
      append_markdown_block_content(block, depth, lines, images);
      append_markdown_children(snapshot, &block.id, depth, lines, images)?;
    }
  }

  Ok(())
}

/// Emit one list/todo item: a leading blank if the item is loose (so the
/// list re-imports loose), the marker line, then continuation lines indented
/// to the item's content column.
fn push_list_item(lines: &mut Vec<String>, block: &Block, indent: &str, marker: &str, rich: &str) {
  if block_data_bool(block, "loose") {
    lines.push(String::new());
  }
  let pad = " ".repeat(marker.len());
  for (i, l) in rich.split('\n').enumerate() {
    if i == 0 {
      lines.push(format!("{indent}{marker}{l}"));
    } else {
      lines.push(format!("{indent}{pad}{l}"));
    }
  }
}

fn append_markdown_block_content(
  block: &Block,
  depth: usize,
  lines: &mut Vec<String>,
  images: &BTreeMap<String, String>,
) {
  // List/todo nesting from `data.indent` (4 spaces per level — valid
  // CommonMark continuation for both `- ` and `1. ` parents); other kinds
  // keep the children-tree depth indent. `list_indent` applies the same 0..=8
  // cap the Dart editor uses, so both engines indent identically (P1-4).
  let level = list_indent(block);
  let indent = if matches!(block.kind.as_str(), "bulleted_list" | "bullet_list" | "numbered_list" | "number_list" | "todo") {
    "    ".repeat(level)
  } else {
    "  ".repeat(depth)
  };
  let text = block.text.trim_end();
  // Inline marks (bold/italic/code/strike/link) rendered back to Markdown.
  let rich = render_inline(block);
  let rich = rich.trim_end();
  match block.kind.as_str() {
    "heading" => {
      let level = heading_level(block, depth);
      if rich.contains('\n') && level <= 2 {
        // Multi-line headings only exist in setext form.
        lines.extend(rich.split('\n').map(str::to_string));
        lines.push(if level == 1 { "===" } else { "---" }.to_string());
      } else {
        lines.push(format!("{} {}", "#".repeat(level), rich.replace('\n', " ")));
      }
      lines.push(String::new());
    }
    "todo" => {
      let marker = if block_data_bool(block, "checked") {
        "x"
      } else {
        " "
      };
      push_list_item(lines, block, &indent, &format!("- [{marker}] "), rich);
    }
    "bulleted_list" | "bullet_list" => {
      let marker = block_data_str(block, "marker").unwrap_or("-");
      push_list_item(lines, block, &indent, &format!("{marker} "), rich);
    }
    "numbered_list" | "number_list" => {
      // Only a run's first item carries `start`; the rest stay `1.` (valid
      // CommonMark — ordered numbering continues regardless). A recorded
      // `marker` restores the `)` delimiter that broke the previous run.
      let n = block.data.get("start").and_then(Value::as_u64).unwrap_or(1);
      let delim = block_data_str(block, "marker").unwrap_or(".");
      push_list_item(lines, block, &indent, &format!("{n}{delim} "), rich);
    }
    "quote" => {
      // Marker prefixes are added by the children walker (depth-aware).
      lines.extend(rich.split('\n').map(str::to_string));
      lines.push(String::new());
    }
    "code_block" | "code" => {
      if block_data_bool(block, "raw") {
        // A raw HTML block writes back verbatim — no fences — so foreign
        // viewers still render the HTML.
        lines.extend(block.text.lines().map(str::to_string));
        lines.push(String::new());
      } else {
        let language = block_data_str(block, "language").unwrap_or("");
        lines.push(format!("```{language}"));
        lines.extend(block.text.lines().map(str::to_string));
        lines.push("```".to_string());
        lines.push(String::new());
      }
    }
    "image" => {
      // Prefer a bundled asset path (ZIP export) for an uploaded image; else
      // fall back to its original filename, then a raw external `url`.
      let asset = block_data_str(block, "file_id").and_then(|id| images.get(id));
      let target: String = match asset {
        Some(path) => path.clone(),
        None => block_data_str(block, "name")
          .filter(|s| !s.is_empty())
          .or_else(|| block_data_str(block, "url"))
          .unwrap_or_default()
          .to_string(),
      };
      // Keep an empty alt empty — `![](url)` must round-trip unchanged.
      match block_data_str(block, "title") {
        Some(t) => lines.push(format!("![{text}]({target} \"{}\")", t.replace('"', "\\\""))),
        None => lines.push(format!("![{text}]({target})")),
      }
      lines.push(String::new());
    }
    "table" => {
      append_table_markdown(block, lines);
    }
    "math_block" => {
      lines.push("$$".to_string());
      lines.extend(block.text.lines().map(str::to_string));
      lines.push("$$".to_string());
      lines.push(String::new());
    }
    "footnote_def" => {
      // `[^label]: content`; continuation lines indent 4 columns (GFM). The
      // content carries inline marks, so use the rich render, not raw text.
      let label = block_data_str(block, "label").unwrap_or("");
      let mut body = rich.lines();
      let first = body.next().unwrap_or("");
      lines.push(format!("[^{label}]: {first}"));
      for cont in body {
        lines.push(if cont.is_empty() {
          String::new()
        } else {
          format!("    {cont}")
        });
      }
      lines.push(String::new());
    }
    "divider" => {
      lines.push("---".to_string());
      lines.push(String::new());
    }
    _ => {
      if !text.is_empty() {
        let rich = escape_block_leader(rich.to_string());
        lines.push(format!("{indent}{rich}"));
        lines.push(String::new());
      }
    }
  }
}

/// Parse inline Markdown (`**b**`, `*i*`/`_i_`, `` `c` ``, `~~s~~`, `[t](url)`)
/// in [text] into clean text plus marks merged into [data] under `"marks"`.
/// Offsets are UTF-16 code-unit indices to match the Flutter client.
fn apply_inline_marks(text: String, data: Value, defs: &RefDefs) -> (String, Value) {
  let parsed = parse_inline_with(&text, defs);
  if parsed.marks.is_empty() {
    return (parsed.text, data);
  }
  let marks: Vec<Value> = parsed
    .marks
    .iter()
    .map(|m| {
      let mut obj = serde_json::Map::new();
      obj.insert("start".into(), json!(m.start));
      obj.insert("end".into(), json!(m.end));
      obj.insert("type".into(), json!(m.kind));
      if let Some(href) = &m.href {
        obj.insert("href".into(), json!(href));
      }
      if let Some(title) = &m.title {
        obj.insert("title".into(), json!(title));
      }
      Value::Object(obj)
    })
    .collect();
  let mut obj = match data {
    Value::Object(map) => map,
    _ => serde_json::Map::new(),
  };
  obj.insert("marks".into(), Value::Array(marks));
  (parsed.text, Value::Object(obj))
}

struct ParsedInline {
  text: String,
  marks: Vec<InlineMark>,
}

/// Link reference definitions: normalized label → (destination, title).
type RefDefs = std::collections::HashMap<String, (String, Option<String>)>;

/// Scan all link reference definitions, fence-aware and multi-line: the
/// destination may sit on the line after `[label]:`, and a (quoted) title
/// may follow on its own line(s). Returns the defs and the set of consumed
/// line indices.
fn collect_ref_definitions(
  raw_lines: &[&str],
) -> (RefDefs, std::collections::HashSet<usize>, std::collections::HashMap<usize, usize>) {
  let mut defs = RefDefs::new();
  let mut def_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();
  // Definitions inside blockquotes (`> [foo]: /url`) — line → quote depth.
  // They define globally; the line itself renders as a contentless `>`
  // (spec ex. 218: the quote ends up empty, the def still resolves).
  let mut quote_def_lines: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
  let mut in_fence: Option<(u8, usize)> = None;
  let mut prev_para = false; // an open paragraph means a def can't start
  let mut i = 0;
  while i < raw_lines.len() {
    let line = raw_lines[i].trim_end();
    let content = line.trim_start();
    let col = line.len() - content.len();
    if let Some((c, n)) = in_fence {
      if fence_close(content, c, n) {
        in_fence = None;
      }
      prev_para = false;
      i += 1;
      continue;
    }
    if col < 4
      && let Some((c, n, _)) = fence_open(content)
    {
      in_fence = Some((c, n));
      prev_para = false;
      i += 1;
      continue;
    }
    if content.is_empty() {
      prev_para = false;
      i += 1;
      continue;
    }
    if !prev_para
      && col < 4
      && content.starts_with('[')
      && let Some((label, dest, title, used)) = parse_ref_definition_multi(raw_lines, i)
    {
      defs.entry(normalize_label(&label)).or_insert((dest, title));
      for k in i..i + used {
        def_lines.insert(k);
      }
      i += used;
      continue; // a definition doesn't open a paragraph
    }
    // A single-line definition inside a blockquote (multi-line defs would
    // need marker-stripped continuation — not a real-world shape).
    if !prev_para && col < 4 && content.starts_with('>') {
      let (qd, qrest) = strip_quote_markers(content);
      let qrest = qrest.trim_start();
      if qd > 0
        && qrest.starts_with('[')
        && let Some((label, dest, title, used)) =
          parse_ref_definition_multi(&[qrest], 0)
        && used == 1
      {
        defs.entry(normalize_label(&label)).or_insert((dest, title));
        quote_def_lines.insert(i, qd);
        i += 1;
        continue;
      }
    }
    // Only true paragraph text can lazily absorb a following def line. Single
    // line blocks (ATX headings, thematic breaks) don't, so a def on the next
    // line still counts — `# [Foo]\n[foo]: /url` defines `foo`.
    prev_para = heading_prefix_level(content).is_none() && !is_divider(content);
    i += 1;
  }
  (defs, def_lines, quote_def_lines)
}

/// `[label]:` at lines[i]; destination on the same or next line; optional
/// quoted title after the destination (same line needs whitespace between)
/// or on following line(s) — titles may span lines. Returns the parsed def
/// and how many lines it consumed.
fn parse_ref_definition_multi(
  raw_lines: &[&str],
  i: usize,
) -> Option<(String, String, Option<String>, usize)> {
  // The label may wrap across lines (`[Foo\n  bar]: /url`): join consecutive
  // lines until the `]:` closer appears, counting how many we spanned.
  let first = raw_lines[i].trim_start();
  let mut joined = first.to_string();
  let mut label_lines = 1;
  let (close, chars) = loop {
    let chars: Vec<char> = joined.chars().collect();
    if let Some(close) = matching_bracket(&chars, 0)
      && chars.get(close + 1) == Some(&':')
    {
      break (close, chars);
    }
    // No closer yet — pull in the next line (label-only blank lines are not
    // allowed inside a definition label).
    let next = raw_lines.get(i + label_lines)?;
    if next.trim().is_empty() {
      return None;
    }
    joined.push('\n');
    joined.push_str(next.trim());
    label_lines += 1;
    if label_lines > 8 {
      return None; // a runaway label is not a definition
    }
  };
  let label: String = chars[1..close].iter().collect();
  if label.trim().is_empty() {
    return None;
  }
  // `[^label]:` is a GFM footnote definition, a block of its own — never a
  // link reference definition. Leave it for the main block scanner.
  if label.starts_with('^') {
    return None;
  }
  for (k, &c) in chars[1..close].iter().enumerate() {
    if (c == '[' || c == ']') && (k == 0 || chars[k] != '\\') {
      return None;
    }
  }
  let after_colon: String = chars[close + 2..].iter().collect();
  let after_colon = after_colon.trim();
  let mut used = label_lines;
  let dest_line = if after_colon.is_empty() {
    // destination on the line after the (possibly multi-line) label
    let l2 = raw_lines.get(i + label_lines)?.trim();
    if l2.is_empty() {
      return None;
    }
    used = label_lines + 1;
    l2.to_string()
  } else {
    after_colon.to_string()
  };
  let (dest, rest, had_ws) = parse_def_dest(&dest_line)?;
  if !rest.is_empty() {
    // a same-line title needs whitespace after the destination
    if !had_ws {
      return None;
    }
    let title = parse_def_title(raw_lines, i + used - 1, &rest)?;
    return Some((label, dest, Some(title.0), used + title.1));
  }
  // maybe a title on the following line(s)
  if let Some(next) = raw_lines.get(i + used) {
    let nt = next.trim();
    if !nt.is_empty()
      && matches!(nt.as_bytes()[0], b'"' | b'\'' | b'(')
      && let Some(title) = parse_def_title(raw_lines, i + used, nt)
    {
      return Some((label, dest, Some(title.0), used + 1 + title.1));
    }
  }
  Some((label, dest, None, used))
}

/// Destination: `<angle form>` or a run of non-whitespace. Returns the
/// unescaped dest, the rest of the line (trimmed), and whether whitespace
/// separated them.
fn parse_def_dest(s: &str) -> Option<(String, String, bool)> {
  if let Some(inner) = s.strip_prefix('<') {
    let gt = inner.find('>')?;
    let rest = &inner[gt + 1..];
    let had_ws = rest.is_empty() || rest.starts_with([' ', '\t']);
    return Some((unescape_md(&inner[..gt]), rest.trim().to_string(), had_ws));
  }
  let end = s.find([' ', '\t']).unwrap_or(s.len());
  if end == 0 {
    return None;
  }
  Some((
    unescape_md(&s[..end]),
    s[end..].trim().to_string(),
    end < s.len(),
  ))
}

/// A quoted title starting at `start` (the trimmed tail of line `line_idx`),
/// possibly spanning lines. Returns (title, extra lines consumed); fails if
/// non-whitespace follows the closing delimiter.
fn parse_def_title(
  raw_lines: &[&str],
  line_idx: usize,
  start: &str,
) -> Option<(String, usize)> {
  let open = start.chars().next()?;
  let close = match open {
    '"' => '"',
    '\'' => '\'',
    '(' => ')',
    _ => return None,
  };
  // same-line close?
  let body = &start[1..];
  if let Some(pos) = find_unescaped_char(body, close) {
    if !body[pos + close.len_utf8()..].trim().is_empty() {
      return None;
    }
    return Some((unescape_md(&body[..pos]), 0));
  }
  // spans lines: accumulate until a line containing the unescaped closer
  let mut acc = body.to_string();
  let mut extra = 0usize;
  loop {
    extra += 1;
    let l = raw_lines.get(line_idx + extra)?;
    let lt = l.trim_end();
    // A blank line may not interrupt a title — it voids the definition
    // (spec ex. 197: everything renders back as paragraphs).
    if lt.trim().is_empty() {
      return None;
    }
    if let Some(pos) = find_unescaped_char(lt, close) {
      if !lt[pos + close.len_utf8()..].trim().is_empty() {
        return None;
      }
      acc.push('\n');
      acc.push_str(&lt[..pos]);
      return Some((unescape_md(&acc), extra));
    }
    acc.push('\n');
    acc.push_str(lt);
  }
}

fn find_unescaped_char(s: &str, target: char) -> Option<usize> {
  let mut prev_backslash = false;
  for (idx, c) in s.char_indices() {
    if c == target && !prev_backslash {
      return Some(idx);
    }
    prev_backslash = c == '\\' && !prev_backslash;
  }
  None
}

fn normalize_label(label: &str) -> String {
  // Reference labels match under Unicode CASE FOLDING, not mere lowercase:
  // ẞ/ß and SS all collapse to "ss" (the only 1:N fold real labels hit).
  label
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase()
    .replace('ß', "ss")
}

/// Unescape backslash-escaped ASCII punctuation (destinations/titles).
fn unescape_md(s: &str) -> String {
  let chars: Vec<char> = s.chars().collect();
  let mut out = String::with_capacity(s.len());
  let mut i = 0;
  while i < chars.len() {
    if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
      out.push(chars[i + 1]);
      i += 2;
    } else if let Some((decoded, used)) = parse_entity(&chars, i) {
      // Entity references decode inside destinations/titles/info strings.
      out.push_str(&decoded);
      i += used;
    } else {
      out.push(chars[i]);
      i += 1;
    }
  }
  out
}

/// Parse an inline-link suffix `(dest "title")` starting right AFTER the `(`.
/// Returns (href, title, index just past the closing `)`).
fn parse_link_suffix(chars: &[char], mut i: usize) -> Option<(String, Option<String>, usize)> {
  // Only ASCII whitespace separates destination and title — U+00A0 and
  // friends are CONTENT (spec ex. 507: the whole thing becomes the href).
  fn md_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000B}' | '\u{000C}')
  }
  let n = chars.len();
  while i < n && md_ws(chars[i]) {
    i += 1;
  }
  // Destination: <may contain spaces> or bare with balanced parens.
  let dest: String;
  if i < n && chars[i] == '<' {
    let mut j = i + 1;
    // A backslash escapes the next char, so `<foo\>` is unterminated (the
    // `\>` is a literal `>`, not the closer) — keep scanning past it.
    while j < n && chars[j] != '>' && chars[j] != '\n' && chars[j] != '<' {
      j += if chars[j] == '\\' && j + 1 < n { 2 } else { 1 };
    }
    if j >= n || chars[j] != '>' {
      return None;
    }
    dest = unescape_md(&chars[i + 1..j].iter().collect::<String>());
    i = j + 1;
  } else {
    let mut depth = 0i32;
    let start = i;
    while i < n {
      let c = chars[i];
      if md_ws(c) {
        break;
      }
      if c == '\\' && i + 1 < n {
        i += 2;
        continue;
      }
      match c {
        '(' => depth += 1,
        ')' => {
          if depth == 0 {
            break;
          }
          depth -= 1;
        }
        _ => {}
      }
      i += 1;
    }
    if depth != 0 {
      return None;
    }
    dest = unescape_md(&chars[start..i].iter().collect::<String>());
  }
  while i < n && md_ws(chars[i]) {
    i += 1;
  }
  // Optional title: "..." / '...' / (...)
  let mut title = None;
  if i < n && matches!(chars[i], '"' | '\'' | '(') {
    let close = if chars[i] == '(' { ')' } else { chars[i] };
    let mut j = i + 1;
    let start = j;
    while j < n && chars[j] != close {
      // A backslash escapes the next char (so an escaped close delimiter does
      // not end the title); keep the raw span and decode it once at the end.
      if chars[j] == '\\' && j + 1 < n {
        j += 2;
        continue;
      }
      j += 1;
    }
    if j >= n {
      return None;
    }
    // Titles decode backslash escapes AND entity references (`&quot;` → `"`).
    title = Some(unescape_md(&chars[start..j].iter().collect::<String>()));
    i = j + 1;
    while i < n && md_ws(chars[i]) {
      i += 1;
    }
  }
  if i < n && chars[i] == ')' { Some((dest, title, i + 1)) } else { None }
}

// NOTE: the uncached `label_contains_link` was removed — every caller now goes
// through `label_has_link_cached`, which shares one memo across the recursive
// parses (see LinkCache). Reintroducing an uncached path would silently restore
// the exponential blow-up on nested links.

/// If a code span, autolink, or raw inline HTML span starts at `chars[i]`,
/// return its exclusive end index. These spans bind tighter than link
/// brackets, so a bracket-matcher must skip over them as a unit.
fn inline_span_end(chars: &[char], i: usize) -> Option<usize> {
  let n = chars.len();
  match chars[i] {
    '`' => {
      // N-backtick run closes only on a run of exactly N.
      let mut run = 0;
      while i + run < n && chars[i + run] == '`' {
        run += 1;
      }
      let mut j = i + run;
      while j < n {
        if chars[j] == '`' {
          let mut m = 0;
          while j + m < n && chars[j + m] == '`' {
            m += 1;
          }
          if m == run {
            return Some(j + m);
          }
          j += m;
        } else {
          j += 1;
        }
      }
      None
    }
    '<' => {
      // Autolink first (it constrains the closing `>`), then raw HTML.
      let mut j = i + 1;
      while j < n && chars[j] != '>' {
        j += 1;
      }
      if j < n {
        let inner: String = chars[i + 1..j].iter().collect();
        if autolink_target(&inner).is_some() {
          return Some(j + 1);
        }
      }
      inline_html_end(chars, i)
    }
    _ => None,
  }
}

/// Find the `]` matching the `[` at [open], honoring nesting and escapes.
///
/// Code spans, autolinks and raw inline HTML bind tighter than link brackets
/// (CommonMark §6.5), so a `]` inside one of those does not close the link:
/// we skip over such spans while scanning. `[foo`](/uri)`` therefore keeps
/// its backtick span intact instead of forming a bogus link.
fn matching_bracket(chars: &[char], open: usize) -> Option<usize> {
  let n = chars.len();
  let mut depth = 0i32;
  let mut j = open;
  while j < n {
    let c = chars[j];
    if c == '\\' && j + 1 < n {
      j += 2;
      continue;
    }
    // A code span / autolink / raw-HTML region swallows any brackets inside.
    if let Some(end) = inline_span_end(chars, j) {
      j = end;
      continue;
    }
    if c == '[' {
      depth += 1;
    } else if c == ']' {
      depth -= 1;
      if depth == 0 {
        return Some(j);
      }
    }
    j += 1;
  }
  None
}

/// Mirror of the Flutter `parseInline`: returns clean text (no markers) and the
/// marks over it, with offsets in UTF-16 code units.
/// Memo of `label_contains_link` results, keyed by label text and SHARED across
/// the recursive parses of one top-level parse.
///
/// Without it, a label that DOES contain a link is parsed twice per nesting
/// level — once to answer the check, then again by the scan that falls through
/// the rejected outer brackets — giving T(n) = 2·T(n-1), i.e. exponential on
/// `[[[a](/u)](/u)](/u)`. The memo makes the second visit a hash hit.
type LinkCache = std::collections::HashMap<String, bool>;

/// Does this link-label content already contain a link (inline, reference, or
/// autolink)? CommonMark §6.3 forbids links nested inside link text, so when
/// this is true the surrounding brackets must stay literal. Images are fine
/// (`[![alt](img)](url)` is a valid linked image), so only `link` marks count.
///
/// Memoized: pure function + pure cache, so this cannot change any output —
/// only how often the answer is recomputed.
fn label_has_link_cached(label: &str, defs: &RefDefs, cache: &mut LinkCache) -> bool {
  if let Some(&hit) = cache.get(label) {
    return hit;
  }
  let answer = parse_inline_memo(label, defs, cache)
    .marks
    .iter()
    .any(|m| m.kind == "link");
  cache.insert(label.to_string(), answer);
  answer
}

fn parse_inline_with(src: &str, defs: &RefDefs) -> ParsedInline {
  let mut cache = LinkCache::new();
  parse_inline_memo(src, defs, &mut cache)
}

fn parse_inline_memo(src: &str, defs: &RefDefs, cache: &mut LinkCache) -> ParsedInline {
  // Line-ending canonicalization (§6.7/6.9/6.12) with span awareness: a run
  // of 2+ trailing spaces before '\n' OUTSIDE code spans / autolinks / raw
  // inline HTML is a hard break — canonicalized to the "\\\n" stored-text
  // convention (html_text renders it as <br/>); soft breaks just drop the
  // trailing spaces. INSIDE those spans every byte is content: `code  ⏎span`
  // keeps its spaces (the code-span handler maps '\n' to ' ') and raw HTML
  // keeps its literal line break. The block layer no longer pre-judges hard
  // breaks — it can't see code spans.
  let src_chars: Vec<char> = src.chars().collect();
  let mut chars: Vec<char> = Vec::with_capacity(src_chars.len());
  {
    let n = src_chars.len();
    let mut i = 0;
    while i < n {
      let c = src_chars[i];
      // Escaped char (incl. "\\`"): never a span opener; copy as-is. A
      // backslash directly before '\n' falls through — it IS a hard break.
      if c == '\\' && i + 1 < n && src_chars[i + 1] != '\n' {
        chars.push(c);
        chars.push(src_chars[i + 1]);
        i += 2;
        continue;
      }
      if c == '`' || c == '<' {
        if let Some(end) = inline_span_end(&src_chars, i) {
          chars.extend_from_slice(&src_chars[i..end]);
          i = end;
          continue;
        }
      }
      if c == '\n' {
        // Hard break = 2+ SPACES immediately before the newline; any other
        // trailing whitespace (tabs included) just drops with a soft break.
        let mut spaces = 0;
        while spaces < chars.len() && chars[chars.len() - 1 - spaces] == ' ' {
          spaces += 1;
        }
        let mut ws = spaces;
        while ws < chars.len() && matches!(chars[chars.len() - 1 - ws], ' ' | '\t') {
          ws += 1;
        }
        chars.truncate(chars.len() - ws);
        if spaces >= 2 {
          chars.push('\\');
        }
        chars.push('\n');
        i += 1;
        continue;
      }
      chars.push(c);
      i += 1;
    }
    // End-of-text trailing whitespace is outside spans by construction.
    while matches!(chars.last(), Some(' ') | Some('\t')) {
      chars.pop();
    }
  }
  let mut out = String::new();
  let mut out_len: usize = 0; // UTF-16 length of `out`
  let mut marks: Vec<InlineMark> = Vec::new();
  let mut delims: Vec<Delim> = Vec::new();
  let mut i = 0;

  // Find the next index of `needle` (as chars) starting at `from`.
  let find_from = |from: usize, needle: &[char]| -> Option<usize> {
    if needle.is_empty() {
      return None;
    }
    let mut j = from;
    while j + needle.len() <= chars.len() {
      if chars[j..j + needle.len()] == *needle {
        return Some(j);
      }
      j += 1;
    }
    None
  };

  // Emit a resolved link or image into out/marks (text parsed recursively).
  let push_link = |kind: &str,
                   label: &str,
                   href: String,
                   title: Option<String>,
                   out: &mut String,
                   out_len: &mut usize,
                   marks: &mut Vec<InlineMark>| {
    let start = *out_len;
    let parsed = parse_inline_with(label, defs);
    out.push_str(&parsed.text);
    *out_len += parsed.text.encode_utf16().count();
    for m in parsed.marks {
      marks.push(InlineMark {
        start: m.start + start,
        end: m.end + start,
        kind: m.kind,
        href: m.href,
        title: m.title,
      });
    }
    marks.push(InlineMark {
      start,
      end: *out_len,
      kind: kind.to_string(),
      href: Some(href),
      title,
    });
  };

  while i < chars.len() {
    // Inline math, LaTeX form: \( … \) (normalized to `$ … $` on write).
    // Checked before the escape arm, which would otherwise eat the `\(`.
    if chars[i] == '\\'
      && chars.get(i + 1) == Some(&'(')
      && let Some(close) = find_pair(&chars, i + 2, '\\', ')')
    {
      let inner: String = chars[i + 2..close].iter().collect();
      let inner = inner.trim().to_string();
      if !inner.is_empty() {
        let start = out_len;
        out.push_str(&inner);
        out_len += inner.encode_utf16().count();
        marks.push(InlineMark {
          start,
          end: out_len,
          kind: "math".into(),
          href: None,
          title: None,
        });
        i = close + 2;
        continue;
      }
    }
    // Inline math, dollar form: `$ … $` — the opener may not be followed
    // by whitespace, the closer may not be preceded by whitespace or
    // followed by a digit (the Pandoc rules; keeps `$5 and $10` literal).
    if chars[i] == '$' && chars.get(i + 1) != Some(&'$') {
      if let Some(close) = find_math_closer(&chars, i + 1) {
        let inner: String = chars[i + 1..close].iter().collect();
        let start = out_len;
        out.push_str(&inner);
        out_len += inner.encode_utf16().count();
        marks.push(InlineMark {
          start,
          end: out_len,
          kind: "math".into(),
          href: None,
          title: None,
        });
        i = close + 1;
        continue;
      }
    }
    // Backslash escape: `\*` is a literal `*` (any ASCII punctuation).
    if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() {
      out.push(chars[i + 1]);
      out_len += chars[i + 1].len_utf16();
      i += 2;
      continue;
    }
    // Entity / numeric character reference: decodes to plain TEXT (the
    // result can't open emphasis or any structure).
    if chars[i] == '&'
      && let Some((decoded, used)) = parse_entity(&chars, i)
    {
      out.push_str(&decoded);
      out_len += decoded.encode_utf16().count();
      i += used;
      continue;
    }
    // Autolink: <https://…> or <user@host> — the URL is both text and target.
    if chars[i] == '<' {
      if let Some(close) = find_from(i + 1, &['>']) {
        let inner: String = chars[i + 1..close].iter().collect();
        let href = autolink_target(&inner);
        if let Some(href) = href {
          let start = out_len;
          out.push_str(&inner);
          out_len += inner.encode_utf16().count();
          marks.push(InlineMark {
            start,
            end: out_len,
            kind: "link".into(),
            href: Some(href),
            title: None,
          });
          i = close + 1;
          continue;
        }
      }
      // Raw inline HTML: a valid tag/comment/PI/declaration/CDATA shape
      // passes through verbatim under an `html` mark.
      if let Some(end) = inline_html_end(&chars, i) {
        let raw: String = chars[i..end].iter().collect();
        let start = out_len;
        out.push_str(&raw);
        out_len += raw.encode_utf16().count();
        marks.push(InlineMark {
          start,
          end: out_len,
          kind: "html".into(),
          href: None,
          title: None,
        });
        i = end;
        continue;
      }
    }
    // Image: ![alt](dest "title") | ![alt][label] | ![alt][] | ![alt] —
    // same bridge as links; the alt keeps its inner marks (HTML flattens
    // them to plain text, markdown re-renders them).
    if chars[i] == '!' && i + 1 < chars.len() && chars[i + 1] == '[' {
      if let Some(close) = matching_bracket(&chars, i + 1) {
        let label: String = chars[i + 2..close].iter().collect();
        if !label.is_empty() {
          if close + 1 < chars.len() && chars[close + 1] == '(' {
            if let Some((href, title, next)) = parse_link_suffix(&chars, close + 2) {
              push_link("image", &label, href, title, &mut out, &mut out_len, &mut marks);
              i = next;
              continue;
            }
          }
          if !defs.is_empty() {
            let (ref_label, next) = if close + 1 < chars.len() && chars[close + 1] == '[' {
              match matching_bracket(&chars, close + 1) {
                Some(end2) => {
                  let second: String = chars[close + 2..end2].iter().collect();
                  if second.is_empty() {
                    (label.clone(), end2 + 1) // collapsed ![alt][]
                  } else {
                    (second, end2 + 1) // full ![alt][label]
                  }
                }
                None => (label.clone(), close + 1),
              }
            } else {
              (label.clone(), close + 1) // shortcut ![alt]
            };
            if let Some((dest, title)) = defs.get(&normalize_label(&ref_label)) {
              push_link("image", &label, dest.clone(), title.clone(), &mut out, &mut out_len, &mut marks);
              i = next;
              continue;
            }
          }
        }
      }
    }
    // Footnote reference: `[^label]` (GFM). The brackets and caret are
    // stripped to bare TEXT (the label) under a `footnote` mark carrying the
    // label as `href` — the same delimiter-strip + mark shape inline math
    // uses. Labels hold no whitespace or brackets; checked before the link
    // arm so `[^x]` never parses as a shortcut link. Whether a matching
    // definition exists is decided at render time, not here (an undefined
    // reference round-trips to `[^x]` regardless and degrades to literal HTML).
    if chars[i] == '['
      && chars.get(i + 1) == Some(&'^')
      && let Some(close) = matching_bracket(&chars, i)
      && close > i + 2
    {
      let label: String = chars[i + 2..close].iter().collect();
      if !label.chars().any(|c| c.is_whitespace() || matches!(c, '[' | ']' | '^')) {
        let start = out_len;
        out.push_str(&label);
        out_len += label.encode_utf16().count();
        marks.push(InlineMark {
          start,
          end: out_len,
          kind: "footnote".into(),
          href: Some(label),
          title: None,
        });
        i = close + 1;
        continue;
      }
    }
    // Link: [text](dest "title") | [text][label] | [text][] | [shortcut]
    if chars[i] == '[' {
      if let Some(close) = matching_bracket(&chars, i) {
        let label: String = chars[i + 1..close].iter().collect();
        // A link's text may not itself contain a link (CommonMark §6.3): if
        // the label holds a top-level `[…](…)`/`[…][…]`/autolink, the OUTER
        // brackets stay literal and the inner link wins. `[foo [bar](/uri)]`
        // is therefore `[foo <a>bar</a>]`, not a nested anchor.
        // The nested-link check is (a) computed LAZILY — it only ever gates the
        // two link forms below, so computing it up-front made EVERY bracket pay
        // a recursive re-parse — and (b) memoized through a cache SHARED across
        // the whole parse (see `LinkCache`), which is what removes the second
        // re-parse of a label that DOES contain a link.
        //
        // Both together turn parsing from exponential to flat on nested input.
        // Measured (release, `import_markdown`), before -> after:
        //   `[[[[a]]]]`            depth 24:   ~6s   -> 0ms
        //   `[[[a](/u)](/u)](/u)`  depth 20:  834ms  -> 0ms   (depth 32: 0ms)
        // This matters because `import_markdown` runs on user-supplied markdown
        // (import + MCP), so ~170 bytes used to pin a core for minutes.
        //
        // Output is unchanged: laziness is `!A && X` -> `X && !A` with both pure
        // (differential-fuzzed, ~2.8M inputs, zero divergence), and the memo is a
        // pure cache over a pure function. Pinned by tests/nested_bracket_perf.rs.
        // Inline form — empty text is allowed (`[](/url)` → empty anchor).
        if close + 1 < chars.len() && chars[close + 1] == '(' {
          if let Some((href, title, next)) = parse_link_suffix(&chars, close + 2) {
            if !label_has_link_cached(&label, defs, cache) {
              push_link("link", &label, href, title, &mut out, &mut out_len, &mut marks);
              i = next;
              continue;
            }
          }
        }
        if !label.is_empty() {
          // Reference forms (full / collapsed / shortcut).
          if !defs.is_empty() {
            let (ref_label, next) = if close + 1 < chars.len() && chars[close + 1] == '[' {
              match matching_bracket(&chars, close + 1) {
                Some(end2) => {
                  let second: String = chars[close + 2..end2].iter().collect();
                  if second.is_empty() {
                    (label.clone(), end2 + 1) // collapsed [text][]
                  } else {
                    (second, end2 + 1) // full [text][label]
                  }
                }
                None => (label.clone(), close + 1),
              }
            } else {
              (label.clone(), close + 1) // shortcut [text]
            };
            if let Some((dest, title)) = defs.get(&normalize_label(&ref_label)) {
              if !label_has_link_cached(&label, defs, cache) {
                push_link("link", &label, dest.clone(), title.clone(), &mut out, &mut out_len, &mut marks);
                i = next;
                continue;
              }
            }
          }
        }
      }
    }
    // Emphasis delimiters (* and _): record the run with its flanking
    // properties; pairing happens after the scan (spec algorithm).
    if chars[i] == '*' || chars[i] == '_' {
      let c = chars[i];
      let mut j = i;
      while j < chars.len() && chars[j] == c {
        j += 1;
      }
      let count = j - i;
      let prev = if i == 0 { None } else { Some(chars[i - 1]) };
      let next = chars.get(j).copied();
      let (can_open, can_close) = flanking(c, prev, next);
      let start = out_len;
      for _ in 0..count {
        out.push(c);
      }
      out_len += count; // BMP chars: 1 UTF-16 unit each
      delims.push(Delim {
        c,
        start,
        cur_start: start,
        count,
        orig: count,
        can_open,
        can_close,
      });
      i = j;
      continue;
    }
    // GFM extended autolinks: bare www./http(s):// URLs and emails at a
    // word boundary (start, whitespace, or `*`/`_`/`~`/`(`).
    if (i == 0
      || chars[i - 1].is_whitespace()
      || matches!(chars[i - 1], '*' | '_' | '~' | '('))
      && let Some((len, href)) = extended_autolink(&chars, i)
    {
      let text: String = chars[i..i + len].iter().collect();
      let start = out_len;
      out.push_str(&text);
      out_len += text.encode_utf16().count();
      marks.push(InlineMark {
        start,
        end: out_len,
        kind: "link".into(),
        href: Some(href),
        title: None,
      });
      i += len;
      continue;
    }
    // Strike: ~~...~~
    // GFM strikethrough: one or two tildes close on a run of the same
    // length (three or more stay literal).
    if chars[i] == '~' {
      let mut n = 0;
      while i + n < chars.len() && chars[i + n] == '~' {
        n += 1;
      }
      if n <= 2 {
        let mut j = i + n;
        let mut close = None;
        while j < chars.len() {
          if chars[j] == '~' && (j == 0 || chars[j - 1] != '\\') {
            let mut m = 0;
            while j + m < chars.len() && chars[j + m] == '~' {
              m += 1;
            }
            if m == n && j > i + n {
              close = Some(j);
              break;
            }
            j += m;
          } else {
            j += 1;
          }
        }
        if let Some(end) = close {
          let inner: String = chars[i + n..end].iter().collect();
          push_inner(&inner, "strike", None, defs, &mut out, &mut out_len, &mut marks);
          i = end + n;
          continue;
        }
      }
    }
    // Inline code: an N-backtick run closes only on a run of exactly N;
    // line endings become spaces; one leading+trailing space strips when
    // both exist and the content isn't all spaces. No escapes inside.
    if chars[i] == '`' {
      let mut n = 0;
      while i + n < chars.len() && chars[i + n] == '`' {
        n += 1;
      }
      let mut j = i + n;
      let mut close = None;
      while j < chars.len() {
        if chars[j] == '`' {
          let mut m = 0;
          while j + m < chars.len() && chars[j + m] == '`' {
            m += 1;
          }
          if m == n {
            close = Some(j);
            break;
          }
          j += m;
        } else {
          j += 1;
        }
      }
      if let Some(end) = close {
        let mut inner: String = chars[i + n..end]
          .iter()
          .map(|&c| if c == '\n' { ' ' } else { c })
          .collect();
        if inner.len() >= 2
          && inner.starts_with(' ')
          && inner.ends_with(' ')
          && !inner.bytes().all(|b| b == b' ')
        {
          inner = inner[1..inner.len() - 1].to_string();
        }
        let start = out_len;
        out.push_str(&inner);
        out_len += inner.encode_utf16().count();
        marks.push(InlineMark {
          start,
          end: out_len,
          kind: "code".into(),
          href: None,
          title: None,
        });
        i = end + n;
        continue;
      }
      // No closer: the run is literal text.
      for _ in 0..n {
        out.push('`');
      }
      out_len += n;
      i += n;
      continue;
    }
    out.push(chars[i]);
    out_len += chars[i].len_utf16();
    i += 1;
  }

  process_emphasis(&mut out, &mut marks, &mut delims);

  ParsedInline { text: out, marks }
}

/// One run of `*`/`_` delimiters, tracked in output (UTF-16) coordinates.
struct Delim {
  c: char,
  start: usize,     // original run start in `out`
  cur_start: usize, // advances as the closer side is consumed
  count: usize,     // remaining delimiter characters
  orig: usize,      // original run length (rule-of-3)
  can_open: bool,
  can_close: bool,
}

fn is_md_punct(c: char) -> bool {
  // CommonMark 0.31: "Unicode punctuation" = general categories P* AND S*
  // (symbols — currency, math, arrows — count too). std has no category
  // tables, so cover the blocks that occur in real text; extend on demand.
  c.is_ascii_punctuation()
    || matches!(c,
      '\u{00A1}'..='\u{00A9}'   // ¡¢£¤¥¦§¨© Latin-1 punct/symbols
        | '\u{00AB}'..='\u{00B1}' // «¬®¯°± (skip ª, a letter)
        | '\u{00B4}' | '\u{00B6}'..='\u{00B8}' | '\u{00BB}' | '\u{00BF}'
        | '\u{00D7}' | '\u{00F7}' // × ÷
        | '\u{2000}'..='\u{206F}' // general punctuation
        | '\u{20A0}'..='\u{20CF}' // currency symbols (€ …)
        | '\u{2100}'..='\u{2BFF}' // letterlike, arrows, math, misc symbols
        | '\u{3000}'..='\u{303F}' // CJK punctuation
        | '\u{FE30}'..='\u{FE4F}' // CJK compat forms
        | '\u{FF01}'..='\u{FF0F}' // fullwidth punct
        | '\u{FF1A}'..='\u{FF20}' | '\u{FF3B}'..='\u{FF40}' | '\u{FF5B}'..='\u{FF65}')
}

/// A CJK character (BMP): ideographs, kana, hangul, bopomofo, AND CJK
/// punctuation (`。、！？「」…`). Used to make emphasis CJK-friendly — see
/// [`flanking`]. Kept byte-identical to the Dart `_isCjk` (marks.dart).
fn is_cjk(c: char) -> bool {
  matches!(c,
    '\u{1100}'..='\u{11FF}'   // Hangul Jamo
      | '\u{2E80}'..='\u{2EFF}' // CJK Radicals Supplement
      | '\u{3000}'..='\u{303F}' // CJK Symbols and Punctuation
      | '\u{3040}'..='\u{30FF}' // Hiragana + Katakana
      | '\u{3100}'..='\u{312F}' // Bopomofo
      | '\u{3130}'..='\u{318F}' // Hangul Compatibility Jamo
      | '\u{31C0}'..='\u{31EF}' // CJK Strokes
      | '\u{3200}'..='\u{33FF}' // Enclosed CJK + CJK Compatibility
      | '\u{3400}'..='\u{4DBF}' // CJK Ext A
      | '\u{4E00}'..='\u{9FFF}' // CJK Unified Ideographs
      | '\u{A000}'..='\u{A4CF}' // Yi
      | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
      | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
      | '\u{FE30}'..='\u{FE4F}' // CJK Compatibility Forms
      | '\u{FF00}'..='\u{FFEF}') // Halfwidth and Fullwidth Forms
}

/// Left/right flanking → (can_open, can_close), with the CJK-friendly amendment
/// (markdown-cjk-friendly). Plain CommonMark treats CJK punctuation (`。`) as
/// "punctuation", so `**加粗。**后文` can't close — a `。` before the `**` and a
/// letter after fail the flanking test. A Chinese sentence ends in `。`/`,` far
/// more often than a space, so this bit constantly (and broke round-trip: the
/// exporter emits `**…。**x` which then wouldn't re-parse). Fix: split
/// "punctuation" into NON-CJK punctuation (strict rule kept) and CJK
/// punctuation/characters, which instead RELAX flanking the way whitespace does
/// in Latin (a CJK char is a word boundary). ASCII inputs are unaffected
/// (`is_cjk` is false), so the CommonMark scoreboard stays 641/641. Mirrored in
/// the Dart `_flanking` (marks.dart).
fn flanking(c: char, prev: Option<char>, next: Option<char>) -> (bool, bool) {
  let prev_ws = prev.is_none_or(char::is_whitespace);
  let next_ws = next.is_none_or(char::is_whitespace);
  let prev_cjk = prev.is_some_and(is_cjk);
  let next_cjk = next.is_some_and(is_cjk);
  let prev_ncp = prev.is_some_and(is_md_punct) && !prev_cjk; // non-CJK punct
  let next_ncp = next.is_some_and(is_md_punct) && !next_cjk;

  let left = !next_ws && (!next_ncp || prev_ws || prev_ncp || prev_cjk);
  let right = !prev_ws && (!prev_ncp || next_ws || next_ncp || next_cjk);

  if c == '_' {
    (
      left && (!right || prev_ncp || prev_cjk),
      right && (!left || next_ncp || next_cjk),
    )
  } else {
    (left, right)
  }
}

/// The spec's process-emphasis: pair closers with the nearest valid opener
/// (same char, rule-of-3), strong before em, deleting used delimiter
/// characters and emitting bold/italic marks over the content between.
fn process_emphasis(out: &mut String, marks: &mut Vec<InlineMark>, delims: &mut [Delim]) {
  if delims.is_empty() {
    return;
  }
  let mut deletions: Vec<(usize, usize)> = Vec::new(); // (start, len) in out coords

  let mut closer_i = 0;
  while closer_i < delims.len() {
    if delims[closer_i].count == 0 || !delims[closer_i].can_close {
      closer_i += 1;
      continue;
    }
    // Nearest opener walking back.
    let mut opener_i = None;
    let mut k = closer_i;
    while k > 0 {
      k -= 1;
      let o = &delims[k];
      if o.count == 0 || !o.can_open || o.c != delims[closer_i].c {
        continue;
      }
      // Rule of 3.
      let cl = &delims[closer_i];
      if (o.can_close || cl.can_open)
        && (o.orig + cl.orig) % 3 == 0
        && !(o.orig % 3 == 0 && cl.orig % 3 == 0)
      {
        continue;
      }
      opener_i = Some(k);
      break;
    }
    let Some(oi) = opener_i else {
      // No opener: if it can't also open, it is pure text now.
      closer_i += 1;
      continue;
    };

    let use_n = if delims[oi].count >= 2 && delims[closer_i].count >= 2 { 2 } else { 1 };
    // Opener consumes from its right edge; closer from its left edge.
    delims[oi].count -= use_n;
    let o_del = delims[oi].start + delims[oi].count;
    deletions.push((o_del, use_n));
    let c_del = delims[closer_i].cur_start;
    deletions.push((c_del, use_n));
    delims[closer_i].cur_start += use_n;
    delims[closer_i].count -= use_n;

    marks.push(InlineMark {
      start: o_del + use_n,
      end: c_del,
      kind: if use_n == 2 { "bold" } else { "italic" }.to_string(),
      href: None,
      title: None,
    });

    // Delimiters between opener and closer can never pair across.
    for d in delims[oi + 1..closer_i].iter_mut() {
      d.count = 0;
    }
    if delims[closer_i].count == 0 {
      closer_i += 1;
    }
  }

  if deletions.is_empty() {
    return;
  }
  deletions.sort_unstable();

  // Rebuild the text without the consumed delimiter characters and remap
  // every mark offset past the deletions.
  let units: Vec<u16> = out.encode_utf16().collect();
  let mut keep: Vec<u16> = Vec::with_capacity(units.len());
  let mut removed_before: Vec<usize> = Vec::with_capacity(units.len() + 1);
  let mut di = 0;
  let mut removed = 0usize;
  let mut skip_until = 0usize;
  for (idx, &u) in units.iter().enumerate() {
    removed_before.push(removed);
    if idx >= skip_until && di < deletions.len() && deletions[di].0 == idx {
      skip_until = idx + deletions[di].1;
      di += 1;
    }
    if idx < skip_until {
      removed += 1;
    } else {
      keep.push(u);
    }
  }
  removed_before.push(removed);

  *out = String::from_utf16_lossy(&keep);
  for m in marks.iter_mut() {
    m.start -= removed_before[m.start.min(removed_before.len() - 1)];
    m.end -= removed_before[m.end.min(removed_before.len() - 1)];
  }
  marks.retain(|m| m.end > m.start);
}

/// CommonMark autolink target for `<inner>`: an absolute URI (scheme:) maps
/// to itself, a bare email maps to `mailto:`; anything else is not an
/// autolink.
fn autolink_target(inner: &str) -> Option<String> {
  if inner.is_empty() || inner.chars().any(|c| c.is_whitespace() || c == '<') {
    return None;
  }
  let bytes = inner.as_bytes();
  // scheme: ALPHA (ALPHA/DIGIT/+/-/.)* ':'
  if let Some(colon) = inner.find(':') {
    let scheme = &inner[..colon];
    if scheme.len() >= 2
      && scheme.len() <= 32
      && scheme.as_bytes()[0].is_ascii_alphabetic()
      && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
      && colon + 1 < inner.len()
    {
      return Some(inner.to_string());
    }
  }
  // email: x@y.z (loose; backslash escapes never count)
  if inner.contains('\\') {
    return None;
  }
  if let Some(at) = inner.find('@') {
    let (local, host) = inner.split_at(at);
    let host = &host[1..];
    if !local.is_empty() && host.contains('.') && !host.ends_with('.') && bytes.iter().filter(|&&b| b == b'@').count() == 1 {
      return Some(format!("mailto:{inner}"));
    }
  }
  None
}

/// Parse [inner] recursively and append it to [out], adding a [kind] mark over
/// the appended range.
fn push_inner(
  inner: &str,
  kind: &str,
  href: Option<String>,
  defs: &RefDefs,
  out: &mut String,
  out_len: &mut usize,
  marks: &mut Vec<InlineMark>,
) {
  let start = *out_len;
  let parsed = parse_inline_with(inner, defs);
  out.push_str(&parsed.text);
  *out_len += parsed.text.encode_utf16().count();
  for m in parsed.marks {
    marks.push(InlineMark {
      start: m.start + start,
      end: m.end + start,
      kind: m.kind,
      href: m.href,
      title: m.title,
    });
  }
  marks.push(InlineMark {
    start,
    end: *out_len,
    kind: kind.to_string(),
    href,
    title: None,
  });
}

/// An inline mark over a `[start, end)` range of a block's plain text.
struct InlineMark {
  start: usize,
  end: usize,
  kind: String,
  href: Option<String>,
  title: Option<String>,
}

fn marks_from_block(block: &Block) -> Vec<InlineMark> {
  let Some(raw) = block.data.get("marks").and_then(Value::as_array) else {
    return Vec::new();
  };
  // Mark offsets index into UTF-16 code units (the Flutter client's string
  // model); convert against this text's UTF-16 view when slicing.
  let units: Vec<u16> = block.text.encode_utf16().collect();
  let len = units.len();
  let mut marks = Vec::new();
  for m in raw {
    let Some(obj) = m.as_object() else { continue };
    let start = obj.get("start").and_then(Value::as_u64).map(|v| v as usize);
    let end = obj.get("end").and_then(Value::as_u64).map(|v| v as usize);
    let kind = obj.get("type").and_then(Value::as_str);
    if let (Some(start), Some(end), Some(kind)) = (start, end, kind) {
      let start = start.min(len);
      let end = end.min(len);
      // Empty-text links (`[](/url)`) are a legitimate zero-width mark; every
      // other kind needs real content to wrap.
      if end > start || kind == "link" {
        marks.push(InlineMark {
          start,
          end,
          kind: kind.to_string(),
          href: obj.get("href").and_then(Value::as_str).map(str::to_string),
          title: obj.get("title").and_then(Value::as_str).map(str::to_string),
        });
      }
    }
  }
  marks
}

/// Render a block's text with its inline marks back to Markdown. Code blocks and
/// tables carry no inline marks and are emitted verbatim by their own arms.
/// Escape characters our (and CommonMark's) inline grammar would otherwise
/// interpret, so literal text survives a round-trip.
fn escape_inline(text: &str) -> String {
  let mut out = String::with_capacity(text.len());
  let chars: Vec<char> = text.chars().collect();
  for (i, &c) in chars.iter().enumerate() {
    // A backslash right before a newline IS a hard break — keep it raw.
    if c == '\\' && chars.get(i + 1) == Some(&'\n') {
      out.push(c);
      continue;
    }
    if matches!(c, '\\' | '*' | '_' | '`' | '~' | '[' | ']' | '<' | '$') {
      out.push('\\');
    }
    out.push(c);
  }
  out
}

/// A paragraph whose text LOOKS like a block marker (`- x`, `> x`, `# x`,
/// `1. x`, `---`) must escape its leader or it changes kind on re-import.
fn escape_block_leader(line: String) -> String {
  let t = line.as_str();
  let compact: String = t.chars().filter(|c| *c != ' ').collect();
  let divider_like = compact.len() >= 3 && compact.chars().all(|c| c == '-');
  // A paragraph that would read as a setext underline gets escaped too.
  let setext_like = !t.is_empty() && (t.bytes().all(|b| b == b'=') || t.bytes().all(|b| b == b'-'));
  let divider_like = divider_like || setext_like;
  let numbered = t
    .find(". ")
    .is_some_and(|dot| dot > 0 && t[..dot].bytes().all(|b| b.is_ascii_digit()));
  if t.starts_with("- ")
    || t.starts_with("+ ")
    || t.starts_with("> ")
    || divider_like
    || (t.starts_with('#') && t[1..].trim_start_matches('#').starts_with(' '))
  {
    return format!("\\{line}");
  }
  if numbered {
    let dot = t.find(". ").unwrap();
    return format!("{}\\. {}", &t[..dot], &t[dot + 2..]);
  }
  line
}

fn render_inline(block: &Block) -> String {
  if matches!(block.kind.as_str(), "code_block" | "code" | "table") {
    return block.text.clone();
  }
  let marks = marks_from_block(block);
  if marks.is_empty() {
    return escape_inline(&block.text);
  }
  let units: Vec<u16> = block.text.encode_utf16().collect();
  let refs: Vec<&InlineMark> = marks.iter().collect();
  render_span(&units, 0, units.len(), &refs)
}

/// Marks whose rendering is TERMINAL: the branch writes the span literally and
/// cannot nest anything inside it, so it must never win an exact-range tie — it
/// would swallow the mark it coincides with. `[`a`](/x)` used to export as
/// `` `a` ``, dropping the link outright (and with it the URL).
fn renders_terminal(kind: &str) -> bool {
  matches!(kind, "code" | "math" | "html" | "footnote")
}

/// Render `[lo, hi)` of the text wrapping marks as properly NESTED markdown:
/// the outermost mark opens once over its whole range with inner marks
/// recursing inside (`**bold *italic* tail**`) — per-segment wrapping breaks
/// round-trips. Marks that only partially overlap an enclosing one are
/// clipped (markdown cannot express crossing ranges).
fn render_span(units: &[u16], lo: usize, hi: usize, marks: &[&InlineMark]) -> String {
  let mut out = String::new();
  let mut pos = lo;
  while pos < hi {
    // The next mark by clipped start; ties prefer the widest (outermost), then
    // the nestable one — a terminal kind sorts last so it renders INSIDE.
    let next = marks
      .iter()
      .enumerate()
      .filter_map(|(i, m)| {
        let s = m.start.max(pos);
        let e = m.end.min(hi);
        (e > s).then_some((s, e, i))
      })
      .min_by_key(|&(s, e, i)| (s, usize::MAX - e, renders_terminal(&marks[i].kind)));
    let Some((s, e, picked)) = next else {
      out.push_str(&escape_inline(&String::from_utf16_lossy(&units[pos..hi])));
      break;
    };
    let mut lead = escape_inline(&String::from_utf16_lossy(&units[pos..s]));
    if marks[picked].kind == "link" && lead.ends_with('!') {
      lead.pop();
      lead.push_str("\\!");
    }
    out.push_str(&lead);
    let inner: Vec<&InlineMark> = marks
      .iter()
      .enumerate()
      .filter(|&(i, m)| i != picked && m.end.min(e) > m.start.max(s))
      .map(|(_, m)| *m)
      .collect();
    let m = marks[picked];
    if m.kind == "html" {
      // Raw inline HTML writes back verbatim.
      out.push_str(&String::from_utf16_lossy(&units[s..e]));
      pos = e;
      continue;
    }
    if m.kind == "math" {
      // LaTeX source is literal — canonical dollar form.
      out.push_str(&format!("${}$", String::from_utf16_lossy(&units[s..e])));
      pos = e;
      continue;
    }
    if m.kind == "footnote" {
      // The span text IS the label; the `[^…]` reference syntax is restored
      // from the mark's href (the label survives even if the span was edited).
      let label = m.href.clone().unwrap_or_else(|| String::from_utf16_lossy(&units[s..e]));
      out.push_str(&format!("[^{label}]"));
      pos = e;
      continue;
    }
    if m.kind == "code" {
      // Code spans are literal — no escaping, no nested marks. The fence is
      // one backtick longer than any run inside; a space pads content that
      // starts/ends with a backtick or with stripped-on-read spaces.
      let raw = String::from_utf16_lossy(&units[s..e]);
      let mut longest = 0usize;
      let mut cur = 0usize;
      for c in raw.chars() {
        if c == '`' {
          cur += 1;
          longest = longest.max(cur);
        } else {
          cur = 0;
        }
      }
      let fence = "`".repeat(longest + 1);
      let pad = raw.starts_with('`')
        || raw.ends_with('`')
        || (raw.starts_with(' ') && raw.ends_with(' ') && !raw.trim().is_empty());
      if pad {
        out.push_str(&format!("{fence} {raw} {fence}"));
      } else {
        out.push_str(&format!("{fence}{raw}{fence}"));
      }
      pos = e;
      continue;
    }
    let body = render_span(units, s, e, &inner);
    out.push_str(&match m.kind.as_str() {
      "bold" => format!("**{body}**"),
      "italic" => format!("*{body}*"),
      "strike" => format!("~~{body}~~"),
      "link" => {
        let href = m.href.as_deref().unwrap_or("");
        let plain = String::from_utf16_lossy(&units[s..e]);
        // A title-less link whose text IS its target → autolink form; a
        // bare `www.` link writes back bare (GFM re-links it on read). Both
        // shorthands write `plain`, so they DISCARD inner marks — only take
        // them when there are none to lose (`[`x`](x)` must stay bracketed).
        if m.title.is_none()
          && inner.is_empty()
          && plain.starts_with("www.")
          && href == format!("http://{plain}")
        {
          plain.to_string()
        } else if m.title.is_none()
          && inner.is_empty()
          && (href == plain || href == format!("mailto:{plain}"))
        {
          format!("<{plain}>")
        } else {
          let dest = if href.contains(char::is_whitespace) {
            format!("<{href}>")
          } else {
            href.to_string()
          };
          match &m.title {
            Some(t) => format!("[{body}]({dest} \"{}\")", t.replace('"', "\\\"")),
            None => format!("[{body}]({dest})"),
          }
        }
      }
      "image" => {
        let href = m.href.as_deref().unwrap_or("");
        let dest = if href.contains(char::is_whitespace) {
          format!("<{href}>")
        } else {
          href.to_string()
        };
        match &m.title {
          Some(t) => format!("![{body}]({dest} \"{}\")", t.replace('"', "\\\"")),
          None => format!("![{body}]({dest})"),
        }
      }
      _ => body,
    });
    pos = e;
  }
  out
}

fn append_table_markdown(block: &Block, lines: &mut Vec<String>) {
  let Some(rows) = block.data.get("rows").and_then(Value::as_array) else {
    return;
  };
  let grid: Vec<Vec<String>> = rows
    .iter()
    .filter_map(|row| {
      row.as_array().map(|cells| {
        cells
          .iter()
          .map(|cell| {
            cell
              .as_str()
              .unwrap_or("")
              .replace('|', "\\|")
              .replace('\n', " ")
              .trim()
              .to_string()
          })
          .collect()
      })
    })
    .collect();
  if grid.is_empty() {
    return;
  }
  let cols = grid.iter().map(Vec::len).max().unwrap_or(0).max(1);
  let row_line = |cells: &[String]| -> String {
    let mut padded = cells.to_vec();
    while padded.len() < cols {
      padded.push(String::new());
    }
    format!("| {} |", padded.join(" | "))
  };
  lines.push(row_line(&grid[0]));
  let aligns = block.data.get("aligns").and_then(Value::as_array);
  let sep: Vec<&str> = (0..cols)
    .map(|c| {
      match aligns
        .and_then(|a| a.get(c))
        .and_then(Value::as_str)
        .unwrap_or("")
      {
        "center" => ":---:",
        "right" => "---:",
        "left" => ":---",
        _ => "---",
      }
    })
    .collect();
  lines.push(format!("| {} |", sep.join(" | ")));
  for row in grid.iter().skip(1) {
    lines.push(row_line(row));
  }
  lines.push(String::new());
}

fn heading_level(block: &Block, depth: usize) -> usize {
  block
    .data
    .get("level")
    .and_then(Value::as_u64)
    .map(|level| level as usize)
    .unwrap_or(depth + 1)
    .clamp(1, 6)
}

fn block_data_str<'a>(block: &'a Block, key: &str) -> Option<&'a str> {
  block.data.get(key).and_then(Value::as_str)
}

fn block_data_bool(block: &Block, key: &str) -> bool {
  block
    .data
    .get(key)
    .and_then(Value::as_bool)
    .unwrap_or(false)
}



pub fn block_index(snapshot: &DocumentSnapshotPayload, block_id: &str) -> Option<usize> {
  snapshot
    .blocks
    .iter()
    .position(|block| block.id == block_id)
}

pub fn is_descendant(snapshot: &DocumentSnapshotPayload, block_id: &str, parent_id: &str) -> bool {
  let Some(index) = block_index(snapshot, parent_id) else {
    return false;
  };

  for child_id in &snapshot.blocks[index].children {
    if child_id == block_id || is_descendant(snapshot, block_id, child_id) {
      return true;
    }
  }

  false
}

#[cfg(test)]
mod security_tests {
  use super::{export_html, import_markdown, strip_unsafe_attrs, tagfilter};

  // Pins what the GFM tagfilter does — and, crucially, what it does NOT do — for
  // raw HTML rendered onto a Mica same-origin surface (the public share page).
  // The 9-tag blacklist neutralizes `<script>`/`<iframe>` etc., but it does
  // nothing about `on*` event-handler attributes on allowed tags. So the raw
  // string `<img onerror=...>` / `<svg onload=...>` passes through VERBATIM.
  // That is exactly why the share-page HTTP response carries a strict CSP
  // (`default-src 'none'`, no `script-src`) — the tagfilter alone is not a
  // sufficient XSS defense. If tagfilter is ever hardened to strip event
  // handlers, update this test AND revisit whether the CSP is still needed.
  #[test]
  fn tagfilter_blacklist_neutralizes_script_but_not_event_handlers() {
    // Blacklisted element tags get their opening `<` escaped -> inert.
    assert!(tagfilter("<script>alert(1)</script>").starts_with("&lt;script"));
    assert!(tagfilter("<iframe src=x>").starts_with("&lt;iframe"));

    // Event handlers on non-blacklisted tags survive UNCHANGED. These are the
    // token-theft vectors the CSP has to catch at the response layer.
    let img = tagfilter("<img src=x onerror=\"steal()\">");
    assert!(img.contains("onerror"), "tagfilter unexpectedly stripped onerror: {img}");
    assert!(img.starts_with("<img"), "img tag should not be escaped: {img}");

    let svg = tagfilter("<svg onload=\"steal()\">");
    assert!(svg.contains("onload"), "tagfilter unexpectedly stripped onload: {svg}");
    assert!(svg.starts_with("<svg"), "svg tag should not be escaped: {svg}");
  }

  // The defense-in-depth layer that DOES strip the event handlers / js: URLs the
  // tagfilter leaves behind — belt-and-suspenders for no-CSP consumers.
  #[test]
  fn strip_unsafe_attrs_removes_event_handlers() {
    // Double-quoted, single-quoted, unquoted, and mixed-case + no-quote forms.
    let a = strip_unsafe_attrs("<img src=x onerror=\"steal()\">");
    assert!(!a.contains("onerror"), "onerror survived: {a}");
    assert!(a.contains("src=x"), "safe src dropped: {a}");
    assert!(a.starts_with("<img"), "tag mangled: {a}");

    let b = strip_unsafe_attrs("<svg onload='x'>");
    assert!(!b.contains("onload"), "onload survived: {b}");

    // Mixed case + unquoted value.
    let c = strip_unsafe_attrs("<body OnLoad=x>");
    assert!(!c.to_ascii_lowercase().contains("onload"), "OnLoad survived: {c}");
    assert_eq!(c, "<body>");

    // Unquoted handler with call syntax.
    let d = strip_unsafe_attrs("<img src=x onerror=steal()>");
    assert!(!d.contains("onerror"), "unquoted onerror survived: {d}");
  }

  #[test]
  fn strip_unsafe_attrs_removes_js_and_vbscript_urls() {
    let a = strip_unsafe_attrs("<a href=\"javascript:alert(1)\">x</a>");
    assert!(!a.contains("javascript"), "javascript: href survived: {a}");

    // Leading whitespace + mixed case.
    let b = strip_unsafe_attrs("<a href=\"  JAVAScript:x\">x</a>");
    assert!(!b.to_ascii_lowercase().contains("javascript"), "js href survived: {b}");

    // Embedded tab in the scheme (browser-normalized bypass).
    let c = strip_unsafe_attrs("<a href=\"java\tscript:x\">x</a>");
    assert!(!c.to_ascii_lowercase().contains("script:"), "tab-split js href survived: {c}");

    // vbscript: on a URL attribute other than href.
    let d = strip_unsafe_attrs("<img src=\"vbscript:msgbox(1)\">");
    assert!(!d.to_ascii_lowercase().contains("vbscript"), "vbscript src survived: {d}");
  }

  #[test]
  fn strip_unsafe_attrs_preserves_safe_content() {
    // A real image with a benign URL and an alt that contains "on" as a word.
    let img = strip_unsafe_attrs("<img src=\"x.png\" alt=\"turn on the light\">");
    assert_eq!(img, "<img src=\"x.png\" alt=\"turn on the light\">");

    // A class literally named "on" — the substring must not trip the on* rule.
    assert_eq!(strip_unsafe_attrs("<div class=\"on\">"), "<div class=\"on\">");

    // Valueless (boolean) attribute survives.
    assert_eq!(strip_unsafe_attrs("<input disabled>"), "<input disabled>");

    // Plain text with `<` that isn't a tag, and `on=` in prose — byte-identical.
    assert_eq!(strip_unsafe_attrs("a < b and on=off"), "a < b and on=off");

    // A comment mentioning onerror is NOT scanned — passes through verbatim.
    assert_eq!(strip_unsafe_attrs("<!-- onerror=x -->"), "<!-- onerror=x -->");

    // Self-closing tag terminator preserved; multibyte text intact.
    assert_eq!(strip_unsafe_attrs("café <br/> more"), "café <br/> more");

    // A URL attribute with a normal http URL is untouched.
    assert_eq!(
      strip_unsafe_attrs("<a href=\"https://x.test/on\">y</a>"),
      "<a href=\"https://x.test/on\">y</a>"
    );
  }

  #[test]
  fn export_html_strips_event_handler_from_raw_block() {
    // A raw (non-type-1) HTML block carrying an onerror handler must come out of
    // the full export pipeline with the handler gone.
    let md = "<div>\n<img src=x onerror=\"steal(document.cookie)\">\n</div>\n";
    let snapshot = import_markdown(md, "root");
    let html = export_html(&snapshot).expect("export_html");
    assert!(!html.contains("onerror"), "onerror leaked into export: {html}");
    assert!(!html.contains("steal("), "handler body leaked into export: {html}");
    assert!(html.contains("<img"), "the img tag itself should survive: {html}");
  }
}
