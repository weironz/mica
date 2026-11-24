//! Desktop in-app self-updater (GitHub Releases -> SHA-256 verification ->
//! silent Inno Setup installer launch with `/MICAWAITPID=<pid>`).
//!
//! Security & reliability invariants preserved from `updater_desktop.dart`:
//! - Verifies BOTH exact byte size AND lowercase SHA-256 hex before ever
//!   launching a downloaded `.exe`. Missing expected SHA-256 is a hard refusal,
//!   never a silent downgrade to size-only.
//! - Uses an RAII temp-dir guard so any network, disk, or integrity failure
//!   cleans up half-downloaded files automatically.
//! - Launches `Mica-Setup-*.exe` directly via argv (`Command::new`) without
//!   `cmd.exe` / `ping` console windows, passing `/MICAWAITPID=<pid>` so Inno
//!   Setup waits for our process to flush and exit cleanly before replacing files.

use std::cmp::Ordering;
use std::fmt;

/// GitHub repository publishing official Mica releases (`owner/name`).
pub const UPDATE_REPO: &str = "weironz/mica";

/// Metadata for a newer release that ships a Windows Inno Setup installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
  /// Latest version without leading `v` (e.g. `0.15.0`).
  pub version: String,
  /// Direct download URL of `Mica-Setup-*.exe`.
  pub download_url: String,
  /// Exact filename of the installer asset (used for checksum-file lookup).
  pub asset_name: String,
  /// Optional release notes (Markdown).
  pub notes: Option<String>,
  /// Expected exact file size from GitHub `assets[].size`.
  pub size: Option<u64>,
  /// Expected lowercase SHA-256 hex digest from GitHub `assets[].digest`
  /// (`sha256:...`) or `sha256-windows-installer.txt`.
  pub sha256: Option<String>,
  /// Optional fallback checksum manifest URL (`sha256-windows-installer.txt`).
  pub sha256_file_url: Option<String>,
}

/// Distinct failure categories so the UI never mislabels a dropped connection
/// as installer tampering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
  UnsupportedPlatform,
  Network(String),
  Integrity(String),
  Io(String),
}

impl fmt::Display for UpdateError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::UnsupportedPlatform => write!(f, "当前平台不支持应用内自动安装更新，请前往 GitHub Releases 手动下载"),
      Self::Network(m) => write!(f, "下载更新失败：{m}"),
      Self::Integrity(m) => write!(f, "安装包完整性校验失败（已自动删除，未执行）：{m}"),
      Self::Io(m) => write!(f, "安装程序启动失败：{m}"),
    }
  }
}

/// Whether this platform supports silent in-place Inno Setup updates.
pub const fn update_supported() -> bool {
  cfg!(target_os = "windows")
}

fn parse_version_parts(raw: &str) -> Vec<u64> {
  let trimmed = raw.trim().trim_start_matches(['v', 'V']);
  trimmed
    .split('.')
    .map(|seg| {
      let digits: String = seg.chars().take_while(|c| c.is_ascii_digit()).collect();
      digits.parse::<u64>().unwrap_or(0)
    })
    .collect()
}

/// Numerically compare two dotted versions (`0.14.11` vs `0.15.0`, tolerating
/// leading `v`, differing component counts, and suffixes like `1.2.0-rc1`).
pub fn compare_versions(a: &str, b: &str) -> Ordering {
  let pa = parse_version_parts(a);
  let pb = parse_version_parts(b);
  let n = pa.len().max(pb.len());
  for i in 0..n {
    let x = pa.get(i).copied().unwrap_or(0);
    let y = pb.get(i).copied().unwrap_or(0);
    if x != y {
      return x.cmp(&y);
    }
  }
  Ordering::Equal
}

