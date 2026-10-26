//! Server-transactions regression tests (#138 #139) — real Postgres:
//!
//!   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//!   cargo test -p mica-app-core --test server_txns_pg
//!
//! Red evidence first ran as the audit probes in
//! `mica-global-audit-20261025/server-transactions` against an isolated
//! database:
//!
//! - #138: outline's two-statement read handed back "old payload = a: original,
//!   new seq = 1" after a concurrent committed write; `apply_derived_operations`
//!   then ACCEPTED `expected_seq=1` and the concurrent edit was lost.
//! - #139-A: 3 REST writes grew the seeded stream 6 -> 9 rows, and the expired
//!   auto version survived — retention only lived on the WebSocket path.
//! - #139-B: with a dummy document draining every even global rid, document C's
//!   own stream grew 7 -> 8 -> 9 -> 10 rows and never pruned (rid-modulo
//!   cadence, per-document starvation).

use mica_app_core::documents::{DocumentOperation, DocumentSnapshotPayload};
use mica_app_core::store;
use mica_app_core::sync;
use mica_core::MicaDoc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Same hardening as sync_pg.rs: skip without a database locally, but a
/// set-but-unusable DATABASE_URL — or a missing one in CI — is a hard failure,
/// never a silent pass.
async fn pool() -> Option<PgPool> {
  let Ok(url) = std::env::var("DATABASE_URL") else {
    assert!(
      std::env::var("CI").is_err(),
      "DATABASE_URL is unset in CI — the postgres service block regressed; \
             these tests must not silently pass"
    );
    return None;
  };
  Some(
    PgPool::connect(&url)
      .await
      .expect("DATABASE_URL is set but the connection failed"),
  )
}

/// Small, injectable cadence so retention fires within a few writes (the audit
/// probes used the same shape: mechanisms identical to the shipped 32/64).
fn tuning() -> sync::SyncTuning {
  sync::SyncTuning {
    catch_up_limit: 1000,
    keep_margin: 1,
    prune_every: 2,
  }
}

/// Seed the FK chain (user -> workspace -> document -> yrs base) with a
/// one-line document: root `r` holding paragraph `a` = "original".
async fn seed_doc(db: &PgPool) -> (Uuid, Uuid, Uuid) {
  let user = Uuid::new_v4();
  let ws = Uuid::new_v4();
  let doc = Uuid::new_v4();
  sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
    .bind(user)
    .bind(format!("{user}@t.dev"))
    .execute(db)
    .await
    .unwrap();
  sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'W',$2)")
    .bind(ws)
    .bind(user)
    .execute(db)
    .await
    .unwrap();
  sqlx::query(
    "INSERT INTO documents(id,workspace_id,root_block_id,current_seq,created_by)
         VALUES($1,$2,'r',0,$3)",
  )
  .bind(doc)
  .bind(ws)
  .bind(user)
  .execute(db)
  .await
  .unwrap();
  let payload: mica_markdown::DocumentSnapshotPayload = serde_json::from_value(json!({
      "schema_version": 1,
      "root_block_id": "r",
      "blocks": [
          {"id":"r","type":"page","children":["a"]},
          {"id":"a","type":"paragraph","text":"original"}
      ]
  }))
  .unwrap();
  let mut tx = db.begin().await.unwrap();
  sync::seed_base_tx(&mut tx, doc, payload).await.unwrap();
  tx.commit().await.unwrap();
  (ws, doc, user)
}

async fn cleanup(db: &PgPool, ws: Uuid, user: Uuid) {
  // workspaces cascade to documents -> workspace_updates, yrs_base, versions.
  sqlx::query("DELETE FROM workspaces WHERE id=$1")
    .bind(ws)
    .execute(db)
    .await
    .ok();
  sqlx::query("DELETE FROM users WHERE id=$1")
    .bind(user)
    .execute(db)
    .await
    .ok();
}

fn block_text(payload: &DocumentSnapshotPayload, id: &str) -> String {
  payload
    .blocks
    .iter()
    .find(|b| b.id == id)
    .map(|b| b.text.clone())
    .unwrap_or_else(|| panic!("block {id} must exist"))
}

