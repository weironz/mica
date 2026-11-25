//! Mica L1+L2 Kernel, CRDT Editing-Trace, Large Knowledge-Base & Whiteboard Benchmark.
//!
//! Zero extra crate dependencies — uses `std::time::Instant`, `std::hint::black_box`,
//! and deterministic synthetic corpora (plus optional external `josephg/editing-traces`
//! JSON replay via `--trace <path.json>`).
//!
//! Usage:
//!   cargo run -p mica-client --release --example mica_bench
//!   cargo run -p mica-client --release --example mica_bench -- --docs 10000 --trace-ops 50000
//!   cargo run -p mica-client --release --example mica_bench -- --trace automerge-paper.json --json

use std::collections::BTreeMap;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use mica_api_types::View;
use mica_board::{
  BoardSession, extract_board_text_json, layout_mindmap_json, markdown_to_mindmap,
  merge_board_json, render_board_json_svg,
};
use mica_client::search::{build_backlink_graph, search_cached_documents};
use mica_client::storage::{DEFAULT_ORIGIN, VaultStore};
use mica_client::tree::PageTreeModel;
use mica_core::{ActivitySource, Block, MicaDoc};
use mica_editor::{DocSelection, EditorCommand, EditorSession, FindReplaceOp};
use mica_md::{HtmlOptions, ParseMode, parse_body, render_html_body};
use serde_json::{Value, json};
use uuid::Uuid;

struct Config {
  blocks: usize,
  docs: usize,
  trace_ops: usize,
  board_nodes: usize,
  trace_path: Option<PathBuf>,
  json_mode: bool,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      blocks: 2_000,
      docs: 1_000,
      trace_ops: 20_000,
      board_nodes: 500,
      trace_path: None,
      json_mode: false,
    }
  }
}

fn parse_args() -> Config {
  let mut cfg = Config::default();
  let mut args = std::env::args().skip(1);
  while let Some(arg) = args.next() {
    match arg.as_str() {
      "--blocks" => {
        if let Some(v) = args.next().and_then(|s| s.parse().ok()) {
          cfg.blocks = v;
        }
      }
      "--docs" => {
        if let Some(v) = args.next().and_then(|s| s.parse().ok()) {
          cfg.docs = v;
        }
      }
      "--trace-ops" => {
        if let Some(v) = args.next().and_then(|s| s.parse().ok()) {
          cfg.trace_ops = v;
        }
      }
      "--board-nodes" => {
        if let Some(v) = args.next().and_then(|s| s.parse().ok()) {
          cfg.board_nodes = v;
        }
      }
      "--trace" => {
        if let Some(p) = args.next() {
          cfg.trace_path = Some(PathBuf::from(p));
        }
      }
      "--json" => cfg.json_mode = true,
      "--help" | "-h" => {
        println!(
          "Usage: mica_bench [--blocks N] [--docs N] [--trace-ops N] [--board-nodes N] [--trace <editing-trace.json>] [--json]"
        );
        std::process::exit(0);
      }
      other => {
        eprintln!("unknown argument: {other}");
        std::process::exit(2);
      }
    }
  }
  cfg
}

struct Rng(u64);

impl Rng {
  fn new(seed: u64) -> Self {
    Self(seed.max(1))
  }

  fn next_u64(&mut self) -> u64 {
    let mut x = self.0;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    self.0 = x;
    x
  }

  fn range(&mut self, upper: usize) -> usize {
    if upper <= 1 {
      0
    } else {
      (self.next_u64() as usize) % upper
    }
  }
}

struct TempDirGuard(PathBuf);

impl TempDirGuard {
  fn create(prefix: &str) -> Self {
    let path = std::env::temp_dir().join(format!("{prefix}_{}", Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create temp bench dir");
    Self(path)
  }

  fn path(&self) -> &Path {
    &self.0
  }
}

impl Drop for TempDirGuard {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.0);
  }
}

#[derive(Clone, Debug)]
struct Percentiles {
  p50_us: f64,
  p95_us: f64,
  p99_us: f64,
  max_us: f64,
}

fn summarize(mut samples: Vec<Duration>) -> Percentiles {
  assert!(!samples.is_empty());
  samples.sort_unstable();
  let at = |pct: f64| -> f64 {
    let idx = ((samples.len() - 1) as f64 * pct).round() as usize;
    samples[idx.min(samples.len() - 1)].as_secs_f64() * 1_000_000.0
  };
  Percentiles {
    p50_us: at(0.50),
    p95_us: at(0.95),
    p99_us: at(0.99),
    max_us: samples.last().unwrap().as_secs_f64() * 1_000_000.0,
  }
}

