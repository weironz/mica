//! Memory ceilings on untrusted archives.
//!
//! Regression for a 125-byte zip that took the whole api-server down: the
//! inflater reserved the header's DECLARED uncompressed size up front, and a
//! failed allocation in Rust aborts the process — no panic for
//! `spawn_blocking` or the task boundary to catch. These tests run in-process,
//! so a regression shows up as the test binary aborting, not as a failure.

use mica_interchange::zip::inflate::inflate;
use mica_interchange::zip::{ZipEntry, build_zip, read_zip, read_zip_within};

/// A complete DEFLATE stream with no output: one final fixed-Huffman block
/// holding only the end-of-block code.
const EMPTY_DEFLATE: [u8; 2] = [0x03, 0x00];

/// Reference CRC-32 (matches the writer's implementation) for hand-assembled
/// fixtures that must be structurally well-formed.
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

#[test]
fn a_huge_declared_size_is_not_reserved_up_front() {
  // Before the fix: Vec::with_capacity(1 << 60) → allocation failure → abort.
  let out = inflate(&EMPTY_DEFLATE, 1 << 60, usize::MAX).expect("valid stream");
  assert!(out.is_empty());
}

#[test]
fn an_archive_lying_about_its_entry_size_is_rejected() {
  let mut zip = build_zip(&[ZipEntry {
    name: "a.md".into(),
    data: EMPTY_DEFLATE.to_vec(),
  }]);
  // Rewrite the STORE entry as DEFLATE and make the central directory claim
  // 4 GiB uncompressed — the shape of the crafted upload. The decode now
  // SUCCEEDS (to empty bytes), but the recorded CRC belongs to the original
  // stored bytes, so the entry is rejected as corrupt rather than accepted.
  let local = 0;
  let central = (0..zip.len() - 4)
    .find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x01, 0x02])
    .expect("central directory");
  zip[local + 8] = 8;
  zip[central + 10] = 8;
  zip[central + 24..central + 28].copy_from_slice(&0xffff_fffe_u32.to_le_bytes());

  let err = read_zip(&zip).expect_err("lying archive must be rejected");
  assert!(err.to_string().contains("a.md"), "{err}");
}

#[test]
fn inflate_refuses_output_past_its_limit() {
  // Stored block, final, 10 bytes.
  let mut stream = vec![0x01, 10, 0, !10u8, 0xff];
  stream.extend_from_slice(b"0123456789");
  assert!(inflate(&stream, 0, 9).is_err());
  assert_eq!(inflate(&stream, 0, 10).unwrap(), b"0123456789");
}

#[test]
fn entries_past_the_shared_budget_are_dropped() {
  let zip = build_zip(&[
    ZipEntry {
      name: "a.md".into(),
      data: vec![b'a'; 100],
    },
    ZipEntry {
      name: "b.md".into(),
      data: vec![b'b'; 100],
    },
  ]);
  let mut budget = 150;
  let entries = read_zip_within(&zip, &mut budget).unwrap();
  assert_eq!(
    entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
    ["a.md"]
  );
  assert_eq!(budget, 50);
}

