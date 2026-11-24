//! Windows single-instance named mutex guard (`#179`).
//!
//! Uses the exact same mutex naming formula as the legacy Flutter runner
//! (`clients/mica_flutter/windows/runner/main.cpp`) so an older Flutter build
//! and the new pure-Rust client mutually exclude each other when pointed at
//! the same `store.db` data directory, while smoke tests overriding `APPDATA`
//! get a distinct per-directory mutex and can run side by side.

/// Base named-mutex prefix for release builds (matches historical Flutter name).
pub const RELEASE_MUTEX_BASE: &str = r"Local\MicaSingleInstance";
/// Base named-mutex prefix for debug builds (matches historical Flutter dev name).
pub const DEBUG_MUTEX_BASE: &str = r"Local\MicaSingleInstanceDev";

/// Normalize a Windows directory string (lowercase, strip trailing separators
/// beyond a drive root like `c:\`) so two spellings compare equal.
pub fn normalize_dir_key(path: &str) -> String {
  let mut lower = path.trim().to_lowercase();
  while lower.len() > 3 && (lower.ends_with('\\') || lower.ends_with('/')) {
    lower.pop();
  }
  lower
}

/// FNV-1a 64-bit hash over UTF-16 code units, matching `SingleInstanceMutexName`
/// in `clients/mica_flutter/windows/runner/main.cpp` byte-for-byte.
pub fn fnv1a_utf16_hex(normalized_dir: &str) -> String {
  let mut hash: u64 = 14695981039346656037u64;
  for unit in normalized_dir.encode_utf16() {
    hash ^= u64::from(unit);
    hash = hash.wrapping_mul(1099511628211u64);
  }
  format!("{hash:016x}")
}

/// Compute the session-scoped single-instance mutex name for `(base, data_root, default_root)`.
pub fn compute_mutex_name(base: &str, data_root: &str, default_root: &str) -> String {
  let root = normalize_dir_key(data_root);
  let def = normalize_dir_key(default_root);
  if root.is_empty() || root == def {
    return base.to_string();
  }
  format!("{base}-{}", fnv1a_utf16_hex(&root))
}

/// Guard keeping the OS single-instance mutex handle open for the process lifetime.
pub struct SingleInstanceGuard {
  #[cfg(target_os = "windows")]
  handle: *mut core::ffi::c_void,
}

#[cfg(target_os = "windows")]
impl Drop for SingleInstanceGuard {
  fn drop(&mut self) {
    if !self.handle.is_null() {
      unsafe {
        win32::CloseHandle(self.handle);
      }
      self.handle = core::ptr::null_mut();
    }
  }
}

/// Acquire the single-instance mutex for the current profile directory.
///
/// - Returns `Some(guard)` if this process is the primary instance (or if OS
///   mutex creation unexpectedly fails, failing open rather than blocking launch).
/// - Returns `None` if another instance already holds the mutex (after
///   attempting to unhide and focus the existing Mica window).
pub fn acquire() -> Option<SingleInstanceGuard> {
  #[cfg(target_os = "windows")]
  {
    let base = if cfg!(debug_assertions) {
      DEBUG_MUTEX_BASE
    } else {
      RELEASE_MUTEX_BASE
    };
    let data_root = win32::data_root();
    let default_root = win32::default_data_root();
    let mutex_name = compute_mutex_name(base, &data_root, &default_root);
    acquire_named_windows_mutex(&mutex_name, true)
  }

  #[cfg(not(target_os = "windows"))]
  {
    Some(SingleInstanceGuard {})
  }
}

#[cfg(target_os = "windows")]
fn acquire_named_windows_mutex(mutex_name: &str, focus_existing: bool) -> Option<SingleInstanceGuard> {
  let wide = win32::to_wide(mutex_name);
  let handle = unsafe { win32::CreateMutexW(core::ptr::null(), 1, wide.as_ptr()) };
  if handle.is_null() {
    // Fail open: never block the only instance from launching.
    return Some(SingleInstanceGuard {
      handle: core::ptr::null_mut(),
    });
  }
  let last_err = unsafe { win32::GetLastError() };
  if last_err == win32::ERROR_ALREADY_EXISTS {
    if focus_existing {
      win32::focus_existing_mica_window();
    }
    unsafe {
      win32::CloseHandle(handle);
    }
    return None;
  }
  Some(SingleInstanceGuard { handle })
}

#[cfg(target_os = "windows")]
mod win32 {
  use core::ffi::c_void;

  pub const ERROR_ALREADY_EXISTS: u32 = 183;
  const SW_SHOW: i32 = 5;
  const SW_RESTORE: i32 = 9;

  #[link(name = "kernel32")]
  unsafe extern "system" {
    pub fn CreateMutexW(
      lp_mutex_attributes: *const c_void,
      b_initial_owner: i32,
      lp_name: *const u16,
    ) -> *mut c_void;
    pub fn GetLastError() -> u32;
    pub fn CloseHandle(h_object: *mut c_void) -> i32;
    fn GetFullPathNameW(
      lp_file_name: *const u16,
      n_buffer_length: u32,
      lp_buffer: *mut u16,
      lp_file_part: *mut *mut u16,
    ) -> u32;
  }

