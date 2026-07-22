//! Import planning: turn raw archive entries into an executable, ordered
//! [ImportPlan]. Pure — no database, no storage, no network; the api-server
//! executes the plan (creates pages, uploads referenced assets, rewires
//! links).

use std::collections::{HashMap, HashSet};

use crate::normalize::{expand_nested_zips, normalize_entries};
use crate::notion::{looks_like_notion_export, strip_notion_id};
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
  /// The single top-level wrapper directory that was collapsed away (if any).
  /// The caller uses it to name a new workspace after the folder/zip.
  pub wrapper_name: Option<String>,
}

/// How the archive's top level maps onto the destination — the "container vs
/// flatten" decision (see docs / the import-convention research).
#[derive(Debug, Clone)]
pub enum ImportMode {
  /// Mirror the archive as-is (no collapse, no wrap). Round-trip / test path.
  AsIs,
  /// Import as a NEW workspace: the workspace IS the container, so a single
  /// redundant top-level wrapper folder is collapsed (its name → workspace
  /// name via [`ImportPlan::wrapper_name`]). Mica exports have no single
  /// wrapper, so this is identity for them (round-trip preserved).
  NewWorkspace,
  /// Import INTO an existing workspace/folder, SMART default (the user's
  /// "auto"): a single top-level wrapper — or a lone entry — SPILLS into the
  /// destination (its children become top-level there); multiple loose roots
  /// WRAP under the given fallback name to keep the destination uncluttered.
  /// This is the AppFlowy/Anytype default. The peel itself happened in
  /// `normalize_entries`, so "spill" here is simply "don't wrap".
  Auto(String),
  /// Import INTO an existing workspace/folder, forcing SPILL (user override):
  /// top-level entries land directly under the destination, never wrapped —
  /// even when there are many loose roots. A single wrapper is still peeled
  /// (that happens in `normalize_entries`).
  IntoLocation,
  /// Import INTO an existing workspace/folder, forcing WRAP (user override):
  /// everything goes under ONE new container folder. Its name is the collapsed
  /// wrapper's name if the archive had one (avoids `zipname > samedir`
  /// double-nesting — the SiYuan bug / Anytype fix), else the given fallback
  /// (the zip filename / picked folder).
  IntoContainer(String),
}

