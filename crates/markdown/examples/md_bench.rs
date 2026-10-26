//! Rust-side performance baseline for the Markdown engine (S0 T0.3).
//!
//! Times `import_markdown`, `export_markdown` and `export_html` over the same
//! three input sizes the Dart baseline uses: an 87-character single paragraph,
//! ~10k characters and ~100k characters built from the conformance fixtures.
//! Deliberately uses `std::time::Instant` and no benchmark harness: the numbers
//! are a coarse regression gate for the refactor, not a statistic.
//!
//! Run: `cargo run -p mica-markdown --release --example md_bench`

use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const SMALL: &str = "Some **bold** and _italic_ text, a `code` span, and a [link](https://x.y) here. 1234567";

fn conformance_corpus() -> String {
  let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/conformance");
  let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
    .expect("read conformance fixtures")
    .map(|entry| entry.expect("dir entry").path())
    .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
    .collect();
  files.sort();
  let mut out = String::new();
  for path in files {
    out.push_str(&std::fs::read_to_string(&path).expect("read fixture"));
    if !out.ends_with('\n') {
      out.push('\n');
    }
    out.push('\n');
  }
  out
}

/// Repeat `unit` until it is at least `target` bytes, then cut at a line
/// boundary so the result is still well-formed Markdown.
fn sized(unit: &str, target: usize) -> String {
  let mut out = String::with_capacity(target + unit.len());
  while out.len() < target {
    out.push_str(unit);
  }
  if let Some(cut) = out[..target.min(out.len())].rfind('\n') {
    out.truncate(cut + 1);
  }
  out
}

fn median(mut samples: Vec<Duration>) -> Duration {
  samples.sort();
  samples[samples.len() / 2]
}

fn bench(label: &str, input: &str) {
  // Two discarded warm-up runs, then seven measured runs; median reported.
  for _ in 0..2 {
    let doc = mica_markdown::import_markdown(black_box(input), "root");
    black_box(mica_markdown::export_markdown(&doc).expect("export_markdown"));
    black_box(mica_markdown::export_html(&doc).expect("export_html"));
  }

  let mut import = Vec::new();
  let mut export_md = Vec::new();
  let mut export_html = Vec::new();
  for _ in 0..7 {
    let start = Instant::now();
    let doc = mica_markdown::import_markdown(black_box(input), "root");
    import.push(start.elapsed());

    let start = Instant::now();
    black_box(mica_markdown::export_markdown(&doc).expect("export_markdown"));
    export_md.push(start.elapsed());

    let start = Instant::now();
    black_box(mica_markdown::export_html(&doc).expect("export_html"));
    export_html.push(start.elapsed());
  }

  let bytes = input.len();
  for (op, sample) in [
    ("import", median(import)),
    ("export_markdown", median(export_md)),
    ("export_html", median(export_html)),
  ] {
    println!(
      "{{\"item\":\"{op}.{label}\",\"bytes\":{bytes},\"median_ms\":{:.3}}}",
      sample.as_secs_f64() * 1000.0
    );
  }
}

fn main() {
  let corpus = conformance_corpus();
  bench("87", SMALL);
  bench("10k", &sized(&corpus, 10_000));
  bench("100k", &sized(&corpus, 100_000));
}