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
  /// Lowest WS sync-protocol version this server still accepts.
  ///
  /// **Defaults to 0, which rejects nobody** — the gate ships inert on purpose.
  /// Its job is to exist BEFORE it is needed: desktop clients are user-installed
  /// binaries that drift at their own pace, and the op-model REST fallback is
  /// what silently catches an old client today. When that fallback is removed
  /// (op-model retirement S4), an old client would start failing in confusing
  /// ways instead of being told why — so the floor is raised at that point, to a
  /// server that already knows how to say "your client is too old".
  pub ws_min_protocol: u32,
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
  /// Whether public self-registration (`POST /auth/register`) is open.
  /// **Default false.** Set `MICA_REGISTRATION_ENABLED=true` (or `1`/`yes`/`on`)
  /// to open it; anything else — including a typo — keeps it closed, which is the
  /// safe direction for a switch whose failure mode is "strangers can mint
  /// accounts on my node". Refuses with 403; login and refresh for existing users
  /// are untouched, and `auth::register` still lets a brand-new instance create
  /// its very first account.
  pub registration_enabled: bool,
  /// Public origin the app is reached at, e.g. `https://mica.example.com` — used
  /// to build the password-reset link that goes in the email (`{base}/reset-
  /// password?token=…`). Set `MICA_APP_BASE_URL`; defaults to
  /// `http://localhost:8080` for local dev. No trailing slash (one is stripped).
  pub app_base_url: String,
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
    // 0 = accept every client, including ones predating the version parameter.
    // Raise to WS_PROTOCOL_VERSION once the op-model fallback is gone (S4).
    let ws_min_protocol = env::var("MICA_WS_MIN_PROTOCOL")
      .ok()
      .and_then(|value| value.parse::<u32>().ok())
      .unwrap_or(0);

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

    // CLOSED by default, and the polarity of the parse flips with it: only an
    // explicit on-value opens registration, so a typo leaves the node private
    // rather than open. That is the safe direction for this switch specifically —
    // what it guards is "anyone on the internet can mint accounts here", and an
    // unbounded account count is the multiplier that makes every other limit
    // (upload size, per-workspace quota) meaningless.
    //
    // It used to default OPEN and production never set the variable, so
    // registering on the public instance needed nothing but a well-formed email.
    // See `auth::register` for the first-run exception that keeps a fresh
    // self-hosted install usable.
    let registration_enabled = registration_open(env::var("MICA_REGISTRATION_ENABLED").ok());

    // Where the emailed reset link points. Trailing slash trimmed so
    // `{base}/reset-password` never doubles up.
    let app_base_url = env::var("MICA_APP_BASE_URL")
      .ok()
      .map(|v| v.trim().trim_end_matches('/').to_string())
      .filter(|v| !v.is_empty())
      .unwrap_or_else(|| "http://localhost:8080".to_string());

    Ok(Self {
      environment,
      http_addr,
      database_url,
      database_max_connections,
      jwt_secret,
      ws_min_protocol,
      access_token_ttl_seconds,
      refresh_token_ttl_seconds,
      cors_allowed_origins,
      seed_test_user,
      registration_enabled,
      app_base_url,
    })
  }
}

/// Is public self-registration open, given the raw `MICA_REGISTRATION_ENABLED`?
///
/// A free function so the polarity is testable without constructing a whole
/// [`Config`] (which needs a DATABASE_URL and a JWT secret to exist at all). The
/// polarity is the entire point of this code, so it gets to be asserted directly.
pub fn registration_open(raw: Option<String>) -> bool {
  raw.is_some_and(|v| {
    matches!(
      v.trim().to_ascii_lowercase().as_str(),
      "true" | "1" | "yes" | "on"
    )
  })
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

#[cfg(test)]
mod registration_switch {
  use super::registration_open;

  /// The default, and the reason this function exists. It used to default OPEN
  /// with production never setting the variable, so anyone could mint an account
  /// on the public instance. An unbounded account count is what makes every other
  /// limit — upload size, per-workspace quota — multipliable.
  #[test]
  fn unset_means_closed() {
    assert!(!registration_open(None));
  }

  #[test]
  fn only_an_explicit_on_value_opens_it() {
    for on in ["true", "1", "yes", "on", "TRUE", " On "] {
      assert!(registration_open(Some(on.to_string())), "{on:?} must open");
    }
  }

  /// Fail-safe polarity: a typo leaves the node private rather than open. The old
  /// parse was the other way round ("anything but false/0/no/off stays open"),
  /// which is the right default for a convenience flag and the wrong one for a
  /// switch guarding who may create accounts.
  #[test]
  fn anything_else_including_a_typo_stays_closed() {
    for off in ["", " ", "false", "0", "no", "off", "ture", "enabled", "maybe"] {
      assert!(
        !registration_open(Some(off.to_string())),
        "{off:?} must NOT open registration"
      );
    }
  }
}
