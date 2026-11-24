use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use mica_core::{Block, MicaDoc};
use mica_sync::{
  EMPTY_YRS_UPDATE, IdbMirrorOp, MERGE_MAX_BYTES, OutboxEntry, PresenceUser, RemoteEntry,
  SyncCursor, SyncEffect, SyncEngine, SyncEvent, SyncPhase, TimerKind, derive_sync_phase,
  frame_ping, frame_presence_update, frame_sync_bootstrap, frame_sync_pull, frame_sync_push,
  is_empty_yrs_update, local_update_retry_delay_ms, merge_run_length, merge_yrs_updates,
  push_slice, reconnect_backoff_ms, repair_sv_for,
};
use serde_json::{Value, json};

fn sample_doc() -> MicaDoc {
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
        text: "hello".into(),
        data: json!({}),
        children: vec![],
      },
    ],
  )
}

#[test]
fn sync_engine_is_send() {
  fn assert_send<T: Send>() {}
  assert_send::<SyncEngine>();
}

#[test]
fn derive_sync_phase_all_combinations() {
  assert_eq!(derive_sync_phase(false, false), SyncPhase::Offline);
  assert_eq!(derive_sync_phase(false, true), SyncPhase::Offline);
  assert_eq!(derive_sync_phase(true, true), SyncPhase::Syncing);
  assert_eq!(derive_sync_phase(true, false), SyncPhase::Synced);
}

#[test]
fn push_slice_respects_window_and_frees_rejected_slots() {
  #[derive(Debug, PartialEq, Eq)]
  struct Entry {
    id: usize,
    sent: bool,
    rejected: bool,
  }
  let queue = vec![
    Entry {
      id: 1,
      sent: true,
      rejected: false,
    },
    Entry {
      id: 2,
      sent: true,
      rejected: true,
    }, // rejected: does not occupy slot
    Entry {
      id: 3,
      sent: false,
      rejected: false,
    },
    Entry {
      id: 4,
      sent: false,
      rejected: false,
    },
    Entry {
      id: 5,
      sent: false,
      rejected: false,
    },
  ];
  let slice = push_slice(&queue, |e| e.sent, |e| e.sent && !e.rejected, 3);
  let ids: Vec<usize> = slice.into_iter().map(|e| e.id).collect();
  // 1 occupies 1 slot, 2 occupies 0 slots -> 2 slots available -> picks 3 and 4.
  assert_eq!(ids, vec![3, 4]);
}

#[test]
fn merge_run_length_handles_empty_cap_and_oversized_first() {
  assert_eq!(merge_run_length(&[], MERGE_MAX_BYTES), 0);
  assert_eq!(merge_run_length(&[100, 200, 300], 1000), 3);
  assert_eq!(merge_run_length(&[400, 400, 400], 1000), 2);
  // Single entry over cap still returns 1 so the outbox never stalls.
  assert_eq!(merge_run_length(&[5_000_000, 100], MERGE_MAX_BYTES), 1);
  // Exact boundary fits.
  assert_eq!(merge_run_length(&[512 * 1024; 10], MERGE_MAX_BYTES), 8);
}

#[test]
fn repair_sv_for_matches_b11_rules() {
  let ack_missing = json!({ "type": "sync.ack", "missing": true, "sv": "AA==" });
  assert_eq!(repair_sv_for(&ack_missing, false), Some("AA=="));
  assert_eq!(repair_sv_for(&ack_missing, true), Some("AA=="));

  let ack_ok = json!({ "type": "sync.ack", "missing": false, "sv": "AA==" });
  assert_eq!(repair_sv_for(&ack_ok, true), None);

  let base_ahead = json!({ "type": "sync.base", "ahead": true, "sv": "AQ==" });
  assert_eq!(repair_sv_for(&base_ahead, true), Some("AQ=="));
  assert_eq!(repair_sv_for(&base_ahead, false), None);

  let base_no_sv = json!({ "type": "sync.base", "ahead": true });
  assert_eq!(repair_sv_for(&base_no_sv, true), None);

  let other = json!({ "type": "sync.update", "missing": true, "ahead": true, "sv": "AA==" });
  assert_eq!(repair_sv_for(&other, true), None);
}

