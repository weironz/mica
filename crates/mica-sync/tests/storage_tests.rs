use mica_core::{Block, MicaDoc};
use mica_sync::{
  IdbHydrateSnapshot, IdbMirrorOp, LogSizes, MemoryMirrorStorage, OutboxEntry, RemoteEntry,
  StorageError, SyncCursor, SyncStorage,
};
use serde_json::json;

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

fn edit_diff(doc: &mut MicaDoc, append: &str) -> Vec<u8> {
  let sv = doc.state_vector();
  let len = doc.to_blocks()[1].text.encode_utf16().count() as u32;
  doc.text_insert("p1", len, append);
  doc.encode_diff(&sv).unwrap()
}

#[test]
fn memory_storage_empty_returns_none_and_zero_cursor() {
  let store = MemoryMirrorStorage::new();
  assert_eq!(store.load().unwrap(), None);
  assert_eq!(store.cursor().unwrap(), SyncCursor::default());
  assert_eq!(store.log_sizes().unwrap(), LogSizes { local: 0, remote: 0 });
  assert!(!store.is_broken());
}

#[test]
fn memory_storage_save_base_and_load_round_trip() {
  let mut store = MemoryMirrorStorage::new();
  let doc = make_doc("hello");
  let state = doc.encode_state();
  store.save_base(&state, 7).unwrap();

  let loaded = store.load().unwrap().expect("stored doc present");
  assert_eq!(loaded.cursor, 7);
  let decoded = MicaDoc::from_update(&loaded.state).unwrap();
  assert_eq!(decoded.to_blocks()[1].text, "hello");

  let txns = store.take_pending_mirror_txns();
  assert_eq!(
    txns,
    vec![IdbMirrorOp::SaveBase {
      state,
      cursor: SyncCursor {
        last_synced_rid: 7,
        pushed_clock: 0,
      },
    }]
  );
  assert!(store.take_pending_mirror_txns().is_empty());
}

#[test]
fn memory_storage_outbox_clock_stays_monotonic_across_trim_and_compact() {
  let mut store = MemoryMirrorStorage::new();
  let mut doc = make_doc("a");
  store.save_base(&doc.encode_state(), 1).unwrap();

  let d1 = edit_diff(&mut doc, "b");
  let d2 = edit_diff(&mut doc, "c");
  let d3 = edit_diff(&mut doc, "d");
  assert_eq!(store.append_outbox(&d1).unwrap(), 1);
  assert_eq!(store.append_outbox(&d2).unwrap(), 2);
  assert_eq!(store.append_outbox(&d3).unwrap(), 3);

  store.advance_cursor(None, Some(2)).unwrap();
  // Trim clamps to pushed_clock (2), preserving clock 3.
  store.trim_outbox_through(99).unwrap();
  let remaining = store.outbox_after(0).unwrap();
  assert_eq!(remaining.len(), 1);
  assert_eq!(remaining[0].clock, 3);

  // Advance to 3 and compact: outbox becomes empty, next clock must be 4 (> pushed_clock).
  store.advance_cursor(None, Some(3)).unwrap();
  store.compact().unwrap();
  assert_eq!(store.log_sizes().unwrap(), LogSizes { local: 0, remote: 0 });

  let d4 = edit_diff(&mut doc, "e");
  assert_eq!(store.append_outbox(&d4).unwrap(), 4);
}

#[test]
fn memory_storage_remote_batch_dedupes_sorts_and_updates_cursor() {
  let mut store = MemoryMirrorStorage::new();
  let mut doc = make_doc("base");
  store.save_base(&doc.encode_state(), 1).unwrap();
  let _ = store.take_pending_mirror_txns();

  let d3 = edit_diff(&mut doc, "-3");
  let d2 = edit_diff(&mut doc, "-2");
  store
    .append_remote_batch(&[(3, d3.clone()), (2, d2.clone()), (2, d2)])
    .unwrap();

  assert_eq!(store.cursor().unwrap().last_synced_rid, 3);
  assert_eq!(store.log_sizes().unwrap(), LogSizes { local: 0, remote: 2 });

  let txns = store.take_pending_mirror_txns();
  assert_eq!(txns.len(), 1);
  match &txns[0] {
    IdbMirrorOp::AppendRemote { rows, cursor } => {
      assert_eq!(rows.iter().map(|r| r.rid).collect::<Vec<_>>(), vec![2, 3]);
      assert_eq!(cursor.last_synced_rid, 3);
    }
    other => panic!("expected AppendRemote, got {other:?}"),
  }
}

