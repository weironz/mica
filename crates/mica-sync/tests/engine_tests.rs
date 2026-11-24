use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use mica_core::{Block, MicaDoc};
use mica_sync::{
  COMPACT_CHECK_EVERY, COMPACT_THRESHOLD, IdbHydrateSnapshot, IdbMirrorOp, LAG_RESYNC_DEBOUNCE_MS,
  LogSizes, MAX_AUTO_REHEAL, MAX_PUSH_REJECTS, MAX_REPAIRS, MERGE_MAX_BYTES, MemoryMirrorStorage,
  OutboxEntry, PING_EVERY_MS, PUSH_WINDOW, PresenceUser, StorageError, StoredDoc, SyncConfig,
  SyncCursor, SyncEffect, SyncEngine, SyncEvent, SyncPhase, SyncStorage, TimerKind,
};
use serde_json::{Value, json};

fn make_doc(text: &str) -> MicaDoc {
  MicaDoc::from_blocks(
    "root",
    &[
      Block {
        id: "root".into(),
        kind: "page".into(),
        text: String::new(),
        data: json!({}),
        children: vec!["p1".into()],
      },
      Block {
        id: "p1".into(),
        kind: "paragraph".into(),
        text: text.into(),
        data: json!({}),
        children: vec![],
      },
    ],
  )
}

fn edit_doc(doc: &mut MicaDoc, append: &str) -> Vec<u8> {
  let sv = doc.state_vector();
  let len = doc.to_blocks()[1].text.encode_utf16().count() as u32;
  doc.text_insert("p1", len, append);
  doc.encode_diff(&sv).unwrap()
}

fn ws_frames(effects: &[SyncEffect]) -> Vec<Value> {
  effects
    .iter()
    .filter_map(|e| match e {
      SyncEffect::SendWsJson { payload, .. } => serde_json::from_str(payload).ok(),
      _ => None,
    })
    .collect()
}

fn ws_pushes(effects: &[SyncEffect]) -> Vec<Value> {
  ws_frames(effects)
    .into_iter()
    .filter(|v| v["type"] == "sync.push")
    .collect()
}

fn bootstrap_engine(engine: &mut SyncEngine, doc: &MicaDoc, base_rid: i64) -> Vec<SyncEffect> {
  let e1 = engine.handle_event(SyncEvent::Connect);
  let epoch = e1
    .iter()
    .find_map(|e| match e {
      SyncEffect::OpenSocket { epoch } => Some(*epoch),
      _ => None,
    })
    .expect("Connect emits OpenSocket");
  engine.handle_event(SyncEvent::SocketOpened { epoch });
  let frame = json!({
    "type": "sync.base",
    "base": STANDARD.encode(doc.encode_state()),
    "base_rid": base_rid,
  })
  .to_string();
  engine.handle_event(SyncEvent::SocketFrame { epoch, json: frame })
}

/// Configurable failing storage wrapper to deterministically test retry and
/// persistence self-heal paths.
struct FlakyStorage {
  inner: MemoryMirrorStorage,
  fail_outbox: Arc<AtomicBool>,
  fail_remote: Arc<AtomicBool>,
  fail_load_io: bool,
}

impl FlakyStorage {
  fn new(fail_outbox: Arc<AtomicBool>, fail_remote: Arc<AtomicBool>) -> Self {
    Self {
      inner: MemoryMirrorStorage::new(),
      fail_outbox,
      fail_remote,
      fail_load_io: false,
    }
  }
}

impl SyncStorage for FlakyStorage {
  fn load(&self) -> Result<Option<StoredDoc>, StorageError> {
    if self.fail_load_io {
      return Err(StorageError::Io("disk I/O failure".into()));
    }
    self.inner.load()
  }
  fn save_base(&mut self, state: &[u8], cursor: i64) -> Result<(), StorageError> {
    self.inner.save_base(state, cursor)
  }
  fn append_outbox(&mut self, diff: &[u8]) -> Result<i64, StorageError> {
    if self.fail_outbox.load(Ordering::SeqCst) {
      return Err(StorageError::Io("disk full".into()));
    }
    self.inner.append_outbox(diff)
  }
  fn outbox_after(&self, pushed_clock: i64) -> Result<Vec<OutboxEntry>, StorageError> {
    self.inner.outbox_after(pushed_clock)
  }
  fn cursor(&self) -> Result<SyncCursor, StorageError> {
    self.inner.cursor()
  }
  fn advance_cursor(
    &mut self,
    last_synced_rid: Option<i64>,
    pushed_clock: Option<i64>,
  ) -> Result<(), StorageError> {
    self.inner.advance_cursor(last_synced_rid, pushed_clock)
  }
  fn trim_outbox_through(&mut self, pushed_clock: i64) -> Result<(), StorageError> {
    self.inner.trim_outbox_through(pushed_clock)
  }
  fn append_remote_batch(&mut self, items: &[(i64, Vec<u8>)]) -> Result<(), StorageError> {
    if self.fail_remote.load(Ordering::SeqCst) {
      return Err(StorageError::Io("remote write failed".into()));
    }
    self.inner.append_remote_batch(items)
  }
  fn log_sizes(&self) -> Result<LogSizes, StorageError> {
    self.inner.log_sizes()
  }
  fn compact(&mut self) -> Result<(), StorageError> {
    self.inner.compact()
  }
  fn is_broken(&self) -> bool {
    self.inner.is_broken()
  }
  fn mark_broken(&mut self) {
    self.inner.mark_broken()
  }
  fn take_pending_mirror_txns(&mut self) -> Vec<IdbMirrorOp> {
    self.inner.take_pending_mirror_txns()
  }
}

// ── 1. Cold start, offline seeding, & reconnect catch-up ─────────────────────

#[test]
fn cold_start_bootstrap_sequence_and_ready() {
  let mut engine = SyncEngine::new(
    SyncConfig {
      client_id: Some(10),
      ..Default::default()
    },
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  assert!(!engine.is_ready());
  assert_eq!(engine.phase(), SyncPhase::Offline);

  let e1 = engine.handle_event(SyncEvent::Connect);
  assert!(e1.contains(&SyncEffect::OpenSocket { epoch: 1 }));
  assert!(e1.contains(&SyncEffect::SyncPhaseChanged(SyncPhase::Offline)));

  let e2 = engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });
  assert_eq!(ws_frames(&e2), vec![json!({ "type": "sync.bootstrap" })]);
  assert!(e2.contains(&SyncEffect::ScheduleTimer {
    kind: TimerKind::Heartbeat,
    delay_ms: PING_EVERY_MS,
    epoch: 1,
  }));

  let server_doc = make_doc("cold");
  let e3 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.base",
      "base": STANDARD.encode(server_doc.encode_state()),
      "base_rid": 12,
    })
    .to_string(),
  });
  assert!(e3.contains(&SyncEffect::ServerConnected));
  assert!(e3.contains(&SyncEffect::Ready {
    root_block_id: "root".into(),
  }));
  assert!(e3.contains(&SyncEffect::SyncPhaseChanged(SyncPhase::Synced)));
  let frames = ws_frames(&e3);
  assert_eq!(frames.len(), 1);
  assert_eq!(frames[0]["type"], "sync.pull");
  assert_eq!(frames[0]["payload"]["since_rid"], 12);
  assert!(frames[0]["payload"]["sv"].is_string());
  assert!(e3.iter().any(|e| matches!(e, SyncEffect::MirrorIdb(ops) if matches!(ops.as_slice(), [IdbMirrorOp::SaveBase { .. }]))));
  assert!(engine.is_ready());
  assert_eq!(engine.cursor(), 12);
  assert_eq!(engine.doc().unwrap().client_id(), 10);
}

#[test]
fn offline_seed_from_local_storage_opens_before_socket() {
  let doc = make_doc("cached offline");
  let store = MemoryMirrorStorage::hydrate(IdbHydrateSnapshot {
    base: Some(doc.encode_state()),
    cursor: SyncCursor {
      last_synced_rid: 33,
      pushed_clock: 0,
    },
    ..Default::default()
  });
  let mut engine = SyncEngine::new(SyncConfig::default(), Some(Box::new(store)));

  let e1 = engine.handle_event(SyncEvent::Connect);
  assert!(e1.contains(&SyncEffect::Ready {
    root_block_id: "root".into(),
  }));
  assert_eq!(engine.cursor(), 33);
  assert_eq!(engine.doc().unwrap().to_blocks()[1].text, "cached offline");

  // SocketOpened sends SV-based catch-up bootstrap (not full cold bootstrap).
  let e2 = engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });
  let frames = ws_frames(&e2);
  assert_eq!(frames[0]["type"], "sync.bootstrap");
  assert!(frames[0]["payload"]["sv"].is_string());
}

