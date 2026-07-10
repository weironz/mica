//! `mica-cli backup …` — Mica's built-in encrypted, deduplicated backup engine.
//!
//! Behind the `backup` feature (ON by default; `--no-default-features` for a
//! light client with no backup). Powered by `rustic_core` (restic
//! repository-format compatible), embedded IN-PROCESS — no external
//! restic/backrest/cron. Snapshots a directory (typically `mica-cli export`'s
//! output) to a repo on local disk or S3/Aliyun OSS (via OpenDAL), with
//! client-side encryption, dedup, incremental transfer, retention, and restore.
//!
//! Repo is a plain path (local) or `opendal:s3:/<path>` (OSS) — same code; OSS
//! just needs backend `--opt`s. The repo password comes from `--password-file`
//! or `MICA_BACKUP_PASSWORD`; backend options (incl. OSS creds) from `--opt`, a
//! packed `MICA_BACKUP_OPTS="k=v …"`, or one-per-variable `MICA_OPT_<KEY>=<v>`
//! (e.g. `MICA_OPT_BUCKET`) — never a bare flag (ps/history leak).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use rustic_backend::BackendOptions;
use rustic_core::{
  BackupOptions, CheckOptions, ConfigOptions, Credentials, ForgetGroups, Grouped, KeepOptions,
  KeyOptions, LocalDestination, LsOptions, PathList, PruneOptions, Repository, RepositoryOptions,
  RestoreOptions, SnapshotGroupCriterion, SnapshotOptions,
};

#[derive(Args)]
pub struct BackupArgs {
  /// Repository: a local path, or `opendal:s3:/<path>` for S3 / Aliyun OSS.
  #[arg(long, env = "MICA_BACKUP_REPO")]
  repo: Option<String>,
  /// File holding the repository password (encryption key). Or set MICA_BACKUP_PASSWORD.
  #[arg(long)]
  password_file: Option<PathBuf>,
  /// Backend option `key=value`, repeatable — e.g. `--opt access_key_id=…`
  /// (or provide them all via the MICA_BACKUP_OPTS env var).
  #[arg(long = "opt", value_parser = parse_kv)]
  opts: Vec<(String, String)>,
  #[command(subcommand)]
  command: BackupCommand,
}