/// Push one keystroke through the WebSocket write path; `editing` is the
/// caller's replica, so successive pushes build on each other.
async fn push_once(
  db: &PgPool,
  ws: Uuid,
  doc: Uuid,
  user: Uuid,
  editing: &mut MicaDoc,
  text: &str,
) -> i64 {
  let sv = editing.state_vector();
  editing.text_insert("a", 5, text);
  let update = editing.encode_diff(&sv).unwrap();
  sync::push_update_checked(db, ws, doc, user, &update, &tuning())
    .await
    .unwrap()
    .rid
}

async fn stream_rows(db: &PgPool, doc: Uuid) -> i64 {
  sqlx::query_scalar("SELECT count(*) FROM workspace_updates WHERE document_id = $1")
    .bind(doc)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn expired_versions(db: &PgPool, doc: Uuid) -> i64 {
  sqlx::query_scalar(
    "SELECT count(*) FROM document_yrs_versions
         WHERE document_id = $1 AND expires_at IS NOT NULL AND expires_at < now()",
  )
  .bind(doc)
  .fetch_one(db)
  .await
  .unwrap()
}

/// Seed an out-of-window stream plus one expired and one named auto version —
/// the state the audit probes established before writing ("6 old stream rows,
/// the retention pass owes a prune").
async fn seed_retention_backlog(db: &PgPool, ws: Uuid, doc: Uuid, user: Uuid) {
  let state = sync::bootstrap_base(db, doc).await.unwrap().state;
  for _ in 0..6 {
    sqlx::query("INSERT INTO workspace_updates(workspace_id,document_id,actor_id,payload) VALUES($1,$2,$3,'\\x')")
      .bind(ws)
      .bind(doc)
      .bind(user)
      .execute(db)
      .await
      .unwrap();
  }
  sqlx::query(
    "INSERT INTO document_yrs_versions(document_id, label, expires_at, state)
         VALUES ($1, NULL, now() - interval '1 day', $2),
                ($1, 'keep-me', NULL, $2)",
  )
  .bind(doc)
  .bind(&state)
  .execute(db)
  .await
  .unwrap();
}

/// #138: `document_outline` hands its caller a (blocks, seq) pair taken from
/// ONE snapshot; a write pinned to that seq must 409 when the document moved
/// meanwhile — in particular when a concurrent write changed the very block the
/// caller anchors on. Before the pairing fix the outline was read as two
/// statements ("old blocks + new seq"), the CAS passed, and the concurrent edit
/// was deleted by a caller that never saw it: anchor EXISTENCE was checked, and
/// existence is not a concurrency guard.
#[tokio::test]
async fn anchor_delete_pinned_to_outline_seq_conflicts_after_concurrent_edit() {
  let Some(db) = pool().await else { return };
  let (ws, doc, user) = seed_doc(&db).await;

  // The outline read: payload and seq from one statement, one snapshot.
  let (outline, seq) = store::current_payload_with_seq(&db, doc)
    .await
    .unwrap()
    .expect("seeded document has a payload");
  assert_eq!(block_text(&outline, "a"), "original");
  assert_eq!(seq, 0, "the seq describes the content just read");

  // A concurrent acknowledged edit lands on the SAME block the caller will
  // anchor its delete on (this is the "metadata eviction" shape from #138:
  // the outline caller never saw this write).
  store::apply_document_operations(
    &db,
    ws,
    doc,
    user,
    &[DocumentOperation::UpdateBlock {
      block_id: "a".into(),
      kind: None,
      text: Some("CONCURRENT EDIT".into()),
      data: None,
    }],
    &tuning(),
  )
  .await
  .unwrap();

  // Delete-by-anchor pinned to the outline's seq: the anchor still EXISTS (the
  // derive closure runs against locked state that holds it), but the document
  // moved since the snapshot the caller planned against — only the seq pair
  // from one snapshot makes this a conflict instead of silent data loss.
  let stale_delete =
    store::apply_derived_operations(&db, ws, doc, user, Some(seq), &tuning(), |locked| {
      assert!(
        locked.blocks.iter().any(|b| b.id == "a"),
        "the anchor exists in locked state — existence is not the guard"
      );
      Ok(vec![DocumentOperation::DeleteBlock {
        block_id: "a".into(),
      }])
    })
    .await;
  assert!(
    matches!(stale_delete, Err(mica_infra::ApiError::Conflict(_))),
    "a delete pinned to a pre-edit snapshot must 409, got {stale_delete:?}"
  );

  // The concurrent edit survived the refused delete.
  let after = store::current_payload(&db, doc)
    .await
    .unwrap()
    .expect("payload still readable");
  assert_eq!(block_text(&after, "a"), "CONCURRENT EDIT");

  cleanup(&db, ws, user).await;
}

/// #139-A: the REST/MCP write path runs the SAME retention pass as the
/// WebSocket path — old stream rows and expired auto versions are pruned on the
/// document's own write cadence; before the shared helper, only the WS path
/// pruned and a purely REST-maintained document grew `workspace_updates`
/// without bound.
#[tokio::test]
async fn rest_writes_run_stream_retention_like_the_ws_path() {
  let Some(db) = pool().await else { return };
  let (ws, doc, user) = seed_doc(&db).await;
  seed_retention_backlog(&db, ws, doc, user).await;

  for i in 0..3 {
    store::apply_document_operations(
      &db,
      ws,
      doc,
      user,
      &[DocumentOperation::UpdateBlock {
        block_id: "a".into(),
        kind: None,
        text: Some(format!("rest-{i}")),
        data: None,
      }],
      &tuning(),
    )
    .await
    .unwrap();
  }

  // Threshold keep_margin + prune_every = 3: the first REST write prunes the 7
  // retained rows down to the keep window (1 row), the third prunes again.
  assert_eq!(
    stream_rows(&db, doc).await,
    1,
    "REST writes prune the stream on the document's own cadence"
  );
  assert_eq!(
    expired_versions(&db, doc).await,
    0,
    "expired auto versions pruned"
  );
  let named: i64 = sqlx::query_scalar(
    "SELECT count(*) FROM document_yrs_versions WHERE document_id = $1 AND label = 'keep-me'",
  )
  .bind(doc)
  .fetch_one(&db)
  .await
  .unwrap();
  assert_eq!(named, 1, "named version survives retention");

  // Migration 0035's trigger must still record the deletion watermark on this
  // path (#139 CRITICAL): a cursor below the pruned window has to re-bootstrap,
  // never silently skip the updates the prune removed.
  let pruned_rid: i64 =
    sqlx::query_scalar("SELECT stream_pruned_rid FROM document_yrs_base WHERE document_id = $1")
      .bind(doc)
      .fetch_one(&db)
      .await
      .unwrap();
  assert!(
    pruned_rid > 0,
    "REST-path pruning advanced the catch-up watermark"
  );
  assert!(
    matches!(
      sync::catch_up_document(&db, doc, 0, 1000).await.unwrap(),
      sync::CatchUp::Rebootstrap(_)
    ),
    "a cursor behind the pruned window re-bootstraps from the base"
  );

  cleanup(&db, ws, user).await;
}

/// #139-B: the prune cadence counts THIS document's stream rows, not the
/// table-wide rid modulo. A dummy document's pushes interleave so target C
/// draws non-cadence global rids — under the old rule C never pruned
/// (cross-document starvation); now C prunes on its own write count.
#[tokio::test]
async fn prune_cadence_is_per_document_despite_interleaved_rids() {
  let Some(db) = pool().await else { return };
  let (ws, doc_c, user) = seed_doc(&db).await;
  seed_retention_backlog(&db, ws, doc_c, user).await;
  let (ws_d, doc_d, user_d) = seed_doc(&db).await;

  let base_c = sync::bootstrap_base(&db, doc_c).await.unwrap();
  let base_d = sync::bootstrap_base(&db, doc_d).await.unwrap();
  let mut editing_c = MicaDoc::from_update(&base_c.state).unwrap();
  let mut editing_d = MicaDoc::from_update(&base_d.state).unwrap();

  for i in 0..4 {
    // The dummy write lands FIRST each round, so C draws whatever parity the
    // global sequence hands it — the old modulo rule's starvation shape.
    push_once(&db, ws_d, doc_d, user_d, &mut editing_d, &format!("d{i}")).await;
    push_once(&db, ws, doc_c, user, &mut editing_c, &format!("c{i}")).await;
  }

  // C's OWN cadence fired (threshold 3 rows): 7 -> 1, then 2 -> 3 -> 1.
  let c_rows = stream_rows(&db, doc_c).await;
  assert!(
    c_rows <= 2,
    "document C pruned on its own cadence, rows = {c_rows}"
  );
  assert_eq!(
    expired_versions(&db, doc_c).await,
    0,
    "C's expired auto versions pruned"
  );
  // The dummy prunes on its own cadence as well — retention is symmetric.
  assert!(
    stream_rows(&db, doc_d).await <= 2,
    "dummy document also pruned on its own cadence"
  );

  cleanup(&db, ws_d, user_d).await;
  cleanup(&db, ws, user).await;
}

/// #139 review follow-up: expired auto versions must be pruned on EVERY write,
/// not only when the document's stream reaches the prune cadence. A document
/// below the stream gate (here: 1-2 retained rows, threshold 3) is the common
/// case for most pages — gating version expiry on stream growth meant its
/// expired rows were never deleted, and `list_yrs_versions` does not filter by
/// TTL, so the history panel kept them forever. Both write paths must clean
/// them regardless of stream size; the stream gate itself is unchanged.
#[tokio::test]
async fn expired_auto_versions_prune_below_the_stream_gate_on_both_paths() {
  let Some(db) = pool().await else { return };
  let (ws, doc, user) = seed_doc(&db).await;
  let state = sync::bootstrap_base(&db, doc).await.unwrap().state;
  // One stale stream row — far below the keep_margin + prune_every gate — plus
  // one expired auto version, one unexpired auto version, and one named row.
  sqlx::query("INSERT INTO workspace_updates(workspace_id,document_id,actor_id,payload) VALUES($1,$2,$3,'\\x')")
    .bind(ws)
    .bind(doc)
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  sqlx::query(
    "INSERT INTO document_yrs_versions(document_id, label, expires_at, state)
         VALUES ($1, NULL, now() - interval '1 day', $2),
                ($1, NULL, now() + interval '1 day', $2),
                ($1, 'keep-me', NULL, $2)",
  )
  .bind(doc)
  .bind(&state)
  .execute(&db)
  .await
  .unwrap();

  // A REST write on a stream that stays below the gate must still drop the
  // EXPIRED auto row — and only that row.
  store::apply_document_operations(
    &db,
    ws,
    doc,
    user,
    &[DocumentOperation::UpdateBlock {
      block_id: "a".into(),
      kind: None,
      text: Some("rest-write".into()),
      data: None,
    }],
    &tuning(),
  )
  .await
  .unwrap();
  assert_eq!(
    expired_versions(&db, doc).await,
    0,
    "REST write prunes expired auto versions below the stream gate"
  );
  let versions: i64 =
    sqlx::query_scalar("SELECT count(*) FROM document_yrs_versions WHERE document_id = $1")
      .bind(doc)
      .fetch_one(&db)
      .await
      .unwrap();
  assert_eq!(
    versions, 2,
    "the unexpired auto row and the named row both survive"
  );

  // Same property on the WebSocket path, on a fresh document that never
  // reaches its stream gate either.
  let (ws2, doc2, user2) = seed_doc(&db).await;
  let state2 = sync::bootstrap_base(&db, doc2).await.unwrap().state;
  sqlx::query(
    "INSERT INTO document_yrs_versions(document_id, label, expires_at, state)
         VALUES ($1, NULL, now() - interval '1 day', $2)",
  )
  .bind(doc2)
  .bind(&state2)
  .execute(&db)
  .await
  .unwrap();
  let base2 = sync::bootstrap_base(&db, doc2).await.unwrap();
  let mut editing = MicaDoc::from_update(&base2.state).unwrap();
  push_once(&db, ws2, doc2, user2, &mut editing, "x").await;
  assert_eq!(
    expired_versions(&db, doc2).await,
    0,
    "a WS push prunes expired auto versions below the stream gate"
  );

  cleanup(&db, ws, user).await;
  cleanup(&db, ws2, user2).await;
}
