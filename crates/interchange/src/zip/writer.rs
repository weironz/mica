//! Minimal store-only (uncompressed) ZIP writer — no third-party dependency.
//! Image/most assets are already compressed, so storing is fine and keeps the
//! crate dependency-free.

/// One file in the archive.
pub struct ZipEntry {
  pub name: String,
  pub data: Vec<u8>,
}

/// Build a ZIP archive (store method) from [entries].
///
/// ZIP64 where — and only where — a value does not fit the classic fields: an
/// entry or offset at/over 4 GiB, or 65535+ entries. Below that the bytes are
/// exactly the classic layout (the golden fixtures pin it). This used to write
/// every size and offset as `as u32`, which past 4 GiB silently TRUNCATED them
/// into an archive no unzipper could read — reachable through "export all
/// workspaces" with the default 5 GiB per-workspace quota.
pub fn build_zip(entries: &[ZipEntry]) -> Vec<u8> {
  build_zip_with_limit(entries, u64::from(u32::MAX), u64::from(u16::MAX))
}

/// [build_zip] with the ZIP64 thresholds injectable, so tests can exercise the
/// ZIP64 layout without allocating 4 GiB.
fn build_zip_with_limit(entries: &[ZipEntry], size_limit: u64, count_limit: u64) -> Vec<u8> {
  const MARK32: u32 = 0xffff_ffff;
  let mut out = Vec::new();
  let mut central = Vec::new();
  let mut offsets: Vec<u64> = Vec::with_capacity(entries.len());
  let mut crcs: Vec<u32> = Vec::with_capacity(entries.len());
  let mut any_zip64 = false;

  for e in entries {
    let crc = crc32(&e.data);
    crcs.push(crc);
    let size = e.data.len() as u64;
    offsets.push(out.len() as u64);
    let big = size >= size_limit;
    any_zip64 |= big;

    // Local file header. Flag bit 11 (0x0800) marks filenames as UTF-8 so
    // non-ASCII names (e.g. CJK) extract correctly.
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&(if big { 45u16 } else { 20u16 }).to_le_bytes()); // version needed
    out.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: UTF-8 names
    out.extend_from_slice(&0u16.to_le_bytes()); // method = store
    out.extend_from_slice(&0u16.to_le_bytes()); // mod time
    out.extend_from_slice(&0u16.to_le_bytes()); // mod date
    out.extend_from_slice(&crc.to_le_bytes());
    let size32 = if big { MARK32 } else { size as u32 };
    out.extend_from_slice(&size32.to_le_bytes()); // compressed
    out.extend_from_slice(&size32.to_le_bytes()); // uncompressed
    out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
    out.extend_from_slice(&(if big { 20u16 } else { 0u16 }).to_le_bytes()); // extra len
    out.extend_from_slice(e.name.as_bytes());
    if big {
      out.extend_from_slice(&0x0001u16.to_le_bytes()); // ZIP64 extra
      out.extend_from_slice(&16u16.to_le_bytes());
      out.extend_from_slice(&size.to_le_bytes()); // uncompressed
      out.extend_from_slice(&size.to_le_bytes()); // compressed
    }
    out.extend_from_slice(&e.data);
  }

  let cd_start = out.len() as u64;
  for (i, e) in entries.iter().enumerate() {
    let size = e.data.len() as u64;
    let big_size = size >= size_limit;
    let big_offset = offsets[i] >= size_limit;
    any_zip64 |= big_offset;
    // ZIP64 extra holds, in this order, exactly the fields marked 0xFFFFFFFF.
    let mut extra = Vec::new();
    if big_size {
      extra.extend_from_slice(&size.to_le_bytes()); // uncompressed
      extra.extend_from_slice(&size.to_le_bytes()); // compressed
    }
    if big_offset {
      extra.extend_from_slice(&offsets[i].to_le_bytes());
    }
    let zip64 = !extra.is_empty();

    central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    central.extend_from_slice(&(if zip64 { 45u16 } else { 20u16 }).to_le_bytes()); // version made by
    central.extend_from_slice(&(if zip64 { 45u16 } else { 20u16 }).to_le_bytes()); // version needed
    central.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: UTF-8 names
    central.extend_from_slice(&0u16.to_le_bytes()); // method = store
    central.extend_from_slice(&0u16.to_le_bytes()); // mod time
    central.extend_from_slice(&0u16.to_le_bytes()); // mod date
    central.extend_from_slice(&crcs[i].to_le_bytes());
    let size32 = if big_size { MARK32 } else { size as u32 };
    central.extend_from_slice(&size32.to_le_bytes());
    central.extend_from_slice(&size32.to_le_bytes());
    central.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
    let extra_len = if zip64 { 4 + extra.len() } else { 0 };
    central.extend_from_slice(&(extra_len as u16).to_le_bytes()); // extra len
    central.extend_from_slice(&0u16.to_le_bytes()); // comment len
    central.extend_from_slice(&0u16.to_le_bytes()); // disk number
    central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
    central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
    let offset32 = if big_offset { MARK32 } else { offsets[i] as u32 };
    central.extend_from_slice(&offset32.to_le_bytes());
    central.extend_from_slice(e.name.as_bytes());
    if zip64 {
      central.extend_from_slice(&0x0001u16.to_le_bytes());
      central.extend_from_slice(&(extra.len() as u16).to_le_bytes());
      central.extend_from_slice(&extra);
    }
  }

  let cd_size = central.len() as u64;
  out.extend_from_slice(&central);

  let count = entries.len() as u64;
  let zip64_end = any_zip64 || count >= count_limit || cd_start >= size_limit || cd_size >= size_limit;
  if zip64_end {
    // ZIP64 end of central directory record + its locator.
    let z64_at = out.len() as u64;
    out.extend_from_slice(&0x0606_4b50u32.to_le_bytes());
    out.extend_from_slice(&44u64.to_le_bytes()); // size of the rest of this record
    out.extend_from_slice(&45u16.to_le_bytes()); // version made by
    out.extend_from_slice(&45u16.to_le_bytes()); // version needed
    out.extend_from_slice(&0u32.to_le_bytes()); // this disk
    out.extend_from_slice(&0u32.to_le_bytes()); // cd start disk
    out.extend_from_slice(&count.to_le_bytes()); // entries on this disk
    out.extend_from_slice(&count.to_le_bytes()); // entries total
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_start.to_le_bytes());
    out.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // disk with the zip64 EOCD
    out.extend_from_slice(&z64_at.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // total disks
  }

  // End of central directory record (markers defer to the ZIP64 record).
  let count16 = if zip64_end { 0xffff } else { count as u16 };
  let cd_size32 = if zip64_end { MARK32 } else { cd_size as u32 };
  let cd_start32 = if zip64_end { MARK32 } else { cd_start as u32 };
  out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
  out.extend_from_slice(&0u16.to_le_bytes()); // disk number
  out.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
  out.extend_from_slice(&count16.to_le_bytes());
  out.extend_from_slice(&count16.to_le_bytes());
  out.extend_from_slice(&cd_size32.to_le_bytes());
  out.extend_from_slice(&cd_start32.to_le_bytes());
  out.extend_from_slice(&0u16.to_le_bytes()); // comment len
  out
}

