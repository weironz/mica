//! The server's OUTBOUND HTTP clients (#45). Every `reqwest::Client::new()`
//! scattered through the routes carried no timeout at all: a wedged upstream
//! (S3 endpoint, mail API, AI provider) parked a request handler — and the
//! client waiting on it — forever, and each site re-created the connection
//! pool. One shared client per use class lives here instead.
//!
//! Three classes, because one total bound cannot fit them all:
//!
//!  * [`outbound_client`] — storage, mail, avatar, export/import helper calls.
//!    Connect + total bounds; a single presigned request is a short operation.
//!  * [`ai_client`] — non-streaming AI completions. Connect + total bounds,
//!    the total from `MICA_AI_HTTP_TIMEOUT_SECS` (default 120 s) because a
//!    provider stalled mid-generation must fail the request, not hold it.
//!  * [`ai_stream_client`] — streaming completions (the AI websocket). Connect
//!    bound plus a per-read idle bound; NO total bound, because a total kills
//!    a long stream that is actively delivering tokens.
use std::sync::LazyLock;
use std::time::Duration;

/// How long any outbound call may spend just getting a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Total bound for an ordinary outbound call: connect through full body.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default for the AI calls, total (non-streaming) or per-read idle
/// (streaming); `MICA_AI_HTTP_TIMEOUT_SECS` overrides it from the environment
/// for providers whose generations legitimately run long.
const DEFAULT_AI_TIMEOUT: Duration = Duration::from_secs(120);

fn ai_timeout() -> Duration {
  std::env::var("MICA_AI_HTTP_TIMEOUT_SECS")
    .ok()
    .and_then(|v| v.trim().parse::<u64>().ok())
    .filter(|secs| *secs > 0)
    .map(Duration::from_secs)
    .unwrap_or(DEFAULT_AI_TIMEOUT)
}

static OUTBOUND: LazyLock<reqwest::Client> = LazyLock::new(|| {
  reqwest::Client::builder()
    .user_agent(concat!("mica-server/", env!("CARGO_PKG_VERSION")))
    .connect_timeout(CONNECT_TIMEOUT)
    .timeout(DEFAULT_TIMEOUT)
    .build()
    .expect("building the shared outbound HTTP client")
});

static AI: LazyLock<reqwest::Client> = LazyLock::new(|| {
  reqwest::Client::builder()
    .user_agent(concat!("mica-server/", env!("CARGO_PKG_VERSION")))
    .connect_timeout(CONNECT_TIMEOUT)
    .timeout(ai_timeout())
    .build()
    .expect("building the AI HTTP client")
});

static AI_STREAM: LazyLock<reqwest::Client> = LazyLock::new(|| {
  reqwest::Client::builder()
    .user_agent(concat!("mica-server/", env!("CARGO_PKG_VERSION")))
    .connect_timeout(CONNECT_TIMEOUT)
    // Per-read, not per-request: a stream that stops delivering for this long
    // is dead; one delivering steadily may run as long as it needs to.
    .read_timeout(ai_timeout())
    .build()
    .expect("building the AI streaming HTTP client")
});

/// The shared client for ordinary outbound calls (storage, mail, …). Cloning a
/// `reqwest::Client` is an Arc bump, so handing back a clone costs nothing.
pub(crate) fn outbound_client() -> reqwest::Client {
  OUTBOUND.clone()
}

/// The shared client for NON-streaming AI calls; total-bound, see above.
pub(crate) fn ai_client() -> reqwest::Client {
  AI.clone()
}

/// The shared client for STREAMING AI calls; no total bound, idle-bound reads.
pub(crate) fn ai_stream_client() -> reqwest::Client {
  AI_STREAM.clone()
}