#[derive(Subcommand)]
enum BackupCommand {
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

pub fn run(json: bool, args: &BackupArgs) -> Result<()> {
  match &args.command {
    BackupCommand::Init => cmd_init(json, args),
    BackupCommand::Snapshot(a) => cmd_snapshot(json, args, a),
    BackupCommand::Snapshots => cmd_snapshots(json, args),
    BackupCommand::Restore(a) => cmd_restore(json, args, a),
    BackupCommand::Forget(a) => cmd_forget(json, args, a),
    BackupCommand::Check => cmd_check(json, args),
  }
}

fn password(args: &BackupArgs) -> Result<String> {
  if let Some(file) = &args.password_file {
    let raw = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    return Ok(raw.trim_end_matches(['\r', '\n']).to_string());
  }
  if let Ok(pw) = std::env::var("MICA_BACKUP_PASSWORD") {
    return Ok(pw);
  }
  anyhow::bail!("no repository password — pass --password-file or set MICA_BACKUP_PASSWORD")
}

fn repo_uri(args: &BackupArgs) -> Result<String> {
  args
    .repo
    .clone()
    .context("no repository — pass --repo <path | opendal:s3:…> or set MICA_BACKUP_REPO")
}

fn backend_options(args: &BackupArgs) -> Result<BackendOptions> {
  // Backend options (incl. S3/OSS credentials) come from the environment so
  // secrets stay off argv/ps. Two styles, lowest-to-highest precedence:
  //   1. MICA_BACKUP_OPTS="k=v k2=v2 …"  — one packed string (handy for a
  //      systemd EnvironmentFile).
  //   2. MICA_OPT_<KEY>=<value>          — one option per variable, so a
  //      compose/.env reads one-per-line; the key is the suffix lowercased,
  //      e.g. MICA_OPT_ACCESS_KEY_ID → access_key_id.
  // CLI `--opt k=v` flags override both.
  let mut map: BTreeMap<String, String> = BTreeMap::new();
  if let Ok(env_opts) = std::env::var("MICA_BACKUP_OPTS") {
    for pair in env_opts.split_whitespace() {
      if let Some((k, v)) = pair.split_once('=') {
        map.insert(k.to_string(), v.to_string());
      }
    }
  }
  for (k, v) in std::env::vars() {
    if let Some(key) = k.strip_prefix("MICA_OPT_") {
      // Skip empties: an unset `${OSS_ROOT:-}` still passes MICA_OPT_ROOT="".
      if !key.is_empty() && !v.is_empty() {
        map.insert(key.to_ascii_lowercase(), v);
      }
    }
  }
  for (k, v) in &args.opts {
    map.insert(k.clone(), v.clone());
  }
  let mut opts = BackendOptions::default().repository(repo_uri(args)?);
  if !map.is_empty() {
    opts = opts.options(map);
  }
  Ok(opts)
}

fn credentials(args: &BackupArgs) -> Result<Credentials> {
  Ok(Credentials::password(password(args)?))
}

fn cmd_init(json: bool, args: &BackupArgs) -> Result<()> {
  let backends = backend_options(args)?.to_backends().context("configuring backend")?;
  Repository::new(&RepositoryOptions::default(), &backends)?
    .init(
      &credentials(args)?,
      &KeyOptions::default(),
      &ConfigOptions::default(),
    )
    .context("initializing repository")?;
  let uri = repo_uri(args)?;
  if json {
    println!("{}", serde_json::json!({ "repo": uri, "initialized": true }));
  } else {
    println!("Initialized repository at {uri}");
  }
  Ok(())
}

fn cmd_snapshot(json: bool, args: &BackupArgs, a: &SnapshotArgs) -> Result<()> {
  let backends = backend_options(args)?.to_backends().context("configuring backend")?;
  let repo = Repository::new(&RepositoryOptions::default(), &backends)?
    .open(&credentials(args)?)
    .context("unlocking repository (wrong password?)")?
    .to_indexed_ids()?;

  let source = PathList::from_string(&a.path.to_string_lossy())?.sanitize()?;
  let snap = SnapshotOptions::default().add_tags(&a.tag)?.to_snapshot()?;
  let snapshot = repo
    .backup(&BackupOptions::default(), &source, snap)
    .context("running backup")?;

  let summary = snapshot.summary.as_ref();
  if json {
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

fn cmd_snapshots(json: bool, args: &BackupArgs) -> Result<()> {
  let backends = backend_options(args)?.to_backends().context("configuring backend")?;
  let repo = Repository::new(&RepositoryOptions::default(), &backends)?.open(&credentials(args)?)?;
  let snaps = repo.get_all_snapshots()?;

  if json {
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

fn cmd_restore(json: bool, args: &BackupArgs, a: &RestoreArgs) -> Result<()> {
  let backends = backend_options(args)?.to_backends().context("configuring backend")?;
  let repo = Repository::new(&RepositoryOptions::default(), &backends)?
    .open(&credentials(args)?)?
    .to_indexed()?;

  let node = repo.node_from_snapshot_path(&a.snapshot, |_| true)?;
  let ls = repo.ls(&node, &LsOptions::default())?;
  let dest = LocalDestination::new(&a.target.to_string_lossy(), true, !node.is_dir())?;
  let opts = RestoreOptions::default();
  let restore_infos = repo.prepare_restore(&opts, ls.clone(), &dest, false)?;
  repo.restore(restore_infos, &opts, ls, &dest).context("restoring")?;

  if json {
    println!("{}", serde_json::json!({ "restored": a.snapshot, "target": a.target }));
  } else {
    println!("Restored {} → {}", a.snapshot, a.target.display());
  }
  Ok(())
}

fn cmd_forget(json: bool, args: &BackupArgs, a: &ForgetArgs) -> Result<()> {
  let backends = backend_options(args)?.to_backends().context("configuring backend")?;
  let repo = Repository::new(&RepositoryOptions::default(), &backends)?.open(&credentials(args)?)?;

  let mut keep = KeepOptions::default();
  if let Some(n) = a.keep_last {
    keep = keep.keep_last(n);
  }
  if let Some(n) = a.keep_daily {
    keep = keep.keep_daily(n);
  }
  if let Some(n) = a.keep_weekly {
    keep = keep.keep_weekly(n);
  }
  if let Some(n) = a.keep_monthly {
    keep = keep.keep_monthly(n);
  }
  if let Some(n) = a.keep_yearly {
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
  if a.prune {
    // `Repository::prune` is the non-deprecated entry but its signature is still
    // in flux across rustic_core minors; the plan+do_prune path is stable.
    let prune_opts = PruneOptions::default();
    let plan = repo.prune_plan(&prune_opts)?;
    #[allow(deprecated)]
    plan.do_prune::<rustic_core::NoProgressBars, _>(&repo, &prune_opts)?;
    pruned = true;
  }

  if json {
    println!("{}", serde_json::json!({ "forgotten": forget_count, "pruned": pruned }));
  } else {
    println!(
      "Forgot {forget_count} snapshot(s){}",
      if pruned { " + pruned freed data" } else { "" }
    );
  }
  Ok(())
}

fn cmd_check(json: bool, args: &BackupArgs) -> Result<()> {
  let backends = backend_options(args)?.to_backends().context("configuring backend")?;
  let repo = Repository::new(&RepositoryOptions::default(), &backends)?.open(&credentials(args)?)?;
  repo.check(CheckOptions::default()).context("integrity check failed")?;
  if json {
    println!("{}", serde_json::json!({ "ok": true }));
  } else {
    println!("Repository OK.");
  }
  Ok(())
}
