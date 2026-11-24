//! Portable self-contained whiteboard SVG materialization (export) and lossless
//! `mica-board` fence restoration (import).
//!
//! - **Mica Backup (`.mica.zip`)** keeps ` ```mica-board ` verbatim in both
//!   `.md` and `.mica.json`.
//! - **Portable Markdown ZIP (`.zip`)** calls [`materialize_portable_boards`]
//!   to render each valid `mica-board` block into a self-contained SVG under
//!   `assets/<name>.svg` (carrying `<metadata id="mica-board-v1">`) and
//!   rewrites the block to a clean `![title](<rel>assets/<name>.svg)` image so
//!   external Markdown viewers (GitHub, Obsidian, Typora) render the diagram.
//! - **Every Markdown import path** (ZIP, folder, single `.md` with local
//!   assets) calls [`restore_embedded_boards_from_archive`] or
//!   [`restore_embedded_boards_in_markdown`] to detect self-contained Mica
//!   board SVGs and transparently restore editable ` ```mica-board ` blocks.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use mica_md::DocumentSnapshotPayload;
use serde_json::Value;

use crate::export_tree::{safe_segment, unique_asset_name};
use crate::import::resolve_ref;
use crate::zip::ZipEntry;

/// Convert valid `mica-board` fenced code blocks in `payload` into portable
/// self-contained `.svg` archive entries under `assets/` and rewrite those
/// blocks to standard Markdown `image` blocks pointing at `{rel_prefix}assets/<name>.svg`.
///
/// Malformed `mica-board` JSON blocks are left untouched so user text is never lost.
pub fn materialize_portable_boards(
  payload: &mut DocumentSnapshotPayload,
  rel_prefix: &str,
  used_assets: &mut HashSet<String>,
) -> Vec<ZipEntry> {
  let mut entries = Vec::new();
  for block in &mut payload.blocks {
    if !matches!(block.kind.as_str(), "code_block" | "code") {
      continue;
    }
    let is_board = block
      .data
      .get("language")
      .and_then(Value::as_str)
      .is_some_and(|lang| lang.eq_ignore_ascii_case("mica-board"));
    if !is_board {
      continue;
    }
    let Ok(snap) = mica_board::BoardSnapshot::parse_json(&block.text) else {
      continue;
    };
    let svg = mica_board::render_board_svg_with_source(&snap, None);
    let title_trimmed = snap.title.trim();
    let stem = if title_trimmed.is_empty() {
      "board".to_string()
    } else {
      let s = safe_segment(title_trimmed);
      if s.is_empty() || s == "untitled" {
        "board".to_string()
      } else {
        s
      }
    };
    let desired = format!("{stem}.svg");
    let asset_name = unique_asset_name(&desired, used_assets);
    entries.push(ZipEntry {
      name: format!("assets/{asset_name}"),
      data: svg.into_bytes(),
    });

    let alt = if title_trimmed.is_empty() {
      "Mica 画板".to_string()
    } else {
      title_trimmed.to_string()
    };
    let target = format!("{rel_prefix}assets/{asset_name}");

    block.kind = "image".to_string();
    block.text = alt;
    if let Some(obj) = block.data.as_object_mut() {
      obj.remove("language");
      obj.remove("file_id");
      obj.remove("name");
      obj.insert("url".to_string(), Value::String(target));
    } else {
      block.data = serde_json::json!({ "url": target });
    }
  }
  entries
}

/// Restore self-contained Mica board SVGs referenced in `markdown` back into
/// editable ` ```mica-board ` fenced code blocks using archive `files`.
///
/// Any archive path whose self-contained board SVG was restored is inserted
/// into `consumed_svgs` so the planner can drop those synthetic preview SVGs
/// from `ImportPlan::files` instead of reporting them as skipped attachments.
pub fn restore_embedded_boards_from_archive(
  markdown: &str,
  archive_path: &str,
  files: &HashMap<String, Vec<u8>>,
  file_paths: &HashSet<String>,
  consumed_svgs: &mut HashSet<String>,
) -> String {
  if files.is_empty() || !markdown.contains("![") {
    return markdown.to_string();
  }
  let consumed = std::cell::RefCell::new(Vec::<String>::new());
  let rewritten = restore_embedded_boards_in_markdown(markdown, |dest| {
    let resolved = resolve_ref(archive_path, dest, file_paths)?;
    if !resolved.to_ascii_lowercase().ends_with(".svg") {
      return None;
    }
    let bytes = files.get(&resolved)?;
    if mica_board::extract_board_json_from_svg(bytes).is_some() {
      consumed.borrow_mut().push(resolved);
    }
    Some(Cow::Borrowed(bytes.as_slice()))
  });
  consumed_svgs.extend(consumed.into_inner());
  rewritten
}

