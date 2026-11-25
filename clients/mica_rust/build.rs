//! Embeds Windows PE resources into the `mica-rust` executable:
//! - `assets/app_icon.ico` as Win32 resource ID `1` (`RT_GROUP_ICON` + `RT_ICON`)
//!   so `nsh-platform-windows`'s `LoadIconW(hinstance, MAKEINTRESOURCEW(1))` shows
//!   the crisp multi-size Mica icon on the taskbar and title bar.
//! - Standard `RT_VERSION` (`VS_VERSION_INFO`, ID `1`) populated from
//!   `CARGO_PKG_VERSION` so Windows File Properties and installer smoke/canary
//!   checks (`(Get-Item mica-rust.exe).VersionInfo.ProductVersion`) read the exact
//!   release version.
//!
//! Writing a standard 32-bit `.res` binary directly keeps the build
//! zero-dependency, offline-safe, and independent of `rc.exe` PATH discovery.

use std::env;
use std::fs;
use std::path::PathBuf;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const APP_ICON_RES_ID: u16 = 1;
const VERSION_RES_ID: u16 = 1;
const RES_MEMORY_FLAGS: u16 = 0x1030; // MOVEABLE | PURE | DISCARDABLE
const LANG_EN_US: u16 = 0x0409;

fn main() {
  println!("cargo:rerun-if-changed=build.rs");
  println!("cargo:rerun-if-changed=Cargo.toml");
  println!("cargo:rerun-if-changed=assets/app_icon.ico");

  // Always inspect target cfg env vars (not host `cfg!`), so Windows hosts
  // cross-compiling to `wasm32-unknown-unknown` never emit Windows linker flags.
  if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
    return;
  }

  let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
  let ico_path = manifest_dir.join("assets").join("app_icon.ico");
  let ico_bytes = fs::read(&ico_path)
    .unwrap_or_else(|err| panic!("failed to read {}: {err}", ico_path.display()));
  let pkg_version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());

  let res_bytes = build_windows_res(&ico_bytes, &pkg_version)
    .unwrap_or_else(|err| panic!("invalid ICO file {}: {err}", ico_path.display()));

  let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
  let res_path = out_dir.join("mica_app_icon.res");
  fs::write(&res_path, &res_bytes)
    .unwrap_or_else(|err| panic!("failed to write {}: {err}", res_path.display()));

  // MSVC `link.exe` and `lld-link` consume `.res` files natively.
  // Scope the linker argument strictly to the `mica-rust` binary target so
  // library/test/cdylib targets are untouched.
  let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
  if target_env == "msvc" {
    println!("cargo:rustc-link-arg-bin=mica-rust={}", res_path.display());
  } else {
    println!(
      "cargo:warning=skipping .res icon linking for unsupported Windows target env '{target_env}'"
    );
  }
}

