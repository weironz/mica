//! Mica's Markdown adapter over the `mica-md` engine (crates/md-core): page
//! titles, the standalone HTML export, MCP text editing and the LaTeX guard,
//! plus a compatibility re-export of every engine item this crate used to
//! define — so app-core / api-server / interchange / mcp-server / mica-core /
//! the FFI crate compile unchanged. New code should depend on `mica-md`.

use std::collections::BTreeMap;

use serde_json::Value;

pub use mica_md::properties;
pub use mica_md::render::{MermaidHostTheme, render_mermaid_svg, render_mermaid_svg_with_id};
pub use mica_md::{
  Block, DocumentSnapshotPayload, import_markdown, import_markdown_fragment, math_run_spans,
  set_image_srcs,
};

mod latex_guard;
mod text_edit;

pub use latex_guard::{JSON_ESCAPE_COLLISIONS, mangled_latex_collision, mangled_latex_message};
pub use text_edit::{
  block_text_matches, block_text_preview, markdown_find_text, replace_block_text,
};

/// A document operation error.
#[derive(Debug, thiserror::Error)]
pub enum DocumentOperationError {
  /// The document's schema version is not supported.
  #[error("unsupported document schema version: {0}")]
  UnsupportedSchemaVersion(i32),

  /// A block referenced by id is not in the document.
  #[error("block not found: {0}")]
  BlockNotFound(String),

  /// A block id already exists in the document.
  #[error("block already exists: {0}")]
  BlockAlreadyExists(String),

  /// The root block cannot be deleted or moved.
  #[error("root block cannot be deleted or moved")]
  CannotMoveRoot,

  /// A parent would become a descendant of the moved block.
  #[error("parent block cannot be a descendant of the moved block")]
  ParentIsDescendant,

  /// A block type is required.
  #[error("block type is required")]
  EmptyBlockType,

  /// A block id is required.
  #[error("block id is required")]
  EmptyBlockId,

  /// A newly inserted block must be a leaf.
  #[error(
    "a newly inserted block must be a leaf (empty children); attach children with their own inserts"
  )]
  InsertBlockWithChildren,
}

impl From<mica_md::MdError> for DocumentOperationError {
  fn from(error: mica_md::MdError) -> Self {
    match error {
      mica_md::MdError::UnsupportedSchemaVersion(v) => Self::UnsupportedSchemaVersion(v),
      mica_md::MdError::BlockNotFound(id) => Self::BlockNotFound(id),
      // The facade never parses wire input; keep the historical mapping so the
      // error space stays total.
      mica_md::MdError::InvalidInput(_) => Self::UnsupportedSchemaVersion(0),
    }
  }
}

/// A document operation result.
pub type DocumentOperationResult<T> = Result<T, DocumentOperationError>;

/// Deserialize a payload while preserving the original schema-version errors.
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

/// Serialize a document to Markdown.
pub fn export_markdown(snapshot: &DocumentSnapshotPayload) -> DocumentOperationResult<String> {
  export_markdown_with_assets(snapshot, &BTreeMap::new())
}

/// Serialize a document to Markdown, rewriting image destinations from `images`.
pub fn export_markdown_with_assets(
  snapshot: &DocumentSnapshotPayload,
  images: &BTreeMap<String, String>,
) -> DocumentOperationResult<String> {
  mica_md::export_markdown_with_assets(snapshot, images).map_err(DocumentOperationError::from)
}

/// Render a document to its HTML body fragment.
pub fn export_html(snapshot: &DocumentSnapshotPayload) -> DocumentOperationResult<String> {
  mica_md::export_html(snapshot).map_err(DocumentOperationError::from)
}

/// The document's own title, if it carries one.
pub fn document_title(snapshot: &DocumentSnapshotPayload) -> Option<&str> {
  let root = block_index(snapshot, &snapshot.root_block_id).map(|i| &snapshot.blocks[i])?;
  root
    .data
    .get("title")
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|title| !title.is_empty())
}

