//! Import planning: turn raw archive entries into an executable, ordered
//! [ImportPlan]. Pure — no database, no storage, no network; the api-server
//! executes the plan (creates pages, uploads referenced assets, rewires
//! links).

use std::collections::{HashMap, HashSet};

use crate::normalize::{expand_nested_zips, normalize_entries};
use crate::notion::{folder_page_index, looks_like_notion_export, strip_notion_id};
use crate::order::order_page_paths;
use crate::zip::ZipFileEntry;

/// One page to create, in creation order (parents always come earlier).
#[derive(Debug)]
pub struct PagePlan {
  /// The archive path this page came from; None for a synthetic directory
  /// page (a folder with no md of its own).
  pub archive_path: Option<String>,
  pub title: String,
  /// Index of the parent page in [ImportPlan::pages]; None = workspace root.
  pub parent: Option<usize>,
  /// Markdown body (leading H1 duplicating the title already stripped);
  /// empty for directory pages.
  pub markdown: String,
}

#[derive(Debug)]
pub struct ImportPlan {
  pub pages: Vec<PagePlan>,
  /// Non-markdown files by raw archive path — uploaded lazily when a page
  /// actually references them.
  pub files: HashMap<String, Vec<u8>>,
  /// All markdown paths (link-rewiring targets).
  pub md_paths: HashSet<String>,
  /// Page index by archive path (for link → page resolution).
  pub page_by_path: HashMap<String, usize>,
  pub notion: bool,
}

pub fn plan_import(raw: Vec<ZipFileEntry>, notion_hint: bool) -> ImportPlan {
  let entries = expand_nested_zips(normalize_entries(raw));

  let manifest = entries
    .iter()
    .find(|e| e.name == "manifest.json")
    .map(|e| String::from_utf8_lossy(&e.bytes).into_owned());

  let mut files: HashMap<String, Vec<u8>> = HashMap::new();
  let mut mds: HashMap<String, String> = HashMap::new();
  for e in entries {
    if e.name.to_lowercase().ends_with(".md") {
      mds.insert(e.name, String::from_utf8_lossy(&e.bytes).into_owned());
    } else if e.name != "manifest.json" {
      files.insert(e.name, e.bytes);
    }
  }

  let notion = notion_hint || looks_like_notion_export(mds.keys().map(String::as_str));
  let clean = |s: &str| -> String {
    if notion { strip_notion_id(s).to_string() } else { s.to_string() }
  };
  let md_for_folder = folder_page_index(mds.keys().map(String::as_str), notion);
  let ordered = order_page_paths(mds.keys().cloned().collect(), manifest.as_deref());

  let mut pages: Vec<PagePlan> = Vec::new();
  let mut page_by_path: HashMap<String, usize> = HashMap::new();
  let mut folder_page: HashMap<String, usize> = HashMap::new();

  for path in &ordered {
    let parts: Vec<&str> = path.split('/').collect();
    // Walk the folder chain. A folder maps to the page exported as
    // `<folder>.md` (modulo Notion IDs) when present; otherwise a synthetic
    // directory page is planned.
    let mut parent: Option<usize> = None;
    let mut folder_path = String::new();
    let mut norm_folder = String::new();
    for seg_raw in parts.iter().take(parts.len().saturating_sub(1)) {
      if !folder_path.is_empty() {
        folder_path.push('/');
      }
      folder_path.push_str(seg_raw);
      let seg = clean(seg_raw);
      if !norm_folder.is_empty() {
        norm_folder.push('/');
      }
      norm_folder.push_str(&seg);

      let existing = md_for_folder
        .get(&norm_folder)
        .and_then(|mdp| page_by_path.get(mdp))
        .copied()
        .or_else(|| folder_page.get(&folder_path).copied());
      let idx = match existing {
        Some(idx) => idx,
        None => {
          pages.push(PagePlan {
            archive_path: None,
            title: seg,
            parent,
            markdown: String::new(),
          });
          let idx = pages.len() - 1;
          folder_page.insert(folder_path.clone(), idx);
          idx
        }
      };
      parent = Some(idx);
    }

    let markdown = &mds[path];
    let base = parts.last().unwrap_or(&"");
    let fallback = clean(
      base.strip_suffix(".md").or_else(|| base.strip_suffix(".MD")).unwrap_or(base),
    );
    let title = title_from_markdown(markdown, &fallback);
    let body = strip_leading_h1(markdown, &title);
    pages.push(PagePlan {
      archive_path: Some(path.clone()),
      title,
      parent,
      markdown: body,
    });
    page_by_path.insert(path.clone(), pages.len() - 1);
  }

  ImportPlan {
    pages,
    files,
    md_paths: mds.into_keys().collect(),
    page_by_path,
    notion,
  }
}

