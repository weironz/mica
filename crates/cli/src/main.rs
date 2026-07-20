//! `mica-cli` — a command-line client for a Mica server.
//!
//! It is a thin client over the REST API (see `client.rs`): every command maps
//! to the same endpoints web/desktop use, so the CLI can never diverge from the
//! product. Designed to be script- and agent-friendly: `--json` on any command
//! for machine-readable stdout, errors as `{"error": ...}` on stderr under
//! `--json`, non-interactive auth via `MICA_TOKEN`/`MICA_SERVER`, and a stable
//! non-zero exit on failure.
//!
//! Backup is one use of `export`: it writes every workspace as Markdown + images
//! into a mirrored directory that an external tool (e.g. restic → Aliyun OSS) can
//! then snapshot incrementally.

mod client;
mod config;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use client::Client;
use config::Config;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "mica-cli", version, about = "Command-line client for a Mica server")]
struct Cli {
  /// Server base URL, e.g. https://mica.example.com (overrides the saved config).
  #[arg(long, global = true, env = "MICA_SERVER")]
  server: Option<String>,
  /// Emit machine-readable JSON (for scripts / agents).
  #[arg(long, global = true)]
  json: bool,
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Authentication (login / whoami / logout).
  #[command(subcommand)]
  Auth(AuthCmd),
  /// Workspaces.
  #[command(subcommand)]
  Ws(WsCmd),
  /// Export workspaces to a directory of Markdown + images (mirrored) — point an
  /// external backup tool (restic / rclone / borg) at the output.
  Export(ExportArgs),
  /// Serve the Mica MCP server over stdio (for Claude Code / Desktop and any
  /// MCP client): list, read, create and write documents through the REST API.
  ///
  /// Register it with the client's own command — there is deliberately no
  /// `install` subcommand. A second way to write the same config file was only
  /// ever a convenience, and it cost more than it saved: it carried its own
  /// flags plus a hardcoded map of six clients' config paths, and people reached
  /// for it instead of `claude mcp add`, the portable command they already know.
  ///
  ///   claude mcp add --scope user mica -- <path-to-mica-cli> mcp
  ///
  /// Passing no `-e` is the cleanest form: the server walks the same credential
  /// chain as every other subcommand (MICA_API_BASE_URL/MICA_PAT →
  /// MICA_SERVER/MICA_TOKEN → whatever `auth login` saved), so no token has to
  /// touch the command line or sit in a client config file.
  Mcp(McpArgs),
}

#[derive(Args)]
struct McpArgs {
  /// Refuse every write tool at call time; read tools stay available.
  /// Also honored from the environment: MICA_MCP_READ_ONLY=1.
  #[arg(long)]
  read_only: bool,
}

#[derive(Subcommand)]
enum AuthCmd {
  /// Log in and save the access token to the config file.
  Login(LoginArgs),
  /// Print the signed-in user.
  Whoami,
  /// Forget the saved token.
  Logout,
  /// Manage long-lived API tokens (create / list / revoke).
  #[command(subcommand)]
  Token(TokenCmd),
}

#[derive(Subcommand)]
enum TokenCmd {
  /// Create an API token — the secret is printed ONCE.
  Create(TokenCreateArgs),
  /// List your API tokens (never the secret).
  List,
  /// Revoke a token by id.
  Revoke { id: Uuid },
}

#[derive(Args)]
struct TokenCreateArgs {
  /// A label for the token.
  #[arg(long)]
  name: String,
  /// Scope, repeatable: `read` and/or `write` (write implies read). Default: read.
  #[arg(long = "scope")]
  scopes: Vec<String>,
  /// Days until the token expires (omit for a token that never expires).
  #[arg(long)]
  expires_days: Option<i64>,
}

#[derive(Args)]
struct LoginArgs {
  /// Account email (or MICA_EMAIL).
  #[arg(long, env = "MICA_EMAIL")]
  email: Option<String>,
  /// Account password (or MICA_PASSWORD; otherwise read from stdin).
  #[arg(long, env = "MICA_PASSWORD")]
  password: Option<String>,
}

#[derive(Subcommand)]
enum WsCmd {
  /// List your workspaces.
  List,
}

#[derive(Args)]
struct ExportArgs {
  /// Output directory (mirrored: unchanged files kept, removed content pruned).
  #[arg(long)]
  out: PathBuf,
  /// Only export this workspace id (default: all your workspaces).
  #[arg(long)]
  ws: Option<Uuid>,
  /// Keep files for content that no longer exists (disable mirror pruning).
  #[arg(long)]
  no_prune: bool,
}