/// Scan `markdown` outside fenced code blocks for standalone image references
/// (`![alt](dest)`) whose target resolves via `resolve_svg` to a self-contained
/// Mica board SVG, replacing each match with a canonical ` ```mica-board ` block.
///
/// Ordinary images, non-Mica SVGs, and image syntax inside code blocks are
/// preserved verbatim.
pub fn restore_embedded_boards_in_markdown<'a>(
  markdown: &str,
  resolve_svg: impl Fn(&str) -> Option<Cow<'a, [u8]>>,
) -> String {
  if !markdown.contains("![") {
    return markdown.to_string();
  }

  let mut out: Vec<String> = Vec::new();
  let mut open_fence: Option<(u8, usize)> = None;
  let mut changed = false;

  for raw_line in markdown.split('\n') {
    let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
    let stripped = strip_quote_prefix(line).1;

    if let Some((fence_ch, fence_len)) = open_fence {
      out.push(line.to_string());
      if is_closing_fence(stripped, fence_ch, fence_len) {
        open_fence = None;
      }
      continue;
    }

    if let Some(fence) = parse_opening_fence(stripped) {
      open_fence = Some(fence);
      out.push(line.to_string());
      continue;
    }

    if let Some((prefix, dest)) = parse_standalone_image_line(line)
      && let Some(bytes) = resolve_svg(dest)
      && let Some(compact_json) = mica_board::extract_board_json_from_svg(bytes.as_ref())
    {
      let fence = fence_for_body(&compact_json);
      out.push(format!("{prefix}{fence}mica-board"));
      out.push(format!("{prefix}{compact_json}"));
      out.push(format!("{prefix}{fence}"));
      changed = true;
      continue;
    }

    out.push(line.to_string());
  }

  if !changed {
    return markdown.to_string();
  }
  out.join("\n")
}

fn strip_quote_prefix(line: &str) -> (&str, &str) {
  let bytes = line.as_bytes();
  let mut idx = 0;
  loop {
    let start = idx;
    let mut spaces = 0;
    while idx < bytes.len() && bytes[idx] == b' ' && spaces < 3 {
      idx += 1;
      spaces += 1;
    }
    if idx < bytes.len() && bytes[idx] == b'>' {
      idx += 1;
      if idx < bytes.len() && bytes[idx] == b' ' {
        idx += 1;
      }
    } else {
      idx = start;
      break;
    }
  }
  (&line[..idx], &line[idx..])
}

fn parse_opening_fence(line: &str) -> Option<(u8, usize)> {
  let trimmed = line.trim_start_matches(' ');
  let leading_spaces = line.len() - trimmed.len();
  if leading_spaces > 3 {
    return None;
  }
  let bytes = trimmed.as_bytes();
  let first = *bytes.first()?;
  if first != b'`' && first != b'~' {
    return None;
  }
  let count = bytes.iter().take_while(|&&b| b == first).count();
  if count < 3 {
    return None;
  }
  let info = &trimmed[count..];
  if first == b'`' && info.contains('`') {
    return None;
  }
  Some((first, count))
}

fn is_closing_fence(line: &str, fence_ch: u8, fence_len: usize) -> bool {
  let trimmed = line.trim_start_matches(' ');
  let leading_spaces = line.len() - trimmed.len();
  if leading_spaces > 3 {
    return false;
  }
  let bytes = trimmed.as_bytes();
  let count = bytes.iter().take_while(|&&b| b == fence_ch).count();
  count >= fence_len && trimmed[count..].trim().is_empty()
}

