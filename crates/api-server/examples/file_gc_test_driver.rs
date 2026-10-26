//! One-shot real collector for scripts/test-files-quota.ps1's isolated stack.
#[allow(dead_code)]
#[path = "../src/blob_gc.rs"]
mod blob_gc;
#[allow(dead_code)]
#[path = "../src/http_client.rs"]
mod http_client;
#[allow(dead_code)]
#[path = "../src/metrics.rs"]
mod metrics;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let db = sqlx::PgPool::connect(&std::env::var("DATABASE_URL")?).await?;
  let storage = mica_infra::S3Config::from_env().expect("test storage must be configured");
  println!("{:?}", blob_gc::sweep_all(&db, &storage, false).await);
  Ok(())
}