#[test]
fn corrupt_local_cache_falls_through_to_cold_bootstrap() {
  let store = MemoryMirrorStorage::hydrate(IdbHydrateSnapshot {
    base: Some(vec![0xFF, 0xFE, 0xFD]),
    cursor: SyncCursor {
      last_synced_rid: 99,
      pushed_clock: 0,
    },
    ..Default::default()
  });
  let mut engine = SyncEngine::new(SyncConfig::default(), Some(Box::new(store)));

  let e1 = engine.handle_event(SyncEvent::Connect);
  // Not ready yet: corrupt cache discarded silently.
  assert!(!e1.iter().any(|e| matches!(e, SyncEffect::Ready { .. })));
  assert!(!engine.is_ready());

  // Sends full sync.bootstrap without sv.
  let e2 = engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });
  assert_eq!(ws_frames(&e2), vec![json!({ "type": "sync.bootstrap" })]);
}

#[test]
fn io_failure_during_seed_emits_fault_and_falls_back_to_cold_bootstrap() {
  let mut flaky = FlakyStorage::new(
    Arc::new(AtomicBool::new(false)),
    Arc::new(AtomicBool::new(false)),
  );
  flaky.fail_load_io = true;
  let mut engine = SyncEngine::new(SyncConfig::default(), Some(Box::new(flaky)));

  let e1 = engine.handle_event(SyncEvent::Connect);
  assert!(e1.contains(&SyncEffect::Fault {
    reason: "local_seed_failed".into(),
    count: 1,
  }));
  assert!(!engine.is_ready());
}

#[test]
fn rebootstrap_merges_unconditionally_even_when_base_rid_not_ahead() {
  let mut server_doc = make_doc("base");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 20);

  // Server has an extra edit, but reports base_rid == 15 (< cursor 20).
  let _ = edit_doc(&mut server_doc, "+backfill");
  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.base",
      "base": STANDARD.encode(server_doc.encode_state()),
      "base_rid": 15,
    })
    .to_string(),
  });
  assert!(
    e.iter()
      .any(|x| matches!(x, SyncEffect::RemoteBlocksChanged { .. }))
  );
  // Cursor did not regress, yet content merged (red line #1).
  assert_eq!(engine.cursor(), 20);
  assert_eq!(engine.doc().unwrap().to_blocks()[1].text, "base+backfill");
}

// ── 2. B2 pagination, remote updates, & block change tracking ────────────────

#[test]
fn sync_updates_paginates_while_cursor_advances_and_stops_on_empty() {
  let mut server_doc = make_doc("p");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  let u2 = edit_doc(&mut server_doc, "2");
  let u3 = edit_doc(&mut server_doc, "3");
  let e1 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.updates",
      "updates": [
        { "rid": 2, "update": STANDARD.encode(&u2) },
        { "rid": 3, "update": STANDARD.encode(&u3) },
      ]
    })
    .to_string(),
  });
  assert_eq!(engine.cursor(), 3);
  assert_eq!(engine.doc().unwrap().to_blocks()[1].text, "p23");
  // Made progress -> issues follow-up sync.pull from rid=3.
  let pulls: Vec<Value> = ws_frames(&e1)
    .into_iter()
    .filter(|f| f["type"] == "sync.pull")
    .collect();
  assert_eq!(pulls.len(), 1);
  assert_eq!(pulls[0]["payload"]["since_rid"], 3);

  // Empty batch terminates pagination.
  let e2 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "sync.updates", "updates": [] }).to_string(),
  });
  assert!(ws_frames(&e2).is_empty());
}

#[test]
fn sync_updates_stops_pagination_when_cursor_does_not_advance() {
  let mut server_doc = make_doc("p");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 10);

  // Duplicate rid <= cursor (10) -> no forward cursor progress -> no follow-up pull.
  let u = edit_doc(&mut server_doc, "x");
  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.updates",
      "updates": [{ "rid": 10, "update": STANDARD.encode(&u) }],
    })
    .to_string(),
  });
  assert!(ws_frames(&e).is_empty());
}

#[test]
fn remote_content_vs_structural_changes_and_json_accessors() {
  let mut server_doc = make_doc("alpha");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  // Content edit on p1 -> changed_ids == Some(["p1"]).
  let u2 = edit_doc(&mut server_doc, "!");
  let e1 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.update",
      "rid": 2,
      "update": STANDARD.encode(&u2),
    })
    .to_string(),
  });
  assert!(e1.contains(&SyncEffect::RemoteBlocksChanged {
    changed_ids: Some(vec!["p1".into()]),
  }));
  let changed_json = engine.changed_blocks_json().expect("content-only delta");
  let changed_blocks: Vec<Value> = serde_json::from_str(&changed_json).unwrap();
  assert_eq!(changed_blocks.len(), 1);
  assert_eq!(changed_blocks[0]["id"], "p1");
  assert_eq!(changed_blocks[0]["text"], "alpha!");
  // Second read without further changes is empty.
  assert_eq!(engine.changed_blocks_json().as_deref(), Some("[]"));

  // Structural edit (insert block p2) -> changed_ids == None.
  let sv = server_doc.state_vector();
  server_doc.insert_block("root", 1, &Block::new("p2", "paragraph").with_text("beta"));
  let u3 = server_doc.encode_diff(&sv).unwrap();
  let e2 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.update",
      "rid": 3,
      "update": STANDARD.encode(&u3),
    })
    .to_string(),
  });
  assert!(e2.contains(&SyncEffect::RemoteBlocksChanged { changed_ids: None }));
  assert_eq!(engine.changed_blocks_json(), None);
  let all: Vec<Value> = serde_json::from_str(&engine.all_blocks_json()).unwrap();
  assert_eq!(all.len(), 3);
  let by_id: Vec<Value> = serde_json::from_str(&engine.blocks_by_id_json(&["p2".into()])).unwrap();
  assert_eq!(by_id[0]["text"], "beta");
}

#[test]
fn ridless_sync_update_applies_without_persisting_or_advancing_cursor() {
  let mut server_doc = make_doc("hi");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 5);

  let u = edit_doc(&mut server_doc, "!");
  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "sync.update", "update": STANDARD.encode(&u) }).to_string(),
  });
  assert_eq!(engine.cursor(), 5);
  assert_eq!(engine.doc().unwrap().to_blocks()[1].text, "hi!");
  assert!(!e.iter().any(|x| matches!(x, SyncEffect::MirrorIdb(_))));
}

// ── 3. Outbox pacing (64-window), ≥8 merging, 4 MiB cap, #159, & poison isolation ──

#[test]
fn durable_outbox_64_window_pacing_on_demerged_backlog() {
  let server_doc = make_doc("w");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  // Disconnect and create 70 offline edits.
  engine.handle_event(SyncEvent::SocketClosed {
    epoch: 1,
    reason: "offline".into(),
  });
  for _ in 0..70 {
    let diff = edit_doc(engine.doc_mut().unwrap(), "x");
    engine.handle_event(SyncEvent::ApplyLocalDiff(diff));
  }

  // Reconnect -> 70 entries merge into one push id="70".
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 2 });
  let after_base = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({
      "type": "sync.base",
      "base": STANDARD.encode(server_doc.encode_state()),
      "base_rid": 1,
    })
    .to_string(),
  });
  let merged = ws_pushes(&after_base);
  assert_eq!(merged.len(), 1);
  assert_eq!(merged[0]["id"], "70");

  // Reject merged push "70" -> de-merges and sends strictly 64 individual pushes!
  let after_reject = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "error", "ack_id": "70", "code": "bad_payload" }).to_string(),
  });
  let batch1 = ws_pushes(&after_reject);
  assert_eq!(batch1.len(), PUSH_WINDOW);
  assert_eq!(batch1[0]["id"], "1");
  assert_eq!(batch1[63]["id"], "64");

  // Acking clock 1 frees exactly 1 slot -> sends clock 65.
  let ack1 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "sync.ack", "ack_id": "1", "rid": 2 }).to_string(),
  });
  let next = ws_pushes(&ack1);
  assert_eq!(next.len(), 1);
  assert_eq!(next[0]["id"], "65");

  // Ack 2..=70 drains completely.
  for clock in 2..=70 {
    engine.handle_event(SyncEvent::SocketFrame {
      epoch: 2,
      json: json!({ "type": "sync.ack", "ack_id": clock.to_string(), "rid": clock + 1 })
        .to_string(),
    });
  }
  assert!(engine.outbox_empty());
  assert_eq!(engine.phase(), SyncPhase::Synced);
}