/// Converts a standard `.ico` file plus package version into a 32-bit Windows
/// `.res` binary containing `RT_ICON` (`1..=N`), `RT_GROUP_ICON` (`1`), and
/// `RT_VERSION` (`1`, `VS_VERSION_INFO`).
fn build_windows_res(ico: &[u8], pkg_version: &str) -> Result<Vec<u8>, String> {
  if ico.len() < 6 {
    return Err("file shorter than 6-byte ICONDIR header".into());
  }
  let reserved = u16::from_le_bytes([ico[0], ico[1]]);
  let kind = u16::from_le_bytes([ico[2], ico[3]]);
  let count = u16::from_le_bytes([ico[4], ico[5]]);
  if reserved != 0 || kind != 1 || count == 0 {
    return Err(format!(
      "unexpected ICONDIR header (reserved={reserved}, type={kind}, count={count})"
    ));
  }

  let dir_end = 6usize
    .checked_add(usize::from(count) * 16)
    .ok_or_else(|| "ICONDIR overflow".to_string())?;
  if ico.len() < dir_end {
    return Err("truncated ICONDIRENTRY table".into());
  }

  let mut out = Vec::with_capacity(ico.len() + 512);

  // 32-byte null header required at the start of every 32-bit `.res` file.
  write_res_header(&mut out, 0, 0, 0, 0, 0);

  // Build `RT_GROUP_ICON` payload (`NEWHEADER` + `N` packed 14-byte `RESDIR` entries).
  let mut group = Vec::with_capacity(6 + usize::from(count) * 14);
  group.extend_from_slice(&0u16.to_le_bytes()); // reserved
  group.extend_from_slice(&1u16.to_le_bytes()); // RES_ICON
  group.extend_from_slice(&count.to_le_bytes());

  for idx in 0..count {
    let entry_off = 6 + usize::from(idx) * 16;
    let entry = &ico[entry_off..entry_off + 16];
    let bytes_in_res = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
    let image_offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
    let image_end = image_offset
      .checked_add(bytes_in_res)
      .ok_or_else(|| format!("frame {} offset overflow", idx + 1))?;
    if bytes_in_res == 0 || image_end > ico.len() {
      return Err(format!(
        "frame {} out of bounds (offset={image_offset}, len={bytes_in_res}, file={})",
        idx + 1,
        ico.len()
      ));
    }

    let icon_id = idx + 1;
    let frame_bytes = &ico[image_offset..image_end];
    write_res_entry(&mut out, RT_ICON, icon_id, frame_bytes);

    // First 12 bytes match `ICONDIRENTRY` (`width..=bytes_in_res`); replace the
    // 4-byte file offset with the 2-byte `RT_ICON` ordinal ID.
    group.extend_from_slice(&entry[0..12]);
    group.extend_from_slice(&icon_id.to_le_bytes());
  }

  write_res_entry(&mut out, RT_GROUP_ICON, APP_ICON_RES_ID, &group);
  let version_info = build_vs_version_info(pkg_version);
  write_res_entry(&mut out, RT_VERSION, VERSION_RES_ID, &version_info);
  Ok(out)
}

fn parse_version_quad(version: &str) -> (u16, u16, u16, u16) {
  let core = version.split(['-', '+']).next().unwrap_or(version);
  let mut parts = core.split('.').map(|s| s.parse::<u16>().unwrap_or(0));
  (
    parts.next().unwrap_or(0),
    parts.next().unwrap_or(0),
    parts.next().unwrap_or(0),
    parts.next().unwrap_or(0),
  )
}

fn build_vs_version_info(pkg_version: &str) -> Vec<u8> {
  let (major, minor, patch, build) = parse_version_quad(pkg_version);
  let ver_ms = (u32::from(major) << 16) | u32::from(minor);
  let ver_ls = (u32::from(patch) << 16) | u32::from(build);

  // 52-byte VS_FIXEDFILEINFO
  let mut fixed = Vec::with_capacity(52);
  for field in [
    0xFEEF_04BDu32, // dwSignature
    0x0001_0000u32, // dwStrucVersion
    ver_ms,         // dwFileVersionMS
    ver_ls,         // dwFileVersionLS
    ver_ms,         // dwProductVersionMS
    ver_ls,         // dwProductVersionLS
    0x0000_003Fu32, // dwFileFlagsMask
    0x0000_0000u32, // dwFileFlags
    0x0004_0004u32, // dwFileOS (VOS_NT_WINDOWS32)
    0x0000_0001u32, // dwFileType (VFT_APP)
    0x0000_0000u32, // dwFileSubtype
    0x0000_0000u32, // dwFileDateMS
    0x0000_0000u32, // dwFileDateLS
  ] {
    fixed.extend_from_slice(&field.to_le_bytes());
  }

  let strings = [
    ("CompanyName", "weironz"),
    ("FileDescription", "Mica"),
    ("FileVersion", pkg_version),
    ("InternalName", "mica-rust"),
    ("OriginalFilename", "mica-rust.exe"),
    ("ProductName", "Mica"),
    ("ProductVersion", pkg_version),
  ]
  .into_iter()
  .map(|(k, v)| build_version_string(k, v))
  .collect::<Vec<_>>();

  let string_table = build_version_block("040904B0", 1, &[], 0, &strings);
  let string_file_info = build_version_block("StringFileInfo", 1, &[], 0, &[string_table]);

  let mut translation_val = Vec::with_capacity(4);
  translation_val.extend_from_slice(&LANG_EN_US.to_le_bytes());
  translation_val.extend_from_slice(&0x04B0u16.to_le_bytes()); // UTF-16LE code page (1200)
  let translation = build_version_block("Translation", 0, &translation_val, 4, &[]);
  let var_file_info = build_version_block("VarFileInfo", 1, &[], 0, &[translation]);

  build_version_block(
    "VS_VERSION_INFO",
    0,
    &fixed,
    u16::try_from(fixed.len()).expect("VS_FIXEDFILEINFO fits u16"),
    &[string_file_info, var_file_info],
  )
}

