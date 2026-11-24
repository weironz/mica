//! Mica core — the shared, storage/transport-agnostic data plane (Phase 2).
//!
//! - [`block`]: the flat block DTO (re-export of `mica_md::Block`, the
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
#[cfg(feature = "store")]
pub mod store_stats;
pub mod text_diff;
/// Page-tree decisions shared by the server and the local store (pure).
pub mod tree_rules;
/// Usage statistics DTOs and UTC hourly-to-local daily activity folding.
pub mod usage_stats;
/// Transactional page-tree (view) subtree operations over the local store —
/// one recursive CTE + one transaction per operation (issue #67).
#[cfg(feature = "store")]
pub mod view_ops;
/// Single authoritative CJK + Latin word & character counting.
pub mod word_count;

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
#[cfg(feature = "store")]
pub use store_stats::LocalWorkspaceMedia;
pub use text_diff::{utf16_text_diff, TextDiff};
pub use usage_stats::{
  ActivitySource, DailyActivityPoint, FallbackDocCreation, HeatmapDayPoint, HourlyActivityRow,
  UsageActivityTotals, UsageHealth, UsageStatsActivity, UsageStatsOverview, UsageSummary,
  WorkspaceUsageItem,
};
#[cfg(feature = "store")]
pub use view_ops::CloneViewOutcome;
pub use word_count::{count_blocks, count_doc, count_text, WordCount};

/// The core crate version, so the desktop client can confirm which native build
/// it is actually bound to (catches stale-dylib mistakes early).
pub fn core_version() -> &'static str {
  env!("CARGO_PKG_VERSION")
}