#[test]
fn memory_storage_issue_161_snapshot_at_enqueue_remote_then_compact() {
  let mut store = MemoryMirrorStorage::new();
  let mut doc = make_doc("v0");
  store.save_base(&doc.encode_state(), 0).unwrap();
  let r1 = edit_diff(&mut doc, "+1");
  store.append_remote(1, &r1).unwrap();
  let _ = store.take_pending_mirror_txns();

  // T1: append remote rid=2, immediately followed by T2: compact().
  let r2 = edit_diff(&mut doc, "+2");
  store.append_remote_batch(&[(2, r2)]).unwrap();
  store.compact().unwrap();

  let txns = store.take_pending_mirror_txns();
  assert_eq!(txns.len(), 2);

  // T1 MUST retain both rid=1 and rid=2 (not the post-compact empty list).
  match &txns[0] {
    IdbMirrorOp::AppendRemote { rows, cursor } => {
      assert_eq!(rows.iter().map(|r| r.rid).collect::<Vec<_>>(), vec![1, 2]);
      assert_eq!(cursor.last_synced_rid, 2);
    }
    other => panic!("expected AppendRemote, got {other:?}"),
  }
  // T2 MUST carry the folded base + cleared remote log.
  match &txns[1] {
    IdbMirrorOp::Compact {
      state,
      remote,
      cursor,
      ..
    } => {
      assert!(remote.is_empty());
      assert_eq!(cursor.last_synced_rid, 2);
      let folded = MicaDoc::from_update(state).unwrap();
      assert_eq!(folded.to_blocks()[1].text, "v0+1+2");
    }
    other => panic!("expected Compact, got {other:?}"),
  }
}

#[test]
fn memory_storage_issue_161_save_base_keeps_its_own_cursor_before_advance() {
  let mut store = MemoryMirrorStorage::new();
  let doc = make_doc("s");
  store.save_base(&doc.encode_state(), 3).unwrap();
  store.advance_cursor(Some(5), None).unwrap();

  let txns = store.take_pending_mirror_txns();
  assert_eq!(txns.len(), 2);
  match &txns[0] {
    IdbMirrorOp::SaveBase { cursor, .. } => assert_eq!(cursor.last_synced_rid, 3),
    other => panic!("expected SaveBase, got {other:?}"),
  }
  match &txns[1] {
    IdbMirrorOp::AdvanceCursor { cursor } => assert_eq!(cursor.last_synced_rid, 5),
    other => panic!("expected AdvanceCursor, got {other:?}"),
  }
}

#[test]
fn memory_storage_hydrate_sorts_and_replays_all_logs() {
  let mut doc = make_doc("init");
  let base = doc.encode_state();
  let r1 = edit_diff(&mut doc, "R1");
  let r2 = edit_diff(&mut doc, "R2");
  let l1 = edit_diff(&mut doc, "L1");
  let l2 = edit_diff(&mut doc, "L2");

  // Provide deliberately unsorted logs to verify hydrate sorts them.
  let store = MemoryMirrorStorage::hydrate(IdbHydrateSnapshot {
    base: Some(base),
    outbox: vec![
      OutboxEntry { clock: 2, bytes: l2 },
      OutboxEntry { clock: 1, bytes: l1 },
    ],
    remote: vec![
      RemoteEntry { rid: 11, bytes: r2 },
      RemoteEntry { rid: 10, bytes: r1 },
    ],
    cursor: SyncCursor {
      last_synced_rid: 11,
      pushed_clock: 0,
    },
  });

  let loaded = store.load().unwrap().unwrap();
  assert_eq!(loaded.cursor, 11);
  let replayed = MicaDoc::from_update(&loaded.state).unwrap();
  assert_eq!(replayed.to_blocks()[1].text, "initR1R2L1L2");
  assert_eq!(
    store.outbox_after(0).unwrap().iter().map(|e| e.clock).collect::<Vec<_>>(),
    vec![1, 2]
  );
}

