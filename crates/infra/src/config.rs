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
  /// Ceiling on the total bytes of stored files per WORKSPACE. `0` = unlimited.
  /// Set `MICA_WORKSPACE_QUOTA_BYTES`; defaults to 1 GiB.
  ///
  /// Per workspace rather than per instance because that is the unit a person
  /// owns and can clean up — an instance-wide cap gives whoever fills it first
  /// the power to stop everyone else. It bounds the damage one workspace can do;
  /// bounding the NUMBER of workspaces is [`registration_enabled`]'s job, and
  /// neither limit means much without the other.
  ///
  /// 1 GiB is chosen against measured reality, not taste: the largest workspace
  /// on the production node holds 57 MB, so this is ~18× headroom for real use
  /// while still capping a single workspace far below a disk.
  pub workspace_quota_bytes: i64,
  /// Window + cadence knobs for the yrs update stream. See [`SyncTuning`].
  pub sync_tuning: SyncTuning,
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

/// Window + cadence of the yrs update stream (`workspace_updates`).
///
/// These were hardcoded constants in `app_core::sync`. They are here because
/// they are the two numbers that decide how much stream a node keeps and how
/// often it pays to trim it — the sort of thing that wants moving on a node with
/// unusually large or unusually chatty documents, and the sort of thing nobody
/// wants to rebuild a binary for. Defaults are exactly the old constants, so an
/// unconfigured node behaves as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncTuning {
  /// Most updates one catch-up pull may return (`MICA_CATCH_UP_LIMIT`). A client
  /// further behind than this pulls again from its advanced cursor.
  pub catch_up_limit: i64,
  /// How many already-folded updates stay on the stream so a briefly offline
  /// client can catch up incrementally (`MICA_STREAM_KEEP_MARGIN`). Below this
  /// margin a client re-bootstraps from the base instead — correct either way,
  /// just more bytes.
  pub keep_margin: i64,
  /// Prune roughly once every this-many rid draws (`MICA_STREAM_PRUNE_EVERY`),
  /// which keeps the DELETE off the hot path. Counts TABLE-wide draws, not one
  /// document's pushes — see the prune site in `app_core::sync::push_update`.
  pub prune_every: i64,
}

impl Default for SyncTuning {
  fn default() -> Self {
    Self {
      catch_up_limit: 1000,
      keep_margin: 64,
      prune_every: 32,
    }
  }
}

impl SyncTuning {
  fn from_env() -> Self {
    let d = Self::default();
    Self {
      catch_up_limit: positive_or(env::var("MICA_CATCH_UP_LIMIT").ok().as_deref(), d.catch_up_limit),
      keep_margin: positive_or(
        env::var("MICA_STREAM_KEEP_MARGIN").ok().as_deref(),
        d.keep_margin,
      ),
      prune_every: positive_or(
        env::var("MICA_STREAM_PRUNE_EVERY").ok().as_deref(),
        d.prune_every,
      ),
    }
  }
}

/// A strictly positive integer from `raw`, or `default` for anything else.
///
/// "Anything else" deliberately includes `0` and negatives, for two different
/// reasons that happen to point the same way: `prune_every = 0` would evaluate
/// `rid % 0` and panic on the push path, and `keep_margin = 0` would delete the
/// very update just inserted. Neither is a state an operator can want, so there
/// is no spelling for them — same principle as [`workspace_quota`], where an
/// unparseable value must not resolve to "no limit".
pub fn positive_or(raw: Option<&str>, default: i64) -> i64 {
  match raw.map(str::trim) {
    None | Some("") => default,
    Some(v) => match v.parse::<i64>() {
      Ok(n) if n > 0 => n,
      _ => default,
    },
  }
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

    let workspace_quota_bytes =
      workspace_quota(env::var("MICA_WORKSPACE_QUOTA_BYTES").ok().as_deref());

    let sync_tuning = SyncTuning::from_env();

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
      workspace_quota_bytes,
      sync_tuning,
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

/// Per-workspace byte ceiling from the raw `MICA_WORKSPACE_QUOTA_BYTES`.
///
/// Default 1 GiB. `0` means unlimited — spelled out so an operator who genuinely
/// wants no cap says so, instead of discovering that a typo removed it.
///
/// **Garbage keeps the default rather than removing the limit.** That is the
/// opposite of how most parsing here works (`unwrap_or` on a bad value usually
/// means "behave as if unset"), and deliberately so: for a quota, "unparseable"
/// must never resolve to "unlimited", or a stray character in a deploy env is a
/// silently disabled safety limit. Negative is treated the same way.
pub fn workspace_quota(raw: Option<&str>) -> i64 {
  const DEFAULT: i64 = 1024 * 1024 * 1024;
  match raw.map(str::trim) {
    None | Some("") => DEFAULT,
    Some(v) => match v.parse::<i64>() {
      Ok(n) if n >= 0 => n,
      _ => DEFAULT,
    },
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

#[cfg(test)]
mod sync_tuning_parse {
  use super::{SyncTuning, positive_or};

  /// An unconfigured node must behave exactly as it did when these were
  /// constants in `app_core::sync` — that is the whole contract of moving them.
  #[test]
  fn the_defaults_are_the_old_constants() {
    let d = SyncTuning::default();
    assert_eq!(d.catch_up_limit, 1000);
    assert_eq!(d.keep_margin, 64);
    assert_eq!(d.prune_every, 32);
  }

  #[test]
  fn a_positive_number_is_taken_literally() {
    assert_eq!(positive_or(Some("7"), 32), 7);
    assert_eq!(positive_or(Some(" 7 "), 32), 7);
  }

  /// The one that matters: `0` has no spelling here. `prune_every = 0` would
  /// evaluate `rid % 0` and panic on every push; `keep_margin = 0` would delete
  /// the update just inserted. A typo must not reach either.
  #[test]
  fn zero_negatives_and_garbage_keep_the_default() {
    for bad in ["0", "-1", "-64", "abc", "1_000", "1.5", "64x", ""] {
      assert_eq!(
        positive_or(Some(bad), 32),
        32,
        "{bad:?} must fall back to the default"
      );
    }
    assert_eq!(positive_or(None, 32), 32);
  }
}

#[cfg(test)]
mod quota_parse {
  use super::workspace_quota;

  const GIB: i64 = 1024 * 1024 * 1024;

  #[test]
  fn unset_or_blank_is_one_gib() {
    assert_eq!(workspace_quota(None), GIB);
    assert_eq!(workspace_quota(Some("")), GIB);
    assert_eq!(workspace_quota(Some("   ")), GIB);
  }

  #[test]
  fn a_number_is_taken_literally_and_zero_means_unlimited() {
    assert_eq!(workspace_quota(Some("0")), 0, "0 = no limit, spelled out");
    assert_eq!(workspace_quota(Some("5000")), 5000);
    assert_eq!(workspace_quota(Some(" 5000 ")), 5000);
  }

  /// The important one, and the opposite of how most parsing here behaves:
  /// garbage keeps the DEFAULT, it does not fall through to "unlimited". For a
  /// safety limit, an unparseable value must never resolve to "no limit" — a
  /// stray character in a deploy environment would silently disable it.
  #[test]
  fn garbage_and_negatives_keep_the_default_rather_than_removing_the_limit() {
    for bad in ["abc", "1GiB", "1_000", "-1", "-99999", "1.5", "∞"] {
      assert_eq!(
        workspace_quota(Some(bad)),
        GIB,
        "{bad:?} must fall back to the default, never to unlimited"
      );
    }
  }
}
