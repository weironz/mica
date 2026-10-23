//! Scheduled backup, in Rust — the replacement for `deploy/mica-backup.sh` and
//! `deploy/mica-backup-loop.sh`.
//!
//! # Why this stopped being a shell script
//!
//! The old script degraded silently. With `MICA_BACKUP_PGURL` unset it logged a
//! WARN, skipped the database, backed up the content, and **exited 0** — so the
//! dead man's switch stayed green while production had no off-site copy of its
//! database for months (`docs/ops/dr-plan.md` §1.1). Nobody wrote that bug: it is
//! what "partial success" looks like in a language where a run is an exit code.
//!
//! Here a run is a [`Report`] of typed [`Leg`]s. A leg either `Ran` or was
//! `Skipped` **with the reason**, and skipping is tolerated only when the
//! operator says so out loud (`MICA_BACKUP_ALLOW_PARTIAL=1`). By default an
//! unconfigured leg FAILS the run, because the failure this exists to prevent is
//! not "the backup broke" — it is "the backup was smaller than you believed".
//!
//! # What did NOT move into Rust
//!
//! `rustic`, `rclone` and `pg_dump` stay external processes. They are the
//! mechanism (encryption, dedup, retention, object mirroring); this module is
//! the policy (which legs, in what order, and what counts as success). Same
//! split the shell version had — see `docs/ops/backup.md`.

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as Proc, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// One leg's outcome. `Skipped` carries WHY, because a reason that lives only in
/// a log line is a reason nobody reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leg {
  Ran,
  Skipped(String),
  /// The leg was configured and TRIED, and it broke.
  ///
  /// Distinct from [`Leg::Skipped`] on purpose: "you did not ask for this" and
  /// "you asked and I could not" are different facts, and the run's exit code
  /// only owes you non-zero for the second. Before this existed, a failing leg
  /// aborted `run_once` with `?` — which sounds strict and is actually the
  /// worst of both, because the legs AFTER it never ran. See [`run_once`].
  Failed(String),
}

impl Leg {
  fn skipped(reason: impl Into<String>) -> Self {
    Leg::Skipped(reason.into())
  }

  fn failed(reason: impl Into<String>) -> Self {
    Leg::Failed(reason.into())
  }
}

/// Whether the content leg really ran, given how many workspaces the export
/// announced and which of them never reached the repository.
///
/// The two counts come from the same token and the same manifest, so they are
/// equal by construction — which is exactly why a difference has to fail the
/// run: something between "listed it" and "stored it" dropped a workspace.
/// (Reconciling against `mica-cli ws list` instead would be comparing the
/// manifest with itself, and would never catch anything.)
///
/// Split out from [run_once] because that function shells out to rustic and
/// rclone, so the decision it makes could not otherwise be tested — and this
/// decision is the whole feature: before it, a missing workspace was a log line
/// and the run still reported success.
fn coverage_leg(total: usize, missed: &[String]) -> Leg {
  if missed.is_empty() {
    return Leg::Ran;
  }
  Leg::skipped(format!(
    "{} of {total} workspace(s) were listed but never snapshotted: {}",
    missed.len(),
    missed.join(", ")
  ))
}

/// What a run did, leg by leg.
#[derive(Debug, Default)]
pub struct Report {
  pub legs: Vec<(&'static str, Leg)>,
}

impl Report {
  fn add(&mut self, name: &'static str, leg: Leg) {
    self.legs.push((name, leg));
  }

  /// The legs that did not run, with their reasons.
  pub fn skipped(&self) -> Vec<(&'static str, &str)> {
    self
      .legs
      .iter()
      .filter_map(|(name, leg)| match leg {
        Leg::Skipped(why) => Some((*name, why.as_str())),
        // Deliberately NOT here: this list feeds "what was not covered, and you
        // told me so". A FAILED leg is a different sentence and gets its own —
        // folding it in would let a breakage read as a configuration choice.
        Leg::Failed(_) | Leg::Ran => None,
      })
      .collect()
  }

  /// One line per leg, for the log and for the failure message.
  pub fn summary(&self) -> String {
    self
      .legs
      .iter()
      .map(|(name, leg)| match leg {
        Leg::Ran => format!("  {name}: ran"),
        Leg::Skipped(why) => format!("  {name}: SKIPPED — {why}"),
        Leg::Failed(why) => format!("  {name}: FAILED — {why}"),
      })
      .collect::<Vec<_>>()
      .join("\n")
  }
}

/// Everything a run reads from the environment, resolved and validated up front
/// so a missing value fails before `pg_dump` has written half a file.
pub struct Settings {
  pub rustic: String,
  pub rclone: String,
  pub export_dir: PathBuf,
  pub oss: Oss,
  pub pgurl: Option<String>,
  pub blob: Option<Blob>,
  pub keep_daily: String,
  pub keep_weekly: String,
  pub keep_monthly: String,
  /// Accept a run with skipped legs. Off by default — see the module docs.
  pub allow_partial: bool,
}

/// The rustic repository backend (Aliyun OSS through opendal's S3 support).
pub struct Oss {
  pub bucket: String,
  pub endpoint: String,
  pub region: String,
  pub access_key_id: String,
  pub secret_access_key: String,
  pub root: String,
}

/// The object-mirror leg: RustFS (source) → OSS (destination), S3 to S3.
pub struct Blob {
  pub src_access_key_id: String,
  pub src_secret_access_key: String,
  pub src_endpoint: String,
  pub src_bucket: String,
  pub dst_bucket: String,
  pub dst_root: String,
}

/// Read an env var, treating empty as absent.
///
/// `FOO=` is not a value. Accepting it as "set" is how a half-filled .env
/// template reaches production looking configured.
fn env(key: &str) -> Option<String> {
  match std::env::var(key) {
    Ok(v) if !v.trim().is_empty() => Some(v),
    _ => None,
  }
}

fn require(key: &str) -> Result<String> {
  env(key).with_context(|| format!("{key} is required for the backup"))
}

impl Settings {
  pub fn from_env() -> Result<Self> {
    let oss = Oss {
      bucket: require("OSS_BUCKET")?,
      endpoint: require("OSS_ENDPOINT")?,
      region: require("OSS_REGION")?,
      access_key_id: require("OSS_ACCESS_KEY_ID")?,
      secret_access_key: require("OSS_SECRET_ACCESS_KEY")?,
      root: env("OSS_ROOT").unwrap_or_else(|| "mica".into()),
    };

    // The blob leg needs its whole set or none of it — half-configured, it would
    // fail deep inside rclone with a message about the wrong thing.
    let blob = match (
      env("RUSTFS_S3_ACCESS_KEY_ID"),
      env("RUSTFS_S3_SECRET_ACCESS_KEY"),
      env("OSS_BLOB_BUCKET"),
    ) {
      (Some(ak), Some(sk), Some(bucket)) => Some(Blob {
        src_access_key_id: ak,
        src_secret_access_key: sk,
        src_endpoint: env("RUSTFS_S3_ENDPOINT").unwrap_or_else(|| "http://rustfs:9000".into()),
        src_bucket: env("RUSTFS_S3_BUCKET").unwrap_or_else(|| "mica".into()),
        dst_bucket: bucket,
        dst_root: env("OSS_BLOB_ROOT").unwrap_or_else(|| "mica-blobs".into()),
      }),
      _ => None,
    };

    Ok(Settings {
      rustic: env("RUSTIC").unwrap_or_else(|| "rustic".into()),
      rclone: env("RCLONE").unwrap_or_else(|| "rclone".into()),
      export_dir: env("MICA_EXPORT_DIR")
        .unwrap_or_else(|| "/var/lib/mica/export".into())
        .into(),
      oss,
      pgurl: env("MICA_BACKUP_PGURL"),
      blob,
      keep_daily: env("KEEP_DAILY").unwrap_or_else(|| "7".into()),
      keep_weekly: env("KEEP_WEEKLY").unwrap_or_else(|| "4".into()),
      keep_monthly: env("KEEP_MONTHLY").unwrap_or_else(|| "6".into()),
      allow_partial: matches!(
        env("MICA_BACKUP_ALLOW_PARTIAL").as_deref(),
        Some("1" | "true" | "yes" | "on")
      ),
    })
  }
}

pub fn log(msg: &str) {
  println!("[{}] mica-backup: {msg}", chrono::Local::now().to_rfc3339());
}

/// Write a config file that holds secrets: owner-only, and replaced atomically
/// so a reader never sees a half-written credential block.
fn write_secret_file(path: &Path, contents: &str) -> Result<()> {
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
  }
  let tmp = path.with_extension("tmp");
  {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
      use std::os::unix::fs::OpenOptionsExt;
      opts.mode(0o600);
    }
    let mut file = opts
      .open(&tmp)
      .with_context(|| format!("opening {}", tmp.display()))?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
  }
  std::fs::rename(&tmp, path).with_context(|| format!("installing {}", path.display()))?;
  Ok(())
}