#[test]
fn reconnect_backoff_six_tiers_and_cap() {
  assert_eq!(reconnect_backoff_ms(0), 500);
  assert_eq!(reconnect_backoff_ms(1), 1_000);
  assert_eq!(reconnect_backoff_ms(2), 2_000);
  assert_eq!(reconnect_backoff_ms(3), 4_000);
  assert_eq!(reconnect_backoff_ms(4), 8_000);
  assert_eq!(reconnect_backoff_ms(5), 16_000);
  assert_eq!(reconnect_backoff_ms(6), 30_000);
  assert_eq!(reconnect_backoff_ms(20), 30_000);
}

#[test]
fn local_update_retry_delays_match_pending_doc_update() {
  assert_eq!(local_update_retry_delay_ms(0), 400);
  assert_eq!(local_update_retry_delay_ms(1), 800);
  assert_eq!(local_update_retry_delay_ms(2), 1_600);
  assert_eq!(local_update_retry_delay_ms(3), 3_200);
  assert_eq!(local_update_retry_delay_ms(4), 6_400);
}

#[test]
fn is_empty_yrs_update_detects_empty_and_canonical_zero_update() {
  assert!(is_empty_yrs_update(&[]));
  assert!(is_empty_yrs_update(&EMPTY_YRS_UPDATE));
  assert!(!is_empty_yrs_update(&[1, 0]));
  assert!(!is_empty_yrs_update(&[0, 0, 1]));
}

#[test]
fn merge_yrs_updates_folds_diffs_and_rejects_corrupt() {
  assert_eq!(merge_yrs_updates(&[]).unwrap(), EMPTY_YRS_UPDATE.to_vec());

  let mut doc = sample_doc();
  let base = doc.encode_state();
  let mut diffs = Vec::new();
  for ch in [" ", "w", "o", "r", "l", "d"] {
    let sv = doc.state_vector();
    let len = doc.to_blocks()[1].text.encode_utf16().count() as u32;
    doc.text_insert("p1", len, ch);
    diffs.push(doc.encode_diff(&sv).unwrap());
  }
  let slices: Vec<&[u8]> = diffs.iter().map(Vec::as_slice).collect();
  let merged = merge_yrs_updates(&slices).expect("valid yrs diffs merge");

  let mut peer = MicaDoc::from_update(&base).unwrap();
  peer.apply_update(&merged).unwrap();
  assert_eq!(peer.to_blocks()[1].text, "hello world");

  // Corrupt update inside batch returns Err.
  let bad = vec![0xFFu8, 0xFF, 0xFF];
  let err = merge_yrs_updates(&[diffs[0].as_slice(), bad.as_slice()]);
  assert!(err.is_err());
}

#[test]
fn outbound_frame_builders_match_wire_schema() {
  let boot_cold: Value = serde_json::from_str(&frame_sync_bootstrap(None)).unwrap();
  assert_eq!(boot_cold, json!({ "type": "sync.bootstrap" }));

  let boot_sv: Value = serde_json::from_str(&frame_sync_bootstrap(Some(&[1, 2]))).unwrap();
  assert_eq!(
    boot_sv,
    json!({ "type": "sync.bootstrap", "payload": { "sv": STANDARD.encode([1, 2]) } })
  );

  let pull_no_sv: Value = serde_json::from_str(&frame_sync_pull(42, None)).unwrap();
  assert_eq!(
    pull_no_sv,
    json!({ "type": "sync.pull", "payload": { "since_rid": 42 } })
  );

  let pull_sv: Value = serde_json::from_str(&frame_sync_pull(42, Some(&[3]))).unwrap();
  assert_eq!(
    pull_sv,
    json!({ "type": "sync.pull", "payload": { "since_rid": 42, "sv": STANDARD.encode([3]) } })
  );

  let push: Value = serde_json::from_str(&frame_sync_push("7", &[9, 8])).unwrap();
  assert_eq!(
    push,
    json!({ "type": "sync.push", "id": "7", "payload": { "update": STANDARD.encode([9, 8]) } })
  );

  let ping: Value = serde_json::from_str(&frame_ping()).unwrap();
  assert_eq!(ping, json!({ "type": "ping" }));

  let pres_plain: Value = serde_json::from_str(&frame_presence_update("Alice", None)).unwrap();
  assert_eq!(
    pres_plain,
    json!({ "type": "presence.update", "payload": { "name": "Alice" } })
  );

  let pres_cur: Value =
    serde_json::from_str(&frame_presence_update("Alice", Some(("b1", 5)))).unwrap();
  assert_eq!(
    pres_cur,
    json!({
      "type": "presence.update",
      "payload": { "name": "Alice", "cursor": { "block": "b1", "offset": 5 } }
    })
  );
}

