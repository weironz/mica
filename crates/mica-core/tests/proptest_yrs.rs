//! Regression tests and property fuzz for untrusted yrs binary updates.
//!
//! On yrs 0.27.3 this surface found invalid UTF-8 UB, an allocation abort
//! (21 bytes requesting 215 TB), and ordinary unwinding panics. Fallible
//! allocation shipped in 0.27.4 (#639); checked UTF-8 shipped in 0.28.0 (#644).
//! The two fixed cases below are ACTIVE regression gates for decode and apply.
//!
//! The four stronger "never panic" properties remain ignored: yrs 0.28.0
//! still panics on malformed updates (e.g. block.rs:92; upstream #415).
//! Do not mistake fixing the two abort cases for a total decoder guarantee.
//! Server and local-store catch_unwind guards remain necessary. Deliberate fuzz:
//!
//! ```text
//! cargo test -p mica-core --test proptest_yrs -- --ignored
//! ```
//!
//! Longer hunt: `PROPTEST_CASES=100000 …`.
use mica_core::{Block, DocError, MicaDoc};
use proptest::prelude::*;

const TEXT: &str = "hello world";

/// A small, valid document — the base the mutation strategies corrupt.
fn sample_state() -> Vec<u8> {
    MicaDoc::from_blocks(
        "r",
        &[
            Block::new("r", "page").with_children(vec!["a".into()]),
            Block::new("a", "paragraph").with_text(TEXT),
        ],
    )
    .encode_state()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Arbitrary bytes: the widest net, covering anything a socket can deliver.
    /// Almost all of it is rejected at the first header byte, which is the point
    /// — the cheap rejection path is also the one nobody looks at.
    #[test]
    #[ignore = "yrs 0.28.0 still panics on malformed updates; upstream #415"]
    fn from_update_never_panics_on_arbitrary_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        let _ = MicaDoc::from_update(&bytes);
    }

    /// Bytes that are ALMOST a real update — a valid encoding with one byte
    /// flipped. This is where decoders actually break: the header parses, the
    /// lengths look plausible, and the reader walks off the end of something.
    /// Random bytes rarely get that far, which is why both strategies exist.
    #[test]
    #[ignore = "yrs 0.28.0 still panics on malformed updates; upstream #415"]
    fn from_update_never_panics_on_a_corrupted_real_state(
        idx in any::<prop::sample::Index>(),
        xor in 1u8..=255,
    ) {
        let mut bad = sample_state();
        let i = idx.index(bad.len());
        bad[i] ^= xor;
        let _ = MicaDoc::from_update(&bad);
    }

    /// The remotely-reachable path in `push_update`: apply an untrusted update
    /// ONTO an existing document. Different code from a cold decode — it merges
    /// into live structures — and it is the one an authenticated client drives.
    #[test]
    #[ignore = "yrs 0.28.0 still panics on malformed updates; upstream #415"]
    fn apply_update_never_panics_on_arbitrary_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        let mut doc = MicaDoc::from_update(&sample_state()).expect("sample decodes");
        let _ = doc.apply_update(&bytes);
    }

    /// Same path, near-miss input. Feeding a document a mutated copy of its own
    /// state exercises the merge with structurally familiar-but-wrong data.
    #[test]
    #[ignore = "yrs 0.28.0 still panics on malformed updates; upstream #415"]
    fn apply_update_never_panics_on_a_corrupted_real_update(
        idx in any::<prop::sample::Index>(),
        xor in 1u8..=255,
    ) {
        let mut bad = sample_state();
        let i = idx.index(bad.len());
        bad[i] ^= xor;
        let mut doc = MicaDoc::from_update(&sample_state()).expect("sample decodes");
        let _ = doc.apply_update(&bad);
    }
}

/// The 21 bytes, kept exactly. A shrunk reproducer is the difference between a
/// bug report someone can act on and a story about a fuzzer.
///
/// yrs 0.27.3 aborted instead of returning an allocation/decode error.
#[test]
fn the_21_byte_allocation_bomb_is_rejected() {
    let hex = "d7548f54770c78002d677698fbbfd5f35730ee3240";
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    assert!(matches!(MicaDoc::from_update(&bytes), Err(DocError::Decode(_))));
    let mut doc = MicaDoc::from_update(&sample_state()).expect("sample decodes");
    assert!(matches!(doc.apply_update(&bytes), Err(DocError::Decode(_))));
}

/// Locate the text rather than a byte offset: client IDs and map ordering vary.
/// Changing ASCII to 0xff keeps the lengths valid but makes the UTF-8 invalid.
#[test]
fn invalid_utf8_is_rejected_by_decode_and_apply() {
    let good = sample_state();
    let mut doc = MicaDoc::from_update(&good).expect("sample decodes");
    assert!(doc.apply_update(&good).is_ok(), "valid baseline applies");
    let mut bad = good;
    let at = bad.windows(TEXT.len()).position(|w| w == TEXT.as_bytes()).expect("encoded text");
    bad[at] = 0xff;
    assert!(matches!(MicaDoc::from_update(&bad), Err(DocError::Decode(_))));
    assert!(matches!(doc.apply_update(&bad), Err(DocError::Decode(_))));
}
