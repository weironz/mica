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
//!
//! Before any of that, the download is held to three checks (issue #38): the
//! bytes must match the release's `sha256-<os>.txt` manifest, the binary must
//! run and report exactly the version it claims, and the version must not be
//! OLDER than the one running unless `--force` says so. A self-update that
//! silently swaps in a tampered, broken, or older binary is worse than no
//! update at all.

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where releases come from. Hardcoded rather than configurable: this command
/// replaces the binary you are running, and "which repo do I trust to hand me an
/// executable" is not a knob worth having.
const REPO: &str = "weironz/mica";

/// Budget for the small requests — the release JSON and the checksum manifest,
/// a few KB each. Anything this slow is a stuck connection, not a slow one.
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);

/// Whole-request budget for the binary download. This used to be a client-wide
/// 120s that covered the download BODY too, which aborted slow-but-healthy
/// transfers of an ~8 MB binary — the tight cap belongs to the metadata, and
/// the payload gets a generous one of its own.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

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
    .timeout(METADATA_TIMEOUT)
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

/// Parse a version the way updates are JUDGED: as a `semver::Version`, never as
/// text. Textual handling is where `contains("0.13.5")` used to accept
/// `0.13.55`, and where a "latest" older than the running build looked
/// installable.
fn parse_version(raw: &str) -> Result<semver::Version> {
  raw
    .trim()
    .trim_start_matches('v')
    .parse()
    .with_context(|| format!("{raw:?} is not a release version (want X.Y.Z)"))
}

/// The first version-shaped word in `--version` output (`mica-cli 0.13.64`).
fn version_in(text: &str) -> Option<semver::Version> {
  text
    .split_whitespace()
    .find_map(|word| word.trim_start_matches('v').parse().ok())
}

pub struct UpdateOutcome {
  pub from: String,
  pub to: String,
  /// False when nothing was written — already current, or `--check`.
  pub replaced: bool,
  pub path: PathBuf,
}

