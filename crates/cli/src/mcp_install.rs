//! `mica-cli mcp install` — write the Mica MCP server entry into an MCP client's
//! config, so a user never hand-edits JSON/TOML or pastes a token by hand.
//!
//! The entry runs THIS binary (`current_exe`) as `mica-cli mcp`, with the server
//! URL and — by default — a freshly-minted PAT in the client's env. Existing
//! config is MERGED, never overwritten: other MCP servers and unrelated keys are
//! preserved, and a malformed file is refused rather than clobbered.

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

use crate::Cli;
use crate::config::Config;

#[derive(clap::Args)]
pub struct InstallArgs {
  /// Which client to configure. Omit and pass `--all` to do every client whose
  /// config directory is present on this machine.
  #[arg(long, value_enum)]
  client: Option<ClientKind>,
  /// Configure every supported client that is present on this machine.
  #[arg(long)]
  all: bool,
  /// Reuse this PAT (mica_pat_…) instead of creating one.
  #[arg(long)]
  pat: Option<String>,
  /// Do not embed a token; rely on this machine's saved `mica-cli auth login`.
  #[arg(long)]
  no_token: bool,
  /// Show what would change without writing anything.
  #[arg(long)]
  dry_run: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ClientKind {
  ClaudeDesktop,
  ClaudeCode,
  Cursor,
  Codex,
  Gemini,
  Windsurf,
}

const ALL: [ClientKind; 6] = [
  ClientKind::ClaudeDesktop,
  ClientKind::ClaudeCode,
  ClientKind::Cursor,
  ClientKind::Codex,
  ClientKind::Gemini,
  ClientKind::Windsurf,
];

impl ClientKind {
  fn label(self) -> &'static str {
    match self {
      ClientKind::ClaudeDesktop => "Claude Desktop",
      ClientKind::ClaudeCode => "Claude Code",
      ClientKind::Cursor => "Cursor",
      ClientKind::Codex => "Codex",
      ClientKind::Gemini => "Gemini CLI",
      ClientKind::Windsurf => "Windsurf",
    }
  }

  /// The config file for this client on the current OS, or None if the client
  /// has no known config location here.
  fn config_path(self) -> Option<PathBuf> {
    let home = home()?;
    match self {
      // Claude Desktop keeps its config in the per-user app-data dir. It ships
      // for macOS and Windows only — there is no official Linux build or path,
      // so we skip rather than guess a location that writes into the void.
      ClientKind::ClaudeDesktop => match os() {
        Os::Windows => Some(appdata()?.join("Claude").join("claude_desktop_config.json")),
        Os::Mac => Some(
          home
            .join("Library")
            .join("Application Support")
            .join("Claude")
            .join("claude_desktop_config.json"),
        ),
        Os::Linux => None,
      },
      // Claude Code stores MCP servers at the TOP LEVEL of ~/.claude.json. We
      // write the file directly rather than shell out to `claude mcp add`,
      // which on Windows records a forward-slash project key the app can't read
      // back (the local-scope trap) — the top-level entry sidesteps that
      // entirely and works on every OS.
      ClientKind::ClaudeCode => Some(home.join(".claude.json")),
      ClientKind::Cursor => Some(home.join(".cursor").join("mcp.json")),
      ClientKind::Codex => Some(home.join(".codex").join("config.toml")),
      ClientKind::Gemini => Some(home.join(".gemini").join("settings.json")),
      ClientKind::Windsurf => Some(
        home
          .join(".codeium")
          .join("windsurf")
          .join("mcp_config.json"),
      ),
    }
  }

  /// Merge the `mica` entry into this client's config, creating the file if
  /// needed. Every client but Codex uses a JSON `mcpServers` map.
  fn install(self, path: &Path, exe: &str, server: &str, pat: Option<&str>) -> Result<()> {
    match self {
      ClientKind::Codex => install_toml(path, exe, server, pat),
      _ => install_json(path, "mcpServers", json_entry(exe, server, pat)),
    }
  }
}

