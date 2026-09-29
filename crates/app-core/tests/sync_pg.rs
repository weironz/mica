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
/// `8 passed`, and never have touched Postgres. docs/lessons.md「测试可以"真空通过"」
/// recorded
/// exactly that ("测试真空通过") after it hid a red test — and the code stayed.
///
/// Now a set-but-unusable DATABASE_URL panics, and in CI a MISSING one panics
/// too, because there the database is always provisioned: its absence means the
/// workflow regressed, not that these assertions may quietly stop running.
/// The shipped stream window + prune cadence. These tests assert sync BEHAVIOUR,
/// not tuning, so they run on the defaults — which are the values that used to be
/// `sync::STREAM_KEEP_MARGIN` / `STREAM_PRUNE_EVERY` constants here.
fn tuning() -> sync::SyncTuning {
    sync::SyncTuning::default()
}

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

/// Seed the FK chain (user → workspace → document → yrs base) with a one-line
/// document: root `r` holding paragraph `a` = "Hello". Returns (ws, doc, user).
///
/// Before S5 this inserted an op-model snapshot and left the base to the lazy
/// bridge, so every test here was implicitly also exercising that bridge. The
/// bridge and its table are gone; the base is now seeded directly, through the
/// same `seed_base_tx` the create/import/clone handlers use.
async fn seed_doc(db: &PgPool) -> (Uuid, Uuid, Uuid) {
    let (ws, doc, user) = seed_doc_bare(db).await;
    let payload: mica_markdown::DocumentSnapshotPayload = serde_json::from_value(json!({
        "schema_version": 1,
        "root_block_id": "r",
        "blocks": [
            {"id":"r","type":"page","children":["a"]},
            {"id":"a","type":"paragraph","text":"Hello"}
        ]
    }))
    .unwrap();
    let mut tx = db.begin().await.unwrap();
    sync::seed_base_tx(&mut tx, doc, payload).await.unwrap();
    tx.commit().await.unwrap();
    (ws, doc, user)
}

/// [`seed_doc`] without the base: user → workspace → `documents` row and nothing
/// else. For the tests that insert their OWN base row — the first-bootstrap race
/// and the content_text backfill both need to be the ones that create it.
async fn seed_doc_bare(db: &PgPool) -> (Uuid, Uuid, Uuid) {
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
    (ws, doc, user)
}

