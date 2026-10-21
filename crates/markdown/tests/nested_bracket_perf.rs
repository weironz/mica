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

/// A plain multi-line paragraph was CUBIC: every continuation line re-parsed
/// the whole paragraph's inlines, and `extended_autolink` copied the entire
/// remaining text at every word boundary of each parse. 2000 lines of `a`
/// (4 KB) took ~2 s in release, 4000 lines ~15 s. Continuations now defer the
/// inline parse to one pass at the end, and the autolink check is O(1).
#[test]
fn a_long_multi_line_paragraph_is_not_cubic() {
    let ms = parse_ms(&"a\n".repeat(4000));
    assert!(ms < 3000, "4000-line paragraph took {ms}ms — extended_autolink copies the rest of the text again");
    let ms = parse_ms(&"Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod\n".repeat(400));
    assert!(ms < 3000, "400 wrapped prose lines took {ms}ms");
}

/// `[<` repeated with no `>` anywhere: the `<` scan ran to the end of the text
/// from every position `matching_bracket` visits, from every `[` — cubic. Then
/// quadratic (every unmatched `[` still scanned to the end), until the bracket
/// answers were memoised per parse (`BracketMemo`): linear now.
#[test]
fn unclosed_angle_brackets_inside_brackets_are_not_cubic() {
    let ms = parse_ms(&"[<".repeat(8000));
    assert!(ms < 3000, "`[<`×8000 took {ms}ms — the bracket memo is gone or the `<` scan no longer stops");
}

/// Long runs of unmatched brackets of every kind the parser asks about.
#[test]
fn unmatched_bracket_runs_are_linear() {
    for (unit, n) in [("[", 50_000), ("![", 20_000), ("[^", 20_000)] {
        let ms = parse_ms(&unit.repeat(n));
        assert!(ms < 3000, "`{unit}`×{n} took {ms}ms — every `[` rescans to the end again");
    }
}

/// `[![[![…a](u)](u)](u)](u)` — links and images ALTERNATING. The shared
/// `LinkCache` above only memoised the link CHECK; an image's label was parsed
/// by `push_link` through a FRESH cache, so every link level below an image
/// re-ran its check from scratch: P(k) = 2·P(k-2). Measured before the fix
/// (`import_markdown`, ms; doubles every two levels):
///
/// | k  | bytes | debug  | release |
/// |----|-------|--------|---------|
/// | 20 |  111  |    7.8 |       — |
/// | 24 |  133  |   31.7 |     5.0 |
/// | 32 |  177  |  515   |    86   |
/// | 36 |  199  | 2029   |   326   |
/// | 40 |  221  | ~8000  |  1317   |
///
/// so k = 60 (~330 bytes) was ~20 min of CPU in release. Label parses are now
/// memoised whole, keyed by (label, depth), and shared by the check and by
/// `push_link`; nesting past `MAX_INLINE_NESTING` stays literal.
#[test]
fn alternating_link_image_nesting_is_not_exponential() {
    for k in [20, 40, 60] {
        let mut src = String::new();
        for d in 0..k {
            src.push_str(if d % 2 == 0 { "[" } else { "![" });
        }
        src.push('a');
        src.push_str(&"](u)".repeat(k));
        let ms = parse_ms(&src);
        assert!(
            ms < 3000,
            "depth-{k} ALTERNATING link/image nesting took {ms}ms — image labels \
             are parsed outside the shared label cache again (P(k) = 2·P(k-2))"
        );
    }
}

/// `![![![…a](u)](u)](u)` thousands deep: each image label is parsed by a
/// recursive call, so ~10 KB of input recursed ~2000 frames and overflowed a
/// 2 MiB stack (tokio's worker default) — a process abort, not a panic. 10-19 #6b.
#[test]
fn deep_image_nesting_does_not_overflow_the_stack() {
    for unit in ["![", "["] {
        let src = format!("{}a{}", unit.repeat(5000), "](u)".repeat(5000));
        let payload = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || mica_markdown::import_markdown(&src, "root"))
            .unwrap()
            .join()
            .expect("must not panic");
        let text = &payload.blocks[1].text;
        assert!(text.contains('a'), "content survives: {}", &text[..text.len().min(80)]);
    }
}
