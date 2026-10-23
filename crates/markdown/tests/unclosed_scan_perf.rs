//! Pins the remaining quadratic inline scans (issue #61) as NON-quadratic.
//!
//! Every shape below has an UNCLOSED (or never-matching) construct repeated n
//! times: the scan for each opener ran to the end of the text, so total work
//! was O(n²) on input of size O(n). `import_markdown` runs on user-supplied
//! markdown (import + MCP), and the client `marks.dart` mirror has the same
//! shapes, so each costs real availability. Same family as
//! `nested_bracket_perf.rs`; thresholds are deliberately loose (seconds
//! against a ~0ms actual) so they catch a return to quadratic without flaking
//! on a loaded CI box.

fn import_ms(src: &str) -> u128 {
    let started = std::time::Instant::now();
    let payload = mica_markdown::import_markdown(src, "root");
    assert!(!payload.blocks.is_empty(), "sanity: it should still parse");
    started.elapsed().as_millis()
}

/// `\(` with no `\)` anywhere: `find_pair` scanned to the end of the text for
/// EVERY `\(` opener — `\(x `×n was O(n²).
#[test]
fn unclosed_lparen_math_is_not_quadratic() {
    let src = "\\(x ".repeat(20_000);
    let ms = import_ms(&src);
    eprintln!("unclosed `\\(` x20000: {ms}ms");
    assert!(ms < 3000, "unclosed `\\(` spam took {ms}ms — find_pair scans to the end again");
}

/// `www.`-style repeats are fine, but `_a`×n puts an EMAIL candidate at every
/// boundary: the local-part scan walked the whole `[alnum._+-]` run to the end
/// of the text from each one — O(n²).
#[test]
fn email_like_boundaries_are_not_quadratic() {
    let src = "_a".repeat(30_000);
    let ms = import_ms(&src);
    eprintln!("`_a` x30000: {ms}ms");
    assert!(ms < 3000, "`_a`×30000 took {ms}ms — the email local-part scan reaches the end again");
}

/// `$a `×n: every `$` opens math per the Pandoc rules, and every later `$` is
/// rejected as a closer (whitespace before it), so each opener scanned to the
/// end — O(n²).
#[test]
fn unclosed_dollar_math_is_not_quadratic() {
    let src = "$a ".repeat(20_000);
    let ms = import_ms(&src);
    eprintln!("unclosed `$` x20000: {ms}ms");
    assert!(ms < 3000, "unclosed `$` spam took {ms}ms — find_math_closer scans to the end again");
}

/// `$a``a`b `×n: every `$` opens math and re-scans the tail THROUGH code spans
/// (each hop calls `code_span_close` again); every later `$` is rejected as a
/// closer, so each opener scanned to the end — O(n²), with the backtick
/// closer search on the path.
#[test]
fn math_scans_through_code_spans_are_not_quadratic() {
    let src = "$a``a`b ".repeat(20_000);
    let ms = import_ms(&src);
    eprintln!("unclosed `$` over code spans x20000: {ms}ms");
    assert!(ms < 3000, "unclosed `$` over code spans took {ms}ms — math/code-span rescans reach the end again");
}

/// `<a `×n: no `>` anywhere, so the autolink `<` scan ran to the end for every
/// `<` — O(n²).
#[test]
fn unclosed_angle_brackets_are_not_quadratic() {
    let src = "<a ".repeat(20_000);
    let ms = import_ms(&src);
    eprintln!("unclosed `<` x20000: {ms}ms");
    assert!(ms < 3000, "unclosed `<` spam took {ms}ms — the `>` scan reaches the end again");
}

/// `a <!--x `×n: the comment terminator search (`-->`) ran to the end for
/// every `<!--` — O(n²) through `inline_html_end`'s literal-pattern scan.
/// (Without the leading letter the line opens an HTML BLOCK and the inline
/// parser never runs.)
#[test]
fn unclosed_html_comments_are_not_quadratic() {
    let src = "a <!--x ".repeat(20_000);
    let ms = import_ms(&src);
    eprintln!("unclosed `<!--` x20000: {ms}ms");
    assert!(ms < 3000, "unclosed `<!--` spam took {ms}ms — inline_html_end rescans the tail again");
}

/// One paragraph of `**a** `×k holds k marks; render_span/html_span picked the
/// next mark by scanning ALL marks for EVERY segment — O(k²) per export, on
/// BOTH the markdown and HTML paths.
#[test]
fn mark_sweeps_are_not_quadratic() {
    let payload = mica_markdown::import_markdown(&"**a** ".repeat(20_000), "root");
    let t = std::time::Instant::now();
    let md = mica_markdown::export_markdown(&payload).expect("markdown export should succeed");
    let md_ms = t.elapsed().as_millis();
    eprintln!("export_markdown on 20000 bold marks: {md_ms}ms");
    assert!(!md.is_empty(), "sanity: it should still export");
    assert!(md_ms < 3000, "export_markdown on 20000 marks took {md_ms}ms — render_span scans all marks per segment again");
    let t = std::time::Instant::now();
    let html = mica_markdown::export_html(&payload).expect("html export should succeed");
    let html_ms = t.elapsed().as_millis();
    eprintln!("export_html on 20000 bold marks: {html_ms}ms");
    assert!(!html.is_empty(), "sanity: it should still export");
    assert!(html_ms < 3000, "export_html on 20000 marks took {html_ms}ms — html_span scans all marks per segment again");
}
