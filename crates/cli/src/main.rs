//! `mica-cli` — a command-line client for a Mica server.
//!
//! It is a thin client over the REST API (see `client.rs`): every command maps
//! to the same endpoints web/desktop use, so the CLI can never diverge from the
//! product. Designed to be script- and agent-friendly: `--json` on any command
//! for machine-readable stdout, errors as `{"error": ...}` on stderr under
//! `--json`, non-interactive auth via `MICA_PAT`/`MICA_SERVER`, and a stable
//! non-zero exit on failure.
//!
//! Backup is one use of `export`: it writes every workspace as Markdown + images
//! into a mirrored directory that an external tool (e.g. restic → Aliyun OSS) can
//! then snapshot incrementally.

mod backup;
mod client;
mod config;

use anyhow::{Context, Result, bail};
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
  /// API token (a PAT from `auth token create`) — overrides the saved login.
  ///
  /// Deliberately NOT wired to clap's `env`: the whole precedence order,
  /// including the retired `MICA_TOKEN` name, is written down once in
  /// [`pick_token`] / [`no_token_hint`] instead of half here and half there.
  #[arg(long, global = true, value_name = "TOKEN")]
  token: Option<String>,
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
  /// The scheduled backup: content + database + object bytes, off-site.
  ///
  /// Replaces the `mica-backup.sh` / `mica-backup-loop.sh` pair. The reason it
  /// is code and not shell is in `backup.rs`: a shell run is an exit code, so
  /// "backed up less than you think" and "backed up everything" looked the same
  /// — and did, in production, for months.
  /// Import archives — by default one new workspace per archive.
  ///
  /// Several archives run one after another, not at once: a big import is
  /// mostly the server fetching images, and firing them off in parallel only
  /// makes them contend. Each job is watched to completion before the next
  /// starts, so the output reads as a straight log of what landed where.
  Import(ImportArgs),
  /// Pages and folders: list, read, move, trash. The bulk forms take many ids
  /// and send ONE request, which is what makes reorganising a workspace from a
  /// script practical instead of a few hundred round trips.
  #[command(subcommand)]
  Page(PageCmd),
  /// The recycle bin: see it, restore from it, empty it.
  #[command(subcommand)]
  Trash(TrashCmd),
  #[command(subcommand)]
  Backup(BackupCmd),
  /// Serve the Mica MCP server over stdio, for any MCP client (Claude Code and
  /// Desktop, Cursor, Codex, Gemini, Windsurf, …): list, read, create and write
  /// documents through the REST API.
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
  /// chain as every other subcommand (MICA_API_BASE_URL / MICA_SERVER for the
  /// server, --token / MICA_PAT for the credential, then whatever `auth login`
  /// saved), so no token has to touch the command line or sit in a client
  /// config file.
  Mcp(McpArgs),
  /// Re-host external image links into Mica storage. Downloads each image with
  /// THIS machine's network (so it works for hosts a CN-hosted server 403s, e.g.
  /// AppFlowy), stores it, and repoints the block. Safe to re-run — images
  /// already on a Mica file_id are skipped.
  RehostImages(RehostImagesArgs),
}

