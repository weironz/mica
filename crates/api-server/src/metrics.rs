//! Prometheus metrics, hand-rolled.
//!
//! # Why there is no metrics crate here
//!
//! The exposition format is a stable, line-oriented text format, and a registry
//! is counters in a map. That is ~150 lines with no dependency, and CLAUDE.md's
//! "use a mature package for glue" caveat is about glue that would otherwise
//! mean re-implementing several platform-native layers (window management,
//! trays). There is no such multiplier here, so in-house-first applies plainly.
//!
//! # Why this exists at all
//!
//! 2026-08-03 production was unreachable for ~26 minutes and nothing said so.
//! The API process was alive and listening the whole time — the box was starved
//! for IO, so every request crawled and the health probes timed out. A liveness
//! check cannot express that: the only question it asks is "did you answer",
//! and the answer was "yes, eventually". What would have shown it, minutes
//! before anything failed, is request latency and DB-pool wait climbing. So the
//! first metrics here are exactly those, not a generic dashboard's worth.
//!
//! # Exposure
//!
//! Mounted on the ROOT router as `/metrics`, deliberately NOT under `/api`:
//! `deploy/nginx.conf` only proxies `/s/`, `/reset-password`, `/verify-email`,
//! `/api/` and `/ws/` to this process, so `/metrics` is unreachable from the
//! public internet while staying reachable inside the compose network.
//! Structural, not a shared token nobody rotates.
//!
//! Verified 2026-08-04 against the running stack, and the mechanism is worth
//! stating exactly because the obvious guess is wrong: a public
//! `GET /metrics` does **not** 404 — nginx's SPA fallback
//! (`try_files $uri $uri/ /index.html`) answers **200 with index.html**,
//! `content-type: text/html`, containing no `mica_` series at all. Safe, but
//! "it 404s" is the kind of half-right claim someone later builds on.
//!
//! **If nginx ever gains a broader proxy rule, this endpoint needs auth that
//! same day.**
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use axum::extract::{MatchedPath, State};
use axum::http::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use mica_app_core::AppState;

/// Histogram bucket upper bounds, in seconds. Prometheus's conventional
/// spread: dense where a healthy API lives (single-digit ms) and reaching far
/// enough out to still record the requests that made 2026-08-03 look fine to a
/// liveness check.
const BUCKETS: [f64; 12] = [
  0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, f64::INFINITY,
];

#[derive(Default)]
struct RouteStat {
  /// One counter per status code, so a route's error rate is visible without
  /// a second metric.
  by_status: HashMap<u16, u64>,
  bucket_counts: [u64; BUCKETS.len()],
  sum_seconds: f64,
  count: u64,
}

/// Everything this process counts.
///
/// A single `Mutex<HashMap>` on the request path is a contention point in
/// principle. In practice every request here also does at least one Postgres
/// round trip, so the lock is held for a rounding error of the total — and the
/// alternative (per-label atomics behind an RwLock) buys nothing measurable
/// while making the code harder to be sure about.
#[derive(Default)]
pub struct Metrics {
  routes: Mutex<HashMap<(String, String), RouteStat>>,
  ws_connections: AtomicI64,
}

impl Metrics {
  pub fn new() -> Self {
    Self::default()
  }

  fn record(&self, method: &str, route: &str, status: u16, seconds: f64) {
    let mut routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
    let stat = routes
      .entry((method.to_string(), route.to_string()))
      .or_default();
    *stat.by_status.entry(status).or_insert(0) += 1;
    stat.count += 1;
    stat.sum_seconds += seconds;
    for (i, upper) in BUCKETS.iter().enumerate() {
      if seconds <= *upper {
        stat.bucket_counts[i] += 1;
      }
    }
  }

  /// A live WebSocket arrived. Returns a guard that decrements on drop, so the
  /// gauge cannot leak on an early return or a panic in the connection task.
  pub fn ws_connected(&'static self) -> WsGuard {
    self.ws_connections.fetch_add(1, Ordering::Relaxed);
    WsGuard(self)
  }
}

