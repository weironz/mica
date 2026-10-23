//! Mica's Markdown engine — the block model ("AST") plus parsing and
//! rendering between it and Markdown/HTML. Pure and I/O-free; document
//! *operations* (insert/update/move) live in `mica-app-core`, archive-level
//! interchange in `mica-interchange`. See docs/architecture/architecture.md.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod block_handlers;
/// Structured page properties over the document root's raw front-matter string
/// (front matter stays the sole authority; this is a lazy view + surgical edit).
pub mod properties;

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

impl Block {
  /// A block with just an id and kind — the seed the builders below grow.
  /// (`mica-core` re-exports this type, so `mica_core::Block::new` is the same
  /// constructor the yrs model has always had.)
  pub fn new(id: impl Into<String>, kind: impl Into<String>) -> Self {
    Block {
      id: id.into(),
      kind: kind.into(),
      text: String::new(),
      data: Value::Null,
      children: Vec::new(),
    }
  }

  pub fn with_text(mut self, text: impl Into<String>) -> Self {
    self.text = text.into();
    self
  }

  pub fn with_data(mut self, data: Value) -> Self {
    self.data = data;
    self
  }

  pub fn with_children(mut self, children: Vec<String>) -> Self {
    self.children = children;
    self
  }
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

  #[error(
    "a newly inserted block must be a leaf (empty children); attach children with their own inserts"
  )]
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

/// The document's own title, if it carries one.
///
/// Lives on the root block's `data`, beside `front_matter` — the one place that
/// is part of the document but is NOT a body block. That distinction is the
/// whole point (see `docs/features/page-title-plan.md`): the title travels with the
/// document, so every consumer sees it, while the body's first line stays free
/// to be whatever the author wants.
///
/// `None` for a document that predates this — the caller falls back to the
/// view's `name` column. There is deliberately no backfill migration: rather
/// than write a CRDT update for every existing page, an old page simply keeps
/// answering from its name until someone edits its title.
///
/// Blank is treated as absent, so an emptied title falls back rather than
/// rendering as nothing.
pub fn document_title(snapshot: &DocumentSnapshotPayload) -> Option<&str> {
  let root = block_index(snapshot, &snapshot.root_block_id).map(|i| &snapshot.blocks[i])?;
  root
    .data
    .get("title")
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|title| !title.is_empty())
}

/// Put the page's title on the root block of a payload that is about to BECOME
/// a document.
///
/// The write counterpart of [`document_title`], and deliberately the only one at
/// the payload level. Every other writer edits a document that already exists
/// and must go through the narrow yrs primitive (`MicaDoc::set_block_prop`), or
/// it invalidates every comment anchor on the page — `docs/features/page-title-plan.md`
/// §5.1. Here there is no document yet and no anchor to lose: the title rides
/// along in the seed and costs no update at all. That is what makes importing an
/// archive compatible with §5.3 ("400 pages must not mean 400 document writes")
/// while still leaving every imported page carrying its own name.
///
/// Blank is ignored rather than stored, matching [`document_title`], which reads
/// blank back as absent — storing one would add a key that answers nothing.
///
/// Returns whether the title was written.
pub fn set_document_title(snapshot: &mut DocumentSnapshotPayload, title: &str) -> bool {
  let title = title.trim();
  if title.is_empty() {
    return false;
  }
  let Some(index) = block_index(snapshot, &snapshot.root_block_id) else {
    return false;
  };
  let data = &mut snapshot.blocks[index].data;
  // `data` defaults to null and is omitted when empty, and front matter lives in
  // this same map — so this has to extend whatever is there, not replace it.
  if !data.is_object() {
    *data = Value::Object(serde_json::Map::new());
  }
  data
    .as_object_mut()
    .expect("just made an object")
    .insert("title".to_string(), Value::String(title.to_string()));
  true
}

/// Put `# {title}` into an exported page's TEXT, after any front matter.
///
/// Only for exports that produce a FILE the user takes away. Deliberately NOT
/// applied to the read endpoints: `GET .../export/markdown` is the read half of
/// a read/PATCH pair (and what MCP `mica_read_document` calls), so a title added
/// there would be written back as a real heading by the next read-modify-write
/// and compound on every round trip.
///
/// Three rules, each of which is wrong to leave out:
///
/// 1. **After the front matter, never before.** YAML front matter is only front
///    matter when it is the first thing in the file; a heading above it turns
///    the opening `---` into a thematic break and the whole block into text.
/// 2. **Not added when the body already opens with that exact heading.** Some
///    people type the title as their first line by hand; welding a second copy
///    on top is the doubling this feature exists to avoid. Exact match only —
///    a leading heading that says something else is the author's content.
/// 3. **Nothing at all for an empty title**, so an untitled page does not export
///    a bare `# `.
///
/// The counterpart is [`strip_page_title`] right below, which import
/// (`mica_interchange`) uses to remove exactly what this writes so
/// export→import stays stable (CLAUDE.md #4). Both find the heading through
/// [`is_page_title_line`] and the front matter through `split_front_matter`, so
/// the format the pair agrees on is defined once.
pub fn with_page_title(markdown: &str, title: &str) -> String {
  let title = title.trim();
  if title.is_empty() {
    return markdown.to_string();
  }
  let heading = format!("# {title}");
  // Reuses the parser's own front-matter split rather than a second one — an
  // unterminated `---` is not front matter, and `...` closes a block too. A
  // private copy of that rule here would be the double-representation this
  // codebase keeps getting burned by.
  let (front, body) = split_front_matter(markdown);
  if leading_heading(body).is_some_and(|line| is_page_title_line(line, title)) {
    return markdown.to_string();
  }
  let body = body.trim_start_matches('\n');
  // Re-fenced exactly the way `export_markdown_with_assets` emits it, empty
  // block included (`---\n---`, not `---\n\n---`).
  let prefix = match front {
    None => String::new(),
    Some(inner) if inner.is_empty() => "---\n---\n".to_string(),
    Some(inner) => format!("---\n{inner}\n---\n"),
  };
  if body.is_empty() {
    return format!("{prefix}{heading}\n");
  }
  format!("{prefix}{heading}\n\n{body}")
}

