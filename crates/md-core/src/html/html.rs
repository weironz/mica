use std::collections::HashMap;

use serde_json::Value;

use crate::block::*;
use crate::export_md::*;
use crate::inline::*;
use crate::span_walk::{SpanTies, SpanWalker};
use crate::{Block, DocumentOperationError, DocumentOperationResult, DocumentSnapshotPayload};
use crate::{mermaid_id, render_math, render_mermaid_svg_with_id};

/// `index` is the id→position map `export_safe_tree` already built for this
/// snapshot — the recursion looks up every child by id, so threaded through
/// instead of re-scanned.
pub(crate) fn append_html_children(
  snapshot: &DocumentSnapshotPayload,
  index: &HashMap<&str, usize>,
  parent_id: &str,
  out: &mut String,
) -> DocumentOperationResult<()> {
  let parent_index = index
    .get(parent_id)
    .copied()
    .ok_or_else(|| DocumentOperationError::BlockNotFound(parent_id.to_string()))?;
  let child_ids = snapshot.blocks[parent_index].children.clone();

  let mut pos = 0;
  while pos < child_ids.len() {
    let child = block_for(snapshot, index, &child_ids[pos])?;
    if quote_depth_of(child) > 0 && li_of(child).is_none() {
      // Collect one quote group (a `qbreak` starts the next blockquote)
      // and rebuild the nested <blockquote> structure from `data.quote`.
      let run_start = pos;
      pos += 1;
      while pos < child_ids.len() {
        let b = block_for(snapshot, index, &child_ids[pos])?;
        if quote_depth_of(b) == 0 || b.data.get("qbreak").is_some() {
          break;
        }
        pos += 1;
      }
      let items: Vec<&Block> = child_ids[run_start..pos]
        .iter()
        .map(|id| block_for(snapshot, index, id))
        .collect::<DocumentOperationResult<_>>()?;
      render_quote_group(snapshot, index, &items, 1, out)?;
    } else if list_tag_for(&child.kind).is_some() {
      // Collect the maximal run of list items (any tag, any `data.indent`
      // level) plus their container children (`data.li`) and render the
      // nested <ul>/<ol> structure.
      let run_start = pos;
      while pos < child_ids.len() {
        let b = block_for(snapshot, index, &child_ids[pos])?;
        let is_li_child = li_of(b).is_some();
        if (list_tag_for(&b.kind).is_none() && !is_li_child)
          || (quote_depth_of(b) > 0 && !is_li_child)
        {
          break;
        }
        pos += 1;
      }
      let items: Vec<&Block> = child_ids[run_start..pos]
        .iter()
        .map(|id| block_for(snapshot, index, id))
        .collect::<DocumentOperationResult<_>>()?;
      render_html_list(snapshot, index, &items, 0, out)?;
    } else {
      append_html_block(snapshot, index, child, out)?;
      pos += 1;
    }
  }

  Ok(())
}

