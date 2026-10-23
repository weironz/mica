//! Connection-local tables allow failure injection without modifying shared data.
use super::*;
use sqlx::{PgPool, postgres::PgPoolOptions};

async fn fixture() -> Option<(PgPool, Uuid, String)> {
  let Ok(url) = std::env::var("DATABASE_URL") else {
    assert!(
      std::env::var("CI").is_err(),
      "DATABASE_URL must be set in CI"
    );
    return None;
  };
  let db = PgPoolOptions::new()
    .max_connections(1)
    .connect(&url)
    .await
    .unwrap();
  sqlx::raw_sql(
    "CREATE TEMP TABLE users (id uuid PRIMARY KEY, email_verified_at timestamptz, deleted_at timestamptz,
       CONSTRAINT fail_verification CHECK (email_verified_at IS NULL));
     CREATE TEMP TABLE email_verification_tokens (token_hash text PRIMARY KEY, user_id uuid, expires_at timestamptz, used_at timestamptz);",
  ).execute(&db).await.unwrap();
  let user = Uuid::new_v4();
  sqlx::query("INSERT INTO users (id) VALUES ($1)")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  let token = mint_token(&db, user).await.unwrap();
  Some((db, user, token))
}

#[tokio::test]
async fn failed_verification_keeps_token_retryable() {
  let Some((db, user, token)) = fixture().await else {
    return;
  };
  assert!(consume_and_verify(&db, &token).await.is_err());
  let unused: bool =
    sqlx::query_scalar("SELECT used_at IS NULL FROM email_verification_tokens WHERE user_id=$1")
      .bind(user)
      .fetch_one(&db)
      .await
      .unwrap();
  assert!(unused);
  sqlx::query("ALTER TABLE pg_temp.users DROP CONSTRAINT fail_verification")
    .execute(&db)
    .await
    .unwrap();
  assert!(consume_and_verify(&db, &token).await.unwrap());
  assert!(!consume_and_verify(&db, &token).await.unwrap());
  let verified: bool =
    sqlx::query_scalar("SELECT email_verified_at IS NOT NULL FROM users WHERE id=$1")
      .bind(user)
      .fetch_one(&db)
      .await
      .unwrap();
  assert!(verified);
}

#[tokio::test]
async fn tombstoned_user_cannot_verify_or_spend_link() {
  let Some((db, user, token)) = fixture().await else {
    return;
  };
  sqlx::query("UPDATE users SET deleted_at=now() WHERE id=$1")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  assert!(!consume_and_verify(&db, &token).await.unwrap());
  let unused: bool =
    sqlx::query_scalar("SELECT used_at IS NULL FROM email_verification_tokens WHERE user_id=$1")
      .bind(user)
      .fetch_one(&db)
      .await
      .unwrap();
  assert!(unused);
}
