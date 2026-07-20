//! P2-M4.4 Postgres integration test for the yrs sync queries. Gated on
//! `DATABASE_URL` — skipped (passes) when unset so CI without a DB is unaffected.
//!
//!   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//!   cargo test -p mica-app-core --test sync_pg

use mica_app_core::{store, sync};
use mica_core::MicaDoc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;
    PgPool::connect(&url).await.ok()
}

/// Seed the FK chain (user → workspace → document → op snapshot) the sync layer
/// builds a yrs base from. Returns (workspace, document, user).
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
    let payload = json!({
        "schema_version": 1,
        "root_block_id": "r",
        "blocks": [
            {"id":"r","type":"page","children":["a"]},
            {"id":"a","type":"paragraph","text":"Hello"}
        ]
    });
    sqlx::query(
        "INSERT INTO document_snapshots(id,document_id,version_seq,schema_version,payload)
         VALUES($1,$2,0,1,$3)",
    )
    .bind(Uuid::new_v4())
    .bind(doc)
    .bind(&payload)
    .execute(db)
    .await
    .unwrap();
    (ws, doc, user)
}

async fn cleanup(db: &PgPool, ws: Uuid, user: Uuid) {
    // workspaces cascade to documents → snapshots, workspace_updates, yrs_base.
    sqlx::query("DELETE FROM workspaces WHERE id=$1").bind(ws).execute(db).await.ok();
    sqlx::query("DELETE FROM users WHERE id=$1").bind(user).execute(db).await.ok();
}

fn para(id: &str, text: &str) -> mica_core::Block {
    mica_core::Block {
        id: id.to_string(),
        kind: "paragraph".to_string(),
        text: text.to_string(),
        data: serde_json::Value::Null,
        children: Vec::new(),
    }
}