/// Tool invocations have an overall deadline, including time spent in retry
/// backoff. Connection timeouts alone do not bound a whole backup leg.
const TOOL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
const PGDUMP_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Capture the small `rclone size --json` answer on disk, not in an unbounded
/// `.output()` buffer. stderr remains visible in the container log.
fn tool_stdout(bin: &str, args: &[&str], what: &str) -> Result<String> {
  use std::io::{Read, Seek};
  let path = std::env::temp_dir().join(format!("mica-tool-{}", uuid::Uuid::new_v4()));
  let result = (|| -> Result<String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
      use std::os::unix::fs::OpenOptionsExt;
      options.mode(0o600);
    }
    let mut out = options
      .open(&path)
      .context("creating the tool output file")?;
    let mut command = Proc::new(bin);
    command.args(args).stdout(out.try_clone()?);
    run_tool_with_timeout(&mut command, what, TOOL_TIMEOUT)?;
    out.rewind()?;
    let mut bytes = Vec::new();
    out.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 {
      bail!("{what} returned more than 64 KiB — refusing an unexpected response");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
  })();
  let _ = std::fs::remove_file(path);
  result
}

/// `count` and `bytes` out of `rclone size --json`.
///
/// Hand-parsed rather than pulling in serde for two integers: this runs in the
/// backup image, and the shape is two fields of a one-line object.
fn rclone_size(rclone: &str, remote: &str) -> Result<(u64, u64)> {
  let out = tool_stdout(
    rclone,
    &rclone_args(&["size", "--json", remote]),
    "rclone size",
  )?;
  parse_rclone_size(&out).with_context(|| {
    format!("could not read count/bytes out of `rclone size --json {remote}`: {out}")
  })
}

/// Split out from [`rclone_size`] so it can be tested without rclone.
fn parse_rclone_size(out: &str) -> Option<(u64, u64)> {
  let field = |name: &str| -> Option<u64> {
    let at = out.find(&format!("\"{name}\""))? + name.len() + 2;
    let rest = out[at..].trim_start_matches([':', ' ']);
    let end = rest
      .find(|c: char| !c.is_ascii_digit())
      .unwrap_or(rest.len());
    rest[..end].parse().ok()
  };
  Some((field("count")?, field("bytes")?))
}

/// Every rclone invocation, with `--config` already on it.
///
/// Not a convenience: `render_rclone_conf` writes to /etc/rclone/rclone.conf,
/// which is NOT where rclone looks by default (~/.config/rclone/rclone.conf).
/// Forgetting the flag does not fail loudly at the config layer — rclone starts
/// fine and dies later with `didn't find section in config file`, naming the
/// REMOTE, so it reads as "your credentials are wrong" rather than "you did not
/// hand me the config". That is exactly how the first automated DR drill failed
/// (2026-08-29): the backup leg had the flag, the restore leg written days
/// later did not, and no test could see the difference because the argument
/// list is only assembled at the call site. Now there is one call site.
fn rclone_args<'a>(args: &[&'a str]) -> Vec<&'a str> {
  let mut all = vec![
    "--config",
    RCLONE_CONF,
    "--transfers",
    "2",
    "--checkers",
    "2",
    "--retries",
    "2",
    "--low-level-retries",
    "3",
    "--retries-sleep",
    "10s",
    "--contimeout",
    "10s",
    "--timeout",
    "60s",
    "--max-duration",
    "30m",
    "--cutoff-mode",
    "hard",
  ];
  all.extend_from_slice(args);
  all
}

fn run_tool(bin: &str, args: &[&str], what: &str) -> Result<()> {
  run_tool_with_timeout(Proc::new(bin).args(args), what, TOOL_TIMEOUT)
}

/// Spawn each Unix command in its own process group: killing only a shell's
/// PID leaves its children (and their I/O/retries) running after the deadline.
/// Arguments are intentionally absent from errors because pg_dump's URL may
/// contain a password.
pub(crate) fn run_tool_with_timeout(
  command: &mut Proc,
  what: &str,
  timeout: Duration,
) -> Result<()> {
  #[cfg(unix)]
  {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
  }
  let mut child = command
    .spawn()
    .with_context(|| format!("could not start {what}"))?;
  let started = Instant::now();
  loop {
    match child.try_wait() {
      Ok(Some(status)) => return tool_status(status, what),
      Ok(None) if started.elapsed() < timeout => {
        std::thread::sleep(
          Duration::from_millis(50).min(timeout.saturating_sub(started.elapsed())),
        );
      }
      outcome => {
        terminate_tool_tree(&mut child);
        // Reap even after kill, or the daemon accumulates zombies. If an OS
        // call fails, retain the timeout/wait error as the primary reason.
        let reaped = child.wait();
        if let Err(error) = outcome {
          return Err(error).with_context(|| format!("waiting for {what}"));
        }
        reaped.with_context(|| format!("reaping timed-out {what}"))?;
        bail!(
          "{what} timed out after {}s — terminated its process tree",
          timeout.as_secs_f64()
        );
      }
    }
  }
}

/// Publish only a complete dump. Keep the previous complete SQL on failure,
/// remove the partial file, and let the caller record failure before trying
/// the independent object/config legs.
fn write_database_dump(command: &mut Proc, directory: &Path, timeout: Duration) -> Result<()> {
  let tmp = directory.join("mica.sql.tmp");
  let result = (|| -> Result<()> {
    std::fs::create_dir_all(directory)?;
    command.stdout(std::fs::File::create(&tmp)?);
    run_tool_with_timeout(command, "pg_dump", timeout)?;
    std::fs::rename(&tmp, directory.join("mica.sql")).context("installing the complete pg_dump")?;
    Ok(())
  })();
  if result.is_err() {
    let _ = std::fs::remove_file(tmp);
  }
  result
}