fn main() {
  let cli = Cli::parse();
  let json = cli.json;
  if let Err(err) = run(cli) {
    if json {
      eprintln!("{}", serde_json::json!({ "error": format!("{err:#}") }));
    } else {
      eprintln!("error: {err:#}");
    }
    std::process::exit(1);
  }
}

fn run(cli: Cli) -> Result<()> {
  let mut cfg = config::load()?;
  match &cli.command {
    Command::Auth(AuthCmd::Login(args)) => cmd_login(&cli, &mut cfg, args),
    Command::Auth(AuthCmd::Whoami) => cmd_whoami(&cli, &cfg),
    Command::Auth(AuthCmd::Logout) => cmd_logout(&mut cfg),
    Command::Auth(AuthCmd::Token(TokenCmd::Create(args))) => cmd_token_create(&cli, &cfg, args),
    Command::Auth(AuthCmd::Token(TokenCmd::List)) => cmd_token_list(&cli, &cfg),
    Command::Auth(AuthCmd::Token(TokenCmd::Revoke { id })) => cmd_token_revoke(&cli, &cfg, *id),
    Command::Ws(WsCmd::List) => cmd_ws_list(&cli, &cfg),
    Command::Export(args) => cmd_export(&cli, &cfg, args),
    Command::Mcp(args) => cmd_mcp(&cli, &cfg, args),
  }
}

// ---------------------------------------------------------------- auth

fn cmd_login(cli: &Cli, cfg: &mut Config, args: &LoginArgs) -> Result<()> {
  let server = cli
    .server
    .clone()
    .or_else(|| cfg.server.clone())
    .context("no server — pass --server <url> (or MICA_SERVER)")?;
  let email = args
    .email
    .clone()
    .context("no email — pass --email (or MICA_EMAIL)")?;
  let password = match args.password.clone() {
    Some(pw) => pw,
    None => read_password_from_stdin()?,
  };

  let auth = Client::new(server.clone(), None)?.login(&email, &password)?;
  cfg.server = Some(server);
  cfg.token = Some(auth.access_token.clone());
  config::save(cfg)?;

  if cli.json {
    print_json(&serde_json::json!({ "user": auth.user, "expires_at": auth.expires_at }))?;
  } else {
    println!(
      "Logged in as {} <{}>. Token saved to {}.",
      auth.user.display_name,
      auth.user.email,
      config::config_path()?.display()
    );
  }
  Ok(())
}

fn cmd_whoami(cli: &Cli, cfg: &Config) -> Result<()> {
  let user = authed_client(cli, cfg)?.me()?;
  if cli.json {
    print_json(&user)?;
  } else {
    println!("{} <{}>  (id {})", user.display_name, user.email, user.id);
  }
  Ok(())
}

fn cmd_logout(cfg: &mut Config) -> Result<()> {
  cfg.token = None;
  config::save(cfg)?;
  println!("Logged out (token cleared).");
  Ok(())
}

fn cmd_token_create(cli: &Cli, cfg: &Config, args: &TokenCreateArgs) -> Result<()> {
  let client = authed_client(cli, cfg)?;
  let scopes = if args.scopes.is_empty() {
    vec!["read".to_string()]
  } else {
    args.scopes.clone()
  };
  let created = client.create_token(&args.name, &scopes, args.expires_days)?;
  if cli.json {
    print_json(&created)?;
  } else {
    println!(
      "Token '{}' created (scopes: {}). Save it now — it will NOT be shown again:\n\n  {}\n",
      created.name,
      created.scopes.join(","),
      created.token
    );
  }
  Ok(())
}

fn cmd_token_list(cli: &Cli, cfg: &Config) -> Result<()> {
  let tokens = authed_client(cli, cfg)?.list_tokens()?;
  if cli.json {
    print_json(&tokens)?;
  } else if tokens.is_empty() {
    println!("(no tokens)");
  } else {
    for t in &tokens {
      println!(
        "{}  {:<20}  [{}]  used:{}  exp:{}",
        t.id,
        t.name,
        t.scopes.join(","),
        t.last_used_at.as_deref().unwrap_or("never"),
        t.expires_at.as_deref().unwrap_or("never"),
      );
    }
  }
  Ok(())
}

fn cmd_token_revoke(cli: &Cli, cfg: &Config, id: Uuid) -> Result<()> {
  authed_client(cli, cfg)?.revoke_token(id)?;
  if cli.json {
    print_json(&serde_json::json!({ "revoked": id.to_string() }))?;
  } else {
    println!("Revoked {id}.");
  }
  Ok(())
}