/// Render one quote group (flat blocks carrying `data.quote` depths) as
/// nested `<blockquote>`: deeper runs recurse, `quote`-kind blocks are the
/// paragraphs, list items group into lists, anything else renders normally.
pub(crate) fn render_quote_group(
  snapshot: &DocumentSnapshotPayload,
  index: &HashMap<&str, usize>,
  items: &[&Block],
  depth: usize,
  out: &mut String,
) -> DocumentOperationResult<()> {
  // A callout: the group head (depth 1) carries `data.alert`. Render a styled
  // blockquote (GitHub's `markdown-alert` classes + a title row) — still the
  // quote pipeline, just extra chrome, no separate block kind.
  // Allow-listed for the same reason as table `aligns`: it lands in a class
  // attribute unescaped.
  let alert = if depth == 1 {
    items
      .first()
      .and_then(|b| b.data.get("alert"))
      .and_then(Value::as_str)
      .filter(|a| matches!(*a, "note" | "tip" | "important" | "warning" | "caution"))
  } else {
    None
  };
  if let Some(a) = alert {
    out.push_str(&format!(
      "<blockquote class=\"markdown-alert markdown-alert-{a}\">\n\
       <p class=\"markdown-alert-title\">{}</p>\n",
      alert_title(a)
    ));
  } else {
    out.push_str("<blockquote>\n");
  }
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
      render_html_list(snapshot, index, &items[s..i], 0, out)?;
      continue;
    }
    if quote_depth_of(items[i]) > depth {
      let s = i;
      while i < items.len() && quote_depth_of(items[i]) > depth {
        i += 1;
      }
      render_quote_group(snapshot, index, &items[s..i], depth + 1, out)?;
      continue;
    }
    if items[i].kind == "quote" {
      let text = html_inline(items[i]);
      if !text.is_empty() {
        out.push_str(&format!("<p>{text}</p>\n"));
      }
    } else {
      append_html_block(snapshot, index, items[i], out)?;
    }
    i += 1;
  }
  out.push_str("</blockquote>\n");
  Ok(())
}

pub(crate) fn list_indent(block: &Block) -> usize {
  // Cap at `MAX_NESTING_DEPTH` (8) to mirror the Dart editor's read
  // (model.dart `indent` getter, `kMaxNestingDepth`). Without this cap the two engines DISAGREE on a block whose
  // `data.indent` was written past 8 by a non-editor client (MCP/REST/API-token,
  // or an import): Rust rendered N levels, Dart rendered 8 — same data, two
  // outputs, violating the round-trip/parity invariant. (P1-4.) Non-destructive,
  // like Dart's: the stored value is untouched, only the rendered depth is bound.
  data_depth(block, "indent").unwrap_or(0)
}

/// Is this block a container child of a list item, and at which level?
/// Clamped like `list_indent` (Dart `liLevel`): an unbounded `data.li` drove
/// `render_html_list` one recursion per level and `export_md`'s
/// `" ".repeat(level * 4 + 4)` — `li: 1e9` was a 4 GB string (P1-6).
pub(crate) fn li_of(block: &Block) -> Option<usize> {
  data_depth(block, "li")
}

/// The effective list level a block sits at: items by `data.indent`,
/// container children by `data.li`.
pub(crate) fn li_level(block: &Block) -> usize {
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
pub(crate) fn render_html_list(
  snapshot: &DocumentSnapshotPayload,
  index: &HashMap<&str, usize>,
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
      render_html_list(snapshot, index, &items[s..i], level + 1, out)?;
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
          render_li_children(snapshot, index, &items[head + 1..end], level, true, out)?;
        }
        out.push_str("</li>\n");
      } else {
        out.push_str("<li>");
        out.push_str(&body);
        if has_children {
          out.push('\n');
          render_li_children(snapshot, index, &items[head + 1..end], level, false, out)?;
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
pub(crate) fn render_li_children(
  snapshot: &DocumentSnapshotPayload,
  index: &HashMap<&str, usize>,
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
        render_quote_group(snapshot, index, &items[s..i], quote_depth_of(items[s]), out)?;
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
        append_html_block(snapshot, index, b, out)?;
      }
      i += 1;
      continue;
    }
    // Deeper: a nested list run (items and their children).
    let s = i;
    while i < items.len() && (li_level(items[i]) > level || li_of(items[i]) == Some(level + 1)) {
      i += 1;
    }
    render_html_list(snapshot, index, &items[s..i], level + 1, out)?;
  }
  Ok(())
}

/// Inline marks rendered to nested HTML (<strong>/<em>/<code>/<del>/<a>),
/// mirroring the markdown render_span structure.
/// Escape inline TEXT for HTML and render hard breaks (`\` + newline in
/// the block text) as `<br />`.
pub(crate) fn html_text(seg: &str) -> String {
  escape_html(seg).replace("\\\n", "<br />\n")
}

