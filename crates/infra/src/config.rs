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
  /// Lifetime of the stateless access JWT. It cannot be revoked before it
  /// expires (the price of being stateless), so this doubles as the worst-case
  /// revocation window — kept short; the client refreshes transparently.
  pub access_token_ttl_seconds: i64,
  /// How long a sign-in survives without touching the password again. The
  /// access token above stays short — it is an unrevocable JWT — and the
  /// refresh token carries the session across its expiry.
  pub refresh_token_ttl_seconds: i64,
  /// Browser origins allowed to read the API cross-origin (CORS). Empty in
  /// production = deny all cross-origin (the bundled web app is same-origin with
  /// /api and needs no grant); set `CORS_ALLOWED_ORIGINS` (comma-separated) to
  /// opt origins in. In development an empty list stays permissive so the
  /// web app on a different localhost port than the API still works.
  pub cors_allowed_origins: Vec<String>,
  /// Test-environment convenience: `MICA_SEED_TEST_USER=email:password`
  /// upserts this account at startup (creating it or resetting its password)
  /// so E2E runs always have known credentials. Hard-ignored in production.
  pub seed_test_user: Option<SeedTestUser>,
  /// Whether public self-registration (`POST /auth/register`) is open. Default
  /// true (the existing behaviour). Set `MICA_REGISTRATION_ENABLED=false` to lock
  /// a self-hosted instance to its current accounts — the endpoint then refuses
  /// with 403 instead of creating accounts. The operator's one switch to run a
  /// public node privately.
  pub registration_enabled: bool,
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

    // 1h default (was 24h). Because the client refreshes transparently, a
    // shorter access token shrinks the window in which a token that SHOULD be
    // dead (password changed, session revoked) still works, at no user-visible
    // cost. Override with ACCESS_TOKEN_TTL_SECONDS.
    let access_token_ttl_seconds = env::var("ACCESS_TOKEN_TTL_SECONDS")
      .ok()
      .and_then(|value| value.parse::<i64>().ok())
      .unwrap_or(60 * 60);

    let refresh_token_ttl_seconds = env::var("REFRESH_TOKEN_TTL_SECONDS")
      .ok()
      .and_then(|value| value.parse::<i64>().ok())
      .unwrap_or(60 * 60 * 24 * 30);

    // Comma-separated browser origins allowed cross-origin. Empty => deny all
    // cross-origin in production (see the `cors_allowed_origins` field doc).
    let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS")
      .ok()
      .map(|value| {
        value
          .split(',')
          .map(|origin| origin.trim().to_string())
          .filter(|origin| !origin.is_empty())
          .collect()
      })
      .unwrap_or_default();

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

    // Open by default; an operator sets it false to keep a public node private.
    // Anything but an explicit off-value ("false"/"0"/"no"/"off") stays open, so
    // a typo never silently locks everyone out.
    let registration_enabled = env::var("MICA_REGISTRATION_ENABLED")
      .map(|v| {
        !matches!(
          v.trim().to_ascii_lowercase().as_str(),
          "false" | "0" | "no" | "off"
        )
      })
      .unwrap_or(true);

    Ok(Self {
      environment,
      http_addr,
      database_url,
      database_max_connections,
      jwt_secret,
      access_token_ttl_seconds,
      refresh_token_ttl_seconds,
      cors_allowed_origins,
      seed_test_user,
      registration_enabled,
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