fn crc32(data: &[u8]) -> u32 {
  let mut crc = 0xffff_ffffu32;
  for &byte in data {
    crc ^= byte as u32;
    for _ in 0..8 {
      let mask = (crc & 1).wrapping_neg();
      crc = (crc >> 1) ^ (0xedb8_8320 & mask);
    }
  }
  !crc
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn crc32_matches_reference() {
    // Known CRC32 of "123456789".
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
  }

  /// Past the classic limits the archive switches to ZIP64 and still reads
  /// back — thresholds lowered so the test need not allocate 4 GiB. (Before,
  /// sizes/offsets were `as u32` and silently truncated past 4 GiB.)
  #[test]
  fn zip64_layout_round_trips_through_the_reader() {
    let entries = vec![
      ZipEntry { name: "a.md".to_string(), data: vec![b'a'; 300] },
      ZipEntry { name: "图/b.png".to_string(), data: vec![7; 10] },
      ZipEntry { name: "c.txt".to_string(), data: b"tail".to_vec() },
    ];
    let zip = build_zip_with_limit(&entries, 100, 2);
    assert!(zip.windows(4).any(|w| w == 0x0606_4b50u32.to_le_bytes()), "zip64 EOCD present");
    let back = crate::zip::read_zip(&zip);
    let got: Vec<(String, Vec<u8>)> = back.into_iter().map(|e| (e.name, e.bytes)).collect();
    let want: Vec<(String, Vec<u8>)> = entries.into_iter().map(|e| (e.name, e.data)).collect();
    assert_eq!(got, want);
  }

  /// Below the limits nothing changes byte-wise (the golden fixtures depend on it).
  #[test]
  fn small_archives_keep_the_classic_layout() {
    let zip = build_zip(&[ZipEntry { name: "a.txt".to_string(), data: b"hi".to_vec() }]);
    assert!(!zip.windows(4).any(|w| w == 0x0606_4b50u32.to_le_bytes()));
  }

  #[test]
  fn zip_has_signatures_and_sizes() {
    let zip = build_zip(&[ZipEntry {
      name: "a.txt".to_string(),
      data: b"hello".to_vec(),
    }]);
    assert_eq!(&zip[0..4], &0x0403_4b50u32.to_le_bytes()); // local header
    // EOCD signature near the end.
    assert!(zip
      .windows(4)
      .any(|w| w == 0x0605_4b50u32.to_le_bytes()));
  }
}
