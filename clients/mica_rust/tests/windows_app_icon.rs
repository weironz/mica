//! Regression tests for the Windows desktop taskbar / executable icon (`app_icon.ico`).
//!
//! Guards both the multi-size ICO asset and (on Windows) the real compiled
//! `mica-rust.exe` PE resource table so `nsh-platform-windows`'s
//! `LoadIconW(hinstance, MAKEINTRESOURCEW(1))` always succeeds.

use std::fs;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn app_icon_ico_contains_expected_multi_size_frames() {
  let ico_path = manifest_dir().join("assets").join("app_icon.ico");
  let bytes = fs::read(&ico_path).expect("clients/mica_rust/assets/app_icon.ico must exist");
  assert!(bytes.len() >= 6, "ICO file too short");
  assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), 0, "ICONDIR.reserved must be 0");
  assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), 1, "ICONDIR.type must be 1 (ICO)");
  let count = u16::from_le_bytes([bytes[4], bytes[5]]);
  assert_eq!(count, 7, "app_icon.ico must carry 7 rasterized sizes");

  let mut sizes = Vec::with_capacity(usize::from(count));
  for i in 0..usize::from(count) {
    let off = 6 + i * 16;
    let width = if bytes[off] == 0 { 256u16 } else { u16::from(bytes[off]) };
    let height = if bytes[off + 1] == 0 { 256u16 } else { u16::from(bytes[off + 1]) };
    assert_eq!(width, height, "icon frame {} must be square", i + 1);
    let len = u32::from_le_bytes([bytes[off + 8], bytes[off + 9], bytes[off + 10], bytes[off + 11]])
      as usize;
    let img_off = u32::from_le_bytes([
      bytes[off + 12],
      bytes[off + 13],
      bytes[off + 14],
      bytes[off + 15],
    ]) as usize;
    assert!(len > 0 && img_off + len <= bytes.len(), "frame {} bounds invalid", i + 1);
    sizes.push(width);
  }
  sizes.sort_unstable();
  assert_eq!(sizes, vec![16, 24, 32, 48, 64, 128, 256]);

  let iss = fs::read_to_string(manifest_dir().join("installer").join("mica.iss"))
    .expect("installer/mica.iss must exist");
  assert!(
    iss.contains(r"SetupIconFile=..\assets\app_icon.ico"),
    "mica.iss must embed app_icon.ico as SetupIconFile"
  );
}

#[cfg(target_os = "windows")]
#[test]
fn mica_rust_exe_embeds_win32_icon_resource_id_1() {
  use core::ffi::c_void;

  const LOAD_LIBRARY_AS_DATAFILE: u32 = 0x0000_0002;
  const IMAGE_ICON: u32 = 1;

  #[link(name = "kernel32")]
  unsafe extern "system" {
    fn LoadLibraryExW(
      lp_lib_file_name: *const u16,
      h_file: *mut c_void,
      dw_flags: u32,
    ) -> *mut c_void;
    fn FreeLibrary(h_lib_module: *mut c_void) -> i32;
  }

  #[link(name = "user32")]
  unsafe extern "system" {
    fn LoadIconW(h_instance: *mut c_void, lp_icon_name: *const u16) -> *mut c_void;
    fn LoadImageW(
      h_inst: *mut c_void,
      name: *const u16,
      r#type: u32,
      cx: i32,
      cy: i32,
      fu_load: u32,
    ) -> *mut c_void;
    fn DestroyIcon(h_icon: *mut c_void) -> i32;
  }

  let exe_path = env!("CARGO_BIN_EXE_mica-rust");
  let wide_path: Vec<u16> = exe_path.encode_utf16().chain(core::iter::once(0)).collect();

  // SAFETY: `wide_path` is NUL-terminated; module is freed before returning.
  let module =
    unsafe { LoadLibraryExW(wide_path.as_ptr(), core::ptr::null_mut(), LOAD_LIBRARY_AS_DATAFILE) };
  assert!(!module.is_null(), "failed to load {exe_path} as Win32 datafile");

  // Matches `nsh-platform-windows/src/window.rs`: `LoadIconW(Some(hinstance), PCWSTR(1 as _))`.
  let resource_id_1 = core::ptr::without_provenance::<u16>(1);
  let class_icon = unsafe { LoadIconW(module, resource_id_1) };
  let small_icon = unsafe { LoadImageW(module, resource_id_1, IMAGE_ICON, 16, 16, 0) };
  let taskbar_icon = unsafe { LoadImageW(module, resource_id_1, IMAGE_ICON, 32, 32, 0) };
  let hidpi_icon = unsafe { LoadImageW(module, resource_id_1, IMAGE_ICON, 48, 48, 0) };

  if !small_icon.is_null() {
    unsafe { DestroyIcon(small_icon) };
  }
  if !taskbar_icon.is_null() {
    unsafe { DestroyIcon(taskbar_icon) };
  }
  if !hidpi_icon.is_null() {
    unsafe { DestroyIcon(hidpi_icon) };
  }
  unsafe { FreeLibrary(module) };

  assert!(
    !class_icon.is_null(),
    "{exe_path} must embed RT_GROUP_ICON with integer resource ID 1 for NSH WNDCLASSEXW.hIcon"
  );
  assert!(!small_icon.is_null(), "{exe_path} must provide 16x16 small icon from resource ID 1");
  assert!(!taskbar_icon.is_null(), "{exe_path} must provide 32x32 taskbar icon from resource ID 1");
  assert!(!hidpi_icon.is_null(), "{exe_path} must provide 48x48 HiDPI icon from resource ID 1");
}