#[derive(Args)]
struct RehostImagesArgs {
  /// Only this workspace id (default: all your workspaces).
  #[arg(long)]
  ws: Option<Uuid>,
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
struct ImportArgs {
  /// Archives to import. Each becomes its own new workspace, named after the
  /// file (unless --ws sends them all into one).
  #[arg(required = true)]
  archives: Vec<PathBuf>,
  /// Import INTO this existing workspace instead of creating new ones.
  #[arg(long)]
  ws: Option<Uuid>,
  /// Import under this folder (a view in --ws) rather than at the root.
  #[arg(long)]
  parent: Option<Uuid>,
  /// Name for the new workspace. Only meaningful with a single archive —
  /// several would all land on the same name.
  #[arg(long)]
  name: Option<String>,
  /// Force Notion adaptation (otherwise auto-detected from the contents).
  #[arg(long)]
  notion: bool,
  /// Do NOT pull externally-linked images into Mica.
  ///
  /// Much faster when the archive links to hosts this server cannot reach —
  /// those cost a timeout each. The links keep working; run
  /// `mica-cli rehost-images` afterwards from a network that CAN reach them.
  #[arg(long)]
  no_rehost: bool,
}

#[derive(Subcommand)]
enum PageCmd {
  /// List pages and folders. Prints the VIEW id (for move/trash) and the
  /// OBJECT id (for read) side by side, because they are not the same id.
  List(PageListArgs),
  /// Print pages' Markdown to stdout. Several ids are fetched in one request.
  Read(PageReadArgs),
  /// Move pages/folders under one parent, in the order given. One request.
  Move(PageMoveArgs),
  /// Send pages/folders (with their subtrees) to the recycle bin. One request,
  /// reversible with `trash restore`.
  Trash(PageTrashArgs),
}

#[derive(Args)]
struct PageListArgs {
  #[arg(long)]
  ws: Uuid,
  /// Only what is under this VIEW id. Omit to start at the top level.
  #[arg(long)]
  parent: Option<Uuid>,
  /// Levels to descend; 1 = direct children only. Omit for all.
  #[arg(long)]
  depth: Option<i32>,
  #[arg(long)]
  limit: Option<i64>,
  #[arg(long)]
  offset: Option<i64>,
  /// Add each page's stored size. SMALL means nearly empty — this is how you
  /// find stub pages without reading every one of them. Large does NOT mean
  /// long: deleted text leaves weight behind, so it is not a word count.
  #[arg(long)]
  with_stats: bool,
}

#[derive(Args)]
struct PageReadArgs {
  #[arg(long)]
  ws: Uuid,
  /// The pages' OBJECT ids (the `object=` column of `page list`). Give several
  /// and they are fetched in ONE request — surveying a workspace should not be
  /// one round trip per page. A page that cannot be read reports inline instead
  /// of failing the rest.
  #[arg(required = true)]
  document_ids: Vec<Uuid>,
}

#[derive(Args)]
struct PageMoveArgs {
  #[arg(long)]
  ws: Uuid,
  /// Destination FOLDER's view id. Omit to move to the workspace root. Only a
  /// folder can hold children; a page id is refused with a reason.
  #[arg(long)]
  to: Option<Uuid>,
  /// VIEW ids to move.
  #[arg(required = true)]
  view_ids: Vec<Uuid>,
}

#[derive(Args)]
struct PageTrashArgs {
  #[arg(long)]
  ws: Uuid,
  /// Required. Bulk deletion is reversible here, but still deliberate.
  #[arg(long)]
  confirm: bool,
  /// VIEW ids to trash. A folder takes its whole subtree with it.
  #[arg(required = true)]
  view_ids: Vec<Uuid>,
}

#[derive(Subcommand)]
enum TrashCmd {
  /// List what is in the recycle bin.
  List(WsArgs),
  /// Restore views (with the subtrees deleted alongside them). One request.
  Restore(TrashRestoreArgs),
  /// PERMANENTLY delete everything in the bin. Not recoverable.
  Empty(TrashEmptyArgs),
}

#[derive(Args)]
struct WsArgs {
  #[arg(long)]
  ws: Uuid,
}

#[derive(Args)]
struct TrashRestoreArgs {
  #[arg(long)]
  ws: Uuid,
  /// VIEW ids from `trash list`.
  #[arg(required = true)]
  view_ids: Vec<Uuid>,
}

#[derive(Args)]
struct TrashEmptyArgs {
  #[arg(long)]
  ws: Uuid,
  /// Required, and it means it: unlike `page trash`, this cannot be undone.
  #[arg(long)]
  confirm: bool,
}

#[derive(Subcommand)]
enum BackupCmd {
  /// Run one backup now and exit non-zero if it covered less than it should.
  Run,
  /// Run one on start, then once a day at BACKUP_HOUR. The container entrypoint.
  ///
  /// A failed run is logged and pinged to the dead man's switch, never fatal —
  /// the loop has to survive a bad night to try again the next one.
  Daemon,
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
    Command::Import(args) => cmd_import(&cli, &cfg, args),
    Command::Page(PageCmd::List(args)) => cmd_page_list(&cli, &cfg, args),
    Command::Page(PageCmd::Read(args)) => cmd_page_read(&cli, &cfg, args),
    Command::Page(PageCmd::Move(args)) => cmd_page_move(&cli, &cfg, args),
    Command::Page(PageCmd::Trash(args)) => cmd_page_trash(&cli, &cfg, args),
    Command::Trash(TrashCmd::List(args)) => cmd_trash_list(&cli, &cfg, args),
    Command::Trash(TrashCmd::Restore(args)) => cmd_trash_restore(&cli, &cfg, args),
    Command::Trash(TrashCmd::Empty(args)) => cmd_trash_empty(&cli, &cfg, args),
    Command::Export(args) => cmd_export(&cli, &cfg, args),
    Command::Backup(BackupCmd::Run) => cmd_backup_run(&cli, &cfg),
    Command::Backup(BackupCmd::Daemon) => cmd_backup_daemon(&cli, &cfg),
    Command::Mcp(args) => cmd_mcp(&cli, &cfg, args),
    Command::RehostImages(args) => cmd_rehost_images(&cli, &cfg, args),
  }
}

