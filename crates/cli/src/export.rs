//! Source-side incremental export. Version records contain paths and metadata,
//! never document/asset bytes. One page archive is materialized at a time.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

use crate::{
  ExportArgs,
  client::{Client, ExportManifest},
  slugify, workspace_dir,
};

const STATE_DIR: &str = ".mica-export";

#[derive(Debug, Default)]
pub struct Stats {
  pub workspaces: usize,
  pub total: usize,
  pub written: usize,
  pub pruned: usize,
  pub pages_exported: usize,
  pub pages_skipped: usize,
  pub bytes_downloaded: u64,
  pub assets_reused: usize,
}

#[derive(Serialize, Deserialize)]
struct WorkspaceState {
  version: u32,
  server: String,
  dir: String,
  tree_revision: String,
  pages: BTreeMap<Uuid, PageState>,
}

#[derive(Serialize, Deserialize)]
struct PageState {
  document_id: Uuid,
  revision: String,
  files: BTreeMap<String, FileStamp>,
  assets: BTreeMap<Uuid, AssetState>,
}

#[derive(Clone, Serialize, Deserialize)]
struct AssetState {
  revision: String,
  path: String,
}

struct KnownAsset {
  state: AssetState,
  stamp: FileStamp,
}

fn asset_source(root: &Path, staging: &Path, asset: &KnownAsset) -> Option<PathBuf> {
  [staging, root].into_iter().find_map(|base| {
    let path = checked_join(base, &asset.state.path).ok()?;
    (stamp(&path).ok()? == asset.stamp).then_some(path)
  })
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct FileStamp {
  len: u64,
  modified_ns: u64,
}

fn stamp(path: &Path) -> Result<FileStamp> {
  let meta = fs::symlink_metadata(path)?;
  ensure!(
    meta.is_file(),
    "export file is not a regular file: {}",
    path.display()
  );
  Ok(FileStamp {
    len: meta.len(),
    modified_ns: meta
      .modified()?
      .duration_since(std::time::UNIX_EPOCH)?
      .as_nanos()
      .try_into()?,
  })
}

/// Reject unsafe paths instead of silently changing their meaning. Export
/// identities must not alias each other after path sanitization.
fn relative(name: &str) -> Result<PathBuf> {
  ensure!(
    !name.is_empty() && !name.contains('\\'),
    "invalid export path: {name:?}"
  );
  let path = Path::new(name);
  ensure!(
    path.components().all(|c| matches!(c, Component::Normal(_))),
    "unsafe export path: {name:?}"
  );
  // Path::components normalizes `.`; reject it explicitly as well.
  ensure!(
    name
      .split('/')
      .all(|s| !s.is_empty() && s != "." && s != ".."),
    "unsafe export path: {name:?}"
  );
  Ok(path.to_path_buf())
}

fn checked_join(root: &Path, name: &str) -> Result<PathBuf> {
  let rel = relative(name)?;
  let mut path = root.to_path_buf();
  for part in rel.components() {
    path.push(part);
    match fs::symlink_metadata(&path) {
      Ok(meta) => ensure!(
        !meta.file_type().is_symlink(),
        "refusing export through symlink: {}",
        path.display()
      ),
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
      Err(e) => return Err(e.into()),
    }
  }
  Ok(path)
}

fn equal_file(path: &Path, bytes: &[u8]) -> Result<bool> {
  let mut file = match File::open(path) {
    Ok(file) => file,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
    Err(e) => return Err(e.into()),
  };
  if file.metadata()?.len() != bytes.len() as u64 {
    return Ok(false);
  }
  let mut buffer = [0u8; 64 * 1024];
  let mut offset = 0;
  loop {
    let n = file.read(&mut buffer)?;
    if n == 0 {
      return Ok(offset == bytes.len());
    }
    if bytes.get(offset..offset + n) != Some(&buffer[..n]) {
      return Ok(false);
    }
    offset += n;
  }
}

/// The temp file is beside the destination so rename is atomic on its FS.
/// Unlike the old mirror, a failed write cannot truncate a successful export.
fn write_changed(root: &Path, name: &str, bytes: &[u8]) -> Result<bool> {
  let path = checked_join(root, name)?;
  if equal_file(&path, bytes)? {
    return Ok(false);
  }
  let parent = path.parent().context("export path has no parent")?;
  fs::create_dir_all(parent)?;
  let temp = parent.join(format!(".mica-write-{}", Uuid::now_v7()));
  let result = (|| -> Result<()> {
    let mut file = OpenOptions::new()
      .write(true)
      .create_new(true)
      .open(&temp)?;
    file.write_all(bytes)?;
    drop(file);
    fs::rename(&temp, &path).with_context(|| format!("installing {}", path.display()))?;
    Ok(())
  })();
  if result.is_err() {
    let _ = fs::remove_file(&temp);
  }
  result?;
  Ok(true)
}

struct Staging(PathBuf);
impl Staging {
  fn new(out: &Path, id: Uuid) -> Result<Self> {
    let path = checked_join(out, &format!("{STATE_DIR}/staging-{id}"))?;
    // This fixed directory is owned by the exporter and inside its locked
    // metadata directory. Clear leftovers from a killed previous process.
    ensure!(
      path.starts_with(out.join(STATE_DIR)),
      "staging path escaped export metadata"
    );
    if path.exists() {
      fs::remove_dir_all(&path)?;
    }
    fs::create_dir_all(&path)?;
    Ok(Self(path))
  }
}
impl Drop for Staging {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.0);
  }
}