pub(crate) fn html_inline(block: &Block) -> String {
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
        escape_html(&escape_href(safe_url(
          m.href.as_deref().unwrap_or(""),
          false
        ))),
        escape_html(t)
      ),
      None => format!(
        "<a href=\"{}\"></a>",
        escape_html(&escape_href(safe_url(
          m.href.as_deref().unwrap_or(""),
          false
        )))
      ),
    };
  }
  let refs: Vec<&InlineMark> = marks.iter().collect();
  html_span(&units, 0, units.len(), &refs)
}

pub(crate) fn html_span(units: &[u16], lo: usize, hi: usize, marks: &[&InlineMark]) -> String {
  let mut out = String::new();
  let mut pos = lo;
  // The pick used to rescan the WHOLE slice per emitted span (and the overlap
  // set again): O(marks²) on a paragraph of sibling marks. The sweep keeps the
  // same selection — earliest clipped start, then widest span; on an exact
  // range tie (the strong+em pair of `***foo***`, delimiters collapsed to the
  // same edges) the mark matched LAST in `process_emphasis` is the outer one
  // (higher index) — behind one sort (issue #61).
  let mut walker = SpanWalker::new(marks, hi, SpanTies::Html);
  while pos < hi {
    let Some(pick) = walker.next_pick(pos) else {
      out.push_str(&html_text(&String::from_utf16_lossy(&units[pos..hi])));
      break;
    };
    let (s, e) = (pick.s, pick.e);
    out.push_str(&html_text(&String::from_utf16_lossy(&units[pos..s])));
    let m = pick.mark;
    if m.kind == "code" {
      let raw = String::from_utf16_lossy(&units[s..e]);
      out.push_str(&format!("<code>{}</code>", escape_html(&raw)));
      pos = e;
      continue;
    }
    if m.kind == "html" {
      // Raw inline HTML passes through unescaped (GFM tagfilter applied, then
      // an event-handler / `javascript:`-URL scrub for no-CSP consumers).
      out.push_str(&strip_unsafe_attrs(&tagfilter(&String::from_utf16_lossy(
        &units[s..e],
      ))));
      pos = e;
      continue;
    }
    if m.kind == "math" {
      let latex = String::from_utf16_lossy(&units[s..e]);
      match render_math(&latex, false) {
        Some(mathml) => out.push_str(&mathml),
        None => out.push_str(&format!(
          "<span class=\"math\">{}</span>",
          escape_html(&latex)
        )),
      }
      pos = e;
      continue;
    }
    if m.kind == "footnote" {
      // GFM reference shape: superscript backlink-anchored to the definition.
      // The label is the href; the visible text is the label too.
      let label = m
        .href
        .clone()
        .unwrap_or_else(|| String::from_utf16_lossy(&units[s..e]));
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
        escape_html(&escape_href(safe_url(
          m.href.as_deref().unwrap_or(""),
          true
        ))),
        escape_html(&alt)
      ));
      pos = e;
      continue;
    }
    let inner = walker.inner(&pick);
    let body = html_span(units, s, e, &inner);
    out.push_str(&match m.kind.as_str() {
      "bold" => format!("<strong>{body}</strong>"),
      "italic" => format!("<em>{body}</em>"),
      "strike" => format!("<del>{body}</del>"),
      "link" => match &m.title {
        Some(t) => format!(
          "<a href=\"{}\" title=\"{}\">{body}</a>",
          escape_html(&escape_href(safe_url(
            m.href.as_deref().unwrap_or(""),
            false
          ))),
          escape_html(t)
        ),
        None => format!(
          "<a href=\"{}\">{body}</a>",
          escape_html(&escape_href(safe_url(
            m.href.as_deref().unwrap_or(""),
            false
          )))
        ),
      },
      _ => body,
    });
    pos = e;
  }
  out
}

