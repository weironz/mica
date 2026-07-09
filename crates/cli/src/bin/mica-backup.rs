//! `mica-backup` — Mica's built-in, encrypted, deduplicated backup engine.
//!
//! Powered by `rustic_core` (a Rust library, restic repository-format compatible),
//! embedded IN-PROCESS — no external restic/backrest/cron binary. It snapshots a
//! directory (typically the tree produced by `mica-cli export`) into a repository
//! on local disk or an S3-compatible store (Aliyun OSS) via OpenDAL, with client-
//! side encryption, dedup, incremental transfer, retention, and restore.
//!
//! This whole engine lives behind the crate's `backup` feature, so the default
//! `mica-cli` and the api-server never pull rustic's dependency tree.
//!
//! Repo location is a plain path (local) or `opendal:s3:<bucket>/<path>` (OSS) —
//! the same code path; OSS just needs `--opt access_key_id=… --opt endpoint=…`.
//! The repo password (encryption key) comes from `--password-file` or the
//! `MICA_BACKUP_PASSWORD` env var — never a command-line flag (ps/history leak).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use rustic_backend::BackendOptions;
use rustic_core::{
  BackupOptions, CheckOptions, ConfigOptions, Credentials, ForgetGroups, Grouped, KeepOptions,
  KeyOptions, LocalDestination, LsOptions, PathList, PruneOptions, Repository, RepositoryOptions,
  RestoreOptions, SnapshotGroupCriterion, SnapshotOptions,
};

#[derive(Parser)]
#[command(
  name = "mica-backup",
  version,
  about = "Encrypted, deduplicated backups for Mica (restic-compatible, powered by rustic_core)"
)]
struct Cli {
  /// Repository: a local path, or `opendal:s3:<bucket>/<path>` for S3 / Aliyun OSS.
  #[arg(long, global = true, env = "MICA_BACKUP_REPO")]
  repo: Option<String>,
  /// File holding the repository password (encryption key). Or set MICA_BACKUP_PASSWORD.
  #[arg(long, global = true)]
  password_file: Option<PathBuf>,
  /// Backend option `key=value`, repeatable — e.g. `--opt access_key_id=… --opt endpoint=…`.
  #[arg(long = "opt", global = true, value_parser = parse_kv)]
  opts: Vec<(String, String)>,
  /// Machine-readable JSON output.
  #[arg(long, global = true)]
  json: bool,
  #[command(subcommand)]
  command: Command,
}

#[derive(Subcommand)]
enum Command {
  /// Initialize a new (empty) encrypted repository.
  Init,
  /// Snapshot a directory into the repository (incremental).
  Snapshot(SnapshotArgs),
  /// List snapshots.
  Snapshots,
  /// Restore a snapshot to a directory.
  Restore(RestoreArgs),
  /// Apply a retention policy (and optionally prune the freed data).
  Forget(ForgetArgs),
  /// Verify repository integrity.
  Check,
}

#[derive(Args)]
struct SnapshotArgs {
  /// Directory to back up.
  #[arg(long)]
  path: PathBuf,
  /// Comma-separated tag(s) to attach.
  #[arg(long, default_value = "mica")]
  tag: String,
}

#[derive(Args)]
struct RestoreArgs {
  /// Snapshot to restore: an id, or `latest` / `latest~1` / ….
  #[arg(long, default_value = "latest")]
  snapshot: String,
  /// Destination directory (created if missing).
  #[arg(long)]
  target: PathBuf,
}

#[derive(Args)]
struct ForgetArgs {
  #[arg(long)]
  keep_last: Option<i32>,
  #[arg(long)]
  keep_daily: Option<i32>,
  #[arg(long)]
  keep_weekly: Option<i32>,
  #[arg(long)]
  keep_monthly: Option<i32>,
  #[arg(long)]
  keep_yearly: Option<i32>,
  /// Also prune the data freed by forgotten snapshots.
  #[arg(long)]
  prune: bool,
}