/// Sweep documents and pull every external image link into Mica storage, using
/// this machine's network for the download (the server can't reach hosts like
/// AppFlowy that block its datacenter IP). Idempotent: blocks already on a
/// file_id are skipped, so re-running only touches what's still a link.
fn cmd_rehost_images(cli: &Cli, cfg: &Config, args: &RehostImagesArgs) -> Result<()> {
  let client = authed_client(cli, cfg)?;
  let workspaces: Vec<Uuid> = match args.ws {
    Some(ws) => vec![ws],
    None => client.list_workspaces()?.into_iter().map(|w| w.id).collect(),
  };

  let (mut rehosted, mut failed) = (0usize, 0usize);
  for ws in workspaces {
    for view in client.list_views(ws)? {
      if view.object_type != "document" || view.is_deleted {
        continue;
      }
      let boot = client.document_bootstrap(ws, view.object_id)?;
      let blocks = boot
        .get("snapshot")
        .and_then(|s| s.get("payload"))
        .and_then(|p| p.get("blocks"))
        .and_then(|b| b.as_array())
        .cloned()
        .unwrap_or_default();
      for block in &blocks {
        if block.get("kind").and_then(|k| k.as_str()) != Some("image") {
          continue;
        }
        let Some(data) = block.get("data") else { continue };
        // Already a Mica file → nothing to do.
        if data.get("file_id").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty()) {
          continue;
        }
        let Some(url) = data
          .get("url")
          .and_then(|u| u.as_str())
          .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        else {
          continue;
        };
        let Some(block_id) = block.get("id").and_then(|i| i.as_str()) else {
          continue;
        };
        let file_name = url_file_name(url);
        let outcome = client
          .download_bytes(url)
          .and_then(|bytes| client.rehost_image(ws, view.object_id, block_id, &file_name, bytes));
        match outcome {
          Ok(()) => {
            rehosted += 1;
            if !cli.json {
              println!("  re-hosted [{}] {url}", view.name);
            }
          }
          Err(err) => {
            failed += 1;
            if !cli.json {
              eprintln!("  FAILED   [{}] {url}: {err:#}", view.name);
            }
          }
        }
      }
    }
  }

  if cli.json {
    print_json(&serde_json::json!({ "rehosted": rehosted, "failed": failed }))?;
  } else {
    println!("Done: {rehosted} re-hosted, {failed} failed.");
  }
  Ok(())
}

// ------------------------------------------------------------------- import