/// Parse a standalone Markdown image line (`[quote/indent]![alt](dest [title])`),
/// returning `(line_prefix, raw_destination)`.
fn parse_standalone_image_line(line: &str) -> Option<(&str, &str)> {
  let (quote_prefix, after_quote) = strip_quote_prefix(line);
  let ws_len = after_quote
    .bytes()
    .take_while(|&b| b == b' ' || b == b'\t')
    .count();
  let prefix_len = quote_prefix.len() + ws_len;
  let prefix = &line[..prefix_len];
  let rest = after_quote[ws_len..].trim_end();
  if !rest.starts_with("![") || !rest.ends_with(')') {
    return None;
  }

  // Walk past `![alt]` respecting backslash escapes and balanced brackets.
  let bytes = rest.as_bytes();
  let mut i = 2usize;
  let mut bracket_depth = 1usize;
  while i < bytes.len() && bracket_depth > 0 {
    match bytes[i] {
      b'\\' => {
        i += 2;
        continue;
      }
      b'[' => bracket_depth += 1,
      b']' => bracket_depth -= 1,
      _ => {}
    }
    i += 1;
  }
  if bracket_depth != 0 || i >= bytes.len() || bytes[i] != b'(' {
    return None;
  }

  let inner = rest[i + 1..rest.len() - 1].trim();
  if inner.is_empty() {
    return None;
  }

  let dest = if let Some(after_lt) = inner.strip_prefix('<') {
    let close = after_lt.find('>')?;
    let d = after_lt[..close].trim();
    let tail = after_lt[close + 1..].trim();
    if !tail.is_empty() && !is_valid_optional_title(tail) {
      return None;
    }
    d
  } else {
    // Parse unbracketed destination up to unescaped whitespace (followed by optional title).
    let mut end = 0usize;
    let mut paren_depth = 0usize;
    let ib = inner.as_bytes();
    while end < ib.len() {
      match ib[end] {
        b'\\' => {
          end += 2;
          continue;
        }
        b'(' => paren_depth += 1,
        b')' => {
          if paren_depth == 0 {
            return None;
          }
          paren_depth -= 1;
        }
        b if b.is_ascii_whitespace() && paren_depth == 0 => break,
        _ => {}
      }
      end += 1;
    }
    let d = &inner[..end];
    let tail = inner[end..].trim();
    if !tail.is_empty() && !is_valid_optional_title(tail) {
      return None;
    }
    d
  };

  if dest.is_empty() {
    return None;
  }
  Some((prefix, dest))
}

