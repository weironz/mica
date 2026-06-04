//! Notion export adaptation — everything Notion-specific about import lives
//! here; the planner consults it only when Notion mode is on (forced by the
//! UI or auto-detected via [looks_like_notion_export]).

use std::collections::HashMap;

/// Strip the ID Notion appends to exported file/folder names — a trailing
/// 32-hex run or a dashed UUID, separated by space/`-`/`_`. No-op for
/// ordinary names.
pub fn strip_notion_id(segment: &str) -> &str {
  if let Some(cut) = suffix_start(segment) { &segment[..cut] } else { segment }
}

fn suffix_start(segment: &str) -> Option<usize> {
  let b = segment.as_bytes();
  let hex = |c: u8| c.is_ascii_hexdigit();
  let sep = |c: u8| c == b' ' || c == b'-' || c == b'_';

  // 32-hex suffix.
  if b.len() > 32 && b[b.len() - 32..].iter().all(|&c| hex(c)) {
    let mut i = b.len() - 32;
    if sep(b[i - 1]) {
      while i > 1 && sep(b[i - 1]) {
        i -= 1;
      }
      if i > 0 {
        return Some(i);
      }
    }
  }
  // Dashed UUID suffix (8-4-4-4-12).
  if b.len() > 36 {
    let tail = &b[b.len() - 36..];
    let groups = [8usize, 4, 4, 4, 12];
    let mut p = 0;
    let mut ok = true;
    for (gi, g) in groups.iter().enumerate() {
      if gi > 0 {
        if tail[p] != b'-' {
          ok = false;
          break;
        }
        p += 1;
      }
      if !tail[p..p + g].iter().all(|&c| hex(c)) {
        ok = false;
        break;
      }
      p += g;
    }
    if ok {
      let mut i = b.len() - 36;
      if sep(b[i - 1]) {
        while i > 1 && sep(b[i - 1]) {
          i -= 1;
        }
        if i > 0 {
          return Some(i);
        }
      }
    }
  }
  None
}

/// True when the archive looks like a Notion export: at least half of the
/// markdown files (and at least one) carry an ID suffix.
pub fn looks_like_notion_export<'a>(md_paths: impl Iterator<Item = &'a str>) -> bool {
  let (mut total, mut ids) = (0usize, 0usize);
  for p in md_paths {
    total += 1;
    let base = base_no_ext(p);
    if strip_notion_id(base) != base {
      ids += 1;
    }
  }
  ids >= 1 && ids * 2 >= total
}

fn base_no_ext(path: &str) -> &str {
  let base = path.rsplit('/').next().unwrap_or(path);
  base.strip_suffix(".md").or_else(|| base.strip_suffix(".MD")).unwrap_or(base)
}

/// Map each folder path to the md page that represents it. With [notion]
/// mode on, matching tolerates ID suffixes per segment — folder `apple/`
/// matches the page exported as `apple 31f5<…32 hex>.md`; off, it is exact.
pub fn folder_page_index<'a>(
  md_paths: impl Iterator<Item = &'a str>,
  notion: bool,
) -> HashMap<String, String> {
  let seg = |s: &'a str| if notion { strip_notion_id(s) } else { s };
  let mut out = HashMap::new();
  for p in md_paths {
    let (dir, base) = match p.rsplit_once('/') {
      Some((d, b)) => (Some(d), b),
      None => (None, p),
    };
    let base = base.strip_suffix(".md").or_else(|| base.strip_suffix(".MD")).unwrap_or(base);
    let key = match dir {
      None => seg(base).to_string(),
      Some(d) => {
        let nd: Vec<&str> = d.split('/').map(seg).collect();
        format!("{}/{}", nd.join("/"), seg(base))
      }
    };
    out.entry(key).or_insert_with(|| p.to_string());
  }
  out
}
