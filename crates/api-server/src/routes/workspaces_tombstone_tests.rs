use super::*;
use sqlx::postgres::PgPoolOptions;

#[tokio::test]
async fn member_queries_exclude_tombstoned_users() {
  let Ok(url) = std::env::var("DATABASE_URL") else {
    assert!(
      std::env::var("CI").is_err(),
      "DATABASE_URL must be set in CI"
    );
    return;
  };
  let db = PgPoolOptions::new()
    .max_connections(1)
    .connect(&url)
    .await
    .unwrap();
  sqlx::raw_sql(
    "CREATE TEMP TABLE users (id uuid PRIMARY KEY, email text, display_name text, avatar_key text, deleted_at timestamptz);
     CREATE TEMP TABLE workspace_members (workspace_id uuid, user_id uuid, role text, joined_at timestamptz DEFAULT now());",
  ).execute(&db).await.unwrap();
  let workspace = Uuid::new_v4();
  let active = Uuid::new_v4();
  let deleted = Uuid::new_v4();
  for user in [active, deleted] {
    sqlx::query(
      "INSERT INTO users (id,email,display_name) VALUES ($1, 'member@example.com', 'Member')",
    )
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
    sqlx::query(
      "INSERT INTO workspace_members (workspace_id,user_id,role) VALUES ($1,$2,'editor')",
    )
    .bind(workspace)
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
  }
  sqlx::query("UPDATE users SET deleted_at=now() WHERE id=$1")
    .bind(deleted)
    .execute(&db)
    .await
    .unwrap();
  let members = fetch_workspace_members(&db, workspace).await.unwrap();
  assert_eq!(members.len(), 1);
  assert_eq!(members[0].user_id, active);
  assert!(
    fetch_workspace_member(&db, workspace, active)
      .await
      .unwrap()
      .is_some()
  );
  assert!(
    fetch_workspace_member(&db, workspace, deleted)
      .await
      .unwrap()
      .is_none()
  );
}
