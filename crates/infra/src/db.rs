use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::config::AppConfig;

// `sqlx::migrate!` embeds migrations/*.sql at COMPILE time. Adding a new file
// (e.g. 0013_password_reset_tokens.sql) does not by itself force a rebuild of
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

/// The token-signing key: whatever the operator configured, or one this server
/// minted for itself and kept.
///
/// Called once at startup, AFTER `run_migrations` (it reads a table those
/// create) and BEFORE `AppState` exists. `configured` is
/// `AppConfig::jwt_secret`, which is EMPTY when `JWT_SECRET` was not set —
/// already validated when it was.
///
/// **Why the database and not a file.** The api container has no volume
/// (`deploy/docker-compose.single.yml`), so a file would vanish on every
/// container recreate and quietly sign everyone out on each upgrade; and two api
/// replicas would each mint their own and reject the other's tokens. One row is
/// shared by construction.
///
/// **Why this does not reopen the `change-me` hole.** Nothing is shipped. The
/// value is 32 bytes from the OS CSPRNG, minted on the instance, different on
/// every install — there is no default anyone could look up. The strict check on
/// an explicitly-configured value is untouched.
///
/// Concurrency: two api processes booting together both try to insert. The
/// `ON CONFLICT DO NOTHING` + re-select means the loser adopts the winner's
/// value rather than overwriting it — which would have invalidated the tokens
/// the winner had already signed.
pub async fn ensure_jwt_secret(db: &PgPool, configured: &str) -> Result<String, sqlx::Error> {
    if !configured.trim().is_empty() {
        return Ok(configured.to_string());
    }
    if let Some(found) = read_secret(db, JWT_SECRET_NAME).await? {
        return Ok(found);
    }
    let minted = random_hex_32();
    sqlx::query("INSERT INTO server_secrets(name, value) VALUES ($1, $2) ON CONFLICT (name) DO NOTHING")
        .bind(JWT_SECRET_NAME)
        .bind(&minted)
        .execute(db)
        .await?;
    // Re-read rather than returning `minted`: if a concurrent boot won the
    // insert, the row holds THEIR value and ours was discarded.
    let stored = read_secret(db, JWT_SECRET_NAME)
        .await?
        .unwrap_or_else(|| minted.clone());
    if stored == minted {
        tracing::info!(
            "no JWT_SECRET configured — minted one for this instance and stored it \
             (the `jwt_secret` row of `server_secrets`). Sessions survive restarts; \
             set JWT_SECRET only if you want to supply your own."
        );
    }
    Ok(stored)
}

const JWT_SECRET_NAME: &str = "jwt_secret";

async fn read_secret(db: &PgPool, name: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>("SELECT value FROM server_secrets WHERE name = $1")
        .bind(name)
        .fetch_optional(db)
        .await
}

/// 32 bytes of OS randomness as lowercase hex — the same shape (and strength)
/// the docs used to ask operators to produce with `openssl rand -hex 32`.
fn random_hex_32() -> String {
    use rand_core::RngCore as _;
    let mut bytes = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