pub(crate) fn append_html_block(
  snapshot: &DocumentSnapshotPayload,
  index: &HashMap<&str, usize>,
  block: &Block,
  out: &mut String,
) -> DocumentOperationResult<()> {
  let text = html_inline(block);
  match block.kind.as_str() {
    "heading" => {
      let level = heading_level(block, 0);
      out.push_str(&format!("<h{level}>{text}</h{level}>\n"));
      append_html_children(snapshot, index, &block.id, out)?;
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
      append_html_children(snapshot, index, &block.id, out)?;
    }
    "quote" => {
      if text.is_empty() {
        out.push_str("<blockquote>\n</blockquote>\n");
      } else {
        out.push_str(&format!("<blockquote>\n<p>{text}</p>\n</blockquote>\n"));
      }
      append_html_children(snapshot, index, &block.id, out)?;
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
        if let Some(svg) = render_mermaid_svg_with_id(&block.text, &mermaid_id(&block.id), None) {
          out.push_str("<div class=\"mermaid\">");
          out.push_str(&svg);
          out.push_str("</div>\n");
          append_html_children(snapshot, index, &block.id, out)?;
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
      append_html_children(snapshot, index, &block.id, out)?;
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
          // Allow-listed: `data` is written by any member (yrs / MCP) and
          // nothing validates it, so a raw value broke out of the attribute.
          a @ ("left" | "center" | "right") => format!(" align=\"{a}\""),
          _ => String::new(),
        }
      };
      let cell_html = |cell: &Value| -> String {
        let (text, data) = decode_table_cell(cell);
        html_inline(&Block {
          id: String::new(),
          kind: "paragraph".to_string(),
          text,
          data,
          children: Vec::new(),
        })
      };
      out.push_str("<table>\n<thead>\n<tr>\n");
      if let Some(head) = rows.first().and_then(Value::as_array) {
        for (c, cell) in head.iter().enumerate() {
          out.push_str(&format!("<th{}>{}</th>\n", attr(c), cell_html(cell)));
        }
      }
      out.push_str("</tr>\n</thead>\n");
      if rows.len() > 1 {
        out.push_str("<tbody>\n");
        for row in rows.iter().skip(1) {
          out.push_str("<tr>\n");
          if let Some(cells) = row.as_array() {
            for (c, cell) in cells.iter().enumerate() {
              out.push_str(&format!("<td{}>{}</td>\n", attr(c), cell_html(cell)));
            }
          }
          out.push_str("</tr>\n");
        }
        out.push_str("</tbody>\n");
      }
      out.push_str("</table>\n");
    }
    "math_block" => match render_math(&block.text, true) {
      Some(mathml) => out.push_str(&format!("<div class=\"math\">{mathml}</div>\n")),
      None => out.push_str(&format!(
        "<div class=\"math\">{}</div>\n",
        escape_html(&block.text)
      )),
    },
    "footnote_def" => {
      // Definitions are not rendered in document order — they are gathered
      // into a single trailing <section class="footnotes"> by export_html.
    }
    "divider" => {
      out.push_str("<hr />\n");
    }
    "image" => {
      let url = escape_html(&escape_href(safe_url(
        block_data_str(block, "url").unwrap_or_default(),
        true,
      )));
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
      append_html_children(snapshot, index, &block.id, out)?;
    }
  }

  Ok(())
}

pub(crate) fn block_for<'a>(
  snapshot: &'a DocumentSnapshotPayload,
  index: &HashMap<&str, usize>,
  block_id: &str,
) -> DocumentOperationResult<&'a Block> {
  let Some(&i) = index.get(block_id) else {
    return Err(DocumentOperationError::BlockNotFound(block_id.to_string()));
  };
  Ok(&snapshot.blocks[i])
}

pub(crate) fn list_tag_for(kind: &str) -> Option<&'static str> {
  match kind {
    "bulleted_list" | "bullet_list" | "todo" => Some("ul"),
    "numbered_list" | "number_list" => Some("ol"),
    _ => None,
  }
}

