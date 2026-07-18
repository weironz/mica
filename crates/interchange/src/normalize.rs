//! Archive normalization for import: drop OS metadata and peel wrapper
//! folders, and expand archives nested one level deep (Notion's
//! whole-workspace exports ship `Part-N.zip` files inside the outer ZIP).

use crate::zip::{ZipFileEntry, read_zip};

/// Drop junk and peel wrapper folders: when everything lives under a single
/// top-level folder with no file beside it (a zipped folder, macOS Finder
/// archives, Notion's `Export-<id>/` shell), strip that level, repeatedly.
///
/// Real content is never peeled: a Mica export keeps `manifest.json` (or a
/// root page's `.md`) at the top level, so the single-folder condition fails.
pub fn normalize_entries(entries: Vec<ZipFileEntry>) -> Vec<ZipFileEntry> {
  let mut out: Vec<ZipFileEntry> = entries.into_iter().filter(|e| !is_junk(&e.name)).collect();
  loop {
    let mut top: Option<&str> = None;
    let mut single = true;
    for e in &out {
      match e.name.split_once('/') {
        None => {
          single = false; // a file at the root → not a wrapper
          break;
        }
        Some((seg, _)) => match top {
          None => top = Some(seg),
          Some(t) if t != seg => {
            single = false;
            break;
          }
          _ => {}
        },
      }
    }
    let Some(top) = top else { break };
    if !single {
      break;
    }
    let cut = top.len() + 1;
    out = out
      .into_iter()
      .map(|e| ZipFileEntry { name: e.name[cut..].to_string(), bytes: e.bytes })
      .collect();
  }
  out
}

/// The innermost single top-level wrapper directory that [normalize_entries]
/// peels away, or `None` when there is no single wrapper (multiple top-level
/// items — e.g. a Mica export). Used to NAME a new workspace / import container
/// after the folder/zip the user handed us, since the peel itself discards the
/// name. Mirrors the peel loop but reports the LAST (innermost, closest to
/// content) segment: `Export-id/MyProject/…` → `MyProject`.
pub fn stripped_wrapper_name(entries: &[ZipFileEntry]) -> Option<String> {
  let mut names: Vec<String> = entries
    .iter()
    .filter(|e| !is_junk(&e.name))
    .map(|e| e.name.clone())
    .collect();
  let mut last: Option<String> = None;
  loop {
    let mut top: Option<&str> = None;
    let mut single = true;
    for n in &names {
      match n.split_once('/') {
        None => {
          single = false;
          break;
        }
        Some((seg, _)) => match top {
          None => top = Some(seg),
          Some(t) if t != seg => {
            single = false;
            break;
          }
          _ => {}
        },
      }
    }
    let Some(top) = top else { break };
    if !single {
      break;
    }
    last = Some(top.to_string());
    let cut = top.len() + 1;
    names = names.into_iter().map(|n| n[cut..].to_string()).collect();
  }
  last
}

/// Any dot-segment hides the whole subtree: `.obsidian/`, `.git/`, `.trash/`,
/// `.DS_Store`, AppleDouble `._*` files…
fn is_junk(path: &str) -> bool {
  let mut last = "";
  for seg in path.split('/') {
    if seg == "__MACOSX" || seg.starts_with('.') {
      return true;
    }
    last = seg;
  }
  last == "Thumbs.db"
}

/// Expand archives nested one level inside [entries]; inner content is
/// normalized, unreadable inner archives are dropped.
pub fn expand_nested_zips(entries: Vec<ZipFileEntry>) -> Vec<ZipFileEntry> {
  if !entries.iter().any(|e| e.name.to_lowercase().ends_with(".zip")) {
    return entries;
  }
  let mut out = Vec::new();
  for e in entries {
    if e.name.to_lowercase().ends_with(".zip") {
      out.extend(normalize_entries(read_zip(&e.bytes)));
    } else {
      out.push(e);
    }
  }
  out
}
