//! Gold fixture for the STORE-ZIP encoder — the cross-engine byte floor.
//!
//! The same archive is built by THREE implementations that must agree
//! byte-for-byte: this crate's `build_zip` (server + desktop FFI), the Flutter
//! web copy (`lib/upload/zip_writer_dart.dart`, checked against the same gold
//! in `test/zip_writer_conformance_test.dart`), and whatever reads the result
//! server-side. A drift here produces an unopenable upload, so the fixture is
//! a hard floor: regenerate ONLY on a deliberate format change, with
//! `GEN_GOLD=1 cargo test -p mica-interchange --test zip_gold`
//! and re-run the Dart conformance test in the same change.
use mica_interchange::{ZipEntry, build_zip};

fn gold_path() -> std::path::PathBuf {
  std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/store_zip.gold")
}

/// The shared fixture inputs. Mirrored VERBATIM in the Dart test — covers a
/// plain ASCII name, a CJK+space name (UTF-8 flag path), fully binary content
/// (every byte value), and an empty file.
fn fixture_entries() -> Vec<ZipEntry> {
  vec![
    ZipEntry {
      name: "README.md".into(),
      data: b"# Mica\n".to_vec(),
    },
    ZipEntry {
      name: "\u{7b14}\u{8bb0}/\u{56fe} 1.png".into(), // 笔记/图 1.png
      data: (0..=255u8).collect(),
    },
    ZipEntry {
      name: "empty.txt".into(),
      data: Vec::new(),
    },
  ]
}

#[test]
fn store_zip_matches_gold() {
  let built = build_zip(&fixture_entries());
  let path = gold_path();
  if std::env::var("GEN_GOLD").is_ok() {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &built).unwrap();
    return;
  }
  let gold = std::fs::read(&path)
    .expect("missing tests/fixtures/store_zip.gold — generate with GEN_GOLD=1");
  assert_eq!(
    built, gold,
    "STORE-ZIP bytes drifted from the gold fixture; if the format change is \
     deliberate, regenerate with GEN_GOLD=1 and update the Dart twin + its \
     conformance test in the same change"
  );
}
