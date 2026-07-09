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
use std::path::PathBuf;

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
    anyhow::bail!("cannot locate a config dir — set MICA_CONFIG, HOME, XDG_CONFIG_HOME, or APPDATA");
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
  std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
  // The token is a bearer credential — keep it owner-only where the OS supports it.
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
  }
  Ok(())
}
