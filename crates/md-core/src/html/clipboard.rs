//! Mica clipboard HTML, ported from EditorController.selectionHtml and
//! inlineToHtml. No rich renderer is called: source math and Mermaid must
//! survive a clipboard round trip independently of the `render` feature.

use serde_json::Value;

use super::html::{safe_url, strip_unsafe_attrs, tagfilter};
use crate::inline::marks_from_block;
use crate::model::{Block, DocumentSnapshotPayload, MdError};
use crate::serialize::span_walk::{SpanTies, SpanWalker};

fn escape(text: &str) -> String {
  text
    .replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
}

fn attr(text: &str) -> String {
  escape(text).replace('"', "&quot;")
}

fn depth(block: &Block) -> usize {
  let d = number(block, "quote", 0).clamp(0, 8) as usize;
  if block.kind == "quote" { d.max(1) } else { d }
}

fn number(block: &Block, key: &str, default: i64) -> i64 {
  block
    .data
    .get(key)
    .and_then(Value::as_i64)
    .or_else(|| {
      block
        .data
        .get(key)
        .and_then(Value::as_f64)
        .map(|x| x as i64)
    })
    .unwrap_or(default)
}

fn string<'a>(block: &'a Block, key: &str) -> Option<&'a str> {
  block.data.get(key).and_then(Value::as_str)
}

fn set_quote(out: &mut String, current: &mut usize, wanted: usize) {
  while *current > wanted {
    out.push_str("</blockquote>");
    *current -= 1;
  }
  while *current < wanted {
    out.push_str("<blockquote>");
    *current += 1;
  }
}

fn is_list(block: &Block) -> bool {
  matches!(
    block.kind.as_str(),
    "bulleted_list" | "numbered_list" | "todo"
  )
}

/// Incoming blocks already represent the selection: the Dart boundary clips
/// marks and removes quote/qbreak from partial selections before calling us.
pub(crate) fn render(doc: &DocumentSnapshotPayload) -> Result<String, MdError> {
  if doc.schema_version != crate::SCHEMA_VERSION {
    return Err(MdError::UnsupportedSchemaVersion(doc.schema_version));
  }
  let root = doc
    .blocks
    .iter()
    .find(|b| b.id == doc.root_block_id)
    .ok_or_else(|| MdError::BlockNotFound(doc.root_block_id.clone()))?;
  let index: std::collections::HashMap<&str, &Block> =
    doc.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
  let blocks: Vec<&Block> = root
    .children
    .iter()
    .map(|id| {
      index
        .get(id.as_str())
        .copied()
        .ok_or_else(|| MdError::BlockNotFound(id.clone()))
    })
    .collect::<Result<_, _>>()?;
  // A full selection of one empty textual node is a collapsed caret in the
  // old controller; only atomic blocks are selectable at that caret.
  if blocks.len() == 1
    && blocks[0].text.is_empty()
    && !matches!(
      blocks[0].kind.as_str(),
      "image" | "divider" | "table" | "math_block"
    )
  {
    return Ok(String::new());
  }
  let mut out = String::new();
  let mut quote = 0;
  let mut i = 0;
  while i < blocks.len() {
    let block = blocks[i];
    match block.kind.as_str() {
      "image" => {
        set_quote(&mut out, &mut quote, 0);
        let src = string(block, "url")
          .or_else(|| string(block, "name"))
          .unwrap_or("");
        out.push_str(&format!(
          "<p><img src=\"{}\" alt=\"{}\"></p>",
          attr(safe_url(src, true)),
          attr(&block.text)
        ));
      }
      "divider" => {
        set_quote(&mut out, &mut quote, 0);
        out.push_str("<hr>");
      }
      "table" => {
        set_quote(&mut out, &mut quote, 0);
        table(block, &mut out);
      }
      "code_block" => {
        set_quote(&mut out, &mut quote, depth(block));
        // Language detection stays in the Dart boundary; the codec only emits
        // the supplied language, including Mermaid source (never SVG).
        let lang = string(block, "language").unwrap_or("");
        let class = if lang.is_empty() {
          String::new()
        } else {
          format!(" class=\"language-{}\"", attr(lang))
        };
        out.push_str(&format!(
          "<pre><code{class}>{}</code></pre>",
          escape(&block.text)
        ));
      }
      "math_block" => {
        set_quote(&mut out, &mut quote, 0);
        out.push_str(&format!(
          "<p><span data-mica-math=\"1\">$${}$$</span></p>",
          escape(&block.text)
        ));
      }
      "footnote_def" => {
        set_quote(&mut out, &mut quote, 0);
        out.push_str(&format!(
          "<p data-mica-fndef=\"{}\">{}</p>",
          attr(string(block, "label").unwrap_or("")),
          inline(block)
        ));
      }
      _ if is_list(block) => {
        let q = depth(block);
        set_quote(&mut out, &mut quote, q);
        let start = i;
        while i < blocks.len() && is_list(blocks[i]) && depth(blocks[i]) == q {
          i += 1;
        }
        let mut cursor = 0;
        list(&blocks[start..i], &mut cursor, 0, &mut out);
        continue;
      }
      "quote" => {
        if block.data.get("qbreak").and_then(Value::as_bool) == Some(true) {
          set_quote(&mut out, &mut quote, 0);
        }
        set_quote(&mut out, &mut quote, depth(block));
        out.push_str(&format!("<p>{}</p>", inline(block)));
      }
      "heading" => {
        set_quote(&mut out, &mut quote, depth(block));
        let level = number(block, "level", 1).clamp(1, 6);
        out.push_str(&format!("<h{level}>{}</h{level}>", inline(block)));
      }
      _ => {
        set_quote(&mut out, &mut quote, depth(block));
        out.push_str(&format!("<p>{}</p>", inline(block)));
      }
    }
    i += 1;
  }
  set_quote(&mut out, &mut quote, 0);
  // Same safety pipeline as Export. Source HTML marks were already escaped;
  // these layers also protect attribute contexts without changing safe HTML.
  Ok(strip_unsafe_attrs(&tagfilter(&out)))
}