pub fn plan_import(raw: Vec<ZipFileEntry>, notion_hint: bool, mode: ImportMode) -> ImportPlan {
  // `normalize_entries` already peels a single top-level wrapper folder
  // (Anytype-style collapse); capture its name FIRST so we can name a new
  // workspace / import container after it (the peel itself discards the name).
  let wrapper_name = crate::normalize::stripped_wrapper_name(&raw);
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

  // Every normalized directory path that has something under it. A `.md` whose
  // own path is in here is a page-WITH-subpages, which Mica's model has no room
  // for (folders are pure containers, pages are leaves) — see the split below.
  let mut dirs_with_children: HashSet<String> = HashSet::new();
  for (path, is_folder) in &ordered {
    let parts: Vec<&str> = path.split('/').collect();
    let chain = if *is_folder { parts.len() } else { parts.len().saturating_sub(1) };
    let mut acc = String::new();
    for seg in parts.iter().take(chain) {
      if !acc.is_empty() {
        acc.push('/');
      }
      acc.push_str(&clean(seg));
      dirs_with_children.insert(acc.clone());
    }
  }

  let mut pages: Vec<PagePlan> = Vec::new();
  let mut page_by_path: HashMap<String, usize> = HashMap::new();
  // Keyed by NORMALIZED path so a Notion `Doc <id-a>/` and its `Doc <id-b>.md`
  // land on the same container even when the two carry different ID suffixes.
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

      // A directory is ALWAYS a folder page. It used to map onto the `.md` that
      // shares its name (`Doc.md` + `Doc/…` → one document that also had
      // children), which produced trees Mica cannot represent: a page is a leaf.
      // The matching `.md` now becomes a leaf page INSIDE this folder instead.
      let idx = match folder_page.get(&norm_folder).copied() {
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
          folder_page.insert(norm_folder.clone(), idx);
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

    // A page that has subpages (Notion's native shape: `Doc.md` next to a
    // `Doc/` holding its children). Mica has no such node — folders hold, pages
    // are leaves — so split it: the folder keeps the name and the children, and
    // the body moves into a same-named leaf page inside it. A parent with no
    // body of its own is just a folder; don't leave an empty page behind.
    let own_norm = if norm_folder.is_empty() {
      title.clone()
    } else {
      format!("{norm_folder}/{title}")
    };
    if dirs_with_children.contains(&own_norm) {
      let folder_idx = match folder_page.get(&own_norm).copied() {
        Some(idx) => idx,
        None => {
          pages.push(PagePlan {
            archive_path: None,
            title: title.clone(),
            parent,
            markdown: String::new(),
            is_folder: true,
          });
          let idx = pages.len() - 1;
          folder_page.insert(own_norm.clone(), idx);
          idx
        }
      };
      if body.trim().is_empty() {
        // Links pointing at the parent page resolve to the folder that replaced it.
        page_by_path.insert(path.clone(), folder_idx);
        continue;
      }
      parent = Some(folder_idx);
    }

    pages.push(PagePlan {
      archive_path: Some(path.clone()),
      title,
      parent,
      markdown: body,
      is_folder: false,
    });
    page_by_path.insert(path.clone(), pages.len() - 1);
  }

  let mut plan = ImportPlan {
    pages,
    files,
    md_paths: mds.into_keys().collect(),
    page_by_path,
    notion,
    wrapper_name: None,
  };
  match mode {
    // Mirror the archive as-is (the peel above still ran, but no container).
    ImportMode::AsIs => {}
    // New workspace = the workspace is the container; the peel already
    // flattened, we just report the peeled name for naming it.
    ImportMode::NewWorkspace => {
      plan.wrapper_name = wrapper_name;
    }
    // Smart default into an existing location: spill a single wrapper (already
    // peeled) or a lone entry directly into the destination; wrap MULTIPLE loose
    // roots under the fallback so they don't litter the destination root. When
    // the peel fired, `wrapper_name` is Some — that IS "there was a single
    // wrapper" — so spill. Otherwise count the surviving top-level nodes.
    ImportMode::Auto(fallback) => {
      let top_level = plan.pages.iter().filter(|p| p.parent.is_none()).count();
      if wrapper_name.is_none() && top_level > 1 {
        wrap_in_container(&mut plan, fallback);
      }
      plan.wrapper_name = wrapper_name;
    }
    // Force spill: the peel already ran, so top-level nodes stay top-level and
    // land directly under the destination — never wrapped.
    ImportMode::IntoLocation => {
      plan.wrapper_name = wrapper_name;
    }
    // Force wrap: everything under ONE container named after the peeled wrapper
    // (if any — no `fallback > wrapper` double-nest) else the fallback (zip/
    // folder name).
    ImportMode::IntoContainer(fallback) => {
      let name = wrapper_name.clone().unwrap_or(fallback);
      plan.wrapper_name = wrapper_name;
      wrap_in_container(&mut plan, name);
    }
  }
  plan
}

