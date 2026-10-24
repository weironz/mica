//! Issue #61: the remaining QUADRATIC inline scans.
//!
//! The main inline loop answered "is there a closer ahead?" by rescanning the
//! whole suffix from every candidate position — an unclosed construct on every
//! other character made that O(n²), and the MCP write guard (`math_run_spans`)
//! shares the math paths. Fixed by per-parse forward memos (`ScanMemo`): a
//! `\)` next-index table for `\(`, failure frontiers for `$`-math closers,
//! backtick run lengths and `<` terminators, and a charset-stop memo for the
//! bare-email local-part scan. The renderers' span walks picked each span with
//! a min over ALL marks and rebuilt each overlap set the same way — O(marks²)
//! on a paragraph of sibling marks — now one sort plus a sweep (`SpanWalker`).
//!
//! Sizes and the 3 s bound follow nested_bracket_perf.rs: seconds against a
//! ~ms actual, so a loaded CI box cannot flake. (`tests/unclosed_scan_perf.rs`
//! covers the same family from the import side; this file pins the exact
//! constructs named in the issue plus both renderers' span walks.)

use std::time::Instant;

fn html_ms(src: &str) -> (String, u128) {
  let started = Instant::now();
  let snapshot = mica_markdown::import_markdown(src, "root");
  let html = mica_markdown::export_html(&snapshot).expect("html");
  (html, started.elapsed().as_millis())
}

/// Unclosed `\(` repeated: every opener scanned to the end for `\)`.
#[test]
fn unclosed_lparen_math_scan_is_not_quadratic() {
  let (html, ms) = html_ms(&"\\(a ".repeat(20_000));
  // An unclosed math opener falls back to CommonMark's punctuation escape:
  // `\(` renders as `(`. Check the entire output, not a backslash that the
  // renderer correctly consumes (CommonMark 0.31.2 example 12).
  assert_eq!(html, format!("<p>{}</p>\n", "(a ".repeat(20_000).trim_end()));
  assert!(
    ms < 3000,
    "`\\(a `×20000 took {ms}ms — the `\\)` next-table memo is gone"
  );
}

/// Unclosed `$` repeated: every opener scanned to the end for a closer.
#[test]
fn unclosed_dollar_math_scan_is_not_quadratic() {
  let (_, ms) = html_ms(&"$a ".repeat(20_000));
  assert!(
    ms < 3000,
    "`$a `×20000 took {ms}ms — find_math_closer lost its failure frontier"
  );
}

/// Unclosed backtick runs: every run scanned to the end for a matching run.
#[test]
fn unclosed_backtick_runs_are_not_quadratic() {
  let (_, ms) = html_ms(&"`a ".repeat(20_000));
  assert!(
    ms < 3000,
    "`a `×20000 took {ms}ms — the run-length failure memo is gone"
  );
}

/// `<` with no `>` anywhere: the autolink `>` scan AND `inline_html_end` both
/// scanned to the end from every `<`.
#[test]
fn unclosed_angle_brackets_are_not_quadratic() {
  let (_, ms) = html_ms(&"<a ".repeat(20_000));
  assert!(
    ms < 3000,
    "`<a `×20000 took {ms}ms — the `>` failure frontier is gone"
  );
}

/// `_a` repeated: every boundary re-ran the bare-email local-part scan to the
/// end of the text.
#[test]
fn repeated_email_boundaries_are_not_quadratic() {
  let (_, ms) = html_ms(&"_a".repeat(20_000));
  assert!(
    ms < 3000,
    "`_a`×20000 took {ms}ms — the email charset-stop memo is gone"
  );
}

/// Unclosed comments with stray `>`s between: `find "-->"` rescanned the
/// suffix per opener.
#[test]
fn unclosed_html_comments_are_not_quadratic() {
  let (_, ms) = html_ms(&"<!-- a > ".repeat(10_000));
  assert!(
    ms < 3000,
    "`<!-- a > `×10000 took {ms}ms — the terminator frontier is gone"
  );
}

/// A paragraph of sibling emphasis marks: html_span/render_span picked each
/// span with a min scan over all marks and rebuilt each overlap set — O(marks²).
#[test]
fn sibling_marks_render_without_quadratic_span_walk() {
  let n = 10_000;
  let src = "*a* ".repeat(n);
  let (html, ms) = html_ms(&src);
  assert_eq!(html.matches("<em>").count(), n, "marks survive the sweep");
  assert!(
    ms < 3000,
    "{n} sibling <em> marks: HTML took {ms}ms — html_span rescans all marks per span again"
  );

  let snapshot = mica_markdown::import_markdown(&src, "root");
  let started = Instant::now();
  let md = mica_markdown::export_markdown(&snapshot).expect("md");
  let ms = started.elapsed().as_millis();
  assert_eq!(
    md.matches("*a*").count(),
    n,
    "marks survive the sweep: {md}"
  );
  assert!(
    ms < 3000,
    "{n} sibling marks: Markdown took {ms}ms — render_span rescans all marks per span again"
  );
}
