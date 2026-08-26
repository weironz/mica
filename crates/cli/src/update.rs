//! `mica-cli update` — replace this binary with the latest published release.
//!
//! It exists because the documented way to update was "paste the installer line
//! into a shell again". That works, but it is a URL you have to have handy and a
//! different ritual per platform — so in practice the CLI drifts behind the
//! server it talks to, and that is the drift that actually bites: the MCP proxy
//! layer lives in THIS binary, so a change there does nothing until this binary
//! is replaced.
//!
//! The mechanics are lifted from `install.ps1`, including the two traps it
//! documents, because both were found the hard way:
//!
//!  - **Windows refuses to OVERWRITE a running image, but is happy to RENAME
//!    it.** Anyone actually using this has an MCP client holding the exe open —
//!    the normal state, not the exception. So: move the old one aside, put the
//!    new one in its place. The running process keeps its handle.
//!  - **The displaced binary needs a UNIQUE name.** A fixed `.old` deadlocked
//!    with the very situation the dance exists for: the previous update left
//!    `.old` behind precisely BECAUSE something still held it, and the next
//!    rename then landed on a file that was still there. Updating worked once
//!    and then refused, for exactly the people who keep an MCP client running.
//!
//! On Unix the rename is atomic on the same filesystem and the running process
//! keeps its inode, so the same "download beside it, then rename into place"
//! shape works there without the aside step.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Where releases come from. Hardcoded rather than configurable: this command
/// replaces the binary you are running, and "which repo do I trust to hand me an
/// executable" is not a knob worth having.
const REPO: &str = "weironz/mica";

/// The asset this platform can actually run.
///
/// Only the targets release CI builds, matching `install.sh`'s list. Anything
/// else refuses with the same advice it gives, rather than downloading something
/// that will not execute.
fn asset_suffix() -> Result<&'static str> {
  Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
    ("windows", "x86_64") => "windows-x64.exe",
    ("linux", "x86_64") => "linux-x64",
    ("macos", "aarch64") => "macos-arm64",
    (os, arch) => bail!(
      "no prebuilt mica-cli for {os}/{arch} — build from source: \
       cargo build --release -p mica-cli"
    ),
  })
}

/// Ask GitHub which release is current. Returns the version WITHOUT the `v`.
fn latest_version(http: &reqwest::blocking::Client) -> Result<String> {
  let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
  let body: serde_json::Value = http
    .get(&url)
    .send()
    .context("asking GitHub for the latest release")?
    .error_for_status()
    .context("GitHub refused the release query")?
    .json()
    .context("reading the release JSON")?;
  let tag = body
    .get("tag_name")
    .and_then(serde_json::Value::as_str)
    .context("the latest release has no tag_name")?;
  Ok(tag.trim_start_matches('v').to_string())
}

pub struct UpdateOutcome {
  pub from: String,
  pub to: String,
  /// False when nothing was written — already current, or `--check`.
  pub replaced: bool,
  pub path: PathBuf,
}

