//! Headless end-to-end integration tests for Mica's pure-Rust M1~M4 client (`clients/mica_rust`),
//! split into domain submodules so every regression suite stays concise and easy to navigate.

mod board_and_e2e;
mod editor_core;
mod images;
mod settings_and_cloud;
mod sidebar_and_shell;
mod tables;
