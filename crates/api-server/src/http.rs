//! The one outbound HTTP client the server uses for object storage, AI
//! providers and mail.
//!
//! `crate::http::client()` has NO timeouts at all: a storage node or provider
//! that accepts the connection and then goes quiet held the request — and the
//! worker task behind it — forever. Built once and cloned (a clone shares the
//! connection pool, which ~20 separate `Client::new()` calls never did).
//!
//! The read timeout bounds a STALL, not the whole request, so a large blob or
//! a long AI stream still completes; it is generous because a non-streaming
//! completion sends nothing until the model has finished. Paths with sharper
//! needs (the SSRF-pinned image fetch, exports) keep their own builders.

use std::sync::OnceLock;
use std::time::Duration;

pub(crate) fn client() -> reqwest::Client {
  static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
  CLIENT
    .get_or_init(|| {
      reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(300))
        .build()
        .expect("building the shared HTTP client")
    })
    .clone()
}
