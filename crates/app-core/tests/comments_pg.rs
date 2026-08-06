//! Postgres integration tests for comments (docs/comments-plan.md).
//!
//! The anchor primitives have unit tests (`mica-core/tests/comment_anchor.rs`);
//! what those CANNOT prove is that an anchor survives the round trip through the
//! database and a folded yrs base — sticky bytes into `bytea`, a real
//! `push_update`, then resolved again. That end-to-end property is the one the
//! feature actually rests on, so it is tested here against a real database.
//!
//! Gated on `DATABASE_URL`, with the same anti-"vacuum pass" rule as sync_pg.rs:
//! a set-but-unusable url panics, and a MISSING url panics in CI (where the
//! database is always provisioned), so these can never silently stop running.
//!
//!   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//!   cargo test -p mica-app-core --test comments_pg

use mica_app_core::{comments, sync};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

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

/// Seed user → workspace → document → op snapshot. The document's block `a` holds
/// "Hello" (5 UTF-16 units), and only the OP model is written — no yrs base — so
/// `comments::load_doc` has to derive one, which is itself worth proving.
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
    // Straight into the yrs base — since S5 the only place content lives. This
    // used to insert an op-model snapshot and leave the base to the lazy bridge.
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
    mica_app_core::sync::seed_base_tx(&mut tx, doc, payload)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    (ws, doc, user)
}

