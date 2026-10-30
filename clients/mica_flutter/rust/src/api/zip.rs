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

/// A workspace archive unpacked for the LOCAL import: the entries after the
/// same normalizations the server applies (junk filtering, single-wrapper
/// peel, nested-zip expansion), plus the peeled wrapper's name.
pub struct UnpackedZip {
    pub wrapper_name: Option<String>,
    /// Entries that become pages (the plan core's Markdown rule). Zero means
    /// the archive holds nothing to import.
    pub markdown_count: u32,
    pub entries: Vec<StoreZipEntry>,
}

/// Read a workspace ZIP for the local (offline) import. The server import
/// reads archives with the same two calls (`read_zip` then the archive
/// normalization inside `plan_import`), so a ZIP imports into the same tree
/// in both worlds. Errors are the reader's message (corrupt, encrypted,
/// over the inflate budget).
pub fn unpack_import_zip(bytes: Vec<u8>) -> Result<UnpackedZip, String> {
    let raw = mica_interchange::read_zip(&bytes).map_err(|e| e.to_string())?;
    // The reader answers a file with no central directory at all (a renamed
    // PDF, a truncated download) with zero entries, not an error. Locally that
    // would be a new, EMPTY workspace and a "done" message; say what happened.
    if raw.is_empty() {
        return Err("not a ZIP archive, or an empty one".to_string());
    }
    let archive = mica_interchange::normalize_archive(raw).map_err(|e| e.to_string())?;
    Ok(UnpackedZip {
        markdown_count: archive.markdown_count() as u32,
        wrapper_name: archive.wrapper_name,
        entries: archive
            .entries
            .into_iter()
            .map(|e| StoreZipEntry {
                name: e.name,
                bytes: e.bytes,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        build_store_zip(
            entries
                .iter()
                .map(|(name, bytes)| StoreZipEntry {
                    name: name.to_string(),
                    bytes: bytes.to_vec(),
                })
                .collect(),
        )
    }

    #[test]
    fn a_single_wrapper_folder_is_peeled_and_named() {
        let out = unpack_import_zip(zip(&[
            ("Vault/A.md", b"# A"),
            ("Vault/sub/B.md", b"# B"),
            ("Vault/img.png", b"\x89PNG"),
        ]))
        .unwrap();
        assert_eq!(out.wrapper_name.as_deref(), Some("Vault"));
        assert_eq!(out.markdown_count, 2);
        let mut names: Vec<_> = out.entries.iter().map(|e| e.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["A.md", "img.png", "sub/B.md"]);
    }

    #[test]
    fn a_nested_archive_is_expanded() {
        let inner = zip(&[("Inner.md", b"# Inner")]);
        let out = unpack_import_zip(zip(&[("Top.md", b"# Top"), ("part.zip", &inner)])).unwrap();
        let names: Vec<_> = out.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(
            names.contains(&"Top.md") && names.iter().any(|n| n.ends_with("Inner.md")),
            "{names:?}"
        );
        assert!(!names.iter().any(|n| n.ends_with(".zip")), "{names:?}");
    }

    #[test]
    fn garbage_is_an_error_not_an_empty_import() {
        assert!(unpack_import_zip(b"not a zip".to_vec()).is_err());
    }
}