fn build_version_string(key: &str, value: &str) -> Vec<u8> {
  let utf16_units: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
  let mut val_bytes = Vec::with_capacity(utf16_units.len() * 2);
  for unit in &utf16_units {
    val_bytes.extend_from_slice(&unit.to_le_bytes());
  }
  let unit_count = u16::try_from(utf16_units.len()).expect("version string fits u16");
  build_version_block(key, 1, &val_bytes, unit_count, &[])
}

fn build_version_block(
  key: &str,
  w_type: u16,
  value: &[u8],
  w_value_len: u16,
  children: &[Vec<u8>],
) -> Vec<u8> {
  let mut buf = Vec::new();
  buf.extend_from_slice(&0u16.to_le_bytes()); // wLength placeholder
  buf.extend_from_slice(&w_value_len.to_le_bytes());
  buf.extend_from_slice(&w_type.to_le_bytes());
  for unit in key.encode_utf16().chain(std::iter::once(0)) {
    buf.extend_from_slice(&unit.to_le_bytes());
  }
  while !buf.len().is_multiple_of(4) {
    buf.push(0);
  }
  buf.extend_from_slice(value);
  for (i, child) in children.iter().enumerate() {
    while !buf.len().is_multiple_of(4) {
      buf.push(0);
    }
    buf.extend_from_slice(child);
    if i + 1 < children.len() {
      while !buf.len().is_multiple_of(4) {
        buf.push(0);
      }
    }
  }
  let total_len = u16::try_from(buf.len()).expect("version block exceeds u16::MAX");
  buf[0..2].copy_from_slice(&total_len.to_le_bytes());
  buf
}

fn write_res_entry(out: &mut Vec<u8>, res_type: u16, res_id: u16, data: &[u8]) {
  let data_size = u32::try_from(data.len()).expect("resource data exceeds u32::MAX");
  write_res_header(out, data_size, res_type, res_id, RES_MEMORY_FLAGS, LANG_EN_US);
  out.extend_from_slice(data);
  while !out.len().is_multiple_of(4) {
    out.push(0);
  }
}

fn write_res_header(
  out: &mut Vec<u8>,
  data_size: u32,
  res_type: u16,
  res_id: u16,
  memory_flags: u16,
  language_id: u16,
) {
  out.extend_from_slice(&data_size.to_le_bytes());
  out.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize (ordinal type + ordinal name)
  out.extend_from_slice(&0xFFFFu16.to_le_bytes());
  out.extend_from_slice(&res_type.to_le_bytes());
  out.extend_from_slice(&0xFFFFu16.to_le_bytes());
  out.extend_from_slice(&res_id.to_le_bytes());
  out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
  out.extend_from_slice(&memory_flags.to_le_bytes());
  out.extend_from_slice(&language_id.to_le_bytes());
  out.extend_from_slice(&0u32.to_le_bytes()); // Version
  out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
}
