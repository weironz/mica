//! Prometheus metrics, hand-rolled.
//!
//! # Why there is no metrics crate here
//!
//! The exposition format is a stable, line-oriented text format, and a registry
//! is counters in a map. That is a few hundred lines with no dependency, and
//! CLAUDE.md's "use a mature package for glue" caveat is about glue that would
//! otherwise mean re-implementing several platform-native layers (window
//! management, trays). There is no such multiplier here, so in-house-first
//! applies plainly.
//!
//! # What is measured, and why these things
//!
//! 2026-08-03 production was unreachable for ~26 minutes and nothing said so.
//! The API process was alive and listening the whole time — the box was starved
//! for IO, so every request crawled and the health probes timed out. A liveness
//! check cannot express that: it only asks "did you answer", and the answer was
//! "yes, eventually".
//!
//! So the metrics here are chosen against **this product's known failure
//! modes**, not against a generic dashboard:
//!
//! * **CRDT integrity** (`mica_crdt_integrity_failures_total`) — red line #1 is
//!   "never diverge silently". A rejected-because-unverifiable update used to
//!   exist only as a log line, so if it ever started happening there was no
//!   curve at all; you would learn about it when a user said their document was
//!   scrambled.
//! * **Push cost** (`mica_crdt_push_*`) — every push decodes and re-encodes the
//!   whole document (see the write-amplification entry in docs/roadmap.md).
//!   Without a number, "how expensive is that really" stays an argument.
//! * **Client lag** (`mica_crdt_lag_notices_total`) — the server telling a
//!   client it fell off the broadcast window is a silent-divergence adjacent
//!   event; it was log-only too.
//! * **DB pool** — the 08-03 symptom was `acquire` taking 3.3s. The pool
//!   gauges show saturation; the *probe* histogram shows the wait itself, which
//!   a 15s-sampled gauge can miss entirely between scrapes.
//! * **Process RSS / FDs** — that incident was a memory-and-IO event. A
//!   hand-rolled exporter gets none of the `process_*` series a client library
//!   would hand you for free, so they are read from `/proc` here.
//! * **Capacity** — the per-workspace quota is enforced at whatever
//!   `MICA_WORKSPACE_QUOTA_BYTES` says (5 GiB by default); "who is about to
//!   hit the wall" was previously answerable only by hand.
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
use std::fmt::Write as _;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, State};
use axum::http::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use mica_app_core::AppState;
use sqlx::PgPool;

/// Histogram bucket upper bounds, in seconds. Prometheus's conventional
/// spread: dense where a healthy request lives (single-digit ms) and reaching
/// far enough out to still record the requests that made 2026-08-03 look fine
/// to a liveness check.
const BUCKETS: [f64; 12] = [
  0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, f64::INFINITY,
];

/// How long a database-derived snapshot is reused. The scrape interval is 15s
/// and these are counts over whole tables — recomputing them every scrape would
/// make the act of watching the system a load on it.
const DB_SNAPSHOT_TTL: Duration = Duration::from_secs(60);

/// One histogram. Shared by every latency series here so the cumulative-bucket
/// rule is implemented once — it is the easiest thing in this file to get
/// subtly wrong, and wrong buckets produce quantiles that look plausible.
#[derive(Default)]
struct Histogram {
  buckets: [u64; BUCKETS.len()],
  sum: f64,
  count: u64,
}

impl Histogram {
  fn observe(&mut self, seconds: f64) {
    self.count += 1;
    self.sum += seconds;
    for (i, upper) in BUCKETS.iter().enumerate() {
      if seconds <= *upper {
        self.buckets[i] += 1;
      }
    }
  }

  /// `labels` is either empty or a rendered label list WITHOUT braces, e.g.
  /// `method="GET",route="/api/health"`.
  fn render(&self, out: &mut String, name: &str, labels: &str) {
    let sep = if labels.is_empty() { "" } else { "," };
    for (i, upper) in BUCKETS.iter().enumerate() {
      let le = if upper.is_infinite() {
        "+Inf".to_string()
      } else {
        format!("{upper}")
      };
      let _ = writeln!(
        out,
        "{name}_bucket{{{labels}{sep}le=\"{le}\"}} {}",
        self.buckets[i]
      );
    }
    let braced = if labels.is_empty() {
      String::new()
    } else {
      format!("{{{labels}}}")
    };
    let _ = writeln!(out, "{name}_sum{braced} {}", self.sum);
    let _ = writeln!(out, "{name}_count{braced} {}", self.count);
  }
}

