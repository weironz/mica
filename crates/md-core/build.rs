//! Bake an FNV-1a 64 fingerprint of the engine's sources into the binary.
//!
//! Mirrors the desktop FFI crate's build script: every file under the crate's
//! `Cargo.toml` + `src/`, sorted by path, fed as `path \0 contents \0`. The
//! algorithm is intentionally the same shape so a binding can compare hashes
//! across crates without a second implementation.

use std::path::{Path, PathBuf};

fn collect(current: &Path, out: &mut Vec<PathBuf>) {
  let mut entries: Vec<PathBuf> = std::fs::read_dir(current)
    .expect("read source dir")
    .map(|entry| entry.expect("dir entry").path())
    .collect();
  entries.sort();
  for path in entries {
    if path.is_dir() {
      collect(&path, out);
    } else if path.is_file() {
      out.push(path);
    }
  }
}

fn main() {
  let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
  println!("cargo:rerun-if-changed=Cargo.toml");
  println!("cargo:rerun-if-changed=src");
  let mut files = vec![manifest.join("Cargo.toml")];
  collect(&manifest.join("src"), &mut files);
  let mut rel: Vec<(String, PathBuf)> = files
    .into_iter()
    .map(|path| {
      let name = path
        .strip_prefix(&manifest)
        .expect("path under crate root")
        .to_string_lossy()
        .replace('\\', "/");
      (name, path)
    })
    .collect();
  rel.sort_by(|a, b| a.0.cmp(&b.0));

  let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
  for (name, path) in rel {
    for b in name.as_bytes().iter().chain(&[0u8]) {
      hash ^= u64::from(*b);
      hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let contents = std::fs::read(path).expect("read source file");
    for b in contents.iter().chain(&[0u8]) {
      hash ^= u64::from(*b);
      hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    println!("cargo:rerun-if-changed={}", name);
  }
  println!("cargo:rustc-env=MICA_MD_SOURCE_HASH={hash:016x}");
}
