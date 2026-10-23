//! Rate limiting for the unauthenticated auth endpoints (login / register), for
//! a single-node deployment behind reverse proxies.
//!
//! Design (from the 2026-07 survey of Vaultwarden / Authelia / Authentik /
//! Gitea): an in-app **per-IP token bucket** plus a **global Argon2 concurrency
//! gate**. The per-IP bucket stops a single source from brute-forcing or
//! spending unbounded Argon2 CPU; the Argon2 semaphore is the only thing that
//! also bounds a *distributed* flood (many IPs, each under the per-IP limit)
//! from pinning every core (~19 MiB per Argon2id op). Deliberately NO hard
//! per-account lock: that lets a third party lock a victim out by failing their
//! logins (lockout-DoS) — the lesson Authelia/Authentik encode by self-healing
//! or challenging instead of denying. In-memory (resets on redeploy, single
//! node only); a horizontally-scaled deployment would need a shared store.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::Extension;
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::sync::Semaphore;

/// Resolve the real client IP behind mica's proxy chain.
///
/// XFF is only CONSULTED when the socket peer is itself a proxy hop
/// ([`is_proxy_hop`]): `X-Forwarded-For` is a client-controlled header, so a
/// directly-exposed socket (peer outside any private/loopback range) that
/// believed it would let anyone spoof arbitrary source IPs and walk straight
/// past the per-IP limiter — each forged value opens a fresh bucket (#34).
/// Only a peer sitting in a proxy segment could legitimately have appended
/// the chain, so only then is the header worth reading.
///
/// Prod is a DOUBLE hop (`client → Traefik → nginx → axum`): the `api` service
/// has no Traefik labels, so `/api` is funnelled through the `web`/nginx
/// container, and nginx's `proxy_set_header X-Real-IP $remote_addr` overwrites
/// `X-Real-IP` with its own peer (Traefik's docker IP) — so that header is
/// useless and the client is the *leftmost* `X-Forwarded-For` entry. The
/// single-server variant is one hop and the client is the *rightmost*. Rather
/// than trust topology, walk XFF right-to-left and return the first PUBLIC
/// address: every proxy hop sits in a private/loopback range (docker networks),
/// and any client-injected spoof always lands to the LEFT of the real entry the
/// proxy chain appended, so it is never reached. Fall back to the socket peer
/// when XFF is absent or entirely private (e.g. a same-LAN client — acceptable,
/// mica's real clients reach it over the internet).
pub fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> IpAddr {
  if is_proxy_hop(peer.ip()) {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
      for part in xff.rsplit(',') {
        if let Ok(ip) = part.trim().parse::<IpAddr>() {
          if !is_proxy_hop(ip) {
            return ip;
          }
        }
      }
    }
  }
  peer.ip()
}

/// Private / loopback / link-local / CGNAT — i.e. "a proxy hop, not a client".
fn is_proxy_hop(ip: IpAddr) -> bool {
  match ip {
    IpAddr::V4(v4) => {
      v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                // 100.64.0.0/10 (CGNAT)
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
    }
    IpAddr::V6(v6) => {
      v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 (ULA)
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 (link-local)
    }
  }
}

/// Per-IP token bucket. `capacity` is the burst; tokens refill at
/// `refill_per_sec`. A plain `Mutex<HashMap>` is ample at one node's login rate.
pub struct RateLimiter {
  buckets: Mutex<HashMap<IpAddr, Bucket>>,
  capacity: f64,
  refill_per_sec: f64,
}

struct Bucket {
  tokens: f64,
  last: Instant,
}

impl RateLimiter {
  pub fn new(capacity: u32, refill_per_sec: f64) -> Self {
    Self {
      buckets: Mutex::new(HashMap::new()),
      capacity: f64::from(capacity),
      refill_per_sec,
    }
  }