#[test]
fn online_only_outbox_window_restore_and_rejected_slot_release() {
  let mut doc = make_doc("mem");
  let base = doc.encode_state();
  let mut diffs = Vec::new();
  for _ in 0..66 {
    diffs.push(edit_doc(&mut doc, "a"));
  }

  let mut engine = SyncEngine::new(
    SyncConfig {
      restore_unacked: diffs,
      ..Default::default()
    },
    None,
  );
  let after_base = bootstrap_engine(&mut engine, &MicaDoc::from_update(&base).unwrap(), 1);
  let pushes = ws_pushes(&after_base);
  assert_eq!(pushes.len(), PUSH_WINDOW);
  assert_eq!(engine.unacked_diffs().len(), 66);

  // Reject id "0" -> releases window slot -> sends id "64" immediately!
  let after_err = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "error", "ack_id": "0", "code": "conflict" }).to_string(),
  });
  let more = ws_pushes(&after_err);
  assert_eq!(more.len(), 1);
  assert_eq!(more[0]["id"], "64");

  // Ack id "1" -> sends id "65".
  let after_ack = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "sync.ack", "ack_id": "1", "rid": 2 }).to_string(),
  });
  let more2 = ws_pushes(&after_ack);
  assert_eq!(more2.len(), 1);
  assert_eq!(more2[0]["id"], "65");
  assert_eq!(engine.unacked_diffs().len(), 65);

  // Reconnect retries the rejected "0" too.
  engine.handle_event(SyncEvent::SocketClosed {
    epoch: 1,
    reason: "drop".into(),
  });
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 2 });
  let re = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "sync.base", "base": STANDARD.encode(&base), "base_rid": 2 }).to_string(),
  });
  let re_pushes = ws_pushes(&re);
  assert_eq!(re_pushes.len(), PUSH_WINDOW);
  assert_eq!(re_pushes[0]["id"], "0");
}

#[test]
fn backlog_under_8_pushes_individually_and_8_or_more_merges() {
  let server_doc = make_doc("m");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);
  engine.handle_event(SyncEvent::SocketClosed {
    epoch: 1,
    reason: "off".into(),
  });

  // 7 edits (< 8) -> pushed individually on reconnect.
  for _ in 0..7 {
    let d = edit_doc(engine.doc_mut().unwrap(), "a");
    engine.handle_event(SyncEvent::ApplyLocalDiff(d));
  }
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 2 });
  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "sync.base", "base": STANDARD.encode(server_doc.encode_state()), "base_rid": 1 }).to_string(),
  });
  assert_eq!(ws_pushes(&e).len(), 7);

  // Add 1 more while offline -> total 8 unpushed -> merges into 1 push id="8"!
  engine.handle_event(SyncEvent::SocketClosed {
    epoch: 2,
    reason: "off".into(),
  });
  let d8 = edit_doc(engine.doc_mut().unwrap(), "b");
  engine.handle_event(SyncEvent::ApplyLocalDiff(d8));

  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 3 });
  let e3 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 3,
    json: json!({ "type": "sync.base", "base": STANDARD.encode(server_doc.encode_state()), "base_rid": 1 }).to_string(),
  });
  let pushes = ws_pushes(&e3);
  assert_eq!(pushes.len(), 1);
  assert_eq!(pushes[0]["id"], "8");

  // Single ack for merged push "8" advances pushed_clock all the way to 8!
  engine.handle_event(SyncEvent::SocketFrame {
    epoch: 3,
    json: json!({ "type": "sync.ack", "ack_id": "8", "rid": 9 }).to_string(),
  });
  assert!(engine.outbox_empty());
  assert_eq!(engine.storage().unwrap().cursor().unwrap().pushed_clock, 8);
}

#[test]
fn issue_159_live_edit_during_capped_merge_window_does_not_skip_unsent_clocks() {
  // Build 10 valid diffs each > 512 KiB so 8 entries fit under 4 MiB and 9..10 remain unsent.
  let mut seed_doc = make_doc("big");
  let base = seed_doc.encode_state();
  let big_chunk = "A".repeat(520 * 1024);
  let mut outbox = Vec::new();
  for clock in 1..=10 {
    let diff = edit_doc(&mut seed_doc, &big_chunk);
    assert!(diff.len() > 512 * 1024);
    assert!(diff.len() < MERGE_MAX_BYTES);
    outbox.push(OutboxEntry { clock, bytes: diff });
  }
  let store = MemoryMirrorStorage::hydrate(IdbHydrateSnapshot {
    base: Some(base.clone()),
    outbox,
    cursor: SyncCursor {
      last_synced_rid: 1,
      pushed_clock: 0,
    },
    ..Default::default()
  });
  let mut engine = SyncEngine::new(SyncConfig::default(), Some(Box::new(store)));
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });

  // Capped merge window sends run 1..=7 or 1..=8 (depends on overhead); verify < 10.
  let after_base = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "sync.base", "base": STANDARD.encode(&base), "base_rid": 1 }).to_string(),
  });
  let merged = ws_pushes(&after_base);
  assert_eq!(merged.len(), 1);
  let run_end: i64 = merged[0]["id"].as_str().unwrap().parse().unwrap();
  assert!((1..10).contains(&run_end));

  // Live edit clock=11 arrives BEFORE merged run is acked!
  let live_diff = edit_doc(engine.doc_mut().unwrap(), "live");
  let live_effects = engine.handle_event(SyncEvent::ApplyLocalDiff(live_diff));
  let live_pushes = ws_pushes(&live_effects);
  assert_eq!(live_pushes.len(), 1);
  assert_eq!(live_pushes[0]["id"], "11");

  // Now the merged run is acked -> unsent clocks (run_end+1)..=10 MUST be pushed!
  let after_ack = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "sync.ack", "ack_id": run_end.to_string(), "rid": 2 }).to_string(),
  });
  let tail_ids: Vec<i64> = ws_pushes(&after_ack)
    .iter()
    .map(|p| p["id"].as_str().unwrap().parse::<i64>().unwrap())
    .collect();
  for expected in (run_end + 1)..=10 {
    assert!(
      tail_ids.contains(&expected),
      "unsent clock {expected} must not be skipped (got {tail_ids:?})"
    );
  }
}

#[test]
fn poison_edit_demerge_isolates_bad_clock_and_stalls_after_budget() {
  let server_doc = make_doc("p");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);
  engine.handle_event(SyncEvent::SocketClosed {
    epoch: 1,
    reason: "off".into(),
  });

  // Queue 10 edits offline.
  for i in 0..10 {
    let d = edit_doc(engine.doc_mut().unwrap(), &i.to_string());
    engine.handle_event(SyncEvent::ApplyLocalDiff(d));
  }

  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 2 });
  let after_base = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "sync.base", "base": STANDARD.encode(server_doc.encode_state()), "base_rid": 1 }).to_string(),
  });
  assert_eq!(ws_pushes(&after_base)[0]["id"], "10");

  // Merged push "10" rejected -> de-merged into individual pushes 1..=10 (reject count = 1).
  let demerged = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "error", "ack_id": "10", "code": "rejected" }).to_string(),
  });
  let demerged_pushes = ws_pushes(&demerged);
  let ids: Vec<&str> = demerged_pushes
    .iter()
    .map(|p| p["id"].as_str().unwrap())
    .collect();
  assert_eq!(ids, vec!["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);

  // Good clocks 1..=4 ack (contiguous progress resets push_rejects to 0!).
  for c in 1..=4 {
    engine.handle_event(SyncEvent::SocketFrame {
      epoch: 2,
      json: json!({ "type": "sync.ack", "ack_id": c.to_string(), "rid": c + 1 }).to_string(),
    });
  }
  assert_eq!(engine.storage().unwrap().cursor().unwrap().pushed_clock, 4);

  // Higher clocks 6..=10 ack out of order, while poison clock 5 keeps rejecting.
  for c in 6..=10 {
    engine.handle_event(SyncEvent::SocketFrame {
      epoch: 2,
      json: json!({ "type": "sync.ack", "ack_id": c.to_string(), "rid": c + 1 }).to_string(),
    });
  }
  // Contiguous pushed_clock MUST NOT skip poison clock 5!
  assert_eq!(engine.storage().unwrap().cursor().unwrap().pushed_clock, 4);

  // Reject clock 5 `MAX_PUSH_REJECTS` times -> retries; (MAX_PUSH_REJECTS + 1)th stalls with Fault.
  for _ in 0..MAX_PUSH_REJECTS {
    let e = engine.handle_event(SyncEvent::SocketFrame {
      epoch: 2,
      json: json!({ "type": "error", "ack_id": "5", "code": "forbidden" }).to_string(),
    });
    assert!(!e.iter().any(|x| matches!(x, SyncEffect::Fault { .. })));
  }
  let stalled = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "error", "ack_id": "5", "code": "forbidden" }).to_string(),
  });
  assert!(stalled.contains(&SyncEffect::Fault {
    reason: "push_rejected".into(),
    count: MAX_PUSH_REJECTS as u32,
  }));

  // While stalled, new live edits persist durably to outbox but do NOT push on the wire.
  let d11 = edit_doc(engine.doc_mut().unwrap(), "z");
  let live = engine.handle_event(SyncEvent::ApplyLocalDiff(d11));
  assert!(ws_pushes(&live).is_empty());
  assert_eq!(
    engine
      .storage()
      .unwrap()
      .outbox_after(4)
      .unwrap()
      .last()
      .unwrap()
      .clock,
    11
  );
}

