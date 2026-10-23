pub mod ai;
pub mod config;
pub mod db;
pub mod error;
pub mod mail;
pub mod storage;
pub mod telemetry;

pub use ai::{AiConfig, AiProvider};
pub use config::{AppConfig, Environment, SyncTuning};
pub use db::{connect_pg_pool, ensure_jwt_secret, ping_pg_pool, run_migrations};
pub use error::{ApiError, ApiResult};
pub use mail::{LogMailer, Mail, Mailer};
pub use storage::{
  BucketProbe, CreateOutcome, DEFAULT_S3_SECRET_KEY, PresignedUpload, S3Config, classify_create,
  classify_head, default_s3_secret_in_use, stored_content_type,
};