#[derive(Default)]
struct RouteStat {
  /// One counter per status code, so a route's error rate is visible without a
  /// second metric.
  by_status: HashMap<u16, u64>,
  latency: Histogram,
}

#[derive(Default)]
struct PushStats {
  latency: Histogram,
  bytes: u64,
  rejected: u64,
}

#[derive(Default)]
struct BlobGcStats {
  sweeps: u64,
  scanned: u64,
  deleted: u64,
  bytes_freed: u64,
  failures: u64,
}

/// The most workspace series one scrape will emit.
///
/// A cap, not a filter. node_exporter does not decide which disks are
/// interesting, and neither does this — every workspace gets a series so the
/// alert rule owns the threshold and the history is there to compute a growth
/// rate from. The cap only exists so a very large instance cannot turn a scrape
/// into a cardinality event, and when it bites it says so
/// (`mica_workspace_series_truncated`) rather than quietly showing fewer.
/// Ordered by usage, so the rows that get dropped are the empty ones.
const MAX_WORKSPACE_SERIES: i64 = 1000;

/// One workspace's storage, for the per-workspace gauges.
#[derive(Clone)]
struct WorkspaceUsage {
  id: String,
  name: String,
  bytes: i64,
}

/// Counts derived from the database. Kept behind [`DB_SNAPSHOT_TTL`].
#[derive(Clone, Default)]
struct DbSnapshot {
  users: i64,
  workspaces: i64,
  documents: i64,
  /// Total bytes across every workspace.
  ///
  /// Computed with the SAME expression the quota enforces
  /// (`sum(byte_size) FROM files WHERE workspace_id = …`, i.e.
  /// `store::workspace_bytes_used`). If the two ever disagree the gauge is
  /// lying about who is near the wall, which is the whole reason it exists —
  /// `the_capacity_gauge_matches_what_the_quota_enforces` pins them together.
  bytes_total: i64,
  /// Per workspace, ordered by usage descending, capped at
  /// [`MAX_WORKSPACE_SERIES`].
  workspace_usage: Vec<WorkspaceUsage>,
  /// How many workspaces the cap left out. Zero in every ordinary case.
  workspaces_truncated: i64,
}

/// Everything this process counts.
///
/// The per-family `Mutex` on the request path is a contention point in
/// principle. In practice every request here also does at least one Postgres
/// round trip, so a lock held for a map lookup is a rounding error — and the
/// alternative (per-label atomics behind an RwLock) buys nothing measurable
/// while making the code harder to be sure about.
#[derive(Default)]
pub struct Metrics {
  routes: Mutex<HashMap<(String, String), RouteStat>>,
  in_flight: AtomicI64,
  ws_connections: AtomicI64,

  integrity_failures: Mutex<HashMap<&'static str, u64>>,
  push: Mutex<PushStats>,
  lag_notices: AtomicU64,

  acquire_probe: Mutex<Histogram>,

  blob_gc: Mutex<BlobGcStats>,
  db_snapshot: Mutex<Option<(Instant, DbSnapshot)>>,
}

impl Metrics {
  pub fn new() -> Self {
    Self::default()
  }

  fn record_http(&self, method: &str, route: &str, status: u16, seconds: f64) {
    let mut routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
    let stat = routes
      .entry((method.to_string(), route.to_string()))
      .or_default();
    *stat.by_status.entry(status).or_insert(0) += 1;
    stat.latency.observe(seconds);
  }

  /// A CRDT update this server refused to apply. `kind` is a small fixed set
  /// (`&'static str`, never user input) so the label can never grow unbounded.
  pub fn integrity_failure(&self, kind: &'static str) {
    let mut m = self
      .integrity_failures
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    *m.entry(kind).or_insert(0) += 1;
  }

  /// One `sync.push`: how long the whole decode → apply → re-encode → upsert
  /// round trip took, how many update bytes arrived, and whether it was
  /// refused.
  pub fn record_push(&self, seconds: f64, bytes: usize, accepted: bool) {
    let mut p = self.push.lock().unwrap_or_else(|e| e.into_inner());
    p.latency.observe(seconds);
    p.bytes += bytes as u64;
    if !accepted {
      p.rejected += 1;
    }
  }