/// One STORE entry whose CENTRAL record carries `cd_sizes` and `extra` verbatim,
/// hand-assembled so the extra field can lie about its own length. The CRC is
/// real, so a structurally well-formed entry reads (and a lying one still fails
/// on structure, not on CRC).
fn zip_with_central_extra(name: &str, data: &[u8], cd_sizes: u32, extra: &[u8]) -> Vec<u8> {
  let crc = crc32(data);
  let mut out = Vec::new();
  // Local header (honest sizes, no extra).
  out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
  out.extend_from_slice(&20u16.to_le_bytes()); // version needed
  out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
  out.extend_from_slice(&0u16.to_le_bytes()); // method
  out.extend_from_slice(&0u16.to_le_bytes()); // mod time
  out.extend_from_slice(&0u16.to_le_bytes()); // mod date
  out.extend_from_slice(&crc.to_le_bytes());
  out.extend_from_slice(&(data.len() as u32).to_le_bytes());
  out.extend_from_slice(&(data.len() as u32).to_le_bytes());
  out.extend_from_slice(&(name.len() as u16).to_le_bytes());
  out.extend_from_slice(&0u16.to_le_bytes());
  out.extend_from_slice(name.as_bytes());
  out.extend_from_slice(data);

  let cd = out.len();
  out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
  out.extend_from_slice(&45u16.to_le_bytes()); // made by
  out.extend_from_slice(&45u16.to_le_bytes()); // needed
  out.extend_from_slice(&0x0800u16.to_le_bytes());
  out.extend_from_slice(&0u16.to_le_bytes()); // method
  out.extend_from_slice(&0u16.to_le_bytes()); // mod time
  out.extend_from_slice(&0u16.to_le_bytes()); // mod date
  out.extend_from_slice(&crc.to_le_bytes());
  out.extend_from_slice(&cd_sizes.to_le_bytes()); // compressed
  out.extend_from_slice(&cd_sizes.to_le_bytes()); // uncompressed
  out.extend_from_slice(&(name.len() as u16).to_le_bytes());
  out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
  out.extend_from_slice(&[0; 2 + 2 + 2 + 4]); // comment len, disk, attrs
  out.extend_from_slice(&0u32.to_le_bytes()); // local header offset
  out.extend_from_slice(name.as_bytes());
  out.extend_from_slice(extra);
  let cd_len = out.len() - cd;

  out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
  out.extend_from_slice(&[0; 4]); // disk numbers
  out.extend_from_slice(&1u16.to_le_bytes());
  out.extend_from_slice(&1u16.to_le_bytes());
  out.extend_from_slice(&(cd_len as u32).to_le_bytes());
  out.extend_from_slice(&(cd as u32).to_le_bytes());
  out.extend_from_slice(&0u16.to_le_bytes());
  out
}

/// C10 (review 2026-10-21): the ZIP64 extra field (0x0001) declares its own
/// length, and the reader bounded its reads by THAT (`end = p + 4 + size`)
/// without checking it against the bytes actually present. A record claiming
/// 16 bytes while carrying 4 indexed past the slice → a panic inside the
/// import's `spawn_blocking`, i.e. a 500 instead of "no pages found" (400).
#[test]
fn a_zip64_extra_longer_than_the_field_is_skipped_not_a_panic() {
  // id 0x0001, declared size 16, but only 4 payload bytes follow.
  let extra = [0x01, 0x00, 0x10, 0x00, 0xaa, 0xbb, 0xcc, 0xdd];
  let zip = zip_with_central_extra("a.md", b"# hi", 0xffff_ffff, &extra);

  let entries = std::panic::catch_unwind(|| read_zip(&zip))
    .expect("read_zip must not panic")
    .unwrap();
  // The sizes are still the 0xffffffff markers, so the entry cannot be located
  // and is dropped like any other unreadable entry.
  assert!(entries.is_empty(), "{entries:?}");

  // What the server's import does with that: no pages → the coded 400
  // (`import_no_markdown`), not a panicked task.
  let plan =
    mica_interchange::plan_import(entries, false, mica_interchange::ImportMode::AsIs).unwrap();
  assert!(plan.pages.is_empty());
}