/// Parse GitHub's `/releases/latest` JSON payload and return `Some(UpdateInfo)`
/// only if `tag_name` is strictly newer than `current_version` and includes a
/// `Mica-Setup-*.exe` asset.
pub fn parse_github_release_json(
  body: &str,
  current_version: &str,
) -> Result<Option<UpdateInfo>, String> {
  let val: serde_json::Value =
    serde_json::from_str(body).map_err(|e| format!("无法解析 GitHub Release 响应：{e}"))?;
  let tag = val
    .get("tag_name")
    .and_then(|v| v.as_str())
    .unwrap_or("")
    .trim();
  let latest = tag.trim_start_matches(['v', 'V']);
  if latest.is_empty() || compare_versions(latest, current_version) != Ordering::Greater {
    return Ok(None);
  }

  let assets = val
    .get("assets")
    .and_then(|v| v.as_array())
    .map(|v| v.as_slice())
    .unwrap_or(&[]);

  let mut setup_asset: Option<&serde_json::Value> = None;
  let mut sha256_file_url: Option<String> = None;

  for asset in assets {
    let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let lower = name.to_ascii_lowercase();
    if setup_asset.is_none() && lower.starts_with("mica-setup") && lower.ends_with(".exe") {
      setup_asset = Some(asset);
    } else if sha256_file_url.is_none()
      && (lower == "sha256-windows-installer.txt" || lower == "sha256sums.txt")
    {
      sha256_file_url = asset
        .get("browser_download_url")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    }
  }

  let Some(setup) = setup_asset else {
    return Ok(None);
  };
  let Some(download_url) = setup.get("browser_download_url").and_then(|v| v.as_str()) else {
    return Ok(None);
  };
  let asset_name = setup
    .get("name")
    .and_then(|v| v.as_str())
    .unwrap_or("Mica-Setup.exe")
    .to_string();
  let size = setup.get("size").and_then(|v| v.as_u64());
  let sha256 = setup
    .get("digest")
    .and_then(|v| v.as_str())
    .and_then(|d| d.strip_prefix("sha256:"))
    .map(|h| h.trim().to_ascii_lowercase())
    .filter(|h| is_valid_sha256_hex(h));
  let notes = val
    .get("body")
    .and_then(|v| v.as_str())
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());

  Ok(Some(UpdateInfo {
    version: latest.to_string(),
    download_url: download_url.to_string(),
    asset_name,
    notes,
    size,
    sha256,
    sha256_file_url,
  }))
}

/// Parse a `sha256sum`-style checksum file (`<64-hex>  <filename>`) for `asset_name`.
pub fn parse_sha256_manifest_for_asset(manifest_text: &str, asset_name: &str) -> Option<String> {
  let target = asset_name.to_ascii_lowercase();
  for line in manifest_text.lines() {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
      continue;
    }
    let mut parts = trimmed.split_whitespace();
    let hash = parts.next()?.to_ascii_lowercase();
    if !is_valid_sha256_hex(&hash) {
      continue;
    }
    let file_col = parts.next().map(|f| {
      f.trim_start_matches('*')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(f)
        .to_ascii_lowercase()
    });
    match file_col {
      Some(name) if name == target => return Some(hash),
      None if manifest_text.lines().filter(|l| !l.trim().is_empty()).count() == 1 => {
        return Some(hash);
      }
      _ => {}
    }
  }
  None
}