pub struct WsGuard(&'static Metrics);

impl Drop for WsGuard {
  fn drop(&mut self) {
    self.0.ws_connections.fetch_sub(1, Ordering::Relaxed);
  }
}

/// The process-wide registry.
///
/// A `static` rather than a field on `AppState` on purpose: the WS guard has to
/// outlive individual handlers, and threading a registry through every layer
/// would buy testability we do not use — the exposition is tested by rendering
/// it, not by injecting a fake.
pub static METRICS: LazyLock<Metrics> = LazyLock::new(Metrics::new);

/// Escape a Prometheus label value (`\`, `"`, newline).
fn esc(s: &str) -> String {
  s.replace('\\', r"\\")
    .replace('"', "\\\"")
    .replace('\n', "\\n")
}

/// Render the exposition text. Pool numbers are sampled HERE rather than
/// tracked continuously — they are already gauges the pool maintains, and
/// sampling at scrape time is both cheaper and less wrong than mirroring them.
pub fn render(state: &AppState) -> String {
  let mut out = String::with_capacity(4096);

  out.push_str("# HELP mica_build_info Version of the running api-server.\n");
  out.push_str("# TYPE mica_build_info gauge\n");
  out.push_str(&format!(
    "mica_build_info{{version=\"{}\"}} 1\n",
    esc(env!("CARGO_PKG_VERSION"))
  ));

  let routes = METRICS.routes.lock().unwrap_or_else(|e| e.into_inner());

  out.push_str("# HELP mica_http_requests_total HTTP requests by route and status.\n");
  out.push_str("# TYPE mica_http_requests_total counter\n");
  for ((method, route), stat) in routes.iter() {
    for (status, n) in stat.by_status.iter() {
      out.push_str(&format!(
        "mica_http_requests_total{{method=\"{}\",route=\"{}\",status=\"{}\"}} {}\n",
        esc(method),
        esc(route),
        status,
        n
      ));
    }
  }

  out.push_str("# HELP mica_http_request_duration_seconds Request latency.\n");
  out.push_str("# TYPE mica_http_request_duration_seconds histogram\n");
  for ((method, route), stat) in routes.iter() {
    for (i, upper) in BUCKETS.iter().enumerate() {
      let le = if upper.is_infinite() {
        "+Inf".to_string()
      } else {
        format!("{upper}")
      };
      out.push_str(&format!(
        "mica_http_request_duration_seconds_bucket{{method=\"{}\",route=\"{}\",le=\"{}\"}} {}\n",
        esc(method),
        esc(route),
        le,
        stat.bucket_counts[i]
      ));
    }
    out.push_str(&format!(
      "mica_http_request_duration_seconds_sum{{method=\"{}\",route=\"{}\"}} {}\n",
      esc(method),
      esc(route),
      stat.sum_seconds
    ));
    out.push_str(&format!(
      "mica_http_request_duration_seconds_count{{method=\"{}\",route=\"{}\"}} {}\n",
      esc(method),
      esc(route),
      stat.count
    ));
  }
  drop(routes);

  // The pool is the metric that would have NAMED 2026-08-03 for what it was:
  // idle collapses to zero and stays there while requests queue behind it.
  out.push_str("# HELP mica_db_pool_connections Postgres pool connections by state.\n");
  out.push_str("# TYPE mica_db_pool_connections gauge\n");
  let size = state.db.size() as i64;
  let idle = state.db.num_idle() as i64;
  out.push_str(&format!("mica_db_pool_connections{{state=\"idle\"}} {idle}\n"));
  out.push_str(&format!(
    "mica_db_pool_connections{{state=\"in_use\"}} {}\n",
    (size - idle).max(0)
  ));

  out.push_str("# HELP mica_ws_connections Live document WebSocket connections.\n");
  out.push_str("# TYPE mica_ws_connections gauge\n");
  out.push_str(&format!(
    "mica_ws_connections {}\n",
    METRICS.ws_connections.load(Ordering::Relaxed)
  ));

  out
}

/// `GET /metrics`.
pub async fn metrics_handler(State(state): State<AppState>) -> Response {
  (
    [(
      axum::http::header::CONTENT_TYPE,
      "text/plain; version=0.0.4; charset=utf-8",
    )],
    render(&state),
  )
    .into_response()
}

/// Time every request and file it under its MATCHED path.
///
/// The matched path (`/api/workspaces/{id}`) rather than the requested one
/// (`/api/workspaces/9f3c…`), because a label built from user input is an
/// unbounded-cardinality bug waiting for its first crawler. Requests that match
/// no route share one bucket for the same reason.
pub async fn track(req: Request<axum::body::Body>, next: Next) -> Response {
  let method = req.method().as_str().to_string();
  let route = req
    .extensions()
    .get::<MatchedPath>()
    .map(|p| p.as_str().to_string())
    .unwrap_or_else(|| "<unmatched>".to_string());
  let started = Instant::now();
  let res = next.run(req).await;
  METRICS.record(
    &method,
    &route,
    res.status().as_u16(),
    started.elapsed().as_secs_f64(),
  );
  res
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The exposition has to parse as Prometheus text, and the easiest way to
  /// get that wrong is a label value carrying a quote.
  #[test]
  fn label_values_are_escaped() {
    assert_eq!(esc(r#"a"b"#), r#"a\"b"#);
    assert_eq!(esc(r"a\b"), r"a\\b");
    assert_eq!(esc("a\nb"), "a\\nb");
  }

  /// A histogram bucket is CUMULATIVE — `le="0.5"` counts everything at or
  /// below 0.5, not just the slice above the previous bound. Getting this
  /// backwards yields a histogram that looks plausible and quantiles that are
  /// nonsense.
  #[test]
  fn buckets_are_cumulative() {
    let m = Metrics::new();
    m.record("GET", "/api/health", 200, 0.001);
    m.record("GET", "/api/health", 200, 0.3);

    let routes = m.routes.lock().unwrap();
    let stat = &routes[&("GET".to_string(), "/api/health".to_string())];
    assert_eq!(stat.bucket_counts[0], 1, "0.005 holds only the 1ms request");
    assert_eq!(stat.bucket_counts[6], 2, "0.5 holds both");
    assert_eq!(
      stat.bucket_counts[BUCKETS.len() - 1],
      2,
      "+Inf holds everything, always"
    );
    assert_eq!(stat.count, 2);
  }

  #[test]
  fn statuses_are_counted_separately() {
    let m = Metrics::new();
    m.record("GET", "/api/x", 200, 0.01);
    m.record("GET", "/api/x", 200, 0.01);
    m.record("GET", "/api/x", 500, 0.01);

    let routes = m.routes.lock().unwrap();
    let stat = &routes[&("GET".to_string(), "/api/x".to_string())];
    assert_eq!(stat.by_status[&200], 2);
    assert_eq!(stat.by_status[&500], 1);
    assert_eq!(stat.count, 3, "the histogram counts every request once");
  }

  /// The gauge must come back down on an early return or a panic, or a few
  /// dropped connections turn into a permanently wrong number that nobody can
  /// tell from real load.
  #[test]
  fn the_ws_gauge_returns_to_zero() {
    static M: LazyLock<Metrics> = LazyLock::new(Metrics::new);
    {
      let _a = M.ws_connected();
      let _b = M.ws_connected();
      assert_eq!(M.ws_connections.load(Ordering::Relaxed), 2);
    }
    assert_eq!(M.ws_connections.load(Ordering::Relaxed), 0);
  }
}
