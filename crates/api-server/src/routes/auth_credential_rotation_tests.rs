//! #123: credential rotation and credential issuing serialize on the user row.
//!
//! The epoch RULE (an older JWT cannot mint a PAT) is covered in `refresh_pg`.
//! This is the race around it: a PAT mint that starts while a rotation is still
//! uncommitted must wait for it and then judge against the NEW epoch — not read
//! the old one, pass, and leave a long-lived credential minted from a token the
//! rotation was meant to kill.
//!
//! (Rewritten 2026-10-26: the first version of this file was hidden by a
//! `*credential*` .gitignore rule and never reached the repository.)

use super::*;
use sqlx::PgPool;
use std::time::Duration;

async fn pool() -> Option<PgPool> {
  let url = mica_infra::test_database_url()?;
  let db = PgPool::connect(&url)
    .await
    .expect("DATABASE_URL is set but the connection failed");
  mica_infra::run_migrations(&db).await.unwrap();
  Some(db)
}

async fn seed_user(db: &PgPool) -> Uuid {
  let user = Uuid::new_v4();
  sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
    .bind(user)
    .bind(format!("{user}@rotation.test"))
    .execute(db)
    .await
    .unwrap();
  user
}

/// A PAT mint racing an uncommitted rotation blocks on the row lock, then
/// refuses the JWT the rotation made stale.
#[tokio::test]
async fn a_pat_mint_waits_for_a_pending_rotation_and_sees_its_epoch() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;

  // The rotation: lock the row and bump the epoch, but do not commit yet.
  let mut rotation = db.begin().await.unwrap();
  lock_user_row(&mut rotation, user).await.unwrap();
  sqlx::query("UPDATE users SET credential_epoch = 500 WHERE id = $1")
    .bind(user)
    .execute(&mut *rotation)
    .await
    .unwrap();

  // A JWT issued before the rotation tries to mint meanwhile.
  let mint = tokio::spawn({
    let db = db.clone();
    async move {
      let mut tx = db.begin().await.unwrap();
      ensure_pat_mint_allowed_in_tx(&mut tx, user, Some(499)).await
    }
  });
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert!(
    !mint.is_finished(),
    "the mint must block on the rotation's row lock, not read the old epoch"
  );

  rotation.commit().await.unwrap();
  let verdict = tokio::time::timeout(Duration::from_secs(10), mint)
    .await
    .expect("the mint must proceed once the rotation commits")
    .unwrap();
  assert!(
    matches!(verdict, Err(ApiError::Unauthorized)),
    "after the rotation commits, the stale JWT is refused: {verdict:?}"
  );
}

/// The same race, the other way round: a rotation that rolls back leaves the
/// old epoch in force, so the waiting mint goes through.
#[tokio::test]
async fn a_rolled_back_rotation_releases_the_mint_unchanged() {
  let Some(db) = pool().await else { return };
  let user = seed_user(&db).await;

  let mut rotation = db.begin().await.unwrap();
  lock_user_row(&mut rotation, user).await.unwrap();
  sqlx::query("UPDATE users SET credential_epoch = 500 WHERE id = $1")
    .bind(user)
    .execute(&mut *rotation)
    .await
    .unwrap();

  let mint = tokio::spawn({
    let db = db.clone();
    async move {
      let mut tx = db.begin().await.unwrap();
      ensure_pat_mint_allowed_in_tx(&mut tx, user, Some(499)).await
    }
  });
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert!(!mint.is_finished());

  rotation.rollback().await.unwrap();
  let verdict = tokio::time::timeout(Duration::from_secs(10), mint)
    .await
    .expect("the mint must proceed once the rotation rolls back")
    .unwrap();
  assert_eq!(verdict.unwrap(), user);
}