  /// `now` is injected so tests can advance time; production calls [`allow`].
  fn allow_at(&self, ip: IpAddr, now: Instant) -> bool {
    let ip = bucket_key(ip);
    let mut buckets = self.buckets.lock().unwrap();
    // Opportunistic cleanup so a spray of unique IPs can't grow the map
    // without bound: drop buckets that have idled back to full.
    if buckets.len() > 8192 {
      let cap = self.capacity;
      let rate = self.refill_per_sec;
      buckets.retain(|_, b| {
        let refilled = b.tokens + now.saturating_duration_since(b.last).as_secs_f64() * rate;
        refilled < cap
      });
    }
    let bucket = buckets.entry(ip).or_insert(Bucket {
      tokens: self.capacity,
      last: now,
    });
    let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
    bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.capacity);
    bucket.last = now;
    if bucket.tokens >= 1.0 {
      bucket.tokens -= 1.0;
      true
    } else {
      false
    }
  }

  pub fn allow(&self, ip: IpAddr) -> bool {
    self.allow_at(ip, Instant::now())
  }
}

/// The bucket an address is charged to.
///
/// IPv6: the /64. A single subscriber is routinely handed a whole /64 (often
/// far more) and can pick any address in it, so a per-ADDRESS bucket let one
/// client rotate past the limit indefinitely. IPv4-mapped IPv6 is charged as
/// the IPv4 address it is.
fn bucket_key(ip: IpAddr) -> IpAddr {
  match ip {
    IpAddr::V4(_) => ip,
    IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
      Some(v4) => IpAddr::V4(v4),
      None => {
        let s = v6.segments();
        IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
      }
    },
  }
}

/// Shared per-IP limiter + Argon2 concurrency gate for the auth endpoints.
pub struct AuthGuard {
  limiter: RateLimiter,
  /// Bounds concurrent password hashes/verifies. Held for the whole auth
  /// request (its dominant cost IS the Argon2 op), so excess concurrent
  /// attempts shed with 429 instead of pinning every core.
  argon2: Arc<Semaphore>,
}

impl AuthGuard {
  pub fn from_env() -> Arc<Self> {
    // Per-IP: burst then ~`per_min` sustained. Argon2: ~cores/2 concurrent.
    let burst = env_parse("AUTH_RATE_BURST").unwrap_or(10);
    let per_min = env_parse::<f64>("AUTH_RATE_PER_MIN").unwrap_or(5.0);
    let permits = env_parse("AUTH_ARGON2_MAX_CONCURRENCY")
      .unwrap_or_else(default_argon2_permits)
      .max(1);
    Arc::new(Self {
      limiter: RateLimiter::new(burst, per_min / 60.0),
      argon2: Arc::new(Semaphore::new(permits)),
    })
  }
}

fn default_argon2_permits() -> usize {
  // HALF the cores, not all of them (#42): the gate's whole point is that a
  // login flood cannot take the machine away from everything else, and a
  // full set of permits would let Argon2 alone saturate every core — WS
  // traffic and `/ready` included, even with the hashing itself off the
  // workers on `spawn_blocking`. `from_env`'s `.max(1)` keeps a 1-core
  // machine at one permit.
  std::thread::available_parallelism()
    .map(|n| n.get() / 2)
    .unwrap_or(2)
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
  std::env::var(key).ok()?.trim().parse().ok()
}

