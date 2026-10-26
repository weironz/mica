//! Failure injection uses connection-local temporary tables, never production rows.
use super::*;
use sqlx::{PgPool, postgres::PgPoolOptions};

async fn fixture() -> Option<PgPool> {
  let Some(url) = mica_infra::test_database_url() else {
    return None;
  };
  let db = PgPoolOptions::new()
    .max_connections(1)
    .connect(&url)
    .await
    .unwrap();
  sqlx::raw_sql(
    "CREATE TEMP TABLE users (id uuid PRIMARY KEY, password_hash text, updated_at timestamptz, deleted_at timestamptz, credential_epoch bigint DEFAULT 0);
     CREATE TEMP TABLE password_reset_tokens (token_hash text PRIMARY KEY, user_id uuid, expires_at timestamptz, used_at timestamptz);
     CREATE TEMP TABLE refresh_tokens (user_id uuid, revoked_at timestamptz CHECK (revoked_at IS NULL));
     CREATE TEMP TABLE api_tokens (user_id uuid);",
  ).execute(&db).await.unwrap();
  Some(db)
}

#[tokio::test]
async fn session_revoke_failure_rolls_back_password_and_token() {
  let Some(db) = fixture().await else { return };
  let user = Uuid::new_v4();
  sqlx::query("INSERT INTO users (id, password_hash) VALUES ($1, 'old-hash')")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  sqlx::query("INSERT INTO refresh_tokens (user_id) VALUES ($1)")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  let token = mint_reset_token(&db, user).await.unwrap();
  assert!(matches!(
    consume_and_reset(&db, &token, "new-password").await,
    Err(ResetError::Internal)
  ));
  let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1")
    .bind(user)
    .fetch_one(&db)
    .await
    .unwrap();
  assert_eq!(hash, "old-hash");
  let unused: bool =
    sqlx::query_scalar("SELECT used_at IS NULL FROM password_reset_tokens WHERE user_id=$1")
      .bind(user)
      .fetch_one(&db)
      .await
      .unwrap();
  assert!(unused, "a failed reset must leave its link retryable");
  sqlx::query("DELETE FROM refresh_tokens")
    .execute(&db)
    .await
    .unwrap();
  consume_and_reset(&db, &token, "new-password")
    .await
    .unwrap();
}

#[tokio::test]
async fn api_token_revoke_failure_rolls_back_every_reset_write() {
  let Some(db) = fixture().await else { return };
  // Let refresh-token revocation succeed, then fail the final PAT deletion.
  sqlx::raw_sql(
    "ALTER TABLE pg_temp.refresh_tokens DROP CONSTRAINT refresh_tokens_revoked_at_check;
     CREATE FUNCTION pg_temp.reject_pat_delete() RETURNS trigger LANGUAGE plpgsql AS $$
       BEGIN RAISE EXCEPTION 'injected PAT deletion failure'; END $$;
     CREATE TRIGGER reject_pat_delete BEFORE DELETE ON pg_temp.api_tokens
       FOR EACH ROW EXECUTE FUNCTION pg_temp.reject_pat_delete();",
  )
  .execute(&db)
  .await
  .unwrap();
  let user = Uuid::new_v4();
  sqlx::query("INSERT INTO users (id, password_hash) VALUES ($1, 'old-hash')")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  sqlx::query("INSERT INTO refresh_tokens (user_id) VALUES ($1)")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  sqlx::query("INSERT INTO api_tokens (user_id) VALUES ($1)")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  let token = mint_reset_token(&db, user).await.unwrap();
  assert!(matches!(
    consume_and_reset(&db, &token, "new-password").await,
    Err(ResetError::Internal)
  ));
  let unchanged: bool = sqlx::query_scalar(
    "SELECT u.password_hash = 'old-hash' AND t.used_at IS NULL
       AND r.revoked_at IS NULL AND EXISTS (SELECT 1 FROM api_tokens WHERE user_id = u.id)
     FROM users u JOIN password_reset_tokens t ON t.user_id = u.id
       JOIN refresh_tokens r ON r.user_id = u.id WHERE u.id = $1",
  )
  .bind(user)
  .fetch_one(&db)
  .await
  .unwrap();
  assert!(
    unchanged,
    "the last failed write must roll back all preceding writes"
  );
  sqlx::query("DROP TRIGGER reject_pat_delete ON pg_temp.api_tokens")
    .execute(&db)
    .await
    .unwrap();
  consume_and_reset(&db, &token, "new-password")
    .await
    .unwrap();
  let revoked: bool = sqlx::query_scalar(
    "SELECT r.revoked_at IS NOT NULL AND NOT EXISTS (SELECT 1 FROM api_tokens WHERE user_id = r.user_id)
     FROM refresh_tokens r WHERE r.user_id = $1",
  ).bind(user).fetch_one(&db).await.unwrap();
  assert!(revoked);
}

#[tokio::test]
async fn tombstoned_user_cannot_reset_password_or_spend_link() {
  let Some(db) = fixture().await else { return };
  let user = Uuid::new_v4();
  sqlx::query("INSERT INTO users (id, password_hash, deleted_at) VALUES ($1, 'old-hash', now())")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  let token = mint_reset_token(&db, user).await.unwrap();
  assert!(matches!(
    consume_and_reset(&db, &token, "new-password").await,
    Err(ResetError::Invalid)
  ));
  let unused: bool =
    sqlx::query_scalar("SELECT used_at IS NULL FROM password_reset_tokens WHERE user_id=$1")
      .bind(user)
      .fetch_one(&db)
      .await
      .unwrap();
  assert!(unused);
}
