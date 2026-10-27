//! Fingerprint the Web binding and its engine using the desktop FNV-1a format.
use std::path::{Path, PathBuf};

const ROOTS: &[&str] = &[
  "crates/md-core/Cargo.toml",
  "crates/md-core/src",
  "crates/md-wasm/Cargo.toml",
  "crates/md-wasm/src",
  "crates/md-wasm/build.rs",
];

fn collect(repo: &Path, rel: &str, files: &mut Vec<String>) {
  let path = repo.join(rel);
  if path.is_dir() {
    for entry in std::fs::read_dir(path).expect("read source dir") {
      let name = entry.expect("source entry").file_name();
      collect(repo, &format!("{rel}/{}", name.to_string_lossy()), files);
    }
  } else {
    assert!(path.is_file(), "missing source: {rel}");
    files.push(rel.to_owned());
  }
}

fn main() {
  let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
  let repo = manifest.join("../..");
  let mut files = Vec::new();
  for root in ROOTS {
    println!("cargo:rerun-if-changed={}", repo.join(root).display());
    collect(&repo, root, &mut files);
  }
  files.sort();
  let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
  for rel in files {
    let contents = std::fs::read(repo.join(&rel)).expect("read source file");
    for byte in rel
      .as_bytes()
      .iter()
      .chain(&[0])
      .chain(&contents)
      .chain(&[0])
    {
      hash ^= u64::from(*byte);
      hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
  }
  println!("cargo:rustc-env=MICA_MD_SOURCE_HASH={hash:016x}");
}