// ---------------------------------------------------------------- workspaces

fn cmd_ws_list(cli: &Cli, cfg: &Config) -> Result<()> {
  let workspaces = authed_client(cli, cfg)?.list_workspaces()?;
  if cli.json {
    print_json(&workspaces)?;
  } else if workspaces.is_empty() {
    println!("(no workspaces)");
  } else {
    for w in &workspaces {
      println!("{}  {:<28}  {}", w.id, w.name, w.role);
    }
  }
  Ok(())
}

// ---------------------------------------------------------------- export

fn cmd_export(cli: &Cli, cfg: &Config, args: &ExportArgs) -> Result<()> {
  let client = authed_client(cli, cfg)?;
  let mut workspaces = client.list_workspaces()?;
  if let Some(id) = args.ws {
    workspaces.retain(|w| w.id == id);
    if workspaces.is_empty() {
      anyhow::bail!("workspace {id} not found (or you are not a member)");
    }
  }

  // Only disambiguate directory names with an id suffix when two workspaces
  // actually collide on the same slug — unique names stay clean.
  let mut slug_counts: HashMap<String, usize> = HashMap::new();
  for w in &workspaces {
    *slug_counts.entry(slugify(&w.name)).or_default() += 1;
  }

  let mut desired: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
  let mut summary = Vec::new();
  for w in &workspaces {
    let zip = client.export_workspace_zip(w.id)?;
    let dir = workspace_dir(w, &slug_counts);
    let mut files = 0usize;
    for entry in mica_interchange::read_zip(&zip) {
      if entry.name.ends_with('/') {
        continue; // directory marker
      }
      desired.insert(Path::new(&dir).join(sanitize_rel(&entry.name)), entry.bytes);
      files += 1;
    }
    summary.push(serde_json::json!({ "id": w.id, "name": w.name, "dir": dir, "files": files }));
  }

  let manifest = serde_json::json!({
    "tool": concat!("mica-cli/", env!("CARGO_PKG_VERSION")),
    "exported_at": chrono::Utc::now().to_rfc3339(),
    "workspaces": summary,
  });
  desired.insert(PathBuf::from("manifest.json"), serde_json::to_vec_pretty(&manifest)?);

  let stats = mirror(&args.out, &desired, !args.no_prune)?;
  if cli.json {
    print_json(&serde_json::json!({
      "out": args.out,
      "workspaces": workspaces.len(),
      "files": stats.total,
      "written": stats.written,
      "pruned": stats.pruned,
    }))?;
  } else {
    println!(
      "Exported {} workspace(s) → {} ({} files: {} written, {} pruned)",
      workspaces.len(),
      args.out.display(),
      stats.total,
      stats.written,
      stats.pruned
    );
  }
  Ok(())
}

// ---------------------------------------------------------------- mcp

/// Serve MCP over stdio. Credentials resolve through the SAME chain every
/// other command uses (env > flag > saved login), plus the standalone
/// server's historical names (`MICA_API_BASE_URL`/`MICA_PAT`) so an existing
/// Claude Code config keeps working after the binary merge — `mica-cli auth
/// login` once and `mica-cli mcp` needs zero further configuration.
fn cmd_mcp(cli: &Cli, cfg: &Config, args: &McpArgs) -> Result<()> {
  let base = std::env::var("MICA_API_BASE_URL")
    .ok()
    .or_else(|| cli.server.clone())
    .or_else(|| cfg.server.clone())
    .context(
      "no server — set MICA_API_BASE_URL / --server / MICA_SERVER, or run `mica-cli auth login`",
    )?;
  let pat = std::env::var("MICA_PAT")
    .ok()
    .or_else(|| std::env::var("MICA_TOKEN").ok())
    .or_else(|| cfg.token.clone())
    .context("no token — set MICA_PAT / MICA_TOKEN, or run `mica-cli auth login`")?;
  let read_only = args.read_only
    || matches!(
      std::env::var("MICA_MCP_READ_ONLY").as_deref(),
      Ok("1") | Ok("true")
    );
  // The only async command: the rest of the CLI is blocking reqwest, so the
  // runtime lives here rather than on main.
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .context("starting the async runtime for the MCP server")?
    .block_on(mica_mcp_server::serve_stdio(base, pat, read_only))
}

// ---------------------------------------------------------------- helpers

