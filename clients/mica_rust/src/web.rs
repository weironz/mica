//! WebAssembly entry point for Mica's pure-Rust L3 shell (`nsh-platform-web`).
//!
//! Built with `nsh-platform-web` (`default-features = false`, zero TSX/JS runtime,
//! zero Dart/CanvasKit): compiles L1 + L2 + L3 into a single standalone `.wasm`.

#![cfg(target_arch = "wasm32")]

nsh_platform_web::rust_app!(
  vec![nsh_base::registry()],
  nsh_core::element::Root::of::<crate::MicaApp>()
);
