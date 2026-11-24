//! Mica L2 headless client application core:
//! - [`claim`]: Account/workspace/document/sync generation guards
//! - [`tree`]: Page-tree ViewModel backed by `mica_core::tree_rules`
//! - [`protocol`]: Single-source API endpoints & decoders over `mica-api-types`
//! - [`search`]: Unified document + `mica-board` search & backlink graph

pub mod cache_origin;
pub mod claim;
pub mod protocol;
pub mod runtime;
pub mod search;
pub mod tree;

pub use cache_origin::{
  CacheOriginError, cloud_cache_origin, is_account_scoped_cloud_origin,
  legacy_cloud_cache_origin, normalize_server_url, require_cloud_cache_origin,
};
pub use claim::{ClaimTracker, SessionClaim, WorldOrigin};
pub use protocol::ApiEndpoints;
pub use runtime::MicaClientRuntime;
pub use search::{
  BacklinkEdge, SearchHit, build_backlink_graph, extract_document_search_text,
  search_cached_documents,
};
pub use tree::{PageTreeModel, TreeRow};