/// Route middleware: throttle THIS route per client IP. Attached where the
/// route is registered (`routes::public_api_router`, `api_argon2_router`,
/// `password_reset::router`) — route-layered by design rather than matched
/// against a path table that must be hand-synced with the router (the blob
/// link and resend-verification were each silently misclassified that way).
///
/// On these routes by policy: `/auth/refresh` (a rotating refresh-token
/// endpoint is an abuse vector, but it does NO hashing so it skips the Argon2
/// gate below), the password-reset pair (`forgot` could spam a victim's inbox
/// or probe timing; the `/reset-password` POST is a token-guessing surface),
/// and `/auth/resend-verification` (each call mails a fresh link — unthrottled,
/// one loop flooded a victim's inbox and spent the operator's mail quota and
/// sender reputation). WS connects are deliberately NOT throttled: they are
/// already token-authenticated and a shared per-IP bucket would throttle a
/// user opening several documents at once — not worth the false positives for
/// a low-severity authed path.
pub async fn throttle_ip(
  ConnectInfo(peer): ConnectInfo<SocketAddr>,
  Extension(guard): Extension<Arc<AuthGuard>>,
  req: Request,
  next: Next,
) -> Response {
  let ip = client_ip(req.headers(), peer);
  if !guard.limiter.allow(ip) {
    return throttled("too many attempts; slow down and try again shortly");
  }
  next.run(req).await
}

/// Route middleware: [`throttle_ip`] plus the global Argon2 concurrency gate,
/// for the routes whose handler runs an Argon2 hash/verify (`/auth/login`,
/// `/auth/register`, `POST /auth/password`, `DELETE /auth/me`). The permit is
/// held for the whole request — its dominant cost IS the Argon2 op — so excess
/// concurrent attempts shed with 429 instead of pinning every core.
///
/// `POST /auth/password` and `DELETE /auth/me` are signed-in, but each
/// verifies the CURRENT password with Argon2: with a stolen access token (or
/// cookie) they were an unthrottled password oracle, and any account could
/// fire hundreds at once and pin every core — the flood the semaphore exists
/// to stop. The `GET`/`PATCH` methods on `/auth/me` carry neither the throttle
/// nor the gate: a GET of `/auth/me` is the profile read and must stay free.
pub async fn throttle_argon2(
  ConnectInfo(peer): ConnectInfo<SocketAddr>,
  Extension(guard): Extension<Arc<AuthGuard>>,
  req: Request,
  next: Next,
) -> Response {
  let ip = client_ip(req.headers(), peer);
  if !guard.limiter.allow(ip) {
    return throttled("too many attempts; slow down and try again shortly");
  }
  let Ok(_permit) = guard.argon2.clone().try_acquire_owned() else {
    return throttled("the server is busy; try again in a moment");
  };
  next.run(req).await
}

