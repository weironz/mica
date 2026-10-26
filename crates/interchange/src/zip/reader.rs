//! ZIP reader: central-directory driven so data-descriptor entries work,
//! with ZIP64 marker support (streamed server archives use 0xffff/0xffffffff
//! placeholders even for small files). STORE and DEFLATE methods; directories
//! and other methods are skipped. Mirrors the test-verified Dart reader this
//! crate replaces.

use super::inflate::inflate;
use super::names::decode_name;
use super::writer::crc32;

#[derive(Debug, Clone)]
pub struct ZipFileEntry {
  pub name: String,
  pub bytes: Vec<u8>,
}

/// Why an archive could not be read as a whole. Raised only for damage that
/// would otherwise turn into silently-wrong content: a decoded entry whose
/// bytes no longer match its recorded CRC-32. Structurally unreadable entries
/// (unsupported method, past the memory budget, truncated headers) are still
/// dropped the way they always were — they are absence, not corruption.
#[derive(Debug, Clone)]
pub enum ZipReadError {
  CrcMismatch { name: String },
}

impl std::fmt::Display for ZipReadError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      ZipReadError::CrcMismatch { name } => write!(f, "CRC mismatch in archive entry {name:?}"),
    }
  }
}

impl std::error::Error for ZipReadError {}

fn u16le(d: &[u8], i: usize) -> u16 {
  u16::from_le_bytes([d[i], d[i + 1]])
}
fn u32le(d: &[u8], i: usize) -> u32 {
  u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}
fn u64le(d: &[u8], i: usize) -> u64 {
  u64::from_le_bytes([
    d[i],
    d[i + 1],
    d[i + 2],
    d[i + 3],
    d[i + 4],
    d[i + 5],
    d[i + 6],
    d[i + 7],
  ])
}

/// Total bytes one archive may expand to (stored + inflated, all entries).
///
/// Uploads are capped at 256 MiB compressed, but that bounds nothing about the
/// expansion: DEFLATE reaches ~1000:1, so a small bomb could still take every
/// byte of RAM. Every caller holds the whole result in memory, so this is a
/// memory ceiling, sized well above real exports (a 1192-entry wiki inflates
/// to a few hundred MiB) and below what a small node can hold. Entries past
/// the budget are dropped like any other unreadable entry.
pub const MAX_INFLATED_TOTAL: usize = 1024 * 1024 * 1024;

pub fn read_zip(data: &[u8]) -> Result<Vec<ZipFileEntry>, ZipReadError> {
  let mut budget = MAX_INFLATED_TOTAL;
  read_zip_within(data, &mut budget)
}