#[test]
fn memory_storage_corrupt_base_and_log_return_corrupt_error() {
  let store = MemoryMirrorStorage::hydrate(IdbHydrateSnapshot {
    base: Some(vec![0xFF, 0xFF, 0xFF]),
    ..Default::default()
  });
  assert!(matches!(store.load(), Err(StorageError::Corrupt(_))));

  let doc = make_doc("ok");
  let store_bad_remote = MemoryMirrorStorage::hydrate(IdbHydrateSnapshot {
    base: Some(doc.encode_state()),
    remote: vec![RemoteEntry {
      rid: 1,
      bytes: vec![0xFF, 0xFF],
    }],
    ..Default::default()
  });
  assert!(matches!(store_bad_remote.load(), Err(StorageError::Corrupt(_))));
}

#[test]
fn memory_storage_mark_broken_freezes_and_save_base_heals() {
  let mut store = MemoryMirrorStorage::new();
  let mut doc = make_doc("start");
  store.save_base(&doc.encode_state(), 1).unwrap();
  let d1 = edit_diff(&mut doc, "!");
  store.append_outbox(&d1).unwrap();

  // Mirror fails: freezes and drops queued txns.
  store.mark_broken();
  assert!(store.is_broken());
  assert!(store.take_pending_mirror_txns().is_empty());

  let d2 = edit_diff(&mut doc, "?");
  assert!(matches!(
    store.append_outbox(&d2),
    Err(StorageError::Unavailable(_))
  ));
  assert!(matches!(
    store.append_remote(2, &d2),
    Err(StorageError::Unavailable(_))
  ));
  assert!(matches!(
    store.advance_cursor(Some(2), None),
    Err(StorageError::Unavailable(_))
  ));
  assert!(matches!(store.compact(), Err(StorageError::Unavailable(_))));
  assert!(store.take_pending_mirror_txns().is_empty());

  // Self-heal via save_base restores healthy state and queues SaveBase.
  store.save_base(&doc.encode_state(), 2).unwrap();
  assert!(!store.is_broken());
  let txns = store.take_pending_mirror_txns();
  assert_eq!(txns.len(), 1);
  assert!(matches!(txns[0], IdbMirrorOp::SaveBase { .. }));

  // Subsequent writes succeed again.
  let d3 = edit_diff(&mut doc, "ok");
  assert!(store.append_outbox(&d3).is_ok());
}

// ── SQLite feature tests ─────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
mod sqlite_tests {
  use std::sync::{Arc, Mutex};

  use base64::engine::general_purpose::STANDARD;
  use base64::Engine as _;
  use mica_core::LocalStore;
  use mica_sync::{
    SqliteSyncStorage, StorageError, SyncConfig, SyncEffect, SyncEngine, SyncEvent, SyncPhase,
    SyncStorage,
  };
  use serde_json::{json, Value};

  use super::{edit_diff, make_doc};

  #[test]
  fn sqlite_absent_doc_returns_ok_none() {
    let local = LocalStore::open_in_memory().unwrap();
    let storage = SqliteSyncStorage::from_store(local, "https://cloud.test|u1", "doc-1", 42);
    assert_eq!(storage.load().unwrap(), None);
    assert_eq!(storage.cursor().unwrap().last_synced_rid, 0);
    assert_eq!(storage.cursor().unwrap().pushed_clock, 0);
  }

