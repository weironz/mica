//! Shared, IO-free Markdown-tree ZIP builder — the export counterpart of
//! [`crate::plan_import`]. Both the cloud (Postgres) and local (SQLite) stores
//! gather their own views + document payloads + image bytes, then call
//! [`build_markdown_tree_zip`] to produce an identically-STRUCTURED archive
//! (same paths / manifest / relative refs), so the export→import round-trip
//! stays ONE invariant across worlds (CLAUDE.md #4). (The two worlds hold
//! different data, so "identical layout", not literally identical bytes — one
//! rare tiebreak, the `assets/` filename when two docs reference the same blob
//! under different names, can differ; it's cosmetic and round-trip-neutral.)
//!
//! The walk was lifted verbatim from the server's `build_tree_zip` (paths,
//! sibling-name dedup, relative `../` asset/link rewriting, manifest); only the
//! IO (fetching payloads + blob bytes) stays store-specific in each caller.

use std::collections::{BTreeMap, HashMap, HashSet};

use mica_markdown::{
  DocumentSnapshotPayload, document_title, export_markdown_with_assets, with_page_title,
};

use crate::zip::ZipEntry;

/// A node in the view tree, store-neutral (ids are plain strings, not `Uuid`).
#[derive(Clone, Debug)]
pub struct TreeNode {
  pub id: String,
  pub parent_id: Option<String>,
  /// Sort key among siblings (zero-padded numeric text or a v7 uuid — either
  /// sorts lexically into insertion order).
  pub position: String,
  pub name: String,
  /// `"document"` | `"folder"`.
  pub object_type: String,
  /// The document id whose payload lives in `payloads` (documents only).
  pub object_id: String,
  /// Emoji / icon shown in the sidebar, if the page has one.
  pub icon: Option<String>,
}

/// One image blob to bundle, keyed by the referencing block's `file_id`.
pub struct ImageAsset {
  /// Original filename → `assets/<name>` (deduped on collision).
  pub name: String,
  pub bytes: Vec<u8>,
  /// Global dedup key: two `file_id`s with the SAME key share one `assets/`
  /// entry (cloud passes the storage object key; local the blob's sha256).
  pub dedup_key: String,
}

/// Build the Markdown-tree ZIP entries from pre-gathered data.
///
/// `root = None` exports the whole workspace; `Some(id)` exports that folder's
/// subtree with paths relative to it. `payloads`: `object_id → payload`
/// (documents only). `images`: `file_id → asset` (external-URL images are
/// simply absent from the map and left as links).
pub fn build_markdown_tree_zip(
  nodes: &[TreeNode],
  root: Option<&str>,
  payloads: &HashMap<String, DocumentSnapshotPayload>,
  images: &HashMap<String, ImageAsset>,
) -> Vec<ZipEntry> {
  build_tree_entries(nodes, root, payloads, images, false, None, &HashSet::new())
}

/// Page-at-a-time exports need asset names independent of which other pages
/// were requested. File ids are stable identities; original names are cosmetic.
pub fn build_incremental_markdown_tree_zip(
  nodes: &[TreeNode],
  payloads: &HashMap<String, DocumentSnapshotPayload>,
  images: &HashMap<String, ImageAsset>,
  view_id: &str,
  omit_assets: &HashSet<String>,
) -> Vec<ZipEntry> {
  build_tree_entries(
    nodes,
    None,
    payloads,
    images,
    true,
    Some(view_id),
    omit_assets,
  )
}

/// The complete round-trip manifest without materializing any CRDT payload.
pub fn markdown_tree_manifest(nodes: &[TreeNode]) -> serde_json::Value {
  let (pages, paths) = tree_paths(nodes, None);
  let pages: Vec<_> = pages
    .into_iter()
    .filter_map(|(node, folder, base)| {
      if node.object_type == "folder" {
        let mut path = folder.join("/");
        if !path.is_empty() {
          path.push('/');
        }
        path.push_str(&base);
        Some(manifest_entry(node, &path, "folder"))
      } else {
        paths
          .get(&node.id)
          .map(|path| manifest_entry(node, path, "document"))
      }
    })
    .collect();
  serde_json::json!({"version": 2, "generator": "mica", "pages": pages})
}

type TreePage<'a> = (&'a TreeNode, Vec<String>, String);

