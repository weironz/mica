//! WebAssembly entry point for Mica's pure-Rust L3 shell (`nsh-platform-web`).
//!
//! Built with `nsh-platform-web` (`default-features = false`, zero TSX/JS runtime,
//! zero Dart/CanvasKit): compiles L1 + L2 + L3 into a single standalone `.wasm`.
//!
//! Passes an empty `ui_sets` list (`vec![]`) because `MicaApp` is 100% pure Rust
//! and never evaluates TSX/JS bundles against `nsh_base::registry()`. Passing
//! `nsh_base::registry()` previously force-rooted all 75 `nsh/base` widgets
//! (`syntect`, `html5ever`, `ratex`, `vte`, `zstd`, `plot`, `rich_editor`, etc.)
//! into the WASM binary so LTO could not dead-strip them.

#![cfg(target_arch = "wasm32")]

nsh_platform_web::rust_app!(
  Vec::new(),
  nsh_core::element::Root::of::<crate::MicaApp>()
);