  #[test]
  fn sqlite_corrupt_snapshot_returns_storage_corrupt() {
    let local = LocalStore::open_in_memory().unwrap();
    let doc = make_doc("clean");
    local.save_doc("origin-a", "doc-corrupt", &doc).unwrap();
    let mut storage = SqliteSyncStorage::from_store(local, "origin-a", "doc-corrupt", 42);
    // Invalid yrs bytes passed to save_base -> StorageError::Corrupt.
    assert!(matches!(
      storage.save_base(&[0xFF, 0xFF, 0xFF], 1),
      Err(StorageError::Corrupt(_))
    ));
    // Corrupt outbox payload stored on disk -> load() returns StorageError::Corrupt.
    storage.append_outbox(&[0xFF, 0xFF, 0xFF]).unwrap();
    assert!(matches!(storage.load(), Err(StorageError::Corrupt(_))));
  }

  #[test]
  fn sqlite_full_append_log_compact_and_monotonic_clock() {
    let local = LocalStore::open_in_memory().unwrap();
    let mut storage = SqliteSyncStorage::from_store(local, "origin-1", "doc-1", 100);
    let mut doc = make_doc("hello");
    storage.save_base(&doc.encode_state(), 10).unwrap();

    let l1 = edit_diff(&mut doc, " A");
    let l2 = edit_diff(&mut doc, " B");
    assert_eq!(storage.append_outbox(&l1).unwrap(), 1);
    assert_eq!(storage.append_outbox(&l2).unwrap(), 2);

    let r1 = edit_diff(&mut doc, " R1");
    let r2 = edit_diff(&mut doc, " R2");
    storage
      .append_remote_batch(&[(11, r1), (12, r2)])
      .unwrap();

    assert_eq!(storage.cursor().unwrap().last_synced_rid, 12);
    assert_eq!(storage.log_sizes().unwrap().local, 2);
    assert_eq!(storage.log_sizes().unwrap().remote, 2);

    // Ack clock 1, keep clock 2 unpushed, then compact.
    storage.advance_cursor(None, Some(1)).unwrap();
    storage.compact().unwrap();

    assert_eq!(storage.log_sizes().unwrap().remote, 0);
    assert_eq!(storage.log_sizes().unwrap().local, 1);
    assert_eq!(storage.outbox_after(1).unwrap()[0].clock, 2);

    // Ack clock 2, trim, and verify clock stays monotonic (next is 3).
    storage.advance_cursor(None, Some(2)).unwrap();
    storage.trim_outbox_through(2).unwrap();
    assert_eq!(storage.log_sizes().unwrap().local, 0);
    let l3 = edit_diff(&mut doc, " C");
    assert_eq!(storage.append_outbox(&l3).unwrap(), 3);

    // Replayed load contains all edits.
    let loaded = storage.load().unwrap().unwrap();
    assert_eq!(loaded.cursor, 12);
    let replayed = mica_core::MicaDoc::from_update(&loaded.state).unwrap();
    assert_eq!(replayed.to_blocks()[1].text, "hello A B R1 R2 C");
  }

  #[test]
  fn sqlite_multi_origin_isolation_for_same_doc_id() {
    let shared = Arc::new(Mutex::new(LocalStore::open_in_memory().unwrap()));
    let mut a = SqliteSyncStorage::new(shared.clone(), "https://s1|u1", "same-doc", 1);
    let mut b = SqliteSyncStorage::new(shared, "https://s1|u2", "same-doc", 2);

    a.save_base(&make_doc("account-1").encode_state(), 5).unwrap();
    b.save_base(&make_doc("account-2").encode_state(), 9).unwrap();

    let la = a.load().unwrap().unwrap();
    let lb = b.load().unwrap().unwrap();
    assert_eq!(la.cursor, 5);
    assert_eq!(lb.cursor, 9);
    assert_eq!(
      mica_core::MicaDoc::from_update(&la.state).unwrap().to_blocks()[1].text,
      "account-1"
    );
    assert_eq!(
      mica_core::MicaDoc::from_update(&lb.state).unwrap().to_blocks()[1].text,
      "account-2"
    );
  }