async fn cleanup(db: &PgPool, ws: Uuid, user: Uuid) {
    // workspaces cascade to documents → yrs_base/comment_threads → comments.
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

#[tokio::test]
async fn a_thread_is_created_with_its_first_comment_and_reads_back() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;

    // load_doc must DERIVE the yrs base from the op-model snapshot (none exists).
    let live = comments::load_doc(&db, doc).await.expect("derives a base");
    let anchor = live
        .sticky_for_range("a", 0, "a", 5)
        .expect("anchors 'Hello'");

    let (thread, first) = comments::create_thread(&db, doc, user, &anchor, "Hello", "why greet?")
        .await
        .unwrap();
    assert_eq!(thread.status, comments::STATUS_OPEN);
    assert_eq!(thread.quote, "Hello");
    assert_eq!(first.body, "why greet?");
    assert!(
        !thread.anchor_start_sticky.is_empty(),
        "sticky bytes persisted"
    );

    let threads = comments::list_threads(&db, doc).await.unwrap();
    assert_eq!(threads.len(), 1);
    let ids: Vec<Uuid> = threads.iter().map(|t| t.id).collect();
    let replies = comments::list_comments(&db, &ids).await.unwrap();
    assert_eq!(replies.len(), 1, "the first comment came with the thread");

    // The anchor survives the DB round trip (bytea → StickyIndex) and resolves.
    let range = live.resolve_range(&threads[0].anchor()).expect("resolves");
    assert_eq!((range.start_offset, range.end_offset), (0, 5));
    assert!(!range.is_empty());

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn an_anchor_survives_a_real_edit_pushed_through_the_database() {
    // THE end-to-end property: store an anchor, push a genuine yrs update that
    // inserts text ahead of it, reload the folded base, and the anchor must have
    // moved with its text. A stored offset would now point at the wrong words.
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;

    let before = comments::load_doc(&db, doc).await.unwrap();
    let anchor = before.sticky_for_range("a", 0, "a", 5).unwrap();
    comments::create_thread(&db, doc, user, &anchor, "Hello", "note")
        .await
        .unwrap();

    // Insert ">> " at the start of block `a` and push it like a client would.
    let mut editing = comments::load_doc(&db, doc).await.unwrap();
    let sv = editing.state_vector();
    editing.text_insert("a", 0, ">> ");
    let update = editing.encode_diff(&sv).unwrap();
    sync::push_update(&db, ws, doc, user, &update, &sync::SyncTuning::default()).await.unwrap();

    let after = comments::load_doc(&db, doc).await.unwrap();
    let stored = comments::list_threads(&db, doc).await.unwrap().remove(0);
    let range = after
        .resolve_range(&stored.anchor())
        .expect("still resolves after the edit");
    assert_eq!(
        (range.start_offset, range.end_offset),
        (3, 8),
        "the anchor must follow its text across a persisted edit"
    );

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn deleting_the_anchored_text_leaves_an_orphan() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;

    let before = comments::load_doc(&db, doc).await.unwrap();
    let anchor = before.sticky_for_range("a", 0, "a", 5).unwrap();
    let (thread, _) = comments::create_thread(&db, doc, user, &anchor, "Hello", "note")
        .await
        .unwrap();

    let mut editing = comments::load_doc(&db, doc).await.unwrap();
    let sv = editing.state_vector();
    editing.text_delete("a", 0, 5); // delete "Hello"
    let update = editing.encode_diff(&sv).unwrap();
    sync::push_update(&db, ws, doc, user, &update, &sync::SyncTuning::default()).await.unwrap();

    let after = comments::load_doc(&db, doc).await.unwrap();
    let live = after
        .resolve_range(&anchor)
        .filter(|range| !range.is_empty());
    assert!(
        live.is_none(),
        "the anchored text is gone — unresolvable or collapsed, both are orphans"
    );

    // That is what the listing endpoint acts on.
    comments::mark_orphaned(&db, &[thread.id]).await.unwrap();
    let reread = comments::fetch_thread(&db, thread.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reread.status, comments::STATUS_ORPHANED);

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn resolving_keeps_the_anchor_and_reopening_clears_the_stamps() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;
    let live = comments::load_doc(&db, doc).await.unwrap();
    let anchor = live.sticky_for_range("a", 0, "a", 5).unwrap();
    let (thread, _) = comments::create_thread(&db, doc, user, &anchor, "Hello", "note")
        .await
        .unwrap();

    comments::set_resolved(&db, thread.id, user, true)
        .await
        .unwrap();
    let done = comments::fetch_thread(&db, thread.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, comments::STATUS_RESOLVED);
    assert_eq!(done.resolved_by, Some(user));
    assert!(done.resolved_at.is_some());
    // The anchor is KEPT so "show resolved" can highlight it again.
    assert_eq!(done.anchor_start_sticky, thread.anchor_start_sticky);
    assert!(live.resolve_range(&done.anchor()).is_some());

    comments::set_resolved(&db, thread.id, user, false)
        .await
        .unwrap();
    let reopened = comments::fetch_thread(&db, thread.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reopened.status, comments::STATUS_OPEN);
    assert!(reopened.resolved_by.is_none());
    assert!(reopened.resolved_at.is_none());

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn mark_orphaned_never_overwrites_a_deliberate_resolve() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;
    let live = comments::load_doc(&db, doc).await.unwrap();
    let anchor = live.sticky_for_range("a", 0, "a", 5).unwrap();
    let (thread, _) = comments::create_thread(&db, doc, user, &anchor, "Hello", "note")
        .await
        .unwrap();
    comments::set_resolved(&db, thread.id, user, true)
        .await
        .unwrap();

    comments::mark_orphaned(&db, &[thread.id]).await.unwrap();

    let still = comments::fetch_thread(&db, thread.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        still.status,
        comments::STATUS_RESOLVED,
        "orphan bookkeeping is for open threads only"
    );

    cleanup(&db, ws, user).await;
}

/// Rewrite the whole document through the sync path, exactly as a REST or MCP
/// write does (`apply_derived_operations` → `set_blocks` → `push_update`).
///
/// `edit` gets the current blocks and returns the blocks to write, so a test can
/// rewrite with IDENTICAL content — which is the point: the text does not change,
/// yet every block's yrs text object is replaced and every anchor on the document
/// dies with it.
async fn rewrite_document(
    db: &PgPool,
    ws: Uuid,
    doc: Uuid,
    user: Uuid,
    edit: impl FnOnce(Vec<mica_core::Block>) -> Vec<mica_core::Block>,
) {
    let mut editing = comments::load_doc(db, doc).await.unwrap();
    let sv = editing.state_vector();
    let blocks = edit(editing.to_blocks());
    editing.set_blocks("r", &blocks);
    let update = editing.encode_diff(&sv).unwrap();
    sync::push_update(db, ws, doc, user, &update, &sync::SyncTuning::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_full_rewrite_kills_the_anchor_and_the_quote_re_anchors_the_thread() {
    // The case Phase 2 ① exists for, end to end through Postgres: any REST/MCP
    // write rewrites every block, so a thread orphans over text that never
    // changed. Matching the saved quote against the reloaded document is what
    // tells that apart from a real deletion.
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;

    let live = comments::load_doc(&db, doc).await.unwrap();
    let anchor = live.sticky_for_range("a", 0, "a", 5).unwrap();
    let (thread, _) = comments::create_thread(&db, doc, user, &anchor, "Hello", "note")
        .await
        .unwrap();

    // Same content, new text objects — plus a paragraph after it, so the match
    // has to be found rather than assumed to be at offset 0 of the first block.
    rewrite_document(&db, ws, doc, user, |mut blocks| {
        blocks[0].children.push("c".into());
        blocks.push(mica_core::Block::new("c", "paragraph").with_text("Goodbye"));
        blocks
    })
    .await;

    let after = comments::load_doc(&db, doc).await.unwrap();
    let stored = comments::fetch_thread(&db, thread.id).await.unwrap().unwrap();
    assert!(
        after
            .resolve_range(&stored.anchor())
            .filter(|r| !r.is_empty())
            .is_none(),
        "the rewrite must have killed the anchor — otherwise this test proves nothing"
    );

    let index = comments::QuoteIndex::from_blocks(&after.to_blocks());
    let (range, fresh) = comments::rematch(&after, &index, &stored.quote, &stored.anchor_start_block)
        .expect("the quoted text is still in the document");
    assert_eq!(range.start_block, "a");
    assert_eq!((range.start_offset, range.end_offset), (0, 5));

    comments::reanchor(&db, thread.id, &fresh).await.unwrap();

    // Persisted, and the stored bytes resolve against the live document again.
    let reread = comments::fetch_thread(&db, thread.id).await.unwrap().unwrap();
    assert_ne!(
        reread.anchor_start_sticky, stored.anchor_start_sticky,
        "the new sticky bytes must have replaced the dead ones"
    );
    let back = after
        .resolve_range(&reread.anchor())
        .expect("the persisted anchor resolves");
    assert_eq!((back.start_offset, back.end_offset), (0, 5));

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn re_anchoring_reopens_an_orphan_but_leaves_a_resolved_thread_resolved() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;
    let live = comments::load_doc(&db, doc).await.unwrap();
    let anchor = live.sticky_for_range("a", 0, "a", 5).unwrap();
    let (orphan, _) = comments::create_thread(&db, doc, user, &anchor, "Hello", "note")
        .await
        .unwrap();
    let (done, _) = comments::create_thread(&db, doc, user, &anchor, "Hello", "settled")
        .await
        .unwrap();
    comments::mark_orphaned(&db, &[orphan.id]).await.unwrap();
    comments::set_resolved(&db, done.id, user, true).await.unwrap();

    let after = comments::load_doc(&db, doc).await.unwrap();
    let index = comments::QuoteIndex::from_blocks(&after.to_blocks());
    let (_, fresh) = comments::rematch(&after, &index, "Hello", "a").unwrap();
    comments::reanchor(&db, orphan.id, &fresh).await.unwrap();
    comments::reanchor(&db, done.id, &fresh).await.unwrap();

    let orphan = comments::fetch_thread(&db, orphan.id).await.unwrap().unwrap();
    assert_eq!(
        orphan.status,
        comments::STATUS_OPEN,
        "it has text under it again"
    );
    let done = comments::fetch_thread(&db, done.id).await.unwrap().unwrap();
    assert_eq!(
        done.status,
        comments::STATUS_RESOLVED,
        "re-anchoring is bookkeeping — it must not re-open a finished discussion"
    );

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn a_thread_whose_text_was_really_deleted_is_not_re_anchored() {
    // The other half of the previous test: when the words are gone, nothing may
    // be matched — a wrong anchor is worse than an orphan.
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;
    let live = comments::load_doc(&db, doc).await.unwrap();
    let anchor = live.sticky_for_range("a", 0, "a", 5).unwrap();
    let (thread, _) = comments::create_thread(&db, doc, user, &anchor, "Hello", "note")
        .await
        .unwrap();

    rewrite_document(&db, ws, doc, user, |mut blocks| {
        for b in blocks.iter_mut() {
            if b.id == "a" {
                b.text = "Something else entirely".to_string();
            }
        }
        blocks
    })
    .await;

    let after = comments::load_doc(&db, doc).await.unwrap();
    let stored = comments::fetch_thread(&db, thread.id).await.unwrap().unwrap();
    let index = comments::QuoteIndex::from_blocks(&after.to_blocks());
    assert!(
        comments::rematch(&after, &index, &stored.quote, &stored.anchor_start_block).is_none(),
        "the quoted text is gone — the thread stays an orphan"
    );

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn replies_are_ordered_and_deleting_a_thread_cascades() {
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;
    let live = comments::load_doc(&db, doc).await.unwrap();
    let anchor = live.sticky_for_range("a", 0, "a", 5).unwrap();
    let (thread, _) = comments::create_thread(&db, doc, user, &anchor, "Hello", "first")
        .await
        .unwrap();
    comments::add_reply(&db, thread.id, user, "second")
        .await
        .unwrap();
    comments::add_reply(&db, thread.id, user, "third")
        .await
        .unwrap();

    let bodies: Vec<String> = comments::list_comments(&db, &[thread.id])
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.body)
        .collect();
    assert_eq!(bodies, vec!["first", "second", "third"]);

    comments::delete_thread(&db, thread.id).await.unwrap();
    assert!(comments::fetch_thread(&db, thread.id)
        .await
        .unwrap()
        .is_none());
    let orphaned_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM comments WHERE thread_id = $1")
            .bind(thread.id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(orphaned_rows, 0, "replies must cascade with their thread");

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn the_status_check_constraint_rejects_an_unknown_state() {
    // The status vocabulary is enforced by the database, not just by Rust — a bad
    // write from anywhere (a migration, a manual fix) cannot leave a row the
    // client has no rendering for.
    let Some(db) = pool().await else { return };
    let (ws, doc, user) = seed_doc(&db).await;
    let live = comments::load_doc(&db, doc).await.unwrap();
    let anchor = live.sticky_for_range("a", 0, "a", 5).unwrap();
    let (thread, _) = comments::create_thread(&db, doc, user, &anchor, "Hello", "note")
        .await
        .unwrap();

    let bad = sqlx::query("UPDATE comment_threads SET status = 'nonsense' WHERE id = $1")
        .bind(thread.id)
        .execute(&db)
        .await;
    assert!(bad.is_err(), "CHECK (status IN (...)) must reject it");

    cleanup(&db, ws, user).await;
}

#[tokio::test]
async fn listing_comments_for_no_threads_is_empty_without_a_query() {
    let Some(db) = pool().await else { return };
    assert!(comments::list_comments(&db, &[]).await.unwrap().is_empty());
}