fn tool_status(status: ExitStatus, what: &str) -> Result<()> {
  if !status.success() {
    bail!("{what} failed ({status})");
  }
  Ok(())
}

fn terminate_tool_tree(child: &mut std::process::Child) {
  #[cfg(unix)]
  {
    unsafe extern "C" {
      fn kill(pid: std::ffi::c_int, signal: std::ffi::c_int) -> std::ffi::c_int;
    }
    // SAFETY: kill takes two integer values. process_group(0) above gives
    // this child a group whose id is its PID; a negative id targets that group.
    // SIGKILL is 9 on the Unix targets supported by this CLI.
    unsafe {
      kill(-(child.id() as std::ffi::c_int), 9);
    }
  }
  #[cfg(windows)]
  {
    let _ = Proc::new("taskkill")
      .args(["/PID", &child.id().to_string(), "/T", "/F"])
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status();
  }
  // Fallback if the group/tree has already exited or OS tree termination fails.
  let _ = child.kill();
}

/// Where the node's credentials file is mounted into this container, read-only.
///
/// A constant and not an env var on purpose: WHICH path it lands on is compose's
/// business, and another knob here is another thing that can be pointed at the
/// wrong file — silently backing up an empty one.
const CONFIG_FILE: &str = "/etc/mica/env.secrets";

const RUSTIC_CONF: &str = "/etc/rustic/rustic.toml";
const RCLONE_CONF: &str = "/etc/rclone/rclone.conf";

/// Render the rustic repo config.
///
/// The repo PASSWORD is deliberately absent — it stays in `RUSTIC_PASSWORD` in
/// the environment, so a leaked config file is not a leaked repository.
/// `enable_virtual_host_style` is required for Aliyun OSS; path style 404s.
fn render_rustic_conf(oss: &Oss) -> Result<()> {
  let conf = format!(
    r#"[repository]
repository = "opendal:s3"

[repository.options]
bucket = "{}"
endpoint = "{}"
region = "{}"
access_key_id = "{}"
secret_access_key = "{}"
root = "{}"
enable_virtual_host_style = "true"
"#,
    oss.bucket, oss.endpoint, oss.region, oss.access_key_id, oss.secret_access_key, oss.root
  );
  write_secret_file(Path::new(RUSTIC_CONF), &conf)
}

fn render_rclone_conf(blob: &Blob, oss: &Oss) -> Result<()> {
  let conf = format!(
    r#"[rustfs]
type = s3
provider = Other
access_key_id = {}
secret_access_key = {}
endpoint = {}
force_path_style = true

[ossblob]
type = s3
provider = Alibaba
access_key_id = {}
secret_access_key = {}
endpoint = {}
"#,
    blob.src_access_key_id,
    blob.src_secret_access_key,
    blob.src_endpoint,
    oss.access_key_id,
    oss.secret_access_key,
    oss.endpoint
  );
  write_secret_file(Path::new(RCLONE_CONF), &conf)
}

#[derive(serde::Deserialize)]
struct ManifestWorkspace {
  id: String,
  name: String,
  dir: String,
}

#[derive(serde::Deserialize)]
struct Manifest {
  workspaces: Vec<ManifestWorkspace>,
}

/// Decide whether a report is acceptable, and say precisely what is missing if
/// it is not.
///
/// This is the whole point of the rewrite, so it is a free function with its own
/// tests rather than a branch buried inside the run.
/// Put an instance's credentials back BEFORE its stack exists.
///
/// Resolves a chicken-and-egg the 2026-08-29 drill walked straight into: the
/// credentials come out of the backup (leg 3c), but reading the backup needs
/// `rustic` plus a repository config — and both live in this image, which on a
/// fresh machine is the only thing there is. So this runs standalone:
///
///     docker run --rm -e OSS_… -e RUSTIC_PASSWORD=… -v /data/mica:/restore ///       mica-cli backup restore-config
///
/// It is a command and not a documented `rustic.toml` snippet on purpose. The
/// repository options have exactly one correct spelling (`enable_virtual_host_style`
/// is required for OSS and was found the hard way), and a copy of them in a
/// runbook is a second representation that drifts from `render_rustic_conf`
/// without anything noticing — the failure this repo keeps relearning.
///
/// ORDER MATTERS: run this BEFORE bringing the stack up. `POSTGRES_PASSWORD` is
/// in the restored file and Postgres uses it at initdb, so a stack started
/// first is a database created with one password while the config names
/// another — and nothing says so until something tries to connect.
pub fn restore_config(settings: &Settings, out: &Path) -> Result<()> {
  render_rustic_conf(&settings.oss)?;
  log(&format!("restore _config → {}", out.display()));
  run_tool(
    &settings.rustic,
    &[
      "restore",
      "latest",
      &out.to_string_lossy(),
      "--filter-label",
      "_config",
    ],
    "rustic restore (config)",
  )?;
  // Say where it landed rather than leaving the caller to guess: rustic restores
  // the ABSOLUTE path it snapshotted, so the file appears under the destination
  // at the same path it had on the original node.
  let landed = out.join(CONFIG_FILE.trim_start_matches('/'));
  if landed.is_file() {
    log(&format!("credentials at {}", landed.display()));
  } else {
    bail!(
      "restore reported success but {} is not there — the snapshot may predate        v0.13.40, when the credentials leg was added (check: rustic snapshots        --filter-label _config)",
      landed.display()
    );
  }
  Ok(())
}

/// Pull the object bytes back: OSS → this instance's RustFS. The mirror leg of
/// [`run_once`] run backwards.
///
/// Why a command and not a documented rclone invocation: the 2026-08-29 drill
/// did it by hand, with the two remotes spelled out as `RCLONE_CONFIG_*`
/// environment variables — a second copy of what `render_rclone_conf` already
/// knows (`force_path_style` for RustFS, `provider = Alibaba` for OSS, both
/// found the hard way). Two spellings of one fact is the drift this repo keeps
/// relearning; there is now one.
///
/// AND it is not allowed to succeed quietly: `rclone copy` exits 0 having
/// copied nothing just as happily as having copied everything, so the drill's
/// real criterion — the two sides agree on object count AND bytes — is checked
/// here rather than left in a runbook for someone to remember.
pub fn restore_objects(settings: &Settings) -> Result<()> {
  let blob = settings.blob.as_ref().context(
    "the object leg is not configured (RUSTFS_S3_* / OSS_BLOB_BUCKET) — nothing to restore from",
  )?;
  render_rclone_conf(blob, &settings.oss)?;
  let src = format!("ossblob:{}/{}", blob.dst_bucket, blob.dst_root);
  let dst = format!("rustfs:{}", blob.src_bucket);

  log(&format!("restore objects {src} → {dst}"));
  run_tool(
    &settings.rclone,
    &rclone_args(&["copy", &src, &dst, "--stats", "30s"]),
    "rclone copy (restore)",
  )?;

  let (src_n, src_b) = rclone_size(&settings.rclone, &src)?;
  let (dst_n, dst_b) = rclone_size(&settings.rclone, &dst)?;
  log(&format!(
    "source {src_n} objects / {src_b} bytes; destination {dst_n} / {dst_b}"
  ));
  if (src_n, src_b) != (dst_n, dst_b) {
    bail!(
      "restore did not converge: {src} has {src_n} objects / {src_b} bytes but        {dst} has {dst_n} / {dst_b}. Images will be missing for exactly the        documents whose blobs did not arrive — a page still opens, the picture        is just broken, which is why this is checked and not eyeballed."
    );
  }
  log("objects restored and verified");
  Ok(())
}

