//! Property fuzz for the Markdown parser.
//!
//! `import_markdown` ingests untrusted text — anything a user types, pastes, or
//! an import feeds it. A panic inside it is a server-crash (DoS) vector, and the
//! parser is exactly the kind of hand-written state machine where a malformed
//! edge case slips through. proptest throws thousands of inputs at it and, on
//! any panic, SHRINKS to a minimal reproducer — copy that into a fixed
//! regression test and fix the parser. The property is simply: **never panic.**
//!
//! (yrs binary-update fuzzing — where a real UB was found by hand — needs
//! cargo-fuzz + a sanitizer, so it lives in CI/Linux, not here.)

use mica_markdown::import_markdown;
use proptest::prelude::*;

const ROOT: &str = "block_root";

// Kept modest so `cargo test` stays a fast regression gate — the full parser is
// ~50ms/case, so thousands would take minutes. For a real fuzzing session run
// locally with more: `PROPTEST_CASES=100000 cargo test -p mica-markdown
// --test proptest_parse` (or point cargo-fuzz at import_markdown on Linux).
proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Arbitrary raw bytes, lossily decoded — the widest robustness net: covers
    /// every byte sequence a socket/upload could deliver, not just tidy text.
    #[test]
    fn import_markdown_never_panics_on_arbitrary_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        let text = String::from_utf8_lossy(&bytes);
        let _ = import_markdown(&text, ROOT);
    }

    /// Strings assembled from Markdown-significant fragments — far likelier to
    /// reach deep block/inline branches (fences, headings, lists, links, tables,
    /// math, raw HTML, footnotes, GFM alerts) than uniform random text.
    #[test]
    fn import_markdown_never_panics_on_markdownish(text in markdownish()) {
        let _ = import_markdown(&text, ROOT);
    }
}

fn markdownish() -> impl Strategy<Value = String> {
    let token = proptest::sample::select(vec![
        "#", "##", "###", "######", "*", "**", "_", "~~", "`", "```", "```rust",
        "~~~", "> ", "> [!NOTE]", "- ", "* ", "1. ", "- [ ] ", "- [x] ",
        "[", "]", "(", ")", "!", "|", "---", "===", "$", "$$", "\\$", "\\",
        "<div>", "</div>", "<img src=x onerror=y>", "&amp;", "&#38;", "[^1]",
        "[^1]: note", "http://a.b/c", "[t](u)", "![a](b)", "\n", "\n\n", "\r\n",
        "\t", "    ", " ", "\u{0}", "\u{feff}", "重", "\u{1F600}",
    ]);
    proptest::collection::vec(token, 0..96).prop_map(|parts| parts.concat())
}
