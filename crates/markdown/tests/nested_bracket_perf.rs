//! Guards against re-introducing EAGER `label_contains_link`.
//!
//! The CommonMark "a link's text may not contain a link" rule needs a recursive
//! re-parse of every bracket label. Computing it up-front for EVERY `[` made
//! even plain bracket nesting exponential (depth 24 ≈ 6s release). It is now
//! computed lazily, so brackets matching no link form cost nothing.
//!
//! WHAT THIS TEST DOES **NOT** COVER — stated plainly so it is not mistaken for
//! a clean bill of health: nested *links* (`[[[a](/u)](/u)](/u)`) are STILL
//! exponential — each level forces the check, gets rejected, and the span is
//! re-parsed on the way back down. Measured in release: depth 16 44ms, 18 191ms,
//! 20 834ms from 121 bytes. `import_markdown` runs on user-supplied markdown, so
//! that residual is a real (pre-existing) DoS vector; it is tracked in
//! docs/code-review-2026-07-20.md and is NOT fixed by the laziness this pins.
//! No assertion is made about that shape here because it would have to encode
//! the bad behavior as "expected".

#[test]
fn plain_bracket_nesting_is_not_exponential() {
    let depth = 24;
    let src = format!("{}a{}", "[".repeat(depth), "]".repeat(depth));
    let started = std::time::Instant::now();
    let payload = mica_markdown::import_markdown(&src, "root");
    let elapsed = started.elapsed();
    assert!(
        !payload.blocks.is_empty(),
        "sanity: the document should still parse"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "depth-{depth} PLAIN bracket nesting took {elapsed:?} — label_contains_link \
         is being evaluated eagerly again (exponential even for non-links); keep it lazy"
    );
}