/// The inverse of [`with_page_title`]: take the `# {title}` line it wrote back
/// off a page's text, leaving any front matter in place above the body.
///
/// For import of archives whose title is KNOWN to be welded onto the text (our
/// own exports, and Notion's). The writer's rules, mirrored:
///
/// - the heading is looked for **after the front matter**, because that is
///   where the writer puts it (a stripper that only looked at the first line of
///   the file missed it on every page with properties);
/// - **exact match only** — a leading heading that says something else is the
///   author's content and survives;
/// - **an empty title strips nothing.**
///
/// The blank lines the writer puts between heading and body go with it;
/// everything else, front matter included, comes back verbatim.
pub fn strip_page_title(markdown: &str, title: &str) -> String {
  let title = title.trim();
  if title.is_empty() {
    return markdown.to_string();
  }
  let (_, body) = split_front_matter(markdown);
  // `body` is a suffix of `markdown`, so this is the front matter exactly as
  // written, fences included ("" when there is none).
  let front = &markdown[..markdown.len() - body.len()];
  let mut offset = 0;
  for line in body.split_inclusive('\n') {
    offset += line.len();
    if line.trim().is_empty() {
      continue;
    }
    if !is_page_title_line(line, title) {
      break;
    }
    let rest = body[offset..].trim_start_matches(['\n', '\r']);
    return format!("{front}{rest}");
  }
  markdown.to_string()
}

/// Whether `line` is the heading [`with_page_title`] writes for `title`
/// (already trimmed): `# {title}`, tolerant of surrounding whitespace only.
fn is_page_title_line(line: &str, title: &str) -> bool {
  line.trim().strip_prefix("# ").map(str::trim) == Some(title)
}

/// The first non-blank line, if it is an ATX heading; otherwise `None`.
fn leading_heading(markdown: &str) -> Option<&str> {
  markdown
    .lines()
    .find(|line| !line.trim().is_empty())
    .filter(|line| line.trim_start().starts_with('#'))
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
  let (safe, index) = export_safe_tree(snapshot);
  let snapshot = &*safe;

  let mut lines = Vec::new();
  let root_index = index
    .get(snapshot.root_block_id.as_str())
    .copied()
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

  append_markdown_children(
    snapshot,
    &index,
    &snapshot.root_block_id,
    0,
    &mut lines,
    images,
  )?;

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
  let (safe, index) = export_safe_tree(snapshot);
  let snapshot = &*safe;

  let mut html = String::new();
  let root_index = index
    .get(snapshot.root_block_id.as_str())
    .copied()
    .ok_or_else(|| DocumentOperationError::BlockNotFound(snapshot.root_block_id.clone()))?;
  let root = &snapshot.blocks[root_index];
  if !root.text.trim().is_empty() {
    html.push_str(&format!("<p>{}</p>\n", escape_html(root.text.trim())));
  }

  append_html_children(snapshot, &index, &snapshot.root_block_id, &mut html)?;
  append_footnotes_section(snapshot, &mut html);

  Ok(html.trim_end().to_string())
}