#[test]
fn corrupt_entry_in_backlog_falls_back_to_individual_pushes() {
  let mut doc = make_doc("c");
  let base = doc.encode_state();
  let mut outbox = Vec::new();
  for clock in 1..=9 {
    let bytes = if clock == 5 {
      vec![0xFF, 0xFF, 0xFF] // corrupt yrs diff
    } else {
      edit_doc(&mut doc, "a")
    };
    outbox.push(OutboxEntry { clock, bytes });
  }
  // Hydrate with clean base only, then append outbox rows manually.
  let store = MemoryMirrorStorage::hydrate(IdbHydrateSnapshot {
    base: Some(base.clone()),
    outbox,
    cursor: SyncCursor {
      last_synced_rid: 1,
      pushed_clock: 0,
    },
    ..Default::default()
  });
  // Load will see corrupt outbox -> falls back to cold bootstrap; sync.base then flushes outbox!
  let mut engine = SyncEngine::new(SyncConfig::default(), Some(Box::new(store)));
  let after_base = bootstrap_engine(&mut engine, &MicaDoc::from_update(&base).unwrap(), 1);
  // Merge fails on clock 5 -> falls back to sending all 9 entries individually!
  let pushes = ws_pushes(&after_base);
  assert_eq!(pushes.len(), 9);
  assert_eq!(pushes[0]["id"], "1");
  assert_eq!(pushes[8]["id"], "9");
}

#[test]
fn out_of_order_acked_ahead_advances_contiguously() {
  let server_doc = make_doc("o");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  for ch in ["1", "2", "3"] {
    let d = edit_doc(engine.doc_mut().unwrap(), ch);
    engine.handle_event(SyncEvent::ApplyLocalDiff(d));
  }
  assert_eq!(engine.phase(), SyncPhase::Syncing);

  // Ack 3, then 2 -> pushed_clock remains 0.
  engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "sync.ack", "ack_id": "3", "rid": 4 }).to_string(),
  });
  engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "sync.ack", "ack_id": "2", "rid": 3 }).to_string(),
  });
  assert_eq!(engine.storage().unwrap().cursor().unwrap().pushed_clock, 0);
  assert_eq!(engine.phase(), SyncPhase::Syncing);

  // Ack 1 -> contiguous prefix jumps straight to 3 -> Synced!
  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "sync.ack", "ack_id": "1", "rid": 2 }).to_string(),
  });
  assert_eq!(engine.storage().unwrap().cursor().unwrap().pushed_clock, 3);
  assert!(e.contains(&SyncEffect::SyncPhaseChanged(SyncPhase::Synced)));
}

// ── 4. B11 history repair ────────────────────────────────────────────────────

#[test]
fn b11_repair_on_missing_ack_is_capped_at_3_and_resets_on_reconnect() {
  let server_doc = make_doc("r");
  let empty_sv = STANDARD.encode(MicaDoc::from_update(&[0, 0]).unwrap().state_vector());
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  for i in 1..=5 {
    let e = engine.handle_event(SyncEvent::SocketFrame {
      epoch: 1,
      json: json!({
        "type": "sync.ack",
        "ack_id": "99",
        "rid": i + 1,
        "missing": true,
        "sv": empty_sv,
      })
      .to_string(),
    });
    let pushes = ws_pushes(&e);
    if i <= MAX_REPAIRS {
      assert_eq!(pushes.len(), 1);
      assert_eq!(pushes[0]["id"], format!("repair-{i}"));
    } else {
      assert!(pushes.is_empty(), "capped at MAX_REPAIRS");
    }
  }

  // Reconnect resets the repair budget.
  engine.handle_event(SyncEvent::SocketClosed {
    epoch: 1,
    reason: "drop".into(),
  });
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 2 });
  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({
      "type": "sync.ack",
      "ack_id": "100",
      "rid": 10,
      "missing": true,
      "sv": empty_sv,
    })
    .to_string(),
  });
  assert_eq!(ws_pushes(&e)[0]["id"], "repair-1");
}

#[test]
fn b11_repair_on_sync_base_ahead_only_when_outbox_empty() {
  let server_doc = make_doc("base");
  let server_sv = STANDARD.encode(server_doc.state_vector());
  let mut local_doc = MicaDoc::from_update(&server_doc.encode_state()).unwrap();
  let _ = edit_doc(&mut local_doc, " + already-acked-but-server-lost");

  // Case A: empty outbox -> sends repair-1 on sync.base(ahead: true).
  let store_empty = MemoryMirrorStorage::hydrate(IdbHydrateSnapshot {
    base: Some(local_doc.encode_state()),
    cursor: SyncCursor {
      last_synced_rid: 5,
      pushed_clock: 1,
    },
    ..Default::default()
  });
  let mut engine_a = SyncEngine::new(SyncConfig::default(), Some(Box::new(store_empty)));
  engine_a.handle_event(SyncEvent::Connect);
  engine_a.handle_event(SyncEvent::SocketOpened { epoch: 1 });
  let ea = engine_a.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.base",
      "base": STANDARD.encode(server_doc.encode_state()),
      "base_rid": 5,
      "ahead": true,
      "sv": server_sv,
    })
    .to_string(),
  });
  let pushes_a = ws_pushes(&ea);
  assert_eq!(pushes_a.len(), 1);
  assert_eq!(pushes_a[0]["id"], "repair-1");

  // Case B: non-empty outbox -> outbox replay runs, NO duplicate repair push!
  let mut local_doc_b = MicaDoc::from_update(&server_doc.encode_state()).unwrap();
  let unpushed = edit_doc(&mut local_doc_b, " + unpushed");
  let store_non_empty = MemoryMirrorStorage::hydrate(IdbHydrateSnapshot {
    base: Some(server_doc.encode_state()),
    outbox: vec![OutboxEntry {
      clock: 1,
      bytes: unpushed,
    }],
    cursor: SyncCursor {
      last_synced_rid: 5,
      pushed_clock: 0,
    },
    ..Default::default()
  });
  let mut engine_b = SyncEngine::new(SyncConfig::default(), Some(Box::new(store_non_empty)));
  engine_b.handle_event(SyncEvent::Connect);
  engine_b.handle_event(SyncEvent::SocketOpened { epoch: 1 });
  let eb = engine_b.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.base",
      "base": STANDARD.encode(server_doc.encode_state()),
      "base_rid": 5,
      "ahead": true,
      "sv": server_sv,
    })
    .to_string(),
  });
  let pushes_b = ws_pushes(&eb);
  assert_eq!(pushes_b.len(), 1);
  assert_eq!(pushes_b[0]["id"], "1"); // only outbox push, no repair-1
}

// ── 5. Integrity fault circuit breaker & persistence self-heal ───────────────

