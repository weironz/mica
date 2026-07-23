//! Property fuzz for the ZIP import pipeline.
//!
//! `read_zip` parses untrusted uploaded archives byte-for-byte (local/central
//! headers, offsets, sizes, extra fields, GBK names) — a classic attack surface
//! (zip bombs, truncated/overlapping records, path traversal). A panic there
//! crashes an import job. We push arbitrary bytes through the whole no-DB stage
//! — `read_zip` → `normalize_entries` (the dot-segment / `..` path filter) →
//! `expand_nested_zips` (recursion on nested archive bytes) — and assert only:
//! **never panic.** On a hit, proptest shrinks to a minimal byte string; copy it
//! into a fixed regression fixture and fix the reader.

use mica_interchange::{expand_nested_zips, normalize_entries, read_zip};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 2048, ..ProptestConfig::default() })]

    #[test]
    fn zip_pipeline_never_panics_on_arbitrary_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..16384),
    ) {
        // read_zip is deliberately lenient (returns whatever it recovered), so
        // push the result on through the security-relevant normalization and the
        // nested-unzip recursion — each is fed attacker-controlled data.
        let entries = read_zip(&bytes);
        let normalized = normalize_entries(entries);
        let _ = expand_nested_zips(normalized);
    }

    /// Bias toward inputs that begin with the ZIP local-file-header magic
    /// (`PK\x03\x04`), so the mutator spends its budget PAST the signature check
    /// exercising header/field parsing rather than bouncing off "not a zip".
    #[test]
    fn zip_pipeline_never_panics_on_pk_prefixed(
        tail in proptest::collection::vec(any::<u8>(), 0..16384),
    ) {
        let mut bytes = vec![0x50, 0x4b, 0x03, 0x04];
        bytes.extend_from_slice(&tail);
        let entries = read_zip(&bytes);
        let normalized = normalize_entries(entries);
        let _ = expand_nested_zips(normalized);
    }
}