/// The ONE Content Security Policy for every static page Mica renders onto a
/// same-origin or offline surface: the HTTP header on the public share page AND
/// the `<meta http-equiv>` of a downloaded HTML export carry this exact string,
/// so the two surfaces cannot drift into different strengths. No `script-src`,
/// so the `default-src 'none'` fallback blocks all script (tags AND `on*`
/// handlers). `form-action 'none'` stops a raw `<form action=…>` from phishing
/// off the trusted origin; `base-uri 'none'` stops a raw `<base href>` from
/// hijacking relative links. (`<meta http-equiv="refresh">` slips past ANY of
/// these directives — that one is dropped by the renderer itself, in
/// `html.rs`'s `strip_unsafe_attrs`.) `img-src` allows `self`/data/https and —
/// for exports opened from `file://`, whose image links are the author's own —
/// plain http; nothing else can fetch anything anywhere.
pub const SAFE_HTML_CSP: &str = "default-src 'none'; img-src 'self' data: https: http:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; form-action 'none'; base-uri 'none'";

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
  // The downloaded file has no server to send the share page's CSP header, so
  // it carries the SAME policy as a <meta> ([SAFE_HTML_CSP]): no script at all
  // (raw HTML blocks pass through by design, and a `<script>` or `on*=` in one
  // ran when the file was opened). Images stay loadable — export inlines them
  // as `data:`, and external ones are the author's own links.
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
     <meta http-equiv=\"Content-Security-Policy\" content=\"{SAFE_HTML_CSP}\">\n\
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
     blockquote.markdown-alert{{padding:.4rem 1rem;color:inherit;border-left-width:4px;}}\n\
     .markdown-alert-title{{display:flex;align-items:center;font-weight:600;\
     margin:0 0 .25rem;text-transform:none;}}\n\
     .markdown-alert-note{{border-left-color:#0969da;}}\n\
     .markdown-alert-note .markdown-alert-title{{color:#0969da;}}\n\
     .markdown-alert-tip{{border-left-color:#1a7f37;}}\n\
     .markdown-alert-tip .markdown-alert-title{{color:#1a7f37;}}\n\
     .markdown-alert-important{{border-left-color:#8250df;}}\n\
     .markdown-alert-important .markdown-alert-title{{color:#8250df;}}\n\
     .markdown-alert-warning{{border-left-color:#9a6700;}}\n\
     .markdown-alert-warning .markdown-alert-title{{color:#9a6700;}}\n\
     .markdown-alert-caution{{border-left-color:#cf222e;}}\n\
     .markdown-alert-caution .markdown-alert-title{{color:#cf222e;}}\n\
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
  use ratex_layout::{LayoutOptions, layout, to_display_list};
  use ratex_svg::{SvgOptions, render_to_svg};
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
    format!(
      "height:{height_em:.4}em;vertical-align:-{:.4}em",
      list.depth
    )
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

/// The host's palette, handed to merman so a diagram is drawn in the SAME
/// colours as the page around it.
///
/// The values are CSS colour strings and they come from the CLIENT — `MicaTokens`
/// is the one place the app's palette is defined, and copying it into Rust would
/// be a second copy free to drift. merman documents this host-theme profile as
/// the supported integration seam, which is why none of this touches the CSS
/// flattener in `mermaid_svg_inline.dart`.
#[derive(Debug, Clone)]
pub struct MermaidHostTheme {
  pub dark: bool,
  pub canvas: String,
  pub surface: String,
  pub surface_alt: String,
  pub text: String,
  pub subtle_text: String,
  pub border: String,
  pub line: String,
  pub error: String,
  pub warning: String,
  pub success: String,
}

/// A Mermaid diagram → a self-contained inline `<svg>` via the headless merman
/// engine (the SAME engine the editor renders with). `id` scopes the SVG's
/// `<style>` so multiple diagrams on one page don't cross-style each other.
/// `None` on a syntax/render error (or feature off) → caller keeps the code.
///
/// `theme` absent keeps merman's own default (light), which is what the export
/// path wants: an exported HTML file is not inside anyone's editor.
#[cfg(feature = "render")]
pub fn render_mermaid_svg_with_id(
  source: &str,
  id: &str,
  theme: Option<&MermaidHostTheme>,
) -> Option<String> {
  use merman::render::{
    HeadlessRenderer, HostThemeAppearance, HostThemeProfile, HostThemeRoles, SvgPipeline,
  };
  // `resvg-safe`: plain shapes/text, no `<foreignObject>`. The editor's raster
  // preview (flutter_svg) can't decode foreignObject, so it already renders
  // this pipeline — using it here too means export == on-screen, and browsers
  // render the safe subset fine.
  let pipeline = SvgPipeline::resvg_safe();
  let renderer = HeadlessRenderer::new()
    .with_strict_parsing()
    .with_diagram_id(id);
  let profile = theme.map(|t| {
    HostThemeProfile::builder()
      .appearance(if t.dark {
        HostThemeAppearance::Dark
      } else {
        HostThemeAppearance::Light
      })
      .roles(HostThemeRoles {
        canvas: Some(t.canvas.clone()),
        surface: Some(t.surface.clone()),
        surface_alt: Some(t.surface_alt.clone()),
        text: Some(t.text.clone()),
        subtle_text: Some(t.subtle_text.clone()),
        border: Some(t.border.clone()),
        line: Some(t.line.clone()),
        error: Some(t.error.clone()),
        warning: Some(t.warning.clone()),
        success: Some(t.success.clone()),
        ..Default::default()
      })
      .build()
  });
  let renderer = match profile.as_ref() {
    Some(p) => renderer.with_host_theme(p),
    None => renderer,
  };
  renderer
    .render_svg_with_pipeline_sync(source, &pipeline)
    .ok()
    .flatten()
}
#[cfg(not(feature = "render"))]
pub fn render_mermaid_svg_with_id(
  _source: &str,
  _id: &str,
  _theme: Option<&MermaidHostTheme>,
) -> Option<String> {
  None
}

/// Single-diagram convenience (the editor's live FFI preview): a stable id.
pub fn render_mermaid_svg(source: &str, theme: Option<&MermaidHostTheme>) -> Option<String> {
  render_mermaid_svg_with_id(source, "mica-mermaid", theme)
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
  import_markdown_inner(markdown, root_block_id, true)
}

/// Parse a Markdown FRAGMENT — content grafted into an existing document rather
/// than content that starts one.
///
/// The only difference from [`import_markdown`] is that a leading `---` is a
/// thematic break here, never front matter. Front matter exists exactly once, at
/// the very top of a file; an appended or inserted chunk is by definition not
/// there, so reading it that way is reading a position the text does not have.
///
/// Not a nicety. Appending
///
/// ```text
/// ---
/// body
/// ---
/// tail
/// ```
///
/// used to lift `body` out as YAML and drop it on a throwaway root: the caller
/// got `tail`, no error, and no way to tell what happened. Reported 2026-08-07
/// by an MCP caller who lost the middle of a page and concluded "avoid `---` in
/// Mica" — a rule the format does not have.
pub fn import_markdown_fragment(markdown: &str, root_block_id: &str) -> DocumentSnapshotPayload {
  import_markdown_inner(markdown, root_block_id, false)
}

pub fn block_index(snapshot: &DocumentSnapshotPayload, block_id: &str) -> Option<usize> {
  snapshot
    .blocks
    .iter()
    .position(|block| block.id == block_id)
}

pub fn is_descendant(snapshot: &DocumentSnapshotPayload, block_id: &str, parent_id: &str) -> bool {
  // Iterative with a visited set: `children` comes from a CRDT any member can
  // write, so it can hold a cycle, and the recursive walk never returned.
  let mut seen: HashSet<&str> = HashSet::new();
  let mut stack = vec![parent_id];
  while let Some(id) = stack.pop() {
    if !seen.insert(id) {
      continue;
    }
    let Some(index) = block_index(snapshot, id) else {
      continue;
    };
    for child_id in &snapshot.blocks[index].children {
      if child_id == block_id {
        return true;
      }
      stack.push(child_id);
    }
  }
  false
}

/// Deepest `children` nesting the exporters recurse into. Real documents nest
/// a handful of levels; this is far above that and far below the stack.
const MAX_EXPORT_DEPTH: usize = 128;

/// The block tree as the recursive exporters need it: every block reached
/// once, no deeper than [MAX_EXPORT_DEPTH].
///
/// The snapshot is materialised from a yrs document any workspace member can
/// push to, and nothing on that path checks the tree's SHAPE. A block listing
/// itself (or an ancestor) in `children` sent `export_html` / `export_markdown`
/// into unbounded recursion — a stack overflow, which aborts the whole server
/// process, reachable through the share page, export and MCP reads. So the
/// exporters walk a repaired copy: a repeat visit is dropped (cycles and
/// multi-parent blocks), and a branch past the depth cap is flattened into its
/// ancestor at the cap, keeping the content. A well-formed tree — every real
/// one — is returned as is, without copying.
///
/// Also returns the id→index map this walk already builds: the recursive
/// exporters look up EVERY child by id, and re-scanning `blocks` per lookup is
/// O(B²) on a flat document — passing the map down makes each lookup O(1).
fn export_safe_tree(
  snapshot: &DocumentSnapshotPayload,
) -> (Cow<'_, DocumentSnapshotPayload>, HashMap<&str, usize>) {
  let index: HashMap<&str, usize> = snapshot
    .blocks
    .iter()
    .enumerate()
    .map(|(i, b)| (b.id.as_str(), i))
    .collect();
  let Some(&root) = index.get(snapshot.root_block_id.as_str()) else {
    // The exporter reports the missing root.
    return (Cow::Borrowed(snapshot), index);
  };

  let well_formed = {
    let mut seen: HashSet<&str> = HashSet::from([snapshot.root_block_id.as_str()]);
    let mut stack = vec![(root, 0usize)];
    let mut ok = true;
    'walk: while let Some((i, depth)) = stack.pop() {
      for child in &snapshot.blocks[i].children {
        if !seen.insert(child.as_str()) || depth + 1 > MAX_EXPORT_DEPTH {
          ok = false;
          break 'walk;
        }
        if let Some(&c) = index.get(child.as_str()) {
          stack.push((c, depth + 1));
        }
      }
    }
    ok
  };
  if well_formed {
    return (Cow::Borrowed(snapshot), index);
  }

  let mut children: Vec<Vec<String>> = vec![Vec::new(); snapshot.blocks.len()];
  let mut seen: HashSet<&str> = HashSet::from([snapshot.root_block_id.as_str()]);
  let mut stack = vec![(root, 0usize)];
  while let Some((i, depth)) = stack.pop() {
    for child in &snapshot.blocks[i].children {
      if !seen.insert(child.as_str()) {
        continue;
      }
      let Some(&c) = index.get(child.as_str()) else {
        children[i].push(child.clone()); // missing: the exporter reports it
        continue;
      };
      children[i].push(child.clone());
      if depth + 1 < MAX_EXPORT_DEPTH {
        stack.push((c, depth + 1));
        continue;
      }
      // At the cap: hoist the whole branch up to `i`, in document order
      // (children pushed reversed onto a stack pop in pre-order).
      let mut branch = vec![c];
      while let Some(b) = branch.pop() {
        if b != c {
          children[i].push(snapshot.blocks[b].id.clone());
        }
        for grandchild in snapshot.blocks[b].children.iter().rev() {
          if seen.insert(grandchild.as_str())
            && let Some(&g) = index.get(grandchild.as_str())
          {
            branch.push(g);
          }
        }
      }
    }
  }
  let mut repaired = snapshot.clone();
  for (block, kids) in repaired.blocks.iter_mut().zip(children) {
    block.children = kids;
  }
  (Cow::Owned(repaired), index)
}

#[cfg(test)]
mod export_xss_tests {
  use super::{export_html, export_html_document, import_markdown};

  fn html(md: &str) -> String {
    export_html(&import_markdown(md, "root")).unwrap()
  }

  /// The downloaded HTML has no CSP header, so script-capable URLs and markup
  /// must not survive into it.
  #[test]
  fn script_urls_are_dropped_from_links_and_images() {
    for md in [
      "[x](javascript:alert(1))",
      "[x](JaVaScRiPt:alert(1))",
      "[x](vbscript:msgbox)",
      "[x](data:text/html,<script>alert(1)</script>)",
      "![x](javascript:alert(1))",
    ] {
      let out = html(md).to_ascii_lowercase();
      assert!(
        !out.contains("javascript:") && !out.contains("vbscript:") && !out.contains("data:text"),
        "{md} -> {out}"
      );
    }
    assert!(html("[x](https://e.com/a:b)").contains("href=\"https://e.com/a:b\""));
    assert!(html("[x](a/b:c)").contains("href=\"a/b:c\""));
    assert!(
      html("![x](data:image/png;base64,AAAA)").contains("src=\"data:image/png;base64,AAAA\"")
    );
  }

  /// `--!>` closes a comment in browsers; the attribute stripper treated the
  /// rest as comment and left the handler in place.
  #[test]
  fn a_bang_comment_close_does_not_hide_markup_from_the_stripper() {
    let out = html("<!-- --!><img src=x onerror=alert(2)> -->\n");
    assert!(!out.contains("onerror"), "{out}");
    let out = html("<!--><img src=x onerror=alert(3)>\n");
    assert!(!out.contains("onerror"), "{out}");
  }

  #[test]
  fn the_exported_document_forbids_script() {
    let doc = export_html_document(&import_markdown("hi", "root"), "t", 800).unwrap();
    assert!(doc.contains("Content-Security-Policy") && doc.contains("default-src 'none'"));
    assert!(!doc.contains("script-src"));
  }
}

#[cfg(test)]
mod tree_shape_tests {
  use super::{DocumentSnapshotPayload, export_html, export_markdown, is_descendant};

  fn doc(blocks: serde_json::Value) -> DocumentSnapshotPayload {
    serde_json::from_value(serde_json::json!({
      "schema_version": 1, "root_block_id": "root", "blocks": blocks,
    }))
    .unwrap()
  }

  /// THE BUG: `children` comes from a CRDT any member can write; a block that
  /// lists itself sent the recursive exporters into a stack overflow, which
  /// aborts the whole server process, not one request.
  #[test]
  fn a_cycle_in_children_exports_instead_of_overflowing() {
    let snapshot = doc(serde_json::json!([
      {"id": "root", "type": "page", "text": "", "data": {}, "children": ["a"]},
      {"id": "a", "type": "quote", "text": "loop", "data": {}, "children": ["a", "root"]},
    ]));
    assert!(export_html(&snapshot).unwrap().contains("loop"));
    assert!(export_markdown(&snapshot).unwrap().contains("loop"));
    assert!(
      !is_descendant(&snapshot, "zzz", "a"),
      "terminates on the cycle"
    );
    assert!(is_descendant(&snapshot, "a", "root"));
  }

  /// A chain far deeper than any real document keeps all its text.
  #[test]
  fn a_very_deep_chain_is_flattened_not_lost() {
    let n = 5000;
    let mut blocks = vec![serde_json::json!(
      {"id": "root", "type": "page", "text": "", "data": {}, "children": ["b0"]}
    )];
    for i in 0..n {
      let children = if i + 1 < n {
        vec![format!("b{}", i + 1)]
      } else {
        vec![]
      };
      blocks.push(serde_json::json!(
        {"id": format!("b{i}"), "type": "quote", "text": format!("t{i}"), "data": {}, "children": children}
      ));
    }
    let md = export_markdown(&doc(serde_json::Value::Array(blocks))).unwrap();
    assert!(md.contains("t0") && md.contains(&format!("t{}", n - 1)));
  }
}

#[cfg(test)]
mod page_title_tests {
  use super::{
    DocumentSnapshotPayload, document_title, set_document_title, strip_page_title, with_page_title,
  };

  fn doc(root_data: serde_json::Value) -> DocumentSnapshotPayload {
    serde_json::from_value(serde_json::json!({
      "schema_version": 1,
      "root_block_id": "root",
      "blocks": [
        {"id": "root", "type": "page", "text": "", "data": root_data, "children": ["b1"]},
        {"id": "b1", "type": "paragraph", "text": "正文。", "data": {}, "children": []}
      ]
    }))
    .unwrap()
  }

  #[test]
  fn a_document_without_a_title_reports_none() {
    // The state every existing page is in. There is no backfill, so this is not
    // an edge case — it is the majority, and the caller must fall back to the
    // view's name rather than show nothing.
    assert_eq!(document_title(&doc(serde_json::json!({}))), None);
  }

  #[test]
  fn the_title_is_read_from_the_root_block() {
    assert_eq!(
      document_title(&doc(serde_json::json!({"title": "我的页"}))),
      Some("我的页")
    );
  }

  /// The pair has to be closed: what the setter writes is what the getter reads.
  /// Import is the only caller, and it has no other way to check its own work.
  #[test]
  fn what_the_setter_writes_the_getter_reads_back() {
    let mut page = doc(serde_json::json!({}));
    assert!(set_document_title(&mut page, "季度回顾"));
    assert_eq!(document_title(&page), Some("季度回顾"));
  }

  /// Front matter lives in the SAME map on the SAME block. An import that
  /// clobbered it would silently drop every page property in the archive.
  #[test]
  fn writing_a_title_leaves_front_matter_alone() {
    let mut page = doc(serde_json::json!({"front_matter": "tags: [a]"}));
    assert!(set_document_title(&mut page, "名字"));
    assert_eq!(document_title(&page), Some("名字"));
    assert_eq!(
      page.blocks[0]
        .data
        .get("front_matter")
        .and_then(|v| v.as_str()),
      Some("tags: [a]")
    );
  }

  /// `data` defaults to null (it is skipped when serialising), so the common
  /// case for a freshly parsed root block is not an object at all.
  #[test]
  fn a_root_block_with_no_data_at_all_still_takes_a_title() {
    let mut page = doc(serde_json::json!(null));
    assert!(page.blocks[0].data.is_null(), "precondition");
    assert!(set_document_title(&mut page, "名字"));
    assert_eq!(document_title(&page), Some("名字"));
  }

  /// An archive entry with no usable name must leave the document titleless so
  /// it falls back to the view's column, rather than storing a key that reads
  /// back as nothing. Mirrors `a_blank_title_counts_as_absent` on the read side.
  #[test]
  fn a_blank_title_is_not_written_at_all() {
    let mut page = doc(serde_json::json!({}));
    assert!(!set_document_title(&mut page, "   "));
    assert!(page.blocks[0].data.get("title").is_none());
    assert_eq!(document_title(&page), None);
  }

  #[test]
  fn surrounding_whitespace_is_trimmed_on_the_way_in() {
    let mut page = doc(serde_json::json!({}));
    assert!(set_document_title(&mut page, "  有空格  "));
    assert_eq!(
      page.blocks[0].data.get("title").and_then(|v| v.as_str()),
      Some("有空格")
    );
  }

  /// A blank title must fall back, not render as an empty heading.
  #[test]
  fn a_blank_title_counts_as_absent() {
    assert_eq!(document_title(&doc(serde_json::json!({"title": ""}))), None);
    assert_eq!(
      document_title(&doc(serde_json::json!({"title": "   "}))),
      None
    );
  }

  /// The title sits BESIDE front matter on the same root block; reading one
  /// must not disturb or depend on the other.
  #[test]
  fn a_title_and_front_matter_coexist_on_the_root_block() {
    let d = doc(serde_json::json!({"title": "我的页", "front_matter": "tags: [a]"}));
    assert_eq!(document_title(&d), Some("我的页"));
    let exported = super::export_markdown(&d).unwrap();
    assert!(
      exported.starts_with("---\ntags: [a]\n---"),
      "front matter still leads the export: {exported:?}"
    );
  }

  #[test]
  fn the_title_leads_an_ordinary_page() {
    assert_eq!(
      with_page_title("正文。\n", "我的页"),
      "# 我的页\n\n正文。\n"
    );
  }

  /// The rule that makes this more than a string concat.
  ///
  /// Front matter is only front matter when it is the FIRST thing in the file.
  /// A heading above it turns the opening `---` into a thematic break, the
  /// closing one into another, and the properties into visible text — silently,
  /// in an export the user is keeping as a backup.
  #[test]
  fn the_title_goes_after_front_matter_not_before_it() {
    let md = "---\ntags: [a]\n---\n正文。\n";
    assert_eq!(
      with_page_title(md, "我的页"),
      "---\ntags: [a]\n---\n# 我的页\n\n正文。\n"
    );
  }

  /// Someone who already types their own title must not get two.
  #[test]
  fn an_identical_leading_heading_is_left_alone() {
    let md = "# 我的页\n\n正文。\n";
    assert_eq!(with_page_title(md, "我的页"), md);
  }

  /// …but a DIFFERENT leading heading is the author's content, not a title.
  #[test]
  fn a_different_leading_heading_survives_and_the_title_goes_above_it() {
    let md = "# 别的标题\n\n正文。\n";
    assert_eq!(
      with_page_title(md, "我的页"),
      "# 我的页\n\n# 别的标题\n\n正文。\n"
    );
  }

  /// Level matters: `## 我的页` is not the heading this writes, so it stays and
  /// the real title is added above it. (This is the shape a user who types
  /// their own H2 by hand ends up with — deliberate, not an accident.)
  #[test]
  fn a_same_named_heading_at_another_level_is_not_treated_as_the_title() {
    let md = "## 我的页\n\n正文。\n";
    assert_eq!(
      with_page_title(md, "我的页"),
      "# 我的页\n\n## 我的页\n\n正文。\n"
    );
  }

  #[test]
  fn an_empty_page_gets_just_the_heading() {
    assert_eq!(with_page_title("", "我的页"), "# 我的页\n");
    assert_eq!(with_page_title("\n\n", "我的页"), "# 我的页\n");
  }

  /// An untitled page must not export a bare `# `.
  #[test]
  fn an_empty_title_changes_nothing() {
    assert_eq!(with_page_title("正文。\n", ""), "正文。\n");
    assert_eq!(with_page_title("正文。\n", "   "), "正文。\n");
  }

  /// An unterminated `---` is not front matter. Swallowing the rest of the page
  /// looking for a closing fence is how an export loses a body.
  #[test]
  fn an_unterminated_fence_is_treated_as_text() {
    let md = "---\n没有收尾\n";
    assert_eq!(with_page_title(md, "我的页"), "# 我的页\n\n---\n没有收尾\n");
  }

  /// The pair, closed: whatever the writer adds, the stripper takes back off —
  /// for every shape the writer has a rule for.
  #[test]
  fn stripping_undoes_writing() {
    for (md, title) in [
      ("正文。\n", "我的页"),
      ("---\ntags: [a]\n---\n正文。\n", "我的页"),
      ("---\n---\n正文。\n", "我的页"),
      ("---\ntags: [a]\n---\n", "我的页"),
      ("", "我的页"),
      ("# 别的标题\n\n正文。\n", "我的页"),
      ("---\n没有收尾\n", "我的页"),
      ("正文。\n", "Q&A: 2026/10"),
    ] {
      let titled = with_page_title(md, title);
      assert_eq!(strip_page_title(&titled, title), md, "{titled:?}");
    }
  }

  /// The case that used to leak: the title sits after the front matter, so a
  /// stripper looking only at the file's first line never saw it.
  #[test]
  fn the_title_is_stripped_from_after_front_matter() {
    assert_eq!(
      strip_page_title("---\ntags: [a]\n---\n# 我的页\n\n正文。\n", "我的页"),
      "---\ntags: [a]\n---\n正文。\n"
    );
  }

  /// Exact match only: a different leading heading, a heading at another level,
  /// or a matching heading that is not the first thing in the body is content.
  #[test]
  fn only_the_exact_leading_title_is_stripped() {
    for md in [
      "# 别的标题\n\n正文。\n",
      "## 我的页\n\n正文。\n",
      "正文。\n\n# 我的页\n",
      "---\ntags: [a]\n---\n正文。\n\n# 我的页\n",
    ] {
      assert_eq!(strip_page_title(md, "我的页"), md);
    }
    assert_eq!(strip_page_title("# \n\n正文。\n", ""), "# \n\n正文。\n");
  }
}

#[cfg(test)]
mod security_tests {
  use super::{
    SAFE_HTML_CSP, export_html, export_html_document, import_markdown, strip_unsafe_attrs,
    tagfilter,
  };

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

  /// THE BUG: the filter copied untouched text one BYTE at a time as `char`,
  /// turning every UTF-8 byte into a Latin-1 char — `<summary>点击展开</summary>`
  /// rendered as mojibake on the share page and in HTML export.
  #[test]
  fn tagfilter_keeps_non_ascii_text_intact() {
    let raw = "<details>\n<summary>点击展开 😀</summary>\n<script>x</script>é</details>";
    assert_eq!(
      tagfilter(raw),
      "<details>\n<summary>点击展开 😀</summary>\n&lt;script>x&lt;/script>é</details>"
    );
    let html = export_html(&import_markdown("<p align=\"center\">居中</p>\n", "root")).unwrap();
    assert!(html.contains("居中"), "{html}");
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
    assert!(
      !c.to_ascii_lowercase().contains("onload"),
      "OnLoad survived: {c}"
    );
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
    assert!(
      !b.to_ascii_lowercase().contains("javascript"),
      "js href survived: {b}"
    );

    // Embedded tab in the scheme (browser-normalized bypass).
    let c = strip_unsafe_attrs("<a href=\"java\tscript:x\">x</a>");
    assert!(
      !c.to_ascii_lowercase().contains("script:"),
      "tab-split js href survived: {c}"
    );

    // vbscript: on a URL attribute other than href.
    let d = strip_unsafe_attrs("<img src=\"vbscript:msgbox(1)\">");
    assert!(
      !d.to_ascii_lowercase().contains("vbscript"),
      "vbscript src survived: {d}"
    );
  }

  #[test]
  fn strip_unsafe_attrs_preserves_safe_content() {
    // A real image with a benign URL and an alt that contains "on" as a word.
    let img = strip_unsafe_attrs("<img src=\"x.png\" alt=\"turn on the light\">");
    assert_eq!(img, "<img src=\"x.png\" alt=\"turn on the light\">");

    // A class literally named "on" — the substring must not trip the on* rule.
    assert_eq!(
      strip_unsafe_attrs("<div class=\"on\">"),
      "<div class=\"on\">"
    );

    // Valueless (boolean) attribute survives.
    assert_eq!(strip_unsafe_attrs("<input disabled>"), "<input disabled>");

    // Plain text with `<` that isn't a tag, and `on=` in prose — byte-identical.
    assert_eq!(strip_unsafe_attrs("a < b and on=off"), "a < b and on=off");

    // A comment mentioning onerror is NOT scanned — passes through verbatim.
    assert_eq!(
      strip_unsafe_attrs("<!-- onerror=x -->"),
      "<!-- onerror=x -->"
    );

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
    assert!(
      !html.contains("onerror"),
      "onerror leaked into export: {html}"
    );
    assert!(
      !html.contains("steal("),
      "handler body leaked into export: {html}"
    );
    assert!(
      html.contains("<img"),
      "the img tag itself should survive: {html}"
    );
  }

  // Issue #31: `<meta http-equiv="refresh">` is a client-side redirect NO CSP
  // directive covers, so a raw-HTML one made a trusted share URL bounce to an
  // arbitrary site. The renderer drops the tag outright.
  #[test]
  fn meta_http_equiv_tags_are_dropped() {
    let r =
      strip_unsafe_attrs("<meta http-equiv=\"refresh\" content=\"0;url=https://evil.example\">");
    assert!(r.is_empty(), "refresh meta survived: {r}");

    // Case-insensitive in BOTH tag and attribute name — browsers are.
    let m = strip_unsafe_attrs("<META HTTP-EQUIV=Refresh CONTENT=\"0; URL=//evil\">");
    assert!(m.is_empty(), "mixed-case refresh meta survived: {m}");

    // Unquoted / valueless http-equiv is still http-equiv.
    let u = strip_unsafe_attrs("<meta http-equiv>");
    assert!(u.is_empty(), "valueless http-equiv meta survived: {u}");

    // A meta WITHOUT http-equiv is inert in body content — kept byte-for-byte.
    assert_eq!(
      strip_unsafe_attrs("<meta name=\"description\" content=\"笔记\">"),
      "<meta name=\"description\" content=\"笔记\">"
    );

    // Dropping must not swallow the document around it.
    assert_eq!(
      strip_unsafe_attrs("前<meta http-equiv=\"refresh\" content=\"0;url=x\">后"),
      "前后"
    );
  }

  #[test]
  fn meta_refresh_gone_from_the_full_export_pipeline() {
    let md =
      "<div>\n<meta http-equiv=\"refresh\" content=\"0;url=https://evil.example\">\n</div>\n";
    let snapshot = import_markdown(md, "root");
    let html = export_html(&snapshot).expect("export_html");
    assert!(
      !html.to_ascii_lowercase().contains("http-equiv"),
      "meta refresh leaked into export: {html}"
    );
    assert!(
      !html.contains("evil.example"),
      "redirect target leaked into export: {html}"
    );
  }

  // The constant the share page sends as a header and the export embeds as a
  // <meta> is ONE string — pin the directives both surfaces must carry, and
  // that the export actually embeds it.
  #[test]
  fn shared_csp_covers_form_action_and_base_uri() {
    assert!(
      SAFE_HTML_CSP.contains("form-action 'none'"),
      "{SAFE_HTML_CSP}"
    );
    assert!(SAFE_HTML_CSP.contains("base-uri 'none'"), "{SAFE_HTML_CSP}");
    assert!(
      SAFE_HTML_CSP.contains("default-src 'none'"),
      "{SAFE_HTML_CSP}"
    );
    assert!(!SAFE_HTML_CSP.contains("script-src"), "{SAFE_HTML_CSP}");

    let doc = export_html_document(&import_markdown("正文。\n", "root"), "标题", 800)
      .expect("export_html_document");
    assert!(
      doc.contains(&format!(
        "http-equiv=\"Content-Security-Policy\" content=\"{SAFE_HTML_CSP}\""
      )),
      "the export must embed the shared policy verbatim: {doc}"
    );
  }
}

mod block;
mod entities;
mod export_md;
mod html;
mod inline;
mod latex_guard;
mod scan_memo;
mod span_walk;

pub use inline::math_run_spans;
pub use latex_guard::{JSON_ESCAPE_COLLISIONS, mangled_latex_collision, mangled_latex_message};

use block::*;
use export_md::*;
use html::*;