#[test]
fn integrity_fault_reheals_up_to_3_times_then_circuit_breaks_and_resets_on_good_base() {
  let server_doc = make_doc("ok");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  let bad_update = json!({
    "type": "sync.update",
    "rid": 2,
    "update": STANDARD.encode([0xFF, 0xFF, 0xFF]),
  })
  .to_string();

  for attempt in 1..=(MAX_AUTO_REHEAL as u32 + 2) {
    let e = engine.handle_event(SyncEvent::SocketFrame {
      epoch: 1,
      json: bad_update.clone(),
    });
    assert!(e.contains(&SyncEffect::Fault {
      reason: "bad_remote_update".into(),
      count: attempt,
    }));
    // Cursor MUST NOT advance past bad remote update.
    assert_eq!(engine.cursor(), 1);
    let frames = ws_frames(&e);
    if (attempt as usize) <= MAX_AUTO_REHEAL {
      // Full sync.bootstrap without sv.
      assert_eq!(frames, vec![json!({ "type": "sync.bootstrap" })]);
    } else {
      assert!(frames.is_empty(), "circuit breaker stopped auto-reheal");
    }
  }

  // A valid sync.base resets the consecutive fault counter.
  engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.base",
      "base": STANDARD.encode(server_doc.encode_state()),
      "base_rid": 3,
    })
    .to_string(),
  });
  let after_reset = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: bad_update,
  });
  assert!(after_reset.contains(&SyncEffect::Fault {
    reason: "bad_remote_update".into(),
    count: 1,
  }));
  assert_eq!(
    ws_frames(&after_reset),
    vec![json!({ "type": "sync.bootstrap" })]
  );
}

#[test]
fn bad_base_on_cold_start_emits_fault_and_retries() {
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });

  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.base",
      "base": STANDARD.encode([0xFF, 0xFF]),
      "base_rid": 1,
    })
    .to_string(),
  });
  assert!(!engine.is_ready());
  assert!(e.contains(&SyncEffect::Fault {
    reason: "bad_base".into(),
    count: 1,
  }));
  assert_eq!(ws_frames(&e), vec![json!({ "type": "sync.bootstrap" })]);
}

#[test]
fn mirror_write_failed_marks_broken_and_next_remote_update_self_heals_via_save_base() {
  let mut server_doc = make_doc("web");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  let e_fail = engine.handle_event(SyncEvent::MirrorWriteFailed);
  assert!(engine.storage().unwrap().is_broken());
  assert!(e_fail.contains(&SyncEffect::Fault {
    reason: "mirror_write_failed".into(),
    count: 1,
  }));

  // Next remote update fails append_remote (storage is broken) -> heals via save_base!
  let u2 = edit_doc(&mut server_doc, "+2");
  let e_heal = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.update",
      "rid": 2,
      "update": STANDARD.encode(&u2),
    })
    .to_string(),
  });
  assert!(
    !engine.storage().unwrap().is_broken(),
    "save_base recovered broken mirror"
  );
  assert!(e_heal.contains(&SyncEffect::Fault {
    reason: "persist_failed".into(),
    count: 2,
  }));
  assert!(e_heal.iter().any(|e| matches!(
    e,
    SyncEffect::MirrorIdb(ops) if ops.iter().any(|op| matches!(op, IdbMirrorOp::SaveBase { cursor, .. } if cursor.last_synced_rid == 2))
  )));
}

// ── 6. Timerless compaction (>256 rows checked every 32 appends) ─────────────

#[test]
fn auto_compaction_fires_on_32nd_append_over_256_rows() {
  let mut server_doc = make_doc("c");
  let base = server_doc.encode_state();
  let mut store = MemoryMirrorStorage::new();
  store.save_base(&base, 0).unwrap();

  // Pre-seed 256 remote rows directly into storage.
  let mut seed_batch = Vec::new();
  for rid in 1..=(COMPACT_THRESHOLD as i64) {
    seed_batch.push((rid, edit_doc(&mut server_doc, ".")));
  }
  store.append_remote_batch(&seed_batch).unwrap();
  let _ = store.take_pending_mirror_txns();

  let mut engine = SyncEngine::new(SyncConfig::default(), Some(Box::new(store)));
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });

  // Deliver 31 single-item batches: total remote rows = 287 > 256, but check runs on 32nd!
  for i in 1..COMPACT_CHECK_EVERY {
    let rid = COMPACT_THRESHOLD as i64 + i as i64;
    let u = edit_doc(&mut server_doc, "a");
    let e = engine.handle_event(SyncEvent::SocketFrame {
      epoch: 1,
      json: json!({
        "type": "sync.updates",
        "updates": [{ "rid": rid, "update": STANDARD.encode(&u) }],
      })
      .to_string(),
    });
    assert!(!e.iter().any(|x| matches!(x, SyncEffect::MirrorIdb(ops) if ops.iter().any(|o| matches!(o, IdbMirrorOp::Compact { .. })))));
  }
  assert_eq!(
    engine.storage().unwrap().log_sizes().unwrap().remote,
    COMPACT_THRESHOLD + COMPACT_CHECK_EVERY - 1
  );

  // 32nd batch triggers compaction!
  let rid_32 = (COMPACT_THRESHOLD + COMPACT_CHECK_EVERY) as i64;
  let u32 = edit_doc(&mut server_doc, "Z");
  let e32 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.updates",
      "updates": [{ "rid": rid_32, "update": STANDARD.encode(&u32) }],
    })
    .to_string(),
  });
  assert_eq!(engine.storage().unwrap().log_sizes().unwrap().remote, 0);
  // Both AppendRemote (T1) and Compact (T2) appear in order (#161)!
  let ops = e32
    .iter()
    .find_map(|x| match x {
      SyncEffect::MirrorIdb(ops) => Some(ops.as_slice()),
      _ => None,
    })
    .expect("MirrorIdb emitted");
  assert_eq!(ops.len(), 2);
  assert!(matches!(ops[0], IdbMirrorOp::AppendRemote { .. }));
  assert!(matches!(ops[1], IdbMirrorOp::Compact { .. }));
}

#[test]
fn compact_now_folds_and_emits_compact_mirror_op() {
  let mut server_doc = make_doc("fold");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  let u2 = edit_doc(&mut server_doc, "+end");
  engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "sync.update", "rid": 2, "update": STANDARD.encode(&u2) }).to_string(),
  });
  assert_eq!(engine.storage().unwrap().log_sizes().unwrap().remote, 1);

  let effects = engine.compact_now();
  assert_eq!(engine.storage().unwrap().log_sizes().unwrap().remote, 0);
  assert!(effects.iter().any(|e| matches!(
    e,
    SyncEffect::MirrorIdb(ops) if matches!(ops.as_slice(), [IdbMirrorOp::Compact { .. }])
  )));
}

// ── 7. Single-slot pending_local_diff retry & FlushForExit ───────────────────

#[test]
fn pending_local_diff_retries_with_backoff_merges_and_recovers() {
  let fail_outbox = Arc::new(AtomicBool::new(true));
  let fail_remote = Arc::new(AtomicBool::new(false));
  let storage = FlakyStorage::new(fail_outbox.clone(), fail_remote);

  let server_doc = make_doc("retry");
  let mut engine = SyncEngine::new(SyncConfig::default(), Some(Box::new(storage)));
  bootstrap_engine(&mut engine, &server_doc, 1);

  // First local edit fails to persist -> retained in single slot + retry scheduled (400ms).
  let d1 = edit_doc(engine.doc_mut().unwrap(), "1");
  let e1 = engine.handle_event(SyncEvent::ApplyLocalDiff(d1));
  assert!(!engine.outbox_empty());
  assert_eq!(engine.phase(), SyncPhase::Syncing);
  assert!(e1.contains(&SyncEffect::ScheduleTimer {
    kind: TimerKind::LocalUpdateRetry,
    delay_ms: 400,
    epoch: 1,
  }));
  assert!(e1.contains(&SyncEffect::Fault {
    reason: "local_update_persist_failed".into(),
    count: 1,
  }));

  // Second local edit while still failing folds into the single slot.
  let d2 = edit_doc(engine.doc_mut().unwrap(), "2");
  engine.handle_event(SyncEvent::ApplyLocalDiff(d2));

  // Retry ticks fail with backoff 800ms, 1600ms.
  let r1 = engine.handle_event(SyncEvent::Tick {
    now_ms: 400,
    timer_kind: TimerKind::LocalUpdateRetry,
    epoch: 1,
  });
  assert!(r1.contains(&SyncEffect::ScheduleTimer {
    kind: TimerKind::LocalUpdateRetry,
    delay_ms: 800,
    epoch: 1,
  }));
  let r2 = engine.handle_event(SyncEvent::Tick {
    now_ms: 1_200,
    timer_kind: TimerKind::LocalUpdateRetry,
    epoch: 1,
  });
  assert!(r2.contains(&SyncEffect::ScheduleTimer {
    kind: TimerKind::LocalUpdateRetry,
    delay_ms: 1_600,
    epoch: 1,
  }));

  // Storage recovers! Next retry flushes the merged diff (both "1" and "2").
  fail_outbox.store(false, Ordering::SeqCst);
  let r3 = engine.handle_event(SyncEvent::Tick {
    now_ms: 2_800,
    timer_kind: TimerKind::LocalUpdateRetry,
    epoch: 1,
  });
  let pushes = ws_pushes(&r3);
  assert_eq!(pushes.len(), 1);
  let merged_bytes = STANDARD
    .decode(pushes[0]["payload"]["update"].as_str().unwrap())
    .unwrap();
  let mut verify = MicaDoc::from_update(&server_doc.encode_state()).unwrap();
  verify.apply_update(&merged_bytes).unwrap();
  assert_eq!(verify.to_blocks()[1].text, "retry12");
}

