pub mod ai;
pub mod config;
pub mod db;
pub mod error;
pub mod mail;
pub mod storage;
pub mod telemetry;

pub use ai::{AiConfig, AiProvider};
pub use config::{AppConfig, Environment};
pub use db::{connect_pg_pool, ping_pg_pool, run_migrations};
pub use error::{ApiError, ApiResult};
pub use mail::{LogMailer, Mail, Mailer};
pub use storage::{PresignedUpload, S3Config};