/// Prepend ONE container folder named `name` and reparent every top-level page
/// under it (Joplin/SiYuan "wrap on import-into-existing"). The wrapper takes
/// index 0; all existing indices shift by 1.
fn wrap_in_container(plan: &mut ImportPlan, name: String) {
  let mut new_pages: Vec<PagePlan> = Vec::with_capacity(plan.pages.len() + 1);
  new_pages.push(PagePlan {
    archive_path: None,
    title: name,
    parent: None,
    markdown: String::new(),
    is_folder: true,
  });
  for p in plan.pages.drain(..) {
    let parent = Some(p.parent.map(|i| i + 1).unwrap_or(0));
    new_pages.push(PagePlan { parent, ..p });
  }
  plan.pages = new_pages;
  for idx in plan.page_by_path.values_mut() {
    *idx += 1;
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
/// the file's own folder first, then the archive root, then — only if the path
/// structure doesn't line up — a UNIQUE basename anywhere in the archive.
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

  // Fallback: match by BASENAME when the folder structure doesn't line up.
  // Foreign exports routinely write `![](screenshot.png)` while the bytes sit
  // under `assets/screenshot.png` (or the reverse, or a differently-named
  // folder) — the path match above misses that, and the image was silently
  // dropped: the asset never uploaded and the ref left a dead link. Only a
  // UNIQUE basename is safe; an ambiguous one (two `logo.png` in different
  // folders) stays unresolved rather than guessing wrong. Reaching here already
  // means no exact-path match exists, so a root file cannot collide with itself.
  let base = decoded.rsplit('/').next().unwrap_or(&decoded);
  if base.is_empty() {
    return None;
  }
  let mut unique: Option<&String> = None;
  for p in paths {
    if p.rsplit('/').next() == Some(base) {
      if unique.is_some() {
        return None; // ambiguous — do not guess
      }
      unique = Some(p);
    }
  }
  unique.cloned()
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

  fn paths(items: &[&str]) -> HashSet<String> {
    items.iter().map(|s| s.to_string()).collect()
  }

  /// A foreign export often writes `![](screenshot.png)` while the bytes sit
  /// under `assets/screenshot.png`. The strict path match finds neither
  /// `screenshot.png` (root) nor `<md dir>/screenshot.png`, so the image used to
  /// be dropped and the ref left dead. The unique-basename fallback recovers it.
  #[test]
  fn a_bare_ref_resolves_to_the_asset_under_a_subfolder() {
    let ps = paths(&["assets/screenshot.png", "note.md"]);
    assert_eq!(
      resolve_ref("note.md", "screenshot.png", &ps),
      Some("assets/screenshot.png".to_string())
    );
    // Also the reverse (href names a folder the archive doesn't use) and a
    // percent-encoded name.
    let ps2 = paths(&["images/图 1.png"]);
    assert_eq!(
      resolve_ref("note.md", "media/%E5%9B%BE%201.png", &ps2),
      Some("images/图 1.png".to_string())
    );
  }

  /// An exact path still wins — the fallback is a last resort, never a detour.
  #[test]
  fn an_exact_path_is_preferred_over_a_basename_match() {
    let ps = paths(&["photo.png", "assets/photo.png"]);
    // `photo.png` from root resolves to the ROOT file, not the subfolder one.
    assert_eq!(resolve_ref("note.md", "photo.png", &ps), Some("photo.png".to_string()));
  }

  /// An ambiguous basename must NOT be guessed — two `logo.png` in different
  /// folders and a bare ref stays unresolved (the old behaviour), so we never
  /// silently wire an image to the wrong file.
  #[test]
  fn an_ambiguous_basename_is_left_unresolved() {
    let ps = paths(&["a/logo.png", "b/logo.png"]);
    assert_eq!(resolve_ref("note.md", "logo.png", &ps), None);
  }

  /// A basename that matches nothing is still None; an external URL never even
  /// reaches the fallback.
  #[test]
  fn no_match_and_external_urls_stay_none() {
    let ps = paths(&["assets/a.png"]);
    assert_eq!(resolve_ref("note.md", "missing.png", &ps), None);
    assert_eq!(resolve_ref("note.md", "https://x.test/a.png", &ps), None);
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
    let plan = plan_import(raw, false, ImportMode::AsIs);
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
    let plan = plan_import(raw, false, ImportMode::AsIs);
    assert_eq!(plan.pages.len(), 1);
    let empty = find(&plan, "Empty");
    assert!(empty.is_folder && empty.parent.is_none());
  }

  // ── ImportMode: container vs flatten (P1) ──────────────────────────────

  // New-workspace import strips a single top-level wrapper (the workspace IS the
  // container) and reports its name for naming the workspace.
  #[test]
  fn new_workspace_collapses_single_wrapper() {
    let raw = vec![e("MyProject/a.md", "# a\n\nhi"), e("MyProject/sub/b.md", "# b\n\nyo")];
    let plan = plan_import(raw, false, ImportMode::NewWorkspace);
    assert_eq!(plan.wrapper_name.as_deref(), Some("MyProject"));
    assert!(plan.pages.iter().all(|p| p.title != "MyProject"), "wrapper removed");
    assert_eq!(parent_title(&plan, "a"), None);
    assert_eq!(parent_title(&plan, "sub"), None);
    assert_eq!(parent_title(&plan, "b"), Some("sub"));
  }

  // Multiple top-level items (a Mica export: loose pages + folders at root) are
  // NOT collapsed → round-trip identity for our own archives.
  #[test]
  fn new_workspace_keeps_multi_top_level() {
    let raw = vec![e("a.md", "# a\n\nhi"), e("F/b.md", "# b\n\nyo")];
    let plan = plan_import(raw, false, ImportMode::NewWorkspace);
    assert_eq!(plan.wrapper_name, None);
    assert_eq!(parent_title(&plan, "a"), None);
    assert_eq!(parent_title(&plan, "F"), None);
    assert_eq!(parent_title(&plan, "b"), Some("F"));
  }

  // A lone top-level DOCUMENT is never collapsed (it IS the content).
  #[test]
  fn new_workspace_single_document_not_collapsed() {
    let plan = plan_import(vec![e("solo.md", "# solo\n\nhi")], false, ImportMode::NewWorkspace);
    assert_eq!(plan.wrapper_name, None);
    assert_eq!(parent_title(&plan, "solo"), None);
  }

  // Into-existing import of loose files wraps them in ONE container named after
  // the source (the given fallback).
  #[test]
  fn into_container_wraps_loose_files() {
    let plan = plan_import(
      vec![e("a.md", "# a"), e("b.md", "# b")],
      false,
      ImportMode::IntoContainer("notes".into()),
    );
    let notes = find(&plan, "notes");
    assert!(notes.is_folder && notes.parent.is_none());
    assert_eq!(parent_title(&plan, "a"), Some("notes"));
    assert_eq!(parent_title(&plan, "b"), Some("notes"));
  }

  // Into-existing import of an archive with a single wrapper uses the WRAPPER's
  // name for the container (no `fallback > wrapper` double-nesting — SiYuan bug).
  #[test]
  fn into_container_collapses_then_wraps_no_double_nest() {
    let plan = plan_import(
      vec![e("MyProject/a.md", "# a"), e("MyProject/sub/b.md", "# b")],
      false,
      ImportMode::IntoContainer("backup".into()),
    );
    let tops: Vec<&str> =
      plan.pages.iter().filter(|p| p.parent.is_none()).map(|p| p.title.as_str()).collect();
    assert_eq!(tops, vec!["MyProject"], "single container = wrapper name, not fallback");
    assert!(plan.pages.iter().all(|p| p.title != "backup"));
    assert_eq!(parent_title(&plan, "a"), Some("MyProject"));
    assert_eq!(parent_title(&plan, "sub"), Some("MyProject"));
    assert_eq!(parent_title(&plan, "b"), Some("sub"));
  }

  // ── ImportMode::Auto — smart default (peel single wrapper / wrap many) ──

  // The user's case: one folder with subfolders, imported into an existing
  // workspace. Auto peels the single wrapper → the subfolders spill to the
  // destination root (outcome X), NOT re-wrapped under the folder's name.
  #[test]
  fn auto_spills_single_wrapper_to_root() {
    let raw = vec![e("MyFolder/SubA/a.md", "# a"), e("MyFolder/SubB/b.md", "# b")];
    let plan = plan_import(raw, false, ImportMode::Auto("MyFolder.zip".into()));
    // Wrapper peeled: no "MyFolder" / no fallback container at the top.
    assert!(plan.pages.iter().all(|p| p.title != "MyFolder" && p.title != "MyFolder.zip"));
    assert_eq!(parent_title(&plan, "SubA"), None, "SubA at root");
    assert_eq!(parent_title(&plan, "SubB"), None, "SubB at root");
    assert_eq!(parent_title(&plan, "a"), Some("SubA"));
  }

  // Multiple loose roots (no shared wrapper): Auto wraps them under the fallback
  // so they don't scatter across the destination root.
  #[test]
  fn auto_wraps_multiple_loose_roots() {
    let plan = plan_import(
      vec![e("a.md", "# a"), e("b.md", "# b")],
      false,
      ImportMode::Auto("notes".into()),
    );
    let notes = find(&plan, "notes");
    assert!(notes.is_folder && notes.parent.is_none());
    assert_eq!(parent_title(&plan, "a"), Some("notes"));
    assert_eq!(parent_title(&plan, "b"), Some("notes"));
  }

  // A lone top-level entry is never wrapped (nothing to declutter).
  #[test]
  fn auto_spills_single_entry() {
    let plan = plan_import(vec![e("solo.md", "# solo")], false, ImportMode::Auto("z".into()));
    assert!(plan.pages.iter().all(|p| p.title != "z"));
    assert_eq!(parent_title(&plan, "solo"), None);
  }

  // ── ImportMode::IntoLocation — forced spill (user override) ──

  // Forced spill: even multiple loose roots land directly at the destination,
  // no container — the override the smart default would have wrapped.
  #[test]
  fn into_location_spills_multiple_roots() {
    let plan = plan_import(
      vec![e("a.md", "# a"), e("b.md", "# b")],
      false,
      ImportMode::IntoLocation,
    );
    assert!(plan.pages.iter().all(|p| !p.is_folder), "no container created");
    assert_eq!(parent_title(&plan, "a"), None);
    assert_eq!(parent_title(&plan, "b"), None);
  }

  // A page that HAS subpages (Doc.md + Doc/Child.md) cannot exist in Mica —
  // folders hold, pages are leaves. It splits into a folder that keeps the name
  // and the children, plus a same-named leaf page holding the body. Nothing is
  // lost: the parent's text and the child both survive, one level deeper.
  #[test]
  fn page_with_subpages_splits_into_folder_plus_leaf() {
    let raw = vec![
      e("manifest.json", &manifest(&[
        ("Doc.md", "Doc", "document"),
        ("Doc/Child.md", "Child", "document"),
      ])),
      e("Doc.md", "# Doc\n\nparent body"),
      e("Doc/Child.md", "# Child\n\nchild body"),
    ];
    let plan = plan_import(raw, false, ImportMode::AsIs);
    let folder = plan.pages.iter().find(|p| p.title == "Doc" && p.is_folder).expect("folder");
    assert!(folder.parent.is_none() && folder.markdown.is_empty());
    let leaf = plan.pages.iter().find(|p| p.title == "Doc" && !p.is_folder).expect("leaf");
    assert_eq!(leaf.markdown.trim(), "# Doc

parent body");
    assert_eq!(leaf.parent.map(|i| &plan.pages[i]).map(|p| p.title.as_str()), Some("Doc"));
    assert_eq!(parent_title(&plan, "Child"), Some("Doc"));
    // The child hangs off the FOLDER, not off the leaf page.
    let child_parent = find(&plan, "Child").parent.expect("has parent");
    assert!(plan.pages[child_parent].is_folder, "child nests under the folder");
    // Invariant: no page ever has a page as its parent.
    assert_no_page_under_page(&plan);
  }

  // A parent page with subpages but NO body of its own is just a folder — don't
  // leave a stray empty page behind (Notion's "index" pages are usually empty).
  #[test]
  fn empty_page_with_subpages_becomes_only_a_folder() {
    let raw = vec![
      e("Doc 1234567890abcdef1234567890abcdef.md", "# Doc\n"),
      e("Doc 1234567890abcdef1234567890abcdef/Child 0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f.md",
        "# Child\n\nchild body"),
    ];
    let plan = plan_import(raw, true, ImportMode::AsIs);
    let docs: Vec<&PagePlan> = plan.pages.iter().filter(|p| p.title == "Doc").collect();
    assert_eq!(docs.len(), 1, "one node named Doc, not a folder + empty page");
    assert!(docs[0].is_folder);
    assert_eq!(parent_title(&plan, "Child"), Some("Doc"));
    assert_no_page_under_page(&plan);
  }

  /// The model invariant this whole split exists to uphold.
  fn assert_no_page_under_page(plan: &ImportPlan) {
    for p in &plan.pages {
      if let Some(parent) = p.parent {
        assert!(
          plan.pages[parent].is_folder,
          "'{}' nests under the page '{}' — pages are leaves",
          p.title,
          plan.pages[parent].title,
        );
      }
    }
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
    let plan = plan_import(raw, false, ImportMode::AsIs);
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
    let plan = plan_import(raw, false, ImportMode::AsIs);
    let section = find(&plan, "Section");
    assert!(section.is_folder, "a bare directory imports as a folder");
    assert!(section.archive_path.is_none() && section.markdown.is_empty());
    assert_eq!(parent_title(&plan, "Note"), Some("Section"));
    assert!(find(&plan, "Appendix").is_folder);
    assert_eq!(parent_title(&plan, "Refs"), Some("Appendix"));
  }
}
