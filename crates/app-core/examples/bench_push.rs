//! What the "write amplification" on every push actually costs, in milliseconds.
//!
//! ```text
//! cargo run --release -p mica-app-core --example bench_push
//! ```
//!
//! `push_update` rebuilds the whole document on every push: decode the base,
//! apply the update, re-encode it, re-derive `content_text`, rewrite the row.
//! That is O(document), not O(update) — a true and often-repeated observation
//! which, on its own, says nothing about whether it MATTERS. This measures the
//! part that scales with document size, so the next person decides with a
//! number instead of the adjective.
//!
//! It exists because the answer was counter-intuitive. Measured 2026-08-04
//! against production's real size distribution (3779 documents: p50 6.6 kB,
//! p90 27 kB, p99 68 kB, max 312 kB of encoded state):
//!
//! ```text
//! p50     6810 B state   0.08 ms
//! p99    67787 B state   1.04 ms
//! max   314209 B state   3.60 ms
//! ```
//!
//! An end-to-end push measured ~21 ms on production, so this "expensive" round
//! trip is 1–15% of it; the rest is database round trips and network. Rewriting
//! it would mean re-implementing GC by hand (the round trip IS the squash — see
//! the roadmap entry) and accepting a stale search index, to save single-digit
//! milliseconds on the largest document that exists. Hence: deliberately not
//! done, with the trigger conditions written into the roadmap entry.
//!
//! Re-run this when documents get much bigger. The cost is roughly linear in
//! state bytes, so it extrapolates.
use std::time::Instant;

/// Block count chosen so the ENCODED STATE lands near a production percentile.
/// State is what the cost scales with, and it runs several times the text bytes
/// — sizing by text instead is how the first cut of this bench ended up
/// measuring documents 3x larger than any that exist.
const CASES: [(&str, usize); 3] = [
    ("p50   ~7 kB", 14),
    ("p99  ~68 kB", 141),
    ("max ~312 kB", 648),
];

fn main() {
    for (label, blocks) in CASES {
        let mut bs = vec![mica_core::Block::new("r", "page")
            .with_children((0..blocks).map(|i| format!("b{i}")).collect())];
        for i in 0..blocks {
            bs.push(
                mica_core::Block::new(format!("b{i}"), "paragraph").with_text(&"字".repeat(120)),
            );
        }
        let state = mica_core::MicaDoc::from_blocks("r", &bs).encode_state();

        // The three things push_update does that scale with the document.
        let n = 300;
        let t = Instant::now();
        for _ in 0..n {
            let d = mica_core::MicaDoc::from_update(&state).unwrap();
            std::hint::black_box(d.encode_state());
            std::hint::black_box(d.to_blocks());
        }
        println!(
            "{label}   state = {:>7} B   decode + re-encode + to_blocks = {:>5.2} ms",
            state.len(),
            t.elapsed().as_secs_f64() * 1000.0 / n as f64
        );
    }
}
