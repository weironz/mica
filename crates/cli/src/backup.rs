//! Scheduled backup, in Rust — the replacement for `deploy/mica-backup.sh` and
//! `deploy/mica-backup-loop.sh`.
//!
//! # Why this stopped being a shell script
//!
//! The old script degraded silently. With `MICA_BACKUP_PGURL` unset it logged a
//! WARN, skipped the database, backed up the content, and **exited 0** — so the
//! dead man's switch stayed green while production had no off-site copy of its
//! database for months (`docs/dr-plan.md` §1.1). Nobody wrote that bug: it is
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
//! split the shell version had — see `docs/backup.md`.

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as Proc;

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

/// Run an external tool, failing with its exit status. Output is inherited so
/// rustic's and rclone's progress reaches the container log unbuffered.
/// Like [`run_tool`] but hands back stdout. Used where the ANSWER matters and
/// not just the exit status — `rclone size`, whose whole point is the number.
fn tool_stdout(bin: &str, args: &[&str], what: &str) -> Result<String> {
  let out = Proc::new(bin)
    .args(args)
    .output()
    .with_context(|| format!("could not start `{bin}` — is it on PATH?"))?;
  if !out.status.success() {
    bail!(
      "{what} failed ({}) — {}",
      out.status,
      String::from_utf8_lossy(&out.stderr).trim()
    );
  }
  Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `count` and `bytes` out of `rclone size --json`.
///
/// Hand-parsed rather than pulling in serde for two integers: this runs in the
/// backup image, and the shape is two fields of a one-line object.
fn rclone_size(rclone: &str, remote: &str) -> Result<(u64, u64)> {
  let out = tool_stdout(rclone, &rclone_args(&["size", "--json", remote]), "rclone size")?;
  parse_rclone_size(&out)
    .with_context(|| format!("could not read count/bytes out of `rclone size --json {remote}`: {out}"))
}

/// Split out from [`rclone_size`] so it can be tested without rclone.
fn parse_rclone_size(out: &str) -> Option<(u64, u64)> {
  let field = |name: &str| -> Option<u64> {
    let at = out.find(&format!("\"{name}\""))? + name.len() + 2;
    let rest = out[at..].trim_start_matches([':', ' ']);
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
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
  let mut all = vec!["--config", RCLONE_CONF];
  all.extend_from_slice(args);
  all
}

fn run_tool(bin: &str, args: &[&str], what: &str) -> Result<()> {
  let status = Proc::new(bin)
    .args(args)
    .status()
    .with_context(|| format!("could not start `{bin}` — is it on PATH?"))?;
  if !status.success() {
    bail!("{what} failed ({status})");
  }
  Ok(())
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
    &rclone_args(&["copy", &src, &dst, "--transfers", "8", "--stats", "30s"]),
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
     database copy for months (docs/dr-plan.md §1.1)."
  )
}

/// Everything one scheduled run does.
///
/// `export` is a closure so the caller hands in the in-process exporter rather
/// than this shelling out to its own binary.
/// Where the run lock lives. `/tmp`, deliberately NOT the export volume: a
/// SIGKILLed run leaves the file behind, and a lock that survives a container
/// restart could match a PID that has since been reused (the daemon is PID 1,
/// so a stale "1" would look alive forever and stop backups for good). `/tmp`
/// is container-local, so a restart clears it — the worst stale lock then lives
/// only as long as the container that made it.
const RUN_LOCK: &str = "/tmp/mica-backup.lock";

/// Whether a lock naming `pid` should be honoured, given whether that process
/// is still alive. Split out so the decision is testable without forking.
///
/// A dead owner's lock is TAKEN, not respected: the alternative is a crashed
/// run silently cancelling every future one.
fn lock_is_live(owner_alive: bool) -> bool {
  owner_alive
}

/// Refuses to start a second concurrent run, and releases on drop.
///
/// Two runs at once corrupt each other rather than merely racing: both write
/// `_pgdump/mica.sql.tmp` and rename it into place, so one of them renames a
/// file the other already moved and dies with `No such file or directory` —
/// an error naming neither the real cause nor the other run. Observed
/// 2026-08-29, when a manual `backup run` collided with the daemon's
/// run-on-start, and it is exactly the collision a DR drill invites.
#[derive(Debug)]
pub struct RunLock {
  path: std::path::PathBuf,
}

impl RunLock {
  pub fn acquire() -> Result<Self> {
    let path = std::path::PathBuf::from(RUN_LOCK);
    loop {
      match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
      {
        Ok(mut file) => {
          use std::io::Write;
          let _ = write!(file, "{}", std::process::id());
          return Ok(Self { path });
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
          let owner = std::fs::read_to_string(&path).unwrap_or_default();
          let pid = owner.trim();
          let alive = !pid.is_empty()
            && std::path::Path::new(&format!("/proc/{pid}")).exists();
          if lock_is_live(alive) {
            bail!(
              "another backup run is in progress (pid {pid}) — refusing to start a second.                Two runs write the same staging files and destroy each other."
            );
          }
          log(&format!(
            "clearing a stale run lock (pid {pid} is gone — a previous run was killed)"
          ));
          // Losing this race is harmless: whoever wins creates the lock, the
          // loser comes back around and sees a LIVE owner.
          let _ = std::fs::remove_file(&path);
        }
        Err(e) => return Err(e).context("creating the backup run lock"),
      }
    }
  }
}

impl Drop for RunLock {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.path);
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
  if Proc::new(&settings.rustic)
    .args(["cat", "config"])
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .status()
    .map(|s| !s.success())
    .unwrap_or(true)
  {
    bail!(
      "rustic repo not initialized, or the backend is unreachable. \
       Initialize once with:  docker exec mica-backup-1 rustic init"
    );
  }

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
      std::fs::create_dir_all(&pgdump_dir)?;
      log(&format!("pg_dump → {}", dump.display()));
      let tmp = pgdump_dir.join("mica.sql.tmp");
      // Dump to a temp file and rename, so an interrupted run never leaves a
      // half-written dump for the next snapshot to pick up.
      let out = std::fs::File::create(&tmp)?;
      let status = Proc::new("pg_dump")
        .args(["--no-owner", "--no-privileges", url])
        .stdout(out)
        .status()
        .context("could not start `pg_dump` — is postgresql-client installed?")?;
      if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("pg_dump failed ({status}) — refusing to ship a truncated dump");
      }
      std::fs::rename(&tmp, &dump)?;
      // Drop any stale gzip from before the switch to plain SQL, or the snapshot
      // below carries both and the restore runbook has two files to choose from.
      let _ = std::fs::remove_file(pgdump_dir.join("mica.sql.gz"));
      report.add("database", Leg::Ran);
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
  match &content_failure {
    Some(reason) => {
      log("skipping the content snapshot — this run wrote no manifest");
      report.add("content", Leg::failed(reason.clone()));
    }
    None => {
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
        log(&format!("MISSING {}: {} not written", ws.id, path.display()));
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
    report.add("content", coverage_leg(manifest.workspaces.len(), &missed));
    }
  }

  // 3b) The dump gets its own lineage (stable label `_pgdump`, never a workspace
  //     id) so retention below applies to it on the same policy.
  if dump.is_file() {
    log("snapshot pg_dump → label=_pgdump");
    run_tool(
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
    )?;
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
    run_tool(
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
    )?;
    report.add("config", Leg::Ran);
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
          "--transfers",
          "4",
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
  run_tool(
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
  )?;

  // Every leg got its chance; now the run owes an honest verdict. Partial
  // success is NOT success — that was the original shell sin (a missing leg
  // logged a WARN and exited 0, and production went months with no off-site
  // copy). What changed here is only the ORDER: report first, fail after, so
  // one broken leg can no longer take the healthy ones down with it.
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
  if client.get(format!("{base}{suffix}")).send().is_err() {
    eprintln!(
      "[{}] mica-backup: healthcheck ping failed (non-fatal)",
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
  fn every_rclone_call_carries_the_config_path() {
    let args = rclone_args(&["copy", "a:x", "b:y"]);
    assert_eq!(&args[..2], &["--config", RCLONE_CONF]);
    assert_eq!(&args[2..], &["copy", "a:x", "b:y"]);
    // and it survives an empty tail
    assert_eq!(rclone_args(&[]), vec!["--config", RCLONE_CONF]);
  }

  #[test]
  fn rclone_size_json_is_read_correctly() {
    assert_eq!(
      parse_rclone_size(r#"{"count":6467,"bytes":1042495633,"sizeless":0}"#),
      Some((6467, 1042495633))
    );
    // pretty-printed, which rclone emits on a terminal
    assert_eq!(
      parse_rclone_size("{
	\"count\": 12,
	\"bytes\": 34
}"),
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
  #[test]
  fn a_second_run_is_refused_while_the_first_holds_the_lock() {
    // 2026-08-29:手动 `backup run` 撞上守护进程的 run-on-start,两个进程都在写
    // `_pgdump/mica.sql.tmp` 再 rename,其中一个 rename 时文件已被抢走,报出
    // `No such file or directory` —— 一条既不指向真正原因、也不提另一个 run
    // 的错。演练与事故处理恰恰最容易制造这种并发。
    let _first = match RunLock::acquire() {
      Ok(lock) => lock,
      // 这台机器上没有 /tmp(非 Linux 开发机)就跳过 —— 断言不了就不假装断言。
      Err(_) => return,
    };
    let second = RunLock::acquire();
    assert!(second.is_err(), "第二个 run 必须被拒绝");
    let msg = format!("{:#}", second.unwrap_err());
    assert!(
      msg.contains("another backup run is in progress"),
      "报错要说出真正的原因,而不是让下一个人去猜 rename 为什么失败: {msg}"
    );
  }

  #[test]
  fn the_lock_is_released_when_the_run_ends() {
    // 否则一次失败的 run 会把之后所有 run 一起锁死 —— 比它要修的并发问题更糟。
    match RunLock::acquire() {
      Ok(lock) => drop(lock),
      Err(_) => return,
    }
    assert!(
      RunLock::acquire().is_ok(),
      "锁必须在 run 结束时释放,包括失败退出的那些"
    );
  }

  #[test]
  fn a_dead_owners_lock_is_taken_not_respected() {
    // 被 SIGKILL 的 run 留下的锁文件,不该让备份从此停摆。
    assert!(lock_is_live(true), "活着的持有者:让路");
    assert!(!lock_is_live(false), "持有者已死:接管");
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
    let now = chrono::Local.with_ymd_and_hms(2026, 8, 29, 5, 30, 0).unwrap();
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
    assert!(summary.contains("content: FAILED — operation timed out"), "{summary}");
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
    let leg = coverage_leg(
      3,
      &["a (ws=one)".to_string(), "b (ws=two)".to_string()],
    );
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
    let now = chrono::Local.with_ymd_and_hms(2026, 7, 31, 5, 30, 0).unwrap();
    // 03:00 already passed today → tomorrow's slot, 21h30m away.
    assert_eq!(seconds_until(now, 3, 0), 21 * 3600 + 30 * 60);
    // 09:00 is still ahead → 3h30m away.
    assert_eq!(seconds_until(now, 9, 0), 3 * 3600 + 30 * 60);
  }
}
