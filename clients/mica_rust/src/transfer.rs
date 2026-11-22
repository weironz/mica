//! Pure-Rust document & workspace import/export utilities.
//!
//! Compiles cleanly on both desktop (`#[cfg(not(target_arch = "wasm32"))]`,
//! writing directly to `<Downloads | Desktop | temp>/mica-exports/`) and
//! WebAssembly (`#[cfg(target_arch = "wasm32")]`, returning structured summaries).

use chrono::Utc;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// Schema identifier embedded in exported `.mica.json` workspace bundles.
pub const WORKSPACE_BUNDLE_FORMAT: &str = "mica-workspace-bundle-v1";

const DEFAULT_DOC_FALLBACK: &str = "untitled";
const DEFAULT_WORKSPACE_FALLBACK: &str = "mica-workspace";
const MAX_FILENAME_CHARS: usize = 80;

/// Single page entry inside a [`WorkspaceBundle`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBundlePage {
  pub title: String,
  pub markdown: String,
}

/// Portable JSON archive representing all pages in a Mica workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBundle {
  pub format: String,
  pub workspace_name: String,
  pub exported_at: String,
  pub page_count: usize,
  pub pages: Vec<WorkspaceBundlePage>,
}

/// Sanitize a document or workspace title into a cross-platform safe filename
/// stem (no path separators, traversal segments, control characters, trailing
/// dots, or Windows reserved device names).
pub fn sanitize_filename(raw: &str, fallback: &str) -> String {
  let mut cleaned = String::with_capacity(raw.len().min(MAX_FILENAME_CHARS));
  let mut prev_sep = false;

  for ch in raw.trim().chars() {
    let is_forbidden = matches!(
      ch,
      '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'
    ) || ch.is_control();

    if is_forbidden {
      if !prev_sep && !cleaned.is_empty() {
        cleaned.push('-');
        prev_sep = true;
      }
      continue;
    }

    if ch.is_whitespace() {
      if !prev_sep && !cleaned.is_empty() {
        cleaned.push(' ');
        prev_sep = true;
      }
      continue;
    }

    cleaned.push(ch);
    prev_sep = false;
  }

  // Collapse any path-traversal dot runs (`..`) into a single dash.
  while cleaned.contains("..") {
    cleaned = cleaned.replace("..", "-");
  }

  let trimmed = cleaned.trim_matches(|c: char| c.is_whitespace() || c == '.' || c == '-');
  let truncated: String = trimmed.chars().take(MAX_FILENAME_CHARS).collect();
  let final_stem = truncated.trim_matches(|c: char| c.is_whitespace() || c == '.' || c == '-');

  if final_stem.is_empty() {
    return fallback.to_string();
  }

  if is_windows_reserved_name(final_stem) {
    format!("_{final_stem}")
  } else {
    final_stem.to_string()
  }
}

fn is_windows_reserved_name(stem: &str) -> bool {
  let upper = stem.to_ascii_uppercase();
  matches!(
    upper.as_str(),
    "CON"
      | "PRN"
      | "AUX"
      | "NUL"
      | "COM1"
      | "COM2"
      | "COM3"
      | "COM4"
      | "COM5"
      | "COM6"
      | "COM7"
      | "COM8"
      | "COM9"
      | "LPT1"
      | "LPT2"
      | "LPT3"
      | "LPT4"
      | "LPT5"
      | "LPT6"
      | "LPT7"
      | "LPT8"
      | "LPT9"
  )
}

