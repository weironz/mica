//! Pins inline parsing as NON-exponential in bracket nesting.
//!
//! CommonMark's "a link's text may not contain a link" rule needs a recursive
//! re-parse of every bracket label. Two things keep that from exploding, and
//! this file guards both:
//!
//! 1. LAZINESS — the check only gates the two link forms, so brackets matching
//!    no link form never pay it. Guards `[[[[a]]]]` (was ~6s at depth 24).
//! 2. A SHARED MEMO (`LinkCache`) — without it, a label that DOES contain a link
//!    is parsed twice per level (once for the check, once by the scan falling
//!    through the rejected brackets): T(n) = 2·T(n-1). Guards
//!    `[[[a](/u)](/u)](/u)` (was 834ms at depth 20, ~8s at 22).
//!
//! Both shapes are asserted because fixing only the first still left a real DoS
//! vector: `import_markdown` runs on user-supplied markdown (import + MCP), so
//! ~170 bytes of the second shape used to pin a core for minutes.
//!
//! Thresholds are deliberately loose (seconds against a ~0ms actual) so they
//! catch a return to exponential without flaking on a loaded CI box.

fn parse_ms(src: &str) -> u128 {
    let started = std::time::Instant::now();
    let payload = mica_markdown::import_markdown(src, "root");
    assert!(!payload.blocks.is_empty(), "sanity: it should still parse");
    started.elapsed().as_millis()
}

#[test]
fn plain_bracket_nesting_is_not_exponential() {
    let depth = 24;
    let src = format!("{}a{}", "[".repeat(depth), "]".repeat(depth));
    let ms = parse_ms(&src);
    assert!(
        ms < 3000,
        "depth-{depth} PLAIN nesting took {ms}ms — the nested-link check is \
         eager again (exponential even for non-links); keep it lazy"
    );
}

#[test]
fn nested_link_nesting_is_not_exponential() {
    let depth = 24;
    let src = format!("{}a{}", "[".repeat(depth), "](/u)".repeat(depth));
    let ms = parse_ms(&src);
    assert!(
        ms < 3000,
        "depth-{depth} NESTED-LINK nesting took {ms}ms — the label_contains_link \
         memo is no longer shared across the recursive parses, so each level is \
         re-parsed twice again (T(n) = 2*T(n-1)); keep LinkCache threaded"
    );
}
