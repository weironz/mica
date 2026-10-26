//! Archive normalization for import: drop OS metadata and peel wrapper
//! folders, and expand archives nested one level deep (Notion's
//! whole-workspace exports ship `Part-N.zip` files inside the outer ZIP).

use crate::zip::{ZipFileEntry, read_zip_within};

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
      .map(|e| ZipFileEntry {
        name: e.name[cut..].to_string(),
        bytes: e.bytes,
      })
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
pub(crate) fn is_junk(path: &str) -> bool {
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
/// normalized. An inner archive whose bytes are STRUCTURALLY unreadable is
/// dropped (legacy policy — absence, not corruption), but a DECODE-LEVEL
/// failure (a CRC mismatch — content that would otherwise import as
/// silently-wrong bytes) propagates as an error, because completing the
/// import with only the healthy volumes would be an incomplete "success"
/// (issue #94). When two inner archives (or an inner archive and the outer
/// one) carry the SAME relative path with DIFFERENT bytes, each such inner
/// archive's whole tree is re-rooted under a prefix unique across every
/// group's paths, so neither the later archive NOR the re-root itself
/// silently overwrites anything (issue #143). Relative references stay
/// intact because every entry in a tree moves together.
pub fn expand_nested_zips(
  entries: Vec<ZipFileEntry>,
) -> Result<Vec<ZipFileEntry>, crate::zip::ZipReadError> {
  if !entries
    .iter()
    .any(|e| e.name.to_lowercase().ends_with(".zip"))
  {
    return Ok(entries);
  }
  // One budget for ALL inner archives: each getting a fresh one would let N
  // nested bombs multiply the ceiling by N.
  let mut budget = crate::zip::MAX_INFLATED_TOTAL;
  // group 0 = the outer archive's own (non-zip) entries; group i = one inner
  // archive's normalized content, keyed by that archive's name minus `.zip`.
  let mut groups: Vec<(Option<String>, Vec<ZipFileEntry>)> = Vec::new();
  let mut outer = Vec::new();
  for e in entries {
    if e.name.to_lowercase().ends_with(".zip") {
      let key = e.name.get(..e.name.len() - 4).unwrap_or(&e.name);
      let key = if key.is_empty() { "nested" } else { key };
      match read_zip_within(&e.bytes, &mut budget) {
        Ok(inner) => groups.push((Some(key.to_string()), normalize_entries(inner))),
        // detectable corruption propagates; structurally unreadable drops
        Err(err @ crate::zip::ZipReadError::CrcMismatch { .. }) => return Err(err),
      }
    } else {
      outer.push(e);
    }
  }
  groups.insert(0, (None, outer));

  // First bytes seen per path as (group, entry) indices — no second copy of
  // any entry's bytes ever floats outside `groups`. A path carried by two
  // groups with different bytes conflicts; identical bytes may still merge
  // into one stored object (paths stay correct either way because references
  // resolve folder-first).
  let mut first: std::collections::HashMap<String, (usize, usize)> =
    std::collections::HashMap::new();
  let mut conflicted: std::collections::HashSet<usize> = std::collections::HashSet::new();
  for (gi, (_, group)) in groups.iter().enumerate() {
    for (ei, e) in group.iter().enumerate() {
      match first.get(&e.name) {
        None => {
          first.insert(e.name.clone(), (gi, ei));
        }
        Some(&(fj, fj_ei)) => {
          if fj != gi && groups[fj].1[fj_ei].bytes != e.bytes {
            conflicted.insert(gi);
            conflicted.insert(fj);
          }
        }
      }
    }
  }

  // Re-root prefixes must be unique against EVERY top-level segment used by
  // any group — "<stem>/" can itself collide with an outer `<stem>/…` path and
  // re-enable the silent overwrite the re-root exists to prevent.
  let mut used_top: std::collections::HashSet<String> = std::collections::HashSet::new();
  for (_, group) in &groups {
    for e in group {
      used_top.insert(
        e.name
          .split_once('/')
          .map(|(top, _)| top)
          .unwrap_or(e.name.as_str())
          .to_lowercase(),
      );
    }
  }
  let mut prefix_of: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
  for (gi, (key, _)) in groups.iter().enumerate() {
    if gi == 0 || !conflicted.contains(&gi) {
      continue;
    }
    let stem = key.clone().unwrap_or_else(|| "nested".to_string());
    let mut prefix = stem.clone();
    let mut n = 1;
    while !used_top.insert(prefix.to_lowercase()) {
      n += 1;
      prefix = format!("{stem}-{n}");
    }
    prefix_of.insert(gi, prefix);
  }

  let mut out = Vec::new();
  for (gi, (_, group)) in groups.into_iter().enumerate() {
    let prefix = prefix_of.get(&gi);
    for e in group {
      match prefix {
        Some(k) => out.push(ZipFileEntry {
          name: format!("{k}/{}", e.name),
          bytes: e.bytes,
        }),
        None => out.push(e),
      }
    }
  }
  Ok(out)
}