pub fn verdict(report: &Report, allow_partial: bool) -> Result<()> {
  let failed: Vec<String> = report
    .legs
    .iter()
    .filter_map(|(name, leg)| match leg {
      Leg::Failed(reason) => Some(format!("{name}: {reason}")),
      _ => None,
    })
    .collect();
  if !failed.is_empty() {
    bail!("{} leg(s) failed — {}", failed.len(), failed.join(" | "));
  }

  let skipped = report.skipped();
  if skipped.is_empty() || allow_partial {
    return Ok(());
  }
  let detail = skipped
    .iter()
    .map(|(name, why)| format!("  {name}: {why}"))
    .collect::<Vec<_>>()
    .join("\n");
  bail!(
    "backup covered less than a full instance — these legs did not run:\n{detail}\n\
     Configure them, or set MICA_BACKUP_ALLOW_PARTIAL=1 to accept a partial backup \
     on purpose. Failing quietly here is what left production without an off-site \
     database copy for months (docs/ops/dr-plan.md §1.1)."
  )
}

/// Keep unrelated legs runnable when a tool fails or reaches its deadline.
fn record_leg(report: &mut Report, name: &'static str, result: Result<Leg>) {
  match result {
    Ok(leg) => report.add(name, leg),
    Err(error) => {
      let reason = format!("{error:#}");
      log(&format!("{name} leg FAILED — {reason}"));
      report.add(name, Leg::failed(reason));
    }
  }
}

/// Everything one scheduled run does.
///
/// The caller supplies the exporter invocation; production runs a child with a
/// deadline, while tests can supply a deterministic failure without network IO.
/// The inode stays in place across runs and container restarts. Only the OS
/// lock determines ownership: stale PID text (especially PID 1) is irrelevant.
const RUN_LOCK: &str = "/tmp/mica-backup.lock";

/// Hold an exclusive OS file lock until drop. Never unlink it: removing a
/// locked file lets a contender create a different inode and acquire a second
/// lock while the first process is still writing the same staging files.
#[derive(Debug)]
pub struct RunLock {
  _file: std::fs::File,
}

impl RunLock {
  pub fn acquire() -> Result<Self> {
    Self::acquire_at(Path::new(RUN_LOCK))
  }

  fn acquire_at(path: &Path) -> Result<Self> {
    let mut file = std::fs::OpenOptions::new()
      .read(true)
      .write(true)
      .create(true)
      .truncate(false)
      .open(path)
      .context("opening the backup run lock")?;
    match file.try_lock() {
      Ok(()) => {}
      Err(std::fs::TryLockError::WouldBlock) => {
        bail!(
          "another backup run is in progress — refusing to start a second. Two runs write the same staging files and destroy each other."
        );
      }
      Err(std::fs::TryLockError::Error(error)) => {
        return Err(error).context("locking the backup run lock");
      }
    }
    // Diagnostic text only; write it after acquiring, never before.
    file.set_len(0)?;
    write!(file, "{}", std::process::id())?;
    Ok(Self { _file: file })
  }
}