#[test]
fn flush_for_exit_persists_pending_diff_without_socket_or_compaction() {
  let fail_outbox = Arc::new(AtomicBool::new(true));
  let storage = FlakyStorage::new(fail_outbox.clone(), Arc::new(AtomicBool::new(false)));
  let server_doc = make_doc("exit");
  let mut engine = SyncEngine::new(SyncConfig::default(), Some(Box::new(storage)));
  bootstrap_engine(&mut engine, &server_doc, 1);

  let d1 = edit_doc(engine.doc_mut().unwrap(), "!");
  engine.handle_event(SyncEvent::ApplyLocalDiff(d1));

  // Storage becomes healthy right before exit.
  fail_outbox.store(false, Ordering::SeqCst);
  let exit_effects = engine.handle_event(SyncEvent::FlushForExit);
  assert!(
    ws_frames(&exit_effects).is_empty(),
    "no WS send on FlushForExit"
  );
  assert!(exit_effects.contains(&SyncEffect::CancelTimer {
    kind: TimerKind::LocalUpdateRetry,
  }));
  assert!(exit_effects.iter().any(|e| matches!(
    e,
    SyncEffect::MirrorIdb(ops) if matches!(ops.as_slice(), [IdbMirrorOp::AppendOutbox { .. }])
  )));
  assert_eq!(engine.storage().unwrap().outbox_after(0).unwrap().len(), 1);
}

#[test]
fn empty_local_diff_is_ignored() {
  let server_doc = make_doc("noop");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  assert!(
    engine
      .handle_event(SyncEvent::ApplyLocalDiff(vec![]))
      .is_empty()
  );
  assert!(
    engine
      .handle_event(SyncEvent::ApplyLocalDiff(vec![0, 0]))
      .is_empty()
  );
  assert!(engine.outbox_empty());
}

// ── 8. Heartbeat, stale timeout, backoff reconnect, epoch isolation, & lag ───

#[test]
fn heartbeat_pings_refreshes_on_pong_and_closes_stale_socket_unix_epoch_clock() {
  let server_doc = make_doc("hb");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  // Use realistic Unix epoch timestamps (~2026) to prove zero clock-origin assumptions.
  let t0: u64 = 1_790_000_000_000;

  // Tick 1: first heartbeat after sync.base -> records baseline, sends ping.
  let e1 = engine.handle_event(SyncEvent::Tick {
    now_ms: t0,
    timer_kind: TimerKind::Heartbeat,
    epoch: 1,
  });
  assert_eq!(ws_frames(&e1), vec![json!({ "type": "ping" })]);

  // Server answers with pong -> keeps link alive.
  engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: r#"{"type":"pong"}"#.into(),
  });

  // Tick 2 (+8s): pong arrived -> still alive, sends ping.
  let e2 = engine.handle_event(SyncEvent::Tick {
    now_ms: t0 + 8_000,
    timer_kind: TimerKind::Heartbeat,
    epoch: 1,
  });
  assert_eq!(ws_frames(&e2), vec![json!({ "type": "ping" })]);

  // Cable pulled (no more frames).
  // +16s (8s silence <= 20s): still alive, sends ping.
  let e3 = engine.handle_event(SyncEvent::Tick {
    now_ms: t0 + 16_000,
    timer_kind: TimerKind::Heartbeat,
    epoch: 1,
  });
  assert_eq!(ws_frames(&e3), vec![json!({ "type": "ping" })]);

  // +24s (16s silence <= 20s): still alive, sends ping.
  let e4 = engine.handle_event(SyncEvent::Tick {
    now_ms: t0 + 24_000,
    timer_kind: TimerKind::Heartbeat,
    epoch: 1,
  });
  assert_eq!(ws_frames(&e4), vec![json!({ "type": "ping" })]);

  // +32s (24s silence > 20s): STALE -> flips offline and closes zombie socket!
  let e5 = engine.handle_event(SyncEvent::Tick {
    now_ms: t0 + 32_000,
    timer_kind: TimerKind::Heartbeat,
    epoch: 1,
  });
  assert!(!engine.is_online());
  assert!(e5.contains(&SyncEffect::SyncPhaseChanged(SyncPhase::Offline)));
  assert!(e5.contains(&SyncEffect::CancelTimer {
    kind: TimerKind::Heartbeat,
  }));
  assert!(e5.contains(&SyncEffect::CloseSocket { epoch: 1 }));
}

#[test]
fn reconnect_backoff_escalates_and_resets_after_valid_frame() {
  let mut engine = SyncEngine::new(SyncConfig::default(), None);
  engine.handle_event(SyncEvent::Connect);

  let expected = [500, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000];
  let mut now = 0u64;
  for (i, &delay) in expected.iter().enumerate() {
    let epoch = (i as u64) + 1;
    let closed = engine.handle_event(SyncEvent::SocketClosed {
      epoch,
      reason: "timeout".into(),
    });
    assert!(closed.contains(&SyncEffect::ScheduleTimer {
      kind: TimerKind::Reconnect,
      delay_ms: delay,
      epoch,
    }));
    now += delay;
    let tick = engine.handle_event(SyncEvent::Tick {
      now_ms: now,
      timer_kind: TimerKind::Reconnect,
      epoch,
    });
    assert!(tick.contains(&SyncEffect::OpenSocket { epoch: epoch + 1 }));
  }

  // Receive a valid frame on the latest epoch -> resets backoff back to 500ms!
  let live_epoch = expected.len() as u64 + 1;
  engine.handle_event(SyncEvent::SocketOpened { epoch: live_epoch });
  engine.handle_event(SyncEvent::SocketFrame {
    epoch: live_epoch,
    json: r#"{"type":"pong"}"#.into(),
  });
  let closed = engine.handle_event(SyncEvent::SocketClosed {
    epoch: live_epoch,
    reason: "drop".into(),
  });
  assert!(closed.contains(&SyncEffect::ScheduleTimer {
    kind: TimerKind::Reconnect,
    delay_ms: 500,
    epoch: live_epoch,
  }));
}

#[test]
fn stale_epoch_events_are_silently_ignored() {
  let server_doc = make_doc("ep");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  // Move to epoch 2.
  let e_conn = engine.handle_event(SyncEvent::Connect);
  assert!(e_conn.contains(&SyncEffect::CloseSocket { epoch: 1 }));
  assert!(e_conn.contains(&SyncEffect::OpenSocket { epoch: 2 }));
  assert_eq!(engine.epoch(), 2);

  // Old epoch 1 events must produce zero effects and mutate nothing.
  assert!(
    engine
      .handle_event(SyncEvent::SocketOpened { epoch: 1 })
      .is_empty()
  );
  assert!(
    engine
      .handle_event(SyncEvent::SocketFrame {
        epoch: 1,
        json: r#"{"type":"pong"}"#.into(),
      })
      .is_empty()
  );
  assert!(
    engine
      .handle_event(SyncEvent::SocketClosed {
        epoch: 1,
        reason: "old".into(),
      })
      .is_empty()
  );
  assert!(
    engine
      .handle_event(SyncEvent::Tick {
        now_ms: 99_999,
        timer_kind: TimerKind::Heartbeat,
        epoch: 1,
      })
      .is_empty()
  );
  assert!(
    engine
      .handle_event(SyncEvent::Tick {
        now_ms: 99_999,
        timer_kind: TimerKind::Reconnect,
        epoch: 1,
      })
      .is_empty()
  );
  assert!(
    engine
      .handle_event(SyncEvent::Tick {
        now_ms: 99_999,
        timer_kind: TimerKind::LagResync,
        epoch: 1,
      })
      .is_empty()
  );
  assert_eq!(engine.epoch(), 2);
  assert!(!engine.is_online());
}