fn report_metric(json_mode: bool, suite: &str, name: &str, value: &str, extra: Value) {
  if json_mode {
    println!(
      "{}",
      json!({
        "suite": suite,
        "metric": name,
        "summary": value,
        "data": extra,
      })
    );
  } else {
    println!("  {:<36} {}", name, value);
  }
}

// ── Suite 1: Large Mixed-Content Markdown Document (default 2,000 blocks) ────

fn generate_large_markdown(target_blocks: usize) -> String {
  let mut out = String::with_capacity(target_blocks * 76);
  out.push_str("# Mica 架构性能压测长文档\n\n");
  let mut blocks = 1usize;
  let mut section = 1usize;
  while blocks < target_blocks {
    match blocks % 12 {
      0 => {
        out.push_str(&format!("## 第 {section} 节：纯 Rust 三层架构与协同引擎\n\n"));
        section += 1;
        blocks += 1;
      }
      3 => {
        out.push_str(
          "```rust\nfn reconcile_crdt(doc: &mut MicaDoc, delta: &[u8]) -> usize {\n    doc.apply_update(delta).unwrap();\n    doc.encode_state().len()\n}\n```\n\n",
        );
        blocks += 1;
      }
      6 => {
        out.push_str(
          "| 模块 | 负责范围 | 延迟目标 |\n| --- | --- | --- |\n| `mica-core` | yrs CRDT 文档与 SQLite | `< 1 ms` |\n| `mica-editor` | 块级编辑与选区规则 | `< 0.5 ms` |\n\n",
        );
        blocks += 1;
      }
      8 => {
        out.push_str("> [!NOTE]\n> 这是用于验证 GFM Callout 与长文档混排渲染稳定性的引用块。\n\n");
        blocks += 1;
      }
      10 => {
        out.push_str("- [x] 完成 **Markdown 规范化往返** 与 `UTF-16` 偏移校验\n");
        out.push_str("- [ ] 持续跟踪 [性能基线](https://mica.dev/perf) 与长文档滚动帧率\n\n");
        blocks += 2;
      }
      _ => {
        out.push_str(&format!(
          "段落 #{blocks}：在全栈纯 Rust 知识库中，**单块细粒度更新** 与 `virtual_list` 视口虚拟化共同保证了超大文档的流畅输入体验，同时保持 CommonMark / GFM 往返无损。\n\n"
        ));
        blocks += 1;
      }
    }
  }
  out
}