fn cmd_import(cli: &Cli, cfg: &Config, args: &ImportArgs) -> Result<()> {
  if args.name.is_some() && args.archives.len() > 1 {
    bail!("--name works with one archive; {} were given (drop it and each is named after its file)", args.archives.len());
  }
  let client = authed_client(cli, cfg)?;
  let mut results: Vec<serde_json::Value> = Vec::new();

  for path in &args.archives {
    let zip = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    // Workspace name from the filename, the way the app names an import.
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
    let name = args.name.as_deref().or(if args.ws.is_some() { None } else { Some(stem) });

    if !cli.json {
      println!("==> {} ({} bytes)", path.display(), zip.len());
    }
    let job_id = client.start_import(
      zip,
      name,
      args.ws,
      args.parent,
      args.notion,
      !args.no_rehost,
    )?;

    // Watch to completion. The server owns the job — this loop only reports.
    let mut last_done = usize::MAX;
    let job = loop {
      let job = client.import_job(job_id)?;
      if !cli.json && job.total > 0 && job.done != last_done {
        println!("    {}/{} pages", job.done, job.total);
        last_done = job.done;
      }
      if job.status != "running" {
        break job;
      }
      std::thread::sleep(std::time::Duration::from_millis(1500));
    };

    // Report per archive and KEEP GOING. One bad archive in a batch of twenty
    // must not throw away the nineteen that worked — and it has to be named,
    // or the run looks like a clean sweep.
    if !cli.json {
      match job.status.as_str() {
        "done" => println!(
          "    done: {} page(s){}",
          job.done,
          if job.skipped.is_empty() {
            String::new()
          } else {
            format!(", {} archive entry(ies) unreferenced and skipped", job.skipped.len())
          }
        ),
        other => eprintln!(
          "    {other}: {} page(s) landed{}",
          job.done,
          job.error.as_deref().map(|e| format!(" — {e}")).unwrap_or_default()
        ),
      }
      if let Some(ws) = job.workspace_id {
        println!("    workspace {ws}");
      }
    }
    results.push(serde_json::json!({
      "archive": path.to_string_lossy(),
      "workspace_id": job.workspace_id,
      "status": job.status,
      "done": job.done,
      "total": job.total,
      "skipped": job.skipped.len(),
    }));
  }

  if cli.json {
    print_json(&results)?;
  }
  // Non-zero when ANY archive did not finish cleanly, so a script can tell.
  if results.iter().any(|r| r["status"] != "done") {
    bail!("one or more archives did not import cleanly (see above)");
  }
  Ok(())
}

// ------------------------------------------------------- pages & recycle bin

/// One row per view. The two ids are printed together on purpose: `view_id`
/// is what move/trash take, `object_id` is what read takes, and reaching for
/// the wrong one is the mistake this layout exists to prevent.
fn print_views(views: &[client::View], with_stats: bool) {
  if views.is_empty() {
    println!("(none)");
    return;
  }
  for v in views {
    let kind = if v.object_type == "folder" { "folder" } else { "page  " };
    let size = match (with_stats, v.state_bytes) {
      (true, Some(bytes)) => format!("  {bytes:>9}"),
      (true, None) => "          -".to_string(),
      _ => String::new(),
    };
    println!("{kind}  view={}  object={}{size}  {}", v.id, v.object_id, v.name);
  }
  println!("\n{} view(s).", views.len());
}

fn cmd_page_list(cli: &Cli, cfg: &Config, args: &PageListArgs) -> Result<()> {
  let client = authed_client(cli, cfg)?;
  let filter = client::ViewFilter {
    parent_view_id: args.parent,
    depth: args.depth,
    limit: args.limit,
    offset: args.offset,
    with_stats: args.with_stats,
  };
  let views = client.list_views_filtered(args.ws, &filter)?;
  if cli.json {
    print_json(&serde_json::json!({ "views": views }))?;
  } else {
    print_views(&views, args.with_stats);
  }
  Ok(())
}

fn cmd_page_read(cli: &Cli, cfg: &Config, args: &PageReadArgs) -> Result<()> {
  let client = authed_client(cli, cfg)?;

  // One id keeps the single-page endpoint so `page read … > out.md` writes the
  // Markdown and nothing else. Several take the batch endpoint: one round trip.
  if let [document_id] = args.document_ids.as_slice() {
    let markdown = client.read_markdown(args.ws, *document_id)?;
    if cli.json {
      print_json(&serde_json::json!({ "markdown": markdown }))?;
    } else {
      println!("{markdown}");
    }
    return Ok(());
  }

  let result = client.batch_read_documents(args.ws, &args.document_ids)?;
  if cli.json {
    print_json(&result)?;
    return Ok(());
  }
  for doc in result.get("documents").and_then(|d| d.as_array()).unwrap_or(&vec![]) {
    let id = doc.get("document_id").and_then(|v| v.as_str()).unwrap_or("?");
    match doc.get("error").and_then(|v| v.as_str()) {
      // Named, not swallowed: a survey that quietly drops pages is worse than
      // one that says which it could not read.
      Some(error) => eprintln!("--- {id}: {error}"),
      None => {
        println!("--- {id}");
        println!("{}", doc.get("markdown").and_then(|v| v.as_str()).unwrap_or(""));
      }
    }
  }
  Ok(())
}