fn is_valid_optional_title(s: &str) -> bool {
  (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
    || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    || (s.starts_with('(') && s.ends_with(')') && s.len() >= 2)
}

fn fence_for_body(body: &str) -> String {
  let mut max_run = 0usize;
  let mut cur = 0usize;
  for b in body.bytes() {
    if b == b'`' {
      cur += 1;
      max_run = max_run.max(cur);
    } else {
      cur = 0;
    }
  }
  "`".repeat(max_run.max(2) + 1)
}

#[cfg(test)]
mod tests {
  use std::collections::HashMap;

  use crate::{
    ImageAsset, ImportMode, TreeNode, build_markdown_only_tree_zip, build_markdown_tree_zip,
    build_zip, plan_import, read_zip,
  };

  fn sample_board(title: &str, node_text: &str) -> mica_board::BoardSnapshot {
    let mut snap = mica_board::BoardSnapshot::template_mindmap();
    snap.title = title.to_string();
    snap.nodes[0].text = node_text.to_string();
    snap
  }

  fn extract_board_snapshots(md: &str) -> Vec<mica_board::BoardSnapshot> {
    let doc = mica_md::compat::import_markdown(md, "root");
    doc
      .blocks
      .iter()
      .filter(|b| {
        matches!(b.kind.as_str(), "code_block" | "code")
          && b
            .data
            .get("language")
            .and_then(|v| v.as_str())
            .is_some_and(|l| l.eq_ignore_ascii_case("mica-board"))
      })
      .filter_map(|b| mica_board::BoardSnapshot::parse_json(&b.text).ok())
      .collect()
  }

  #[test]
  fn backup_keeps_fence_while_markdown_zip_emits_self_contained_svg_and_roundtrips() {
    let b1 = sample_board("系统架构", "核心网关 <v2> & 路由");
    let b2 = sample_board("系统架构", "二级子目录画板");
    let plain_svg = b"<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'/>".to_vec();

    let root_md = format!(
      "引言段落。\n\n```mica-board\n{}\n```\n\n```mica-board\n{{broken json\n```\n\n![普通图](plain.svg)\n\n```markdown\n![字面示例](assets/系统架构.svg)\n```\n",
      b1.to_compact_json()
    );
    let nested_md = format!(
      "嵌套说明。\n\n```mica-board\n{}\n```\n",
      b2.to_compact_json()
    );

    let mut root_payload = mica_md::compat::import_markdown(&root_md, "root");
    for block in &mut root_payload.blocks {
      if block.kind == "image" {
        block.data = serde_json::json!({ "file_id": "img-plain", "name": "plain.svg" });
      }
    }
    let nested_payload = mica_md::compat::import_markdown(&nested_md, "root");

    let nodes = vec![
      TreeNode {
        id: "v-root".into(),
        parent_id: None,
        position: "a0".into(),
        name: "首页".into(),
        object_type: "document".into(),
        object_id: "d-root".into(),
        icon: None,
      },
      TreeNode {
        id: "v-folder".into(),
        parent_id: None,
        position: "a1".into(),
        name: "设计".into(),
        object_type: "folder".into(),
        object_id: "f-design".into(),
        icon: None,
      },
      TreeNode {
        id: "v-nested".into(),
        parent_id: Some("v-folder".into()),
        position: "a0".into(),
        name: "架构评审".into(),
        object_type: "document".into(),
        object_id: "d-nested".into(),
        icon: None,
      },
    ];
    let payloads = HashMap::from([
      ("d-root".to_string(), root_payload),
      ("d-nested".to_string(), nested_payload),
    ]);
    let images = HashMap::from([(
      "img-plain".to_string(),
      ImageAsset {
        dedup_key: "sha-plain".into(),
        name: "plain.svg".into(),
        bytes: plain_svg,
      },
    )]);

    // 1. Mica backup (.mica.zip) keeps ```mica-board verbatim (no board SVGs emitted).
    let backup_entries = build_markdown_tree_zip(&nodes, None, &payloads, &images);
    let backup_root_md = String::from_utf8(
      backup_entries
        .iter()
        .find(|e| e.name == "首页.md")
        .unwrap()
        .data
        .clone(),
    )
    .unwrap();
    assert!(backup_root_md.contains("```mica-board"));
    assert!(!backup_entries.iter().any(|e| e.name.contains("系统架构")));

    // 2. Portable Markdown ZIP materializes self-contained SVGs under assets/ with clean image syntax.
    let md_entries = build_markdown_only_tree_zip(&nodes, None, &payloads, &images);
    let svg_names: Vec<&str> = md_entries
      .iter()
      .filter(|e| e.name.starts_with("assets/") && e.name.ends_with(".svg"))
      .map(|e| e.name.as_str())
      .collect();
    assert_eq!(
      svg_names.len(),
      3,
      "1 plain svg + 2 distinct board svgs: {svg_names:?}"
    );
    let md_root = String::from_utf8(
      md_entries
        .iter()
        .find(|e| e.name == "首页.md")
        .unwrap()
        .data
        .clone(),
    )
    .unwrap();
    let md_nested = String::from_utf8(
      md_entries
        .iter()
        .find(|e| e.name == "设计/架构评审.md")
        .unwrap()
        .data
        .clone(),
    )
    .unwrap();
    assert!(
      md_root.contains("![系统架构](assets/系统架构.svg)"),
      "{md_root}"
    );
    assert!(
      md_root.contains("{broken json"),
      "malformed board fence must be preserved"
    );
    assert!(
      md_nested.contains("![系统架构](../assets/系统架构-1.svg)"),
      "{md_nested}"
    );

    // 3. Re-importing the portable Markdown ZIP restores editable ```mica-board blocks losslessly!
    let zip_bytes = build_zip(&md_entries);
    let raw = read_zip(&zip_bytes).unwrap();
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();

    let imported_root = plan.pages.iter().find(|p| p.title == "首页").unwrap();
    let root_boards = extract_board_snapshots(&imported_root.markdown);
    assert_eq!(root_boards, vec![b1]);
    assert!(
      imported_root
        .markdown
        .contains("![普通图](assets/plain.svg)")
    );
    assert!(
      imported_root
        .markdown
        .contains("![字面示例](assets/系统架构.svg)"),
      "image example inside fenced code block must not be rewritten"
    );
    assert!(imported_root.markdown.contains("{broken json"));

    let imported_nested = plan.pages.iter().find(|p| p.title == "架构评审").unwrap();
    let nested_boards = extract_board_snapshots(&imported_nested.markdown);
    assert_eq!(nested_boards, vec![b2]);
    // Restored board preview SVGs are removed from `plan.files` so server imports
    // never report them as "skipped unreferenced attachments", while plain SVGs stay!
    assert!(!plan.files.contains_key("assets/系统架构.svg"));
    assert!(!plan.files.contains_key("assets/系统架构-1.svg"));
    assert!(plan.files.contains_key("assets/plain.svg"));
  }

  #[test]
  fn board_with_triple_backticks_restores_using_four_backtick_fence() {
    let board = sample_board("代码画板", "含 ```rust\nfn main(){}\n``` 代码");
    let md = format!("````mica-board\n{}\n````\n", board.to_compact_json());
    let payload = mica_md::compat::import_markdown(&md, "root");
    let nodes = vec![TreeNode {
      id: "v1".into(),
      parent_id: None,
      position: "a0".into(),
      name: "Page".into(),
      object_type: "document".into(),
      object_id: "d1".into(),
      icon: None,
    }];
    let payloads = HashMap::from([("d1".to_string(), payload)]);
    let entries = build_markdown_only_tree_zip(&nodes, None, &payloads, &HashMap::new());
    let zip_bytes = build_zip(&entries);
    let plan = plan_import(read_zip(&zip_bytes).unwrap(), false, ImportMode::AsIs).unwrap();
    let page = &plan.pages[0];
    assert!(
      page.markdown.contains("````mica-board"),
      "{}",
      page.markdown
    );
    assert_eq!(extract_board_snapshots(&page.markdown), vec![board]);
  }
}