pub(crate) fn authed_client(cli: &Cli, cfg: &Config) -> Result<Client> {
  let server = cli
    .server
    .clone()
    .or_else(|| cfg.server.clone())
    .context("no server — run `mica-cli auth login --server <url>` or set MICA_SERVER")?;
  let token = std::env::var("MICA_TOKEN")
    .ok()
    .or_else(|| cfg.token.clone())
    .context("not logged in — run `mica-cli auth login` or set MICA_TOKEN")?;
  Client::new(server, Some(token))
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
  println!("{}", serde_json::to_string_pretty(value)?);
  Ok(())
}

fn read_password_from_stdin() -> Result<String> {
  use std::io::BufRead;
  let mut line = String::new();
  std::io::stdin()
    .lock()
    .read_line(&mut line)
    .context("reading password from stdin")?;
  let pw = line.trim_end_matches(['\r', '\n']).to_string();
  if pw.is_empty() {
    anyhow::bail!("no password provided (pass --password, MICA_PASSWORD, or pipe it on stdin)");
  }
  Ok(pw)
}

/// A stable, filesystem-safe directory name for a workspace. Uses the plain
/// slugified name; falls back to an id suffix only when the name is empty or
/// two workspaces share the same slug (so their pages can't clobber each other).
fn workspace_dir(w: &client::Workspace, slug_counts: &HashMap<String, usize>) -> String {
  let slug = slugify(&w.name);
  let ambiguous = slug.is_empty() || slug_counts.get(&slug).copied().unwrap_or(0) > 1;
  if !ambiguous {
    return slug;
  }
  let id = w.id.simple().to_string();
  let short = &id[..8];
  if slug.is_empty() {
    format!("workspace-{short}")
  } else {
    format!("{slug}-{short}")
  }
}

fn slugify(name: &str) -> String {
  let mut out = String::new();
  let mut pending_dash = false;
  for ch in name.chars() {
    if ch.is_ascii_alphanumeric() {
      if pending_dash && !out.is_empty() {
        out.push('-');
      }
      out.push(ch.to_ascii_lowercase());
      pending_dash = false;
    } else {
      pending_dash = true;
    }
  }
  out
}

/// Drop `..`, absolute, and prefix components so a zip entry name can only ever
/// land inside the target directory.
fn sanitize_rel(name: &str) -> PathBuf {
  let mut out = PathBuf::new();
  for comp in Path::new(name).components() {
    if let std::path::Component::Normal(c) = comp {
      out.push(c);
    }
  }
  out
}

struct MirrorStats {
  total: usize,
  written: usize,
  pruned: usize,
}

/// Reconcile `out` to exactly `desired`: write changed files, and (unless
/// `prune` is false) delete any file that is no longer wanted, then drop empty
/// dirs. Identical files are left untouched so an external incremental backup
/// (restic) sees a minimal diff.
fn mirror(out: &Path, desired: &BTreeMap<PathBuf, Vec<u8>>, prune: bool) -> Result<MirrorStats> {
  fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;

  let mut written = 0;
  for (rel, bytes) in desired {
    let path = out.join(rel);
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent)?;
    }
    let changed = match fs::read(&path) {
      Ok(current) => &current != bytes,
      Err(_) => true,
    };
    if changed {
      fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
      written += 1;
    }
  }

  let mut pruned = 0;
  if prune {
    let mut existing = Vec::new();
    collect_files(out, out, &mut existing)?;
    for rel in existing {
      if !desired.contains_key(&rel) {
        let _ = fs::remove_file(out.join(&rel));
        pruned += 1;
      }
    }
    prune_empty_dirs(out)?;
  }

  Ok(MirrorStats { total: desired.len(), written, pruned })
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
  for entry in fs::read_dir(dir)? {
    let path = entry?.path();
    if path.is_dir() {
      collect_files(root, &path, out)?;
    } else if let Ok(rel) = path.strip_prefix(root) {
      out.push(rel.to_path_buf());
    }
  }
  Ok(())
}

/// Remove empty sub-directories of `dir` (never `dir` itself). Returns whether
/// `dir` ended up empty.
fn prune_empty_dirs(dir: &Path) -> Result<bool> {
  let mut empty = true;
  for entry in fs::read_dir(dir)? {
    let path = entry?.path();
    if path.is_dir() {
      if prune_empty_dirs(&path)? {
        let _ = fs::remove_dir(&path);
      } else {
        empty = false;
      }
    } else {
      empty = false;
    }
  }
  Ok(empty)
}