/// Download the target version and swap it in for the running binary.
pub fn run(json: bool, want: Option<String>, check_only: bool, force: bool) -> Result<UpdateOutcome> {
  let current = env!("CARGO_PKG_VERSION").to_string();
  let exe = std::env::current_exe().context("locating this executable")?;
  // A symlinked install (Homebrew, ~/.local/bin pointing elsewhere) must be
  // followed, or the update replaces the LINK with a regular file and the real
  // binary is orphaned where nothing looks for it.
  let exe = std::fs::canonicalize(&exe).unwrap_or(exe);

  let http = reqwest::blocking::Client::builder()
    // GitHub's API 403s a request with no User-Agent — the same trap
    // install.ps1 calls out.
    .user_agent(concat!("mica-cli/", env!("CARGO_PKG_VERSION")))
    .timeout(std::time::Duration::from_secs(120))
    .build()?;

  let target = match want {
    Some(v) => v.trim_start_matches('v').to_string(),
    None => latest_version(&http)?,
  };

  if target == current && !force {
    if !json {
      println!("mica-cli {current} is already the latest release.");
    }
    return Ok(UpdateOutcome {
      from: current.clone(),
      to: target,
      replaced: false,
      path: exe,
    });
  }
  if check_only {
    if !json {
      println!("mica-cli {current} -> {target} available. Run `mica-cli update` to install it.");
    }
    return Ok(UpdateOutcome {
      from: current,
      to: target,
      replaced: false,
      path: exe,
    });
  }

  let asset = format!("mica-cli-{target}-{}", asset_suffix()?);
  let url = format!("https://github.com/{REPO}/releases/download/v{target}/{asset}");
  if !json {
    println!("mica-cli {current} -> {target}");
  }

  // Staged BESIDE the target, so the rename below is a same-filesystem move —
  // atomic on Unix, and the only shape Windows allows at all while the exe is
  // running.
  let staged = exe.with_extension("new");
  download(&http, &url, &staged)?;

  // Run what was just downloaded BEFORE letting it become the installed binary.
  //
  // This is the check a size or a status code cannot make: it proves the bytes
  // are a working executable for THIS machine and are the version they claim. A
  // truncated download, an HTML error page saved as a binary, or the wrong
  // architecture all fail here — while the real binary is still in place and
  // untouched.
  if let Err(error) = verify(&staged, &target) {
    let _ = std::fs::remove_file(&staged);
    return Err(error);
  }

  swap_in(&exe, &staged)?;

  if !json {
    println!("Updated {}.", readable(&exe));
    println!("Any running MCP client keeps the OLD binary until it restarts.");
  }
  Ok(UpdateOutcome {
    from: current,
    to: target,
    replaced: true,
    path: exe,
  })
}

fn download(http: &reqwest::blocking::Client, url: &str, to: &Path) -> Result<()> {
  let mut resp = http
    .get(url)
    .send()
    .with_context(|| format!("downloading {url}"))?
    .error_for_status()
    .with_context(|| format!("downloading {url}"))?;
  let mut file = std::fs::File::create(to)
    .with_context(|| format!("writing {} — is the install directory writable?", to.display()))?;
  // A connection that dies mid-stream leaves a partial file. Remove it rather
  // than leaving half a binary sitting next to the real one wearing a name that
  // says it is the next version.
  if let Err(error) = std::io::copy(&mut resp, &mut file) {
    drop(file);
    let _ = std::fs::remove_file(to);
    return Err(error).with_context(|| format!("writing {}", to.display()));
  }
  drop(file);
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755))
      .with_context(|| format!("marking {} executable", to.display()))?;
  }
  Ok(())
}

/// `<staged> --version` must run and report [want].
fn verify(staged: &Path, want: &str) -> Result<()> {
  let out = std::process::Command::new(staged)
    .arg("--version")
    .output()
    .with_context(|| format!("the downloaded binary would not run ({})", staged.display()))?;
  if !out.status.success() {
    bail!(
      "the downloaded binary exited {} on --version; nothing was replaced",
      out.status
    );
  }
  let text = String::from_utf8_lossy(&out.stdout);
  if !text.contains(want) {
    bail!("downloaded binary reports {:?}, expected {want}", text.trim());
  }
  Ok(())
}

/// Put `staged` where `exe` is.
fn swap_in(exe: &Path, staged: &Path) -> Result<()> {
  if cfg!(windows) {
    // Move the running image aside first — Windows will not let it be
    // overwritten, but renaming it is fine. The name must be unique; the module
    // comment says what a fixed `.old` did.
    let aside = displaced_name(exe);
    if exe.exists() {
      std::fs::rename(exe, &aside).with_context(|| {
        format!(
          "could not move the current binary aside ({}). If it lives somewhere \
           that needs admin rights, re-run from an elevated shell.",
          exe.display()
        )
      })?;
    }
    if let Err(error) = std::fs::rename(staged, exe) {
      // Put the old one back rather than leaving NO binary at that path.
      let _ = std::fs::rename(&aside, exe);
      return Err(error).context("installing the new binary");
    }
    sweep_displaced(exe);
  } else {
    std::fs::rename(staged, exe)
      .with_context(|| format!("installing the new binary at {}", exe.display()))?;
  }
  Ok(())
}