#[test]
fn lag_notice_resyncs_immediately_and_debounces_burst_with_trailing_edge() {
  let server_doc = make_doc("lag");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  let lag_frame = json!({
    "type": "error",
    "code": "client_out_of_date",
    "message": "lagged",
  })
  .to_string();

  // First notice sends immediate SV catch-up and starts 1s debounce window.
  let e1 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: lag_frame.clone(),
  });
  assert_eq!(engine.lag_resync_count(), 1);
  assert_eq!(ws_frames(&e1)[0]["type"], "sync.bootstrap");
  assert!(ws_frames(&e1)[0]["payload"]["sv"].is_string());
  assert!(e1.contains(&SyncEffect::ScheduleTimer {
    kind: TimerKind::LagResync,
    delay_ms: LAG_RESYNC_DEBOUNCE_MS,
    epoch: 1,
  }));

  // Burst of 3 more notices inside the 1s window sends NOTHING immediately.
  for _ in 0..3 {
    let burst = engine.handle_event(SyncEvent::SocketFrame {
      epoch: 1,
      json: lag_frame.clone(),
    });
    assert!(ws_frames(&burst).is_empty());
  }
  assert_eq!(engine.lag_resync_count(), 1);

  // End of window fires exactly ONE trailing catch-up.
  let trail = engine.handle_event(SyncEvent::Tick {
    now_ms: 1_000,
    timer_kind: TimerKind::LagResync,
    epoch: 1,
  });
  assert_eq!(engine.lag_resync_count(), 2);
  assert_eq!(ws_frames(&trail)[0]["type"], "sync.bootstrap");

  // Next window with no further notices fires nothing.
  let quiet = engine.handle_event(SyncEvent::Tick {
    now_ms: 2_000,
    timer_kind: TimerKind::LagResync,
    epoch: 1,
  });
  assert_eq!(engine.lag_resync_count(), 2);
  assert!(ws_frames(&quiet).is_empty());
}

// ── 9. Presence, AcceptedSeq, malformed JSON, & two-replica convergence ──────

#[test]
fn presence_roster_filters_self_tracks_cursors_and_reannounces_on_reconnect() {
  let mut engine = SyncEngine::new(
    SyncConfig {
      self_name: Some("Alice".into()),
      ..Default::default()
    },
    None,
  );
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });

  // document.bootstrap sets own connection_id and announces presence.
  let e1 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "document.bootstrap", "connection_id": "conn-self" }).to_string(),
  });
  assert_eq!(
    ws_frames(&e1),
    vec![json!({ "type": "presence.update", "payload": { "name": "Alice" } })]
  );

  // presence.state with self + peer -> self filtered out.
  let e2 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "presence.state",
      "presences": [
        { "connection_id": "conn-self", "user_id": "u1", "data": { "name": "Alice" } },
        { "connection_id": "conn-bob", "user_id": "u2", "data": { "name": "Bob", "cursor": { "block": "p1", "offset": 4 } } },
      ]
    })
    .to_string(),
  });
  assert!(e2.contains(&SyncEffect::PresenceChanged(vec![PresenceUser {
    connection_id: "conn-bob".into(),
    user_id: "u2".into(),
    name: "Bob".into(),
    cursor_block_id: Some("p1".into()),
    cursor_offset: Some(4),
  }])));

  // Send local cursor.
  let e3 = engine.handle_event(SyncEvent::SendPresenceCursor {
    block_id: Some("p1".into()),
    offset: Some(7),
  });
  assert_eq!(
    ws_frames(&e3),
    vec![json!({
      "type": "presence.update",
      "payload": { "name": "Alice", "cursor": { "block": "p1", "offset": 7 } }
    })]
  );

  // Peer leaves -> roster becomes empty.
  let e4 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "presence.leave", "connection_id": "conn-bob" }).to_string(),
  });
  assert!(e4.contains(&SyncEffect::PresenceChanged(vec![])));

  // Reconnect re-announces saved cursor on new document.bootstrap.
  engine.handle_event(SyncEvent::SocketClosed {
    epoch: 1,
    reason: "drop".into(),
  });
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 2 });
  let e5 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "document.bootstrap", "connection_id": "conn-self-2" }).to_string(),
  });
  assert_eq!(
    ws_frames(&e5),
    vec![json!({
      "type": "presence.update",
      "payload": { "name": "Alice", "cursor": { "block": "p1", "offset": 7 } }
    })]
  );
}

#[test]
fn document_update_accepted_emits_accepted_seq() {
  let mut engine = SyncEngine::new(SyncConfig::default(), None);
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });

  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "document.update.accepted", "server_seq": 42 }).to_string(),
  });
  assert!(e.contains(&SyncEffect::AcceptedSeq(42)));
}

#[test]
fn malformed_json_and_non_object_frames_are_ignored() {
  let mut engine = SyncEngine::new(SyncConfig::default(), None);
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });

  assert!(
    engine
      .handle_event(SyncEvent::SocketFrame {
        epoch: 1,
        json: "not json{".into(),
      })
      .is_empty()
  );
  assert!(
    engine
      .handle_event(SyncEvent::SocketFrame {
        epoch: 1,
        json: "[1, 2, 3]".into(),
      })
      .is_empty()
  );
  assert!(!engine.is_online());
}

#[test]
fn two_replicas_offline_concurrent_edits_converge_identically() {
  let mut hub_doc = make_doc("Hello");
  let initial_state = hub_doc.encode_state();

  let mut desktop = SyncEngine::new(
    SyncConfig {
      client_id: Some(1001),
      ..Default::default()
    },
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  let mut web = SyncEngine::new(
    SyncConfig {
      client_id: None, // random web actor
      ..Default::default()
    },
    Some(Box::new(MemoryMirrorStorage::new())),
  );

  bootstrap_engine(&mut desktop, &hub_doc, 1);
  bootstrap_engine(&mut web, &hub_doc, 1);

  // Both go offline and edit concurrently (desktop prepends, web appends).
  desktop.handle_event(SyncEvent::SocketClosed {
    epoch: 1,
    reason: "off".into(),
  });
  web.handle_event(SyncEvent::SocketClosed {
    epoch: 1,
    reason: "off".into(),
  });

  let desktop_diff = {
    let d = desktop.doc_mut().unwrap();
    let sv = d.state_vector();
    d.text_insert("p1", 0, "[Desktop] ");
    d.encode_diff(&sv).unwrap()
  };
  desktop.handle_event(SyncEvent::ApplyLocalDiff(desktop_diff));

  let web_diff = {
    let d = web.doc_mut().unwrap();
    let sv = d.state_vector();
    d.text_insert("p1", 5, " [Web]");
    d.encode_diff(&sv).unwrap()
  };
  web.handle_event(SyncEvent::ApplyLocalDiff(web_diff));

  // Both reconnect and push to the hub.
  desktop.handle_event(SyncEvent::Connect);
  desktop.handle_event(SyncEvent::SocketOpened { epoch: 2 });
  let d_effects = desktop.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "sync.base", "base": STANDARD.encode(&initial_state), "base_rid": 1 })
      .to_string(),
  });
  let d_push_bytes = STANDARD
    .decode(
      ws_pushes(&d_effects)[0]["payload"]["update"]
        .as_str()
        .unwrap(),
    )
    .unwrap();
  hub_doc.apply_update(&d_push_bytes).unwrap();
  desktop.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "sync.ack", "ack_id": "1", "rid": 2 }).to_string(),
  });

  web.handle_event(SyncEvent::Connect);
  web.handle_event(SyncEvent::SocketOpened { epoch: 2 });
  let w_effects = web.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json:
      json!({ "type": "sync.base", "base": STANDARD.encode(hub_doc.encode_state()), "base_rid": 2 })
        .to_string(),
  });
  let w_push_bytes = STANDARD
    .decode(
      ws_pushes(&w_effects)[0]["payload"]["update"]
        .as_str()
        .unwrap(),
    )
    .unwrap();
  hub_doc.apply_update(&w_push_bytes).unwrap();
  web.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({ "type": "sync.ack", "ack_id": "1", "rid": 3 }).to_string(),
  });

  // Hub fans web's update back to desktop.
  desktop.handle_event(SyncEvent::SocketFrame {
    epoch: 2,
    json: json!({
      "type": "sync.update",
      "rid": 3,
      "update": STANDARD.encode(&w_push_bytes),
    })
    .to_string(),
  });

  assert_eq!(
    desktop.doc().unwrap().to_blocks()[1].text,
    "[Desktop] Hello [Web]"
  );
  assert_eq!(
    web.doc().unwrap().to_blocks()[1].text,
    "[Desktop] Hello [Web]"
  );
  assert_eq!(hub_doc.to_blocks()[1].text, "[Desktop] Hello [Web]");
  assert_eq!(desktop.phase(), SyncPhase::Synced);
  assert_eq!(web.phase(), SyncPhase::Synced);
}

