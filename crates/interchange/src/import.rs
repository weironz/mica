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
  /// empty for directory/folder pages.
  pub markdown: String,
  /// A pure container (folder) — no document content. A directory that has no
  /// `.md` of its own, or a manifest `type:"folder"` entry. The executor
  /// creates a `object_type='folder'` view (no document/snapshot) for these.
  pub is_folder: bool,
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

  // Planning order: manifest entries first (they carry pre-order + explicit
  // `type`, so folders — even empty ones with no `.md` — round-trip), then any
  // `.md` the manifest didn't mention (partial/foreign archives), as documents.
  // A folder entry's `path` is a directory (no `.md`); a document entry's is the
  // `.md` file.
  let manifest_docs: Vec<String> = mds.keys().cloned().collect();
  let entries = manifest_entries(manifest.as_deref());
  // A folder has no `.md`/H1 to recover its display name from — its real name
  // lives only in the manifest `title`. Key by the folder's dir path so the
  // chain walk restores the true name instead of the sanitized path segment
  // (spaces/punctuation collapse to `_` in the path). Documents are unaffected:
  // their name round-trips through the `# {name}` H1 the export prepends.
  let folder_titles: HashMap<String, String> = entries
    .iter()
    .filter(|(_, is_folder, _)| *is_folder)
    .filter_map(|(path, _, title)| title.clone().map(|t| (path.clone(), t)))
    .collect();
  let mut ordered: Vec<(String, bool)> = Vec::new();
  let mut queued: HashSet<String> = HashSet::new();
  for (path, is_folder, _) in entries {
    if is_folder {
      if queued.insert(path.clone()) {
        ordered.push((path, true));
      }
    } else if mds.contains_key(&path) && queued.insert(path.clone()) {
      ordered.push((path, false));
    }
  }
  for path in order_page_paths(manifest_docs, manifest.as_deref()) {
    if queued.insert(path.clone()) {
      ordered.push((path, false));
    }
  }

  let mut pages: Vec<PagePlan> = Vec::new();
  let mut page_by_path: HashMap<String, usize> = HashMap::new();
  let mut folder_page: HashMap<String, usize> = HashMap::new();

  for (path, is_folder) in &ordered {
    let parts: Vec<&str> = path.split('/').collect();
    // For a folder entry the whole path is the folder chain; for a document the
    // ancestors are folders and the last segment is the file.
    let chain = if *is_folder { parts.len() } else { parts.len().saturating_sub(1) };

    let mut parent: Option<usize> = None;
    let mut folder_path = String::new();
    let mut norm_folder = String::new();
    for seg_raw in parts.iter().take(chain) {
      if !folder_path.is_empty() {
        folder_path.push('/');
      }
      folder_path.push_str(seg_raw);
      let seg = clean(seg_raw);
      if !norm_folder.is_empty() {
        norm_folder.push('/');
      }
      norm_folder.push_str(&seg);

      // Prefer a `.md`-backed container (old convention / an existing
      // document-with-children: `Doc.md` + `Doc/…`) — that folder maps to the
      // DOCUMENT page, keeping its body. Else reuse a synthetic folder, else
      // create one (a pure container → object_type='folder').
      let existing = md_for_folder
        .get(&norm_folder)
        .and_then(|mdp| page_by_path.get(mdp))
        .copied()
        .or_else(|| folder_page.get(&folder_path).copied());
      let idx = match existing {
        Some(idx) => idx,
        None => {
          // Prefer the manifest's real name; fall back to the (cleaned) path
          // segment for manifest-less / foreign archives.
          let title = folder_titles
            .get(&folder_path)
            .cloned()
            .unwrap_or_else(|| seg.clone());
          pages.push(PagePlan {
            archive_path: None,
            title,
            parent,
            markdown: String::new(),
            is_folder: true,
          });
          let idx = pages.len() - 1;
          folder_page.insert(folder_path.clone(), idx);
          idx
        }
      };
      parent = Some(idx);
    }

    // A folder entry is fully realized by the chain walk above (its leaf folder
    // page). A document entry adds its own page under the walked ancestors.
    if *is_folder {
      continue;
    }
    let markdown = &mds[path];
    let base = parts.last().unwrap_or(&"");
    let fallback = clean(
      base.strip_suffix(".md").or_else(|| base.strip_suffix(".MD")).unwrap_or(base),
    );
    // The file name IS the page name; the body is the body. Nothing is promoted
    // out of the text and nothing is stripped from it — a page's name is a
    // property of the page, not a line inside it, and our own export writes the
    // name into the file name, never into the text.
    //
    // Notion is the one exception, and it is not ours to fix: a Notion export
    // puts the title in the file name AND repeats it as the body's first `# H1`
    // (which is why every third-party Notion importer ships a "remove duplicate
    // title" step). Importing that verbatim would show the title twice on every
    // page. So strip it there, and ONLY there — and only when it matches the
    // name exactly, so a heading that merely happens to lead the page survives.
    let title = fallback;
    let body = if notion {
      strip_leading_h1(markdown, &title)
    } else {
      markdown.clone()
    };
    pages.push(PagePlan {
      archive_path: Some(path.clone()),
      title,
      parent,
      markdown: body,
      is_folder: false,
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

/// Parse the export `manifest.json` into `(path, is_folder, title)` in listed
/// (pre-order) order. `title` is the entry's original display name (used to
/// restore folder names, which have no `.md`/H1 to recover from). Entries
/// without a `type` field default to document (v1 manifests, pre-folder).
/// Empty/absent manifest → no entries.
fn manifest_entries(manifest_json: Option<&str>) -> Vec<(String, bool, Option<String>)> {
  let Some(json) = manifest_json else {
    return Vec::new();
  };
  let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
    return Vec::new();
  };
  let Some(pages) = value.get("pages").and_then(|p| p.as_array()) else {
    return Vec::new();
  };
  pages
    .iter()
    .filter_map(|p| {
      let path = p.get("path")?.as_str()?.to_string();
      let is_folder = p.get("type").and_then(|t| t.as_str()) == Some("folder");
      let title = p
        .get("title")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
      Some((path, is_folder, title))
    })
    .collect()
}

/// Drop a leading `# <title>` that duplicates the page name. Notion-import only
/// — see the call site. Exact match, nothing else: a leading heading that says
/// something different is the author's content and must survive.
fn strip_leading_h1(markdown: &str, title: &str) -> String {
  let mut lines = markdown.lines();
  let mut head = Vec::new();
  for line in lines.by_ref() {
    if line.trim().is_empty() {
      head.push(line);
      continue;
    }
    if line.trim().strip_prefix("# ").map(str::trim) == Some(title) {
      return lines.collect::<Vec<_>>().join("
");
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

#[cfg(test)]
mod tests {
  use super::*;

  fn e(name: &str, bytes: &str) -> ZipFileEntry {
    ZipFileEntry { name: name.to_string(), bytes: bytes.as_bytes().to_vec() }
  }

  /// Build the `manifest.json` a Mica export emits for the given entries.
  fn manifest(entries: &[(&str, &str, &str)]) -> String {
    let pages: Vec<serde_json::Value> = entries
      .iter()
      .map(|(path, title, ty)| serde_json::json!({"path": path, "title": title, "type": ty}))
      .collect();
    serde_json::json!({"version": 1, "generator": "mica", "pages": pages}).to_string()
  }

  fn find<'a>(plan: &'a ImportPlan, title: &str) -> &'a PagePlan {
    plan.pages.iter().find(|p| p.title == title).expect("page present")
  }
  fn parent_title<'a>(plan: &'a ImportPlan, title: &str) -> Option<&'a str> {
    find(plan, title).parent.map(|i| plan.pages[i].title.as_str())
  }

  // A pure folder with a child document (the new Mica export shape): the folder
  // imports as a folder page (no content), the child nests under it.
  #[test]
  fn folder_with_child_imports_as_folder() {
    let raw = vec![
      e("manifest.json", &manifest(&[
        ("Chapter", "Chapter", "folder"),
        ("Chapter/Intro.md", "Intro", "document"),
      ])),
      e("Chapter/Intro.md", "# Intro\n\nhello"),
    ];
    let plan = plan_import(raw, false);
    let chapter = find(&plan, "Chapter");
    assert!(chapter.is_folder, "Chapter is a folder");
    assert!(chapter.archive_path.is_none() && chapter.markdown.is_empty());
    let intro = find(&plan, "Intro");
    assert!(!intro.is_folder);
    // Verbatim: the leading heading is the author's content, not a title to
    // harvest. The name came from the file name (`Intro.md`), as it should.
    assert_eq!(intro.markdown.trim(), "# Intro

hello");
    assert_eq!(parent_title(&plan, "Intro"), Some("Chapter"));
  }

  // An EMPTY folder (only in the manifest, no `.md`, no children) survives the
  // round-trip — this is what the manifest folder entry buys us.
  #[test]
  fn empty_folder_survives_via_manifest() {
    let raw = vec![
      e("manifest.json", &manifest(&[("Empty", "Empty", "folder")])),
    ];
    let plan = plan_import(raw, false);
    assert_eq!(plan.pages.len(), 1);
    let empty = find(&plan, "Empty");
    assert!(empty.is_folder && empty.parent.is_none());
  }

  // A document that HAS children (Doc.md + Doc/Child.md, both type=document) stays
  // a DOCUMENT (keeps its body) — it must NOT be turned into a folder.
  #[test]
  fn document_with_children_stays_a_document() {
    let raw = vec![
      e("manifest.json", &manifest(&[
        ("Doc.md", "Doc", "document"),
        ("Doc/Child.md", "Child", "document"),
      ])),
      e("Doc.md", "# Doc\n\nparent body"),
      e("Doc/Child.md", "# Child\n\nchild body"),
    ];
    let plan = plan_import(raw, false);
    let doc = find(&plan, "Doc");
    assert!(!doc.is_folder, "Doc keeps document identity");
    assert_eq!(doc.markdown.trim(), "# Doc

parent body");
    assert_eq!(parent_title(&plan, "Child"), Some("Doc"));
  }

  // Folder names round-trip through the manifest `title`, NOT the sanitized dir
  // path. Export collapses spaces/punctuation to `_` in the path; import must
  // restore the real name from `title` (a folder has no `.md`/H1 to recover it).
  #[test]
  fn folder_name_with_spaces_round_trips_via_manifest_title() {
    let raw = vec![
      e("manifest.json", &manifest(&[
        ("My_Folder", "My Folder", "folder"),
        ("My_Folder/Note.md", "Note", "document"),
      ])),
      e("My_Folder/Note.md", "# Note\n\nbody"),
    ];
    let plan = plan_import(raw, false);
    let folder = find(&plan, "My Folder");
    assert!(folder.is_folder, "folder keeps its real name from the manifest");
    assert_eq!(parent_title(&plan, "Note"), Some("My Folder"));
    // The sanitized path segment must NEVER leak in as a display name.
    assert!(
      plan.pages.iter().all(|p| p.title != "My_Folder"),
      "sanitized path segment must not survive as a title",
    );
  }

  // A foreign archive with no manifest (Obsidian/plain markdown tree): a
  // directory that contains files becomes a folder (not an empty document).
  // Two top-level dirs so neither is peeled as a lone wrapper by normalize.
  #[test]
  fn foreign_directory_without_manifest_becomes_folder() {
    let raw = vec![
      e("Section/Note.md", "# Note\n\nbody"),
      e("Appendix/Refs.md", "# Refs\n\nlinks"),
    ];
    let plan = plan_import(raw, false);
    let section = find(&plan, "Section");
    assert!(section.is_folder, "a bare directory imports as a folder");
    assert!(section.archive_path.is_none() && section.markdown.is_empty());
    assert_eq!(parent_title(&plan, "Note"), Some("Section"));
    assert!(find(&plan, "Appendix").is_folder);
    assert_eq!(parent_title(&plan, "Refs"), Some("Appendix"));
  }
}