/// Regression for the "document is suddenly unreadable" incident: a yrs base
/// whose `meta.root` was wiped must still read.
///
/// A client that seeded from a rootless local copy pushed `meta.root = ""`.
/// Because a yrs map insert is last-writer-wins that erased the root for every
/// replica, and the root block went with it — leaving all content parentless.
/// Every read then aborted with `block not found: ` (an EMPTY id in the message
/// is this bug's signature), so a fully intact document looked like data loss.
#[tokio::test]
async fn a_base_that_lost_its_root_still_reads() {
    let Some(db) = pool().await else {
        eprintln!("skipping a_base_that_lost_its_root_still_reads: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;

    // Materialise the exact production damage: rootless base, content intact
    // but parentless, and the root block absent from the block set.
    let damaged = MicaDoc::from_blocks("", &[para("x", "First"), para("y", "Second")]);
    assert_eq!(damaged.root_block_id(), "", "fixture really is rootless");
    sqlx::query(
        "INSERT INTO document_yrs_base(document_id,state,state_vector,base_rid,updated_at)
         VALUES($1,$2,$3,1,now())
         ON CONFLICT (document_id) DO UPDATE SET
             state=excluded.state, state_vector=excluded.state_vector,
             base_rid=excluded.base_rid, updated_at=now()",
    )
    .bind(doc)
    .bind(damaged.encode_state())
    .bind(damaged.state_vector())
    .execute(&db)
    .await
    .unwrap();

    let payload = store::current_payload(&db, doc)
        .await
        .expect("read must not error on a damaged base")
        .expect("payload");

    // The op-model snapshot still knows the real root — don't adopt the empty one.
    assert_eq!(payload.root_block_id, "r");
    // ...and the vanished root block is rebuilt over the orphans, in order, so
    // the document renders instead of erroring.
    let root = payload
        .blocks
        .iter()
        .find(|b| b.id == "r")
        .expect("root block rebuilt");
    assert_eq!(root.children, vec!["x", "y"]);
    assert_eq!(payload.blocks.iter().find(|b| b.id == "x").unwrap().text, "First");

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn push_pull_bootstrap_round_trip() {
    let Some(db) = pool().await else {
        eprintln!("skipping push_pull_bootstrap_round_trip: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;

    // Base is built lazily from the op snapshot on first access; rid 0 so far.
    let base = sync::bootstrap_base(&db, doc).await.unwrap();
    assert_eq!(base.base_rid, 0);
    let client = MicaDoc::from_update(&base.state).unwrap();
    assert_eq!(
        client.to_blocks().iter().find(|b| b.id == "a").unwrap().text,
        "Hello"
    );

    // Client edits its replica and pushes the diff.
    let mut editing = MicaDoc::from_update(&base.state).unwrap();
    let sv = editing.state_vector();
    editing.text_insert("a", 5, " world");
    let update = editing.encode_diff(&sv).unwrap();
    let rid = sync::push_update(&db, ws, doc, user, &update).await.unwrap();
    assert!(rid > 0, "push assigns a positive rid");

    // A fresh bootstrap now folds the edit into the base, current to `rid`.
    let base2 = sync::bootstrap_base(&db, doc).await.unwrap();
    assert_eq!(base2.base_rid, rid);
    let fresh = MicaDoc::from_update(&base2.state).unwrap();
    assert_eq!(
        fresh.to_blocks().iter().find(|b| b.id == "a").unwrap().text,
        "Hello world"
    );

    // The stream pull returns exactly that update, and replaying it onto the
    // ORIGINAL base converges to the same document (catch-up path is sound).
    let ups = sync::pull_document_updates(&db, doc, 0, 100).await.unwrap();
    assert_eq!(ups.len(), 1);
    assert_eq!(ups[0].rid, rid);
    let mut catchup = MicaDoc::from_update(&base.state).unwrap();
    catchup.apply_update(&ups[0].payload).unwrap();
    assert_eq!(catchup.to_blocks(), fresh.to_blocks());

    // Pull after the latest rid is empty; document_head reports it.
    assert!(sync::pull_document_updates(&db, doc, rid, 100).await.unwrap().is_empty());
    assert_eq!(sync::document_head(&db, doc).await.unwrap(), rid);

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn stream_prunes_and_stale_cursor_rebootstraps() {
    let Some(db) = pool().await else {
        eprintln!("skipping stream_prunes_and_stale_cursor_rebootstraps: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;
    let base = sync::bootstrap_base(&db, doc).await.unwrap();
    let mut editing = MicaDoc::from_update(&base.state).unwrap();

    // Push enough updates that periodic pruning kicks in.
    let mut last_rid = 0;
    for _ in 0..90 {
        let sv = editing.state_vector();
        editing.text_insert("a", 5, "x");
        let update = editing.encode_diff(&sv).unwrap();
        last_rid = sync::push_update(&db, ws, doc, user, &update).await.unwrap();
    }

    // The stream is bounded — old folded updates were pruned, not all 90 kept.
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM workspace_updates WHERE document_id = $1")
            .bind(doc)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(count < 90, "stream pruned (got {count} rows for 90 pushes)");

    // A stale cursor (0) re-bootstraps from the base — the gap was pruned.
    match sync::catch_up_document(&db, doc, 0, 1000).await.unwrap() {
        sync::CatchUp::Rebootstrap(b) => {
            let d = MicaDoc::from_update(&b.state).unwrap();
            let text = d.to_blocks().into_iter().find(|x| x.id == "a").unwrap().text;
            assert!(text.starts_with("Hello") && text.len() >= 95, "base has all edits: {text}");
        }
        sync::CatchUp::Updates(_) => panic!("expected rebootstrap for a stale cursor"),
    }

    // A recent cursor catches up incrementally (no rebootstrap).
    match sync::catch_up_document(&db, doc, last_rid - 1, 1000).await.unwrap() {
        sync::CatchUp::Updates(u) => assert_eq!(u.len(), 1),
        sync::CatchUp::Rebootstrap(_) => panic!("a recent cursor should pull incrementally"),
    }

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn version_history_converges_and_names() {
    let Some(db) = pool().await else {
        eprintln!("skipping version_history_converges_and_names: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;
    let base = sync::bootstrap_base(&db, doc).await.unwrap();
    let mut editing = MicaDoc::from_update(&base.state).unwrap();

    // A burst of edits inside the 10-min window converges to exactly ONE auto
    // version (AFFiNE's min-interval), not one per push.
    for _ in 0..5 {
        let sv = editing.state_vector();
        editing.text_insert("a", 5, "x");
        let update = editing.encode_diff(&sv).unwrap();
        sync::push_update(&db, ws, doc, user, &update).await.unwrap();
    }
    let autos = store::list_yrs_versions(&db, doc).await.unwrap();
    assert_eq!(autos.len(), 1, "burst converges to one auto version");
    assert!(autos[0].label.is_none(), "auto snapshot has no label");

    // The captured blob decodes back to a real document state.
    let state = store::fetch_yrs_version_state(&db, doc, autos[0].id)
        .await
        .unwrap()
        .expect("version state present");
    let snap = MicaDoc::from_update(&state).unwrap();
    assert!(
        snap.to_blocks().iter().any(|b| b.text.contains("Hello")),
        "snapshot holds the document content"
    );

    // A manual named version lands immediately and is distinct from autos.
    let named = store::create_named_yrs_version(&db, doc, "milestone", user)
        .await
        .unwrap();
    assert_eq!(named.label.as_deref(), Some("milestone"));
    assert_eq!(
        store::list_yrs_versions(&db, doc).await.unwrap().len(),
        2,
        "named version adds a second row"
    );

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn version_retention_prunes_expired_autos_keeps_named() {
    let Some(db) = pool().await else {
        eprintln!("skipping version_retention_prunes_expired_autos_keeps_named: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;
    let base = sync::bootstrap_base(&db, doc).await.unwrap();

    // An already-expired auto row (retention should drop it) + a named row with
    // no expiry (retention must keep it forever).
    sqlx::query(
        "INSERT INTO document_yrs_versions(document_id, label, expires_at, state)
         VALUES ($1, NULL, now() - interval '1 day', $2),
                ($1, 'keep-me', NULL, $2)",
    )
    .bind(doc)
    .bind(&base.state)
    .execute(&db)
    .await
    .unwrap();

    // Push until a retention pass runs (it piggybacks the rid % 32 prune).
    let mut editing = MicaDoc::from_update(&base.state).unwrap();
    loop {
        let sv = editing.state_vector();
        editing.text_insert("a", 5, "y");
        let update = editing.encode_diff(&sv).unwrap();
        let rid = sync::push_update(&db, ws, doc, user, &update).await.unwrap();
        if rid % 32 == 0 {
            break;
        }
    }

    let expired: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM document_yrs_versions
         WHERE document_id = $1 AND expires_at IS NOT NULL AND expires_at < now()",
    )
    .bind(doc)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(expired, 0, "expired auto versions pruned");

    let named: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM document_yrs_versions WHERE document_id = $1 AND label = 'keep-me'",
    )
    .bind(doc)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(named, 1, "named version survives retention");

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn restore_reverts_document_and_converges_replicas() {
    let Some(db) = pool().await else {
        eprintln!("skipping restore_reverts_document_and_converges_replicas: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;
    let base = sync::bootstrap_base(&db, doc).await.unwrap();
    let mut editing = MicaDoc::from_update(&base.state).unwrap();

    // Edit "Hello" → "Hello there", pin a named version there.
    let sv = editing.state_vector();
    editing.text_insert("a", 5, " there");
    let up = editing.encode_diff(&sv).unwrap();
    sync::push_update(&db, ws, doc, user, &up).await.unwrap();
    let version = store::create_named_yrs_version(&db, doc, "v-there", user)
        .await
        .unwrap();

    // Edit further → "Hello there, world".
    let sv2 = editing.state_vector();
    editing.text_insert("a", 11, ", world");
    let up2 = editing.encode_diff(&sv2).unwrap();
    sync::push_update(&db, ws, doc, user, &up2).await.unwrap();
    let head = MicaDoc::from_update(&sync::bootstrap_base(&db, doc).await.unwrap().state).unwrap();
    assert_eq!(
        head.to_blocks().iter().find(|x| x.id == "a").unwrap().text,
        "Hello there, world"
    );

    // Restore to the named version.
    let vstate = store::fetch_yrs_version_state(&db, doc, version.id)
        .await
        .unwrap()
        .unwrap();
    let (rid, update) = sync::restore_yrs_version(&db, ws, doc, user, &vstate)
        .await
        .unwrap();
    assert!(rid > 0 && !update.is_empty(), "restore is a real forward update");

    // The server base reverted to the version's content...
    let reverted =
        MicaDoc::from_update(&sync::bootstrap_base(&db, doc).await.unwrap().state).unwrap();
    assert_eq!(
        reverted.to_blocks().iter().find(|x| x.id == "a").unwrap().text,
        "Hello there",
        "base reverted to the restored version"
    );

    // ...and a client still holding the post-version replica converges when it
    // applies the broadcast restore update — a new revision, not a hard reset.
    editing.apply_update(&update).unwrap();
    assert_eq!(
        editing.to_blocks().iter().find(|x| x.id == "a").unwrap().text,
        "Hello there",
        "replica converges to the restored content"
    );

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn rejects_garbage_update() {
    let Some(db) = pool().await else {
        eprintln!("skipping rejects_garbage_update: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;
    // A non-decodable update must be rejected (the §10 integrity gate) and leave
    // no row on the stream.
    let err = sync::push_update(&db, ws, doc, user, &[9, 9, 9, 9]).await;
    assert!(err.is_err(), "garbage update is rejected");
    assert_eq!(sync::document_head(&db, doc).await.unwrap(), 0, "nothing persisted");
    cleanup(&db, ws, user).await;
}

/// Regression (2026-07-19, found in prod): an op-model write on a document that
/// HAS a yrs base must land in the yrs base too — otherwise the write is
/// invisible forever.
///
/// The bug: `apply_derived_operations` derived from the op-model snapshot and
/// wrote only the op-model snapshot, while `store::current_payload` (every read,
/// export and MCP fetch) returns the YRS blocks whenever a base exists. So an
/// MCP append on any document ever opened in the editor returned ok, advanced
/// `current_seq`, grew `document_snapshots` — and could never be read back. Data
/// was acknowledged and stored where nothing looks.
///
/// Pins all three halves: the read sees it, the yrs base carries it, and the
/// change is on the stream so a live editor converges instead of rebootstrapping.
#[tokio::test]
async fn op_write_lands_in_the_yrs_base_a_reader_actually_sees() {
    let Some(db) = pool().await else {
        eprintln!("skipping op_write_lands_in_the_yrs_base_a_reader_actually_sees: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;

    // Give the document a yrs base and diverge it from the op snapshot exactly
    // the way opening it in the editor does: a real yrs edit.
    let base = sync::bootstrap_base(&db, doc).await.unwrap();
    let mut editing = MicaDoc::from_update(&base.state).unwrap();
    let sv = editing.state_vector();
    editing.text_insert("a", 5, " world");
    let edit = editing.encode_diff(&sv).unwrap();
    let head_before = sync::push_update(&db, ws, doc, user, &edit).await.unwrap();

    // An op-model write (the MCP/REST markdown path) appends a block.
    let applied = store::apply_document_operations(
        &db,
        ws,
        doc,
        user,
        &[mica_app_core::documents::DocumentOperation::InsertBlock {
            block: serde_json::from_value(json!({"id":"b","type":"paragraph","text":"appended"}))
                .unwrap(),
            parent_id: "r".to_string(),
            index: None,
        }],
    )
    .await
    .unwrap();

    // 1. The READ path sees it — this is what silently failed in prod.
    let read = store::current_payload(&db, doc).await.unwrap().unwrap();
    let appended = read
        .blocks
        .iter()
        .find(|b| b.id == "b")
        .expect("appended block must be visible to readers");
    assert_eq!(appended.text, "appended");
    // ...and the op write derived from the YRS truth, so the concurrent yrs edit
    // survives instead of being computed away from a stale baseline.
    assert_eq!(
        read.blocks.iter().find(|b| b.id == "a").unwrap().text,
        "Hello world",
        "op write must build on the yrs state, not the stale op snapshot"
    );

    // 2. The yrs base itself carries it (a fresh client bootstrap gets it).
    let base2 = sync::bootstrap_base(&db, doc).await.unwrap();
    let fresh = MicaDoc::from_update(&base2.state).unwrap();
    assert!(
        fresh.to_blocks().iter().any(|b| b.id == "b"),
        "a client bootstrapping now must receive the appended block"
    );

    // 3. It rode the stream as a yrs update, so an already-open editor converges
    //    by applying it — no rebootstrap.
    let yrs = applied.yrs.expect("op write must produce a yrs update to broadcast");
    assert!(yrs.rid > head_before, "the write takes a new stream rid");
    let ups = sync::pull_document_updates(&db, doc, head_before, 100).await.unwrap();
    assert!(ups.iter().any(|u| u.rid == yrs.rid), "update is on the stream");
    let mut live = MicaDoc::from_update(&base.state).unwrap();
    live.apply_update(&edit).unwrap();
    for u in &ups {
        live.apply_update(&u.payload).unwrap();
    }
    assert!(
        live.to_blocks().iter().any(|b| b.id == "b"),
        "a live editor applying the stream sees the appended block"
    );

    cleanup(&db, ws, user).await;
}
