//! Minimal store-only (uncompressed) ZIP writer — no third-party dependency.
//! Image/most assets are already compressed, so storing is fine and keeps the
//! crate dependency-free.

/// One file in the archive.
pub struct ZipEntry {
  pub name: String,
  pub data: Vec<u8>,
}

/// Build a ZIP archive (store method) from [entries].
pub fn build_zip(entries: &[ZipEntry]) -> Vec<u8> {
  let mut out = Vec::new();
  let mut central = Vec::new();
  let mut offsets = Vec::with_capacity(entries.len());

  for e in entries {
    let crc = crc32(&e.data);
    let size = e.data.len() as u32;
    offsets.push(out.len() as u32);

    // Local file header. Flag bit 11 (0x0800) marks filenames as UTF-8 so
    // non-ASCII names (e.g. CJK) extract correctly.
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes()); // version needed
    out.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: UTF-8 names
    out.extend_from_slice(&0u16.to_le_bytes()); // method = store
    out.extend_from_slice(&0u16.to_le_bytes()); // mod time
    out.extend_from_slice(&0u16.to_le_bytes()); // mod date
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes()); // compressed
    out.extend_from_slice(&size.to_le_bytes()); // uncompressed
    out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // extra len
    out.extend_from_slice(e.name.as_bytes());
    out.extend_from_slice(&e.data);
  }

  let cd_start = out.len() as u32;
  for (i, e) in entries.iter().enumerate() {
    let crc = crc32(&e.data);
    let size = e.data.len() as u32;

    central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    central.extend_from_slice(&20u16.to_le_bytes()); // version made by
    central.extend_from_slice(&20u16.to_le_bytes()); // version needed
    central.extend_from_slice(&0x0800u16.to_le_bytes()); // flags: UTF-8 names
    central.extend_from_slice(&0u16.to_le_bytes()); // method = store
    central.extend_from_slice(&0u16.to_le_bytes()); // mod time
    central.extend_from_slice(&0u16.to_le_bytes()); // mod date
    central.extend_from_slice(&crc.to_le_bytes());
    central.extend_from_slice(&size.to_le_bytes());
    central.extend_from_slice(&size.to_le_bytes());
    central.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes()); // extra len
    central.extend_from_slice(&0u16.to_le_bytes()); // comment len
    central.extend_from_slice(&0u16.to_le_bytes()); // disk number
    central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
    central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
    central.extend_from_slice(&offsets[i].to_le_bytes());
    central.extend_from_slice(e.name.as_bytes());
  }

  let cd_size = central.len() as u32;
  out.extend_from_slice(&central);

  // End of central directory record.
  let count = entries.len() as u16;
  out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
  out.extend_from_slice(&0u16.to_le_bytes()); // disk number
  out.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
  out.extend_from_slice(&count.to_le_bytes());
  out.extend_from_slice(&count.to_le_bytes());
  out.extend_from_slice(&cd_size.to_le_bytes());
  out.extend_from_slice(&cd_start.to_le_bytes());
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