/// Only after every download and the final structure check succeeded may a
/// stale file/directory be removed to make room for its replacement.
fn prepare_target(
  root: &Path,
  name: &str,
  desired: &BTreeSet<PathBuf>,
  allow_prune: bool,
) -> Result<usize> {
  let rel = relative(name)?;
  let mut prefix = PathBuf::new();
  let mut removed = 0;
  let count = rel.components().count();
  for (index, part) in rel.components().enumerate() {
    prefix.push(part);
    let path = checked_join(
      root,
      &prefix
        .to_str()
        .context("non-UTF8 export path")?
        .replace('\\', "/"),
    )?;
    let meta = match fs::symlink_metadata(&path) {
      Ok(meta) => meta,
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
      Err(e) => return Err(e.into()),
    };
    let last = index + 1 == count;
    if !last && meta.is_file() {
      ensure!(
        !desired.contains(&prefix),
        "export file is also used as a directory: {}",
        prefix.display()
      );
      ensure!(
        allow_prune,
        "--no-prune prevents replacing stale file {} with a directory",
        prefix.display()
      );
      fs::remove_file(path)?;
      removed += 1;
    } else if last && meta.is_dir() {
      ensure!(
        !desired
          .iter()
          .any(|p| p != &prefix && p.starts_with(&prefix)),
        "export path is also used as a directory: {}",
        prefix.display()
      );
      ensure!(
        allow_prune,
        "--no-prune prevents replacing stale directory {} with a file",
        prefix.display()
      );
      let mut files = Vec::new();
      crate::collect_files(&path, &path, &mut files)?;
      for file in files {
        fs::remove_file(path.join(file))?;
        removed += 1;
      }
      crate::prune_empty_dirs(&path)?;
      fs::remove_dir(&path)?;
    }
  }
  Ok(removed)
}

fn state_path(id: Uuid) -> String {
  format!("{STATE_DIR}/{id}.json")
}
fn dirty_path(id: Uuid) -> String {
  format!("{STATE_DIR}/{id}.dirty")
}