fn run_suite_large_doc(cfg: &Config) {
  if !cfg.json_mode {
    println!("\n=== [1/4] Large Mixed Document ({} target blocks) ===", cfg.blocks);
  }
  let md = generate_large_markdown(cfg.blocks);
  let bytes = md.len();

  // 1. Parse Markdown
  let t0 = Instant::now();
  let parsed = parse_body(black_box(&md), ParseMode::Document);
  let parse_ms = t0.elapsed().as_secs_f64() * 1000.0;
  let block_count = parsed.len();
  report_metric(
    cfg.json_mode,
    "large_doc",
    "parse_markdown",
    &format!("{parse_ms:.2} ms ({block_count} blocks, {:.1} KiB)", bytes as f64 / 1024.0),
    json!({ "ms": parse_ms, "blocks": block_count, "bytes": bytes }),
  );

  // 2. Build EditorSession (MicaDoc yrs CRDT hydration)
  let t0 = Instant::now();
  let mut session = EditorSession::from_markdown(black_box(&md));
  let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
  report_metric(
    cfg.json_mode,
    "large_doc",
    "session_from_markdown (CRDT)",
    &format!("{load_ms:.2} ms ({} blocks)", session.blocks().len()),
    json!({ "ms": load_ms, "blocks": session.blocks().len() }),
  );

  // Locate a paragraph near the middle of the document for realistic typing.
  let mid_idx = session
    .blocks()
    .iter()
    .enumerate()
    .skip(session.blocks().len() / 2)
    .find(|(_, b)| b.kind == "paragraph" && !b.text.is_empty())
    .map(|(i, _)| i)
    .unwrap_or(0);
  let _ = session.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(mid_idx, 4)),
  });

  // 3. Typing latency (300 consecutive InsertText commands in middle of 2,000-block doc)
  let mut typing_samples = Vec::with_capacity(300);
  let chars = ["a", "b", "测", "试", "x", " ", "文", "档", "1", "2"];
  for i in 0..300 {
    let ch = chars[i % chars.len()];
    let start = Instant::now();
    black_box(session.dispatch(EditorCommand::InsertText {
      text: ch.to_string(),
    }));
    typing_samples.push(start.elapsed());
  }
  let typing_stats = summarize(typing_samples);
  report_metric(
    cfg.json_mode,
    "large_doc",
    "typing_insert_text (300 ops)",
    &format!(
      "p50={:.1} µs  p95={:.1} µs  p99={:.1} µs  max={:.1} µs",
      typing_stats.p50_us, typing_stats.p95_us, typing_stats.p99_us, typing_stats.max_us
    ),
    json!({
      "p50_us": typing_stats.p50_us,
      "p95_us": typing_stats.p95_us,
      "p99_us": typing_stats.p99_us,
      "max_us": typing_stats.max_us,
    }),
  );

  // 4. Enter split + Backspace merge (100 pairs)
  let mut split_samples = Vec::with_capacity(100);
  let mut merge_samples = Vec::with_capacity(100);
  for _ in 0..100 {
    let s0 = Instant::now();
    black_box(session.dispatch(EditorCommand::SplitBlock));
    split_samples.push(s0.elapsed());

    let m0 = Instant::now();
    black_box(session.dispatch(EditorCommand::DeleteBackward));
    merge_samples.push(m0.elapsed());
  }
  let split_stats = summarize(split_samples);
  let merge_stats = summarize(merge_samples);
  report_metric(
    cfg.json_mode,
    "large_doc",
    "split_block (Enter, 100 ops)",
    &format!(
      "p50={:.1} µs  p95={:.1} µs  p99={:.1} µs",
      split_stats.p50_us, split_stats.p95_us, split_stats.p99_us
    ),
    json!({ "p50_us": split_stats.p50_us, "p95_us": split_stats.p95_us, "p99_us": split_stats.p99_us }),
  );
  report_metric(
    cfg.json_mode,
    "large_doc",
    "merge_block (Backspace, 100 ops)",
    &format!(
      "p50={:.1} µs  p95={:.1} µs  p99={:.1} µs",
      merge_stats.p50_us, merge_stats.p95_us, merge_stats.p99_us
    ),
    json!({ "p50_us": merge_stats.p50_us, "p95_us": merge_stats.p95_us, "p99_us": merge_stats.p99_us }),
  );

  // 5. Undo / Redo (50 pairs)
  let mut undo_samples = Vec::with_capacity(50);
  for _ in 0..50 {
    if !session.can_undo() {
      break;
    }
    let u0 = Instant::now();
    black_box(session.dispatch(EditorCommand::Undo));
    undo_samples.push(u0.elapsed());
  }
  let mut redo_samples = Vec::with_capacity(undo_samples.len());
  for _ in 0..undo_samples.len() {
    if !session.can_redo() {
      break;
    }
    let r0 = Instant::now();
    black_box(session.dispatch(EditorCommand::Redo));
    redo_samples.push(r0.elapsed());
  }
  if !undo_samples.is_empty() && !redo_samples.is_empty() {
    let u = summarize(undo_samples);
    let r = summarize(redo_samples);
    report_metric(
      cfg.json_mode,
      "large_doc",
      "undo / redo (50 pairs)",
      &format!(
        "undo p50={:.1} µs p99={:.1} µs | redo p50={:.1} µs p99={:.1} µs",
        u.p50_us, u.p99_us, r.p50_us, r.p99_us
      ),
      json!({ "undo_p50_us": u.p50_us, "undo_p99_us": u.p99_us, "redo_p50_us": r.p50_us, "redo_p99_us": r.p99_us }),
    );
  }

  // 6. In-page FindAll
  let t0 = Instant::now();
  let find_patch = session.dispatch(EditorCommand::FindReplace {
    op: FindReplaceOp::FindAll {
      query: "Rust".to_string(),
      case_sensitive: false,
    },
  });
  let find_ms = t0.elapsed().as_secs_f64() * 1000.0;
  let hits = find_patch.find_matches.map_or(0, |m| m.len());
  report_metric(
    cfg.json_mode,
    "large_doc",
    "in_page_find_all ('Rust')",
    &format!("{find_ms:.2} ms ({hits} matches)"),
    json!({ "ms": find_ms, "matches": hits }),
  );

  // 7. Export Markdown & HTML
  let empty_assets = BTreeMap::new();
  let t0 = Instant::now();
  let exported_md = black_box(session.export_markdown(&empty_assets));
  let exp_md_ms = t0.elapsed().as_secs_f64() * 1000.0;
  report_metric(
    cfg.json_mode,
    "large_doc",
    "export_markdown",
    &format!("{exp_md_ms:.2} ms ({:.1} KiB)", exported_md.len() as f64 / 1024.0),
    json!({ "ms": exp_md_ms, "bytes": exported_md.len() }),
  );

  let t0 = Instant::now();
  let html = black_box(render_html_body(&parsed, &HtmlOptions::default()));
  let exp_html_ms = t0.elapsed().as_secs_f64() * 1000.0;
  report_metric(
    cfg.json_mode,
    "large_doc",
    "export_html",
    &format!("{exp_html_ms:.2} ms ({:.1} KiB)", html.len() as f64 / 1024.0),
    json!({ "ms": exp_html_ms, "bytes": html.len() }),
  );
}

