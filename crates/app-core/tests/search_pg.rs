//! Postgres integration tests for the search index's refresh — the cursor and
//! the arena reservation are both decided by `BodyIndex::refresh`, against real
//! rows, so they are pinned here rather than with an internal fake.
//!
//! Gated on `DATABASE_URL`, with the same anti-"vacuum pass" rule as
//! `sync_pg.rs`: a set-but-unusable url panics, and a MISSING url panics in CI
//! (where the database is always provisioned), so these can never silently stop
//! running.
//!
//!   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//!   cargo test -p mica-app-core --test search_pg

use std::{
  str::FromStr,
  sync::Arc,
  time::{Duration, Instant},
};

use mica_app_core::search::BodyIndex;
use sqlx::{
  PgPool,
  postgres::{PgConnectOptions, PgPoolOptions},
};
use uuid::Uuid;

/// Both tests do a full-table load into their own index, so they must not run
/// against the same database at once — one's rows would land in the other's
/// index. Serializing them (and wiping the tables they share) keeps them
/// honest without asking the runner for `--test-threads=1`.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Remove every row the tests create, in FK order. Called at START too, so a
/// test that failed (and therefore never cleaned up) cannot leak rows into the
/// next one.
async fn wipe(db: &PgPool) {
  sqlx::query("DELETE FROM document_yrs_base")
    .execute(db)
    .await
    .ok();
  sqlx::query("DELETE FROM documents").execute(db).await.ok();
  sqlx::query("DELETE FROM workspaces").execute(db).await.ok();
  sqlx::query("DELETE FROM users").execute(db).await.ok();
}

/// How long the stand-in for a slow read blocks for. It must exceed the
/// refresh lap (5 s) plus the moment the concurrent write lands, or the cursor
/// the old code took afterwards would still sit below that write.
const SLOW_READ: Duration = Duration::from_secs(8);

fn database_url() -> Option<String> {
  let Ok(url) = std::env::var("DATABASE_URL") else {
    assert!(
      std::env::var("CI").is_err(),
      "DATABASE_URL is unset in CI — the postgres service block regressed; \
             these tests must not silently pass"
    );
    return None;
  };
  Some(url)
}

/// A pool on `search_path`, first entry first. `None` keeps the database
/// default (`public`).
async fn pool(url: &str, search_path: Option<&str>) -> PgPool {
  let mut options = PgConnectOptions::from_str(url).expect("DATABASE_URL must parse");
  if let Some(path) = search_path {
    options = options.options([("search_path", path)]);
  }
  PgPoolOptions::new()
    .max_connections(2)
    .connect_with(options)
    .await
    .expect("DATABASE_URL is set but the connection failed")
}

/// user → workspace → document → one `document_yrs_base` row whose
/// `content_text` is `body`. Returns (workspace, document, user).
async fn seed_document(db: &PgPool, body: &str) -> (Uuid, Uuid, Uuid) {
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
  write_body(db, doc, body).await;
  (ws, doc, user)
}

/// A normal committed write of a body — the same statement shape the sync path
/// uses, so `updated_at` is the writing statement's `clock_timestamp()`.
async fn write_body(db: &PgPool, doc: Uuid, body: &str) {
  sqlx::query(
    "INSERT INTO document_yrs_base(document_id, state, state_vector, content_text, updated_at)
         VALUES($1, ''::bytea, ''::bytea, $2, clock_timestamp())
         ON CONFLICT (document_id) DO UPDATE
         SET content_text = EXCLUDED.content_text, updated_at = clock_timestamp()",
  )
  .bind(doc)
  .bind(body)
  .execute(db)
  .await
  .unwrap();
}

