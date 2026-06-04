//! ZIP entry-name decoding. Precedence per the ZIP spec and common tools:
//! the UTF-8 flag (bit 11), then the Info-ZIP Unicode Path extra field
//! (0x7075), then strict UTF-8 (most tools write UTF-8 without setting the
//! flag), and finally GBK — what Windows Explorer produces on a CJK locale.

include!("gbk_table.rs");

pub fn decode_name(raw: &[u8], flags: u16, extra: &[u8]) -> String {
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