// ── Suite 2: CRDT Real-Style Editing Trace & Two-Peer Merge ──────────────────

struct TracePatch {
  pos_u16: u32,
  del_u16: u32,
  insert: String,
}

fn load_external_editing_trace(path: &Path) -> Result<Vec<TracePatch>, String> {
  let raw = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
  let root: Value = serde_json::from_str(&raw).map_err(|e| format!("parse JSON: {e}"))?;
  let txns = root
    .get("txns")
    .and_then(Value::as_array)
    .ok_or_else(|| "missing `txns` array (expected josephg/editing-traces format)".to_string())?;
  let mut out = Vec::with_capacity(txns.len());
  for txn in txns {
    if let Some(patches) = txn.get("patches").and_then(Value::as_array) {
      for p in patches {
        let Some(arr) = p.as_array() else { continue };
        if arr.len() < 3 {
          continue;
        }
        let pos = arr[0].as_u64().unwrap_or(0) as u32;
        let del = arr[1].as_u64().unwrap_or(0) as u32;
        let ins = arr[2].as_str().unwrap_or("").to_string();
        out.push(TracePatch {
          pos_u16: pos,
          del_u16: del,
          insert: ins,
        });
      }
    }
  }
  Ok(out)
}

/// Deterministic human-like editing trace (typing bursts, mid-sentence edits,
/// backspace corrections, and bilingual CJK/Latin prose) used when `--trace` is omitted.
fn generate_synthetic_editing_trace(total_ops: usize) -> Vec<TracePatch> {
  let mut rng = Rng::new(0x5EED_C0DE_2026);
  let tokens = [
    "a", "e", "t", " ", "s", "r", "i", "n", "知", "识", "库", "笔", "记", "协", "同", "编", "辑",
    "o", "l", ".", "\n",
  ];
  let mut out = Vec::with_capacity(total_ops);
  let mut doc_len: u32 = 0;
  let mut cursor: u32 = 0;

  for step in 0..total_ops {
    // Periodically jump cursor back into earlier text like real human revision.
    if step > 0 && step % 97 == 0 && doc_len > 20 {
      cursor = rng.range(doc_len as usize) as u32;
    }
    let roll = rng.range(100);
    if roll < 18 && cursor > 0 {
      // Backspace 1..=3 chars
      let del = (1 + rng.range(3) as u32).min(cursor);
      cursor -= del;
      doc_len -= del;
      out.push(TracePatch {
        pos_u16: cursor,
        del_u16: del,
        insert: String::new(),
      });
    } else {
      let tok = tokens[rng.range(tokens.len())];
      let ins_len = tok.encode_utf16().count() as u32;
      out.push(TracePatch {
        pos_u16: cursor,
        del_u16: 0,
        insert: tok.to_string(),
      });
      cursor += ins_len;
      doc_len += ins_len;
    }
  }
  out
}

