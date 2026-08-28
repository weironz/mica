//! Apply the embedded migrations to `DATABASE_URL` — the same set the server
//! runs at startup, callable without booting the server (dev convenience:
//! `DATABASE_URL=... cargo run -p mica-infra --example apply_migrations`).
#[tokio::main]
async fn main() {
  let url = std::env::var("DATABASE_URL").expect("set DATABASE_URL");
  let pool = sqlx::PgPool::connect(&url).await.expect("connect");
  mica_infra::run_migrations(&pool).await.expect("migrate");
  println!("migrations applied");
}