fn throttled(msg: &str) -> Response {
  (StatusCode::TOO_MANY_REQUESTS, msg.to_string()).into_response()
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::net::Ipv4Addr;
  use std::time::Duration;

  fn hm(xff: Option<&str>) -> HeaderMap {
    let mut h = HeaderMap::new();
    if let Some(v) = xff {
      h.insert("x-forwarded-for", v.parse().unwrap());
    }
    h
  }

  const PEER: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 20, 0, 5)), 40000);

  #[test]
  fn prod_double_hop_takes_the_client_left_of_the_docker_hops() {
    // client, then nginx appended Traefik's docker IP.
    let ip = client_ip(&hm(Some("203.0.113.7, 10.0.0.2")), PEER);
    assert_eq!(ip, "203.0.113.7".parse::<IpAddr>().unwrap());
  }

  #[test]
  fn single_server_hop_takes_the_appended_rightmost_client() {
    let ip = client_ip(&hm(Some("203.0.113.7")), PEER);
    assert_eq!(ip, "203.0.113.7".parse::<IpAddr>().unwrap());
  }

  #[test]
  fn a_spoofed_left_entry_is_never_reached() {
    // Attacker prepends a fake public IP; the proxy chain appends the real
    // client (and a docker hop). Walking from the right lands on the real
    // client; the spoof sits to its left, unreachable.
    let ip = client_ip(&hm(Some("8.8.8.8, 203.0.113.7, 10.0.0.2")), PEER);
    assert_eq!(ip, "203.0.113.7".parse::<IpAddr>().unwrap());
    // Single-server spoof: nginx appends the real peer last.
    let ip2 = client_ip(&hm(Some("8.8.8.8, 203.0.113.7")), PEER);
    assert_eq!(ip2, "203.0.113.7".parse::<IpAddr>().unwrap());
  }

  #[test]
  fn all_private_or_no_xff_falls_back_to_the_socket_peer() {
    assert_eq!(client_ip(&hm(Some("10.0.0.1, 10.0.0.2")), PEER), PEER.ip());
    assert_eq!(client_ip(&hm(None), PEER), PEER.ip());
  }

  /// #34: XFF is a client-controlled header. A directly-exposed socket (the
  /// peer is a PUBLIC address, not a proxy hop) must not read it at all — else
  /// anyone forges a fresh "client IP" per request and never touches the
  /// per-IP bucket guarding brute force.
  #[test]
  fn xff_is_ignored_when_the_peer_is_not_a_proxy_hop() {
    let public_peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9)), 40000);
    assert_eq!(
      client_ip(&hm(Some("8.8.8.8")), public_peer),
      public_peer.ip(),
      "a non-proxy peer's XFF must never be trusted"
    );
    // Even a public entry among proxies does not rescue a header the peer had
    // no business relaying.
    assert_eq!(
      client_ip(&hm(Some("203.0.113.7, 10.0.0.2")), public_peer),
      public_peer.ip()
    );
  }

  #[test]
  fn bucket_allows_the_burst_then_throttles_and_refills() {
    let rl = RateLimiter::new(2, 1.0); // burst 2, 1 token/sec
    let ip: IpAddr = "203.0.113.7".parse().unwrap();
    let t0 = Instant::now();
    assert!(rl.allow_at(ip, t0));
    assert!(rl.allow_at(ip, t0));
    assert!(!rl.allow_at(ip, t0), "burst exhausted");
    // One second later, one token has refilled.
    assert!(rl.allow_at(ip, t0 + Duration::from_secs(1)));
    assert!(!rl.allow_at(ip, t0 + Duration::from_secs(1)));
  }

  /// One subscriber owns a whole /64; a per-address bucket let them rotate
  /// past the limit forever.
  #[test]
  fn an_ipv6_slash_64_shares_one_bucket() {
    let rl = RateLimiter::new(2, 0.0);
    let now = Instant::now();
    let a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
    let b: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
    let other: IpAddr = "2001:db8:1:3::1".parse().unwrap();
    assert!(rl.allow_at(a, now));
    assert!(rl.allow_at(b, now));
    assert!(!rl.allow_at(a, now), "same /64, budget spent");
    assert!(
      rl.allow_at(other, now),
      "a different /64 is a different client"
    );
    let mapped: IpAddr = "::ffff:203.0.113.7".parse().unwrap();
    assert_eq!(bucket_key(mapped), "203.0.113.7".parse::<IpAddr>().unwrap());
  }

  #[test]
  fn buckets_are_independent_per_ip() {
    let rl = RateLimiter::new(1, 0.0);
    let a: IpAddr = "203.0.113.7".parse().unwrap();
    let b: IpAddr = "203.0.113.8".parse().unwrap();
    let t0 = Instant::now();
    assert!(rl.allow_at(a, t0));
    assert!(!rl.allow_at(a, t0));
    assert!(rl.allow_at(b, t0), "a different IP has its own bucket");
  }
}

/// The middlewares themselves, exercised AS route layers (drive an assembled
/// router rather than a table — the classifications they encode now live on
/// the routes, asserted per-route in `routes::mod`'s `router_tests`).
#[cfg(test)]
mod route_layer_tests {
  use super::*;
  use axum::Router;
  use axum::body::Body;
  use axum::routing::post;
  use std::net::Ipv4Addr;
  use tower::ServiceExt;