fn run_suite_crdt_trace(cfg: &Config) {
  let (source_label, patches) = match &cfg.trace_path {
    Some(p) => {
      let loaded = load_external_editing_trace(p).expect("load external editing-traces JSON");
      (format!("external {}", p.display()), loaded)
    }
    None => (
      format!("synthetic human trace ({} ops)", cfg.trace_ops),
      generate_synthetic_editing_trace(cfg.trace_ops),
    ),
  };

  if !cfg.json_mode {
    println!("\n=== [2/4] CRDT Editing-Trace Replay ({source_label}) ===");
  }

  // Replay directly through `MicaDoc` (yrs CRDT text block) to measure pure CRDT
  // history growth, early-vs-late step latency, state bloat, and cold decode.
  let block_id = "trace-b0";
  let mut doc = MicaDoc::from_blocks_with_client_id(
    "root",
    &[Block {
      id: block_id.into(),
      kind: "paragraph".into(),
      text: String::new(),
      data: json!({}),
      children: vec![],
    }],
    Some(1),
  );

  let mut samples = Vec::with_capacity(patches.len());
  let replay_start = Instant::now();
  let mut current_u16_len: u32 = 0;

  for p in &patches {
    let pos = p.pos_u16.min(current_u16_len);
    let del = p.del_u16.min(current_u16_len.saturating_sub(pos));
    let ins_u16 = p.insert.encode_utf16().count() as u32;
    let t0 = Instant::now();
    if del > 0 {
      doc.text_delete(block_id, pos, del);
    }
    if !p.insert.is_empty() {
      doc.text_insert(block_id, pos, &p.insert);
    }
    samples.push(t0.elapsed());
    current_u16_len = current_u16_len - del + ins_u16;
  }

  let total_ms = replay_start.elapsed().as_secs_f64() * 1000.0;
  let ops_per_sec = patches.len() as f64 / (total_ms / 1000.0).max(1e-6);

  let window = 2_000.min(samples.len() / 2).max(1);
  let early = summarize(samples[..window].to_vec());
  let late = summarize(samples[samples.len() - window..].to_vec());
  let overall = summarize(samples);

  let final_text = doc
    .to_blocks()
    .into_iter()
    .next()
    .map(|b| b.text)
    .unwrap_or_default();
  let t_enc = Instant::now();
  let encoded = black_box(doc.encode_state());
  let encode_ms = t_enc.elapsed().as_secs_f64() * 1000.0;
  let ratio = encoded.len() as f64 / final_text.len().max(1) as f64;

  let t_dec = Instant::now();
  let decoded = black_box(MicaDoc::from_update(&encoded).expect("decode yrs state"));
  let decode_ms = t_dec.elapsed().as_secs_f64() * 1000.0;
  let t_sess = Instant::now();
  let _hydrated = black_box(EditorSession::from_doc(decoded));
  let hydrate_ms = t_sess.elapsed().as_secs_f64() * 1000.0;

  report_metric(
    cfg.json_mode,
    "crdt_trace",
    "replay_throughput",
    &format!("{total_ms:.2} ms ({:.0} ops/s, {} ops)", ops_per_sec, patches.len()),
    json!({ "total_ms": total_ms, "ops_per_sec": ops_per_sec, "ops": patches.len() }),
  );
  report_metric(
    cfg.json_mode,
    "crdt_trace",
    "step_latency (overall)",
    &format!(
      "p50={:.2} µs  p95={:.2} µs  p99={:.2} µs",
      overall.p50_us, overall.p95_us, overall.p99_us
    ),
    json!({ "p50_us": overall.p50_us, "p95_us": overall.p95_us, "p99_us": overall.p99_us }),
  );
  report_metric(
    cfg.json_mode,
    "crdt_trace",
    &format!("early vs late ({window}-op window)"),
    &format!(
      "early p50={:.2} µs -> late p50={:.2} µs (drift {:.2}x)",
      early.p50_us,
      late.p50_us,
      late.p50_us / early.p50_us.max(0.01)
    ),
    json!({ "early_p50_us": early.p50_us, "late_p50_us": late.p50_us }),
  );
  report_metric(
    cfg.json_mode,
    "crdt_trace",
    "state_size & cold_decode",
    &format!(
      "text={} B  yrs={} B ({ratio:.2}x) | encode={encode_ms:.2} ms  decode={decode_ms:.2} ms  hydrate={hydrate_ms:.2} ms",
      final_text.len(),
      encoded.len()
    ),
    json!({
      "text_bytes": final_text.len(),
      "yrs_bytes": encoded.len(),
      "overhead_ratio": ratio,
      "encode_ms": encode_ms,
      "decode_ms": decode_ms,
      "hydrate_ms": hydrate_ms,
    }),
  );

  // Two-peer concurrent remote merge (500 alternating updates via EditorSession)
  let mut peer_a = EditorSession::from_markdown_with_client_id("# 协同压测\n\n初始正文内容。", Some(10));
  let mut peer_b = EditorSession::from_doc(
    MicaDoc::from_update_with_client_id(&peer_a.encode_state(), Some(20)).expect("clone peer B"),
  );
  let _ = peer_a.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(1, 2)),
  });
  let _ = peer_b.dispatch(EditorCommand::SetSelection {
    selection: Some(DocSelection::caret(1, 5)),
  });

  let mut merge_samples = Vec::with_capacity(500);
  for i in 0..500 {
    let sv_a = peer_a.state_vector();
    let _ = peer_b.dispatch(EditorCommand::InsertText {
      text: format!("B{i}"),
    });
    let delta = peer_b
      .doc()
      .encode_diff(&sv_a)
      .expect("encode diff from peer B");
    let t0 = Instant::now();
    black_box(peer_a.dispatch(EditorCommand::ApplyRemoteUpdate { update: delta }));
    merge_samples.push(t0.elapsed());
  }
  let m = summarize(merge_samples);
  report_metric(
    cfg.json_mode,
    "crdt_trace",
    "remote_crdt_merge (500 updates)",
    &format!(
      "p50={:.1} µs  p95={:.1} µs  p99={:.1} µs",
      m.p50_us, m.p95_us, m.p99_us
    ),
    json!({ "p50_us": m.p50_us, "p95_us": m.p95_us, "p99_us": m.p99_us }),
  );
}