fn cmd_page_move(cli: &Cli, cfg: &Config, args: &PageMoveArgs) -> Result<()> {
  let client = authed_client(cli, cfg)?;
  let outcome = client.batch_move_views(args.ws, &args.view_ids, args.to)?;
  report_batch(cli, "moved", &outcome)
}

fn cmd_page_trash(cli: &Cli, cfg: &Config, args: &PageTrashArgs) -> Result<()> {
  if !args.confirm {
    bail!(
      "refusing to trash {} view(s) without --confirm (reversible with `mica-cli trash restore`)",
      args.view_ids.len()
    );
  }
  let client = authed_client(cli, cfg)?;
  let outcome = client.batch_trash_views(args.ws, &args.view_ids)?;
  report_batch(cli, "trashed", &outcome)
}

fn cmd_trash_list(cli: &Cli, cfg: &Config, args: &WsArgs) -> Result<()> {
  let client = authed_client(cli, cfg)?;
  let views = client.list_trash(args.ws)?;
  if cli.json {
    print_json(&serde_json::json!({ "views": views }))?;
  } else {
    print_views(&views, false);
  }
  Ok(())
}

fn cmd_trash_restore(cli: &Cli, cfg: &Config, args: &TrashRestoreArgs) -> Result<()> {
  let client = authed_client(cli, cfg)?;
  let outcome = client.batch_restore_views(args.ws, &args.view_ids)?;
  report_batch(cli, "restored", &outcome)
}

fn cmd_trash_empty(cli: &Cli, cfg: &Config, args: &TrashEmptyArgs) -> Result<()> {
  if !args.confirm {
    bail!("refusing to empty the recycle bin without --confirm — this cannot be undone");
  }
  let client = authed_client(cli, cfg)?;
  let result = client.empty_trash(args.ws)?;
  if cli.json {
    print_json(&result)?;
  } else {
    println!("Recycle bin emptied: {result}");
  }
  Ok(())
}

/// Report what a batch call DID, never what was asked of it. `affected` counts
/// subtrees too, so it is normally larger than the id list; `skipped` names the
/// ids that were already gone. Printing the request size as the result is the
/// failure `reorder_views` was fixed for — a partial run reads as a full one.
fn report_batch(cli: &Cli, verb: &str, outcome: &client::BatchOutcome) -> Result<()> {
  if cli.json {
    print_json(outcome)?;
    return Ok(());
  }
  println!("{} {verb} (subtrees included).", outcome.affected);
  if !outcome.skipped.is_empty() {
    println!(
      "{} requested id(s) were skipped — already in that state, or gone:",
      outcome.skipped.len()
    );
    for id in &outcome.skipped {
      println!("  {id}");
    }
  }
  Ok(())
}

