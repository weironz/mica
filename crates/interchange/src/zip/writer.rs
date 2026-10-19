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
  let mut writer = ZipStreamWriter::with_limits(size_limit, count_limit);
  let mut out = Vec::new();
  for e in entries {
    out.extend_from_slice(&writer.local_header(&e.name, &e.data));
    out.extend_from_slice(&e.data);
  }
  out.extend_from_slice(&writer.finish());
  out
}

/// The same archive as [build_zip], produced one entry at a time (D3): only
/// the central directory's per-entry metadata stays in memory, so an export
/// can send each entry as soon as it exists instead of assembling the whole
/// archive first. Store method, so every entry's CRC and size are known from
/// its bytes — no data descriptors, and the layout is byte-identical to
/// [build_zip] for the same entries.
pub struct ZipStreamWriter {
  size_limit: u64,
  count_limit: u64,
  written: u64,
  central: Vec<CentralRecord>,
  any_zip64: bool,
}

struct CentralRecord {
  name: String,
  crc: u32,
  size: u64,
  offset: u64,
}

impl Default for ZipStreamWriter {
  fn default() -> Self {
    Self::with_limits(u64::from(u32::MAX), u64::from(u16::MAX))
  }
}

impl ZipStreamWriter {
  fn with_limits(size_limit: u64, count_limit: u64) -> Self {
    Self { size_limit, count_limit, written: 0, central: Vec::new(), any_zip64: false }
  }

  /// The local header for one entry. Emit it, then `data` itself, before the
  /// next call — the offsets recorded here assume exactly that.
  pub fn local_header(&mut self, name: &str, data: &[u8]) -> Vec<u8> {
    const MARK32: u32 = 0xffff_ffff;
    let crc = crc32(data);
    let size = data.len() as u64;
    let big = size >= self.size_limit;
    self.any_zip64 |= big;

    // Flag bit 11 (0x0800) marks filenames as UTF-8 so non-ASCII names (e.g.
    // CJK) extract correctly.
    let mut out = Vec::with_capacity(30 + name.len() + 20);
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
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&(if big { 20u16 } else { 0u16 }).to_le_bytes()); // extra len
    out.extend_from_slice(name.as_bytes());
    if big {
      out.extend_from_slice(&0x0001u16.to_le_bytes()); // ZIP64 extra
      out.extend_from_slice(&16u16.to_le_bytes());
      out.extend_from_slice(&size.to_le_bytes()); // uncompressed
      out.extend_from_slice(&size.to_le_bytes()); // compressed
    }
    self.central.push(CentralRecord { name: name.to_string(), crc, size, offset: self.written });
    self.written += out.len() as u64 + size;
    out
  }

  /// Bytes written so far (local headers + data).
  pub fn written(&self) -> u64 {
    self.written
  }

  /// The central directory and end records that close the archive.
  pub fn finish(mut self) -> Vec<u8> {
    const MARK32: u32 = 0xffff_ffff;
    let mut central = Vec::new();
    let cd_start = self.written;
    for r in &self.central {
      let big_size = r.size >= self.size_limit;
      let big_offset = r.offset >= self.size_limit;
      self.any_zip64 |= big_offset;
      // ZIP64 extra holds, in this order, exactly the fields marked 0xFFFFFFFF.
      let mut extra = Vec::new();
      if big_size {
        extra.extend_from_slice(&r.size.to_le_bytes()); // uncompressed
        extra.extend_from_slice(&r.size.to_le_bytes()); // compressed
      }
      if big_offset {
        extra.extend_from_slice(&r.offset.to_le_bytes());
      }
      let zip64 = !extra.is_empty();

      central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
      central.extend_from_slice(&(if zip64 { 45u16 } else { 20u16 }).to_le_bytes()); // version made by
      central.extend_from_slice(&(if zip64 { 45u16 } else { 20u16 }).to_le_bytes()); // version needed
      central.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: UTF-8 names
      central.extend_from_slice(&0u16.to_le_bytes()); // method = store
      central.extend_from_slice(&0u16.to_le_bytes()); // mod time
      central.extend_from_slice(&0u16.to_le_bytes()); // mod date
      central.extend_from_slice(&r.crc.to_le_bytes());
      let size32 = if big_size { MARK32 } else { r.size as u32 };
      central.extend_from_slice(&size32.to_le_bytes());
      central.extend_from_slice(&size32.to_le_bytes());
      central.extend_from_slice(&(r.name.len() as u16).to_le_bytes());
      let extra_len = if zip64 { 4 + extra.len() } else { 0 };
      central.extend_from_slice(&(extra_len as u16).to_le_bytes()); // extra len
      central.extend_from_slice(&0u16.to_le_bytes()); // comment len
      central.extend_from_slice(&0u16.to_le_bytes()); // disk number
      central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
      central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
      let offset32 = if big_offset { MARK32 } else { r.offset as u32 };
      central.extend_from_slice(&offset32.to_le_bytes());
      central.extend_from_slice(r.name.as_bytes());
      if zip64 {
        central.extend_from_slice(&0x0001u16.to_le_bytes());
        central.extend_from_slice(&(extra.len() as u16).to_le_bytes());
        central.extend_from_slice(&extra);
      }
    }

    let cd_size = central.len() as u64;
    let mut out = central;
    let count = self.central.len() as u64;
    let zip64_end = self.any_zip64
      || count >= self.count_limit
      || cd_start >= self.size_limit
      || cd_size >= self.size_limit;
    if zip64_end {
      // ZIP64 end of central directory record + its locator.
      let z64_at = cd_start + cd_size;
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

#[cfg(test)]
mod byte_pins {
  use super::*;
  fn fnv(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
      h ^= x as u64;
      h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
  }
  fn cases() -> Vec<(Vec<ZipEntry>, u64, u64)> {
    let mk = |n: &str, d: Vec<u8>| ZipEntry { name: n.to_string(), data: d };
    vec![
      (vec![mk("a.txt", b"hi".to_vec())], u64::from(u32::MAX), u64::from(u16::MAX)),
      (vec![mk("a.md", vec![b'a'; 300]), mk("图/b.png", vec![7; 10]), mk("c.txt", b"tail".to_vec())], 100, 2),
      (vec![mk("x", vec![1; 50]), mk("y", vec![2; 60]), mk("z", vec![3; 5])], 100, 100),
      (vec![mk("1", vec![]), mk("2", vec![9]), mk("3", vec![8; 3])], 1000, 3),
    ]
  }
  /// D3: the streaming rewrite must not move a byte. These fingerprints were
  /// taken from the pre-streaming writer for a classic archive and the three
  /// ZIP64 triggers (entry size, offset, entry count).
  #[test]
  fn streaming_writer_is_byte_identical_to_the_old_one() {
    let pins = [0x82b7_7c20_e15a_55ca_u64, 0x8031_7b83_ca95_a5c4, 0xbb20_86ae_9ea2_4591, 0x9040_327f_9705_7508];
    for ((e, s, c), pin) in cases().into_iter().zip(pins) {
      assert_eq!(fnv(&build_zip_with_limit(&e, s, c)), pin);
    }
  }
}
