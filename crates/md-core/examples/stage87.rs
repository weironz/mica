//! Stage probe for the full-boundary 10k paths: where the 2.3 ms outside
//! render_html_body goes (wire JSON encode, typed parse/serialize), measured
//! on the same 172-block corpus as clipboard_stage87.
//! Run: cargo run -p mica-md --release --features render --example stage87
use std::hint::black_box;
use std::time::{Duration, Instant};

use mica_md::{ParseMode, SerializeOptions, parse_body, serialize_body, wire};

fn fixture10k() -> String {
  let dir =
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../markdown/tests/fixtures/conformance");
  let mut files: Vec<_> = std::fs::read_dir(&dir)
    .expect("fixtures")
    .filter_map(|e| e.ok())
    .map(|e| e.path())
    .filter(|p| p.extension().is_some_and(|x| x == "md"))
    .collect();
  files.sort();
  let mut unit = String::new();
  for path in files {
    unit.push_str(&std::fs::read_to_string(&path).unwrap());
    if !unit.ends_with('\n') {
      unit.push('\n');
    }
    unit.push('\n');
  }
  let mut out = String::with_capacity(10_000 + unit.len());
  while out.len() < 10_000 {
    out.push_str(&unit);
  }
  if let Some(cut) = out[..10_000].rfind('\n') {
    out.truncate(cut + 1);
  }
  out
}

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

fn main() {
  let md = fixture10k();
  println!("input_bytes={}", md.len());
  println!(
    "parse_body {:.6} ms",
    median(|| parse_body(black_box(&md), ParseMode::Document).len())
  );
  let blocks = parse_body(&md, ParseMode::Document);
  println!("blocks={}", blocks.len());
  println!(
    "serialize_body {:.6} ms",
    median(|| {
      serialize_body(black_box(&blocks), &SerializeOptions::default()).len()
    })
  );
  println!(
    "wire_parse_body_json {:.6} ms",
    median(|| wire::parse_body_json(black_box(&md), false).unwrap().len())
  );
  let blocks_json = wire::parse_body_json(&md, false).unwrap();
  println!(
    "blocks_json_bytes={}",
    blocks_json.len()
  );
  println!(
    "wire_serialize_body_json {:.6} ms",
    median(|| wire::serialize_body_json(black_box(&blocks_json), "{}").unwrap().len())
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