/// GFM tagfilter: escape the opening `<` of the nine disallowed raw-HTML
/// tags so they render inert while everything else passes through.
pub(crate) fn tagfilter(s: &str) -> String {
  const BAD: &[&str] = &[
    "title",
    "textarea",
    "style",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "script",
    "plaintext",
  ];
  let bytes = s.as_bytes();
  let mut out = String::with_capacity(s.len());
  // Copy untouched text as whole `&str` runs. Pushing `bytes[i] as char` one
  // byte at a time reinterpreted every UTF-8 byte as Latin-1, so any non-ASCII
  // text in raw HTML (`<summary>点击展开</summary>`) came out as mojibake. `<`
  // is ASCII, so every cut below lands on a char boundary.
  let mut copied = 0;
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
      let boundary = matches!(
        bytes.get(j),
        None | Some(b' ' | b'\t' | b'\n' | b'>' | b'/')
      );
      if BAD.contains(&name.as_str()) && boundary {
        out.push_str(&s[copied..i]);
        out.push_str("&lt;");
        i += 1;
        copied = i;
        continue;
      }
    }
    i += 1;
  }
  out.push_str(&s[copied..]);
  out
}

/// Names of attributes whose value is fetched as a URL — the vectors for a
/// `javascript:`/`vbscript:` scheme injection. Lower-cased for comparison.
pub(crate) const URL_ATTRS: &[&str] = &[
  "href",
  "src",
  "xlink:href",
  "action",
  "formaction",
  "poster",
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
pub(crate) fn is_unsafe_attr(name: &str, value: &str) -> bool {
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
/// `url` unless its scheme can run script, in which case empty.
///
/// The share page has a CSP; the downloaded HTML export does not, so a plain
/// Markdown link `[x](javascript:…)` ran when clicked in the saved file.
/// `data:` is allowed only for images (`data:image/…` — what export inlines);
/// an `<img>` never executes it. Matching mirrors the URL parser: leading
/// whitespace/control chars and tabs/newlines inside the scheme are ignored,
/// case-insensitively.
pub(crate) fn safe_url(url: &str, for_image: bool) -> &str {
  let normalized: String = url
    .trim_start_matches(|c: char| c <= ' ')
    .chars()
    .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
    .collect::<String>()
    .to_ascii_lowercase();
  let Some(colon) = normalized.find(':') else {
    return url; // no scheme: relative
  };
  let scheme = &normalized[..colon];
  if !scheme
    .chars()
    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
  {
    return url; // the `:` is later in a relative path, not a scheme
  }
  let dangerous = match scheme {
    "javascript" | "vbscript" => true,
    "data" => !(for_image && normalized[colon + 1..].starts_with("image/")),
    _ => false,
  };
  if dangerous { "" } else { url }
}

pub(crate) fn strip_unsafe_attrs(s: &str) -> String {
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
    // `<!-- … -->` comment: copy verbatim, do not scan. It must end where a
    // BROWSER ends it, or markup the browser renders is skipped here: `<!-->`
    // and `<!--->` are complete (abrupt) comments and `--!>` also closes one,
    // so `<!-- --!><img onerror=…> -->` used to pass through unscanned.
    if s[i + 1..].starts_with("!--") {
      let rest = &s[i + 4..];
      let end = if rest.starts_with('>') {
        i + 5
      } else if rest.starts_with("->") {
        i + 6
      } else {
        let close = [
          rest.find("-->").map(|k| k + 3),
          rest.find("--!>").map(|k| k + 4),
        ]
        .into_iter()
        .flatten()
        .min();
        close.map_or(b.len(), |k| i + 4 + k)
      };
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
    // The tag is assembled in `tag` first and only appended once it has been
    // fully parsed: a `<meta http-equiv=…>` must be DROPPED, and that is only
    // knowable after its attributes have been scanned.
    let mut tag = String::new();
    tag.push('<');
    if closing {
      tag.push('/');
    }
    let name_start = j;
    while b
      .get(j)
      .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'-')
    {
      j += 1;
    }
    let tag_name = &s[name_start..j];
    tag.push_str(tag_name);
    // `<meta http-equiv="refresh" content="0;url=…">` is a client-side redirect
    // no CSP can veto (`form-action`/`base-uri` do not cover it): on the share
    // page it makes a trusted URL bounce to an arbitrary site. No http-equiv
    // value is meaningful in body content, so the whole tag is dropped. A
    // `<meta>` WITHOUT http-equiv (charset, name=…) is inert and kept.
    let is_meta = !closing && tag_name.eq_ignore_ascii_case("meta");
    let mut meta_has_http_equiv = false;
    // Attribute region: loop until the tag terminator (`>` / `/>` / EOF).
    loop {
      let ws_start = j;
      while b.get(j).is_some_and(u8::is_ascii_whitespace) {
        j += 1;
      }
      let ws = &s[ws_start..j];
      match b.get(j) {
        None => {
          tag.push_str(ws);
          break;
        }
        Some(&b'>') => {
          tag.push_str(ws);
          tag.push('>');
          j += 1;
          break;
        }
        Some(&b'/') if b.get(j + 1) == Some(&b'>') => {
          tag.push_str(ws);
          tag.push_str("/>");
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
        tag.push_str(ws);
        tag.push(b[j] as char);
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
            while b
              .get(k)
              .is_some_and(|c| !c.is_ascii_whitespace() && *c != b'>')
            {
              k += 1;
            }
            value = &s[vs..k];
          }
        }
        j = k;
      }
      let attr_text = &s[attr_start..j];
      if is_meta && name.eq_ignore_ascii_case("http-equiv") {
        meta_has_http_equiv = true;
      }
      if is_unsafe_attr(name, value) {
        // Drop: skip the leading whitespace AND the attribute entirely.
      } else {
        tag.push_str(ws);
        tag.push_str(attr_text);
      }
    }
    if !(is_meta && meta_has_http_equiv) {
      out.push_str(&tag);
    }
    // Advance the outer cursor past the tag the attribute loop just consumed
    // (`j` walked to the `>` / `/>` / EOF). Without this the outer `while` would
    // re-scan the same tag forever.
    i = j;
  }
  out
}

