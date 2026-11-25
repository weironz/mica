//! L3 Native UI Shell (`clients/mica_rust`) End-to-End Input-to-Frame Latency Benchmark.
//!
//! Runs the real `MicaAppInMemory` widget tree, NSH reactive signals, `virtual_list`,
//! and `EditorSession` pipeline in a 1280x820 `Headless` renderer (zero mocks) so
//! desktop L3 frame budgets can be benchmarked deterministically even on locked or
//! headless Windows/Linux/macOS CI machines.
//!
//! Usage:
//!   cargo run --manifest-path clients/mica_rust/Cargo.toml --release --example l3_frame_bench

use std::time::{Duration, Instant};

use nsh_core::geometry::{px, size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

fn summarize_ms(mut samples: Vec<Duration>) -> (f64, f64, f64, f64) {
  samples.sort_unstable();
  let pct = |p: f64| -> f64 {
    let idx = (((samples.len() - 1) as f64) * p).round() as usize;
    samples[idx.min(samples.len() - 1)].as_secs_f64() * 1000.0
  };
  (
    pct(0.50),
    pct(0.95),
    pct(0.99),
    samples.last().unwrap().as_secs_f64() * 1000.0,
  )
}

fn main() {
  println!("Mica L3 Native UI Shell Frame Latency Benchmark (NSH 1280x820 Release)");

  // 1. Shell mount + first frame
  let t0 = Instant::now();
  let mut config = EngineConfig::new(Root::of::<mica_rust::MicaAppInMemory>());
  config.viewport = size(px(1280.0), px(820.0));
  config.environment.os = nsh_core::platform::Os::Windows;
  let mut app = Headless::start(config).expect("start headless NSH app");
  app.wait_for("欢迎使用 Mica").expect("first frame ready");
  let first_frame_ms = t0.elapsed().as_secs_f64() * 1000.0;
  println!("  shell_mount_first_frame              {first_frame_ms:.2} ms");

  // 2. New blank page + 40 real keystrokes (input -> EditorSession -> NSH signal -> frame layout)
  app.press("id:new-page-btn").expect("new page");
  app.wait_for("未命名页面 2").expect("new page visible");
  app
    .type_text("id:mica-rich-editor", "Warmup ")
    .expect("warmup");
  app.wait_for("Warmup ").expect("warmup visible");

  let keys = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
  let mut short_type = Vec::with_capacity(40);
  for i in 0..40 {
    let k = keys[i % keys.len()];
    let ts = Instant::now();
    app
      .type_text("id:mica-editor-ime-sink", k)
      .expect("short key");
    short_type.push(ts.elapsed());
  }
  let (p50, p95, p99, max) = summarize_ms(short_type);
  println!(
    "  l3_typing_short_doc (40 keys)        p50={p50:.2} ms  p95={p95:.2} ms  p99={p99:.2} ms  max={max:.2} ms"
  );

  // 3. Build a 100-block document via real Enter splits + measure split frame latency
  let mut split_lats = Vec::with_capacity(100);
  for i in 0..100 {
    let ts = Instant::now();
    app.press_key("enter").expect("enter split");
    split_lats.push(ts.elapsed());
    if i % 10 == 0 {
      app
        .type_text("id:mica-editor-ime-sink", "段落正文性能测试内容")
        .expect("fill block");
    }
  }
  let (sp50, sp95, sp99, smax) = summarize_ms(split_lats);
  println!(
    "  l3_split_block_enter (100 ops)       p50={sp50:.2} ms  p95={sp95:.2} ms  p99={sp99:.2} ms  max={smax:.2} ms"
  );

  // 4. Typing inside the 100+ block virtualized document
  let mut multi_type = Vec::with_capacity(40);
  for i in 0..40 {
    let k = keys[i % keys.len()];
    let ts = Instant::now();
    app
      .type_text("id:mica-editor-ime-sink", k)
      .expect("multi-block key");
    multi_type.push(ts.elapsed());
  }
  let (mp50, mp95, mp99, mmax) = summarize_ms(multi_type);
  println!(
    "  l3_typing_100block_doc (40 keys)     p50={mp50:.2} ms  p95={mp95:.2} ms  p99={mp99:.2} ms  max={mmax:.2} ms"
  );

  // 5. Undo / Redo frame latency (20 pairs)
  let mut undo_lats = Vec::with_capacity(20);
  for _ in 0..20 {
    let ts = Instant::now();
    app.press("id:ed-undo").expect("undo");
    undo_lats.push(ts.elapsed());
  }
  let (up50, up95, up99, umax) = summarize_ms(undo_lats);
  println!(
    "  l3_undo_frame (20 ops)               p50={up50:.2} ms  p95={up95:.2} ms  p99={up99:.2} ms  max={umax:.2} ms"
  );

  // 6. Toggle Outline Sidebar + Search Modal open/query frame latency
  let t_outline = Instant::now();
  app.press("id:toggle-right-panel-btn").expect("open outline");
  app.wait_for("页面大纲").expect("outline visible");
  let outline_ms = t_outline.elapsed().as_secs_f64() * 1000.0;
  app.press("id:right-sidebar-close").expect("close outline");

  let t_search = Instant::now();
  app.press("id:open-search-btn").expect("open search");
  app.type_text("id:search-query-input", "性能测试").expect("search query");
  let search_ms = t_search.elapsed().as_secs_f64() * 1000.0;
  println!("  l3_open_outline_sidebar              {outline_ms:.2} ms");
  println!("  l3_open_and_query_search_modal       {search_ms:.2} ms");
  println!("\nL3 native shell frame benchmark completed.");
}
