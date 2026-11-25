//! Mica L2 headless client application core (zero UI / `nsh-*` dependencies):
//! - [`cache_origin`]: Account-scoped `#111` cloud cache origin keys
//! - [`claim`]: Account/workspace/document/sync generation guards
//! - [`net`]: Async HTTP `CloudConnector` (`ureq` on desktop, `fetch` on Web)
//! - [`protocol`]: Single-source API endpoints & decoders over `mica-api-types`
//! - [`runtime`]: Headless client runtime shell
//! - [`search`]: Unified document + `mica-board` search & backlink graph
//! - [`storage`]: `VaultStore` SQLite / memory persistence & legacy store adoption
//! - [`tree`]: Page-tree ViewModel backed by `mica_core::tree_rules`
//! - [`web_idb`]: Web IndexedDB + Web Locks write-behind mirror for `mica-sync`
//! - [`web_runtime`]: Browser Fetch / `localStorage` / file-picker runtime helpers
//! - [`ws_transport`]: Cross-platform WebSocket transport (`tungstenite` / `web_sys::WebSocket`)

pub mod cache_origin;
pub mod claim;
pub mod net;
pub mod protocol;
pub mod runtime;
pub mod search;
pub mod storage;
pub mod tree;
pub mod web_idb;
pub mod web_runtime;
pub mod ws_transport;

pub use cache_origin::{
  CacheOriginError, cloud_cache_origin, is_account_scoped_cloud_origin, legacy_cloud_cache_origin,
  normalize_server_url, require_cloud_cache_origin,
};
pub use claim::{ClaimTracker, SessionClaim, WorldOrigin};
pub use net::{UploadedFile, build_file_blob_url, download_url_bytes, parse_mica_blob_link, sha256_hex};
pub use protocol::ApiEndpoints;
pub use runtime::MicaClientRuntime;
pub use search::{
  BacklinkEdge, SearchHit, build_backlink_graph, extract_document_search_text,
  search_cached_documents,
};
pub use tree::{PageTreeModel, TreeRow};