// ── Suite 3: Large Synthetic Knowledge Base (SQLite Vault + Tree + Search + Graph) ─

fn run_suite_knowledge_base(cfg: &Config) {
  if !cfg.json_mode {
    println!("\n=== [3/4] Synthetic Knowledge Base ({} pages + nested folders) ===", cfg.docs);
  }
  let tmp = TempDirGuard::create("mica_kb_bench");
  let db_path = tmp.path().join("store.db");
  let db_path_str = db_path.to_str().expect("utf8 temp path");
  let ws_id = Uuid::new_v4();
  let now = Utc::now();
  let mut rng = Rng::new(0x00A1_CE55_1000);

  let folder_count = (cfg.docs / 10).max(5);
  let mut folder_ids = Vec::with_capacity(folder_count);
  let mut views = Vec::with_capacity(folder_count + cfg.docs);
  for i in 0..folder_count {
    let fid = Uuid::new_v4();
    let parent = if i >= 5 {
      Some(folder_ids[rng.range(i.min(20))])
    } else {
      None
    };
    folder_ids.push(fid);
    views.push(View {
      id: fid,
      workspace_id: ws_id,
      parent_view_id: parent,
      object_id: fid,
      object_type: "folder".to_string(),
      name: format!("目录分组-{i:04}"),
      icon: None,
      position: format!("{:010}", (i + 1) * 10),
      is_deleted: false,
      created_by: Uuid::nil(),
      created_at: now,
      updated_at: now,
      state_bytes: None,
    });
  }

  let mut page_ids = Vec::with_capacity(cfg.docs);
  for _ in 0..cfg.docs {
    page_ids.push(Uuid::new_v4());
  }

  let topics = [
    "分布式系统",
    "Rust 内存模型",
    "PostgreSQL 索引调优",
    "CRDT 协同原理",
    "前端虚拟滚动",
    "零拷贝序列化",
    "网络故障排查",
    "知识图谱构建",
  ];

  // 1. Bulk ingest into real on-disk SQLite VaultStore
  let t_ingest = Instant::now();
  let mut search_corpus: Vec<(Uuid, String, Vec<Block>)> = Vec::with_capacity(cfg.docs);
  let mut graph_corpus: Vec<(Uuid, Vec<Block>)> = Vec::with_capacity(cfg.docs);
  {
    let vault = VaultStore::open_at(db_path_str).expect("open sqlite bench vault");
    for f in &views {
      vault.save_view(DEFAULT_ORIGIN, f).expect("save folder view");
    }
    for (i, &doc_id) in page_ids.iter().enumerate() {
      let parent = Some(folder_ids[rng.range(folder_ids.len())]);
      let topic = topics[i % topics.len()];
      let title = if i == cfg.docs / 2 {
        format!("稀有检索靶点-QuantumNeedle-{i:05}")
      } else {
        format!("{topic}-设计笔记-{i:05}")
      };
      let link_target = page_ids[(i + 7) % page_ids.len()];
      let link_target2 = page_ids[(i + 31) % page_ids.len()];
      let md = format!(
        "---\ntags: [rust, kb]\n---\n# {title}\n\n关于 {topic} 的核心要点记录，编号 {i}。参考页面：[关联文档](mica://page/{link_target}) 与 [延伸阅读](mica://page/{link_target2})。\n\n```rust\nlet score = {i} * 2;\n```\n"
      );
      let page_view = View {
        id: doc_id,
        workspace_id: ws_id,
        parent_view_id: parent,
        object_id: doc_id,
        object_type: "page".to_string(),
        name: title.clone(),
        icon: None,
        position: format!("{:010}", (folder_count + i + 1) * 10),
        is_deleted: false,
        created_by: Uuid::nil(),
        created_at: now,
        updated_at: now,
        state_bytes: None,
      };
      vault.save_view(DEFAULT_ORIGIN, &page_view).expect("save page view");
      vault
        .save_markdown(DEFAULT_ORIGIN, doc_id, &md)
        .expect("save markdown");
      vault.record_doc_created(DEFAULT_ORIGIN, ws_id, doc_id, ActivitySource::Manual, &md);

      // Build lightweight parsed block list for search & backlink graph suites
      let blocks = vec![
        Block {
          id: format!("h-{i}"),
          kind: "heading".into(),
          text: title.clone(),
          data: json!({ "level": 1 }),
          children: vec![],
        },
        Block {
          id: format!("p-{i}"),
          kind: "paragraph".into(),
          text: format!("关于 {topic} 的核心要点记录，编号 {i}。"),
          data: json!({
            "marks": [
              { "kind": "link", "start": 0, "end": 4, "href": format!("mica://page/{link_target}") },
              { "kind": "link", "start": 5, "end": 9, "href": format!("mica://page/{link_target2}") },
            ]
          }),
          children: vec![],
        },
      ];
      search_corpus.push((doc_id, title, blocks.clone()));
      graph_corpus.push((doc_id, blocks));
    }
  }
  let ingest_ms = t_ingest.elapsed().as_secs_f64() * 1000.0;
  let db_bytes = fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
  report_metric(
    cfg.json_mode,
    "knowledge_base",
    "sqlite_vault_bulk_ingest",
    &format!(
      "{ingest_ms:.1} ms ({:.0} docs/s, store.db {:.2} MiB)",
      cfg.docs as f64 / (ingest_ms / 1000.0).max(1e-6),
      db_bytes as f64 / (1024.0 * 1024.0)
    ),
    json!({ "ms": ingest_ms, "docs": cfg.docs, "db_bytes": db_bytes }),
  );

  // 2. Cold open + PageTreeModel build + visible/all_live_rows
  let t_cold = Instant::now();
  let vault = VaultStore::open_at(db_path_str).expect("reopen sqlite bench vault");
  let loaded_views = vault.list_workspace_views(DEFAULT_ORIGIN, ws_id);
  let tree = PageTreeModel::new(loaded_views);
  let visible = black_box(tree.visible_rows());
  let all_live = black_box(tree.all_live_rows());
  let cold_ms = t_cold.elapsed().as_secs_f64() * 1000.0;
  report_metric(
    cfg.json_mode,
    "knowledge_base",
    "cold_open_vault_and_tree",
    &format!(
      "{cold_ms:.2} ms (top_collapsed={} rows, all_live={} rows)",
      visible.len(),
      all_live.len()
    ),
    json!({ "ms": cold_ms, "visible_rows": visible.len(), "all_live_rows": all_live.len() }),
  );

  // 3. Random single-doc load from SQLite (200 reads)
  let mut read_samples = Vec::with_capacity(200);
  for _ in 0..200 {
    let target = page_ids[rng.range(page_ids.len())];
    let t0 = Instant::now();
    black_box(vault.load_markdown(DEFAULT_ORIGIN, target));
    read_samples.push(t0.elapsed());
  }
  let r_stats = summarize(read_samples);
  report_metric(
    cfg.json_mode,
    "knowledge_base",
    "sqlite_random_doc_load (200 reads)",
    &format!(
      "p50={:.1} µs  p95={:.1} µs  p99={:.1} µs",
      r_stats.p50_us, r_stats.p95_us, r_stats.p99_us
    ),
    json!({ "p50_us": r_stats.p50_us, "p95_us": r_stats.p95_us, "p99_us": r_stats.p99_us }),
  );

  // 4. Full-vault search (common hit, rare needle, worst-case miss)
  for (label, query) in [
    ("search_common_hit", "PostgreSQL"),
    ("search_rare_needle", "QuantumNeedle"),
    ("search_worst_case_miss", "NonExistentQueryTermXYZ999"),
  ] {
    let t0 = Instant::now();
    let hits = black_box(search_cached_documents(&search_corpus, query, 20));
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    report_metric(
      cfg.json_mode,
      "knowledge_base",
      label,
      &format!("{ms:.2} ms ({} hits across {} docs)", hits.len(), cfg.docs),
      json!({ "ms": ms, "hits": hits.len(), "docs": cfg.docs }),
    );
  }

  // 5. Backlink graph construction across all pages
  let t_graph = Instant::now();
  let edges = black_box(build_backlink_graph(&graph_corpus));
  let graph_ms = t_graph.elapsed().as_secs_f64() * 1000.0;
  report_metric(
    cfg.json_mode,
    "knowledge_base",
    "build_backlink_graph",
    &format!("{graph_ms:.2} ms ({} edges)", edges.len()),
    json!({ "ms": graph_ms, "edges": edges.len() }),
  );

  // 6. Usage stats SQL aggregation (overview + 30d activity)
  let t_stats = Instant::now();
  let ov = black_box(vault.usage_overview(DEFAULT_ORIGIN).expect("usage_overview"));
  let act = black_box(
    vault
      .usage_activity(DEFAULT_ORIGIN, "30d", 480, None, false)
      .expect("usage_activity"),
  );
  let stats_ms = t_stats.elapsed().as_secs_f64() * 1000.0;
  report_metric(
    cfg.json_mode,
    "knowledge_base",
    "usage_stats_sql_aggregation",
    &format!(
      "{stats_ms:.2} ms (docs={}, daily_points={})",
      ov.summary.live_page_count,
      act.daily.len()
    ),
    json!({ "ms": stats_ms, "total_docs": ov.summary.live_page_count, "daily_points": act.daily.len() }),
  );
}