  /// The server told a client it fell off the broadcast window.
  pub fn lag_notice(&self) {
    self.lag_notices.fetch_add(1, Ordering::Relaxed);
  }

  pub fn record_acquire_probe(&self, seconds: f64) {
    self
      .acquire_probe
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .observe(seconds);
  }

  pub fn record_blob_gc(&self, scanned: u64, deleted: u64, bytes_freed: u64) {
    let mut g = self.blob_gc.lock().unwrap_or_else(|e| e.into_inner());
    g.sweeps += 1;
    g.scanned += scanned;
    g.deleted += deleted;
    g.bytes_freed += bytes_freed;
  }

  pub fn blob_gc_failed(&self) {
    self
      .blob_gc
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .failures += 1;
  }

  /// A live WebSocket arrived. Returns a guard that decrements on drop, so the
  /// gauge cannot leak on an early return or a panic in the connection task.
  pub fn ws_connected(&'static self) -> Guard {
    self.ws_connections.fetch_add(1, Ordering::Relaxed);
    Guard(&self.ws_connections)
  }

  fn request_started(&'static self) -> Guard {
    self.in_flight.fetch_add(1, Ordering::Relaxed);
    Guard(&self.in_flight)
  }
}

/// Decrements its counter on drop. The gauges it backs must survive early
/// returns and panics — a few leaked increments turn into a number nobody can
/// tell from real load.
pub struct Guard(&'static AtomicI64);

impl Drop for Guard {
  fn drop(&mut self) {
    self.0.fetch_sub(1, Ordering::Relaxed);
  }
}

/// The process-wide registry.
///
/// A `static` rather than a field on `AppState` on purpose: the guards have to
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

// ── process (Linux only) ────────────────────────────────────────────────────

/// Resident set size in bytes, from `/proc/self/status`.
///
/// Linux only, and that is not a hedge: the server runs in a Linux container,
/// while a Windows dev box simply omits these series rather than reporting a
/// zero that would read as "no memory used".
fn process_rss_bytes() -> Option<u64> {
  let status = std::fs::read_to_string("/proc/self/status").ok()?;
  let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
  let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
  Some(kb * 1024)
}

fn process_open_fds() -> Option<u64> {
  Some(std::fs::read_dir("/proc/self/fd").ok()?.count() as u64)
}

// ── database-derived snapshot ───────────────────────────────────────────────

async fn load_db_snapshot(db: &PgPool) -> Option<DbSnapshot> {
  // Counts first. Cheap at this scale, and the byte aggregate rides
  // `idx_files_workspace_bytes` (an index-only scan).
  let row: (i64, i64, i64, i64) = sqlx::query_as(
    r#"
    SELECT
      (SELECT count(*) FROM users)::bigint,
      (SELECT count(*) FROM workspaces)::bigint,
      (SELECT count(*) FROM documents)::bigint,
      coalesce((SELECT sum(total) FROM
        (SELECT sum(byte_size) AS total FROM files GROUP BY workspace_id) t), 0)::bigint
    "#,
  )
  .fetch_one(db)
  .await
  .ok()?;

  // LEFT JOIN, so a workspace with no blobs still reports 0 rather than
  // vanishing: "this one is empty" and "this one is not being measured" have to
  // look different, and a missing series reads as the latter.
  let rows: Vec<(uuid::Uuid, String, i64)> = sqlx::query_as(
    r#"
    SELECT w.id, w.name, coalesce(sum(f.byte_size), 0)::bigint AS used
    FROM workspaces w
    LEFT JOIN files f ON f.workspace_id = w.id
    GROUP BY w.id, w.name
    ORDER BY used DESC, w.id
    LIMIT $1
    "#,
  )
  .bind(MAX_WORKSPACE_SERIES)
  .fetch_all(db)
  .await
  .ok()?;

  Some(DbSnapshot {
    users: row.0,
    workspaces: row.1,
    documents: row.2,
    bytes_total: row.3,
    workspaces_truncated: (row.1 - rows.len() as i64).max(0),
    workspace_usage: rows
      .into_iter()
      .map(|(id, name, bytes)| WorkspaceUsage {
        id: id.to_string(),
        name,
        bytes,
      })
      .collect(),
  })
}

/// The cached snapshot, refreshed at most once per [`DB_SNAPSHOT_TTL`].
///
/// A failed query returns the STALE value rather than nothing: a scrape that
/// silently drops series looks identical to "the number went to zero", and a
/// minute-old count is a better answer than a wrong one.
async fn db_snapshot(state: &AppState) -> Option<DbSnapshot> {
  let cached = {
    let guard = METRICS
      .db_snapshot
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    guard.clone()
  };
  if let Some((at, snap)) = &cached {
    if at.elapsed() < DB_SNAPSHOT_TTL {
      return Some(snap.clone());
    }
  }
  match load_db_snapshot(&state.db).await {
    Some(fresh) => {
      *METRICS
        .db_snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), fresh.clone()));
      Some(fresh)
    }
    None => cached.map(|(_, snap)| snap),
  }
}