/// [read_zip] drawing from a caller-held byte budget, so nested archives share
/// ONE ceiling instead of each getting a fresh one.
pub fn read_zip_within(data: &[u8], budget: &mut usize) -> Result<Vec<ZipFileEntry>, ZipReadError> {
  // Locate the end-of-central-directory record, scanning back over a
  // possible archive comment (up to 64 KiB).
  const EOCD_LEN: usize = 22;
  if data.len() < EOCD_LEN {
    return Ok(Vec::new());
  }
  let stop = data.len().saturating_sub(EOCD_LEN + 0xffff);
  let mut eocd = None;
  let mut i = data.len() - EOCD_LEN;
  loop {
    if u32le(data, i) == 0x0605_4b50 {
      eocd = Some(i);
      break;
    }
    if i == stop {
      break;
    }
    i -= 1;
  }
  let Some(eocd) = eocd else {
    return read_local_entries(data, budget);
  };

  let mut total = u64::from(u16le(data, eocd + 10));
  let mut off = u64::from(u32le(data, eocd + 16));
  // ZIP64: marker values defer to the zip64 EOCD record (via its locator).
  if (total == 0xffff || off == 0xffff_ffff) && eocd >= 20 {
    let loc = eocd - 20;
    if u32le(data, loc) == 0x0706_4b50 {
      let z64 = usize::try_from(u64le(data, loc + 8)).unwrap_or(usize::MAX);
      if z64.saturating_add(56) <= data.len() && u32le(data, z64) == 0x0606_4b50 {
        total = u64le(data, z64 + 32);
        off = u64le(data, z64 + 48);
      }
    }
  }

  let mut out = Vec::new();
  let mut off = usize::try_from(off).unwrap_or(usize::MAX);
  for _ in 0..total {
    if off.saturating_add(46) > data.len() || u32le(data, off) != 0x0201_4b50 {
      break;
    }
    let flags = u16le(data, off + 8);
    let method = u16le(data, off + 10);
    let crc = u32le(data, off + 16);
    let mut comp_size = u64::from(u32le(data, off + 20));
    let mut uncomp_size = u64::from(u32le(data, off + 24));
    let name_len = usize::from(u16le(data, off + 28));
    let extra_len = usize::from(u16le(data, off + 30));
    let comment_len = usize::from(u16le(data, off + 32));
    let mut local_off = u64::from(u32le(data, off + 42));
    if off + 46 + name_len + extra_len > data.len() {
      break;
    }
    let extra = &data[off + 46 + name_len..off + 46 + name_len + extra_len];
    // ZIP64 extra field (0x0001): 8-byte values for exactly the fields that
    // hold the 0xffffffff marker, in spec order.
    if comp_size == 0xffff_ffff || uncomp_size == 0xffff_ffff || local_off == 0xffff_ffff {
      let mut p = 0;
      while p + 4 <= extra.len() {
        let id = u16le(extra, p);
        let size = usize::from(u16le(extra, p + 2));
        if id == 0x0001 {
          let mut q = p + 4;
          // The field's own `size` is untrusted: bound it by the bytes that
          // are actually there, or a record claiming more than it carries
          // reads past `extra` (C10). A short field then leaves the marker in
          // place and the entry is dropped as unlocatable below.
          let end = (p + 4 + size).min(extra.len());
          if uncomp_size == 0xffff_ffff && q + 8 <= end {
            uncomp_size = u64le(extra, q);
            q += 8;
          }
          if comp_size == 0xffff_ffff && q + 8 <= end {
            comp_size = u64le(extra, q);
            q += 8;
          }
          if local_off == 0xffff_ffff && q + 8 <= end {
            local_off = u64le(extra, q);
          }
          break;
        }
        p += 4 + size;
      }
    }
    let name = decode_name(&data[off + 46..off + 46 + name_len], flags, extra);
    off += 46 + name_len + extra_len + comment_len;
    if name.ends_with('/') {
      continue; // directory
    }

    // The local header's name/extra lengths may differ from the central
    // directory's — read them to find where the data actually starts.
    let local_off = usize::try_from(local_off).unwrap_or(usize::MAX);
    if local_off.saturating_add(30) > data.len() || u32le(data, local_off) != 0x0403_4b50 {
      continue;
    }
    let l_name = usize::from(u16le(data, local_off + 26));
    let l_extra = usize::from(u16le(data, local_off + 28));
    let start = local_off + 30 + l_name + l_extra;
    let comp_size = usize::try_from(comp_size).unwrap_or(usize::MAX);
    if start.saturating_add(comp_size) > data.len() {
      continue;
    }
    let uncomp_size = usize::try_from(uncomp_size).unwrap_or(usize::MAX);
    if let Some(bytes) = decode_entry(
      &data[start..start + comp_size],
      method,
      uncomp_size,
      crc,
      &name,
      budget,
    )? {
      out.push(ZipFileEntry { name, bytes });
    }
  }
  Ok(out)
}

fn decode_entry(
  comp: &[u8],
  method: u16,
  uncomp_size: usize,
  expected_crc: u32,
  name: &str,
  budget: &mut usize,
) -> Result<Option<Vec<u8>>, ZipReadError> {
  let bytes = match method {
    0 if comp.len() <= *budget => comp.to_vec(),
    8 => match inflate(comp, uncomp_size, *budget) {
      Ok(bytes) => bytes,
      Err(_) => return Ok(None), // structurally unreadable — dropped, not an error
    },
    _ => return Ok(None), // unsupported compression method, or over budget
  };
  *budget -= bytes.len();
  if crc32(&bytes) != expected_crc {
    return Err(ZipReadError::CrcMismatch {
      name: name.to_string(),
    });
  }
  Ok(Some(bytes))
}

/// Fallback for archives without a readable central directory: walk local
/// headers front-to-back (cannot handle data-descriptor entries). Uses the
/// local header's own CRC.
fn read_local_entries(data: &[u8], budget: &mut usize) -> Result<Vec<ZipFileEntry>, ZipReadError> {
  let mut out = Vec::new();
  let mut i = 0;
  while i + 30 <= data.len() {
    if u32le(data, i) != 0x0403_4b50 {
      break;
    }
    let flags = u16le(data, i + 6);
    let method = u16le(data, i + 8);
    let crc = u32le(data, i + 14);
    let comp_size = u32le(data, i + 18) as usize;
    let uncomp_size = u32le(data, i + 22) as usize;
    let name_len = usize::from(u16le(data, i + 26));
    let extra_len = usize::from(u16le(data, i + 28));
    let name_start = i + 30;
    let data_start = name_start + name_len + extra_len;
    if data_start.saturating_add(comp_size) > data.len() {
      break;
    }
    let name = decode_name(
      &data[name_start..name_start + name_len],
      flags,
      &data[name_start + name_len..data_start],
    );
    if !name.ends_with('/')
      && let Some(bytes) = decode_entry(
        &data[data_start..data_start + comp_size],
        method,
        uncomp_size,
        crc,
        &name,
        budget,
      )?
    {
      out.push(ZipFileEntry { name, bytes });
    }
    i = data_start + comp_size;
  }
  Ok(out)
}
