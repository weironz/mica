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
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        for part in xff.rsplit(',') {
            if let Ok(ip) = part.trim().parse::<IpAddr>() {
                if !is_proxy_hop(ip) {
                    return ip;
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
        let mut buckets = self.buckets.lock().unwrap();
        // Opportunistic cleanup so a spray of unique IPs can't grow the map
        // without bound: drop buckets that have idled back to full.
        if buckets.len() > 8192 {
            let cap = self.capacity;
            let rate = self.refill_per_sec;
            buckets.retain(|_, b| {
                let refilled =
                    b.tokens + now.saturating_duration_since(b.last).as_secs_f64() * rate;
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
        // Per-IP: burst then ~`per_min` sustained. Argon2: ~cores concurrent.
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
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok()?.trim().parse().ok()
}

/// Per-IP throttled: the credential endpoints. `refresh` is included (a rotating
/// refresh-token endpoint is an abuse vector) but does NOT hash, so it skips the
/// Argon2 gate below. WS connects are deliberately NOT here: they are already
/// token-authenticated and a shared per-IP bucket would throttle a user opening
/// several documents at once — not worth the false positives for a low-severity
/// authed path.
fn is_rate_limited(path: &str) -> bool {
    spends_argon2(path) || path.ends_with("/auth/refresh")
}

/// Endpoints that run an Argon2 hash/verify — the ones the concurrency gate must
/// bound. `refresh` is excluded: it is a DB token rotation, and holding a scarce
/// Argon2 permit for it would starve real logins.
fn spends_argon2(path: &str) -> bool {
    path.ends_with("/auth/login") || path.ends_with("/auth/register")
}

/// Outer middleware: throttle the auth endpoints per client IP and cap
/// concurrent Argon2 work. Every non-auth path passes straight through.
pub async fn auth_rate_limit(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Extension(guard): Extension<Arc<AuthGuard>>,
    req: Request,
    next: Next,
) -> Response {
    let (limited, argon2) = {
        let path = req.uri().path();
        (is_rate_limited(path), spends_argon2(path))
    };
    if !limited {
        return next.run(req).await;
    }
    let ip = client_ip(req.headers(), peer);
    if !guard.limiter.allow(ip) {
        return throttled("too many attempts; slow down and try again shortly");
    }
    // login/register also share a global Argon2 concurrency budget; drop the
    // permit when the request ends. `refresh` is rate-limited above but does no
    // hashing, so it holds no permit (that would starve real logins).
    let _permit = if argon2 {
        match guard.argon2.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => return throttled("the server is busy; try again in a moment"),
        }
    } else {
        None
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

    #[test]
    fn refresh_is_rate_limited_but_not_argon2_gated() {
        assert!(is_rate_limited("/api/auth/login"));
        assert!(is_rate_limited("/api/auth/register"));
        assert!(is_rate_limited("/api/auth/refresh"));
        assert!(!is_rate_limited("/api/workspaces"));
        assert!(!is_rate_limited("/ws/workspaces/x/documents/y"));
        // Only login/register hash — refresh must not hold an Argon2 permit.
        assert!(spends_argon2("/api/auth/login"));
        assert!(spends_argon2("/api/auth/register"));
        assert!(!spends_argon2("/api/auth/refresh"));
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
