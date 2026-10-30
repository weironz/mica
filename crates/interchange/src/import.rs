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
  /// The sidebar emoji, from a manifest-v2 `icon`. None for v1 archives, for
  /// foreign archives, and for synthetic folders we invent (a Mica export
  /// states its folders explicitly, so those never need one).
  pub icon: Option<String>,
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

pub fn plan_import(
  raw: Vec<ZipFileEntry>,
  notion_hint: bool,
  mode: ImportMode,
) -> Result<ImportPlan, crate::zip::ZipReadError> {
  let archive = normalize_archive(raw)?;
  Ok(plan_core(
    archive.entries,
    notion_hint,
    archive.wrapper_name,
    mode,
  ))
}

/// An archive's entries after the archive-only normalizations, ready for the
/// plan core.
#[derive(Debug)]
pub struct NormalizedArchive {
  pub entries: Vec<ZipFileEntry>,
  /// The single top-level wrapper directory that was peeled away, if any.
  pub wrapper_name: Option<String>,
}

impl NormalizedArchive {
  /// How many entries would become pages. Zero means the import would create
  /// nothing — a caller can refuse before making an empty workspace.
  pub fn markdown_count(&self) -> usize {
    self
      .entries
      .iter()
      .filter(|e| is_markdown_path(&e.name))
      .count()
  }
}

/// The archive half of [`plan_import`]: junk filtering, the single-wrapper
/// peel and nested-zip expansion. Split out so the desktop's LOCAL zip import
/// (which plans through [`plan_vault_import`] on the device) reads an archive
/// exactly the way the server does, instead of growing a second copy.
pub fn normalize_archive(
  raw: Vec<ZipFileEntry>,
) -> Result<NormalizedArchive, crate::zip::ZipReadError> {
  // `normalize_entries` already peels a single top-level wrapper folder
  // (Anytype-style collapse); capture its name FIRST so we can name a new
  // workspace / import container after it (the peel itself discards the name).
  let wrapper_name = crate::normalize::stripped_wrapper_name(&raw);
  let entries = expand_nested_zips(normalize_entries(raw))?;
  Ok(NormalizedArchive {
    entries,
    wrapper_name,
  })
}

/// Plan a walked DIRECTORY tree (the desktop "open my vault" folder picker,
/// issue #84), as opposed to an archive. Same plan core as an archive import,
/// minus the two normalizations only archives need: wrapper peeling (the
/// folder the user picked IS the container — a single top-level dir inside it
/// is real content, not packaging) and nested-zip expansion (a `.zip` in a
/// vault is a vault file — an attachment, not an archive). Junk filtering
/// stays: hidden/dot segments are hidden on-disk too.
pub fn plan_vault_import(entries: Vec<ZipFileEntry>) -> ImportPlan {
  let entries = entries
    .into_iter()
    .filter(|e| !crate::normalize::is_junk(&e.name))
    .collect();
  plan_core(entries, false, None, ImportMode::AsIs)
}