fn escape_html_text(input: &str) -> String {
  let mut out = String::with_capacity(input.len());
  for ch in input.chars() {
    match ch {
      '&' => out.push_str("&amp;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      '"' => out.push_str("&quot;"),
      '\'' => out.push_str("&#39;"),
      _ => out.push(ch),
    }
  }
  out
}

/// Wrap an HTML fragment in a standalone warm-paper HTML5 document using
/// Mica's Notion-inspired `#FFFFFF` / `#37352F` typography.
pub fn wrap_standalone_html(title: &str, html_body: &str) -> String {
  let display_title = if title.trim().is_empty() {
    "Untitled"
  } else {
    title.trim()
  };
  let safe_title = escape_html_text(display_title);

  format!(
    r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{safe_title}</title>
  <style>
    :root {{ color-scheme: light; }}
    * {{ box-sizing: border-box; }}
    html, body {{
      margin: 0;
      padding: 0;
      background: #FFFFFF;
      color: #37352F;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
      font-size: 16px;
      line-height: 1.7;
      -webkit-font-smoothing: antialiased;
    }}
    main.mica-doc {{
      max-width: 740px;
      margin: 48px auto;
      padding: 0 24px 72px;
    }}
    h1, h2, h3, h4 {{
      color: #37352F;
      font-weight: 600;
      line-height: 1.3;
      margin: 1.5em 0 0.5em;
    }}
    h1 {{ font-size: 1.9rem; margin-top: 0; }}
    h2 {{ font-size: 1.45rem; }}
    h3 {{ font-size: 1.18rem; }}
    p {{ margin: 0.65em 0; }}
    a {{ color: #2563EB; text-decoration: none; }}
    a:hover {{ text-decoration: underline; }}
    blockquote {{
      margin: 1em 0;
      padding: 0.25em 0 0.25em 1em;
      border-left: 3px solid #D3D2CE;
      color: #5F5E5B;
    }}
    code {{
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 0.9em;
      background: #F7F6F3;
      padding: 0.15em 0.35em;
      border-radius: 4px;
    }}
    pre {{
      background: #F7F6F3;
      border: 1px solid #E9E9E7;
      border-radius: 8px;
      padding: 14px 16px;
      overflow-x: auto;
    }}
    pre code {{ background: transparent; padding: 0; }}
    hr {{ border: 0; border-top: 1px solid #E9E9E7; margin: 1.5em 0; }}
    table {{
      width: 100%;
      border-collapse: collapse;
      margin: 1em 0;
      font-size: 0.94rem;
    }}
    th, td {{
      border: 1px solid #E9E9E7;
      padding: 8px 12px;
      text-align: left;
    }}
    th {{ background: #F7F7F5; font-weight: 600; }}
    img {{ max-width: 100%; border-radius: 6px; }}
  </style>
</head>
<body>
  <main class="mica-doc">
{html_body}
  </main>
</body>
</html>
"#
  )
}

/// Build a structured [`WorkspaceBundle`] payload for export or testing.
pub fn build_workspace_bundle(workspace_name: &str, pages: &[(String, String)]) -> WorkspaceBundle {
  let ws_name = if workspace_name.trim().is_empty() {
    "Mica Workspace".to_string()
  } else {
    workspace_name.trim().to_string()
  };
  let bundle_pages: Vec<WorkspaceBundlePage> = pages
    .iter()
    .map(|(title, markdown)| WorkspaceBundlePage {
      title: title.clone(),
      markdown: markdown.clone(),
    })
    .collect();
  WorkspaceBundle {
    format: WORKSPACE_BUNDLE_FORMAT.to_string(),
    workspace_name: ws_name,
    exported_at: Utc::now().to_rfc3339(),
    page_count: bundle_pages.len(),
    pages: bundle_pages,
  }
}

/// Serialize all workspace pages into a pretty-printed `mica-workspace-bundle-v1`
/// JSON string.
pub fn serialize_workspace_bundle(
  workspace_name: &str,
  pages: &[(String, String)],
) -> Result<String, String> {
  let bundle = build_workspace_bundle(workspace_name, pages);
  serde_json::to_string_pretty(&bundle)
    .map_err(|e| format!("failed to serialize workspace bundle: {e}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn default_export_dir() -> PathBuf {
  dirs_next::download_dir()
    .or_else(dirs_next::desktop_dir)
    .unwrap_or_else(std::env::temp_dir)
    .join("mica-exports")
}

#[cfg(not(target_arch = "wasm32"))]
fn write_export_file(dir: &Path, filename: &str, content: &str) -> Result<String, String> {
  std::fs::create_dir_all(dir)
    .map_err(|e| format!("failed to create export directory {}: {e}", dir.display()))?;
  let target = dir.join(filename);
  std::fs::write(&target, content)
    .map_err(|e| format!("failed to write export file {}: {e}", target.display()))?;
  Ok(target.display().to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn export_markdown_to_dir(dir: &Path, title: &str, markdown: &str) -> Result<String, String> {
  let stem = sanitize_filename(title, DEFAULT_DOC_FALLBACK);
  write_export_file(dir, &format!("{stem}.md"), markdown)
}

#[cfg(not(target_arch = "wasm32"))]
fn export_html_to_dir(dir: &Path, title: &str, html_body: &str) -> Result<String, String> {
  let stem = sanitize_filename(title, DEFAULT_DOC_FALLBACK);
  let html_doc = wrap_standalone_html(title, html_body);
  write_export_file(dir, &format!("{stem}.html"), &html_doc)
}

#[cfg(not(target_arch = "wasm32"))]
fn export_workspace_bundle_to_dir(
  dir: &Path,
  workspace_name: &str,
  pages: &[(String, String)],
) -> Result<String, String> {
  let stem = sanitize_filename(workspace_name, DEFAULT_WORKSPACE_FALLBACK);
  let json = serialize_workspace_bundle(workspace_name, pages)?;
  write_export_file(dir, &format!("{stem}.mica.json"), &json)
}

/// Export a Markdown document to `<Downloads|Desktop|temp>/mica-exports/<safe_title>.md`
/// on desktop and return the written file path, or return a summary string on `wasm32`.
pub fn export_markdown_file(title: &str, markdown: &str) -> Result<String, String> {
  #[cfg(not(target_arch = "wasm32"))]
  {
    export_markdown_to_dir(&default_export_dir(), title, markdown)
  }
  #[cfg(target_arch = "wasm32")]
  {
    let stem = sanitize_filename(title, DEFAULT_DOC_FALLBACK);
    Ok(format!("{stem}.md（{} 字节，Web 端预览导出）", markdown.len()))
  }
}

/// Wrap `html_body` in a standalone warm-paper HTML5 document (`#FFFFFF` /
/// `#37352F`), write it to `<export_dir>/<safe_title>.html` on desktop, and
/// return the written path (or a summary string on `wasm32`).
pub fn export_html_file(title: &str, html_body: &str) -> Result<String, String> {
  #[cfg(not(target_arch = "wasm32"))]
  {
    export_html_to_dir(&default_export_dir(), title, html_body)
  }
  #[cfg(target_arch = "wasm32")]
  {
    let stem = sanitize_filename(title, DEFAULT_DOC_FALLBACK);
    let doc = wrap_standalone_html(title, html_body);
    Ok(format!("{stem}.html（{} 字节，Web 端预览导出）", doc.len()))
  }
}

/// Export all workspace pages as a `mica-workspace-bundle-v1` JSON file to
/// `<export_dir>/<safe_ws>.mica.json` on desktop, returning the written path
/// (or a summary string on `wasm32`).
pub fn export_workspace_bundle(
  workspace_name: &str,
  pages: &[(String, String)],
) -> Result<String, String> {
  #[cfg(not(target_arch = "wasm32"))]
  {
    export_workspace_bundle_to_dir(&default_export_dir(), workspace_name, pages)
  }
  #[cfg(target_arch = "wasm32")]
  {
    let stem = sanitize_filename(workspace_name, DEFAULT_WORKSPACE_FALLBACK);
    let json = serialize_workspace_bundle(workspace_name, pages)?;
    Ok(format!(
      "{stem}.mica.json（{} 篇页面，{} 字节，Web 端预览导出）",
      pages.len(),
      json.len()
    ))
  }
}

/// Read a UTF-8 `.md` file from disk on desktop and return
/// `(title_from_filename, markdown_content)`.
pub fn import_markdown_from_path(path: &str) -> Result<(String, String), String> {
  let trimmed = strip_surrounding_quotes(path.trim());
  if trimmed.is_empty() {
    return Err("文件路径不能为空".to_string());
  }

  #[cfg(not(target_arch = "wasm32"))]
  {
    let file_path = Path::new(trimmed);
    if !file_path.exists() {
      return Err(format!("文件不存在：{}", file_path.display()));
    }
    if !file_path.is_file() {
      return Err(format!("目标不是文件：{}", file_path.display()));
    }
    let bytes = std::fs::read(file_path)
      .map_err(|e| format!("读取文件失败 {}：{e}", file_path.display()))?;
    let raw = String::from_utf8(bytes)
      .map_err(|e| format!("文件不是有效的 UTF-8 编码 {}：{e}", file_path.display()))?;
    let content = raw.strip_prefix('\u{feff}').unwrap_or(&raw).to_string();
    let title = file_path
      .file_stem()
      .and_then(|s| s.to_str())
      .map(str::trim)
      .filter(|s| !s.is_empty())
      .unwrap_or("导入文档")
      .to_string();
    Ok((title, content))
  }
  #[cfg(target_arch = "wasm32")]
  {
    let _ = trimmed;
    Err("Web 端暂不支持直接读取本地文件路径".to_string())
  }
}

fn strip_surrounding_quotes(s: &str) -> &str {
  if s.len() >= 2
    && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
  {
    &s[1..s.len() - 1]
  } else {
    s
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[cfg(not(target_arch = "wasm32"))]
  fn temp_test_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("mica-transfer-{tag}-{}", uuid::Uuid::new_v4()))
  }

  #[test]
  fn filename_sanitization_strips_forbidden_and_traversal() {
    assert_eq!(sanitize_filename("  欢迎使用 Mica  ", "untitled"), "欢迎使用 Mica");
    assert_eq!(
      sanitize_filename("Project: Q1/Q2 <Draft>*?", "untitled"),
      "Project-Q1-Q2 Draft"
    );
    assert_eq!(sanitize_filename("../../etc/passwd", "untitled"), "etc-passwd");
    assert_eq!(sanitize_filename("...///...", "untitled"), "untitled");
    assert_eq!(sanitize_filename("CON", "untitled"), "_CON");
    assert_eq!(sanitize_filename("   ", "fallback"), "fallback");
  }

  #[test]
  fn standalone_html_wraps_warm_paper_document_and_escapes_title() {
    let html = wrap_standalone_html("A <B> & \"C\"", "<h1>Hello</h1><p>Body</p>");
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<title>A &lt;B&gt; &amp; &quot;C&quot;</title>"));
    assert!(html.contains("background: #FFFFFF;"));
    assert!(html.contains("color: #37352F;"));
    assert!(html.contains("<h1>Hello</h1><p>Body</p>"));
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn markdown_export_and_import_round_trip() {
    let dir = temp_test_dir("md");
    let body = "# 架构记录\n\n纯 Rust 导出与导入测试。\n\n- 条目 1\n- 条目 2\n";
    let written = export_markdown_to_dir(&dir, "架构记录: v1/2", body).expect("export md");
    assert!(written.ends_with(".md"));

    let (title, imported) = import_markdown_from_path(&written).expect("import md");
    assert_eq!(title, "架构记录-v1-2");
    assert_eq!(imported, body);

    // Quoted path & validation cases.
    let quoted = format!("\"{written}\"");
    let (quoted_title, quoted_body) = import_markdown_from_path(&quoted).expect("quoted import");
    assert_eq!(quoted_title, title);
    assert_eq!(quoted_body, body);
    assert!(import_markdown_from_path("   ").is_err());

    // Invalid UTF-8 file should be rejected cleanly.
    let bad_path = dir.join("invalid.md");
    std::fs::write(&bad_path, [0xffu8, 0xfeu8, 0x00u8]).expect("write invalid bytes");
    assert!(import_markdown_from_path(&bad_path.display().to_string()).is_err());

    let _ = std::fs::remove_dir_all(&dir);
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn html_and_workspace_bundle_export_round_trip() {
    let dir = temp_test_dir("bundle");

    let html_path =
      export_html_to_dir(&dir, "会议纪要", "<h1>会议纪要</h1><p>内容</p>").expect("export html");
    let html_raw = std::fs::read_to_string(&html_path).expect("read html");
    assert!(html_raw.contains("<title>会议纪要</title>"));
    assert!(html_raw.contains("<h1>会议纪要</h1>"));

    let pages = vec![
      ("首页".to_string(), "# 首页\n\n欢迎".to_string()),
      ("路线图".to_string(), "- M3 完成".to_string()),
    ];
    let bundle_path =
      export_workspace_bundle_to_dir(&dir, "Mica 本地工作区", &pages).expect("export bundle");
    assert!(bundle_path.ends_with(".mica.json"));

    let raw_json = std::fs::read_to_string(&bundle_path).expect("read bundle");
    let parsed: WorkspaceBundle = serde_json::from_str(&raw_json).expect("parse bundle json");
    assert_eq!(parsed.format, WORKSPACE_BUNDLE_FORMAT);
    assert_eq!(parsed.workspace_name, "Mica 本地工作区");
    assert_eq!(parsed.page_count, 2);
    assert_eq!(parsed.pages.len(), 2);
    assert_eq!(parsed.pages[0].title, "首页");
    assert_eq!(parsed.pages[1].markdown, "- M3 完成");

    let _ = std::fs::remove_dir_all(&dir);
  }
}
