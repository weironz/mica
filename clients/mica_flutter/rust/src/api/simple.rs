//! flutter_rust_bridge surface for the desktop client.
//!
//! Thin wrapper over the shared `mica-core` crate — keep all logic in mica-core
//! and only expose it here. The CRDT/store
//! APIs land in docs/architecture/phase2-offline-crdt.md.

/// Version of the bound native core, so Dart can confirm the dylib it loaded.
#[flutter_rust_bridge::frb(sync)]
pub fn core_version() -> String {
    mica_core::core_version().to_string()
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    // Default utilities - feel free to customize
    flutter_rust_bridge::setup_default_user_utils();
}
