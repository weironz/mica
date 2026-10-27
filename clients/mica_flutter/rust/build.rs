//! Bake a fingerprint of this library's Rust sources into the binary.
//!
//! Tests load whatever `rust_lib_mica_flutter` sits on disk (beside the
//! executable, else FRB's `rust/target/release` fallback), and nothing ties
//! that file to the checkout: a library left over from an older build loads
//! fine and quietly tests old code. `test/support/native_build.dart` recomputes
//! this hash from the working tree and refuses to run against a mismatch.
//!
//! The algorithm is mirrored there, byte for byte: FNV-1a 64 over every file
//! under ROOTS, sorted by repo-relative `/` path, each fed as
//! `path \0 contents \0`. ROOTS travels inside the binary too, so the Dart side
//! never keeps its own copy of the list.
use std::path::{Path, PathBuf};

/// Repo-relative inputs of this library: the bridge crate plus its path deps.
const ROOTS: &[&str] = &[
  "clients/mica_flutter/rust/Cargo.lock",
  "clients/mica_flutter/rust/Cargo.toml",
  "clients/mica_flutter/rust/build.rs",
  "clients/mica_flutter/rust/src",
  "crates/interchange/Cargo.toml",
  "crates/interchange/src",
  "crates/markdown/Cargo.toml",
  "crates/markdown/src",
  "crates/md-core/Cargo.toml",
  "crates/md-core/src",
  "crates/mica-core/Cargo.toml",
  "crates/mica-core/src",
];

fn collect(repo: &Path, rel: &str, out: &mut Vec<String>) {
  let path = repo.join(rel);
  if path.is_dir() {
    for entry in std::fs::read_dir(&path).expect("read source dir") {
      let name = entry.expect("dir entry").file_name();
      collect(repo, &format!("{rel}/{}", name.to_string_lossy()), out);
    }
  } else if path.is_file() {
    out.push(rel.to_owned());
  }
}

fn main() {
  let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
  let repo = manifest.join("../../..");
  let mut files = Vec::new();
  for root in ROOTS {
    println!("cargo:rerun-if-changed={}", repo.join(root).display());
    collect(&repo, root, &mut files);
  }
  files.sort();
  let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
  let mut feed = |bytes: &[u8]| {
    for b in bytes {
      hash ^= u64::from(*b);
      hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
  };
  for rel in &files {
    feed(rel.as_bytes());
    feed(&[0]);
    feed(&std::fs::read(repo.join(rel)).expect("read source file"));
    feed(&[0]);
  }
  println!("cargo:rustc-env=MICA_FFI_SOURCE_HASH={hash:016x}");
  println!("cargo:rustc-env=MICA_FFI_SOURCE_ROOTS={}", ROOTS.join(";"));
}
