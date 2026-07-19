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

use mica_markdown::{DocumentSnapshotPayload, export_markdown_with_assets};

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
  let mut by_parent: HashMap<Option<&str>, Vec<&TreeNode>> = HashMap::new();
  for n in nodes {
    by_parent.entry(n.parent_id.as_deref()).or_default().push(n);
  }
  for list in by_parent.values_mut() {
    list.sort_by(|a, b| a.position.cmp(&b.position));
  }
  let mut pages: Vec<(&TreeNode, Vec<String>, String)> = Vec::new();
  collect_page_paths(&by_parent, root, &Vec::new(), &mut pages);

  let mut entries: Vec<ZipEntry> = Vec::new();
  // Global (whole-archive) asset dedup by the caller's key.
  let mut asset_by_key: HashMap<String, String> = HashMap::new();
  let mut used_assets: HashSet<String> = HashSet::new();
  let mut used_paths: HashSet<String> = HashSet::new();
  let mut manifest_pages: Vec<serde_json::Value> = Vec::new();

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

  for (node, folder, base) in pages {
    if node.object_type == "folder" {
      // A folder is a pure container: NO `.md`, just a manifest entry so the
      // directory — even an empty one — round-trips.
      let mut dir = String::new();
      for seg in &folder {
        dir.push_str(seg);
        dir.push('/');
      }
      dir.push_str(&base);
      manifest_pages.push(serde_json::json!({
        "path": dir,
        "title": node.name,
        "type": "folder",
      }));
      continue;
    }
    if node.object_type != "document" {
      continue;
    }
    let Some(payload) = payloads.get(&node.object_id) else {
      continue;
    };
    let mut payload = payload.clone();
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
      let a = if let Some(existing) = asset_by_key.get(&asset.dedup_key) {
        existing.clone()
      } else {
        let a = unique_asset_name(&asset.name, &mut used_assets);
        entries.push(ZipEntry {
          name: format!("assets/{a}"),
          data: asset.bytes.clone(),
        });
        asset_by_key.insert(asset.dedup_key.clone(), a.clone());
        a
      };
      img_map.insert(file_id.clone(), format!("{rel}assets/{a}"));
    }

    let body = export_markdown_with_assets(&payload, &img_map).unwrap_or_default();
    let Some(path) = path_by_view.get(&node.id).cloned() else {
      continue;
    };
    manifest_pages.push(serde_json::json!({
      "path": path,
      "title": node.name,
      "type": "document",
    }));
    // The body verbatim — the page NAME rides on the file name + manifest
    // `title`, never a heading welded onto the text.
    entries.push(ZipEntry {
      name: path,
      data: body.into_bytes(),
    });
  }

  if !manifest_pages.is_empty() {
    let manifest = serde_json::json!({
      "version": 1,
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
  let mut used = HashSet::new();
  for child in children {
    let base = unique_sibling_base(child, &mut used);
    out.push((child, folder.to_vec(), base.clone()));
    let mut sub = folder.to_vec();
    sub.push(base);
    collect_page_paths(by_parent, Some(child.id.as_str()), &sub, out);
  }
}

/// Pick a per-sibling base name whose emitted archive paths cannot collide. A
/// document occupies `<base>.md` and, if it has children, `<base>/`; a folder
/// occupies `<base>/`. Reserving BOTH forms stops a document `notes` and a
/// sibling folder `notes.md` from both emitting `notes.md`. On collision the
/// later sibling is bumped (`-2`, `-3`…), and that bumped base also becomes its
/// children's nesting prefix.
fn unique_sibling_base(node: &TreeNode, used: &mut HashSet<String>) -> String {
  let seg = safe_segment(&node.name);
  let is_doc = node.object_type == "document";
  let occupied = |base: &str| -> Vec<String> {
    if is_doc {
      vec![base.to_string(), format!("{base}.md")]
    } else {
      vec![base.to_string()]
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
  if used.insert(candidate.clone()) {
    return candidate;
  }
  let (stem, ext) = match candidate.strip_suffix(".md") {
    Some(s) => (s.to_string(), ".md"),
    None => (candidate.clone(), ""),
  };
  let mut n = 2;
  loop {
    let next = format!("{stem}-{n}{ext}");
    if used.insert(next.clone()) {
      return next;
    }
    n += 1;
  }
}

/// Make a unique `assets/` filename, appending `-1`, `-2`… on collision.
fn unique_asset_name(name: &str, used: &mut HashSet<String>) -> String {
  if used.insert(name.to_string()) {
    return name.to_string();
  }
  let (stem, ext) = match name.rsplit_once('.') {
    Some((s, e)) => (s.to_string(), format!(".{e}")),
    None => (name.to_string(), String::new()),
  };
  let mut n = 1;
  loop {
    let candidate = format!("{stem}-{n}{ext}");
    if used.insert(candidate.clone()) {
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