/// Last path segment of a URL as a filename (drops the query), defaulting to
/// "image" when there is nothing usable — the server derives the mime from it.
fn url_file_name(url: &str) -> String {
  url
    .split('?')
    .next()
    .unwrap_or(url)
    .rsplit('/')
    .next()
    .filter(|s| !s.is_empty())
    .unwrap_or("image")
    .to_string()
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

/// One scheduled backup, then a verdict on what it actually covered.
///
/// The export runs IN PROCESS — this binary already is the exporter, so the old
/// arrangement (a shell script shelling out to `mica-cli export`) had a Rust
/// program being conducted by bash. Now it is the other way round.
fn cmd_backup_run(cli: &Cli, cfg: &Config) -> Result<()> {
  let settings = backup::Settings::from_env()?;
  let report = backup::run_once(&settings, |dir| {
    cmd_export(
      cli,
      cfg,
      &ExportArgs {
        out: dir.to_path_buf(),
        ws: None,
        no_prune: false,
      },
    )
  })?;
  backup::log(&format!("run finished:\n{}", report.summary()));
  // The verdict is separate from the run on purpose: everything below the
  // failure line still happened, and the operator needs to be told what DID get
  // backed up even when the answer is "not enough".
  backup::verdict(&report, settings.allow_partial)
}

/// The container entrypoint: one run on start, then daily at `BACKUP_HOUR`.
///
/// A failed run must never kill the loop — tonight's outage is not a reason to
/// stop trying tomorrow — so failures are logged and pinged, not propagated.
fn cmd_backup_daemon(cli: &Cli, cfg: &Config) -> Result<()> {
  let hour: u32 = std::env::var("BACKUP_HOUR")
    .ok()
    .and_then(|h| h.parse().ok())
    .filter(|h| *h < 24)
    .unwrap_or(3);
  let on_start = std::env::var("BACKUP_ON_START").as_deref() != Ok("0");

  let run = |first: bool| {
    if !first {
      backup::log("starting scheduled run");
    }
    match cmd_backup_run(cli, cfg) {
      Ok(()) => {
        backup::log("run ok");
        backup::ping_healthcheck("");
      }
      Err(error) => {
        eprintln!("[{}] mica-backup: run FAILED — {error:#}", chrono::Local::now().to_rfc3339());
        backup::ping_healthcheck("/fail");
      }
    }
  };

  if on_start {
    run(true);
  }
  loop {
    let wait = backup::seconds_until(chrono::Local::now(), hour);
    backup::log(&format!("sleeping {wait}s until {hour:02}:00"));
    std::thread::sleep(std::time::Duration::from_secs(wait as u64));
    run(false);
  }
}

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
  // Same resolver as every other command — see [`pick_token`] for why this is
  // no longer a second, slightly different chain.
  let pat = resolve_token(cli, cfg).with_context(no_token_hint)?;
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

/// The token to authenticate with, in precedence order.
///
/// Pure so the order can be tested; [`resolve_token`] is the thin wrapper that
/// reads the environment.
///
/// ONE variable, `MICA_PAT`, and one definition of the order. Both used to be
/// plural: the `mcp` subcommand read `MICA_PAT` while every other command read
/// only `MICA_TOKEN`, so following the MCP docs and then running `mica-cli ws
/// list` reported "not logged in" without saying which name it wanted
/// (2026-08-12). `MICA_PAT` is the survivor because it is the one the docs have
/// always told people to set, so it is the one already in real MCP configs —
/// and because the value it holds literally begins `mica_pat_`.
///
/// BLANK IS NOT A TOKEN. `MICA_PAT=` in a systemd unit or a compose file
/// resolves to an empty string, not to "unset" — spending it earns a 401 that
/// says nothing about the real problem. Same rule the server learned for
/// `JWT_SECRET` (see `resolve_jwt_secret`).
pub(crate) fn pick_token(
  flag: Option<&str>,
  pat_env: Option<&str>,
  saved: Option<&str>,
) -> Option<String> {
  [flag, pat_env, saved]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|value| !value.is_empty())
    .map(str::to_string)
}

fn resolve_token(cli: &Cli, cfg: &Config) -> Option<String> {
  let pat = std::env::var("MICA_PAT").ok();
  pick_token(cli.token.as_deref(), pat.as_deref(), cfg.token.as_deref())
}

/// What to say when nothing resolved.
///
/// Checks for the RETIRED name and says so outright. Dropping a variable that
/// silently stops working is the failure this whole change is about — someone
/// with `MICA_TOKEN` set would otherwise read "not signed in" while staring at a
/// correctly-spelled token they had exported themselves.
fn no_token_hint() -> String {
  let stale = std::env::var("MICA_TOKEN")
    .ok()
    .is_some_and(|v| !v.trim().is_empty());
  if stale {
    return "not signed in — MICA_TOKEN is no longer read; rename it to MICA_PAT \
            (or pass --token, or run `mica-cli auth login`)"
      .to_string();
  }
  "not signed in — pass --token, set MICA_PAT, or run `mica-cli auth login`".to_string()
}