fn list(items: &[&Block], i: &mut usize, level: usize, out: &mut String) {
  let mut tag: Option<&str> = None;
  let mut li_open = false;
  while *i < items.len() {
    let block = items[*i];
    let indent = number(block, "indent", 0).clamp(0, 8) as usize;
    if indent < level {
      break;
    }
    if indent > level {
      list(items, i, level + 1, out);
      continue;
    }
    let want = if block.kind == "numbered_list" {
      "ol"
    } else {
      "ul"
    };
    if tag != Some(want) {
      if li_open {
        out.push_str("</li>");
        li_open = false;
      }
      if let Some(tag) = tag {
        out.push_str(&format!("</{tag}>"));
      }
      if want == "ol"
        && block.data.get("start").is_some_and(Value::is_i64)
        && number(block, "start", 1) != 1
      {
        out.push_str(&format!("<ol start=\"{}\">", number(block, "start", 1)));
      } else {
        out.push_str(&format!("<{want}>"));
      }
      tag = Some(want);
    }
    if li_open {
      out.push_str("</li>");
    }
    out.push_str("<li>");
    if block.kind == "todo" {
      let checked = if block.data.get("checked").and_then(Value::as_bool) == Some(true) {
        " checked"
      } else {
        ""
      };
      out.push_str(&format!("<input type=\"checkbox\"{checked} disabled> "));
    }
    out.push_str(&inline(block));
    li_open = true;
    *i += 1;
  }
  if li_open {
    out.push_str("</li>");
  }
  if let Some(tag) = tag {
    out.push_str(&format!("</{tag}>"));
  }
}

fn table(block: &Block, out: &mut String) {
  out.push_str("<table>");
  if let Some(rows) = block.data.get("rows").and_then(Value::as_array) {
    for (i, row) in rows.iter().enumerate() {
      let Some(cells) = row.as_array() else {
        continue;
      };
      out.push_str("<tr>");
      let tag = if i == 0 { "th" } else { "td" };
      for cell in cells {
        let decoded = crate::decode_cell(cell);
        let block = Block::new("", "paragraph")
          .with_text(decoded.text)
          .with_data(serde_json::json!({"marks": decoded.marks}));
        out.push_str(&format!("<{tag}>{}</{tag}>", inline(&block)));
      }
      out.push_str("</tr>");
    }
  }
  out.push_str("</table>");
}

fn inline(block: &Block) -> String {
  let marks = marks_from_block(block);
  let refs: Vec<_> = marks.iter().filter(|m| m.end > m.start).collect();
  let units: Vec<_> = block.text.encode_utf16().collect();
  span(&units, 0, units.len(), &refs)
}

fn span(units: &[u16], lo: usize, hi: usize, marks: &[&crate::inline::InlineMark]) -> String {
  let text = |a, b| String::from_utf16_lossy(&units[a..b]);
  let mut out = String::new();
  let mut walk = SpanWalker::new(marks, hi, SpanTies::Html);
  let mut pos = lo;
  while pos < hi {
    let Some(pick) = walk.next_pick(pos) else {
      out.push_str(&escape(&text(pos, hi)));
      break;
    };
    let (s, e, mark) = (pick.s, pick.e, pick.mark);
    out.push_str(&escape(&text(pos, s)));
    match mark.kind.as_str() {
      "code" => out.push_str(&format!("<code>{}</code>", escape(&text(s, e)))),
      "math" => out.push_str(&format!(
        "<span data-mica-math=\"1\">${}$</span>",
        escape(&text(s, e))
      )),
      "footnote" => out.push_str(&format!(
        "<sup data-mica-footnote=\"{}\">{}</sup>",
        attr(mark.href.as_deref().unwrap_or(&text(s, e))),
        escape(&text(s, e))
      )),
      "html" => out.push_str(&escape(&text(s, e))),
      kind => {
        let inner = walk.inner(&pick);
        let body = span(units, s, e, &inner);
        match kind {
          "bold" => out.push_str(&format!("<strong>{body}</strong>")),
          "italic" => out.push_str(&format!("<em>{body}</em>")),
          "strike" => out.push_str(&format!("<s>{body}</s>")),
          "link" => out.push_str(&format!(
            "<a href=\"{}\">{body}</a>",
            attr(safe_url(mark.href.as_deref().unwrap_or(""), false))
          )),
          "image" => out.push_str(&format!(
            "<img src=\"{}\" alt=\"{}\">",
            attr(safe_url(mark.href.as_deref().unwrap_or(""), true)),
            attr(&text(s, e))
          )),
          _ => out.push_str(&body),
        }
      }
    }
    pos = e;
  }
  out
}
