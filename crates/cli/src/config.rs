//! Persisted CLI config: the server URL + saved access token.
//!
//! Location (first that resolves): `$MICA_CONFIG`, else
//! `$XDG_CONFIG_HOME/mica/config.json`, else `$HOME/.config/mica/config.json`
//! (unix) / `%APPDATA%\mica\config.json` (windows). Written `0600` on unix.
//!
//! Both fields are overridable per-invocation without touching the file:
//! `--server`/`MICA_SERVER` and the `MICA_TOKEN` env var take precedence, so an
//! agent or CI can stay fully stateless.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub server: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub token: Option<String>,
}

pub fn config_path() -> Result<PathBuf> {
  if let Ok(explicit) = std::env::var("MICA_CONFIG") {
    return Ok(PathBuf::from(explicit));
  }
  let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
    PathBuf::from(xdg)
  } else if let Ok(home) = std::env::var("HOME") {
    PathBuf::from(home).join(".config")
  } else if let Ok(appdata) = std::env::var("APPDATA") {
    PathBuf::from(appdata)
  } else {
    anyhow::bail!(
      "cannot locate a config dir — set MICA_CONFIG, HOME, XDG_CONFIG_HOME, or APPDATA"
    );
  };
  Ok(base.join("mica").join("config.json"))
}

pub fn load() -> Result<Config> {
  let path = config_path()?;
  match std::fs::read(&path) {
    Ok(bytes) => serde_json::from_slice(&bytes)
      .with_context(|| format!("parsing {} (delete it to reset)", path.display())),
    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
    Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
  }
}

pub fn save(config: &Config) -> Result<()> {
  let path = config_path()?;
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent)?;
  }
  let json = serde_json::to_vec_pretty(config)?;
  write_private(&path, &json)?;
  Ok(())
}

/// Write `bytes` to `path` so the file is never readable beyond its owner —
/// not even in the window between the write and a follow-up chmod.
///
/// The old `fs::write` + `set_permissions` created the file with the umask
/// default (typically 0644) and only tightened it AFTER the token had hit the
/// disk — a world-readable window for a bearer credential. Creating with mode
/// 0600 closes that; a file that already exists with loose permissions (an
/// older CLI's write, or a copied-in config) is tightened BEFORE the new
/// token is written, since `.mode()` applies only at creation.
#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
  use std::io::Write;
  use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
  if path.exists() {
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
  }
  let mut file = std::fs::OpenOptions::new()
    .write(true)
    .create(true)
    .truncate(true)
    .mode(0o600)
    .open(path)
    .with_context(|| format!("writing {}", path.display()))?;
  file
    .write_all(bytes)
    .with_context(|| format!("writing {}", path.display()))?;
  Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
  std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  // The on-disk shape only. `config_path`/`load`/`save` resolve the location
  // from process-global env (MICA_CONFIG / HOME / …) and read/write the real
  // user config dir; mutating env is unsafe under edition 2024 and races other
  // tests, so those are deliberately not exercised here (see the report).

  #[test]
  fn round_trips_both_fields() {
    let cfg = Config {
      server: Some("https://mica.example.com".to_string()),
      token: Some("secret-token".to_string()),
    };
    let json = serde_json::to_string(&cfg).unwrap();
    let back: Config = serde_json::from_str(&json).unwrap();
    assert_eq!(back.server.as_deref(), Some("https://mica.example.com"));
    assert_eq!(back.token.as_deref(), Some("secret-token"));
  }

  #[test]
  fn omits_none_fields_when_serializing() {
    // `skip_serializing_if = "Option::is_none"` → an empty config is `{}`, not
    // `{"server":null,"token":null}`.
    let cfg = Config::default();
    assert_eq!(serde_json::to_string(&cfg).unwrap(), "{}");

    let cfg = Config {
      server: Some("https://s".to_string()),
      token: None,
    };
    assert_eq!(
      serde_json::to_string(&cfg).unwrap(),
      r#"{"server":"https://s"}"#
    );
  }

  #[test]
  fn parses_partial_and_empty_json() {
    // Missing fields default to None (a fresh config with only a server set).
    let cfg: Config = serde_json::from_str(r#"{"server":"https://s"}"#).unwrap();
    assert_eq!(cfg.server.as_deref(), Some("https://s"));
    assert!(cfg.token.is_none());

    // An empty object is a valid, all-None config.
    let cfg: Config = serde_json::from_str("{}").unwrap();
    assert!(cfg.server.is_none() && cfg.token.is_none());
  }

  // These write to an explicit path in the temp dir — unlike
  // `config_path`/`load`/`save`, no environment variable is resolved, so the
  // reasons above (unsafe env mutation under edition 2024, racing other
  // tests) do not apply.

  /// A scratch path unique to this test run, so parallel runs never share one.
  fn scratch_path(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap()
      .as_nanos();
    std::env::temp_dir().join(format!(
      "mica-cli-{label}-{}-{nanos:x}.json",
      std::process::id()
    ))
  }

  /// #41: the token is a bearer credential, and a file that lands with the
  /// umask default (typically 0644) is world-readable for as long as it
  /// exists that way. The old write-then-chmod left exactly that window; the
  /// file must be born 0600 instead.
  #[test]
  fn a_new_file_is_born_owner_only() {
    let path = scratch_path("born");
    write_private(&path, br#"{"token":"secret"}"#).unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(on_disk.contains("secret"), "the bytes must land");
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let mode = std::fs::metadata(&path).unwrap().permissions().mode();
      assert_eq!(
        mode & 0o777,
        0o600,
        "a token file must never exist group/world-readable — not even briefly"
      );
    }
    let _ = std::fs::remove_file(&path);
  }

  /// The repair half: `.mode()` only applies at creation, so a file that
  /// already sits there with loose permissions (written by an older CLI, or
  /// copied in) must be tightened by the next save — before the new token is
  /// written into it, not after.
  #[cfg(unix)]
  #[test]
  fn a_loose_existing_file_is_tightened_by_the_next_save() {
    use std::os::unix::fs::PermissionsExt;
    let path = scratch_path("tighten");
    std::fs::write(&path, br#"{"token":"old"}"#).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    write_private(&path, br#"{"token":"new"}"#).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let _ = std::fs::remove_file(&path);
  }
}
