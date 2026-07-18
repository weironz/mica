//! Mica core — the shared, storage/transport-agnostic data plane (Phase 2).
//!
//! - [`block`]: the flat block DTO (mirror of `crates/markdown::Block`).
//! - [`marks`]: inline marks ↔ yrs `Text` formatting attributes.
//! - [`doc`]: the yrs CRDT document model ([`doc::MicaDoc`]) — P2-M1.
//!
//! Local store (SQLite), sync engine, and the editor binding arrive in later
//! milestones — see `docs/phase2-offline-crdt.md`. The round-trip smoke
//! functions below back the P2-M0 FFI pipeline check.

pub mod block;
pub mod doc;
pub mod marks;
#[cfg(feature = "store")]
pub mod store;

pub use block::Block;
pub use doc::{DocError, MicaDoc};
pub use marks::{marks_from_data, Mark};
#[cfg(feature = "store")]
pub use store::{
  Identity, LocalStore, LocalVersion, LocalView, LocalWorkspace, StoreError, SyncCursor,
};

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
