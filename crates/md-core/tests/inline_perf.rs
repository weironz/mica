//! Issue #61 cases that used to live only in the Dart mirror's
//! `inline_scan_perf_test` and were deleted with it (S4 T4.9), now pinned on
//! the engine's public API. Same sizes and 3 s bound as
//! `crates/markdown/tests/inline_scan_perf.rs`: seconds against a ~ms actual.

use std::time::Instant;

use mica_md::{Mark, math_run_spans, serialize_inline};

/// `math_run_spans` runs over whole pasted documents (HTML paste, MCP LaTeX
/// guard); an unclosed `\(` used to cost a full tail scan at every opener.
#[test]
fn math_run_spans_over_unclosed_lparen_is_linear() {
  let src = "\\(".repeat(20_000);
  let started = Instant::now();
  let spans = math_run_spans(&src);
  let ms = started.elapsed().as_millis();
  assert!(spans.is_empty(), "sanity: nothing ever closes");
  assert!(
    ms < 3000,
    "math_run_spans over 20000 unclosed `\\(` took {ms}ms"
  );
}

/// Deeply overlapping marks exercise the outermost-first recursion of the
/// inline serializer: each level picks from the surviving marks.
#[test]
fn inline_export_with_nested_marks_is_not_quadratic() {
  const DEPTH: usize = 300;
  let text = "x".repeat(2 * DEPTH);
  let marks: Vec<Mark> = (0..DEPTH)
    .map(|k| Mark {
      start: k,
      end: 2 * DEPTH - k,
      kind: "bold".to_string(),
      href: None,
      title: None,
    })
    .collect();
  let started = Instant::now();
  let rendered = serialize_inline(&text, &marks);
  let ms = started.elapsed().as_millis();
  assert!(!rendered.is_empty(), "sanity: it renders");
  assert!(ms < 3000, "serializing {DEPTH} nested marks took {ms}ms");
}