#[cfg(test)]
mod security_tests {
  use super::{strip_unsafe_attrs, tagfilter};

  // Pins what the GFM tagfilter does — and, crucially, what it does NOT do — for
  // raw HTML rendered onto a Mica same-origin surface (the public share page).
  #[test]
  fn tagfilter_blacklist_neutralizes_script_but_not_event_handlers() {
    assert!(tagfilter("<script>alert(1)</script>").starts_with("&lt;script"));
    assert!(tagfilter("<iframe src=x>").starts_with("&lt;iframe"));

    let img = tagfilter("<img src=x onerror=\"steal()\">");
    assert!(
      img.contains("onerror"),
      "tagfilter unexpectedly stripped onerror: {img}"
    );
    assert!(
      img.starts_with("<img"),
      "img tag should not be escaped: {img}"
    );

    let svg = tagfilter("<svg onload=\"steal()\">");
    assert!(
      svg.contains("onload"),
      "tagfilter unexpectedly stripped onload: {svg}"
    );
    assert!(
      svg.starts_with("<svg"),
      "svg tag should not be escaped: {svg}"
    );
  }

  #[test]
  fn tagfilter_keeps_non_ascii_text_intact() {
    let raw = "<details>\n<summary>点击展开 😀</summary>\n<script>x</script>é</details>";
    assert_eq!(
      tagfilter(raw),
      "<details>\n<summary>点击展开 😀</summary>\n&lt;script>x&lt;/script>é</details>"
    );
  }