pub fn run(cli: &Cli, cfg: &Config, args: &InstallArgs) -> Result<()> {
  // Resolve targets, keeping only the ones actually present when doing --all so
  // we neither error on absent clients nor mint a token for nothing.
  let targets: Vec<(ClientKind, PathBuf)> = if args.all {
    ALL
      .iter()
      .filter_map(|k| k.config_path().filter(|p| p.exists()).map(|p| (*k, p)))
      .collect()
  } else {
    let kind = args
      .client
      .context("pass --client <name> (or --all). Names: claude-desktop, claude-code, cursor, codex, gemini, windsurf")?;
    let path = kind
      .config_path()
      .with_context(|| format!("{} has no known config location on this OS", kind.label()))?;
    vec![(kind, path)]
  };
  if targets.is_empty() {
    println!("No supported MCP client config found on this machine. Nothing to do.");
    return Ok(());
  }

  let server = cli
    .server
    .clone()
    .or_else(|| cfg.server.clone())
    .context("no server — run `mica-cli auth login --server <url>` first")?;
  let exe = std::env::current_exe()
    .context("locating this mica-cli binary")?
    .to_string_lossy()
    .into_owned();
  let pat = resolve_pat(cli, cfg, args)?;

  // Only mirror Claude Desktop → Claude Code when the latter isn't already an
  // explicit target, so `--all` doesn't write ~/.claude.json twice.
  let claude_code_is_target = targets
    .iter()
    .any(|(k, _)| *k == ClientKind::ClaudeCode);
  let claude_code_path = ClientKind::ClaudeCode.config_path();

  for (kind, path) in &targets {
    if args.dry_run {
      println!("would configure {} → {}", kind.label(), path.display());
    } else {
      kind
        .install(path, &exe, &server, pat.as_deref())
        .with_context(|| format!("configuring {} at {}", kind.label(), path.display()))?;
      println!("configured {} → {}", kind.label(), path.display());
    }

    // Belt-and-suspenders for wrapper runtimes. claude_desktop_config.json is
    // NOT durable under Claude Code Desktop (and similar): they regenerate it
    // on launch and wipe the entry we just wrote, so the server silently never
    // connects. When a Claude Code config (~/.claude.json) already exists — the
    // tell-tale that such a runtime is installed — mirror the entry there too,
    // where it survives restarts. Gated on the file already existing so a plain
    // Claude Desktop user (who never reads ~/.claude.json) gets nothing extra.
    if should_mirror_to_claude_code(*kind, claude_code_path.as_deref(), claude_code_is_target) {
      let cc = claude_code_path.as_deref().expect("gated on Some path");
      if args.dry_run {
        println!(
          "  would also mirror to {} → {} (durable under wrapper runtimes)",
          ClientKind::ClaudeCode.label(),
          cc.display()
        );
      } else {
        ClientKind::ClaudeCode
          .install(cc, &exe, &server, pat.as_deref())
          .with_context(|| format!("mirroring to Claude Code at {}", cc.display()))?;
        println!(
          "  also mirrored to {} → {}\n  \
           (claude_desktop_config.json isn't durable under wrapper runtimes like \
           Claude Code Desktop; this copy survives restarts)",
          ClientKind::ClaudeCode.label(),
          cc.display()
        );
      }
    }
  }

  if args.dry_run {
    println!("(dry run — nothing written)");
  } else {
    match pat {
      Some(_) => println!("\nEmbedded a fresh PAT. Restart the client to pick up the MCP server."),
      None => println!(
        "\nNo token embedded — the server relies on this machine's saved login. Restart the client."
      ),
    }
  }
  Ok(())
}

/// Decide the credential written into the client config: none (`--no-token`, rely
/// on the saved login), an explicit `--pat`, or a freshly-created one.
fn resolve_pat(cli: &Cli, cfg: &Config, args: &InstallArgs) -> Result<Option<String>> {
  if args.no_token {
    return Ok(None);
  }
  if let Some(p) = &args.pat {
    return Ok(Some(p.clone()));
  }
  let created = crate::authed_client(cli, cfg)?
    .create_token("mica-mcp", &["read".to_string(), "write".to_string()], None)
    .context("creating a PAT for the MCP client (run `mica-cli auth login` first, or pass --pat / --no-token)")?;
  Ok(Some(created.token))
}