/// Put the page's title on the root block of a payload about to become a
/// document. Returns whether the title was written.
pub fn set_document_title(snapshot: &mut DocumentSnapshotPayload, title: &str) -> bool {
  let title = title.trim();
  if title.is_empty() {
    return false;
  }
  let Some(index) = block_index(snapshot, &snapshot.root_block_id) else {
    return false;
  };
  let data = &mut snapshot.blocks[index].data;
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
pub fn with_page_title(markdown: &str, title: &str) -> String {
  let title = title.trim();
  if title.is_empty() {
    return markdown.to_string();
  }
  let heading = format!("# {title}");
  let (front, body) = mica_md::split_front_matter(markdown);
  if leading_heading(body).is_some_and(|line| is_page_title_line(line, title)) {
    return markdown.to_string();
  }
  let body = body.trim_start_matches('\n');
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

/// The inverse of [`with_page_title`]: remove the `# {title}` line it wrote.
pub fn strip_page_title(markdown: &str, title: &str) -> String {
  let title = title.trim();
  if title.is_empty() {
    return markdown.to_string();
  }
  let (_, body) = mica_md::split_front_matter(markdown);
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

/// Whether `line` is the heading [`with_page_title`] writes for `title`.
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

/// Position of a block id in the flat block table.
pub fn block_index(snapshot: &DocumentSnapshotPayload, block_id: &str) -> Option<usize> {
  snapshot
    .blocks
    .iter()
    .position(|block| block.id == block_id)
}

/// Whether `block_id` is a descendant of `parent_id`.
pub fn is_descendant(snapshot: &DocumentSnapshotPayload, block_id: &str, parent_id: &str) -> bool {
  let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
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

/// The one Content Security Policy for every static page Mica renders.
pub const SAFE_HTML_CSP: &str = "default-src 'none'; img-src 'self' data: https: http:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; form-action 'none'; base-uri 'none'";

/// Wrap the [`export_html`] fragment in a standalone, self-contained HTML5
/// document.
pub fn export_html_document(
  snapshot: &DocumentSnapshotPayload,
  title: &str,
  content_width_px: u32,
) -> DocumentOperationResult<String> {
  let body = export_html(snapshot)?;
  let safe_title = escape_html(title.trim());
  let width = content_width_px.clamp(640, 2400);
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

fn escape_html(text: &str) -> String {
  let mut out = String::with_capacity(text.len());
  for ch in text.chars() {
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
    assert_eq!(document_title(&doc(serde_json::json!({}))), None);
  }

  #[test]
  fn the_title_is_read_from_the_root_block() {
    assert_eq!(
      document_title(&doc(serde_json::json!({"title": "我的页"}))),
      Some("我的页")
    );
  }

  #[test]
  fn what_the_setter_writes_the_getter_reads_back() {
    let mut page = doc(serde_json::json!({}));
    assert!(set_document_title(&mut page, "季度回顾"));
    assert_eq!(document_title(&page), Some("季度回顾"));
  }

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

  #[test]
  fn a_root_block_with_no_data_at_all_still_takes_a_title() {
    let mut page = doc(serde_json::json!(null));
    assert!(page.blocks[0].data.is_null(), "precondition");
    assert!(set_document_title(&mut page, "名字"));
    assert_eq!(document_title(&page), Some("名字"));
  }

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

  #[test]
  fn a_blank_title_counts_as_absent() {
    assert_eq!(document_title(&doc(serde_json::json!({"title": ""}))), None);
    assert_eq!(
      document_title(&doc(serde_json::json!({"title": "   "}))),
      None
    );
  }

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

  #[test]
  fn the_title_goes_after_front_matter_not_before_it() {
    let md = "---\ntags: [a]\n---\n正文。\n";
    assert_eq!(
      with_page_title(md, "我的页"),
      "---\ntags: [a]\n---\n# 我的页\n\n正文。\n"
    );
  }

  #[test]
  fn an_identical_leading_heading_is_left_alone() {
    let md = "# 我的页\n\n正文。\n";
    assert_eq!(with_page_title(md, "我的页"), md);
  }

  #[test]
  fn a_different_leading_heading_survives_and_the_title_goes_above_it() {
    let md = "# 别的标题\n\n正文。\n";
    assert_eq!(
      with_page_title(md, "我的页"),
      "# 我的页\n\n# 别的标题\n\n正文。\n"
    );
  }

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

  #[test]
  fn an_empty_title_changes_nothing() {
    assert_eq!(with_page_title("正文。\n", ""), "正文。\n");
    assert_eq!(with_page_title("正文。\n", "   "), "正文。\n");
  }

  #[test]
  fn an_unterminated_fence_is_treated_as_text() {
    let md = "---\n没有收尾\n";
    assert_eq!(with_page_title(md, "我的页"), "# 我的页\n\n---\n没有收尾\n");
  }

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

  #[test]
  fn the_title_is_stripped_from_after_front_matter() {
    assert_eq!(
      strip_page_title("---\ntags: [a]\n---\n# 我的页\n\n正文。\n", "我的页"),
      "---\ntags: [a]\n---\n正文。\n"
    );
  }

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
mod facade_security_tests {
  use super::{SAFE_HTML_CSP, export_html, export_html_document, import_markdown, is_descendant};

  #[test]
  fn the_exported_document_forbids_script() {
    let doc = export_html_document(&import_markdown("hi", "root"), "t", 800).unwrap();
    assert!(doc.contains("Content-Security-Policy") && doc.contains("default-src 'none'"));
    assert!(!doc.contains("script-src"));
  }

  #[test]
  fn descendant_walk_terminates_on_cycles() {
    let snapshot = serde_json::from_value(serde_json::json!({
      "schema_version": 1, "root_block_id": "root", "blocks": [
        {"id": "root", "type": "page", "text": "", "data": {}, "children": ["a"]},
        {"id": "a", "type": "quote", "text": "loop", "data": {}, "children": ["a", "root"]}
      ]
    }))
    .unwrap();
    assert!(
      !is_descendant(&snapshot, "zzz", "a"),
      "terminates on the cycle"
    );
    assert!(is_descendant(&snapshot, "a", "root"));
  }

  #[test]
  fn export_html_strips_event_handler_from_raw_block() {
    let md = "<div>\n<img src=x onerror=\"steal(document.cookie)\">\n</div>\n";
    let snapshot = import_markdown(md, "root");
    let html = export_html(&snapshot).expect("export_html");
    assert!(
      !html.contains("onerror"),
      "onerror leaked into export: {html}"
    );
    assert!(!html.contains("steal("), "handler body leaked: {html}");
    assert!(
      html.contains("<img"),
      "the img tag itself should survive: {html}"
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
    assert!(!html.contains("evil.example"), "redirect target leaked");
  }

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
