//! STORE-ZIP packing over FFI — the desktop half of the zip_writer de-dup.
//!
//! `lib/upload/zip_writer.dart` used to be a hand-written Dart twin of
//! `mica_interchange::zip::writer` (the encoder the server and the export
//! paths already use). Two hand-rolled binary encoders that must stay
//! byte-compatible is the worst kind of double representation — a drift
//! produces an unopenable archive. Desktop now calls this; web keeps the Dart
//! reference implementation (no FFI there), and a shared gold fixture pins
//! the two byte-for-byte (`crates/interchange/tests/zip_gold.rs` ↔
//! `test/zip_writer_conformance_test.dart`).

use flutter_rust_bridge::frb;

/// One file headed into the archive (folder/multi-file import packing).
pub struct StoreZipEntry {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Build a STORE (uncompressed) ZIP — the upload container for server-side
/// import. No compression on purpose: it goes straight to our own backend,
/// and md/images don't gain much anyway. UTF-8 name flag set.
#[frb(sync)]
pub fn build_store_zip(entries: Vec<StoreZipEntry>) -> Vec<u8> {
    let entries: Vec<mica_interchange::ZipEntry> = entries
        .into_iter()
        .map(|e| mica_interchange::ZipEntry {
            name: e.name,
            data: e.bytes,
        })
        .collect();
    mica_interchange::build_zip(&entries)
}