/// First `# ` heading, else the fallback (cleaned filename).
fn title_from_markdown(markdown: &str, fallback: &str) -> String {
  for line in markdown.lines() {
    let t = line.trim();
    if t.is_empty() {
      continue;
    }
    if let Some(h) = t.strip_prefix("# ") {
      return h.trim().to_string();
    }
    break;
  }
  fallback.to_string()
}

/// Drop a leading `# <title>` line that duplicates the page title (exports
/// prepend one; round-trips must not double it).
fn strip_leading_h1(markdown: &str, title: &str) -> String {
  let mut lines = markdown.lines();
  let mut head = Vec::new();
  for line in lines.by_ref() {
    if line.trim().is_empty() {
      head.push(line);
      continue;
    }
    if line.trim().strip_prefix("# ").map(str::trim) == Some(title) {
      return lines.collect::<Vec<_>>().join("\n");
    }
    break;
  }
  markdown.to_string()
}

/// Resolve a Markdown reference (`../assets/图 1.png`, `Page%20<id>.md`)
/// found inside [from_file] to an archive path in [paths]. Returns None when
/// the reference is external (has a URL scheme) or matches nothing. Tries
/// the file's own folder first, then the archive root.
pub fn resolve_ref(from_file: &str, href: &str, paths: &HashSet<String>) -> Option<String> {
  let mut u = href.trim();
  if u.is_empty() || has_scheme(u) {
    return None;
  }
  u = u.split(['#', '?']).next().unwrap_or(u);
  let decoded = percent_decode(u);

  let dir: Vec<&str> = match from_file.rsplit_once('/') {
    Some((d, _)) => d.split('/').collect(),
    None => Vec::new(),
  };
  for base in [&dir[..], &[]] {
    let mut stack: Vec<&str> = base.to_vec();
    for seg in decoded.split('/') {
      match seg {
        "" | "." => {}
        ".." => {
          stack.pop();
        }
        s => stack.push(s),
      }
    }
    let candidate = stack.join("/");
    if paths.contains(&candidate) {
      return Some(candidate);
    }
  }
  None
}

fn has_scheme(s: &str) -> bool {
  let Some(colon) = s.find(':') else { return false };
  let prefix = &s[..colon];
  if let Some(slash) = s.find('/')
    && slash < colon
  {
    return false;
  }
  !prefix.is_empty()
    && prefix.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
    && prefix.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
}

fn percent_decode(s: &str) -> String {
  let b = s.as_bytes();
  let mut out = Vec::with_capacity(b.len());
  let mut i = 0;
  while i < b.len() {
    if b[i] == b'%' && i + 2 < b.len() {
      let hi = (b[i + 1] as char).to_digit(16);
      let lo = (b[i + 2] as char).to_digit(16);
      if let (Some(hi), Some(lo)) = (hi, lo) {
        out.push((hi * 16 + lo) as u8);
        i += 3;
        continue;
      }
    }
    out.push(b[i]);
    i += 1;
  }
  String::from_utf8_lossy(&out).into_owned()
}