  #[test]
  fn strip_unsafe_attrs_removes_event_handlers() {
    let a = strip_unsafe_attrs("<img src=x onerror=\"steal()\">");
    assert!(!a.contains("onerror"), "onerror survived: {a}");
    assert!(a.contains("src=x"), "safe src dropped: {a}");
    assert!(a.starts_with("<img"), "tag mangled: {a}");

    let b = strip_unsafe_attrs("<svg onload='x'>");
    assert!(!b.contains("onload"), "onload survived: {b}");

    let c = strip_unsafe_attrs("<body OnLoad=x>");
    assert!(
      !c.to_ascii_lowercase().contains("onload"),
      "OnLoad survived: {c}"
    );
    assert_eq!(c, "<body>");

    let d = strip_unsafe_attrs("<img src=x onerror=steal()>");
    assert!(!d.contains("onerror"), "unquoted onerror survived: {d}");
  }

  #[test]
  fn strip_unsafe_attrs_removes_js_and_vbscript_urls() {
    let a = strip_unsafe_attrs("<a href=\"javascript:alert(1)\">x</a>");
    assert!(!a.contains("javascript"), "javascript: href survived: {a}");

    let b = strip_unsafe_attrs("<a href=\"  JAVAScript:x\">x</a>");
    assert!(
      !b.to_ascii_lowercase().contains("javascript"),
      "js href survived: {b}"
    );

    let c = strip_unsafe_attrs("<a href=\"java\tscript:x\">x</a>");
    assert!(
      !c.to_ascii_lowercase().contains("script:"),
      "tab-split js href survived: {c}"
    );

    let d = strip_unsafe_attrs("<img src=\"vbscript:msgbox(1)\">");
    assert!(
      !d.to_ascii_lowercase().contains("vbscript"),
      "vbscript src survived: {d}"
    );
  }

  #[test]
  fn strip_unsafe_attrs_preserves_safe_content() {
    let img = strip_unsafe_attrs("<img src=\"x.png\" alt=\"turn on the light\">");
    assert_eq!(img, "<img src=\"x.png\" alt=\"turn on the light\">");

    assert_eq!(
      strip_unsafe_attrs("<div class=\"on\">"),
      "<div class=\"on\">"
    );
    assert_eq!(strip_unsafe_attrs("<input disabled>"), "<input disabled>");
    assert_eq!(strip_unsafe_attrs("a < b and on=off"), "a < b and on=off");
    assert_eq!(
      strip_unsafe_attrs("<!-- onerror=x -->"),
      "<!-- onerror=x -->"
    );
    assert_eq!(strip_unsafe_attrs("café <br/> more"), "café <br/> more");
    assert_eq!(
      strip_unsafe_attrs("<a href=\"https://x.test/on\">y</a>"),
      "<a href=\"https://x.test/on\">y</a>"
    );
  }

  #[test]
  fn meta_http_equiv_tags_are_dropped() {
    let r =
      strip_unsafe_attrs("<meta http-equiv=\"refresh\" content=\"0;url=https://evil.example\">");
    assert!(r.is_empty(), "refresh meta survived: {r}");

    let m = strip_unsafe_attrs("<META HTTP-EQUIV=Refresh CONTENT=\"0; URL=//evil\">");
    assert!(m.is_empty(), "mixed-case refresh meta survived: {m}");

    let u = strip_unsafe_attrs("<meta http-equiv>");
    assert!(u.is_empty(), "valueless http-equiv meta survived: {u}");

    assert_eq!(
      strip_unsafe_attrs("<meta name=\"description\" content=\"笔记\">"),
      "<meta name=\"description\" content=\"笔记\">"
    );

    assert_eq!(
      strip_unsafe_attrs("前<meta http-equiv=\"refresh\" content=\"0;url=x\">后"),
      "前后"
    );
  }
}