pub fn run_once(settings: &Settings, export: impl FnOnce(&Path) -> Result<()>) -> Result<Report> {
  // Before anything touches the staging tree. Held for the whole run; released
  // by Drop, including on the `bail!`s below.
  let _lock = RunLock::acquire()?;
  let mut report = Report::default();

  render_rustic_conf(&settings.oss)?;

  // The repo must have been initialized once, deliberately NOT here: a
  // misconfigured backend must fail loudly rather than quietly create a second,
  // empty repository that then looks like a working backup.
  run_tool_with_timeout(
    Proc::new(&settings.rustic)
      .args(["cat", "config"])
      .stdout(Stdio::null())
      .stderr(Stdio::null()),
    "rustic repository probe",
    PROBE_TIMEOUT,
  ).context("rustic repo not initialized, or the backend is unreachable. Initialize once with: docker exec mica-backup-1 rustic init")?;

  // 1) Content: every workspace as Markdown + images.
  //
  // NOT `?`. A content-export failure used to abort the whole run, and the two
  // legs below — the database dump and the object bytes — have no dependency on
  // it whatsoever. Production spent 2026-07-31 .. 08-29 with NO off-site
  // database copy for exactly that reason: the export could not authenticate
  // (`MICA_TOKEN` retired in the CLI, never renamed in `deploy/docker-compose.yml`),
  // and the dump that would still have worked never got the chance. The most
  // irreplaceable leg was being held hostage by the least.
  //
  // Partial success is still not success: a failed leg is recorded and
  // `run_once` returns Err at the end (see the report check below). What
  // changed is only that every leg gets to try first — which is the opposite of
  // the original shell sin, where a missing leg logged a WARN and exited 0.
  log(&format!(
    "export all workspaces → {}",
    settings.export_dir.display()
  ));
  let content_failure = match export(&settings.export_dir) {
    Ok(()) => None,
    Err(error) => {
      let reason = format!("{error:#}");
      log(&format!("content export FAILED — {reason}"));
      log("continuing to the database and object legs — they do not depend on it");
      Some(reason)
    }
  };

  // 2) Database. UNCOMPRESSED on purpose: gzip scrambles content similarity, so
  //    two dumps of a barely-changed database dedup to almost nothing and every
  //    day costs a full copy. Plain SQL lets rustic's content-defined chunking
  //    see that most of today's dump is yesterday's, and it compresses the
  //    chunks itself. (The hand-taken `pre-*.sql.gz` restore points are a
  //    different path and stay gzipped — one-off files, not a lineage.)
  //
  //    MEASURED on production 2026-08-29, so this is not just an argument:
  //
  //      processed 1 files, 634.2 MiB
  //      Added to the repo: 9.4 MiB (raw: 48.6 MiB)
  //
  //    67x. And the line above it read `using no parent` — the container id is
  //    the rustic hostname and changes every deploy, so there was no parent
  //    snapshot to diff against. It deduplicated anyway, because the dedup is
  //    content-defined chunking, not a diff against a parent. Gzip would have
  //    made all 634 MiB look new every single day.
  let pgdump_dir = settings.export_dir.join("_pgdump");
  let dump = pgdump_dir.join("mica.sql");
  match &settings.pgurl {
    Some(url) => {
      log(&format!("pg_dump → {}", dump.display()));
      let result = write_database_dump(
        Proc::new("pg_dump").args(["--no-owner", "--no-privileges", url]),
        &pgdump_dir,
        PGDUMP_TIMEOUT,
      );
      if let Err(error) = result {
        let reason = format!("{error:#}");
        log(&format!("database leg FAILED — {reason}"));
        report.add("database", Leg::failed(reason));
      } else {
        report.add("database", Leg::Ran);
      }
      // Drop any stale gzip from before the switch to plain SQL, or the snapshot
      // below carries both and the restore runbook has two files to choose from.
      let _ = std::fs::remove_file(pgdump_dir.join("mica.sql.gz"));
    }
    None => report.add(
      "database",
      Leg::skipped(
        "MICA_BACKUP_PGURL unset — no accounts, memberships, CRDT history, comments or share tokens are backed up",
      ),
    ),
  }

  // 3) Snapshot each workspace — ONLY when this run's export produced a
  //     manifest. A failed export leaves either no manifest or yesterday's, and
  //     snapshotting against a stale one would re-store yesterday's directories
  //     under today's date: a backup that looks fresh and is not. That is worse
  //     than an obviously missing one, so the leg is recorded as failed instead.
  let content_result = match &content_failure {
    Some(reason) => Err(anyhow::anyhow!(reason.clone())),
    None => (|| -> Result<Leg> {
      // 3) Snapshot each workspace as its own retention lineage: label = the STABLE
      //    workspace id (a rename never splits history), tag = the readable name.
      let manifest_path = settings.export_dir.join("manifest.json");
      let manifest: Manifest = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
          .with_context(|| format!("reading {}", manifest_path.display()))?,
      )
      .context("parsing the export manifest")?;

      let mut count = 0usize;
      let mut missed: Vec<String> = Vec::new();
      for ws in &manifest.workspaces {
        let path = settings.export_dir.join(&ws.dir);
        if !path.is_dir() {
          // A workspace the export announced but did not write. This used to be a
          // log line and nothing else: the run still reported success and still
          // pinged the healthcheck, so a workspace could quietly stop being backed
          // up while every signal said the backups were fine. Collected here and
          // turned into a skipped leg below — the whole point of the dead man's
          // switch is that "less than you think" and "everything" must not look
          // the same.
          log(&format!(
            "MISSING {}: {} not written",
            ws.id,
            path.display()
          ));
          missed.push(format!("{} (ws={})", ws.id, ws.name));
          continue;
        }
        log(&format!(
          "snapshot {} (ws={}) → label={}",
          ws.dir, ws.name, ws.id
        ));
        run_tool(
          &settings.rustic,
          &[
            "backup",
            &path.to_string_lossy(),
            "--label",
            &ws.id,
            "--tag",
            &format!("ws={}", ws.name),
            "--tag",
            "mica",
          ],
          "rustic backup",
        )?;
        count += 1;
      }
      log(&format!(
        "snapshotted {count}/{} workspace(s)",
        manifest.workspaces.len()
      ));
      Ok(coverage_leg(manifest.workspaces.len(), &missed))
    })(),
  };
  record_leg(&mut report, "content", content_result);

  // 3b) The dump gets its own lineage (stable label `_pgdump`, never a workspace
  //     id) so retention below applies to it on the same policy.
  if report
    .legs
    .iter()
    .any(|(name, leg)| *name == "database" && *leg == Leg::Ran)
  {
    // A failed dump may leave yesterday's complete file in place. Never label
    // that old file as a fresh backup after this run failed or timed out.
    log("snapshot pg_dump → label=_pgdump");
    let result = run_tool(
      &settings.rustic,
      &[
        "backup",
        &pgdump_dir.to_string_lossy(),
        "--label",
        "_pgdump",
        "--tag",
        "pgdump",
        "--tag",
        "mica",
      ],
      "rustic backup (pgdump)",
    );
    record_leg(&mut report, "database snapshot", result.map(|()| Leg::Ran));
  }

  // 3c) The credentials themselves — the one class of thing that was backed up
  //     NOWHERE. dr-plan.md §2.2 named it in July ("不备这类,前两类备得再好也
  //     白搭") and nothing was done, because the cost of the gap is invisible
  //     until the day you need it. The 2026-08-29 drill made it concrete: a
  //     rebuilt node got all the way to a running, empty stack and then stopped
  //     dead, because restoring the DATA needs credentials that existed only in
  //     one person's password manager.
  //
  //     WHAT THIS CANNOT SOLVE, stated plainly: RUSTIC_PASSWORD opens this
  //     repository and it is inside this file. A copy in the repo cannot help
  //     you open the repo — the combination is in the safe. So it still has to
  //     be carried out of band; what this leg buys is that it is the ONLY one
  //     that must be, instead of thirteen. (2026-08-29: the carried copy lives
  //     in GitHub Secrets, so the recovery path needs no human at all — which
  //     also means whoever holds that repository holds the data. That trade is
  //     recorded in dr-plan §2.2, not made silently here.)
  //
  //     The file is stored as-is: rustic encrypts the whole repository, so these
  //     bytes are exactly as exposed as the documents already beside them.
  if Path::new(CONFIG_FILE).is_file() {
    log("snapshot credentials → label=_config");
    let result = run_tool(
      &settings.rustic,
      &[
        "backup",
        CONFIG_FILE,
        "--label",
        "_config",
        "--tag",
        "config",
        "--tag",
        "mica",
      ],
      "rustic backup (config)",
    );
    record_leg(&mut report, "config", result.map(|()| Leg::Ran));
  } else {
    // Skipped, therefore FAILS the run by default (see `verdict`). That is the
    // point: an instance whose credentials are backed up nowhere cannot be
    // rebuilt from its backups, and finding that out during the rebuild is
    // exactly the failure mode this whole module exists to prevent.
    report.add(
      "config",
      Leg::skipped(format!(
        "{CONFIG_FILE} is not mounted — this instance's credentials are backed up          NOWHERE, so a rebuilt machine cannot be handed them back. Mount          .env.secrets there (deploy/docker-compose.yml does)."
      )),
    );
  }

  // 4) Object bytes, S3 → S3, DELIBERATELY NOT THROUGH RUSTIC. Blobs are
  //    content-addressed and immutable — the app never rewrites a key — so there
  //    is no "the version from three days ago" to restore, and a mirror IS a
  //    backup. Bytes never touch this container's disk, object metadata
  //    (content-type, which decides how a browser renders an image) rides along,
  //    and the result is restorable with any S3 client: no rustic, no password.
  //
  //    `copy`, not `sync`: sync mirrors deletions, so one bad blob_gc would
  //    replicate straight into the backup. copy is append-only; the cost is that
  //    GC'd orphans linger off-site, which at image volumes is not worth caring
  //    about.
  match &settings.blob {
    Some(blob) => {
      render_rclone_conf(blob, &settings.oss)?;
      let src = format!("rustfs:{}", blob.src_bucket);
      let dst = format!("ossblob:{}/{}", blob.dst_bucket, blob.dst_root);
      log(&format!("rclone copy {src} → {dst} (objects, no rustic)"));
      match run_tool(
        &settings.rclone,
        &rclone_args(&[
          "copy",
          &src,
          &dst,
          "--stats-one-line",
          "--stats",
          "30s",
        ]),
        "rclone copy",
      ) {
        // Same rule as the content leg: record and carry on. This one is LAST,
        // so `?` here only ever skipped retention — but it was the same latent
        // shape, and on 2026-08-29 it did fire (403 from RustFS: rclone had been
        // handed a second, stale copy of credentials that only exist once).
        Ok(()) => report.add("objects", Leg::Ran),
        Err(error) => {
          let reason = format!("{error:#}");
          log(&format!("objects leg FAILED — {reason}"));
          report.add("objects", Leg::failed(reason));
        }
      }
    }
    None => report.add(
      "objects",
      Leg::skipped(
        "RUSTFS_S3_ACCESS_KEY_ID / RUSTFS_S3_SECRET_ACCESS_KEY / OSS_BLOB_BUCKET unset — images are only covered indirectly, through the content export",
      ),
    ),
  }

  // 5) Retention PER lineage (group by label), then prune once.
  log(&format!(
    "retention: keep {}d / {}w / {}m per lineage + prune",
    settings.keep_daily, settings.keep_weekly, settings.keep_monthly
  ));
  let retention = run_tool(
    &settings.rustic,
    &[
      "forget",
      "--group-by",
      "label",
      "--keep-daily",
      &settings.keep_daily,
      "--keep-weekly",
      &settings.keep_weekly,
      "--keep-monthly",
      &settings.keep_monthly,
      "--prune",
    ],
    "rustic forget",
  );
  record_leg(&mut report, "retention", retention.map(|()| Leg::Ran));

  // Every leg got its chance; now the run owes an honest verdict. Partial
  // success is NOT success — that was the original shell sin (a missing leg
  // logged a WARN and exited 0, and production went months with no off-site
  // copy). What changed here is only the ORDER: report first, fail after, so
  // one broken leg can no longer take the healthy ones down with it.
  verdict(&report, true)?;

  Ok(report)
}

