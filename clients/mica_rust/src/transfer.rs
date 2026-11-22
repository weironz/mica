//! Pure-Rust document & workspace import/export utilities.
//!
//! Compiles cleanly on both desktop (`#[cfg(not(target_arch = "wasm32"))]`,
//! supporting native OS open/save dialogs via `nsh_core::prelude::{pick_paths, pick_save_path}`
//! with fallback to `<Downloads | Desktop | temp>/mica-exports/`) and
//! WebAssembly (`#[cfg(target_arch = "wasm32")]`, returning structured summaries).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use mica_api_types::View;
#[cfg(not(target_arch = "wasm32"))]
use mica_interchange::ZipFileEntry;
use mica_interchange::{
  ImportMode, ImportPlan, TreeNode, build_markdown_only_tree_zip,
  build_markdown_tree_zip, build_zip, normalize_archive, plan_import, plan_vault_import, read_zip,
};
use mica_md::{DocumentSnapshotPayload, ParseOptions, parse as parse_markdown};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

/// ZIP export archive layout produced by `mica-interchange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveExportFormat {
  /// Full Mica backup (`.mica.zip`) with `manifest.json` + `.mica.json` sidecars + `.md`.
  BackupMicaZip,
  /// Portable Markdown-only ZIP (`.zip`) without `.mica.json` sidecars.
  MarkdownZip,
}

/// Normalized page specification produced by ZIP archive or local folder import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedPageSpec {
  pub title: String,
  pub markdown: String,
  pub folder_path: Vec<String>,
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

