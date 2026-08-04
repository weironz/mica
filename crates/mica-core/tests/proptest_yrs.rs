//! Property fuzz for the yrs binary-update surface — **and what it found.**
//!
//! This is the third of the three surfaces that eat untrusted bytes (the others
//! are `mica-markdown`'s `proptest_parse.rs` and `mica-interchange`'s
//! `proptest_zip.rs`, both of which came up clean). It arrived last because a
//! hand-written xor fuzz had already found UB in yrs itself, and UB is not what
//! proptest catches — the process was ABORTING, so the surface was parked for
//! cargo-fuzz + a sanitizer.
//!
//! **2026-08-04: parked was the wrong call. The surface is not clean.** Within
//! seconds these properties turned up THREE distinct failures on yrs 0.27.3
//! (the newest release; `unreachable_unchecked`, the old culprit, is indeed gone
//! from its source — which is why "upstream fixed it" looked true for about ten
//! minutes):
//!
//! 1. **`assert!` panic** — `yrs-0.27.3/src/block.rs:92`,
//!    `value & Self::MASK == 0`. Unwinding, so `guarded_from_update`'s
//!    `catch_unwind` contains it server-side. The mildest of the three.
//! 2. **Undefined behaviour** — `invalid value for char`, a NON-unwinding panic.
//!    `catch_unwind` structurally cannot contain it; in release the check is
//!    compiled out and it is silent UB.
//! 3. **Unbounded allocation** — 21 bytes make yrs ask for **215 TB**, and the
//!    allocation failure aborts the process. Reproduces in debug AND release,
//!    and `catch_unwind` does not stop it (see `the_21_byte_abort` below).
//!
//! (2) and (3) are reachable from `push_update`, i.e. by any AUTHENTICATED
//! client, and neither is containable from this side: bounding the input does
//! not help (21 bytes), and pre-validating would mean re-implementing the
//! decoder. It needs an upstream fix or process isolation.
//!
//! **Everything here is `#[ignore]`d on purpose**, not because it is flaky but
//! because two of the three failures ABORT the test process — an un-ignored
//! suite here would take CI down rather than report. Run it deliberately:
//!
//! ```text
//! cargo test -p mica-core --test proptest_yrs -- --ignored
//! ```
//!
//! Un-ignore these the day the upstream fix lands; they are the regression gate
//! for it. Longer hunt: `PROPTEST_CASES=100000 …`.
use mica_core::{Block, MicaDoc};
use proptest::prelude::*;

/// A small, valid document — the base the mutation strategies corrupt.
fn sample_state() -> Vec<u8> {
    MicaDoc::from_blocks(
        "r",
        &[
            Block::new("r", "page").with_children(vec!["a".into()]),
            Block::new("a", "paragraph").with_text("hello world"),
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
    #[ignore = "aborts or fails: upstream yrs, see the module docs"]
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
    #[ignore = "aborts or fails: upstream yrs, see the module docs"]
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
    #[ignore = "aborts or fails: upstream yrs, see the module docs"]
    fn apply_update_never_panics_on_arbitrary_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        let mut doc = MicaDoc::from_update(&sample_state()).expect("sample decodes");
        let _ = doc.apply_update(&bytes);
    }

    /// Same path, near-miss input. Feeding a document a mutated copy of its own
    /// state exercises the merge with structurally familiar-but-wrong data.
    #[test]
    #[ignore = "aborts or fails: upstream yrs, see the module docs"]
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
/// `catch_unwind` is here to make the point rather than to help: the call never
/// returns, so the assertion below is unreachable and the process dies instead.
#[test]
#[ignore = "aborts the process: yrs 0.27.3 asks for 215 TB on 21 bytes"]
fn the_21_byte_abort() {
    let hex = "d7548f54770c78002d677698fbbfd5f35730ee3240";
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let survived = std::panic::catch_unwind(|| MicaDoc::from_update(&bytes).is_ok());
    panic!("expected the process to abort before here, got {survived:?}");
}
