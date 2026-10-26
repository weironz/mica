//! Postgres integration test for the #107 close-the-loop half the unit tests
//! cannot see: the LIST route must not just REPORT a re-opened thread, it must
//! persist the derivation back into `comment_threads` so the stored status stops
//! contradicting the document.
//!
//! Flow under test: delete the anchored text → list (persists `orphaned`) → undo
//! (text restored, anchor live and non-empty again) → list → the response says
//! `open` AND the row says `open`.
//!
//! Gated on `DATABASE_URL`, hardened the same way as the other route tests:
//! skipped locally without a database, but a set-but-unreachable URL — or a
//! missing one in CI — is a hard failure.
//!
//!   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//!   cargo test -p mica-api-server comments_pg_tests

use super::comments::{ThreadDto, ThreadsResponse, list};
use axum::{
  Json,
  extract::{Path, State},
  http::{HeaderMap, header::AUTHORIZATION},
};
use mica_app_core::{AppState, comments, sync};
use mica_infra::{Environment, LogMailer};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const JWT_SECRET: &str = "comments-pg-test-secret";

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

fn test_state(db: &PgPool) -> AppState {
  let mut config = mica_infra::AppConfig::from_env().unwrap();
  config.jwt_secret = JWT_SECRET.into();
  config.environment = Environment::Test;
  AppState::new(config, db.clone(), std::sync::Arc::new(LogMailer))
}

/// A real JWT, minted the way `auth_response` does, so `user_id_from_headers`
/// resolves [user] from the bearer header.
fn bearer_headers(user: Uuid) -> HeaderMap {
  #[derive(serde::Serialize)]
  struct TestClaims {
    sub: String,
    exp: usize,
  }
  let claims = TestClaims {
    sub: user.to_string(),
    exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
  };
  let token = jsonwebtoken::encode(
    &jsonwebtoken::Header::default(),
    &claims,
    &jsonwebtoken::EncodingKey::from_secret(JWT_SECRET.as_bytes()),
  )
  .unwrap();
  let mut headers = HeaderMap::new();
  headers.insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
  headers
}

/// user → workspace (+ membership) → document with block `a` = "hello world".
async fn seed(db: &PgPool) -> (Uuid, Uuid, Uuid) {
  let user = Uuid::new_v4();
  let ws = Uuid::new_v4();
  let doc = Uuid::new_v4();
  sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
    .bind(user)
    .bind(format!("{user}@comments-pg.test"))
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
    "INSERT INTO workspace_members(workspace_id,user_id,role,position) VALUES($1,$2,'owner',$3)",
  )
  .bind(ws)
  .bind(user)
  .bind(crate::routes::workspaces::pad_position(1))
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
          {"id":"a","type":"paragraph","text":"hello world"}
      ]
  }))
  .unwrap();
  let mut tx = db.begin().await.unwrap();
  sync::seed_base_tx(&mut tx, doc, payload).await.unwrap();
  tx.commit().await.unwrap();
  (ws, doc, user)
}

async fn stored_status(db: &PgPool, thread: Uuid) -> String {
  sqlx::query_scalar::<_, String>("SELECT status FROM comment_threads WHERE id = $1")
    .bind(thread)
    .fetch_one(db)
    .await
    .unwrap()
}

/// The thread in a listing response (there is exactly one).
fn only_thread(resp: &ThreadsResponse) -> &ThreadDto {
  assert_eq!(resp.threads.len(), 1);
  &resp.threads[0]
}