/// Alias for [`wrap_standalone_html`].
pub fn build_standalone_html(title: &str, html_body: &str) -> String {
  wrap_standalone_html(title, html_body)
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

// ── Real `mica-interchange` ZIP Export & Import ──────────────────────────────

fn views_to_tree_nodes(views: &[View]) -> Vec<TreeNode> {
  views
    .iter()
    .filter(|v| !v.is_deleted)
    .map(|v| TreeNode {
      id: v.id.to_string(),
      parent_id: v.parent_view_id.map(|p| p.to_string()),
      position: v.position.clone(),
      name: v.name.clone(),
      object_type: if v.object_type == "folder" {
        "folder".to_string()
      } else {
        "document".to_string()
      },
      object_id: v.object_id.to_string(),
      icon: v.icon.clone(),
    })
    .collect()
}

fn docs_to_payloads(
  views: &[View],
  docs: &HashMap<Uuid, String>,
) -> HashMap<String, DocumentSnapshotPayload> {
  let opts = ParseOptions::default();
  let mut payloads = HashMap::new();
  for v in views {
    if v.is_deleted || v.object_type == "folder" {
      continue;
    }
    let key = v.object_id.to_string();
    if payloads.contains_key(&key) {
      continue;
    }
    let md = docs.get(&v.object_id).map(String::as_str).unwrap_or("");
    payloads.insert(key, parse_markdown(md, &opts));
  }
  payloads
}

/// Build an in-memory ZIP archive for the entire workspace using `mica-interchange`.
///
/// - [`ArchiveExportFormat::BackupMicaZip`] writes `manifest.json` + `.mica.json` sidecars + `.md`.
/// - [`ArchiveExportFormat::MarkdownZip`] writes `manifest.json` + `.md` (portable Markdown export).
pub fn build_workspace_zip_bytes(
  _ws_name: &str,
  views: &[View],
  docs: &HashMap<Uuid, String>,
  format: ArchiveExportFormat,
) -> Result<Vec<u8>, String> {
  let nodes = views_to_tree_nodes(views);
  let payloads = docs_to_payloads(views, docs);
  let images = HashMap::new();
  let entries = match format {
    ArchiveExportFormat::BackupMicaZip => build_markdown_tree_zip(&nodes, None, &payloads, &images),
    ArchiveExportFormat::MarkdownZip => {
      build_markdown_only_tree_zip(&nodes, None, &payloads, &images)
    }
  };
  Ok(build_zip(&entries))
}

/// Build an in-memory ZIP archive for a single folder subtree or single document view.
pub fn build_subtree_zip_bytes(
  root_id: Uuid,
  views: &[View],
  docs: &HashMap<Uuid, String>,
  format: ArchiveExportFormat,
) -> Result<Vec<u8>, String> {
  let target = views
    .iter()
    .find(|v| !v.is_deleted && v.id == root_id)
    .ok_or_else(|| format!("未找到要导出的节点：{root_id}"))?;

  let payloads = docs_to_payloads(views, docs);
  let images = HashMap::new();

  let entries = if target.object_type == "folder" {
    let nodes = views_to_tree_nodes(views);
    let root_str = root_id.to_string();
    match format {
      ArchiveExportFormat::BackupMicaZip => {
        build_markdown_tree_zip(&nodes, Some(&root_str), &payloads, &images)
      }
      ArchiveExportFormat::MarkdownZip => {
        build_markdown_only_tree_zip(&nodes, Some(&root_str), &payloads, &images)
      }
    }
  } else {
    let single = vec![TreeNode {
      id: target.id.to_string(),
      parent_id: None,
      position: target.position.clone(),
      name: target.name.clone(),
      object_type: "document".to_string(),
      object_id: target.object_id.to_string(),
      icon: target.icon.clone(),
    }];
    match format {
      ArchiveExportFormat::BackupMicaZip => {
        build_markdown_tree_zip(&single, None, &payloads, &images)
      }
      ArchiveExportFormat::MarkdownZip => {
        build_markdown_only_tree_zip(&single, None, &payloads, &images)
      }
    }
  };

  Ok(build_zip(&entries))
}

fn plan_to_imported_pages(plan: &ImportPlan) -> Vec<ImportedPageSpec> {
  let mut out = Vec::new();
  for page in &plan.pages {
    if page.is_folder {
      continue;
    }
    let mut folder_path = Vec::new();
    let mut cursor = page.parent;
    let mut depth_guard = 0usize;
    while let Some(pidx) = cursor {
      if depth_guard >= plan.pages.len() {
        break;
      }
      depth_guard += 1;
      let Some(parent) = plan.pages.get(pidx) else {
        break;
      };
      if !parent.title.trim().is_empty() {
        folder_path.push(parent.title.clone());
      }
      cursor = parent.parent;
    }
    folder_path.reverse();
    out.push(ImportedPageSpec {
      title: page.title.clone(),
      markdown: page.markdown.clone(),
      folder_path,
    });
  }
  out
}

/// Parse a `.mica.zip`, Markdown `.zip`, or Notion "Markdown & CSV" `.zip`
/// archive into normalized [`ImportedPageSpec`] records using `mica-interchange`.
pub fn parse_zip_archive_to_pages(
  bytes: &[u8],
  notion_mode: bool,
) -> Result<Vec<ImportedPageSpec>, String> {
  let raw = read_zip(bytes).map_err(|e| format!("读取 ZIP 归档失败：{e}"))?;
  if notion_mode {
    let plan = plan_import(raw, true, ImportMode::AsIs)
      .map_err(|e| format!("解析 Notion 归档失败：{e}"))?;
    let pages = plan_to_imported_pages(&plan);
    if pages.is_empty() {
      return Err(
        "导入失败：压缩包里没有 Markdown 文件。如果是从 Notion 导出的，请选「Markdown & CSV」而不是「HTML」。"
          .to_string(),
      );
    }
    return Ok(pages);
  }

  let archive = normalize_archive(raw).map_err(|e| format!("解析 ZIP 归档失败：{e}"))?;
  if archive.markdown_count() == 0 {
    return Err(
      "导入失败：压缩包里没有 Markdown 文件。如果是从 Notion 导出的，请选「Markdown & CSV」而不是「HTML」。"
        .to_string(),
    );
  }
  let plan = plan_vault_import(archive.entries);
  let pages = plan_to_imported_pages(&plan);
  if pages.is_empty() {
    return Err("归档中没有可导入的页面".to_string());
  }
  Ok(pages)
}

/// Recursively walk a local directory for `.md` / `.markdown` files and plan them
/// via `mica-interchange::plan_vault_import`.
///
/// When `wrap_root_folder` is `true`, the picked directory's own name is prepended
/// as the outer folder segment in each [`ImportedPageSpec::folder_path`].
pub fn import_folder_from_disk(
  dir: &Path,
  wrap_root_folder: bool,
) -> Result<Vec<ImportedPageSpec>, String> {
  #[cfg(not(target_arch = "wasm32"))]
  {
    if !dir.exists() {
      return Err(format!("目录不存在：{}", dir.display()));
    }
    if !dir.is_dir() {
      return Err(format!("目标不是文件夹：{}", dir.display()));
    }

    let mut entries: Vec<ZipFileEntry> = Vec::new();
    walk_markdown_dir(dir, dir, &mut entries)?;
    if entries.is_empty() {
      return Err(format!("文件夹中没有找到 Markdown 文件：{}", dir.display()));
    }

    let plan = plan_vault_import(entries);
    let mut pages = plan_to_imported_pages(&plan);
    if pages.is_empty() {
      return Err("文件夹中没有可导入的页面".to_string());
    }

    if wrap_root_folder {
      let root_name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("导入文件夹")
        .to_string();
      for page in &mut pages {
        page.folder_path.insert(0, root_name.clone());
      }
    }

    Ok(pages)
  }
  #[cfg(target_arch = "wasm32")]
  {
    let _ = (dir, wrap_root_folder);
    Err("Web 端暂不支持直接读取本地文件夹".to_string())
  }
}

#[cfg(not(target_arch = "wasm32"))]
fn walk_markdown_dir(
  root: &Path,
  current: &Path,
  out: &mut Vec<ZipFileEntry>,
) -> Result<(), String> {
  let read = std::fs::read_dir(current)
    .map_err(|e| format!("读取目录失败 {}：{e}", current.display()))?;
  let mut children: Vec<PathBuf> = Vec::new();
  for entry in read {
    let entry = entry.map_err(|e| format!("遍历目录失败 {}：{e}", current.display()))?;
    let path = entry.path();
    let name = entry.file_name();
    let name_str = name.to_string_lossy();
    // Skip hidden/dot files and directories (`.git`, `.obsidian`, `.DS_Store`, etc.).
    if name_str.starts_with('.') {
      continue;
    }
    children.push(path);
  }
  children.sort();

  for path in children {
    if path.is_dir() {
      walk_markdown_dir(root, &path, out)?;
    } else if path.is_file() {
      let is_md = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"));
      if !is_md {
        continue;
      }
      let rel = path
        .strip_prefix(root)
        .map_err(|e| format!("计算相对路径失败 {}：{e}", path.display()))?;
      let mut archive_name = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
      if archive_name.to_ascii_lowercase().ends_with(".markdown") {
        let trim_len = archive_name.len() - ".markdown".len();
        archive_name.truncate(trim_len);
        archive_name.push_str(".md");
      }
      let bytes = std::fs::read(&path)
        .map_err(|e| format!("读取文件失败 {}：{e}", path.display()))?;
      out.push(ZipFileEntry {
        name: archive_name,
        bytes,
      });
    }
  }
  Ok(())
}

// ── Native OS File Dialog Wrappers (nsh-core 0.6.1) ─────────────────────────

/// Show a native OS Save-As dialog and write `bytes` to the chosen destination.
///
/// - Returns `Ok(Some(path))` when written.
/// - Returns `Ok(None)` when the user cancels the dialog.
/// - On headless / unsupported desktop environments where the OS dialog is
///   unavailable, gracefully falls back to writing into `default_export_dir()`.
pub async fn save_bytes_with_dialog(
  suggested_name: &str,
  filter_name: &str,
  exts: &[&str],
  bytes: &[u8],
) -> Result<Option<PathBuf>, String> {
  #[cfg(not(target_arch = "wasm32"))]
  {
    use nsh_core::file_dialog::{FileFilter, SaveDialog};
    use nsh_core::prelude::pick_save_path;

    let mut dialog = SaveDialog::new().file_name(suggested_name);
    if !exts.is_empty() {
      dialog = dialog.filter(FileFilter::new(filter_name, exts.iter().copied()));
    }

    match pick_save_path(dialog).await {
      Ok(Some(path)) => {
        if let Some(parent) = path.parent()
          && !parent.as_os_str().is_empty()
        {
          std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}：{e}", parent.display()))?;
        }
        std::fs::write(&path, bytes)
          .map_err(|e| format!("写入文件失败 {}：{e}", path.display()))?;
        Ok(Some(path))
      }
      Ok(None) => Ok(None),
      Err(_unavailable) => {
        let dir = default_export_dir();
        std::fs::create_dir_all(&dir)
          .map_err(|e| format!("创建导出目录失败 {}：{e}", dir.display()))?;
        let target = dir.join(suggested_name);
        std::fs::write(&target, bytes)
          .map_err(|e| format!("写入导出文件失败 {}：{e}", target.display()))?;
        Ok(Some(target))
      }
    }
  }
  #[cfg(target_arch = "wasm32")]
  {
    let _ = (suggested_name, filter_name, exts, bytes);
    Err("Web 端暂不支持原生保存对话框".to_string())
  }
}