fn tree_paths<'a>(
  nodes: &'a [TreeNode],
  root: Option<&str>,
) -> (Vec<TreePage<'a>>, HashMap<String, String>) {
  let mut by_parent: HashMap<Option<&str>, Vec<&TreeNode>> = HashMap::new();
  for n in nodes {
    by_parent.entry(n.parent_id.as_deref()).or_default().push(n);
  }
  for list in by_parent.values_mut() {
    list.sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.id.cmp(&b.id)));
  }
  let mut pages: Vec<(&TreeNode, Vec<String>, String)> = Vec::new();
  collect_page_paths(&by_parent, root, &Vec::new(), &mut pages);

  let mut used_paths: HashSet<String> = HashSet::new();

  // Final zip path per document, decided up front so page links can target
  // pages that come later in the tree.
  let mut path_by_view: HashMap<String, String> = HashMap::new();
  for (node, folder, base) in &pages {
    if node.object_type != "document" {
      continue;
    }
    let mut path = String::new();
    for seg in folder {
      path.push_str(seg);
      path.push('/');
    }
    path.push_str(base);
    path.push_str(".md");
    path = unique_zip_path(path, &mut used_paths);
    path_by_view.insert(node.id.clone(), path);
  }
  (pages, path_by_view)
}

fn build_tree_entries(
  nodes: &[TreeNode],
  root: Option<&str>,
  payloads: &HashMap<String, DocumentSnapshotPayload>,
  images: &HashMap<String, ImageAsset>,
  stable_assets: bool,
  selected_view: Option<&str>,
  omit_assets: &HashSet<String>,
) -> Vec<ZipEntry> {
  let (pages, path_by_view) = tree_paths(nodes, root);
  let mut entries: Vec<ZipEntry> = Vec::new();
  let mut asset_by_key: HashMap<String, String> = HashMap::new();
  let mut used_assets: HashSet<String> = HashSet::new();
  let mut manifest_pages: Vec<serde_json::Value> = Vec::new();

  for (node, folder, base) in pages {
    if selected_view.is_some_and(|id| id != node.id) {
      continue;
    }
    if node.object_type == "folder" {
      // A folder is a pure container: NO `.md`, just a manifest entry so the
      // directory — even an empty one — round-trips.
      let mut dir = String::new();
      for seg in &folder {
        dir.push_str(seg);
        dir.push('/');
      }
      dir.push_str(&base);
      manifest_pages.push(manifest_entry(node, &dir, "folder"));
      continue;
    }
    if node.object_type != "document" {
      continue;
    }
    let Some(original) = payloads.get(&node.object_id) else {
      continue;
    };
    let mut payload = original.clone();
    // Internal page links (`mica://page/<viewId>`) → relative `.md` links.
    rewrite_page_links(&mut payload, folder.len(), &path_by_view);

    // Image assets used by this page (de-duplicated globally by dedup key).
    let rel = "../".repeat(folder.len());
    let mut img_map: BTreeMap<String, String> = BTreeMap::new();
    let mut wanted: Vec<String> = Vec::new();
    for b in &payload.blocks {
      if b.kind != "image" {
        continue;
      }
      if let Some(id) = b.data.get("file_id").and_then(|v| v.as_str()) {
        if !wanted.iter().any(|w| w == id) {
          wanted.push(id.to_string());
        }
      }
    }
    for file_id in &wanted {
      let Some(asset) = images.get(file_id) else {
        continue;
      };
      let dedup_key = if stable_assets {
        file_id
      } else {
        &asset.dedup_key
      };
      let a = if let Some(existing) = asset_by_key.get(dedup_key) {
        existing.clone()
      } else {
        let a = if stable_assets {
          format!("{}-{}", safe_segment(file_id), safe_segment(&asset.name))
        } else {
          unique_asset_name(&asset.name, &mut used_assets)
        };
        if !omit_assets.contains(file_id) {
          entries.push(ZipEntry {
            name: format!("assets/{a}"),
            data: asset.bytes.clone(),
          });
        }
        asset_by_key.insert(dedup_key.clone(), a.clone());
        a
      };
      img_map.insert(file_id.clone(), format!("{rel}assets/{a}"));
    }

    let body = export_markdown_with_assets(&payload, &img_map).unwrap_or_default();
    let Some(path) = path_by_view.get(&node.id).cloned() else {
      continue;
    };
    manifest_pages.push(manifest_entry(node, &path, "document"));

    // The structural half, beside the Markdown: the block model verbatim.
    //
    // Serialized from `original`, NOT from the rewritten `payload`: the markdown
    // needs internal links turned into relative `.md` paths, and this file needs
    // the opposite — `mica://page/<viewId>` kept intact, because identity is the
    // whole reason it exists. An importer remaps those through the manifest's
    // ids; a relative path would have thrown that away.
    //
    // `assets` maps the blocks' `file_id`s to the archive paths of the bytes, so
    // a consumer can resolve images without re-deriving the naming rules.
    entries.push(ZipEntry {
      name: format!("{}.mica.json", path.trim_end_matches(".md")),
      data: serde_json::to_vec_pretty(&serde_json::json!({
        "schema_version": original.schema_version,
        "root_block_id": original.root_block_id,
        "blocks": original.blocks,
        "assets": img_map,
      }))
      .unwrap_or_default(),
    });
    // The title leads the text, after any front matter.
    //
    // This REVERSES a deliberate earlier decision — "the body verbatim; the page
    // NAME rides on the file name + manifest `title`, never a heading welded
    // onto the text" — reversed by the user on 2026-08-27, because a file name
    // only survives as long as the file does: paste the page somewhere, copy it,
    // read it through MCP, and the name is gone. See `docs/page-title-plan.md`.
    //
    // The title comes from the DOCUMENT when it has one and falls back to the
    // view name otherwise. There is no backfill migration, so today almost every
    // page takes the fallback — that is the plan, not an unfinished edge.
    //
    // Import strips this again when the manifest says `generator: mica`, which
    // is what keeps export→import stable (CLAUDE.md #4). The two must move
    // together; changing one alone silently doubles the title per round trip.
    let title = document_title(&payload).unwrap_or(node.name.as_str());
    entries.push(ZipEntry {
      name: path,
      data: with_page_title(&body, title).into_bytes(),
    });
  }

  if !manifest_pages.is_empty() {
    let manifest = serde_json::json!({
      // v2 (2026-08-30) adds `id` / `parent_id` / `position` / `icon` to every
      // entry and a `<page>.mica.json` beside each `<page>.md`. v1 recorded the
      // tree by PATH alone, which is enough to rebuild a shape but not to carry
      // identity — links between pages, and any external reference to a page,
      // could not survive a round trip. Readers must keep accepting v1: archives
      // already downloaded do not get re-issued.
      "version": 2,
      "generator": "mica",
      "pages": manifest_pages,
    });
    entries.insert(
      0,
      ZipEntry {
        name: "manifest.json".to_string(),
        data: serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
      },
    );
  }
  if entries.is_empty() {
    entries.push(ZipEntry {
      name: "README.md".to_string(),
      data: b"(empty)".to_vec(),
    });
  }
  entries
}