async fn cleanup(db: &PgPool, ws: Uuid, user: Uuid) {
    // workspaces cascade to documents → workspace_updates, yrs_base, versions.
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

    // `documents.root_block_id` still knows the real root — don't adopt the empty
    // one. (Before S5 this healed from the op-model snapshot's copy of it.)
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

    // The seeded base has taken no stream updates yet, so rid 0.
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
    let rid = sync::push_update(&db, ws, doc, user, &update, &tuning()).await.unwrap();
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
    // `rid % prune_every == 0` on the pushing row's rid, and rid comes
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
        let rid = sync::push_update(&db, ws, doc, user, &update, &tuning()).await.unwrap();
        pushes += 1;
        if first_rid == 0 {
            first_rid = rid;
        }
        if rid % tuning().prune_every == 0 && rid - first_rid > tuning().keep_margin {
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
        sync::push_update(&db, ws, doc, user, &update, &tuning()).await.unwrap();
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
        let rid = sync::push_update(&db, ws, doc, user, &update, &tuning()).await.unwrap();
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
    sync::push_update(&db, ws, doc, user, &up, &tuning()).await.unwrap();
    let version = store::create_named_yrs_version(&db, doc, "v-there", user)
        .await
        .unwrap();

    // Edit further → "Hello there, world".
    let sv2 = editing.state_vector();
    editing.text_insert("a", 11, ", world");
    let up2 = editing.encode_diff(&sv2).unwrap();
    sync::push_update(&db, ws, doc, user, &up2, &tuning()).await.unwrap();
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
    let (rid, update) = sync::restore_yrs_version(&db, ws, doc, user, &vstate, &tuning())
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
    let err = sync::push_update(&db, ws, doc, user, &[9, 9, 9, 9], &tuning()).await;
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
    let head_before = sync::push_update(&db, ws, doc, user, &edit, &tuning()).await.unwrap();

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
    let (ws, doc, user) = seed_doc_bare(&db).await;

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

// ── P1-13: the gap decision and the pull must share one snapshot ─────────────

/// P1-13: a prune committing while `catch_up_document` is deciding must not let
/// the client skip the pruned range.
///
/// The bug: `catch_up_document` made three separate autocommit reads — MIN(rid),
/// base_rid, then the pull. Each got its OWN snapshot under READ COMMITTED, so a
/// prune landing between them produced a decision and a pull that disagreed. The
/// gap test saw the pre-prune MIN (no gap), the pull saw the post-prune stream,
/// and the rows it returned started ABOVE the client's cursor. The client
/// advanced its cursor across updates it never received, and those updates now
/// exist only inside a base nobody told it to re-bootstrap from — silent,
/// permanent divergence, triggered by the very pruning that bounds growth.
///
/// The interleaving is CONSTRUCTED with a real barrier: an `ACCESS EXCLUSIVE`
/// lock on `document_yrs_base` blocks the SECOND read (`base_rid`) while the
/// first (MIN) is already done, and the prune commits in that window. Blocking
/// the second read specifically is what makes this discriminating — under the
/// fix the third read shares the transaction snapshot taken at the second, so it
/// still sees the rows the prune removed; unfixed, it re-snapshots.
///
/// The assertion is the invariant a catch-up client relies on: whatever comes
/// back, it must be usable from `since_rid` — either a contiguous run starting at
/// `since_rid + 1`, or a re-bootstrap. A run that starts higher is the hole.
#[tokio::test]
async fn catch_up_decision_and_pull_share_one_snapshot() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc_bare(&db).await;

    // A stream with known rids, and a base claiming to have folded up to the
    // cursor. The payloads are never applied here — this test is about which
    // ROWS come back, and hand-built update bytes would only add failure modes
    // that have nothing to do with the property under test.
    let mut rids = Vec::new();
    for n in 0..6u8 {
        let rid: i64 = sqlx::query_scalar(
            "INSERT INTO workspace_updates(workspace_id, document_id, actor_id, payload)
             VALUES ($1,$2,$3,$4) RETURNING rid",
        )
        .bind(ws)
        .bind(doc)
        .bind(user)
        .bind(vec![n])
        .fetch_one(&db)
        .await
        .unwrap();
        rids.push(rid);
    }
    // Values, not positions: `rid` comes from a TABLE-wide sequence, so this
    // document's rows are NOT consecutive — a parallel test drawing rids leaves
    // holes between them. Assuming `rids[i] + 1 == rids[i+1]` asserts something
    // the schema never promised (it failed exactly that way on the first full
    // suite run: expected 242, got 243, from an unrelated document's insert).
    let m_old = rids[0];
    let needed = m_old;
    // `since_rid = m_old - 1` makes the gap test a NO-OP on the pre-prune stream
    // (`since_rid + 1 < m_old` is false), while the client still genuinely needs
    // `m_old` — which the prune is about to delete. That is the trap.
    let since_rid = m_old - 1;
    let base_rid = m_old;

    // What the DECISION snapshot sees: every row the client is owed. Captured
    // before the prune, because those are exactly the rows the pull must return.
    let owed: Vec<i64> = sqlx::query_scalar(
        "SELECT rid FROM workspace_updates WHERE document_id = $1 AND rid > $2 ORDER BY rid",
    )
    .bind(doc)
    .bind(since_rid)
    .fetch_all(&db)
    .await
    .unwrap();
    assert!(
        owed.contains(&m_old),
        "the row about to be pruned must be one the client still needs"
    );

    sqlx::query(
        "INSERT INTO document_yrs_base(document_id, state, state_vector, base_rid, updated_at)
         VALUES ($1,$2,$3,$4,now())",
    )
    .bind(doc)
    .bind(vec![0u8])
    .bind(vec![0u8])
    .bind(base_rid)
    .execute(&db)
    .await
    .unwrap();

    // Hold the base table so the SECOND read blocks. The MIN read is already
    // done by then, which is exactly the window the bug lived in.
    let mut blocker = db.begin().await.unwrap();
    sqlx::query("LOCK TABLE document_yrs_base IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();

    let db2 = db.clone();
    let catch_up = tokio::spawn(async move {
        sync::catch_up_document(&db2, doc, since_rid, 1000).await
    });

    // Wait until the call is actually parked on the lock — polling `pg_locks`
    // rather than sleeping a fixed interval, so the barrier is observed rather
    // than assumed.
    let mut waited = 0u32;
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks l
               JOIN pg_class c ON c.oid = l.relation
              WHERE NOT l.granted AND c.relname = 'document_yrs_base'",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        if waiting > 0 {
            break;
        }
        assert!(waited < 200, "catch_up never reached the blocked second read");
        waited += 1;
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    // Commit the prune while the decision is half-made. `since_rid + 1` is gone.
    sqlx::query("DELETE FROM workspace_updates WHERE document_id = $1 AND rid <= $2")
        .bind(doc)
        .bind(needed)
        .execute(&db)
        .await
        .unwrap();
    blocker.rollback().await.unwrap();

    match catch_up.await.unwrap().unwrap() {
        sync::CatchUp::Updates(updates) => {
            let got: Vec<i64> = updates.iter().map(|u| u.rid).collect();
            assert_eq!(
                got, owed,
                "the pull must return exactly the rows the decision snapshot saw it owed \
                 ({}); anything shorter means the client advanced its cursor across updates \
                 it never received — the pruned row {m_old} survives only inside a base it \
                 was not told to re-bootstrap from",
                owed.len()
            );
        }
        // A re-bootstrap is always a correct answer — the client gets the base
        // that absorbed everything. It is simply not what the buggy code did.
        sync::CatchUp::Rebootstrap(_) => {
            panic!(
                "expected an incremental run for a cursor the base has already folded past; \
                 a re-bootstrap here means the gap test saw the post-prune stream, so this \
                 test no longer exercises the interleaving it was written for"
            )
        }
    }

    cleanup(&db, ws, user).await;
}

// ── P0-01: two writers must not lose each other's fold ───────────────────────

/// Text of block `id` as stored in the folded base.
///
/// The INVARIANT under test is about `state` — the one representation every read,
/// export and MCP fetch consults — so the assertions decode the base rather than
/// trusting any derived column to stand in for it.
fn base_block_text(state: &[u8], id: &str) -> String {
    MicaDoc::from_update(state)
        .unwrap()
        .to_blocks()
        .into_iter()
        .find(|b| b.id == id)
        .map(|b| b.text)
        .unwrap_or_default()
}

/// The base's `content_text` — the search projection that must move in the SAME
/// statement as `state`, never lag it.
async fn base_content_text(db: &PgPool, doc: Uuid) -> String {
    sqlx::query_scalar("SELECT content_text FROM document_yrs_base WHERE document_id = $1")
        .bind(doc)
        .fetch_one(db)
        .await
        .unwrap()
}

/// P0-01a: two concurrent WebSocket pushes must both reach the folded base.
///
/// The bug (`push_update` before this fix): it began a transaction and read the
/// base with a plain unlocked SELECT. Two pushes both read base B, both folded
/// their own update onto it, and the second `ON CONFLICT DO UPDATE SET
/// state = excluded.state` overwrote the first's fold — while BOTH
/// `workspace_updates` rows committed. The stream then advertised an update the
/// base never absorbed, so a client catching up from the stream and a client
/// reading the base disagree permanently (red line #1).
///
/// The interleaving is CONSTRUCTED, not raced: an outer transaction takes the
/// `documents` row lock that `push_update` needs, the losing push is spawned and
/// blocks on it, and only then is the winner's fold committed. That makes the
/// pre-fix failure deterministic rather than occasional — a test that only fails
/// under random timing is not a regression test.
///
/// The strongest assertion is the last one: the base must equal what folding the
/// whole stream from scratch produces. That is exactly what the bug broke, and it
/// holds regardless of which writer the lock happens to serialize first.
#[tokio::test]
async fn concurrent_pushes_both_reach_the_base() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;

    // Two editors, each holding the SAME stored base (nobody has pushed yet, so
    // their state vectors agree) and each inserting its own marker at offset 5,
    // right after "Hello".
    let base = sync::bootstrap_base(&db, doc).await.unwrap();
    let mut a = MicaDoc::from_update(&base.state).unwrap();
    let mut b = MicaDoc::from_update(&base.state).unwrap();
    let sv_a = a.state_vector();
    a.text_insert("a", 5, "AAA");
    let update_a = a.encode_diff(&sv_a).unwrap();
    let sv_b = b.state_vector();
    b.text_insert("a", 5, "BBB");
    let update_b = b.encode_diff(&sv_b).unwrap();

    // Weave the lock: A's fold + stream row sit uncommitted in `tx`, holding the
    // `documents` row lock. B's push — spawned here — must wait for it.
    let mut tx = db.begin().await.unwrap();
    sqlx::query(
        "SELECT id FROM documents WHERE id = $1 AND workspace_id = $2 FOR UPDATE",
    )
    .bind(doc)
    .bind(ws)
    .execute(&mut *tx)
    .await
    .unwrap();
    let rid_a: i64 = sqlx::query_scalar(
        "INSERT INTO workspace_updates(workspace_id, document_id, actor_id, payload)
         VALUES ($1,$2,$3,$4) RETURNING rid",
    )
    .bind(ws)
    .bind(doc)
    .bind(user)
    .bind(&update_a)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO document_yrs_base(document_id, state, state_vector, base_rid, content_text, updated_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (document_id) DO UPDATE SET
             state = excluded.state, state_vector = excluded.state_vector,
             base_rid = excluded.base_rid, content_text = excluded.content_text,
             updated_at = now()",
    )
    .bind(doc)
    .bind(a.encode_state())
    .bind(a.state_vector())
    .bind(rid_a)
    .bind(sync::content_text_from_doc(&a))
    .execute(&mut *tx)
    .await
    .unwrap();

    let db2 = db.clone();
    let loser = tokio::spawn(async move {
        sync::push_update(&db2, ws, doc, user, &update_b, &tuning()).await
    });
    // B is now parked on the row lock. 500ms is the same margin the bootstrap
    // test above uses to make "B has blocked" a certainty rather than a hope.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    tx.commit().await.unwrap();

    let rid_b = loser.await.unwrap().unwrap();
    assert!(rid_b > rid_a, "the waiting push is serialized after the winner");

    // 1. The base — the one representation — carries BOTH edits.
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT state FROM document_yrs_base WHERE document_id = $1")
            .bind(doc)
            .fetch_one(&db)
            .await
            .unwrap();
    let text = base_block_text(&stored, "a");
    assert!(
        text.contains("AAA") && text.contains("BBB"),
        "both pushes must survive in the base, got {text:?} — a missing marker is the \
         lost update: both stream rows committed but the base absorbed only one"
    );

    // 2. `base_rid` points at the LAST folded push, so a client that catches up
    //    from the stream and one that reads the base agree on where they are.
    let base_rid: i64 =
        sqlx::query_scalar("SELECT base_rid FROM document_yrs_base WHERE document_id = $1")
            .bind(doc)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(base_rid, rid_b, "base_rid must name the highest folded rid");

    // 3. The search projection moved with it (same statement, so it cannot lag).
    let content = base_content_text(&db, doc).await;
    assert!(
        content.contains("AAA") && content.contains("BBB"),
        "content_text is co-written with state and must hold both edits, got {content:?}"
    );

    // 4. The invariant that makes the other three meaningful: the base equals the
    //    fold of EVERY stream row. This is what the stream promises a catching-up
    //    client, and it is the property the lost update violated.
    let rows: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT payload FROM workspace_updates WHERE document_id = $1 ORDER BY rid",
    )
    .bind(doc)
    .fetch_all(&db)
    .await
    .unwrap();
    let mut folded = MicaDoc::from_update(&base.state).unwrap();
    for row in &rows {
        folded.apply_update(row).unwrap();
    }
    assert_eq!(
        base_block_text(&folded.encode_state(), "a"),
        text,
        "folding the {} stream rows from the base must reproduce the stored base exactly",
        rows.len()
    );

    cleanup(&db, ws, user).await;
}

