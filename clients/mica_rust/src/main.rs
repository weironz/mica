//! Native desktop launcher for the Mica L3 pure-Rust shell (Route 2-B).

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn main() -> impl std::process::Termination {
  platform::run_app(
    platform::AppSource::Rust(platform::Root::of::<mica_rust::MicaApp>()),
    platform::AppOptions {
      title: "Mica (Fullstack Rust — Route 2-B)".to_owned(),
      #[cfg(any(target_os = "linux", target_os = "windows"))]
      app_id: "com.mica.rust.client".to_owned(),
      size: (1200, 800),
      ..Default::default()
    },
  )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("Build native binary on Windows/macOS/Linux; use nsh-platform-web for wasm32.");