/// Show a native OS file picker for importing `.md` / `.zip` files.
///
/// Returns `Ok(Vec::new())` if the user cancels.
pub async fn pick_import_files(
  filter_name: &str,
  exts: &[&str],
  multiple: bool,
) -> Result<Vec<PathBuf>, String> {
  #[cfg(not(target_arch = "wasm32"))]
  {
    use nsh_core::file_dialog::{FileFilter, OpenDialog};
    use nsh_core::prelude::pick_paths;

    let mut dialog = OpenDialog::files().multiple(multiple);
    if !exts.is_empty() {
      dialog = dialog.filter(FileFilter::new(filter_name, exts.iter().copied()));
    }
    match pick_paths(dialog).await {
      Ok(Some(paths)) => Ok(paths),
      Ok(None) => Ok(Vec::new()),
      Err(err) => Err(format!("打开文件选择器失败：{err}")),
    }
  }
  #[cfg(target_arch = "wasm32")]
  {
    let _ = (filter_name, exts, multiple);
    Err("Web 端暂不支持原生文件选择器".to_string())
  }
}

/// Show a native OS folder picker for importing a Markdown vault directory.
///
/// Returns `Ok(None)` if the user cancels.
pub async fn pick_import_folder() -> Result<Option<PathBuf>, String> {
  #[cfg(not(target_arch = "wasm32"))]
  {
    use nsh_core::file_dialog::OpenDialog;
    use nsh_core::prelude::pick_paths;

    let dialog = OpenDialog::folders().multiple(false);
    match pick_paths(dialog).await {
      Ok(Some(mut paths)) => Ok(paths.drain(..).next()),
      Ok(None) => Ok(None),
      Err(err) => Err(format!("打开文件夹选择器失败：{err}")),
    }
  }
  #[cfg(target_arch = "wasm32")]
  {
    Err("Web 端暂不支持原生文件夹选择器".to_string())
  }
}