/// P0-01b: a REST/MCP op write and a WebSocket push, interleaved, must not
/// overwrite each other.
///
/// The two paths take the same row lock in the same order (`documents` first) —
/// that is the whole fix, and this is the test that would fail if only ONE of
/// them locked. Same constructed interleaving as above: the op write's
/// transaction holds the lock, `push_update` blocks on it, then the op commits.
#[tokio::test]
async fn op_write_and_push_interleave_without_losing_either() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;

    // A push lands first, so the document has a yrs base the op path will fold
    // into rather than seed.
    let base = sync::bootstrap_base(&db, doc).await.unwrap();
    let mut editing = MicaDoc::from_update(&base.state).unwrap();
    let sv = editing.state_vector();
    editing.text_insert("a", 5, "-push1");
    let first = editing.encode_diff(&sv).unwrap();
    sync::push_update(&db, ws, doc, user, &first, &tuning()).await.unwrap();

    // The concurrent pair, both against the same stored base.
    let base = sync::bootstrap_base(&db, doc).await.unwrap();
    let mut pusher = MicaDoc::from_update(&base.state).unwrap();
    let sv = pusher.state_vector();
    pusher.text_insert("a", 0, "PUSH-");
    let push_update_bytes = pusher.encode_diff(&sv).unwrap();

    // Hold the lock, then let the push block on it while the op write's fold and
    // stream row sit uncommitted inside `tx` (same shape as the test above).
    let mut tx = db.begin().await.unwrap();
    sqlx::query("SELECT id FROM documents WHERE id = $1 AND workspace_id = $2 FOR UPDATE")
        .bind(doc)
        .bind(ws)
        .execute(&mut *tx)
        .await
        .unwrap();
    let rid_a: i64 = sqlx::query_scalar(
        "INSERT INTO workspace_updates(workspace_id, document_id, actor_id, payload)
         VALUES ($1,$2,$3,$4) RETURNING rid",
    )
    .bind(ws)
    .bind(doc)
    .bind(user)
    .bind(&push_update_bytes)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO document_yrs_base(document_id, state, state_vector, base_rid, content_text, updated_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (document_id) DO UPDATE SET
             state = excluded.state, state_vector = excluded.state_vector,
             base_rid = excluded.base_rid, content_text = excluded.content_text,
             updated_at = now()",
    )
    .bind(doc)
    .bind(pusher.encode_state())
    .bind(pusher.state_vector())
    .bind(rid_a)
    .bind(sync::content_text_from_doc(&pusher))
    .execute(&mut *tx)
    .await
    .unwrap();

    // The op write — the REST/MCP path — while the lock is held.
    let db2 = db.clone();
    let op = tokio::spawn(async move {
        store::apply_document_operations(
            &db2,
            ws,
            doc,
            user,
            &[mica_app_core::documents::DocumentOperation::InsertBlock {
                block: serde_json::from_value(json!({
                    "id":"rest","type":"paragraph","text":"rest-op"
                }))
                .unwrap(),
                parent_id: "r".to_string(),
                index: None,
            }],
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    tx.commit().await.unwrap();

    let applied = op.await.unwrap().unwrap();
    assert!(applied.yrs.is_some(), "the op path folds into the base and streams it");

    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT state FROM document_yrs_base WHERE document_id = $1")
            .bind(doc)
            .fetch_one(&db)
            .await
            .unwrap();
    let doc_now = MicaDoc::from_update(&stored).unwrap();
    let blocks = doc_now.to_blocks();
    assert!(
        blocks.iter().any(|b| b.id == "rest" && b.text == "rest-op"),
        "the REST/MCP write must survive a concurrent push"
    );
    let a_text = blocks
        .iter()
        .find(|b| b.id == "a")
        .map(|b| b.text.clone())
        .unwrap_or_default();
    assert!(
        a_text.contains("PUSH-") && a_text.contains("-push1"),
        "both the blocked push and the earlier push must survive, got {a_text:?}"
    );

    cleanup(&db, ws, user).await;
}

/// The document's stored search text.
/// `link_targets` as stored. `None` is the "never derived" sentinel migration
/// 0019 leaves behind; `Some(vec![])` means derived-and-empty, which is what the
/// backfill must converge to so it stops revisiting the row every boot.
async fn stored_link_targets(db: &PgPool, doc: Uuid) -> Option<Vec<Uuid>> {
    sqlx::query_scalar("SELECT link_targets FROM document_yrs_base WHERE document_id = $1")
        .bind(doc)
        .fetch_one(db)
        .await
        .unwrap()
}

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

/// `link_targets` is the backlink index, and it is co-written with the base by
/// the same contract `content_text` has (red line #1). The half that actually
/// breaks in practice is REMOVAL: an index that only ever gains entries still
/// passes every "the backlink shows up" test, while the panel keeps listing a
/// page whose link was deleted. So this asserts both directions.
#[tokio::test]
async fn writes_maintain_link_targets_in_both_directions() {
    let Some(db) = pool().await else {
        eprintln!("skipping writes_maintain_link_targets_in_both_directions: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;
    let target = Uuid::new_v4();

    use mica_app_core::documents::DocumentOperation;
    let link_op = |data: serde_json::Value| {
        vec![DocumentOperation::UpdateBlock {
            block_id: "a".to_string(),
            kind: None,
            text: Some("see other page".to_string()),
            data: Some(data),
        }]
    };

    // Adding a page link puts the target in the index.
    store::apply_document_operations(
        &db,
        ws,
        doc,
        user,
        &link_op(serde_json::json!({
            "marks": [{"type": "link", "href": format!("mica://page/{target}"), "start": 0, "end": 3}]
        })),
    )
    .await
    .unwrap();
    assert_eq!(
        stored_link_targets(&db, doc).await,
        Some(vec![target]),
        "the co-write derived the link target from the same doc it stored"
    );

    // Removing it takes the target back OUT. Before the index this was free —
    // the scan re-read the blocks every time — so nothing in the old code could
    // go stale here, and nothing tested it.
    store::apply_document_operations(&db, ws, doc, user, &link_op(serde_json::json!({"marks": []})))
        .await
        .unwrap();
    assert_eq!(
        stored_link_targets(&db, doc).await,
        Some(Vec::new()),
        "a deleted link must leave the index, not linger in it"
    );

    cleanup(&db, ws, user).await;
}

// ── views.name as a projection of the document's title ──────────────────────
//
// The third instance of the pattern content_text and link_targets follow. Two
// things make it different and worth its own tests: the other two write a column
// beside the base in the same statement, this one writes a DIFFERENT TABLE; and
// it must be a complete no-op for the many documents that carry no title at all
// (there is no backfill migration — `docs/page-title-plan.md` §4.1).

/// Give a seeded document a `views` row, which `seed_doc` does not — the tests
/// that came before this one only ever looked at `document_yrs_base`, so the
/// fixture never needed a view. The projection writes `views.name`, so these do.
async fn seed_view_for(db: &PgPool, ws: Uuid, doc: Uuid, user: Uuid, name: &str) -> Uuid {
    let view = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO views(id,workspace_id,object_id,object_type,name,position,created_by)
         VALUES($1,$2,$3,'document',$4,'0',$5)",
    )
    .bind(view)
    .bind(ws)
    .bind(doc)
    .bind(name)
    .bind(user)
    .execute(db)
    .await
    .unwrap();
    view
}

async fn stored_view_name(db: &PgPool, doc: Uuid) -> String {
    sqlx::query_scalar("SELECT name FROM views WHERE object_id = $1")
        .bind(doc)
        .fetch_one(db)
        .await
        .unwrap()
}

/// Writing a title into the document renames the page. This IS the authority
/// transfer: afterwards the document is where the name comes from.
#[tokio::test]
async fn a_title_written_into_the_document_moves_the_view_name() {
    let Some(db) = pool().await else {
        eprintln!("skipping a_title_written_into_the_document_moves_the_view_name: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;
    seed_view_for(&db, ws, doc, user, "原来的名字").await;
    let before = stored_view_name(&db, doc).await;
    assert_ne!(before, "新标题", "the fixture must not already be named this");

    let (_, update) = sync::set_document_title(&db, ws, doc, user, "新标题", &tuning())
        .await
        .unwrap();

    assert!(!update.is_empty(), "a real title change produces an update");
    assert_eq!(stored_view_name(&db, doc).await, "新标题");

    cleanup(&db, ws, user).await;
}

/// An ORDINARY body edit must not touch the name.
///
/// The one that would bite silently: `push_update` runs on every keystroke, so a
/// projection that fired unconditionally would rewrite every page's name on
/// every edit — to nothing, for the majority of pages that have no title.
#[tokio::test]
async fn an_ordinary_edit_leaves_an_untitled_pages_name_alone() {
    let Some(db) = pool().await else {
        eprintln!("skipping an_ordinary_edit_leaves_an_untitled_pages_name_alone: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;
    seed_view_for(&db, ws, doc, user, "不该被改的名字").await;
    let before = stored_view_name(&db, doc).await;
    assert!(!before.is_empty(), "fixture has a name to protect");

    let base = sync::bootstrap_base(&db, doc).await.unwrap();
    let mut editing = MicaDoc::from_update(&base.state).unwrap();
    let sv = editing.state_vector();
    editing.text_insert("a", 5, " 又写了几个字");
    let update = editing.encode_diff(&sv).unwrap();
    sync::push_update(&db, ws, doc, user, &update, &tuning()).await.unwrap();

    assert_eq!(
        stored_view_name(&db, doc).await,
        before,
        "a body edit on a page with no document title must not rename it"
    );

    cleanup(&db, ws, user).await;
}

/// Once a page HAS a title, ordinary edits keep the name pinned to it — so a
/// rename written straight to the column gets re-derived away on the next push.
/// That is exactly why `update_view` must also write the document.
#[tokio::test]
async fn a_body_edit_re_derives_the_name_from_the_documents_title() {
    let Some(db) = pool().await else {
        eprintln!(
            "skipping a_body_edit_re_derives_the_name_from_the_documents_title: no DATABASE_URL"
        );
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;
    seed_view_for(&db, ws, doc, user, "起初的名字").await;
    sync::set_document_title(&db, ws, doc, user, "文档说了算", &tuning())
        .await
        .unwrap();

    // Somebody writes the column directly, the way a pre-P2 rename would.
    sqlx::query("UPDATE views SET name = '绕过文档改的名' WHERE object_id = $1")
        .bind(doc)
        .execute(&db)
        .await
        .unwrap();

    let base = sync::document_base(&db, doc).await.unwrap().unwrap();
    let mut editing = MicaDoc::from_update(&base.state).unwrap();
    let sv = editing.state_vector();
    editing.text_insert("a", 0, "编辑正文。");
    let update = editing.encode_diff(&sv).unwrap();
    sync::push_update(&db, ws, doc, user, &update, &tuning()).await.unwrap();

    assert_eq!(
        stored_view_name(&db, doc).await,
        "文档说了算",
        "the column is a projection: a write that bypassed the document loses"
    );

    cleanup(&db, ws, user).await;
}

/// Renaming to the name it already has must not spend a push — no version
/// snapshot, no wake-up for every open editor.
#[tokio::test]
async fn setting_the_same_title_twice_is_a_no_op() {
    let Some(db) = pool().await else {
        eprintln!("skipping setting_the_same_title_twice_is_a_no_op: no DATABASE_URL");
        return;
    };
    let (ws, doc, user) = seed_doc(&db).await;
    seed_view_for(&db, ws, doc, user, "起初的名字").await;
    sync::set_document_title(&db, ws, doc, user, "同一个名字", &tuning())
        .await
        .unwrap();

    let (_, second) = sync::set_document_title(&db, ws, doc, user, "同一个名字", &tuning())
        .await
        .unwrap();

    assert!(second.is_empty(), "the second write produced no update");

    cleanup(&db, ws, user).await;
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
    sync::push_update(&db, ws, doc, user, &update, &tuning()).await.unwrap();

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
    sync::push_update(&db, ws, doc, user, &update2, &tuning()).await.unwrap();
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
    let (ws, doc, user) = seed_doc_bare(&db).await;

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
    assert_eq!(
        stored_link_targets(&db, doc).await,
        None,
        "link_targets starts at the NULL sentinel"
    );
    let filled = sync::backfill_derived_columns(&db).await.unwrap();
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

    // A link-free document must land on `Some([])`, NOT stay NULL — that is the
    // whole point of the NULL sentinel. If this regressed to NULL the backfill
    // would re-decode this row on every single boot and never converge.
    assert_eq!(
        stored_link_targets(&db, doc).await,
        Some(Vec::new()),
        "derived-and-empty, so the next pass skips it"
    );

    // Idempotent: a second pass changes nothing (the filled row now matches
    // neither sentinel; the corrupt row is re-skipped) and terminates.
    sync::backfill_derived_columns(&db).await.unwrap();
    assert_eq!(stored_content_text(&db, doc).await, "回填后的可搜索文本");
    assert_eq!(stored_link_targets(&db, doc).await, Some(Vec::new()));

    cleanup(&db, ws, user).await;
}

/// S5's invariant, in the strongest form it can take: the op-model tables are
/// GONE, so "a write does not grow them" is enforced by the schema itself rather
/// than by a count.
///
/// S4's version of this counted rows before and after a write, because a negative
/// claim ("nothing writes these any more") is the kind that rots in silence —
/// re-adding an INSERT would have broken nothing. Migration 0016 turns that claim
/// into a structural fact, and this pins the fact: re-creating any of the three
/// tables, in a migration or by hand, fails here.
#[tokio::test]
async fn the_op_model_tables_are_gone() {
    let Some(db) = pool().await else { return };
    for table in ["document_snapshots", "document_updates", "document_versions"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables
                            WHERE table_schema = 'public' AND table_name = $1)",
        )
        .bind(table)
        .fetch_one(&db)
        .await
        .unwrap();
        assert!(!exists, "{table} must not exist after migration 0016");
    }
}

/// A write lands, and lands in the ONE place left. Counting rows in the dropped
/// tables is no longer possible, so the assertion moves to what actually matters:
/// the content reads back afterwards through the yrs base alone.
#[tokio::test]
async fn a_write_round_trips_through_the_base_alone() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;

    let applied = store::apply_document_operations(
        &db,
        ws,
        doc,
        user,
        &[mica_app_core::documents::DocumentOperation::InsertBlock {
            block: mica_markdown::Block {
                id: "b1".to_string(),
                kind: "paragraph".to_string(),
                text: "written after S5".to_string(),
                data: serde_json::Value::Null,
                children: Vec::new(),
            },
            parent_id: "r".to_string(),
            index: Some(0),
        }],
    )
    .await
    .expect("the write must be accepted");

    assert!(applied.yrs.is_some(), "the yrs half IS the write now");
    let payload = store::current_payload(&db, doc)
        .await
        .unwrap()
        .expect("the document must read back");
    assert!(
        payload.blocks.iter().any(|b| b.text == "written after S5"),
        "content must round-trip through the yrs base alone, got {:?}",
        payload.blocks
    );
    // The pre-existing paragraph survives — a write must not clobber the base it
    // was derived from.
    assert!(payload.blocks.iter().any(|b| b.text == "Hello"));
    assert_eq!(payload.root_block_id, "r");

    cleanup(&db, ws, user).await;
}

/// A document with no base at all must open as an EMPTY page rooted at
/// `documents.root_block_id` — not a 404, not a crash.
///
/// `ensure_base_tx` used to fold an op-model snapshot here; with that table gone
/// there is nothing to fold and nothing to lose, because a base-less document is
/// a content-less one (every write path seeds the base in the same transaction as
/// the `documents` row). This should never fire in practice — it is what stands
/// between a document that somehow lost its base and a socket that closes on
/// every attempt to open it, forever.
#[tokio::test]
async fn a_document_with_no_base_opens_as_an_empty_page() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;
    sqlx::query("DELETE FROM document_yrs_base WHERE document_id=$1")
        .bind(doc)
        .execute(&db)
        .await
        .unwrap();

    let base = sync::bootstrap_base(&db, doc)
        .await
        .expect("a base-less document must not be an error");
    let head = MicaDoc::from_update(&base.state).unwrap();
    assert_eq!(
        head.root_block_id(),
        "r",
        "the root must come from documents.root_block_id"
    );

    cleanup(&db, ws, user).await;
}

/// 量「长离线重连」的两条路径:攒了 N 条更新后,逐条推 vs 合并成一条再推。
///
/// 不是回归门,是一次测量 —— 所以 `#[ignore]`,要跑就显式跑:
/// `DATABASE_URL=… cargo test -p mica-app-core --test sync_pg -- --ignored offline_reconnect --nocapture`
///
/// 合并的做法就是条目里写的那句「先用 yrs merge 把尾巴合成一条」:把 N 条应用到一个
/// 临时 doc,再对**离线前**的 state vector 出一次 diff。等价于 N 条的净效果。
#[tokio::test]
#[ignore = "measurement, not a gate — needs a live DB and prints numbers"]
async fn offline_reconnect_merge_measurement() {
    let Some(db) = pool().await else { return };
    for n in [10usize, 50, 200] {
        // ── 造 N 条更新:模拟离线期间逐次编辑同一个块 ──
        let (ws, doc, user) = seed_doc(&db).await;
        let base: Vec<u8> =
            sqlx::query_scalar("SELECT state FROM document_yrs_base WHERE document_id = $1")
                .bind(doc)
                .fetch_one(&db)
                .await
                .unwrap();
        // The client is a DIFFERENT actor from the server that seeded the base —
        // otherwise the updates merge trivially and the measurement flatters itself.
        let sv0 = MicaDoc::from_update(&base).unwrap().state_vector();
        let mut d = MicaDoc::from_update_with_client_id(&base, Some(4242)).unwrap();
        let mut updates = Vec::new();
        let mut text = String::from("Hello");
        for i in 0..n {
            let before = d.state_vector();
            text.push_str(&format!(" w{i}"));
            d.set_block_text("a", &text, &[]);
            updates.push(d.encode_diff(&before).unwrap());
        }
        let bytes: usize = updates.iter().map(|u| u.len()).sum();

        // ── 路径 A:逐条推(今天的行为) ──
        let t = std::time::Instant::now();
        for u in &updates {
            sync::push_update(&db, ws, doc, user, u, &tuning()).await.unwrap();
        }
        let one_by_one = t.elapsed();
        cleanup(&db, ws, user).await;

        // ── 路径 B:先合并再推一次 ──
        let (ws2, doc2, user2) = seed_doc(&db).await;
        let t = std::time::Instant::now();
        let mut scratch = MicaDoc::from_update(&base).unwrap();
        for u in &updates {
            scratch.apply_update(u).unwrap();
        }
        let merged = scratch.encode_diff(&sv0).unwrap();
        let merge_cost = t.elapsed();
        sync::push_update(&db, ws2, doc2, user2, &merged, &tuning()).await.unwrap();
        let merged_total = t.elapsed();
        cleanup(&db, ws2, user2).await;

        println!(
            "n={n:>3}  逐条 {:>7.1} ms ({} 次往返, {} B)  |  合并后 {:>6.1} ms              (其中合并本身 {:.1} ms, 1 次往返, {} B)  |  省 {:.1}x",
            one_by_one.as_secs_f64() * 1000.0,
            n,
            bytes,
            merged_total.as_secs_f64() * 1000.0,
            merge_cost.as_secs_f64() * 1000.0,
            merged.len(),
            one_by_one.as_secs_f64() / merged_total.as_secs_f64(),
        );
    }
}
