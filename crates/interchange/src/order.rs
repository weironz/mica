//! Page ordering for import: manifest order first (restores sibling order),
//! unknown files parents-first + natural-sorted.

use std::cmp::Ordering;
use std::collections::HashMap;

/// Order markdown paths: paths listed in the export's `manifest.json` come
/// first, in manifest (pre-order page-tree) order; files the manifest doesn't
/// know about follow, shallower first, natural-sorted (`2 < 10`).
pub fn order_page_paths(md_paths: Vec<String>, manifest_json: Option<&str>) -> Vec<String> {
  let mut manifest_index: HashMap<String, usize> = HashMap::new();
  if let Some(json) = manifest_json
    && let Ok(value) = serde_json::from_str::<serde_json::Value>(json)
    && let Some(pages) = value.get("pages").and_then(|p| p.as_array())
  {
    for (i, p) in pages.iter().enumerate() {
      if let Some(path) = p.get("path").and_then(|v| v.as_str()) {
        manifest_index.entry(path.to_string()).or_insert(i);
      }
    }
  }
  let mut paths = md_paths;
  paths.sort_by(|a, b| {
    match (manifest_index.get(a), manifest_index.get(b)) {
      (Some(ia), Some(ib)) => return ia.cmp(ib),
      (Some(_), None) => return Ordering::Less,
      (None, Some(_)) => return Ordering::Greater,
      (None, None) => {}
    }
    let da = a.matches('/').count();
    let db = b.matches('/').count();
    if da != db {
      return da.cmp(&db);
    }
    natural_compare(a, b)
  });
  paths
}

/// Compare strings with digit runs ordered numerically (`2.md` < `10.md`).
pub fn natural_compare(a: &str, b: &str) -> Ordering {
  let ab = a.as_bytes();
  let bb = b.as_bytes();
  let (mut i, mut j) = (0usize, 0usize);
  while i < ab.len() && j < bb.len() {
    let (ca, cb) = (ab[i], bb[j]);
    if ca.is_ascii_digit() && cb.is_ascii_digit() {
      let (mut i2, mut j2) = (i, j);
      while i2 < ab.len() && ab[i2].is_ascii_digit() {
        i2 += 1;
      }
      while j2 < bb.len() && bb[j2].is_ascii_digit() {
        j2 += 1;
      }
      let na: u128 = a[i..i2].parse().unwrap_or(u128::MAX);
      let nb: u128 = b[j..j2].parse().unwrap_or(u128::MAX);
      match na.cmp(&nb) {
        Ordering::Equal => {}
        other => return other,
      }
      i = i2;
      j = j2;
    } else {
      match ca.cmp(&cb) {
        Ordering::Equal => {}
        other => return other,
      }
      i += 1;
      j += 1;
    }
  }
  (ab.len() - i).cmp(&(bb.len() - j))
}