  #[test]
  fn sqlite_end_to_end_engine_persists_across_restart() {
    let shared = Arc::new(Mutex::new(LocalStore::open_in_memory().unwrap()));
    let storage = SqliteSyncStorage::new(shared.clone(), "https://cloud|u1", "doc-e2e", 777);
    let mut engine = SyncEngine::new(
      SyncConfig {
        client_id: Some(777),
        ..Default::default()
      },
      Some(Box::new(storage)),
    );

    // 1. Cold bootstrap from server.
    engine.handle_event(SyncEvent::Connect);
    engine.handle_event(SyncEvent::SocketOpened { epoch: 1 });
    let server_doc = make_doc("server");
    let base_frame = json!({
      "type": "sync.base",
      "base": STANDARD.encode(server_doc.encode_state()),
      "base_rid": 5,
    })
    .to_string();
    let effects = engine.handle_event(SyncEvent::SocketFrame {
      epoch: 1,
      json: base_frame,
    });
    assert!(effects.iter().any(|e| matches!(e, SyncEffect::Ready { .. })));
    assert_eq!(engine.phase(), SyncPhase::Synced);

    // 2. Go offline and make a local edit.
    engine.handle_event(SyncEvent::SocketClosed {
      epoch: 1,
      reason: "net".into(),
    });
    let diff = {
      let d = engine.doc_mut().unwrap();
      let sv = d.state_vector();
      d.text_insert("p1", 6, " + offline");
      d.encode_diff(&sv).unwrap()
    };
    engine.handle_event(SyncEvent::ApplyLocalDiff(diff));
    assert!(!engine.outbox_empty());

    // 3. Simulate process restart with a fresh SyncEngine over the SAME SQLite store.
    let storage2 = SqliteSyncStorage::new(shared, "https://cloud|u1", "doc-e2e", 777);
    let mut engine2 = SyncEngine::new(
      SyncConfig {
        client_id: Some(777),
        ..Default::default()
      },
      Some(Box::new(storage2)),
    );
    let effects2 = engine2.handle_event(SyncEvent::Connect);
    // Seeds immediately from SQLite BEFORE socket opens!
    assert!(effects2
      .iter()
      .any(|e| matches!(e, SyncEffect::Ready { root_block_id } if root_block_id == "root")));
    assert_eq!(engine2.doc().unwrap().to_blocks()[1].text, "server + offline");
    assert_eq!(engine2.cursor(), 5);
    assert!(!engine2.outbox_empty());

    // Reconnect sends SV bootstrap, then sync.base replays outbox push clock=1.
    let opened = engine2.handle_event(SyncEvent::SocketOpened { epoch: 1 });
    let sent: Vec<Value> = opened
      .iter()
      .filter_map(|e| match e {
        SyncEffect::SendWsJson { payload, .. } => serde_json::from_str(payload).ok(),
        _ => None,
      })
      .collect();
    assert_eq!(sent[0]["type"], "sync.bootstrap");
    assert!(sent[0]["payload"]["sv"].is_string());

    let re_base = json!({
      "type": "sync.base",
      "base": STANDARD.encode(server_doc.encode_state()),
      "base_rid": 5,
    })
    .to_string();
    let after_base = engine2.handle_event(SyncEvent::SocketFrame {
      epoch: 1,
      json: re_base,
    });
    let pushes: Vec<Value> = after_base
      .iter()
      .filter_map(|e| match e {
        SyncEffect::SendWsJson { payload, .. } => serde_json::from_str::<Value>(payload).ok(),
        _ => None,
      })
      .filter(|v| v["type"] == "sync.push")
      .collect();
    assert_eq!(pushes.len(), 1);
    assert_eq!(pushes[0]["id"], "1");

    // Server acks push 1 -> synced!
    let ack = json!({ "type": "sync.ack", "ack_id": "1", "rid": 6 }).to_string();
    engine2.handle_event(SyncEvent::SocketFrame {
      epoch: 1,
      json: ack,
    });
    assert!(engine2.outbox_empty());
    assert_eq!(engine2.phase(), SyncPhase::Synced);
    assert_eq!(engine2.cursor(), 6);
  }
}
