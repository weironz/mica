//! flutter_rust_bridge surface for the desktop client.
//!
//! Thin wrapper over the shared `mica-core` crate — keep all logic in mica-core
//! and only expose it here. P2-M0 just validates the round-trip; the CRDT/store
//! APIs land in later milestones (docs/phase2-offline-crdt.md).

/// FFI round-trip smoke test (string → string), delegated to mica-core.
#[flutter_rust_bridge::frb(sync)]
pub fn greet(name: String) -> String {
    mica_core::greet(&name)
}

/// FFI round-trip smoke test (ints), delegated to mica-core.
#[flutter_rust_bridge::frb(sync)]
pub fn add(a: i64, b: i64) -> i64 {
    mica_core::add(a, b)
}

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