#[test]
fn sync_events_and_effects_serde_round_trip() {
  let events = vec![
    SyncEvent::Connect,
    SyncEvent::SocketOpened { epoch: 1 },
    SyncEvent::SocketClosed {
      epoch: 1,
      reason: "eof".into(),
    },
    SyncEvent::SocketFrame {
      epoch: 1,
      json: r#"{"type":"pong"}"#.into(),
    },
    SyncEvent::Tick {
      now_ms: 8000,
      timer_kind: TimerKind::Heartbeat,
      epoch: 1,
    },
    SyncEvent::ApplyLocalDiff(vec![1, 2, 3]),
    SyncEvent::SendPresenceCursor {
      block_id: Some("b1".into()),
      offset: Some(4),
    },
    SyncEvent::FlushForExit,
    SyncEvent::MirrorWriteFailed,
  ];
  for ev in events {
    let encoded = serde_json::to_string(&ev).unwrap();
    let decoded: SyncEvent = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, ev);
  }

  let effects = vec![
    SyncEffect::OpenSocket { epoch: 2 },
    SyncEffect::CloseSocket { epoch: 1 },
    SyncEffect::SendWsJson {
      epoch: 2,
      payload: r#"{"type":"ping"}"#.into(),
    },
    SyncEffect::ScheduleTimer {
      kind: TimerKind::Reconnect,
      delay_ms: 500,
      epoch: 2,
    },
    SyncEffect::CancelTimer {
      kind: TimerKind::Heartbeat,
    },
    SyncEffect::MirrorIdb(vec![
      IdbMirrorOp::SaveBase {
        state: vec![0, 0],
        cursor: SyncCursor {
          last_synced_rid: 3,
          pushed_clock: 1,
        },
      },
      IdbMirrorOp::AppendOutbox {
        rows: vec![OutboxEntry {
          clock: 2,
          bytes: vec![4, 5],
        }],
      },
      IdbMirrorOp::AppendRemote {
        rows: vec![RemoteEntry {
          rid: 4,
          bytes: vec![6],
        }],
        cursor: SyncCursor {
          last_synced_rid: 4,
          pushed_clock: 1,
        },
      },
      IdbMirrorOp::AdvanceCursor {
        cursor: SyncCursor {
          last_synced_rid: 4,
          pushed_clock: 2,
        },
      },
      IdbMirrorOp::Compact {
        state: vec![0, 0],
        outbox: vec![],
        remote: vec![],
        cursor: SyncCursor {
          last_synced_rid: 4,
          pushed_clock: 2,
        },
      },
    ]),
    SyncEffect::Ready {
      root_block_id: "root".into(),
    },
    SyncEffect::RemoteBlocksChanged {
      changed_ids: Some(vec!["p1".into()]),
    },
    SyncEffect::SyncPhaseChanged(SyncPhase::Syncing),
    SyncEffect::ServerConnected,
    SyncEffect::PresenceChanged(vec![PresenceUser {
      connection_id: "c1".into(),
      user_id: "u1".into(),
      name: "Bob".into(),
      cursor_block_id: Some("p1".into()),
      cursor_offset: Some(3),
    }]),
    SyncEffect::AcceptedSeq(99),
    SyncEffect::Fault {
      reason: "bad_base".into(),
      count: 1,
    },
  ];
  for eff in effects {
    let encoded = serde_json::to_string(&eff).unwrap();
    let decoded: SyncEffect = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, eff);
  }
}