/// One `manifest.json` entry. `path` is where the page landed in the archive;
/// everything else is the page's own identity, which is what v2 added.
fn manifest_entry(node: &TreeNode, path: &str, kind: &str) -> serde_json::Value {
  let mut e = serde_json::json!({
    "path": path,
    "title": node.name,
    "type": kind,
    "id": node.id,
    "position": node.position,
  });
  // Absent rather than null: a reader that does not know these keys should see
  // nothing, not a null it has to special-case.
  if let Some(parent) = &node.parent_id {
    e["parent_id"] = serde_json::json!(parent);
  }
  if let Some(icon) = &node.icon {
    e["icon"] = serde_json::json!(icon);
  }
  e
}

/// Flatten the page tree into `(node, ancestor-folder segments, unique base)`,
/// in tree order, giving each page a name unique among its siblings.
fn collect_page_paths<'a>(
  by_parent: &HashMap<Option<&'a str>, Vec<&'a TreeNode>>,
  parent: Option<&str>,
  folder: &[String],
  out: &mut Vec<(&'a TreeNode, Vec<String>, String)>,
) {
  let Some(children) = by_parent.get(&parent) else {
    return;
  };
  // Every archive root also contains exporter-owned entries. This applies to
  // subtree exports too: their root has a parent id but no output prefix.
  let mut used = if folder.is_empty() {
    HashSet::from(["manifest.json".to_string(), "assets".to_string()])
  } else {
    HashSet::new()
  };
  for child in children {
    let base = unique_sibling_base(child, &mut used);
    out.push((child, folder.to_vec(), base.clone()));
    let mut sub = folder.to_vec();
    sub.push(base);
    collect_page_paths(by_parent, Some(child.id.as_str()), &sub, out);
  }
}

/// Pick a per-sibling base name whose emitted archive paths cannot collide. A
/// document occupies `<base>.md`, `<base>.mica.json` and its nesting prefix
/// `<base>/`; a folder occupies `<base>/`. Reserving every form stops a document
/// `notes` and a sibling folder `notes.md` from both emitting `notes.md`. On collision the
/// later sibling is bumped (`-2`, `-3`…), and that bumped base also becomes its
/// children's nesting prefix. Compare case-insensitively so the same archive
/// can be installed on Windows without merging folders or overwriting pages.
fn unique_sibling_base(node: &TreeNode, used: &mut HashSet<String>) -> String {
  let seg = safe_segment(&node.name);
  let is_doc = node.object_type == "document";
  let occupied = |base: &str| -> Vec<String> {
    if is_doc {
      vec![
        base.to_lowercase(),
        format!("{base}.md").to_lowercase(),
        format!("{base}.mica.json").to_lowercase(),
      ]
    } else {
      vec![base.to_lowercase()]
    }
  };
  let free = |base: &str, used: &HashSet<String>| occupied(base).iter().all(|n| !used.contains(n));
  let base = if free(&seg, used) {
    seg
  } else {
    let mut n = 2;
    loop {
      let candidate = format!("{seg}-{n}");
      if free(&candidate, used) {
        break candidate;
      }
      n += 1;
    }
  };
  for name in occupied(&base) {
    used.insert(name);
  }
  base
}

