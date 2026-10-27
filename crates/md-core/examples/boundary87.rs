//! Narrow native release probe matching the FFI crate's default release profile.
//! Run: cargo run -p mica-md --release --features render --example boundary87
//! No timing is taken from a debug build. No cache or diagnostic API is used.
use mica_md::{BlockIds, ParseMode, ParseOptions, parse, parse_body, wire};
use std::hint::black_box;
use std::time::{Duration, Instant};

const INPUT: &str =
  "Hot paths: **bold** and `code` plus a [link](https://example.com) in one paragraph now!";

fn measure<T>(label: &str, mut run: impl FnMut() -> T) {
  let batch = |n: usize, run: &mut dyn FnMut() -> T| {
    let started = Instant::now();
    for _ in 0..n {
      black_box(run());
    }
    started.elapsed()
  };
  let mut n = 32;
  while batch(n, &mut run) < Duration::from_millis(50) {
    n *= 2;
  }
  for _ in 0..2 {
    batch(n, &mut run);
  }
  let mut samples: Vec<f64> = (0..7)
    .map(|_| batch(n, &mut run).as_secs_f64() * 1000.0 / n as f64)
    .collect();
  samples.sort_by(f64::total_cmp);
  println!(
    "{{\"item\":\"{label}\",\"batch\":{n},\"median_ms\":{:.9}}}",
    samples[3]
  );
}

fn main() {
  assert_eq!(INPUT.len(), 87);
  if cfg!(debug_assertions) || !cfg!(feature = "render") {
    panic!("run this probe with --release --features render");
  }
  let opts = ParseOptions {
    mode: ParseMode::Document,
    root_id: "root".into(),
    ids: BlockIds::Sequential { prefix: "b".into() },
  };
  measure("parse-only", || parse(black_box(INPUT), black_box(&opts)));
  measure("parse_body", || {
    parse_body(black_box(INPUT), ParseMode::Document)
  });
  measure("parse_body_json", || {
    wire::parse_body_json(black_box(INPUT), false).unwrap()
  });
}
