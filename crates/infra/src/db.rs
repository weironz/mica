use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::config::AppConfig;

// `sqlx::migrate!` embeds migrations/*.sql at COMPILE time. Adding a new file
// (e.g. 0012_document_content_text.sql) does not by itself force a rebuild of
// this crate, so touch this file whenever a migration is added or the running
// binary silently keeps the old set (see CLAUDE.md 运维要点).
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

pub async fn connect_pg_pool(config: &AppConfig) -> Result<PgPool, sqlx::Error> {
  PgPoolOptions::new()
    .max_connections(config.database_max_connections)
    .connect(&config.database_url)
    .await
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
  MIGRATOR.run(pool).await
}

pub async fn ping_pg_pool(pool: &PgPool) -> Result<(), sqlx::Error> {
  sqlx::query_scalar::<_, i32>("SELECT 1")
    .fetch_one(pool)
    .await?;
  Ok(())
}