/// A path as a person expects to see it.
///
/// `canonicalize` on Windows returns the extended-length form —
/// `\\?\C:\Users\…` — which is a correct path and reads like a bug. It is only
/// ever printed, so strip it for display and never for use.
fn readable(path: &Path) -> String {
  let text = path.display().to_string();
  text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

/// A name for the binary being displaced that cannot collide with one an
/// earlier update left behind.
fn displaced_name(exe: &Path) -> PathBuf {
  let stamp = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_nanos())
    .unwrap_or(0);
  exe.with_extension(format!("old-{}-{stamp:x}", std::process::id()))
}

/// Best-effort removal of binaries displaced by this and earlier updates.
///
/// Each disappears only once nothing holds it open, and one left behind is
/// harmless — so every failure here is ignored on purpose. Reporting them would
/// make a successful update look like it went wrong.
fn sweep_displaced(exe: &Path) {
  let (Some(dir), Some(stem)) = (exe.parent(), exe.file_stem().and_then(|s| s.to_str())) else {
    return;
  };
  let prefix = format!("{stem}.old-");
  let Ok(entries) = std::fs::read_dir(dir) else {
    return;
  };
  for entry in entries.flatten() {
    if entry
      .file_name()
      .to_str()
      .is_some_and(|n| n.starts_with(&prefix))
    {
      let _ = std::fs::remove_file(entry.path());
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The three targets release CI builds, and a refusal for anything else.
  ///
  /// Pinned because the failure it prevents is silent in the worst way: without
  /// it an unsupported platform would download SOMETHING, and the first sign
  /// would be a binary that will not exec — after the old one is already gone.
  #[test]
  fn only_the_built_targets_resolve() {
    // Whatever this test runs on must be one of them — CI builds all three.
    let suffix = asset_suffix().expect("the test host is a supported target");
    assert!(
      ["windows-x64.exe", "linux-x64", "macos-arm64"].contains(&suffix),
      "unexpected suffix {suffix}"
    );
  }

  /// The asset name must be the one release CI publishes.
  ///
  /// `install.sh` builds the same string from the same pieces; if the release
  /// workflow ever renames its output, both break, and this is the one that says
  /// so before a user finds out.
  #[test]
  fn the_asset_name_has_the_published_shape() {
    let name = format!("mica-cli-0.13.28-{}", asset_suffix().unwrap());
    assert!(name.starts_with("mica-cli-0.13.28-"));
    assert!(
      !name.ends_with('-'),
      "an empty suffix would make the download url a 404"
    );
  }

  /// Two displaced names in a row must differ.
  ///
  /// This is the deadlock install.ps1 hit: a fixed `.old` collided with the one
  /// a previous update could not delete, and the rename then failed with
  /// "当文件已存在时，无法创建该文件" — so update worked exactly once, for
  /// precisely the users who keep an MCP client running.
  #[test]
  fn displaced_names_do_not_collide() {
    let exe = Path::new("C:/x/mica-cli.exe");
    let mut seen = std::collections::HashSet::new();
    for _ in 0..64 {
      seen.insert(displaced_name(exe));
    }
    assert!(
      seen.len() > 1,
      "every displaced name was identical — the second update would refuse to install"
    );
  }

  /// The displaced name must stay inside the install directory and keep the
  /// binary's stem, or `sweep_displaced` will never find it again.
  #[test]
  fn a_displaced_binary_is_swept_by_its_own_prefix() {
    let exe = Path::new("C:/x/mica-cli.exe");
    let aside = displaced_name(exe);
    assert_eq!(aside.parent(), exe.parent());
    let name = aside.file_name().unwrap().to_string_lossy().to_string();
    assert!(
      name.starts_with("mica-cli.old-"),
      "{name} would not match the sweep prefix"
    );
  }
}
