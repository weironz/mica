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
  if std::env::args().skip(1).any(|a| a == "--version" || a == "-V") {
    println!("Mica {}", env!("CARGO_PKG_VERSION"));
    std::process::exit(0);
  }

  // #179: Enforce single instance per profile directory (matching legacy Flutter's
  // named mutex so old Flutter and new pure-Rust clients never concurrently open
  // the same `store.db`). Surfaces the existing Mica window and exits 0.
  let Some(_single_instance_guard) = mica_rust::single_instance::acquire() else {
    std::process::exit(0);
  };

  platform::run_app(
    platform::AppSource::Rust(platform::Root::of::<mica_rust::MicaApp>()),
    platform::AppOptions {
      title: "Mica".to_owned(),
      #[cfg(any(target_os = "linux", target_os = "windows"))]
      app_id: "com.mica.rust.client".to_owned(),
      size: (1280, 820),
      ..Default::default()
    },
  )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("Build native binary on Windows/macOS/Linux; use nsh-platform-web for wasm32.");