/// Best-effort dead man's switch. A monitor alerts when the expected success
/// ping stops arriving, which catches a wedged loop or a dead container that no
/// stderr line reaches anyone about.
///
/// Never fatal: a flaky monitor endpoint must not be able to take the backup
/// down with it.
pub fn ping_healthcheck(suffix: &str) {
  let Some(base) = env("HEALTHCHECK_URL") else {
    return;
  };
  let Ok(client) = reqwest::blocking::Client::builder()
    .timeout(std::time::Duration::from_secs(10))
    .build()
  else {
    return;
  };
  // `error_for_status`, not just `send().is_err()`. A transport failure is the
  // OBVIOUS way this breaks and the least likely one: the interesting failure is
  // a URL with a typo, or a check somebody deleted — healthchecks.io answers
  // those with 404, which `send()` reports as SUCCESS. You would then have a
  // dead man's switch that never fires and never says why, which is strictly
  // worse than having none: it is the belief that somebody is watching.
  //
  // Non-fatal on purpose, still: a backup that ran must not be reported as
  // failed because the notification service was down. The line goes to stderr
  // and the run's own verdict is unaffected.
  let outcome = client
    .get(format!("{base}{suffix}"))
    .send()
    .and_then(|r| r.error_for_status());
  if let Err(err) = outcome {
    eprintln!(
      "[{}] mica-backup: healthcheck ping failed (non-fatal): {err}",
      chrono::Local::now().to_rfc3339()
    );
  }
}

/// Seconds until the next `hour`:00 local time.
///
/// Always in the future: if today's slot has already passed, this is
/// tomorrow's. Pure so the scheduling rule is testable without waiting a day.
/// `BACKUP_HOUR` as a wall-clock time: `3`, `03`, `3:00`, `03:30`.
///
/// Returns `None` for anything it cannot read, so the caller can say so out loud
/// instead of silently backing up at some other time — the old code did
/// `.parse().ok().filter(<24).unwrap_or(3)`, which turned a typo into 03:00
/// without a word.
///
/// **The name stays `BACKUP_HOUR` even though it now takes minutes.** A second
/// name for one setting is precisely what caused two of today's incidents
/// (`MICA_TOKEN`/`MICA_PAT`, `S3_*`/`RUSTFS_S3_*`); a slightly narrow name is
/// the cheaper wrong thing than two names that can drift apart.
pub fn parse_backup_at(raw: &str) -> Option<(u32, u32)> {
  let raw = raw.trim();
  if raw.is_empty() {
    return None;
  }
  let (h, m) = match raw.split_once(':') {
    Some((h, m)) => (h, m),
    None => (raw, "0"),
  };
  let hour: u32 = h.trim().parse().ok()?;
  let minute: u32 = m.trim().parse().ok()?;
  (hour < 24 && minute < 60).then_some((hour, minute))
}