async fn cleanup(db: &PgPool, ws: Uuid, user: Uuid) {
  // workspaces cascade to documents → yrs_base.
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

/// **Issue #105.** The refresh cursor must come from the same snapshot as the
/// rows it advances past. Taking a clock read AFTER the SELECT puts the cursor
/// beyond every row committed while the SELECT was running, and those rows are
/// then missed by the refresh that is supposed to fetch them and by every one
/// after it (an idle refresh does not move the cursor at all).
///
/// The slow read is a view shadowing `document_yrs_base` for one pool: its fix
/// is a `pg_sleep` that runs AFTER the statement's snapshot, exactly the shape
/// of a cold-cache or pool-starved SELECT that takes seconds.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_write_committed_during_a_slow_refresh_is_still_indexed() {
  let _guard = SERIAL.lock().await;
  let Some(url) = database_url() else {
    return;
  };
  let db = pool(&url, None).await;
  wipe(&db).await;
  let (ws, doc, user) = seed_document(&db, "oldmarker").await;
  let index = Arc::new(BodyIndex::new(false));
  index.refresh(&db).await.unwrap();
  assert_eq!(index.matching_ids("oldmarker").await, vec![doc]);
  // The view is created only now: the full load above is an ordinary read, and
  // the shadow must outlive the slow refresh only.
  let schema = format!("search_slow_{}", doc.simple());
  sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
    .execute(&db)
    .await
    .unwrap();
  sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
    "CREATE VIEW {schema}.document_yrs_base AS
       SELECT b.document_id, b.content_text, b.updated_at
       FROM public.document_yrs_base b,
            LATERAL (SELECT pg_sleep({}) AS nap) slow",
    SLOW_READ.as_secs_f64()
  )))
  .execute(&db)
  .await
  .unwrap();
  let slow_db = pool(&url, Some(&format!("{schema},public"))).await;
  // Connect before the write lands, so the ONLY delay in the refresh below is
  // the read itself.
  sqlx::query("SELECT 1").execute(&slow_db).await.unwrap();

  let refreshing = Arc::clone(&index);
  let slow_db_for_task = slow_db.clone();
  let slow_refresh = tokio::spawn(async move { refreshing.refresh(&slow_db_for_task).await });

  tokio::time::sleep(Duration::from_secs(1)).await;
  write_body(&db, doc, "newmarker").await;
  slow_refresh
    .await
    .unwrap()
    .expect("the slow refresh itself succeeds");

  // The repro is only real if that read's snapshot really predates the write.
  assert!(
    index.matching_ids("newmarker").await.is_empty(),
    "the slow read started after the write — nothing is being raced"
  );

  sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
    .execute(&db)
    .await
    .unwrap();

  // The cursor must still sit below the write, so the next refresh picks it up
  // — and keeps picking it up without duplicating or regressing.
  for attempt in 1..=3 {
    index.refresh(&db).await.unwrap();
    assert_eq!(
      index.matching_ids("newmarker").await,
      vec![doc],
      "refresh {attempt} after the slow one must see the committed write"
    );
    assert!(
      index.matching_ids("oldmarker").await.is_empty(),
      "the replaced body must be gone"
    );
  }

  cleanup(&db, ws, user).await;
}

/// **Issue #106.** A refresh that replaces a body with the same text must not
/// grow the reservation: the bytes it copies in are the bytes it makes dead.
/// Sizing the regrow to `live + incoming` (before the replaced body is dead)
/// left ~2x the live corpus reserved after any overlap refresh.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unchanged_overlap_refresh_keeps_the_reservation_flat() {
  let _guard = SERIAL.lock().await;
  let Some(url) = database_url() else {
    return;
  };
  let db = pool(&url, None).await;
  wipe(&db).await;
  let body = "a".repeat(8 * 1024 * 1024);
  let (ws, doc, user) = seed_document(&db, &body).await;
  let written = Instant::now();

  let index = BodyIndex::new(false);
  index.refresh(&db).await.unwrap();
  let (live_first, capacity_first, _) = index.footprint().await;
  assert_eq!(live_first, body.len(), "the body is indexed once");
  assert_eq!(index.indexed_documents().await, 1);

  // The 5 s lap re-reads a body written moments ago, so this refresh really
  // replaces it. Outside the lap the refresh is idle and proves nothing.
  assert!(
    written.elapsed() < Duration::from_secs(4),
    "the write fell outside the refresh lap — the second refresh would be idle"
  );
  index.refresh(&db).await.unwrap();
  let (live_second, capacity_second, _) = index.footprint().await;

  assert_eq!(live_second, body.len(), "still one live copy");
  assert_eq!(
    index.matching_ids("aaaa").await,
    vec![doc],
    "the body is still searchable after being replaced"
  );
  assert!(
    capacity_second <= capacity_first,
    "reservation grew from {capacity_first} to {capacity_second} bytes to hold \
     {live_second} live bytes — the replaced body was sized as if both copies live"
  );

  cleanup(&db, ws, user).await;
}
