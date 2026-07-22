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

/// Skipping without a database is a local convenience; skipping WITH one is a
/// lie. This used to be `PgPool::connect(&url).await.ok()?`, which turned a
/// failed connection into a skip: you could export DATABASE_URL, watch
/// `8 passed`, and never have touched Postgres. docs/lessons.md:84 recorded
/// exactly that ("测试真空通过") after it hid a red test — and the code stayed.
///
/// Now a set-but-unusable DATABASE_URL panics, and in CI a MISSING one panics
/// too, because there the database is always provisioned: its absence means the
/// workflow regressed, not that these assertions may quietly stop running.
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

    // "Push N times" cannot guarantee a prune: the cadence check is
    // `rid % STREAM_PRUNE_EVERY == 0` on the pushing row's rid, and rid comes
    // from the TABLE-wide sequence — a concurrently running test that draws
    // the cadence rid prunes ITS document, not ours (2026-07-21 CI: 90 rows
    // for 90 pushes, zero prunes). So push until one of OUR pushes draws a
    // cadence rid far enough past our first rid that the keep-margin cutoff
    // reaches our earliest row — from there the prune is deterministic.
    let mut first_rid = 0i64;
    let last_rid: i64;
    let mut pushes = 0i64;
    loop {
        let sv = editing.state_vector();
        editing.text_insert("a", 5, "x");
        let update = editing.encode_diff(&sv).unwrap();
        let rid = sync::push_update(&db, ws, doc, user, &update).await.unwrap();
        pushes += 1;
        if first_rid == 0 {
            first_rid = rid;
        }
        if rid % sync::STREAM_PRUNE_EVERY == 0 && rid - first_rid > sync::STREAM_KEEP_MARGIN {
            last_rid = rid;
            break;
        }
        assert!(
            pushes < 500,
            "no cadence rid drawn in {pushes} pushes — contention beyond plausible"
        );
    }

    // That last push pruned this doc's rows at rid <= last_rid - margin, and
    // first_rid sits below that cutoff by construction — so rows went away.
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM workspace_updates WHERE document_id = $1")
            .bind(doc)
            .fetch_one(&db)
            .await
            .unwrap();
    assert!(count < pushes, "stream pruned (got {count} rows for {pushes} pushes)");

    // A stale cursor (0) re-bootstraps from the base — the gap was pruned.
    // Every push folds synchronously, so the base holds ALL edits: "Hello"
    // (5) + one 'x' per push, exactly.
    match sync::catch_up_document(&db, doc, 0, 1000).await.unwrap() {
        sync::CatchUp::Rebootstrap(b) => {
            let d = MicaDoc::from_update(&b.state).unwrap();
            let text = d.to_blocks().into_iter().find(|x| x.id == "a").unwrap().text;
            assert!(text.starts_with("Hello"), "base has all edits: {text}");
            assert_eq!(text.len() as i64, 5 + pushes, "base has all edits: {text}");
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

/// An op-model (MCP/REST) write must capture an AUTO version snapshot, on the
/// same cadence as the collaborative sync path — otherwise a document only ever
/// maintained over MCP/REST accrues no user-visible version history at all.
///
/// Pins both halves: the first op write creates exactly one auto version whose
/// blob decodes to the written content, and a rapid second write inside the
/// 10-minute window does NOT add another (the cadence guard, so a burst of MCP
/// edits converges to one version rather than flooding the table).
#[tokio::test]
async fn op_write_captures_an_auto_version_on_cadence() {
    let Some(db) = pool().await else {
        eprintln!("skipping op_write_captures_an_auto_version_on_cadence: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;

    // No version history before the first write (seed_doc only lays the op
    // snapshot; ensure_base folds it lazily on the first op write).
    assert!(
        store::list_yrs_versions(&db, doc).await.unwrap().is_empty(),
        "a freshly seeded document has no versions yet"
    );

    async fn insert(db: &PgPool, ws: Uuid, doc: Uuid, user: Uuid, id: &str, text: &str) {
        store::apply_document_operations(
            db,
            ws,
            doc,
            user,
            &[mica_app_core::documents::DocumentOperation::InsertBlock {
                block: serde_json::from_value(json!({"id": id, "type": "paragraph", "text": text}))
                    .unwrap(),
                parent_id: "r".to_string(),
                index: None,
            }],
        )
        .await
        .unwrap();
    }

    insert(&db, ws, doc, user, "b", "first op write").await;
    let autos = store::list_yrs_versions(&db, doc).await.unwrap();
    assert_eq!(autos.len(), 1, "the first op write captures one auto version");
    assert!(autos[0].label.is_none(), "an op-path auto snapshot has no label");

    // The blob is a real decodable document state carrying the write.
    let state = store::fetch_yrs_version_state(&db, doc, autos[0].id)
        .await
        .unwrap()
        .expect("version state present");
    let snap = MicaDoc::from_update(&state).unwrap();
    assert!(
        snap.to_blocks().iter().any(|b| b.id == "b"),
        "the auto version holds the op-written block"
    );

    // A second op write moments later stays inside the cadence window — still
    // exactly one auto version, not two.
    insert(&db, ws, doc, user, "c", "second op write").await;
    assert_eq!(
        store::list_yrs_versions(&db, doc).await.unwrap().len(),
        1,
        "a second write inside the 10-min window does not add an auto version"
    );

    cleanup(&db, ws, user).await;
}

/// Regression: two connections bootstrapping a FRESH document concurrently must
/// receive the SAME base. `ensure_base_tx` used to return its locally-built
/// state even when its INSERT lost the `ON CONFLICT DO NOTHING` race — and
/// `MicaDoc::from_blocks` mints a random yrs actor per call, so the loser's
/// client lived in a parallel CRDT universe: every peer edit referenced the
/// stored base's structs and parked as PENDING forever (applyUpdate still
/// returned Ok — permanent silent divergence, red line #1). Found via the flaky
/// `cloud_sync_test` (B never saw A's paragraph on a fresh doc).
#[tokio::test]
async fn concurrent_first_bootstrap_returns_one_universe() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;

    // The winner: an open transaction that has inserted its base but not yet
    // committed — exactly what a concurrent bootstrapper races against.
    let winner_doc = MicaDoc::from_blocks("r", &[para("r", ""), para("a", "Hello")]);
    let winner_state = winner_doc.encode_state();
    let mut tx = db.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO document_yrs_base(document_id,state,state_vector,base_rid,updated_at)
         VALUES ($1,$2,$3,0,now())",
    )
    .bind(doc)
    .bind(&winner_state)
    .bind(winner_doc.state_vector())
    .execute(&mut *tx)
    .await
    .unwrap();

    // The loser: bootstraps while the winner is uncommitted. It reads "no base"
    // (READ COMMITTED), builds its own twin, and its INSERT then blocks on the
    // winner's unique-index lock until the commit below releases it.
    let db2 = db.clone();
    let racer = tokio::spawn(async move { sync::bootstrap_base(&db2, doc).await });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    tx.commit().await.unwrap();

    let got = racer.await.unwrap().unwrap();
    assert_eq!(
        got.state, winner_state,
        "the losing bootstrapper must hand out the STORED base, not its own twin \
         (a twin has a different yrs actor — peers' edits would pend forever)"
    );

    cleanup(&db, ws, user).await;
}

// ── FTS M1: document_yrs_base.content_text (the search index) ────────────────

/// The document's stored search text.
async fn stored_content_text(db: &PgPool, doc: Uuid) -> String {
    sqlx::query_scalar("SELECT content_text FROM document_yrs_base WHERE document_id = $1")
        .bind(doc)
        .fetch_one(db)
        .await
        .unwrap()
}

/// The pure projection `content_text` MUST equal: block text in tree order,
/// empties dropped, newline-joined. Mirrors `sync::content_text_from_doc` (which
/// is pub(crate), so it can't be called from this integration crate) — asserting
/// the stored column equals THIS is the red-line #1 check (index == derivation of
/// the base, no second source of truth).
fn derive_text(state: &[u8]) -> String {
    MicaDoc::from_update(state)
        .unwrap()
        .to_blocks()
        .iter()
        .map(|b| b.text.trim_end_matches('\n'))
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The collaborative push path keeps content_text in lockstep with the base, and
/// the derived text is exactly a pure projection of the stored base — including
/// CJK, so a 3-char and a 2-char substring both live in the column an `ILIKE`
/// scans. Multiple writes must not drift the index off the base.
#[tokio::test]
async fn push_update_maintains_content_text() {
    let Some(db) = pool().await else {
        eprintln!("skipping push_update_maintains_content_text: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;

    let base = sync::bootstrap_base(&db, doc).await.unwrap();
    let mut editing = MicaDoc::from_update(&base.state).unwrap();
    let sv = editing.state_vector();
    // Replace "Hello" with a Chinese sentence.
    editing.text_insert("a", 5, "，全文搜索索引化");
    let update = editing.encode_diff(&sv).unwrap();
    sync::push_update(&db, ws, doc, user, &update).await.unwrap();

    let stored = stored_content_text(&db, doc).await;
    let base2 = sync::document_base(&db, doc).await.unwrap().unwrap();
    assert_eq!(
        stored,
        derive_text(&base2.state),
        "content_text is a pure projection of the base it is stored beside"
    );
    // 3+ char CJK hit and 2-char substring hit both present in the index.
    assert!(stored.contains("全文搜索"), "3+ char CJK substring: {stored}");
    assert!(stored.contains("索引"), "2-char CJK substring: {stored}");

    // A second push must not leave a stale index: content_text tracks the base.
    let mut editing2 = MicaDoc::from_update(&base2.state).unwrap();
    let sv2 = editing2.state_vector();
    editing2.text_insert("a", 0, "追加：");
    let update2 = editing2.encode_diff(&sv2).unwrap();
    sync::push_update(&db, ws, doc, user, &update2).await.unwrap();
    let base3 = sync::document_base(&db, doc).await.unwrap().unwrap();
    assert_eq!(
        stored_content_text(&db, doc).await,
        derive_text(&base3.state),
        "repeated writes do not drift the index off the base"
    );

    cleanup(&db, ws, user).await;
}

/// The REST/MCP op-model write path (`store::apply_document_operations`) folds
/// through yrs and must maintain content_text identically to the sync path.
#[tokio::test]
async fn op_write_maintains_content_text() {
    let Some(db) = pool().await else {
        eprintln!("skipping op_write_maintains_content_text: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;

    use mica_app_core::documents::DocumentOperation;
    let ops = vec![DocumentOperation::UpdateBlock {
        block_id: "a".to_string(),
        kind: None,
        text: Some("中文全文检索".to_string()),
        data: None,
    }];
    store::apply_document_operations(&db, ws, doc, user, &ops)
        .await
        .unwrap();

    let stored = stored_content_text(&db, doc).await;
    let base = sync::document_base(&db, doc).await.unwrap().unwrap();
    assert_eq!(stored, derive_text(&base.state));
    assert!(stored.contains("全文检索"), "3+ char CJK: {stored}");
    assert!(stored.contains("检索"), "2-char CJK substring: {stored}");

    cleanup(&db, ws, user).await;
}

/// Startup backfill fills every still-empty row from its base, and a base that
/// fails to decode is skipped (warn-logged) without erroring or blocking. A
/// genuinely-empty row and a decoded one are the two `content_text = ''` shapes
/// the keyset walk must both survive.
#[tokio::test]
async fn backfill_fills_valid_and_skips_corrupt() {
    let Some(db) = pool().await else {
        eprintln!("skipping backfill_fills_valid_and_skips_corrupt: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;

    // A valid base inserted WITHOUT content_text (the pre-0012 shape → '').
    let good = MicaDoc::from_blocks("r", &[para("r", ""), para("a", "回填后的可搜索文本")]);
    sqlx::query(
        "INSERT INTO document_yrs_base(document_id,state,state_vector,base_rid,updated_at)
         VALUES ($1,$2,$3,0,now())",
    )
    .bind(doc)
    .bind(good.encode_state())
    .bind(good.state_vector())
    .execute(&db)
    .await
    .unwrap();

    // A second document whose base is undecodable garbage — must be skipped.
    let doc2 = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO documents(id,workspace_id,root_block_id,current_seq,created_by)
         VALUES($1,$2,'r',0,$3)",
    )
    .bind(doc2)
    .bind(ws)
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO document_yrs_base(document_id,state,state_vector,base_rid,updated_at)
         VALUES ($1,$2,$3,0,now())",
    )
    .bind(doc2)
    .bind(b"not a yrs update".to_vec())
    .bind(b"nope".to_vec())
    .execute(&db)
    .await
    .unwrap();

    assert_eq!(stored_content_text(&db, doc).await, "", "starts empty");
    let filled = sync::backfill_content_text(&db).await.unwrap();
    assert!(filled >= 1, "at least the valid row was indexed");

    assert_eq!(
        stored_content_text(&db, doc).await,
        "回填后的可搜索文本",
        "valid base backfilled from its yrs state"
    );
    assert_eq!(
        stored_content_text(&db, doc2).await,
        "",
        "undecodable base skipped, left empty, no error"
    );

    // Idempotent: a second pass changes nothing (the filled row is skipped by the
    // content_text='' filter; the corrupt row is re-skipped) and terminates.
    sync::backfill_content_text(&db).await.unwrap();
    assert_eq!(stored_content_text(&db, doc).await, "回填后的可搜索文本");

    cleanup(&db, ws, user).await;
}