pub fn seconds_until(now: chrono::DateTime<chrono::Local>, hour: u32, minute: u32) -> i64 {
  use chrono::{Duration, TimeZone};
  let slot = now
    .date_naive()
    .and_hms_opt(hour.min(23), minute.min(59), 0)
    .unwrap_or_else(|| now.naive_local());
  let mut target = chrono::Local
    .from_local_datetime(&slot)
    .single()
    .unwrap_or(now);
  if target <= now {
    target += Duration::days(1);
  }
  (target - now).num_seconds().max(0)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn report(legs: &[(&'static str, Option<&str>)]) -> Report {
    let mut r = Report::default();
    for (name, skipped) in legs {
      r.add(
        name,
        match skipped {
          Some(why) => Leg::skipped(*why),
          None => Leg::Ran,
        },
      );
    }
    r
  }

  // The bug this module exists to prevent: every leg reported, one of them
  // silently absent, and the run still counted as a success.
  #[test]
  fn a_skipped_leg_fails_the_run_by_default() {
    let r = report(&[("content", None), ("database", Some("PGURL unset"))]);
    let err = verdict(&r, false).unwrap_err().to_string();
    assert!(err.contains("database"), "must name the leg: {err}");
    assert!(err.contains("PGURL unset"), "must give the reason: {err}");
  }

  #[test]
  fn a_full_run_passes() {
    assert!(verdict(&report(&[("content", None), ("database", None)]), false).is_ok());
  }

  /// The credentials leg has to fail LOUDLY when it is not configured, and this
  /// pins that rather than trusting it to stay true. An instance whose
  /// .env.secrets is backed up nowhere cannot be rebuilt from its backups at
  /// all — the 2026-08-29 drill reached a running, empty stack and stopped
  /// there. A silent skip would report that instance as fully backed up, which
  /// is the exact shape of the failure this module was written after.
  /// Two integers out of one line, hand-parsed rather than with serde — so the
  /// shapes rclone actually emits are pinned here. The one that matters is the
  /// ZERO case: a `copy` that transferred nothing exits 0, and `{"count":0}`
  /// parsing as None instead of Some(0) would turn "restored nothing" into
  /// "could not check", which reads as a tooling problem rather than as the
  /// data loss it is.
  /// The invariant that the first automated DR drill died on: `--config` must
  /// be on EVERY rclone invocation, because rclone's default config path is not
  /// the one we render to. Cheap to pin, and the failure it prevents is
  /// expensive to read — rclone names the REMOTE in the error, so a missing
  /// config looks like wrong credentials.
  #[test]
  fn every_rclone_call_is_bounded_and_carries_the_config_path() {
    let args = rclone_args(&["copy", "a:x", "b:y"]);
    assert_eq!(&args[..2], &["--config", RCLONE_CONF]);
    assert!(args.ends_with(&["copy", "a:x", "b:y"]));
    for pair in [
      ["--transfers", "2"],
      ["--checkers", "2"],
      ["--retries", "2"],
      ["--low-level-retries", "3"],
      ["--contimeout", "10s"],
      ["--timeout", "60s"],
      ["--max-duration", "30m"],
      ["--cutoff-mode", "hard"],
    ] {
      assert!(
        args.windows(2).any(|window| window == pair),
        "missing {pair:?}"
      );
    }
  }

  #[test]
  fn rclone_size_json_is_read_correctly() {
    assert_eq!(
      parse_rclone_size(r#"{"count":6467,"bytes":1042495633,"sizeless":0}"#),
      Some((6467, 1042495633))
    );
    // pretty-printed, which rclone emits on a terminal
    assert_eq!(
      parse_rclone_size(
        "{
	\"count\": 12,
	\"bytes\": 34
}"
      ),
      Some((12, 34))
    );
    assert_eq!(parse_rclone_size(r#"{"count":0,"bytes":0}"#), Some((0, 0)));
    assert_eq!(parse_rclone_size("not json at all"), None);
    // present but unparseable must not silently become 0
    assert_eq!(parse_rclone_size(r#"{"count":null,"bytes":null}"#), None);
  }

  #[test]
  fn an_unmounted_credentials_file_fails_the_run() {
    let r = report(&[
      ("content", None),
      ("database", None),
      ("config", Some("/etc/mica/env.secrets is not mounted")),
    ]);
    let err = verdict(&r, false).unwrap_err().to_string();
    assert!(err.contains("config"), "must name the leg: {err}");
    assert!(err.contains("not mounted"), "must give the reason: {err}");
  }

  /// A workspace listed by the export but never written was a log line and
  /// nothing more: `content` was marked `Ran` unconditionally, so the run
  /// passed, the healthcheck was pinged, and the only trace was a line in a
  /// container log nobody reads. The reason names the workspace, because
  /// "some workspace is missing" is not something you can act on.
  fn test_path() -> PathBuf {
    std::env::temp_dir().join(format!("mica-backup-test-{}", uuid::Uuid::new_v4()))
  }

  /// Take the lock right after a `drop` released it.
  ///
  /// flock belongs to the open file DESCRIPTION, and the process-lifecycle
  /// tests in this binary fork helpers concurrently: a child forked in the
  /// instant before the `drop` shares the description until its exec closes it
  /// (CLOEXEC), so an immediate re-acquire can see WouldBlock. That window
  /// failed two of these tests intermittently (0.13.64 release gate). A real
  /// leak never releases, so the deadline still catches one.
  fn reacquire(path: &Path) -> RunLock {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
      match RunLock::acquire_at(path) {
        Ok(lock) => return lock,
        Err(error) => assert!(
          std::time::Instant::now() < deadline,
          "never released: {error:#}"
        ),
      }
      std::thread::sleep(Duration::from_millis(10));
    }
  }

  #[test]
  fn a_second_run_is_refused_while_the_first_holds_the_lock() {
    let path = test_path();
    let first = RunLock::acquire_at(&path).unwrap();
    let msg = format!("{:#}", RunLock::acquire_at(&path).unwrap_err());
    assert!(msg.contains("another backup run is in progress"), "{msg}");
    drop(first);
    // The inode is retained. A third run must still coordinate with a second
    // run that opened the file while the first held it.
    assert!(path.exists());
    let next = reacquire(&path);
    assert!(RunLock::acquire_at(&path).is_err());
    drop(next);
    std::fs::remove_file(path).unwrap();
  }

  #[test]
  fn a_stale_pid_one_file_does_not_block_a_new_run() {
    let path = test_path();
    std::fs::write(&path, "1").unwrap();
    let lock = RunLock::acquire_at(&path).unwrap();
    drop(lock);
    drop(reacquire(&path));
    std::fs::remove_file(path).unwrap();
  }

  fn helper_command(mode: &str) -> Proc {
    let mut command = Proc::new(std::env::current_exe().unwrap());
    command
      .args([
        "--ignored",
        "--exact",
        "backup::tests::tool_process_helper",
        "--nocapture",
      ])
      .env("MICA_BACKUP_TEST_HELPER", mode);
    command
  }

  // A separate OS process exercises the production spawn/wait/lock paths.
  #[test]
  #[ignore = "subprocess helper launched by the process lifecycle tests"]
  fn tool_process_helper() {
    match std::env::var("MICA_BACKUP_TEST_HELPER").unwrap().as_str() {
      "sleep" => std::thread::sleep(Duration::from_secs(30)),
      "partial_dump" => {
        println!("partial new SQL");
        std::thread::sleep(Duration::from_secs(30));
      }
      "fail" => std::process::exit(7),
      "lock" => {
        let path = PathBuf::from(std::env::var_os("MICA_BACKUP_TEST_LOCK").unwrap());
        let _lock = RunLock::acquire_at(&path).unwrap();
        std::fs::write(path.with_extension("ready"), "ready").unwrap();
        std::thread::sleep(Duration::from_secs(30));
      }
      _ => panic!("unknown helper mode"),
    }
  }

  #[test]
  fn a_crashed_process_releases_the_os_lock() {
    let path = test_path();
    let ready = path.with_extension("ready");
    let mut child = helper_command("lock")
      .env("MICA_BACKUP_TEST_LOCK", &path)
      .stdout(Stdio::null())
      .spawn()
      .unwrap();
    let started = Instant::now();
    while !ready.exists() && started.elapsed() < Duration::from_secs(5) {
      std::thread::sleep(Duration::from_millis(10));
    }
    let was_ready = ready.exists();
    let refused = was_ready && RunLock::acquire_at(&path).is_err();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(was_ready, "helper failed to acquire its lock");
    assert!(refused, "a separate process must see the held OS lock");
    let recovered = RunLock::acquire_at(&path).unwrap();
    drop(recovered);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(ready).unwrap();
  }

  #[test]
  fn hung_tools_are_terminated_and_report_a_timeout() {
    let started = Instant::now();
    let err = run_tool_with_timeout(
      &mut helper_command("sleep"),
      "sleeping test tool",
      Duration::from_millis(100),
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(5));
  }

  #[test]
  fn tools_that_exit_unsuccessfully_are_not_reported_as_timeouts() {
    let err = run_tool_with_timeout(
      &mut helper_command("fail"),
      "failing test tool",
      Duration::from_secs(5),
    )
    .unwrap_err();
    let reason = format!("{err:#}");
    assert!(reason.contains("failed"), "{reason}");
    assert!(!reason.contains("timed out"), "{reason}");
  }

  #[test]
  fn a_timed_out_leg_is_recorded_without_aborting_the_next_leg() {
    let mut report = Report::default();
    let failed = run_tool_with_timeout(
      &mut helper_command("sleep"),
      "test snapshot",
      Duration::from_millis(100),
    );
    record_leg(&mut report, "content", failed.map(|()| Leg::Ran));
    record_leg(&mut report, "objects", Ok(Leg::Ran));
    assert!(matches!(report.legs[0], ("content", Leg::Failed(_))));
    assert_eq!(report.legs[1], ("objects", Leg::Ran));
    assert!(verdict(&report, true).is_err());
  }

  #[test]
  fn a_timed_out_dump_keeps_the_previous_complete_file_and_removes_the_partial() {
    let directory = test_path();
    std::fs::create_dir(&directory).unwrap();
    let old_dump = directory.join("mica.sql");
    std::fs::write(&old_dump, "previous complete SQL").unwrap();
    let result = write_database_dump(
      &mut helper_command("partial_dump"),
      &directory,
      Duration::from_millis(100),
    );
    assert!(format!("{:#}", result.unwrap_err()).contains("timed out"));
    assert_eq!(
      std::fs::read_to_string(&old_dump).unwrap(),
      "previous complete SQL"
    );
    assert!(!directory.join("mica.sql.tmp").exists());
    std::fs::remove_dir_all(directory).unwrap();
  }

  #[cfg(unix)]
  #[test]
  fn a_timeout_also_terminates_shell_descendants() {
    let marker = test_path();
    let mut command = Proc::new("sh");
    command
      .args([
        "-c",
        "(touch \"$1.ready\"; sleep 1; touch \"$1\") & wait",
        "test",
      ])
      .arg(&marker)
      .stdout(Stdio::null())
      .stderr(Stdio::null());
    let err = run_tool_with_timeout(
      &mut command,
      "shell with descendant",
      Duration::from_millis(250),
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("timed out"));
    let ready = PathBuf::from(format!("{}.ready", marker.display()));
    assert!(
      ready.exists(),
      "the shell descendant must actually have started"
    );
    std::thread::sleep(Duration::from_millis(1200));
    assert!(!marker.exists(), "the descendant survived the timeout");
    std::fs::remove_file(ready).unwrap();
  }

  #[test]
  fn backup_hour_reads_both_a_bare_hour_and_a_wall_clock_time() {
    // 「配置与实际一致」是这条的由来:BACKUP_HOUR=3 曾经跑在 11:00(容器无 TZ,
    // chrono::Local 静默退化成 UTC)。TZ 修好了「3 点是哪个 3 点」,这条修的是
    // 「3 看起来像不像一个时间」。
    assert_eq!(parse_backup_at("3"), Some((3, 0)));
    assert_eq!(parse_backup_at("03"), Some((3, 0)));
    assert_eq!(parse_backup_at("3:00"), Some((3, 0)));
    assert_eq!(parse_backup_at("03:30"), Some((3, 30)));
    assert_eq!(parse_backup_at(" 23:59 "), Some((23, 59)));

    // 读不出来就是读不出来 —— 交给调用方去出声,而不是在这里假装成 03:00。
    // 旧代码 `.filter(<24).unwrap_or(3)` 会把下面每一个都变成 3 点,不留痕迹。
    for bad in ["", "  ", "25", "3:60", "-1", "3:", "abc", "3:00:00", "3.5"] {
      assert_eq!(parse_backup_at(bad), None, "should not parse: {bad:?}");
    }
  }

  #[test]
  fn the_schedule_honours_minutes() {
    use chrono::TimeZone;
    let now = chrono::Local
      .with_ymd_and_hms(2026, 8, 29, 5, 30, 0)
      .unwrap();
    // 同一天更晚的 05:45 → 15 分钟后
    assert_eq!(seconds_until(now, 5, 45), 15 * 60);
    // 已经过去的 05:15 → 明天,而不是负数或立刻
    assert_eq!(seconds_until(now, 5, 15), 24 * 3600 - 15 * 60);
  }

  #[test]
  fn a_failed_leg_is_not_a_skipped_one_and_shows_up_in_the_summary() {
    // 2026-08-29 事故的形状:内容导出失败过去会 `?` 掉整个 run,于是与它毫无
    // 依赖的 pg_dump 和对象同步一起没跑 —— 生产 29 天没有异地数据库副本。
    // 现在每条腿都跑,失败被记录,run 最后仍然失败。这条钉住两件事:
    //   1. FAILED 不能混进 `skipped()` —— 那份清单的语义是「你没让我做」,
    //      把「我做不到」折进去,故障就读成了配置选择;
    //   2. summary 必须把它显式说出来。
    let mut report = Report::default();
    report.add("content", Leg::failed("operation timed out"));
    report.add("database", Leg::Ran);
    report.add("objects", Leg::skipped("credentials unset"));

    let skipped: Vec<_> = report.skipped().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
      skipped,
      vec!["objects"],
      "只有真正被跳过的才进这份清单: {:?}",
      report.skipped()
    );

    let summary = report.summary();
    assert!(
      summary.contains("content: FAILED — operation timed out"),
      "{summary}"
    );
    assert!(summary.contains("database: ran"), "{summary}");
    assert!(summary.contains("objects: SKIPPED"), "{summary}");
  }

  #[test]
  fn a_workspace_that_never_reached_the_repo_fails_the_run() {
    // Driven through the real decision, not a hand-written leg: the bug was
    // that this branch always produced `Ran`.
    let leg = coverage_leg(24, &["0fc35d86 (ws=IDC)".to_string()]);
    let mut r = Report::default();
    r.add("content", leg);
    r.add("database", Leg::Ran);

    let err = verdict(&r, false).unwrap_err().to_string();
    assert!(err.contains("content"), "must name the leg: {err}");
    assert!(err.contains("ws=IDC"), "must name WHICH workspace: {err}");
    assert!(err.contains("24"), "must show it against the total: {err}");
  }

  #[test]
  fn a_run_that_stored_every_workspace_is_not_marked_skipped() {
    assert_eq!(coverage_leg(24, &[]), Leg::Ran);
  }

  /// Every missing workspace is named. Reporting only a count would leave the
  /// operator diffing two lists by hand at the worst possible moment.
  #[test]
  fn every_missing_workspace_is_named() {
    let leg = coverage_leg(3, &["a (ws=one)".to_string(), "b (ws=two)".to_string()]);
    let Leg::Skipped(why) = leg else {
      panic!("two missing workspaces must not count as a clean run");
    };
    assert!(why.contains("ws=one") && why.contains("ws=two"), "{why}");
    assert!(why.contains("2 of 3"), "{why}");
  }

  /// `allow_partial` is for a deliberately reduced configuration (content-only,
  /// no database). It must NOT wave through a workspace that silently vanished
  /// — that is not a configuration choice, it is data loss in progress. Pinned
  /// because the two look identical at the `Leg::Skipped` level, and the next
  /// person to add an escape hatch will be tempted to reuse this one.
  #[test]
  fn allow_partial_still_reports_a_vanished_workspace_in_the_summary() {
    let r = report(&[(
      "content",
      Some("1 of 24 workspace(s) were listed but never snapshotted: 0fc35d86 (ws=IDC)"),
    )]);
    assert!(
      r.summary().contains("ws=IDC"),
      "the summary is what reaches the log and the failure mail: {}",
      r.summary()
    );
    assert_eq!(r.skipped().len(), 1);
  }

  // Content-only IS a legitimate configuration — it just has to be chosen, not
  // stumbled into.
  #[test]
  fn opting_in_makes_a_partial_run_acceptable() {
    let r = report(&[("content", None), ("objects", Some("no blob remote"))]);
    assert!(verdict(&r, true).is_ok());
  }

  #[test]
  fn the_summary_marks_skipped_legs_loudly() {
    let s = report(&[("content", None), ("objects", Some("no blob remote"))]).summary();
    assert!(s.contains("content: ran"));
    assert!(s.contains("objects: SKIPPED"));
  }

  #[test]
  fn the_next_run_is_always_in_the_future() {
    use chrono::TimeZone;
    let now = chrono::Local
      .with_ymd_and_hms(2026, 7, 31, 5, 30, 0)
      .unwrap();
    // 03:00 already passed today → tomorrow's slot, 21h30m away.
    assert_eq!(seconds_until(now, 3, 0), 21 * 3600 + 30 * 60);
    // 09:00 is still ahead → 3h30m away.
    assert_eq!(seconds_until(now, 9, 0), 3 * 3600 + 30 * 60);
  }
}