/// The shared planner behind [`plan_import`] (archives) and
/// [`plan_vault_import`] (walked directories). `entries` must already be
/// normalized for its source (see the callers); `wrapper_name` is the peeled
/// single top-level dir an archive had, `None` otherwise.
fn plan_core(
  entries: Vec<ZipFileEntry>,
  notion_hint: bool,
  wrapper_name: Option<String>,
  mode: ImportMode,
) -> ImportPlan {
  let manifest = entries
    .iter()
    .find(|e| e.name == "manifest.json")
    .map(|e| String::from_utf8_lossy(&e.bytes).into_owned());

  let mut files: HashMap<String, Vec<u8>> = HashMap::new();
  let mut mds: HashMap<String, String> = HashMap::new();
  for e in entries {
    if is_markdown_path(&e.name) {
      mds.insert(e.name, String::from_utf8_lossy(&e.bytes).into_owned());
    } else if e.name != "manifest.json" && !is_block_json(&e.name) {
      // Everything left over is an attachment — which is why the structural
      // files have to be excluded BY NAME here. `<page>.mica.json` (manifest v2)
      // would otherwise be uploaded as an image beside the page it describes.
      files.insert(e.name, e.bytes);
    }
  }

  let notion = notion_hint || looks_like_notion_export(mds.keys().map(String::as_str));
  // Our OWN archives now lead each page's text with `# <title>` (2026-08-27, see
  // `docs/features/page-title-plan.md`), so importing one has to take that line back off
  // or every export→import cycle stacks another heading — the round-trip red
  // line (CLAUDE.md #4).
  //
  // Keyed on the manifest's `generator`, not on "there happens to be a
  // manifest": a hand-built or third-party archive with a manifest is not ours,
  // and its leading heading is the author's content.
  let mica_export = manifest
    .as_deref()
    .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
    .and_then(|m| {
      m.get("generator")
        .and_then(|g| g.as_str())
        .map(|g| g == "mica")
    })
    .unwrap_or(false);
  let clean = |s: &str| -> String {
    if notion {
      strip_notion_id(s).to_string()
    } else {
      s.to_string()
    }
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
  // (spaces/punctuation collapse to `_` in the path).
  //
  // Documents need the same, for the same reason: `Meeting notes` is exported as
  // `Meeting_notes.md`, a second `Same` as `Same-2.md`. Reading the name off the
  // file renamed the page on every round trip, and then the `# Meeting notes`
  // the export wrote no longer matched that name and stayed in the body too.
  // Only for OUR archives (`mica_export`): a foreign manifest's `title` is not
  // known to mean what ours does, and foreign behaviour is unchanged.
  //
  // 〔An earlier version of this comment said documents round-trip "through the
  // manifest `title` and the file name". The file name half was never true for
  // any name `safe_segment` rewrites, and nothing read the manifest half.〕
  let folder_titles: HashMap<String, String> = entries
    .iter()
    .filter(|e| e.is_folder)
    .filter_map(|e| e.title.clone().map(|t| (e.path.clone(), t)))
    .collect();
  let doc_titles: HashMap<String, String> = if mica_export {
    entries
      .iter()
      .filter(|e| !e.is_folder)
      .filter_map(|e| {
        let title = e.title.as_deref()?.trim();
        (!title.is_empty()).then(|| (e.path.clone(), title.to_string()))
      })
      .collect()
  } else {
    HashMap::new()
  };
  // Icons for BOTH kinds, keyed by the manifest's own path (a folder's is its
  // directory, a document's is its `.md`) — the two never collide, so one map
  // serves both lookups below.
  let icons: HashMap<String, String> = entries
    .iter()
    .filter_map(|e| e.icon.clone().map(|i| (e.path.clone(), i)))
    .collect();
  let mut ordered: Vec<(String, bool)> = Vec::new();
  let mut queued: HashSet<String> = HashSet::new();
  for e in entries {
    if e.is_folder {
      if queued.insert(e.path.clone()) {
        ordered.push((e.path, true));
      }
    } else if mds.contains_key(&e.path) && queued.insert(e.path.clone()) {
      ordered.push((e.path, false));
    }
  }
  for path in order_page_paths(manifest_docs, manifest.as_deref()) {
    if queued.insert(path.clone()) {
      ordered.push((path, false));
    }
  }

  // Directory identity. `clean` strips a Notion id suffix from each segment, so
  // a folder `Doc <id-a>/` and its page `Doc <id-b>.md` normally share one
  // cleaned name and must pair into one folder-plus-leaf. But when TWO distinct
  // pages claim that cleaned name — whether via two child directories (issue
  // #151) or via one page's own directory plus another page that has none —
  // pairing them into one folder would merge unrelated subtrees, so each is
  // keyed by its RAW path instead. A lone differing-id pair still unites.
  let clean_path =
    |path: &str| -> String { path.split('/').map(&clean).collect::<Vec<_>>().join("/") };
  // Raw directories and document stems that collapse to each cleaned name.
  let mut dir_claims: HashMap<String, Vec<String>> = HashMap::new();
  let mut doc_claims: HashMap<String, Vec<String>> = HashMap::new();
  for (path, is_folder) in &ordered {
    let parts: Vec<&str> = path.split('/').collect();
    let chain = if *is_folder {
      parts.len()
    } else {
      parts.len().saturating_sub(1)
    };
    for k in 1..=chain {
      let raw = parts[..k].join("/");
      dir_claims.entry(clean_path(&raw)).or_default().push(raw);
    }
    if !*is_folder {
      let stem = strip_md_ext(path);
      doc_claims
        .entry(clean_path(stem))
        .or_default()
        .push(stem.to_string());
    }
  }
  for list in dir_claims.values_mut() {
    list.sort();
    list.dedup();
  }
  for list in doc_claims.values_mut() {
    list.sort();
    list.dedup();
  }
  // Contested names include the document stems; single-owner names keep only
  // the directory, which is what lets a lone differing-id pair pair up.
  let mut raw_by_clean = dir_claims;
  for (name, docs) in doc_claims {
    let dirs = raw_by_clean.get(&name).map_or(0, |v| v.len());
    if dirs > 1 || docs.len() > 1 {
      raw_by_clean.entry(name).or_default().extend(docs);
    }
  }
  for list in raw_by_clean.values_mut() {
    list.sort();
    list.dedup();
  }
  let dir_key = |raw: &str| -> String {
    match raw_by_clean.get(&clean_path(raw)) {
      Some(rs) if rs.len() > 1 => format!("raw:{raw}"),
      _ => format!("clean:{}", clean_path(raw)),
    }
  };
  // Every directory (by identity) that has something under it. A `.md` whose
  // own directory is in here is a page-WITH-subpages, which Mica's model has no
  // room for (folders are pure containers, pages are leaves) — see the split.
  let mut dirs_with_children: HashSet<String> = HashSet::new();
  for (path, is_folder) in &ordered {
    let parts: Vec<&str> = path.split('/').collect();
    let chain = if *is_folder {
      parts.len()
    } else {
      parts.len().saturating_sub(1)
    };
    for k in 1..=chain {
      dirs_with_children.insert(dir_key(&parts[..k].join("/")));
    }
  }

  let mut pages: Vec<PagePlan> = Vec::new();
  let mut page_by_path: HashMap<String, usize> = HashMap::new();
  // Keyed by directory identity (see `dir_key`): the cleaned name in the common
  // case, the raw path when several raw directories share one cleaned name.
  let mut folder_page: HashMap<String, usize> = HashMap::new();

  for (path, is_folder) in &ordered {
    let parts: Vec<&str> = path.split('/').collect();
    // For a folder entry the whole path is the folder chain; for a document the
    // ancestors are folders and the last segment is the file.
    let chain = if *is_folder {
      parts.len()
    } else {
      parts.len().saturating_sub(1)
    };

    let mut parent: Option<usize> = None;
    let mut folder_path = String::new();
    for &seg_raw in parts.iter().take(chain) {
      if !folder_path.is_empty() {
        folder_path.push('/');
      }
      folder_path.push_str(seg_raw);
      let seg = clean(seg_raw);
      let key = dir_key(&folder_path);

      // A directory is ALWAYS a folder page. It used to map onto the `.md` that
      // shares its name (`Doc.md` + `Doc/…` → one document that also had
      // children), which produced trees Mica cannot represent: a page is a leaf.
      // The matching `.md` now becomes a leaf page INSIDE this folder instead.
      let idx = match folder_page.get(&key).copied() {
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
            icon: icons.get(&folder_path).cloned(),
          });
          let idx = pages.len() - 1;
          folder_page.insert(key, idx);
          idx
        }
      };
      parent = Some(idx);
    }
    // page). A document entry adds its own page under the walked ancestors.
    if *is_folder {
      continue;
    }
    let markdown = &mds[path];
    let base = parts.last().unwrap_or(&"");
    // The page's name as the archive's PATH spells it. It keys the tree (the
    // page-with-subpages split below matches it against directory names), so it
    // stays the path's spelling even when the display name differs.
    let stem = clean(strip_md_ext(base));
    // The display name: the manifest `title` for our own archives (see
    // `doc_titles`), the file name for everything else. A leading
    // `# <that name>` is stripped in exactly two cases, and left alone in every
    // other:
    //
    // - **Notion exports**, which put the title in the file name AND repeat it
    //   as the body's first `# H1` (which is why every third-party Notion
    //   importer ships a "remove duplicate title" step). Importing that verbatim
    //   would show the title twice on every page.
    // - **Our own exports**, since 2026-08-27 — they now write the title into
    //   the text too (`docs/features/page-title-plan.md`), so this is the other half of
    //   that change. Without it every export→import cycle stacks one more
    //   heading, which is the round-trip red line (CLAUDE.md #4).
    //   〔This reverses the rule that used to live here: "a page's name is a
    //   property of the page, not a line inside it, and our own export writes
    //   the name into the file name, never into the text." The name is still a
    //   property — it just travels inside the document now, so that every reader
    //   of the document sees it. See the plan's §7.〕
    //
    // A foreign archive (no manifest, or a manifest some other tool wrote) is
    // NOT stripped: its leading heading is the author's content. And the match
    // is exact in both cases, so a heading that merely happens to lead the page
    // survives. The heading is looked for after any front matter, which is
    // where `with_page_title` writes it; `mica_markdown::strip_page_title` is
    // its inverse and lives beside it so the two cannot drift.
    let title = doc_titles
      .get(path)
      .cloned()
      .unwrap_or_else(|| stem.clone());
    let body = if notion || mica_export {
      mica_markdown::strip_page_title(markdown, &title)
    } else {
      markdown.clone()
    };

    // A page that has subpages (Notion's native shape: `Doc.md` next to a
    // `Doc/` holding its children). Mica has no such node — folders hold, pages
    // are leaves — so split it: the folder keeps the name and the children, and
    // the body moves into a same-named leaf page inside it. A parent with no
    // body of its own is just a folder; don't leave an empty page behind.
    // Matched by RAW directory identity (see `dir_key` above): the doc's own
    // directory, spelled as the archive spells it, must be the SAME identity as
    // the child directory chain — so `Doc <id-a>.md` pairs with `Doc <id-a>/`
    // even when a same-named sibling page exists.
    let own_key = dir_key(strip_md_ext(path));
    if dirs_with_children.contains(&own_key) {
      let folder_idx = match folder_page.get(&own_key).copied() {
        Some(idx) => idx,
        None => {
          pages.push(PagePlan {
            archive_path: None,
            title: title.clone(),
            parent,
            markdown: String::new(),
            is_folder: true,
            icon: None,
          });
          let idx = pages.len() - 1;
          folder_page.insert(own_key.clone(), idx);
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
      icon: icons.get(path).cloned(),
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
    icon: None,
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

/// One `pages[]` row of the export manifest.
struct ManifestEntry {
  path: String,
  is_folder: bool,
  /// The entry's original display name — used to restore folder names, which
  /// have no `.md`/H1 to recover from.
  title: Option<String>,
  /// v2 only. v1 archives have no icon anywhere, so None is the honest answer
  /// rather than a default.
  icon: Option<String>,
}

/// Parse the export `manifest.json` in listed (pre-order) order. Entries
/// without a `type` field default to document (v1 manifests, pre-folder).
/// Empty/absent manifest → no entries.
///
/// Reads v1 and v2 with the same code: v2 only ADDED fields
/// (`id`/`parent_id`/`position`/`icon`), so every v1 key still means what it
/// meant. The version number is therefore not branched on — doing so would
/// invent a second parser for one format.
fn manifest_entries(manifest_json: Option<&str>) -> Vec<ManifestEntry> {
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
      // Absent OR empty both mean "no icon": the export omits the key when the
      // page has none, but a hand-edited archive can carry `"icon": ""`, and an
      // empty emoji would render as a blank slot rather than as no icon.
      let icon = p
        .get("icon")
        .and_then(|t| t.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());
      Some(ManifestEntry {
        path,
        is_folder,
        title,
        icon,
      })
    })
    .collect()
}

/// `name` without its `.md` extension, in any case (`.md`, `.MD`, `.Md`, `.mD`)
/// — the same set [`is_markdown_path`] accepts, so a file that is read as
/// Markdown never keeps its extension in the page name.
pub(crate) fn strip_md_ext(name: &str) -> &str {
  match name
    .len()
    .checked_sub(3)
    .and_then(|at| name.get(at..).map(|ext| (at, ext)))
  {
    Some((at, ext)) if ext.eq_ignore_ascii_case(".md") => &name[..at],
    _ => name,
  }
}

/// Whether an archive entry is a Markdown page (by extension, any case).
fn is_markdown_path(name: &str) -> bool {
  strip_md_ext(name).len() != name.len()
}

/// Resolve a Markdown reference (`../assets/图 1.png`, `Page%20<id>.md`)
/// found inside [from_file] to an archive path in [paths]. Returns None when
/// the reference is external (has a URL scheme) or matches nothing. Tries
/// the file's own folder first, then the archive root, then — only if the path
/// structure doesn't line up — a UNIQUE basename anywhere in the archive.
/// Whether `s` is a Typora image-size token: `100%`, `629`, `800x600`.
///
/// Deliberately narrow. The alternative — cutting at the first space — would
/// mangle every filename with a space in it, and archives are full of those
/// ("Screen Shot 2024.png"). A wrong strip is worse than a missed one: it turns
/// a resolvable reference into an unresolvable one.
fn is_size_suffix(s: &str) -> bool {
  let s = s.trim();
  if s.is_empty() {
    return false;
  }
  let core = s.strip_suffix('%').unwrap_or(s);
  match core.split_once('x') {
    Some((w, h)) => {
      !w.is_empty()
        && !h.is_empty()
        && w.bytes().all(|b| b.is_ascii_digit())
        && h.bytes().all(|b| b.is_ascii_digit())
    }
    None => !core.is_empty() && core.bytes().all(|b| b.is_ascii_digit()),
  }
}

pub fn resolve_ref(from_file: &str, href: &str, paths: &HashSet<String>) -> Option<String> {
  let mut u = href.trim();
  if u.is_empty() || has_scheme(u) {
    return None;
  }
  u = u.split(['#', '?']).next().unwrap_or(u);
  // Typora-style size suffix: `![](img.png =100%)`, `=629`, `=800x600`. It is
  // NOT CommonMark — the whole `img.png =100%` arrives here as one "url", so
  // the path match AND the basename fallback both miss, and the image is
  // dropped as "not in the archive". Found in real data: 8 images across 10
  // pages had come in that way, each leaving behind a relative link no client
  // can resolve. Stripped only when what follows the space actually looks like
  // a size, so a filename that genuinely contains " =" keeps it.
  if let Some((head, size)) = u.rsplit_once(" =")
    && !head.trim().is_empty()
    && is_size_suffix(size)
  {
    u = head.trim_end();
  }
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
  let Some(colon) = s.find(':') else {
    return false;
  };
  let prefix = &s[..colon];
  if let Some(slash) = s.find('/')
    && slash < colon
  {
    return false;
  }
  !prefix.is_empty()
    && prefix
      .chars()
      .next()
      .is_some_and(|c| c.is_ascii_alphabetic())
    && prefix
      .chars()
      .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
}

/// Decode `%XX` escapes back to a UTF-8 string; leave malformed escapes alone.
///
/// Byte-based on purpose: a `%` followed by a multi-byte char must stay
/// literal, and slicing the `str` there would not be on a char boundary.
pub fn percent_decode(s: &str) -> String {
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

/// The block-model sidecar a manifest-v2 export writes beside each `<page>.md`.
///
/// Named as a suffix rather than listed in the manifest on purpose: the check
/// has to work before the manifest is parsed (and when there is none), because
/// getting it wrong does not fail — it silently uploads the file as an
/// attachment on the page it was describing.
fn is_block_json(name: &str) -> bool {
  name.to_lowercase().ends_with(".mica.json")
}

#[cfg(test)]
mod tests {
  use super::*;

  fn e(name: &str, bytes: &str) -> ZipFileEntry {
    ZipFileEntry {
      name: name.to_string(),
      bytes: bytes.as_bytes().to_vec(),
    }
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
    assert_eq!(
      resolve_ref("note.md", "photo.png", &ps),
      Some("photo.png".to_string())
    );
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
    plan
      .pages
      .iter()
      .find(|p| p.title == title)
      .expect("page present")
  }
  fn parent_title<'a>(plan: &'a ImportPlan, title: &str) -> Option<&'a str> {
    find(plan, title)
      .parent
      .map(|i| plan.pages[i].title.as_str())
  }

  // A pure folder with a child document (the new Mica export shape): the folder
  // imports as a folder page (no content), the child nests under it.
  #[test]
  fn folder_with_child_imports_as_folder() {
    let raw = vec![
      e(
        "manifest.json",
        &manifest(&[
          ("Chapter", "Chapter", "folder"),
          ("Chapter/Intro.md", "Intro", "document"),
        ]),
      ),
      e("Chapter/Intro.md", "# Intro\n\nhello"),
    ];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    let chapter = find(&plan, "Chapter");
    assert!(chapter.is_folder, "Chapter is a folder");
    assert!(chapter.archive_path.is_none() && chapter.markdown.is_empty());
    let intro = find(&plan, "Intro");
    assert!(!intro.is_folder);
    // The manifest says `generator: mica`, so the leading `# Intro` is the
    // title OUR export wrote and it comes back off — otherwise every
    // export→import cycle stacks another one. (Before 2026-08-27 this asserted
    // the opposite, because our export did not write it; see
    // `docs/features/page-title-plan.md` §7.) The name still comes from the file name.
    assert_eq!(intro.markdown.trim(), "hello");
    assert_eq!(parent_title(&plan, "Intro"), Some("Chapter"));
  }

  // The stripping above is narrow on purpose. These three pin the edges of it —
  // each one is a way to silently eat somebody's content.

  // A leading heading that says something ELSE is the author's writing, even in
  // our own archive. Exact match, nothing looser.
  #[test]
  fn a_differently_named_leading_heading_survives_our_own_import() {
    let raw = vec![
      e(
        "manifest.json",
        &manifest(&[("Intro.md", "Intro", "document")]),
      ),
      e("Intro.md", "# 别的标题\n\nhello"),
    ];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    assert_eq!(find(&plan, "Intro").markdown.trim(), "# 别的标题\n\nhello");
  }

  // Same text, same name — but nobody said this archive is ours. A hand-built
  // or third-party zip's first heading is content, and stripping it would be
  // data loss with no error.
  #[test]
  fn a_foreign_archive_keeps_its_leading_heading() {
    let raw = vec![e("Intro.md", "# Intro\n\nhello")];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    assert_eq!(find(&plan, "Intro").markdown.trim(), "# Intro\n\nhello");
  }

  // A manifest written by some OTHER tool is not ours either. The flag is the
  // `generator` field, not the mere presence of a manifest.
  #[test]
  fn a_manifest_from_another_generator_does_not_enable_stripping() {
    let foreign = serde_json::json!({
      "version": 1,
      "generator": "someone-else",
      "pages": [{"path": "Intro.md", "title": "Intro", "type": "document"}],
    })
    .to_string();
    let raw = vec![
      e("manifest.json", &foreign),
      e("Intro.md", "# Intro\n\nhello"),
    ];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    assert_eq!(find(&plan, "Intro").markdown.trim(), "# Intro\n\nhello");
  }

  // An EMPTY folder (only in the manifest, no `.md`, no children) survives the
  // round-trip — this is what the manifest folder entry buys us.
  #[test]
  fn empty_folder_survives_via_manifest() {
    let raw = vec![e(
      "manifest.json",
      &manifest(&[("Empty", "Empty", "folder")]),
    )];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    assert_eq!(plan.pages.len(), 1);
    let empty = find(&plan, "Empty");
    assert!(empty.is_folder && empty.parent.is_none());
  }

  // ── ImportMode: container vs flatten (P1) ──────────────────────────────

  // New-workspace import strips a single top-level wrapper (the workspace IS the
  // container) and reports its name for naming the workspace.
  #[test]
  fn new_workspace_collapses_single_wrapper() {
    let raw = vec![
      e("MyProject/a.md", "# a\n\nhi"),
      e("MyProject/sub/b.md", "# b\n\nyo"),
    ];
    let plan = plan_import(raw, false, ImportMode::NewWorkspace).unwrap();
    assert_eq!(plan.wrapper_name.as_deref(), Some("MyProject"));
    assert!(
      plan.pages.iter().all(|p| p.title != "MyProject"),
      "wrapper removed"
    );
    assert_eq!(parent_title(&plan, "a"), None);
    assert_eq!(parent_title(&plan, "sub"), None);
    assert_eq!(parent_title(&plan, "b"), Some("sub"));
  }

  // Multiple top-level items (a Mica export: loose pages + folders at root) are
  // NOT collapsed → round-trip identity for our own archives.
  #[test]
  fn new_workspace_keeps_multi_top_level() {
    let raw = vec![e("a.md", "# a\n\nhi"), e("F/b.md", "# b\n\nyo")];
    let plan = plan_import(raw, false, ImportMode::NewWorkspace).unwrap();
    assert_eq!(plan.wrapper_name, None);
    assert_eq!(parent_title(&plan, "a"), None);
    assert_eq!(parent_title(&plan, "F"), None);
    assert_eq!(parent_title(&plan, "b"), Some("F"));
  }

  // A lone top-level DOCUMENT is never collapsed (it IS the content).
  #[test]
  fn new_workspace_single_document_not_collapsed() {
    let plan = plan_import(
      vec![e("solo.md", "# solo\n\nhi")],
      false,
      ImportMode::NewWorkspace,
    )
    .unwrap();
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
    )
    .unwrap();
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
    )
    .unwrap();
    let tops: Vec<&str> = plan
      .pages
      .iter()
      .filter(|p| p.parent.is_none())
      .map(|p| p.title.as_str())
      .collect();
    assert_eq!(
      tops,
      vec!["MyProject"],
      "single container = wrapper name, not fallback"
    );
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
    let raw = vec![
      e("MyFolder/SubA/a.md", "# a"),
      e("MyFolder/SubB/b.md", "# b"),
    ];
    let plan = plan_import(raw, false, ImportMode::Auto("MyFolder.zip".into())).unwrap();
    // Wrapper peeled: no "MyFolder" / no fallback container at the top.
    assert!(
      plan
        .pages
        .iter()
        .all(|p| p.title != "MyFolder" && p.title != "MyFolder.zip")
    );
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
    )
    .unwrap();
    let notes = find(&plan, "notes");
    assert!(notes.is_folder && notes.parent.is_none());
    assert_eq!(parent_title(&plan, "a"), Some("notes"));
    assert_eq!(parent_title(&plan, "b"), Some("notes"));
  }

  // A lone top-level entry is never wrapped (nothing to declutter).
  #[test]
  fn auto_spills_single_entry() {
    let plan = plan_import(
      vec![e("solo.md", "# solo")],
      false,
      ImportMode::Auto("z".into()),
    )
    .unwrap();
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
    )
    .unwrap();
    assert!(
      plan.pages.iter().all(|p| !p.is_folder),
      "no container created"
    );
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
      e(
        "manifest.json",
        &manifest(&[
          ("Doc.md", "Doc", "document"),
          ("Doc/Child.md", "Child", "document"),
        ]),
      ),
      e("Doc.md", "# Doc\n\nparent body"),
      e("Doc/Child.md", "# Child\n\nchild body"),
    ];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    let folder = plan
      .pages
      .iter()
      .find(|p| p.title == "Doc" && p.is_folder)
      .expect("folder");
    assert!(folder.parent.is_none() && folder.markdown.is_empty());
    let leaf = plan
      .pages
      .iter()
      .find(|p| p.title == "Doc" && !p.is_folder)
      .expect("leaf");
    // `# Doc` is the title our own export wrote (manifest `generator: mica`),
    // so it is stripped and only the body remains — see
    // `folder_with_child_imports_as_folder` for why this flipped.
    assert_eq!(leaf.markdown.trim(), "parent body");
    assert_eq!(
      leaf
        .parent
        .map(|i| &plan.pages[i])
        .map(|p| p.title.as_str()),
      Some("Doc")
    );
    assert_eq!(parent_title(&plan, "Child"), Some("Doc"));
    // The child hangs off the FOLDER, not off the leaf page.
    let child_parent = find(&plan, "Child").parent.expect("has parent");
    assert!(
      plan.pages[child_parent].is_folder,
      "child nests under the folder"
    );
    // Invariant: no page ever has a page as its parent.
    assert_no_page_under_page(&plan);
  }

  /// The export/import pair, closed end to end and byte for byte.
  ///
  /// Export writes `# {title}` into the text (`mica_markdown::with_page_title`);
  /// import takes exactly that line back off (`strip_page_title`) and the caller
  /// puts the title on the document's root instead
  /// (`mica_markdown::set_document_title`, done in `api-server`'s importer).
  /// Each half is pinned on its own above; what this pins is that the two
  /// COMPOSE — a page exported, imported and exported again produces the same
  /// bytes, with one `# 季度回顾` and not two, and not zero.
  ///
  /// This is the loop CLAUDE.md #4 calls an invariant, and it is the only test
  /// that walks all of it: half the rule lives in `mica_markdown` and half here,
  /// so either half can be changed alone without any other test noticing.
  #[test]
  fn a_mica_export_survives_import_and_re_export_byte_for_byte() {
    let title = "季度回顾";
    // Exactly what the exporter puts in the archive: the rendered body with the
    // title line welded on top, and the file named after the title (our export
    // writes the name into the file name — that is where import reads it back).
    let source = mica_markdown::import_markdown("第一段。\n\n## 小节\n\n第二段。", "root");
    let exported = mica_markdown::with_page_title(
      &mica_markdown::export_markdown(&source).expect("export"),
      title,
    );
    assert!(
      exported.starts_with("# 季度回顾\n\n"),
      "precondition: {exported:?}"
    );

    let raw = vec![
      e(
        "manifest.json",
        &manifest(&[("季度回顾.md", title, "document")]),
      ),
      e("季度回顾.md", &exported),
    ];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    let page = find(&plan, title);
    assert!(!page.markdown.starts_with('#'), "the title line came off");

    // What the importer then builds and stores.
    let mut reimported = mica_markdown::import_markdown(&page.markdown, "root");
    assert!(mica_markdown::set_document_title(
      &mut reimported,
      &page.title
    ));

    // And what exporting THAT produces.
    let re_exported = mica_markdown::with_page_title(
      &mica_markdown::export_markdown(&reimported).expect("re-export"),
      mica_markdown::document_title(&reimported).expect("title travelled in the document"),
    );
    assert_eq!(
      re_exported, exported,
      "export → import → export is not stable"
    );
  }

  /// The same loop for a page whose body genuinely opens with a heading of its
  /// own. `strip_page_title` matches exactly, so that heading is content and
  /// must survive — and the title still has to come back exactly once, above it.
  #[test]
  fn a_body_that_opens_with_its_own_heading_round_trips_too() {
    let title = "周报";
    let source = mica_markdown::import_markdown("# 另一个标题\n\n正文。", "root");
    let exported = mica_markdown::with_page_title(
      &mica_markdown::export_markdown(&source).expect("export"),
      title,
    );

    let raw = vec![
      e(
        "manifest.json",
        &manifest(&[("周报.md", title, "document")]),
      ),
      e("周报.md", &exported),
    ];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    let page = find(&plan, title);
    assert!(
      page.markdown.trim_start().starts_with("# 另一个标题"),
      "content kept"
    );

    let mut reimported = mica_markdown::import_markdown(&page.markdown, "root");
    mica_markdown::set_document_title(&mut reimported, &page.title);
    let re_exported = mica_markdown::with_page_title(
      &mica_markdown::export_markdown(&reimported).expect("re-export"),
      mica_markdown::document_title(&reimported).expect("title"),
    );
    assert_eq!(re_exported, exported);
  }

  // A parent page with subpages but NO body of its own is just a folder — don't
  // leave a stray empty page behind (Notion's "index" pages are usually empty).
  #[test]
  fn empty_page_with_subpages_becomes_only_a_folder() {
    let raw = vec![
      e("Doc 1234567890abcdef1234567890abcdef.md", "# Doc\n"),
      e(
        "Doc 1234567890abcdef1234567890abcdef/Child 0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f.md",
        "# Child\n\nchild body",
      ),
    ];
    let plan = plan_import(raw, true, ImportMode::AsIs).unwrap();
    let docs: Vec<&PagePlan> = plan.pages.iter().filter(|p| p.title == "Doc").collect();
    assert_eq!(
      docs.len(),
      1,
      "one node named Doc, not a folder + empty page"
    );
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
          p.title, plan.pages[parent].title,
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
      e(
        "manifest.json",
        &manifest(&[
          ("My_Folder", "My Folder", "folder"),
          ("My_Folder/Note.md", "Note", "document"),
        ]),
      ),
      e("My_Folder/Note.md", "# Note\n\nbody"),
    ];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    let folder = find(&plan, "My Folder");
    assert!(
      folder.is_folder,
      "folder keeps its real name from the manifest"
    );
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
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    let section = find(&plan, "Section");
    assert!(section.is_folder, "a bare directory imports as a folder");
    assert!(section.archive_path.is_none() && section.markdown.is_empty());
    assert_eq!(parent_title(&plan, "Note"), Some("Section"));
    assert!(find(&plan, "Appendix").is_folder);
    assert_eq!(parent_title(&plan, "Refs"), Some("Appendix"));
  }

  /// Icons survive export -> import.
  ///
  /// THE reason the import side had to learn manifest v2: v0.13.44 started
  /// WRITING `icon` into the manifest and nothing read it, so an exported
  /// workspace came back with every page icon blank — silently, because a
  /// missing emoji looks exactly like a page that never had one.
  ///
  /// Built with the REAL exporter rather than a hand-written manifest, so the
  /// two halves cannot drift: rename the key the export writes and this fails
  /// here instead of in somebody's workspace.
  #[test]
  fn manifest_v2_icons_survive_export_then_import() {
    use crate::export_tree::{TreeNode, build_markdown_tree_zip};

    let node =
      |id: &str, parent: Option<&str>, name: &str, ty: &str, icon: Option<&str>| TreeNode {
        id: id.to_string(),
        parent_id: parent.map(str::to_string),
        position: "0000000010".to_string(),
        name: name.to_string(),
        object_type: ty.to_string(),
        object_id: format!("obj-{id}"),
        icon: icon.map(str::to_string),
      };
    let nodes = vec![
      node("f1", None, "归档", "folder", Some("📁")),
      node("p1", Some("f1"), "周报", "document", Some("😃")),
      // A sibling with NO icon: absent has to stay absent, not inherit.
      node("p2", Some("f1"), "日报", "document", None),
    ];
    let mut payloads = HashMap::new();
    for id in ["p1", "p2"] {
      payloads.insert(
        format!("obj-{id}"),
        mica_markdown::import_markdown("正文。", "root"),
      );
    }
    let raw: Vec<ZipFileEntry> = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new())
      .into_iter()
      .map(|z| ZipFileEntry {
        name: z.name,
        bytes: z.data,
      })
      .collect();
    assert!(
      raw.iter().any(|e| e.name == "manifest.json"),
      "precondition: the exporter wrote a manifest"
    );

    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    assert_eq!(
      find(&plan, "归档").icon.as_deref(),
      Some("📁"),
      "folder icon"
    );
    assert_eq!(
      find(&plan, "周报").icon.as_deref(),
      Some("😃"),
      "document icon"
    );
    assert_eq!(find(&plan, "日报").icon, None, "no icon stays no icon");
  }

  /// Export → import → export with the REAL exporter, for exactly the names the
  /// old loop test (`季度回顾`) could not see: ones `safe_segment` rewrites.
  ///
  /// The page name used to come from the file name, so `Meeting notes` came back
  /// as `Meeting_notes`, and the `# Meeting notes` the export wrote no longer
  /// matched it and stayed in the body — renamed AND doubled. The `-2` a
  /// duplicate sibling gets is the same failure without any punctuation at all.
  /// A page with properties lost differently: the title sits AFTER the front
  /// matter, and the stripper only looked at the very first line.
  #[test]
  fn names_the_file_name_cannot_carry_survive_export_then_import() {
    use crate::export_tree::{TreeNode, build_markdown_tree_zip};

    let node = |id: &str, parent: Option<&str>, name: &str, ty: &str| TreeNode {
      id: id.to_string(),
      parent_id: parent.map(str::to_string),
      position: format!("00000000{id:0>2}"),
      name: name.to_string(),
      object_type: ty.to_string(),
      object_id: format!("obj-{id}"),
      icon: None,
    };
    let nodes = vec![
      node("p1", None, "Meeting notes", "document"),
      node("p2", None, "Q&A: 2026/10", "document"),
      node("p3", None, "带属性的页", "document"),
      node("f1", None, "Notes", "folder"),
      node("p4", Some("f1"), "Same", "document"),
      node("p5", Some("f1"), "Same", "document"),
    ];
    let mut payloads = HashMap::new();
    for id in ["p1", "p2", "p4", "p5"] {
      payloads.insert(
        format!("obj-{id}"),
        mica_markdown::import_markdown("正文。", "root"),
      );
    }
    payloads.insert(
      "obj-p3".to_string(),
      mica_markdown::import_markdown("---\ntags: [a]\nstatus: draft\n---\n正文。", "root"),
    );
    let raw: Vec<ZipFileEntry> = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new())
      .into_iter()
      .map(|z| ZipFileEntry {
        name: z.name,
        bytes: z.data,
      })
      .collect();
    let exported: HashMap<String, String> = raw
      .iter()
      .map(|e| {
        (
          e.name.clone(),
          String::from_utf8_lossy(&e.bytes).into_owned(),
        )
      })
      .collect();
    // Preconditions: the names really were rewritten on the way out, so this
    // test is about the case the file name cannot carry.
    for path in ["Meeting_notes.md", "Q_A_2026_10.md", "Notes/Same-2.md"] {
      assert!(
        exported.contains_key(path),
        "precondition: {path} in {:?}",
        exported.keys()
      );
    }
    assert!(
      exported["带属性的页.md"].starts_with("---\ntags: [a]\nstatus: draft\n---\n# 带属性的页\n"),
      "precondition: the title sits after the front matter: {:?}",
      exported["带属性的页.md"]
    );

    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    let mut names: Vec<&str> = plan
      .pages
      .iter()
      .filter(|p| !p.is_folder)
      .map(|p| p.title.as_str())
      .collect();
    names.sort_unstable();
    assert_eq!(
      names,
      [
        "Meeting notes",
        "Q&A: 2026/10",
        "Same",
        "Same",
        "带属性的页"
      ]
    );

    for page in plan.pages.iter().filter(|p| !p.is_folder) {
      let path = page.archive_path.as_deref().expect("a document has a path");
      assert!(
        !page.markdown.contains(&format!("# {}", page.title)),
        "{path}: the title line came off: {:?}",
        page.markdown
      );
      // What the importer stores, and what exporting THAT produces.
      let mut reimported = mica_markdown::import_markdown(&page.markdown, "root");
      assert!(mica_markdown::set_document_title(
        &mut reimported,
        &page.title
      ));
      let re_exported = mica_markdown::with_page_title(
        &mica_markdown::export_markdown(&reimported).expect("re-export"),
        mica_markdown::document_title(&reimported).expect("title"),
      );
      assert_eq!(
        re_exported, exported[path],
        "{path}: export → import → export is not stable"
      );
    }

    // The properties came through as properties, not as text.
    let with_props = find(&plan, "带属性的页");
    let reimported = mica_markdown::import_markdown(&with_props.markdown, "root");
    assert_eq!(
      reimported.blocks[0]
        .data
        .get("front_matter")
        .and_then(|v| v.as_str()),
      Some("tags: [a]\nstatus: draft")
    );
    assert_eq!(parent_title(&plan, "Same"), Some("Notes"));
  }

  /// `.Md` counts as Markdown when the archive is read, so it has to come off the
  /// name too — otherwise the page is called `Note.Md`.
  #[test]
  fn the_md_extension_comes_off_whatever_its_case() {
    let raw = vec![
      e("a/Note.Md", "x"),
      e("a/Other.mD", "y"),
      e("a/Third.MD", "z"),
    ];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    for name in ["Note", "Other", "Third"] {
      assert!(!find(&plan, name).is_folder, "{name}");
    }
  }

  /// A v1 archive has no `icon` key anywhere. None is the honest answer — the
  /// alternative (defaulting to something) would put an icon on pages that
  /// never had one.
  #[test]
  fn a_v1_manifest_leaves_every_icon_unset() {
    let raw = vec![
      e("manifest.json", &manifest(&[("A.md", "A", "document")])),
      e("A.md", "body"),
    ];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    assert_eq!(find(&plan, "A").icon, None);
  }

  /// `"icon": ""` is not an icon. The exporter omits the key entirely, but a
  /// hand-edited or third-party archive can carry a blank one, and storing it
  /// would render as an empty slot where the fallback glyph belongs.
  #[test]
  fn an_empty_icon_string_counts_as_no_icon() {
    let json = serde_json::json!({
      "version": 2, "generator": "mica",
      "pages": [{"path": "A.md", "title": "A", "type": "document", "icon": "   "}],
    })
    .to_string();
    let raw = vec![e("manifest.json", &json), e("A.md", "body")];
    let plan = plan_import(raw, false, ImportMode::AsIs).unwrap();
    assert_eq!(find(&plan, "A").icon, None);
  }

  // ── plan_vault_import (issue #84): a walked folder is planned by the very
  // same core as an archive, minus the two normalizations only archives need.

  /// The folder the user PICKED is the container, so a single top-level dir
  /// inside it is real content — never peeled. (`normalize_entries` strips it
  /// for a zip; a vault has no zip's packaging layer.)
  #[test]
  fn a_vault_keeps_its_single_toplevel_dir() {
    let raw = vec![
      e("vault/a.md", "A"),
      e("vault/inner/b.md", "B"),
      e("vault/inner/deeper/c.md", "C"),
    ];
    let plan = plan_vault_import(raw);
    assert!(find(&plan, "vault").is_folder);
    assert_eq!(parent_title(&plan, "a"), Some("vault"));
    assert!(find(&plan, "inner").is_folder);
    assert_eq!(parent_title(&plan, "b"), Some("inner"));
    assert_eq!(parent_title(&plan, "c"), Some("deeper"));
    assert_eq!(plan.wrapper_name, None);
  }

  /// A zip IN a vault is a vault file, not an archive: it stays stored bytes,
  /// never exploded into pages.
  #[test]
  fn a_zip_inside_a_vault_is_an_attachment_not_pages() {
    let raw = vec![e("a.md", "A"), e("backup.zip", "PK")];
    let plan = plan_vault_import(raw);
    assert_eq!(plan.pages.len(), 1);
    assert!(plan.files.contains_key("backup.zip"));
  }

  /// Hidden segments are hidden on disk too: dot-dirs and Apple junk drop out
  /// of both the page tree and the attachments.
  #[test]
  fn a_vault_drops_hidden_paths() {
    let raw = vec![
      e("a.md", "A"),
      e(".obsidian/config.md", "x"),
      e(".DS_Store", "x"),
      e("assets/.hidden.png", "x"),
    ];
    let plan = plan_vault_import(raw);
    assert_eq!(plan.pages.len(), 1);
    assert_eq!(plan.md_paths, paths(&["a.md"]));
    assert!(plan.files.is_empty());
  }

  /// A vault is foreign content: its leading heading is the author's content
  /// and survives, and the page name still comes from the file name.
  #[test]
  fn a_vault_page_keeps_its_leading_heading() {
    let raw = vec![e("notes/Intro.md", "# Intro\n\nhello")];
    let plan = plan_vault_import(raw);
    assert_eq!(find(&plan, "Intro").markdown.trim(), "# Intro\n\nhello");
  }
}
