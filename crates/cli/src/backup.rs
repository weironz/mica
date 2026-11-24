//! Scheduled Markdown export backup, in Rust.
//!
//! # Scope (mechanism vs policy)
//!
//! PostgreSQL and RustFS off-site backups are handled by external dedicated
//! services (Databasus and Zerobyte). This module's only job is to keep a fresh,
//! verified Markdown + images mirror on disk (`MICA_EXPORT_DIR`) so Zerobyte can
//! incrementally relay it to Aliyun OSS (`mica-backup/mica-md`):
//!
//! - Exclusive OS lock (`/tmp/mica-backup.lock`) so overlapping runs never
//!   corrupt the mirror directory.
//! - Hard wall-clock deadline on the child `mica-cli export` process (killing
//!   its Unix process group on timeout so no orphan continues writing).
//! - Post-export manifest verification so a run only succeeds when every
//!   workspace announced in `manifest.json` actually landed on disk.
//! - Daily wall-clock scheduling (`BACKUP_HOUR`) plus best-effort dead man's
//!   switch (`HEALTHCHECK_URL` / `<URL>/fail`).

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(any(windows, test))]
use std::process::Stdio;
use std::process::{Command as Proc, ExitStatus};
use std::time::{Duration, Instant};

/// Everything a backup export run reads from the environment.
pub struct Settings {
  pub export_dir: PathBuf,
}

/// Read an env var, treating empty/whitespace as absent.
fn env(key: &str) -> Option<String> {
  match std::env::var(key) {
    Ok(v) if !v.trim().is_empty() => Some(v),
    _ => None,
  }
}

impl Settings {
  pub fn from_env() -> Result<Self> {
    Ok(Settings {
      export_dir: env("MICA_EXPORT_DIR")
        .unwrap_or_else(|| "/var/lib/mica/export".into())
        .into(),
    })
  }
}

pub fn log(msg: &str) {
  println!("[{}] mica-backup: {msg}", chrono::Local::now().to_rfc3339());
}

/// Spawn each Unix command in its own process group: killing only a parent PID
/// leaves descendants running after the deadline.
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
        // Reap even after kill, or the daemon accumulates zombies.
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

/// Summary of one completed export run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
  pub workspaces: usize,
}

/// Verify that `export_dir/manifest.json` is valid and every workspace listed
/// in it has its mirrored directory on disk.
pub fn verify_export_manifest(export_dir: &Path) -> Result<ExportSummary> {
  let manifest_path = export_dir.join("manifest.json");
  let raw = std::fs::read_to_string(&manifest_path)
    .with_context(|| format!("reading {}", manifest_path.display()))?;
  let manifest: Manifest =
    serde_json::from_str(&raw).with_context(|| format!("parsing {}", manifest_path.display()))?;

  let mut missing = Vec::new();
  for ws in &manifest.workspaces {
    let dir = export_dir.join(&ws.dir);
    if !dir.is_dir() {
      missing.push(format!("{} (ws={}, dir={})", ws.id, ws.name, ws.dir));
    }
  }
  if !missing.is_empty() {
    bail!(
      "{} of {} workspace(s) were listed in manifest.json but missing on disk: {}",
      missing.len(),
      manifest.workspaces.len(),
      missing.join(", ")
    );
  }
  Ok(ExportSummary {
    workspaces: manifest.workspaces.len(),
  })
}

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

/// Run one Markdown export pass under the exclusive OS lock and verify its
/// resulting `manifest.json` coverage.
pub fn run_once(
  settings: &Settings,
  export: impl FnOnce(&Path) -> Result<()>,
) -> Result<ExportSummary> {
  let _lock = RunLock::acquire()?;
  log(&format!(
    "export all workspaces → {}",
    settings.export_dir.display()
  ));
  export(&settings.export_dir)?;
  let summary = verify_export_manifest(&settings.export_dir)?;
  log(&format!(
    "verified {} workspace(s) in {}",
    summary.workspaces,
    settings.export_dir.display()
  ));
  Ok(summary)
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

/// `BACKUP_HOUR` as a wall-clock time: `3`, `03`, `3:00`, `03:30`.
///
/// Returns `None` for anything it cannot read so the caller announces the
/// fallback instead of silently substituting `03:00`.
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

/// Seconds until the next `hour:minute` local time. Always in the future.
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

  fn test_path() -> PathBuf {
    std::env::temp_dir().join(format!("mica-backup-test-{}", uuid::Uuid::new_v4()))
  }

  fn helper_command(mode: &str) -> Proc {
    let mut cmd = Proc::new(std::env::current_exe().unwrap());
    cmd
      .args(["--exact", "backup::tests::subprocess_helper", "--ignored"])
      .env("MICA_BACKUP_TEST_HELPER", mode);
    cmd
  }

  #[test]
  #[ignore = "helper entrypoint for subprocess timeout and lock tests"]
  fn subprocess_helper() {
    match std::env::var("MICA_BACKUP_TEST_HELPER").unwrap().as_str() {
      "sleep" => std::thread::sleep(Duration::from_secs(30)),
      "fail" => std::process::exit(2),
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
  fn verify_manifest_accepts_present_workspace_directories() {
    let root = test_path();
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::create_dir_all(root.join("ops")).unwrap();
    std::fs::write(
      root.join("manifest.json"),
      r#"{"workspaces":[{"id":"ws-1","name":"Notes","dir":"notes"},{"id":"ws-2","name":"Ops","dir":"ops"}]}"#,
    )
    .unwrap();
    let summary = verify_export_manifest(&root).unwrap();
    assert_eq!(summary, ExportSummary { workspaces: 2 });
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn verify_manifest_fails_when_a_listed_workspace_directory_is_missing() {
    let root = test_path();
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(
      root.join("manifest.json"),
      r#"{"workspaces":[{"id":"ws-1","name":"Notes","dir":"notes"},{"id":"ws-2","name":"IDC","dir":"idc"}]}"#,
    )
    .unwrap();
    let err = verify_export_manifest(&root).unwrap_err().to_string();
    assert!(err.contains("1 of 2"), "{err}");
    assert!(err.contains("ws=IDC"), "{err}");
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn verify_manifest_fails_when_manifest_is_absent() {
    let root = test_path();
    std::fs::create_dir_all(&root).unwrap();
    let err = verify_export_manifest(&root).unwrap_err().to_string();
    assert!(err.contains("manifest.json"), "{err}");
    std::fs::remove_dir_all(root).unwrap();
  }

  #[test]
  fn backup_hour_reads_both_a_bare_hour_and_a_wall_clock_time() {
    assert_eq!(parse_backup_at("3"), Some((3, 0)));
    assert_eq!(parse_backup_at("03"), Some((3, 0)));
    assert_eq!(parse_backup_at("3:00"), Some((3, 0)));
    assert_eq!(parse_backup_at("03:30"), Some((3, 30)));
    assert_eq!(parse_backup_at(" 23:59 "), Some((23, 59)));

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
    assert_eq!(seconds_until(now, 5, 45), 15 * 60);
    assert_eq!(seconds_until(now, 5, 15), 24 * 3600 - 15 * 60);
  }

  #[test]
  fn the_next_run_is_always_in_the_future() {
    use chrono::TimeZone;
    let now = chrono::Local
      .with_ymd_and_hms(2026, 7, 31, 5, 30, 0)
      .unwrap();
    assert_eq!(seconds_until(now, 3, 0), 21 * 3600 + 30 * 60);
    assert_eq!(seconds_until(now, 9, 0), 3 * 3600 + 30 * 60);
  }
}
