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

#[test]
fn a_huge_declared_size_is_not_reserved_up_front() {
  // Before the fix: Vec::with_capacity(1 << 60) → allocation failure → abort.
  let out = inflate(&EMPTY_DEFLATE, 1 << 60, usize::MAX).expect("valid stream");
  assert!(out.is_empty());
}

#[test]
fn an_archive_lying_about_its_entry_size_still_reads() {
  let mut zip = build_zip(&[ZipEntry { name: "a.md".into(), data: EMPTY_DEFLATE.to_vec() }]);
  // Rewrite the STORE entry as DEFLATE and make the central directory claim
  // 4 GiB uncompressed — the shape of the crafted upload.
  let local = 0;
  let central = (0..zip.len() - 4)
    .find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x01, 0x02])
    .expect("central directory");
  zip[local + 8] = 8;
  zip[central + 10] = 8;
  zip[central + 24..central + 28].copy_from_slice(&0xffff_fffe_u32.to_le_bytes());

  let entries = read_zip(&zip);
  assert_eq!(entries.len(), 1);
  assert!(entries[0].bytes.is_empty());
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
    ZipEntry { name: "a.md".into(), data: vec![b'a'; 100] },
    ZipEntry { name: "b.md".into(), data: vec![b'b'; 100] },
  ]);
  let mut budget = 150;
  let entries = read_zip_within(&zip, &mut budget);
  assert_eq!(entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["a.md"]);
  assert_eq!(budget, 50);
}