// ── Legacy / Direct Disk Export & Import Helpers ─────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
pub fn default_export_dir() -> PathBuf {
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

/// Alias for [`export_markdown_file`].
pub fn export_markdown_to_disk(title: &str, markdown: &str) -> Result<String, String> {
  export_markdown_file(title, markdown)
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

/// Alias for [`export_html_file`].
pub fn export_html_to_disk(title: &str, html_body: &str) -> Result<String, String> {
  export_html_file(title, html_body)
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

/// Alias for [`export_workspace_bundle`].
pub fn export_workspace_bundle_to_disk(
  workspace_name: &str,
  pages: &[(String, String)],
) -> Result<String, String> {
  export_workspace_bundle(workspace_name, pages)
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

/// Alias for [`import_markdown_from_path`].
pub fn import_markdown_from_disk(path: &str) -> Result<(String, String), String> {
  import_markdown_from_path(path)
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

  fn sample_view(
    ws_id: Uuid,
    id: Uuid,
    object_id: Uuid,
    parent: Option<Uuid>,
    object_type: &str,
    name: &str,
    position: &str,
  ) -> View {
    let now = Utc::now();
    View {
      id,
      workspace_id: ws_id,
      parent_view_id: parent,
      object_id,
      object_type: object_type.to_string(),
      name: name.to_string(),
      icon: None,
      position: position.to_string(),
      is_deleted: false,
      created_by: Uuid::new_v4(),
      created_at: now,
      updated_at: now,
      state_bytes: None,
    }
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

  #[test]
  fn mica_zip_backup_and_markdown_zip_round_trip() {
    let ws_id = Uuid::new_v4();
    let folder_id = Uuid::new_v4();
    let doc1_view_id = Uuid::new_v4();
    let doc1_obj_id = Uuid::new_v4();
    let doc2_view_id = Uuid::new_v4();
    let doc2_obj_id = Uuid::new_v4();

    let views = vec![
      sample_view(ws_id, folder_id, folder_id, None, "folder", "产品设计", "a0"),
      sample_view(
        ws_id,
        doc1_view_id,
        doc1_obj_id,
        Some(folder_id),
        "page",
        "架构规范",
        "a0",
      ),
      sample_view(ws_id, doc2_view_id, doc2_obj_id, None, "page", "首页", "a1"),
    ];
    let mut docs = HashMap::new();
    docs.insert(doc1_obj_id, "第一段正文。\n\n- 要点 A\n- 要点 B".to_string());
    docs.insert(doc2_obj_id, "欢迎使用 Mica 纯 Rust 客户端。".to_string());

    // 1. Full `.mica.zip` backup includes sidecar `.mica.json` + manifest.json + `.md`
    let backup_bytes =
      build_workspace_zip_bytes("研发工作区", &views, &docs, ArchiveExportFormat::BackupMicaZip)
        .expect("build backup zip");
    let raw_entries = read_zip(&backup_bytes).expect("read backup zip");
    assert!(raw_entries.iter().any(|e| e.name == "manifest.json"));
    assert!(raw_entries.iter().any(|e| e.name.ends_with(".mica.json")));

    let imported_backup =
      parse_zip_archive_to_pages(&backup_bytes, false).expect("import backup zip");
    assert_eq!(imported_backup.len(), 2);
    let nested = imported_backup
      .iter()
      .find(|p| p.title == "架构规范")
      .expect("nested doc");
    assert_eq!(nested.folder_path, vec!["产品设计".to_string()]);
    assert!(nested.markdown.contains("第一段正文"));
    assert!(nested.markdown.contains("要点 A"));
    let root_doc = imported_backup
      .iter()
      .find(|p| p.title == "首页")
      .expect("root doc");
    assert!(root_doc.folder_path.is_empty());
    assert!(root_doc.markdown.contains("欢迎使用 Mica"));

    // 2. Portable Markdown `.zip` omits `.mica.json` sidecars and still round-trips!
    let md_zip_bytes =
      build_workspace_zip_bytes("研发工作区", &views, &docs, ArchiveExportFormat::MarkdownZip)
        .expect("build md zip");
    let md_entries = read_zip(&md_zip_bytes).expect("read md zip");
    assert!(!md_entries.iter().any(|e| e.name.ends_with(".mica.json")));
    let imported_md = parse_zip_archive_to_pages(&md_zip_bytes, false).expect("import md zip");
    assert_eq!(imported_md.len(), 2);

    // 3. Subtree ZIP export for folder contains only its descendants
    let sub_bytes = build_subtree_zip_bytes(
      folder_id,
      &views,
      &docs,
      ArchiveExportFormat::MarkdownZip,
    )
    .expect("build subtree zip");
    let imported_sub = parse_zip_archive_to_pages(&sub_bytes, false).expect("import subtree zip");
    assert_eq!(imported_sub.len(), 1);
    assert_eq!(imported_sub[0].title, "架构规范");
    assert!(imported_sub[0].folder_path.is_empty());
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn import_folder_walks_nested_markdown_and_wraps_root() {
    let dir = temp_test_dir("vault");
    let sub = dir.join("项目文档");
    std::fs::create_dir_all(&sub).expect("create sub");
    std::fs::create_dir_all(dir.join(".hidden")).expect("create hidden");
    std::fs::write(dir.join("概览.md"), "总览内容").expect("write root md");
    std::fs::write(sub.join("接口.markdown"), "接口定义").expect("write nested md");
    std::fs::write(dir.join(".hidden").join("secret.md"), "ignored").expect("write hidden md");

    let unwrapped = import_folder_from_disk(&dir, false).expect("import folder unwrapped");
    assert_eq!(unwrapped.len(), 2);
    let nested = unwrapped.iter().find(|p| p.title == "接口").expect("nested");
    assert_eq!(nested.folder_path, vec!["项目文档".to_string()]);

    let wrapped = import_folder_from_disk(&dir, true).expect("import folder wrapped");
    assert_eq!(wrapped.len(), 2);
    let root_folder = dir.file_name().unwrap().to_string_lossy().to_string();
    let wrapped_nested = wrapped.iter().find(|p| p.title == "接口").expect("wrapped nested");
    assert_eq!(
      wrapped_nested.folder_path,
      vec![root_folder, "项目文档".to_string()]
    );

    let _ = std::fs::remove_dir_all(&dir);
  }
}