/// Should configuring Claude Desktop also mirror the entry into the Claude Code
/// config? Only when (a) we're configuring Claude Desktop, (b) Claude Code isn't
/// already an explicit target (else `--all` writes it twice), and (c) a Claude
/// Code config file already exists — the tell-tale of a wrapper runtime that
/// wipes claude_desktop_config.json. We never conjure ~/.claude.json for a user
/// who doesn't have it (a plain Claude Desktop install never reads it).
fn should_mirror_to_claude_code(
  kind: ClientKind,
  claude_code_path: Option<&Path>,
  claude_code_is_target: bool,
) -> bool {
  kind == ClientKind::ClaudeDesktop
    && !claude_code_is_target
    && claude_code_path.is_some_and(Path::exists)
}

/// The stdio-server entry shared by every JSON `mcpServers` client.
fn json_entry(exe: &str, server: &str, pat: Option<&str>) -> Value {
  let mut env = Map::new();
  env.insert("MICA_API_BASE_URL".to_string(), json!(server));
  if let Some(p) = pat {
    env.insert("MICA_PAT".to_string(), json!(p));
  }
  json!({ "command": exe, "args": ["mcp"], "env": Value::Object(env) })
}

/// Merge `{ <top_key>: { "mica": <entry> } }` into a JSON config, preserving
/// everything else. Refuses a file whose top level (or `<top_key>`) is not an
/// object rather than overwriting real data.
fn install_json(path: &Path, top_key: &str, entry: Value) -> Result<()> {
  let mut root = read_json_object(path)?;
  let servers = root
    .entry(top_key.to_string())
    .or_insert_with(|| Value::Object(Map::new()));
  let servers = servers
    .as_object_mut()
    .with_context(|| format!("`{top_key}` in {} is not an object", path.display()))?;
  servers.insert("mica".to_string(), entry);
  write_atomic(
    path,
    serde_json::to_vec_pretty(&Value::Object(root))?.as_slice(),
  )
}

/// Codex uses TOML: an `[mcp_servers.mica]` table. Parsed/merged as a JSON-ish
/// `toml::Value` so unrelated tables survive.
fn install_toml(path: &Path, exe: &str, server: &str, pat: Option<&str>) -> Result<()> {
  let mut root: toml::Table = match std::fs::read_to_string(path) {
    Ok(s) => s
      .parse()
      .with_context(|| format!("parsing {} (fix or remove it)", path.display()))?,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
    Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
  };

  let mut env = toml::Table::new();
  env.insert(
    "MICA_API_BASE_URL".into(),
    toml::Value::String(server.to_string()),
  );
  if let Some(p) = pat {
    env.insert("MICA_PAT".into(), toml::Value::String(p.to_string()));
  }
  let mut entry = toml::Table::new();
  entry.insert("command".into(), toml::Value::String(exe.to_string()));
  entry.insert(
    "args".into(),
    toml::Value::Array(vec![toml::Value::String("mcp".into())]),
  );
  entry.insert("env".into(), toml::Value::Table(env));

  let servers = root
    .entry("mcp_servers".to_string())
    .or_insert_with(|| toml::Value::Table(toml::Table::new()));
  let servers = servers
    .as_table_mut()
    .with_context(|| format!("`mcp_servers` in {} is not a table", path.display()))?;
  servers.insert("mica".to_string(), toml::Value::Table(entry));

  write_atomic(path, toml::to_string_pretty(&root)?.as_bytes())
}

/// Read a JSON config as an object map, or an empty one if the file is absent.
fn read_json_object(path: &Path) -> Result<Map<String, Value>> {
  match std::fs::read(path) {
    Ok(bytes) => {
      let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {} (fix or remove it)", path.display()))?;
      match value {
        Value::Object(map) => Ok(map),
        _ => bail!("{} is not a JSON object", path.display()),
      }
    }
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
    Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
  }
}

/// Write via a temp file + rename so a crash mid-write cannot truncate an
/// existing config. Creates parent dirs.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
  }
  let tmp = path.with_extension("mica-tmp");
  std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
  std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
  Ok(())
}

// ── OS + path helpers (env-var based, matching config.rs — no `dirs` dep) ─────

enum Os {
  Windows,
  Mac,
  Linux,
}

fn os() -> Os {
  if cfg!(target_os = "windows") {
    Os::Windows
  } else if cfg!(target_os = "macos") {
    Os::Mac
  } else {
    Os::Linux
  }
}