fn is_valid_sha256_hex(s: &str) -> bool {
  s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Build the argv passed directly to Inno Setup (`Mica-Setup-*.exe`).
/// Matches `updater_desktop.dart::setupArgs` and `installer/mica.iss` `[Code]`.
pub fn setup_args(log_path: &str, wait_pid: u32) -> Vec<String> {
  vec![
    "/VERYSILENT".to_string(),
    "/NOCANCEL".to_string(),
    "/CLOSEAPPLICATIONS".to_string(),
    "/NORESTART".to_string(),
    format!("/MICAWAITPID={wait_pid}"),
    format!("/LOG={log_path}"),
  ]
}

/// Strictly verify installer bytes against BOTH expected size and SHA-256 hex.
/// A missing expected SHA-256 is refused (never silently downgraded to size-only).
#[cfg(not(target_arch = "wasm32"))]
pub fn verify_installer_bytes(
  bytes: &[u8],
  expected_size: Option<u64>,
  expected_sha256: Option<&str>,
) -> Result<(), UpdateError> {
  use sha2::{Digest, Sha256};

  if bytes.is_empty() {
    return Err(UpdateError::Integrity("安装包为空".to_string()));
  }
  if let Some(size) = expected_size
    && bytes.len() as u64 != size
  {
    return Err(UpdateError::Integrity(format!(
      "文件大小不匹配（期望 {size} B，实际 {} B）",
      bytes.len()
    )));
  }
  let Some(expected) = expected_sha256.filter(|h| is_valid_sha256_hex(h)) else {
    return Err(UpdateError::Integrity(
      "发布元数据缺少 SHA-256 校验值，已拒绝执行未校验安装包".to_string(),
    ));
  };
  let digest = Sha256::digest(bytes);
  let actual: String = digest.iter().map(|b| format!("{b:02x}")).collect();
  if actual != expected.to_ascii_lowercase() {
    return Err(UpdateError::Integrity(format!(
      "SHA-256 不匹配（期望 {expected}，实际 {actual}）"
    )));
  }
  Ok(())
}

/// Query GitHub Releases for a newer Windows installer and resolve its SHA-256
/// digest (from `assets[].digest` or fallback `sha256-windows-installer.txt`).
#[cfg(not(target_arch = "wasm32"))]
pub fn check_for_update_blocking(current_version: &str) -> Result<Option<UpdateInfo>, String> {
  let agent = ureq::Agent::config_builder()
    .timeout_global(Some(std::time::Duration::from_secs(15)))
    .http_status_as_error(false)
    .build()
    .new_agent();
  let url = format!("https://api.github.com/repos/{UPDATE_REPO}/releases/latest");
  let mut resp = agent
    .get(&url)
    .header("Accept", "application/vnd.github+json")
    .header("User-Agent", &format!("Mica/{current_version}"))
    .call()
    .map_err(|e| format!("无法连接 GitHub 检查更新：{e}"))?;

  let status = resp.status().as_u16();
  let body = resp
    .body_mut()
    .read_to_string()
    .map_err(|e| format!("读取更新响应失败：{e}"))?;
  if status != 200 {
    return Err(format!("GitHub Releases 返回 HTTP {status}"));
  }

  let mut info = match parse_github_release_json(&body, current_version)? {
    Some(i) => i,
    None => return Ok(None),
  };

  if info.sha256.is_none()
    && let Some(ref manifest_url) = info.sha256_file_url
    && let Ok(mut mresp) = agent
      .get(manifest_url)
      .header("User-Agent", &format!("Mica/{current_version}"))
      .call()
    && mresp.status().as_u16() == 200
    && let Ok(manifest_text) = mresp.body_mut().read_to_string()
  {
    info.sha256 = parse_sha256_manifest_for_asset(&manifest_text, &info.asset_name);
  }

  Ok(Some(info))
}

#[cfg(not(target_arch = "wasm32"))]
struct TempDirGuard {
  path: std::path::PathBuf,
  keep: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for TempDirGuard {
  fn drop(&mut self) {
    if !self.keep {
      let _ = std::fs::remove_dir_all(&self.path);
    }
  }
}

/// Download the Windows installer to a fresh temp directory, verify exact size
/// and SHA-256, and launch it silently with `/MICAWAITPID=<wait_pid>`.
///
/// On ANY error the temp directory and partial installer are deleted automatically.
#[cfg(not(target_arch = "wasm32"))]
pub fn download_and_launch_installer_blocking(
  info: &UpdateInfo,
  wait_pid: u32,
  on_progress: impl Fn(f32),
) -> Result<std::path::PathBuf, UpdateError> {
  use std::io::{Read, Write};

  if !update_supported() {
    return Err(UpdateError::UnsupportedPlatform);
  }

  let dir_path = std::env::temp_dir().join(format!("mica_update_{}", uuid::Uuid::new_v4()));
  std::fs::create_dir_all(&dir_path).map_err(|e| UpdateError::Io(e.to_string()))?;
  let mut guard = TempDirGuard {
    path: dir_path.clone(),
    keep: false,
  };

  let setup_path = dir_path.join(format!("Mica-Setup-{}.exe", info.version));
  let log_path = dir_path.join("inno-install.log");

  let agent = ureq::Agent::config_builder()
    .timeout_recv_response(Some(std::time::Duration::from_secs(20)))
    .timeout_recv_body(Some(std::time::Duration::from_secs(300)))
    .http_status_as_error(false)
    .build()
    .new_agent();

  let mut resp = agent
    .get(&info.download_url)
    .header("User-Agent", &format!("Mica/{}", env!("CARGO_PKG_VERSION")))
    .call()
    .map_err(|e| UpdateError::Network(e.to_string()))?;

  let status = resp.status().as_u16();
  if status != 200 {
    return Err(UpdateError::Network(format!("HTTP {status}")));
  }

  let total = info.size.unwrap_or(0);
  let mut file =
    std::fs::File::create(&setup_path).map_err(|e| UpdateError::Io(e.to_string()))?;
  let mut reader = resp.body_mut().as_reader();
  let mut buf = [0u8; 64 * 1024];
  let mut bytes = Vec::with_capacity(total.min(64 * 1024 * 1024) as usize);

  loop {
    let n = reader
      .read(&mut buf)
      .map_err(|e| UpdateError::Network(e.to_string()))?;
    if n == 0 {
      break;
    }
    file
      .write_all(&buf[..n])
      .map_err(|e| UpdateError::Io(e.to_string()))?;
    bytes.extend_from_slice(&buf[..n]);
    if total > 0 {
      on_progress((bytes.len() as f32 / total as f32).clamp(0.0, 1.0));
    }
  }
  file.sync_all().map_err(|e| UpdateError::Io(e.to_string()))?;
  drop(file);

  // Strict pre-launch verification: reject truncated, tampered, or unsigned files.
  verify_installer_bytes(&bytes, info.size, info.sha256.as_deref())?;

  let args = setup_args(&log_path.to_string_lossy(), wait_pid);
  std::process::Command::new(&setup_path)
    .args(&args)
    .spawn()
    .map_err(|e| UpdateError::Io(e.to_string()))?;

  guard.keep = true;
  Ok(setup_path)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn compare_versions_handles_v_prefix_patch_minor_and_prerelease_suffix() {
    assert_eq!(compare_versions("0.15.0", "0.14.11"), Ordering::Greater);
    assert_eq!(compare_versions("v0.14.12", "0.14.11"), Ordering::Greater);
    assert_eq!(compare_versions("0.14.11", "v0.14.11"), Ordering::Equal);
    assert_eq!(compare_versions("0.14.10", "0.14.11"), Ordering::Less);
    assert_eq!(compare_versions("0.15", "0.14.99"), Ordering::Greater);
    assert_eq!(compare_versions("1.2.0-rc1", "1.2.0"), Ordering::Equal);
  }

  #[test]
  fn parse_github_release_selects_windows_installer_and_digest_or_checksum_fallback() {
    let digest = "a".repeat(64);
    let json = format!(
      r#"{{
        "tag_name": "v0.15.0",
        "body": "Fullstack Rust release",
        "assets": [
          {{
            "name": "mica-cli-linux.tar.gz",
            "size": 1000,
            "browser_download_url": "https://example.invalid/cli.tar.gz"
          }},
          {{
            "name": "Mica-Setup-0.15.0.exe",
            "size": 12345678,
            "digest": "sha256:{digest}",
            "browser_download_url": "https://example.invalid/Mica-Setup-0.15.0.exe"
          }},
          {{
            "name": "sha256-windows-installer.txt",
            "size": 90,
            "browser_download_url": "https://example.invalid/sha256-windows-installer.txt"
          }}
        ]
      }}"#
    );

    let info = parse_github_release_json(&json, "0.14.11")
      .unwrap()
      .expect("newer release with Windows installer");
    assert_eq!(info.version, "0.15.0");
    assert_eq!(info.asset_name, "Mica-Setup-0.15.0.exe");
    assert_eq!(info.size, Some(12345678));
    assert_eq!(info.sha256.as_deref(), Some(digest.as_str()));
    assert_eq!(
      info.sha256_file_url.as_deref(),
      Some("https://example.invalid/sha256-windows-installer.txt")
    );

    // Same or older version returns None.
    assert!(parse_github_release_json(&json, "0.15.0").unwrap().is_none());
    assert!(parse_github_release_json(&json, "0.16.0").unwrap().is_none());
  }

  #[test]
  fn parse_sha256_manifest_matches_exact_asset_name() {
    let h1 = "1".repeat(64);
    let h2 = "2".repeat(64);
    let manifest = format!("{h1}  other.zip\n{h2}  *dist/Mica-Setup-0.15.0.exe\n");
    assert_eq!(
      parse_sha256_manifest_for_asset(&manifest, "Mica-Setup-0.15.0.exe").as_deref(),
      Some(h2.as_str())
    );
    assert!(parse_sha256_manifest_for_asset(&manifest, "missing.exe").is_none());
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn verify_installer_bytes_enforces_exact_size_and_mandatory_sha256() {
    use sha2::{Digest, Sha256};

    let payload = b"mica-pure-rust-installer-payload";
    let expected: String = Sha256::digest(payload)
      .iter()
      .map(|b| format!("{b:02x}"))
      .collect();

    assert!(verify_installer_bytes(payload, Some(payload.len() as u64), Some(&expected)).is_ok());

    // Missing SHA-256 MUST be refused (no downgrade to size-only).
    assert!(matches!(
      verify_installer_bytes(payload, Some(payload.len() as u64), None),
      Err(UpdateError::Integrity(_))
    ));

    // Size mismatch rejected.
    assert!(matches!(
      verify_installer_bytes(payload, Some(999), Some(&expected)),
      Err(UpdateError::Integrity(_))
    ));

    // Tampered byte with identical size rejected.
    let mut tampered = payload.to_vec();
    tampered[0] ^= 0xff;
    assert!(matches!(
      verify_installer_bytes(&tampered, Some(payload.len() as u64), Some(&expected)),
      Err(UpdateError::Integrity(_))
    ));
  }

  #[test]
  fn setup_args_passes_silent_waitpid_and_log_without_shell() {
    let args = setup_args(r"C:\Temp\inno.log", 4242);
    assert_eq!(
      args,
      vec![
        "/VERYSILENT",
        "/NOCANCEL",
        "/CLOSEAPPLICATIONS",
        "/NORESTART",
        "/MICAWAITPID=4242",
        r"/LOG=C:\Temp\inno.log",
      ]
    );
  }
}