// ── Suite 4: Large Infinite Whiteboard / Mindmap (default 500 nodes) ─────────

fn run_suite_whiteboard(cfg: &Config) {
  if !cfg.json_mode {
    println!("\n=== [4/4] Infinite Whiteboard / Mindmap ({} nodes) ===", cfg.board_nodes);
  }
  let mut md = String::from("# Mica 压测画布根节点\n");
  let branches = 10usize;
  let per_branch = (cfg.board_nodes.saturating_sub(1) / branches).max(1);
  for b in 0..branches {
    md.push_str(&format!("- 主题分支 {b}\n"));
    for leaf in 0..per_branch {
      md.push_str(&format!("  - 子节点 {b}-{leaf} 架构说明\n"));
    }
  }
  let base_json = markdown_to_mindmap(&md, None).to_compact_json();

  let t0 = Instant::now();
  let session = black_box(BoardSession::from_json(&base_json));
  let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
  let node_count = session.snapshot().nodes.len();

  let t1 = Instant::now();
  let laid_out = black_box(layout_mindmap_json(&base_json).expect("layout_mindmap_json"));
  let layout_ms = t1.elapsed().as_secs_f64() * 1000.0;

  let t2 = Instant::now();
  let svg = black_box(render_board_json_svg(&laid_out, None).expect("render_board_json_svg"));
  let svg_ms = t2.elapsed().as_secs_f64() * 1000.0;

  let t3 = Instant::now();
  let merged = black_box(merge_board_json(&base_json, &laid_out, &base_json));
  let merge_ms = t3.elapsed().as_secs_f64() * 1000.0;

  let t4 = Instant::now();
  let extracted = black_box(extract_board_text_json(&merged));
  let extract_ms = t4.elapsed().as_secs_f64() * 1000.0;

  report_metric(
    cfg.json_mode,
    "whiteboard",
    "board_load_and_auto_layout",
    &format!("load={load_ms:.2} ms  layout={layout_ms:.2} ms ({node_count} nodes)"),
    json!({ "load_ms": load_ms, "layout_ms": layout_ms, "nodes": node_count }),
  );
  report_metric(
    cfg.json_mode,
    "whiteboard",
    "board_svg_merge_and_search_text",
    &format!(
      "svg={svg_ms:.2} ms ({:.1} KiB)  merge={merge_ms:.2} ms  text={extract_ms:.2} ms ({} B)",
      svg.len() as f64 / 1024.0,
      extracted.len()
    ),
    json!({ "svg_ms": svg_ms, "svg_bytes": svg.len(), "merge_ms": merge_ms, "extract_ms": extract_ms }),
  );
}

fn main() {
  let cfg = parse_args();
  if !cfg.json_mode {
    println!("Mica Pure-Rust Performance Benchmark Suite (L1 + L2)");
  }
  run_suite_large_doc(&cfg);
  run_suite_crdt_trace(&cfg);
  run_suite_knowledge_base(&cfg);
  run_suite_whiteboard(&cfg);
  if !cfg.json_mode {
    println!("\nBenchmark suite completed.");
  }
}