/// Rewrite internal page links (`mica://page/<viewId>`) in link marks to
/// relative paths of the target page's `.md`, so the markdown is standard.
/// Links to pages outside the archive keep their `mica://` href.
fn rewrite_page_links(
  payload: &mut DocumentSnapshotPayload,
  folder_depth: usize,
  path_by_view: &HashMap<String, String>,
) {
  const SCHEME: &str = "mica://page/";
  for block in &mut payload.blocks {
    let Some(marks) = block
      .data
      .get_mut("marks")
      .and_then(serde_json::Value::as_array_mut)
    else {
      continue;
    };
    for mark in marks {
      let Some(obj) = mark.as_object_mut() else {
        continue;
      };
      let Some(target) = obj
        .get("href")
        .and_then(serde_json::Value::as_str)
        .and_then(|href| href.strip_prefix(SCHEME))
        .and_then(|id| path_by_view.get(id))
      else {
        continue;
      };
      let rel = format!("{}{target}", "../".repeat(folder_depth));
      obj.insert("href".into(), serde_json::json!(rel));
    }
  }
}

/// De-dup a `.md` path: append `-2`, `-3`… before the `.md` on collision.
fn unique_zip_path(candidate: String, used: &mut HashSet<String>) -> String {
  if used.insert(candidate.to_lowercase()) {
    return candidate;
  }
  let (stem, ext) = match candidate.strip_suffix(".md") {
    Some(s) => (s.to_string(), ".md"),
    None => (candidate.clone(), ""),
  };
  let mut n = 2;
  loop {
    let next = format!("{stem}-{n}{ext}");
    if used.insert(next.to_lowercase()) {
      return next;
    }
    n += 1;
  }
}

/// Make a unique `assets/` filename, appending `-1`, `-2`… on collision.
fn unique_asset_name(name: &str, used: &mut HashSet<String>) -> String {
  if used.insert(name.to_lowercase()) {
    return name.to_string();
  }
  let (stem, ext) = match name.rsplit_once('.') {
    Some((s, e)) => (s.to_string(), format!(".{e}")),
    None => (name.to_string(), String::new()),
  };
  let mut n = 1;
  loop {
    let candidate = format!("{stem}-{n}{ext}");
    if used.insert(candidate.to_lowercase()) {
      return candidate;
    }
    n += 1;
  }
}