fn load_state(root: &Path, id: Uuid) -> Option<WorkspaceState> {
  let path = checked_join(root, &state_path(id)).ok()?;
  serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn page_paths(manifest: &ExportManifest) -> Result<BTreeMap<Uuid, String>> {
  ensure!(
    manifest.version == 1,
    "unsupported export manifest version {}",
    manifest.version
  );
  ensure!(
    !manifest.structure_revision.is_empty(),
    "export manifest has no structure revision"
  );
  ensure!(
    !manifest.tree_revision.is_empty(),
    "export manifest has no tree revision"
  );
  ensure!(
    manifest.manifest["version"] == 2 && manifest.manifest["generator"] == "mica",
    "invalid Mica tree manifest"
  );
  let entries = manifest.manifest["pages"]
    .as_array()
    .context("export manifest has no page list")?;
  let mut paths = BTreeMap::new();
  let mut used = BTreeSet::new();
  for entry in entries {
    if entry["type"] != "document" {
      continue;
    }
    let id: Uuid = entry["id"]
      .as_str()
      .context("manifest page has no id")?
      .parse()?;
    let path = entry["path"]
      .as_str()
      .context("manifest page has no path")?;
    relative(path)?;
    ensure!(
      path.ends_with(".md") && path != "manifest.json",
      "invalid Markdown path {path:?}"
    );
    ensure!(
      used.insert(path.to_lowercase()),
      "colliding export path {path:?}"
    );
    ensure!(
      paths.insert(id, path.to_string()).is_none(),
      "duplicate export page {id}"
    );
  }
  let ids: BTreeSet<_> = manifest.pages.iter().map(|p| p.view_id).collect();
  ensure!(
    ids.len() == manifest.pages.len() && ids == paths.keys().copied().collect(),
    "export manifest page/revision lists disagree"
  );
  Ok(paths)
}

fn page_is_reusable(root: &Path, page: &PageState, md: &str) -> bool {
  let sidecar = format!("{}.mica.json", md.trim_end_matches(".md"));
  page.files.contains_key(md)
    && page.files.contains_key(&sidecar)
    && page.files.iter().all(|(name, recorded)| {
      checked_join(root, name)
        .and_then(|p| stamp(&p))
        .is_ok_and(|actual| &actual == recorded)
    })
}

fn prune(root: &Path, desired: &BTreeSet<PathBuf>, preserve_state: bool) -> Result<usize> {
  let mut existing = Vec::new();
  crate::collect_files(root, root, &mut existing)?;
  let mut removed = 0;
  for rel in existing {
    // The scheduled backup stages its SQL beside the content mirror. Removing
    // it here would destroy the previous complete dump before pg_dump succeeds.
    if preserve_state && (rel.starts_with(STATE_DIR) || rel.starts_with("_pgdump")) {
      continue;
    }
    if !desired.contains(&rel) {
      fs::remove_file(root.join(&rel)).with_context(|| format!("pruning {}", rel.display()))?;
      removed += 1;
    }
  }
  crate::prune_empty_dirs(root)?;
  Ok(removed)
}

pub fn run(client: &Client, args: &ExportArgs) -> Result<Stats> {
  fs::create_dir_all(&args.out)?;
  let out = fs::canonicalize(&args.out)?;
  let state_dir = checked_join(&out, STATE_DIR)?;
  fs::create_dir_all(&state_dir)?;
  // Keep the inode, including after exit. Unlinking an OS lock permits two
  // writers to lock different inodes for the same output directory.
  let lock = OpenOptions::new()
    .read(true)
    .write(true)
    .create(true)
    .truncate(false)
    .open(checked_join(&out, &format!("{STATE_DIR}/lock"))?)?;
  lock
    .try_lock()
    .context("another export is using this output directory")?;

  let all_workspaces = client.list_workspaces()?;
  let mut slug_counts = HashMap::new();
  for w in &all_workspaces {
    *slug_counts.entry(slugify(&w.name)).or_default() += 1;
  }
  let mut dirs = BTreeSet::new();
  for w in &all_workspaces {
    ensure!(
      dirs.insert(workspace_dir(w, &slug_counts).to_lowercase()),
      "workspace directory names collide"
    );
  }
  let workspaces: Vec<_> = all_workspaces
    .iter()
    .filter(|w| args.ws.is_none_or(|id| id == w.id))
    .collect();
  ensure!(
    args.ws.is_none() || !workspaces.is_empty(),
    "workspace not found (or you are not a member)"
  );
  let mut stats = Stats {
    workspaces: workspaces.len(),
    ..Stats::default()
  };
  // A scoped export must not erase other workspaces or their manifest records.
  let mut summary = if args.ws.is_some() {
    checked_join(&out, "manifest.json")
      .ok()
      .and_then(|p| fs::read(p).ok())
      .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
      .and_then(|v| v["workspaces"].as_array().cloned())
      .unwrap_or_default()
  } else {
    Vec::new()
  };
  let mut desired_all = BTreeSet::new();
  for w in workspaces {
    let manifest = client
      .export_manifest(w.id)
      .with_context(|| format!("reading export inventory for {}", w.id))?;
    let paths = page_paths(&manifest)?;
    let dir = workspace_dir(w, &slug_counts);
    let ws_root = checked_join(&out, &dir)?;
    fs::create_dir_all(&ws_root)?;
    let cached = load_state(&out, w.id);
    let cache_base = cached.as_ref().filter(|s| {
      !args.full
        && s.version == 1
        && s.server == client.server()
        && s.dir == dir
        && !out.join(dirty_path(w.id)).exists()
    });
    let reusable = cache_base.filter(|s| s.tree_revision == manifest.tree_revision);
    // Any interrupted/failed write invalidates this workspace's cache. Even a
    // remote edit/revert racing this run cannot make partial files look current.
    write_changed(&out, &dirty_path(w.id), b"in progress\n")?;
    let staging = Staging::new(&out, w.id)?;
    let mut next = WorkspaceState {
      version: 1,
      server: client.server().to_string(),
      dir: dir.clone(),
      tree_revision: manifest.tree_revision.clone(),
      pages: BTreeMap::new(),
    };
    let mut desired = BTreeSet::new();
    let mut downloaded = BTreeSet::new();
    let asset_versions: BTreeMap<_, _> = manifest
      .files
      .iter()
      .map(|f| (f.file_id, f.revision.as_str()))
      .collect();
    ensure!(
      asset_versions.len() == manifest.files.len(),
      "duplicate file id in export inventory"
    );
    let mut known_assets = BTreeMap::new();
    if let Some(cache) = cache_base {
      for page in cache.pages.values() {
        for (id, asset) in &page.assets {
          if asset_versions.get(id).copied() != Some(asset.revision.as_str()) {
            continue;
          }
          let Some(recorded) = page.files.get(&asset.path) else {
            continue;
          };
          let known = KnownAsset {
            state: asset.clone(),
            stamp: recorded.clone(),
          };
          if asset_source(&ws_root, &staging.0, &known).is_some() {
            known_assets.insert(*id, known);
          }
        }
      }
    }
    for page in &manifest.pages {
      let md = &paths[&page.view_id];
      if let Some(previous) = reusable
        .and_then(|s| s.pages.get(&page.view_id))
        .filter(|p| {
          p.document_id == page.document_id
            && p.revision == page.revision
            && p
              .assets
              .iter()
              .all(|(id, asset)| asset_versions.get(id).copied() == Some(asset.revision.as_str()))
            && page_is_reusable(&ws_root, p, md)
        })
      {
        desired.extend(previous.files.keys().map(PathBuf::from));
        // A version record is small; no page or asset content is retained.
        next.pages.insert(
          page.view_id,
          PageState {
            document_id: previous.document_id,
            revision: previous.revision.clone(),
            assets: previous.assets.clone(),
            files: previous.files.clone(),
          },
        );
        stats.pages_skipped += 1;
        continue;
      }
      let mut skip_assets = Vec::new();
      let preferred = cache_base
        .and_then(|s| s.pages.get(&page.view_id))
        .into_iter()
        .flat_map(|p| p.assets.keys());
      for id in preferred.chain(known_assets.keys()) {
        if skip_assets.len() == 64 {
          break;
        }
        if !skip_assets.contains(id)
          && known_assets
            .get(id)
            .is_some_and(|a| asset_source(&ws_root, &staging.0, a).is_some())
        {
          skip_assets.push(*id);
        }
      }
      let (revision, structure, zip) = client
        .export_page_zip(w.id, page.view_id, &skip_assets)
        .with_context(|| format!("exporting page {} in {}", page.view_id, w.id))?;
      ensure!(
        structure == manifest.structure_revision,
        "page {} was rendered with a different workspace structure; retry the export",
        page.view_id
      );
      stats.bytes_downloaded += zip.len() as u64;
      let entries = mica_interchange::read_zip(&zip);
      drop(zip);
      let sidecar = format!("{}.mica.json", md.trim_end_matches(".md"));
      let names: BTreeSet<_> = entries
        .iter()
        .filter(|e| !e.name.ends_with('/'))
        .map(|e| e.name.as_str())
        .collect();
      ensure!(
        names.contains(md.as_str()) && names.contains(sidecar.as_str()),
        "page {} archive lacks Markdown or sidecar",
        page.view_id
      );
      let sidecar_data: serde_json::Value =
        serde_json::from_slice(&entries.iter().find(|e| e.name == sidecar).unwrap().bytes)?;
      let mut assets = BTreeMap::new();
      let mut files = BTreeMap::new();
      for (id, path) in sidecar_data["assets"]
        .as_object()
        .context("page sidecar has no asset mapping")?
      {
        let id: Uuid = id.parse()?;
        let archive_path = path
          .as_str()
          .context("invalid sidecar asset path")?
          .trim_start_matches("../");
        relative(archive_path)?;
        ensure!(
          archive_path.starts_with("assets/"),
          "page archive has invalid asset path {id}"
        );
        if !names.contains(archive_path) {
          let known = known_assets
            .get(&id)
            .filter(|a| a.state.path == archive_path && skip_assets.contains(&id))
            .with_context(|| format!("page archive is missing asset {id}"))?;
          let source = asset_source(&ws_root, &staging.0, known)
            .with_context(|| format!("reused asset {id} disappeared during export"))?;
          files.insert(archive_path.to_string(), stamp(&source)?);
          desired.insert(relative(archive_path)?);
          stats.assets_reused += 1;
        }
        assets.insert(
          id,
          AssetState {
            path: archive_path.to_string(),
            revision: asset_versions
              .get(&id)
              .with_context(|| format!("asset {id} is missing from export inventory"))?
              .to_string(),
          },
        );
      }
      for entry in entries {
        if entry.name.ends_with('/') || entry.name == "manifest.json" {
          continue;
        }
        ensure!(
          entry.name == *md || entry.name == sidecar || entry.name.starts_with("assets/"),
          "unexpected page archive entry {:?}",
          entry.name
        );
        ensure!(
          !files.contains_key(&entry.name),
          "duplicate page archive entry {:?}",
          entry.name
        );
        let rel = relative(&entry.name)?;
        write_changed(&staging.0, &entry.name, &entry.bytes)?;
        files.insert(entry.name.clone(), stamp(&staging.0.join(&rel))?);
        downloaded.insert(entry.name);
        desired.insert(rel);
      }
      for (id, asset) in &assets {
        known_assets.insert(
          *id,
          KnownAsset {
            state: asset.clone(),
            stamp: files[&asset.path].clone(),
          },
        );
      }
      next.pages.insert(
        page.view_id,
        PageState {
          document_id: page.document_id,
          revision,
          files,
          assets,
        },
      );
      stats.pages_exported += 1;
    }
    let after = client.export_manifest(w.id)?;
    ensure!(
      after.structure_revision == manifest.structure_revision
        && after.manifest == manifest.manifest,
      "workspace {} structure changed during export; retry to rebuild it safely",
      w.id
    );
    write_changed(
      &staging.0,
      "manifest.json",
      &serde_json::to_vec_pretty(&manifest.manifest)?,
    )?;
    downloaded.insert("manifest.json".to_string());
    desired.insert(PathBuf::from("manifest.json"));
    if manifest.manifest["pages"]
      .as_array()
      .is_some_and(Vec::is_empty)
    {
      write_changed(&staging.0, "README.md", b"(empty)")?;
      downloaded.insert("README.md".to_string());
      desired.insert(PathBuf::from("README.md"));
    }
    for name in downloaded {
      stats.pruned += prepare_target(&ws_root, &name, &desired, !args.no_prune)?;
      // Staging is on disk. Reading one installed file never buffers a whole
      // workspace or overlaps the next page's ZIP allocation.
      let bytes = fs::read(checked_join(&staging.0, &name)?)?;
      stats.written += usize::from(write_changed(&ws_root, &name, &bytes)?);
    }
    for page in next.pages.values_mut() {
      for (name, recorded) in &mut page.files {
        *recorded = stamp(&checked_join(&ws_root, name)?)?;
      }
    }
    if !args.no_prune {
      stats.pruned += prune(&ws_root, &desired, false)?;
    }
    write_changed(&out, &state_path(w.id), &serde_json::to_vec(&next)?)?;
    fs::remove_file(checked_join(&out, &dirty_path(w.id))?)?;
    stats.total += desired.len();
    desired_all.extend(desired.into_iter().map(|p| Path::new(&dir).join(p)));
    summary.retain(|entry| entry["id"].as_str() != Some(&w.id.to_string()));
    summary.push(serde_json::json!({"id":w.id, "name":w.name, "dir":dir, "files":desired_all.iter().filter(|p| p.starts_with(&dir)).count()}));
  }
  let manifest = serde_json::json!({"tool":concat!("mica-cli/",env!("CARGO_PKG_VERSION")),
    "exported_at":chrono::Utc::now().to_rfc3339(), "workspaces":summary});
  stats.written += usize::from(write_changed(
    &out,
    "manifest.json",
    &serde_json::to_vec_pretty(&manifest)?,
  )?);
  desired_all.insert(PathBuf::from("manifest.json"));
  stats.total += 1;
  if args.ws.is_none() && !args.no_prune {
    stats.pruned += prune(&out, &desired_all, true)?;
  }
  Ok(stats)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::{Value, json};
  use std::net::TcpListener;
  use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
  };
  use std::thread;
  use std::time::Duration;

  fn scratch(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("mica-incremental-{tag}-{}", Uuid::now_v7()))
  }

  fn ws(name: &str, id: Uuid) -> crate::client::Workspace {
    crate::client::Workspace {
      id,
      name: name.into(),
      owner_id: Uuid::nil(),
      role: "owner".into(),
      created_at: String::new(),
      updated_at: String::new(),
    }
  }

  const WS: Uuid = Uuid::from_u128(1);
  const A: Uuid = Uuid::from_u128(2);
  const B: Uuid = Uuid::from_u128(3);
  const IMAGE: Uuid = Uuid::from_u128(4);

  struct Remote {
    workspaces: Value,
    inventories: BTreeMap<Uuid, ExportManifest>,
    pages: BTreeMap<Uuid, (String, Vec<u8>)>,
    fail_page: Option<Uuid>,
    // Simulates an edit between inventory and rendering without changing the
    // directory tree. The actual archive revision must be recorded instead.
    after_page: Option<ExportManifest>,
    structure_override: Option<String>,
    paths: Vec<String>,
  }

  struct Server {
    remote: Arc<Mutex<Remote>>,
    stopped: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    base: String,
  }

  impl Server {
    fn new() -> Self {
      let listener = TcpListener::bind("127.0.0.1:0").unwrap();
      listener.set_nonblocking(true).unwrap();
      let base = format!("http://{}", listener.local_addr().unwrap());
      let remote = Arc::new(Mutex::new(Remote {
        workspaces: json!({"workspaces":[ws("Notes",WS)]}),
        inventories: [(
          WS,
          inventory("tree1", &[(A, "a.md", "a1"), (B, "b.md", "b1")]),
        )]
        .into(),
        pages: [
          (A, archive("a.md", "alpha", "a1", true)),
          (B, archive("b.md", "bravo", "b1", false)),
        ]
        .into(),
        fail_page: None,
        after_page: None,
        structure_override: None,
        paths: Vec::new(),
      }));
      let stopped = Arc::new(AtomicBool::new(false));
      let state = remote.clone();
      let stop = stopped.clone();
      let worker = thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
          let (mut stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
              thread::sleep(Duration::from_millis(2));
              continue;
            }
            Err(e) => panic!("test HTTP accept: {e}"),
          };
          stream.set_nonblocking(false).unwrap();
          stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
          let mut request = Vec::new();
          let mut buf = [0; 1024];
          while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).unwrap();
            assert_ne!(n, 0);
            request.extend_from_slice(&buf[..n]);
          }
          let line = String::from_utf8(request).unwrap();
          let path = line
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_string();
          let mut state = state.lock().unwrap();
          state.paths.push(path.clone());
          let clean_path = path.split('?').next().unwrap();
          let parts: Vec<_> = clean_path.split('/').collect();
          let (status, header, body) = if path == "/api/workspaces" {
            (
              200,
              String::new(),
              serde_json::to_vec(&state.workspaces).unwrap(),
            )
          } else if parts.last() == Some(&"export-manifest") {
            let ws: Uuid = parts[3].parse().unwrap();
            (
              200,
              String::new(),
              serde_json::to_vec(&state.inventories[&ws]).unwrap(),
            )
          } else if parts.last() == Some(&"export.zip") {
            let id: Uuid = parts[5].parse().unwrap();
            if state.fail_page == Some(id) {
              (503, String::new(), b"injected failure".to_vec())
            } else {
              let (revision, mut bytes) = state.pages[&id].clone();
              if path.contains(&IMAGE.to_string()) {
                bytes = mica_interchange::build_zip(
                  &mica_interchange::read_zip(&bytes)
                    .into_iter()
                    .filter(|e| !e.name.starts_with("assets/"))
                    .map(|e| mica_interchange::ZipEntry {
                      name: e.name,
                      data: e.bytes,
                    })
                    .collect::<Vec<_>>(),
                );
              }
              let workspace: Uuid = parts[3].parse().unwrap();
              let structure = state
                .structure_override
                .clone()
                .unwrap_or_else(|| state.inventories[&workspace].structure_revision.clone());
              if let Some(next) = state.after_page.take() {
                state.inventories.insert(WS, next);
              }
              (
                200,
                format!(
                  "x-mica-export-revision: {revision}\r\nx-mica-export-structure-revision: {structure}\r\n"
                ),
                bytes,
              )
            }
          } else {
            (404, String::new(), Vec::new())
          };
          drop(state);
          let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{header}\r\n",
            body.len()
          );
          stream.write_all(response.as_bytes()).unwrap();
          stream.write_all(&body).unwrap();
        }
      });
      Self {
        remote,
        stopped,
        worker: Some(worker),
        base,
      }
    }

    fn client(&self) -> Client {
      Client::new(&self.base, Some("test-token".into())).unwrap()
    }
    fn page_requests(&self) -> usize {
      self
        .remote
        .lock()
        .unwrap()
        .paths
        .iter()
        .filter(|p| p.split('?').next().unwrap().ends_with("export.zip"))
        .count()
    }
  }

  impl Drop for Server {
    fn drop(&mut self) {
      self.stopped.store(true, Ordering::Relaxed);
      if let Some(worker) = self.worker.take() {
        let result = worker.join();
        if !thread::panicking() {
          result.unwrap();
        }
      }
    }
  }

  fn inventory(tree: &str, pages: &[(Uuid, &str, &str)]) -> ExportManifest {
    serde_json::from_value(json!({"version":1,"structure_revision":tree,"tree_revision":tree,
      "manifest":{"version":2,"generator":"mica","pages":pages.iter().map(|(id,path,_)|
        json!({"id":id,"path":path,"type":"document","title":path,"position":"1"})).collect::<Vec<_>>()},
      "pages":pages.iter().map(|(id,_,rev)|json!({"view_id":id,"document_id":id,"revision":rev})).collect::<Vec<_>>(),
      "files":[{"file_id":IMAGE,"revision":"image1"}]
    })).unwrap()
  }

  fn archive(path: &str, body: &str, revision: &str, image: bool) -> (String, Vec<u8>) {
    let mut entries = vec![
      mica_interchange::ZipEntry {
        name: path.into(),
        data: body.as_bytes().to_vec(),
      },
      mica_interchange::ZipEntry {
        name: format!("{}.mica.json", path.trim_end_matches(".md")),
        data:serde_json::to_vec(&json!({"blocks":[],"assets":if image {json!({IMAGE.to_string():"assets/photo.png"})} else {json!({})}})).unwrap(),
      },
    ];
    if image {
      entries.push(mica_interchange::ZipEntry {
        name: "assets/photo.png".into(),
        data: b"image bytes".to_vec(),
      });
    }
    (revision.into(), mica_interchange::build_zip(&entries))
  }

  fn args(tag: &str) -> ExportArgs {
    ExportArgs {
      out: scratch(tag),
      ws: None,
      no_prune: false,
      full: false,
    }
  }

  #[test]
  fn second_export_skips_source_and_a_body_only_edit_fetches_one_page() {
    let server = Server::new();
    let client = server.client();
    let args = args("incremental");
    fs::create_dir_all(args.out.join("_pgdump")).unwrap();
    fs::write(args.out.join("_pgdump/mica.sql"), b"previous complete dump").unwrap();
    let first = run(&client, &args).unwrap();
    assert_eq!((first.pages_exported, first.pages_skipped), (2, 0));
    let mtime = fs::metadata(args.out.join("notes/b.md"))
      .unwrap()
      .modified()
      .unwrap();
    let second = run(&client, &args).unwrap();
    assert_eq!(
      (
        second.pages_exported,
        second.pages_skipped,
        second.bytes_downloaded
      ),
      (0, 2, 0)
    );
    assert_eq!(server.page_requests(), 2);
    assert_eq!(fs::read(args.out.join("_pgdump/mica.sql")).unwrap(), b"previous complete dump");
    {
      let mut state = server.remote.lock().unwrap();
      state.inventories.insert(
        WS,
        inventory("tree1", &[(A, "a.md", "a2"), (B, "b.md", "b1")]),
      );
      state
        .pages
        .insert(A, archive("a.md", "alpha edited", "a2", true));
    }
    let third = run(&client, &args).unwrap();
    assert_eq!((third.pages_exported, third.pages_skipped), (1, 1));
    assert_eq!(
      third.assets_reused, 1,
      "a body edit must not download an unchanged image again"
    );
    assert_eq!(server.page_requests(), 3);
    assert_eq!(
      fs::read(args.out.join("notes/a.md")).unwrap(),
      b"alpha edited"
    );
    assert_eq!(
      fs::metadata(args.out.join("notes/b.md"))
        .unwrap()
        .modified()
        .unwrap(),
      mtime
    );
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn missing_asset_and_modified_local_page_are_repaired() {
    let server = Server::new();
    let client = server.client();
    let args = args("repair");
    run(&client, &args).unwrap();
    fs::remove_file(args.out.join("notes/assets/photo.png")).unwrap();
    let stats = run(&client, &args).unwrap();
    assert_eq!((stats.pages_exported, stats.pages_skipped), (1, 1));
    assert!(args.out.join("notes/assets/photo.png").exists());
    fs::write(
      args.out.join("notes/b.md"),
      b"local corruption with a different length",
    )
    .unwrap();
    assert_eq!(run(&client, &args).unwrap().pages_exported, 1);
    assert_eq!(fs::read(args.out.join("notes/b.md")).unwrap(), b"bravo");
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn failed_page_does_not_commit_versions_or_prune_and_retry_rebuilds() {
    let server = Server::new();
    let client = server.client();
    let args = args("failed");
    run(&client, &args).unwrap();
    let old = fs::read(args.out.join(state_path(WS))).unwrap();
    fs::write(args.out.join("notes/retain-until-success.md"), b"old").unwrap();
    {
      let mut state = server.remote.lock().unwrap();
      state.inventories.insert(
        WS,
        inventory("tree1", &[(A, "a.md", "a2"), (B, "b.md", "b2")]),
      );
      state
        .pages
        .insert(A, archive("a.md", "new alpha", "a2", true));
      state
        .pages
        .insert(B, archive("b.md", "new bravo", "b2", false));
      state.fail_page = Some(B);
    }
    assert!(run(&client, &args).is_err());
    assert_eq!(fs::read(args.out.join(state_path(WS))).unwrap(), old);
    assert_eq!(
      fs::read(args.out.join("notes/a.md")).unwrap(),
      b"alpha",
      "downloads are staged until the whole workspace succeeds"
    );
    assert!(args.out.join(dirty_path(WS)).exists());
    assert!(args.out.join("notes/retain-until-success.md").exists());
    server.remote.lock().unwrap().fail_page = None;
    assert_eq!(run(&client, &args).unwrap().pages_exported, 2);
    assert!(!args.out.join(dirty_path(WS)).exists());
    assert!(!args.out.join("notes/retain-until-success.md").exists());
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn actual_rendered_revision_wins_over_a_racing_inventory() {
    let server = Server::new();
    let client = server.client();
    let args = args("concurrent-body");
    {
      let mut state = server.remote.lock().unwrap();
      state
        .pages
        .insert(A, archive("a.md", "edited during export", "a2", true));
      state.after_page = Some(inventory("tree1", &[(A, "a.md", "a2"), (B, "b.md", "b1")]));
    }
    run(&client, &args).unwrap();
    let saved = load_state(&args.out, WS).unwrap();
    assert_eq!(saved.pages[&A].revision, "a2");
    assert_eq!(run(&client, &args).unwrap().pages_exported, 0);
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn a_structural_race_is_rejected_and_the_dirty_marker_survives() {
    let server = Server::new();
    let client = server.client();
    let args = args("concurrent-tree");
    server.remote.lock().unwrap().after_page = Some(inventory(
      "tree2",
      &[(A, "renamed.md", "a1"), (B, "b.md", "b1")],
    ));
    assert!(
      run(&client, &args)
        .unwrap_err()
        .to_string()
        .contains("structure")
    );
    assert!(args.out.join(dirty_path(WS)).exists());
    assert!(load_state(&args.out, WS).is_none());
    server
      .remote
      .lock()
      .unwrap()
      .pages
      .insert(A, archive("renamed.md", "alpha", "a1", true));
    assert_eq!(run(&client, &args).unwrap().pages_exported, 2);
    assert!(!args.out.join("notes/a.md").exists());
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn removing_a_page_rebuilds_links_and_prunes_its_files_but_no_prune_keeps_them() {
    for keep in [false, true] {
      let server = Server::new();
      let client = server.client();
      let mut args = args(if keep { "keep" } else { "prune" });
      run(&client, &args).unwrap();
      args.no_prune = keep;
      server
        .remote
        .lock()
        .unwrap()
        .inventories
        .insert(WS, inventory("tree2", &[(B, "b.md", "b1")]));
      assert_eq!(run(&client, &args).unwrap().pages_exported, 1);
      assert_eq!(args.out.join("notes/a.md").exists(), keep);
      assert_eq!(args.out.join("notes/assets/photo.png").exists(), keep);
      fs::remove_dir_all(args.out).unwrap();
    }
  }

  #[test]
  fn scoped_export_preserves_other_workspace_files_and_manifest_records() {
    let server = Server::new();
    let client = server.client();
    let mut args = args("scoped");
    let other = Uuid::from_u128(10);
    {
      let mut state = server.remote.lock().unwrap();
      state.workspaces = json!({"workspaces":[ws("Notes",WS),ws("Other",other)]});
      state.inventories.insert(other, inventory("empty", &[]));
    }
    run(&client, &args).unwrap();
    args.ws = Some(WS);
    let stats = run(&client, &args).unwrap();
    assert_eq!(stats.pages_exported, 0);
    assert!(args.out.join("other/README.md").exists());
    let root: Value =
      serde_json::from_slice(&fs::read(args.out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(root["workspaces"].as_array().unwrap().len(), 2);
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn full_rebuild_and_corrupt_cache_do_not_silently_skip_pages() {
    let server = Server::new();
    let client = server.client();
    let mut args = args("full");
    run(&client, &args).unwrap();
    args.full = true;
    assert_eq!(run(&client, &args).unwrap().pages_exported, 2);
    args.full = false;
    fs::write(args.out.join(state_path(WS)), b"truncated").unwrap();
    assert_eq!(run(&client, &args).unwrap().pages_exported, 2);
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn page_structure_header_detects_a_directory_change_and_revert() {
    let server = Server::new();
    let client = server.client();
    let args = args("tree-aba");
    server.remote.lock().unwrap().structure_override = Some("intermediate-tree".into());
    assert!(
      run(&client, &args)
        .unwrap_err()
        .to_string()
        .contains("different workspace structure")
    );
    assert!(load_state(&args.out, WS).is_none());
    assert!(!args.out.join("notes/a.md").exists());
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn an_unrelated_upload_skips_all_pages_and_asset_repairs_only_fetch_references() {
    let server = Server::new();
    let client = server.client();
    let args = args("file-inventory");
    run(&client, &args).unwrap();
    {
      let mut state = server.remote.lock().unwrap();
      let current = state.inventories.get_mut(&WS).unwrap();
      current.structure_revision = "upload-changed-file-list".into();
      current.files.push(crate::client::ExportFileRevision {
        file_id: Uuid::from_u128(99),
        revision: "unrelated-upload".into(),
      });
    }
    assert_eq!(
      run(&client, &args).unwrap().pages_exported,
      0,
      "uploading an unused image must not re-export thousands of pages"
    );
    {
      let mut state = server.remote.lock().unwrap();
      let current = state.inventories.get_mut(&WS).unwrap();
      current.structure_revision = "repaired-one-file".into();
      current
        .files
        .iter_mut()
        .find(|f| f.file_id == IMAGE)
        .unwrap()
        .revision = "image2".into();
    }
    let stats = run(&client, &args).unwrap();
    assert_eq!((stats.pages_exported, stats.pages_skipped), (1, 1));
    assert_eq!(run(&client, &args).unwrap().pages_exported, 0);
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn stale_files_and_directories_can_exchange_roles_after_successful_downloads() {
    let server = Server::new();
    let client = server.client();
    let mut args = args("file-directory");
    run(&client, &args).unwrap();
    {
      let mut state = server.remote.lock().unwrap();
      state
        .inventories
        .insert(WS, inventory("tree2", &[(A, "a.md/child.md", "a2")]));
      state
        .pages
        .insert(A, archive("a.md/child.md", "child", "a2", false));
    }
    args.no_prune = true;
    assert!(
      run(&client, &args)
        .unwrap_err()
        .to_string()
        .contains("--no-prune")
    );
    assert_eq!(fs::read(args.out.join("notes/a.md")).unwrap(), b"alpha");
    args.no_prune = false;
    run(&client, &args).unwrap();
    assert_eq!(
      fs::read(args.out.join("notes/a.md/child.md")).unwrap(),
      b"child"
    );
    {
      let mut state = server.remote.lock().unwrap();
      state
        .inventories
        .insert(WS, inventory("tree3", &[(A, "a.md", "a3")]));
      state
        .pages
        .insert(A, archive("a.md", "page again", "a3", false));
    }
    run(&client, &args).unwrap();
    assert_eq!(
      fs::read(args.out.join("notes/a.md")).unwrap(),
      b"page again"
    );
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn concurrent_export_is_rejected_before_any_network_work() {
    let server = Server::new();
    let client = server.client();
    let args = args("locked");
    fs::create_dir_all(args.out.join(STATE_DIR)).unwrap();
    let lock = OpenOptions::new()
      .read(true)
      .write(true)
      .create(true)
      .truncate(false)
      .open(args.out.join(STATE_DIR).join("lock"))
      .unwrap();
    lock.try_lock().unwrap();
    assert!(
      run(&client, &args)
        .unwrap_err()
        .to_string()
        .contains("another export")
    );
    assert!(server.remote.lock().unwrap().paths.is_empty());
    drop(lock);
    fs::remove_dir_all(args.out).unwrap();
  }

  #[test]
  fn traversal_and_aliases_are_rejected_instead_of_sanitized() {
    for path in [
      "../a.md",
      "/a.md",
      "a/../b.md",
      "./a.md",
      "a//b.md",
      "a\\b.md",
      "",
    ] {
      assert!(relative(path).is_err(), "{path:?}");
    }
    assert_eq!(
      relative("folder/页面.md").unwrap(),
      Path::new("folder").join("页面.md")
    );
  }

  #[cfg(unix)]
  #[test]
  fn symlinked_exports_are_rejected_and_pruning_never_follows_them() {
    let out = scratch("symlink-root");
    let outside = scratch("symlink-outside");
    fs::create_dir_all(&out).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep.md"), b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, out.join("escape")).unwrap();
    assert!(write_changed(&out, "escape/keep.md", b"changed").is_err());
    prune(&out, &BTreeSet::new(), false).unwrap();
    assert_eq!(fs::read(outside.join("keep.md")).unwrap(), b"keep");
    fs::remove_dir_all(out).unwrap();
    fs::remove_dir_all(outside).unwrap();
  }
}