fn parse_kv(s: &str) -> Result<(String, String), String> {
  s.split_once('=')
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .ok_or_else(|| format!("expected key=value, got '{s}'"))
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

fn password(cli: &Cli) -> Result<String> {
  if let Some(file) = &cli.password_file {
    let raw = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    return Ok(raw.trim_end_matches(['\r', '\n']).to_string());
  }
  if let Ok(pw) = std::env::var("MICA_BACKUP_PASSWORD") {
    return Ok(pw);
  }
  anyhow::bail!("no repository password — pass --password-file or set MICA_BACKUP_PASSWORD")
}

fn repo_uri(cli: &Cli) -> Result<String> {
  cli
    .repo
    .clone()
    .context("no repository — pass --repo <path | opendal:s3:…> or set MICA_BACKUP_REPO")
}

fn backend_options(cli: &Cli) -> Result<BackendOptions> {
  // Backend options (incl. S3/OSS credentials) may come from the env var
  // MICA_BACKUP_OPTS ("k=v k2=v2 …") so secrets stay off argv/ps — CLI `--opt`
  // flags override. The systemd unit puts the whole OSS config there.
  let mut map: BTreeMap<String, String> = BTreeMap::new();
  if let Ok(env_opts) = std::env::var("MICA_BACKUP_OPTS") {
    for pair in env_opts.split_whitespace() {
      if let Some((k, v)) = pair.split_once('=') {
        map.insert(k.to_string(), v.to_string());
      }
    }
  }
  for (k, v) in &cli.opts {
    map.insert(k.clone(), v.clone());
  }
  let mut opts = BackendOptions::default().repository(repo_uri(cli)?);
  if !map.is_empty() {
    opts = opts.options(map);
  }
  Ok(opts)
}

fn credentials(cli: &Cli) -> Result<Credentials> {
  Ok(Credentials::password(password(cli)?))
}

fn run(cli: Cli) -> Result<()> {
  match &cli.command {
    Command::Init => cmd_init(&cli),
    Command::Snapshot(args) => cmd_snapshot(&cli, args),
    Command::Snapshots => cmd_snapshots(&cli),
    Command::Restore(args) => cmd_restore(&cli, args),
    Command::Forget(args) => cmd_forget(&cli, args),
    Command::Check => cmd_check(&cli),
  }
}

fn cmd_init(cli: &Cli) -> Result<()> {
  let backends = backend_options(cli)?.to_backends().context("configuring backend")?;
  Repository::new(&RepositoryOptions::default(), &backends)?
    .init(
      &credentials(cli)?,
      &KeyOptions::default(),
      &ConfigOptions::default(),
    )
    .context("initializing repository")?;
  let uri = repo_uri(cli)?;
  if cli.json {
    println!("{}", serde_json::json!({ "repo": uri, "initialized": true }));
  } else {
    println!("Initialized repository at {uri}");
  }
  Ok(())
}

fn cmd_snapshot(cli: &Cli, args: &SnapshotArgs) -> Result<()> {
  let backends = backend_options(cli)?.to_backends().context("configuring backend")?;
  let repo = Repository::new(&RepositoryOptions::default(), &backends)?
    .open(&credentials(cli)?)
    .context("unlocking repository (wrong password?)")?
    .to_indexed_ids()?;

  let source = PathList::from_string(&args.path.to_string_lossy())?.sanitize()?;
  let snap = SnapshotOptions::default().add_tags(&args.tag)?.to_snapshot()?;
  let snapshot = repo
    .backup(&BackupOptions::default(), &source, snap)
    .context("running backup")?;

  let summary = snapshot.summary.as_ref();
  if cli.json {
    println!(
      "{}",
      serde_json::json!({
        "snapshot": snapshot.id.to_string(),
        "files_new": summary.map(|s| s.files_new),
        "files_changed": summary.map(|s| s.files_changed),
        "data_added": summary.map(|s| s.data_added),
      })
    );
  } else {
    println!(
      "Snapshot {} — {} new / {} changed files, {} bytes added",
      snapshot.id,
      summary.map(|s| s.files_new).unwrap_or(0),
      summary.map(|s| s.files_changed).unwrap_or(0),
      summary.map(|s| s.data_added).unwrap_or(0),
    );
  }
  Ok(())
}

fn cmd_snapshots(cli: &Cli) -> Result<()> {
  let backends = backend_options(cli)?.to_backends().context("configuring backend")?;
  let repo = Repository::new(&RepositoryOptions::default(), &backends)?.open(&credentials(cli)?)?;
  let snaps = repo.get_all_snapshots()?;

  if cli.json {
    let list: Vec<_> = snaps
      .iter()
      .map(|s| {
        serde_json::json!({
          "id": s.id.to_string(),
          "time": s.time.to_string(),
          "hostname": s.hostname,
          "tags": format!("{}", s.tags),
        })
      })
      .collect();
    println!("{}", serde_json::to_string_pretty(&list)?);
  } else if snaps.is_empty() {
    println!("(no snapshots)");
  } else {
    for s in &snaps {
      println!("{}  {}  [{}]", &s.id.to_string()[..8], s.time, s.tags);
    }
  }
  Ok(())
}

fn cmd_restore(cli: &Cli, args: &RestoreArgs) -> Result<()> {
  let backends = backend_options(cli)?.to_backends().context("configuring backend")?;
  let repo = Repository::new(&RepositoryOptions::default(), &backends)?
    .open(&credentials(cli)?)?
    .to_indexed()?;

  let node = repo.node_from_snapshot_path(&args.snapshot, |_| true)?;
  let ls = repo.ls(&node, &LsOptions::default())?;
  let dest = LocalDestination::new(&args.target.to_string_lossy(), true, !node.is_dir())?;
  let opts = RestoreOptions::default();
  let restore_infos = repo.prepare_restore(&opts, ls.clone(), &dest, false)?;
  repo.restore(restore_infos, &opts, ls, &dest).context("restoring")?;

  if cli.json {
    println!("{}", serde_json::json!({ "restored": args.snapshot, "target": args.target }));
  } else {
    println!("Restored {} → {}", args.snapshot, args.target.display());
  }
  Ok(())
}

fn cmd_forget(cli: &Cli, args: &ForgetArgs) -> Result<()> {
  let backends = backend_options(cli)?.to_backends().context("configuring backend")?;
  let repo = Repository::new(&RepositoryOptions::default(), &backends)?.open(&credentials(cli)?)?;

  let mut keep = KeepOptions::default();
  if let Some(n) = args.keep_last {
    keep = keep.keep_last(n);
  }
  if let Some(n) = args.keep_daily {
    keep = keep.keep_daily(n);
  }
  if let Some(n) = args.keep_weekly {
    keep = keep.keep_weekly(n);
  }
  if let Some(n) = args.keep_monthly {
    keep = keep.keep_monthly(n);
  }
  if let Some(n) = args.keep_yearly {
    keep = keep.keep_yearly(n);
  }

  let snaps = repo.get_all_snapshots()?;
  let grouped = Grouped::from_items(snaps, SnapshotGroupCriterion::default());
  let forget_groups =
    ForgetGroups::from_grouped_snapshots_with_retention(grouped, &keep, &jiff::Zoned::now())?;
  let forget_ids = forget_groups.into_forget_ids();
  let forget_count = forget_ids.len();
  repo.delete_snapshots(&forget_ids)?;

  let mut pruned = false;
  if args.prune {
    // `Repository::prune` is the non-deprecated entry but its signature is still
    // in flux across rustic_core minors; the plan+do_prune path is stable.
    let prune_opts = PruneOptions::default();
    let plan = repo.prune_plan(&prune_opts)?;
    #[allow(deprecated)]
    plan.do_prune::<rustic_core::NoProgressBars, _>(&repo, &prune_opts)?;
    pruned = true;
  }

  if cli.json {
    println!("{}", serde_json::json!({ "forgotten": forget_count, "pruned": pruned }));
  } else {
    println!(
      "Forgot {forget_count} snapshot(s){}",
      if pruned { " + pruned freed data" } else { "" }
    );
  }
  Ok(())
}

fn cmd_check(cli: &Cli) -> Result<()> {
  let backends = backend_options(cli)?.to_backends().context("configuring backend")?;
  let repo = Repository::new(&RepositoryOptions::default(), &backends)?.open(&credentials(cli)?)?;
  repo.check(CheckOptions::default()).context("integrity check failed")?;
  if cli.json {
    println!("{}", serde_json::json!({ "ok": true }));
  } else {
    println!("Repository OK.");
  }
  Ok(())
}
