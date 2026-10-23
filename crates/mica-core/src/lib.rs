//! Mica core — the shared, storage/transport-agnostic data plane (Phase 2).
//!
//! - [`block`]: the flat block DTO (re-export of `mica_markdown::Block`, the
//!   single definition shared with the markdown/REST model).
//! - [`marks`]: inline marks ↔ yrs `Text` formatting attributes.
//! - [`doc`]: the yrs CRDT document model ([`doc::MicaDoc`]) — P2-M1.
//!
//! The local store (SQLite) has since landed behind the `store` feature (the
//! `store` module below); the sync engine and editor binding are tracked in
//! `docs/architecture/phase2-offline-crdt.md`.

pub mod block;
pub mod doc;
pub mod marks;
pub mod quote_match;
pub mod search_text;
#[cfg(feature = "store")]
pub mod store;
pub mod text_diff;
/// Transactional page-tree (view) subtree operations over the local store —
/// one recursive CTE + one transaction per operation (issue #67).
#[cfg(feature = "store")]
pub mod view_ops;

pub use block::page_link_targets;
pub use block::Block;
pub use doc::{DocError, MicaDoc};
pub use marks::{marks_from_data, Mark};
pub use quote_match::QuoteIndex;
#[cfg(feature = "store")]
pub use store::{
  Identity, LocalBacklink, LocalGraph, LocalGraphEdge, LocalGraphNode, LocalSearchHit, LocalStore,
  LocalVersion, LocalView, LocalWorkspace, StoreError, SyncCursor,
};
pub use text_diff::{utf16_text_diff, TextDiff};
#[cfg(feature = "store")]
pub use view_ops::CloneViewOutcome;

/// The core crate version, so the desktop client can confirm which native build
/// it is actually bound to (catches stale-dylib mistakes early).
pub fn core_version() -> &'static str {
  env!("CARGO_PKG_VERSION")
}
