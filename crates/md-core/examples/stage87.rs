//! Stage probe for the full-boundary 10k paths: where the 2.3 ms outside
//! render_html_body goes (wire JSON encode, typed parse/serialize), measured
//! on the same 172-block corpus as clipboard_stage87.
//! Run: cargo run -p mica-md --release --features render --example stage87
use std::hint::black_box;
use std::time::{Duration, Instant};

use mica_md::{ParseMode, SerializeOptions, parse_body, serialize_body, wire};

fn median<T>(mut run: impl FnMut() -> T) -> f64 {
  let mut batch = |n: usize| {
    let start = Instant::now();
    for _ in 0..n {
      black_box(run());
    }
    start.elapsed()
  };
  let mut n = 8;
  while batch(n) < Duration::from_millis(40) {
    n *= 2;
  }
  for _ in 0..2 {
    batch(n);
  }
  let mut samples: Vec<f64> = (0..7)
    .map(|_| batch(n).as_secs_f64() * 1000.0 / n as f64)
    .collect();
  samples.sort_by(f64::total_cmp);
  samples[3]
}

fn bench_sample() -> String {
  // The Dart benchmark's exact input: markdownFixtureSample repeated and
  // truncated at exactly 10000 UTF-16 code units. The sample text is extracted
  // verbatim from the Dart source file at a pinned path, so the corpus matches
  // D's bytes rather than approximating them from the fixture directory.
  let dart_source = std::fs::read_to_string(
    "D:/codes/mica/.claude/worktrees/agent-a64add55c7b76fbd3/clients/mica_flutter/test/support/markdown_bench_inputs.dart",
  )
  .expect("read Dart bench inputs");
  let start = dart_source
    .find("markdownFixtureSample = r'''")
    .map(|i| i + "markdownFixtureSample = r'''".len())
    .expect("sample start marker");
  let end = dart_source[start..]
    .find("'''")
    .map(|i| start + i)
    .expect("sample end marker");
  let sample = &dart_source[start..end];
  let mut out = String::with_capacity(10_000 + sample.len());
  while out.encode_utf16().count() < 10_000 {
    out.push_str(sample);
  }
  // Dart substring truncates by UTF-16 code units.
  let mut units = 0usize;
  for (idx, ch) in out.char_indices() {
    if units + ch.len_utf16() > 10_000 {
      out.truncate(idx);
      break;
    }
    units += ch.len_utf16();
  }
  out
}

fn main() {
  let md = bench_sample();
  println!("input_bytes={}", md.len());
  println!("input_chars={}", md.chars().count());
  println!(
    "parse_body {:.6} ms",
    median(|| parse_body(black_box(&md), ParseMode::Document).len())
  );
  let blocks = parse_body(&md, ParseMode::Document);
  println!("blocks={}", blocks.len());
  println!(
    "serialize_body {:.6} ms",
    median(|| { serialize_body(black_box(&blocks), &SerializeOptions::default()).len() })
  );
  println!(
    "wire_parse_body_json {:.6} ms",
    median(|| wire::parse_body_json(black_box(&md), false).unwrap().len())
  );
  let blocks_json = wire::parse_body_json(&md, false).unwrap();
  println!("blocks_json_bytes={}", blocks_json.len());
  println!(
    "wire_serialize_body_json {:.6} ms",
    median(|| wire::serialize_body_json(black_box(&blocks_json), "{}")
      .unwrap()
      .len())
  );
  println!(
    "wire_render_html_body_json_clipboard {:.6} ms",
    median(|| {
      wire::render_html_body_json(black_box(&blocks_json), "{}", "clipboard")
        .unwrap()
        .len()
    })
  );
  println!(
    "wire_render_html_body_json_export {:.6} ms",
    median(|| {
      wire::render_html_body_json(black_box(&blocks_json), "{}", "export")
        .unwrap()
        .len()
    })
  );
}