#[test]
fn headless_session_without_self_name_opts_out_of_presence_announcements() {
  let mut engine = SyncEngine::new(
    SyncConfig {
      self_name: None,
      ..Default::default()
    },
    None,
  );
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });

  let e1 = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "document.bootstrap", "connection_id": "c1" }).to_string(),
  });
  assert!(ws_frames(&e1).is_empty());

  let e2 = engine.handle_event(SyncEvent::SendPresenceCursor {
    block_id: Some("p1".into()),
    offset: Some(1),
  });
  assert!(ws_frames(&e2).is_empty());
}

#[test]
fn presence_update_and_clear_cursor_payload() {
  let mut engine = SyncEngine::new(
    SyncConfig {
      self_name: Some("Alice".into()),
      ..Default::default()
    },
    None,
  );
  engine.handle_event(SyncEvent::Connect);
  engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });

  let e_peer = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "presence.update",
      "connection_id": "conn-2",
      "user_id": "u2",
      "data": { "name": "Carol", "cursor": { "block": "p1", "offset": 2 } }
    })
    .to_string(),
  });
  assert!(
    e_peer.contains(&SyncEffect::PresenceChanged(vec![PresenceUser {
      connection_id: "conn-2".into(),
      user_id: "u2".into(),
      name: "Carol".into(),
      cursor_block_id: Some("p1".into()),
      cursor_offset: Some(2),
    }]))
  );

  // Set then clear local cursor -> cleared frame omits cursor key.
  engine.handle_event(SyncEvent::SendPresenceCursor {
    block_id: Some("p1".into()),
    offset: Some(3),
  });
  let e_clear = engine.handle_event(SyncEvent::SendPresenceCursor {
    block_id: None,
    offset: None,
  });
  assert_eq!(
    ws_frames(&e_clear),
    vec![json!({ "type": "presence.update", "payload": { "name": "Alice" } })]
  );
}

#[test]
fn sync_updates_batch_with_bad_item_persists_prefix_faults_and_halts_pagination() {
  let mut server_doc = make_doc("batch");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 1);

  let u2 = edit_doc(&mut server_doc, "+good");
  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.updates",
      "updates": [
        { "rid": 2, "update": STANDARD.encode(&u2) },
        { "rid": 3, "update": STANDARD.encode([0xFF, 0xFF, 0xFF]) },
      ]
    })
    .to_string(),
  });
  // First valid update applied and persisted; cursor stopped at 2.
  assert_eq!(engine.cursor(), 2);
  assert_eq!(engine.doc().unwrap().to_blocks()[1].text, "batch+good");
  assert!(e.contains(&SyncEffect::Fault {
    reason: "bad_remote_update".into(),
    count: 1,
  }));
  // Emitted full sync.bootstrap self-heal, NOT a sync.pull pagination loop.
  let frames = ws_frames(&e);
  assert_eq!(frames, vec![json!({ "type": "sync.bootstrap" })]);
}

#[test]
fn remote_batch_persist_failure_triggers_save_base_self_heal() {
  let fail_remote = Arc::new(AtomicBool::new(true));
  let storage = FlakyStorage::new(Arc::new(AtomicBool::new(false)), fail_remote);
  let mut server_doc = make_doc("heal");
  let mut engine = SyncEngine::new(SyncConfig::default(), Some(Box::new(storage)));
  bootstrap_engine(&mut engine, &server_doc, 1);

  let u2 = edit_doc(&mut server_doc, "+2");
  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.updates",
      "updates": [{ "rid": 2, "update": STANDARD.encode(&u2) }],
    })
    .to_string(),
  });
  assert!(e.contains(&SyncEffect::Fault {
    reason: "persist_failed".into(),
    count: 1,
  }));
  // Live doc was snapshotted as base via save_base self-heal.
  let loaded = engine.storage().unwrap().load().unwrap().unwrap();
  assert_eq!(loaded.cursor, 2);
  assert_eq!(
    MicaDoc::from_update(&loaded.state).unwrap().to_blocks()[1].text,
    "heal+2"
  );
}

#[test]
fn local_update_retry_caps_at_5_attempts() {
  let fail_outbox = Arc::new(AtomicBool::new(true));
  let storage = FlakyStorage::new(fail_outbox, Arc::new(AtomicBool::new(false)));
  let server_doc = make_doc("cap");
  let mut engine = SyncEngine::new(SyncConfig::default(), Some(Box::new(storage)));
  bootstrap_engine(&mut engine, &server_doc, 1);

  // Initial fail schedules attempt 1 (400ms).
  let d = edit_doc(engine.doc_mut().unwrap(), "!");
  let e0 = engine.handle_event(SyncEvent::ApplyLocalDiff(d));
  assert!(e0.contains(&SyncEffect::ScheduleTimer {
    kind: TimerKind::LocalUpdateRetry,
    delay_ms: 400,
    epoch: 1,
  }));

  // Ticks 1..=4 schedule 800, 1600, 3200, 6400 ms.
  let expected_next = [800u64, 1_600, 3_200, 6_400];
  let mut now = 400u64;
  for &next_delay in &expected_next {
    let e = engine.handle_event(SyncEvent::Tick {
      now_ms: now,
      timer_kind: TimerKind::LocalUpdateRetry,
      epoch: 1,
    });
    assert!(e.contains(&SyncEffect::ScheduleTimer {
      kind: TimerKind::LocalUpdateRetry,
      delay_ms: next_delay,
      epoch: 1,
    }));
    now += next_delay;
  }

  // 5th retry fails and schedules NO further timer (budget exhausted).
  let final_retry = engine.handle_event(SyncEvent::Tick {
    now_ms: now,
    timer_kind: TimerKind::LocalUpdateRetry,
    epoch: 1,
  });
  assert!(!final_retry.iter().any(|e| matches!(
    e,
    SyncEffect::ScheduleTimer {
      kind: TimerKind::LocalUpdateRetry,
      ..
    }
  )));
  assert!(!engine.outbox_empty());
}

#[test]
fn bad_base_on_rebootstrap_preserves_existing_doc_and_emits_fault() {
  let server_doc = make_doc("intact");
  let mut engine = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine, &server_doc, 5);

  let e = engine.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({
      "type": "sync.base",
      "base": "not-valid-base64!!!",
      "base_rid": 9,
    })
    .to_string(),
  });
  assert!(e.contains(&SyncEffect::Fault {
    reason: "bad_base".into(),
    count: 1,
  }));
  assert_eq!(engine.cursor(), 5);
  assert_eq!(engine.doc().unwrap().to_blocks()[1].text, "intact");
}

#[test]
fn memory_mirror_snapshot_rehydrate_simulates_web_tab_reload() {
  let mut server_doc = make_doc("tab1");
  let mut engine1 = SyncEngine::new(
    SyncConfig::default(),
    Some(Box::new(MemoryMirrorStorage::new())),
  );
  bootstrap_engine(&mut engine1, &server_doc, 1);

  let u2 = edit_doc(&mut server_doc, " + remote");
  engine1.handle_event(SyncEvent::SocketFrame {
    epoch: 1,
    json: json!({ "type": "sync.update", "rid": 2, "update": STANDARD.encode(&u2) }).to_string(),
  });
  engine1.handle_event(SyncEvent::SocketClosed {
    epoch: 1,
    reason: "close".into(),
  });
  let local_diff = edit_doc(engine1.doc_mut().unwrap(), " + local");
  engine1.handle_event(SyncEvent::ApplyLocalDiff(local_diff));

  // Simulate IDB snapshot persisted from tab 1 and hydrated into tab 2.
  let loaded = engine1.storage().unwrap().load().unwrap().unwrap();
  let unpushed = engine1.storage().unwrap().outbox_after(0).unwrap();
  let store2 = MemoryMirrorStorage::hydrate(IdbHydrateSnapshot {
    base: Some(loaded.state),
    outbox: unpushed,
    remote: vec![],
    cursor: engine1.storage().unwrap().cursor().unwrap(),
  });

  let mut engine2 = SyncEngine::new(SyncConfig::default(), Some(Box::new(store2)));
  let effects = engine2.handle_event(SyncEvent::Connect);
  assert!(effects.contains(&SyncEffect::Ready {
    root_block_id: "root".into(),
  }));
  assert_eq!(engine2.cursor(), 2);
  assert!(!engine2.outbox_empty());
  assert_eq!(
    engine2.doc().unwrap().to_blocks()[1].text,
    "tab1 + remote + local"
  );
}
