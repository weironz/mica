//! P2-M4.4 Postgres integration test for the yrs sync queries. Gated on
//! `DATABASE_URL` — skipped (passes) when unset so CI without a DB is unaffected.
//!
//!   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//!   cargo test -p mica-app-core --test sync_pg

use mica_app_core::sync;
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
