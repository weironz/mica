// Mica pure-Rust WebAssembly offline shell Service Worker.
// Stamped at build time by scripts/build-web-rust.sh with the current WASM &
// preview bundle hashes so each release uses a distinct versioned CacheStorage
// bucket and prunes older release caches on activation.

const CACHE_NAME = "mica-shell-__WASM_HASH__-__PREVIEW_HASH__";
const CORE_PRECACHE = ["./", "./favicon.svg"];

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) => cache.addAll(CORE_PRECACHE))
      .then(() => self.skipWaiting())
      .catch(() => {})
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys
            .filter((k) => k.startsWith("mica-shell-") && k !== CACHE_NAME)
            .map((k) => caches.delete(k))
        )
      )
      .then(() => self.clients.claim())
  );
});

function isDynamicOrBypassPath(pathname) {
  return (
    pathname.startsWith("/api/") ||
    pathname.startsWith("/ws/") ||
    pathname.startsWith("/s/") ||
    pathname === "/reset-password" ||
    pathname === "/verify-email" ||
    pathname === "/privacy" ||
    pathname === "/terms" ||
    pathname === "/sw.js"
  );
}

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;

  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  if (isDynamicOrBypassPath(url.pathname)) return;

  // HTML navigations use Network-First so new releases take effect immediately,
  // falling back to the cached shell when F5-reloaded offline.
  if (req.mode === "navigate" || url.pathname === "/" || url.pathname === "/index.html") {
    event.respondWith(
      fetch(req)
        .then((resp) => {
          if (resp && resp.ok && resp.type === "basic") {
            const clone = resp.clone();
            caches
              .open(CACHE_NAME)
              .then((cache) => cache.put("./", clone))
              .catch(() => {});
          }
          return resp;
        })
        .catch(async () => (await caches.match("./")) || Response.error())
    );
    return;
  }

  // Content-hashed static JS/WASM (?v=...), lazy preview modules, and icons use
  // Cache-First so offline reloads boot instantaneously without network.
  const hasVersionQuery = url.searchParams.has("v");
  const isCacheableStatic =
    (hasVersionQuery && /\.(?:js|wasm)$/.test(url.pathname)) ||
    url.pathname.startsWith("/mica_preview/") ||
    url.pathname === "/favicon.svg";

  if (!isCacheableStatic) return;

  event.respondWith(
    caches.match(req).then((cached) => {
      if (cached) return cached;
      return fetch(req).then((resp) => {
        if (resp && resp.ok && resp.type === "basic") {
          const clone = resp.clone();
          caches
            .open(CACHE_NAME)
            .then((cache) => cache.put(req, clone))
            .catch(() => {});
        }
        return resp;
      });
    })
  );
});