  #[link(name = "user32")]
  unsafe extern "system" {
    fn FindWindowW(lp_class_name: *const u16, lp_window_name: *const u16) -> *mut c_void;
    fn ShowWindow(h_wnd: *mut c_void, n_cmd_show: i32) -> i32;
    fn IsIconic(h_wnd: *mut c_void) -> i32;
    fn SetForegroundWindow(h_wnd: *mut c_void) -> i32;
  }

  pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
  }

  fn normalize_full_path(raw: &str) -> String {
    let wide = to_wide(raw);
    let needed = unsafe {
      GetFullPathNameW(wide.as_ptr(), 0, core::ptr::null_mut(), core::ptr::null_mut())
    };
    if needed == 0 {
      return super::normalize_dir_key(raw);
    }
    let mut buf = vec![0u16; needed as usize];
    let len = unsafe {
      GetFullPathNameW(
        wide.as_ptr(),
        needed,
        buf.as_mut_ptr(),
        core::ptr::null_mut(),
      )
    };
    if len == 0 || len >= needed {
      return super::normalize_dir_key(raw);
    }
    let full = String::from_utf16_lossy(&buf[..len as usize]);
    super::normalize_dir_key(&full)
  }

  pub fn data_root() -> String {
    match std::env::var("APPDATA") {
      Ok(val) if !val.trim().is_empty() => normalize_full_path(&val),
      _ => normalize_full_path("."),
    }
  }

  pub fn default_data_root() -> String {
    // `dirs_next::config_dir()` queries `SHGetKnownFolderPath(FOLDERID_RoamingAppData)`
    // directly on Windows (ignoring `%APPDATA%` overrides), matching Flutter's `DefaultDataRoot`.
    dirs_next::config_dir()
      .and_then(|p| p.to_str().map(normalize_full_path))
      .unwrap_or_default()
  }

  pub fn focus_existing_mica_window() {
    let title_mica = to_wide("Mica");
    let flutter_cls = to_wide("FLUTTER_RUNNER_WIN32_WINDOW");
    let flutter_title = to_wide("mica_flutter");

    // 1. Pure-Rust NSH window (title == "Mica")
    // 2. Legacy Flutter window (if still open during upgrade transition)
    let mut existing = unsafe { FindWindowW(core::ptr::null(), title_mica.as_ptr()) };
    if existing.is_null() {
      existing = unsafe { FindWindowW(flutter_cls.as_ptr(), title_mica.as_ptr()) };
    }
    if existing.is_null() {
      existing = unsafe { FindWindowW(flutter_cls.as_ptr(), flutter_title.as_ptr()) };
    }
    if existing.is_null() {
      existing = unsafe { FindWindowW(flutter_cls.as_ptr(), core::ptr::null()) };
    }
    if !existing.is_null() {
      unsafe {
        ShowWindow(existing, SW_SHOW);
        if IsIconic(existing) != 0 {
          ShowWindow(existing, SW_RESTORE);
        }
        SetForegroundWindow(existing);
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn mutex_name_uses_historical_name_for_default_appdata_and_hashes_custom_dir() {
    assert_eq!(
      compute_mutex_name(
        RELEASE_MUTEX_BASE,
        r"C:\Users\Alice\AppData\Roaming\",
        r"c:\users\alice\appdata\roaming",
      ),
      RELEASE_MUTEX_BASE
    );
    assert_eq!(
      compute_mutex_name(RELEASE_MUTEX_BASE, "", r"c:\users\alice\appdata\roaming"),
      RELEASE_MUTEX_BASE
    );

    let custom = compute_mutex_name(
      RELEASE_MUTEX_BASE,
      r"C:\Temp\MicaSmoke\",
      r"C:\Users\Alice\AppData\Roaming",
    );
    assert!(custom.starts_with(r"Local\MicaSingleInstance-"));
    assert_eq!(custom.len(), RELEASE_MUTEX_BASE.len() + 1 + 16);
    // Case- and trailing-slash-insensitive equality.
    assert_eq!(
      custom,
      compute_mutex_name(
        RELEASE_MUTEX_BASE,
        r"c:\temp\micasmoke",
        r"c:\users\alice\appdata\roaming",
      )
    );
  }

  #[cfg(target_os = "windows")]
  #[test]
  fn windows_named_mutex_rejects_second_holder_until_first_drops() {
    let unique = format!(r"Local\MicaSingleInstanceTest-{}", uuid::Uuid::new_v4());
    let first = acquire_named_windows_mutex(&unique, false);
    assert!(first.is_some(), "first acquisition must succeed");

    let second = acquire_named_windows_mutex(&unique, false);
    assert!(second.is_none(), "second acquisition while held must return None");

    drop(first);
    let third = acquire_named_windows_mutex(&unique, false);
    assert!(third.is_some(), "acquisition after drop must succeed");
  }
}
