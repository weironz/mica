//! ZIP entry-name decoding. Precedence per the ZIP spec and common tools:
//! the UTF-8 flag (bit 11), then the Info-ZIP Unicode Path extra field
//! (0x7075), then strict UTF-8 (most tools write UTF-8 without setting the
//! flag), and finally GBK — what Windows Explorer produces on a CJK locale.

include!("gbk_table.rs");

/// A zip entry's path, always `/`-separated.
///
/// The spec is unambiguous — APPNOTE 4.4.17.1 says the separator MUST be `/`,
/// even on Windows — but plenty of Windows tools write `\` anyway, and this
/// importer splits paths on `/` only. A `docker\yaml\image.md` entry therefore
/// split into nothing: every page landed flat at the workspace root carrying
/// its own path as its NAME (`docker\yaml\image`), and the folder tree the
/// archive described was silently gone. 309 pages, 0 folders, no error.
///
/// Normalised here because this is the one point both readers (central
/// directory and the local-header fallback) pass through, and because the
/// separator is a property of the ARCHIVE FORMAT: letting it leak inward would
/// mean each of the eight path-splitting sites downstream defending itself,
/// which is how one rule ends up written eight times and then diverging.
///
/// The trade: `\` is a legal character in a Unix filename, so an entry that
/// genuinely contains one is rewritten. In a zip entry name a `\` is a Windows
/// separator essentially every time, and losing the whole tree is far worse
/// than renaming a pathological file.
pub fn decode_name(raw: &[u8], flags: u16, extra: &[u8]) -> String {
  decode_raw_name(raw, flags, extra).replace('\\', "/")
}

fn decode_raw_name(raw: &[u8], flags: u16, extra: &[u8]) -> String {
  if flags & 0x800 != 0 {
    return String::from_utf8_lossy(raw).into_owned();
  }
  // Info-ZIP Unicode Path extra field: id, size, 1-byte version, 4-byte crc,
  // then the UTF-8 name.
  let mut i = 0;
  while i + 4 <= extra.len() {
    let id = u16::from_le_bytes([extra[i], extra[i + 1]]);
    let size = usize::from(u16::from_le_bytes([extra[i + 2], extra[i + 3]]));
    if id == 0x7075 && size >= 5 && i + 4 + size <= extra.len() {
      return String::from_utf8_lossy(&extra[i + 9..i + 4 + size]).into_owned();
    }
    i += 4 + size;
  }
  match std::str::from_utf8(raw) {
    Ok(s) => {
      // GBK byte pairs are frequently *also* valid UTF-8, but then decode
      // into blocks essentially absent from real filenames (Latin
      // Extended-B / IPA / Greek symbols, U+0180–U+03FF). If that happens
      // and GBK decodes cleanly, it was GBK all along ("图片" → "ͼƬ").
      if !s.chars().any(|c| ('\u{0180}'..='\u{03FF}').contains(&c)) {
        return s.to_string();
      }
      let g = decode_gbk(raw);
      if g.contains('\u{FFFD}') { s.to_string() } else { g }
    }
    Err(_) => decode_gbk(raw),
  }
}

#[cfg(test)]
mod separator_tests {
  use super::*;

  const UTF8_FLAG: u16 = 0x800;

  /// The reported failure: a Windows-made archive of a docker wiki imported as
  /// 309 pages and 0 folders, each page named after its own path.
  #[test]
  fn backslash_paths_become_slash_paths() {
    assert_eq!(
      decode_name(br"docker\docker-compose\yaml\image.md", UTF8_FLAG, &[]),
      "docker/docker-compose/yaml/image.md"
    );
  }

  /// Spec-conformant archives are the common case and must be untouched.
  #[test]
  fn forward_slashes_are_left_alone() {
    assert_eq!(
      decode_name(b"docker/yaml/image.md", UTF8_FLAG, &[]),
      "docker/yaml/image.md"
    );
  }

  /// Some tools mix them within one archive — and, occasionally, one name.
  #[test]
  fn a_mixed_path_normalises_whole() {
    assert_eq!(
      decode_name(br"docker\yaml/image.md", UTF8_FLAG, &[]),
      "docker/yaml/image.md"
    );
  }

  /// Normalisation must survive EVERY decoding route, not just the flagged
  /// UTF-8 one — a CJK archive from Windows Explorer arrives GBK-encoded, which
  /// is exactly the population most likely to carry backslashes in the first
  /// place. `\` is 0x5C in both encodings, so the split has to hold after the
  /// GBK table runs.
  #[test]
  fn gbk_names_are_normalised_too() {
    // "图片" in GBK (0xCD3C 0xC6AC) between backslash separators, no UTF-8 flag.
    let raw = b"a\\\xCD\xBC\xC6\xAC\\b.md";
    let decoded = decode_name(raw, 0, &[]);
    assert!(!decoded.contains('\\'), "still has a backslash: {decoded}");
    assert_eq!(decoded.split('/').count(), 3, "{decoded}");
  }

  /// The Info-ZIP Unicode Path extra field is a third decoding route, and it
  /// wins over the raw name — it has to be normalised as well.
  #[test]
  fn the_unicode_path_extra_field_is_normalised_too() {
    let name = br"x\y.md";
    let mut extra = vec![0x75, 0x70]; // id 0x7075
    let size = (5 + name.len()) as u16;
    extra.extend_from_slice(&size.to_le_bytes());
    extra.push(1); // version
    extra.extend_from_slice(&[0, 0, 0, 0]); // crc
    extra.extend_from_slice(name);

    assert_eq!(decode_name(b"ignored", 0, &extra), "x/y.md");
  }

  /// A name with no separator at all is not a path and must come back intact.
  #[test]
  fn a_bare_filename_is_unchanged() {
    assert_eq!(decode_name(b"README.md", UTF8_FLAG, &[]), "README.md");
  }
}

/// Decode GBK/cp936 bytes. Bytes < 0x80 pass through as ASCII; invalid
/// sequences become U+FFFD.
pub fn decode_gbk(bytes: &[u8]) -> String {
  let table: Vec<char> = GBK_TABLE.chars().collect();
  let mut out = String::with_capacity(bytes.len());
  let mut i = 0;
  while i < bytes.len() {
    let b = bytes[i];
    if b < 0x80 {
      out.push(b as char);
      i += 1;
      continue;
    }
    if (0x81..=0xFE).contains(&b) && i + 1 < bytes.len() {
      let t = bytes[i + 1];
      if (0x40..=0xFE).contains(&t) && t != 0x7F {
        let idx = (usize::from(b) - 0x81) * 190 + (usize::from(t) - 0x40)
          - usize::from(t > 0x7F);
        out.push(table[idx]);
        i += 2;
        continue;
      }
    }
    out.push('\u{FFFD}');
    i += 1;
  }
  out
}