/// Sanitize a page name into a filesystem-safe path segment (alphanumerics,
/// `-_.` kept; runs of anything else collapse to a single `_`).
fn safe_segment(name: &str) -> String {
  let mut out = String::new();
  let mut prev_us = false;
  for ch in name.chars() {
    if ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.') {
      out.push(ch);
      prev_us = ch == '_';
    } else if !prev_us {
      out.push('_');
      prev_us = true;
    }
  }
  let tidy = out.trim_matches('_').to_string();
  if tidy.is_empty() {
    "untitled".to_string()
  } else {
    tidy
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn doc(id: &str, parent: Option<&str>, pos: &str, name: &str) -> TreeNode {
    TreeNode {
      id: id.to_string(),
      parent_id: parent.map(str::to_string),
      position: pos.to_string(),
      name: name.to_string(),
      object_type: "document".to_string(),
      object_id: format!("obj_{id}"),
      icon: None,
    }
  }
  fn folder(id: &str, parent: Option<&str>, pos: &str, name: &str) -> TreeNode {
    TreeNode {
      object_type: "folder".to_string(),
      ..doc(id, parent, pos, name)
    }
  }
  fn payload(text: &str) -> DocumentSnapshotPayload {
    mica_markdown::import_markdown(text, "block_root")
  }
  /// A payload whose body has one image block referencing [file_id] (import
  /// resolves URLs to `file_id` form; we mint it directly to hit the asset path).
  fn payload_with_image(text: &str, file_id: &str) -> DocumentSnapshotPayload {
    let mut p = mica_markdown::import_markdown(text, "block_root");
    let img_id = format!("block_img_{file_id}");
    p.blocks.push(mica_markdown::Block {
      id: img_id.clone(),
      kind: "image".to_string(),
      text: String::new(),
      data: serde_json::json!({ "file_id": file_id, "name": format!("{file_id}.png") }),
      children: vec![],
    });
    if let Some(root) = p.blocks.iter_mut().find(|b| b.id == "block_root") {
      root.children.push(img_id);
    }
    p
  }
  fn names(entries: &[ZipEntry]) -> Vec<String> {
    entries.iter().map(|e| e.name.clone()).collect()
  }
  fn body(entries: &[ZipEntry], name: &str) -> String {
    String::from_utf8(entries.iter().find(|e| e.name == name).unwrap().data.clone()).unwrap()
  }

  #[test]
  fn nests_documents_under_folders_with_relative_paths() {
    let nodes = vec![
      doc("a", None, "0000000010", "Alpha"),
      folder("f", None, "0000000020", "Notes"),
      doc("b", Some("f"), "0000000010", "Beta"),
    ];
    let mut payloads = HashMap::new();
    payloads.insert("obj_a".to_string(), payload("hello alpha"));
    payloads.insert("obj_b".to_string(), payload("hello beta"));
    let entries = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new());
    let got = names(&entries);
    assert!(got.contains(&"Alpha.md".to_string()), "{got:?}");
    assert!(got.contains(&"Notes/Beta.md".to_string()), "{got:?}");
    assert!(got.contains(&"manifest.json".to_string()));
  }

  #[test]
  fn inventory_and_single_page_export_keep_complete_tree_paths() {
    let nodes = vec![
      folder("f", None, "0", "Notes"),
      doc("a", Some("f"), "0", "Same"),
      doc("b", Some("f"), "1", "Same"),
    ];
    let inventory = markdown_tree_manifest(&nodes);
    let pages = inventory["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[2]["path"], "Notes/Same-2.md");
    let mut p = payload("see same");
    p.blocks
      .iter_mut()
      .find(|b| !b.text.is_empty())
      .unwrap()
      .data =
      serde_json::json!({"marks":[{"type":"link","href":"mica://page/a","start":0,"end":3}]});
    let entries = build_incremental_markdown_tree_zip(
      &nodes,
      &HashMap::from([("obj_b".into(), p)]),
      &HashMap::new(),
      "b",
      &HashSet::new(),
    );
    assert!(names(&entries).contains(&"Notes/Same-2.md".into()));
    assert!(!names(&entries).contains(&"Notes/Same.md".into()));
    assert!(body(&entries, "Notes/Same-2.md").contains("../Notes/Same.md"));
    assert!(names(&entries).contains(&"Notes/Same-2.mica.json".into()));
    assert_eq!(markdown_tree_manifest(&[])["pages"], serde_json::json!([]));
  }

  #[test]
  fn case_only_page_names_keep_distinct_paths_and_links_in_all_exports() {
    let nodes = vec![
      folder("f", None, "0", "AI-Agent"),
      doc("a", Some("f"), "0", "Chatbox"),
      doc("b", Some("f"), "1", "chatbox"),
      doc("c", Some("f"), "2", "CHATBOX-2"),
    ];
    let mut linked = payload("see chatbox");
    linked
      .blocks
      .iter_mut()
      .find(|b| !b.text.is_empty())
      .unwrap()
      .data =
      serde_json::json!({"marks":[{"type":"link","href":"mica://page/b","start":0,"end":3}]});
    let payloads = HashMap::from([
      ("obj_a".into(), linked),
      ("obj_b".into(), payload("lowercase page")),
      ("obj_c".into(), payload("existing suffix")),
    ]);
    let inventory = markdown_tree_manifest(&nodes);
    let pages = inventory["pages"].as_array().unwrap();
    assert_eq!(pages[1]["path"], "AI-Agent/Chatbox.md");
    assert_eq!(pages[2]["path"], "AI-Agent/chatbox-2.md");
    assert_eq!(pages[3]["path"], "AI-Agent/CHATBOX-2-2.md");
    assert_eq!(pages[2]["title"], "chatbox");
    assert_eq!(pages[2]["id"], "b");
    let full = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new());
    for (id, path) in [
      ("a", "AI-Agent/Chatbox.md"),
      ("b", "AI-Agent/chatbox-2.md"),
      ("c", "AI-Agent/CHATBOX-2-2.md"),
    ] {
      let single = build_incremental_markdown_tree_zip(
        &nodes,
        &payloads,
        &HashMap::new(),
        id,
        &HashSet::new(),
      );
      assert_eq!(body(&full, path), body(&single, path));
      let sidecar = path.strip_suffix(".md").unwrap().to_string() + ".mica.json";
      assert_eq!(body(&full, &sidecar), body(&single, &sidecar));
    }
    assert!(body(&full, "AI-Agent/Chatbox.md").contains("../AI-Agent/chatbox-2.md"));
    let distinct: HashSet<_> = full.iter().map(|entry| entry.name.to_lowercase()).collect();
    assert_eq!(distinct.len(), full.len());
  }

  #[test]
  fn case_only_folders_and_document_paths_reserve_their_child_prefixes() {
    let nodes = vec![
      folder("f", None, "0", "Knative"),
      doc("a", Some("f"), "0", "Intro"),
      folder("g", None, "1", "knative"),
      doc("b", Some("g"), "0", "Intro"),
      doc("c", None, "2", "Notes"),
      folder("h", None, "3", "NOTES.MD"),
      doc("d", Some("h"), "0", "Intro"),
      folder("i", None, "4", "Other.MD"),
      doc("e", None, "5", "other"),
    ];
    let inventory = markdown_tree_manifest(&nodes);
    let paths: Vec<_> = inventory["pages"]
      .as_array()
      .unwrap()
      .iter()
      .map(|page| page["path"].as_str().unwrap())
      .collect();
    assert_eq!(
      paths,
      [
        "Knative",
        "Knative/Intro.md",
        "knative-2",
        "knative-2/Intro.md",
        "Notes.md",
        "NOTES.MD-2",
        "NOTES.MD-2/Intro.md",
        "Other.MD",
        "other-2.md"
      ]
    );
    let payloads = ["a", "b", "c", "d", "e"]
      .map(|id| (format!("obj_{id}"), payload(id)))
      .into_iter()
      .collect();
    let full = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new());
    for (id, path) in [
      ("a", "Knative/Intro.md"),
      ("b", "knative-2/Intro.md"),
      ("d", "NOTES.MD-2/Intro.md"),
    ] {
      let single = build_incremental_markdown_tree_zip(
        &nodes,
        &payloads,
        &HashMap::new(),
        id,
        &HashSet::new(),
      );
      assert_eq!(body(&full, path), body(&single, path));
    }
  }

  #[test]
  fn case_only_asset_names_keep_distinct_bytes_and_references() {
    let nodes = vec![doc("a", None, "0", "A"), doc("b", None, "1", "B")];
    let payloads = HashMap::from([
      ("obj_a".into(), payload_with_image("first", "file-a")),
      ("obj_b".into(), payload_with_image("second", "file-b")),
    ]);
    let images = HashMap::from([
      (
        "file-a".into(),
        ImageAsset {
          name: "Photo.PNG".into(),
          bytes: vec![1],
          dedup_key: "a".into(),
        },
      ),
      (
        "file-b".into(),
        ImageAsset {
          name: "photo.png".into(),
          bytes: vec![2],
          dedup_key: "b".into(),
        },
      ),
    ]);
    let entries = build_markdown_tree_zip(&nodes, None, &payloads, &images);
    assert_eq!(
      entries
        .iter()
        .find(|e| e.name == "assets/Photo.PNG")
        .unwrap()
        .data,
      [1]
    );
    assert_eq!(
      entries
        .iter()
        .find(|e| e.name == "assets/photo-1.png")
        .unwrap()
        .data,
      [2]
    );
    assert!(body(&entries, "A.md").contains("assets/Photo.PNG"));
    assert!(body(&entries, "B.md").contains("assets/photo-1.png"));
    let sidecar: serde_json::Value = serde_json::from_str(&body(&entries, "B.mica.json")).unwrap();
    assert_eq!(sidecar["assets"]["file-b"], "assets/photo-1.png");
  }

  #[test]
  fn folders_cannot_occupy_sidecars_or_archive_owned_paths() {
    let nodes = vec![
      folder("root", None, "0", "Root"),
      doc("a", Some("root"), "0", "A"),
      folder("f", Some("root"), "1", "a.MICA.JSON"),
      doc("child", Some("f"), "0", "Child"),
      folder("g", Some("root"), "2", "B.MICA.JSON"),
      doc("b", Some("root"), "3", "b"),
      folder("h", Some("root"), "4", "Manifest.json"),
      doc("manifest_child", Some("h"), "0", "Child"),
      folder("i", Some("root"), "5", "Assets"),
      doc("asset_child", Some("i"), "0", "Child"),
    ];
    let (pages, paths) = tree_paths(&nodes, Some("root"));
    assert_eq!(paths["a"], "A.md");
    assert_eq!(paths["child"], "a.MICA.JSON-2/Child.md");
    assert_eq!(paths["b"], "b-2.md");
    assert_eq!(paths["manifest_child"], "Manifest.json-2/Child.md");
    assert_eq!(paths["asset_child"], "Assets-2/Child.md");
    let payloads = ["a", "child", "b", "manifest_child", "asset_child"]
      .map(|id| (format!("obj_{id}"), payload(id)))
      .into_iter()
      .collect();
    let entries = build_markdown_tree_zip(&nodes, Some("root"), &payloads, &HashMap::new());
    let files: HashSet<_> = entries.iter().map(|e| e.name.to_lowercase()).collect();
    for (node, folder, base) in pages {
      if node.object_type == "folder" {
        let path = folder
          .into_iter()
          .chain([base])
          .collect::<Vec<_>>()
          .join("/");
        assert!(
          !files.contains(&path.to_lowercase()),
          "folder overwrites file: {path}"
        );
      }
    }
  }

  #[test]
  fn separate_page_exports_do_not_collide_on_original_asset_names() {
    let nodes = vec![doc("a", None, "0", "A"), doc("b", None, "1", "B")];
    let export = |page: &str, file: &str, bytes: Vec<u8>| {
      build_incremental_markdown_tree_zip(
        &nodes,
        &HashMap::from([(format!("obj_{page}"), payload_with_image("picture", file))]),
        &HashMap::from([(
          file.into(),
          ImageAsset {
            name: "pasted-image.png".into(),
            bytes,
            dedup_key: "same-key".into(),
          },
        )]),
        page,
        &HashSet::new(),
      )
    };
    let a = export("a", "file-a", vec![1]);
    let b = export("b", "file-b", vec![2]);
    assert!(names(&a).contains(&"assets/file-a-pasted-image.png".into()));
    assert!(names(&b).contains(&"assets/file-b-pasted-image.png".into()));
    let shared = export("b", "file-a", vec![1]);
    assert!(names(&shared).contains(&"assets/file-a-pasted-image.png".into()));
  }

  #[test]
  fn reusing_an_asset_omits_bytes_but_preserves_markdown_and_sidecar_references() {
    let nodes = vec![doc("a", None, "0", "A")];
    let payloads = HashMap::from([("obj_a".into(), payload_with_image("picture", "shared"))]);
    let images = HashMap::from([(
      "shared".into(),
      ImageAsset {
        name: "photo.png".into(),
        bytes: vec![1, 2, 3],
        dedup_key: "K".into(),
      },
    )]);
    let full =
      build_incremental_markdown_tree_zip(&nodes, &payloads, &images, "a", &HashSet::new());
    let reused = build_incremental_markdown_tree_zip(
      &nodes,
      &payloads,
      &images,
      "a",
      &HashSet::from(["shared".into()]),
    );
    assert!(names(&full).contains(&"assets/shared-photo.png".into()));
    assert!(!names(&reused).iter().any(|p| p.starts_with("assets/")));
    assert_eq!(body(&full, "A.md"), body(&reused, "A.md"));
    assert_eq!(body(&full, "A.mica.json"), body(&reused, "A.mica.json"));
    let sidecar: serde_json::Value = serde_json::from_str(&body(&reused, "A.mica.json")).unwrap();
    assert_eq!(sidecar["assets"]["shared"], "assets/shared-photo.png");
  }

  /// manifest v2 carries IDENTITY, not just shape. v1 recorded `path` alone,
  /// which can rebuild a tree but cannot tell a re-imported page that it is the
  /// same page — so nothing pointing at it survives the round trip.
  #[test]
  fn manifest_v2_carries_ids_parents_and_position() {
    let nodes = vec![
      folder("f", None, "0000000020", "Notes"),
      doc("b", Some("f"), "0000000010", "Beta"),
    ];
    let mut payloads = HashMap::new();
    payloads.insert("obj_b".to_string(), payload("hi"));
    let entries = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new());
    let m: serde_json::Value = serde_json::from_slice(
      &entries.iter().find(|e| e.name == "manifest.json").unwrap().data,
    )
    .unwrap();
    assert_eq!(m["version"], 2);
    let pages = m["pages"].as_array().unwrap();
    let beta = pages.iter().find(|p| p["title"] == "Beta").unwrap();
    assert_eq!(beta["id"], "b");
    assert_eq!(beta["parent_id"], "f");
    assert_eq!(beta["position"], "0000000010");
    // Absent, not null — a reader that does not know the key sees nothing.
    assert!(beta.get("icon").is_none(), "{beta}");
    let notes = pages.iter().find(|p| p["title"] == "Notes").unwrap();
    assert_eq!(notes["id"], "f");
    assert!(notes.get("parent_id").is_none(), "root folder has no parent: {notes}");
  }

  /// The sidecar exists so the block model survives; the Markdown beside it is
  /// for humans. Both, not either.
  #[test]
  fn each_document_gets_a_block_json_beside_its_markdown() {
    let nodes = vec![doc("a", None, "0000000010", "Alpha")];
    let mut payloads = HashMap::new();
    payloads.insert("obj_a".to_string(), payload("hello"));
    let entries = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new());
    let got = names(&entries);
    assert!(got.contains(&"Alpha.md".to_string()), "{got:?}");
    assert!(got.contains(&"Alpha.mica.json".to_string()), "{got:?}");
    let j: serde_json::Value =
      serde_json::from_slice(&entries.iter().find(|e| e.name == "Alpha.mica.json").unwrap().data)
        .unwrap();
    assert!(j["blocks"].as_array().unwrap().iter().any(|b| b["id"] == "block_root"));
    assert_eq!(j["root_block_id"], "block_root");
  }

  /// The two files answer to different consumers and must NOT agree here: the
  /// Markdown needs a relative `.md` link a human/editor can follow, the JSON
  /// needs the `mica://page/<id>` an importer can remap. Rewriting the JSON too
  /// would throw away the identity the JSON exists to carry — and the bug would
  /// be invisible until somebody re-imported and found the links broken.
  #[test]
  fn block_json_keeps_internal_links_while_markdown_relativises_them() {
    let nodes = vec![
      doc("a", None, "0000000010", "Alpha"),
      doc("b", None, "0000000020", "Beta"),
    ];
    let mut p = payload("see beta");
    // A link mark on the root block pointing at page "b".
    if let Some(root) = p.blocks.iter_mut().find(|b| b.id == "block_root") {
      root.data = serde_json::json!({
        "marks": [{ "type": "link", "href": "mica://page/b", "start": 0, "end": 3 }]
      });
    }
    let mut payloads = HashMap::new();
    payloads.insert("obj_a".to_string(), p);
    payloads.insert("obj_b".to_string(), payload("beta"));
    let entries = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new());
    let j: serde_json::Value =
      serde_json::from_slice(&entries.iter().find(|e| e.name == "Alpha.mica.json").unwrap().data)
        .unwrap();
    let href = j["blocks"]
      .as_array()
      .unwrap()
      .iter()
      .find_map(|b| b["data"]["marks"][0]["href"].as_str())
      .expect("the link mark survived into the json");
    assert_eq!(href, "mica://page/b", "the JSON must keep identity");
  }

  #[test]
  fn root_at_a_folder_gives_relative_subtree() {
    let nodes = vec![
      folder("f", None, "0000000010", "Notes"),
      doc("b", Some("f"), "0000000010", "Beta"),
    ];
    let mut payloads = HashMap::new();
    payloads.insert("obj_b".to_string(), payload("hi"));
    // Rooted at the folder → its child sits at the archive root, not under Notes/.
    let entries = build_markdown_tree_zip(&nodes, Some("f"), &payloads, &HashMap::new());
    assert!(names(&entries).contains(&"Beta.md".to_string()), "{:?}", names(&entries));
  }

  #[test]
  fn doc_and_sibling_folder_with_colliding_name_do_not_clobber() {
    // A document "notes" and a sibling folder "notes.md" must not both emit
    // `notes.md` (that dropped a node in the import dedup).
    let nodes = vec![
      doc("d", None, "0000000010", "notes"),
      folder("g", None, "0000000020", "notes.md"),
    ];
    let mut payloads = HashMap::new();
    payloads.insert("obj_d".to_string(), payload("x"));
    let entries = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new());
    let got = names(&entries);
    let md_count = got.iter().filter(|n| n.ends_with(".md") && *n != "README.md").count();
    // Exactly one `.md` file (the document); the folder emits none. No collision.
    assert_eq!(md_count, 1, "{got:?}");
  }

  #[test]
  fn shared_image_dedups_to_one_asset_with_relative_depth() {
    // Two pages reference the SAME image (same dedup_key): one `assets/` entry,
    // each page linking it at the right `../` depth (the cloud/local shared
    // asset path — cloud keys the dedup by object_key, local by file_id/sha).
    let nodes = vec![
      doc("a", None, "0000000010", "Alpha"),
      folder("f", None, "0000000020", "Notes"),
      doc("b", Some("f"), "0000000010", "Beta"),
    ];
    let mut payloads = HashMap::new();
    payloads.insert("obj_a".to_string(), payload_with_image("alpha", "shared"));
    payloads.insert("obj_b".to_string(), payload_with_image("beta", "shared"));
    let mut images = HashMap::new();
    images.insert(
      "shared".to_string(),
      ImageAsset {
        name: "pic.png".to_string(),
        bytes: vec![1, 2, 3],
        dedup_key: "K".to_string(),
      },
    );
    let entries = build_markdown_tree_zip(&nodes, None, &payloads, &images);
    let assets: Vec<&str> = entries
      .iter()
      .map(|e| e.name.as_str())
      .filter(|n| n.starts_with("assets/"))
      .collect();
    assert_eq!(assets, vec!["assets/pic.png"], "{:?}", names(&entries));
    // Root page → `assets/…`; the page under Notes/ → `../assets/…`.
    assert!(
      body(&entries, "Alpha.md").contains("(assets/pic.png)"),
      "{}",
      body(&entries, "Alpha.md")
    );
    assert!(
      body(&entries, "Notes/Beta.md").contains("(../assets/pic.png)"),
      "{}",
      body(&entries, "Notes/Beta.md")
    );
  }
}