fn home() -> Option<PathBuf> {
  std::env::var_os("HOME")
    .or_else(|| std::env::var_os("USERPROFILE"))
    .map(PathBuf::from)
}

fn appdata() -> Option<PathBuf> {
  std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn json_entry_carries_command_args_and_token() {
    let e = json_entry(
      "/opt/mica-cli",
      "https://mica.example.com",
      Some("mica_pat_x"),
    );
    assert_eq!(e["command"], json!("/opt/mica-cli"));
    assert_eq!(e["args"], json!(["mcp"]));
    assert_eq!(
      e["env"]["MICA_API_BASE_URL"],
      json!("https://mica.example.com")
    );
    assert_eq!(e["env"]["MICA_PAT"], json!("mica_pat_x"));
  }

  #[test]
  fn no_token_omits_the_pat() {
    let e = json_entry("/opt/mica-cli", "https://s", None);
    assert!(e["env"].get("MICA_PAT").is_none());
    assert_eq!(e["env"]["MICA_API_BASE_URL"], json!("https://s"));
  }

  #[test]
  fn merge_preserves_other_servers_and_keys() {
    let dir = std::env::temp_dir().join(format!("mica-mcpinstall-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cfg.json");
    std::fs::write(
      &path,
      serde_json::to_vec_pretty(&json!({
        "theme": "dark",
        "mcpServers": { "other": { "command": "x" } }
      }))
      .unwrap(),
    )
    .unwrap();

    install_json(
      &path,
      "mcpServers",
      json_entry("/m", "https://s", Some("t")),
    )
    .unwrap();

    let back: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(back["theme"], json!("dark"), "unrelated keys survive");
    assert_eq!(
      back["mcpServers"]["other"]["command"],
      json!("x"),
      "other servers survive"
    );
    assert_eq!(
      back["mcpServers"]["mica"]["command"],
      json!("/m"),
      "mica added"
    );
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn merge_refuses_a_non_object_top_level() {
    let dir = std::env::temp_dir().join(format!("mica-mcpinstall-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cfg.json");
    std::fs::write(&path, b"[1,2,3]").unwrap();
    assert!(install_json(&path, "mcpServers", json!({})).is_err());
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn mirror_only_when_claude_code_config_exists() {
    let dir = std::env::temp_dir().join(format!("mica-mcpmirror-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let existing = dir.join(".claude.json");
    std::fs::write(&existing, b"{}").unwrap();
    let missing = dir.join("nope.json");

    // Claude Desktop + an existing ~/.claude.json (wrapper tell-tale) → mirror.
    assert!(should_mirror_to_claude_code(
      ClientKind::ClaudeDesktop,
      Some(&existing),
      false
    ));
    // No Claude Code config → don't conjure one for a plain Desktop user.
    assert!(!should_mirror_to_claude_code(
      ClientKind::ClaudeDesktop,
      Some(&missing),
      false
    ));
    // Claude Code already an explicit target (e.g. --all) → don't write twice.
    assert!(!should_mirror_to_claude_code(
      ClientKind::ClaudeDesktop,
      Some(&existing),
      true
    ));
    // Configuring some other client → never mirror.
    assert!(!should_mirror_to_claude_code(
      ClientKind::Cursor,
      Some(&existing),
      false
    ));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn codex_toml_merges_a_table() {
    let dir = std::env::temp_dir().join(format!("mica-mcpinstall-toml-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(
      &path,
      "model = \"gpt\"\n[mcp_servers.other]\ncommand = \"x\"\n",
    )
    .unwrap();

    install_toml(&path, "/m", "https://s", Some("t")).unwrap();

    let back: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert_eq!(
      back["model"].as_str(),
      Some("gpt"),
      "unrelated keys survive"
    );
    let servers = back["mcp_servers"].as_table().unwrap();
    assert!(servers.contains_key("other"), "other servers survive");
    let mica = servers["mica"].as_table().unwrap();
    assert_eq!(mica["command"].as_str(), Some("/m"));
    assert_eq!(
      mica["env"].as_table().unwrap()["MICA_PAT"].as_str(),
      Some("t")
    );
    std::fs::remove_dir_all(&dir).ok();
  }
}