  async fn ok() -> &'static str {
    "ok"
  }

  const CLIENT: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), 40000);

  fn post_req(path: &str) -> Request<Body> {
    let mut req = Request::post(path).body(Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo(CLIENT));
    req
  }

  /// A 2-token burst that never refills, and `permits` Argon2 slots.
  fn guard(permits: usize) -> Arc<AuthGuard> {
    Arc::new(AuthGuard {
      limiter: RateLimiter::new(2, 0.0),
      argon2: Arc::new(Semaphore::new(permits)),
    })
  }

  #[tokio::test]
  async fn throttle_ip_allows_the_burst_then_answers_429() {
    let app = Router::new()
      .route(
        "/x",
        post(ok).route_layer(axum::middleware::from_fn(throttle_ip)),
      )
      .layer(Extension(guard(1)));
    for _ in 0..2 {
      let response = app.clone().oneshot(post_req("/x")).await.unwrap();
      assert_eq!(response.status(), StatusCode::OK);
    }
    let response = app.oneshot(post_req("/x")).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
  }

  #[tokio::test]
  async fn throttle_ip_holds_no_argon2_permit() {
    let guard = guard(1);
    // The only permit is already held: `throttle_ip` (refresh / forgot /
    // reset / resend-verification) must still serve — holding one would
    // starve real logins, the exact failure the split exists to prevent.
    let _held = guard.argon2.clone().try_acquire_owned().unwrap();
    let app = Router::new()
      .route(
        "/x",
        post(ok).route_layer(axum::middleware::from_fn(throttle_ip)),
      )
      .layer(Extension(guard));
    let response = app.oneshot(post_req("/x")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
  }

  #[tokio::test]
  async fn throttle_argon2_sheds_when_every_permit_is_held() {
    let guard = guard(1);
    let _held = guard.argon2.clone().try_acquire_owned().unwrap();
    let app = Router::new()
      .route(
        "/x",
        post(ok).route_layer(axum::middleware::from_fn(throttle_argon2)),
      )
      .layer(Extension(guard));
    let response = app.oneshot(post_req("/x")).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
  }

  #[tokio::test]
  async fn throttle_argon2_tracks_the_bucket_too() {
    let app = Router::new()
      .route(
        "/x",
        post(ok).route_layer(axum::middleware::from_fn(throttle_argon2)),
      )
      .layer(Extension(guard(4)));
    for _ in 0..2 {
      let response = app.clone().oneshot(post_req("/x")).await.unwrap();
      assert_eq!(response.status(), StatusCode::OK);
    }
    let response = app.oneshot(post_req("/x")).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
  }
}

/// [`throttle_ip`] attached as a `route_layer` on one method router — the
/// spelling used at each mount, so a route's throttle travels with its
/// registration instead of with a table.
pub fn ip_throttled<S>(route: axum::routing::MethodRouter<S>) -> axum::routing::MethodRouter<S>
where
  S: Clone + Send + Sync + 'static,
{
  route.route_layer(axum::middleware::from_fn(throttle_ip))
}

/// [`throttle_argon2`] attached as a `route_layer` on one method router.
pub fn argon2_throttled<S>(route: axum::routing::MethodRouter<S>) -> axum::routing::MethodRouter<S>
where
  S: Clone + Send + Sync + 'static,
{
  route.route_layer(axum::middleware::from_fn(throttle_argon2))
}

/// Route-layer tests in `routes::mod` build a guard through here: a 2-token
/// burst that never refills and `permits` Argon2 slots, so "throttled" and
/// "gated" each show up deterministically on the third rapid call from one
/// client.
#[cfg(test)]
pub fn test_guard(permits: usize) -> Arc<AuthGuard> {
  Arc::new(AuthGuard {
    limiter: RateLimiter::new(2, 0.0),
    argon2: Arc::new(Semaphore::new(permits)),
  })
}

#[cfg(test)]
impl AuthGuard {
  /// Seize one Argon2 permit and hold it — the "every permit is taken" state
  /// the shed-429 tests are built on.
  pub fn argon2_hog(self: &Arc<Self>) -> tokio::sync::OwnedSemaphorePermit {
    self
      .argon2
      .clone()
      .try_acquire_owned()
      .expect("test_semaphore has permits left")
  }
}