/// Periodically time one `acquire()` against the pool.
///
/// A PROBE, and the name says so: it does not instrument every acquisition
/// (sqlx acquires implicitly inside every `fetch_*`, so real coverage would
/// mean touching every call site). It measures the thing that actually went
/// wrong on 2026-08-03 — how long it takes to get a connection right now —
/// which the idle/in_use gauges can only imply, and can miss entirely between
/// two 15s scrapes.
pub fn spawn_pool_probe(db: PgPool) {
  tokio::spawn(async move {
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    loop {
      tick.tick().await;
      let started = Instant::now();
      match db.acquire().await {
        Ok(conn) => {
          METRICS.record_acquire_probe(started.elapsed().as_secs_f64());
          drop(conn);
        }
        // A failed acquire is the pool being unusable; record the full wait so
        // the histogram shows the stall rather than skipping the sample.
        Err(_) => METRICS.record_acquire_probe(started.elapsed().as_secs_f64()),
      }
    }
  });
}

// ── exposition ──────────────────────────────────────────────────────────────

fn render(state: &AppState, snapshot: Option<&DbSnapshot>) -> String {
  let mut out = String::with_capacity(8192);

  out.push_str("# HELP mica_build_info Version of the running api-server.\n");
  out.push_str("# TYPE mica_build_info gauge\n");
  let _ = writeln!(
    out,
    "mica_build_info{{version=\"{}\"}} 1",
    esc(env!("CARGO_PKG_VERSION"))
  );

  // ── HTTP ──
  {
    let routes = METRICS.routes.lock().unwrap_or_else(|e| e.into_inner());
    out.push_str("# HELP mica_http_requests_total HTTP requests by route and status.\n");
    out.push_str("# TYPE mica_http_requests_total counter\n");
    for ((method, route), stat) in routes.iter() {
      for (status, n) in stat.by_status.iter() {
        let _ = writeln!(
          out,
          "mica_http_requests_total{{method=\"{}\",route=\"{}\",status=\"{}\"}} {n}",
          esc(method),
          esc(route),
          status
        );
      }
    }
    out.push_str("# HELP mica_http_request_duration_seconds Request latency.\n");
    out.push_str("# TYPE mica_http_request_duration_seconds histogram\n");
    for ((method, route), stat) in routes.iter() {
      let labels = format!("method=\"{}\",route=\"{}\"", esc(method), esc(route));
      stat
        .latency
        .render(&mut out, "mica_http_request_duration_seconds", &labels);
    }
  }
  out.push_str("# HELP mica_http_requests_in_flight Requests currently being served.\n");
  out.push_str("# TYPE mica_http_requests_in_flight gauge\n");
  let _ = writeln!(
    out,
    "mica_http_requests_in_flight {}",
    METRICS.in_flight.load(Ordering::Relaxed)
  );

  // ── CRDT ──
  {
    let failures = METRICS
      .integrity_failures
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    out.push_str(
      "# HELP mica_crdt_integrity_failures_total Updates refused as unverifiable (red line #1).\n",
    );
    out.push_str("# TYPE mica_crdt_integrity_failures_total counter\n");
    // Always emit the zero series: a counter that only appears once it fires
    // cannot be alerted on before it fires, which is the one moment that
    // matters. `absent()` rules are a workaround for not doing this.
    if failures.is_empty() {
      out.push_str("mica_crdt_integrity_failures_total{kind=\"push\"} 0\n");
    }
    for (kind, n) in failures.iter() {
      let _ = writeln!(
        out,
        "mica_crdt_integrity_failures_total{{kind=\"{}\"}} {n}",
        esc(kind)
      );
    }
  }
  {
    let push = METRICS.push.lock().unwrap_or_else(|e| e.into_inner());
    out.push_str("# HELP mica_crdt_push_duration_seconds Whole sync.push round trip.\n");
    out.push_str("# TYPE mica_crdt_push_duration_seconds histogram\n");
    push
      .latency
      .render(&mut out, "mica_crdt_push_duration_seconds", "");
    out.push_str("# HELP mica_crdt_push_bytes_total Update bytes received.\n");
    out.push_str("# TYPE mica_crdt_push_bytes_total counter\n");
    let _ = writeln!(out, "mica_crdt_push_bytes_total {}", push.bytes);
    out.push_str("# HELP mica_crdt_push_rejected_total Pushes the server refused.\n");
    out.push_str("# TYPE mica_crdt_push_rejected_total counter\n");
    let _ = writeln!(out, "mica_crdt_push_rejected_total {}", push.rejected);
  }
  out.push_str("# HELP mica_crdt_lag_notices_total Clients told they fell off the window.\n");
  out.push_str("# TYPE mica_crdt_lag_notices_total counter\n");
  let _ = writeln!(
    out,
    "mica_crdt_lag_notices_total {}",
    METRICS.lag_notices.load(Ordering::Relaxed)
  );

  out.push_str("# HELP mica_ws_connections Live document WebSocket connections.\n");
  out.push_str("# TYPE mica_ws_connections gauge\n");
  let _ = writeln!(
    out,
    "mica_ws_connections {}",
    METRICS.ws_connections.load(Ordering::Relaxed)
  );

  // ── database ──
  out.push_str("# HELP mica_db_pool_connections Postgres pool connections by state.\n");
  out.push_str("# TYPE mica_db_pool_connections gauge\n");
  let size = state.db.size() as i64;
  let idle = state.db.num_idle() as i64;
  let _ = writeln!(out, "mica_db_pool_connections{{state=\"idle\"}} {idle}");
  let _ = writeln!(
    out,
    "mica_db_pool_connections{{state=\"in_use\"}} {}",
    (size - idle).max(0)
  );
  {
    let probe = METRICS
      .acquire_probe
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    out.push_str(
      "# HELP mica_db_acquire_probe_seconds Time to take one pooled connection (sampled).\n",
    );
    out.push_str("# TYPE mica_db_acquire_probe_seconds histogram\n");
    probe.render(&mut out, "mica_db_acquire_probe_seconds", "");
  }

  // ── blob GC ──
  {
    let gc = METRICS.blob_gc.lock().unwrap_or_else(|e| e.into_inner());
    out.push_str("# HELP mica_blob_gc_sweeps_total Completed blob GC sweeps.\n");
    out.push_str("# TYPE mica_blob_gc_sweeps_total counter\n");
    let _ = writeln!(out, "mica_blob_gc_sweeps_total {}", gc.sweeps);
    out.push_str("# HELP mica_blob_gc_objects_total Blobs seen and reclaimed.\n");
    out.push_str("# TYPE mica_blob_gc_objects_total counter\n");
    let _ = writeln!(out, "mica_blob_gc_objects_total{{op=\"scanned\"}} {}", gc.scanned);
    let _ = writeln!(out, "mica_blob_gc_objects_total{{op=\"deleted\"}} {}", gc.deleted);
    out.push_str("# HELP mica_blob_gc_bytes_freed_total Bytes reclaimed.\n");
    out.push_str("# TYPE mica_blob_gc_bytes_freed_total counter\n");
    let _ = writeln!(out, "mica_blob_gc_bytes_freed_total {}", gc.bytes_freed);
    out.push_str("# HELP mica_blob_gc_failures_total Sweeps that ended in an error.\n");
    out.push_str("# TYPE mica_blob_gc_failures_total counter\n");
    let _ = writeln!(out, "mica_blob_gc_failures_total {}", gc.failures);
  }

  // ── process (Linux) ──
  if let Some(rss) = process_rss_bytes() {
    out.push_str("# HELP mica_process_resident_bytes Resident set size.\n");
    out.push_str("# TYPE mica_process_resident_bytes gauge\n");
    let _ = writeln!(out, "mica_process_resident_bytes {rss}");
  }
  if let Some(fds) = process_open_fds() {
    out.push_str("# HELP mica_process_open_fds Open file descriptors.\n");
    out.push_str("# TYPE mica_process_open_fds gauge\n");
    let _ = writeln!(out, "mica_process_open_fds {fds}");
  }

  // ── capacity / business ──
  if let Some(s) = snapshot {
    out.push_str("# HELP mica_users_total Registered users.\n");
    out.push_str("# TYPE mica_users_total gauge\n");
    let _ = writeln!(out, "mica_users_total {}", s.users);
    out.push_str("# HELP mica_workspaces_total Workspaces.\n");
    out.push_str("# TYPE mica_workspaces_total gauge\n");
    let _ = writeln!(out, "mica_workspaces_total {}", s.workspaces);
    out.push_str("# HELP mica_documents_total Documents (including trashed).\n");
    out.push_str("# TYPE mica_documents_total gauge\n");
    let _ = writeln!(out, "mica_documents_total {}", s.documents);
    out.push_str("# HELP mica_storage_bytes Stored bytes across the instance.\n");
    out.push_str("# TYPE mica_storage_bytes gauge\n");
    let _ = writeln!(out, "mica_storage_bytes{{scope=\"total\"}} {}", s.bytes_total);

    // Per workspace, shaped like node_exporter's per-filesystem series: expose
    // the fact for EVERY workspace and let the alert rule own the threshold.
    //
    //   mica_workspace_bytes_used / on() group_left() mica_workspace_quota_bytes > 0.8
    //
    // An earlier draft emitted only workspaces already past a hard-coded 50%.
    // That bakes policy into the binary (redeploy to retune), and worse, it
    // destroys the history: a workspace appears only once it is already in
    // trouble, so there is no earlier data to compute a growth rate from — and
    // the growth rate is the only thing that warns you EARLY.
    //
    // `largest_workspace` used to be a separate series; it is `max()` of this
    // one now, and two ways to say the same number is how they drift.
    out.push_str("# HELP mica_workspace_bytes_used Stored bytes, per workspace.\n");
    out.push_str("# TYPE mica_workspace_bytes_used gauge\n");
    for w in &s.workspace_usage {
      let _ = writeln!(
        out,
        "mica_workspace_bytes_used{{workspace_id=\"{}\"}} {}",
        esc(&w.id),
        w.bytes
      );
    }
    // The name rides an info metric rather than every sample: a rename would
    // otherwise fork the whole series and break its history, and the standard
    // join (`* on(workspace_id) group_left(name)`) puts it back on the graph.
    out.push_str("# HELP mica_workspace_info Workspace id → name, for joins.\n");
    out.push_str("# TYPE mica_workspace_info gauge\n");
    for w in &s.workspace_usage {
      let _ = writeln!(
        out,
        "mica_workspace_info{{workspace_id=\"{}\",name=\"{}\"}} 1",
        esc(&w.id),
        esc(&w.name)
      );
    }
    out.push_str("# HELP mica_workspace_series_truncated Workspaces left out of the scrape.\n");
    out.push_str("# TYPE mica_workspace_series_truncated gauge\n");
    let _ = writeln!(
      out,
      "mica_workspace_series_truncated {}",
      s.workspaces_truncated
    );

    // Unlabelled on purpose: the quota is ONE config value today, so copying it
    // onto every workspace series would be N copies of a constant pretending to
    // be per-workspace data. If per-workspace overrides ever land, this gains
    // the label then — and the rule above already reads that way.
    out.push_str("# HELP mica_workspace_quota_bytes The per-workspace limit in force.\n");
    out.push_str("# TYPE mica_workspace_quota_bytes gauge\n");
    let _ = writeln!(
      out,
      "mica_workspace_quota_bytes {}",
      state.config.workspace_quota_bytes
    );
  }

  out
}

