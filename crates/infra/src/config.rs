use std::{env, net::SocketAddr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
  Development,
  Test,
  Production,
}

impl Environment {
  fn from_env_value(value: &str) -> Self {
    match value {
      "prod" | "production" => Self::Production,
      "test" => Self::Test,
      _ => Self::Development,
    }
  }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
  pub environment: Environment,
  pub http_addr: SocketAddr,
  pub database_url: String,
  pub database_max_connections: u32,
  pub jwt_secret: String,
  pub access_token_ttl_seconds: i64,
  /// Test-environment convenience: `MICA_SEED_TEST_USER=email:password`
  /// upserts this account at startup (creating it or resetting its password)
  /// so E2E runs always have known credentials. Hard-ignored in production.
  pub seed_test_user: Option<SeedTestUser>,
}

#[derive(Debug, Clone)]
pub struct SeedTestUser {
  pub email: String,
  pub password: String,
}

impl AppConfig {
  pub fn from_env() -> Result<Self, ConfigError> {
    let _ = dotenvy::dotenv();

    let environment = env::var("APP_ENV")
      .map(|value| Environment::from_env_value(&value))
      .unwrap_or(Environment::Development);

    let http_addr = env::var("HTTP_ADDR")
      .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
      .parse::<SocketAddr>()
      .map_err(|source| ConfigError::InvalidSocketAddr { source })?;

    let database_url = env::var("DATABASE_URL").map_err(|_| ConfigError::MissingDatabaseUrl)?;

    let database_max_connections = env::var("DATABASE_MAX_CONNECTIONS")
      .ok()
      .and_then(|value| value.parse::<u32>().ok())
      .unwrap_or(10);

    let jwt_secret = env::var("JWT_SECRET").map_err(|_| ConfigError::MissingJwtSecret)?;

    let access_token_ttl_seconds = env::var("ACCESS_TOKEN_TTL_SECONDS")
      .ok()
      .and_then(|value| value.parse::<i64>().ok())
      .unwrap_or(60 * 60 * 24);

    // `email:password` — the password may itself contain `:`, so split once.
    // Never honored in production, no matter what the variable says.
    let seed_test_user = if environment == Environment::Production {
      None
    } else {
      env::var("MICA_SEED_TEST_USER").ok().and_then(|raw| {
        let (email, password) = raw.split_once(':')?;
        let (email, password) = (email.trim(), password.trim());
        if email.is_empty() || !email.contains('@') || password.len() < 8 {
          return None;
        }
        Some(SeedTestUser {
          email: email.to_ascii_lowercase(),
          password: password.to_string(),
        })
      })
    };

    Ok(Self {
      environment,
      http_addr,
      database_url,
      database_max_connections,
      jwt_secret,
      access_token_ttl_seconds,
      seed_test_user,
    })
  }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
  #[error("DATABASE_URL is required")]
  MissingDatabaseUrl,

  #[error("JWT_SECRET is required")]
  MissingJwtSecret,

  #[error("HTTP_ADDR is invalid")]
  InvalidSocketAddr { source: std::net::AddrParseError },
}