pub(crate) fn authed_client(cli: &Cli, cfg: &Config) -> Result<Client> {
  let server = cli
    .server
    .clone()
    .or_else(|| cfg.server.clone())
    .context("no server — run `mica-cli auth login --server <url>` or set MICA_SERVER")?;
  let token = resolve_token(cli, cfg).with_context(no_token_hint)?;
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

// -------------------------------------------------------------------- tests
//
// Only the pure, network-free logic is covered here: filename/slug derivation,
// zip-entry path sanitization, workspace directory naming, and the mirror
// reconcile that the backup sidecar relies on. The REST `Client` methods need a
// live server and are deliberately out of scope.
#[cfg(test)]
mod tests {
  use super::*;

  /// The token order, pinned — it used to be written twice and the two copies
  /// accepted different variable names.
  #[test]
  fn the_flag_beats_the_environment_and_the_saved_login() {
    assert_eq!(
      pick_token(Some("flag"), Some("pat"), Some("saved")).as_deref(),
      Some("flag")
    );
    assert_eq!(
      pick_token(None, Some("pat"), Some("saved")).as_deref(),
      Some("pat")
    );
    assert_eq!(pick_token(None, None, Some("saved")).as_deref(), Some("saved"));
  }

  #[test]
  fn a_blank_value_is_not_a_token_and_does_not_shadow_the_next_source() {
    // `MICA_PAT=` in a unit file resolves to an empty string, not to unset.
    // Spending it buys a 401 that explains nothing; falling through to the
    // saved login is what the user meant. Same rule as `resolve_jwt_secret`.
    assert_eq!(
      pick_token(None, Some(""), Some("saved")).as_deref(),
      Some("saved")
    );
    assert_eq!(pick_token(None, Some("   "), None), None);
    assert_eq!(pick_token(Some(""), None, None), None);
  }

  #[test]
  fn surrounding_whitespace_is_trimmed() {
    // A trailing newline is what `MICA_PAT=$(cat token.txt)` gives you, and it
    // travels all the way into the Authorization header.
    assert_eq!(
      pick_token(None, Some(" mica_pat_x\n"), None).as_deref(),
      Some("mica_pat_x")
    );
  }

  #[test]
  fn nothing_anywhere_is_none() {
    assert_eq!(pick_token(None, None, None), None);
  }

  #[test]
  fn url_file_name_variants() {
    // Normal path → last segment.
    assert_eq!(url_file_name("https://example.com/a/b/photo.png"), "photo.png");
    // Query string is dropped (the server derives the mime from the name).
    assert_eq!(url_file_name("https://example.com/a/photo.png?v=123&x=y"), "photo.png");
    // Query directly after the segment.
    assert_eq!(url_file_name("https://example.com/img?token=abc"), "img");
    // Trailing slash → nothing usable → default.
    assert_eq!(url_file_name("https://example.com/"), "image");
    // No slash at all in the (query-stripped) string → the whole thing.
    assert_eq!(url_file_name("noslash"), "noslash");
    // Empty input → default.
    assert_eq!(url_file_name(""), "image");
    // Only a query → default.
    assert_eq!(url_file_name("?foo=bar"), "image");
  }

  #[test]
  fn slugify_basics() {
    assert_eq!(slugify("My Notes"), "my-notes");
    // Runs of non-alphanumerics collapse to a single dash, none leading/trailing.
    assert_eq!(slugify("  Hello,  World!! "), "hello-world");
    assert_eq!(slugify("already-slug"), "already-slug");
    // Non-ASCII is dropped (ascii_alphanumeric only).
    assert_eq!(slugify("笔记 Notes"), "notes");
    // Nothing usable → empty (caller falls back to an id).
    assert_eq!(slugify("你好"), "");
    assert_eq!(slugify(""), "");
  }

  #[test]
  fn sanitize_rel_blocks_traversal() {
    use std::path::Component;
    // `..`, absolute prefixes, and the root are all stripped — the result can
    // only ever contain Normal components, so it stays inside the target dir.
    for input in ["../../etc/passwd", "/etc/passwd", "a/../../b", "./a/b"] {
      let out = sanitize_rel(input);
      assert!(
        out.components().all(|c| matches!(c, Component::Normal(_))),
        "sanitize_rel({input:?}) leaked a non-Normal component: {out:?}"
      );
    }
    // A clean relative path is preserved.
    assert_eq!(sanitize_rel("pages/note.md"), PathBuf::from("pages").join("note.md"));
    // The dangerous parts are dropped but the safe tail survives.
    assert_eq!(sanitize_rel("../../etc/passwd"), PathBuf::from("etc").join("passwd"));
  }

  fn ws(name: &str, id: Uuid) -> client::Workspace {
    client::Workspace {
      id,
      name: name.to_string(),
      owner_id: Uuid::nil(),
      role: "owner".to_string(),
      created_at: String::new(),
      updated_at: String::new(),
    }
  }

  #[test]
  fn workspace_dir_naming() {
    let id = Uuid::parse_str("0123456789abcdef0123456789abcdef").unwrap();
    // Unique slug → clean, no id suffix.
    let counts: HashMap<String, usize> = [("my-notes".to_string(), 1)].into_iter().collect();
    assert_eq!(workspace_dir(&ws("My Notes", id), &counts), "my-notes");

    // Two workspaces collide on the same slug → disambiguate with 8 hex chars.
    let counts: HashMap<String, usize> = [("notes".to_string(), 2)].into_iter().collect();
    assert_eq!(workspace_dir(&ws("Notes", id), &counts), "notes-01234567");

    // Empty/unslugifiable name → `workspace-<8hex>`.
    let counts: HashMap<String, usize> = HashMap::new();
    assert_eq!(workspace_dir(&ws("你好", id), &counts), "workspace-01234567");
  }

  /// A unique scratch directory under the OS temp dir (no env mutation, no
  /// collision with parallel tests).
  fn scratch(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("mica-cli-test-{tag}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
  }

  #[test]
  fn mirror_writes_updates_and_prunes() {
    let out = scratch("mirror");

    // First mirror: two files, both written.
    let mut desired: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
    desired.insert(PathBuf::from("a.md"), b"alpha".to_vec());
    desired.insert(Path::new("sub").join("b.md"), b"bravo".to_vec());
    let stats = mirror(&out, &desired, true).unwrap();
    assert_eq!((stats.total, stats.written, stats.pruned), (2, 2, 0));
    assert_eq!(fs::read(out.join("a.md")).unwrap(), b"alpha");

    // Re-mirror identical content: nothing rewritten (minimal diff for restic).
    let stats = mirror(&out, &desired, true).unwrap();
    assert_eq!((stats.written, stats.pruned), (0, 0));

    // Change one file, drop the other: one written, the stale file + its now-empty
    // dir pruned.
    let mut desired: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
    desired.insert(PathBuf::from("a.md"), b"alpha-2".to_vec());
    let stats = mirror(&out, &desired, true).unwrap();
    assert_eq!((stats.total, stats.written, stats.pruned), (1, 1, 1));
    assert_eq!(fs::read(out.join("a.md")).unwrap(), b"alpha-2");
    assert!(!out.join("sub").join("b.md").exists());
    assert!(!out.join("sub").exists(), "empty dir should be pruned");

    let _ = fs::remove_dir_all(&out);
  }

  #[test]
  fn mirror_no_prune_keeps_stale_files() {
    let out = scratch("noprune");

    let mut desired: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
    desired.insert(PathBuf::from("keep.md"), b"x".to_vec());
    desired.insert(PathBuf::from("gone.md"), b"y".to_vec());
    mirror(&out, &desired, true).unwrap();

    // Drop gone.md but disable pruning → it survives on disk, count stays 0.
    let mut desired: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
    desired.insert(PathBuf::from("keep.md"), b"x".to_vec());
    let stats = mirror(&out, &desired, false).unwrap();
    assert_eq!(stats.pruned, 0);
    assert!(out.join("gone.md").exists(), "no_prune must keep stale files");

    let _ = fs::remove_dir_all(&out);
  }
}