/// `GET /metrics`.
pub async fn metrics_handler(State(state): State<AppState>) -> Response {
  let snapshot = db_snapshot(&state).await;
  (
    [(
      axum::http::header::CONTENT_TYPE,
      "text/plain; version=0.0.4; charset=utf-8",
    )],
    render(&state, snapshot.as_ref()),
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
  let _in_flight = METRICS.request_started();
  let method = req.method().as_str().to_string();
  let route = req
    .extensions()
    .get::<MatchedPath>()
    .map(|p| p.as_str().to_string())
    .unwrap_or_else(|| "<unmatched>".to_string());
  let started = Instant::now();
  let res = next.run(req).await;
  METRICS.record_http(
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

  /// The exposition has to parse as Prometheus text, and the easiest way to get
  /// that wrong is a label value carrying a quote.
  #[test]
  fn label_values_are_escaped() {
    assert_eq!(esc(r#"a"b"#), r#"a\"b"#);
    assert_eq!(esc(r"a\b"), r"a\\b");
    assert_eq!(esc("a\nb"), "a\\nb");
  }

  /// A histogram bucket is CUMULATIVE — `le="0.5"` counts everything at or
  /// below 0.5, not just the slice above the previous bound. Backwards yields a
  /// histogram that looks plausible and quantiles that are nonsense.
  #[test]
  fn buckets_are_cumulative() {
    let mut h = Histogram::default();
    h.observe(0.001);
    h.observe(0.3);
    assert_eq!(h.buckets[0], 1, "0.005 holds only the 1ms sample");
    assert_eq!(h.buckets[6], 2, "0.5 holds both");
    assert_eq!(h.buckets[BUCKETS.len() - 1], 2, "+Inf holds everything");
    assert_eq!(h.count, 2);
  }

  /// Rendering with and without labels are different code paths (the comma
  /// before `le`, the braces on `_sum`), and a malformed line poisons the whole
  /// scrape rather than just its own series.
  #[test]
  fn histogram_renders_both_labelled_and_bare() {
    let mut h = Histogram::default();
    h.observe(0.02);

    let mut bare = String::new();
    h.render(&mut bare, "m", "");
    assert!(bare.contains("m_bucket{le=\"0.025\"} 1"), "{bare}");
    assert!(bare.contains("m_count 1"), "{bare}");
    assert!(!bare.contains("{}"), "no empty brace pair: {bare}");

    let mut labelled = String::new();
    h.render(&mut labelled, "m", "route=\"/x\"");
    assert!(
      labelled.contains("m_bucket{route=\"/x\",le=\"0.025\"} 1"),
      "{labelled}"
    );
    assert!(labelled.contains("m_count{route=\"/x\"} 1"), "{labelled}");
  }

  #[test]
  fn statuses_are_counted_separately() {
    let m = Metrics::new();
    m.record_http("GET", "/api/x", 200, 0.01);
    m.record_http("GET", "/api/x", 200, 0.01);
    m.record_http("GET", "/api/x", 500, 0.01);

    let routes = m.routes.lock().unwrap();
    let stat = &routes[&("GET".to_string(), "/api/x".to_string())];
    assert_eq!(stat.by_status[&200], 2);
    assert_eq!(stat.by_status[&500], 1);
    assert_eq!(stat.latency.count, 3, "the histogram counts every request");
  }

  /// The gauges must come back down on an early return or a panic, or a few
  /// dropped connections turn into a permanently wrong number nobody can tell
  /// from real load.
  #[test]
  fn gauges_return_to_zero() {
    static M: LazyLock<Metrics> = LazyLock::new(Metrics::new);
    {
      let _a = M.ws_connected();
      let _b = M.ws_connected();
      let _c = M.request_started();
      assert_eq!(M.ws_connections.load(Ordering::Relaxed), 2);
      assert_eq!(M.in_flight.load(Ordering::Relaxed), 1);
    }
    assert_eq!(M.ws_connections.load(Ordering::Relaxed), 0);
    assert_eq!(M.in_flight.load(Ordering::Relaxed), 0);
  }

  /// A rejected push must move BOTH the integrity counter and the rejected
  /// counter — they answer different questions ("is the data plane sound" vs
  /// "are clients being turned away") and collapsing them loses one.
  #[test]
  fn a_refused_push_is_counted_twice_on_purpose() {
    let m = Metrics::new();
    m.record_push(0.01, 128, false);
    m.integrity_failure("push");

    assert_eq!(m.push.lock().unwrap().rejected, 1);
    assert_eq!(m.push.lock().unwrap().bytes, 128);
    assert_eq!(m.integrity_failures.lock().unwrap()[&"push"], 1);
  }
}
