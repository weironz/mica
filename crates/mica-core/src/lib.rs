//! Mica core — the shared, storage/transport-agnostic data plane (Phase 2).
//!
//! For now (P2-M0) this only carries round-trip smoke functions used to validate
//! the Flutter ↔ Rust (flutter_rust_bridge v2) pipeline end to end. The CRDT
//! document model (yrs), local store (SQLite), and sync engine arrive in later
//! milestones — see `docs/phase2-offline-crdt.md`.

/// The core crate version, so the desktop client can confirm which native build
/// it is actually bound to (catches stale-dylib mistakes early).
pub fn core_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// FFI round-trip smoke test: a string in, a string out.
pub fn greet(name: &str) -> String {
    format!("Hello from mica-core, {name}")
}

/// FFI round-trip smoke test: integers across the boundary.
pub fn add(a: i64, b: i64) -> i64 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greet_includes_name() {
        assert_eq!(greet("Mica"), "Hello from mica-core, Mica");
    }

    #[test]
    fn add_works() {
        assert_eq!(add(2, 40), 42);
    }
}