/// Download the target version and swap it in for the running binary.
pub fn run(
  json: bool,
  want: Option<String>,
  check_only: bool,
  force: bool,
) -> Result<UpdateOutcome> {
  let current = parse_version(env!("CARGO_PKG_VERSION"))?;
  let exe = std::env::current_exe().context("locating this executable")?;
  // A symlinked install (Homebrew, ~/.local/bin pointing elsewhere) must be
  // followed, or the update replaces the LINK with a regular file and the real
  // binary is orphaned where nothing looks for it.
  let exe = std::fs::canonicalize(&exe).unwrap_or(exe);

  let http = reqwest::blocking::Client::builder()
    // GitHub's API 403s a request with no User-Agent — the same trap
    // install.ps1 calls out.
    .user_agent(concat!("mica-cli/", env!("CARGO_PKG_VERSION")))
    // Connection-level only: a client-wide timeout also covers the download
    // body, which is how 120s used to cut off transfers that were merely slow
    // (see DOWNLOAD_TIMEOUT for the other half of this split).
    .connect_timeout(METADATA_TIMEOUT)
    .build()?;

  let target = match want {
    Some(v) => parse_version(&v)?,
    None => parse_version(&latest_version(&http)?)?,
  };

  if target == current && !force {
    if !json {
      println!("mica-cli {current} is already the latest release.");
    }
    return Ok(UpdateOutcome {
      from: current.to_string(),
      to: target.to_string(),
      replaced: false,
      path: exe,
    });
  }
  // A downgrade must be a decision someone made with `--force`, never a side
  // effect of "latest" lagging behind the running build — a dev build, or a
  // rolled-back release. The old code replaced the newer binary with the older
  // one without a word.
  if target < current && !force {
    bail!(
      "mica-cli {current} is NEWER than {target} — refusing to downgrade. \
       Pass --force to install {target} anyway."
    );
  }
  if check_only {
    if !json {
      println!("mica-cli {current} -> {target} available. Run `mica-cli update` to install it.");
    }
    return Ok(UpdateOutcome {
      from: current.to_string(),
      to: target.to_string(),
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

  // Checksum BEFORE anything executes the bytes: a tampered or truncated
  // download dies here, while the real binary is still in place and untouched.
  // The manifest is published by the same chain that builds the binary; a
  // release without one (or without our asset's line) is an error, not a skip —
  // "no checksum available" must never quietly mean "no verification".
  if let Err(error) = verify_checksum(&http, &target, &asset, &staged) {
    let _ = std::fs::remove_file(&staged);
    return Err(error);
  }

  // Run what was just downloaded BEFORE letting it become the installed binary.
  //
  // This is the check a checksum cannot make: it proves the bytes are a working
  // executable for THIS machine and are the version they claim. The wrong
  // architecture still fails here — while the real binary is still in place and
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
    from: current.to_string(),
    to: target.to_string(),
    replaced: true,
    path: exe,
  })
}

fn download(http: &reqwest::blocking::Client, url: &str, to: &Path) -> Result<()> {
  let mut resp = http
    .get(url)
    .timeout(DOWNLOAD_TIMEOUT)
    .send()
    .with_context(|| format!("downloading {url}"))?
    .error_for_status()
    .with_context(|| format!("downloading {url}"))?;
  let mut file = std::fs::File::create(to).with_context(|| {
    format!(
      "writing {} — is the install directory writable?",
      to.display()
    )
  })?;
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

/// Verify the staged download against the release's `sha256-<os>.txt`.
///
/// The manifest is what the release chain publishes alongside every asset
/// (`scripts/release-artifacts.sh` locally, the release workflow in CI):
/// `sha256sum` output, one `<digest>  <name>` line per file.
fn verify_checksum(
  http: &reqwest::blocking::Client,
  target: &semver::Version,
  asset: &str,
  staged: &Path,
) -> Result<()> {
  let url = format!(
    "https://github.com/{REPO}/releases/download/v{target}/sha256-{}.txt",
    std::env::consts::OS
  );
  let manifest = http
    .get(&url)
    .timeout(METADATA_TIMEOUT)
    .send()
    .with_context(|| format!("fetching {url}"))?
    .error_for_status()
    .with_context(|| {
      format!(
        "{url} is missing — this release carries no checksums, so its binaries cannot be verified"
      )
    })?
    .text()
    .context("reading the checksum manifest")?;
  let expected = checksum_for(&manifest, asset).with_context(|| {
    format!("no checksum for {asset} in {url} — refusing to install unverified bytes")
  })?;
  let actual = sha256_of(staged)?;
  if !expected.eq_ignore_ascii_case(&actual) {
    bail!(
      "checksum mismatch for {asset}: the release says {expected}, the \
       download says {actual} — nothing was replaced"
    );
  }
  Ok(())
}

/// The digest `sha256sum` writes for `asset`, out of a manifest.
///
/// Matches the WHOLE file name, never a substring — `contains`-style matching
/// would hand back the linux digest for the windows asset.
fn checksum_for(manifest: &str, asset: &str) -> Option<String> {
  for line in manifest.lines() {
    let mut fields = line.split_whitespace();
    let (Some(digest), Some(name)) = (fields.next(), fields.next()) else {
      continue;
    };
    // `sha256sum` marks binary-mode lines `*name`; both spellings must match.
    if name.strip_prefix('*').unwrap_or(name) == asset {
      return Some(digest.to_ascii_lowercase());
    }
  }
  None
}

/// The sha256 of a file on disk, hex, lower-case — streamed, because the
/// binary it hashes is several MB.
fn sha256_of(path: &Path) -> Result<String> {
  let mut file = std::fs::File::open(path)
    .with_context(|| format!("reading {} back for its checksum", path.display()))?;
  let mut hasher = Sha256::new();
  let mut buffer = [0u8; 64 * 1024];
  loop {
    let read = file
      .read(&mut buffer)
      .with_context(|| format!("reading {}", path.display()))?;
    if read == 0 {
      break;
    }
    hasher.update(&buffer[..read]);
  }
  Ok(format!("{:x}", hasher.finalize()))
}

/// `<staged> --version` must run and report EXACTLY [want].
///
/// This used to be `text.contains(want)`, which accepted a binary reporting
/// `0.13.55` when `0.13.5` was wanted — a prefix is not a version. The output
/// is parsed and compared as a whole version instead.
fn verify(staged: &Path, want: &semver::Version) -> Result<()> {
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
  let reported = version_in(&text).with_context(|| {
    format!(
      "the downloaded binary printed no version ({:?})",
      text.trim()
    )
  })?;
  if reported != *want {
    bail!("downloaded binary reports {reported}, expected {want}");
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

  /// The three checks issue #38 asked for, pinned at the pure-function level:
  /// semver ordering (not textual), exact version match (not `contains`),
  /// and whole-name checksum lookup (not substring).

  /// Ordering must be semver, not textual: "0.13.10" sorts after "0.13.9" as
  /// versions, and "0.13.55" after "0.13.5" — the exact pair the old string
  /// compare could not tell apart.
  #[test]
  fn versions_order_by_semver_not_text() {
    assert!(parse_version("0.13.10").unwrap() > parse_version("0.13.9").unwrap());
    assert!(parse_version("0.13.55").unwrap() > parse_version("0.13.5").unwrap());
    assert!(parse_version("0.14.0").unwrap() > parse_version("0.13.99").unwrap());
    // A release is NEWER than its own pre-release, the way a downgrade guard
    // has to see it.
    assert!(parse_version("0.14.0").unwrap() > parse_version("0.14.0-rc.1").unwrap());
  }

  /// The old `--version` check was `text.contains(want)`, so a binary
  /// reporting 0.13.55 passed a want of 0.13.5.
  #[test]
  fn a_version_prefix_is_not_that_version() {
    let want = parse_version("0.13.5").unwrap();
    assert_eq!(version_in("mica-cli 0.13.5\n").unwrap(), want);
    assert_ne!(version_in("mica-cli 0.13.55\n").unwrap(), want);
  }

  #[test]
  fn version_in_reads_the_announced_version_and_rejects_the_rest() {
    assert_eq!(
      version_in("mica-cli 0.13.64\n").unwrap().to_string(),
      "0.13.64"
    );
    assert_eq!(version_in("mica-cli v1.2.3").unwrap().to_string(), "1.2.3");
    assert_eq!(
      version_in("0.14.0-rc.1\n").unwrap().to_string(),
      "0.14.0-rc.1"
    );
    assert!(
      version_in("no version in here").is_none(),
      "no word parses as a version"
    );
  }

  /// `--to` accepts (and `latest_version` returns) strings; anything that is
  /// not X.Y.Z must be refused before a URL is built from it.
  #[test]
  fn parse_version_strips_the_v_and_refuses_garbage() {
    assert_eq!(parse_version("v0.13.5").unwrap().to_string(), "0.13.5");
    assert_eq!(parse_version(" 0.13.5 ").unwrap().to_string(), "0.13.5");
    assert!(parse_version("0.13").is_err(), "two components");
    assert!(parse_version("latest").is_err(), "a word is not a version");
    assert!(parse_version("").is_err(), "empty");
  }

  /// The manifest lookup must match the whole asset name: `contains`-style
  /// matching would hand back the linux digest for the windows asset, and a
  /// digest for a version you did not ask for.
  #[test]
  fn checksum_for_matches_the_whole_asset_name() {
    let manifest = "1111111111111111111111111111111111111111111111111111111111111111  Mica-Setup-0.13.64.exe\n\
                    2222222222222222222222222222222222222222222222222222222222222222  mica-cli-0.13.64-windows-x64.exe\n\
                    3333333333333333333333333333333333333333333333333333333333333333  mica-cli-0.13.64-linux-x64\n";
    assert_eq!(
      checksum_for(manifest, "mica-cli-0.13.64-windows-x64.exe").unwrap(),
      "2222222222222222222222222222222222222222222222222222222222222222"
    );
    assert_eq!(
      checksum_for(manifest, "mica-cli-0.13.64-linux-x64").unwrap(),
      "3333333333333333333333333333333333333333333333333333333333333333"
    );
    assert!(
      checksum_for(manifest, "mica-cli-0.13.65-windows-x64.exe").is_none(),
      "a version that is not in the manifest has no checksum"
    );
    assert!(checksum_for("", "anything").is_none());
    // `sha256sum` binary mode writes `*name`; both spellings must match.
    assert_eq!(
      checksum_for(
        "4444444444444444444444444444444444444444444444444444444444444444 *asset.bin",
        "asset.bin"
      )
      .unwrap(),
      "4444444444444444444444444444444444444444444444444444444444444444"
    );
    // Uppercase digests come back normalized, so the comparison cannot fail on
    // spelling alone.
    assert_eq!(
      checksum_for(
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA  asset.bin",
        "asset.bin"
      )
      .unwrap(),
      "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
  }

  /// The staged file is hashed from disk, streamed — the digest must be the
  /// contents', not the path's.
  #[test]
  fn sha256_of_hashes_the_file_contents() {
    let path = std::env::temp_dir().join(format!(
      "mica-cli-sha-test-{}-{:x}.bin",
      std::process::id(),
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
    ));
    std::fs::write(&path, b"hello\n").unwrap();
    // sha256("hello\n"), known independently of this code.
    assert_eq!(
      sha256_of(&path).unwrap(),
      "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
    );
    let _ = std::fs::remove_file(&path);
  }
}