/// The well-formed twin: a ZIP64 extra that really carries both sizes is read,
/// so the bound did not simply switch ZIP64 off.
#[test]
fn a_well_formed_zip64_extra_still_reads() {
  let data = b"# hi";
  let mut extra = vec![0x01, 0x00, 0x10, 0x00];
  extra.extend_from_slice(&(data.len() as u64).to_le_bytes()); // uncompressed
  extra.extend_from_slice(&(data.len() as u64).to_le_bytes()); // compressed
  let zip = zip_with_central_extra("a.md", data, 0xffff_ffff, &extra);

  let entries = read_zip(&zip).unwrap();
  assert_eq!(entries.len(), 1);
  assert_eq!(entries[0].bytes, data);
}
/// A single DEFLATE entry whose recorded CRC is caller-supplied — hand-assembled
/// so the test can make the decode SUCCEED while the CRC is wrong (the shape a
/// bit-flipped transfer produces when the inflate stream survives intact).
fn deflate_zip(name: &str, crc: u32, deflate: &[u8], uncomp_size: u32) -> Vec<u8> {
  let mut out = Vec::new();
  out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
  out.extend_from_slice(&20u16.to_le_bytes()); // version needed
  out.extend_from_slice(&0u16.to_le_bytes()); // flags
  out.extend_from_slice(&8u16.to_le_bytes()); // method = deflate
  out.extend_from_slice(&0u16.to_le_bytes()); // mod time
  out.extend_from_slice(&0u16.to_le_bytes()); // mod date
  out.extend_from_slice(&crc.to_le_bytes());
  out.extend_from_slice(&(deflate.len() as u32).to_le_bytes()); // compressed
  out.extend_from_slice(&uncomp_size.to_le_bytes()); // uncompressed
  out.extend_from_slice(&(name.len() as u16).to_le_bytes());
  out.extend_from_slice(&0u16.to_le_bytes()); // extra len
  out.extend_from_slice(name.as_bytes());
  out.extend_from_slice(deflate);

  let cd = out.len();
  out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
  out.extend_from_slice(&20u16.to_le_bytes()); // made by
  out.extend_from_slice(&20u16.to_le_bytes()); // needed
  out.extend_from_slice(&0u16.to_le_bytes()); // flags
  out.extend_from_slice(&8u16.to_le_bytes()); // method
  out.extend_from_slice(&0u16.to_le_bytes()); // mod time
  out.extend_from_slice(&0u16.to_le_bytes()); // mod date
  out.extend_from_slice(&crc.to_le_bytes());
  out.extend_from_slice(&(deflate.len() as u32).to_le_bytes());
  out.extend_from_slice(&uncomp_size.to_le_bytes());
  out.extend_from_slice(&(name.len() as u16).to_le_bytes());
  out.extend_from_slice(&0u16.to_le_bytes()); // extra len
  out.extend_from_slice(&0u16.to_le_bytes()); // comment len
  out.extend_from_slice(&0u16.to_le_bytes()); // disk number
  out.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
  out.extend_from_slice(&0u32.to_le_bytes()); // external attrs
  out.extend_from_slice(&0u32.to_le_bytes()); // local header offset
  out.extend_from_slice(name.as_bytes());
  let cd_len = out.len() - cd;

  out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
  out.extend_from_slice(&[0; 4]); // disk numbers
  out.extend_from_slice(&1u16.to_le_bytes()); // entries on this disk
  out.extend_from_slice(&1u16.to_le_bytes()); // entries total
  out.extend_from_slice(&(cd_len as u32).to_le_bytes());
  out.extend_from_slice(&(cd as u32).to_le_bytes());
  out.extend_from_slice(&0u16.to_le_bytes());
  out
}

/// Issue #94: a single corrupted STORE byte (payload mutated, recorded CRC kept)
/// must be rejected — never imported as a page with silently-wrong content.
#[test]
fn corrupted_store_entry_is_not_silently_read() {
  let mut bytes = build_zip(&[ZipEntry {
    name: "note.md".into(),
    data: b"critical=100".to_vec(),
  }]);
  let at = bytes
    .windows(12)
    .position(|w| w == b"critical=100")
    .expect("payload present");
  bytes[at + 9] = b'9';
  let err = read_zip(&bytes).expect_err("corrupted STORE entry must be rejected");
  assert!(err.to_string().contains("note.md"), "{err}");
}

/// Issue #94: DEFLATE entries verify CRC too — a decode that succeeds but does
/// not match the recorded CRC must be rejected.
#[test]
fn corrupted_deflate_entry_is_not_silently_read() {
  // raw DEFLATE of "hello world" (11 bytes out, 13 bytes stored).
  const DEFLATE_HELLO: [u8; 13] = [
    0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0x28, 0xcf, 0x2f, 0xca, 0x49, 0x01, 0x00,
  ];
  // crc("hello world") is 0x0d4a1185; record the off-by-one so decode
  // succeeds but the checksum cannot match.
  let zip = deflate_zip("note.md", 0x0d4a_1184, &DEFLATE_HELLO, 11);
  let err = read_zip(&zip).expect_err("CRC-mismatched DEFLATE entry must be rejected");
  assert!(err.to_string().contains("note.md"), "{err}");
}
