//! Stage-level probe for the 10k Clipboard HTML gap. Not a benchmark gate:
//! prints per-stage medians so the dominant cost is measured, not guessed.
//! Run: cargo run -p mica-md --release --example clipboard_stage87
use std::hint::black_box;
use std::time::{Duration, Instant};

use mica_md::{Block, HtmlOptions, ParseMode, parse_body, render_html_body};
use serde_json::Value;

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

fn synthetic(blocks: &[Block]) -> mica_md::DocumentSnapshotPayload {
  let mut doc_blocks = Vec::with_capacity(blocks.len() + 1);
  let children: Vec<String> = (0..blocks.len()).map(|i| format!("\u{1}b{i}")).collect();
  doc_blocks.push(Block {
    id: "\u{1}root".to_string(),
    kind: "page".to_string(),
    text: String::new(),
    data: Value::Null,
    children: children.clone(),
  });
  for (i, block) in blocks.iter().enumerate() {
    let mut block = block.clone();
    block.id = children[i].clone();
    block.children.clear();
    doc_blocks.push(block);
  }
  mica_md::DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id: "\u{1}root".to_string(),
    blocks: doc_blocks,
  }
}

fn main() {
  let md = fixture10k();
  println!("input_bytes={}", md.len());
  let blocks = parse_body(&md, ParseMode::Document);
  println!("blocks={}", blocks.len());

  println!(
    "full_render {:.6} ms",
    median(|| render_html_body(black_box(&blocks), &HtmlOptions::default()))
  );
  println!(
    "synthetic_doc {:.6} ms",
    median(|| synthetic(&blocks).blocks.len())
  );
  // render_html_body minus the synthetic-doc stage isolates the clipboard
  // walk itself (mark extraction + span walking + escaping + safety passes).
  let built = render_html_body(&blocks, &HtmlOptions::default());
  println!("output_bytes={}", built.len());
}