/// Delete anchored text → list orphans it → undo → list re-opens it AND writes
/// that back to the row.
#[tokio::test]
async fn listing_undo_reopens_and_persists_the_thread() {
  let Some(db) = pool().await else { return };
  let (ws, doc, user) = seed(&db).await;

  let live = comments::load_doc(&db, doc).await.unwrap();
  let anchor = live.sticky_for_range("a", 6, "a", 11).unwrap();
  let (thread, _) = comments::create_thread(&db, doc, user, &anchor, "world", "note")
    .await
    .unwrap();
  assert_eq!(stored_status(&db, thread.id).await, comments::STATUS_OPEN);

  // Delete "world" and list: the route derives + persists the orphan.
  let mut editing = comments::load_doc(&db, doc).await.unwrap();
  let sv = editing.state_vector();
  editing.text_delete("a", 6, 5);
  sync::push_update(
    &db,
    ws,
    doc,
    user,
    &editing.encode_diff(&sv).unwrap(),
    &sync::SyncTuning::default(),
  )
  .await
  .unwrap();
  let Json(orphaned) = list(
    State(test_state(&db)),
    bearer_headers(user),
    Path((ws, doc)),
  )
  .await
  .unwrap();
  assert_eq!(only_thread(&orphaned).status, comments::STATUS_ORPHANED);
  assert_eq!(
    stored_status(&db, thread.id).await,
    comments::STATUS_ORPHANED
  );

  // Undo: restore the text. yrs' UndoManager (which re-inserts the original
  // items, leaving the sticky a live but partial range) is not exposed through
  // MicaDoc's public API, so the undo's end state is reproduced directly: text
  // restored AND the thread's sticky re-pointed at the restored text — live,
  // non-empty — while the stored status is still the `orphaned` the first
  // listing wrote. That is exactly the state the bug was filed on.
  let mut restored = comments::load_doc(&db, doc).await.unwrap();
  let sv = restored.state_vector();
  restored.text_insert("a", 6, "world");
  sync::push_update(
    &db,
    ws,
    doc,
    user,
    &restored.encode_diff(&sv).unwrap(),
    &sync::SyncTuning::default(),
  )
  .await
  .unwrap();
  let after = comments::load_doc(&db, doc).await.unwrap();
  let live_anchor = after.sticky_for_range("a", 6, "a", 11).unwrap();
  sqlx::query(
    "UPDATE comment_threads SET anchor_start_sticky = $2, anchor_end_sticky = $3 WHERE id = $1",
  )
  .bind(thread.id)
  .bind(&live_anchor.start_sticky)
  .bind(&live_anchor.end_sticky)
  .execute(&db)
  .await
  .unwrap();

  // The second listing: the response is open, and so is the row now.
  let Json(revived) = list(
    State(test_state(&db)),
    bearer_headers(user),
    Path((ws, doc)),
  )
  .await
  .unwrap();
  assert_eq!(only_thread(&revived).status, comments::STATUS_OPEN);
  assert_eq!(stored_status(&db, thread.id).await, comments::STATUS_OPEN);

  cleanup(&db, ws, user).await;
}

/// The same undo, but for a thread the user had deliberately resolved: the
/// guarded write must leave `resolved` alone — text coming back is bookkeeping,
/// not a reason to re-open a finished discussion.
#[tokio::test]
async fn listing_undo_never_reopens_a_resolved_thread() {
  let Some(db) = pool().await else { return };
  let (ws, doc, user) = seed(&db).await;

  let live = comments::load_doc(&db, doc).await.unwrap();
  let anchor = live.sticky_for_range("a", 6, "a", 11).unwrap();
  let (thread, _) = comments::create_thread(&db, doc, user, &anchor, "world", "note")
    .await
    .unwrap();
  comments::set_resolved(&db, thread.id, user, true)
    .await
    .unwrap();
  assert_eq!(
    stored_status(&db, thread.id).await,
    comments::STATUS_RESOLVED
  );

  // Text deleted, then restored — the same undo stand-in as the test above.
  let mut editing = comments::load_doc(&db, doc).await.unwrap();
  let sv = editing.state_vector();
  editing.text_delete("a", 6, 5);
  sync::push_update(
    &db,
    ws,
    doc,
    user,
    &editing.encode_diff(&sv).unwrap(),
    &sync::SyncTuning::default(),
  )
  .await
  .unwrap();
  let mut restored = comments::load_doc(&db, doc).await.unwrap();
  let sv = restored.state_vector();
  restored.text_insert("a", 6, "world");
  sync::push_update(
    &db,
    ws,
    doc,
    user,
    &restored.encode_diff(&sv).unwrap(),
    &sync::SyncTuning::default(),
  )
  .await
  .unwrap();
  let after = comments::load_doc(&db, doc).await.unwrap();
  let live_anchor = after.sticky_for_range("a", 6, "a", 11).unwrap();
  sqlx::query(
    "UPDATE comment_threads SET anchor_start_sticky = $2, anchor_end_sticky = $3 WHERE id = $1",
  )
  .bind(thread.id)
  .bind(&live_anchor.start_sticky)
  .bind(&live_anchor.end_sticky)
  .execute(&db)
  .await
  .unwrap();

  let Json(revived) = list(
    State(test_state(&db)),
    bearer_headers(user),
    Path((ws, doc)),
  )
  .await
  .unwrap();
  assert_eq!(only_thread(&revived).status, comments::STATUS_RESOLVED);
  assert_eq!(
    stored_status(&db, thread.id).await,
    comments::STATUS_RESOLVED
  );

  cleanup(&db, ws, user).await;
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
